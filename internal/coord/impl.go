package coord

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sort"
	"strings"
	"sync"
	"time"
	"unicode/utf8"

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

// IsPerson reports whether key names a person ("user:<deviceId>").
func IsPerson(key string) bool { return strings.HasPrefix(key, "user:") }

// Row payloads. Every row carries "at" (this device's clock when it was
// written) so a replay folds to the same times.
type taskCreatedRow struct {
	TaskID       string    `json:"taskId"`
	Title        string    `json:"title"`
	Objective    string    `json:"objective"`
	DoneCriteria string    `json:"doneCriteria,omitempty"`
	RepoKey      string    `json:"repoKey,omitempty"`
	Scope        []string  `json:"scope,omitempty"`
	Shared       bool      `json:"shared,omitempty"`
	Links        []Link    `json:"links,omitempty"`
	By           string    `json:"by"`
	At           time.Time `json:"at"`
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
	By        string     `json:"by"`
	At        time.Time  `json:"at"`
}

// taskMemberRow is task.joined and task.left.
type taskMemberRow struct {
	TaskID string    `json:"taskId"`
	Who    string    `json:"who"`
	By     string    `json:"by"`
	At     time.Time `json:"at"`
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
	st     *store.Store
	roster interface{ Agents() []presence.AgentCard }
	now    func() time.Time
	grace  time.Duration

	mu          sync.Mutex
	tasks       map[string]*Task
	order       map[string]uint64 // task id -> fold position of its task.created row
	nextOrder   uint64
	held        map[string]*leaseRec // by canonical resource
	firstAbsent map[string]time.Time // holder key -> first seen missing from the roster
	comments    map[string]*Comment  // by comment id
	byTask      map[string][]string  // task id -> comment ids, fold order
	refusals    []*refusalRec        // possibly active, fold order (gcRefusals drops the rest)

	// Subscribe (subscribe.go). Changes queue in pending while c.mu is held
	// and go out in unlock; replay queues none.
	subs      map[chan Change]*subscriber
	pending   []Change
	replaying bool
	expiry    *time.Timer // fires at expiryAt, the next lease's Until
	expiryAt  time.Time
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
		tasks:       map[string]*Task{},
		order:       map[string]uint64{},
		held:        map[string]*leaseRec{},
		firstAbsent: map[string]time.Time{},
		comments:    map[string]*Comment{},
		byTask:      map[string][]string{},
		subs:        map[chan Change]*subscriber{},
	}
	if c.now == nil {
		c.now = time.Now
	}
	if c.grace <= 0 {
		c.grace = GoneGrace
	}
	if err := c.replay(ctx); err != nil {
		return nil, err
	}
	return &Coord{impl: c}, nil
}

// replay folds every task./lease. row in HLC order. AfterHLC is strictly
// after, so each next page starts one tick early and skips rows already
// applied at the boundary HLC; that keeps rows from two devices that share an
// HLC (possible once rows replicate) from being skipped.
func (c *coord) replay(ctx context.Context) error {
	f := store.Filter{Kinds: []string{"task.", "lease.", "blocker."}, Limit: store.MaxListLimit}
	type rowID struct {
		dev string
		seq int64
	}
	seen := map[rowID]bool{} // rows at the previous page's last HLC
	c.replaying = true
	defer func() { c.replaying = false }()
	for {
		evs, err := c.st.List(ctx, f)
		if err != nil {
			return fmt.Errorf("coord: replay: %w", err)
		}
		progressed := false
		for _, ev := range evs {
			if seen[rowID{ev.DeviceID, ev.Seq}] {
				continue
			}
			progressed = true
			c.apply(ev)
		}
		c.prune(c.stamp())
		if len(evs) < f.Limit || !progressed {
			return nil
		}
		last := evs[len(evs)-1].HLC
		clear(seen)
		for _, ev := range evs {
			if ev.HLC == last {
				seen[rowID{ev.DeviceID, ev.Seq}] = true
			}
		}
		f.AfterHLC = last - 1
	}
}

// apply folds one row. Rows are applied as written; validation happened
// before the append.
func (c *coord) apply(ev store.Event) {
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
			CreatedAt: t, UpdatedAt: t, Shared: p.Shared, Links: links,
		}
		c.note(Change{Kind: TaskChanged, TaskID: p.TaskID})
	case KindTaskClaimed:
		var p taskClaimedRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		if t := c.tasks[p.TaskID]; t != nil && p.By != "" {
			addHolder(t, p.By)
			if t.State == Submitted {
				t.State = Working
			}
			t.UpdatedAt = at(p.At)
			c.note(Change{Kind: TaskChanged, TaskID: p.TaskID, Holder: p.By})
		}
	case KindTaskAssigned:
		var p taskAssignedRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		if t := c.tasks[p.TaskID]; t != nil && p.To != "" {
			addHolder(t, p.To)
			t.UpdatedAt = at(p.At)
			c.note(Change{Kind: TaskChanged, TaskID: p.TaskID, Holder: p.To})
		}
	case KindTaskState:
		var p taskStateRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		if t := c.tasks[p.TaskID]; t != nil && ValidState(p.State) {
			t.State = p.State
			t.Note = p.Note
			t.Reason, t.BlockedBy = "", nil
			if p.State == Blocked {
				t.Reason = p.Reason
				if p.BlockedBy != nil {
					b := *p.BlockedBy
					t.BlockedBy = &b
				}
			}
			t.UpdatedAt = at(p.At)
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
		if ev.Kind == KindTaskJoined {
			if !contains(t.Holders, p.Who) {
				t.Holders = append(t.Holders, p.Who)
			}
			if t.State == Submitted {
				t.State = Working
			}
		} else {
			t.Holders = removeStr(t.Holders, p.Who)
		}
		syncHolder(t)
		t.UpdatedAt = at(p.At)
		c.note(Change{Kind: TaskChanged, TaskID: p.TaskID, Holder: p.Who})
	case KindTaskShared:
		var p taskSharedRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		if t := c.tasks[p.TaskID]; t != nil {
			t.Shared = p.Shared
			t.UpdatedAt = at(p.At)
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
		t.UpdatedAt = at(p.At)
		c.note(Change{Kind: TaskChanged, TaskID: p.TaskID, Holder: t.Holder})
	case KindTaskComment:
		var p taskCommentRow
		if json.Unmarshal(ev.Payload, &p) != nil || p.CommentID == "" {
			return
		}
		t := c.tasks[p.TaskID]
		if t == nil || c.comments[p.CommentID] != nil {
			return
		}
		cm := &Comment{ID: p.CommentID, TaskID: p.TaskID, By: p.By, Text: p.Text, Wake: p.Wake, At: at(p.At)}
		c.comments[cm.ID] = cm
		c.byTask[t.ID] = append(c.byTask[t.ID], cm.ID)
		t.Comments++
		// A comment is activity on the card: it floats up its Board group.
		t.UpdatedAt = cm.At
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
		t.UpdatedAt = at(p.At)
		c.note(Change{Kind: TaskChanged, TaskID: p.TaskID, Holder: t.Holder})
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
		c.held[res.Name] = &leaseRec{Lease: p, res: res}
		c.note(Change{Kind: LeaseChanged, Resource: res.Name, Holder: p.Holder})
	case KindLeaseRel:
		var p leaseReleasedRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		if c.dropLease(p.Resource, p.Holder) {
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
func (c *coord) prune(now time.Time) {
	for name, l := range c.held {
		if !l.Until.After(now) {
			delete(c.held, name)
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

// syncHolder keeps Holder = Holders[0].
func syncHolder(t *Task) {
	if len(t.Holders) == 0 {
		t.Holders, t.Holder = nil, ""
		return
	}
	t.Holder = t.Holders[0]
}

// isHolder reports whether key is one of t's holders.
func isHolder(t *Task, key string) bool { return contains(t.Holders, key) }

// mayManage: the creator, any holder or a person.
func mayManage(t *Task, by string) bool {
	return by == t.CreatedBy || isHolder(t, by) || IsPerson(by)
}

// restoreState is where Unblock leaves a task: Working with holders, else
// Submitted.
func restoreState(t *Task) TaskState {
	if len(t.Holders) > 0 {
		return Working
	}
	return Submitted
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

// write appends a row and folds it. The caller holds c.mu.
func (c *coord) write(ctx context.Context, by, kind string, payload any) error {
	ev, err := c.st.Append(ctx, actorFor(by), kind, payload)
	if err != nil {
		return fmt.Errorf("coord: %w", err)
	}
	c.apply(ev)
	return nil
}

// actorFor maps a caller to the store's actor forms.
func actorFor(by string) string {
	switch {
	case by == "irori":
		return "irori"
	case IsPerson(by):
		return "user"
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
	if t.Links != nil {
		out.Links = append([]Link(nil), t.Links...)
	}
	if t.BlockedBy != nil {
		b := *t.BlockedBy
		out.BlockedBy = &b
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
		if l.Until.After(now) {
			tracked[l.Holder] = true
		}
	}
	for k := range c.firstAbsent {
		if !tracked[k] {
			delete(c.firstAbsent, k)
		}
	}
	for k := range tracked {
		if IsPerson(k) || live[k] {
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
	by = strings.TrimSpace(by)
	if by == "" {
		return Task{}, fmt.Errorf("%w: no caller", ErrInvalid)
	}
	title := strings.TrimSpace(nt.Title)
	if title == "" {
		return Task{}, fmt.Errorf("%w: a task needs a title", ErrInvalid)
	}
	if utf8.RuneCountInString(title) > maxTitle {
		return Task{}, fmt.Errorf("%w: title longer than %d characters", ErrInvalid, maxTitle)
	}
	obj := strings.TrimSpace(nt.Objective)
	done := strings.TrimSpace(nt.DoneCriteria)
	if utf8.RuneCountInString(obj) > maxObjective || utf8.RuneCountInString(done) > maxObjective {
		return Task{}, fmt.Errorf("%w: objective or done criteria longer than %d characters", ErrInvalid, maxObjective)
	}
	repoKey := strings.TrimSpace(nt.RepoKey)
	if repoKey != "" {
		if err := checkRepoKey(repoKey); err != nil {
			return Task{}, err
		}
	}
	var scope []string
	for _, g := range nt.Scope {
		if strings.TrimSpace(g) == "" {
			continue
		}
		n, err := NormalizeGlob(g)
		if err != nil {
			return Task{}, err
		}
		scope = append(scope, n)
	}
	if len(scope) > maxScope {
		return Task{}, fmt.Errorf("%w: more than %d scope globs", ErrInvalid, maxScope)
	}
	var links []Link
	for _, s := range nt.Links {
		if strings.TrimSpace(s) == "" {
			continue
		}
		l, err := ParseLink(s)
		if err != nil {
			return Task{}, err
		}
		if linkIndex(links, l.Ref) < 0 {
			links = append(links, l)
		}
	}
	if len(links) > MaxLinks {
		return Task{}, fmt.Errorf("%w: more than %d links", ErrInvalid, MaxLinks)
	}
	reason := strings.TrimSpace(nt.BlockReason)
	if reason == "" && nt.BlockedBy != nil {
		return Task{}, fmt.Errorf("%w: BlockedBy needs a BlockReason", ErrInvalid)
	}
	if err := checkReason(reason, false); err != nil {
		return Task{}, err
	}

	c.mu.Lock()
	defer c.unlock()
	var bb *BlockedBy
	if reason != "" {
		var err error
		if bb, err = c.checkBlocker(nt.BlockedBy, ""); err != nil {
			return Task{}, err
		}
	}
	at := c.stamp()
	id := newID(at)
	if err := c.write(ctx, by, KindTaskCreated, taskCreatedRow{
		TaskID: id, Title: title, Objective: obj, DoneCriteria: done,
		RepoKey: repoKey, Scope: scope, Shared: nt.Shared, Links: links, By: by, At: at,
	}); err != nil {
		return Task{}, err
	}
	if reason != "" {
		if err := c.write(ctx, by, KindTaskState, taskStateRow{
			TaskID: id, State: Blocked, Reason: reason, BlockedBy: bb, By: by, At: at,
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
			f.Open && IsTerminal(t.State) {
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
	// An assignee claiming a still-submitted task writes the claimed row,
	// which moves it to working.
	if isHolder(t, by) && t.State != Submitted {
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
	c.mu.Lock()
	defer c.unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, false, err
	}
	if !mayManage(t, by) && !(wide && len(t.Holders) == 0) {
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
	if !mayManage(t, by) {
		return Task{}, fmt.Errorf("%w: change task %s", ErrNotAllowed, id)
	}
	if t.State == s && t.Note == note {
		return copyTask(t), nil
	}
	if err := c.write(ctx, by, KindTaskState, taskStateRow{TaskID: id, State: s, Note: note, By: by, At: c.stamp()}); err != nil {
		return Task{}, err
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
		if l.Holder != holder && l.Until.After(now) && conflicts(l.res, res) {
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
	if old := c.held[res.Name]; old != nil && old.Holder == holder && old.Until.After(now) {
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
	if l == nil || !l.Until.After(now) {
		return fmt.Errorf("%w: no lease on %s", ErrNotFound, name)
	}
	if l.Holder != by && !IsPerson(by) {
		return fmt.Errorf("%w: %s is leased by %s", ErrHeld, name, l.Holder)
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
		if l.Until.After(now) {
			out = append(out, l.Lease)
		}
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
		if l.Until.After(now) && c.gone(l.Holder, now) {
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
