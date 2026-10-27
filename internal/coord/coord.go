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
	ErrBadResource   = errors.New("coord: resource must be path:<repoKey>:<glob> or a name like gpu/qwen")
	ErrBadState      = errors.New("coord: not a task state")
	ErrUnimplemented = errors.New("coord: not implemented yet")
)

const (
	DefaultTTL = 30 * time.Minute
	MaxTTL     = 4 * time.Hour
	GoneGrace  = 3 * time.Minute // holder gone this long releases its leases
)

// TaskState is an A2A TaskState subset.
type TaskState string

const (
	Submitted     TaskState = "submitted"
	Working       TaskState = "working"
	InputRequired TaskState = "input_required"
	Completed     TaskState = "completed"
	Failed        TaskState = "failed"
	Canceled      TaskState = "canceled"
)

// Task is the folded view of one task.
type Task struct {
	ID           string    `json:"id"`
	Title        string    `json:"title"`
	Objective    string    `json:"objective"`
	DoneCriteria string    `json:"doneCriteria,omitempty"`
	RepoKey      string    `json:"repoKey,omitempty"`
	Scope        []string  `json:"scope,omitempty"` // path globs
	State        TaskState `json:"state"`
	Holder       string    `json:"holder,omitempty"` // the single writer (agent key)
	CreatedBy    string    `json:"createdBy"`
	Note         string    `json:"note,omitempty"`
	CreatedAt    time.Time `json:"createdAt"`
	UpdatedAt    time.Time `json:"updatedAt"`
}

// NewTask is CreateTask's input.
type NewTask struct {
	Title, Objective, DoneCriteria, RepoKey string
	Scope                                   []string
}

// TaskFilter narrows ListTasks; zero values mean any.
type TaskFilter struct {
	RepoKey string
	Holder  string
	Open    bool // only non-terminal states
}

// Lease is an advisory claim on a resource.
type Lease struct {
	Resource   string    `json:"resource"`
	Holder     string    `json:"holder"`
	Until      time.Time `json:"until"`
	StolenFrom string    `json:"stolenFrom,omitempty"`
	AcquiredAt time.Time `json:"acquiredAt"`
}

// Options configure New.
type Options struct {
	Now       func() time.Time
	GoneGrace time.Duration // test override
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

// ClaimTask makes by the holder. Fails with ErrHeld if another live holder has it.
func (c *Coord) ClaimTask(ctx context.Context, by, id string) (Task, error) {
	return c.impl.claimTask(ctx, by, id)
}

// AssignTask sets the holder to `to` (by may be the creator, the holder, or a person).
func (c *Coord) AssignTask(ctx context.Context, by, id, to string) (Task, error) {
	return c.impl.assignTask(ctx, by, id, to)
}

// SetTaskState: only the holder, the creator or a person ("user:*") may.
func (c *Coord) SetTaskState(ctx context.Context, by, id string, s TaskState, note string) (Task, error) {
	return c.impl.setTaskState(ctx, by, id, s, note)
}

// Acquire takes or renews resource for holder. ttl 0 = DefaultTTL, capped at
// MaxTTL. An unexpired lease held by another (for a path resource: any
// overlapping glob in the same repo) fails with ErrHeld unless steal.
func (c *Coord) Acquire(ctx context.Context, holder, resource string, ttl time.Duration, steal bool) (Lease, error) {
	return c.impl.acquire(ctx, holder, resource, ttl, steal)
}

// Release drops holder's lease (a person may release anyone's).
func (c *Coord) Release(ctx context.Context, by, resource string) error {
	return c.impl.release(ctx, by, resource)
}

// Leases lists unexpired leases.
func (c *Coord) Leases() []Lease { return c.impl.leases() }

// Sweep releases leases whose holder has been gone for GoneGrace. Core calls it
// with the inbox sweep.
func (c *Coord) Sweep(ctx context.Context) error { return c.impl.sweep(ctx) }

// Overlaps reports whether two path globs can match a common path.
func Overlaps(a, b string) bool { return overlaps(a, b) }
