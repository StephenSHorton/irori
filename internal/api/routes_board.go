package api

// Board and browsing routes (docs/design/board.md "Contracts"). CONTRACT for
// the api, CLI and GUI work packages: paths, query parameters, statuses and
// JSON shapes below are binding. All routes keep v0.1's rules (bearer token,
// Host check, no Origin) and are GET-only reads. Agents don't use them: they
// browse with the board, overview, search and show tools (POST
// /v1/tools/{name}), and Board writes go through those tools or the GUI.
// These routes are for the CLI, tests and future clients.
//
// Viewer. Who is asking decides what a route may show (search.Query.Viewer,
// search.ShowOptions.Viewer, coord.BoardFilter.For):
//
//	X-Irori-Attach: <attachSecret>  -> that attached session; Viewer = its key
//	X-Irori-Member: <memberSecret>  -> that Grok member;      Viewer = its key
//	neither                         -> the person's view;     Viewer = ""
//
// The person's view (bearer token only) is the GUI's: it sees everything,
// messages included. An identity header that is present but empty, blank,
// unknown, superseded or closed, or both headers at once, is 403: a stale
// or unset secret never falls back to the wider person's view. A turn row
// whose prompt is an Irori fence keeps its messages' text for the person
// and the turn's own agent only (capture withholds it from everyone else).
//
// A route whose service isn't running answers 503 (board and tasks need
// coord; search, show and overview need capture; overview needs the inbox
// too). A malformed parameter is 400. Errors are {"error": "<text>"}. Lists
// are never null: an empty result is [].
//
//	GET /v1/board?repo=<repoKey>&done=N
//	    -> coord.BoardView {needsYou, blocked, working, open, done, doneTotal, leases}
//	    repo "" = every repo. NeedsYou is the viewer's: For = the agent's
//	    key, or for the person "user:<deviceId>" (Core.UserSender's key, the
//	    GUI's): tasks held by or blocked on that person, or blocked on a
//	    person with no ref, plus InputRequired. A blocker naming another
//	    person's "user:*" key is in Blocked, not NeedsYou. done caps the Done
//	    group (default coord.DefaultDoneCard, max coord.MaxDoneCard; larger
//	    is capped).
//
//	GET /v1/tasks/{id}
//	    -> TaskDetail {task: coord.Task, comments: []coord.Comment oldest first}
//	    404 when there is no such task. Tasks and comments have no readers:
//	    every viewer sees the same card.
//
//	GET /v1/search?q=<words>&kinds=<k,k>&repo=<repoKey>&agent=<key>&before=<RFC3339>&offset=N&limit=N
//	    -> SearchResult {hits: []search.Hit, more, next}
//	    q "" lists newest first. kinds may repeat and may be comma-separated;
//	    each is a search.Kind except "agent", which is show-only (400 for an
//	    unknown or show-only kind). limit default search.DefaultLimit, max
//	    search.MaxLimit (larger is capped). offset >= 0 skips hits already
//	    shown; more says another page exists and next is its offset. before
//	    is a time filter, not a pager. Docs the viewer may not read are never
//	    hits. An index failure is 500.
//
//	GET /v1/show?ref=<kind:id>&offset=N&limit=N
//	    -> search.Shown {ref, kind, title, text, more, next, related}
//	    ref is required; a malformed ref is 400. offset (bytes, >= 0) pages
//	    the text: pass the previous next while more. limit default
//	    search.DefaultShow, max search.MaxShow. 404 when nothing has the ref,
//	    and also when the viewer may not read it (the same answer, so a
//	    caller can't tell the two apart).
//
//	GET /v1/overview?agent=<key>
//	    -> capture.Overview for that agent: live agents and what each is
//	    doing, its inbox counts and newest open messages, blockers and open
//	    tasks in its repo, recent commits and PRs. Every line has a ref. An
//	    identity header picks the agent (agent=, if given, must match it:
//	    403 otherwise, since the overview carries that agent's inbox). The
//	    person's view needs agent= (400 without).

import (
	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/search"
)

// TaskDetail is the GET /v1/tasks/{id} body.
type TaskDetail struct {
	Task     coord.Task      `json:"task"`
	Comments []coord.Comment `json:"comments"`
}

// SearchResult is the GET /v1/search body.
type SearchResult struct {
	Hits []search.Hit `json:"hits"`
	More bool         `json:"more"`           // another page exists
	Next int          `json:"next,omitempty"` // the offset for it, when More
}
