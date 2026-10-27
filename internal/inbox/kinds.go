package inbox

// Log kinds the inbox writes and replays. Payloads only ever gain fields.
const (
	KindMessageSent = "message.sent"   // Message
	KindDelivery    = "delivery.state" // {deliveryId, messageId, to, repoKey, state, hold?, via?, wakeId?, note?, until?, movedTo?, by}
	KindWakeIssued  = "wake.issued"    // {wakeId, to, token, reminder, deliveryIds}
	KindAlias       = "agent.alias"    // {key, alias}
	KindPolicySet   = "policy.set"     // Policy + {by}
	KindTurnEngaged = "turn.engaged"   // {agent, wakeId, turnId}
	KindTurnEnded   = "turn.ended"     // {agent, wakeId, turnId}
)
