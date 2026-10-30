//! The kinds registry (store.md §2.8; I:internal/store/kinds.go and the
//! Board column in board.go).
//!
//! Every kind the tree writes is a constant here, and the [`kinds!`] table
//! that defines the constants is also the registry, so a kind can't exist
//! without a Side and (when it replicates) a Board verdict. Go's two maps
//! (`kindSides`, `kindBoards`) are one table here; `tests/kinds.rs` checks it
//! against the Go-exported table and the consistency rules Go's
//! `TestEveryKindIsClassified` checks.
//!
//! Payload shapes are owned by the areas that write each kind
//! (I:internal/store/kinds.go:5-120 has the notes).

use std::collections::HashMap;
use std::sync::OnceLock;

/// Whether a kind replicates (Go `Side`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// Stored and replicated by the private log (peers' newer kinds), but
    /// every kind this tree writes is classified.
    Unclassified,
    /// In `events`; syncs to the person's devices.
    Replicated,
    /// In `local_events`; never leaves the device.
    Local,
}

/// A kind's verdict for a shared board's log (Go `Board`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Board {
    /// Refused by a board log, like every kind it does not name.
    Unclassified,
    /// A board log stores it.
    Accepts,
    /// It never enters a board log.
    Refuses,
}

/// One registry row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindEntry {
    pub kind: &'static str,
    pub side: Side,
    pub board: Board,
}

macro_rules! kinds {
    ($( $(#[$m:meta])* $name:ident = $val:literal => $side:ident $(, $board:ident)? ;)*) => {
        $( $(#[$m])* pub const $name: &str = $val; )*

        /// The registry: every kind constant with its Side and Board column.
        pub static REGISTRY: &[KindEntry] = &[
            $( KindEntry { kind: $name, side: Side::$side, board: kinds!(@board $($board)?) }, )*
        ];
    };
    (@board) => { Board::Unclassified };
    (@board $b:ident) => { Board::$b };
}

kinds! {
    // store (v0.1)
    /// `{"name","os","version"}` once per process start.
    DEVICE_STARTED = "device.started" => Replicated, Refuses;
    /// A presence AgentCard the first time a session is seen.
    AGENT_SEEN = "agent.seen" => Replicated, Refuses;
    /// `{"key","status","name","permissionMode"?}`.
    AGENT_STATUS = "agent.status" => Replicated, Refuses;
    /// `{"key","reason"}`.
    AGENT_GONE = "agent.gone" => Replicated, Refuses;
    // v0.4 identity (written by identity)
    /// `{cert, personPub, personSig}`.
    PERSON_DEVICE_ADDED = "person.device.added" => Replicated, Refuses;
    /// `{revoke: {personId, deviceId, cut, cutHash, at, reason}, personSig}`.
    PERSON_DEVICE_REVOKED = "person.device.revoked" => Replicated, Refuses;
    /// `{personId, epoch, personPub, at}`.
    PERSON_KEY_ROTATED = "person.key.rotated" => Replicated, Refuses;
    /// `{personId, name, rootDeviceId, personPub, personSig}`.
    PERSON_PROFILE = "person.profile" => Replicated, Refuses;
    /// The v3 migration's placeholder for a row moved to `local_events`;
    /// payload `{}`.
    LOCAL_MOVED = "local.moved" => Replicated, Refuses;
    // inbox
    MESSAGE_SENT = "message.sent" => Replicated, Refuses;
    DELIVERY_STATE = "delivery.state" => Replicated, Refuses;
    WAKE_ISSUED = "wake.issued" => Replicated, Refuses;
    AGENT_ALIAS = "agent.alias" => Replicated, Refuses;
    AGENT_NAME = "agent.name" => Replicated, Refuses;
    TURN_ENGAGED = "turn.engaged" => Replicated, Refuses;
    TURN_ENDED = "turn.ended" => Replicated, Refuses;
    /// inbox's allow and block lists, Pause all.
    POLICY_SET = "policy.set" => Local;
    // core
    /// A Grok member bound.
    MEMBER_JOINED = "member.joined" => Local;
    // coord
    TASK_CREATED = "task.created" => Replicated, Accepts;
    TASK_CLAIMED = "task.claimed" => Replicated, Accepts;
    TASK_ASSIGNED = "task.assigned" => Replicated, Accepts;
    TASK_STATUS = "task.status" => Replicated, Accepts;
    TASK_JOINED = "task.joined" => Replicated, Accepts;
    TASK_LEFT = "task.left" => Replicated, Accepts;
    /// Names a private task; Share writes a fresh task.created.
    TASK_SHARED = "task.shared" => Replicated, Refuses;
    TASK_LINKED = "task.linked" => Replicated, Accepts;
    TASK_COMMENT = "task.comment" => Replicated, Accepts;
    TASK_STEPS = "task.steps" => Replicated, Accepts;
    TASK_STEP = "task.step" => Replicated, Accepts;
    TASK_WATCHED = "task.watched" => Replicated, Accepts;
    LEASE_ACQUIRED = "lease.acquired" => Replicated, Accepts;
    LEASE_RELEASED = "lease.released" => Replicated, Accepts;
    LEASE_REFUSED = "lease.refused" => Replicated, Accepts;
    BLOCKER_CLOSED = "blocker.closed" => Replicated, Accepts;
    BLOCKER_OPTION = "blocker.option" => Replicated, Accepts;
    BLOCKER_RESOLVED = "blocker.resolved" => Replicated, Accepts;
    TASK_EDITED = "task.edited" => Replicated, Refuses;
    TAKE_REQUESTED = "take.requested" => Replicated, Refuses;
    TAKE_STATE = "take.state" => Replicated, Refuses;
    /// A card's parent is a card on the same board (closure checks it).
    TASK_PARENT = "task.parent" => Replicated, Accepts;
    /// The person's own card order in their window.
    TASK_PINNED = "task.pinned" => Replicated, Refuses;
    /// The person deleted a card (card #57): it leaves every view; the
    /// log keeps it. Go v0.5 Cores store and replicate it and ignore it.
    TASK_DELETED = "task.deleted" => Replicated, Refuses;
    // capture
    CAPTURE_TURN = "capture.turn" => Replicated, Refuses;
    CAPTURE_EDIT = "capture.edit" => Replicated, Refuses;
    CAPTURE_SUMMARY = "capture.summary" => Replicated, Refuses;
    CAPTURE_COMMITCMD = "capture.commitcmd" => Replicated, Refuses;
    PR_LEDGER = "pr.ledger" => Replicated, Refuses;
    REPO_SEEN = "repo.seen" => Replicated, Refuses;
    COMMIT_SEEN = "commit.seen" => Replicated, Refuses;
    COMMIT_LINKED = "commit.linked" => Replicated, Refuses;
    DECISION_RECORDED = "decision.recorded" => Replicated, Refuses;
    // v0.4 requests and owner rows
    DELIVERY_DECIDED = "delivery.decided" => Replicated, Refuses;
    ASK_POSTED = "ask.posted" => Replicated, Refuses;
    ASK_ANSWERED = "ask.answered" => Replicated, Refuses;
    ASK_RESOLVED = "ask.resolved" => Replicated, Refuses;
    REVIVE_REQUESTED = "revive.requested" => Replicated, Refuses;
    /// `{sha256, size, name, mime}`: the writer holds and serves this file.
    ATTACH_OFFERED = "attach.offered" => Replicated, Accepts;
    LEASE_STOLEN = "lease.stolen" => Replicated, Accepts;
    // home server
    /// `{v, port, lan, v6, public, via, reach, at}`.
    NODE_ADDR = "node.addr" => Replicated, Refuses;
    // devcolor
    DEVICE_COLOR = "device.color" => Replicated, Refuses;
    // v0.5 shared boards: board logs only
    BOARD_CREATED = "board.created" => Replicated, Accepts;
    BOARD_RENAMED = "board.renamed" => Replicated, Accepts;
    BOARD_MEMBER_ADDED = "board.member.added" => Replicated, Accepts;
    BOARD_MEMBER_ROLE = "board.member.role" => Replicated, Accepts;
    BOARD_MEMBER_REMOVED = "board.member.removed" => Replicated, Accepts;
    BOARD_MEMBER_LEFT = "board.member.left" => Replicated, Accepts;
    BOARD_SERVER = "board.server" => Replicated, Accepts;
    MEMBER_PROFILE = "member.profile" => Replicated, Accepts;
    MEMBER_DEVICE_ADDED = "member.device.added" => Replicated, Accepts;
    MEMBER_DEVICE_REVOKED = "member.device.revoked" => Replicated, Accepts;
    MEMBER_KEY_ROTATED = "member.key.rotated" => Replicated, Accepts;
    /// `{cardId, name, kind, personId, status}`; actor `agent:<cardId>`.
    AGENT_CARD = "agent.card" => Replicated, Accepts;
    REVIEW_REQUESTED = "review.requested" => Replicated, Accepts;
    REVIEW_REVIEWER = "review.reviewer" => Replicated, Accepts;
    // v0.5 private-log rows, never in a board log
    BOARD_JOINED = "board.joined" => Replicated, Refuses;
    BOARD_FORGOT = "board.forgot" => Replicated, Refuses;
    RECEIVE_SET = "receive.set" => Replicated, Refuses;
    AUTONOMY_SET = "autonomy.set" => Replicated, Refuses;
    /// `{privateTaskId, boardId, taskId}`.
    SHARED_COPY = "board.sharedcopy" => Replicated, Refuses;
    /// `{boardId, cardId, agentKey}`.
    BOARD_CARD_OF = "board.cardof" => Replicated, Refuses;
    // device-local board rows
    /// `{cardId, agentKey}`; still read, no longer written.
    BOARD_CARD_KEY = "board.cardkey" => Local;
    /// `{personId, personPub, rootDeviceId, personEpoch}`: a board's anchor.
    BOARD_PIN = "board.pin" => Local;
    /// `{boardId, upTo}`: this device forgot the board.
    BOARD_FORGOTTEN = "board.forgotten" => Local;
}

fn index() -> &'static HashMap<&'static str, (Side, Board)> {
    static INDEX: OnceLock<HashMap<&'static str, (Side, Board)>> = OnceLock::new();
    INDEX.get_or_init(|| {
        REGISTRY
            .iter()
            .map(|e| (e.kind, (e.side, e.board)))
            .collect()
    })
}

/// Go `KindSide`.
pub fn kind_side(kind: &str) -> Side {
    index().get(kind).map_or(Side::Unclassified, |e| e.0)
}

/// Go `KindBoard`.
pub fn kind_board(kind: &str) -> Board {
    index().get(kind).map_or(Board::Unclassified, |e| e.1)
}

/// Go `IsLocal`: written with append_local, refused by append and insert.
pub fn is_local(kind: &str) -> bool {
    kind_side(kind) == Side::Local
}

/// Go `IsPersonKind`: an identity row of the private log.
pub fn is_person_kind(kind: &str) -> bool {
    kind.starts_with("person.")
}

/// Go `BoardAccepted`: a board log stores it.
pub fn board_accepted(kind: &str) -> bool {
    !is_local(kind) && kind_board(kind) == Board::Accepts
}

/// Go `IsBoardIdentityKind`: board.*, member.* and agent.card, which a board
/// store's Verifier folds.
pub fn is_board_identity_kind(kind: &str) -> bool {
    board_accepted(kind)
        && (kind.starts_with("board.") || kind.starts_with("member.") || kind == AGENT_CARD)
}

/// Board actor prefixes (Go `PersonActorPrefix`, `AgentActorPrefix`).
pub const PERSON_ACTOR_PREFIX: &str = "person:";
pub const AGENT_ACTOR_PREFIX: &str = "agent:";

/// Go `BoardActorOK`: `irori`, `person:<id>` or `agent:<cardId>`.
pub fn board_actor_ok(actor: &str) -> bool {
    if actor == "irori" {
        return true;
    }
    let id = actor
        .strip_prefix(PERSON_ACTOR_PREFIX)
        .or_else(|| actor.strip_prefix(AGENT_ACTOR_PREFIX));
    id.is_some_and(board_id)
}

/// Go `boardID`: a person or card id as a board key carries it.
pub(crate) fn board_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id
            .bytes()
            .any(|b| matches!(b, b':' | b'/' | b'\\' | b' ' | b'\t' | b'\r' | b'\n'))
}

/// Kinds `hold_appends` never holds: a revoke and its rotation.
pub(crate) fn never_held(kind: &str) -> bool {
    matches!(
        kind,
        PERSON_DEVICE_REVOKED | PERSON_KEY_ROTATED | MEMBER_DEVICE_REVOKED | MEMBER_KEY_ROTATED
    )
}
