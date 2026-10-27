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

// Release reasons in lease.released rows.
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

// ValidState reports whether s is one of the six task states.
func ValidState(s TaskState) bool {
	switch s {
	case Submitted, Working, InputRequired, Completed, Failed, Canceled:
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
	By     string    `json:"by"`
	At     time.Time `json:"at"`
}

type leaseReleasedRow struct {
	Resource string    `json:"resource"`
	Holder   string    `json:"holder"`
	Reason   string    `json:"reason"`
	By       string    `json:"by"`
	At       time.Time `json:"at"`
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
	held        map[string]*leaseRec // by canonical resource
	firstAbsent map[string]time.Time // holder key -> first seen missing from the roster
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
		held:        map[string]*leaseRec{},
		firstAbsent: map[string]time.Time{},
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
	f := store.Filter{Kinds: []string{"task.", "lease."}, Limit: store.MaxListLimit}
	type rowID struct {
		dev string
		seq int64
	}
	seen := map[rowID]bool{} // rows at the previous page's last HLC
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
		c.tasks[p.TaskID] = &Task{
			ID: p.TaskID, Title: p.Title, Objective: p.Objective, DoneCriteria: p.DoneCriteria,
			RepoKey: p.RepoKey, Scope: p.Scope, State: Submitted, CreatedBy: p.By,
			CreatedAt: t, UpdatedAt: t,
		}
	case KindTaskClaimed:
		var p taskClaimedRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		if t := c.tasks[p.TaskID]; t != nil {
			t.Holder = p.By
			if t.State == Submitted {
				t.State = Working
			}
			t.UpdatedAt = at(p.At)
		}
	case KindTaskAssigned:
		var p taskAssignedRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		if t := c.tasks[p.TaskID]; t != nil {
			t.Holder = p.To
			t.UpdatedAt = at(p.At)
		}
	case KindTaskState:
		var p taskStateRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		if t := c.tasks[p.TaskID]; t != nil && ValidState(p.State) {
			t.State = p.State
			t.Note = p.Note
			t.UpdatedAt = at(p.At)
		}
	case KindLeaseAcq:
		var p Lease
		if json.Unmarshal(ev.Payload, &p) != nil || p.Resource == "" {
			return
		}
		res, err := ParseResource(p.Resource)
		if err != nil {
			res = Resource{Kind: ResourceNamed, Name: p.Resource}
		}
		p.AcquiredAt = at(p.AcquiredAt)
		c.held[res.Name] = &leaseRec{Lease: p, res: res}
	case KindLeaseRel:
		var p leaseReleasedRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		name := p.Resource
		if n, err := NormalizeResource(p.Resource); err == nil {
			name = n
		}
		if l := c.held[name]; l != nil && l.Holder == p.Holder {
			delete(c.held, name)
		}
	}
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
	return out
}

// ---- gone tracking ----

// observe refreshes first-absent times for every holder that matters. A
// holder is absent when the roster has no card for it or its card is "gone".
// People are never absent; a nil roster means nobody is.
func (c *coord) observe(now time.Time) {
	if c.roster == nil {
		return
	}
	live := map[string]bool{}
	for _, a := range c.roster.Agents() {
		if a.Status != "gone" {
			live[a.Key] = true
		}
	}
	tracked := map[string]bool{}
	for _, t := range c.tasks {
		if t.Holder != "" && !IsTerminal(t.State) {
			tracked[t.Holder] = true
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
		} else if _, ok := c.firstAbsent[k]; !ok {
			c.firstAbsent[k] = now
		}
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

	c.mu.Lock()
	defer c.mu.Unlock()
	at := c.stamp()
	id := newID(at)
	if err := c.write(ctx, by, KindTaskCreated, taskCreatedRow{
		TaskID: id, Title: title, Objective: obj, DoneCriteria: done,
		RepoKey: repoKey, Scope: scope, By: by, At: at,
	}); err != nil {
		return Task{}, err
	}
	return copyTask(c.tasks[id]), nil
}

func (c *coord) listTasks(f TaskFilter) []Task {
	c.mu.Lock()
	defer c.mu.Unlock()
	out := []Task{}
	for _, t := range c.tasks {
		if f.RepoKey != "" && t.RepoKey != f.RepoKey ||
			f.Holder != "" && t.Holder != f.Holder ||
			f.Open && IsTerminal(t.State) {
			continue
		}
		out = append(out, copyTask(t))
	}
	sort.Slice(out, func(i, j int) bool {
		if !out[i].CreatedAt.Equal(out[j].CreatedAt) {
			return out[i].CreatedAt.Before(out[j].CreatedAt)
		}
		return out[i].ID < out[j].ID
	})
	return out
}

func (c *coord) getTask(id string) (Task, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
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
	defer c.mu.Unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, err
	}
	if t.Holder == by {
		return copyTask(t), nil
	}
	now := c.stamp()
	if t.Holder != "" {
		c.observe(now)
		if !c.gone(t.Holder, now) {
			return Task{}, fmt.Errorf("%w: task %s is held by %s", ErrHeld, id, t.Holder)
		}
	}
	if err := c.write(ctx, by, KindTaskClaimed, taskClaimedRow{TaskID: id, By: by, At: now}); err != nil {
		return Task{}, err
	}
	return copyTask(t), nil
}

func (c *coord) assignTask(ctx context.Context, by, id, to string) (Task, error) {
	by, to = strings.TrimSpace(by), strings.TrimSpace(to)
	if by == "" || to == "" {
		return Task{}, fmt.Errorf("%w: assign needs a caller and an assignee", ErrInvalid)
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, err
	}
	if by != t.CreatedBy && by != t.Holder && !IsPerson(by) {
		return Task{}, fmt.Errorf("%w: assign task %s", ErrNotAllowed, id)
	}
	if t.Holder == to {
		return copyTask(t), nil
	}
	if err := c.write(ctx, by, KindTaskAssigned, taskAssignedRow{TaskID: id, To: to, By: by, At: c.stamp()}); err != nil {
		return Task{}, err
	}
	return copyTask(t), nil
}

func (c *coord) setTaskState(ctx context.Context, by, id string, s TaskState, note string) (Task, error) {
	by = strings.TrimSpace(by)
	if by == "" {
		return Task{}, fmt.Errorf("%w: no caller", ErrInvalid)
	}
	if !ValidState(s) {
		return Task{}, fmt.Errorf("%w: %q (submitted|working|input_required|completed|failed|canceled)", ErrBadState, s)
	}
	note = strings.TrimSpace(note)
	if utf8.RuneCountInString(note) > maxObjective {
		return Task{}, fmt.Errorf("%w: note longer than %d characters", ErrInvalid, maxObjective)
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, err
	}
	if by != t.CreatedBy && by != t.Holder && !IsPerson(by) {
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
	defer c.mu.Unlock()
	now := c.stamp()
	var victims []*leaseRec
	for _, l := range c.held {
		if l.Holder != holder && l.Until.After(now) && conflicts(l.res, res) {
			victims = append(victims, l)
		}
	}
	sort.Slice(victims, func(i, j int) bool { return victims[i].Resource < victims[j].Resource })
	if len(victims) > 0 && !steal {
		v := victims[0]
		return Lease{}, fmt.Errorf("%w: %s is leased by %s until %s (pass steal to take it)",
			ErrHeld, v.Resource, v.Holder, v.Until.Format(time.RFC3339))
	}
	var from []string
	for _, v := range victims {
		if err := c.write(ctx, holder, KindLeaseRel, leaseReleasedRow{
			Resource: v.Resource, Holder: v.Holder, Reason: ReasonStolen, By: holder, At: now,
		}); err != nil {
			return Lease{}, err
		}
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
	if err := c.write(ctx, holder, KindLeaseAcq, l); err != nil {
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
	defer c.mu.Unlock()
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
	defer c.mu.Unlock()
	now := c.stamp()
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
	c.mu.Lock()
	defer c.mu.Unlock()
	now := c.stamp()
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
