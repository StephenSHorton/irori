package capture

// The overview tool's data (docs/design/board.md "Contracts").
//
// CONTRACT (implemented by the capture work package, A3). Capture fills
// what it owns (live agents and what each is doing, recent commits and PRs)
// and copies in what the caller gathered from coord and inbox
// (OverviewInput), so capture imports neither. Every line carries a ref, and
// every list is short: the overview is the top of the progressive
// disclosure, not a report.

import (
	"context"
	"time"
)

// Overview limits.
const (
	OverviewAgents  = 12  // live agents listed (others counted in MoreAgents)
	OverviewLines   = 5   // per list (commits, PRs, tasks, blockers, inbox)
	OverviewLineMax = 120 // runes per line
)

// Line is one overview line.
type Line struct {
	Ref  string    `json:"ref"`  // what to show for more
	Text string    `json:"text"` // one line, <= OverviewLineMax runes
	At   time.Time `json:"at,omitzero"`
}

// AgentLine is one live agent.
type AgentLine struct {
	Key     string `json:"key"`
	Name    string `json:"name"`
	Status  string `json:"status"` // presence status (busy, idle, ...)
	RepoKey string `json:"repoKey,omitempty"`
	// Doing is one line: the open turn's prompt if it has one; else its
	// current task (OverviewInput.Current); else its last reply. Ref is
	// "turn:<id>" (an open turn: "agent:<key>"), "task:<id>" or "turn:<id>".
	Doing Line `json:"doing"`
}

// OverviewInput is what the caller adds from the services capture doesn't
// own. Lists are already capped at OverviewLines by the caller.
type OverviewInput struct {
	InboxOpen  int             // the caller's open deliveries
	InboxHeld  int             // held from the caller
	Inbox      []Line          // its newest open deliveries ("msg:<id>")
	Blockers   []Line          // blocked tasks in the caller's repo ("task:<id>")
	Tasks      []Line          // other open tasks in the caller's repo, the caller's own first
	MoreTasks  int             // open tasks not listed
	Current    map[string]Line // agent key -> the task it holds that changed last
	RepoKey    string          // the caller's repo ("" = none)
	BoardCount int             // every open card on the Board, any repo
}

// Overview answers the overview tool for one agent.
type Overview struct {
	For        string      `json:"for"` // the caller's agent key
	RepoKey    string      `json:"repoKey,omitempty"`
	Agents     []AgentLine `json:"agents"` // live, the caller first, then same repo, then by name
	MoreAgents int         `json:"moreAgents,omitempty"`
	InboxOpen  int         `json:"inboxOpen"`
	InboxHeld  int         `json:"inboxHeld,omitempty"`
	Inbox      []Line      `json:"inbox,omitempty"`
	Blockers   []Line      `json:"blockers,omitempty"`
	Tasks      []Line      `json:"tasks,omitempty"`
	MoreTasks  int         `json:"moreTasks,omitempty"`
	Commits    []Line      `json:"commits,omitempty"` // the caller's repo, newest first ("commit:<sha>")
	PRs        []Line      `json:"prs,omitempty"`     // the caller's repo ("pr:<key>")
	// Hint is the closing line telling how to go deeper, e.g. "show <ref>
	// for any line; search <words> for older history; board for every card".
	Hint string `json:"hint"`
}

// Overview builds the overview for agent key `forKey`.
func (c *Capture) Overview(ctx context.Context, forKey string, in OverviewInput) Overview {
	return Overview{For: forKey, RepoKey: in.RepoKey}
}
