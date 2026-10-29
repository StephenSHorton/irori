package store

import "strings"

// Event kinds written in v0.1. Payloads are JSON objects; add fields, never
// repurpose them, because old rows live forever.
const (
	// KindDeviceStarted: {"name","os","version"} once per process start.
	KindDeviceStarted = "device.started"
	// KindAgentSeen: a presence.AgentCard the first time a session is seen.
	KindAgentSeen = "agent.seen"
	// KindAgentStatus: {"key","status","name","permissionMode"?} when status,
	// name or the permission mode changes (the mode is omitted when unknown).
	KindAgentStatus = "agent.status"
	// KindAgentGone: {"key","reason"} when a session's file disappears or its pid dies.
	KindAgentGone = "agent.gone"
)

// v0.4 identity kinds (design/v0.4-devices.md §2). Written by internal/identity.
const (
	// KindPersonDeviceAdded: {cert, personPub, personSig}: a device joins the person.
	KindPersonDeviceAdded = "person.device.added"
	// KindPersonDeviceRevoked: {revoke: {personId, deviceId, cut, cutHash, at, reason}, personSig}.
	KindPersonDeviceRevoked = "person.device.revoked"
	// KindPersonKeyRotated: {personId, epoch, personPub, at}; the public key only.
	KindPersonKeyRotated = "person.key.rotated"
	// KindPersonProfile: {personId, name, rootDeviceId, personPub, personSig}.
	KindPersonProfile = "person.profile"
	// KindLocalMoved: the placeholder the one-time migration leaves at the
	// seq of a row it moved to local_events (§5.4). Payload {}. Every fold
	// skips it; it exists so seqs stay gapless.
	KindLocalMoved = "local.moved"
)

// KindAttachOffered: {sha256, size, name, mime}: the writing device holds
// this attachment and serves it to the person's other devices (v0.4 §6.6).
// Written when a file is attached and after every fetch, so any device that
// has the file can serve it. Written by internal/mesh and core.
const KindAttachOffered = "attach.offered"

// KindNodeAddr: {v, port, lan, v6, public, via, reach, at}: where the
// writing device's home server can be reached (v0.4 §4.2, decision 33).
// Only that device writes its own; it is written only when something in it
// changes. Written by internal/home.
const KindNodeAddr = "node.addr"

// v0.5 Shared Boards kinds (design/v0.5-shared-boards.md §2.1, §2.2, §3).
// Board logs only: the board's own rows, each member's board-scoped
// identity objects, the reduced agent card and peer review. Reserved here
// so they ship classified; later steps write them.
const (
	KindBoardCreated        = "board.created"
	KindBoardRenamed        = "board.renamed"
	KindBoardMemberAdded    = "board.member.added"
	KindBoardMemberRole     = "board.member.role"
	KindBoardMemberRemoved  = "board.member.removed"
	KindBoardMemberLeft     = "board.member.left"
	KindBoardServer         = "board.server"
	KindMemberProfile       = "member.profile"
	KindMemberDeviceAdded   = "member.device.added"
	KindMemberDeviceRevoked = "member.device.revoked"
	KindMemberKeyRotated    = "member.key.rotated"
	// KindAgentCard: {cardId, name, kind, personId, status}: the only trace
	// of an agent in a board log (§2.4). Its actor is "agent:<cardId>".
	KindAgentCard       = "agent.card"
	KindReviewRequested = "review.requested"
	KindReviewReviewer  = "review.reviewer"
)

// v0.5 private-log kinds, replicated among the person's own devices only
// and never accepted by a board log (§4.1, §4.6, §7.2, §7.3).
const (
	KindBoardJoined = "board.joined"
	KindBoardForgot = "board.forgot"
	KindReceiveSet  = "receive.set"
	KindAutonomySet = "autonomy.set"
)

// KindBoardCardKey: {cardId, agentKey}: which local session a board's agent
// card is (§2.4's device-local board_cards). It lives in the board store's
// local_events, so it never leaves the device.
const KindBoardCardKey = "board.cardkey"

// Device-local kinds, which live in local_events and never replicate (§5.1).
// They are named here as strings because their packages import store.
const (
	// KindPolicySet is inbox.KindPolicySet: allow and block lists, Pause all.
	KindPolicySet = "policy.set"
	// KindMemberJoined is core.KindMemberJoined: a Grok member bound.
	KindMemberJoined = "member.joined"
)

// Side says whether a kind replicates.
type Side int

const (
	// Unclassified kinds are stored and replicate (peers' newer kinds arrive
	// this way), but every kind this tree writes must be classified.
	Unclassified Side = iota
	// Replicated rows go in events and sync to the person's devices.
	Replicated
	// Local rows go in local_events and never leave the device.
	Local
)

// kindSides is the registry: every kind in the tree and its side.
// TestEveryKindIsClassified fails when a new kind constant is missing here.
var kindSides = map[string]Side{
	// store
	KindDeviceStarted: Replicated, KindAgentSeen: Replicated, KindAgentStatus: Replicated, KindAgentGone: Replicated,
	KindPersonDeviceAdded: Replicated, KindPersonDeviceRevoked: Replicated, KindPersonKeyRotated: Replicated,
	KindPersonProfile: Replicated, KindLocalMoved: Replicated,
	// inbox
	"message.sent": Replicated, "delivery.state": Replicated, "wake.issued": Replicated,
	"agent.alias": Replicated, "agent.name": Replicated, "turn.engaged": Replicated, "turn.ended": Replicated,
	KindPolicySet: Local,
	// core
	KindMemberJoined: Local,
	// coord
	"task.created": Replicated, "task.claimed": Replicated, "task.assigned": Replicated, "task.status": Replicated,
	"task.joined": Replicated, "task.left": Replicated, "task.shared": Replicated, "task.linked": Replicated,
	"task.comment": Replicated, "task.steps": Replicated, "task.step": Replicated, "task.watched": Replicated,
	"lease.acquired": Replicated, "lease.released": Replicated, "lease.refused": Replicated,
	"blocker.closed": Replicated, "blocker.option": Replicated, "blocker.resolved": Replicated,
	"task.edited": Replicated, "take.requested": Replicated, "take.state": Replicated,
	// capture
	"capture.turn": Replicated, "capture.edit": Replicated, "capture.summary": Replicated,
	"capture.commitcmd": Replicated, "pr.ledger": Replicated, "repo.seen": Replicated,
	"commit.seen": Replicated, "commit.linked": Replicated, "decision.recorded": Replicated,
	// v0.4 requests and owner rows (§6.0), reserved here so they ship classified.
	"delivery.decided": Replicated, "ask.posted": Replicated, "ask.answered": Replicated,
	"ask.resolved": Replicated, "revive.requested": Replicated, KindAttachOffered: Replicated,
	"lease.stolen": Replicated,
	// home server (decision 33)
	KindNodeAddr: Replicated,
	// devcolor: the person's colour pick for a device (card #22)
	"device.color": Replicated,
	// v0.5 Shared Boards (their Board column is kindBoards, board.go)
	KindBoardCreated: Replicated, KindBoardRenamed: Replicated, KindBoardMemberAdded: Replicated,
	KindBoardMemberRole: Replicated, KindBoardMemberRemoved: Replicated, KindBoardMemberLeft: Replicated,
	KindBoardServer: Replicated, KindMemberProfile: Replicated, KindMemberDeviceAdded: Replicated,
	KindMemberDeviceRevoked: Replicated, KindMemberKeyRotated: Replicated, KindAgentCard: Replicated,
	KindReviewRequested: Replicated, KindReviewReviewer: Replicated,
	KindBoardJoined: Replicated, KindBoardForgot: Replicated, KindReceiveSet: Replicated, KindAutonomySet: Replicated,
	KindBoardCardKey: Local,
}

// KindSide reports kind's side.
func KindSide(kind string) Side { return kindSides[kind] }

// IsLocal reports whether kind is device-local: written with AppendLocal,
// refused by Append and by Insert.
func IsLocal(kind string) bool { return kindSides[kind] == Local }

// IsPersonKind reports an identity row (person.*), which the Verifier folds.
func IsPersonKind(kind string) bool { return strings.HasPrefix(kind, "person.") }
