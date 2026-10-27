package main

import (
	"cmp"
	"context"
	"fmt"
	"slices"
	"sort"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/presence"
)

// The cards window (docs/design/v0.4-devices.md §9, v0.3.3): BoardService's
// reads and the person's answer to a blocker. Every list is non-nil, so the
// webview never gets null.

// ScopeMine is the only scope until Shared Boards land: the person's own
// devices.
const (
	ScopeMine      = "mine"
	ScopeMineLabel = "My devices"
	// recentLines is how many activity lines Card returns.
	recentLines = 8
	// NoteItem is the blocker id of an input_required card's question: its
	// note, shown and answered like a blocker item.
	NoteItem = "note"
	// noteQuestion stands in for an input_required card with no note.
	noteQuestion = "The agent on it is waiting on your input."
)

// AgentRef is someone on a card, as the window shows them.
type AgentRef struct {
	Key  string `json:"key"`
	Name string `json:"name"` // the roster's name, "You" for the person, else the key
	// Status is the roster's status ("busy", "idle", ...), "gone" when the
	// roster has no live card, or "person".
	Status string `json:"status"`
	Live   bool   `json:"live"`  // in the roster and not gone (the live dot)
	Where  string `json:"where"` // the machine: "This PC" until devices pair (v0.4)
	// Assigned: a holder that was assigned the card and hasn't taken it up.
	Assigned bool `json:"assigned,omitempty"`
}

// CardProgress is done steps over all steps; Total 0 means no bar.
type CardProgress struct {
	Done  int `json:"done"`
	Total int `json:"total"`
}

// CardSummary is one card in the grid.
type CardSummary struct {
	ID         string          `json:"id"`
	Title      string          `json:"title"`
	State      coord.TaskState `json:"state"`
	RepoKey    string          `json:"repoKey"`
	Scope      string          `json:"scope"`      // ScopeMine
	ScopeLabel string          `json:"scopeLabel"` // "My devices"
	Progress   CardProgress    `json:"progress"`
	// OpenBlockers counts the items waiting on a decision; BlockerQuestion
	// is the first one's question ("" when none).
	OpenBlockers    int        `json:"openBlockers"`
	BlockerQuestion string     `json:"blockerQuestion"`
	Working         []AgentRef `json:"working"`  // holders, in join order
	Watching        []AgentRef `json:"watching"` // watchers that don't hold it
	UpdatedAt       time.Time  `json:"updatedAt"`
}

// CardOption is one recommended answer.
type CardOption struct {
	Answer string    `json:"answer"`
	Why    string    `json:"why"`
	By     AgentRef  `json:"by"`
	At     time.Time `json:"at"`
}

// CardResolution is how a blocker was resolved.
type CardResolution struct {
	Answer string    `json:"answer"`
	Option int       `json:"option"` // index into Options, or -1 for own words or a close
	By     AgentRef  `json:"by"`
	At     time.Time `json:"at"`
	Closed bool      `json:"closed"` // closed without an answer (unblocked, state change, Irori)
}

// CardBlocker is one blocker item, open or resolved.
type CardBlocker struct {
	ID       string          `json:"id"`
	Question string          `json:"question"`
	Context  string          `json:"context"`
	RaisedBy AgentRef        `json:"raisedBy"`
	At       time.Time       `json:"at"`
	Options  []CardOption    `json:"options"`
	Resolved *CardResolution `json:"resolved"` // null while open
}

// CardPR is a pull request linked from the card, with what capture's PR
// ledger knows: who opened it, reviewed it or otherwise touched it. CI and
// review state are not read from GitHub in this release.
// TODO(v0.4 People, decision 29): peer review requests to other people's
// agents ("Ask for a review") once Shared Boards land.
type CardPR struct {
	Repo       string     `json:"repo"` // "owner/name"
	Number     int        `json:"number"`
	URL        string     `json:"url"`
	Label      string     `json:"label"` // "GitHub #57"
	OpenedBy   []AgentRef `json:"openedBy"`
	ReviewedBy []AgentRef `json:"reviewedBy"`
	TouchedBy  []AgentRef `json:"touchedBy"`
}

// CardActivity is one line of Recent: Text is Who's name and what they did.
type CardActivity struct {
	At   time.Time `json:"at"`
	Who  AgentRef  `json:"who"`
	Text string    `json:"text"`
}

// CardDetail is one card opened beside the grid.
type CardDetail struct {
	CardSummary
	Goal     string          `json:"goal"` // the objective
	DoneWhen string          `json:"doneWhen"`
	Note     string          `json:"note"`
	Steps    []coord.Step    `json:"steps"`
	Blockers []CardBlocker   `json:"blockers"` // open first (oldest first), then resolved (newest first)
	PRs      []CardPR        `json:"prs"`
	Links    []coord.Link    `json:"links"`  // every link, PRs too
	Recent   []CardActivity  `json:"recent"` // newest first
	Comments []coord.Comment `json:"comments"`
}

// people resolves keys against the roster once per call.
type people struct {
	byKey map[string]presence.AgentCard
	user  string
}

func (b *BoardService) people() people {
	p := people{byKey: map[string]presence.AgentCard{}, user: b.user.Key}
	if b.agents != nil {
		for _, a := range b.agents() {
			p.byKey[a.Key] = a
		}
	}
	return p
}

func (p people) ref(key string) AgentRef {
	switch {
	case key == "irori":
		return AgentRef{Key: key, Name: "Irori", Status: "irori", Live: true, Where: "This PC"}
	case coord.IsPerson(key):
		name := "Someone"
		if key == p.user {
			name = "You"
		}
		return AgentRef{Key: key, Name: name, Status: "person", Live: true, Where: "This PC"}
	}
	r := AgentRef{Key: key, Name: key, Status: "gone", Where: "This PC"}
	if a, ok := p.byKey[key]; ok {
		if a.Name != "" {
			r.Name = a.Name
		}
		r.Status = a.Status
		r.Live = a.Status != "gone"
	}
	return r
}

func (p people) refs(keys []string) []AgentRef {
	out := []AgentRef{}
	for _, k := range keys {
		out = append(out, p.ref(k))
	}
	return out
}

func cardSummary(t coord.Task, p people) CardSummary {
	d, n := t.Progress()
	s := CardSummary{
		ID: t.ID, Title: t.Title, State: t.State, RepoKey: t.RepoKey,
		Scope: ScopeMine, ScopeLabel: ScopeMineLabel,
		Progress: CardProgress{Done: d, Total: n}, Working: []AgentRef{}, Watching: []AgentRef{},
		UpdatedAt: t.UpdatedAt,
	}
	if open := t.OpenBlockers(); len(open) > 0 {
		s.OpenBlockers, s.BlockerQuestion = len(open), open[0].Question
	} else if t.State == coord.Blocked {
		s.OpenBlockers, s.BlockerQuestion = 1, t.Reason
	} else if t.State == coord.InputRequired {
		s.OpenBlockers, s.BlockerQuestion = 1, cmp.Or(strings.TrimSpace(t.Note), noteQuestion)
	}
	for _, h := range t.Holders {
		r := p.ref(h)
		r.Assigned = slices.Contains(t.Assigned, h)
		s.Working = append(s.Working, r)
	}
	for _, w := range t.Watchers {
		if !slices.Contains(t.Holders, w) {
			s.Watching = append(s.Watching, p.ref(w))
		}
	}
	return s
}

// Cards lists every live card (not completed, failed or canceled) for the
// grid: cards waiting on a decision first, then the newest change first.
func (b *BoardService) Cards() []CardSummary {
	p := b.people()
	out := []CardSummary{}
	for _, t := range b.coord.ListTasks(coord.TaskFilter{Open: true}) {
		out = append(out, cardSummary(t, p))
	}
	sort.SliceStable(out, func(i, j int) bool {
		if (out[i].OpenBlockers > 0) != (out[j].OpenBlockers > 0) {
			return out[i].OpenBlockers > 0
		}
		if !out[i].UpdatedAt.Equal(out[j].UpdatedAt) {
			return out[i].UpdatedAt.After(out[j].UpdatedAt)
		}
		return out[i].ID > out[j].ID
	})
	return out
}

// Card opens one card: goal, steps, blockers with their recommended answers
// and resolutions, who is on it, PRs, recent activity and comments. Any
// card can be opened, done ones too.
func (b *BoardService) Card(id string) (CardDetail, error) {
	t, ok := b.coord.GetTask(strings.TrimSpace(id))
	if !ok {
		return CardDetail{}, fmt.Errorf("%w: task %q", coord.ErrNotFound, id)
	}
	p := b.people()
	d := CardDetail{
		CardSummary: cardSummary(t, p), Goal: t.Objective, DoneWhen: t.DoneCriteria, Note: t.Note,
		Steps: nonNil(t.Steps), Blockers: []CardBlocker{}, PRs: []CardPR{}, Links: nonNil(t.Links),
		Recent: []CardActivity{}, Comments: nonNil(b.coord.Comments(t.ID)),
	}
	var resolved []CardBlocker
	if t.State == coord.InputRequired {
		d.Blockers = append(d.Blockers, CardBlocker{ID: NoteItem, Question: d.BlockerQuestion,
			RaisedBy: p.ref(t.UpdatedBy), At: t.UpdatedAt, Options: []CardOption{}})
	}
	for _, bl := range t.Blockers {
		cb := CardBlocker{ID: bl.ID, Question: bl.Question, Context: bl.Context, RaisedBy: p.ref(bl.RaisedBy), At: bl.At, Options: []CardOption{}}
		for _, o := range bl.Options {
			cb.Options = append(cb.Options, CardOption{Answer: o.Answer, Why: o.Why, By: p.ref(o.By), At: o.At})
		}
		if r := bl.Resolved; r != nil {
			cb.Resolved = &CardResolution{Answer: r.Answer, Option: r.Option, By: p.ref(r.By), At: r.At, Closed: r.Closed}
			resolved = append(resolved, cb)
			continue
		}
		d.Blockers = append(d.Blockers, cb)
	}
	slices.Reverse(resolved)
	d.Blockers = append(d.Blockers, resolved...)
	for _, l := range t.Links {
		if pr, ok := b.cardPR(l, p); ok {
			d.PRs = append(d.PRs, pr)
		}
	}
	acts := b.coord.Activity(t.ID)
	if len(acts) > recentLines {
		acts = acts[:recentLines]
	}
	for _, a := range acts {
		who := p.ref(a.By)
		d.Recent = append(d.Recent, CardActivity{At: a.At, Who: who, Text: who.Name + " " + a.What})
	}
	return d, nil
}

// cardPR reads a GitHub PR link and what the ledger knows of it.
func (b *BoardService) cardPR(l coord.Link, p people) (CardPR, bool) {
	repo, n, ok := coord.PRLink(l)
	if !ok {
		return CardPR{}, false
	}
	pr := CardPR{Repo: repo, Number: n, URL: l.Ref, Label: l.Label, OpenedBy: []AgentRef{}, ReviewedBy: []AgentRef{}, TouchedBy: []AgentRef{}}
	if b.prs == nil {
		return pr, true
	}
	key := ledger.Key(repo, n)
	seen := map[string]bool{}
	for _, e := range b.prs(repo) {
		if e.Key != key || seen[string(e.Role)+"\x00"+e.Session] {
			continue
		}
		seen[string(e.Role)+"\x00"+e.Session] = true
		r := p.ref(e.Session)
		switch e.Role {
		case ledger.Author:
			pr.OpenedBy = append(pr.OpenedBy, r)
		case ledger.Reviewer:
			pr.ReviewedBy = append(pr.ReviewedBy, r)
		default:
			pr.TouchedBy = append(pr.TouchedBy, r)
		}
	}
	return pr, true
}

// AnswerResult is Answer's reply: the card after the answer, and who heard.
type AnswerResult struct {
	Card CardDetail `json:"card"`
	Told Told       `json:"told"`
}

// Answer resolves one blocker as the person: option is the index of the
// recommended answer picked, or -1 to answer with text. The answer reaches
// the agent that raised it (decision 18): a wake when wake is true (the
// window's default, since that agent is blocked on the person), else at
// its next turn. The card leaves blocked when no blocker is left open.
func (b *BoardService) Answer(taskID, blockerID string, option int, text string, wake bool) (AnswerResult, error) {
	if option < -1 {
		return AnswerResult{}, fmt.Errorf("%w: option is an index or -1", coord.ErrInvalid)
	}
	ctx := context.Background()
	if strings.TrimSpace(blockerID) == NoteItem {
		return b.answerInput(ctx, strings.TrimSpace(taskID), option, text, wake)
	}
	t, n, err := b.coord.Answer(ctx, b.user.Key, strings.TrimSpace(taskID), strings.TrimSpace(blockerID),
		coord.BlockerAnswer{Pick: option + 1, Text: text, Wake: wake})
	if err != nil {
		return AnswerResult{}, err
	}
	told := b.tell(ctx, n, TellNextTurn)
	d, err := b.Card(t.ID)
	if err != nil {
		return AnswerResult{}, err
	}
	return AnswerResult{Card: d, Told: told}, nil
}

// answerInput answers an input_required card's note: the answer is the
// person's comment to its holders (a wake when wake), and the card goes
// back to working (submitted when nobody holds it).
func (b *BoardService) answerInput(ctx context.Context, id string, option int, text string, wake bool) (AnswerResult, error) {
	t, ok := b.coord.GetTask(id)
	switch {
	case !ok:
		return AnswerResult{}, fmt.Errorf("%w: task %q", coord.ErrNotFound, id)
	case t.State != coord.InputRequired:
		return AnswerResult{}, fmt.Errorf("%w: task %s is %s, not waiting on your input", coord.ErrBadState, id, t.State)
	case option != -1:
		return AnswerResult{}, fmt.Errorf("%w: this question has no recommended answers; answer in your own words", coord.ErrInvalid)
	}
	_, n, err := b.coord.AddComment(ctx, b.user.Key, id, text, wake)
	if err != nil {
		return AnswerResult{}, err
	}
	told := b.tell(ctx, n, TellNextTurn)
	back := coord.Submitted
	if len(t.Holders) > len(t.Assigned) {
		back = coord.Working
	}
	if _, err := b.coord.SetTaskState(ctx, b.user.Key, id, back, ""); err != nil {
		return AnswerResult{}, err
	}
	d, err := b.Card(id)
	if err != nil {
		return AnswerResult{}, err
	}
	return AnswerResult{Card: d, Told: told}, nil
}
