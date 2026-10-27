package api

import (
	"context"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/search"
)

// Client methods for the Board and browsing routes (routes_board.go). Each
// takes an Identity: the zero Identity is the person's view (everything);
// an attach or member secret reads as that agent.

// Board reads the Board (GET /v1/board). f.For is ignored: the viewer is
// id. f.DoneLimit 0 is the server's default.
func (c *Client) Board(ctx context.Context, id Identity, f coord.BoardFilter) (coord.BoardView, error) {
	v := url.Values{}
	setNonEmpty(v, "repo", f.RepoKey)
	if f.DoneLimit > 0 {
		v.Set("done", strconv.Itoa(f.DoneLimit))
	}
	var out coord.BoardView
	err := c.impl.do(ctx, http.MethodGet, "/v1/board", v, id, nil, nil, &out)
	return out, err
}

// Task reads one card and its comments (GET /v1/tasks/{id}); an unknown id
// is a 404 StatusError.
func (c *Client) Task(ctx context.Context, id Identity, taskID string) (TaskDetail, error) {
	var out TaskDetail
	err := c.impl.do(ctx, http.MethodGet, "/v1/tasks/"+url.PathEscape(taskID), nil, id, nil, nil, &out)
	return out, err
}

// Search finds hits (GET /v1/search). q.Viewer is ignored: the viewer is
// id. Page with q.Offset = the previous result's Next while More.
func (c *Client) Search(ctx context.Context, id Identity, q search.Query) (SearchResult, error) {
	v := url.Values{}
	setNonEmpty(v, "q", q.Text)
	if len(q.Kinds) > 0 {
		ks := make([]string, len(q.Kinds))
		for i, k := range q.Kinds {
			ks[i] = string(k)
		}
		v.Set("kinds", strings.Join(ks, ","))
	}
	setNonEmpty(v, "repo", q.RepoKey)
	setNonEmpty(v, "agent", q.Agent)
	if !q.Before.IsZero() {
		v.Set("before", q.Before.UTC().Format(time.RFC3339Nano))
	}
	if q.Offset > 0 {
		v.Set("offset", strconv.Itoa(q.Offset))
	}
	if q.Limit > 0 {
		v.Set("limit", strconv.Itoa(q.Limit))
	}
	var out SearchResult
	err := c.impl.do(ctx, http.MethodGet, "/v1/search", v, id, nil, nil, &out)
	return out, err
}

// Show renders one ref (GET /v1/show). o.Viewer is ignored: the viewer is
// id. A malformed ref is a 400 StatusError; nothing there, or nothing id may
// read, is a 404.
func (c *Client) Show(ctx context.Context, id Identity, ref string, o search.ShowOptions) (search.Shown, error) {
	v := url.Values{"ref": {ref}}
	if o.Offset > 0 {
		v.Set("offset", strconv.Itoa(o.Offset))
	}
	if o.Limit > 0 {
		v.Set("limit", strconv.Itoa(o.Limit))
	}
	var out search.Shown
	err := c.impl.do(ctx, http.MethodGet, "/v1/show", v, id, nil, nil, &out)
	return out, err
}

// Overview reads an agent's overview (GET /v1/overview). With an agent
// Identity, agent may be "" (it is the caller); the person's view needs it.
func (c *Client) Overview(ctx context.Context, id Identity, agent string) (capture.Overview, error) {
	v := url.Values{}
	setNonEmpty(v, "agent", agent)
	var out capture.Overview
	err := c.impl.do(ctx, http.MethodGet, "/v1/overview", v, id, nil, nil, &out)
	return out, err
}
