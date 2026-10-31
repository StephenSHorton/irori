//! The Board's public data types, in Go's JSON shapes (coord.md §2.3).
//! Field order and `omitempty` are Go's: the GUI and API read these, and
//! the replay goldens compare their bytes with Go's.

use gocompat::time::GoTime;
use serde::Serialize;

use crate::gojson::omit;

/// Task states (an A2A subset plus `blocked`).
pub type TaskState = String;
pub const SUBMITTED: &str = "submitted";
pub const WORKING: &str = "working";
pub const INPUT_REQUIRED: &str = "input_required";
pub const COMPLETED: &str = "completed";
pub const FAILED: &str = "failed";
pub const CANCELED: &str = "canceled";
pub const BLOCKED: &str = "blocked";

/// Whether `s` closes a task.
pub fn is_terminal(s: &str) -> bool {
    s == COMPLETED || s == FAILED || s == CANCELED
}

/// Whether `s` is one of the seven states.
pub fn valid_state(s: &str) -> bool {
    matches!(
        s,
        SUBMITTED | WORKING | INPUT_REQUIRED | COMPLETED | FAILED | CANCELED | BLOCKED
    )
}

crate::go_struct! {
    /// A file on a comment or blocker (Go `blob.FileRef`).
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct FileRef {
        pub sha256: String = "sha256",
        pub name: String = "name",
        pub size: i64 = "size",
        pub mime: String = "mime" "omit",
        pub device: String = "device",
    }
}

/// What a card's link points at.
pub const LINK_URL: &str = "url";
pub const LINK_PATH: &str = "path";
pub const LINK_COMMIT: &str = "commit";

crate::go_struct! {
    /// One link out from a card.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Link {
        pub kind: String = "kind",
        pub reference: String = "ref",
        pub label: String = "label",
    }
}

/// What a blocked task waits on.
pub const BLOCKED_ON_LEASE: &str = "lease";
pub const BLOCKED_ON_TASK: &str = "task";
pub const BLOCKED_ON_PERSON: &str = "person";
pub const BLOCKED_ON_AGENT: &str = "agent";
pub const BLOCKED_ON_EXTERNAL: &str = "external";

crate::go_struct! {
    /// What is in the way.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct BlockedBy {
        pub kind: String = "kind",
        pub reference: String = "ref" "omit",
    }
}

crate::go_struct! {
    /// One ordered step of a task.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Step {
        pub id: String = "id",
        pub text: String = "text",
        pub done: bool = "done",
        pub doing: bool = "doing" "omit",
    }
}

crate::go_struct! {
    /// One answer to a blocker (Go `coord.Option`).
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct BlockerOption {
        pub answer: String = "answer",
        pub why: String = "why" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
        pub recommended: bool = "recommended" "omit",
        pub link: Option<Link> = "link" "omit",
    }
}

/// How a blocker item was resolved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Resolution {
    pub answer: String,
    /// The picked option's index, or -1 for own words (and a close).
    pub option: i64,
    pub by: String,
    pub at: GoTime,
    #[serde(skip_serializing_if = "omit")]
    pub closed: bool,
    #[serde(skip_serializing_if = "omit")]
    pub note: String,
}

/// Where an open blocker item stands (derived, never stored).
pub const BLOCKER_OPEN: &str = "open";
pub const BLOCKER_ANSWERED: &str = "answered";
pub const BLOCKER_REPLIED: &str = "replied";

/// A row's position in the log: `(hlc, device, seq)`.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RowPos {
    pub hlc: u64,
    pub dev: String,
    pub seq: i64,
}

impl RowPos {
    pub fn of(ev: &store::Event) -> RowPos {
        RowPos {
            hlc: ev.hlc.0,
            dev: ev.device_id.clone(),
            seq: ev.seq,
        }
    }

    pub fn before(&self, b: &RowPos) -> bool {
        self < b
    }
}

/// One item of what blocks a task.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Blocker {
    pub id: String,
    pub question: String,
    #[serde(skip_serializing_if = "omit")]
    pub context: String,
    pub raised_by: String,
    pub at: GoTime,
    #[serde(skip_serializing_if = "omit")]
    pub options: Vec<BlockerOption>,
    #[serde(skip_serializing_if = "omit")]
    pub resolved: Option<Resolution>,
    #[serde(skip_serializing_if = "omit")]
    pub links: Vec<Link>,
    #[serde(skip_serializing_if = "omit")]
    pub attachments: Vec<FileRef>,
    #[serde(skip_serializing_if = "omit")]
    pub on: Option<BlockedBy>,
    #[serde(skip_serializing_if = "omit")]
    pub status: String,
    #[serde(skip_serializing_if = "omit")]
    pub sent_to: String,
    #[serde(skip_serializing_if = "omit")]
    pub missing: Vec<String>,
    /// The row's reason: Task.reason while this is the first open item.
    #[serde(skip)]
    pub(crate) reason: String,
    /// Folded from a reason-only row: a later one replaces it.
    #[serde(skip)]
    pub(crate) legacy: bool,
    /// The position of the row that last replaced the question.
    #[serde(skip)]
    pub(crate) asked: RowPos,
    /// The position of the revision that last replaced the options (card
    /// #50): a recommendation logged before it was for options gone now.
    #[serde(skip)]
    pub(crate) options_at: RowPos,
}

impl Blocker {
    /// Whether the item still waits on an answer.
    pub fn open(&self) -> bool {
        self.resolved.is_none()
    }
}

crate::go_struct! {
    /// How an ask ended.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Outcome {
        pub kind: String = "kind",
        pub task: String = "task" "omit",
        pub note: String = "note",
    }
}

/// An ask's state on its card (Go `coord.Ask`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Ask {
    #[serde(skip_serializing_if = "omit")]
    pub wake: bool,
    #[serde(rename = "repoKey", skip_serializing_if = "omit")]
    pub repo_key: String,
    pub owner: String,
    #[serde(skip_serializing_if = "omit")]
    pub taker: String,
    #[serde(skip_serializing_if = "omit")]
    pub reason: String,
    pub told: i64,
    #[serde(skip_serializing_if = "omit")]
    pub heard: Vec<String>,
    pub since: GoTime,
    #[serde(skip_serializing_if = "omit")]
    pub unclaimed: bool,
    #[serde(skip_serializing_if = "omit")]
    pub waiting: Vec<String>,
    #[serde(skip_serializing_if = "omit")]
    pub outcome: Option<Outcome>,
}

impl Ask {
    /// Whether the ask still runs under the take lock.
    pub fn is_open(&self) -> bool {
        self.outcome.is_none()
    }
}

/// Who went away from a card and who was told.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Returned {
    pub was: String,
    pub at: GoTime,
    #[serde(skip_serializing_if = "omit")]
    pub row: String,
    #[serde(skip_serializing_if = "omit")]
    pub person: bool,
}

/// A parent's progress and state from its subtasks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rollup {
    pub state: TaskState,
    pub done: i64,
    pub total: i64,
    #[serde(skip_serializing_if = "omit")]
    pub blocked: String,
    #[serde(skip_serializing_if = "omit")]
    pub blocked_question: String,
}

/// The folded view of one task.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    #[serde(skip_serializing_if = "omit")]
    pub short: String,
    #[serde(skip_serializing_if = "omit")]
    pub short_was: String,
    #[serde(skip_serializing_if = "omit")]
    pub tag: String,
    #[serde(skip_serializing_if = "omit")]
    pub withdrawn: bool,
    pub title: String,
    pub objective: String,
    #[serde(skip_serializing_if = "omit")]
    pub done_criteria: String,
    #[serde(skip_serializing_if = "omit")]
    pub repo_key: String,
    #[serde(skip_serializing_if = "omit")]
    pub scope: Vec<String>,
    pub state: TaskState,
    #[serde(skip_serializing_if = "omit")]
    pub holder: String,
    #[serde(skip_serializing_if = "omit")]
    pub holders: Vec<String>,
    #[serde(skip_serializing_if = "omit")]
    pub assigned: Vec<String>,
    #[serde(skip_serializing_if = "omit")]
    pub shared: bool,
    pub created_by: String,
    #[serde(skip_serializing_if = "omit")]
    pub note: String,
    pub created_at: GoTime,
    pub updated_at: GoTime,
    #[serde(skip_serializing_if = "omit")]
    pub updated_by: String,
    #[serde(skip_serializing_if = "omit")]
    pub reason: String,
    #[serde(skip_serializing_if = "omit")]
    pub blocked_by: Option<BlockedBy>,
    #[serde(skip_serializing_if = "omit")]
    pub links: Vec<Link>,
    #[serde(skip_serializing_if = "omit")]
    pub comments: i64,
    #[serde(skip_serializing_if = "omit")]
    pub steps: Vec<Step>,
    #[serde(skip_serializing_if = "omit")]
    pub blockers: Vec<Blocker>,
    #[serde(skip_serializing_if = "omit")]
    pub watchers: Vec<String>,
    #[serde(skip_serializing_if = "omit")]
    pub ask: Option<Ask>,
    #[serde(skip_serializing_if = "omit")]
    pub returned: Option<Returned>,
    #[serde(skip_serializing_if = "omit")]
    pub parent: String,
    #[serde(skip_serializing_if = "omit")]
    pub subtasks: Vec<String>,
    #[serde(skip_serializing_if = "omit")]
    pub rollup: Option<Rollup>,
    #[serde(skip_serializing_if = "omit")]
    pub pinned: bool,
    #[serde(skip_serializing_if = "omit")]
    pub pin_rank: f64,
}

impl Task {
    /// How a card is named: `#12`, `mlg#12` on a shared board, or
    /// `task:<id>` before the fold numbered it.
    pub fn short_ref(&self) -> String {
        if self.short.is_empty() {
            return format!("task:{}", self.id);
        }
        format!("{}#{}", self.tag, self.short)
    }

    /// The short ref then the full one: `#12, task:<id>`.
    pub fn refs(&self) -> String {
        if self.short.is_empty() {
            return format!("task:{}", self.id);
        }
        format!("{}, task:{}", self.short_ref(), self.id)
    }

    /// Done steps over all steps.
    pub fn progress(&self) -> (i64, i64) {
        let done = self.steps.iter().filter(|s| s.done).count() as i64;
        (done, self.steps.len() as i64)
    }

    /// The items still waiting, oldest first.
    pub fn open_blockers(&self) -> Vec<Blocker> {
        self.blockers.iter().filter(|b| b.open()).cloned().collect()
    }

    /// The ask while the take lock applies.
    pub fn open_ask(&self) -> Option<&Ask> {
        self.ask.as_ref().filter(|a| a.is_open())
    }

    /// Where an ask stands, for show and the board tool (`""` for a card
    /// that isn't one).
    pub fn ask_line(&self) -> String {
        let Some(a) = &self.ask else {
            return String::new();
        };
        if let Some(o) = &a.outcome {
            let mut s = format!("closed ({}): {}", o.kind, o.note);
            if !o.task.is_empty() {
                s.push_str(&format!(" (task:{})", o.task));
            }
            return s;
        }
        if !a.taker.is_empty() {
            return format!(
                "the person asked the room; {} took it (ask_close ends it with what it became)",
                a.taker
            );
        }
        if !a.waiting.is_empty() {
            return format!(
                "the person asked the room; waiting for device {} to hand it out",
                a.owner
            );
        }
        if a.unclaimed {
            return format!(
                "the person asked the room; nobody took it yet ({} told): ask_claim takes it",
                a.told
            );
        }
        format!(
            "the person asked the room; {} agents told, nobody has taken it: ask_claim takes it",
            a.told
        )
    }
}

/// A task's state as readers show it: a parent's comes from its subtasks.
pub fn effective_state(t: &Task) -> &str {
    match &t.rollup {
        Some(r) => &r.state,
        None => &t.state,
    }
}

/// CreateTask's input.
#[derive(Debug, Clone, Default)]
pub struct NewTask {
    pub title: String,
    pub objective: String,
    pub done_criteria: String,
    pub repo_key: String,
    pub scope: Vec<String>,
    pub shared: bool,
    /// Each parsed by `parse_link`.
    pub links: Vec<String>,
    /// Creates the task already blocked.
    pub block_reason: String,
    pub blocked_by: Option<BlockedBy>,
    /// The first step list (ids are minted s1, s2, ...).
    pub steps: Vec<Step>,
    /// A short ref or task id: makes the task a subtask of that card.
    pub parent: String,
}

/// Narrows `list_tasks`; zero values mean any.
#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    pub repo_key: String,
    /// Matches any of holders.
    pub holder: String,
    /// Only non-terminal states.
    pub open: bool,
}

/// An advisory claim on a resource.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lease {
    pub resource: String,
    pub holder: String,
    pub until: GoTime,
    #[serde(skip_serializing_if = "omit")]
    pub stolen_from: String,
    pub acquired_at: GoTime,
    #[serde(skip_serializing_if = "omit")]
    pub conflict: String,
}

/// What a Subscribe change is about.
pub const TASK_CHANGED: &str = "task";
pub const LEASE_CHANGED: &str = "lease";
pub const LEASE_EXPIRED: &str = "lease_expired";
pub const LEASE_REFUSED: &str = "lease_refused";
pub const COMMENT_ADDED: &str = "comment";
pub const RESYNC: &str = "resync";

/// A notification for transports and the UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub kind: String,
    #[serde(skip_serializing_if = "omit")]
    pub task_id: String,
    #[serde(skip_serializing_if = "omit")]
    pub resource: String,
    #[serde(skip_serializing_if = "omit")]
    pub holder: String,
    #[serde(skip_serializing_if = "omit")]
    pub reason: String,
}

/// One task.comment row as folded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    pub task_id: String,
    pub by: String,
    pub text: String,
    pub wake: bool,
    pub at: GoTime,
    #[serde(skip_serializing_if = "omit")]
    pub attachments: Vec<FileRef>,
    #[serde(skip_serializing_if = "omit")]
    pub to: String,
    #[serde(skip_serializing_if = "omit")]
    pub msg_id: String,
    #[serde(skip_serializing_if = "omit")]
    pub gone: bool,
    #[serde(skip_serializing_if = "omit")]
    pub held: bool,
    #[serde(skip_serializing_if = "omit")]
    pub files: i64,
    #[serde(skip_serializing_if = "omit")]
    pub blocker_id: String,
    #[serde(skip_serializing_if = "omit")]
    pub pick: i64,
    #[serde(skip_serializing_if = "omit")]
    pub missing: Vec<String>,
    #[serde(skip_serializing_if = "omit")]
    pub unheard: bool,
    pub device: String,
    #[serde(skip)]
    pub hlc: u64,
    #[serde(skip)]
    pub seq: i64,
    #[serde(skip)]
    pub(crate) primary: bool,
    #[serde(skip)]
    pub(crate) passed: String,
    #[serde(skip)]
    pub(crate) person: bool,
}

impl Comment {
    pub(crate) fn pos(&self) -> RowPos {
        RowPos {
            hlc: self.hlc,
            dev: self.device.clone(),
            seq: self.seq,
        }
    }
}

/// An inbox message about a card, for `record_message`.
#[derive(Debug, Clone, Default)]
pub struct MessageEntry {
    pub task_id: String,
    pub msg_id: String,
    pub from: String,
    pub to: String,
    pub text: String,
    pub files: i64,
    pub held: bool,
    pub gone: bool,
    pub fill: bool,
    /// `irori` writes the row as Irori itself; else as `from`.
    pub writer: String,
}

/// Why a Notice exists.
pub const NOTICE_ASSIGNED: &str = "assigned";
pub const NOTICE_COMMENT: &str = "comment";
pub const NOTICE_UNBLOCKED: &str = "unblocked";
pub const NOTICE_RETURNED: &str = "returned";
pub const NOTICE_REOPENED: &str = "reopened";
pub const NOTICE_ANSWERED: &str = "answered";
pub const NOTICE_WATCHED: &str = "watched";
pub const NOTICE_ASK: &str = "ask";
pub const NOTICE_TAKEN: &str = "taken";
pub const NOTICE_ASK_COMMENT: &str = "ask_comment";

/// Something coord wants agents to hear; the caller routes it once per
/// recipient (coord.md §1.1).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub kind: String,
    pub task_id: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub title: String,
    #[serde(skip_serializing_if = "omit")]
    pub to: Vec<String>,
    pub by: String,
    pub text: String,
    pub wake: bool,
    #[serde(skip_serializing_if = "omit")]
    pub attachments: Vec<FileRef>,
    #[serde(skip_serializing_if = "omit")]
    pub watching: bool,
    #[serde(skip_serializing_if = "omit")]
    pub person: bool,
}

crate::go_struct! {
    /// One lease.refused row.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Refusal {
        pub resource: String = "resource",
        pub by: String = "by",
        pub held_by: String = "heldBy",
        pub held: String = "held",
        pub until: GoTime = "until",
        pub at: GoTime = "at",
        pub conflict: bool = "conflict" "omit",
    }
}

/// Narrows `board`.
#[derive(Debug, Clone, Default)]
pub struct BoardFilter {
    pub repo_key: String,
    /// The viewer (`user:*` or an agent key).
    pub viewer: String,
    /// 0 = DEFAULT_DONE_CARD, at most MAX_DONE_CARD.
    pub done_limit: i64,
}

/// One task on the Board (Go embeds Task, so its fields come first).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Card {
    pub task: Task,
    pub last_comment: Option<Comment>,
    pub resolved: bool,
    pub gone: Vec<String>,
    pub overlaps: Vec<String>,
}

impl Serialize for Card {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut extra = crate::gojson::Extra::default();
        if let Some(c) = &self.last_comment {
            extra.field("lastComment", c)?;
        }
        if self.resolved {
            extra.field("resolved", &true)?;
        }
        if !self.gone.is_empty() {
            extra.field("gone", &self.gone)?;
        }
        if !self.overlaps.is_empty() {
            extra.field("overlaps", &self.overlaps)?;
        }
        extra.embed(s, &self.task)
    }
}

/// A lease shown on the Board because it blocks something.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeaseCard {
    pub lease: Lease,
    pub refused: Vec<Refusal>,
    pub blocks: Vec<String>,
}

impl Serialize for LeaseCard {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut extra = crate::gojson::Extra::default();
        if !self.refused.is_empty() {
            extra.field("refused", &self.refused)?;
        }
        if !self.blocks.is_empty() {
            extra.field("blocks", &self.blocks)?;
        }
        extra.embed(s, &self.lease)
    }
}

/// The Board, grouped (every array non-null).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardView {
    pub needs_you: Vec<Card>,
    pub blocked: Vec<Card>,
    pub working: Vec<Card>,
    pub open: Vec<Card>,
    pub done: Vec<Card>,
    pub done_total: i64,
    pub leases: Vec<LeaseCard>,
}

/// One line of what happened on a card.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub at: GoTime,
    pub by: String,
    pub what: String,
    pub kind: String,
    pub device: String,
    #[serde(skip_serializing_if = "omit")]
    pub blocker_id: String,
    /// The line of a blocker revision: it belongs in the blocker's thread.
    #[serde(skip)]
    pub revised: bool,
    #[serde(skip)]
    pub hlc: u64,
    #[serde(skip)]
    pub seq: i64,
}

impl Activity {
    pub(crate) fn pos(&self) -> RowPos {
        RowPos {
            hlc: self.hlc,
            dev: self.device.clone(),
            seq: self.seq,
        }
    }
}

/// Raise's input.
#[derive(Debug, Clone, Default)]
pub struct NewBlocker {
    pub question: String,
    pub context: String,
    pub options: Vec<BlockerOption>,
    pub blocked_by: Option<BlockedBy>,
    pub links: Vec<String>,
    pub files: Vec<FileRef>,
}

/// Revise's input: what an open blocker asks now.
#[derive(Debug, Clone, Default)]
pub struct BlockerRevision {
    /// The new question; empty keeps the current one.
    pub question: String,
    /// The new context; `None` keeps the current one, `Some("")` drops it.
    pub context: Option<String>,
    /// The choices now; empty means none (a plain question).
    pub options: Vec<BlockerOption>,
}

/// Answer's input.
#[derive(Debug, Clone, Default)]
pub struct BlockerAnswer {
    /// The 1-based option picked, or 0 for `text`.
    pub pick: i64,
    pub text: String,
    pub wake: bool,
    pub resolve: bool,
}

/// CommentOn's input.
#[derive(Debug, Clone, Default)]
pub struct CommentInput {
    pub text: String,
    pub wake: bool,
    pub files: Vec<FileRef>,
    pub blocker_id: String,
    pub missing: Vec<String>,
}

/// SetStep's input; `None` leaves a field alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StepUpdate {
    pub done: Option<bool>,
    pub doing: Option<bool>,
}

/// PostAsk's input.
#[derive(Debug, Clone, Default)]
pub struct NewAsk {
    pub text: String,
    pub files: Vec<FileRef>,
    pub repo_key: String,
    pub wake: bool,
}

/// ask_claim's answer.
#[derive(Debug, Clone, Default)]
pub struct ClaimResult {
    pub task: Task,
    pub taker: String,
    pub mine: bool,
    pub pending: bool,
    pub owner: String,
}

/// ask_close's input.
#[derive(Debug, Clone, Default)]
pub struct AskOutcome {
    pub kind: String,
    pub note: String,
    pub title: String,
    pub objective: String,
    pub done_when: String,
    pub steps: Vec<Step>,
    pub task: String,
    pub parent: String,
}

/// One subtask Split makes.
#[derive(Debug, Clone, Default)]
pub struct NewSubtask {
    pub title: String,
    pub objective: String,
    pub done_criteria: String,
    /// Step ids of the card being split (they move) or new step texts.
    pub steps: Vec<String>,
}

/// What a shared board's fold set aside: rows naming something the log
/// doesn't hold yet, those dropped past the cap, and rows the writer
/// rules ignored, by writing device.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAside {
    pub unresolved: usize,
    pub dropped: usize,
    pub ignored: usize,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub ignored_by: std::collections::BTreeMap<String, usize>,
}
