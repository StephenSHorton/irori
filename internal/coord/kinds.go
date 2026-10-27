package coord

// Log kinds. Payloads only ever gain fields.
const (
	KindTaskCreated  = "task.created"   // Task fields + {by}
	KindTaskClaimed  = "task.claimed"   // {taskId, by}
	KindTaskAssigned = "task.assigned"  // {taskId, to, by}
	KindTaskState    = "task.status"    // {taskId, state, note, by}
	KindLeaseAcq     = "lease.acquired" // Lease
	KindLeaseRel     = "lease.released" // {resource, holder, reason, by}
)
