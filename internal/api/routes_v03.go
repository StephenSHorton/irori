package api

// v0.3 routes (docs/design/v0.3-capture.md §10). CONTRACT for the api, mcp
// and GUI work packages: paths, query parameters, statuses and JSON shapes
// below are binding. All routes keep v0.1's rules (bearer token, Host check,
// no Origin). They are GET-only reads open to any token holder, like
// GET /v1/agents: no X-Irori-Attach or X-Irori-Member secret is needed.
// Writes (decisions) go through POST /v1/tools/{name} as in v0.2.
//
// When the backend has no capture service (Backend.Capture() is nil) every
// route below answers 503. A malformed parameter is 400. Errors are
// {"error": "<text>"}. Lists are never null: an empty result is [].
//
// This list supersedes design §10 where they differ: `before` is an RFC 3339
// time (the last entry's "at"), not a turn id, and the timeline body is
// capture.Timeline, not {turns, open}.
//
//	GET /v1/timeline?agent=<key>&before=<RFC3339>&limit=N&subagents=1
//	    -> capture.Timeline {key, entries: []capture.Entry, more}
//	    agent is required. limit counts turns (default 20, max 100; larger is
//	    capped). Page with before = the last entry's at while more is true.
//	    The open turn of a live session comes first, only when before is
//	    absent. subagents=1 (or true) includes subagent and workflow turns.
//	    Irori messages are not in the timeline: merge them from the inbox.
//
//	GET /v1/edits?repo=<repoKey>&path=<repo-relative>&agent=<key>&limit=N
//	    -> []capture.EditRow, newest first (limit default 50, max 500)
//
//	GET /v1/commits?repo=<repoKey>&limit=N
//	    -> []capture.CommitView, newest committer time first; no repo = all
//	    repos (limit default 50, max 500)
//
//	GET /v1/commits/{sha}
//	    -> capture.CommitView. sha is 7-64 hex digits: a full sha or a unique
//	    prefix (any repo). 400 when not hex or too short; 404 when unknown or
//	    when the prefix matches more than one commit.
//
//	GET /v1/blobs/{id}
//	    -> the blob's bytes (a patch or a summary), Content-Type
//	    "text/plain; charset=utf-8", X-Content-Type-Options: nosniff,
//	    Cache-Control: no-store. id is 64 lowercase hex (400 otherwise,
//	    checked before any filesystem access); 404 when not stored. Errors are
//	    still JSON.
//
//	GET /v1/decisions?repo=<repoKey>&path=<repo-relative>&query=<text>&all=1&limit=N
//	    -> []capture.DecisionView, newest first, anchors re-checked. all=1 (or
//	    true) includes superseded decisions. limit default 50, max 500.
//	    Backslashes in path are read as forward slashes.
//
//	GET /v1/prs?repo=<owner/name>
//	    -> []ledger.Entry for that GitHub repo; no repo = all.
//
//	GET /v1/capture/status
//	    -> capture.Status
//
//	GET /v1/why?path=<p>&line=N&repo=<repoKey>&cwd=<dir>
//	    -> capture.WhyResult. path is required; absolute, or relative to cwd
//	    (then to the repo root). line is optional, 1-based. 404 when the path
//	    is not inside a known git repo (capture.ErrNoRepo).

// Limits for the v0.3 list routes. Larger limits are capped, not refused.
const (
	MaxTimelineLimit = 100
	MaxCaptureLimit  = 500
)
