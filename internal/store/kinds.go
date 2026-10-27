package store

// Event kinds written in v0.1. Payloads are JSON objects; add fields, never
// repurpose them, because old rows live forever.
const (
	// KindDeviceStarted: {"name","os","version"} once per process start.
	KindDeviceStarted = "device.started"
	// KindAgentSeen: a presence.AgentCard the first time a session is seen.
	KindAgentSeen = "agent.seen"
	// KindAgentStatus: {"key","status","name"} when status or name changes.
	KindAgentStatus = "agent.status"
	// KindAgentGone: {"key","reason"} when a session's file disappears or its pid dies.
	KindAgentGone = "agent.gone"
)
