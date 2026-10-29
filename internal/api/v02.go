package api

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strconv"
	"strings"
	"unicode"

	"github.com/StephenSHorton/irori/internal/attach"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/tools"
)

// maxRequestBody caps a v0.2 POST body (tool args carry at most a few
// 8000-char texts).
const maxRequestBody = 256 << 10

// MaxInboxLimit caps GET /v1/inbox?limit=.
const MaxInboxLimit = 500

// postRoute reports whether path accepts POST.
func postRoute(path string) bool {
	switch path {
	case "/v1/members", "/v1/policy",
		"/v1/deliveries/delivered", "/v1/deliveries/engaged", "/v1/deliveries/turn-ended":
		return true
	}
	return strings.HasPrefix(path, "/v1/tools/")
}

// v02 serves the routes in routes_v02.go; false means not one of them.
func (h *handler) v02(w http.ResponseWriter, r *http.Request) bool {
	p := r.URL.Path
	switch {
	case p == "/v1/attach", p == "/v1/members", p == "/v1/policy", p == "/v1/inbox",
		p == "/v1/tools", strings.HasPrefix(p, "/v1/tools/"), strings.HasPrefix(p, "/v1/deliveries/"):
	default:
		return false
	}
	if h.s.b.Inbox() == nil {
		writeError(w, http.StatusServiceUnavailable, "messaging is not running")
		return true
	}
	get := r.Method == http.MethodGet
	switch {
	case p == "/v1/attach" && get:
		h.attach(w, r)
	case p == "/v1/members" && !get:
		h.join(w, r)
	case p == "/v1/policy" && get:
		writeJSON(w, http.StatusOK, h.s.b.Inbox().Policy())
	case p == "/v1/policy":
		h.setPolicy(w, r)
	case p == "/v1/inbox" && get:
		h.inbox(w, r)
	case p == "/v1/tools" && get:
		h.toolSpecs(w, r)
	case strings.HasPrefix(p, "/v1/tools/") && !get:
		h.tool(w, r, strings.TrimPrefix(p, "/v1/tools/"))
	case strings.HasPrefix(p, "/v1/deliveries/") && !get:
		h.control(w, r, strings.TrimPrefix(p, "/v1/deliveries/"))
	case postRoute(p) == get:
		// A GET on a POST-only route or the reverse.
		allow := http.MethodGet
		if postRoute(p) {
			allow = http.MethodPost
		}
		w.Header().Set("Allow", allow)
		writeError(w, http.StatusMethodNotAllowed, "method not allowed")
	default:
		writeError(w, http.StatusNotFound, "no such route")
	}
	return true
}

// caller is who a request acts as, from exactly one identity header.
type caller struct {
	sender inbox.Sender
	path   inbox.Path
	attach bool // through a mod attach secret (not a member)
}

// caller resolves X-Irori-Attach / X-Irori-Member. ok is false when neither
// (or both) is present or the secret is unknown, superseded or closed.
func (h *handler) caller(r *http.Request) (caller, bool) {
	as, ms := r.Header.Get(attach.HeaderAttach), r.Header.Get(attach.HeaderMember)
	switch {
	case as != "" && ms == "":
		st := h.s.streams.bySecret(as)
		if st == nil || st.path != inbox.PathMod {
			return caller{}, false
		}
		snd := st.sender
		for _, c := range h.s.b.Agents() {
			if c.Key == snd.Key && c.Name != "" {
				snd.Name = c.Name // a renamed session sends under its new name
				break
			}
		}
		return caller{sender: snd, path: inbox.PathMod, attach: true}, true
	case ms != "" && as == "":
		m, ok := h.s.b.MemberBySecret(ms)
		if !ok {
			return caller{}, false
		}
		return caller{sender: inbox.Sender{Key: m.Key, Name: m.Name, Kind: "grok"}, path: inbox.PathChannel}, true
	}
	return caller{}, false
}

func forbidden(w http.ResponseWriter) {
	writeError(w, http.StatusForbidden, "this route needs a current "+attach.HeaderAttach+" or "+attach.HeaderMember+" secret")
}

// readBody decodes a JSON object body into v. An empty body leaves v as is.
func readBody(w http.ResponseWriter, r *http.Request, v any) error {
	raw, err := io.ReadAll(http.MaxBytesReader(w, r.Body, maxRequestBody))
	if err != nil {
		return fmt.Errorf("body: %w", err)
	}
	if len(strings.TrimSpace(string(raw))) == 0 {
		return nil
	}
	if err := json.Unmarshal(raw, v); err != nil {
		return fmt.Errorf("body must be a JSON object: %w", err)
	}
	return nil
}

// inboxStatus maps inbox errors to HTTP statuses.
func inboxStatus(err error) int {
	switch {
	case errors.Is(err, inbox.ErrNotFound), errors.Is(err, inbox.ErrUnknownAgent):
		return http.StatusNotFound
	case errors.Is(err, inbox.ErrNotYours):
		return http.StatusForbidden
	case errors.Is(err, inbox.ErrBadState), errors.Is(err, inbox.ErrAmbiguous):
		return http.StatusConflict
	case errors.Is(err, inbox.ErrRateLimited):
		return http.StatusTooManyRequests
	case errors.Is(err, inbox.ErrTooLong):
		return http.StatusRequestEntityTooLarge
	}
	return http.StatusInternalServerError
}

type okBody struct {
	OK bool `json:"ok"`
}

// control serves POST /v1/deliveries/{delivered,engaged,turn-ended}.
func (h *handler) control(w http.ResponseWriter, r *http.Request, what string) {
	if what != "delivered" && what != "engaged" && what != "turn-ended" {
		writeError(w, http.StatusNotFound, "no such route")
		return
	}
	c, ok := h.caller(r)
	if !ok || !c.attach {
		forbidden(w)
		return
	}
	var body struct {
		Token  string `json:"token"`
		TurnID string `json:"turnId"`
		// Appended (engaged only): the mod appended this wake after its
		// host refused the submit, so the wake did not start the turn.
		Appended bool `json:"appended"`
	}
	if err := readBody(w, r, &body); err != nil {
		writeError(w, http.StatusBadRequest, err.Error())
		return
	}
	if strings.TrimSpace(body.Token) == "" {
		writeError(w, http.StatusBadRequest, "token is required")
		return
	}
	ib, agent := h.s.b.Inbox(), c.sender.Key
	var err error
	switch what {
	case "delivered":
		err = ib.MarkDelivered(r.Context(), agent, body.Token)
	case "engaged":
		if body.Appended {
			err = ib.MarkAppendedEngaged(r.Context(), agent, body.Token, body.TurnID)
		} else {
			err = ib.MarkEngaged(r.Context(), agent, body.Token, body.TurnID)
		}
	case "turn-ended":
		err = ib.TurnEnded(r.Context(), agent, body.Token, body.TurnID)
	}
	if err != nil {
		writeError(w, inboxStatus(err), err.Error())
		return
	}
	writeJSON(w, http.StatusOK, okBody{OK: true})
}

func (h *handler) toolSpecs(w http.ResponseWriter, r *http.Request) {
	client := r.URL.Query().Get("client")
	switch client {
	case "":
		client = "claude"
	case "claude", "grok":
	default:
		writeError(w, http.StatusBadRequest, "client must be claude or grok")
		return
	}
	ts := h.s.b.Tools()
	specs := []tools.Spec{}
	if ts != nil {
		if s := ts.Specs(client); s != nil {
			specs = s
		}
	}
	writeJSON(w, http.StatusOK, specs)
}

// ToolResult is the POST /v1/tools/{name} body.
type ToolResult struct {
	Text string `json:"text"`
}

func (h *handler) tool(w http.ResponseWriter, r *http.Request, name string) {
	c, ok := h.caller(r)
	if !ok {
		forbidden(w)
		return
	}
	ts := h.s.b.Tools()
	if ts == nil {
		writeError(w, http.StatusServiceUnavailable, "tools are not running")
		return
	}
	var body struct {
		Args json.RawMessage `json:"args"`
	}
	if err := readBody(w, r, &body); err != nil {
		writeError(w, http.StatusBadRequest, err.Error())
		return
	}
	args := body.Args
	if len(args) == 0 || string(args) == "null" {
		args = json.RawMessage(`{}`)
	}
	text, err := ts.Call(r.Context(), tools.Caller{Sender: c.sender, Path: c.path}, name, args)
	switch {
	case errors.Is(err, tools.ErrUnknownTool):
		writeError(w, http.StatusNotFound, "no tool called "+strconv.Quote(name))
		return
	case err != nil:
		writeError(w, http.StatusBadRequest, err.Error())
		return
	}
	writeJSON(w, http.StatusOK, ToolResult{Text: text})
}

func (h *handler) setPolicy(w http.ResponseWriter, r *http.Request) {
	c, ok := h.caller(r)
	if !ok || !c.attach || r.Header.Get(attach.HeaderOrigin) != "composer" {
		writeError(w, http.StatusForbidden, "policy changes need an attached mod and a person at the composer ("+attach.HeaderOrigin+": composer)")
		return
	}
	var p inbox.Policy
	if err := readBody(w, r, &p); err != nil {
		writeError(w, http.StatusBadRequest, err.Error())
		return
	}
	ib := h.s.b.Inbox()
	// Typed at the composer, but through the loopback API: v0.4 (§2) lets
	// only the GUI mint "user" rows, so the row's actor is "api:composer@<key>",
	// which still names the session.
	if err := ib.SetPolicy(r.Context(), p, inbox.APIKeyPrefix+"composer@"+c.sender.Key); err != nil {
		writeError(w, http.StatusBadRequest, err.Error())
		return
	}
	writeJSON(w, http.StatusOK, ib.Policy())
}

func (h *handler) inbox(w http.ResponseWriter, r *http.Request) {
	c, ok := h.caller(r)
	if !ok {
		forbidden(w)
		return
	}
	q := r.URL.Query()
	all := q.Get("all") == "1" || q.Get("all") == "true"
	limit := inbox.DefaultInboxList
	if v := q.Get("limit"); v != "" {
		n, err := strconv.Atoi(v)
		if err != nil || n < 1 {
			writeError(w, http.StatusBadRequest, "limit must be a positive integer")
			return
		}
		limit = min(n, MaxInboxLimit)
	}
	writeJSON(w, http.StatusOK, h.s.b.Inbox().For(c.sender.Key, all, limit))
}

// MaxMemberName caps a member's display name (the inbox alias limit).
const MaxMemberName = inbox.MaxAliasLen

func (h *handler) join(w http.ResponseWriter, r *http.Request) {
	var body struct {
		Name    string `json:"name"`
		Session string `json:"session"`
	}
	if err := readBody(w, r, &body); err != nil {
		writeError(w, http.StatusBadRequest, err.Error())
		return
	}
	name := strings.TrimSpace(body.Name)
	if name == "" || len([]rune(name)) > MaxMemberName || strings.ContainsFunc(name, unicode.IsControl) {
		writeError(w, http.StatusBadRequest, fmt.Sprintf("name is required: 1-%d characters, no control characters", MaxMemberName))
		return
	}
	if len(body.Session) > 200 || strings.ContainsFunc(body.Session, unicode.IsControl) {
		writeError(w, http.StatusBadRequest, "session is too long or has control characters")
		return
	}
	m, err := h.s.b.JoinMember(r.Context(), name, strings.TrimSpace(body.Session))
	if err != nil {
		writeError(w, http.StatusInternalServerError, err.Error())
		return
	}
	writeJSON(w, http.StatusOK, m)
}
