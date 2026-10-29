package api

import (
	"context"
	"crypto/subtle"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/discovery"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// Event query limits for GET /v1/events.
const (
	DefaultEventLimit = 100
	MaxEventLimit     = 1000
)

// Health is the GET /v1/health body.
type Health struct {
	OK       bool   `json:"ok"`
	Version  string `json:"version"`
	DeviceID string `json:"deviceId"`
	// Net is the mesh (v0.4 §6.8); absent when the backend has none.
	Net *NetHealth `json:"net,omitempty"`
}

// NetHealth is the net layer's state (off, on, removed, error) and how
// many other devices are connected now.
type NetHealth struct {
	State string `json:"state"`
	Peers int    `json:"peers"`
}

// meshBackend is a backend with v0.4's mesh (core.Core); GET /v1/devices
// and health's net read it.
type meshBackend interface {
	Mesh() *mesh.Node
}

func (s *server) node() *mesh.Node {
	if m, ok := s.b.(meshBackend); ok {
		return m.Mesh()
	}
	return nil
}

type server struct {
	b     Backend
	token []byte
	// person is the per-run secret of the person's view (decision D10(a)):
	// minted here, held only in memory and handed to the GUI, never
	// written to disk or logged.
	person  []byte
	streams *streams
}

func newServer(b Backend, token string) *Server {
	person, err := discovery.NewToken()
	if err != nil {
		panic(fmt.Sprintf("api: no randomness for the person's secret: %v", err))
	}
	return &Server{impl: &server{b: b, token: []byte(token), person: []byte(person), streams: newStreams()}}
}

// personSecret is Server.PersonSecret.
func (s *Server) personSecret() string { return string(s.impl.person) }

// asPerson reads the person header: present says whether one was sent,
// ok whether it is this run's secret. Constant-time.
func (s *server) asPerson(r *http.Request) (present, ok bool) {
	vals := r.Header.Values(HeaderPerson)
	if len(vals) == 0 {
		return false, false
	}
	return true, len(vals) == 1 && len(s.person) > 0 && subtle.ConstantTimeCompare([]byte(vals[0]), s.person) == 1
}

// handler serves the routes for a listener on port.
func (s *Server) handler(port int) http.Handler {
	return &handler{s: s.impl, hosts: allowedHosts(port), port: port}
}

func (s *Server) serve(ctx context.Context, ln net.Listener) error {
	tcp, ok := ln.Addr().(*net.TCPAddr)
	if !ok {
		ln.Close()
		return fmt.Errorf("api: listener %s is not TCP", ln.Addr())
	}
	// Cancel on return too, so the shutdown goroutine never outlives a failed Serve.
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	srv := &http.Server{
		Handler:           s.handler(tcp.Port),
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       10 * time.Second,
		WriteTimeout:      30 * time.Second, // attach streams lift it for themselves
		IdleTimeout:       60 * time.Second,
		MaxHeaderBytes:    64 << 10,
		// Requests (attach streams above all) end when ctx does, so Shutdown
		// never waits on a stream that is only idling.
		BaseContext: func(net.Listener) context.Context { return ctx },
	}
	stopped := make(chan struct{})
	go func() {
		defer close(stopped)
		<-ctx.Done()
		sctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		if err := srv.Shutdown(sctx); err != nil {
			srv.Close()
		}
	}()
	err := srv.Serve(ln)
	if errors.Is(err, http.ErrServerClosed) {
		<-stopped
		return nil
	}
	return err
}

// allowedHosts are the Host headers a local client sends. Anything else
// (a rebinding attacker's domain pointing at 127.0.0.1) is refused.
func allowedHosts(port int) map[string]bool {
	p := strconv.Itoa(port)
	return map[string]bool{
		"127.0.0.1:" + p: true,
		"localhost:" + p: true,
	}
}

type handler struct {
	s     *server
	hosts map[string]bool
	port  int
}

func (h *handler) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	// Browsers always send Origin on cross-origin requests and on any
	// non-GET; no legitimate client of this API is a browser.
	if _, ok := r.Header["Origin"]; ok {
		writeError(w, http.StatusForbidden, "browser requests are not allowed")
		return
	}
	if !h.hosts[strings.ToLower(r.Host)] {
		writeError(w, http.StatusForbidden, "bad host")
		return
	}
	post := postRoute(r.URL.Path) || pairPost(r.URL.Path)
	if r.Method != http.MethodGet && !(post && r.Method == http.MethodPost) {
		allow := http.MethodGet
		if post {
			allow += ", " + http.MethodPost
		}
		w.Header().Set("Allow", allow)
		writeError(w, http.StatusMethodNotAllowed, "method not allowed")
		return
	}
	if r.URL.Path == "/v1/health" {
		hl := Health{OK: true, Version: h.s.b.Version(), DeviceID: h.s.b.Device().ID}
		if n := h.s.node(); n != nil {
			hl.Net = &NetHealth{State: n.Status().State, Peers: n.Connected()}
		}
		writeJSON(w, http.StatusOK, hl)
		return
	}
	// Auth before routing, so an unauthenticated caller can't map the routes.
	if !h.s.authorized(r) {
		w.Header().Set("WWW-Authenticate", `Bearer realm="irori"`)
		writeError(w, http.StatusUnauthorized, "missing or wrong token")
		return
	}
	if h.v02(w, r) || h.v03(w, r) || h.board(w, r) || h.pairRoutes(w, r) {
		return
	}
	switch r.URL.Path {
	case "/v1/device":
		writeJSON(w, http.StatusOK, h.s.b.Device())
	case "/v1/agents":
		agents := h.s.b.Agents()
		if agents == nil {
			agents = []presence.AgentCard{}
		}
		writeJSON(w, http.StatusOK, agents)
	case "/v1/events":
		h.events(w, r)
	case "/v1/events/stream":
		h.eventsStream(w, r)
	case "/v1/devices":
		h.devices(w, r)
	default:
		writeError(w, http.StatusNotFound, "no such route")
	}
}

func (s *server) authorized(r *http.Request) bool {
	if len(s.token) == 0 {
		return false // never serve with an empty token
	}
	got, ok := strings.CutPrefix(r.Header.Get("Authorization"), "Bearer ")
	if !ok {
		return false
	}
	return subtle.ConstantTimeCompare([]byte(got), s.token) == 1
}

func (h *handler) events(w http.ResponseWriter, r *http.Request) {
	person, ok := h.rowsView(w, r)
	if !ok {
		return
	}
	q := r.URL.Query()
	f := store.Filter{Limit: DefaultEventLimit, Desc: true}
	for _, k := range q["kind"] {
		if k != "" {
			f.Kinds = append(f.Kinds, k)
		}
	}
	f.DeviceID = q.Get("device")
	if v := q.Get("limit"); v != "" {
		n, err := strconv.Atoi(v)
		if err != nil || n < 1 {
			writeError(w, http.StatusBadRequest, "limit must be a positive integer")
			return
		}
		f.Limit = min(n, MaxEventLimit)
	}
	evs, err := h.s.b.Events(r.Context(), f)
	if err != nil {
		writeError(w, http.StatusInternalServerError, err.Error())
		return
	}
	if evs == nil {
		evs = []store.Event{}
	}
	for i := range evs {
		evs[i] = viewRow(evs[i], person)
	}
	writeJSON(w, http.StatusOK, evs)
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}

type errorBody struct {
	Error string `json:"error"`
}

func writeError(w http.ResponseWriter, status int, msg string) {
	writeJSON(w, status, errorBody{Error: msg})
}

// devices serves GET /v1/devices[?detail=1]: the person's devices, this one
// first, as Settings → Devices shows them; detail adds what is behind each
// state and warning (path, RTT, skew, refused and held rows).
func (h *handler) devices(w http.ResponseWriter, r *http.Request) {
	n := h.s.node()
	if n == nil {
		writeError(w, http.StatusServiceUnavailable, "this Core has no devices layer")
		return
	}
	d := r.URL.Query().Get("detail")
	writeJSON(w, http.StatusOK, n.Devices(d == "1" || d == "true"))
}
