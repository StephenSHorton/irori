package coord

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"sort"
	"strings"
	"sync"
	"time"
	"unicode/utf8"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/owner"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// Errors added beside the contract's.
var (
	ErrNotAllowed = errors.New("coord: only the holder, the creator or a person may do that")
	ErrInvalid    = errors.New("coord: invalid argument")
)

// Release reasons in lease.released rows. A steal writes no lease.released
// row: the stealer's lease.acquired row names the leases it takes in
// "stolen", so a steal is one append and cannot half-happen. ReasonStolen is
// kept for readers that label those leases.
const (
	ReasonReleased   = "released"
	ReasonStolen     = "stolen"
	ReasonHolderGone = "holder_gone"
)

const (
	maxTitle     = 200
	maxObjective = 8000
	maxScope     = 64
)

// IsTerminal reports whether s closes a task.
func IsTerminal(s TaskState) bool { return s == Completed || s == Failed || s == Canceled }

// ValidState reports whether s is one of the seven task states.
func ValidState(s TaskState) bool {
	switch s {
	case Submitted, Working, InputRequired, Completed, Failed, Canceled, Blocked:
		return true
	}
	return false
}

// isOwner reports whether key names the person: exactly "user:<deviceId>"
// for a device certified to them (Options.IsOwner, v0.4 §2). A made-up
// "user:..." key is not a person.
func (c *coord) isOwner(key string) bool { return c.ownerFn(key) }

// IsPerson reports whether key is the person's (the rule coord applies).
func (c *Coord) IsPerson(key string) bool { return c.impl.isOwner(key) }

// Row payloads. Every row carries "at" (this device's clock when it was
// written) so a replay folds to the same times.
type taskCreatedRow struct {
	TaskID string `json:"taskId"`
	// Num is the short number the card claims (short.go): one more than
	// the highest its device had folded. Absent on older rows.
	Num          int      `json:"num,omitempty"`
	Title        string   `json:"title"`
	Objective    string   `json:"objective"`
	DoneCriteria string   `json:"doneCriteria,omitempty"`
	RepoKey      string   `json:"repoKey,omitempty"`
	Scope        []string `json:"scope,omitempty"`
	Shared       bool     `json:"shared,omitempty"`
	Links        []Link   `json:"links,omitempty"`
	// Ask makes the card an ask (ask.go): the person's prompt to the room.
	Ask *askField `json:"ask,omitempty"`
	// Parent makes the card a subtask of that task id (subtasks.go).
	Parent string    `json:"parent,omitempty"`
	By     string    `json:"by"`
	At     time.Time `json:"at"`
}

type taskClaimedRow struct {
	TaskID string    `json:"taskId"`
	By     string    `json:"by"`
	At     time.Time `json:"at"`
}

type taskAssignedRow struct {
	TaskID string    `json:"taskId"`
	To     string    `json:"to"`
	By     string    `json:"by"`
	At     time.Time `json:"at"`
}

type taskStateRow struct {
	TaskID string    `json:"taskId"`
	State  TaskState `json:"state"`
	Note   string    `json:"note,omitempty"`
	// Reason and BlockedBy only when State is Blocked.
	Reason    string     `json:"reason,omitempty"`
	BlockedBy *BlockedBy `json:"blockedBy,omitempty"`
	// Blocker is the item a blocked row raises (cards.go); absent on rows
	// written before items, which fold to an item made from Reason.
	Blocker *blockerRow `json:"blocker,omitempty"`
	// Outcome ends an ask (ask.go): what it became.
	Outcome *Outcome  `json:"outcome,omitempty"`
	By      string    `json:"by"`
	At      time.Time `json:"at"`
}

// taskMemberRow is task.joined and task.left.
type taskMemberRow struct {
	TaskID string    `json:"taskId"`
	Who    string    `json:"who"`
	By     string    `json:"by"`
	At     time.Time `json:"at"`
	// Reason is "gone" when Irori took a holder that went away off the card
	// (card #16); Person then says the card came back to the person.
	Reason string `json:"reason,omitempty"`
	Person bool   `json:"person,omitempty"`
}

type taskSharedRow struct {
	TaskID string    `json:"taskId"`
	Shared bool      `json:"shared"`
	By     string    `json:"by"`
	At     time.Time `json:"at"`
}

type taskLinkedRow struct {
	TaskID  string    `json:"taskId"`
	Link    Link      `json:"link"`
	Removed bool      `json:"removed,omitempty"`
	By      string    `json:"by"`
	At      time.Time `json:"at"`
}

type taskCommentRow struct {
	TaskID    string    `json:"taskId"`
	CommentID string    `json:"commentId"`
	By        string    `json:"by"`
	Text      string    `json:"text"`
	Wake      bool      `json:"wake"`
	At        time.Time `json:"at"`
	// Attachments: v0.4 §6.6; the refs carry the attaching device.
	Attachments []blob.FileRef `json:"attachments,omitempty"`
	// A mirrored inbox message (RecordMessage): commentId == msgId. to is
	// the recipient; gone is the recipient-gone fallback's row (actor
	// irori, by the sender); held means the text was left off; files is the
	// message's attachment count (the refs are never copied). fill is the
	// text of a message recorded held, written (actor irori) once the
	// recipient's device let it through its wake policy.
	To    string `json:"to,omitempty"`
	MsgID string `json:"msgId,omitempty"`
	Gone  bool   `json:"gone,omitempty"`
	Held  bool   `json:"held,omitempty"`
	Files int    `json:"files,omitempty"`
	Fill  bool   `json:"fill,omitempty"`
	// blockerId: a comment on that blocker item, the person's answer (pick:
	// 1-based option, 0 own words; to: the raiser) or an agent's reply
	// (missing: what it still needs). Old peers fold it as a plain comment.
	BlockerID string   `json:"blockerId,omitempty"`
	Pick      int      `json:"pick,omitempty"`
	Missing   []string `json:"missing,omitempty"`
	// unheard: the person's comment had no agent around to hear it.
	Unheard bool `json:"unheard,omitempty"`
}

type blockerClosedRow struct {
	TaskID   string    `json:"taskId"`
	By       string    `json:"by"`
	Note     string    `json:"note,omitempty"`
	Restored TaskState `json:"restored"`
	At       time.Time `json:"at"`
}

type leaseReleasedRow struct {
	Resource string    `json:"resource"`
	Holder   string    `json:"holder"`
	Reason   string    `json:"reason"`
	By       string    `json:"by"`
	At       time.Time `json:"at"`
}

// leaseAcquiredRow is a Lease plus the leases a steal took; the fold drops
// each one that its holder still holds.
type leaseAcquiredRow struct {
	Lease
	Stolen []stolenLease `json:"stolen,omitempty"`
}

type stolenLease struct {
	Resource string `json:"resource"`
	Holder   string `json:"holder"`
}

type leaseRec struct {
	Lease
	res Resource
}

type coord struct {
	st          *store.Store
	roster      interface{ Agents() []presence.AgentCard }
	now         func() time.Time
	grace       time.Duration
	holderGrace time.Duration
	ownerFn     func(string) bool // Options.IsOwner
	revoked     func(string) bool // Options.Revoked
	ignored     int               // owner-only rows from the wrong device (v0.4 §6.0)
	ignoredBy   map[string]int    // and per writing device
	onBoard     bool              // Options.Board (boardlog.go)
	person      string            // Options.Person
	aside       []store.Event     // board rows whose references don't resolve yet

	mu          sync.Mutex
	tasks       map[string]*Task
	order       map[string]uint64 // task id -> fold position of its task.created row
	nextOrder   uint64
	held        map[string]*leaseRec   // by canonical resource
	firstAbsent map[string]time.Time   // holder key -> first seen missing from the roster
	comments    map[string]*Comment    // by comment id
	byTask      map[string][]string    // task id -> comment ids, fold order
	refusals    []*refusalRec          // possibly active, fold order (gcRefusals drops the rest)
	blockerSeq  map[string]int         // task id -> highest blocker item number minted
	stepSeq     map[string]int         // task id -> highest step number minted
	activity    map[string][]Activity  // task id -> top MaxActivity lines by log position, lowest first
	watchq      map[string]*watchQueue // task id -> changes watchers haven't heard (not on replay)

	// Short refs (short.go): every task.created row's claim, the tasks
	// that have one, each number's claimants in cursor order, short ->
	// task id, the highest number claimed (never lowered), and when a
	// card's ref last moved (in memory: search re-reads it).
	claims      []shortClaim
	claimed     map[string]bool
	shortGroups map[int][]shortClaim
	shortIdx    map[string]string
	shortTop    int
	shortMoved  map[string]time.Time

	// Subscribe (subscribe.go). Changes queue in pending while c.mu is held
	// and go out in unlock; replay queues none.
	subs      map[chan Change]*subscriber
	pending   []Change
	replaying bool
	expiry    *time.Timer // fires at expiryAt, the next lease's Until
	expiryAt  time.Time

	// Across devices (mesh.go): the highest seq folded per device, leases
	// that lost a conflict (by resource), conflicts this device already
	// refused, leases the person stole (by stolenKey), and this device's
	// own leases that ran out and still need their release row.
	last      map[string]int64
	conflicts map[string]*leaseRec
	refused   map[string]bool
	stolen    map[string]bool
	lapsed    []Lease

	// Asks (ask.go): take requests, the owner's states and its memory.
	asks askBook

	// Subtasks (subtasks.go): every parent link row, in fold order;
	// relink sorts them.
	links []parentLink
}

func newCoord(ctx context.Context, st *store.Store, roster interface{ Agents() []presence.AgentCard }, opts Options) (*Coord, error) {
	if st == nil {
		return nil, fmt.Errorf("%w: nil store", ErrInvalid)
	}
	c := &coord{
		st:          st,
		roster:      roster,
		now:         opts.Now,
		grace:       opts.GoneGrace,
		ownerFn:     opts.IsOwner,
		revoked:     opts.Revoked,
		onBoard:     opts.Board,
		person:      opts.Person,
		tasks:       map[string]*Task{},
		order:       map[string]uint64{},
		held:        map[string]*leaseRec{},
		firstAbsent: map[string]time.Time{},
		comments:    map[string]*Comment{},
		byTask:      map[string][]string{},
		blockerSeq:  map[string]int{},
		stepSeq:     map[string]int{},
		activity:    map[string][]Activity{},
		watchq:      map[string]*watchQueue{},
		claimed:     map[string]bool{},
		shortGroups: map[int][]shortClaim{},
		shortIdx:    map[string]string{},
		shortMoved:  map[string]time.Time{},
		subs:        map[chan Change]*subscriber{},
		last:        map[string]int64{},
		conflicts:   map[string]*leaseRec{},
		refused:     map[string]bool{},
		stolen:      map[string]bool{},
		asks:        newAskBook(),
	}
	if c.now == nil {
		c.now = time.Now
	}
	if c.ownerFn == nil {
		self := c.selfKey()
		c.ownerFn = func(k string) bool { return k == self }
	}
	if c.grace <= 0 {
		c.grace = GoneGrace
	}
	if c.holderGrace = opts.HolderGrace; c.holderGrace <= 0 {
		c.holderGrace = HolderGrace
	}
	if err := c.replay(ctx); err != nil {
		return nil, err
	}
	return &Coord{impl: c}, nil
}

// replay folds every task./lease./blocker. row in (hlc, device, seq) order,
// paging with the store's cursor so rows from two devices that share an HLC
// (possible once rows replicate) are never skipped at a page boundary.
func (c *coord) replay(ctx context.Context) error {
	f := store.Filter{Kinds: followKinds, Limit: store.MaxListLimit}
	c.replaying = true
	defer func() { c.replaying = false }()
	for {
		evs, err := c.st.List(ctx, f)
		if err != nil {
			return fmt.Errorf("coord: replay: %w", err)
		}
		for _, ev := range evs {
			c.apply(ev)
			if ev.Seq > c.last[ev.DeviceID] {
				c.last[ev.DeviceID] = ev.Seq
			}
		}
		c.prune(c.stamp())
		if len(evs) < f.Limit {
			c.renumber()
			c.relink()
			return nil
		}
		f.After = evs[len(evs)-1].Cursor()
	}
}

// writerOK applies the owner table (v0.4 §6.0): lease rows come only from
// the holder's device (a refusal from the refused acquirer's). A row from
// any other device is ignored and counted. On one device every holder is
// this device's, so nothing changes there.
func (c *coord) writerOK(ev store.Event) bool {
	switch ev.Kind {
	case KindLeaseAcq:
		var row leaseAcquiredRow
		if json.Unmarshal(ev.Payload, &row) != nil {
			return true // apply drops it
		}
		return owner.MayWrite(row.Holder, ev.DeviceID)
	case KindLeaseRel:
		var p leaseReleasedRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return true
		}
		return owner.MayWrite(p.Holder, ev.DeviceID)
	case KindLeaseRefused:
		var r Refusal
		if json.Unmarshal(ev.Payload, &r) != nil {
			return true
		}
		return owner.MayWrite(r.By, ev.DeviceID)
	case KindTaskComment:
		// Only a mirrored message and a blocker comment are checked. A
		// mirrored message's sender's device writes it, and a gone or fill
		// row may also come from the recipient's. An agent's blocker reply
		// comes from its own device (the person's answer is signed "user").
		// Plain comments are left as they were.
		var p taskCommentRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return true
		}
		switch {
		case p.MsgID != "":
			return mayRecord(p.By, p.To, p.Gone || p.Fill, ev.DeviceID)
		case p.BlockerID != "" && !c.personActor(ev):
			return owner.MayWrite(p.By, ev.DeviceID)
		}
		return true
	}
	return true
}

// mayRecord is the writer rule for a mirrored message's row: the sender's
// device, or the recipient's for a gone or fill row (byRecipient).
func mayRecord(by, to string, byRecipient bool, dev string) bool {
	return owner.MayWrite(by, dev) || byRecipient && owner.MayWrite(to, dev)
}

// IgnoredRows counts owner-only rows coord ignored because a device other
// than the owner wrote them.
func (c *Coord) IgnoredRows() int {
	c.impl.mu.Lock()
	defer c.impl.mu.Unlock()
	return c.impl.ignored
}

// apply folds one row, then any rows the take lock kept back that it lets
// through now (ask.go).
func (c *coord) apply(ev store.Event) {
	c.fold(ev)
	c.foldDeferred()
	c.retryAside()
	if !c.replaying {
		c.relink()
	}
}

// fold folds one row. Rows are applied as written; validation happened
// before the append.
func (c *coord) fold(ev store.Event) {
	if !c.writerOK(ev) {
		c.ignore(ev.DeviceID)
		return
	}
	if !c.asideGate(ev) {
		return
	}
	if ok, wait := c.askGate(ev); !ok {
		if wait != "" {
			c.asks.deferRow(wait, ev) // the take lock (ask.go)
		}
		return
	}
	at := func(t time.Time) time.Time {
		if t.IsZero() {
			return ev.HLC.Wall().UTC()
		}
		return t
	}
	switch ev.Kind {
	case KindTaskCreated:
		var p taskCreatedRow
		if json.Unmarshal(ev.Payload, &p) != nil || p.TaskID == "" {
			return
		}
		t := at(p.At)
		if _, dup := c.tasks[p.TaskID]; !dup {
			c.nextOrder++
			c.order[p.TaskID] = c.nextOrder
		}
		var links []Link
		for _, l := range p.Links {
			if len(links) < MaxLinks && l.Ref != "" && linkIndex(links, l.Ref) < 0 {
				links = append(links, l)
			}
		}
		c.tasks[p.TaskID] = &Task{
			ID: p.TaskID, Title: p.Title, Objective: p.Objective, DoneCriteria: p.DoneCriteria,
			RepoKey: p.RepoKey, Scope: p.Scope, State: Submitted, CreatedBy: p.By,
			CreatedAt: t, UpdatedAt: t, UpdatedBy: p.By, Shared: p.Shared, Links: links,
		}
		c.foldAskCreated(c.tasks[p.TaskID], p.Ask, ev, p.By)
		if p.Parent != "" {
			c.foldParent(p.TaskID, p.Parent, ev)
		}
		c.claimShort(p.TaskID, ev.Cursor(), p.Num)
		if pt := c.tasks[p.Parent]; p.Parent != "" && pt != nil {
			// A new subtask: the parent's holders hear it (relink hasn't run).
			c.touch(ev, c.tasks[p.TaskID], t, p.By, "put the card up as a subtask of "+pt.ShortRef(), true, pt.Holders...)
		} else {
			c.touch(ev, c.tasks[p.TaskID], t, p.By, "put the card up", false)
		}
		c.note(Change{Kind: TaskChanged, TaskID: p.TaskID})
	case KindTaskClaimed:
		var p taskClaimedRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		if t := c.tasks[p.TaskID]; t != nil && p.By != "" {
			addHolder(t, p.By)
			t.Assigned = removeStr(t.Assigned, p.By)
			syncHolder(t)
			if t.State == Submitted {
				t.State = Working
			}
			t.UpdatedAt, t.UpdatedBy = at(p.At), p.By
			c.touch(ev, t, t.UpdatedAt, p.By, "took it up", true)
			c.note(Change{Kind: TaskChanged, TaskID: p.TaskID, Holder: p.By})
		}
	case KindTaskAssigned:
		var p taskAssignedRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		if t := c.tasks[p.TaskID]; t != nil && p.To != "" {
			engaged := isHolder(t, p.To) && !contains(t.Assigned, p.To)
			addHolder(t, p.To)
			if !engaged && !contains(t.Assigned, p.To) {
				t.Assigned = append(t.Assigned, p.To)
			}
			syncHolder(t)
			t.UpdatedAt, t.UpdatedBy = at(p.At), p.By
			c.touch(ev, t, t.UpdatedAt, p.By, "assigned it to "+p.To, true)
			c.note(Change{Kind: TaskChanged, TaskID: p.TaskID, Holder: p.To})
		}
	case KindTaskState:
		var p taskStateRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		if t := c.tasks[p.TaskID]; t != nil && ValidState(p.State) {
			was := t.State
			when := at(p.At)
			if openAsk(t) != nil {
				o := p.Outcome
				if o == nil {
					o = &Outcome{Kind: OutcomeNone, Note: cmpStr(p.Note, "closed by the person")}
				}
				c.closeAskFold(t, o, ev.Cursor())
			}
			t.State = p.State
			t.Note = p.Note
			t.Assigned = removeStr(t.Assigned, p.By)
			syncHolder(t)
			what, raised := "", ""
			if p.State == Blocked {
				// BlockedBy follows the items (syncReason below).
				what, raised = c.foldBlocked(t, p, ev, was == Blocked, when)
			} else {
				closeOpen(t, p.By, p.Note, when)
				if was != p.State {
					what = "set it to " + string(p.State)
				}
			}
			syncReason(t)
			t.UpdatedAt, t.UpdatedBy = when, p.By
			line := lineOf(ev, when, p.By, what)
			line.BlockerID = raised // a raise line names its item
			c.touchLine(t, line, true)
			if raised != "" {
				// A blocker comment that reached this device before its raise.
				c.refreshBlocker(t, raised)
			}
			c.note(Change{Kind: TaskChanged, TaskID: p.TaskID, Holder: t.Holder})
		}
	case KindTaskJoined, KindTaskLeft:
		var p taskMemberRow
		if json.Unmarshal(ev.Payload, &p) != nil || p.Who == "" {
			return
		}
		t := c.tasks[p.TaskID]
		if t == nil {
			return
		}
		what := "left it"
		if ev.Kind == KindTaskJoined {
			if !contains(t.Holders, p.Who) {
				t.Holders = append(t.Holders, p.Who)
			}
			if t.State == Submitted {
				t.State = Working
			}
			what = "joined"
		} else {
			t.Holders = removeStr(t.Holders, p.Who)
			if p.Reason == LeftGone {
				what = "went away"
				if len(t.Holders) == 0 {
					t.Returned = &Returned{Was: p.Who, At: at(p.At), Person: p.Person, Row: fmt.Sprintf("%s:%d", ev.DeviceID, ev.Seq)}
				}
			}
		}
		if p.Who != p.By {
			what += " (" + p.Who + ")"
		}
		t.Assigned = removeStr(t.Assigned, p.Who)
		syncHolder(t)
		t.UpdatedAt, t.UpdatedBy = at(p.At), p.By
		c.touch(ev, t, t.UpdatedAt, p.By, what, false)
		c.note(Change{Kind: TaskChanged, TaskID: p.TaskID, Holder: p.Who})
	case KindTaskShared:
		var p taskSharedRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		if t := c.tasks[p.TaskID]; t != nil {
			t.Shared = p.Shared
			t.UpdatedAt, t.UpdatedBy = at(p.At), p.By
			c.note(Change{Kind: TaskChanged, TaskID: p.TaskID, Holder: t.Holder})
		}
	case KindTaskLinked:
		var p taskLinkedRow
		if json.Unmarshal(ev.Payload, &p) != nil || p.Link.Ref == "" {
			return
		}
		t := c.tasks[p.TaskID]
		if t == nil {
			return
		}
		i := linkIndex(t.Links, p.Link.Ref)
		switch {
		case p.Removed && i >= 0:
			t.Links = append(t.Links[:i:i], t.Links[i+1:]...)
			if len(t.Links) == 0 {
				t.Links = nil
			}
		case !p.Removed && i < 0 && len(t.Links) < MaxLinks:
			t.Links = append(t.Links, p.Link)
		default:
			return
		}
		t.UpdatedAt, t.UpdatedBy = at(p.At), p.By
		_, _, isPR := prLink(p.Link)
		if p.Removed {
			c.touch(ev, t, t.UpdatedAt, p.By, "removed the link "+p.Link.Label, isPR)
		} else {
			c.touch(ev, t, t.UpdatedAt, p.By, "linked "+p.Link.Label, isPR)
		}
		c.note(Change{Kind: TaskChanged, TaskID: p.TaskID, Holder: t.Holder})
	case KindTaskComment:
		var p taskCommentRow
		if json.Unmarshal(ev.Payload, &p) != nil || p.CommentID == "" {
			return
		}
		if cm := c.comments[p.CommentID]; cm != nil {
			c.mergeComment(cm, p, ev, at)
			return
		}
		if p.MsgID != "" {
			p.BlockerID, p.Pick, p.Missing = "", 0, nil // a message is never on a blocker
		}
		t := c.tasks[p.TaskID]
		if t == nil {
			return
		}
		primary := !p.Gone && !p.Fill
		cm := &Comment{
			ID: p.CommentID, TaskID: p.TaskID, By: p.By, Text: p.Text, Wake: p.Wake, At: at(p.At), Attachments: p.Attachments,
			To: p.To, MsgID: p.MsgID, Gone: p.Gone, Held: p.Held, Files: p.Files,
			BlockerID: p.BlockerID, Pick: max(p.Pick, 0), Missing: p.Missing,
			Device: ev.DeviceID, HLC: ev.HLC, Seq: ev.Seq, primary: primary,
			person: c.personRow(ev, p.By),
		}
		cm.Unheard = p.Unheard && cm.person
		if !primary && !p.Held {
			cm.passed = p.Text
		}
		c.comments[cm.ID] = cm
		c.byTask[t.ID] = append(c.byTask[t.ID], cm.ID)
		t.Comments++
		// A comment is activity on the card: it floats up its Board group.
		// Only a primary row moves it, so a mirrored message's gone or fill
		// row can't, whichever arrives first.
		if primary {
			t.UpdatedAt = cm.At
		}
		// A mirrored message is its own thread entry: no activity line, so
		// the line's position can't depend on which of its rows came first.
		// A blocker comment's line names its item, so the main thread
		// keeps the event while the comment goes in the item's own thread.
		switch {
		case p.BlockerID != "":
			what := "replied on blocker " + p.BlockerID
			if cm.person {
				what = "answered blocker " + p.BlockerID
			}
			line := lineOf(ev, cm.At, cm.By, what)
			line.BlockerID = p.BlockerID
			c.touchLine(t, line, false)
			c.refreshBlocker(t, p.BlockerID)
			c.note(Change{Kind: TaskChanged, TaskID: p.TaskID, Holder: t.Holder})
		case p.MsgID == "":
			c.touch(ev, t, cm.At, cm.By, "commented", false)
		}
		c.note(Change{Kind: CommentAdded, TaskID: p.TaskID})
	case KindBlockerClosed:
		var p blockerClosedRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		t := c.tasks[p.TaskID]
		if t == nil || t.State != Blocked {
			return
		}
		s := p.Restored
		if !ValidState(s) || s == Blocked || IsTerminal(s) {
			s = restoreState(t)
		}
		t.State = s
		t.Reason, t.BlockedBy = "", nil
		if p.Note != "" {
			t.Note = p.Note
		}
		t.UpdatedAt, t.UpdatedBy = at(p.At), p.By
		closeOpen(t, p.By, p.Note, t.UpdatedAt)
		what := "unblocked it"
		if p.Note != "" {
			what += ": " + p.Note
		}
		c.touch(ev, t, t.UpdatedAt, p.By, what, true)
		c.note(Change{Kind: TaskChanged, TaskID: p.TaskID, Holder: t.Holder})
	case KindBlockerOption, KindBlockerResolved, KindTaskSteps, KindTaskStep, KindTaskWatched:
		c.applyCard(ev, at)
	case KindTaskParent:
		var p taskParentRow
		if json.Unmarshal(ev.Payload, &p) != nil || p.TaskID == "" {
			return
		}
		c.foldParent(p.TaskID, p.Parent, ev)
		if t := c.tasks[p.TaskID]; t != nil {
			what := "took it out of its parent"
			var also []string // the new parent's holders: relink hasn't run yet
			if p.Parent != "" {
				what = "made it part of " + c.refOf(p.Parent)
				if pt := c.tasks[p.Parent]; pt != nil {
					also = pt.Holders
				}
			}
			c.touch(ev, t, at(p.At), p.By, what, true, also...)
		}
	case KindTakeRequested, KindTakeState, KindTaskEdited:
		c.applyTake(ev, at)
	case KindLeaseRefused:
		var r Refusal
		if json.Unmarshal(ev.Payload, &r) != nil || r.Resource == "" || r.By == "" || r.HeldBy == "" {
			return
		}
		res, err := ParseResource(r.Resource)
		if err != nil {
			res = Resource{Kind: ResourceNamed, Name: r.Resource}
		}
		r.At = at(r.At)
		if r.Conflict {
			c.endConflict(res.Name, r.By)
		}
		c.refusals = append(c.refusals, &refusalRec{Refusal: r, res: res})
		c.note(Change{Kind: LeaseRefused, Resource: res.Name, Holder: r.By})
	case KindLeaseAcq:
		var row leaseAcquiredRow
		if json.Unmarshal(ev.Payload, &row) != nil || row.Resource == "" {
			return
		}
		for _, v := range row.Stolen {
			if c.dropLease(v.Resource, v.Holder) {
				c.note(Change{Kind: LeaseChanged, Resource: v.Resource, Holder: v.Holder, Reason: ReasonStolen})
			}
		}
		p := row.Lease
		res, err := ParseResource(p.Resource)
		if err != nil {
			res = Resource{Kind: ResourceNamed, Name: p.Resource}
		}
		p.AcquiredAt = at(p.AcquiredAt)
		if c.stolen[stolenKey(res.Name, p.Holder, p.AcquiredAt)] {
			return // a renewal of a lease the person stole doesn't revive it
		}
		rec := &leaseRec{Lease: p, res: res}
		if l := c.conflicts[res.Name]; l != nil && l.Holder == p.Holder {
			rec.Conflict = l.Conflict // the contested lease renewed: still contested
			c.conflicts[res.Name] = rec
			return
		}
		if c.conflictOf(rec, row.Stolen) {
			return
		}
		if old := c.held[res.Name]; old != nil && old.Holder == p.Holder {
			rec.Conflict = old.Conflict
		}
		c.held[res.Name] = rec
		c.note(Change{Kind: LeaseChanged, Resource: res.Name, Holder: p.Holder})
	case KindLeaseStolen:
		c.applyStolen(ev)
	case KindLeaseRel:
		var p leaseReleasedRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		name := p.Resource
		if n, err := NormalizeResource(p.Resource); err == nil {
			name = n
		}
		c.lapsed = slices.DeleteFunc(c.lapsed, func(l Lease) bool { return l.Resource == name && l.Holder == p.Holder })
		if c.endConflict(name, p.Holder) {
			c.note(Change{Kind: LeaseChanged, Resource: p.Resource, Holder: p.Holder, Reason: p.Reason})
		}
	}
}

// dropLease removes resource's lease if holder still holds it, and reports
// whether it did.
func (c *coord) dropLease(resource, holder string) bool {
	name := resource
	if n, err := NormalizeResource(resource); err == nil {
		name = n
	}
	if l := c.held[name]; l != nil && l.Holder == holder {
		delete(c.held, name)
		return true
	}
	return false
}

// prune forgets expired leases so held stays the size of what is live. The
// caller holds c.mu (or is replaying).
// Only this device's own leases lapse here; another device's ends with a
// row from its holder's device (v0.4 §6.4). A lapsed own lease is queued
// for its release row (holderDuties), so other devices end it too.
func (c *coord) prune(now time.Time) {
	for name, l := range c.held {
		if !c.live(l, now) {
			delete(c.held, name)
			// At replay only a recent lapse still needs its row: older ones
			// were never seen by another device as live (before v0.4).
			// A removed device's lease ends with no row at all.
			if c.localKey(l.Holder) && (!c.replaying || now.Sub(l.Until) < lapsedWithin) {
				c.lapsed = append(c.lapsed, l.Lease)
			}
			c.note(Change{Kind: LeaseExpired, Resource: name, Holder: l.Holder})
		}
	}
	c.gcRefusals(now)
}

// addHolder records who as working t: on a Shared task who is added (once),
// otherwise who replaces the holder.
func addHolder(t *Task, who string) {
	switch {
	case !t.Shared:
		t.Holders = []string{who}
	case !contains(t.Holders, who):
		t.Holders = append(t.Holders, who)
	}
	syncHolder(t)
}

// syncHolder keeps Holder = Holders[0] and Assigned a subset of Holders.
func syncHolder(t *Task) {
	var keep []string
	for _, a := range t.Assigned {
		if isHolder(t, a) {
			keep = append(keep, a)
		}
	}
	t.Assigned = keep
	if len(t.Holders) == 0 {
		t.Holders, t.Holder = nil, ""
		return
	}
	t.Holder = t.Holders[0]
	t.Returned = nil
}

// isHolder reports whether key is one of t's holders.
func isHolder(t *Task, key string) bool { return contains(t.Holders, key) }

// mayManage: the creator, any holder or a person.
func (c *coord) mayManage(t *Task, by string) bool {
	return by == t.CreatedBy || isHolder(t, by) || c.isOwner(by)
}

// restoreState is where Unblock leaves a task: Working when a holder has
// taken it up, else Submitted (no holders, or only assignees that haven't
// claimed: a submitted task stays submitted until its assignee claims).
func restoreState(t *Task) TaskState {
	for _, h := range t.Holders {
		if !contains(t.Assigned, h) {
			return Working
		}
	}
	return Submitted
}

// engaged reports whether key holds t and has taken it up (not only been
// assigned it).
func engaged(t *Task, key string) bool {
	return isHolder(t, key) && !contains(t.Assigned, key)
}

func removeStr(xs []string, x string) []string {
	out := xs[:0:0]
	for _, v := range xs {
		if v != x {
			out = append(out, v)
		}
	}
	if len(out) == 0 {
		return nil
	}
	return out
}

func linkIndex(ls []Link, ref string) int {
	for i, l := range ls {
		if l.Ref == ref {
			return i
		}
	}
	return -1
}

// write appends a row as by and folds it. The caller holds c.mu.
func (c *coord) write(ctx context.Context, by, kind string, payload any) error {
	return c.writeAs(ctx, c.actorFor(by), kind, payload)
}

// writeAs is write with the store's actor given ("irori" for a row Irori
// writes on someone's behalf). The caller holds c.mu.
func (c *coord) writeAs(ctx context.Context, actor, kind string, payload any) error {
	ev, err := c.st.Append(ctx, actor, kind, payload)
	if err != nil {
		return fmt.Errorf("coord: %w", err)
	}
	c.apply(ev)
	return nil
}

// actorFor maps a caller to the store's actor forms.
func (c *coord) actorFor(by string) string {
	switch {
	case by == "irori":
		return "irori"
	case c.onBoard:
		return by // a board key is its own actor; the store refuses any other
	case c.isOwner(by):
		return "user"
	case strings.HasPrefix(by, "api:"):
		return by // the loopback API on an agent's behalf: never "user" (v0.4 §2)
	}
	return "agent:" + by
}

func (c *coord) stamp() time.Time { return c.now().UTC().Round(0) }

func copyTask(t *Task) Task {
	out := *t
	if t.Scope != nil {
		out.Scope = append([]string(nil), t.Scope...)
	}
	if t.Holders != nil {
		out.Holders = append([]string(nil), t.Holders...)
	}
	if t.Assigned != nil {
		out.Assigned = append([]string(nil), t.Assigned...)
	}
	if t.Links != nil {
		out.Links = append([]Link(nil), t.Links...)
	}
	if t.BlockedBy != nil {
		b := *t.BlockedBy
		out.BlockedBy = &b
	}
	if t.Steps != nil {
		out.Steps = append([]Step(nil), t.Steps...)
	}
	if t.Subtasks != nil {
		out.Subtasks = append([]string(nil), t.Subtasks...)
	}
	if t.Rollup != nil {
		r := *t.Rollup
		out.Rollup = &r
	}
	if t.Blockers != nil {
		out.Blockers = make([]Blocker, len(t.Blockers))
		for i, b := range t.Blockers {
			out.Blockers[i] = copyBlocker(b)
		}
	}
	if t.Watchers != nil {
		out.Watchers = append([]string(nil), t.Watchers...)
	}
	if t.Ask != nil {
		a := *t.Ask
		a.Heard, a.Waiting = slices.Clone(a.Heard), slices.Clone(a.Waiting)
		if a.Outcome != nil {
			o := *a.Outcome
			a.Outcome = &o
		}
		out.Ask = &a
	}
	return out
}

// ---- gone tracking ----

// observe refreshes first-absent times for every holder that matters. A
// holder is absent when the roster has no card for it or its card is "gone".
// People are never absent; a nil roster means nobody is.
//
// A holder's clock starts when observe first sees it absent, or earlier at a
// "gone" card's UpdatedAt (presence stamps it when it marks the card gone). A
// holder with no card at all gets no such seed, and presence.Snapshot drops
// gone cards rather than keeping them, so with today's roster the grace runs
// from the first observe (Sweep, or a ClaimTask on its task) after the holder
// left: Core must call Sweep every few seconds. The clock is in memory and
// restarts with the process.
func (c *coord) observe(now time.Time) {
	if c.roster == nil {
		return
	}
	live := map[string]bool{}
	goneAt := map[string]time.Time{}
	for _, a := range c.roster.Agents() {
		if a.Status != "gone" {
			live[a.Key] = true
		} else if !a.UpdatedAt.IsZero() && a.UpdatedAt.Before(now) {
			goneAt[a.Key] = a.UpdatedAt
		}
	}
	tracked := map[string]bool{}
	for _, t := range c.tasks {
		if IsTerminal(t.State) {
			continue
		}
		for _, h := range t.Holders {
			tracked[h] = true
		}
	}
	for _, l := range c.held {
		if c.live(l, now) {
			tracked[l.Holder] = true
		}
	}
	for k := range c.firstAbsent {
		if !tracked[k] {
			delete(c.firstAbsent, k)
		}
	}
	for k := range tracked {
		// Another device's agent is never gone by this device's view of it:
		// its host's Core releases what it held (v0.4 §6.4).
		if c.isOwner(k) || live[k] || !c.localKey(k) {
			delete(c.firstAbsent, k)
			continue
		}
		first, ok := c.firstAbsent[k]
		if !ok {
			first = now
		}
		if g, ok := goneAt[k]; ok && g.Before(first) {
			first = g
		}
		c.firstAbsent[k] = first
	}
}

// gone reports whether key has been absent for the grace period.
func (c *coord) gone(key string, now time.Time) bool {
	t, ok := c.firstAbsent[key]
	return ok && now.Sub(t) >= c.grace
}

// ---- tasks ----

func (c *coord) createTask(ctx context.Context, by string, nt NewTask) (Task, error) {
	ct, err := cleanNew(by, nt)
	if err != nil {
		return Task{}, err
	}
	c.mu.Lock()
	defer c.unlock()
	if nt.Parent != "" {
		pid, err := c.taskRefLocked(nt.Parent)
		if err != nil {
			return Task{}, err
		}
		if _, err := c.checkParent(pid, ""); err != nil {
			return Task{}, err
		}
		ct.parent = pid
	}
	return c.createLocked(ctx, ct.by, ct)
}

// cleanTask is a NewTask checked and trimmed, ready for createLocked.
type cleanTask struct {
	by, title, obj, done, repoKey, reason, parent string
	scope                                         []string
	links                                         []Link
	shared                                        bool
	blockedBy                                     *BlockedBy
	steps                                         []Step
}

// cleanNew checks and trims nt (CreateTask's rules). Parent is left to the
// caller, which resolves it under c.mu.
func cleanNew(by string, nt NewTask) (cleanTask, error) {
	by = strings.TrimSpace(by)
	if by == "" {
		return cleanTask{}, fmt.Errorf("%w: no caller", ErrInvalid)
	}
	title := strings.TrimSpace(nt.Title)
	if title == "" {
		return cleanTask{}, fmt.Errorf("%w: a task needs a title", ErrInvalid)
	}
	if utf8.RuneCountInString(title) > maxTitle {
		return cleanTask{}, fmt.Errorf("%w: title longer than %d characters", ErrInvalid, maxTitle)
	}
	obj := strings.TrimSpace(nt.Objective)
	done := strings.TrimSpace(nt.DoneCriteria)
	if utf8.RuneCountInString(obj) > maxObjective || utf8.RuneCountInString(done) > maxObjective {
		return cleanTask{}, fmt.Errorf("%w: objective or done criteria longer than %d characters", ErrInvalid, maxObjective)
	}
	repoKey := strings.TrimSpace(nt.RepoKey)
	if repoKey != "" {
		if err := checkRepoKey(repoKey); err != nil {
			return cleanTask{}, err
		}
	}
	var scope []string
	for _, g := range nt.Scope {
		if strings.TrimSpace(g) == "" {
			continue
		}
		n, err := NormalizeGlob(g)
		if err != nil {
			return cleanTask{}, err
		}
		scope = append(scope, n)
	}
	if len(scope) > maxScope {
		return cleanTask{}, fmt.Errorf("%w: more than %d scope globs", ErrInvalid, maxScope)
	}
	var links []Link
	for _, s := range nt.Links {
		if strings.TrimSpace(s) == "" {
			continue
		}
		l, err := ParseLink(s)
		if err != nil {
			return cleanTask{}, err
		}
		if linkIndex(links, l.Ref) < 0 {
			links = append(links, l)
		}
	}
	if len(links) > MaxLinks {
		return cleanTask{}, fmt.Errorf("%w: more than %d links", ErrInvalid, MaxLinks)
	}
	reason := strings.TrimSpace(nt.BlockReason)
	if reason == "" && nt.BlockedBy != nil {
		return cleanTask{}, fmt.Errorf("%w: BlockedBy needs a BlockReason", ErrInvalid)
	}
	if err := checkReason(reason, false); err != nil {
		return cleanTask{}, err
	}
	steps := append([]Step(nil), nt.Steps...)
	if err := checkSteps(steps); err != nil {
		return cleanTask{}, err
	}
	return cleanTask{by: by, title: title, obj: obj, done: done, repoKey: repoKey, reason: reason, scope: scope,
		links: links, shared: nt.Shared, blockedBy: nt.BlockedBy, steps: newSteps(steps)}, nil
}

// createLocked writes a checked task's rows. c.mu is held.
func (c *coord) createLocked(ctx context.Context, by string, ct cleanTask) (Task, error) {
	reason, steps := ct.reason, ct.steps
	var bb *BlockedBy
	if reason != "" {
		var err error
		if bb, err = c.checkBlocker(ct.blockedBy, ""); err != nil {
			return Task{}, err
		}
	}
	at := c.stamp()
	id := newID(at)
	if err := c.write(ctx, by, KindTaskCreated, taskCreatedRow{
		TaskID: id, Num: c.nextShort(), Title: ct.title, Objective: ct.obj, DoneCriteria: ct.done,
		RepoKey: ct.repoKey, Scope: ct.scope, Shared: ct.shared, Links: ct.links, Parent: ct.parent, By: by, At: at,
	}); err != nil {
		return Task{}, err
	}
	if len(steps) > 0 {
		if err := c.write(ctx, by, KindTaskSteps, taskStepsRow{TaskID: id, Steps: steps, By: by, At: at}); err != nil {
			return Task{}, err
		}
	}
	if reason != "" {
		q, cx := blockQuestion(reason)
		if err := c.write(ctx, by, KindTaskState, taskStateRow{
			TaskID: id, State: Blocked, Reason: reason, BlockedBy: bb,
			Blocker: &blockerRow{ID: c.nextBlockerID(id), Question: q, Context: cx, ReasonOnly: true, On: bb}, By: by, At: at,
		}); err != nil {
			return Task{}, err
		}
	}
	return copyTask(c.tasks[id]), nil
}

func (c *coord) listTasks(f TaskFilter) []Task {
	c.mu.Lock()
	defer c.unlock()
	out := []Task{}
	for _, t := range c.tasks {
		if f.RepoKey != "" && t.RepoKey != f.RepoKey ||
			f.Holder != "" && !isHolder(t, f.Holder) ||
			f.Open && IsTerminal(EffectiveState(*t)) { // a parent ends with its subtasks
			continue
		}
		out = append(out, copyTask(t))
	}
	// Creation time, then fold order (ids minted in one millisecond have
	// random tails), then id.
	sort.Slice(out, func(i, j int) bool {
		if !out[i].CreatedAt.Equal(out[j].CreatedAt) {
			return out[i].CreatedAt.Before(out[j].CreatedAt)
		}
		if oi, oj := c.order[out[i].ID], c.order[out[j].ID]; oi != oj {
			return oi < oj
		}
		return out[i].ID < out[j].ID
	})
	return out
}

func (c *coord) getTask(id string) (Task, bool) {
	c.mu.Lock()
	defer c.unlock()
	t, ok := c.tasks[id]
	if !ok {
		return Task{}, false
	}
	return copyTask(t), true
}

// openTask finds a non-terminal task. The caller holds c.mu.
func (c *coord) openTask(id string) (*Task, error) {
	t := c.tasks[id]
	if t == nil {
		return nil, fmt.Errorf("%w: task %q", ErrNotFound, id)
	}
	if IsTerminal(t.State) {
		return nil, fmt.Errorf("%w: task %s is %s", ErrBadState, id, t.State)
	}
	return t, nil
}

func (c *coord) claimTask(ctx context.Context, by, id string) (Task, error) {
	by = strings.TrimSpace(by)
	if by == "" {
		return Task{}, fmt.Errorf("%w: no caller", ErrInvalid)
	}
	c.mu.Lock()
	defer c.unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, err
	}
	if openAsk(t) != nil {
		return Task{}, askRefusal(t, "one agent takes it")
	}
	// An assignee claiming writes the claimed row (which moves a submitted
	// task to working and takes the assignee off Assigned).
	if engaged(t, by) && t.State != Submitted {
		return copyTask(t), nil
	}
	now := c.stamp()
	// A shared task takes any number of holders: claiming it is joining.
	if !t.Shared {
		for _, h := range t.Holders {
			if h == by {
				continue
			}
			c.observe(now)
			if !c.gone(h, now) {
				return Task{}, fmt.Errorf("%w: task %s is held by %s", ErrHeld, id, h)
			}
		}
	}
	if err := c.write(ctx, by, KindTaskClaimed, taskClaimedRow{TaskID: id, By: by, At: now}); err != nil {
		return Task{}, err
	}
	return copyTask(t), nil
}

func (c *coord) assignTask(ctx context.Context, by, id, to string) (Task, error) {
	t, _, err := c.assign(ctx, by, id, to, false)
	return t, err
}

// assign is AssignTask (wide false: the creator, a holder or a person) and
// Assign (wide: also anyone while the task has no holder). The bool reports
// whether a row was written.
func (c *coord) assign(ctx context.Context, by, id, to string, wide bool) (Task, bool, error) {
	by, to = strings.TrimSpace(by), strings.TrimSpace(to)
	if by == "" || to == "" {
		return Task{}, false, fmt.Errorf("%w: assign needs a caller and an assignee", ErrInvalid)
	}
	if err := checkKey(to); err != nil {
		return Task{}, false, err
	}
	c.mu.Lock()
	defer c.unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, false, err
	}
	if openAsk(t) != nil {
		return Task{}, false, askRefusal(t, "nobody is assigned one")
	}
	if !c.mayManage(t, by) && !(wide && len(t.Holders) == 0) {
		return Task{}, false, fmt.Errorf("%w: assign task %s", ErrNotAllowed, id)
	}
	if isHolder(t, to) {
		return copyTask(t), false, nil
	}
	if err := c.write(ctx, by, KindTaskAssigned, taskAssignedRow{TaskID: id, To: to, By: by, At: c.stamp()}); err != nil {
		return Task{}, false, err
	}
	return copyTask(t), true, nil
}

func (c *coord) setTaskState(ctx context.Context, by, id string, s TaskState, note string) (Task, error) {
	by = strings.TrimSpace(by)
	if by == "" {
		return Task{}, fmt.Errorf("%w: no caller", ErrInvalid)
	}
	if !ValidState(s) {
		return Task{}, fmt.Errorf("%w: %q (submitted|working|input_required|completed|failed|canceled)", ErrBadState, s)
	}
	if s == Blocked {
		return Task{}, fmt.Errorf("%w: use Block, which records what is in the way", ErrBadState)
	}
	note = strings.TrimSpace(note)
	if utf8.RuneCountInString(note) > maxObjective {
		return Task{}, fmt.Errorf("%w: note longer than %d characters", ErrInvalid, maxObjective)
	}
	c.mu.Lock()
	defer c.unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, err
	}
	if openAsk(t) != nil {
		// The person closes an ask (nothing came of it); nobody else sets
		// its state.
		if !c.isOwner(by) || !IsTerminal(s) {
			return Task{}, askRefusal(t, "its state follows its outcome")
		}
		note = cmpStr(note, "closed by the person")
		if err := c.write(ctx, by, KindTaskState, taskStateRow{TaskID: id, State: Canceled, Note: note, By: by, At: c.stamp(),
			Outcome: &Outcome{Kind: OutcomeNone, Note: cut(note, MaxOutcome)}}); err != nil {
			return Task{}, err
		}
		return copyTask(t), nil
	}
	if !c.mayManage(t, by) {
		return Task{}, fmt.Errorf("%w: change task %s", ErrNotAllowed, id)
	}
	cascade, err := c.parentStateRefusal(t, s)
	if err != nil {
		return Task{}, err
	}
	if t.State == s && t.Note == note {
		return copyTask(t), nil
	}
	if err := c.write(ctx, by, KindTaskState, taskStateRow{TaskID: id, State: s, Note: note, By: by, At: c.stamp()}); err != nil {
		return Task{}, err
	}
	if cascade {
		// Canceling a parent cancels its subtasks still in play.
		for _, k := range slices.Clone(t.Subtasks) {
			if kt := c.tasks[k]; kt != nil && !IsTerminal(kt.State) {
				if err := c.write(ctx, by, KindTaskState, taskStateRow{TaskID: k, State: Canceled,
					Note: "its parent " + t.ShortRef() + " was canceled", By: by, At: c.stamp()}); err != nil {
					return Task{}, err
				}
			}
		}
	}
	return copyTask(t), nil
}

// ---- leases ----

func (c *coord) acquire(ctx context.Context, holder, resource string, ttl time.Duration, steal bool) (Lease, error) {
	holder = strings.TrimSpace(holder)
	if holder == "" {
		return Lease{}, fmt.Errorf("%w: no holder", ErrInvalid)
	}
	res, err := ParseResource(resource)
	if err != nil {
		return Lease{}, err
	}
	if ttl <= 0 {
		ttl = DefaultTTL
	}
	ttl = min(ttl, MaxTTL)

	c.mu.Lock()
	defer c.unlock()
	now := c.stamp()
	c.prune(now)
	var victims []*leaseRec
	for _, l := range c.held {
		if l.Holder != holder && c.live(l, now) && conflicts(l.res, res) {
			victims = append(victims, l)
		}
	}
	sort.Slice(victims, func(i, j int) bool { return victims[i].Resource < victims[j].Resource })
	if len(victims) > 0 && !steal {
		v := victims[0]
		held := fmt.Errorf("%w: %s is leased by %s until %s (pass steal to take it)",
			ErrHeld, v.Resource, v.Holder, v.Until.Format(time.RFC3339))
		if err := c.refuse(ctx, holder, res, victims, now); err != nil {
			return Lease{}, fmt.Errorf("%w (logging the refusal failed: %v)", held, err)
		}
		return Lease{}, held
	}
	var from []string
	var stolen []stolenLease
	for _, v := range victims {
		stolen = append(stolen, stolenLease{Resource: v.Resource, Holder: v.Holder})
		if !contains(from, v.Holder) {
			from = append(from, v.Holder)
		}
	}
	sort.Strings(from)

	l := Lease{Resource: res.Name, Holder: holder, Until: now.Add(ttl), AcquiredAt: now, StolenFrom: strings.Join(from, ",")}
	if old := c.held[res.Name]; old != nil && old.Holder == holder && c.live(old, now) {
		// A renewal keeps when it was first taken and from whom.
		l.AcquiredAt = old.AcquiredAt
		if l.StolenFrom == "" {
			l.StolenFrom = old.StolenFrom
		}
	}
	// One row: the fold drops the stolen leases as it takes this one.
	if err := c.write(ctx, holder, KindLeaseAcq, leaseAcquiredRow{Lease: l, Stolen: stolen}); err != nil {
		return Lease{}, err
	}
	return c.held[res.Name].Lease, nil
}

func contains(xs []string, x string) bool {
	for _, v := range xs {
		if v == x {
			return true
		}
	}
	return false
}

func (c *coord) release(ctx context.Context, by, resource string) error {
	by = strings.TrimSpace(by)
	if by == "" {
		return fmt.Errorf("%w: no caller", ErrInvalid)
	}
	name, err := NormalizeResource(resource)
	if err != nil {
		return err
	}
	c.mu.Lock()
	defer c.unlock()
	now := c.stamp()
	l := c.held[name]
	if l == nil || !c.live(l, now) {
		return fmt.Errorf("%w: no lease on %s", ErrNotFound, name)
	}
	if l.Holder != by && !c.isOwner(by) {
		return fmt.Errorf("%w: %s is leased by %s", ErrHeld, name, l.Holder)
	}
	if !c.localKey(l.Holder) {
		// Only its holder's device releases a lease; the person ends
		// another device's with lease.stolen (v0.4 §6.4).
		return c.steal(ctx, by, l)
	}
	return c.write(ctx, by, KindLeaseRel, leaseReleasedRow{
		Resource: name, Holder: l.Holder, Reason: ReasonReleased, By: by, At: now,
	})
}

func (c *coord) leases() []Lease {
	c.mu.Lock()
	defer c.unlock()
	now := c.stamp()
	c.prune(now)
	out := []Lease{}
	for _, l := range c.held {
		if c.live(l, now) {
			out = append(out, l.Lease)
		}
	}
	for _, l := range c.conflicts {
		out = append(out, l.Lease) // contested: both show, marked (§6.4)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Resource < out[j].Resource })
	return out
}

func (c *coord) sweep(ctx context.Context) error {
	_, err := c.sweepNotices(ctx)
	return err
}

// releaseGone releases the leases of holders gone for the grace period. The
// caller holds c.mu.
func (c *coord) releaseGone(ctx context.Context, now time.Time) error {
	c.prune(now)
	c.observe(now)
	var gone []*leaseRec
	for _, l := range c.held {
		if l.Until.After(now) && c.localKey(l.Holder) && c.gone(l.Holder, now) {
			gone = append(gone, l)
		}
	}
	sort.Slice(gone, func(i, j int) bool { return gone[i].Resource < gone[j].Resource })
	for _, l := range gone {
		if err := c.write(ctx, "irori", KindLeaseRel, leaseReleasedRow{
			Resource: l.Resource, Holder: l.Holder, Reason: ReasonHolderGone, By: "irori", At: now,
		}); err != nil {
			return err
		}
	}
	return nil
}

// ignore counts one row of dev ignored by the writer rules (§6.0). c.mu is
// held (or replay runs).
func (c *coord) ignore(dev string) {
	c.ignored++
	if c.ignoredBy == nil {
		c.ignoredBy = map[string]int{}
	}
	c.ignoredBy[dev]++
}

// IgnoredBy counts the ignored rows per writing device, for Devices' warning
// line (§8, §9).
func (c *Coord) IgnoredBy() map[string]int {
	c.impl.mu.Lock()
	defer c.impl.mu.Unlock()
	out := make(map[string]int, len(c.impl.ignoredBy))
	for d, n := range c.impl.ignoredBy {
		out[d] = n
	}
	return out
}
