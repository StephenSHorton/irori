//! TypeScript shapes for the area types the window receives as they are.
//!
//! The area crates serialize Go's JSON byte for byte (their goldens pin
//! it), so the window sends their values unchanged and these structs only
//! describe that JSON to specta: they are never built or serialized. Each
//! one is the Go struct's JSON as Wails typed it
//! (I:frontend/bindings/github.com/StephenSHorton/irori/internal/*/models.ts
//! at be67a86): a Go `omitempty` field is optional, every other field is
//! always there. `tests/shapes.rs` checks each against the real JSON of a
//! filled area value, so a field added on either side fails a test.
//!
//! [`Typed`] carries an area value under one of these shapes, for a
//! command's argument or result; inside a window struct a field takes the
//! shape with `#[specta(type = ...)]`.

#![allow(dead_code)]

use std::marker::PhantomData;

use serde::{Deserialize, Serialize};
use specta::Type;

/// An area value `T` the bindings type as `M`.
pub struct Typed<T, M>(pub T, PhantomData<fn() -> M>);

impl<T, M> Typed<T, M> {
    pub fn new(v: T) -> Self {
        Typed(v, PhantomData)
    }
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: Serialize, M> Serialize for Typed<T, M> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

impl<'de, T: Deserialize<'de>, M> Deserialize<'de> for Typed<T, M> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        T::deserialize(d).map(Typed::new)
    }
}

impl<T, M: Type> Type for Typed<T, M> {
    fn definition(types: &mut specta::Types) -> specta::datatype::DataType {
        M::definition(types)
    }
}

/// Go `time.Time`: RFC3339Nano text (the zero time is
/// `"0001-01-01T00:00:00Z"`, never null).
pub type Time = String;

// ---- presence, device, store ---------------------------------------------

/// presence.AgentCard.
#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub key: String,
    pub device_id: String,
    pub agent: String,
    pub session_id: String,
    pub pid: i64,
    pub name: String,
    pub title: String,
    #[specta(optional)]
    pub name_source: String,
    #[specta(optional)]
    pub ai_title: String,
    pub cwd: String,
    pub status: String,
    pub session_kind: String,
    pub entrypoint: String,
    pub version: String,
    /// Go writes a nil slice as null.
    pub peer_features: Option<Vec<String>>,
    pub started_at: Time,
    pub updated_at: Time,
    pub last_activity: Time,
    pub last_text: String,
    pub transcript: String,
    pub repo_key: String,
    pub permission_mode: String,
    #[specta(optional)]
    pub device_state: String,
}

/// presence.ChangeKind.
#[derive(Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Updated,
    Removed,
}

/// presence.Change.
#[derive(Serialize, Type)]
pub struct Change {
    pub kind: ChangeKind,
    pub card: AgentCard,
}

/// device.Identity (device.json).
#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub id: String,
    pub name: String,
    pub os: String,
    #[specta(optional)]
    pub person_id: String,
    #[specta(optional)]
    pub device_pub: String,
}

/// store.Event: one log row. `payload` is the row's JSON as stored; `hlc`
/// is a 64-bit clock and loses precision as a JS number, so the window
/// only shows it.
#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub device_id: String,
    pub seq: i64,
    pub hlc: i64,
    pub actor: String,
    pub kind: String,
    pub payload: serde_json::Value,
}

// ---- blob --------------------------------------------------------------------

/// blob.FileRef.
#[derive(Serialize, Type)]
pub struct FileRef {
    pub sha256: String,
    pub name: String,
    pub size: i64,
    #[specta(optional)]
    pub mime: String,
    pub device: String,
}

// ---- inbox ---------------------------------------------------------------------

/// inbox.State.
#[derive(Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Queued,
    Held,
    Delivered,
    Engaged,
    Answered,
    Deferred,
    Skipped,
    Rejected,
    Dismissed,
    Moved,
}

/// inbox.HoldReason.
#[derive(Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HoldReason {
    Blocked,
    NotAllowed,
    BypassReceiver,
    Paused,
    HopLimit,
    Unowned,
    MemberWake,
    CrossPerson,
    MemberLease,
}

/// inbox.DeliveryMode.
#[derive(Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    Wake,
    NextTurn,
}

/// inbox.Mode (the wake policy).
#[derive(Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Everyone,
    Allowlist,
    Open,
}

/// inbox.AddressKind.
#[derive(Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum AddressKind {
    Agent,
    Repo,
    Reply,
    Pr,
}

/// inbox.Trust.
#[derive(Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Trust {
    Untrusted,
    User,
}

/// inbox.Sender.
#[derive(Serialize, Type)]
pub struct Sender {
    pub key: String,
    pub name: String,
    pub kind: String,
}

/// inbox.Address.
#[derive(Serialize, Type)]
pub struct Address {
    pub kind: AddressKind,
    pub value: String,
}

/// inbox.Message.
#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub from: Sender,
    pub to: Address,
    pub text: String,
    #[specta(optional)]
    pub reason: String,
    #[specta(optional)]
    pub reply_to: String,
    #[specta(optional)]
    pub parent_id: String,
    pub hop: i64,
    pub trust: Trust,
    pub sent_at: Time,
    #[serde(rename = "ref")]
    #[specta(optional)]
    pub ref_: String,
    #[specta(optional)]
    pub notice: bool,
    #[specta(optional)]
    pub attachments: Vec<FileRef>,
}

/// inbox.Delivery.
#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub id: String,
    pub message_id: String,
    pub to: String,
    #[specta(optional)]
    pub repo_key: String,
    #[specta(optional)]
    pub mode: DeliveryMode,
    pub state: State,
    #[specta(optional)]
    pub hold: HoldReason,
    #[specta(optional)]
    pub via: String,
    #[specta(optional)]
    pub wake_id: String,
    #[specta(optional)]
    pub note: String,
    #[serde(rename = "for")]
    #[specta(optional)]
    pub for_: String,
    #[specta(optional)]
    pub until: Time,
    #[specta(optional)]
    pub moved_to: String,
    pub reminded: bool,
    #[specta(optional)]
    pub held_for: HoldReason,
    #[specta(optional)]
    pub decided_by: String,
    #[specta(optional)]
    pub decided_at: Time,
    pub created_at: Time,
    pub updated_at: Time,
    pub message: Message,
}

/// inbox.Policy.
#[derive(Serialize, Type)]
pub struct Policy {
    pub mode: Mode,
    pub block: Vec<String>,
    pub allow: Vec<String>,
    pub paused: bool,
}

// ---- coord ---------------------------------------------------------------------

/// coord.TaskState.
#[derive(Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Submitted,
    Working,
    InputRequired,
    Completed,
    Failed,
    Canceled,
    Blocked,
}

/// coord.LinkKind.
#[derive(Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind {
    Url,
    Path,
    Commit,
}

/// coord.BlockerKind.
#[derive(Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum BlockerKind {
    Lease,
    Task,
    Person,
    Agent,
    External,
}

/// coord.BlockerStatus.
#[derive(Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum BlockerStatus {
    Open,
    Answered,
    Replied,
}

/// coord.Link.
#[derive(Serialize, Type)]
pub struct Link {
    pub kind: LinkKind,
    #[serde(rename = "ref")]
    pub ref_: String,
    pub label: String,
}

/// coord.Step.
#[derive(Serialize, Type)]
pub struct Step {
    pub id: String,
    pub text: String,
    pub done: bool,
    #[specta(optional)]
    pub doing: bool,
}

/// coord.BlockedBy.
#[derive(Serialize, Type)]
pub struct BlockedBy {
    pub kind: BlockerKind,
    #[serde(rename = "ref")]
    #[specta(optional)]
    pub ref_: String,
}

/// coord.Option: one recommended answer.
#[derive(Serialize, Type)]
pub struct BlockerOption {
    pub answer: String,
    #[specta(optional)]
    pub why: String,
    pub by: String,
    pub at: Time,
    #[specta(optional)]
    pub recommended: bool,
    #[specta(optional)]
    pub link: Link,
}

/// coord.Resolution.
#[derive(Serialize, Type)]
pub struct Resolution {
    pub answer: String,
    pub option: i64,
    pub by: String,
    pub at: Time,
    #[specta(optional)]
    pub closed: bool,
    #[specta(optional)]
    pub note: String,
}

/// coord.Blocker.
#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Blocker {
    pub id: String,
    pub question: String,
    #[specta(optional)]
    pub context: String,
    pub raised_by: String,
    pub at: Time,
    #[specta(optional)]
    pub options: Vec<BlockerOption>,
    #[specta(optional)]
    pub resolved: Resolution,
    #[specta(optional)]
    pub links: Vec<Link>,
    #[specta(optional)]
    pub attachments: Vec<FileRef>,
    #[specta(optional)]
    pub on: BlockedBy,
    #[specta(optional)]
    pub status: BlockerStatus,
    #[specta(optional)]
    pub sent_to: String,
    #[specta(optional)]
    pub missing: Vec<String>,
}

/// coord.Outcome: how an ask ended.
#[derive(Serialize, Type)]
pub struct Outcome {
    pub kind: String,
    #[specta(optional)]
    pub task: String,
    pub note: String,
}

/// coord.Ask.
#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    #[specta(optional)]
    pub wake: bool,
    #[specta(optional)]
    pub repo_key: String,
    pub owner: String,
    #[specta(optional)]
    pub taker: String,
    #[specta(optional)]
    pub reason: String,
    pub told: i64,
    #[specta(optional)]
    pub heard: Vec<String>,
    pub since: Time,
    #[specta(optional)]
    pub unclaimed: bool,
    #[specta(optional)]
    pub waiting: Vec<String>,
    #[specta(optional)]
    pub outcome: Outcome,
}

/// coord.Returned.
#[derive(Serialize, Type)]
pub struct Returned {
    pub was: String,
    pub at: Time,
    #[specta(optional)]
    pub row: String,
    #[specta(optional)]
    pub person: bool,
}

/// coord.Rollup.
#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Rollup {
    pub state: TaskState,
    pub done: i64,
    pub total: i64,
    #[specta(optional)]
    pub blocked: String,
    #[specta(optional)]
    pub blocked_question: String,
}

/// coord.Task.
#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    #[specta(optional)]
    pub short: String,
    #[specta(optional)]
    pub short_was: String,
    #[specta(optional)]
    pub tag: String,
    #[specta(optional)]
    pub withdrawn: bool,
    pub title: String,
    pub objective: String,
    #[specta(optional)]
    pub done_criteria: String,
    #[specta(optional)]
    pub repo_key: String,
    #[specta(optional)]
    pub scope: Vec<String>,
    pub state: TaskState,
    #[specta(optional)]
    pub holder: String,
    #[specta(optional)]
    pub holders: Vec<String>,
    #[specta(optional)]
    pub assigned: Vec<String>,
    #[specta(optional)]
    pub shared: bool,
    pub created_by: String,
    #[specta(optional)]
    pub note: String,
    pub created_at: Time,
    pub updated_at: Time,
    #[specta(optional)]
    pub updated_by: String,
    #[specta(optional)]
    pub reason: String,
    #[specta(optional)]
    pub blocked_by: BlockedBy,
    #[specta(optional)]
    pub links: Vec<Link>,
    #[specta(optional)]
    pub comments: i64,
    #[specta(optional)]
    pub steps: Vec<Step>,
    #[specta(optional)]
    pub blockers: Vec<Blocker>,
    #[specta(optional)]
    pub watchers: Vec<String>,
    #[specta(optional)]
    pub ask: Ask,
    #[specta(optional)]
    pub returned: Returned,
    #[specta(optional)]
    pub parent: String,
    #[specta(optional)]
    pub subtasks: Vec<String>,
    #[specta(optional)]
    pub rollup: Rollup,
    #[specta(optional)]
    pub pinned: bool,
    #[specta(optional)]
    pub pin_rank: i64,
}

/// coord.Comment.
#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    pub task_id: String,
    pub by: String,
    pub text: String,
    pub wake: bool,
    pub at: Time,
    #[specta(optional)]
    pub attachments: Vec<FileRef>,
    #[specta(optional)]
    pub to: String,
    #[specta(optional)]
    pub msg_id: String,
    #[specta(optional)]
    pub gone: bool,
    #[specta(optional)]
    pub held: bool,
    #[specta(optional)]
    pub files: i64,
    #[specta(optional)]
    pub blocker_id: String,
    #[specta(optional)]
    pub pick: i64,
    #[specta(optional)]
    pub missing: Vec<String>,
    #[specta(optional)]
    pub unheard: bool,
    pub device: String,
}
