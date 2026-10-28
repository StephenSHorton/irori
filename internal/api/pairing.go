package api

import (
	"context"
	"errors"
	"net/http"
	"strings"
	"unicode"

	"github.com/StephenSHorton/irori/internal/mesh/roomnet"
	"github.com/StephenSHorton/irori/internal/pair"
)

// pairBackend is a backend with v0.4's pairing (core.Core). The handlers
// below call only its joining side (routes_pair.go).
type pairBackend interface {
	Pairing() *pair.Pairer
}

func (s *server) pairer() *pair.Pairer {
	if p, ok := s.b.(pairBackend); ok {
		return p.Pairing()
	}
	return nil
}

// pairPost reports whether path is a pairing route that accepts POST.
func pairPost(path string) bool {
	switch path {
	case pathPairJoin, pathPairKey, pathPairCancel:
		return true
	}
	return false
}

// JoinView is what the API shows of st: everything on the joining side,
// and only the mode and state of an Add, whose code and key are secrets
// the person types (§3).
func JoinView(st pair.Status) pair.Status {
	if st.Mode == pair.ModeAdd {
		return pair.Status{Mode: st.Mode, State: st.State}
	}
	return st
}

// pairRoutes serves routes_pair.go; false means not one of them.
func (h *handler) pairRoutes(w http.ResponseWriter, r *http.Request) bool {
	p := r.URL.Path
	switch p {
	case pathPairJoin, pathPairStatus, pathPairKey, pathPairCancel:
	default:
		return false
	}
	get := r.Method == http.MethodGet
	if get != (p == pathPairStatus) {
		allow := http.MethodPost
		if p == pathPairStatus {
			allow = http.MethodGet
		}
		w.Header().Set("Allow", allow)
		writeError(w, http.StatusMethodNotAllowed, "method not allowed")
		return true
	}
	pr := h.s.pairer()
	if pr == nil {
		writeError(w, http.StatusServiceUnavailable, "this Core has no pairing")
		return true
	}
	switch p {
	case pathPairStatus:
		writeJSON(w, http.StatusOK, JoinView(pr.Status()))
	case pathPairJoin:
		h.pairJoin(w, r, pr)
	case pathPairKey:
		h.pairKey(w, r, pr)
	case pathPairCancel:
		pr.CancelJoin()
		writeJSON(w, http.StatusOK, JoinView(pr.Status()))
	}
	return true
}

// pairField trims a body field and refuses one too long or with control
// characters. The message never repeats the value.
func pairField(v, what string) (string, error) {
	v = strings.TrimSpace(v)
	if len(v) > MaxPairField || strings.ContainsFunc(v, unicode.IsControl) {
		return "", errors.New(what + " is too long or has control characters")
	}
	return v, nil
}

func (h *handler) pairJoin(w http.ResponseWriter, r *http.Request, pr *pair.Pairer) {
	var body struct {
		Code   string `json:"code"`
		Server string `json:"server"`
	}
	if err := readBody(w, r, &body); err != nil {
		writeError(w, http.StatusBadRequest, err.Error())
		return
	}
	code, err := pairField(body.Code, "code")
	if err != nil {
		writeError(w, http.StatusBadRequest, err.Error())
		return
	}
	server, err := pairField(body.Server, "server")
	if err != nil {
		writeError(w, http.StatusBadRequest, err.Error())
		return
	}
	if server != "" {
		hp, err := roomnet.HostPort(server, 0)
		if err != nil {
			writeError(w, http.StatusBadRequest, "the home server address is a host and port, like 203.0.113.7:24817")
			return
		}
		server = hp
	}
	st, err := pr.JoinVia(code, server)
	switch {
	case errors.Is(err, pair.ErrBadCode):
		writeError(w, http.StatusBadRequest, err.Error())
	case errors.Is(err, pair.ErrBusy), errors.Is(err, pair.ErrInMesh):
		writeError(w, http.StatusConflict, err.Error())
	case err != nil:
		writeError(w, http.StatusInternalServerError, err.Error())
	default:
		writeJSON(w, http.StatusOK, JoinView(st))
	}
}

func (h *handler) pairKey(w http.ResponseWriter, r *http.Request, pr *pair.Pairer) {
	var body struct {
		Key string `json:"key"`
	}
	if err := readBody(w, r, &body); err != nil {
		writeError(w, http.StatusBadRequest, err.Error())
		return
	}
	key, err := pairField(body.Key, "key")
	if err != nil {
		writeError(w, http.StatusBadRequest, err.Error())
		return
	}
	// The grant may be adopted inside this call: a client that hangs up
	// must not cut that short, so it runs past the request's end.
	err = pr.ConfirmKey(context.WithoutCancel(r.Context()), key)
	switch {
	case errors.Is(err, pair.ErrWrongKey):
		writeError(w, http.StatusUnprocessableEntity, err.Error())
	case errors.Is(err, pair.ErrNotOpen):
		// ErrNotOpen's own text is about words, the adding side's step.
		writeError(w, http.StatusConflict, "no pairing is waiting for a key")
	case err != nil:
		writeError(w, http.StatusInternalServerError, err.Error())
	default:
		writeJSON(w, http.StatusOK, JoinView(pr.Status()))
	}
}
