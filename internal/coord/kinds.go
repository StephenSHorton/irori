package coord

// Log kinds. Payloads only ever gain fields. Every row carries "at".
const (
	// Task fields + {by}; the Board adds {shared?, links?: []Link}. A task
	// created blocked writes task.created then task.status (state blocked)
	// under one lock.
	KindTaskCreated = "task.created"
	// {taskId, by}. Not shared: by replaces Holders. Shared: by is appended
	// to Holders (claiming a shared task is joining it).
	KindTaskClaimed = "task.claimed"
	// {taskId, to, by}. Not shared: to replaces Holders. Shared: to is
	// appended. A submitted task stays submitted until the assignee claims.
	KindTaskAssigned = "task.assigned"
	// {taskId, state, note, by}; the Board adds {reason?, blockedBy?} when
	// state is "blocked". Leaving blocked clears reason and blockedBy.
	KindTaskState = "task.status"
	KindLeaseAcq  = "lease.acquired" // Lease
	KindLeaseRel  = "lease.released" // {resource, holder, reason, by}

	// The Board (docs/design/board.md "Contracts").
	KindTaskJoined  = "task.joined"  // {taskId, who, by}; shared tasks only
	KindTaskLeft    = "task.left"    // {taskId, who, by}; removes who from Holders
	KindTaskShared  = "task.shared"  // {taskId, shared, by}
	KindTaskLinked  = "task.linked"  // {taskId, link: Link, removed?, by}; removed drops the link with that ref
	KindTaskComment = "task.comment" // {taskId, commentId, by, text, wake}
	// KindBlockerClosed is Unblock: {taskId, by, note, restored: TaskState}.
	// It is its own row so "anyone may close a blocker" (decision 19) never
	// loosens task.status's permission check. by "irori" when SweepNotices
	// closed a blocker that resolved by itself.
	KindBlockerClosed = "blocker.closed"
	// KindLeaseRefused is a refused Acquire: Refusal.
	KindLeaseRefused = "lease.refused"
)
