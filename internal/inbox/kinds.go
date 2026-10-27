package inbox

// Log kinds the inbox writes and replays. Payloads only ever gain fields.
// Every payload carries "at" (RFC 3339, the inbox clock); readers fall back
// to the row's HLC wall time when it is missing.
const (
	KindMessageSent = "message.sent" // Message
	// KindDelivery is a full snapshot of one delivery's mutable fields, so
	// the current state is the newest row per deliveryId:
	// {deliveryId, messageId, to, repoKey?, state, hold?, via?, wakeId?, note?,
	// until?, movedTo?, movedFrom?, reminded?, engagedOnce?, mode?,
	// by, at}. mode "" reads as "wake" (rows before next-turn delivery).
	// movedFrom names the delivery a move replaced; engagedOnce is set once
	// the recipient engaged or deferred it.
	KindDelivery = "delivery.state"
	// KindWakeIssued is one wake or append: {wakeId, to, token, reminder,
	// deliveryIds, path?, mode?, overflow?, at}. mode "next_turn" marks an
	// append (token "[irori-append:<id>]") or a channel push holding only
	// next_turn items; "" reads as "wake".
	KindWakeIssued = "wake.issued"
	KindAlias      = "agent.alias" // {key, alias, at}
	// KindAgentName records the card name presence showed for a key when the
	// inbox first saw it, or saw it change: {key, name (lowercased), at}.
	KindAgentName   = "agent.name"
	KindPolicySet   = "policy.set"   // Policy + {by, at}
	KindTurnEngaged = "turn.engaged" // {agent, wakeId, turnId, at}
	KindTurnEnded   = "turn.ended"   // {agent, wakeId, turnId, at}
)
