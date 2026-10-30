//! The cards window (I:cards_service.go): the grid, one opened card and
//! the person's answer to a blocker, as BoardService's reads. Every list is
//! always serialized, so the webview never gets null or a missing array.
//!
//! Shared boards (v0.5, M9) are not here yet: every card is on the private
//! board, so Go's router, `boardCards` and `boardDetail` have nothing to
//! add.

use std::collections::{BTreeMap, HashMap};

use coord::FileRef;
use coord::{Activity, Comment, Link, Step, Task};
use gocompat::time::GoTime;
use inbox::{Delivery, st};
use serde::Serialize;
use specta::Type;

use crate::Window;
use crate::ts;

/// The only scope until Shared Boards land: the person's own devices.
pub const SCOPE_MINE: &str = "mine";
pub const SCOPE_MINE_LABEL: &str = "My devices";
/// How many activity lines Card returns.
const RECENT_LINES: usize = 8;
/// How many thread entries Card returns, the newest.
pub const MAX_THREAD_VIEW: usize = 300;
/// The blocker id of an input_required card's question.
pub const NOTE_ITEM: &str = "note";
const NOTE_QUESTION: &str = "The agent on it is waiting on your input.";
/// How recent agents' messages to each other on a card must be for its
/// tile to show them talking (card #22).
pub const TALK_WINDOW_MS: i64 = 5 * 60 * 1000;
const MAX_TALKING: usize = 4;
/// A working card with no activity this long, and nobody busy on it,
/// shows as stale (card #16).
pub const STALE_AFTER_MS: i64 = 2 * 3600 * 1000;

// ---- window types -------------------------------------------------------------

/// Someone on a card, as the window shows them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentRef {
    pub key: String,
    /// The roster's name, "You" for the person, else the key.
    pub name: String,
    /// The roster's status, "gone" without a live card, or "person".
    pub status: String,
    pub live: bool,
    /// "This PC", or the other device's name.
    #[serde(rename = "where")]
    pub where_: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub assigned: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub waiting: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub device: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub device_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub color: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub offline: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub conflict: String,
}

/// Done steps over all steps; total 0 means no bar.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CardProgress {
    pub done: i64,
    pub total: i64,
}

/// What a card's holder is doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Holding {
    #[default]
    Nobody,
    Person,
    Waiting,
    Assigned,
    Active,
    Idle,
    Offline,
    Gone,
}

/// The holder line.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CardHolder {
    pub who: AgentRef,
    pub state: Holding,
    pub others: i64,
}

/// Another card: a subtask's parent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CardRef {
    pub id: String,
    pub short: String,
    pub title: String,
}

/// Ask states, as the tile's line shows them.
pub const ASK_OPEN: &str = "open";
pub const ASK_WAITING: &str = "waiting";
pub const ASK_UNCLAIMED: &str = "unclaimed";
pub const ASK_TAKEN: &str = "taken";
pub const ASK_CLOSED: &str = "closed";

/// An ask on its tile and page.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardAsk {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub taker: Option<AgentRef>,
    pub told: i64,
    pub owner: String,
    pub wake: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional, type = Option<ts::Outcome>)]
    pub outcome: Option<coord::Outcome>,
    pub outcome_ref: String,
}

/// One card in the grid.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardSummary {
    pub id: String,
    pub short: String,
    pub short_was: String,
    pub title: String,
    #[specta(type = ts::TaskState)]
    pub state: String,
    pub repo_key: String,
    pub scope: String,
    pub scope_label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub board: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub tag: String,
    pub progress: CardProgress,
    pub open_blockers: i64,
    pub blocker_question: String,
    pub waiting: i64,
    pub working: Vec<AgentRef>,
    pub watching: Vec<AgentRef>,
    pub holder: CardHolder,
    #[specta(type = ts::Time)]
    pub updated_at: GoTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub returned: Option<AgentRef>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub stale: bool,
    pub devices: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub accent: String,
    pub talking: Vec<AgentRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub ask: Option<CardAsk>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub parent: Option<CardRef>,
    #[serde(skip_serializing_if = "is_zero")]
    #[specta(optional)]
    pub subtasks: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub subtask_blocked: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub pinned: bool,
    #[serde(skip_serializing_if = "is_zero_f")]
    #[specta(optional, type = i64)]
    pub pin_rank: f64,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

fn is_zero_f(n: &f64) -> bool {
    *n == 0.0
}

/// One row of a parent's subtask list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CardSubtask {
    pub id: String,
    pub short: String,
    pub title: String,
    #[specta(type = ts::TaskState)]
    pub state: String,
    pub holder: CardHolder,
    pub progress: CardProgress,
    pub blocked: bool,
}

/// One answer to pick.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CardOption {
    pub answer: String,
    pub why: String,
    pub by: AgentRef,
    #[specta(type = ts::Time)]
    pub at: GoTime,
    pub recommended: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional, type = Option<ts::Link>)]
    pub link: Option<Link>,
}

/// How a blocker was resolved.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CardResolution {
    pub answer: String,
    /// Index into options, or -1 for own words or a close.
    pub option: i64,
    pub by: AgentRef,
    #[specta(type = ts::Time)]
    pub at: GoTime,
    pub closed: bool,
    pub note: String,
}

/// Blocker states, CardBlocker.state.
pub const BLOCKER_OPEN: &str = "open";
pub const BLOCKER_ANSWERED: &str = "answered";
pub const BLOCKER_RESOLVED: &str = "resolved";
pub const BLOCKER_CLOSED: &str = "closed";

/// One blocker item, open or resolved.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardBlocker {
    pub id: String,
    pub question: String,
    pub context: String,
    pub raised_by: AgentRef,
    #[specta(type = ts::Time)]
    pub at: GoTime,
    pub options: Vec<CardOption>,
    /// null while open.
    pub resolved: Option<CardResolution>,
    pub state: String,
    #[specta(type = Vec<ts::Link>)]
    pub links: Vec<Link>,
    #[specta(type = Vec<ts::FileRef>)]
    pub attachments: Vec<FileRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub sent_to: Option<AgentRef>,
    pub missing: Vec<String>,
    pub thread: Vec<CardThreadItem>,
}

/// A pull request linked from the card.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardPR {
    pub repo: String,
    pub number: i64,
    pub url: String,
    pub label: String,
    pub opened_by: Vec<AgentRef>,
    pub reviewed_by: Vec<AgentRef>,
    pub touched_by: Vec<AgentRef>,
}

/// One line of Recent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CardActivity {
    #[specta(type = ts::Time)]
    pub at: GoTime,
    pub who: AgentRef,
    pub text: String,
}

/// Thread entry kinds.
pub const THREAD_COMMENT: &str = "comment";
pub const THREAD_MESSAGE: &str = "message";
pub const THREAD_EVENT: &str = "event";

/// One entry of a card's thread.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardThreadItem {
    pub id: String,
    pub kind: String,
    #[specta(type = ts::Time)]
    pub at: GoTime,
    pub who: AgentRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub to: Option<AgentRef>,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub device: String,
    pub text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub blocker_id: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[specta(optional)]
    pub missing: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub gone: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub held: bool,
    #[serde(skip_serializing_if = "is_zero")]
    #[specta(optional)]
    pub files: i64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub dismissed: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub delivery_id: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[specta(optional, type = Vec<ts::FileRef>)]
    pub attachments: Vec<FileRef>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub unheard: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[specta(optional)]
    pub pending: Vec<String>,
}

/// A message on a card's thread in full, for the person.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CardMessage {
    pub text: String,
    /// One line per attachment: name, size, type, hash.
    pub files: Vec<String>,
}

/// The shared board a card lives on (v0.5 §8). Always absent until M9.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardBoard {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub visible_to: Vec<String>,
    pub can_withdraw: bool,
}

/// Where a private card was shared to (Go `core.SharedCopy`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SharedCopy {
    pub board_id: String,
    pub board_name: String,
    pub task_id: String,
    #[serde(rename = "ref")]
    pub ref_: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub withdrawn: bool,
}

/// One card opened beside the grid.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardDetail {
    #[serde(flatten)]
    pub summary: CardSummary,
    pub goal: String,
    pub done_when: String,
    pub note: String,
    #[specta(type = Vec<ts::Step>)]
    pub steps: Vec<Step>,
    pub blockers: Vec<CardBlocker>,
    pub prs: Vec<CardPR>,
    #[specta(type = Vec<ts::Link>)]
    pub links: Vec<Link>,
    pub recent: Vec<CardActivity>,
    #[specta(type = Vec<ts::Comment>)]
    pub comments: Vec<Comment>,
    pub thread: Vec<CardThreadItem>,
    pub thread_earlier: i64,
    pub history_trimmed: bool,
    pub subtask_list: Vec<CardSubtask>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub on_board: Option<CardBoard>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub shared_to: Option<SharedCopy>,
}

/// One page of the Done tab.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct DonePage {
    pub cards: Vec<CardSummary>,
    /// The cursor for the next page ("" on the last).
    pub next: String,
}

// ---- people --------------------------------------------------------------------

type OwnerFn = Box<dyn Fn(&str) -> bool>;

/// Resolves keys against the roster once per call (Go `people`).
#[derive(Default)]
pub(crate) struct People {
    by_key: HashMap<String, coord::roster::AgentCard>,
    user: String,
    owner: Option<OwnerFn>,
    self_id: String,
    names: HashMap<String, String>,
    paired: bool,
    colors: BTreeMap<String, String>,
    conflicts: HashMap<String, String>,
    unsent: std::collections::HashSet<String>,
    heard: HashMap<String, Vec<Delivery>>,
}

fn unsent_key(agent: &str, r: &str) -> String {
    format!("{agent}\x00{r}")
}

impl Window {
    /// Go `BoardService.people`.
    pub(crate) fn people(&self) -> People {
        let c = &self.core;
        let mut p = People {
            user: c.user_sender().key,
            self_id: c.device_id().to_string(),
            ..People::default()
        };
        for d in c.inbox().open() {
            let m = &d.message;
            if m.notice
                && m.reason == coord::NOTICE_ASSIGNED
                && (d.state == st::QUEUED || d.state == st::HELD)
                && m.ref_.starts_with("task:")
            {
                p.unsent.insert(unsent_key(&d.to, &m.ref_));
            }
            if let Some(cid) = m.ref_.strip_prefix("comment:")
                && m.notice
                && m.reason == coord::NOTICE_COMMENT
                && (d.state == st::QUEUED || d.state == st::HELD || d.state == st::DELIVERED)
            {
                p.heard.entry(cid.to_string()).or_default().push(d.clone());
            }
        }
        // The Chain's rule, never coord's lock: `around` in person_comment runs
        // under it (coord.is_person would lock again).
        let core = self.core.clone();
        p.owner = Some(Box::new(move |k: &str| core.is_owner(k)));
        for l in c.coord().leases() {
            if !l.conflict.is_empty() {
                p.conflicts.insert(l.holder.clone(), l.resource.clone());
            }
        }
        for a in c.mesh_agents() {
            p.by_key.insert(a.key.clone(), a);
        }
        p.colors = c.device_colors();
        let mut live = 0;
        for d in c.certified_devices() {
            p.names
                .insert(d.cert.device_id.clone(), d.cert.label.clone());
            if !d.revoked {
                live += 1;
            }
        }
        p.paired = live > 1;
        p
    }
}

impl People {
    /// The device a key lives on: "This PC" for this one, else its label.
    pub(crate) fn where_(&self, dev: &str) -> String {
        if dev.is_empty() || self.self_id.is_empty() || dev == self.self_id {
            return "This PC".into();
        }
        match self.names.get(dev) {
            Some(n) if !n.is_empty() => n.clone(),
            _ => "Another device".into(),
        }
    }

    /// The device chip for something `dev` wrote.
    fn chip(&self, dev: &str) -> String {
        if !self.paired || dev.is_empty() || dev == self.self_id {
            return String::new();
        }
        self.where_(dev)
    }

    fn is_owner(&self, key: &str) -> bool {
        self.owner.as_ref().is_some_and(|f| f(key))
    }

    pub(crate) fn ref_(&self, key: &str) -> AgentRef {
        if key == "irori" {
            return AgentRef {
                key: key.into(),
                name: "Irori".into(),
                status: "irori".into(),
                live: true,
                where_: "This PC".into(),
                ..AgentRef::default()
            };
        }
        if key == self.user {
            return AgentRef {
                key: key.into(),
                name: "You".into(),
                status: "person".into(),
                live: true,
                where_: "This PC".into(),
                ..AgentRef::default()
            };
        }
        if self.is_owner(key) {
            let dev = key
                .strip_prefix(irori_identity::USER_KEY_PREFIX)
                .unwrap_or(key);
            return AgentRef {
                key: key.into(),
                name: "You".into(),
                status: "person".into(),
                live: true,
                where_: self.where_(dev),
                ..AgentRef::default()
            };
        }
        let mut dev = key
            .split_once('/')
            .map(|(d, _)| d)
            .unwrap_or("")
            .to_string();
        let mut r = AgentRef {
            key: key.into(),
            name: key.into(),
            status: "gone".into(),
            conflict: self.conflicts.get(key).cloned().unwrap_or_default(),
            ..AgentRef::default()
        };
        if let Some(a) = self.by_key.get(key) {
            if !a.name.is_empty() {
                r.name = a.name.clone();
            }
            r.status = a.status.clone();
            r.live = a.status != "gone";
            r.offline = a.device_state == "offline";
            if !a.device_id.is_empty() {
                dev = a.device_id.clone();
            }
        }
        r.where_ = self.where_(&dev);
        r.device = self.chip(&dev);
        if self.paired {
            r.color = self.colors.get(&dev).cloned().unwrap_or_default();
        }
        r.device_id = dev;
        r
    }

    pub(crate) fn refs(&self, keys: &[String]) -> Vec<AgentRef> {
        keys.iter().map(|k| self.ref_(k)).collect()
    }
}

fn ms(t: &GoTime) -> i64 {
    t.unix_milli_value()
}

// ---- summaries -----------------------------------------------------------------

pub(crate) fn card_summary(t: &Task, p: &People) -> CardSummary {
    let (d, n) = t.progress();
    let mut s = CardSummary {
        id: t.id.clone(),
        short: t.short.clone(),
        short_was: t.short_was.clone(),
        title: t.title.clone(),
        state: coord::effective_state(t).to_string(),
        repo_key: t.repo_key.clone(),
        scope: SCOPE_MINE.into(),
        scope_label: SCOPE_MINE_LABEL.into(),
        progress: CardProgress { done: d, total: n },
        updated_at: t.updated_at,
        subtasks: t.subtasks.len() as i64,
        pinned: t.pinned,
        pin_rank: t.pin_rank,
        ..CardSummary::default()
    };
    if let Some(r) = &t.rollup {
        s.progress = CardProgress {
            done: r.done,
            total: r.total,
        };
    }
    let open = t.open_blockers();
    if !open.is_empty() {
        for bl in &open {
            if bl.status == coord::BLOCKER_ANSWERED {
                s.waiting += 1;
                continue;
            }
            s.open_blockers += 1;
            if s.open_blockers == 1 {
                s.blocker_question = bl.question.clone();
            }
        }
    } else if t.state == coord::BLOCKED {
        s.open_blockers = 1;
        s.blocker_question = t.reason.clone();
    } else if t.state == coord::INPUT_REQUIRED {
        s.open_blockers = 1;
        let note = crate::trim(&t.note);
        s.blocker_question = if note.is_empty() {
            NOTE_QUESTION.into()
        } else {
            note.to_string()
        };
    }
    for h in &t.holders {
        let mut r = p.ref_(h);
        r.assigned = t.assigned.contains(h);
        r.waiting = r.assigned && p.unsent.contains(&unsent_key(h, &format!("task:{}", t.id)));
        s.working.push(r);
    }
    for w in &t.watchers {
        if !t.holders.contains(w) {
            s.watching.push(p.ref_(w));
        }
    }
    s.holder = holder_line(&s.working);
    for r in &s.working {
        if !r.device_id.is_empty() && !s.devices.contains(&r.device_id) {
            s.devices.push(r.device_id.clone());
        }
    }
    if s.devices.len() == 1 && p.paired {
        s.accent = p.colors.get(&s.devices[0]).cloned().unwrap_or_default();
    }
    s.ask = card_ask(t, p, None);
    if let Some(ret) = &t.returned
        && t.holders.is_empty()
    {
        s.returned = Some(p.ref_(&ret.was));
    }
    s
}

/// `t`'s CardAsk, or None when `t` isn't an ask. `refr` names another
/// card by id.
pub(crate) fn card_ask(
    t: &Task,
    p: &People,
    refr: Option<&dyn Fn(&str) -> String>,
) -> Option<CardAsk> {
    let a = t.ask.as_ref()?;
    let mut out = CardAsk {
        told: a.told,
        owner: p.where_(&a.owner),
        wake: a.wake,
        ..CardAsk::default()
    };
    if let Some(o) = &a.outcome {
        out.state = ASK_CLOSED.into();
        out.outcome = Some(o.clone());
        if !o.task.is_empty()
            && let Some(f) = refr
        {
            out.outcome_ref = f(&o.task);
        }
    } else if !a.taker.is_empty() {
        out.state = ASK_TAKEN.into();
        out.taker = Some(p.ref_(&a.taker));
    } else if !a.waiting.is_empty() {
        out.state = ASK_WAITING.into();
    } else if a.unclaimed {
        out.state = ASK_UNCLAIMED.into();
    } else {
        out.state = ASK_OPEN.into();
    }
    Some(out)
}

/// The agents that wrote to each other on a card within the talk window,
/// newest first, when there are at least two.
fn talking(cs: &[Comment], p: &People, now: i64) -> Vec<AgentRef> {
    let mut keys: Vec<String> = Vec::new();
    let add = |k: &str, keys: &mut Vec<String>| {
        if k.is_empty()
            || k == "irori"
            || k == p.user
            || p.is_owner(k)
            || keys.iter().any(|x| x == k)
        {
            return;
        }
        keys.push(k.to_string());
    };
    for c in cs.iter().rev() {
        if now - ms(&c.at) > TALK_WINDOW_MS {
            break;
        }
        add(&c.by, &mut keys);
        add(&c.to, &mut keys);
    }
    if keys.len() < 2 {
        return Vec::new();
    }
    keys.truncate(MAX_TALKING);
    p.refs(&keys)
}

fn stale(s: &CardSummary, last: &GoTime, now: i64) -> bool {
    if s.state != coord::WORKING
        || s.holder.state == Holding::Active
        || s.holder.state == Holding::Person
    {
        return false;
    }
    !last.is_zero() && now - ms(last) >= STALE_AFTER_MS
}

/// The holder that took the card up (the first), else the first assigned.
pub(crate) fn holder_line(working: &[AgentRef]) -> CardHolder {
    if working.is_empty() {
        return CardHolder {
            state: Holding::Nobody,
            ..CardHolder::default()
        };
    }
    let i = working.iter().position(|r| !r.assigned).unwrap_or(0);
    let w = working[i].clone();
    let state = if w.status == "person" {
        Holding::Person
    } else if w.waiting {
        Holding::Waiting
    } else if w.assigned {
        Holding::Assigned
    } else if w.status == "gone" {
        Holding::Gone
    } else if w.offline {
        Holding::Offline
    } else if w.status == "idle" {
        Holding::Idle
    } else {
        Holding::Active
    };
    CardHolder {
        who: w,
        state,
        others: working.len() as i64 - 1,
    }
}

fn open_ask_summary(s: &CardSummary) -> bool {
    s.ask.as_ref().is_some_and(|a| a.state != ASK_CLOSED)
}

/// Go `time.Time.After` on the cards' order: newest first, then id.
fn newer(a: &GoTime, a_id: &str, b: &GoTime, b_id: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match b.unix_nano().cmp(&a.unix_nano()) {
        Ordering::Equal => b_id.cmp(a_id),
        o => o,
    }
}

impl Window {
    fn now_ms(&self) -> i64 {
        ms(&GoTime::from_system(std::time::SystemTime::now()))
    }

    /// BoardService.Cards: every live card for the grid.
    pub fn cards(&self) -> Vec<CardSummary> {
        let co = self.core.coord();
        let p = self.people();
        let now = self.now_ms();
        let tasks = co.list_tasks(&coord::TaskFilter {
            open: true,
            ..Default::default()
        });
        let mut out = Vec::with_capacity(tasks.len());
        for t in &tasks {
            let mut s = card_summary(t, &p);
            let mut last = t.updated_at;
            if let Some(acts) = co.activity(&t.id)
                && let Some(a) = acts.first()
                && a.at.unix_nano() > last.unix_nano()
            {
                last = a.at;
            }
            s.stale = s.subtasks == 0 && stale(&s, &last, now);
            s.talking = talking(&co.comments(&t.id).unwrap_or_default(), &p, now);
            self.family_of(t, &mut s);
            out.push(s);
        }
        out.sort_by(|a, b| {
            let (aa, ab) = (open_ask_summary(a), open_ask_summary(b));
            if aa != ab {
                return ab.cmp(&aa);
            }
            let (ba, bb) = (a.open_blockers > 0, b.open_blockers > 0);
            if ba != bb {
                return bb.cmp(&ba);
            }
            newer(&a.updated_at, &a.id, &b.updated_at, &b.id)
        });
        let mut pins: Vec<CardSummary> = out.iter().filter(|s| s.pinned).cloned().collect();
        pins.sort_by(|a, b| {
            a.pin_rank
                .partial_cmp(&b.pin_rank)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let by_id: HashMap<String, CardSummary> =
            out.iter().map(|s| (s.id.clone(), s.clone())).collect();
        let kids: HashMap<String, Vec<String>> = tasks
            .iter()
            .map(|t| (t.id.clone(), t.subtasks.clone()))
            .collect();
        let mut pinned = Vec::new();
        let mut used = std::collections::HashSet::new();
        for s in pins {
            if used.contains(&s.id) {
                continue;
            }
            used.insert(s.id.clone());
            let id = s.id.clone();
            pinned.push(s);
            for k in kids.get(&id).into_iter().flatten() {
                if let Some(ks) = by_id.get(k)
                    && !ks.pinned
                {
                    pinned.push(ks.clone());
                    used.insert(k.clone());
                }
            }
        }
        let rest: Vec<CardSummary> = out.into_iter().filter(|s| !used.contains(&s.id)).collect();
        pinned.extend(families(rest, &tasks));
        pinned
    }

    /// BoardService.DoneCards: one page of the closed cards.
    pub fn done_cards(&self, limit: i64, before: &str) -> Result<DonePage, String> {
        let co = self.core.coord();
        let limit = if limit <= 0 {
            coord::DEFAULT_DONE_CARD
        } else {
            limit
        }
        .min(coord::MAX_DONE_CARD) as usize;
        let before = crate::trim(before);
        let mut cursor: Option<(GoTime, String)> = None;
        if !before.is_empty() {
            let bad = || {
                format!(
                    "{}: cursor {}",
                    coord::ErrorKind::Invalid.text(),
                    gocompat::quote::quote(before)
                )
            };
            let (ts, id) = before.split_once(' ').ok_or_else(bad)?;
            let at = GoTime::parse_rfc3339(ts).ok_or_else(bad)?;
            if id.is_empty() {
                return Err(bad());
            }
            cursor = Some((at, id.to_string()));
        }
        let mut done: Vec<Task> = co
            .list_tasks(&coord::TaskFilter::default())
            .into_iter()
            .filter(|t| coord::is_terminal(coord::effective_state(t)))
            .collect();
        done.sort_by(|a, b| newer(&a.updated_at, &a.id, &b.updated_at, &b.id));
        if let Some((at, id)) = &cursor {
            let i = done.partition_point(|t| {
                newer(&t.updated_at, &t.id, at, id) == std::cmp::Ordering::Less
            });
            let i = if i < done.len() && done[i].id == *id && done[i].updated_at.equal(at) {
                i + 1
            } else {
                i
            };
            done.drain(..i);
        }
        let mut out = DonePage::default();
        if done.len() > limit {
            let last = &done[limit - 1];
            out.next = format!(
                "{} {}",
                last.updated_at.utc().format_rfc3339_nano(),
                last.id
            );
            done.truncate(limit);
        }
        let p = self.people();
        let refr = |id: &str| co.get_task(id).map(|x| x.short_ref()).unwrap_or_default();
        for t in &done {
            let mut s = card_summary(t, &p);
            s.ask = card_ask(t, &p, Some(&refr));
            out.cards.push(s);
        }
        Ok(out)
    }

    /// The subtask fields that need other cards.
    fn family_of(&self, t: &Task, s: &mut CardSummary) {
        let co = self.core.coord();
        if !t.parent.is_empty()
            && let Some(pt) = co.get_task(&t.parent)
        {
            s.parent = Some(CardRef {
                id: pt.id.clone(),
                short: pt.short.clone(),
                title: pt.title.clone(),
            });
        }
        if let Some(r) = &t.rollup
            && !r.blocked.is_empty()
            && let Some(k) = co.get_task(&r.blocked)
        {
            s.subtask_blocked = format!("{}: {}", k.short_ref(), r.blocked_question);
        }
    }

    /// The cards waiting on a decision (the tray's number).
    pub fn decide_count(&self) -> usize {
        let p = People::default();
        self.core
            .coord()
            .list_tasks(&coord::TaskFilter {
                open: true,
                ..Default::default()
            })
            .iter()
            .filter(|t| card_summary(t, &p).open_blockers > 0)
            .count()
    }

    /// BoardService.Card: one card opened.
    pub fn card(&self, id: &str) -> Result<CardDetail, String> {
        let co = self.core.coord();
        let id = crate::trim(id);
        let Some(t) = co.get_task(id) else {
            return Err(not_found_task(id));
        };
        let comments = co.comments(&t.id).unwrap_or_default();
        let acts = co.activity(&t.id).unwrap_or_default();
        let p = self.people();
        let mut d = CardDetail {
            summary: card_summary(&t, &p),
            goal: t.objective.clone(),
            done_when: t.done_criteria.clone(),
            note: t.note.clone(),
            steps: t.steps.clone(),
            links: t.links.clone(),
            comments,
            ..CardDetail::default()
        };
        self.family_of(&t, &mut d.summary);
        for kid in &t.subtasks {
            let Some(k) = co.get_task(kid) else { continue };
            let ks = card_summary(&k, &p);
            d.subtask_list.push(CardSubtask {
                id: k.id.clone(),
                short: k.short.clone(),
                title: k.title.clone(),
                state: ks.state.clone(),
                holder: ks.holder.clone(),
                progress: ks.progress.clone(),
                blocked: ks.open_blockers > 0,
            });
        }
        let refr = |id: &str| co.get_task(id).map(|x| x.short_ref()).unwrap_or_default();
        d.summary.ask = card_ask(&t, &p, Some(&refr));
        let (thread, mut by_blocker, earlier) = build_thread(&d.comments, &acts, &t, &p);
        d.thread = thread;
        d.thread_earlier = earlier as i64;
        self.held_on_thread(&mut d.thread);
        for items in by_blocker.values_mut() {
            self.held_on_thread(items);
        }
        d.history_trimmed = acts.len() == coord::MAX_ACTIVITY;
        let mut resolved = Vec::new();
        if t.state == coord::INPUT_REQUIRED {
            d.blockers.push(CardBlocker {
                id: NOTE_ITEM.into(),
                question: d.summary.blocker_question.clone(),
                raised_by: p.ref_(&t.updated_by),
                at: t.updated_at,
                state: BLOCKER_OPEN.into(),
                ..CardBlocker::default()
            });
        }
        for bl in &t.blockers {
            let mut cb = CardBlocker {
                id: bl.id.clone(),
                question: bl.question.clone(),
                context: bl.context.clone(),
                raised_by: p.ref_(&bl.raised_by),
                at: bl.at,
                state: if bl.status.is_empty() {
                    BLOCKER_OPEN.into()
                } else {
                    bl.status.clone()
                },
                missing: bl.missing.clone(),
                thread: by_blocker.remove(&bl.id).unwrap_or_default(),
                links: bl.links.clone(),
                attachments: bl.attachments.clone(),
                ..CardBlocker::default()
            };
            for o in &bl.options {
                cb.options.push(CardOption {
                    answer: o.answer.clone(),
                    why: o.why.clone(),
                    by: p.ref_(&o.by),
                    at: o.at,
                    recommended: o.recommended,
                    link: o.link.clone(),
                });
            }
            if let Some(r) = &bl.resolved {
                cb.resolved = Some(CardResolution {
                    answer: r.answer.clone(),
                    option: r.option,
                    by: p.ref_(&r.by),
                    at: r.at,
                    closed: r.closed,
                    note: r.note.clone(),
                });
                cb.state = if r.closed {
                    BLOCKER_CLOSED
                } else {
                    BLOCKER_RESOLVED
                }
                .into();
                cb.missing = Vec::new();
                resolved.push(cb);
                continue;
            }
            if cb.state == BLOCKER_ANSWERED && !bl.sent_to.is_empty() {
                cb.sent_to = Some(p.ref_(&bl.sent_to));
            }
            d.blockers.push(cb);
        }
        resolved.reverse();
        d.blockers.extend(resolved);
        for l in &t.links {
            if let Some(pr) = self.card_pr(l, &p) {
                d.prs.push(pr);
            }
        }
        for a in acts.iter().take(RECENT_LINES) {
            let who = p.ref_(&a.by);
            d.recent.push(CardActivity {
                at: a.at,
                text: format!("{} {}", who.name, a.what),
                who,
            });
        }
        Ok(d)
    }

    /// BoardService.Message: a message on a card's thread in full.
    pub fn message(&self, task_id: &str, msg_id: &str) -> Result<CardMessage, String> {
        let (task_id, msg_id) = (crate::trim(task_id), crate::trim(msg_id));
        let nf = coord::ErrorKind::NotFound.text();
        let cm = match self.core.coord().get_comment(msg_id) {
            Some(cm) if !cm.msg_id.is_empty() && cm.task_id == task_id => cm,
            _ => {
                return Err(format!(
                    "{nf}: no message {} on task {}",
                    gocompat::quote::quote(msg_id),
                    gocompat::quote::quote(task_id)
                ));
            }
        };
        let Some(m) = self.core.inbox().message(&cm.msg_id) else {
            return Err(format!(
                "{nf}: message {} isn't on this device yet",
                gocompat::quote::quote(msg_id)
            ));
        };
        Ok(CardMessage {
            text: m.text.clone(),
            files: m
                .attachments
                .iter()
                .map(|f| crate::inbox_blob_ref(f).describe())
                .collect(),
        })
    }

    /// The delivery behind each held message on a thread.
    fn held_on_thread(&self, items: &mut [CardThreadItem]) {
        for it in items.iter_mut() {
            if !it.held || it.kind != THREAD_MESSAGE {
                continue;
            }
            for d in self.core.inbox().of_message(&it.id) {
                if d.state == st::HELD {
                    it.delivery_id = d.id.clone();
                    it.dismissed = false;
                    break;
                }
                if d.state == st::DISMISSED && it.delivery_id.is_empty() {
                    it.delivery_id = d.id.clone();
                    it.dismissed = true;
                }
            }
        }
    }

    /// A GitHub PR link and what capture's ledger knows of it.
    fn card_pr(&self, l: &Link, p: &People) -> Option<CardPR> {
        let (repo, n) = coord::pr_link(l)?;
        let mut pr = CardPR {
            repo: repo.clone(),
            number: n,
            url: l.reference.clone(),
            label: l.label.clone(),
            ..CardPR::default()
        };
        let key = ledger::key(&repo, n);
        let mut seen = std::collections::HashSet::new();
        for e in self.core.capture().prs(&repo) {
            let k = format!("{}\x00{}", e.role.as_str(), e.session);
            if e.key != key || !seen.insert(k) {
                continue;
            }
            let r = p.ref_(&e.session);
            if e.role == ledger::Role::AUTHOR {
                pr.opened_by.push(r);
            } else if e.role == ledger::Role::REVIEWER {
                pr.reviewed_by.push(r);
            } else {
                pr.touched_by.push(r);
            }
        }
        Some(pr)
    }
}

pub(crate) fn not_found_task(id: &str) -> String {
    format!(
        "{}: task {}",
        coord::ErrorKind::NotFound.text(),
        gocompat::quote::quote(id)
    )
}

/// Keeps a parent's subtasks right after it and moves the family to where
/// its most pressing card sorted.
fn families(sorted: Vec<CardSummary>, tasks: &[Task]) -> Vec<CardSummary> {
    let kids: HashMap<&str, &Vec<String>> = tasks
        .iter()
        .filter(|t| !t.subtasks.is_empty())
        .map(|t| (t.id.as_str(), &t.subtasks))
        .collect();
    let at: HashMap<String, usize> = sorted
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.clone(), i))
        .collect();
    let head = |s: &CardSummary| -> String {
        if let Some(p) = &s.parent
            && at.contains_key(&p.id)
        {
            return p.id.clone();
        }
        s.id.clone()
    };
    let mut out = Vec::with_capacity(sorted.len());
    let mut placed = std::collections::HashSet::new();
    for s in &sorted {
        let h = head(s);
        if !placed.insert(h.clone()) {
            continue;
        }
        out.push(sorted[at[&h]].clone());
        for k in kids.get(h.as_str()).into_iter().flat_map(|v| v.iter()) {
            if let Some(&i) = at.get(k) {
                out.push(sorted[i].clone());
            }
        }
    }
    out
}

/// A row's place in the log: (HLC, device, seq).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ThreadPos {
    hlc: u64,
    dev: String,
    seq: i64,
}

/// Merges a card's comments and its activity (newest first) into one
/// thread in log order, keeping the newest MAX_THREAD_VIEW.
fn build_thread(
    comments: &[Comment],
    acts: &[Activity],
    t: &Task,
    p: &People,
) -> (
    Vec<CardThreadItem>,
    HashMap<String, Vec<CardThreadItem>>,
    usize,
) {
    let kept: std::collections::HashSet<&str> = t.blockers.iter().map(|b| b.id.as_str()).collect();
    let mut all: Vec<(ThreadPos, CardThreadItem)> = Vec::with_capacity(comments.len() + acts.len());
    let mut mine: Vec<(ThreadPos, CardThreadItem)> = Vec::new();
    for cm in comments {
        let mut it = CardThreadItem {
            id: cm.id.clone(),
            kind: THREAD_COMMENT.into(),
            at: cm.at,
            who: p.ref_(&cm.by),
            device: p.chip(&cm.device),
            text: cm.text.clone(),
            gone: cm.gone,
            held: cm.held,
            files: cm.files,
            attachments: cm.attachments.clone(),
            blocker_id: cm.blocker_id.clone(),
            missing: cm.missing.clone(),
            unheard: cm.unheard,
            ..CardThreadItem::default()
        };
        if !cm.msg_id.is_empty() {
            it.kind = THREAD_MESSAGE.into();
        } else if it.who.status == "person" {
            it.pending = pending_lines(p.heard.get(&cm.id).map(Vec::as_slice).unwrap_or(&[]), t, p);
        }
        if !cm.to.is_empty() {
            it.to = Some(p.ref_(&cm.to));
        }
        let pos = ThreadPos {
            hlc: cm.hlc,
            dev: cm.device.clone(),
            seq: cm.seq,
        };
        if !cm.blocker_id.is_empty() && kept.contains(cm.blocker_id.as_str()) {
            mine.push((pos, it));
            continue;
        }
        all.push((pos, it));
    }
    mine.sort_by(|a, b| a.0.cmp(&b.0));
    let mut by_blocker: HashMap<String, Vec<CardThreadItem>> = HashMap::new();
    for (_, it) in mine {
        by_blocker
            .entry(it.blocker_id.clone())
            .or_default()
            .push(it);
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    for a in acts.iter().rev() {
        if a.kind == store::kinds::TASK_COMMENT && a.blocker_id.is_empty() {
            continue;
        }
        let mut id = format!("{}:{}", a.device, a.seq);
        let n = seen.get(&id).copied().unwrap_or(0);
        if n > 0 {
            seen.insert(id.clone(), n + 1);
            id = format!("{id}.{n}");
        } else {
            seen.insert(id.clone(), 1);
        }
        let who = p.ref_(&a.by);
        all.push((
            ThreadPos {
                hlc: a.hlc,
                dev: a.device.clone(),
                seq: a.seq,
            },
            CardThreadItem {
                id,
                kind: THREAD_EVENT.into(),
                at: a.at,
                device: p.chip(&a.device),
                text: format!("{} {}", who.name, a.what),
                blocker_id: a.blocker_id.clone(),
                who,
                ..CardThreadItem::default()
            },
        ));
    }
    all.sort_by(|a, b| a.0.cmp(&b.0));
    let mut agent_later = false;
    for (_, it) in all.iter_mut().rev() {
        if it.kind == THREAD_EVENT {
            continue;
        }
        it.unheard = it.unheard && !agent_later;
        agent_later = agent_later || (it.who.status != "person" && it.who.status != "irori");
    }
    let mut earlier = 0;
    if all.len() > MAX_THREAD_VIEW {
        earlier = all.len() - MAX_THREAD_VIEW;
        all.drain(..earlier);
    }
    (
        all.into_iter().map(|(_, it)| it).collect(),
        by_blocker,
        earlier,
    )
}

/// A person's comment's Pending lines from its notices still open.
fn pending_lines(ds: &[Delivery], t: &Task, p: &People) -> Vec<String> {
    let mut out = Vec::new();
    for d in ds {
        if d.to.is_empty() || (t.watchers.contains(&d.to) && !t.holders.contains(&d.to)) {
            continue;
        }
        let who = p.ref_(&d.to);
        match d.state.as_str() {
            st::QUEUED if who.status == "busy" => {
                out.push(format!("queued for {} (busy)", who.name))
            }
            st::QUEUED => out.push(format!("queued for {}", who.name)),
            st::HELD => out.push(format!("held for {}", who.name)),
            st::DELIVERED => out.push(format!("sent to {}, not read yet", who.name)),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(status: &str, assigned: bool) -> AgentRef {
        AgentRef {
            status: status.into(),
            assigned,
            ..AgentRef::default()
        }
    }

    #[test]
    fn holder_line_prefers_who_took_it() {
        assert_eq!(holder_line(&[]).state, Holding::Nobody);
        let h = holder_line(&[r("busy", true), r("idle", false)]);
        assert_eq!((h.state, h.others), (Holding::Idle, 1));
        assert_eq!(holder_line(&[r("busy", true)]).state, Holding::Assigned);
        assert_eq!(holder_line(&[r("gone", false)]).state, Holding::Gone);
        assert_eq!(holder_line(&[r("person", false)]).state, Holding::Person);
        assert_eq!(holder_line(&[r("busy", false)]).state, Holding::Active);
    }

    #[test]
    fn where_names_devices() {
        let mut p = People {
            self_id: "me".into(),
            ..People::default()
        };
        p.names.insert("mac".into(), "Mac".into());
        assert_eq!(p.where_(""), "This PC");
        assert_eq!(p.where_("me"), "This PC");
        assert_eq!(p.where_("mac"), "Mac");
        assert_eq!(p.where_("x"), "Another device");
        assert_eq!(p.chip("mac"), "");
        p.paired = true;
        assert_eq!(p.chip("mac"), "Mac");
    }
}
