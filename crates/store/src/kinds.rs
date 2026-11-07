//! The kinds registry (store.md §2.8; I:internal/store/kinds.go and the
//! Board column in board.go).
//!
//! Every kind the tree writes is a constant here, and the [`kinds!`] table
//! that defines the constants is also the registry, so a kind can't exist
//! without a Side and (when it replicates) a Board verdict. Go's two maps
//! (`kindSides`, `kindBoards`) are one table here; `tests/it/kinds.rs` checks it
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
    /// Refused by a board log, like every kind it does not name, unless
    /// [`board_tolerated`] says a pulled row of it is stored.
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
    /// The person edited their comment before any agent read it (card
    /// #105). Private cards only: a board member's device bridges the
    /// first row on its own, so a board log never takes an edit.
    TASK_COMMENT_EDITED = "task.comment.edited" => Replicated, Refuses;
    TASK_STEPS = "task.steps" => Replicated, Accepts;
    TASK_STEP = "task.step" => Replicated, Accepts;
    TASK_WATCHED = "task.watched" => Replicated, Accepts;
    /// A card's spot on the Board's canvas (card #93).
    TASK_PLACED = "task.placed" => Replicated, Accepts;
    /// Irori's hand-out of an unheld card to one free agent at a time
    /// (card #82).
    TASK_OFFERED = "task.offered" => Replicated, Accepts;
    /// Where a card's work is (card #137): the holder's worktree, branch,
    /// last commit and whether it's pushed, written by the holder's
    /// device. Private cards only: worktree paths stay with the person.
    TASK_WHERE = "task.where" => Replicated, Refuses;
    /// Waits (card #87): one-shot wakes on a task's state or a branch.
    WAIT_CREATED = "wait.created" => Replicated, Accepts;
    WAIT_FIRED = "wait.fired" => Replicated, Accepts;
    WAIT_CANCELED = "wait.canceled" => Replicated, Accepts;
    LEASE_ACQUIRED = "lease.acquired" => Replicated, Accepts;
    LEASE_RELEASED = "lease.released" => Replicated, Accepts;
    LEASE_REFUSED = "lease.refused" => Replicated, Accepts;
    BLOCKER_CLOSED = "blocker.closed" => Replicated, Accepts;
    BLOCKER_OPTION = "blocker.option" => Replicated, Accepts;
    BLOCKER_RESOLVED = "blocker.resolved" => Replicated, Accepts;
    TASK_EDITED = "task.edited" => Replicated, Refuses;
    TAKE_REQUESTED = "take.requested" => Replicated, Refuses;
    TAKE_STATE = "take.state" => Replicated, Refuses;
    /// An ask's owner hands it to a device that can run agents (card #541).
    ASK_MOVED = "ask.moved" => Replicated, Refuses;
    /// A card's parent is a card on the same board (closure checks it).
    TASK_PARENT = "task.parent" => Replicated, Accepts;
    /// The person's own card order in their window.
    TASK_PINNED = "task.pinned" => Replicated, Refuses;
    /// The person deleted a card (card #57): it leaves every view; the
    /// log keeps it. Go v0.5 Cores store and replicate it and ignore it.
    TASK_DELETED = "task.deleted" => Replicated, Refuses;
    /// The person cleared a done card out of the window's lists (card
    /// #138): search and show still find it. The private log only, so a
    /// shared board's card is cleared for this person alone.
    TASK_ARCHIVED = "task.archived" => Replicated, Refuses;
    /// The person paused a card (card #207): its holders are stopped and
    /// their tools refused until Resume. The private log only.
    TASK_PAUSED = "task.paused" => Replicated, Refuses;
    /// A shared card's owner changing hands (card #432):
    /// `{taskId, op: offer|request|accept|decline, person, blockerId?, by, at}`.
    /// `person` is who would own it. Shared boards only.
    TASK_OWNER = "task.owner" => Replicated, Accepts;
    /// A notification Irori showed the person (card #377): `{key, kind,
    /// title, body, level, taskId?, blockerId?, askId?, deliveryId?,
    /// boardId?, from?, at}`. The private log only; it syncs to their
    /// phone, which shows the history.
    NOTIFICATION_POSTED = "notification.posted" => Replicated, Refuses;
    /// The person read notifications (card #377): `{keys, at}`.
    NOTIFICATION_READ = "notification.read" => Replicated, Refuses;
    /// A spawned agent for a card (spawned agents §2, card #446):
    /// `{taskId, gen, step?, subtask?, op: requested|failed|canceled,
    /// device, repoKey, folder, worktree?, model, effort, permissionMode,
    /// name, reason?, by, at}`. The newest `gen` per card (and part) wins;
    /// a pending request keeps the card out of hand-out. `device` is the
    /// device that starts it, which may be another of the person's
    /// (card #455); on a shared board only the card's author's devices
    /// write it.
    TASK_SPAWN = "task.spawn" => Replicated, Accepts;
    /// The spawned_for mark (card #446): `{taskId, key, gen, step?,
    /// subtask?, device, by, at}`. Agent `key` was started for card
    /// `taskId` and is never offered another; kept after it goes. On a
    /// shared board `key` is the agent's `agent:<cardId>` (card #455).
    TASK_SPAWNED = "task.spawned" => Replicated, Accepts;
    /// What a device can start agents in (card #455): `{device, os,
    /// repos: [{repoKey, folder, trusted}], at}`, actor `irori`, written by
    /// that device only when it changes. The device that hands a card out
    /// picks where its agent starts from these (decision 7 (a)); a device
    /// with none is never picked.
    SPAWN_HOST = "spawn.host" => Replicated, Refuses;
    /// The person asks another of their devices to end a spawned agent it
    /// runs (card #455): `{key, taskId, by, atDevice, at}`, actor `user`.
    /// Only the device the agent runs on acts on it.
    SPAWN_END_REQUESTED = "spawn.end.requested" => Replicated, Refuses;
    /// The person asks for a card's agent now, from any of their devices
    /// (card #455): `{taskId, note?, by, atDevice, at}`, actor `user`. The
    /// device that hands the card out acts on it, with the note in the brief.
    SPAWN_START_REQUESTED = "spawn.start.requested" => Replicated, Refuses;
    /// The person asks the device a card's agent must start on to trust
    /// its repo (card #455): `{taskId, gen, device, repoKey, folder, by,
    /// atDevice, at}`, actor `user`. That device only asks the person in
    /// its own window; trust is written by their confirm there, never by
    /// this row.
    TRUST_REQUESTED = "trust.requested" => Replicated, Refuses;
    /// A board's spawned-agent settings (card #446): `{boardId, on, by,
    /// at}`, actor `user`; `""` is the private board. Set in the app only.
    SPAWN_SET = "spawn.set" => Replicated, Refuses;
    /// A device's cap on spawned agents (card #446): `{device, cap, by,
    /// at}`, actor `user`.
    SPAWN_CAP = "spawn.cap" => Replicated, Refuses;
    /// The person's "Offer it cards" on an agent they started (card #453):
    /// `{key, on, by, at}`, actor `user`. Without it a terminal agent is
    /// never handed unheld cards, asks or threads. Set in the app only.
    AGENT_OFFER = "agent.offer" => Replicated, Refuses;
    /// "Transcribe what I say in calls" (card #656): `{on, by, at}`, actor
    /// `user`, newest wins, none means on. Set in the app only.
    TRANSCRIBE_SET = "transcribe.set" => Replicated, Refuses;
    /// The themes new agents' names don't come from (card #702): `{off,
    /// by, at}`, actor `user`, newest wins, none means every theme on. Set
    /// in the app only.
    NAMES_THEMES = "names.themes" => Replicated, Refuses;
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
    /// `{color, at}`, actor `user`: the person's colour on shared boards
    /// (card #385); boards learn it from member.profile.
    PERSON_COLOR = "person.color" => Replicated, Refuses;
    /// `{name, avatar}`: an agent's face, kept by its name (card #407).
    AGENT_AVATAR = "agent.avatar" => Replicated, Refuses;
    /// `{avatar, atDevice}`, actor `user`: the person's avatar on shared
    /// spaces (card #393); spaces learn it from member.profile.
    PERSON_AVATAR = "person.avatar" => Replicated, Refuses;
    // devname (card #184)
    DEVICE_NAME = "device.name" => Replicated, Refuses;
    /// `{deviceId, sides, atDevice}`, actor `user`: the work/personal
    /// sides a device runs, the first its new cards' (card #494). Counts
    /// only when the signing device is `deviceId`; no row, every side.
    DEVICE_SIDES = "device.sides" => Replicated, Refuses;
    // v0.5 shared boards: board logs only
    BOARD_CREATED = "board.created" => Replicated, Accepts;
    BOARD_RENAMED = "board.renamed" => Replicated, Accepts;
    BOARD_OPEN_CARDS = "board.opencards" => Replicated, Accepts;
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
    /// An ask's take request on a board (the private `take.requested`).
    BOARD_ASK_REQUESTED = "board.ask.requested" => Replicated, Accepts;
    /// An ask's take lock, written by the asking device (`take.state`).
    BOARD_ASK_STATE = "board.ask.state" => Replicated, Accepts;
    /// [`ASK_MOVED`] in a board's log.
    BOARD_ASK_MOVED = "board.ask.moved" => Replicated, Accepts;
    /// An ask's outcome edit of a card (`task.edited`).
    BOARD_ASK_EDITED = "board.ask.edited" => Replicated, Accepts;
    REVIEW_REQUESTED = "review.requested" => Replicated, Accepts;
    REVIEW_REVIEWER = "review.reviewer" => Replicated, Accepts;
    /// How a card's linked PR stands on GitHub (card #208): `{taskId, pr,
    /// state?, review?, mergedAt?, error?, by, at}`, by `irori`.
    PR_STATE = "pr.state" => Replicated, Accepts;
    /// Notes (decision 45, card #401): in the private log for Mine, in a
    /// board's log for that board. `{noteId, kind, text, tags, source?,
    /// taskId?, commentId?, blockerId?, by, at}`.
    NOTE_PROPOSED = "note.proposed" => Replicated, Accepts;
    /// `{noteId, by, at}`: a person accepts a proposal or an agent's
    /// pending edit or removal.
    NOTE_ACCEPTED = "note.accepted" => Replicated, Accepts;
    /// `{noteId, kind?, text?, tags?, why?, by, at}`.
    NOTE_EDITED = "note.edited" => Replicated, Accepts;
    /// `{noteId, why?, by, at}`.
    NOTE_REMOVED = "note.removed" => Replicated, Accepts;
    // Spaces (card #392, decision 46): a space's log is a v0.5 board log,
    // and its id is that board's. These add more boards to it; the first
    // board is the space's own (board.created). An Irori before spaces
    // refuses them, so a space writes them only once every member device
    // said it reads spaces (mesh SPACES_PROTOCOL). Personal's boards are
    // these rows in the private log (card #558), where an older Irori
    // stores them and folds past them, so they need no gate there.
    /// `{boardId, name, tag, by, at}`: another board in the space.
    SPACE_BOARD_CREATED = "space.board.created" => Replicated, Accepts;
    /// `{boardId, name, tag, by, at}`: a board in the space renamed.
    SPACE_BOARD_RENAMED = "space.board.renamed" => Replicated, Accepts;
    /// `{boardId, description, by, at}`: a board's description (card
    /// #690), empty clears it; the newest row stands. Stored unread by an
    /// Irori at hello `tolerant` 1, so a space writes it only once every
    /// member device is there.
    SPACE_BOARD_DESCRIBED = "space.board.described" => Replicated, Accepts;
    /// `{boardId, closed, by, at}`: a board closed (read-only, out of the
    /// lists) or reopened (`closed: false`) (card #690); the newest valid
    /// row stands. Gated like `space.board.described`.
    SPACE_BOARD_CLOSED = "space.board.closed" => Replicated, Accepts;
    // merge desk (card #415, docs/design/merge-desk.md §7): the person's
    // private log in v1
    /// `{repoKey, enabled, takesPrs, pollDevice, trusted[], perPrOk, by,
    /// at}`: person only. Older rows also carry `holder`, `checks` and
    /// `mergeMethod`, no longer written or read (card #418).
    REPO_DESK = "repo.desk" => Replicated, Refuses;
    /// `{repoKey, truncated, prs[], by, at}`: Irori's snapshot of the
    /// repo's open PRs, written on change by the poll device.
    DESK_PRS = "desk.prs" => Replicated, Refuses;
    /// `{repoKey, pr, note?, cleared?, by, at}`: no longer written (card
    /// #418, the desk never merges); old rows are folded past.
    DESK_HOLD = "desk.hold" => Replicated, Refuses;
    /// `{repoKey, paused, by, at}`: no longer written (card #418); old
    /// rows are folded past.
    DESK_PAUSE = "desk.pause" => Replicated, Refuses;
    /// `{repoKey, pr, headSha, by, at}`: legacy, read only. Old rows still
    /// decode; nothing writes it since card #625 removed Outside PR's OK.
    DESK_OK = "desk.ok" => Replicated, Refuses;
    /// `{repoKey, pr, headSha, baseSha?, by, at}`: the old desk holder's
    /// merge run, no longer written (card #418); old rows are folded past.
    DESK_RUN = "desk.run" => Replicated, Refuses;
    // v0.5 private-log rows, never in a board log
    BOARD_JOINED = "board.joined" => Replicated, Refuses;
    BOARD_FORGOT = "board.forgot" => Replicated, Refuses;
    /// `{boardId, creator, with, at}`: a hidden two-person DM space (card
    /// #538). It plays board.joined's part and says the space is hidden;
    /// an older device stores it and ignores it, so it never opens one.
    DM_SPACE_JOINED = "dm.space.joined" => Replicated, Refuses;
    RECEIVE_SET = "receive.set" => Replicated, Refuses;
    AUTONOMY_SET = "autonomy.set" => Replicated, Refuses;
    /// The person's threads on a shared board: shared with every member,
    /// or private to them and their own agents (card #185).
    THREADS_SET = "threads.set" => Replicated, Refuses;
    /// `{privateTaskId, boardId, taskId}`.
    SHARED_COPY = "board.sharedcopy" => Replicated, Refuses;
    /// `{boardId, cardId, agentKey}`.
    BOARD_CARD_OF = "board.cardof" => Replicated, Refuses;
    // talk (#389): channels, DMs and thread posts (docs/design/talk-channels.md §6)
    /// `{channelId, spaceId, name, topic?, by}`.
    TALK_CHANNEL_CREATED = "talk.channel.created" => Replicated, Accepts;
    /// `{channelId, name?, topic?, archived?}`.
    TALK_CHANNEL_EDITED = "talk.channel.edited" => Replicated, Accepts;
    /// `{convId, member: {kind, key}, op: join|leave, by}`.
    TALK_MEMBER = "talk.member" => Replicated, Accepts;
    /// `{postId, convId, text, mentions[], agentsNow, threadFrom?, attach[]?, by, at}`.
    TALK_POST = "talk.post" => Replicated, Accepts;
    /// `{convId, upTo, by}`: a read cursor, the person's own devices only.
    TALK_READ = "talk.read" => Replicated, Refuses;
    /// `{postId, taskId}`: a card made from a message.
    TALK_CARD = "talk.card" => Replicated, Accepts;
    /// `{callId, convId, seg, startMs, endMs, text, by, at}`: one finished
    /// phrase of a huddle, written only by the speaker's own device (card #656).
    CALL_SEGMENT = "call.segment" => Replicated, Accepts;
    // device-local board rows
    /// `{cardId, agentKey}`; still read, no longer written.
    BOARD_CARD_KEY = "board.cardkey" => Local;
    /// `{personId, personPub, rootDeviceId, personEpoch}`: a board's anchor.
    BOARD_PIN = "board.pin" => Local;
    /// `{boardId, upTo}`: this device forgot the board.
    BOARD_FORGOTTEN = "board.forgotten" => Local;
    /// `{deviceId, personId, coreVersion, spaces, spawns, tolerant, seq,
    /// atMs}`: the last hello this device heard from `deviceId` on the
    /// board (card #577).
    BOARD_HELLO = "board.hello" => Local;
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

/// The row families the tolerant reader stores unknown kinds of: content,
/// never membership. `task.` and `space.` since v0.12.4 (card #579, the
/// hello's `tolerant` 1); `talk.` and `call.` since `tolerant` 2 (card #652,
/// ahead of calls, #571).
pub const TOLERATED_FAMILIES: [&str; 4] = ["task.", "space.", "talk.", "call."];

/// The tolerant reader (docs/design/no-server.md §3.5): a pulled row of a
/// [`TOLERATED_FAMILIES`] kind this build has never heard of. A board log
/// stores it, so a newer Irori's kind doesn't stall this device, and it
/// folds after the update. A kind the registry names keeps its verdict, and
/// any other unknown family stays refused: stalling on a membership or
/// revoke row fails closed, and skipping one fails open.
///
/// A kind stored this way must never become [`Board::Refuses`], or the build
/// that says so stalls on rows already in the log: `tests/board.rs` pins the
/// kinds of these families that are refused today.
pub fn board_tolerated(kind: &str) -> bool {
    TOLERATED_FAMILIES
        .iter()
        .any(|f| kind.strip_prefix(f).is_some_and(|n| !n.is_empty()))
        && !index().contains_key(kind)
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
