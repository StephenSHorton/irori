package coord

// Asks (docs/design/board.md "Asks"): the person talking to the room. An
// ask is a card whose task.created carries an ask field, written by the
// person. Every free agent hears it, exactly one takes it, they talk it
// through on the card's thread, and it ends as a card (in place), a change
// to another card, or nothing.
//
// The take lock follows the owner rule (v0.4 §6.0): the device that wrote
// the ask's task.created owns its take. Anyone writes take.requested; only
// the owner writes take.state, the current taker as a snapshot, and the
// fold keeps the owner's newest. A claim needs the owner reachable; with
// it offline the ask waits, and nothing about it moves on its own.

import (
	"context"
	"encoding/json"
	"fmt"
	"slices"
	"sort"
	"strings"
	"sync"
	"time"
	"unicode"
	"unicode/utf8"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/owner"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// Ask kinds. The ask.* names belong to approvals (internal/ask), so the
// take lock's rows are take.*.
const (
	// KindTakeRequested is a request on an ask's take, written by any
	// device: {taskId, by, op: claim|release|reopen, at}. by is an agent of
	// the writing device (claim, release) or the person's key (reopen).
	KindTakeRequested = "take.requested"
	// KindTakeState is the ask's current taker as a snapshot, written by
	// the ask's owner device only: {taskId, taker, reason, req?, told,
	// heard?, since, unclaimed?, seen?, at}. taker "" means open.
	KindTakeState = "take.state"
	// KindTaskEdited changes a card's text: {taskId, title?, objective?,
	// doneCriteria?, by, from?, note?, at}. from is the ask an edit outcome
	// came from, note its one line.
	KindTaskEdited = "task.edited"
)

// Take request ops.
const (
	TakeClaim   = "claim"
	TakeRelease = "release"
	TakeReopen  = "reopen"
)

// Take state reasons.
const (
	TakePosted    = "posted"
	TakeTaken     = "taken"
	TakeReleased  = "released"
	TakeGone      = "gone"
	TakeReopened  = "reopened"
	TakeUnclaimed = "unclaimed"
)

// Outcome kinds (ask_close).
const (
	OutcomeCard = "card" // the ask became the card, in place
	OutcomeEdit = "edit" // it changed another card
	OutcomeNone = "none" // nothing came of it
)

// Ask limits and timing.
const (
	// AskUnclaimed is how long an ask may go untaken before the person
	// hears: the "few minutes" the sweep already uses.
	AskUnclaimed = GoneGrace
	// AskWait is how long ask_claim waits for the owner's answer.
	AskWait     = 5 * time.Second
	maxAskTitle = 80
	// maxAskNotice is how much of the prompt a notice carries.
	maxAskNotice = 600
	MaxOutcome   = 300 // runes in an outcome's note
)

// Notice kinds of asks.
const (
	NoticeAsk   NoticeKind = "ask"   // the person asks the room; Notice.Person
	NoticeTaken NoticeKind = "taken" // the answer to a claim that couldn't wait for it
	// NoticeAskComment is the person's comment to the room on an ask nobody
	// took yet (Ref task:<id>, like NoticeAsk): stale once it is taken.
	NoticeAskComment NoticeKind = "ask_comment"
)

// Outcome is how an ask ended (task.status's outcome).
type Outcome struct {
	Kind string `json:"kind"`           // OutcomeCard, OutcomeEdit or OutcomeNone
	Task string `json:"task,omitempty"` // edit: the card it changed
	Note string `json:"note"`
}

// Ask is an ask's state on its card (Task.Ask), folded from its rows.
type Ask struct {
	Wake    bool   `json:"wake,omitempty"`
	RepoKey string `json:"repoKey,omitempty"`
	// Owner is the device that hands out the take: the one the person
	// posted from.
	Owner string `json:"owner"`
	// Taker is who holds the ask ("" while open); Reason is the owner's
	// latest take.state reason ("" before its first).
	Taker  string `json:"taker,omitempty"`
	Reason string `json:"reason,omitempty"`
	// Told is how many agents heard it; Heard are they (agent keys).
	Told  int      `json:"told"`
	Heard []string `json:"heard,omitempty"`
	// Since is when the ask last opened (posted, reopened, given back or
	// its taker gone): the unclaimed clock's start.
	Since time.Time `json:"since"`
	// Unclaimed: nobody took it within AskUnclaimed (or nobody was free
	// to hear it). A claim is still granted.
	Unclaimed bool `json:"unclaimed,omitempty"`
	// Waiting are agents on this device that asked to claim it and haven't
	// heard back from the owner device.
	Waiting []string `json:"waiting,omitempty"`
	// Outcome is set once the ask closed; it is a card from then on.
	Outcome *Outcome `json:"outcome,omitempty"`
}

// Open reports whether the ask still runs under the take lock.
func (a *Ask) Open() bool { return a != nil && a.Outcome == nil }

// askField is task.created's ask.
type askField struct {
	Wake    bool   `json:"wake,omitempty"`
	RepoKey string `json:"repoKey,omitempty"`
}

type takeRequestedRow struct {
	TaskID string    `json:"taskId"`
	By     string    `json:"by"`
	Op     string    `json:"op"`
	At     time.Time `json:"at"`
}

// takeReqRef names one take.requested row.
type takeReqRef struct {
	Device string `json:"device"`
	Seq    int64  `json:"seq"`
}

type takeStateRow struct {
	TaskID    string      `json:"taskId"`
	Taker     string      `json:"taker"`
	Reason    string      `json:"reason"`
	Req       *takeReqRef `json:"req,omitempty"`
	Told      int         `json:"told"`
	Heard     []string    `json:"heard,omitempty"`
	Since     time.Time   `json:"since"`
	Unclaimed bool        `json:"unclaimed,omitempty"`
	// Seen is, per device, the highest take.requested seq on this ask the
	// owner had acted on when it wrote this row: a request at or below it
	// is answered, so a restart never grants an old claim.
	Seen map[string]int64 `json:"seen,omitempty"`
	// Was is the taker a released or gone row ended.
	Was string    `json:"was,omitempty"`
	At  time.Time `json:"at"`
}

type taskEditedRow struct {
	TaskID       string    `json:"taskId"`
	Title        *string   `json:"title,omitempty"`
	Objective    *string   `json:"objective,omitempty"`
	DoneCriteria *string   `json:"doneCriteria,omitempty"`
	By           string    `json:"by"`
	From         string    `json:"from,omitempty"`
	Note         string    `json:"note,omitempty"`
	At           time.Time `json:"at"`
}

// takeReq is one take.requested row as folded.
type takeReq struct {
	takeReqRef
	By, Op string
	// done: answered (covered by the owner's seen, or acted on here).
	// waiting: an ask_claim on this device still waits for the answer, so
	// it reports it and no notice goes.
	done, waiting bool
}

// askBook is the take lock's fold beside the tasks.
type askBook struct {
	reqs     map[string][]*takeReq       // task id -> requests, fold order
	stateSeq map[string]int64            // task id -> seq of the owner's take.state folded
	seen     map[string]map[string]int64 // task id -> what the owner acted on (owner's own memory)
	notices  []Notice                    // fold-made notices for the sweep (never on replay)
	again    map[string]askAgain         // task id -> told again at the next sweep (owner)

	// The lock decides by position, not by arrival (askGate): marks are the
	// owner's take.states folded, deferred the rows it couldn't decide yet,
	// tried again (retry) once the ask's next take.state or close folds.
	marks    map[string][]takeMark    // ask id -> the owner's take.states, any order
	deferred map[string][]store.Event // ask id -> rows waiting on its lock, fold order
	retry    []string                 // ask ids whose deferred rows to fold again
	closedAt map[string]store.Cursor  // ask id -> the row that closed it

	// stale is read by the inbox's Stale hook, which runs under the inbox's
	// lock: it has its own lock so it never waits on coord's.
	staleMu sync.RWMutex
	stale   map[string]string // task id -> taker (or "" with closed) of an ask no longer open to all
	closed  map[string]bool
}

// askAgain is an ask opened again, for the sweep to tell the room: skip
// is the agent that left it, reason the take.state reason to write.
type askAgain struct {
	skip, why, reason string
	wake              bool
}

// takeMark is one owner take.state: where it sits and who it named.
type takeMark struct {
	at    store.Cursor
	taker string
}

func newAskBook() askBook {
	return askBook{reqs: map[string][]*takeReq{}, stateSeq: map[string]int64{}, seen: map[string]map[string]int64{},
		again: map[string]askAgain{}, stale: map[string]string{}, closed: map[string]bool{},
		marks: map[string][]takeMark{}, deferred: map[string][]store.Event{}, closedAt: map[string]store.Cursor{}}
}

// reset forgets what the fold built (a revoke's cut re-folds); stale
// entries are rebuilt by the re-fold.
func (b *askBook) reset() {
	b.reqs, b.stateSeq, b.seen, b.notices = map[string][]*takeReq{}, map[string]int64{}, map[string]map[string]int64{}, nil
	b.again = map[string]askAgain{}
	b.marks, b.deferred, b.retry, b.closedAt = map[string][]takeMark{}, map[string][]store.Event{}, nil, map[string]store.Cursor{}
	b.staleMu.Lock()
	b.stale, b.closed = map[string]string{}, map[string]bool{}
	b.staleMu.Unlock()
}

func (b *askBook) setStale(id, taker string, closed bool) {
	b.staleMu.Lock()
	defer b.staleMu.Unlock()
	switch {
	case closed:
		b.closed[id] = true
		delete(b.stale, id)
	case taker != "":
		b.stale[id] = taker
	default:
		delete(b.stale, id)
	}
}

// ---- posting ----

// NewAsk is PostAsk's input: what the person typed, files on it, the repo
// they picked ("" any) and whether to wake the agents.
type NewAsk struct {
	Text    string
	Files   []blob.FileRef
	RepoKey string
	Wake    bool
}

// AskTitle is a prompt's title: its first line, cut at a word to 80
// characters.
func AskTitle(text string) string {
	line, _, _ := strings.Cut(strings.TrimSpace(text), "\n")
	line = strings.Join(strings.Fields(line), " ")
	if utf8.RuneCountInString(line) <= maxAskTitle {
		return line
	}
	r := []rune(line)[:maxAskTitle-1]
	if i := strings.LastIndexFunc(string(r), unicode.IsSpace); i > maxAskTitle/2 {
		return strings.TrimSpace(string(r)[:i]) + "…"
	}
	return string(r) + "…"
}

// PostAsk is the person asking the room: it makes the ask's card (owned by
// this device) and tells every free agent among agents (the mesh roster).
// The notice is NoticeAsk with Person set (route it from the person); with
// nobody free the ask is unclaimed at once and the notice has no To.
func (c *Coord) PostAsk(ctx context.Context, by string, in NewAsk, agents []presence.AgentCard) (Task, Notice, error) {
	return c.impl.postAsk(ctx, by, in, agents)
}

func (c *coord) postAsk(ctx context.Context, by string, in NewAsk, agents []presence.AgentCard) (Task, Notice, error) {
	by, text, err := checkComment(by, in.Text, in.Files)
	if err != nil {
		return Task{}, Notice{}, err
	}
	if !c.isOwner(by) {
		return Task{}, Notice{}, fmt.Errorf("%w: only the person asks the room", ErrNotAllowed)
	}
	repoKey := strings.TrimSpace(in.RepoKey)
	if repoKey != "" {
		if err := checkRepoKey(repoKey); err != nil {
			return Task{}, Notice{}, err
		}
	}
	c.mu.Lock()
	defer c.unlock()
	at := c.stamp()
	id := newID(at)
	if err := c.write(ctx, by, KindTaskCreated, taskCreatedRow{
		TaskID: id, Num: c.nextShort(), Title: AskTitle(text), Objective: text, RepoKey: repoKey,
		Ask: &askField{Wake: in.Wake, RepoKey: repoKey}, By: by, At: at,
	}); err != nil {
		return Task{}, Notice{}, err
	}
	if len(in.Files) > 0 {
		if _, err := c.writeComment(ctx, c.actorFor(by), taskCommentRow{
			TaskID: id, CommentID: newID(at), By: by, Text: "Files with the ask.", At: at, Attachments: in.Files,
		}); err != nil {
			return Task{}, Notice{}, err
		}
	}
	t := c.tasks[id]
	free := c.freeAgents(t, agents, nil)
	if err := c.writeTake(ctx, t, takeStateRow{Reason: TakePosted, Told: len(free), Heard: free, Since: at, Unclaimed: len(free) == 0}); err != nil {
		return Task{}, Notice{}, err
	}
	n := c.askNotice(t, free, in.Wake, "")
	n.Attachments = in.Files
	return copyTask(t), n, nil
}

// askNotice tells to about ask t, from the person. why, when set, says
// why it is told again.
func (c *coord) askNotice(t *Task, to []string, wake bool, why string) Notice {
	ref := t.ShortRef()
	var b strings.Builder
	fmt.Fprintf(&b, "The person asks the room (%s)", t.Refs())
	if why != "" {
		b.WriteString(", again: " + why)
	}
	fmt.Fprintf(&b, ":\n%s\n", cut(t.Objective, maxAskNotice))
	fmt.Fprintf(&b, "If you can take it on now, ask_claim %s: one agent takes it, and you hear if someone else got there first. If you're busy, leave it.", ref)
	b.WriteString(deeper(t.ID))
	return Notice{Kind: NoticeAsk, TaskID: t.ID, Ref: "task:" + t.ID, Title: t.Title, By: t.CreatedBy,
		To: slices.Clone(to), Wake: wake, Text: b.String(), Person: true}
}

// freeAgents are the agents among roster that may take ask t now, sorted:
// in the roster, idle, on a device that is neither offline nor removed,
// in the ask's repo when it names one, and not in skip. The caller holds
// c.mu.
func (c *coord) freeAgents(t *Task, roster []presence.AgentCard, skip []string) []string {
	var out []string
	for _, a := range roster {
		dev := cmpOrDev(a.DeviceID, owner.DeviceOf(a.Key))
		switch {
		case a.Key == "" || a.Key == sweeper || c.isOwner(a.Key) || a.Status != "idle":
		case a.DeviceState == "offline":
		case c.revoked != nil && dev != "" && c.revoked(dev):
		case t.Ask != nil && t.Ask.RepoKey != "" && a.RepoKey != t.Ask.RepoKey:
		case slices.Contains(skip, a.Key) || slices.Contains(out, a.Key):
		default:
			out = append(out, a.Key)
		}
	}
	sort.Strings(out)
	return out
}

func cmpOrDev(a, b string) string {
	if a != "" {
		return a
	}
	return b
}

// writeTake writes the owner's take.state for t (this device owns it),
// with what it has acted on. The caller holds c.mu.
func (c *coord) writeTake(ctx context.Context, t *Task, row takeStateRow) error {
	row.TaskID, row.At = t.ID, c.stamp()
	if row.Since.IsZero() && t.Ask != nil {
		row.Since = t.Ask.Since
	}
	if s := c.asks.seen[t.ID]; len(s) > 0 {
		row.Seen = make(map[string]int64, len(s))
		for d, n := range s {
			row.Seen[d] = n
		}
	}
	return c.writeAs(ctx, sweeper, KindTakeState, row)
}

// ---- the fold ----

// foldAskCreated sets up t's ask from its task.created row: only the
// person's row makes an ask (else it is a plain card). c.mu is held.
func (c *coord) foldAskCreated(t *Task, a *askField, ev store.Event, by string) {
	if a == nil || !c.personRow(ev, by) {
		return
	}
	t.Ask = &Ask{Wake: a.Wake, RepoKey: a.RepoKey, Owner: ev.DeviceID, Since: t.CreatedAt}
}

// openAsk is t's ask while the take lock applies.
func openAsk(t *Task) *Ask {
	if t == nil || !t.Ask.Open() {
		return nil
	}
	return t.Ask
}

func (c *coord) applyTake(ev store.Event, at func(time.Time) time.Time) {
	switch ev.Kind {
	case KindTakeRequested:
		var p takeRequestedRow
		if json.Unmarshal(ev.Payload, &p) != nil || p.TaskID == "" || p.By == "" {
			return
		}
		if !owner.MayWrite(p.By, ev.DeviceID) || owner.DeviceOf(p.By) == "" {
			c.ignore(ev.DeviceID)
			return
		}
		switch p.Op {
		case TakeClaim, TakeRelease:
		case TakeReopen:
			if !c.personRow(ev, p.By) {
				c.ignore(ev.DeviceID)
				return
			}
		default:
			return
		}
		r := &takeReq{takeReqRef: takeReqRef{Device: ev.DeviceID, Seq: ev.Seq}, By: p.By, Op: p.Op}
		for _, o := range c.asks.reqs[p.TaskID] {
			if o.takeReqRef == r.takeReqRef {
				return
			}
		}
		t := c.tasks[p.TaskID]
		// A request the owner's latest state already covers is answered.
		r.done = c.covered(p.TaskID, r.takeReqRef)
		c.asks.reqs[p.TaskID] = append(c.asks.reqs[p.TaskID], r)
		if t != nil {
			if p.Op == TakeClaim {
				c.touch(ev, t, at(p.At), p.By, "asked to take it", false)
			}
			c.syncWaiting(t)
			c.note(Change{Kind: TaskChanged, TaskID: t.ID})
		}
	case KindTakeState:
		var p takeStateRow
		if json.Unmarshal(ev.Payload, &p) != nil || p.TaskID == "" {
			return
		}
		t := c.tasks[p.TaskID]
		if t == nil || t.Ask == nil {
			return
		}
		if ev.DeviceID != t.Ask.Owner {
			c.ignore(ev.DeviceID) // only the posting device hands it out
			return
		}
		// Marked even once closed: a row the taker wrote before the close
		// is decided by the take it saw, whatever arrived first here.
		c.asks.mark(t.ID, ev.Cursor(), p.Taker)
		a := t.Ask
		if a.Outcome != nil || ev.Seq <= c.asks.stateSeq[t.ID] {
			return
		}
		c.asks.stateSeq[t.ID] = ev.Seq
		c.asks.lastSeen(t.ID, p.Seen)
		was := a.Taker
		a.Taker, a.Reason, a.Told, a.Since, a.Unclaimed = p.Taker, p.Reason, p.Told, at(p.Since), p.Unclaimed
		a.Heard = slices.Clone(p.Heard)
		if p.Taker != "" {
			a.Unclaimed = false
			t.Holders = []string{p.Taker}
		} else {
			t.Holders = nil
		}
		t.Assigned = nil
		syncHolder(t)
		when := at(p.At)
		t.UpdatedAt = when
		c.answerRequests(t, p)
		c.asks.setStale(t.ID, a.Taker, false)
		switch {
		case p.Taker != "" && p.Taker != was:
			t.UpdatedBy = p.Taker
			c.touch(ev, t, when, p.Taker, "took it", true)
		case p.Reason == TakeReleased:
			c.touch(ev, t, when, cmpOrDev(p.Was, was), "gave it back", true)
		case p.Reason == TakeGone:
			c.touch(ev, t, when, sweeper, "opened it again: "+cmpOrDev(p.Was, was)+" went away", true)
		case p.Reason == TakeReopened:
			c.touch(ev, t, when, t.CreatedBy, "asked the room again", true)
		case p.Reason == TakeUnclaimed && p.Taker == "" && was == "":
			c.touch(ev, t, when, sweeper, "nobody took it yet", false)
		}
		c.note(Change{Kind: TaskChanged, TaskID: t.ID, Holder: t.Holder})
	case KindTaskEdited:
		// askGate has checked who wrote it.
		var p taskEditedRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		t := c.tasks[p.TaskID]
		if t == nil {
			return
		}
		if p.Title != nil && strings.TrimSpace(*p.Title) != "" {
			t.Title = *p.Title
		}
		if p.Objective != nil {
			t.Objective = *p.Objective
		}
		if p.DoneCriteria != nil {
			t.DoneCriteria = *p.DoneCriteria
		}
		when := at(p.At)
		t.UpdatedAt, t.UpdatedBy = when, p.By
		what := "edited the card"
		if p.From != "" {
			what = "changed it from the ask " + c.shortOf(p.From)
			if p.Note != "" {
				what += ": " + p.Note
			}
		}
		c.touch(ev, t, when, p.By, what, true)
		c.note(Change{Kind: TaskChanged, TaskID: t.ID, Holder: t.Holder})
	}
}

// shortOf is a card's short ref by id, or task:<id>. c.mu is held.
func (c *coord) shortOf(id string) string {
	if t := c.tasks[id]; t != nil {
		return t.ShortRef()
	}
	return "task:" + id
}

// lastSeen records the owner's seen map from its newest state.
func (b *askBook) lastSeen(id string, seen map[string]int64) {
	m := map[string]int64{}
	for d, n := range b.seen[id] {
		m[d] = n
	}
	for d, n := range seen {
		m[d] = max(m[d], n)
	}
	b.seen[id] = m
}

// covered reports whether the owner's newest state answered request r.
func (c *coord) covered(id string, r takeReqRef) bool {
	return c.asks.seen[id][r.Device] >= r.Seq
}

// answerRequests marks the requests a take.state answers, and queues the
// answer for this device's agents whose ask_claim stopped waiting: a claim
// is answered by any taker; a release by the taker it named leaving. The
// caller holds c.mu.
func (c *coord) answerRequests(t *Task, p takeStateRow) {
	for _, r := range c.asks.reqs[t.ID] {
		if r.done {
			continue
		}
		answered := c.covered(t.ID, r.takeReqRef) || (p.Req != nil && *p.Req == r.takeReqRef)
		switch r.Op {
		case TakeClaim:
			answered = answered || p.Taker != ""
		case TakeRelease:
			answered = answered || p.Taker != r.By
		}
		if !answered {
			continue
		}
		r.done = true
		if r.Op != TakeClaim || r.waiting || c.replaying || r.Device != c.st.DeviceID() || p.Taker == "" {
			continue
		}
		c.asks.notices = append(c.asks.notices, c.takenNotice(t, r.By, p.Taker))
	}
	c.syncWaiting(t)
}

// takenNotice tells claimer who got ask t.
func (c *coord) takenNotice(t *Task, claimer, taker string) Notice {
	n := Notice{Kind: NoticeTaken, TaskID: t.ID, Ref: "task:" + t.ID, Title: t.Title, By: sweeper, To: []string{claimer}}
	if taker == claimer {
		n.Text = fmt.Sprintf("You have the ask %q (%s). %s%s", t.Title, t.Refs(), youHaveIt, deeper(t.ID))
	} else {
		n.Text = fmt.Sprintf("%s took the ask %q (%s) first; nothing to do unless you want to chime in (board_comment).%s",
			taker, t.Title, t.Refs(), deeper(t.ID))
	}
	return n
}

const youHaveIt = "Talk it through with the person on the card first if it isn't clear what they want, then ask_close it with what it became: a card, a change to another card, or nothing."

// syncWaiting refreshes Ask.Waiting: this device's agents with an
// unanswered claim. The caller holds c.mu.
func (c *coord) syncWaiting(t *Task) {
	if t == nil || t.Ask == nil {
		return
	}
	var w []string
	for _, r := range c.asks.reqs[t.ID] {
		if !r.done && r.Op == TakeClaim && r.Device == c.st.DeviceID() && !slices.Contains(w, r.By) {
			w = append(w, r.By)
		}
	}
	t.Ask.Waiting = w
}

// closeAskFold ends t's ask once a task.status with an outcome (or the
// person's terminal state) folds; at is that row. The caller holds c.mu.
func (c *coord) closeAskFold(t *Task, o *Outcome, at store.Cursor) {
	t.Ask.Outcome = o
	t.Ask.Waiting = nil
	for _, r := range c.asks.reqs[t.ID] {
		r.done = true
	}
	c.asks.closedAt[t.ID] = at
	c.asks.wake(t.ID)
	c.asks.setStale(t.ID, "", true)
}

// mark records an owner take.state on ask id, and has the rows waiting on
// it tried again.
func (b *askBook) mark(id string, at store.Cursor, taker string) {
	if slices.ContainsFunc(b.marks[id], func(m takeMark) bool { return m.at == at }) {
		return
	}
	b.marks[id] = append(b.marks[id], takeMark{at: at, taker: taker})
	b.wake(id)
}

// wake queues ask id's deferred rows to fold again after the current row.
func (b *askBook) wake(id string) {
	if len(b.deferred[id]) > 0 && !slices.Contains(b.retry, id) {
		b.retry = append(b.retry, id)
	}
}

// deferRow keeps ev until ask id's lock can decide it.
func (b *askBook) deferRow(id string, ev store.Event) {
	cur := ev.Cursor()
	if slices.ContainsFunc(b.deferred[id], func(o store.Event) bool { return o.Cursor() == cur }) {
		return
	}
	b.deferred[id] = append(b.deferred[id], ev)
}

// takerAt is who held ask id where a row at sits: the taker of the owner's
// last take.state before it ("" for none). That is what replay, which folds
// in (hlc, device, seq) order, would hold there, whatever order the rows
// reached this device in.
func (c *coord) takerAt(id string, at store.Cursor) string {
	var best *takeMark
	for i, m := range c.asks.marks[id] {
		if cursorLess(m.at, at) && (best == nil || cursorLess(best.at, m.at)) {
			best = &c.asks.marks[id][i]
		}
	}
	if best == nil {
		return ""
	}
	return best.taker
}

// askGate is the take lock in the fold. It reports whether ev folds now;
// when not, wait names the ask whose next take.state or close may let it
// through ("" drops it). Order matters across devices (a third device can
// get the taker's close before the owner's take), so the lock reads a
// row's position against the owner's take.states, not the state folded so
// far, and a row it can't place yet waits instead of being lost.
//
// On an open ask it refuses task.claimed, task.joined and task.assigned
// (an older device's board_join can't take it around the lock), and a
// task.status counts only with an outcome from the person or from whoever
// held the take where the row sits, or as the person's close. Once the
// ask closed, those rows count again from the closing row on. A
// task.edited needs the person, or an ask's taker acting for that ask
// (on the ask itself, or with from), or on a plain card its creator or a
// holder. The caller holds c.mu.
func (c *coord) askGate(ev store.Event) (ok bool, wait string) {
	switch ev.Kind {
	case KindTaskEdited:
		return c.editGate(ev)
	case KindTaskClaimed, KindTaskJoined, KindTaskAssigned, KindTaskState:
	default:
		return true, ""
	}
	var p struct {
		TaskID  string    `json:"taskId"`
		State   TaskState `json:"state"`
		By      string    `json:"by"`
		Outcome *Outcome  `json:"outcome"`
	}
	if json.Unmarshal(ev.Payload, &p) != nil {
		return true, ""
	}
	t := c.tasks[p.TaskID]
	if t == nil || t.Ask == nil {
		return true, ""
	}
	cur := ev.Cursor()
	if !t.Ask.Open() {
		// Written before the close: the ask was open there, and replay
		// refuses it, so this device does too.
		return !cursorLess(cur, c.asks.closedAt[t.ID]), ""
	}
	person := c.personRow(ev, p.By)
	switch {
	case ev.Kind != KindTaskState:
	case p.Outcome != nil && (person || p.By != "" && c.takerAt(t.ID, cur) == p.By):
		return true, ""
	case p.Outcome == nil && person && IsTerminal(p.State):
		return true, ""
	}
	return false, t.ID
}

// editGate is askGate for task.edited.
func (c *coord) editGate(ev store.Event) (bool, string) {
	var p taskEditedRow
	if json.Unmarshal(ev.Payload, &p) != nil || p.TaskID == "" || p.By == "" {
		return false, ""
	}
	if c.personRow(ev, p.By) {
		return true, ""
	}
	if !owner.MayWrite(p.By, ev.DeviceID) {
		c.ignore(ev.DeviceID)
		return false, ""
	}
	ask := p.From
	switch {
	case ask == p.TaskID:
		return false, ""
	case ask == "":
		t := c.tasks[p.TaskID]
		if t == nil {
			return false, ""
		}
		if t.Ask == nil {
			return p.By == t.CreatedBy || isHolder(t, p.By), ""
		}
		ask = t.ID
	}
	if c.takerAt(ask, ev.Cursor()) == p.By {
		return true, ""
	}
	return false, ask
}

// foldDeferred folds again the rows waiting on asks whose lock moved. The
// caller holds c.mu.
func (c *coord) foldDeferred() {
	for len(c.asks.retry) > 0 {
		id := c.asks.retry[0]
		c.asks.retry = c.asks.retry[1:]
		rows := c.asks.deferred[id]
		delete(c.asks.deferred, id)
		for _, ev := range rows {
			c.fold(ev)
		}
	}
}

// ---- the owner's duties ----

// askDuties is what this device does as the owner of its open asks:
// answer requests in the order they folded. Run from Follow's tick and
// after every local request. The caller holds c.mu.
func (c *coord) askDuties(ctx context.Context) {
	if c.replaying {
		return
	}
	var ids []string
	for id, rs := range c.asks.reqs {
		if len(rs) > 0 {
			ids = append(ids, id)
		}
	}
	sort.Strings(ids)
	now := c.stamp()
	for _, id := range ids {
		t := c.tasks[id]
		a := openAsk(t)
		if a == nil || a.Owner != c.st.DeviceID() || IsTerminal(t.State) {
			continue
		}
		for _, r := range c.asks.reqs[id] {
			if r.done || c.covered(id, r.takeReqRef) {
				r.done = true
				continue
			}
			if err := c.ownerAct(ctx, t, r, now); err != nil {
				return // tried again on the next tick
			}
		}
	}
}

// ownerAct answers one request on t, which this device owns. The caller
// holds c.mu.
func (c *coord) ownerAct(ctx context.Context, t *Task, r *takeReq, now time.Time) error {
	a := t.Ask
	seen := c.asks.seen[t.ID]
	if seen == nil {
		seen = map[string]int64{}
		c.asks.seen[t.ID] = seen
	}
	mark := func() { seen[r.Device] = max(seen[r.Device], r.Seq) }
	switch r.Op {
	case TakeClaim:
		c.observe(now)
		if a.Taker != "" || c.gone(r.By, now) {
			// Taken already: nothing is written. The claimer's own device
			// tells it once it folds the taker.
			mark()
			r.done = true
			return nil
		}
		mark()
		ref := r.takeReqRef
		return c.writeTake(ctx, t, takeStateRow{Taker: r.By, Reason: TakeTaken, Req: &ref, Told: a.Told, Heard: a.Heard})
	case TakeRelease:
		mark()
		if a.Taker != r.By {
			r.done = true
			return nil
		}
		if err := c.writeTake(ctx, t, takeStateRow{Reason: TakeReleased, Was: r.By, Told: a.Told, Heard: a.Heard, Since: now}); err != nil {
			return err
		}
		c.asks.again[t.ID] = askAgain{skip: r.By, why: "the agent that took it gave it back", reason: TakeReleased}
		return nil
	case TakeReopen:
		mark()
		if a.Taker != "" {
			r.done = true
			return nil
		}
		c.asks.again[t.ID] = askAgain{wake: true, why: "the person wants it picked up", reason: TakeReopened}
		r.done = true
		return nil
	}
	return nil
}

// SweepAsks does the timed part of the asks with the mesh roster (Core
// calls it with SweepNotices): as the owner, it opens an ask again whose
// taker went away, tells agents that became free while it is open, marks
// it unclaimed after AskUnclaimed (at once when nobody heard), and
// answers a reopen. On every device it hands out the answers to claims
// that stopped waiting. Notices with Person set go from the person, the
// rest from Irori.
func (c *Coord) SweepAsks(ctx context.Context, agents []presence.AgentCard) ([]Notice, error) {
	return c.impl.sweepAsks(ctx, agents)
}

func (c *coord) sweepAsks(ctx context.Context, agents []presence.AgentCard) ([]Notice, error) {
	c.mu.Lock()
	defer c.unlock()
	c.askDuties(ctx)
	now := c.stamp()
	c.observe(now)
	out := c.asks.notices
	c.asks.notices = nil
	again := c.asks.again
	c.asks.again = map[string]askAgain{}
	// A write that fails keeps what wasn't told yet for the next sweep.
	fail := func(err error) ([]Notice, error) {
		for id, ag := range again {
			if _, ok := c.asks.again[id]; !ok {
				c.asks.again[id] = ag
			}
		}
		return out, err
	}
	inRoster := map[string]presence.AgentCard{}
	for _, a := range agents {
		inRoster[a.Key] = a
	}
	var ids []string
	for id, t := range c.tasks {
		if a := openAsk(t); a != nil && a.Owner == c.st.DeviceID() && !IsTerminal(t.State) {
			ids = append(ids, id)
		}
	}
	sort.Strings(ids)
	for _, id := range ids {
		t := c.tasks[id]
		a := t.Ask
		if a.Taker != "" {
			if !c.takerGone(a.Taker, inRoster, now) {
				continue
			}
			was := a.Taker
			if err := c.writeTake(ctx, t, takeStateRow{Reason: TakeGone, Was: was, Since: now}); err != nil {
				return fail(err)
			}
			again[id] = askAgain{skip: was, why: "the agent that took it went away", reason: TakeGone}
		}
		if ag, ok := again[id]; ok {
			// Opened again (given back, gone, or the person's Wake them):
			// every free agent hears it again, the one that left aside.
			if a.Taker != "" {
				continue // taken again meanwhile
			}
			free := c.freeAgents(t, agents, []string{ag.skip})
			if err := c.writeTake(ctx, t, takeStateRow{Reason: ag.reason, Told: len(free), Heard: free, Since: now, Unclaimed: len(free) == 0}); err != nil {
				return fail(err)
			}
			delete(again, id)
			if len(free) > 0 {
				out = append(out, c.askNotice(t, free, ag.wake, ag.why))
			}
			continue
		}
		// Open: tell who became free, then the unclaimed clock. A late tell
		// wakes only inside the person's own window: AskUnclaimed from a
		// post with Wake, or from Wake them. Later, or after a taker left,
		// it waits for the agent's next turn, so an old ask never wakes a
		// session that only now went idle.
		free := c.freeAgents(t, agents, a.Heard)
		unclaimed := a.Unclaimed || a.Told+len(free) == 0 || now.Sub(a.Since) >= AskUnclaimed
		if len(free) == 0 && unclaimed == a.Unclaimed {
			continue
		}
		heard := append(slices.Clone(a.Heard), free...)
		reason := a.Reason
		if unclaimed && !a.Unclaimed {
			reason = TakeUnclaimed
		}
		if err := c.writeTake(ctx, t, takeStateRow{Reason: reason, Told: len(heard), Heard: heard, Unclaimed: unclaimed}); err != nil {
			return fail(err)
		}
		if len(free) > 0 {
			wake := now.Sub(a.Since) < AskUnclaimed && (a.Reason == TakeReopened || a.Reason == TakePosted && a.Wake)
			out = append(out, c.askNotice(t, free, wake, ""))
		}
	}
	// An ask whose posting device was removed can't be handed out: the
	// person's close, from this device.
	for _, t := range c.tasks {
		if a := openAsk(t); a != nil && c.revoked != nil && c.revoked(a.Owner) && !IsTerminal(t.State) {
			if self := c.selfKey(); c.isOwner(self) {
				_ = c.write(ctx, self, KindTaskState, taskStateRow{TaskID: t.ID, State: Canceled, Note: "posting device removed",
					Outcome: &Outcome{Kind: OutcomeNone, Note: "posting device removed"}, By: self, At: now})
			}
		}
	}
	return out, nil
}

// takerGone reports whether an ask's taker went away: on this device, out
// of the roster for GoneGrace; on another, its device retired it (it wrote
// agent.gone, or started again). A taker on a device that is merely
// offline keeps it. The caller holds c.mu.
func (c *coord) takerGone(taker string, roster map[string]presence.AgentCard, now time.Time) bool {
	if c.isOwner(taker) {
		return false
	}
	if c.localKey(taker) {
		return c.gone(taker, now)
	}
	a, ok := roster[taker]
	return !ok || a.Status == "gone"
}

// ---- claim, close ----

// ClaimResult is ask_claim's answer.
type ClaimResult struct {
	Task Task
	// Taker is who holds it now ("" when the owner hasn't answered).
	Taker string
	// Mine: the caller has it. Pending: the request is written and the
	// owner device hasn't answered (the caller hears at its next turn).
	Mine, Pending bool
	// Owner is the device that hands it out.
	Owner string
}

// AskClaim asks for an ask's take as agent by and waits up to wait for the
// owner device's answer (on this device the answer is immediate). An ask
// already taken is refused with ErrHeld, naming the taker; nothing is
// written then.
func (c *Coord) AskClaim(ctx context.Context, by, id string, wait time.Duration) (ClaimResult, error) {
	return c.impl.askClaim(ctx, by, id, wait)
}

func (c *coord) askClaim(ctx context.Context, by, id string, wait time.Duration) (ClaimResult, error) {
	by, _, err := callers(by)
	if err != nil {
		return ClaimResult{}, err
	}
	if c.isOwner(by) || by == sweeper {
		return ClaimResult{}, fmt.Errorf("%w: an agent takes an ask", ErrInvalid)
	}
	c.mu.Lock()
	t, err := c.openTask(id)
	if err == nil && openAsk(t) == nil {
		err = fmt.Errorf("%w: %s isn't an open ask", ErrBadState, t.ShortRef())
	}
	if err != nil {
		c.unlock()
		return ClaimResult{}, err
	}
	a := t.Ask
	if a.Taker == by {
		res := ClaimResult{Task: copyTask(t), Taker: by, Mine: true, Owner: a.Owner}
		c.unlock()
		return res, nil
	}
	if a.Taker != "" {
		c.unlock()
		return ClaimResult{}, fmt.Errorf("%w: %s took %s. To add something, board_comment it", ErrHeld, a.Taker, t.ShortRef())
	}
	if err := c.write(ctx, by, KindTakeRequested, takeRequestedRow{TaskID: id, By: by, Op: TakeClaim, At: c.stamp()}); err != nil {
		c.unlock()
		return ClaimResult{}, err
	}
	rs := c.asks.reqs[id]
	req := rs[len(rs)-1]
	req.waiting = true
	c.askDuties(ctx) // the owner here answers at once
	c.unlock()

	sub, stop := c.subscribeLocal()
	defer stop()
	deadline := time.NewTimer(wait)
	defer deadline.Stop()
	for {
		c.mu.Lock()
		t := c.tasks[id]
		if t.Ask.Outcome != nil {
			req.waiting = false
			c.unlock()
			return ClaimResult{}, fmt.Errorf("%w: %s closed before the owner answered", ErrBadState, t.ShortRef())
		}
		res := ClaimResult{Task: copyTask(t), Owner: t.Ask.Owner, Taker: t.Ask.Taker, Mine: t.Ask.Taker == by, Pending: t.Ask.Taker == ""}
		if !res.Pending {
			req.waiting = false
			c.unlock()
			return res, nil
		}
		c.unlock()
		select {
		case <-sub:
			continue
		case <-deadline.C:
		case <-ctx.Done():
		}
		c.mu.Lock()
		req.waiting = false
		t = c.tasks[id]
		res = ClaimResult{Task: copyTask(t), Owner: t.Ask.Owner, Taker: t.Ask.Taker, Mine: t.Ask.Taker == by, Pending: t.Ask.Taker == ""}
		c.unlock()
		return res, nil
	}
}

// subscribeLocal is a change signal for askClaim's wait.
func (c *coord) subscribeLocal() (<-chan Change, func()) {
	ctx, cancel := context.WithCancel(context.Background())
	return c.subscribe(ctx), cancel
}

// AskOutcome is ask_close's input: Kind and Note always; for a card its
// Title (required), Objective, DoneWhen and Steps; for an edit the card it
// changes (Task) and any of Title, Objective and DoneWhen for it.
type AskOutcome struct {
	Kind                       string
	Note                       string
	Title, Objective, DoneWhen string
	Steps                      []Step
	Task                       string
}

// AskClose ends an open ask with its outcome; only its taker or the person
// may. card: the ask becomes the card in place, held by the taker. edit:
// the other card changes and the ask completes. none: it is canceled.
func (c *Coord) AskClose(ctx context.Context, by, id string, o AskOutcome) (Task, error) {
	return c.impl.askClose(ctx, by, id, o)
}

func (c *coord) askClose(ctx context.Context, by, id string, o AskOutcome) (Task, error) {
	by, _, err := callers(by)
	if err != nil {
		return Task{}, err
	}
	o.Note = strings.Join(strings.Fields(o.Note), " ")
	if o.Note == "" && c.isOwner(by) {
		o.Note = "closed by the person"
	}
	if err := checkText("the note", o.Note, 1, MaxOutcome); err != nil {
		return Task{}, err
	}
	title, obj, done := strings.TrimSpace(o.Title), strings.TrimSpace(o.Objective), strings.TrimSpace(o.DoneWhen)
	if utf8.RuneCountInString(title) > maxTitle {
		return Task{}, fmt.Errorf("%w: title longer than %d characters", ErrInvalid, maxTitle)
	}
	if utf8.RuneCountInString(obj) > maxObjective || utf8.RuneCountInString(done) > maxObjective {
		return Task{}, fmt.Errorf("%w: objective or done criteria longer than %d characters", ErrInvalid, maxObjective)
	}
	steps := append([]Step(nil), o.Steps...)
	if err := checkSteps(steps); err != nil {
		return Task{}, err
	}
	c.mu.Lock()
	defer c.unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, err
	}
	a := openAsk(t)
	if a == nil {
		return Task{}, fmt.Errorf("%w: %s isn't an open ask", ErrBadState, t.ShortRef())
	}
	person := c.isOwner(by)
	if !person && (a.Taker == "" || a.Taker != by) {
		if a.Taker == "" {
			return Task{}, fmt.Errorf("%w: ask_claim %s first", ErrNotAllowed, t.ShortRef())
		}
		return Task{}, fmt.Errorf("%w: %s holds %s; only it or the person closes it", ErrNotAllowed, a.Taker, t.ShortRef())
	}
	now := c.stamp()
	str := func(s string) *string { return &s }
	switch o.Kind {
	case OutcomeCard:
		if title == "" {
			return Task{}, fmt.Errorf("%w: a card outcome needs a title", ErrInvalid)
		}
		row := taskEditedRow{TaskID: id, Title: str(title), By: by, At: now}
		if obj != "" {
			row.Objective = str(obj)
		}
		if done != "" {
			row.DoneCriteria = str(done)
		}
		if err := c.write(ctx, by, KindTaskEdited, row); err != nil {
			return Task{}, err
		}
		if len(steps) > 0 {
			if err := c.write(ctx, by, KindTaskSteps, taskStepsRow{TaskID: id, Steps: newSteps(steps), By: by, At: now}); err != nil {
				return Task{}, err
			}
		}
		taker := a.Taker
		if err := c.write(ctx, by, KindTaskState, taskStateRow{TaskID: id, State: Working, Note: o.Note, By: by, At: now,
			Outcome: &Outcome{Kind: OutcomeCard, Note: o.Note}}); err != nil {
			return Task{}, err
		}
		if taker != "" && !c.isOwner(taker) {
			// The taker holds the card. The taker's own device writes its
			// claim; the person closing it for the taker leaves it as is.
			if by == taker {
				if err := c.write(ctx, by, KindTaskClaimed, taskClaimedRow{TaskID: id, By: by, At: now}); err != nil {
					return Task{}, err
				}
			}
		}
	case OutcomeEdit:
		target := strings.TrimSpace(o.Task)
		m := c.tasks[target]
		if m == nil || target == id {
			return Task{}, fmt.Errorf("%w: an edit outcome needs the card it changed (task)", ErrNotFound)
		}
		switch {
		case openAsk(m) != nil:
			return Task{}, fmt.Errorf("%w: %s is an open ask; its prompt is the person's", ErrNotAllowed, m.ShortRef())
		case IsTerminal(m.State):
			return Task{}, fmt.Errorf("%w: %s is %s; an edit outcome changes a card still in play", ErrBadState, m.ShortRef(), m.State)
		}
		row := taskEditedRow{TaskID: target, By: by, From: id, Note: o.Note, At: now}
		if title != "" {
			row.Title = str(title)
		}
		if obj != "" {
			row.Objective = str(obj)
		}
		if done != "" {
			row.DoneCriteria = str(done)
		}
		if err := c.write(ctx, by, KindTaskEdited, row); err != nil {
			return Task{}, err
		}
		if err := c.write(ctx, by, KindTaskState, taskStateRow{TaskID: id, State: Completed, Note: "Changed " + m.ShortRef() + ": " + o.Note,
			By: by, At: now, Outcome: &Outcome{Kind: OutcomeEdit, Task: target, Note: o.Note}}); err != nil {
			return Task{}, err
		}
	case OutcomeNone:
		if err := c.write(ctx, by, KindTaskState, taskStateRow{TaskID: id, State: Canceled, Note: o.Note, By: by, At: now,
			Outcome: &Outcome{Kind: OutcomeNone, Note: o.Note}}); err != nil {
			return Task{}, err
		}
	default:
		return Task{}, fmt.Errorf("%w: outcome is card, edit or none, not %q", ErrInvalid, o.Kind)
	}
	return copyTask(c.tasks[id]), nil
}

// AskRelease gives an ask back (board_leave by its taker): a release
// request the owner device acts on.
func (c *coord) askRelease(ctx context.Context, by string, t *Task) error {
	if err := c.write(ctx, by, KindTakeRequested, takeRequestedRow{TaskID: t.ID, By: by, Op: TakeRelease, At: c.stamp()}); err != nil {
		return err
	}
	c.askDuties(ctx)
	return nil
}

// AskAgain is the person's "Wake them" on an unclaimed ask: every free
// agent hears it again, woken. On the owner device the sweep does it; from
// another device it is a reopen request the owner acts on.
func (c *Coord) AskAgain(ctx context.Context, by, id string) error {
	x := c.impl
	if !x.isOwner(by) {
		return fmt.Errorf("%w: only the person asks again", ErrNotAllowed)
	}
	x.mu.Lock()
	defer x.unlock()
	t, err := x.openTask(id)
	if err != nil {
		return err
	}
	a := openAsk(t)
	if a == nil {
		return fmt.Errorf("%w: %s isn't an open ask", ErrBadState, t.ShortRef())
	}
	if a.Taker != "" {
		return fmt.Errorf("%w: %s took it", ErrHeld, a.Taker)
	}
	if err := x.write(ctx, by, KindTakeRequested, takeRequestedRow{TaskID: id, By: by, Op: TakeReopen, At: x.stamp()}); err != nil {
		return err
	}
	x.askDuties(ctx)
	return nil
}

// StaleNotice is the inbox's Stale hook: a notice about an ask (ref
// task:<id>) that to should no longer hear, because someone else took the
// ask or it closed, gets a note to skip it with; "" keeps it. Safe to call
// under any lock: it only reads a map of its own.
func (c *Coord) StaleNotice(ref, to string) string {
	id, ok := strings.CutPrefix(ref, "task:")
	if !ok {
		return ""
	}
	b := &c.impl.asks
	b.staleMu.RLock()
	defer b.staleMu.RUnlock()
	if b.closed[id] {
		return "the ask closed"
	}
	if taker := b.stale[id]; taker != "" && taker != to {
		return "taken by " + taker
	}
	return ""
}

// askLine says where an ask stands, for show and the board tool ("" for a
// card that isn't one).
func askLine(t *Task) string {
	a := t.Ask
	switch {
	case a == nil:
		return ""
	case a.Outcome != nil:
		s := "closed (" + a.Outcome.Kind + "): " + a.Outcome.Note
		if a.Outcome.Task != "" {
			s += " (task:" + a.Outcome.Task + ")"
		}
		return s
	case a.Taker != "":
		return "the person asked the room; " + a.Taker + " took it (ask_close ends it with what it became)"
	case len(a.Waiting) > 0:
		return "the person asked the room; waiting for device " + a.Owner + " to hand it out"
	case a.Unclaimed:
		return fmt.Sprintf("the person asked the room; nobody took it yet (%d heard it): ask_claim takes it", a.Told)
	}
	return fmt.Sprintf("the person asked the room; %d agents heard it, nobody has taken it: ask_claim takes it", a.Told)
}

// AskLine is askLine for readers outside coord.
func (t Task) AskLine() string { return askLine(&t) }

// askRefusal is the error the lock gives a board tool on an open ask.
func askRefusal(t *Task, what string) error {
	return fmt.Errorf("%w: %s is an ask: %s; use ask_claim to take it and ask_close to end it", ErrNotAllowed, t.ShortRef(), what)
}
