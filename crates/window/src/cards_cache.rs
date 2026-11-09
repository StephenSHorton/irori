//! The card grid's summary cache (card #1147, list-signals.md §6.1).
//!
//! A grid row is built from three kinds of input:
//! - the card itself, kept here as each log's open cards ([`Entry`]);
//! - what the row reads from other cards and the log: its family fields
//!   (order, waits, parent, the thread it came from), its newest activity,
//!   its recent comments and its place on the canvas ([`Aux`]), each with
//!   the card ids it read (`deps`);
//! - People, the roster, the clock and the board-key mapping, read again
//!   on every read and never kept. A gone wait, a part's lead and a shared
//!   card's pin are read then too, because they move with no event.
//!
//! The board pump keeps it ([`Window::cache_batch`]): a card a batch names
//! is read again, and every row that read it (or a card it pointed at
//! before or points at now) drops its Aux, which the next read builds
//! again. A loop of waits that changed drops its rows too. A resync clears
//! everything. The cache is used only while a pump keeps it
//! ([`Window::cache_live`]); otherwise every read is cold, as before.
//!
//! The drift read ([`Window::cards_cold`]) builds a fresh cache, logs how
//! many rows differ from the kept one (`drift cards-cache <n>`) and keeps
//! the fresh one.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use coord::{Comment, Coord, Task, TaskFilter};
use gocompat::time::GoTime;
use serde::Serialize;
use specta::Type;

use crate::Window;
use crate::cards::{
    CanvasPos, CardSummary, FamilyAux, People, TALK_WINDOW_MS, card_place, card_summary,
    family_apply, family_aux, newer, ours, pos_of, stale, talking, weigh,
};
use crate::signals::add_id;

/// A patch read's answer (`board_cards_patch`): the rows asked for that
/// are still live, freshly built, and every live card's id in grid order.
/// A card that closed is missing from `order`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardsPatch {
    pub rows: Vec<CardSummary>,
    pub order: Vec<String>,
}

/// One open card of a log the window reads.
#[derive(Clone)]
struct Entry {
    /// "" for the private log, else the board's id.
    log: String,
    /// As its log holds it (a shared card's keys are board keys).
    task: Task,
    /// Its row needs the person (open blockers > 0): the grid's sort and
    /// the tray's count. People-free.
    blocked: bool,
}

impl Entry {
    /// Whether the grid lists it: not a thread, nor a withdrawn shared card.
    fn grid(&self) -> bool {
        !crate::threads::is_thread(&self.task) && (self.log.is_empty() || !self.task.withdrawn)
    }

    /// Whether the tray counts it: every private card, thread or not, and
    /// a shared board's grid cards (see [`Window::decide_count`]).
    fn counted(&self) -> bool {
        self.log.is_empty() || self.grid()
    }
}

/// What a row reads beyond its own card and People.
struct Aux {
    fam: FamilyAux,
    /// The newest activity line's time.
    act_at: Option<GoTime>,
    /// The comments still inside the talk window when it was built, in log
    /// order: a later read only drops more of them.
    talk: Vec<Comment>,
    /// A shared card's comment authors (board keys), for `ours`.
    commenters: Vec<String>,
    pos: Option<CanvasPos>,
    /// Every card id it read, its own among them.
    deps: Vec<String>,
}

/// A log the window reads.
struct Log {
    id: String,
    co: Arc<Coord>,
    board: Option<irori_core::BoardSummary>,
}

/// The grid's summary cache.
#[derive(Default)]
pub struct CardsCache {
    /// A pump keeps it: reads use it.
    live: bool,
    built: bool,
    logs: Vec<Log>,
    entries: HashMap<String, Entry>,
    aux: HashMap<String, Aux>,
    /// Card id → the rows whose Aux read it.
    readers: HashMap<String, HashSet<String>>,
    /// Agent key → the rows that show it.
    shown: HashMap<String, HashSet<String>>,
    keys_of: HashMap<String, Vec<String>>,
    /// Each open card's loop of waits as of the last batch.
    rings: HashMap<String, Vec<String>>,
}

impl CardsCache {
    fn clear(&mut self) {
        *self = CardsCache {
            live: self.live,
            ..CardsCache::default()
        };
    }

    fn drop_aux(&mut self, id: &str) {
        if let Some(a) = self.aux.remove(id) {
            for d in a.deps {
                if let Some(r) = self.readers.get_mut(&d) {
                    r.remove(id);
                    if r.is_empty() {
                        self.readers.remove(&d);
                    }
                }
            }
        }
    }

    fn put_aux(&mut self, id: &str, a: Aux) {
        self.drop_aux(id);
        for d in &a.deps {
            self.readers
                .entry(d.clone())
                .or_default()
                .insert(id.to_string());
        }
        self.aux.insert(id.to_string(), a);
    }

    fn set_keys(&mut self, id: &str, keys: Vec<String>) {
        if let Some(old) = self.keys_of.remove(id) {
            for k in old {
                if let Some(r) = self.shown.get_mut(&k) {
                    r.remove(id);
                    if r.is_empty() {
                        self.shown.remove(&k);
                    }
                }
            }
        }
        if keys.is_empty() {
            return;
        }
        for k in &keys {
            self.shown
                .entry(k.clone())
                .or_default()
                .insert(id.to_string());
        }
        self.keys_of.insert(id.to_string(), keys);
    }

    fn log(&self, id: &str) -> Option<&Log> {
        self.logs.iter().find(|l| l.id == id)
    }

    /// The grid's order over the cards kept: pinned first, then families
    /// together, as [`Window::cards_in`] sorts them. `pins` are the
    /// person's pins on shared cards (their own log's).
    fn order(&self, pins: &HashMap<String, f64>) -> Vec<String> {
        let keys: Vec<SortKey> = self
            .entries
            .iter()
            .filter(|(_, e)| e.grid())
            .map(|(id, e)| {
                let t = &e.task;
                let (pinned, rank) = match pins.get(id).filter(|_| !e.log.is_empty()) {
                    Some(r) => (true, *r),
                    None => (t.pinned, t.pin_rank),
                };
                SortKey {
                    id,
                    ask: t.ask.as_ref().is_some_and(|a| a.outcome.is_none()),
                    blocked: e.blocked,
                    at: &t.updated_at,
                    pinned,
                    rank,
                    parent: &t.parent,
                    kids: &t.subtasks,
                }
            })
            .collect();
        grid_order(keys)
    }
}

/// What the grid sorts a row by.
struct SortKey<'a> {
    id: &'a str,
    /// An open ask.
    ask: bool,
    blocked: bool,
    at: &'a GoTime,
    pinned: bool,
    rank: f64,
    parent: &'a str,
    kids: &'a [String],
}

/// [`Window::cards_in`]'s order over sort keys: open asks, then cards
/// waiting on the person, then newest; pinned cards first by rank, each
/// pinned parent with its unpinned subtasks; the rest by family, each
/// family where its first card sorted.
fn grid_order(mut keys: Vec<SortKey>) -> Vec<String> {
    keys.sort_by(|a, b| {
        if a.ask != b.ask {
            return b.ask.cmp(&a.ask);
        }
        if a.blocked != b.blocked {
            return b.blocked.cmp(&a.blocked);
        }
        newer(a.at, a.id, b.at, b.id)
    });
    let by_id: HashMap<&str, &SortKey> = keys.iter().map(|k| (k.id, k)).collect();
    let mut pins: Vec<&SortKey> = keys.iter().filter(|k| k.pinned).collect();
    pins.sort_by(|a, b| {
        a.rank
            .partial_cmp(&b.rank)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut out: Vec<String> = Vec::with_capacity(keys.len());
    let mut used: HashSet<&str> = HashSet::new();
    for k in pins {
        if !used.insert(k.id) {
            continue;
        }
        out.push(k.id.to_string());
        for kid in k.kids {
            if let Some(ks) = by_id.get(kid.as_str())
                && !ks.pinned
            {
                out.push(kid.clone());
                used.insert(ks.id);
            }
        }
    }
    let rest: Vec<&SortKey> = keys.iter().filter(|k| !used.contains(k.id)).collect();
    let at: HashMap<&str, usize> = rest.iter().enumerate().map(|(i, k)| (k.id, i)).collect();
    let head = |k: &SortKey<'_>| -> usize {
        if !k.parent.is_empty()
            && let Some(&i) = at.get(k.parent)
        {
            return i;
        }
        at[k.id]
    };
    let mut placed: HashSet<usize> = HashSet::new();
    for k in &rest {
        let h = head(k);
        if !placed.insert(h) {
            continue;
        }
        out.push(rest[h].id.to_string());
        for kid in rest[h].kids {
            if let Some(&i) = at.get(kid.as_str()) {
                out.push(rest[i].id.to_string());
            }
        }
    }
    out
}

/// Card t's agents whose going away can give it a gone wait (coord's
/// `stranded`): who it was given to, and who its open waits are on or
/// from. Resolved in no People, so noted by hand.
fn gone_keys(t: &Task, keys: &mut Vec<String>) {
    for k in &t.assigned {
        add_id(keys, k);
    }
    for b in t.blockers.iter().filter(|b| b.open()) {
        if let Some(on) = &b.on
            && on.kind == coord::BLOCKED_ON_AGENT
        {
            add_id(keys, &on.reference);
        }
        add_id(keys, &b.sent_to);
        add_id(keys, &b.raised_by);
    }
}

/// What a read builds once per log: the followers and loops a row's
/// family reads, and the canvas positions.
struct LogRead {
    follows: crate::later::Followers,
    loops: coord::OrderLoops,
    placed: HashMap<String, (i64, i64)>,
}

/// One read's People, clock and pins, and its per-log reads.
struct Read<'a> {
    p: &'a People,
    now: i64,
    /// The person's pins on shared cards.
    pins: HashMap<String, f64>,
    logs: HashMap<String, LogRead>,
}

impl Window {
    /// The summary cache is kept by a running board pump from now on
    /// (`true`), or no longer (`false`): every read is cold then.
    pub(crate) fn cache_live(&self, live: bool) {
        let mut c = self.cache();
        c.clear();
        c.live = live;
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, CardsCache> {
        self.cards_cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Drops everything kept: a change the cache can't name (a device's
    /// sides change what its cards wait on, with no card change).
    pub(crate) fn cache_clear(&self) {
        self.cache().clear();
    }

    /// Whether `t` (raw, of log `log`) needs the person, as the tray counts
    /// it: its People-free summary's open blockers.
    fn blocked(&self, log: &str, t: &Task) -> bool {
        let p = People::default();
        if log.is_empty() {
            card_summary(t, &p).open_blockers > 0
        } else {
            card_summary(&self.priv_task(t.clone()), &p).open_blockers > 0
        }
    }

    fn build(&self, c: &mut CardsCache) {
        if c.built {
            return;
        }
        let live = c.live;
        *c = CardsCache {
            live,
            built: true,
            ..CardsCache::default()
        };
        c.logs.push(Log {
            id: String::new(),
            co: self.core.coord().clone(),
            board: None,
        });
        for s in self.core.board_list() {
            let Ok(co) = self.core.board_coord(&s.board_id) else {
                continue;
            };
            if !s.ready {
                continue;
            }
            c.logs.push(Log {
                id: s.board_id.clone(),
                co,
                board: Some(s),
            });
        }
        let open = TaskFilter {
            open: true,
            ..Default::default()
        };
        for i in 0..c.logs.len() {
            let (id, co) = (c.logs[i].id.clone(), c.logs[i].co.clone());
            let loops = co.order_loops();
            for t in co.list_tasks(&open) {
                if let Some(ring) = loops.get(&t.id) {
                    c.rings.insert(t.id.clone(), ring.to_vec());
                }
                let blocked = self.blocked(&id, &t);
                c.entries.insert(
                    t.id.clone(),
                    Entry {
                        log: id.clone(),
                        task: t,
                        blocked,
                    },
                );
            }
        }
    }

    /// Reads card `id` again into the cache (from whichever log holds it),
    /// or drops it when it closed or went.
    fn refresh_entry(&self, c: &mut CardsCache, id: &str) {
        let found = c
            .logs
            .iter()
            .find_map(|l| l.co.get_task(id).map(|t| (l.id.clone(), t)));
        match found {
            Some((log, t)) if !coord::is_terminal(coord::effective_state(&t)) => {
                let blocked = self.blocked(&log, &t);
                c.entries
                    .insert(id.to_string(), Entry { log, task: t, blocked });
            }
            _ => {
                c.entries.remove(id);
                c.set_keys(id, Vec::new());
            }
        }
        c.drop_aux(id);
    }

    /// The board pump's batch on the cache (list-signals.md §6.1): `cards`
    /// changed, each with the cards it pointed at before or points at now
    /// (`refs`). Returns every row to read again: the cards, the rows that
    /// read one of them or one of their refs, and the rows whose loop of
    /// waits changed. Nothing while the cache isn't built: its first read
    /// builds it from the logs.
    pub(crate) fn cache_batch(&self, cards: &[(String, Vec<String>)]) -> Vec<String> {
        let mut c = self.cache();
        let mut out: Vec<String> = Vec::new();
        if !c.live || !c.built || cards.is_empty() {
            return out;
        }
        let mut dirty: Vec<String> = Vec::new();
        for (id, refs) in cards {
            add_id(&mut dirty, id);
            for r in std::iter::once(id).chain(refs) {
                if let Some(rs) = c.readers.get(r) {
                    for row in rs {
                        add_id(&mut dirty, row);
                    }
                }
            }
        }
        for (id, _) in cards {
            self.refresh_entry(&mut c, id);
        }
        // Loops of waits: one new wait closes a loop for every card on it.
        let logs: Vec<(String, Arc<Coord>)> =
            c.logs.iter().map(|l| (l.id.clone(), l.co.clone())).collect();
        let mut rings: HashMap<String, Vec<String>> = HashMap::new();
        for (log, co) in logs {
            let loops = co.order_loops();
            for (id, e) in &c.entries {
                if e.log == log
                    && let Some(ring) = loops.get(id)
                {
                    rings.insert(id.clone(), ring.to_vec());
                }
            }
        }
        for (id, ring) in &rings {
            if c.rings.get(id) != Some(ring) {
                add_id(&mut dirty, id);
            }
        }
        for id in c.rings.keys() {
            if !rings.contains_key(id) {
                add_id(&mut dirty, id);
            }
        }
        c.rings = rings;
        for id in &dirty {
            c.drop_aux(id);
            add_id(&mut out, id);
        }
        out
    }

    /// The rows that show agent `key`.
    pub(crate) fn cache_rows_of(&self, key: &str) -> Vec<String> {
        let c = self.cache();
        let mut out: Vec<String> = c
            .shown
            .get(key)
            .map(|r| r.iter().cloned().collect())
            .unwrap_or_default();
        out.sort();
        out
    }

    /// The rows that show another device's agent: a roster change names
    /// no key.
    pub(crate) fn cache_rows_of_others(&self) -> Vec<String> {
        let me = format!("{}/", self.core.device_id());
        let c = self.cache();
        let mut out: Vec<String> = Vec::new();
        for (k, rows) in &c.shown {
            if k.contains('/') && !k.starts_with(&me) && !k.starts_with("board:") {
                for r in rows {
                    add_id(&mut out, r);
                }
            }
        }
        out.sort();
        out
    }

    /// Builds row `id`'s Aux from its log.
    fn aux_of(&self, l: &Log, lr: &LogRead, t: &Task, s: &CardSummary, now: i64) -> Aux {
        let co = &l.co;
        let seen: RefCell<Vec<String>> = RefCell::new(vec![t.id.clone()]);
        let get = |id: &str| {
            add_id(&mut seen.borrow_mut(), id);
            co.get_task(id)
        };
        let fam = family_aux(co, &get, (&lr.loops, &lr.follows), t, s);
        let comments = co.comments(&t.id).unwrap_or_default();
        let mut from = comments.len();
        while from > 0 && now - comments[from - 1].at.unix_milli_value() <= TALK_WINDOW_MS {
            from -= 1;
        }
        Aux {
            fam,
            act_at: co
                .activity(&t.id)
                .and_then(|a| a.first().map(|a| a.at)),
            talk: comments[from..].to_vec(),
            commenters: if l.board.is_some() && t.comments > 0 {
                comments.iter().map(|c| c.by.clone()).collect()
            } else {
                Vec::new()
            },
            pos: pos_of(&lr.placed, &t.id),
            deps: seen.into_inner(),
        }
    }

    /// Row `id`, built from the cache: its Aux again when it has none.
    /// None when it isn't a live grid card.
    fn cached_row(&self, c: &mut CardsCache, r: &mut Read, id: &str) -> Option<CardSummary> {
        let e = c.entries.get(id).filter(|e| e.grid())?.clone();
        let l = c.log(&e.log)?;
        let co = l.co.clone();
        let board = l.board.clone();
        let p = r.p;
        let t = &e.task;
        p.watch_keys();
        let pt = if board.is_some() {
            self.priv_task(t.clone())
        } else {
            t.clone()
        };
        let mut s = card_summary(&pt, p);
        if !c.aux.contains_key(id) {
            let lr = r.logs.entry(e.log.clone()).or_insert_with(|| {
                let open: Vec<&Task> = c
                    .entries
                    .values()
                    .filter(|x| x.log == e.log)
                    .map(|x| &x.task)
                    .collect();
                LogRead {
                    follows: crate::later::Followers::of(open),
                    loops: co.order_loops(),
                    placed: co.positions(),
                }
            });
            let l = c.log(&e.log)?;
            let a = self.aux_of(l, lr, t, &s, r.now);
            c.put_aux(id, a);
        }
        let a = c.aux.get(id)?;
        let mut last = t.updated_at;
        if let Some(at) = a.act_at
            && at.unix_nano() > last.unix_nano()
        {
            last = at;
        }
        match &board {
            None => {
                s.pos = a.pos;
                s.stale = s.subtasks == 0 && stale(&s, &last, r.now);
                s.talking = talking(&a.talk, p, r.now);
            }
            Some(b) => {
                let private = |k: &str| self.core.private_key(k);
                s.given_no =
                    co.mine_to_serve(t) && t.given_said.as_ref().is_some_and(|y| !y.yes);
                s.ours = ours(t, [], private)
                    || (t.comments > 0
                        && ours(t, a.commenters.iter().map(String::as_str), private));
                s.pos = a.pos;
                s.scope = crate::boards::SCOPE_BOARD.into();
                s.scope_label = b.name.clone();
                s.board = b.board_id.clone();
                (s.tag, s.space_board) = card_place(t, b);
                self.board_owner(&b.board_id, t, &mut s);
                weigh(&mut s, &pt, p);
                s.stale = s.subtasks == 0 && stale(&s, &last, r.now);
                s.talking = talking(&self.priv_comments(a.talk.clone()), p, r.now);
                if let Some(rank) = r.pins.get(&pt.id) {
                    s.pinned = true;
                    s.pin_rank = *rank;
                }
            }
        }
        family_apply(&co, p, t, &a.fam, &mut s);
        let mut keys = p.seen_keys();
        gone_keys(t, &mut keys);
        if board.is_some() {
            gone_keys(&pt, &mut keys);
        }
        c.set_keys(id, keys);
        Some(s)
    }

    fn read_of<'a>(&self, p: &'a People) -> Read<'a> {
        Read {
            p,
            now: self.now_ms(),
            pins: self.core.coord().pinned_ranks(),
            logs: HashMap::new(),
        }
    }

    /// Every grid row from cache `c`, in grid order.
    fn grid_from(&self, c: &mut CardsCache, p: &People) -> Vec<CardSummary> {
        self.build(c);
        let mut r = self.read_of(p);
        let order = c.order(&r.pins);
        let mut out = Vec::with_capacity(order.len());
        for id in &order {
            if let Some(s) = self.cached_row(c, &mut r, id) {
                out.push(s);
            }
        }
        out
    }

    /// BoardService.Cards: every live card for the grid, from the summary
    /// cache while a pump keeps it (card #1147), else read cold.
    pub fn cards(&self) -> Vec<CardSummary> {
        let mut c = self.cache();
        if !c.live {
            drop(c);
            return self.cards_in(crate::cards::SCOPE_ALL);
        }
        let p = self.people();
        self.grid_from(&mut c, &p)
    }

    /// The grid's patch read (card #1147, `board_cards_patch`): the live
    /// rows among `ids`, each read again from its log, and every live
    /// card's id in grid order. Without a pump keeping the cache, the rows
    /// come from a cold read.
    pub fn cards_patch(&self, ids: &[String]) -> CardsPatch {
        let mut c = self.cache();
        if !c.live {
            drop(c);
            let all = self.cards_in(crate::cards::SCOPE_ALL);
            let want: HashSet<&str> = ids.iter().map(String::as_str).collect();
            return CardsPatch {
                order: all.iter().map(|s| s.id.clone()).collect(),
                rows: all.into_iter().filter(|s| want.contains(s.id.as_str())).collect(),
            };
        }
        self.build(&mut c);
        let mut want: Vec<String> = Vec::new();
        for id in ids {
            add_id(&mut want, id);
        }
        for id in &want {
            self.refresh_entry(&mut c, id);
        }
        let p = self.people();
        let mut r = self.read_of(&p);
        let rows = want
            .iter()
            .filter_map(|id| self.cached_row(&mut c, &mut r, id))
            .collect();
        CardsPatch {
            rows,
            order: c.order(&r.pins),
        }
    }

    /// The grid's drift read (card #1147, `board_cards_cold`): every row
    /// built afresh from the logs, never from the cache. It logs how many
    /// rows the cache had wrong (`drift cards-cache <n>`, with their ids)
    /// and keeps the fresh cache.
    pub fn cards_cold(&self) -> Vec<CardSummary> {
        let mut c = self.cache();
        if !c.live {
            drop(c);
            return self.cards_in(crate::cards::SCOPE_ALL);
        }
        let p = self.people();
        let kept = self.grid_from(&mut c, &p);
        let mut fresh = CardsCache {
            live: true,
            ..CardsCache::default()
        };
        let cold = self.grid_from(&mut fresh, &p);
        let (n, ids) = rows_differ(&kept, &cold);
        let mut line = format!("drift cards-cache {n}");
        if !ids.is_empty() {
            let shown: Vec<&str> = ids.iter().take(20).map(String::as_str).collect();
            line.push_str(&format!(" {}", shown.join(" ")));
        }
        irori_core::log(&line);
        *c = fresh;
        cold
    }

    /// Both reads at once with one People and clock, changing nothing kept
    /// but the Aux the cached read builds: for tests that check the cache
    /// against a cold read (card #1147). None while no pump keeps it.
    pub fn cards_cache_check(&self) -> Option<(Vec<CardSummary>, Vec<CardSummary>)> {
        let mut c = self.cache();
        if !c.live {
            return None;
        }
        let p = self.people();
        let kept = self.grid_from(&mut c, &p);
        let mut fresh = CardsCache {
            live: true,
            ..CardsCache::default()
        };
        Some((kept, self.grid_from(&mut fresh, &p)))
    }

    /// The tray's count from the cache (card #1147): every open card's
    /// kept open blockers, and its gone waits read now.
    pub(crate) fn cached_decide_count(&self) -> Option<usize> {
        let mut c = self.cache();
        if !c.live {
            return None;
        }
        self.build(&mut c);
        let mut n = 0;
        for (id, e) in &c.entries {
            if !e.counted() {
                continue;
            }
            let Some(l) = c.log(&e.log) else { continue };
            if e.blocked || !l.co.gone_waits(id).is_empty() {
                n += 1;
            }
        }
        Some(n)
    }
}

/// How many rows two grid reads differ by, and which: ids in only one,
/// rows that serialize differently, and one more when the rows both hold
/// are in another order.
pub fn rows_differ(a: &[CardSummary], b: &[CardSummary]) -> (usize, Vec<String>) {
    let ser = |s: &CardSummary| serde_json::to_string(s).unwrap_or_default();
    let in_b: HashMap<&str, String> = b.iter().map(|s| (s.id.as_str(), ser(s))).collect();
    let in_a: HashSet<&str> = a.iter().map(|s| s.id.as_str()).collect();
    let mut ids: Vec<String> = Vec::new();
    for s in a {
        if in_b.get(s.id.as_str()) != Some(&ser(s)) {
            ids.push(s.id.clone());
        }
    }
    for s in b {
        if !in_a.contains(s.id.as_str()) {
            ids.push(s.id.clone());
        }
    }
    let ca: Vec<&str> = a
        .iter()
        .map(|s| s.id.as_str())
        .filter(|id| in_b.contains_key(id))
        .collect();
    let cb: Vec<&str> = b
        .iter()
        .map(|s| s.id.as_str())
        .filter(|id| in_a.contains(id))
        .collect();
    let n = ids.len() + usize::from(ca != cb);
    (n, ids)
}
