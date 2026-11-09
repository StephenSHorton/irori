//! The Office Assistant's one conversation as rows (card #1102,
//! docs/design/one-assistant.md §4): the person's questions, the replies,
//! the chat's follow-up lines, owner's-OK drafts and New chat are private
//! `assistant.*` rows, so every one of the person's devices shows the same
//! conversation. This module holds their payloads, the fold every device
//! runs the same, the one writer that scrubs a row before it is signed, and
//! the paged list the chat reads.
//!
//! The fold goes by the store's own order, `(hlc, device_id, seq)`, never by
//! when a row happened to arrive, so two devices that got the same rows in
//! any order show the same list:
//!
//! - A turn is its `asked` row and the first `said` row for it. A `said`
//!   row that comes before its question waits for it, unseen.
//! - `update` lines: the first per key. Drafts: the first per request.
//!   `posted`: the first per turn, filling the turn's `posted` when its
//!   reply has none. A `stop` marks the turn stopped.
//! - The newest New chat hides every turn whose question is older, and
//!   every line and draft older than it. Imported rows (the history the
//!   assistant's computer brought in, §10) count as older than any New
//!   chat, and sort before every turn that isn't imported.
//! - A row whose actor isn't its kind's is skipped.
//!
//! No tool writes these rows, and the events tool and `/v1/events` leave
//! their text out (`tools::observe::redact`, `api::redact`).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use store::{Event, Filter, Store, kinds};

use super::{Core, Services};
use crate::assistant_draft;
use crate::events::CoreEvent;

pub const ASKED: &str = kinds::ASSISTANT_CHAT_ASKED;
pub const SAID: &str = kinds::ASSISTANT_CHAT_SAID;
pub const UPDATE: &str = kinds::ASSISTANT_CHAT_UPDATE;
pub const POSTED: &str = kinds::ASSISTANT_CHAT_POSTED;
pub const CLEARED: &str = kinds::ASSISTANT_CHAT_CLEARED;
pub const STOP: &str = kinds::ASSISTANT_CHAT_STOP;
pub const ANSWER_REQUESTED: &str = kinds::ASSISTANT_ANSWER_REQUESTED;
pub const ANSWER_DRAFT: &str = kinds::ASSISTANT_ANSWER_DRAFT;

/// Every kind of the conversation.
pub const KINDS: [&str; 8] = [
    ASKED,
    SAID,
    UPDATE,
    POSTED,
    CLEARED,
    STOP,
    ANSWER_REQUESTED,
    ANSWER_DRAFT,
];

/// The words a `said` row carries for a turn the person stopped, or that
/// New chat ended while it ran. The chat keys Ask again on them (§6).
pub const STOPPED: &str = "Stopped";

/// What a running turn says as it goes (one-assistant §5): `delta`, `tool`,
/// `changed`, `done` or `error`, as the window's run hears it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatEvent {
    pub kind: String,
    pub text: String,
}

/// Which turn a [`ChatJob`] is. Every turn on the assistant's computer that
/// holds the assistant secret goes through the one worker (§5); assistants
/// talk's owner's-OK turn (#1064) adds its own kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ChatKind {
    #[default]
    Chat,
    /// The owner's-OK turn (assistants-talk card 8, §7): a draft answer to
    /// another person's question, from the question fenced, with read tools
    /// only. It holds no grants, has no history and no markers, and its
    /// reply is a draft row, never a chat turn.
    OwnerOk,
}

/// One question for the window to run (§5): Core built the prompt and the
/// grants from rows; the window runs it and blocks until it ends.
#[derive(Debug, Clone, Default)]
pub struct ChatJob {
    pub turn: String,
    /// The whole prompt, history and question, as plain text.
    pub prompt: String,
    /// The asking device's screen snapshot, for the `screen` tool.
    pub screen: String,
    /// What the person's own words allow it to change (card #987); none
    /// for a late question.
    pub grants: Vec<tools::Grant>,
    /// Tripped by a `stop` row, or a New chat newer than the question: the
    /// run ends at once.
    pub cancel: Arc<AtomicBool>,
    pub kind: ChatKind,
    /// The files that came with the question: the window reads them from
    /// its store, fetching from the device that holds them if need be.
    pub files: Vec<AskedFile>,
    /// The question as the held session gets it (card 8, §8): what it
    /// hasn't seen since its last message ("Since we last spoke:"), then
    /// the question alone. [`Self::prompt`] seeds a new session instead.
    pub message: String,
    /// The held session this question belongs to: the window keeps its
    /// `claude` only while this stays the same, and seeds a new one from
    /// [`Self::prompt`] when it changes (New chat, another computer, a
    /// failed or stopped turn).
    pub session: u64,
    /// The turn's own system prompt (an [`ChatKind::OwnerOk`] turn); empty
    /// for the chat, whose prompt is the window's.
    pub system: String,
}

/// How a run ended.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatDone {
    /// The reply, markers included, trimmed.
    pub text: String,
    /// The run failed: in the words the chat shows, naming the computer.
    pub error: String,
    /// What its write tools changed, a line each.
    pub changed: Vec<String>,
    /// The thread its draft posted by itself.
    pub posted: Option<Posted>,
    /// The label of the screen it looked at.
    pub looked: Option<String>,
    /// It never ran because Irori is closing: write no reply, so the next
    /// start's catch-up answers it.
    pub later: bool,
}

/// The desktop's chat turn, installed with `Core::set_chat_turn`: runs
/// `job`, telling `emit` each event as it comes (false: stop), and returns
/// once the run has ended. (The design calls it `ChatTurn`; that name is
/// the fold's turn here.)
pub type ChatTurnFn = Arc<dyn Fn(&ChatJob, &dyn Fn(ChatEvent) -> bool) -> ChatDone + Send + Sync>;

/// The most a text field holds, in bytes: a reply is cut here, well inside
/// the mesh's 8 MiB message.
pub const MAX_TEXT: usize = inbox::MAX_PERSON_TEXT;
/// The most of the asking device's screen a question carries, in
/// characters, as the window sends today.
pub const MAX_SCREEN_CHARS: usize = 4000;
/// The most files one question carries.
pub const MAX_FILES: usize = 10;
/// A page of the list when the caller names no size.
pub const PAGE: usize = 50;
/// The most one page holds.
pub const MAX_PAGE: usize = 500;

/// The actor a kind's rows must carry: `user` for what the person did,
/// `irori` for what Irori or the assistant wrote.
pub fn actor_of(kind: &str) -> Option<&'static str> {
    match kind {
        ASKED | POSTED | CLEARED | STOP | ANSWER_REQUESTED => Some("user"),
        SAID | UPDATE | ANSWER_DRAFT => Some("irori"),
        _ => None,
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A file that came with a question (card #999, one-assistant card 4): a
/// reference only. The bytes stay on the device that holds them (`device`),
/// which offers them to the person's other devices; the assistant's computer
/// fetches them when it runs the turn.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AskedFile {
    pub sha256: String,
    pub name: String,
    pub size: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub mime: String,
    /// The device that holds the bytes.
    pub device: String,
    /// The window checked, when it took the file in, whether the assistant
    /// can read this kind of file.
    pub can_read: bool,
}

/// A question the person typed (`assistant.chat.asked`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Asked {
    /// A new ULID: the turn's id.
    pub turn: String,
    pub text: String,
    /// The asking device's screen snapshot, cut to [`MAX_SCREEN_CHARS`].
    #[serde(skip_serializing_if = "String::is_empty")]
    pub screen: String,
    /// The device the person typed on.
    pub device: String,
    /// Unix milliseconds on that device.
    pub at: i64,
    /// The turn this one asks again (Ask again).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub again: String,
    /// Brought in from the device's old history (§10): never run.
    #[serde(skip_serializing_if = "is_false")]
    pub imported: bool,
    /// The huddle it was asked from (call-agents card 11, Divide).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub call_id: String,
    /// The files that came with it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<AskedFile>,
}

/// A thread a reply posted, or the person posted from its draft.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Posted {
    pub id: String,
    pub short: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tag: String,
}

/// The assistant's reply (`assistant.chat.said`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Said {
    pub turn: String,
    pub text: String,
    /// What the run's write tools changed, a line each (card #987).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<String>,
    /// The thread the reply posted by itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posted: Option<Posted>,
    /// The label of the screen it looked at, never its text.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub looked: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    /// The computer didn't get to it (§6).
    #[serde(skip_serializing_if = "is_false")]
    pub skipped: bool,
    /// The assistant's computer.
    pub device: String,
    pub at: i64,
    #[serde(skip_serializing_if = "is_false")]
    pub imported: bool,
}

/// What a follow-up line lets the person do: today `{kind: "answer",
/// card, blocker}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Act {
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub card: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub blocker: String,
}

/// A follow-up line (`assistant.chat.update`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Update {
    /// The thread or post and the fact: the first row per key wins.
    pub key: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub thread: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub act: Option<Act>,
    /// The tap's request it ends (§9).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub request: String,
    pub at: i64,
    #[serde(skip_serializing_if = "is_false")]
    pub imported: bool,
}

/// The person posted a reply's draft (`assistant.chat.posted`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PostedThread {
    pub turn: String,
    pub posted: Posted,
}

/// New chat (`assistant.chat.cleared`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Cleared {
    pub by: String,
    pub at: i64,
}

/// The person stopped a turn (`assistant.chat.stop`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Stop {
    pub turn: String,
    pub by: String,
    pub at: i64,
}

/// Let it answer or Answer this (`assistant.answer.requested`, §9).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AnswerRequested {
    pub request: String,
    pub post: String,
    pub space: String,
    /// `let` or `draft`.
    pub how: String,
    pub by: String,
    pub at: i64,
}

/// The owner's-OK draft (`assistant.answer.draft`, §9).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AnswerDraft {
    pub request: String,
    pub post: String,
    pub space: String,
    /// Who asked, as the space names them ("Eve", "Kit (Eve's agent)"):
    /// what the Send button says.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub asker: String,
    pub text: String,
    pub device: String,
    pub at: i64,
}

/// One row of the conversation, as its writer hands it to
/// [`Core::assistant_chat_write`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ChatRow {
    Asked(Asked),
    Said(Said),
    Update(Update),
    Posted(PostedThread),
    Cleared(Cleared),
    Stop(Stop),
    AnswerRequested(AnswerRequested),
    AnswerDraft(AnswerDraft),
}

impl ChatRow {
    pub fn kind(&self) -> &'static str {
        match self {
            ChatRow::Asked(_) => ASKED,
            ChatRow::Said(_) => SAID,
            ChatRow::Update(_) => UPDATE,
            ChatRow::Posted(_) => POSTED,
            ChatRow::Cleared(_) => CLEARED,
            ChatRow::Stop(_) => STOP,
            ChatRow::AnswerRequested(_) => ANSWER_REQUESTED,
            ChatRow::AnswerDraft(_) => ANSWER_DRAFT,
        }
    }

    /// The actor its row is written with.
    pub fn actor(&self) -> &'static str {
        actor_of(self.kind()).unwrap_or("irori")
    }

    /// Refuses a row the fold would never show: one with no id to fold by.
    fn check(&self) -> Result<(), String> {
        let (what, id) = match self {
            ChatRow::Asked(r) if r.files.len() > MAX_FILES => {
                return Err(format!("at most {MAX_FILES} files"));
            }
            ChatRow::Asked(r) => ("turn", &r.turn),
            ChatRow::Said(r) => ("turn", &r.turn),
            ChatRow::Posted(r) if r.posted.id.trim().is_empty() => {
                return Err(format!("an {} row needs a thread", self.kind()));
            }
            ChatRow::Posted(r) => ("turn", &r.turn),
            ChatRow::Stop(r) => ("turn", &r.turn),
            ChatRow::Update(r) => ("key", &r.key),
            ChatRow::AnswerRequested(r) => ("request", &r.request),
            ChatRow::AnswerDraft(r) => ("request", &r.request),
            ChatRow::Cleared(_) => return Ok(()),
        };
        if id.trim().is_empty() {
            return Err(format!("an {} row needs a {what}", self.kind()));
        }
        Ok(())
    }

    /// Runs `f` (the scrubber) over every text field, then cuts each to its
    /// most. Scrubbing first, so a cut never leaves half a secret that no
    /// longer matches.
    fn scrub(&mut self, f: &dyn Fn(&str) -> String) {
        let text = |s: &mut String| *s = cut_bytes(&f(s), MAX_TEXT);
        match self {
            ChatRow::Asked(r) => {
                text(&mut r.text);
                for file in &mut r.files {
                    text(&mut file.name);
                }
                r.screen = f(&r.screen).chars().take(MAX_SCREEN_CHARS).collect();
            }
            ChatRow::Said(r) => {
                text(&mut r.text);
                text(&mut r.error);
                text(&mut r.looked);
                for l in &mut r.changed {
                    text(l);
                }
            }
            ChatRow::Update(r) => text(&mut r.text),
            ChatRow::AnswerDraft(r) => text(&mut r.text),
            ChatRow::Posted(_)
            | ChatRow::Cleared(_)
            | ChatRow::Stop(_)
            | ChatRow::AnswerRequested(_) => {}
        }
    }
}

/// `s` cut to at most `max` bytes, on a character boundary.
fn cut_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// A row's place in the store's `(hlc, device_id, seq)` order.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Order {
    pub(crate) hlc: u64,
    pub(crate) device: String,
    pub(crate) seq: i64,
}

impl Order {
    pub(crate) fn of(ev: &Event) -> Order {
        Order {
            hlc: ev.hlc.0,
            device: ev.device_id.clone(),
            seq: ev.seq,
        }
    }
}

/// A row the fold keeps, with its place and the device that wrote it.
#[derive(Debug, Clone)]
struct Held<T> {
    at: Order,
    row: T,
}

/// Keeps `row` under `id` unless a row there comes first in store order.
fn first_wins<T>(m: &mut HashMap<String, Held<T>>, id: String, at: Order, row: T) -> bool {
    if m.get(&id).is_some_and(|cur| cur.at <= at) {
        return false;
    }
    m.insert(id, Held { at, row });
    true
}

#[derive(Debug, Default)]
struct State {
    asked: HashMap<String, Held<Asked>>,
    said: HashMap<String, Held<Said>>,
    posted: HashMap<String, Held<Posted>>,
    stops: HashMap<String, Held<Stop>>,
    updates: HashMap<String, Held<Update>>,
    drafts: HashMap<String, Held<AnswerDraft>>,
    /// Let it answer and Answer this taps, by request (§9).
    requests: HashMap<String, Held<AnswerRequested>>,
    /// The newest New chat.
    cleared: Option<Order>,
}

impl State {
    /// Whether New chat hides a row at `at`.
    fn hidden(&self, at: &Order, imported: bool) -> bool {
        self.cleared.as_ref().is_some_and(|c| imported || at < c)
    }
}

/// One turn: the question, and its reply once there is one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatTurn {
    pub asked: Asked,
    /// The device that wrote the question's row.
    pub asked_on: String,
    pub said: Option<Said>,
    /// The thread the reply posted by itself, or the person posted from its
    /// draft, whichever the fold has.
    pub posted: Option<Posted>,
    /// The person stopped it.
    pub stopped: bool,
}

/// One entry of the shown conversation.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatEntry {
    Turn(Box<ChatTurn>),
    Update(Update),
    Draft(AnswerDraft),
}

/// An entry and where it sits: pass its `cursor` as `before` for the next
/// (older) page.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatItem {
    pub cursor: String,
    pub entry: ChatEntry,
}

/// A page of the conversation, newest first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatPage {
    pub items: Vec<ChatItem>,
    /// Older entries remain.
    pub more: bool,
}

/// Where an entry shows: imported entries before every other, then store
/// order.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Place {
    later: bool,
    at: Order,
}

impl Place {
    fn new(imported: bool, at: &Order) -> Place {
        Place {
            later: !imported,
            at: at.clone(),
        }
    }

    fn cursor(&self) -> String {
        format!(
            "{}.{}.{}.{}",
            u8::from(self.later),
            self.at.hlc,
            self.at.seq,
            self.at.device
        )
    }

    fn parse(s: &str) -> Option<Place> {
        let mut it = s.splitn(4, '.');
        let later = match it.next()? {
            "0" => false,
            "1" => true,
            _ => return None,
        };
        let hlc = it.next()?.parse().ok()?;
        let seq = it.next()?.parse().ok()?;
        let device = it.next()?.to_string();
        Some(Place {
            later,
            at: Order { hlc, device, seq },
        })
    }
}

/// Folds the conversation's rows. Safe for concurrent use.
#[derive(Debug, Default)]
pub struct Fold {
    m: Mutex<State>,
    /// Held across an import's check and its writes, so two calls can't
    /// both pass the check.
    import_gate: Mutex<()>,
}

impl Fold {
    pub fn new() -> Fold {
        Fold::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.m.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Forgets every row.
    pub fn reset(&self) {
        *self.lock() = State::default();
    }

    /// Folds `st` again from the start (a resync) and only then puts the
    /// result in place, so a read meanwhile sees the old fold, never a
    /// half-loaded one. A row written while it reloads also reaches the
    /// kind loop, which folds it again after.
    pub fn reload(&self, st: &Store) {
        let fresh = Fold::new();
        load(st, &fresh);
        let next = std::mem::take(&mut *fresh.lock());
        *self.lock() = next;
    }

    /// Folds one row; reports whether the fold changed. A row folded twice
    /// (written here, then heard from the kind loop) changes nothing.
    /// The taps that nothing has served yet, oldest first: no `update`
    /// line or draft names the request (§9, "Serving writes a record").
    /// The tap `request` and where it sits in store order.
    pub(crate) fn request_at(&self, request: &str) -> Option<(Order, AnswerRequested)> {
        self.lock()
            .requests
            .get(request)
            .map(|h| (h.at.clone(), h.row.clone()))
    }

    /// The owner's-OK draft for tap `request`, if one was written.
    pub(crate) fn draft_for(&self, request: &str) -> Option<AnswerDraft> {
        self.lock().drafts.get(request).map(|h| h.row.clone())
    }

    /// The `update` line keyed `key`, if there is one.
    pub(crate) fn update_for(&self, key: &str) -> Option<Update> {
        self.lock().updates.get(key).map(|h| h.row.clone())
    }

    pub(crate) fn unserved_requests(&self) -> Vec<AnswerRequested> {
        let st = self.lock();
        let mut out: Vec<(&Order, &AnswerRequested)> = st
            .requests
            .values()
            .filter(|r| !st.drafts.contains_key(&r.row.request))
            .filter(|r| !st.updates.values().any(|u| u.row.request == r.row.request))
            .map(|r| (&r.at, &r.row))
            .collect();
        out.sort_by(|a, b| a.0.cmp(b.0));
        out.into_iter().map(|(_, r)| r.clone()).collect()
    }

    pub fn apply(&self, ev: &Event) -> bool {
        let Some(actor) = actor_of(&ev.kind) else {
            return false;
        };
        if ev.actor != actor {
            return false;
        }
        let at = Order::of(ev);
        let mut st = self.lock();
        match ev.kind.as_str() {
            ASKED => match decode::<Asked>(ev) {
                Some(r) if !r.turn.is_empty() => first_wins(&mut st.asked, r.turn.clone(), at, r),
                _ => false,
            },
            SAID => match decode::<Said>(ev) {
                Some(r) if !r.turn.is_empty() => first_wins(&mut st.said, r.turn.clone(), at, r),
                _ => false,
            },
            POSTED => match decode::<PostedThread>(ev) {
                Some(r) if !r.turn.is_empty() && !r.posted.id.is_empty() => {
                    first_wins(&mut st.posted, r.turn, at, r.posted)
                }
                _ => false,
            },
            STOP => match decode::<Stop>(ev) {
                Some(r) if !r.turn.is_empty() => first_wins(&mut st.stops, r.turn.clone(), at, r),
                _ => false,
            },
            UPDATE => match decode::<Update>(ev) {
                Some(r) if !r.key.is_empty() => first_wins(&mut st.updates, r.key.clone(), at, r),
                _ => false,
            },
            ANSWER_DRAFT => match decode::<AnswerDraft>(ev) {
                Some(r) if !r.request.is_empty() => {
                    first_wins(&mut st.drafts, r.request.clone(), at, r)
                }
                _ => false,
            },
            CLEARED => {
                if decode::<Cleared>(ev).is_none() || st.cleared.as_ref().is_some_and(|c| *c >= at)
                {
                    return false;
                }
                st.cleared = Some(at);
                true
            }
            // Requests are served by the assistant's computer (§9); they
            // show nothing by themselves.
            ANSWER_REQUESTED => {
                if let Some(r) = decode::<AnswerRequested>(ev).filter(|r| !r.request.is_empty()) {
                    first_wins(&mut st.requests, r.request.clone(), at, r);
                }
                false
            }
            _ => false,
        }
    }

    /// The shown conversation, in place order, oldest first.
    fn shown(&self) -> Vec<(Place, ChatEntry)> {
        let st = self.lock();
        let mut out = Vec::new();
        for (turn, a) in &st.asked {
            if st.hidden(&a.at, a.row.imported) {
                continue;
            }
            let said = st.said.get(turn).map(|s| s.row.clone());
            let posted = said
                .as_ref()
                .and_then(|s| s.posted.clone())
                .filter(|p| !p.id.is_empty())
                .or_else(|| st.posted.get(turn).map(|p| p.row.clone()));
            out.push((
                Place::new(a.row.imported, &a.at),
                ChatEntry::Turn(Box::new(ChatTurn {
                    asked: a.row.clone(),
                    asked_on: a.at.device.clone(),
                    said,
                    posted,
                    stopped: st.stops.contains_key(turn),
                })),
            ));
        }
        for u in st.updates.values() {
            if !st.hidden(&u.at, u.row.imported) {
                out.push((
                    Place::new(u.row.imported, &u.at),
                    ChatEntry::Update(u.row.clone()),
                ));
            }
        }
        for d in st.drafts.values() {
            if !st.hidden(&d.at, false) {
                out.push((Place::new(false, &d.at), ChatEntry::Draft(d.row.clone())));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// A page of the shown conversation, newest first: up to `limit`
    /// entries (0 is [`PAGE`], at most [`MAX_PAGE`]) older than the entry
    /// whose cursor is `before` ("" starts at the newest).
    pub fn list(&self, before: &str, limit: usize) -> Result<ChatPage, String> {
        let before = if before.is_empty() {
            None
        } else {
            Some(Place::parse(before).ok_or_else(|| format!("no chat position {before}"))?)
        };
        let limit = if limit == 0 {
            PAGE
        } else {
            limit.min(MAX_PAGE)
        };
        let mut older = self
            .shown()
            .into_iter()
            .rev()
            .filter(|(p, _)| before.as_ref().is_none_or(|b| p < b));
        let items = older
            .by_ref()
            .take(limit)
            .map(|(p, entry)| ChatItem {
                cursor: p.cursor(),
                entry,
            })
            .collect();
        Ok(ChatPage {
            items,
            more: older.next().is_some(),
        })
    }
}

/// How many turns of history go to the model (`HISTORY_CAP` turns are
/// built, as the chat kept; the prompt uses the last [`HISTORY_TURNS`]).
pub const HISTORY_CAP: usize = 50;
/// The turns the prompt shows, as the window's `render_prompt` does.
pub const HISTORY_TURNS: usize = 12;
/// The most of the person's earlier questions `grants_for` reads.
pub const EARLIER_CAP: usize = 50;
/// The header of a run of follow-up lines in the history (`historyFor`).
pub const UPDATES_HEAD: &str = "(The chat's own lines on the work it follows:)";

/// The head of what a held session hasn't seen since its last message
/// (card 8, §8).
pub const SINCE_HEAD: &str = "Since we last spoke:";

/// A turn as the worker reads it: where its question sits, whether New
/// chat hides it, and the turn.
#[derive(Debug, Clone)]
pub(crate) struct TurnAt {
    pub(crate) at: Order,
    pub(crate) hidden: bool,
    pub(crate) turn: ChatTurn,
}

/// What the history adds to a reply whose draft did not post: the model
/// must never say it did.
pub const NOT_POSTED: &str =
    "(This draft was not posted. It waits for the person to tap Post thread.)";

/// A reply as the history gives it to the model: with a posted draft's
/// thread named, or a draft that did not post said so.
fn reply_text(text: &str, posted: Option<&Posted>) -> String {
    match posted {
        Some(p) => format!(
            "{text}\n\n(This draft was posted as thread {}#{}.)",
            p.tag, p.short
        ),
        None if assistant_draft::has_draft(text).is_some() => format!("{text}\n\n{NOT_POSTED}"),
        None => text.to_string(),
    }
}

impl Fold {
    /// Turn `id`, hidden by New chat or not.
    pub(crate) fn turn_at(&self, id: &str) -> Option<TurnAt> {
        let st = self.lock();
        let a = st.asked.get(id)?;
        let said = st.said.get(id).map(|s| s.row.clone());
        let posted = said
            .as_ref()
            .and_then(|s| s.posted.clone())
            .filter(|p| !p.id.is_empty())
            .or_else(|| st.posted.get(id).map(|p| p.row.clone()));
        Some(TurnAt {
            at: a.at.clone(),
            hidden: st.hidden(&a.at, a.row.imported),
            turn: ChatTurn {
                asked: a.row.clone(),
                asked_on: a.at.device.clone(),
                said,
                posted,
                stopped: st.stops.contains_key(id),
            },
        })
    }

    /// Turn `id`, hidden or not.
    pub fn turn(&self, id: &str) -> Option<ChatTurn> {
        self.turn_at(id).map(|t| t.turn)
    }

    /// Whether any imported row is held, hidden by New chat or not: the
    /// import happens once.
    pub(crate) fn has_imported(&self) -> bool {
        let st = self.lock();
        st.asked.values().any(|a| a.row.imported)
            || st.said.values().any(|a| a.row.imported)
            || st.updates.values().any(|a| a.row.imported)
    }

    /// The questions with no reply that New chat doesn't hide, imported
    /// ones left out, in store order: each with its turn and whether the
    /// person stopped it.
    pub(crate) fn unanswered(&self) -> Vec<(Order, String, bool)> {
        let st = self.lock();
        let mut out: Vec<(Order, String, bool)> = st
            .asked
            .iter()
            .filter(|(turn, a)| {
                !a.row.imported && !st.said.contains_key(*turn) && !st.hidden(&a.at, false)
            })
            .map(|(turn, a)| (a.at.clone(), turn.clone(), st.stops.contains_key(turn)))
            .collect();
        out.sort();
        out
    }

    /// What the person typed before the question at `at`, after the newest
    /// New chat, imported questions left out: the newest `max`, oldest
    /// first. `grants_for` reads it, so "close it" means the card they last
    /// named, and nothing asked after the question ever counts.
    pub(crate) fn typed_before(&self, at: &Order, max: usize) -> Vec<String> {
        let st = self.lock();
        let mut rows: Vec<(&Order, &str)> = st
            .asked
            .values()
            .filter(|a| !a.row.imported && a.at < *at && !st.hidden(&a.at, false))
            .map(|a| (&a.at, a.row.text.as_str()))
            .collect();
        rows.sort();
        let skip = rows.len().saturating_sub(max);
        rows.into_iter()
            .skip(skip)
            .map(|(_, t)| t.to_string())
            .collect()
    }

    /// What a held session hasn't seen (card 8, §8), one line each, oldest
    /// first: the entries after the question at `from` and before the one
    /// at `at` that the session didn't write (follow-up lines, and turns
    /// another computer answered), then the threads posted from its own
    /// replies (`mine`) that it wasn't told of (`told`). Returns the lines
    /// and the turns whose threads they tell.
    pub(crate) fn since(
        &self,
        from: &Order,
        at: &Order,
        mine: &HashSet<String>,
        told: &HashSet<String>,
    ) -> (Vec<String>, Vec<String>) {
        let (lo, hi) = (Place::new(false, from), Place::new(false, at));
        let mut lines = Vec::new();
        let mut posted = Vec::new();
        for (place, entry) in self.shown() {
            match entry {
                ChatEntry::Turn(t) => {
                    if mine.contains(&t.asked.turn) {
                        if let Some(p) = t.posted.as_ref().filter(|_| !told.contains(&t.asked.turn))
                        {
                            lines.push(format!(
                                "- Your draft in reply to \"{}\" was posted as thread {}#{}.",
                                short_line(&t.asked.text),
                                p.tag,
                                p.short
                            ));
                            posted.push(t.asked.turn.clone());
                        } else if t.posted.is_none() {
                            // A draft meant to post by itself that didn't,
                            // told once (its own key, so a later tap on
                            // Post thread is still told).
                            let key = format!("{} {UNPOSTED}", t.asked.turn);
                            let unposted = t.said.as_ref().is_some_and(|s| {
                                assistant_draft::has_draft(&s.text) == Some(false)
                            });
                            if unposted && !told.contains(&key) {
                                lines.push(format!(
                                    "- Your draft in reply to \"{}\" was not posted. It waits for the person to tap Post thread.",
                                    short_line(&t.asked.text)
                                ));
                                posted.push(key);
                            }
                        }
                        continue;
                    }
                    if place <= lo || place >= hi {
                        continue;
                    }
                    let Some(said) = t.said.as_ref() else {
                        continue;
                    };
                    if said.skipped || !said.error.is_empty() {
                        continue;
                    }
                    lines.push(format!(
                        "- The person asked: {}\n  The reply: {}",
                        with_files(&t.asked),
                        cut_chars(
                            reply_text(&said.text, t.posted.as_ref()).trim(),
                            SINCE_REPLY
                        )
                    ));
                }
                ChatEntry::Update(u) if place > lo && place < hi => {
                    lines.push(format!("- {}", u.text.trim()));
                }
                _ => {}
            }
        }
        (lines, posted)
    }

    /// The conversation before the question at `at`, as history turns
    /// (`true`: the assistant's), the port of the chat's `historyFor`:
    /// each answered turn as the person's words and the reply, a posted
    /// draft naming its thread; each run of follow-up lines as one turn of
    /// the assistant's. Left out: turns with no reply yet, skipped or
    /// failed ones (an imported question with no reply is one), and
    /// drafts, which carry another person's words. The newest
    /// [`HISTORY_CAP`].
    pub(crate) fn history_before(&self, at: &Order) -> Vec<(bool, String)> {
        let upto = Place::new(false, at);
        let mut out: Vec<(bool, String)> = Vec::new();
        let mut ups: Vec<String> = Vec::new();
        let flush = |out: &mut Vec<(bool, String)>, ups: &mut Vec<String>| {
            if !ups.is_empty() {
                let lines: Vec<String> = ups.iter().map(|u| format!("- {u}")).collect();
                out.push((true, format!("{UPDATES_HEAD}\n{}", lines.join("\n"))));
                ups.clear();
            }
        };
        for (place, entry) in self.shown() {
            if place >= upto {
                break;
            }
            match entry {
                ChatEntry::Turn(t) => {
                    let Some(said) = t.said.as_ref() else {
                        continue;
                    };
                    if said.skipped || !said.error.is_empty() {
                        continue;
                    }
                    flush(&mut out, &mut ups);
                    out.push((false, with_files(&t.asked)));
                    out.push((true, reply_text(&said.text, t.posted.as_ref())));
                }
                ChatEntry::Update(u) => ups.push(u.text),
                ChatEntry::Draft(_) => {}
            }
        }
        flush(&mut out, &mut ups);
        let skip = out.len().saturating_sub(HISTORY_CAP);
        out.drain(..skip);
        out
    }
}

/// What marks a `told` key for a draft that did not post.
const UNPOSTED: &str = "unposted";

/// The most of another computer's reply a since line carries, in
/// characters: the message stays smaller than a seed.
const SINCE_REPLY: usize = 600;

/// `s` cut to `max` characters, with an ellipsis when cut.
fn cut_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max - 1).collect();
    format!("{}\u{2026}", cut.trim_end())
}

/// A question cut to one short line, for a since line.
fn short_line(text: &str) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    cut_chars(&one, 80)
}

/// A question's words for the history, with a line naming its files, those
/// read and those not (so the model never answers as if it had read one).
fn with_files(a: &Asked) -> String {
    let names = |can_read: bool| -> Vec<String> {
        a.files
            .iter()
            .filter(|f| f.can_read == can_read)
            .map(|f| format!("{:?}", f.name))
            .collect()
    };
    let (read, unread) = (names(true), names(false));
    let mut out = a.text.trim().to_string();
    for (head, list) in [("attached", read), ("attached, not readable", unread)] {
        if !list.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("[{head}: {}]", list.join(", ")));
        }
    }
    out
}

fn decode<T: for<'de> Deserialize<'de>>(ev: &Event) -> Option<T> {
    serde_json::from_slice(&ev.payload).ok()
}

/// Folds every conversation row in `st` into `f` (at start and on a
/// resync).
pub(super) fn load(st: &Store, f: &Fold) -> bool {
    // A store that can't be read leaves what was folded so far.
    let mut changed = false;
    let mut q = Filter {
        kinds: KINDS.iter().map(|k| k.to_string()).collect(),
        limit: store::MAX_LIST_LIMIT,
        ..Filter::default()
    };
    loop {
        let Ok(evs) = st.list(&q) else {
            return changed;
        };
        for ev in &evs {
            changed |= f.apply(ev);
        }
        if (evs.len() as i64) < q.limit {
            return changed;
        }
        q.after = evs[evs.len() - 1].cursor();
    }
}

impl Services {
    /// Writes one conversation row: refused without its id, scrubbed of
    /// secrets and cut to size before it is signed, written with its kind's
    /// actor, folded at once, and announced.
    pub(crate) fn write_chat(&self, mut row: ChatRow) -> Result<Event, String> {
        row.check()?;
        row.scrub(&|s| self.scrubber.string(s));
        let ev = self
            .store
            .append(row.actor(), row.kind(), &row)
            .map_err(|e| e.to_string())?;
        if self.assistant_chat.apply(&ev) {
            self.hub.publish(CoreEvent::AssistantChat);
        }
        Ok(ev)
    }
}

impl Core {
    /// A page of the assistant's conversation, newest first (see
    /// [`Fold::list`]).
    pub fn assistant_chat_list(&self, before: &str, limit: usize) -> Result<ChatPage, String> {
        self.s.assistant_chat.list(before, limit)
    }

    /// Writes one row of the assistant's conversation (see
    /// [`Services::write_chat`]). Only the window and the assistant's
    /// computer call it, never a tool: #987's grants are made from the
    /// question rows.
    pub fn assistant_chat_write(&self, row: ChatRow) -> Result<Event, String> {
        self.s.write_chat(row)
    }

    /// The person asks the assistant (one-assistant §5): writes the
    /// question's row with a new turn, this device and the time, and
    /// returns the turn. The assistant's computer answers it, wherever
    /// that is. `screen` is this device's snapshot; `again` the turn it
    /// asks again (Ask again), or "".
    pub fn assistant_chat_ask(
        &self,
        text: &str,
        screen: &str,
        again: &str,
    ) -> Result<String, String> {
        self.assistant_chat_ask_from(text, screen, again, "", Vec::new())
    }

    /// [`Self::assistant_chat_ask`] with the files that go with the
    /// question. A question may be files and no words.
    pub fn assistant_chat_ask_with(
        &self,
        text: &str,
        screen: &str,
        again: &str,
        files: Vec<AskedFile>,
    ) -> Result<String, String> {
        self.assistant_chat_ask_from(text, screen, again, "", files)
    }

    /// [`Self::assistant_chat_ask_with`] from a huddle (Divide with my
    /// assistant, call-agents §9): `call_id` is the huddle, or "". The
    /// assistant's computer then puts the huddle in the prompt, fenced.
    /// Refused when the huddle isn't here or isn't over.
    pub fn assistant_chat_ask_from(
        &self,
        text: &str,
        screen: &str,
        again: &str,
        call_id: &str,
        files: Vec<AskedFile>,
    ) -> Result<String, String> {
        if text.trim().is_empty() && files.is_empty() {
            return Err("ask something first".into());
        }
        let call_id = call_id.trim();
        if !call_id.is_empty() {
            self.s.huddle_divide_context(call_id)?;
        }
        let now = (self.s.now)();
        let turn = coord::new_id(&now);
        self.s.write_chat(ChatRow::Asked(Asked {
            turn: turn.clone(),
            text: text.trim().to_string(),
            screen: screen.trim().to_string(),
            device: self.s.device.id.clone(),
            at: now.unix_milli_value(),
            again: again.trim().to_string(),
            call_id: call_id.to_string(),
            files,
            ..Asked::default()
        }))?;
        Ok(turn)
    }

    /// The person stops `turn`, from any device: a turn that hasn't
    /// started never runs, and a running one ends at once.
    pub fn assistant_chat_stop(&self, turn: &str) -> Result<(), String> {
        if self.s.assistant_chat.turn(turn).is_none() {
            return Err(format!("no question {turn}"));
        }
        self.s.write_chat(ChatRow::Stop(Stop {
            turn: turn.to_string(),
            by: format!("user:{}", self.s.device.id),
            at: (self.s.now)().unix_milli_value(),
        }))?;
        Ok(())
    }

    /// Now, in unix milliseconds, as the rows stamp `at`.
    pub fn assistant_chat_now(&self) -> i64 {
        (self.s.now)().unix_milli_value()
    }

    /// Brings this computer's old chat history in, once (one-assistant §10,
    /// card #1105): refused unless this device is the assistant's computer
    /// and no imported row is held yet. The rows are `imported`, scrubbed
    /// like any row, written in the history's order, and never run.
    pub fn assistant_chat_import(&self, entries: &[ImportEntry]) -> Result<Imported, String> {
        let _gate = self
            .s
            .assistant_chat
            .import_gate
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !self.s.is_assistant_home() {
            return Err("only the assistant's computer imports its history".into());
        }
        if self.s.assistant_chat.has_imported() {
            return Err(ALREADY_IMPORTED.into());
        }
        if entries.len() > MAX_IMPORT {
            return Err(format!("at most {MAX_IMPORT} entries"));
        }
        let now = (self.s.now)();
        let mut new_turn = || coord::new_id(&now);
        let rows = import_rows(
            entries,
            &self.s.device.id,
            now.unix_milli_value(),
            &mut new_turn,
        );
        let mut done = Imported::default();
        for row in rows {
            match &row {
                ChatRow::Asked(_) => done.turns += 1,
                ChatRow::Update(_) => done.updates += 1,
                _ => {}
            }
            self.s.write_chat(row)?;
        }
        Ok(done)
    }

    /// Turn `turn` as the fold has it, hidden by New chat or not.
    pub fn assistant_chat_turn(&self, turn: &str) -> Option<ChatTurn> {
        self.s.assistant_chat.turn(turn)
    }

    /// Opens the reply of `turn` as the assistant's computer `device` runs
    /// it (card #1107): its events in order, from the text so far. The
    /// receiver ends when the turn does, when the stream drops, or at once
    /// when `device` is not connected or runs no such turn: the caller then
    /// waits for the `said` row, which is the reply whatever the stream
    /// carried. Dropping the receiver closes the stream.
    pub fn assistant_stream(
        &self,
        device: &str,
        turn: &str,
    ) -> Result<Receiver<ChatEvent>, String> {
        use mesh::assistant_stream as a;
        let node = self.s.mesh.node.clone();
        let (device, turn) = (device.to_string(), turn.to_string());
        let (tx, rx) = std::sync::mpsc::channel::<ChatEvent>();
        let (opened_tx, opened_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        self.pairing_runtime().spawn(async move {
            let opened = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                node.open_assistant_stream(&device, &turn),
            )
            .await;
            let mut st = match opened {
                Ok(Ok(st)) => {
                    let _ = opened_tx.send(Ok(()));
                    st
                }
                Ok(Err(e)) => {
                    let _ = opened_tx.send(Err(e));
                    return;
                }
                Err(_) => {
                    let _ = opened_tx.send(Err("assistant: the stream did not open".into()));
                    return;
                }
            };
            while let Some((kind, body)) = a::read_frame(&mut st).await {
                let k = match kind {
                    a::T_DELTA => "delta",
                    a::T_TOOL => "tool",
                    a::T_CHANGED => "changed",
                    a::T_DONE => "done",
                    a::T_ERROR => "error",
                    _ => continue,
                };
                let ev = ChatEvent {
                    kind: k.into(),
                    text: String::from_utf8_lossy(&body).into_owned(),
                };
                if tx.send(ev).is_err() {
                    return; // the watcher left: dropping st closes the stream
                }
                if kind == a::T_DONE || kind == a::T_ERROR {
                    return;
                }
            }
        });
        opened_rx
            .recv_timeout(std::time::Duration::from_secs(12))
            .map_err(|_| "assistant: the stream did not open".to_string())??;
        Ok(rx)
    }
}

// ---- moving today's history (card #1105, one-assistant.md §10) ---------------

/// The refusal for a second import. The chat treats it as done.
pub const ALREADY_IMPORTED: &str = "the history was already imported";

/// The most entries one import takes: the old chat kept 50.
pub const MAX_IMPORT: usize = 500;

/// The words on a person's question the old history has no reply for.
pub const NO_REPLY: &str = "(no reply)";

/// One entry of the old localStorage history, as `parseHistory` read it
/// (`frontend/src/lib/assistant.ts`'s `ChatEntry`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ImportEntry {
    /// `person` or `assistant`.
    pub role: String,
    pub text: String,
    /// A plain line of the chat's own (an error, "Stopped.").
    pub note: bool,
    /// A note saying what the chat changed.
    pub changed: bool,
    pub posted: Option<Posted>,
    /// The thread an assistant line follows: the entry is a follow-up line.
    pub update: String,
    pub act: Option<Act>,
    /// A reply sent on a thread by an older build: not a question.
    pub to: String,
    /// The label of the screen the reply looked at.
    pub looked: String,
}

/// What an import wrote.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Imported {
    pub turns: usize,
    pub updates: usize,
}

/// A reply being gathered from the assistant entries after one question.
#[derive(Default)]
struct Gathered {
    texts: Vec<String>,
    changed: Vec<String>,
    errors: Vec<String>,
    posted: Option<Posted>,
    looked: String,
}

impl Gathered {
    fn into_said(self, turn: &str, device: &str, at: i64) -> Said {
        let none = self.texts.is_empty() && self.changed.is_empty() && self.errors.is_empty();
        Said {
            turn: turn.into(),
            text: self.texts.join("\n\n"),
            changed: self.changed,
            posted: self.posted,
            looked: self.looked,
            error: if none {
                NO_REPLY.into()
            } else {
                self.errors.join("\n")
            },
            device: device.into(),
            at,
            imported: true,
            ..Said::default()
        }
    }
}

/// Maps the old history to rows (§10), in the order they must be written;
/// every row is `imported`. `new_turn` makes a turn id.
///
/// - A person's entry becomes an `asked` row.
/// - The assistant entries after it, up to the next person's entry, become
///   its `said` row: the text, its `posted`, its `changed` notes, and an
///   error note as `error`. `looked` keeps only the label.
/// - A question with no reply gets a `said` row whose `error` is
///   "(no reply)", so nothing imported is left unanswered.
/// - `update` entries become `update` rows keyed by thread and text.
/// - Assistant entries before the first question (the 50-line cap cut it),
///   and old `to` entries (and the assistant entries after them), are
///   dropped.
pub fn import_rows(
    entries: &[ImportEntry],
    device: &str,
    at: i64,
    new_turn: &mut dyn FnMut() -> String,
) -> Vec<ChatRow> {
    let mut rows = Vec::new();
    // The open question and its gathered reply.
    let mut open: Option<(String, Gathered)> = None;
    let close = |open: &mut Option<(String, Gathered)>, rows: &mut Vec<ChatRow>| {
        if let Some((turn, g)) = open.take() {
            rows.push(ChatRow::Said(g.into_said(&turn, device, at)));
        }
    };
    for e in entries {
        let text = e.text.trim();
        match e.role.as_str() {
            "person" => {
                close(&mut open, &mut rows);
                if !e.to.is_empty() {
                    continue;
                }
                let turn = new_turn();
                rows.push(ChatRow::Asked(Asked {
                    turn: turn.clone(),
                    text: if text.is_empty() {
                        "(attachment)"
                    } else {
                        text
                    }
                    .into(),
                    device: device.into(),
                    at,
                    imported: true,
                    ..Asked::default()
                }));
                open = Some((turn, Gathered::default()));
            }
            "assistant" if !e.update.is_empty() => {
                if text.is_empty() {
                    continue;
                }
                rows.push(ChatRow::Update(Update {
                    key: format!("{}:{}", e.update, text),
                    thread: e.update.clone(),
                    text: text.into(),
                    act: e.act.clone(),
                    at,
                    imported: true,
                    ..Update::default()
                }));
            }
            "assistant" => {
                let Some((_, g)) = open.as_mut() else {
                    continue;
                };
                if text.is_empty() {
                    continue;
                }
                if e.note && e.changed {
                    g.changed.push(text.into());
                } else if e.note {
                    g.errors.push(text.into());
                } else {
                    g.texts.push(text.into());
                    if g.posted.is_none() {
                        g.posted = e.posted.clone().filter(|p| !p.id.is_empty());
                    }
                    if g.looked.is_empty() {
                        g.looked = e.looked.trim().into();
                    }
                }
            }
            _ => {}
        }
    }
    close(&mut open, &mut rows);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use store::Timestamp;

    fn ev(kind: &str, actor: &str, hlc: u64, dev: &str, seq: i64, p: impl Serialize) -> Event {
        Event {
            device_id: dev.into(),
            seq,
            hlc: Timestamp(hlc),
            actor: actor.into(),
            kind: kind.into(),
            payload: serde_json::to_vec(&p).unwrap(),
            sig: Vec::new(),
        }
    }

    fn asked(turn: &str, text: &str) -> Asked {
        Asked {
            turn: turn.into(),
            text: text.into(),
            ..Asked::default()
        }
    }

    fn said(turn: &str, text: &str) -> Said {
        Said {
            turn: turn.into(),
            text: text.into(),
            ..Said::default()
        }
    }

    /// What a list shows, oldest first, as short strings.
    fn shown(f: &Fold) -> Vec<String> {
        let mut items = f.list("", MAX_PAGE).unwrap().items;
        items.reverse();
        items
            .into_iter()
            .map(|i| match i.entry {
                ChatEntry::Turn(t) => format!(
                    "{}>{}",
                    t.asked.text,
                    t.said.map(|s| s.text).unwrap_or_default()
                ),
                ChatEntry::Update(u) => format!("u:{}", u.text),
                ChatEntry::Draft(d) => format!("d:{}", d.text),
            })
            .collect()
    }

    /// Every order of `rows` folds to the same list.
    fn same_in_any_order(rows: &[Event]) -> Vec<String> {
        let a = Fold::new();
        rows.iter().for_each(|r| {
            a.apply(r);
        });
        let want = shown(&a);
        let b = Fold::new();
        rows.iter().rev().for_each(|r| {
            b.apply(r);
        });
        assert_eq!(shown(&b), want, "reversed");
        let c = Fold::new();
        for (i, r) in rows.iter().enumerate() {
            if i % 2 == 1 {
                c.apply(r);
            }
        }
        for (i, r) in rows.iter().enumerate() {
            if i % 2 == 0 {
                c.apply(r);
            }
        }
        assert_eq!(shown(&c), want, "interleaved");
        want
    }

    #[test]
    fn a_reply_waits_for_its_question() {
        let f = Fold::new();
        assert!(f.apply(&ev(SAID, "irori", 20, "mac", 1, said("t1", "four"))));
        assert!(shown(&f).is_empty());
        assert!(f.apply(&ev(ASKED, "user", 10, "phone", 1, asked("t1", "2+2?"))));
        assert_eq!(shown(&f), ["2+2?>four"]);
    }

    #[test]
    fn the_first_reply_and_the_first_line_per_key_win_in_any_order() {
        let rows = [
            ev(ASKED, "user", 10, "phone", 1, asked("t1", "q")),
            ev(SAID, "irori", 30, "pc", 1, said("t1", "second")),
            ev(SAID, "irori", 20, "mac", 1, said("t1", "first")),
            ev(
                UPDATE,
                "irori",
                40,
                "pc",
                2,
                Update {
                    key: "k".into(),
                    text: "pc's".into(),
                    ..Update::default()
                },
            ),
            ev(
                UPDATE,
                "irori",
                40,
                "mac",
                2,
                Update {
                    key: "k".into(),
                    text: "mac's".into(),
                    ..Update::default()
                },
            ),
        ];
        // At one hlc the lower device id comes first.
        assert_eq!(same_in_any_order(&rows), ["q>first", "u:mac's"]);
    }

    #[test]
    fn turns_and_lines_go_by_store_order_and_imports_go_first() {
        let rows = [
            ev(ASKED, "user", 10, "phone", 1, asked("t1", "early")),
            ev(SAID, "irori", 11, "mac", 1, said("t1", "a1")),
            ev(
                UPDATE,
                "irori",
                12,
                "mac",
                2,
                Update {
                    key: "k1".into(),
                    text: "line".into(),
                    ..Update::default()
                },
            ),
            ev(ASKED, "user", 13, "phone", 2, asked("t2", "later")),
            ev(
                ANSWER_DRAFT,
                "irori",
                14,
                "mac",
                3,
                AnswerDraft {
                    request: "r1".into(),
                    text: "draft".into(),
                    ..AnswerDraft::default()
                },
            ),
            ev(
                ASKED,
                "user",
                50,
                "mac",
                4,
                Asked {
                    imported: true,
                    ..asked("old", "imported q")
                },
            ),
            ev(
                SAID,
                "irori",
                51,
                "mac",
                5,
                Said {
                    imported: true,
                    ..said("old", "imported a")
                },
            ),
        ];
        assert_eq!(
            same_in_any_order(&rows),
            [
                "imported q>imported a",
                "early>a1",
                "u:line",
                "later>",
                "d:draft"
            ]
        );
    }

    #[test]
    fn posted_fills_a_reply_without_one_and_stop_marks_the_turn() {
        let f = Fold::new();
        f.apply(&ev(ASKED, "user", 10, "phone", 1, asked("t1", "q")));
        f.apply(&ev(SAID, "irori", 11, "mac", 1, said("t1", "a")));
        let p = |id: &str| Posted {
            id: id.into(),
            short: "12".into(),
            tag: String::new(),
        };
        f.apply(&ev(
            POSTED,
            "user",
            13,
            "pc",
            1,
            PostedThread {
                turn: "t1".into(),
                posted: p("later"),
            },
        ));
        f.apply(&ev(
            POSTED,
            "user",
            12,
            "phone",
            2,
            PostedThread {
                turn: "t1".into(),
                posted: p("first"),
            },
        ));
        f.apply(&ev(
            STOP,
            "user",
            14,
            "pc",
            2,
            Stop {
                turn: "t1".into(),
                ..Stop::default()
            },
        ));
        let page = f.list("", 0).unwrap();
        let ChatEntry::Turn(t) = &page.items[0].entry else {
            panic!("{page:?}")
        };
        assert_eq!(t.posted.as_ref().unwrap().id, "first");
        assert!(t.stopped);
        assert_eq!(t.asked_on, "phone");

        // A reply that posted by itself keeps its own.
        let g = Fold::new();
        g.apply(&ev(ASKED, "user", 10, "phone", 1, asked("t1", "q")));
        g.apply(&ev(
            SAID,
            "irori",
            11,
            "mac",
            1,
            Said {
                posted: Some(p("own")),
                ..said("t1", "a")
            },
        ));
        g.apply(&ev(
            POSTED,
            "user",
            12,
            "phone",
            2,
            PostedThread {
                turn: "t1".into(),
                posted: p("tap"),
            },
        ));
        let ChatEntry::Turn(t) = &g.list("", 0).unwrap().items[0].entry else {
            panic!()
        };
        assert_eq!(t.posted.as_ref().unwrap().id, "own");
    }

    #[test]
    fn the_history_says_which_drafts_did_not_post() {
        let draft = "Posting it.\n<<thread>>\nFix the lobby.\n<</thread>>";
        let unclosed = "Posting it.\n<<thread>>\nFix the stairs.";
        let check = "Waiting for you.\n<<thread check>>\nFix the roof.\n<</thread>>";
        let p = Posted {
            id: "id".into(),
            short: "12".into(),
            tag: String::new(),
        };
        let f = Fold::new();
        f.apply(&ev(ASKED, "user", 10, "phone", 1, asked("t1", "lobby")));
        f.apply(&ev(
            SAID,
            "irori",
            11,
            "mac",
            1,
            Said {
                posted: Some(p.clone()),
                ..said("t1", draft)
            },
        ));
        f.apply(&ev(ASKED, "user", 12, "phone", 2, asked("t2", "stairs")));
        f.apply(&ev(SAID, "irori", 13, "mac", 2, said("t2", unclosed)));
        f.apply(&ev(ASKED, "user", 14, "phone", 3, asked("t3", "roof")));
        f.apply(&ev(SAID, "irori", 15, "mac", 3, said("t3", check)));
        f.apply(&ev(ASKED, "user", 16, "phone", 4, asked("t4", "plain")));
        f.apply(&ev(
            SAID,
            "irori",
            17,
            "mac",
            4,
            said("t4", "Two cards wait."),
        ));
        f.apply(&ev(
            ASKED,
            "user",
            18,
            "phone",
            5,
            asked("t5", "where is it?"),
        ));
        let at = f.turn_at("t5").unwrap().at;
        let replies: Vec<String> = f
            .history_before(&at)
            .into_iter()
            .filter(|(mine, _)| *mine)
            .map(|(_, t)| t)
            .collect();
        assert_eq!(
            replies,
            [
                format!("{draft}\n\n(This draft was posted as thread #12.)"),
                format!("{unclosed}\n\n{NOT_POSTED}"),
                format!("{check}\n\n{NOT_POSTED}"),
                "Two cards wait.".to_string(),
            ]
        );

        // A held session is told once of its own draft that didn't post by
        // itself, and still told when a tap posts it later.
        let mine: HashSet<String> = ["t1", "t2", "t3", "t4"].map(String::from).into();
        let from = f.turn_at("t4").unwrap().at;
        let (lines, told) = f.since(&from, &at, &mine, &HashSet::new());
        assert_eq!(
            lines,
            [
                "- Your draft in reply to \"lobby\" was posted as thread #12.",
                "- Your draft in reply to \"stairs\" was not posted. It waits for the person to tap Post thread.",
            ]
        );
        let told: HashSet<String> = told.into_iter().collect();
        assert!(f.since(&from, &at, &mine, &told).0.is_empty());
        f.apply(&ev(
            POSTED,
            "user",
            19,
            "phone",
            6,
            PostedThread {
                turn: "t2".into(),
                posted: Posted {
                    short: "13".into(),
                    ..p
                },
            },
        ));
        assert_eq!(
            f.since(&from, &at, &mine, &told).0,
            ["- Your draft in reply to \"stairs\" was posted as thread #13."]
        );
    }

    #[test]
    fn new_chat_hides_what_came_before_it() {
        let line = |key: &str, text: &str| Update {
            key: key.into(),
            text: text.into(),
            ..Update::default()
        };
        let rows = [
            ev(
                ASKED,
                "user",
                5,
                "mac",
                1,
                Asked {
                    imported: true,
                    ..asked("old", "imported")
                },
            ),
            ev(ASKED, "user", 10, "phone", 1, asked("t1", "before")),
            ev(UPDATE, "irori", 11, "mac", 2, line("k1", "old line")),
            ev(
                ANSWER_DRAFT,
                "irori",
                12,
                "mac",
                3,
                AnswerDraft {
                    request: "r".into(),
                    text: "old draft".into(),
                    ..AnswerDraft::default()
                },
            ),
            ev(CLEARED, "user", 20, "phone", 2, Cleared::default()),
            // The reply to a hidden turn stays hidden, whenever it came.
            ev(SAID, "irori", 21, "mac", 4, said("t1", "late reply")),
            ev(ASKED, "user", 22, "phone", 3, asked("t2", "after")),
            ev(UPDATE, "irori", 23, "mac", 5, line("k2", "new line")),
            // An import written after New chat is still older than it.
            ev(
                ASKED,
                "user",
                30,
                "mac",
                6,
                Asked {
                    imported: true,
                    ..asked("old2", "imported late")
                },
            ),
            // An older New chat changes nothing.
            ev(CLEARED, "user", 15, "pc", 1, Cleared::default()),
        ];
        assert_eq!(same_in_any_order(&rows), ["after>", "u:new line"]);
    }

    #[test]
    fn a_row_with_the_wrong_actor_or_no_id_is_skipped() {
        let f = Fold::new();
        assert!(!f.apply(&ev(ASKED, "irori", 10, "mac", 1, asked("t1", "forged"))));
        assert!(!f.apply(&ev(SAID, "user", 11, "mac", 2, said("t1", "forged"))));
        assert!(!f.apply(&ev(ASKED, "agent:mac/x", 12, "mac", 3, asked("t2", "x"))));
        assert!(!f.apply(&ev(ASKED, "user", 13, "mac", 4, asked("", "no turn"))));
        assert!(!f.apply(&ev(CLEARED, "irori", 14, "mac", 5, Cleared::default())));
        let mut bad = ev(ASKED, "user", 15, "mac", 6, asked("t3", "x"));
        bad.payload = b"[1]".to_vec();
        assert!(!f.apply(&bad));
        assert!(shown(&f).is_empty());
        // Not one of the conversation's kinds.
        assert!(!f.apply(&ev(
            "assistant.names.set",
            "user",
            16,
            "mac",
            7,
            Cleared::default()
        )));
    }

    #[test]
    fn pages_go_newest_first_by_cursor() {
        let f = Fold::new();
        for i in 0..7u64 {
            f.apply(&ev(
                ASKED,
                "user",
                10 + i,
                "phone",
                i as i64 + 1,
                asked(&format!("t{i}"), &format!("q{i}")),
            ));
        }
        let text = |p: &ChatPage| -> Vec<String> {
            p.items
                .iter()
                .map(|i| match &i.entry {
                    ChatEntry::Turn(t) => t.asked.text.clone(),
                    _ => String::new(),
                })
                .collect()
        };
        let p1 = f.list("", 3).unwrap();
        assert_eq!(text(&p1), ["q6", "q5", "q4"]);
        assert!(p1.more);
        let p2 = f.list(&p1.items[2].cursor, 3).unwrap();
        assert_eq!(text(&p2), ["q3", "q2", "q1"]);
        assert!(p2.more);
        let p3 = f.list(&p2.items[2].cursor, 3).unwrap();
        assert_eq!(text(&p3), ["q0"]);
        assert!(!p3.more);
        assert!(f.list("nonsense", 3).is_err());
        assert_eq!(
            Place::parse(&p1.items[0].cursor).unwrap().cursor(),
            p1.items[0].cursor
        );
    }

    #[test]
    fn scrub_reaches_every_text_field_and_cuts() {
        let key = format!("sk-ant-api03-{}", "a1B2".repeat(10));
        let f = |s: &str| scrub::string(s);
        let mut a = ChatRow::Asked(Asked {
            turn: "t".into(),
            text: format!("my key {key}"),
            screen: format!("{key} {}", "x".repeat(MAX_SCREEN_CHARS)),
            ..Asked::default()
        });
        a.scrub(&f);
        let ChatRow::Asked(a) = a else { panic!() };
        assert_eq!(a.text, "my key [redacted:anthropic]");
        assert!(a.screen.starts_with("[redacted:anthropic]"), "{}", a.screen);
        assert_eq!(a.screen.chars().count(), MAX_SCREEN_CHARS);

        let mut s = ChatRow::Said(Said {
            turn: "t".into(),
            text: format!("{key} {}", "é".repeat(MAX_TEXT)),
            error: key.clone(),
            looked: key.clone(),
            changed: vec![key.clone()],
            ..Said::default()
        });
        s.scrub(&f);
        let ChatRow::Said(s) = s else { panic!() };
        assert!(s.text.starts_with("[redacted:anthropic] é"));
        assert!(s.text.len() <= MAX_TEXT && s.text.len() > MAX_TEXT - 4);
        assert_eq!(s.error, "[redacted:anthropic]");
        assert_eq!(s.looked, "[redacted:anthropic]");
        assert_eq!(s.changed, ["[redacted:anthropic]"]);

        for mut r in [
            ChatRow::Update(Update {
                key: "k".into(),
                text: key.clone(),
                ..Update::default()
            }),
            ChatRow::AnswerDraft(AnswerDraft {
                request: "r".into(),
                text: key.clone(),
                ..AnswerDraft::default()
            }),
        ] {
            r.scrub(&f);
            let v = serde_json::to_value(&r).unwrap();
            assert_eq!(v["text"], "[redacted:anthropic]", "{v}");
        }
    }

    #[test]
    fn rows_without_their_id_are_refused() {
        assert!(ChatRow::Asked(Asked::default()).check().is_err());
        assert!(ChatRow::Update(Update::default()).check().is_err());
        assert!(
            ChatRow::AnswerDraft(AnswerDraft::default())
                .check()
                .is_err()
        );
        assert!(ChatRow::Cleared(Cleared::default()).check().is_ok());
        assert!(
            ChatRow::Posted(PostedThread {
                turn: "t".into(),
                posted: Posted::default(),
            })
            .check()
            .is_err()
        );
    }

    #[test]
    fn rows_serialize_in_camel_case_and_leave_empties_out() {
        let v = serde_json::to_value(ChatRow::Asked(Asked {
            turn: "t".into(),
            text: "q".into(),
            call_id: "c".into(),
            ..Asked::default()
        }))
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({"turn": "t", "text": "q", "device": "", "at": 0, "callId": "c"})
        );
        let s: Said = serde_json::from_str(r#"{"turn":"t","later":"field"}"#).unwrap();
        assert_eq!(s.turn, "t");
    }
}
