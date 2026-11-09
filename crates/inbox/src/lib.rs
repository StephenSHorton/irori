//! `irori-inbox`: Irori's messaging core, a port of I:internal/inbox
//! (inbox.md).
//!
//! Messages, the per-recipient delivery state machine, the wake policy
//! (the person's own holds), the rate and hop limits, wake and append planning
//! (ride-along, MaxWakeTries), the sweeper and cross-device ownership. All
//! state is folded from the event log at [`Inbox::new`]; every mutation
//! appends a row first and then folds it, so live state and replayed state
//! come from the same code ([`state`]).
//!
//! Beside the core: [`notice`] (routing a Board notice through the inbox,
//! with watch-digest dedupe) and [`watch`] (the held-card and
//! message-to-the-person card logic of I:internal/notify/watch.go, as pure
//! logic over a card sink).
//!
//! The API is synchronous, like the store under it. Go's `context.Context`
//! is dropped: nothing here waits on anything but the store and the hooks.
//!
//! Merge notes: [`AgentCard`] and [`FileRef`] are local stand-ins for the
//! `presence` and `blob` crates, carrying only the fields the inbox reads
//! or writes; [`safe_name`] is a copy of `blob.SafeName`.

mod attach;
mod bridge;
mod edit;
mod fence;
mod ids;
mod mesh;
mod messages;
pub mod notice;
mod policy;
mod pr;
mod progress;
mod route;
pub mod search;
mod state;
mod sweep;
mod tm;
mod wake;
pub mod watch;

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use gocompat::time::GoTime;
use serde::Serialize;

pub use attach::safe_name;
pub use bridge::{
    AUTONOMY_DEFAULT, AutonomySetting, BOARD_KIND, BoardFrom, BridgeRow, RECEIVE_DEFAULT,
    ReceiveSetting, THREADS_DEFAULT, ThreadsSetting, autonomy, board_sender_key, receive,
    receive_decision, row_kind, threads_mode,
};
pub use edit::{EditState, editing};
pub use fence::{batch, fence, fence_prompt, is_wind_down};
pub use ids::IdGen;
pub use messages::clip_show;
pub use pr::normalize_pr;
pub use progress::{ProgressBy, progress_of};
pub use state::{Inbox, SawOthers, Subscription, decode_message};
pub use wake::TalkWake;

// ---- errors ---------------------------------------------------------------

/// The errors callers branch on (I:internal/inbox/inbox.go:26-40,
/// state.go:18-23). The texts are tool output: keep them exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    UnknownAgent,
    Ambiguous,
    RateLimited,
    NotYours,
    NotFound,
    BadState,
    TooLong,
    Unimplemented,
    NoCard,
    SelfAddress,
    Empty,
    BadPr,
    /// No sentinel: a plain error (Go `fmt.Errorf` without `%w`).
    Other,
}

impl ErrorKind {
    /// The sentinel's text.
    pub fn text(self) -> &'static str {
        match self {
            ErrorKind::UnknownAgent => "inbox: no agent by that name or key",
            ErrorKind::Ambiguous => "inbox: that name matches more than one agent",
            ErrorKind::RateLimited => "inbox: too many messages between these two agents",
            ErrorKind::NotYours => "inbox: that delivery isn't addressed to you",
            ErrorKind::NotFound => "inbox: no such delivery or message",
            ErrorKind::BadState => "inbox: not allowed in the delivery's current state",
            ErrorKind::TooLong => "inbox: message text is too long",
            ErrorKind::Unimplemented => "inbox: not implemented yet",
            ErrorKind::NoCard => "inbox: the message names no card here",
            ErrorKind::SelfAddress => "inbox: that's you; address another agent",
            ErrorKind::Empty => "inbox: give the message text",
            ErrorKind::BadPr => "inbox: not a pull request address",
            ErrorKind::Other => "inbox: error",
        }
    }
}

/// An inbox error: a kind to branch on and the full Go message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    msg: String,
}

impl Error {
    /// The bare sentinel (Go `return ErrX`).
    pub fn sentinel(kind: ErrorKind) -> Error {
        Error {
            kind,
            msg: kind.text().to_string(),
        }
    }

    /// Go `fmt.Errorf("%w: <detail>", ErrX)`.
    pub fn wrap(kind: ErrorKind, detail: impl fmt::Display) -> Error {
        Error {
            kind,
            msg: format!("{}: {detail}", kind.text()),
        }
    }

    /// A plain error with this exact text.
    pub fn other(msg: impl Into<String>) -> Error {
        Error {
            kind: ErrorKind::Other,
            msg: msg.into(),
        }
    }

    /// Go `fmt.Errorf("<prefix>: %w", err)`: the kind is kept.
    pub fn context(self, prefix: impl fmt::Display) -> Error {
        Error {
            kind: self.kind,
            msg: format!("{prefix}: {}", self.msg),
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Go `errors.Is(err, ErrX)`.
    pub fn is(&self, kind: ErrorKind) -> bool {
        self.kind == kind
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

// ---- constants ------------------------------------------------------------

/// The note on a delivery the sweeper took away from a session that ended.
pub const NOTE_RECIPIENT_GONE: &str = "recipient gone";
/// The note on a wind-down the person turned off before it reached its
/// agent (card #787).
pub const NOTE_WIND_DOWN_OFF: &str = "wind down turned off";

pub const MAX_TEXT: usize = 8000;
/// The person's own message may run to this many bytes (card #265); it
/// reaches an agent clipped to [`MAX_TEXT`] characters with a show pointer.
pub const MAX_PERSON_TEXT: usize = 512 << 10;
pub const MAX_WAKE_BYTES: usize = 16 << 10;
/// The most a wake's Notes section adds to its prompt (card #412).
pub const MAX_NOTES_BYTES: usize = 6 << 10;
/// The most a wake's "the person said" section adds (card #555).
pub const MAX_SAID_BYTES: usize = 3 << 10;
pub const DIRECT_LIMIT: usize = 8;
pub const DIRECT_WINDOW: Duration = Duration::from_secs(10 * 60);
/// The runaway-loop guard: a message this many replies down one chain of
/// agent messages is held (card #71 raised it from Go's 6, which held
/// ordinary back-and-forth; the per-pair rate limit bounds a two-agent
/// loop long before this). A chain starts counting again once either agent
/// in it makes progress (card #488, progress.rs).
pub const MAX_HOP: i64 = 50;
pub const UNENGAGED_LIMIT: Duration = Duration::from_secs(10 * 60);
/// How long a wake item may wait on an idle agent before Irori counts the
/// agent as not answering (card #110). An idle session that reads its
/// wakes takes one up within seconds.
pub const NOT_ANSWERING_AFTER: Duration = Duration::from_secs(5 * 60);
pub const GONE_GRACE: Duration = Duration::from_secs(3 * 60);
pub const NO_CARD_GRACE: Duration = Duration::from_secs(10 * 60);
pub const QUIET_LIMIT: Duration = Duration::from_secs(60 * 60);
pub const MAX_DEFER: Duration = Duration::from_secs(12 * 60 * 60);
pub const REATTACH_GRACE: Duration = Duration::from_secs(2 * 60);
pub const MAX_ALIASES: usize = 10;
pub const MAX_ALIAS_LEN: usize = 60;
pub const MIN_KEY_PREFIX_LEN: usize = 8;
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(60);
pub const DEFAULT_INBOX_LIST: usize = 50;
/// Mod-path wakes a delivery gets, each never confirmed, before it goes out
/// as an append instead (card #23).
pub const MAX_WAKE_TRIES: i64 = 3;
/// Caps defer/skip notes and send reasons (runes).
pub const MAX_NOTE: usize = 500;
/// Caps SendOptions.ref (runes).
pub const MAX_REF: usize = 200;
pub const MAX_ATTACHMENTS: usize = 10;
/// Each subscriber's queue size; a full reader gets [`ChangeKind::Resync`]
/// in place of the next change that fits.
pub const SUB_BUFFER: usize = 64;
/// The sender key of notices Irori itself sends.
pub const SYSTEM_KEY: &str = "irori";
/// The PermissionMode value that means permissions are bypassed.
pub const BYPASS_MODE: &str = "bypassPermissions";
/// Starts the `by` of a row the loopback API writes for an agent.
pub const API_KEY_PREFIX: &str = "api:";

/// Log kinds the inbox writes and replays (I:internal/inbox/kinds.go).
pub mod kinds {
    pub use store::kinds::{
        AGENT_ALIAS, AGENT_NAME, AGENT_SEEN, AUTONOMY_SET, DELIVERY_DECIDED, DELIVERY_STATE,
        MESSAGE_SENT, POLICY_SET, RECEIVE_SET, THREADS_SET, TURN_ENDED, TURN_ENGAGED, WAKE_ISSUED,
    };
}

/// Delivery states.
pub mod st {
    pub const QUEUED: &str = "queued";
    pub const HELD: &str = "held";
    pub const DELIVERED: &str = "delivered";
    pub const ENGAGED: &str = "engaged";
    pub const ANSWERED: &str = "answered";
    pub const DEFERRED: &str = "deferred";
    pub const SKIPPED: &str = "skipped";
    pub const REJECTED: &str = "rejected";
    pub const DISMISSED: &str = "dismissed";
    pub const MOVED: &str = "moved";

    /// Go `State.Open`.
    pub fn open(s: &str) -> bool {
        matches!(s, QUEUED | HELD | DELIVERED | ENGAGED | DEFERRED)
    }
}

/// Hold reasons.
pub mod hold {
    pub const BLOCKED: &str = "blocked";
    pub const NOT_ALLOWED: &str = "not_allowed";
    /// Retired (card #71): never set now; an old row held for it is
    /// decided again when its host starts.
    pub const BYPASS_RECEIVER: &str = "bypass_receiver";
    pub const PAUSED: &str = "paused";
    pub const HOP_LIMIT: &str = "hop_limit";
    pub const UNOWNED: &str = "unowned";
    /// Another person asked to wake an agent and the receive setting asks
    /// first (v0.5 §7.2).
    pub const MEMBER_WAKE: &str = "member_wake";
    /// Another person's row to a bypass session, or their agent's
    /// assignment, under Ask me or Closed (v0.5 §7.2, B2).
    pub const CROSS_PERSON: &str = "cross_person";
    /// Another person's lease request (v0.5 §5.4).
    pub const MEMBER_LEASE: &str = "member_lease";
    /// It came from a person removed from the board, or who left it,
    /// before it reached the agent (v0.5 §4.3).
    pub const MEMBER_REMOVED: &str = "member_removed";
    /// The person took their comment back to edit it before any agent read
    /// it (card #105). Their own hold: never asked about, never counted.
    pub const EDITING: &str = "editing";
    /// Irori picked an agent here for a card on a side this device doesn't
    /// run (card #495): a hand-out, or an ask that didn't name it. The
    /// person's Release delivers it.
    pub const OFF_SIDE: &str = "off_side";
}

/// Delivery modes.
pub mod mode {
    pub const WAKE: &str = "wake";
    pub const NEXT_TURN: &str = "next_turn";
    /// The person's stop (card #207): cuts the agent's running turn and
    /// becomes its next prompt. Never held by Pause all, never gated
    /// behind an outstanding wake; only the person sends it.
    pub const STOP: &str = "stop";
}

/// Address kinds.
pub mod addr {
    pub const AGENT: &str = "agent";
    pub const REPO: &str = "repo";
    pub const REPLY: &str = "reply";
    pub const PR: &str = "pr";
}

/// Trust levels.
pub mod trust {
    pub const UNTRUSTED: &str = "untrusted";
    pub const USER: &str = "user";
}

/// Wake paths.
pub mod path {
    pub const MOD: &str = "mod";
    pub const CHANNEL: &str = "channel";
}

/// Policy modes. `OPEN` was "Full auto" (decision 36): `everyone` without
/// the bypass_receiver hold. That hold is gone (card #71), so `OPEN` now
/// behaves as `EVERYONE`; it is still read from old rows.
pub mod policy_mode {
    pub const EVERYONE: &str = "everyone";
    pub const ALLOWLIST: &str = "allowlist";
    pub const OPEN: &str = "open";
}

/// delivery.decided choices.
pub mod choice {
    pub const RELEASE: &str = "release";
    pub const DISMISS: &str = "dismiss";
    pub const MOVE: &str = "move";
    /// Wind down turned off (card #787): a wind-down not reached yet closes.
    pub const CANCEL: &str = "cancel";
}

// ---- types ----------------------------------------------------------------

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero_time(t: &GoTime) -> bool {
    t.is_zero()
}

/// Who is sending, as the Core established it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Sender {
    pub key: String,
    pub name: String,
    /// `"claude"`, `"grok"`, `"user"` or `"irori"`.
    pub kind: String,
}

impl Sender {
    pub fn new(key: &str, name: &str, kind: &str) -> Sender {
        Sender {
            key: key.into(),
            name: name.into(),
            kind: kind.into(),
        }
    }

    /// A person at the GUI (kind `user`).
    pub fn is_user(&self) -> bool {
        self.kind == "user"
    }

    pub(crate) fn is_system(&self) -> bool {
        self.key == SYSTEM_KEY
    }
}

/// Irori's own sender.
pub fn system_sender() -> Sender {
    Sender::new(SYSTEM_KEY, "Irori", "irori")
}

/// A message's target before routing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Address {
    pub kind: String,
    pub value: String,
}

impl Address {
    pub fn new(kind: &str, value: &str) -> Address {
        Address {
            kind: kind.into(),
            value: value.into(),
        }
    }
}

/// A file on a message (blob.FileRef; merge: replace with the blob crate's).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FileRef {
    pub sha256: String,
    pub name: String,
    pub size: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub mime: String,
    pub device: String,
}

/// One sent message (the message.sent payload).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Message {
    pub id: String,
    pub from: Sender,
    pub to: Address,
    pub text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(rename = "replyTo", skip_serializing_if = "String::is_empty")]
    pub reply_to: String,
    #[serde(rename = "parentId", skip_serializing_if = "String::is_empty")]
    pub parent_id: String,
    pub hop: i64,
    pub trust: String,
    #[serde(rename = "sentAt")]
    pub sent_at: GoTime,
    #[serde(rename = "ref", skip_serializing_if = "String::is_empty")]
    pub ref_: String,
    #[serde(skip_serializing_if = "is_false")]
    pub notice: bool,
    /// The person's stop (card #207): sent with mode `stop`.
    #[serde(skip_serializing_if = "is_false")]
    pub stop: bool,
    /// Where a bridged board row came from (v0.5 §7.1); empty otherwise.
    #[serde(skip_serializing_if = "BoardFrom::is_empty")]
    pub board: BoardFrom,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<FileRef>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub mentions: Vec<Mention>,
    /// The sender had read another person's text (card #1075): the agent
    /// that reads this is flagged too, as the hop count rides on a send.
    #[serde(skip_serializing_if = "is_false")]
    pub others: bool,
    /// The device that wrote the message.sent row (memory only).
    #[serde(skip)]
    pub writer: String,
}

/// One message's journey to one recipient (the API shape,
/// I:internal/inbox/inbox.go:200-226).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Delivery {
    pub id: String,
    #[serde(rename = "messageId")]
    pub message_id: String,
    pub to: String,
    #[serde(rename = "repoKey", skip_serializing_if = "String::is_empty")]
    pub repo_key: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub mode: String,
    pub state: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub hold: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub via: String,
    #[serde(rename = "wakeId", skip_serializing_if = "String::is_empty")]
    pub wake_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(rename = "for", skip_serializing_if = "String::is_empty")]
    pub for_: String,
    /// omitempty never omits a time: the zero time is written.
    pub until: GoTime,
    #[serde(rename = "movedTo", skip_serializing_if = "String::is_empty")]
    pub moved_to: String,
    pub reminded: bool,
    #[serde(rename = "heldFor", skip_serializing_if = "String::is_empty")]
    pub held_for: String,
    #[serde(rename = "decidedBy", skip_serializing_if = "String::is_empty")]
    pub decided_by: String,
    #[serde(rename = "decidedAt", skip_serializing_if = "is_zero_time")]
    pub decided_at: GoTime,
    #[serde(rename = "createdAt")]
    pub created_at: GoTime,
    #[serde(rename = "updatedAt")]
    pub updated_at: GoTime,
    pub message: Message,
}

/// One prompt to submit, or (mode next_turn) one append.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Wake {
    pub id: String,
    pub to: String,
    pub token: String,
    pub reminder: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub mode: String,
    #[serde(rename = "deliveryIds")]
    pub delivery_ids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub overflow: Vec<String>,
    pub prompt: String,
    /// The card the wake's notes were picked for (card #412).
    #[serde(rename = "notesTask", skip_serializing_if = "String::is_empty")]
    pub notes_task: String,
    /// The accepted notes the wake carried (card #412).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<WakeNote>,
    /// The agent's card the person's words were picked for (card #555).
    #[serde(rename = "saidFor", skip_serializing_if = "String::is_empty")]
    pub said_for: String,
    /// What the person said lately on the cards around it (card #555).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub said: Vec<WakeSaid>,
}

/// SendWith's choices beyond the text.
#[derive(Debug, Clone, Default)]
pub struct SendOptions {
    pub reason: String,
    /// `""` picks the default: next_turn for the person or the system,
    /// wake for an agent.
    pub mode: String,
    pub ref_: String,
    pub notice: bool,
    pub attachments: Vec<FileRef>,
    /// The agents the message mentions (stored on its message.sent row).
    pub mentions: Vec<Mention>,
    /// A message id: the new message continues its chain (a mention's
    /// copy names the message it copies).
    pub parent: String,
}

/// The reason on the stop a card's Pause sends each holder (card #207); its
/// ref is the card.
pub const REASON_PAUSED: &str = "paused";
/// The reason on the wake a card's Resume sends each holder (card #207).
pub const REASON_RESUMED: &str = "resumed";
/// The reason on the person's Wind down all (card #220,
/// docs/design/wind-down.md): save your place, hand back your cards and
/// stop. Only the person sends it; Pause all doesn't hold it; it waits for
/// the agent's safe point (a wake, never a stop).
pub const REASON_WIND_DOWN: &str = "wind_down";

/// The reason on a mentioned agent's copy of a message (card #77).
pub const REASON_MENTIONED: &str = "mentioned";

/// One agent a message mentions (the JSON of `coord::Mention`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Mention {
    pub key: String,
    pub name: String,
    #[serde(skip_serializing_if = "is_false")]
    pub gone: bool,
}

/// The per-device wake policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Policy {
    pub mode: String,
    pub block: Vec<String>,
    pub allow: Vec<String>,
    pub paused: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            mode: policy_mode::EVERYONE.into(),
            block: vec![],
            allow: vec![],
            paused: false,
        }
    }
}

/// What a Subscribe notification says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    /// To has something to submit: call NextWake.
    WakeReady,
    /// To has next_turn items and no append outstanding.
    AppendReady,
    /// A delivery to To was held.
    HeldNew,
    /// A delivery left To.
    MovedAway,
    /// Anything else; UIs refresh.
    Updated,
    /// This reader lost changes when its queue was full: re-read
    /// everything (list-signals.md §3.1). It comes in place of the next
    /// change that fits, with empty `to` and `delivery_id`.
    Resync,
}

impl ChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeKind::WakeReady => "wake_ready",
            ChangeKind::AppendReady => "append_ready",
            ChangeKind::HeldNew => "held",
            ChangeKind::MovedAway => "moved",
            ChangeKind::Updated => "updated",
            ChangeKind::Resync => "resync",
        }
    }
}

impl Serialize for ChangeKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// A notification for transports and the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub kind: ChangeKind,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub to: String,
    #[serde(rename = "deliveryId", skip_serializing_if = "String::is_empty")]
    pub delivery_id: String,
    /// The delivery's first row folded here: it was made, or it reached
    /// this device (Talk shows an agent's message as a DM from then,
    /// list-signals.md §3.2).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub created: bool,
    /// The delivery's recipient before this row (empty: nobody yet), when
    /// an existing delivery changed recipient without moving (an assign, a
    /// release): its message leaves that DM, if any, for the new one's.
    #[serde(rename = "wasTo", skip_serializing_if = "Option::is_none")]
    pub was_to: Option<String>,
}

/// The presence fields the inbox routes against (presence.AgentCard;
/// merge: map the presence crate's card onto this, or replace it).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentCard {
    pub key: String,
    pub device_id: String,
    pub session_id: String,
    /// `"claude"`, `"grok"`.
    pub agent: String,
    pub name: String,
    /// `"gone"` cards are left out of routing.
    pub status: String,
    pub repo_key: String,
    pub permission_mode: String,
}

/// The presence the inbox routes against (core.Core / presence.Watcher).
pub trait Roster: Send + Sync {
    fn agents(&self) -> Vec<AgentCard>;
}

/// The inbox's clock.
pub type NowFn = Arc<dyn Fn() -> GoTime + Send + Sync>;
/// Options.PROwners: a PR's owner agent keys, best first.
pub type PrOwnersFn = Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>;
/// Options.IsOwner / Options.Revoked.
pub type KeyPredicate = Arc<dyn Fn(&str) -> bool + Send + Sync>;
/// Options.OnRecipientGone: the task id the message went on, or an error
/// (kind NoCard: no card here yet).
pub type OnGoneFn = Arc<dyn Fn(&Message, &str) -> Result<String> + Send + Sync>;
/// Options.OnPassed.
pub type OnPassedFn = Arc<dyn Fn(&Message, &str) -> Result<()> + Send + Sync>;
/// Options.Stale: the note to close a stale notice with, or `""`.
pub type StaleFn = Arc<dyn Fn(&Delivery) -> String + Send + Sync>;
/// Options.OffSide (card #495): whether message m to `recipient` is a
/// pick Irori made for a card on a side the recipient's device doesn't
/// run.
pub type OffSideFn = Arc<dyn Fn(&Message, &str) -> bool + Send + Sync>;
/// Options.Notes (card #412): the accepted notes for a wake to `agent`,
/// given the `task:<id>` refs of the messages in it (the first card it is
/// about), best first.
pub type NotesFn = Arc<dyn Fn(&str, &[String]) -> WakeNotes + Send + Sync>;

/// The notes one wake carries (card #412, docs/design/notes.md §6, §11),
/// recorded on its wake.issued row. Text and names are as written (other
/// agents' words); the prompt escapes them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeNotes {
    /// The card the notes were picked for (`""`: none; Mine's
    /// everywhere, machine and repo notes only).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub task: String,
    pub notes: Vec<WakeNote>,
    /// The agent's own card, by its short ref, when `said` isn't empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub said_for: String,
    /// What the person said lately on the cards around it (card #555).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub said: Vec<WakeSaid>,
}

/// One thing the person said on a card near the agent's (card #555).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeSaid {
    /// The comment's id (no `comment:` prefix).
    pub id: String,
    /// The card it is on, by its short ref.
    pub card: String,
    pub text: String,
}

/// One note in a wake.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeNote {
    /// The note's id (no `note:` prefix).
    pub id: String,
    /// The tag that matched.
    pub why: String,
    pub kind: String,
    pub text: String,
    /// The author and the accepter, as names.
    pub by: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub accepted_by: String,
}

/// Configures [`Inbox::new`] (I:internal/inbox/inbox.go:340-378).
#[derive(Clone, Default)]
pub struct Options {
    pub device_id: String,
    /// Default: the system clock. Merge note: Go's `time.Now()` is in the
    /// local zone, so Go rows carry the local offset; pass a local-offset
    /// clock from Core to write the same bytes (UTC here otherwise).
    pub now: Option<NowFn>,
    pub pr_owners: Option<PrOwnersFn>,
    /// Default: exactly `"user:" + device_id`.
    pub is_owner: Option<KeyPredicate>,
    /// Runs without the inbox lock held.
    pub on_recipient_gone: Option<OnGoneFn>,
    /// Runs without the inbox lock held.
    pub on_passed: Option<OnPassedFn>,
    /// Runs under the inbox lock: it must not call the inbox.
    pub stale: Option<StaleFn>,
    /// Runs under the inbox lock: it must not call the inbox, nor wait on
    /// anything that may be waiting on the inbox (a shared board's coord
    /// is: its writes read the hop here).
    pub notes: Option<NotesFn>,
    /// Whether Irori started agent `key` for one card (card #446): it is
    /// never a repo's front desk. Runs under the inbox lock.
    pub spawned: Option<KeyPredicate>,
    /// Holds a pick Irori made across the work/personal line (card #495).
    /// Runs under the inbox lock: it must not call the inbox.
    pub off_side: Option<OffSideFn>,
    /// Test overrides; zero means the constant.
    pub unengaged_limit: Duration,
    pub gone_grace: Duration,
    pub quiet_limit: Duration,
    pub reattach_grace: Duration,
    pub no_card_grace: Duration,
}

impl fmt::Debug for Options {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Options")
            .field("device_id", &self.device_id)
            .finish_non_exhaustive()
    }
}
