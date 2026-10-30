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
mod fence;
mod ids;
mod mesh;
mod messages;
pub mod notice;
mod policy;
mod pr;
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
pub use fence::{batch, fence, fence_prompt};
pub use ids::IdGen;
pub use pr::normalize_pr;
pub use state::{Inbox, Subscription};

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

pub const MAX_TEXT: usize = 8000;
pub const MAX_WAKE_BYTES: usize = 16 << 10;
pub const DIRECT_LIMIT: usize = 8;
pub const DIRECT_WINDOW: Duration = Duration::from_secs(10 * 60);
/// The runaway-loop guard: a message this many replies down one chain of
/// agent messages is held (card #71 raised it from Go's 6, which held
/// ordinary back-and-forth; the per-pair rate limit bounds a two-agent
/// loop long before this).
pub const MAX_HOP: i64 = 50;
pub const UNENGAGED_LIMIT: Duration = Duration::from_secs(10 * 60);
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
/// Each subscriber's queue size; a full queue drops the change.
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
        AGENT_ALIAS, AGENT_NAME, AGENT_SEEN, DELIVERY_DECIDED, DELIVERY_STATE, MESSAGE_SENT,
        POLICY_SET, TURN_ENDED, TURN_ENGAGED, WAKE_ISSUED,
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
}

/// Delivery modes.
pub mod mode {
    pub const WAKE: &str = "wake";
    pub const NEXT_TURN: &str = "next_turn";
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<FileRef>,
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
}

impl ChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeKind::WakeReady => "wake_ready",
            ChangeKind::AppendReady => "append_ready",
            ChangeKind::HeldNew => "held",
            ChangeKind::MovedAway => "moved",
            ChangeKind::Updated => "updated",
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
