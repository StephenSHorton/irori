// Package coord is tasks and leases (docs/design/v0.2-talk.md §6; port notes
// in docs/research/v02/suzuri.md §6, §7 and §12). Like the inbox, state is a
// fold over the event log, rebuilt at New; every mutation appends first.
//
// CONTRACT (implemented by the coord work package; keep these signatures):
package coord

import (
	"context"
	"errors"
	"time"

	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

var (
	ErrNotFound      = errors.New("coord: no such task or lease")
	ErrHeld          = errors.New("coord: held by someone else")
	ErrAmbiguous     = errors.New("coord: that short ref names more than one card")
	ErrBadResource   = errors.New("coord: resource must be path:<repoKey>:<glob> or a name like gpu/qwen")
	ErrBadState      = errors.New("coord: not a task state")
	ErrUnimplemented = errors.New("coord: not implemented yet")
)

const (
	DefaultTTL = 30 * time.Minute
	MaxTTL     = 4 * time.Hour
	GoneGrace  = 3 * time.Minute // holder gone this long releases its leases
	// HolderGrace: a holder gone this long is taken off its card, and a
	// card nobody is left on goes back to open (card #16). Longer than
	// GoneGrace, so a session that restarts keeps its cards.
	HolderGrace = 10 * time.Minute
)

// TaskState is an A2A TaskState subset, plus Blocked (decision 17).
type TaskState string

const (
	Submitted     TaskState = "submitted"
	Working       TaskState = "working"
	InputRequired TaskState = "input_required" // waiting on a person's answer
	Completed     TaskState = "completed"
	Failed        TaskState = "failed"
	Canceled      TaskState = "canceled"
	// Blocked: something is in the way (Task.Reason, Task.BlockedBy). Entered
	// only through Block or NewTask.BlockReason (SetTaskState refuses it, so
	// a blocked task always has a reason) and left through Unblock or a later
	// SetTaskState. Not terminal.
	Blocked TaskState = "blocked"
)

// Task is the folded view of one task.
type Task struct {
	ID string `json:"id"`
	// Short is the card's short ref without its "#" ("12", or "12a" and
	// "12b" for two cards that claimed 12 apart: short.go). It is derived
	// by the fold, never written. ShortWas is the number such a card
	// claimed ("12") and showed until the clash settled; "" when no other
	// card claimed it.
	Short        string    `json:"short,omitempty"`
	ShortWas     string    `json:"shortWas,omitempty"`
	Title        string    `json:"title"`
	Objective    string    `json:"objective"`
	DoneCriteria string    `json:"doneCriteria,omitempty"`
	RepoKey      string    `json:"repoKey,omitempty"`
	Scope        []string  `json:"scope,omitempty"` // path globs
	State        TaskState `json:"state"`
	// Holder is Holders[0], or "". It is kept so v0.2 readers (observe,
	// tools, the GUI) keep working, and is derived: the fold maintains
	// Holders and copies the first one here.
	Holder string `json:"holder,omitempty"`
	// Holders are who work it: agent keys, or a person's "user:*" key when
	// it was assigned to a person. A task that isn't Shared has at most one;
	// a Shared task has any number and any of them may work it (decision
	// 20). Join order, no duplicates. The fold, observe (gone tracking),
	// TaskFilter.Holder and every permission check iterate Holders; Holder
	// is only for readers.
	Holders []string `json:"holders,omitempty"`
	// Assigned are the Holders that were assigned and haven't taken the
	// task up yet (claimed, joined, or posted its state). An assignee is
	// told by a notice and sees the task in its NeedsYou until then; Holders
	// minus Assigned are who actually work it.
	Assigned  []string  `json:"assigned,omitempty"`
	Shared    bool      `json:"shared,omitempty"`
	CreatedBy string    `json:"createdBy"`
	Note      string    `json:"note,omitempty"`
	CreatedAt time.Time `json:"createdAt"`
	UpdatedAt time.Time `json:"updatedAt"`
	// UpdatedBy is who wrote the row that last changed the task's state,
	// holders, sharing or links (an agent key or "user:*"): the actor of
	// the latest move. Comments don't set it.
	UpdatedBy string `json:"updatedBy,omitempty"`
	// Reason and BlockedBy are set while State is Blocked and cleared when
	// it leaves Blocked. Reason is the first open blocker item's reason (its
	// question, or the text Block was given), kept for v0.2 readers;
	// BlockedBy is the On of the first open item that has one.
	Reason    string     `json:"reason,omitempty"`
	BlockedBy *BlockedBy `json:"blockedBy,omitempty"`
	Links     []Link     `json:"links,omitempty"`    // board.md "Links out"; <= MaxLinks
	Comments  int        `json:"comments,omitempty"` // how many; read them with Comments
	// Cards (cards.go). Steps in order; Blockers are the open items and
	// the newest KeepResolved resolved ones, in the order raised; Watchers
	// are agent keys, in the order they started watching.
	Steps    []Step    `json:"steps,omitempty"`
	Blockers []Blocker `json:"blockers,omitempty"`
	Watchers []string  `json:"watchers,omitempty"`
	// Ask is set on an ask (ask.go): the person's prompt to the room, its
	// take and, once closed, its outcome.
	Ask *Ask `json:"ask,omitempty"`
	// Returned is set while nobody is on a card because its last holder
	// went away and Irori took it off (card #16); it clears when anyone is
	// on it again.
	Returned *Returned `json:"returned,omitempty"`
	// Subtasks (subtasks.go), all derived by the fold: Parent is the task
	// id this card is a subtask of; Subtasks are a parent's, by short ref;
	// Rollup is a parent's progress and state from them (EffectiveState).
	Parent   string   `json:"parent,omitempty"`
	Subtasks []string `json:"subtasks,omitempty"`
	Rollup   *Rollup  `json:"rollup,omitempty"`
}

// Returned says who went away from a card and who was told.
type Returned struct {
	Was string    `json:"was"` // the holder Irori took off
	At  time.Time `json:"at"`
	// Row names the task.left row that returned it ("<device>:<seq>"),
	// unique where At may not be: two rows in one clock step share a time
	// (card #25). The watcher keys its card on it.
	Row string `json:"row,omitempty"`
	// Person: the card came back to the person (its creator was the person
	// or had gone too), so every desktop device shows a notification.
	// Otherwise its creator agent was told.
	Person bool `json:"person,omitempty"`
}

// NewTask is CreateTask's input.
type NewTask struct {
	Title, Objective, DoneCriteria, RepoKey string
	Scope                                   []string
	Shared                                  bool
	Links                                   []string // each one parsed by ParseLink
	// BlockReason, when set, creates the task already Blocked (decision 17:
	// a blocker with no task behind it, such as a port held by a dead dev
	// server, becomes a small blocked task). BlockedBy is optional.
	BlockReason string
	BlockedBy   *BlockedBy
	// Steps, when given, are the task's first step list (checked like
	// SetSteps; ids are minted s1, s2, ... and any given are ignored),
	// written as a task.steps row right after task.created.
	Steps []Step
	// Parent, when given (a short ref or task id), makes the task a
	// subtask of that card (subtasks.go).
	Parent string
}

// TaskFilter narrows ListTasks; zero values mean any.
type TaskFilter struct {
	RepoKey string
	Holder  string // matches any of Holders
	Open    bool   // only non-terminal states
}

// Lease is an advisory claim on a resource.
type Lease struct {
	Resource   string    `json:"resource"`
	Holder     string    `json:"holder"`
	Until      time.Time `json:"until"`
	StolenFrom string    `json:"stolenFrom,omitempty"`
	AcquiredAt time.Time `json:"acquiredAt"`
	// Conflict names the other holder when two devices took this resource
	// while partitioned (v0.4 §6.4): the lower device id's lease stands.
	Conflict string `json:"conflict,omitempty"`
}

// ChangeKind classifies a Subscribe notification.
type ChangeKind string

const (
	TaskChanged  ChangeKind = "task"          // created, claimed, assigned, joined, left, shared, linked, blocked, unblocked, a state change, steps, blocker items or watchers
	LeaseChanged ChangeKind = "lease"         // acquired, renewed, stolen or released
	LeaseExpired ChangeKind = "lease_expired" // a lease ran out
	LeaseRefused ChangeKind = "lease_refused" // a lease.refused row; Resource and Holder (the refused caller) set
	CommentAdded ChangeKind = "comment"       // a task.comment row; TaskID set
	Resync       ChangeKind = "resync"        // changes were dropped: re-read everything
)

// Change is a notification for transports and the UI. Reason is the release
// reason (ReasonReleased, ReasonHolderGone) on a LeaseChanged release.
type Change struct {
	Kind     ChangeKind `json:"kind"`
	TaskID   string     `json:"taskId,omitempty"`
	Resource string     `json:"resource,omitempty"`
	Holder   string     `json:"holder,omitempty"`
	Reason   string     `json:"reason,omitempty"`
}

// Options configure New.
type Options struct {
	Now         func() time.Time
	GoneGrace   time.Duration // test override
	HolderGrace time.Duration // test override
	// IsOwner reports whether a "user:<device>" key is the person's (v0.4
	// identity.Chain.IsOwner). Nil accepts exactly "user:<store device>".
	IsOwner func(key string) bool
	// Revoked reports a removed device (v0.4 §3, Revoke): its leases end
	// everywhere without a row from it. Nil revokes nobody.
	Revoked func(device string) bool
}

// Coord holds tasks and leases. Safe for concurrent use.
type Coord struct {
	impl *coord
}

// New replays the log.
func New(ctx context.Context, st *store.Store, roster interface{ Agents() []presence.AgentCard }, opts Options) (*Coord, error) {
	return newCoord(ctx, st, roster, opts)
}

func (c *Coord) CreateTask(ctx context.Context, by string, t NewTask) (Task, error) {
	return c.impl.createTask(ctx, by, t)
}
func (c *Coord) ListTasks(f TaskFilter) []Task  { return c.impl.listTasks(f) }
func (c *Coord) GetTask(id string) (Task, bool) { return c.impl.getTask(id) }

// ClaimTask makes by the holder and moves a submitted task to working (also
// when by was assigned it). Fails with ErrHeld if another live holder has it;
// a holder counts as gone after GoneGrace absent from the roster (see Sweep).
func (c *Coord) ClaimTask(ctx context.Context, by, id string) (Task, error) {
	return c.impl.claimTask(ctx, by, id)
}

// AssignTask sets the holder to `to` (by may be the creator, the holder, or a person).
// It is the v0.2 form; new callers use Assign (board.go), which also widens
// who may assign and returns the notice to route.
func (c *Coord) AssignTask(ctx context.Context, by, id, to string) (Task, error) {
	return c.impl.assignTask(ctx, by, id, to)
}

// SetTaskState: only the holder, the creator or a person ("user:*") may (on
// a Shared task, any holder). Blocked is refused with ErrBadState ("use
// Block"). Moving a blocked task to another state this way also clears its
// Reason and BlockedBy. InputRequired is still accepted here (older rows
// and tests use it), but no caller in Irori sets it: agents reach it only
// through board_status, which refuses it without open blockers and leaves a
// card that has them as it is (v0.4.2, one blocker per ask).
func (c *Coord) SetTaskState(ctx context.Context, by, id string, s TaskState, note string) (Task, error) {
	return c.impl.setTaskState(ctx, by, id, s, note)
}

// Reopen moves a finished card (completed, failed or canceled) back into
// play: working when a holder has taken it up, else submitted. Its holders
// stay. Its creator, a holder or a person may. A finished parent comes back
// with a reopened subtask; a parent whose subtasks have all ended is
// refused (its state follows them: reopen a subtask instead). The notice
// is NoticeReopened to the holders other than by (with none, its creator if
// an agent).
func (c *Coord) Reopen(ctx context.Context, by, id string) (Task, Notice, error) {
	return c.impl.reopen(ctx, by, id, ReopenNote)
}

// Acquire takes or renews resource for holder. ttl 0 = DefaultTTL, capped at
// MaxTTL. An unexpired lease held by another (for a path resource: any
// overlapping glob in the same repo) fails with ErrHeld unless steal. A
// refusal first appends a lease.refused row (Refusal, board.go), at most one
// per (resource, caller, holder) while that refusal is active.
func (c *Coord) Acquire(ctx context.Context, holder, resource string, ttl time.Duration, steal bool) (Lease, error) {
	return c.impl.acquire(ctx, holder, resource, ttl, steal)
}

// Release drops holder's lease (a person may release anyone's).
func (c *Coord) Release(ctx context.Context, by, resource string) error {
	return c.impl.release(ctx, by, resource)
}

// Leases lists unexpired leases.
func (c *Coord) Leases() []Lease { return c.impl.leases() }

// Sweep releases leases whose holder has been gone for GoneGrace, and closes
// blockers that resolved by themselves; it is SweepNotices with the notices
// dropped, so Core should call SweepNotices and route them. Core calls it
// with the inbox sweep, and must call it every few seconds: a holder that left
// the roster without a "gone" card starts its grace clock only when Sweep (or
// ClaimTask on its task) first sees it missing.
func (c *Coord) Sweep(ctx context.Context) error { return c.impl.sweep(ctx) }

// Subscribe delivers a Change after every task or lease mutation, including
// the sweeper's releases and a lease running out (noticed by a timer armed
// for the next expiry while anyone is subscribed), until ctx ends; then the
// channel closes. It is buffered: a slow reader drops changes and gets one
// Resync once it has room, so readers should re-read state on any change.
func (c *Coord) Subscribe(ctx context.Context) <-chan Change { return c.impl.subscribe(ctx) }

// Overlaps reports whether two path globs can match a common path.
func Overlaps(a, b string) bool { return overlaps(a, b) }
