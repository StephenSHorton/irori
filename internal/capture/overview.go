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
	"fmt"
	"slices"
	"sort"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/presence"
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

// Overview builds the overview for agent key `forKey`. When in.RepoKey is
// "" the caller's live card's repo is used for the agent order, commits and
// PRs.
func (c *Capture) Overview(ctx context.Context, forKey string, in OverviewInput) Overview {
	return c.impl.overview(forKey, in)
}

func (c *capture) overview(forKey string, in OverviewInput) Overview {
	cards := c.liveCards() // the roster, before c.mu
	agents := make([]AgentLine, 0, len(cards))
	for _, cd := range cards {
		agents = append(agents, AgentLine{
			Key: cd.Key, Name: cardName(cd.Name, cd.Title, cd.Key), Status: cd.Status, RepoKey: cd.RepoKey,
		})
		if cd.Key == forKey && in.RepoKey == "" {
			in.RepoKey = cd.RepoKey
		}
	}
	rank := func(a AgentLine) int {
		switch {
		case a.Key == forKey:
			return 0
		case in.RepoKey != "" && a.RepoKey == in.RepoKey:
			return 1
		}
		return 2
	}
	sort.Slice(agents, func(i, j int) bool {
		a, b := agents[i], agents[j]
		if ra, rb := rank(a), rank(b); ra != rb {
			return ra < rb
		}
		if na, nb := strings.ToLower(a.Name), strings.ToLower(b.Name); na != nb {
			return na < nb
		}
		return a.Key < b.Key
	})
	out := Overview{
		For: forKey, RepoKey: in.RepoKey, Agents: agents,
		InboxOpen: in.InboxOpen, InboxHeld: in.InboxHeld,
		Inbox: capLines(in.Inbox), Blockers: capLines(in.Blockers), Tasks: capLines(in.Tasks),
		MoreTasks: in.MoreTasks + max(0, len(in.Tasks)-OverviewLines),
	}
	if len(agents) > OverviewAgents {
		out.Agents, out.MoreAgents = agents[:OverviewAgents], len(agents)-OverviewAgents
	}

	c.mu.RLock()
	for i := range out.Agents {
		a := &out.Agents[i]
		a.Doing = c.doing(a.Key, in.Current)
	}
	c.mu.RUnlock()

	if in.RepoKey != "" {
		for _, v := range c.commits(in.RepoKey, OverviewLines) {
			out.Commits = append(out.Commits, Line{Ref: commitRef(v.SHA), Text: clipLine(v.Subject), At: v.At})
		}
		if repo := ledger.RepoFromKey(in.RepoKey); repo != "" { // "" would mean every repo
			seen := map[string]bool{}
			for _, e := range c.prs(repo) {
				if seen[e.Key] {
					continue
				}
				seen[e.Key] = true
				if len(out.PRs) == OverviewLines {
					break
				}
				text := fmt.Sprintf("PR %s#%d %s %s", e.Repo, e.Number, e.Role, c.nameOf(e.Session, cards))
				out.PRs = append(out.PRs, Line{Ref: e.Key, Text: clipLine(text), At: e.At})
			}
		}
	}
	out.Hint = "show <ref> for any line; search <words> for older history; board for every card"
	if in.BoardCount > 0 {
		out.Hint += fmt.Sprintf(" (%d open)", in.BoardCount)
	}
	if out.MoreAgents > 0 {
		out.Hint += fmt.Sprintf("; %d more agents live (search by agent)", out.MoreAgents)
	}
	return out
}

// doing is an agent's one line; c.mu is read-locked. The open turn's
// prompt (ref agent:<key>, the open turn has no row yet), else the task it
// holds (in.Current), else its last reply.
func (c *capture) doing(key string, current map[string]Line) Line {
	if t := c.open[openKey(key, "")]; t != nil && strings.TrimSpace(t.Prompt) != "" {
		return Line{Ref: agentRef(key), Text: clipLine(hideMessages(c.scrub.String(t.Prompt), hideAll, key)), At: t.StartedAt}
	}
	if l, ok := current[key]; ok && l.Ref != "" {
		l.Text = clipLine(l.Text)
		return l
	}
	var last *TurnRow
	for _, t := range c.turns[key] {
		if t.AgentID == "" && strings.TrimSpace(t.Reply) != "" && (last == nil || t.StartedAt.After(last.StartedAt)) {
			last = t
		}
	}
	if last != nil {
		return Line{Ref: turnRef(last.ID), Text: clipLine(last.Reply), At: last.StartedAt}
	}
	return Line{Ref: agentRef(key)}
}

// nameOf is a live agent's display name, else its key.
func (c *capture) nameOf(key string, cards map[string]presence.AgentCard) string {
	for _, cd := range cards {
		if cd.Key == key {
			return cardName(cd.Name, cd.Title, cd.Key)
		}
	}
	return key
}

// clipLine is the first non-blank line of s, at most OverviewLineMax runes.
func clipLine(s string) string { return clipRunes(oneLine(firstLine(s)), OverviewLineMax) }

// capLines keeps at most OverviewLines lines, each clipped to one line.
func capLines(ls []Line) []Line {
	if len(ls) == 0 {
		return nil
	}
	out := slices.Clone(ls[:min(len(ls), OverviewLines)])
	for i := range out {
		out[i].Text = clipLine(out[i].Text)
	}
	return out
}
