//! The Board's public data types, in Go's JSON shapes (coord.md §2.3).
//! Field order and `omitempty` are Go's: the GUI and API read these, and
//! the replay goldens compare their bytes with Go's.

use std::collections::BTreeMap;

use gocompat::time::GoTime;
use serde::Serialize;

use crate::gojson::omit;

/// Task states (an A2A subset plus `blocked` and `in_review`).
pub type TaskState = String;
pub const SUBMITTED: &str = "submitted";
pub const WORKING: &str = "working";
pub const INPUT_REQUIRED: &str = "input_required";
pub const COMPLETED: &str = "completed";
pub const FAILED: &str = "failed";
pub const CANCELED: &str = "canceled";
pub const BLOCKED: &str = "blocked";
/// The holder's work is done and waits on a review of its PR or work
/// (card #191): set by the holder's board_review, or board_status; a
/// reviewer's changes put it back to working; the holder then completes.
pub const IN_REVIEW: &str = "in_review";

/// Whether `s` closes a task.
pub fn is_terminal(s: &str) -> bool {
    s == COMPLETED || s == FAILED || s == CANCELED
}

/// Whether `s` is one of the eight states.
pub fn valid_state(s: &str) -> bool {
    matches!(
        s,
        SUBMITTED | WORKING | INPUT_REQUIRED | COMPLETED | FAILED | CANCELED | BLOCKED | IN_REVIEW
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

crate::go_struct! {
    /// One agent a comment, ask, answer or message names (card #77).
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Mention {
        /// The agent's key in the log's own form (agent:<cardId> on a board).
        pub key: String = "key",
        /// The name as it was written, for the @name in the text.
        pub name: String = "name",
        /// Gone when written: stored, never delivered.
        pub gone: bool = "gone" "omit",
    }
}

/// A next-turn reading's notice kind (card #391): what an agent hasn't
/// read in a conversation; a newer one replaces one still waiting.
pub const NOTICE_READING: &str = "reading";

/// A mention's notice kind, and the most mentions one write carries.
pub const NOTICE_MENTIONED: &str = "mentioned";
pub const MAX_MENTIONS: usize = 10;

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
/// A least app version on named devices (card #485): the sweep closes it
/// once each of them has started with that version or a later one.
pub const BLOCKED_ON_VERSION: &str = "version";

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
        /// The agent doing this step, a co-holder of the card while the
        /// step is open (card #86).
        pub owner: String = "owner" "omit",
        /// The card's lead asks Irori for a helper on this step (spawned
        /// agents §3). Only the card's author or the author's agents may
        /// set it; once set it stays.
        pub spawn: bool = "spawn" "omit",
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
    /// The person answered in their own words since these options were
    /// offered (card #90): they are set aside, kept for the record, until
    /// a revision offers new ones.
    #[serde(skip_serializing_if = "omit")]
    pub options_aside: bool,
    /// A done check (card #112): the agent says it's done; only the
    /// person's Done closes the card, and their words send it back.
    #[serde(skip_serializing_if = "omit")]
    pub done_check: bool,
    /// An ownership offer or request (card #432): Accept moves the card to
    /// the other person, Decline leaves it with its owner.
    #[serde(skip_serializing_if = "omit")]
    pub transfer: bool,
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

    /// The GitHub pull request an open external wait is on (card #509):
    /// its ref when that is a PR URL, else the first PR among its links.
    /// Such a wait is the person's merge, so it counts in Needs you.
    pub fn merge_wait(&self) -> Option<String> {
        let on = self.on.as_ref()?;
        if !self.open() || on.kind != BLOCKED_ON_EXTERNAL {
            return None;
        }
        let from_ref = super::links::parse_link(&on.reference).ok();
        from_ref
            .iter()
            .chain(self.links.iter())
            .find(|l| super::links::pr_link(l).is_some())
            .map(|l| l.reference.clone())
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

/// A shared card's owner changing hands, waiting on the other side
/// (card #432).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerTransfer {
    /// `offer` (the owner offers it to `person`) or `request` (`person`
    /// asks the owner for it).
    pub op: String,
    /// Who would own it: a person id.
    pub person: String,
    /// The Accept/Decline item it raised.
    #[serde(skip_serializing_if = "omit")]
    pub blocker_id: String,
}

/// Where Irori's hand-out of an unheld card stands (card #82): it goes to
/// one free agent at a time, and on to the next only on an event.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    /// The agent it went to; empty while no agent is free.
    #[serde(skip_serializing_if = "omit")]
    pub to: String,
    pub reason: String,
    /// When it went to `to` (or began to wait).
    pub since: GoTime,
    /// Every agent it went to so far.
    #[serde(skip_serializing_if = "omit")]
    pub heard: Vec<String>,
}

/// An ask's state on its card (Go `coord.Ask`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Ask {
    /// On a thread, its "Agents answer now" (card #391): off, no agent is
    /// woken or handed it. Every other ask wakes.
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
    /// Irori handed it to taker, which hasn't acted on it yet: it moves on
    /// if taker passes, ends its turn without touching it, or goes away.
    #[serde(skip_serializing_if = "omit")]
    pub handed: bool,
    #[serde(skip_serializing_if = "omit")]
    pub waiting: Vec<String>,
    #[serde(skip_serializing_if = "omit")]
    pub outcome: Option<Outcome>,
    /// The agents the ask went to; empty: the room.
    #[serde(skip_serializing_if = "omit")]
    pub mentions: Vec<Mention>,
    /// A thread (card #116): a conversation with the room, not a card. It
    /// stays open until the person resolves it.
    #[serde(skip_serializing_if = "omit")]
    pub thread: bool,
    /// A private thread about this shared board (card #185): in the
    /// person's own log, heard only by their own agents.
    #[serde(skip_serializing_if = "omit")]
    pub board: String,
    /// The Talk channel a thread was started from (a message there, card
    /// #454): its `ch_` id. Its newest lines go in a spawned agent's brief.
    #[serde(skip_serializing_if = "omit")]
    pub channel: String,
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
    /// The board in its space (card #392): empty for the space's own board.
    #[serde(skip_serializing_if = "omit")]
    pub board: String,
    /// The work/personal side it's on (card #494): only devices that run
    /// this side pick it up on their own. Empty: any side.
    #[serde(skip_serializing_if = "omit")]
    pub side: String,
    /// The side its task.created gave it: `side` while it has no parent;
    /// a subtask's `side` is its parent's (relink).
    #[serde(skip)]
    pub own_side: String,
    /// Why it waits on its side (card #495): "no device here runs Work"
    /// while the device that picks an agent for it doesn't run its side.
    /// Set when read (get_task, list_tasks, board), never folded.
    #[serde(skip_serializing_if = "omit")]
    pub side_wait: String,
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
    /// Hand-off notes by assignee, kept while they're offered the card and
    /// haven't taken it up (card #85).
    #[serde(skip_serializing_if = "omit")]
    pub handoffs: BTreeMap<String, String>,
    /// Agents that own an open step and don't hold the card: they hear it
    /// like a holder until their steps are done (card #86). Derived.
    #[serde(skip_serializing_if = "omit")]
    pub co_holders: Vec<String>,
    #[serde(skip_serializing_if = "omit")]
    pub shared: bool,
    pub created_by: String,
    /// The person who owns it on a shared board (card #432): decides Done
    /// and answers its questions. Empty: its creator's person.
    #[serde(skip_serializing_if = "omit")]
    pub owner: String,
    /// An ownership offer or request waiting on the other side (card #432).
    #[serde(skip_serializing_if = "omit")]
    pub owner_transfer: Option<OwnerTransfer>,
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
    /// Irori's hand-out of this unheld card (card #82).
    #[serde(skip_serializing_if = "omit")]
    pub offer: Option<Offer>,
    /// Its spawn requests, one per part (card #446): the card's own and
    /// any for a step or subtask.
    #[serde(skip)]
    pub spawns: Vec<super::spawn::Spawn>,
    /// How many of its spawned agents went away in a row without
    /// finishing (card #447): a crash. Two stop its spawning until the
    /// person says to start another; a person's state row clears it.
    #[serde(skip)]
    pub spawn_crashes: u32,
    /// Where the holder's work is (card #137), kept after it stops so
    /// whoever takes the card over can find it.
    #[serde(rename = "where", skip_serializing_if = "omit")]
    pub where_: Option<Where>,
    /// The holder's last line on what's done and what's next (card #137).
    #[serde(skip_serializing_if = "omit")]
    pub left_off: Option<LeftOff>,
    #[serde(skip_serializing_if = "omit")]
    pub returned: Option<Returned>,
    /// Who let it go last and when: its last holder left or went away
    /// (card #338). A hand-out from before then is stale, so the card
    /// goes out again as if just posted.
    #[serde(skip)]
    pub let_go: Option<(String, GoTime)>,
    /// Who left the card and hasn't come back on it (card #481): an
    /// answered blocker that went to one of them has nobody to act on it.
    #[serde(skip)]
    pub left: Vec<String>,
    #[serde(skip_serializing_if = "omit")]
    pub parent: String,
    /// A helper's part (spawned agents §3): the subtask was split off
    /// marked `spawn` by the parent's author or the author's agent. Its
    /// agent's completed goes to the parent's holder (the lead), not to
    /// the person.
    #[serde(skip_serializing_if = "omit")]
    pub spawn: bool,
    /// Its own "Up to N agents" (spawned agents §3): 0 for the board's
    /// Agents per card, 1 for the lead alone.
    #[serde(skip_serializing_if = "omit")]
    pub agents: i64,
    #[serde(skip_serializing_if = "omit")]
    pub subtasks: Vec<String>,
    /// The thread this card was made from (card #155).
    #[serde(skip_serializing_if = "omit")]
    pub from: String,
    /// On a thread: the cards made or changed from it, first first.
    /// Derived from the cards' rows.
    #[serde(skip_serializing_if = "omit")]
    pub made: Vec<String>,
    #[serde(skip_serializing_if = "omit")]
    pub rollup: Option<Rollup>,
    #[serde(skip_serializing_if = "omit")]
    pub pinned: bool,
    #[serde(skip_serializing_if = "omit")]
    pub pin_rank: f64,
    /// The person paused the card (card #207): its holders' tools are off
    /// until Resume. Copied on at read time, like the pin.
    #[serde(skip_serializing_if = "omit")]
    pub paused: bool,
}

impl Task {
    /// Every open item the person answered (card #204): the card waits on
    /// its holder acting on the answers, so it reads Working, not blocked on
    /// them. False with no open item.
    pub fn all_answered(&self) -> bool {
        let mut open = self.blockers.iter().filter(|b| b.open()).peekable();
        open.peek().is_some() && open.all(|b| b.status == BLOCKER_ANSWERED)
    }

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
        if a.thread {
            return match a.reason.as_str() {
                "answered" => "the person's thread, answered: it stays open until the person resolves it; answer their follow-ups with board_comment".into(),
                _ if !a.taker.is_empty() => format!(
                    "the person's thread; Irori handed it to {}: answer it with board_comment (board_leave passes it on)",
                    a.taker
                ),
                _ => format!(
                    "the person's thread, not answered yet ({} told): answer it with board_comment if you can",
                    a.told
                ),
            };
        }
        if !a.taker.is_empty() && a.handed {
            return format!(
                "the person asked the room; Irori handed it to {}, who holds it (ask_close ends it with what it became; board_leave passes it on)",
                a.taker
            );
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
    /// A thread's short ref or task id: the card is made from it (card
    /// #155). Empty: the one open thread the caller answered, if any.
    pub from: String,
    /// On a space (card #392): another board in it, by id; empty for the
    /// space's own board.
    pub board: String,
    /// "Up to N agents" (spawned agents §3): 0 for the board's Agents per
    /// card, 1 for the lead alone.
    pub agents: i64,
    /// The work/personal side (card #494). Empty: a subtask's parent's,
    /// else the side of this device (the first it runs), else none.
    /// Ignored on a shared board.
    pub side: String,
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
/// A shared board's sidebar pin moved (card #233): the pin row's key, a
/// [`BOARD_PIN_PREFIX`] id, is in `task_id`. No card changed.
pub const BOARD_PIN_CHANGED: &str = "board_pin";
/// The pin-row key prefix of a shared board's sidebar pin (card #135).
pub const BOARD_PIN_PREFIX: &str = "board:";

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
    /// The question of the blocker it answers, cut short (card #490).
    #[serde(skip)]
    pub blocker_gist: String,
    #[serde(skip_serializing_if = "omit")]
    pub pick: i64,
    #[serde(skip_serializing_if = "omit")]
    pub missing: Vec<String>,
    #[serde(skip_serializing_if = "omit")]
    pub unheard: bool,
    #[serde(skip_serializing_if = "omit")]
    pub mentions: Vec<Mention>,
    /// A reviewer's verdict on a private card: approved or changes.
    #[serde(skip_serializing_if = "omit")]
    pub verdict: String,
    /// When the person last edited it before an agent read it (card #105).
    #[serde(skip_serializing_if = "crate::gojson::omit_zero_time")]
    pub edited_at: GoTime,
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
    /// The agents the message mentions (they get their own copies).
    pub mentions: Vec<Mention>,
}

/// Why a Notice exists.
pub const NOTICE_ASSIGNED: &str = "assigned";
/// An agent was given a step (card #86).
pub const NOTICE_STEP_OWNER: &str = "step_owner";
pub const NOTICE_COMMENT: &str = "comment";
pub const NOTICE_UNBLOCKED: &str = "unblocked";
pub const NOTICE_RETURNED: &str = "returned";
pub const NOTICE_REOPENED: &str = "reopened";
pub const NOTICE_ANSWERED: &str = "answered";
pub const NOTICE_WATCHED: &str = "watched";
/// A wait went off (card #87): always a wake.
pub const NOTICE_WAIT: &str = "wait";
pub const NOTICE_ASK: &str = "ask";
pub const NOTICE_TAKEN: &str = "taken";
pub const NOTICE_ASK_COMMENT: &str = "ask_comment";
/// An unheld card offered to the free agents (card #82).
pub const NOTICE_OFFER: &str = "offer";
/// A holder ended its turn on a working card with no blocker and no change
/// (card #305): name what it waits on, or keep going.
pub const NOTICE_IDLE: &str = "idle";

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
    /// The person's stop (card #207): sent with mode stop. Set by the
    /// window only, never by coord.
    #[serde(skip_serializing_if = "omit")]
    pub stop: bool,
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
    /// Open items whose agent is gone, on a card nobody holds (card #485).
    /// Not part of the Go shape: the tools and the window read it.
    pub stranded: Vec<super::stranded::Stranded>,
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
    /// What an agent here is told about sides (card #496): the sides this
    /// device runs, and that other sides' cards are only read. Empty when
    /// it runs every side.
    #[serde(skip_serializing_if = "omit")]
    pub side_line: String,
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
    /// The blocker's question, cut short, when the line is about one
    /// (card #490): the person reads it in place of the blocker's id.
    #[serde(skip)]
    pub blocker_gist: String,
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
    pub mentions: Vec<Mention>,
    /// Files with an answer that isn't a resolve (card #195): they go on
    /// its comment in the blocker's thread and with its notice.
    pub files: Vec<FileRef>,
}

/// CommentOn's input.
#[derive(Debug, Clone, Default)]
pub struct CommentInput {
    pub text: String,
    pub wake: bool,
    pub files: Vec<FileRef>,
    pub blocker_id: String,
    pub missing: Vec<String>,
    pub mentions: Vec<Mention>,
    /// A review verdict (#88): approved or changes; empty for a plain comment.
    pub verdict: String,
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
    /// Set: the ask goes only to these agents, woken.
    pub mentions: Vec<Mention>,
    /// A thread (card #116): the agents on the cards it names and the ones
    /// it mentions hear it, and at once one idle agent or its own spawned
    /// agent (card #512).
    pub thread: bool,
    /// A private thread about this shared board (card #185); private log
    /// only.
    pub board: String,
    /// A thread with "Agents answer now" off (card #391): no agent is woken
    /// or handed it; the agents it names hear it at their next turn.
    pub quiet: bool,
    /// A thread started from a message in this Talk channel (its `ch_` id,
    /// card #454); private log only.
    pub channel: String,
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

/// board_edit's input: `None` leaves that field as it is.
#[derive(Debug, Clone, Default)]
pub struct TaskEdit {
    pub title: Option<String>,
    pub objective: Option<String>,
    pub done_when: Option<String>,
    /// A thread's short ref or task id: the change comes from it (card
    /// #155). Empty: the one open thread the caller answered, if any.
    pub from: String,
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
    /// Asks for a helper on this subtask (spawned agents §3).
    pub spawn: bool,
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

/// One open wait (card #87): `by` is woken once when task `task_id`
/// reaches a state in `until`, or when `branch` of `repo` moves off the
/// shas it had (`refs`, "<ref>=<sha>"). See waits.rs.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Wait {
    pub id: String,
    pub by: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub task_id: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub until: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub from_state: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub repo: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub root: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub branch: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub at: GoTime,
    pub expires: GoTime,
}

/// What a new wait waits on (the `wait` tool's arguments, resolved).
#[derive(Debug, Clone, Default)]
pub struct WaitSpec {
    pub task_id: String,
    pub until: Vec<String>,
    pub repo: String,
    pub root: String,
    pub branch: String,
    /// "<ref>=<sha>" for each ref of the branch as it is now; empty sha
    /// when the ref doesn't exist yet.
    pub refs: Vec<String>,
    pub note: String,
}

/// Where a card's work is (card #137): the holder's session as its device
/// saw it last.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Where {
    pub who: String,
    #[serde(skip_serializing_if = "omit")]
    pub cwd: String,
    #[serde(skip_serializing_if = "omit")]
    pub worktree: String,
    #[serde(skip_serializing_if = "omit")]
    pub branch: String,
    #[serde(skip_serializing_if = "omit")]
    pub head: String,
    #[serde(skip_serializing_if = "omit")]
    pub pushed: bool,
    pub at: GoTime,
}

impl Where {
    /// The same place: everything but when it was seen.
    pub fn same_place(&self, o: &Where) -> bool {
        self.who == o.who
            && self.cwd == o.cwd
            && self.worktree == o.worktree
            && self.branch == o.branch
            && self.head == o.head
            && self.pushed == o.pushed
    }

    /// One line: "branch feat/x @ 1a2b3c4 (pushed) in /path/to/worktree".
    pub fn line(&self) -> String {
        let place = if self.worktree.is_empty() {
            self.cwd.as_str()
        } else {
            self.worktree.as_str()
        };
        if self.branch.is_empty() && self.head.is_empty() {
            return format!("{place} (not a git repo)");
        }
        let mut s = String::new();
        if self.branch.is_empty() {
            s.push_str("detached");
        } else {
            s.push_str("branch ");
            s.push_str(&self.branch);
        }
        if !self.head.is_empty() {
            s.push_str(" @ ");
            s.push_str(self.head.get(..7).unwrap_or(&self.head));
        }
        s.push_str(if self.pushed {
            " (pushed)"
        } else {
            " (not pushed)"
        });
        if !place.is_empty() {
            s.push_str(" in ");
            s.push_str(place);
        }
        s
    }
}

/// A holder's one line on what's done and what's next (card #137).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeftOff {
    pub text: String,
    pub by: String,
    pub at: GoTime,
}

impl Task {
    /// What a new holder needs to pick the work up (card #137): where the
    /// last holder's work is and where it left off, one line each; "" when
    /// neither is known.
    pub fn handover_lines(&self) -> String {
        let mut s = String::new();
        // Only a git place says where to continue.
        if let Some(w) = self.where_.as_ref().filter(|w| !w.worktree.is_empty()) {
            s.push_str(&format!("\nWhere: {}", w.line()));
            if !w.branch.is_empty() {
                s.push_str(if w.pushed {
                    "; continue from that branch"
                } else {
                    "; not pushed, so the last commits may be only on that machine"
                });
            }
        }
        if let Some(l) = &self.left_off {
            s.push_str(&format!("\nLeft off: {}", l.text));
        }
        s
    }
}
