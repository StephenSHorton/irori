package api

import (
	"errors"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/decision"
	"github.com/StephenSHorton/irori/internal/ledger"
)

// v03 serves the routes in routes_v03.go; false means not one of them.
func (h *handler) v03(w http.ResponseWriter, r *http.Request) bool {
	p := r.URL.Path
	switch {
	case p == "/v1/timeline", p == "/v1/edits", p == "/v1/commits", p == "/v1/decisions",
		p == "/v1/prs", p == "/v1/capture/status", p == "/v1/why",
		strings.HasPrefix(p, "/v1/commits/"), strings.HasPrefix(p, "/v1/blobs/"):
	default:
		return false
	}
	c := h.s.b.Capture()
	if c == nil {
		writeError(w, http.StatusServiceUnavailable, "capture is not running")
		return true
	}
	// Every v0.3 route is GET; ServeHTTP has already refused other methods.
	switch {
	case p == "/v1/timeline":
		h.timeline(w, r, c)
	case p == "/v1/edits":
		h.edits(w, r, c)
	case p == "/v1/commits":
		h.commits(w, r, c)
	case strings.HasPrefix(p, "/v1/commits/"):
		h.commit(w, c, strings.TrimPrefix(p, "/v1/commits/"))
	case strings.HasPrefix(p, "/v1/blobs/"):
		h.blob(w, c, strings.TrimPrefix(p, "/v1/blobs/"))
	case p == "/v1/decisions":
		h.decisions(w, r, c)
	case p == "/v1/prs":
		prs := c.PRs(strings.TrimSpace(r.URL.Query().Get("repo")))
		if prs == nil {
			prs = []ledger.Entry{}
		}
		writeJSON(w, http.StatusOK, prs)
	case p == "/v1/capture/status":
		writeJSON(w, http.StatusOK, c.Status())
	case p == "/v1/why":
		h.why(w, r, c)
	}
	return true
}

// limitParam reads ?limit=: def when absent, capped at hi, an error
// (already written as 400) when not a positive integer.
func limitParam(w http.ResponseWriter, q url.Values, def, hi int) (int, bool) {
	return positiveParam(w, q, "limit", def, hi)
}

// positiveParam is limitParam for any parameter name.
func positiveParam(w http.ResponseWriter, q url.Values, name string, def, hi int) (int, bool) {
	v := q.Get(name)
	if v == "" {
		return def, true
	}
	n, err := strconv.Atoi(v)
	if err != nil || n < 1 {
		writeError(w, http.StatusBadRequest, name+" must be a positive integer")
		return 0, false
	}
	return min(n, hi), true
}

func boolParam(q url.Values, name string) bool {
	v := q.Get(name)
	return v == "1" || v == "true"
}

func (h *handler) timeline(w http.ResponseWriter, r *http.Request, c *capture.Capture) {
	q := r.URL.Query()
	tq := capture.TimelineQuery{Key: strings.TrimSpace(q.Get("agent")), Subagents: boolParam(q, "subagents")}
	if tq.Key == "" {
		writeError(w, http.StatusBadRequest, "agent is required")
		return
	}
	if v := q.Get("before"); v != "" {
		t, err := time.Parse(time.RFC3339Nano, v)
		if err != nil {
			writeError(w, http.StatusBadRequest, "before must be an RFC 3339 time")
			return
		}
		tq.Before = t
	}
	var ok bool
	if tq.Limit, ok = limitParam(w, q, 20, MaxTimelineLimit); !ok {
		return
	}
	tl := c.Timeline(tq)
	if tl.Entries == nil {
		tl.Entries = []capture.Entry{}
	}
	writeJSON(w, http.StatusOK, tl)
}

func (h *handler) edits(w http.ResponseWriter, r *http.Request, c *capture.Capture) {
	q := r.URL.Query()
	eq := capture.EditQuery{
		RepoKey: strings.TrimSpace(q.Get("repo")),
		Path:    q.Get("path"),
		Key:     strings.TrimSpace(q.Get("agent")),
	}
	var ok bool
	if eq.Limit, ok = limitParam(w, q, 50, MaxCaptureLimit); !ok {
		return
	}
	edits := c.Edits(eq)
	if edits == nil {
		edits = []capture.EditRow{}
	}
	writeJSON(w, http.StatusOK, edits)
}

func (h *handler) commits(w http.ResponseWriter, r *http.Request, c *capture.Capture) {
	q := r.URL.Query()
	limit, ok := limitParam(w, q, 50, MaxCaptureLimit)
	if !ok {
		return
	}
	cs := c.Commits(strings.TrimSpace(q.Get("repo")), limit)
	if cs == nil {
		cs = []capture.CommitView{}
	}
	writeJSON(w, http.StatusOK, cs)
}

// isHex reports whether s is n hex digits (either case) with lo <= n <= hi.
func isHex(s string, lo, hi int) bool {
	if len(s) < lo || len(s) > hi {
		return false
	}
	for _, r := range s {
		if !('0' <= r && r <= '9' || 'a' <= r && r <= 'f' || 'A' <= r && r <= 'F') {
			return false
		}
	}
	return true
}

func (h *handler) commit(w http.ResponseWriter, c *capture.Capture, sha string) {
	if !isHex(sha, 7, 64) {
		writeError(w, http.StatusBadRequest, "sha must be 7 to 64 hex digits")
		return
	}
	v, ok := c.Commit(sha)
	if !ok {
		writeError(w, http.StatusNotFound, "no commit "+sha+" (or the prefix is ambiguous)")
		return
	}
	writeJSON(w, http.StatusOK, v)
}

func (h *handler) blob(w http.ResponseWriter, c *capture.Capture, id string) {
	data, err := c.Blob(id)
	switch {
	case errors.Is(err, blob.ErrBadID):
		writeError(w, http.StatusBadRequest, err.Error())
		return
	case errors.Is(err, blob.ErrNotFound):
		writeError(w, http.StatusNotFound, "no such blob")
		return
	case err != nil:
		writeError(w, http.StatusInternalServerError, err.Error())
		return
	}
	w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.Header().Set("Content-Length", strconv.Itoa(len(data)))
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write(data)
}

func (h *handler) decisions(w http.ResponseWriter, r *http.Request, c *capture.Capture) {
	q := r.URL.Query()
	f := decision.Filter{
		RepoKey: strings.TrimSpace(q.Get("repo")),
		Path:    strings.ReplaceAll(q.Get("path"), `\`, "/"),
		Query:   q.Get("query"),
		All:     boolParam(q, "all"),
	}
	var ok bool
	if f.Limit, ok = limitParam(w, q, 50, MaxCaptureLimit); !ok {
		return
	}
	ds := c.Decisions(f)
	if ds == nil {
		ds = []capture.DecisionView{}
	}
	writeJSON(w, http.StatusOK, ds)
}

func (h *handler) why(w http.ResponseWriter, r *http.Request, c *capture.Capture) {
	q := r.URL.Query()
	wq := capture.WhyQuery{
		Path:    strings.TrimSpace(q.Get("path")),
		RepoKey: strings.TrimSpace(q.Get("repo")),
		Cwd:     strings.TrimSpace(q.Get("cwd")),
	}
	if wq.Path == "" {
		writeError(w, http.StatusBadRequest, "path is required")
		return
	}
	if v := q.Get("line"); v != "" {
		n, err := strconv.Atoi(v)
		if err != nil || n < 1 {
			writeError(w, http.StatusBadRequest, "line must be a positive integer")
			return
		}
		wq.Line = n
	}
	res, err := c.Why(r.Context(), wq)
	switch {
	case errors.Is(err, capture.ErrNoRepo):
		writeError(w, http.StatusNotFound, err.Error())
		return
	case err != nil:
		writeError(w, http.StatusInternalServerError, err.Error())
		return
	}
	if res.Edits == nil {
		res.Edits = []capture.EditRow{}
	}
	writeJSON(w, http.StatusOK, res)
}
