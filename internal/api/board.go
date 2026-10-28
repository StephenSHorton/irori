package api

import (
	"errors"
	"net/http"
	"net/url"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/attach"
	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/search"
)

// board serves the routes in routes_board.go; false means not one of them.
func (h *handler) board(w http.ResponseWriter, r *http.Request) bool {
	p := r.URL.Path
	switch {
	case p == "/v1/board", p == "/v1/search", p == "/v1/show", p == "/v1/overview",
		strings.HasPrefix(p, "/v1/tasks/"):
	default:
		return false
	}
	// Every route here is GET; ServeHTTP has already refused other methods.
	viewer, ok := h.viewer(w, r)
	if !ok {
		return true
	}
	switch {
	case p == "/v1/board":
		h.boardView(w, r, h.boardFor(viewer))
	case strings.HasPrefix(p, "/v1/tasks/"):
		h.task(w, strings.TrimPrefix(p, "/v1/tasks/"))
	case p == "/v1/search":
		h.search(w, r, viewer)
	case p == "/v1/show":
		h.show(w, r, viewer)
	case p == "/v1/overview":
		h.overview(w, r, viewer)
	}
	return true
}

// viewer is who a browsing request reads as: "" (the person) with this
// run's person secret, the attached session's or member's key with theirs,
// and AgentViewer with none (decision D10(a): the bearer token alone is an
// unidentified agent). A header that is sent but doesn't resolve (empty or
// blank included), or two at once, is 403 (already written), never a wider
// view.
func (h *handler) viewer(w http.ResponseWriter, r *http.Request) (string, bool) {
	agentHdr := len(r.Header.Values(attach.HeaderAttach)) > 0 || len(r.Header.Values(attach.HeaderMember)) > 0
	if present, ok := h.s.asPerson(r); present {
		if !ok || agentHdr {
			writeError(w, http.StatusForbidden, "wrong "+HeaderPerson+" secret (only the Irori app holds it; send it alone)")
			return "", false
		}
		return "", true
	}
	if !agentHdr {
		return AgentViewer, true
	}
	c, ok := h.caller(r)
	if !ok {
		writeError(w, http.StatusForbidden, "unknown or superseded "+attach.HeaderAttach+" / "+attach.HeaderMember+" secret (send at most one)")
		return "", false
	}
	return c.sender.Key, true
}

// boardFor is BoardFilter.For for a viewer: the agent's key, or for the
// person the GUI's own key ("user:<device>", Core.UserSender), so tasks
// assigned to the person land in their NeedsYou as they do in the app.
func (h *handler) boardFor(viewer string) string {
	if viewer != "" {
		return viewer
	}
	return "user:" + h.s.b.Device().ID
}

func (h *handler) coordOr503(w http.ResponseWriter) *coord.Coord {
	co := h.s.b.Coord()
	if co == nil {
		writeError(w, http.StatusServiceUnavailable, "the board is not running")
	}
	return co
}

func (h *handler) captureOr503(w http.ResponseWriter) *capture.Capture {
	c := h.s.b.Capture()
	if c == nil {
		writeError(w, http.StatusServiceUnavailable, "capture is not running")
	}
	return c
}

// intParam reads a non-negative integer ?name= (def when absent); false
// means a 400 was written.
func intParam(w http.ResponseWriter, q url.Values, name string, def int) (int, bool) {
	v := q.Get(name)
	if v == "" {
		return def, true
	}
	n, err := strconv.Atoi(v)
	if err != nil || n < 0 {
		writeError(w, http.StatusBadRequest, name+" must be a non-negative integer")
		return 0, false
	}
	return n, true
}

func (h *handler) boardView(w http.ResponseWriter, r *http.Request, forKey string) {
	co := h.coordOr503(w)
	if co == nil {
		return
	}
	q := r.URL.Query()
	done, ok := positiveParam(w, q, "done", coord.DefaultDoneCard, coord.MaxDoneCard)
	if !ok {
		return
	}
	v := co.Board(coord.BoardFilter{RepoKey: strings.TrimSpace(q.Get("repo")), For: forKey, DoneLimit: done})
	writeJSON(w, http.StatusOK, nonNilBoard(v))
}

func nonNilBoard(v coord.BoardView) coord.BoardView {
	for _, g := range []*[]coord.Card{&v.NeedsYou, &v.Blocked, &v.Working, &v.Open, &v.Done} {
		if *g == nil {
			*g = []coord.Card{}
		}
	}
	if v.Leases == nil {
		v.Leases = []coord.LeaseCard{}
	}
	return v
}

func (h *handler) task(w http.ResponseWriter, id string) {
	co := h.coordOr503(w)
	if co == nil {
		return
	}
	if id == "" || strings.Contains(id, "/") {
		writeError(w, http.StatusNotFound, "no such route")
		return
	}
	t, ok := co.GetTask(id)
	if !ok {
		writeError(w, http.StatusNotFound, "no task "+strconv.Quote(id))
		return
	}
	cs := co.Comments(id)
	if cs == nil {
		cs = []coord.Comment{}
	}
	writeJSON(w, http.StatusOK, TaskDetail{Task: t, Comments: cs})
}

// searchKinds reads ?kinds= (repeatable, comma-separated). false means a
// 400 was written.
func searchKinds(w http.ResponseWriter, q url.Values) ([]search.Kind, bool) {
	var out []search.Kind
	for _, v := range q["kinds"] {
		for _, k := range strings.Split(v, ",") {
			k = strings.ToLower(strings.TrimSpace(k))
			if k == "" {
				continue
			}
			kind := search.Kind(k)
			if kind == search.KindAgent || !slices.Contains(search.Kinds, kind) {
				writeError(w, http.StatusBadRequest, "kinds: "+strconv.Quote(k)+" is not a searchable kind")
				return nil, false
			}
			if !slices.Contains(out, kind) {
				out = append(out, kind)
			}
		}
	}
	return out, true
}

func (h *handler) search(w http.ResponseWriter, r *http.Request, viewer string) {
	c := h.captureOr503(w)
	if c == nil {
		return
	}
	q := r.URL.Query()
	sq := search.Query{
		Text:    q.Get("q"),
		RepoKey: strings.TrimSpace(q.Get("repo")),
		Agent:   strings.TrimSpace(q.Get("agent")),
		Viewer:  viewer,
	}
	var ok bool
	if sq.Kinds, ok = searchKinds(w, q); !ok {
		return
	}
	if v := q.Get("before"); v != "" {
		t, err := time.Parse(time.RFC3339Nano, v)
		if err != nil {
			writeError(w, http.StatusBadRequest, "before must be an RFC 3339 time")
			return
		}
		sq.Before = t
	}
	if sq.Offset, ok = intParam(w, q, "offset", 0); !ok {
		return
	}
	limit, ok := limitParam(w, q, search.DefaultLimit, search.MaxLimit)
	if !ok {
		return
	}
	sq.Limit = limit + 1 // one more says whether another page exists
	hits, err := c.Search(r.Context(), sq)
	if err != nil {
		if r.Context().Err() != nil {
			return // the caller went away
		}
		// Words are quoted before FTS5 sees them, and the index has no
		// sentinel for a parse error: whatever is left is the server's.
		writeError(w, http.StatusInternalServerError, err.Error())
		return
	}
	res := SearchResult{Hits: hits}
	if len(res.Hits) > limit {
		res.Hits, res.More, res.Next = res.Hits[:limit], true, sq.Offset+limit
	}
	if res.Hits == nil {
		res.Hits = []search.Hit{}
	}
	writeJSON(w, http.StatusOK, res)
}

func (h *handler) show(w http.ResponseWriter, r *http.Request, viewer string) {
	c := h.captureOr503(w)
	if c == nil {
		return
	}
	q := r.URL.Query()
	ref := strings.TrimSpace(q.Get("ref"))
	if ref == "" {
		writeError(w, http.StatusBadRequest, "ref is required")
		return
	}
	o := search.ShowOptions{Viewer: viewer}
	var ok bool
	if o.Offset, ok = intParam(w, q, "offset", 0); !ok {
		return
	}
	if o.Limit, ok = limitParam(w, q, search.DefaultShow, search.MaxShow); !ok {
		return
	}
	sh, err := c.Show(r.Context(), ref, o)
	switch {
	case errors.Is(err, search.ErrBadRef):
		writeError(w, http.StatusBadRequest, err.Error())
		return
	case errors.Is(err, search.ErrNotFound):
		writeError(w, http.StatusNotFound, "nothing has the ref "+strconv.Quote(ref))
		return
	case err != nil:
		writeError(w, http.StatusInternalServerError, err.Error())
		return
	}
	writeJSON(w, http.StatusOK, sh)
}

func (h *handler) overview(w http.ResponseWriter, r *http.Request, viewer string) {
	agent := strings.TrimSpace(r.URL.Query().Get("agent"))
	switch {
	case viewer == AgentViewer:
		writeError(w, http.StatusForbidden, "an overview carries an agent's inbox: send that agent's "+attach.HeaderAttach+" or "+attach.HeaderMember+" secret")
		return
	case viewer != "" && agent != "" && agent != viewer:
		writeError(w, http.StatusForbidden, "an agent reads only its own overview")
		return
	case viewer == "" && agent == "":
		writeError(w, http.StatusBadRequest, "agent is required")
		return
	case viewer != "":
		agent = viewer
	}
	c := h.captureOr503(w)
	if c == nil {
		return
	}
	co := h.coordOr503(w)
	if co == nil {
		return
	}
	ib := h.s.b.Inbox()
	if ib == nil {
		writeError(w, http.StatusServiceUnavailable, "messaging is not running")
		return
	}
	in := overviewInput(co, ib, h.s.b.Agents(), agent)
	ov := c.Overview(r.Context(), agent, in)
	if ov.Agents == nil {
		ov.Agents = []capture.AgentLine{}
	}
	writeJSON(w, http.StatusOK, ov)
}
