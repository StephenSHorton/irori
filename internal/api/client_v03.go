package api

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"time"

	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/decision"
	"github.com/StephenSHorton/irori/internal/ledger"
)

// Client methods for the v0.3 routes (routes_v03.go). None needs an
// Identity: the reads are open to any token holder.

// Timeline reads one agent's history page (GET /v1/timeline). q.Before zero
// is the newest page.
func (c *Client) Timeline(ctx context.Context, q capture.TimelineQuery) (capture.Timeline, error) {
	v := url.Values{"agent": {q.Key}}
	if !q.Before.IsZero() {
		v.Set("before", q.Before.UTC().Format(time.RFC3339Nano))
	}
	if q.Limit > 0 {
		v.Set("limit", strconv.Itoa(q.Limit))
	}
	if q.Subagents {
		v.Set("subagents", "1")
	}
	var tl capture.Timeline
	err := c.impl.get(ctx, "/v1/timeline", v, true, &tl)
	return tl, err
}

// Edits lists captured edits (GET /v1/edits).
func (c *Client) Edits(ctx context.Context, q capture.EditQuery) ([]capture.EditRow, error) {
	v := url.Values{}
	setNonEmpty(v, "repo", q.RepoKey)
	setNonEmpty(v, "path", q.Path)
	setNonEmpty(v, "agent", q.Key)
	if q.Limit > 0 {
		v.Set("limit", strconv.Itoa(q.Limit))
	}
	var out []capture.EditRow
	err := c.impl.get(ctx, "/v1/edits", v, true, &out)
	return out, err
}

// Commits lists a repo's commits ("" = all repos) (GET /v1/commits).
func (c *Client) Commits(ctx context.Context, repoKey string, limit int) ([]capture.CommitView, error) {
	v := url.Values{}
	setNonEmpty(v, "repo", repoKey)
	if limit > 0 {
		v.Set("limit", strconv.Itoa(limit))
	}
	var out []capture.CommitView
	err := c.impl.get(ctx, "/v1/commits", v, true, &out)
	return out, err
}

// Commit reads one commit by sha or unique 7+ hex prefix
// (GET /v1/commits/{sha}); an unknown or ambiguous sha is a 404 StatusError.
func (c *Client) Commit(ctx context.Context, sha string) (capture.CommitView, error) {
	var cv capture.CommitView
	err := c.impl.get(ctx, "/v1/commits/"+url.PathEscape(sha), nil, true, &cv)
	return cv, err
}

// Blob reads a stored patch or summary (GET /v1/blobs/{id}).
func (c *Client) Blob(ctx context.Context, id string) ([]byte, error) {
	return c.impl.raw(ctx, "/v1/blobs/"+url.PathEscape(id))
}

// Decisions lists decisions with their anchors re-checked (GET /v1/decisions).
func (c *Client) Decisions(ctx context.Context, f decision.Filter) ([]capture.DecisionView, error) {
	v := url.Values{}
	setNonEmpty(v, "repo", f.RepoKey)
	setNonEmpty(v, "path", f.Path)
	setNonEmpty(v, "query", f.Query)
	if f.All {
		v.Set("all", "1")
	}
	if f.Limit > 0 {
		v.Set("limit", strconv.Itoa(f.Limit))
	}
	var out []capture.DecisionView
	err := c.impl.get(ctx, "/v1/decisions", v, true, &out)
	return out, err
}

// PRs lists PR ledger entries for a GitHub "owner/name" ("" = all)
// (GET /v1/prs).
func (c *Client) PRs(ctx context.Context, repo string) ([]ledger.Entry, error) {
	v := url.Values{}
	setNonEmpty(v, "repo", repo)
	var out []ledger.Entry
	err := c.impl.get(ctx, "/v1/prs", v, true, &out)
	return out, err
}

// CaptureStatus reads capture's health (GET /v1/capture/status).
func (c *Client) CaptureStatus(ctx context.Context) (capture.Status, error) {
	var st capture.Status
	err := c.impl.get(ctx, "/v1/capture/status", nil, true, &st)
	return st, err
}

// Why asks what is behind a path and line (GET /v1/why). A path outside any
// known repo is a 404 StatusError.
func (c *Client) Why(ctx context.Context, q capture.WhyQuery) (capture.WhyResult, error) {
	v := url.Values{"path": {q.Path}}
	if q.Line > 0 {
		v.Set("line", strconv.Itoa(q.Line))
	}
	setNonEmpty(v, "repo", q.RepoKey)
	setNonEmpty(v, "cwd", q.Cwd)
	var res capture.WhyResult
	err := c.impl.get(ctx, "/v1/why", v, true, &res)
	return res, err
}

func setNonEmpty(v url.Values, k, s string) {
	if s != "" {
		v.Set(k, s)
	}
}

// raw GETs a non-JSON body with the token; error bodies are still JSON.
func (c *client) raw(ctx context.Context, path string) ([]byte, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, c.base+path, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", "Bearer "+c.lf.Token)
	req.Header.Set("Accept", "text/plain")
	resp, err := c.http.Do(req)
	if err != nil {
		return nil, fmt.Errorf("irori api %s: %w", path, err)
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(io.LimitReader(resp.Body, maxBody))
	if err != nil {
		return nil, fmt.Errorf("irori api %s: %w", path, err)
	}
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		var eb errorBody
		_ = json.Unmarshal(body, &eb)
		return nil, &StatusError{Path: path, Code: resp.StatusCode, Message: eb.Error}
	}
	return body, nil
}
