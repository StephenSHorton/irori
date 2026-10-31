//! Row payloads (coord.md §2.2), in Go's field order with Go's omit
//! rules. Decoding is Go `json.Unmarshal` (gojson); a row read from the
//! log is never re-marshalled.

use std::collections::BTreeMap;

use gocompat::time::GoTime;

use super::types::{BlockedBy, BlockerOption, FileRef, Lease, Link, Outcome, Step};
use crate::gojson::{GoDecode, decode, field, nil_vec, omit};

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct AskField {
        pub wake: bool = "wake" "omit",
        pub repo_key: String = "repoKey" "omit",
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
