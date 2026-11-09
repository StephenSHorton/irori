//! The list signals' wire contract (list-signals.md §3.3, §4, §8): the
//! envelopes that say which ids a change touched, the fold of a Core batch
//! into one, and the expansion of a card change to every row that shows it.
//!
//! Every envelope carries `seq` (minted by [`Window::emit_change`], one
//! count per event name) and `all`. `all: true` means "read everything", as
//! before; anything this module can't name becomes `all`, so a list can't go
//! stale (D2).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use coord::{Coord, Task, TaskFilter};
use irori_core::CoreEvent;
use serde::Serialize;
use specta::Type;

use crate::Window;
use crate::inbox::InboxCounts;

/// More ids than this in one change is `all`: a catch-up folds thousands of
/// rows, and reading them one by one is slower than one full read (§3.3).
pub const MAX_IDS: usize = 200;

/// The debug-only switch that skips a `seq` now and then: every Nth emit of
/// a name jumps over one, so the page sees a gap (§8).
pub const DEV_SKIP_SEQ_ENV: &str = "IRORI_DEV_SKIP_SEQ";
/// The debug-only switch that gives the Core feeds this many slots, so they
/// overflow (§8).
pub const DEV_FEED_QUEUE_ENV: &str = "IRORI_DEV_FEED_QUEUE";

/// `board:changed`: which cards, notes and desks a change touched.
/// `cards` includes thread ids and is already expanded (§3.3).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardsChange {
    pub seq: u32,
    pub all: bool,
    pub cards: Vec<String>,
    pub notes: Vec<String>,
    pub desk: bool,
}

/// `talk:changed`: conversation ids (bare thread ids for a thread's read
/// cursor) and call ids.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkSignal {
    pub seq: u32,
    pub all: bool,
    pub convs: Vec<String>,
    pub calls: Vec<String>,
}

/// `notifications:changed`: notification keys and ask ids.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NotifSignal {
    pub seq: u32,
    pub all: bool,
    pub keys: Vec<String>,
    pub asks: Vec<String>,
}

/// `inbox:changed`: the counts, as they always were, plus what changed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InboxSignal {
    #[serde(flatten)]
    pub counts: InboxCounts,
    pub seq: u32,
    pub all: bool,
    pub deliveries: Vec<String>,
    pub cards: Vec<String>,
}

/// `devices:changed`: always `all` (rare, and names, colours and faces show
/// on every row).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DevicesSignal {
    pub seq: u32,
    pub all: bool,
}

/// `boards:changed`: its lists are small and always read whole.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BoardsSignal {
    pub seq: u32,
}

/// `assistant:changed`: `all` after a lost event. Later `turns`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantSignal {
    pub seq: u32,
    pub all: bool,
}

/// A debug-only switch's value: a positive number in a debug build, else 0
/// (a release build ignores it, like `IRORI_DEV_PORT_OFFSET`).
pub fn dev_switch_of(v: Option<std::ffi::OsString>) -> usize {
    if !cfg!(debug_assertions) {
        return 0;
    }
    v.and_then(|v| v.to_str().and_then(|s| s.trim().parse::<usize>().ok()))
        .unwrap_or(0)
}

/// [`dev_switch_of`] for the environment variable `name`.
pub fn dev_switch(name: &str) -> usize {
    dev_switch_of(std::env::var_os(name))
}

/// Adds `v` to `list` once, in first-seen order; empty ids are left out.
pub(crate) fn add_id(list: &mut Vec<String>, v: &str) {
    if !v.is_empty() && !list.iter().any(|x| x == v) {
        list.push(v.to_string());
    }
}

// ---- Talk, notifications, inbox ----------------------------------------------

/// Folds a batch of Talk events (list-signals.md §3.2): the union of their
/// conversations and calls, `thread:` stripped from a conversation id. A
/// lost event, a change that names nothing and `all` make it `all`.
pub fn fold_talk(batch: &[CoreEvent]) -> (bool, Vec<String>, Vec<String>) {
    let (mut all, mut convs, mut calls) = (false, Vec::new(), Vec::new());
    for ev in batch {
        match ev {
            CoreEvent::Talk(t) if !t.is_all() => {
                for c in &t.convs {
                    add_id(&mut convs, c.strip_prefix("thread:").unwrap_or(c));
                }
                for c in &t.calls {
                    add_id(&mut calls, c);
                }
            }
            CoreEvent::Talk(_) | CoreEvent::Resync => all = true,
            _ => {}
        }
    }
    if all {
        convs.clear();
        calls.clear();
    }
    (all, convs, calls)
}

/// Folds a batch of Notifications and Asks events: the union of their keys
/// and ask ids. An empty ask id (asks lost or refolded), a lost event, a
/// store resync and a change with no keys make it `all`.
pub fn fold_notifications(batch: &[CoreEvent]) -> (bool, Vec<String>, Vec<String>) {
    let (mut all, mut keys, mut asks) = (false, Vec::new(), Vec::new());
    for ev in batch {
        match ev {
            CoreEvent::Notifications(n) if !n.is_all() => {
                for k in &n.keys {
                    add_id(&mut keys, k);
                }
            }
            CoreEvent::Notifications(_) | CoreEvent::Resync => all = true,
            CoreEvent::Asks(id) if id.is_empty() => all = true,
            CoreEvent::Asks(id) => add_id(&mut asks, id),
            _ => {}
        }
    }
    if all {
        keys.clear();
        asks.clear();
    }
    (all, keys, asks)
}

/// Whether an inbox change touches no delivery the lists can name: a
/// `POLICY_SET`, the mesh re-fold and the receive/autonomy/threads settings
/// send `Updated` with both `to` and `delivery_id` empty (§3.2), and the
/// inbox's own `Resync` says it lost changes.
pub fn inbox_all(ch: &inbox::Change) -> bool {
    ch.kind == inbox::ChangeKind::Resync
        || (ch.kind == inbox::ChangeKind::Updated && ch.to.is_empty() && ch.delivery_id.is_empty())
}

/// Folds a batch of Inbox events: the deliveries they name. `all` for a
/// change that names none (see [`inbox_all`]), a lost event or a kick.
pub fn fold_inbox(batch: &[CoreEvent], kicked: bool) -> (bool, Vec<String>) {
    let (mut all, mut ds) = (kicked, Vec::new());
    for ev in batch {
        match ev {
            CoreEvent::Inbox(ch) if inbox_all(ch) => all = true,
            CoreEvent::Inbox(ch) => add_id(&mut ds, &ch.delivery_id),
            CoreEvent::Resync => all = true,
            _ => {}
        }
    }
    if all {
        ds.clear();
    }
    (all, ds)
}

// ---- the kind map ------------------------------------------------------------

/// Where a Board change goes (§4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    /// `cards` ← the change's `task_id`.
    Card,
    /// `notes` ← the change's `resource` (the note id).
    Note,
    /// `boards:changed` only, not `cards` (its `task_id` is `board:<id>`).
    Pin,
    /// `desk: true`.
    Desk,
    /// The holder's cards, through the rows that show it (§3.3 step 3).
    Lease,
    /// Read everything.
    All,
}

/// Board change kind → bucket. A kind that isn't here is [`Bucket::All`].
/// The lease kinds name their holder, whose rows the summary cache knows
/// (card #1147).
pub const KINDS: &[(&str, Bucket)] = &[
    (coord::TASK_CHANGED, Bucket::Card),
    (coord::COMMENT_ADDED, Bucket::Card),
    (coord::NOTE_CHANGED, Bucket::Note),
    (coord::BOARD_PIN_CHANGED, Bucket::Pin),
    (coord::DESK_CHANGED, Bucket::Desk),
    (coord::LEASE_CHANGED, Bucket::Lease),
    (coord::LEASE_EXPIRED, Bucket::Lease),
    (coord::LEASE_REFUSED, Bucket::Lease),
    (coord::RESYNC, Bucket::All),
    (irori_core::spawn_capable::AGENTS_CHANGED, Bucket::All),
];

/// The bucket of a change kind; an empty or unmapped kind is `All`.
pub fn bucket_of(kind: &str) -> Bucket {
    KINDS
        .iter()
        .find(|(k, _)| *k == kind)
        .map_or(Bucket::All, |(_, b)| *b)
}

/// One board pump batch folded: what its events name, before expansion.
#[derive(Debug, Default)]
pub struct Folded {
    pub all: bool,
    /// Events were lost: the expansion's indexes can't be trusted.
    pub lost: bool,
    /// A change the grid's summary cache can't follow: it starts again
    /// (card #1147). Not `AGENTS_CHANGED`, which moves only what every
    /// read builds anew.
    pub cold: bool,
    /// Lease holders the batch names: their rows show a conflict.
    pub holders: Vec<String>,
    /// The batch held a lease change.
    pub lease: bool,
    pub cards: Vec<String>,
    pub notes: Vec<String>,
    pub desk: bool,
    /// A shared board's sidebar pin moved: `boards:changed`.
    pub pin: bool,
    /// Agents whose presence changed.
    pub agents: Vec<String>,
    /// Another device's roster changed (no key says which agent).
    pub roster: bool,
    /// Deliveries the batch's inbox changes name.
    pub deliveries: Vec<String>,
    /// A presence change that may change the tray's count (an agent went
    /// away: a wait on it is the person's, card #520).
    pub gone: bool,
    /// The batch held anything but presence and roster changes.
    pub board_or_inbox: bool,
}

/// Folds a board pump batch with the kind table.
pub fn fold_board(batch: &[CoreEvent]) -> Folded {
    let mut f = Folded::default();
    for ev in batch {
        match ev {
            CoreEvent::Board(ch) => {
                f.board_or_inbox = true;
                match bucket_of(&ch.kind) {
                    Bucket::Card if ch.task_id.is_empty() => {
                        f.all = true;
                        f.cold = true;
                    }
                    Bucket::Card => add_id(&mut f.cards, &ch.task_id),
                    Bucket::Note if ch.resource.is_empty() => f.all = true,
                    Bucket::Note => add_id(&mut f.notes, &ch.resource),
                    Bucket::Pin => f.pin = true,
                    Bucket::Desk => f.desk = true,
                    Bucket::Lease => {
                        f.lease = true;
                        add_id(&mut f.holders, &ch.holder);
                    }
                    Bucket::All => {
                        f.all = true;
                        f.lost |= ch.kind == coord::RESYNC;
                        f.cold |= ch.kind != irori_core::spawn_capable::AGENTS_CHANGED;
                    }
                }
            }
            CoreEvent::Inbox(ch) => {
                f.board_or_inbox = true;
                if inbox_all(ch) {
                    f.all = true;
                    f.lost |= ch.kind == inbox::ChangeKind::Resync;
                } else {
                    add_id(&mut f.deliveries, &ch.delivery_id);
                }
            }
            CoreEvent::Presence(ch) => {
                add_id(&mut f.agents, &ch.card.key);
                f.gone |= ch.kind == presence::ChangeKind::Removed || ch.card.status == "gone";
            }
            CoreEvent::Roster => f.roster = true,
            CoreEvent::Resync => {
                f.board_or_inbox = true;
                f.all = true;
                f.lost = true;
            }
            _ => {}
        }
    }
    f.cold |= f.lost;
    f
}

// ---- the expansion -----------------------------------------------------------

/// What the board pump remembers between batches, to turn the cards a
/// batch names into every row that shows them (§3.3).
#[derive(Default)]
pub struct Expander {
    built: bool,
    /// Each card's neighbours: its parent, subtasks, thread, after cards
    /// and the cards its blockers wait on (one hop), and the reverse.
    fwd: HashMap<String, Vec<String>>,
    rev: HashMap<String, HashSet<String>>,
    /// Thread → the cards it made or changed (`Task.made`), and the reverse
    /// index the window lacks: card → its threads.
    made_of: HashMap<String, Vec<String>>,
    made_by: HashMap<String, HashSet<String>>,
    /// Agent key → the cards whose rows show it (holders, step owners,
    /// who left).
    held_by: HashMap<String, HashSet<String>>,
    held_of: HashMap<String, Vec<String>>,
    /// The spawn queue's words as of the last batch.
    places: HashMap<String, usize>,
    no_agent: HashMap<String, String>,
    held_until: Option<i64>,
    /// Card → when its row changes by the clock (ms).
    due: HashMap<String, i64>,
    /// Each lease holder in a conflict, with its resource, as of the last
    /// lease change (a conflict names only its winner).
    conflicts: Option<HashMap<String, String>>,
}

impl Expander {
    /// The earliest time (ms) a row changes by the clock, if any.
    pub fn next_due(&self) -> Option<i64> {
        self.due.values().copied().min()
    }

    fn clear_card(&mut self, id: &str) {
        if let Some(old) = self.fwd.remove(id) {
            for o in old {
                if let Some(s) = self.rev.get_mut(&o) {
                    s.remove(id);
                }
            }
        }
        if let Some(old) = self.made_of.remove(id) {
            for c in old {
                if let Some(s) = self.made_by.get_mut(&c) {
                    s.remove(id);
                }
            }
        }
        if let Some(old) = self.held_of.remove(id) {
            for k in old {
                if let Some(s) = self.held_by.get_mut(&k) {
                    s.remove(id);
                }
            }
        }
        self.due.remove(id);
    }

    /// Takes card `id`'s edges, thread cards, agents and clock times from
    /// `t` (`None`: it is gone). Returns the neighbours it had and has now.
    fn refresh(&mut self, id: &str, t: Option<&Task>, co: Option<&Coord>, now: i64) -> Vec<String> {
        let mut touched: Vec<String> = self.fwd.get(id).cloned().unwrap_or_default();
        self.clear_card(id);
        let Some(t) = t else { return touched };
        let mut edges: Vec<String> = Vec::new();
        add_id(&mut edges, &t.parent);
        add_id(&mut edges, &t.from);
        for s in &t.subtasks {
            add_id(&mut edges, s);
        }
        for c in t.after_cards() {
            add_id(&mut edges, &c);
        }
        for b in &t.blockers {
            if let Some(on) = &b.on
                && on.kind == coord::BLOCKED_ON_TASK
            {
                add_id(&mut edges, &on.reference);
            }
        }
        edges.retain(|e| e != id);
        for e in &edges {
            self.rev
                .entry(e.clone())
                .or_default()
                .insert(id.to_string());
            add_id(&mut touched, e);
        }
        self.fwd.insert(id.to_string(), edges);
        if crate::threads::is_thread(t) && !t.made.is_empty() {
            for c in &t.made {
                self.made_by
                    .entry(c.clone())
                    .or_default()
                    .insert(id.to_string());
            }
            self.made_of.insert(id.to_string(), t.made.clone());
        }
        let mut keys: Vec<String> = Vec::new();
        for h in t.holders.iter().chain(&t.left) {
            add_id(&mut keys, h);
        }
        for s in &t.steps {
            add_id(&mut keys, &s.owner);
        }
        for k in &keys {
            self.held_by
                .entry(k.clone())
                .or_default()
                .insert(id.to_string());
        }
        if !keys.is_empty() {
            self.held_of.insert(id.to_string(), keys);
        }
        if let Some(due) = due_of(t, co, now) {
            self.due.insert(id.to_string(), due);
        }
        touched
    }
}

/// When, after `now`, card `t`'s row changes with no event (§3.3 step 5):
/// its talking line ends, it turns stale, or a starting request passes its
/// grace. `None` when nothing is ahead.
fn due_of(t: &Task, co: Option<&Coord>, now: i64) -> Option<i64> {
    let mut at: Vec<i64> = Vec::new();
    if let Some(sp) = t.spawn_request().filter(|s| s.pending()) {
        at.push(sp.at.unix_milli_value() + coord::SPAWN_GRACE.as_millis() as i64);
    }
    if let Some(co) = co {
        let mut last = t.updated_at.unix_milli_value();
        if let Some(acts) = co.activity(&t.id)
            && let Some(a) = acts.first()
        {
            last = last.max(a.at.unix_milli_value());
        }
        if t.state == coord::WORKING && !t.updated_at.is_zero() {
            at.push(last + crate::cards::STALE_AFTER_MS);
        }
        if let Some(c) = co.comments(&t.id).and_then(|cs| cs.last().cloned()) {
            at.push(c.at.unix_milli_value() + crate::cards::TALK_WINDOW_MS);
        }
    }
    at.into_iter().filter(|a| *a > now).min()
}

impl Window {
    /// Every log this device reads: the private one, then each ready shared
    /// board's.
    pub fn coords(&self) -> Vec<Arc<Coord>> {
        let mut out = vec![self.core.coord().clone()];
        for s in self.core.board_list() {
            if s.ready
                && let Ok(bc) = self.core.board_coord(&s.board_id)
            {
                out.push(bc);
            }
        }
        out
    }

    /// The cards a delivery's message is about (inbox:changed.cards, §4):
    /// a `task:` ref is the card; a `comment:` ref is its comment's card; a
    /// message recorded on a card is the comment with its message id.
    pub fn delivery_cards(&self, coords: &[Arc<Coord>], did: &str) -> Vec<String> {
        let mut out = Vec::new();
        let Some(d) = self.core.inbox().get(did) else {
            return out;
        };
        let m = &d.message;
        if let Some(id) = m.ref_.strip_prefix("task:") {
            add_id(&mut out, id);
        }
        let comment_task = |cid: &str| coords.iter().find_map(|co| co.get_comment(cid));
        if let Some(cid) = m.ref_.strip_prefix("comment:")
            && let Some(c) = comment_task(cid)
        {
            add_id(&mut out, &c.task_id);
        }
        if let Some(c) = comment_task(&m.id) {
            add_id(&mut out, &c.task_id);
        }
        out
    }

    /// The deliveries' cards, the union (an unreadable delivery names
    /// none).
    pub fn deliveries_cards(&self, ds: &[String]) -> Vec<String> {
        if ds.is_empty() {
            return Vec::new();
        }
        let coords = self.coords();
        let mut out = Vec::new();
        for d in ds {
            for c in self.delivery_cards(&coords, d) {
                add_id(&mut out, &c);
            }
        }
        out
    }

    fn rebuild(&self, ex: &mut Expander, coords: &[Arc<Coord>], now: i64) {
        *ex = Expander::default();
        for co in coords {
            for t in co.list_tasks(&TaskFilter::default()) {
                ex.refresh(&t.id.clone(), Some(&t), Some(co), now);
            }
        }
        ex.built = true;
    }

    /// Turns a folded board batch into the `board:changed` payload's ids
    /// (§3.3): the batch's cards and the cards of its inbox changes, then
    /// everything whose row shows one: the threads that made it, one hop of
    /// family, the cards of the agents that moved, the cards whose queue
    /// words moved. `None` when the batch dirties no row. More than
    /// [`MAX_IDS`] is `all`.
    pub fn expand(&self, ex: &mut Expander, f: &mut Folded) -> Option<CardsChange> {
        let now = self.now_ms();
        let coords = self.coords();
        if f.lost {
            ex.built = false;
        }
        if f.cold {
            self.cache_clear();
        }
        let queue = self.core.spawn_queue();
        let no_agent = self.core.no_agent();
        let held_until = queue.held_until.map(|h| h.unix_milli_value());
        let mut ids: Vec<String> = Vec::new();
        // The indexes follow every batch they can trust, `all` or not: a
        // card change missed here would leave its edges stale.
        if !f.lost {
            if !ex.built {
                self.rebuild(ex, &coords, now);
            }
            // T: the batch's cards and its inbox changes' cards.
            let mut t_ids: Vec<String> = f.cards.clone();
            for c in self.deliveries_cards(&f.deliveries) {
                add_id(&mut t_ids, &c);
            }
            // The cards whose rows show an agent that moved, or a lease
            // holder whose conflict did.
            let mut keys: Vec<String> = f.agents.clone();
            for h in &f.holders {
                add_id(&mut keys, h);
            }
            if f.lease {
                for k in self.conflicts_moved(ex) {
                    add_id(&mut keys, &k);
                }
            }
            for k in &keys {
                if let Some(cs) = ex.held_by.get(k) {
                    for c in cs {
                        add_id(&mut ids, c);
                    }
                }
                for c in self.cache_rows_of(k) {
                    add_id(&mut ids, &c);
                }
            }
            if f.roster {
                let me = format!("{}/", self.core.device_id());
                let others: Vec<&String> = ex
                    .held_by
                    .iter()
                    .filter(|(k, _)| !k.starts_with(&me))
                    .flat_map(|(_, cs)| cs)
                    .collect();
                for c in others {
                    add_id(&mut ids, c);
                }
                for c in self.cache_rows_of_others() {
                    add_id(&mut ids, &c);
                }
            }
            let mut changed: Vec<(String, Vec<String>)> = Vec::new();
            for c in &t_ids {
                add_id(&mut ids, c);
                let co = coords.iter().find(|co| co.get_task(c).is_some());
                let t = co.and_then(|co| co.get_task(c));
                let touched = ex.refresh(c, t.as_ref(), co.map(|co| co.as_ref()), now);
                for n in &touched {
                    add_id(&mut ids, n);
                }
                for n in ex.rev.get(c).into_iter().flatten() {
                    add_id(&mut ids, n);
                }
                for n in ex.made_by.get(c).into_iter().flatten() {
                    add_id(&mut ids, n);
                }
                changed.push((c.clone(), touched));
            }
            // The rows that read a changed card, or a card it pointed at
            // before or points at now, and the loops of waits that moved
            // (the summary cache's index, list-signals.md §6.1).
            for c in self.cache_batch(&changed) {
                add_id(&mut ids, &c);
            }
        }
        // Queue words: one card starting moves every queued card's place.
        let mut moved: Vec<String> = Vec::new();
        for (k, v) in &queue.places {
            if ex.places.get(k) != Some(v) {
                add_id(&mut moved, k);
            }
        }
        for k in ex.places.keys() {
            if !queue.places.contains_key(k) {
                add_id(&mut moved, k);
            }
        }
        for (k, v) in &no_agent {
            if ex.no_agent.get(k) != Some(v) {
                add_id(&mut moved, k);
            }
        }
        for k in ex.no_agent.keys() {
            if !no_agent.contains_key(k) {
                add_id(&mut moved, k);
            }
        }
        if ex.held_until != held_until {
            for k in queue.places.keys().chain(ex.places.keys()) {
                add_id(&mut moved, k);
            }
        }
        ex.places = queue.places.clone();
        ex.no_agent = no_agent;
        ex.held_until = held_until;
        for m in moved {
            add_id(&mut ids, &m);
        }
        let all = f.all || ids.len() > MAX_IDS;
        if !all && ids.is_empty() && f.notes.is_empty() && !f.desk {
            return None;
        }
        Some(CardsChange {
            seq: 0,
            all,
            cards: if all { Vec::new() } else { ids },
            notes: if all {
                Vec::new()
            } else {
                std::mem::take(&mut f.notes)
            },
            desk: f.desk,
        })
    }

    /// The holders whose lease conflict began or ended since the last
    /// lease change: a conflict's note names only its winner, and its end
    /// clears the other side's unnamed. The first call names nobody.
    fn conflicts_moved(&self, ex: &mut Expander) -> Vec<String> {
        let now: HashMap<String, String> = self
            .core
            .coord()
            .leases()
            .into_iter()
            .filter(|l| !l.conflict.is_empty())
            .map(|l| (l.holder, l.resource))
            .collect();
        let mut out: Vec<String> = Vec::new();
        if let Some(was) = &ex.conflicts {
            for (k, v) in &now {
                if was.get(k) != Some(v) {
                    add_id(&mut out, k);
                }
            }
            for k in was.keys() {
                if !now.contains_key(k) {
                    add_id(&mut out, k);
                }
            }
        }
        ex.conflicts = Some(now);
        out
    }

    /// The cards whose rows change by the clock now (§3.3 step 5), taken
    /// off the schedule and set again for what is still ahead. The usage
    /// hold's end moves every queued card's words.
    pub fn due_cards(&self, ex: &mut Expander) -> Vec<String> {
        let now = self.now_ms();
        if !ex.built {
            self.rebuild(ex, &self.coords(), now);
        }
        let mut out: Vec<String> = ex
            .due
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(id, _)| id.clone())
            .collect();
        out.sort();
        if ex.held_until.is_some_and(|h| h <= now) {
            let q = self.core.spawn_queue();
            ex.held_until = q.held_until.map(|h| h.unix_milli_value());
            for k in ex.places.keys() {
                add_id(&mut out, k);
            }
        }
        if out.is_empty() {
            return out;
        }
        let coords = self.coords();
        for id in &out {
            let co = coords.iter().find(|co| co.get_task(id).is_some());
            let t = co.and_then(|co| co.get_task(id));
            ex.refresh(id, t.as_ref(), co.map(|co| co.as_ref()), now);
            // Passed: don't fire again.
            if ex.due.get(id).is_some_and(|at| *at <= now) {
                ex.due.remove(id);
            }
        }
        out
    }
}
