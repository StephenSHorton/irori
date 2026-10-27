package inbox

// Log kinds the inbox writes and replays. Payloads only ever gain fields.
// Every payload carries "at" (RFC 3339, the inbox clock); readers fall back
// to the row's HLC wall time when it is missing.
const (
	KindMessageSent = "message.sent" // Message
	// KindDelivery is a full snapshot of one delivery's mutable fields, so
	// the current state is the newest row per deliveryId:
	// {deliveryId, messageId, to, repoKey?, state, hold?, via?, wakeId?, note?,
	// until?, movedTo?, movedFrom?, reminded?, engagedOnce?, by, at}.
	// movedFrom names the delivery a move replaced; engagedOnce is set once
	// the recipient engaged or deferred it.
	KindDelivery    = "delivery.state"
	KindWakeIssued  = "wake.issued"  // {wakeId, to, token, reminder, deliveryIds, path?, at}
	KindAlias       = "agent.alias"  // {key, alias, at}
	KindPolicySet   = "policy.set"   // Policy + {by, at}
	KindTurnEngaged = "turn.engaged" // {agent, wakeId, turnId, at}
	KindTurnEnded   = "turn.ended"   // {agent, wakeId, turnId, at}
)
