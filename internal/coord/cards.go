package coord

// Cards (docs/design/v0.4-devices.md §9 and §11 Release 1, v0.3.3): what
// the cards window needs from a task beyond the Board. Steps give progress,
// blockers become line items with recommended answers (blocked stays a task
// state, decision 17), and agents can watch a card.
//
// CONTRACT (keep these signatures). Every change is a row folded like the
// rest of coord, so a replay rebuilds steps, blockers and watchers. Rows
// only gain fields:
//   - task.status with state blocked gains {blocker: {id, question, context,
//     options}}: one row enters Blocked and adds the item. An older row with
//     only a reason folds to an item whose question is the reason.
//   - blocker.option, blocker.resolved, task.steps, task.step and
//     task.watched are new (kinds.go).
//
// Watch notices are not returned by the calls that cause them: coord queues
// what changed on a watched card (in memory, never on replay) and
// SweepNotices hands out one NoticeWatched digest per card and watcher, so
// a watcher hears at its next turn without a notice per tick. Routing
// (tools.RouteNotice) folds a digest into one still waiting, unread, for the
// same card (MergeWatch), so sweeps between turns add up to one message. A
// restart drops a digest that hadn't gone out yet.

import (
	"context"
	"time"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/hlc"
)

// Card limits.
const (
	MaxQuestion     = 200 // runes in a blocker's question
	MaxContext      = 800 // runes in a blocker's context, written for a newcomer
	MaxAnswer       = 200 // runes in a recommended answer
	MaxWhy          = 400 // runes in a recommendation's reason
	MaxOwnAnswer    = 800 // runes in an answer written in someone's own words
	MaxOptions      = 6   // recommended answers per blocker
	MaxOpenBlockers = 8   // open blocker items per task
	KeepResolved    = 10  // resolved blocker items a task keeps (oldest drop first)
	MaxSteps        = 20  // steps per task
	MaxStepText     = 200 // runes in a step
	MaxActivity     = 500 // activity lines a task keeps
	MaxMissing      = 6   // lines in a blocker reply's missing list
	MaxMissingText  = 200 // runes in one missing line
)

// Step is one ordered step of a task. At most one step is Doing (the
// current one); a Done step is never Doing. Ids are short ("s1", "s2", ...),
// minted per task and never reused.
type Step struct {
	ID    string `json:"id"`
	Text  string `json:"text"`
	Done  bool   `json:"done"`
	Doing bool   `json:"doing,omitempty"`
}

// Option is one answer to a blocker, from an agent involved (the raiser in
// Raise, or a holder or watcher through Recommend). Recommended marks the
// one its raiser recommends among the choices (at most one per Raise); an
// option added through Recommend is always recommended, and so is every
// option of a blocker.option row written before the flag existed.
type Option struct {
	Answer      string    `json:"answer"`
	Why         string    `json:"why,omitempty"`
	By          string    `json:"by"`
	At          time.Time `json:"at"`
	Recommended bool      `json:"recommended,omitempty"`
}

// Resolution is how a blocker item was resolved.
type Resolution struct {
	Answer string `json:"answer"`
	// Option is the index of the picked Option, or -1 for an answer in the
	// answerer's own words (and for a close).
	Option int       `json:"option"`
	By     string    `json:"by"`
	At     time.Time `json:"at"`
	// Closed: resolved without an answer to the question, by Unblock, by a
	// state change through SetTaskState, or by the sweeper when the lease or
	// task it waited on went away. Answer then holds the note, if any.
	Closed bool `json:"closed,omitempty"`
	// Note is the resolver's line beside the check once the person had
	// answered: what the agent will do now (Answer stays the person's).
	Note string `json:"note,omitempty"`
}

// BlockerStatus is where an open blocker item stands, derived in the fold
// from its blocker comments (never stored). Resolved and closed items keep
// their last status; Resolved says they are done.
type BlockerStatus string

const (
	BlockerOpen     BlockerStatus = "open"     // no answer yet (an agent's comment only adds context)
	BlockerAnswered BlockerStatus = "answered" // the person answered last: with the raiser
	BlockerReplied  BlockerStatus = "replied"  // an agent replied to the person's answer: with the person
)

// Blocker is one item of what blocks a task. Ids are short ("b1", "b2",
// ...), minted per task and never reused.
type Blocker struct {
	ID       string `json:"id"`
	Question string `json:"question"`
	// Context is written for someone who has never been there: enough to
	// decide quickly.
	Context  string      `json:"context,omitempty"`
	RaisedBy string      `json:"raisedBy"`
	At       time.Time   `json:"at"`
	Options  []Option    `json:"options,omitempty"`
	Resolved *Resolution `json:"resolved,omitempty"`
	// On is what this item waits on. Raise defaults it to a person ("" =
	// whoever is at the app): a question waits on a decision. Task.BlockedBy
	// is the first open item's On. An item on a lease or a task is closed by
	// the sweeper when that goes away; other items stay open.
	On *BlockedBy `json:"on,omitempty"`
	// Status, SentTo and Missing are computed from the item's blocker
	// comments, never written to a row. SentTo is who the person's latest
	// answer went to (the raiser then); Missing is what the agent's reply
	// to it says is still needed.
	Status  BlockerStatus `json:"status,omitempty"`
	SentTo  string        `json:"sentTo,omitempty"`
	Missing []string      `json:"missing,omitempty"`

	reason string // the row's reason: Task.Reason while this is the first open item
	legacy bool   // folded from a reason-only row: a later one replaces it
	// asked is the position of the row that replaced the question (zero
	// for one never replaced): blocker comments before it were about the
	// old question, so they no longer count.
	asked rowPos
}

// Open reports whether the item still waits on an answer.
func (b Blocker) Open() bool { return b.Resolved == nil }

// NewBlocker is Raise's input. Question is required (1..MaxQuestion runes);
// Context at most MaxContext; Options at most MaxOptions, each Answer
// 1..MaxAnswer and Why at most MaxWhy (their By is the raiser), at most one
// of them Recommended (else ErrInvalid). BlockedBy is
// the item's On, checked like Block's; nil means a person (whoever is at the
// app), so the card is in the person's Needs you.
type NewBlocker struct {
	Question  string     `json:"question"`
	Context   string     `json:"context"`
	Options   []Option   `json:"options"`
	BlockedBy *BlockedBy `json:"blockedBy,omitempty"`
}

// BlockerAnswer is Answer's input. Pick is the 1-based number of the
// recommended Option chosen, or 0 to answer with Text (1..MaxOwnAnswer
// runes). Wake is the answerer's choice for the raiser (decision 18).
// Resolve resolves the item; without it only a person may answer, and the
// answer goes to the raiser while the item stays open. Once the person has
// answered, Resolve keeps their answer and Text is the resolver's note: an
// agent must give one (one line, at most MaxAnswer runes) and can't pick.
type BlockerAnswer struct {
	Pick    int
	Text    string
	Wake    bool
	Resolve bool
}

// CommentInput is CommentOn's input. With BlockerID it is a reply on that
// open item (agents only): it asks back or, in Missing (at most MaxMissing
// lines of MaxMissingText runes), says what the person's answer still
// misses. The item stays open and goes back to the person.
type CommentInput struct {
	Text      string
	Wake      bool
	Files     []blob.FileRef
	BlockerID string
	Missing   []string
}

// StepUpdate is SetStep's input; nil leaves a field alone. Doing true makes
// this the current step (any other step stops being current); Done true
// also stops it being current.
type StepUpdate struct {
	Done  *bool `json:"done,omitempty"`
	Doing *bool `json:"doing,omitempty"`
}

// Activity is one line of what happened on a card: By (a key, "irori" for
// Irori itself) did What, a short phrase without the actor ("raised a
// blocker: …", "ticked “Tag v0.6.0”"). Kind is the row's kind and Device
// the device that wrote it. Lines are sorted by the row's position in the
// log, (HLC, Device, Seq), and the fold keeps the highest MaxActivity per
// task, so live and replay keep the same ones; replay rebuilds them. HLC
// and Seq never go on the wire (an HLC overflows a JS number).
type Activity struct {
	At        time.Time     `json:"at"`
	By        string        `json:"by"`
	What      string        `json:"what"`
	Kind      string        `json:"kind"`
	Device    string        `json:"device"`
	BlockerID string        `json:"blockerId,omitempty"` // the item a raise line raised, or a blocker comment is on
	HLC       hlc.Timestamp `json:"-"`
	Seq       int64         `json:"-"`
}

// More notice kinds.
const (
	NoticeAnswered NoticeKind = "answered" // your blocker was answered
	NoticeWatched  NoticeKind = "watched"  // a card you watch changed (SweepNotices digest)
)

// Progress is done steps over all steps (0, 0 when the task has none: no bar).
func (t Task) Progress() (done, total int) {
	for _, s := range t.Steps {
		if s.Done {
			done++
		}
	}
	return done, len(t.Steps)
}

// OpenBlockers lists the items still waiting, oldest first.
func (t Task) OpenBlockers() []Blocker {
	var out []Blocker
	for _, b := range t.Blockers {
		if b.Open() {
			out = append(out, b)
		}
	}
	return out
}

// Raise adds a blocker item to an open task and moves it to Blocked (the
// holder(s), the creator or a person may, like Block). An open item with
// the same question (and the same BlockedBy) makes it a no-op that returns
// that item. At most MaxOpenBlockers open items.
func (c *Coord) Raise(ctx context.Context, by, id string, nb NewBlocker) (Task, Blocker, error) {
	return c.impl.raise(ctx, by, id, nb)
}

// Recommend adds a recommended answer to an open blocker item. Any agent
// involved may: a holder, a watcher, the raiser, the creator, or a person.
// The same answer as an option not yet recommended marks that option
// Recommended (it keeps its By); as a recommended one, it is a no-op. At
// most MaxOptions per item.
func (c *Coord) Recommend(ctx context.Context, by, id, blockerID, answer, why string) (Task, error) {
	return c.impl.recommend(ctx, by, id, blockerID, answer, why)
}

// Answer answers or resolves one open blocker item. Without a.Resolve it
// is the person's answer (agents get ErrInvalid): a blocker comment sent to
// the raiser, and the item stays open (answered) until the raiser, a holder
// or the person resolves it. With a.Resolve anyone may resolve it (decision
// 19), by picking a recommended option or in their own words; once the
// person has answered, their answer stands and Text is the resolver's note
// (required of an agent). When no open item remains the task leaves Blocked
// like Unblock (Working when a holder has taken it up, else Submitted). The
// notice is NoticeAnswered to the raiser when that is an agent other than
// by, and to the holders other than by (with no holders, the creator), with
// Wake as the answerer chose. ErrNotFound for no such item; ErrBadState
// when it is already resolved.
func (c *Coord) Answer(ctx context.Context, by, id, blockerID string, a BlockerAnswer) (Task, Notice, error) {
	return c.impl.answer(ctx, by, id, blockerID, a)
}

// CommentOn is AddComment with more: without in.BlockerID it is exactly
// AddComment. With it, an agent replies on that open blocker item: no
// notice (the person is the audience, and the item is back in their Needs
// you); ErrBadState when the item is resolved, ErrInvalid for a person
// (they answer it instead).
func (c *Coord) CommentOn(ctx context.Context, by, taskID string, in CommentInput) (Comment, Notice, error) {
	return c.impl.commentOn(ctx, by, taskID, in)
}

// SetSteps replaces a task's step list (the holder(s), the creator or a
// person). A step keeps its id when the input gives it, or when its text
// matches an existing step; others get new ids. Texts are 1..MaxStepText
// runes, at most MaxSteps steps, at most one Doing, never Done and Doing. An
// unchanged list is a no-op.
func (c *Coord) SetSteps(ctx context.Context, by, id string, steps []Step) (Task, error) {
	return c.impl.setSteps(ctx, by, id, steps)
}

// SetStep ticks, unticks or marks the current step (same permission as
// SetSteps). ErrNotFound for no such step; a change to nothing is a no-op.
func (c *Coord) SetStep(ctx context.Context, by, id, stepID string, u StepUpdate) (Task, error) {
	return c.impl.setStep(ctx, by, id, stepID, u)
}

// Watch makes agent by a watcher of an open task: SweepNotices tells it,
// at its next turn, when the card's state, steps, blockers or PR links
// change. People see every card and can't watch (ErrInvalid). Watching
// twice is a no-op.
func (c *Coord) Watch(ctx context.Context, by, id string) (Task, error) {
	return c.impl.watch(ctx, by, id, true)
}

// Unwatch stops by watching a task (any state). Not watching is a no-op.
func (c *Coord) Unwatch(ctx context.Context, by, id string) (Task, error) {
	return c.impl.watch(ctx, by, id, false)
}

// Activity lists a task's newest activity lines, newest first (nil for no
// such task).
func (c *Coord) Activity(id string) []Activity { return c.impl.activityOf(id) }

// PRLink reads a GitHub pull request out of a link: its "owner/name" and
// number. ok is false for anything else.
func PRLink(l Link) (repo string, number int, ok bool) { return prLink(l) }
