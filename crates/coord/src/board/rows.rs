//! Row payloads (coord.md §2.2), in Go's field order with Go's omit
//! rules. Decoding is Go `json.Unmarshal` (gojson); a row read from the
//! log is never re-marshalled.

use std::collections::BTreeMap;

use gocompat::time::GoTime;

use super::types::{BlockedBy, BlockerOption, FileRef, Lease, Link, Mention, Outcome, Step};
use crate::gojson::{GoDecode, decode, field, nil_vec, omit};

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct AskField {
        pub wake: bool = "wake" "omit",
        pub repo_key: String = "repoKey" "omit",
        pub mentions: Vec<Mention> = "mentions" "omit",
        /// A thread (card #116): a conversation, not a card.
        pub thread: bool = "thread" "omit",
        /// A private thread about a shared board (card #185): it lives in
        /// the person's own log and shows on that board's page.
        pub board: String = "board" "omit",
        /// The device that hands it out and on (card #408), when it isn't
        /// the one that posted it: a phone, which runs no agents and sleeps,
        /// names one of the person's computers.
        pub owner: String = "owner" "omit",
        /// A thread started from a message in this Talk channel (card
        /// #454): its `ch_` id.
        pub channel: String = "channel" "omit",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskCreatedRow {
        pub task_id: String = "taskId",
        pub num: i64 = "num" "omit",
        pub title: String = "title",
        pub objective: String = "objective",
        pub done_criteria: String = "doneCriteria" "omit",
        pub repo_key: String = "repoKey" "omit",
        pub scope: Vec<String> = "scope" "omit",
        pub shared: bool = "shared" "omit",
        pub links: Vec<Link> = "links" "omit",
        pub ask: Option<AskField> = "ask" "omit",
        pub parent: String = "parent" "omit",
        /// A helper's part of its parent (spawned agents §3).
        pub spawn: bool = "spawn" "omit",
        /// The thread the card was made from (card #155).
        pub from: String = "from" "omit",
        /// The board in a space it's on (card #392): a `space.board.created`
        /// id; empty for the space's own board (and every older card).
        pub board: String = "board" "omit",
        /// The work/personal side it's on (card #494): the side of the
        /// device it was typed on, or its parent's; empty for any side
        /// (and every older card). Private log only.
        pub side: String = "side" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskClaimedRow {
        pub task_id: String = "taskId",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskAssignedRow {
        pub task_id: String = "taskId",
        pub to: String = "to",
        pub by: String = "by",
        pub at: GoTime = "at",
        /// The hand-off note (card #85): what's done, where, what's next.
        pub note: String = "note" "omit",
    }
}

crate::go_struct! {
    /// The item a blocked task.status raises.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct BlockerRow {
        pub id: String = "id",
        pub question: String = "question",
        pub context: String = "context" "omit",
        pub options: Vec<BlockerOption> = "options" "omit",
        pub reason_only: bool = "reasonOnly" "omit",
        pub replace: bool = "replace" "omit",
        /// With replace: a revision, whose options replace the item's
        /// (none clears them). Older folds ignore it and keep their options.
        pub revise: bool = "revise" "omit",
        pub on: Option<BlockedBy> = "on" "omit",
        pub links: Vec<Link> = "links" "omit",
        pub attachments: Vec<FileRef> = "attachments" "omit",
        /// A done check (card #112): an agent says the card is done and the
        /// person confirms it. Older folds ignore it: a plain blocker.
        pub done_check: bool = "doneCheck" "omit",
        /// An ownership offer or request (card #432). Older folds see a
        /// plain question.
        pub transfer: bool = "transfer" "omit",
    }
}

crate::go_struct! {
    /// How a card's linked PR stands on GitHub (card #208), written by
    /// the device that holds the card as `irori`.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct PrStateRow {
        pub task_id: String = "taskId",
        pub pr: String = "pr",
        pub state: String = "state" "omit",
        pub review: String = "review" "omit",
        pub merged_at: GoTime = "mergedAt",
        pub error: String = "error" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
        /// OPEN heard by a device that knew the PR closed: a reopen
        /// (card #548). An OPEN without it can't undo a closing.
        pub over_closed: bool = "overClosed" "omit",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskStateRow {
        pub task_id: String = "taskId",
        pub state: String = "state",
        pub note: String = "note" "omit",
        pub reason: String = "reason" "omit",
        pub blocked_by: Option<BlockedBy> = "blockedBy" "omit",
        pub blocker: Option<BlockerRow> = "blocker" "omit",
        pub outcome: Option<Outcome> = "outcome" "omit",
        pub withdrawn: bool = "withdrawn" "omit",
        /// The person reopens their resolved thread (card #169): its ask
        /// is open again. Older folds see a plain reopened card.
        pub reopen_ask: bool = "reopenAsk" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// task.joined and task.left.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskMemberRow {
        pub task_id: String = "taskId",
        pub who: String = "who",
        pub by: String = "by",
        pub at: GoTime = "at",
        pub reason: String = "reason" "omit",
        pub person: bool = "person" "omit",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskSharedRow {
        pub task_id: String = "taskId",
        pub shared: bool = "shared",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskLinkedRow {
        pub task_id: String = "taskId",
        pub link: Link = "link",
        pub removed: bool = "removed" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskCommentRow {
        pub task_id: String = "taskId",
        pub comment_id: String = "commentId",
        pub by: String = "by",
        pub text: String = "text",
        pub wake: bool = "wake",
        pub at: GoTime = "at",
        pub attachments: Vec<FileRef> = "attachments" "omit",
        pub to: String = "to" "omit",
        pub msg_id: String = "msgId" "omit",
        pub gone: bool = "gone" "omit",
        pub held: bool = "held" "omit",
        pub files: i64 = "files" "omit",
        pub fill: bool = "fill" "omit",
        pub blocker_id: String = "blockerId" "omit",
        pub pick: i64 = "pick" "omit",
        pub missing: Vec<String> = "missing" "omit",
        pub unheard: bool = "unheard" "omit",
        pub mentions: Vec<Mention> = "mentions" "omit",
        /// A review verdict on a private card (#88): approved or changes.
        pub verdict: String = "verdict" "omit",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct BlockerClosedRow {
        pub task_id: String = "taskId",
        pub by: String = "by",
        pub note: String = "note" "omit",
        pub restored: String = "restored",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct LeaseReleasedRow {
        pub resource: String = "resource",
        pub holder: String = "holder",
        pub reason: String = "reason",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct StolenLease {
        pub resource: String = "resource",
        pub holder: String = "holder",
    }
}

/// lease.acquired: a Lease (embedded in Go) plus the leases a steal took.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LeaseAcquiredRow {
    pub lease: Lease,
    pub stolen: Vec<StolenLease>,
}

impl serde::Serialize for LeaseAcquiredRow {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut extra = crate::gojson::Extra::default();
        if !self.stolen.is_empty() {
            extra.field("stolen", &self.stolen)?;
        }
        extra.embed(s, &self.lease)
    }
}

impl GoDecode for Lease {
    fn schema() -> gocompat::json::Schema {
        gocompat::json::Schema::Struct(vec![
            ("resource", String::schema()),
            ("holder", String::schema()),
            ("until", GoTime::schema()),
            ("stolenFrom", String::schema()),
            ("acquiredAt", GoTime::schema()),
            ("conflict", String::schema()),
        ])
    }
    fn from_out(o: &gocompat::json::Out) -> Self {
        Lease {
            resource: field(o, "resource"),
            holder: field(o, "holder"),
            until: field(o, "until"),
            stolen_from: field(o, "stolenFrom"),
            acquired_at: field(o, "acquiredAt"),
            conflict: field(o, "conflict"),
        }
    }
}

impl GoDecode for LeaseAcquiredRow {
    fn schema() -> gocompat::json::Schema {
        let gocompat::json::Schema::Struct(mut fields) = Lease::schema() else {
            unreachable!()
        };
        fields.push(("stolen", Vec::<StolenLease>::schema()));
        gocompat::json::Schema::Struct(fields)
    }
    fn from_out(o: &gocompat::json::Out) -> Self {
        LeaseAcquiredRow {
            lease: Lease::from_out(o),
            stolen: field(o, "stolen"),
        }
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct LeaseStolenRow {
        pub resource: String = "resource",
        pub holder: String = "holder",
        pub acquired_at: GoTime = "acquiredAt",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct BlockerOptionRow {
        pub task_id: String = "taskId",
        pub blocker_id: String = "blockerId",
        pub answer: String = "answer",
        pub why: String = "why" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct BlockerResolvedRow {
        pub task_id: String = "taskId",
        pub blocker_id: String = "blockerId",
        pub answer: String = "answer",
        pub option: i64 = "option",
        pub by: String = "by",
        pub restored: String = "restored" "omit",
        pub closed: bool = "closed" "omit",
        pub at: GoTime = "at",
        pub note: String = "note" "omit",
    }
}

crate::go_struct! {
    /// task.steps: `steps` has no omitempty, so an empty list is `null`.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskStepsRow {
        pub task_id: String = "taskId",
        #[serde(serialize_with = "nil_vec")]
        pub steps: Vec<Step> = "steps",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskStepRow {
        pub task_id: String = "taskId",
        pub step_id: String = "stepId",
        pub done: Option<bool> = "done" "omit",
        pub doing: Option<bool> = "doing" "omit",
        /// Sets the step's owner; `Some("")` clears it (card #86).
        pub owner: Option<String> = "owner" "omit",
        /// The holder's one line on what's done and what's next (card
        /// #137), shown on the card as "Left off". Older folds ignore it.
        pub left_off: String = "leftOff" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// task.where (card #137): where the holder's work on a card is.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskWhereRow {
        pub task_id: String = "taskId",
        /// The holder it describes.
        pub who: String = "who",
        pub cwd: String = "cwd" "omit",
        /// The git worktree's root; empty outside a repo.
        pub worktree: String = "worktree" "omit",
        pub branch: String = "branch" "omit",
        /// The last commit (HEAD's sha).
        pub head: String = "head" "omit",
        /// origin's copy of the branch is at HEAD.
        pub pushed: bool = "pushed" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskWatchedRow {
        pub task_id: String = "taskId",
        pub who: String = "who",
        pub watching: bool = "watching",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// task.deleted: the person took a card off the Board for good.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskDeletedRow {
        pub task_id: String = "taskId",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// A shared card's owner changing hands (card #432).
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskOwnerRow {
        pub task_id: String = "taskId",
        /// offer, request, accept or decline.
        pub op: String = "op",
        /// Who would own it: a person id.
        pub person: String = "person",
        pub blocker_id: String = "blockerId" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskPausedRow {
        pub task_id: String = "taskId",
        pub paused: bool = "paused",
        pub note: String = "note",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskArchivedRow {
        pub task_id: String = "taskId",
        pub archived: bool = "archived",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskPinnedRow {
        pub task_id: String = "taskId",
        pub pinned: bool = "pinned",
        pub rank: f64 = "rank" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskParentRow {
        pub task_id: String = "taskId",
        pub parent: String = "parent",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TakeRequestedRow {
        pub task_id: String = "taskId",
        pub by: String = "by",
        pub op: String = "op",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// Names one take.requested row.
    #[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
    pub struct TakeReqRef {
        pub device: String = "device",
        pub seq: i64 = "seq",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TakeStateRow {
        pub task_id: String = "taskId",
        pub taker: String = "taker",
        pub reason: String = "reason",
        pub req: Option<TakeReqRef> = "req" "omit",
        pub told: i64 = "told",
        pub heard: Vec<String> = "heard" "omit",
        pub since: GoTime = "since",
        pub unclaimed: bool = "unclaimed" "omit",
        pub seen: BTreeMap<String, i64> = "seen" "omit",
        pub was: String = "was" "omit",
        pub handed: bool = "handed" "omit",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// task.edited: a set `*string` is written even when `""`.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskEditedRow {
        pub task_id: String = "taskId",
        pub title: Option<String> = "title" "omit",
        pub objective: Option<String> = "objective" "omit",
        pub done_criteria: Option<String> = "doneCriteria" "omit",
        pub by: String = "by",
        pub from: String = "from" "omit",
        pub note: String = "note" "omit",
        /// A thread's "Agents answer now" (card #391): set only by the
        /// person, on a thread.
        pub agents_now: Option<bool> = "agentsNow" "omit",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// The fields askGate reads from any task row.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct GateRow {
        pub task_id: String = "taskId",
        pub state: String = "state",
        pub by: String = "by",
        pub outcome: Option<Outcome> = "outcome",
    }
}

crate::go_struct! {
    /// The key an `agent.gone` or `agent.seen` row names (card #110).
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct GoneKeyRow {
        pub key: String = "key",
        /// "finished" when Irori ended a spawned agent (card #446).
        pub reason: String = "reason" "omit",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskIdRow {
        pub task_id: String = "taskId",
        pub who: String = "who",
    }
}

/// Decodes a payload as Go would; `None` drops the row.
pub fn row<T: GoDecode>(payload: &[u8]) -> Option<T> {
    decode(payload)
}

crate::go_struct! {
    /// A board's peer review request (v0.5 §10): `from` asks `to` (or the
    /// other people's dedicated reviewers) to review a PR the card links.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct ReviewRequestedRow {
        pub task_id: String = "taskId",
        pub pr: String = "pr",
        pub from: String = "from",
        pub to: String = "to" "omit",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// A person dedicating one of their agents as reviewer, for the board
    /// or one repo; an empty reviewer clears it.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct ReviewReviewerRow {
        pub reviewer: String = "reviewer",
        pub repo_key: String = "repoKey" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// wait.created (card #87): `by` asks to be woken once when task
    /// `task_id` reaches a state in `until`, or when `branch` of `repo`
    /// moves off the shas in `refs` ("<ref>=<sha>", as seen at creation).
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct WaitCreatedRow {
        pub id: String = "id",
        pub by: String = "by",
        pub task_id: String = "taskId" "omit",
        pub until: Vec<String> = "until" "omit",
        pub from_state: String = "fromState" "omit",
        pub repo: String = "repo" "omit",
        pub root: String = "root" "omit",
        pub branch: String = "branch" "omit",
        pub refs: Vec<String> = "refs" "omit",
        pub note: String = "note" "omit",
        pub at: GoTime = "at",
        pub expires: GoTime = "expires",
        pub board: String = "board" "omit",
        pub card: String = "card" "omit",
        pub title: String = "title" "omit",
    }
}

crate::go_struct! {
    /// wait.fired: the wait went off once; `what` is what happened.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct WaitFiredRow {
        pub id: String = "id",
        pub what: String = "what",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// wait.canceled: by its waiter, or by Irori when the waiter's session
    /// is gone or the wait expired (`reason`).
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct WaitCanceledRow {
        pub id: String = "id",
        pub reason: String = "reason" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// task.placed (card #93): where a card sits on the Board's canvas,
    /// in canvas pixels; the newest row wins.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskPlacedRow {
        pub task_id: String = "taskId",
        pub x: i64 = "x",
        pub y: i64 = "y",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// task.offered (card #82): where Irori's hand-out of an unheld card
    /// stands. `to` is the one free agent it went to (empty while it
    /// waits for one); `was` the agent it moved on from; `heard` every
    /// agent it went to so far.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskOfferedRow {
        pub task_id: String = "taskId",
        pub to: String = "to" "omit",
        pub was: String = "was" "omit",
        pub reason: String = "reason",
        pub heard: Vec<String> = "heard" "omit",
        pub since: GoTime = "since",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// task.spawn (card #446): a request to start an agent for a card, or
    /// its end as failed or canceled. `gen` counts the card's requests;
    /// only the newest is acted on.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskSpawnRow {
        pub task_id: String = "taskId",
        pub generation: i64 = "gen",
        pub step: String = "step" "omit",
        pub subtask: String = "subtask" "omit",
        pub op: String = "op",
        pub device: String = "device",
        pub repo_key: String = "repoKey" "omit",
        pub folder: String = "folder" "omit",
        pub worktree: String = "worktree" "omit",
        pub model: String = "model" "omit",
        pub effort: String = "effort" "omit",
        pub permission_mode: String = "permissionMode",
        pub name: String = "name" "omit",
        pub reason: String = "reason" "omit",
        /// The person's words for the agent's brief, carried to the
        /// device that starts it (card #455).
        pub note: String = "note" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// task.spawned (card #446): agent `key` was started for card
    /// `taskId` (the spawned_for mark).
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskSpawnedRow {
        pub task_id: String = "taskId",
        pub key: String = "key",
        pub generation: i64 = "gen",
        pub step: String = "step" "omit",
        pub subtask: String = "subtask" "omit",
        pub device: String = "device",
        /// The shared board whose card it is, on the person's own mark for
        /// a board card's agent (card #455); empty for this log's card.
        pub board: String = "board" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// task.comment.edited (card #105): the person's new text for their
    /// comment, written before any agent read it.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct TaskCommentEditedRow {
        pub task_id: String = "taskId",
        pub comment_id: String = "commentId",
        pub by: String = "by",
        pub text: String = "text",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// A new note (card #401).
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct NoteProposedRow {
        pub note_id: String = "noteId",
        pub kind: String = "kind",
        pub text: String = "text",
        pub tags: Vec<String> = "tags",
        pub source: String = "source" "omit",
        pub task_id: String = "taskId" "omit",
        pub comment_id: String = "commentId" "omit",
        pub blocker_id: String = "blockerId" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct NoteAcceptedRow {
        pub note_id: String = "noteId",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// Each given field replaces the note's (an agent's waits for an
    /// accept when the note is live).
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct NoteEditedRow {
        pub note_id: String = "noteId",
        pub kind: String = "kind" "omit",
        pub text: String = "text" "omit",
        pub tags: Option<Vec<String>> = "tags" "omit",
        pub why: String = "why" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct NoteRemovedRow {
        pub note_id: String = "noteId",
        pub why: String = "why" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}
