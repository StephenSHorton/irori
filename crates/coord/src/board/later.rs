//! Not yet and order (card #570, docs/design/later-order.md). A card held
//! as Not yet is BLOCKED with one open item that carries `later`, what
//! brings it back; "after #N" is a plain wait on #N. Both ride on the
//! blocker rows every device reads, so no new row kind.
//!
//! Every rule here is checked at write time and none in the fold: a fold
//! that refused a row an older device took would part ways with it.

use std::collections::{HashMap, HashSet, VecDeque};

use gocompat::quote::quote;
use gocompat::time::GoTime;
use store::kinds;

use super::cards::{block_question, count_open, open_with_question};
use super::comments::check_reason;
use super::error::{Error, Result, bad_state, invalid, not_allowed, not_found};
use super::fold::{Fold, LEFT_PARKED};
use super::rows::{BlockerResolvedRow, BlockerRow, TaskMemberRow, TaskStateRow, TaskStepRow};
use super::tasks::CleanTask;
use super::types::*;
use super::util::{SWEEPER, caller, cmp_str, cut, deeper};
use super::version_wait::{parse_version_wait, valid_version};
use super::wait_chain::{ring_order, task_waits};
use super::{MAX_OPEN_BLOCKERS, MAX_QUESTION};
use crate::gotime;
use crate::names::trim_space;

/// What a held card's question starts with: the person's word for it.
const NOT_YET: &str = "Not yet";

/// Why a held card's starting agent was called off.
pub(crate) const HELD_SPAWN: &str = "the card is Not yet";

/// How many cards a circle's message names before it skips some.
const MAX_CIRCLE_NAMES: usize = 8;

/// What brings a held card back. The row keeps it as the `later` string
/// and the item's `on`; a value this build doesn't know reads as
/// [`Back::Word`], so the card stays held and never closes by itself.
#[derive(Debug, Clone, PartialEq)]
pub enum Back {
    /// When this card (by task id) finishes.
    Card(String),
    /// When this time comes.
    Date(GoTime),
    /// When these devices run this version: a version wait's ref,
    /// `0.13.0 <device> [<device>…]`.
    Version(String),
    /// When the person says so.
    Word,
}

impl Back {
    /// The `later` string. A card's id goes in `on`, where every device's
    /// chains and circles read it.
    pub fn encode(&self) -> String {
        match self {
            Back::Card(_) => "card".into(),
            Back::Date(d) => format!("date {}", d.utc().format_rfc3339()),
            Back::Version(r) => format!("version {r}"),
            Back::Word => "word".into(),
        }
    }

    /// Reads a `later` string and the item's `on`.
    pub fn decode(later: &str, on: Option<&BlockedBy>) -> Back {
        let (word, rest) = later.split_once(' ').unwrap_or((later, ""));
        let known = match word {
            "card" if rest.is_empty() => on
                .filter(|on| on.kind == BLOCKED_ON_TASK && !on.reference.is_empty())
                .map(|on| Back::Card(on.reference.clone())),
            "date" => GoTime::parse_rfc3339(rest).map(Back::Date),
            "version" => {
                let (min, devs) = parse_version_wait(rest);
                (valid_version(&min) && !devs.is_empty()).then(|| Back::Version(rest.into()))
            }
            _ => None,
        };
        known.unwrap_or(Back::Word)
    }

    /// What the item waits on: the card, else Irori, which is calm on every
    /// build and never strands.
    pub fn on(&self) -> BlockedBy {
        match self {
            Back::Card(id) => BlockedBy {
                kind: BLOCKED_ON_TASK.into(),
                reference: id.clone(),
            },
            _ => BlockedBy {
                kind: BLOCKED_ON_AGENT.into(),
                reference: SWEEPER.into(),
            },
        }
    }
}

impl Blocker {
    /// What brings the card back, when this item is a Not yet.
    pub fn back(&self) -> Option<Back> {
        (!self.later.is_empty()).then(|| Back::decode(&self.later, self.on.as_ref()))
    }
}

impl Task {
    /// The cards this one comes after (card #570): its open plain after
    /// items, in the order they were raised. A hold until a card, a
    /// reason-only block and a question are no order.
    pub fn after_cards(&self) -> Vec<String> {
        after_items(self)
    }

    /// The cards this one came after and no longer waits on: its closed
    /// plain after items, for the window's order to show the done steps.
    pub fn after_cards_ended(&self) -> Vec<String> {
        after_where(self, |b| !b.open())
    }

    /// The open item that holds this card as Not yet, if any.
    pub fn parked(&self) -> Option<&Blocker> {
        self.blockers
            .iter()
            .find(|b| b.open() && !b.later.is_empty())
    }
}

/// `2026-12-01`.
pub(crate) fn day_of(d: &GoTime) -> String {
    let (y, m, day, ..) = d.utc().civil();
    format!("{y:04}-{m:02}-{day:02}")
}

/// Whether open item b is the person's to see: a question, a done check
/// or an ownership change. A wait on a PR is its agent's (card #970).
fn for_person(b: &Blocker) -> bool {
    b.done_check
        || b.transfer
        || !b.options.is_empty()
        || b.on.as_ref().is_none_or(|on| on.kind == BLOCKED_ON_PERSON)
}

/// Whether k is held and nothing else on it needs anyone.
pub(crate) fn only_held(k: &Task) -> bool {
    k.parked().is_some()
        && !k
            .blockers
            .iter()
            .any(|b| b.open() && b.later.is_empty() && for_person(b))
}

/// The hold a Not yet item comes from: the last part of its id. A hold
/// changed, reworded or taken by a subtask keeps it, and revise can't
/// change an id, so it names the hold whatever its words.
pub(crate) fn root_of(id: &str) -> &str {
    let parts: Vec<&str> = id.split('-').collect();
    match parts.as_slice() {
        [_, r] | [_, _, r] => r,
        _ => "",
    }
}

/// Whether two items are the same hold, one on a parent and one a subtask
/// took from it, or one a change replaces.
fn same_hold(a: &Blocker, b: &Blocker) -> bool {
    if a.later.is_empty() || b.later.is_empty() {
        return false;
    }
    let r = root_of(&a.id);
    (!r.is_empty() && r == root_of(&b.id)) || (a.later == b.later && a.question == b.question)
}

fn held_by_someone(t: &Task) -> bool {
    !t.holders.is_empty() || !t.co_holders.is_empty()
}

pub(crate) fn parked_refusal(t: &Task, what: &str) -> Error {
    let why = t.parked().map(|b| b.question.as_str()).unwrap_or_default();
    not_allowed(format!(
        "{} is held ({why}): {what}. It comes back by itself, or when the person brings it back",
        t.short_ref()
    ))
}

pub(crate) fn word_refusal(t: &Task) -> Error {
    not_allowed(format!(
        "{} waits for the person's word: only a person brings it back",
        t.short_ref()
    ))
}

/// The question an "after" item asks: its own text, because the fold keeps
/// one open item per question.
fn after_question(o: &Task) -> String {
    cut(
        &format!("After {}: {}", o.short_ref(), o.title),
        MAX_QUESTION,
    )
}

/// One Not yet item, ready to write on a card.
#[derive(Debug, Clone, Default)]
pub(crate) struct Park {
    pub(crate) reason: String,
    pub(crate) question: String,
    pub(crate) context: String,
    pub(crate) on: BlockedBy,
    pub(crate) later: String,
    pub(crate) links: Vec<Link>,
    pub(crate) attachments: Vec<FileRef>,
    /// The hold it carries on ([`root_of`]); empty for a new one.
    pub(crate) root: String,
}

impl Park {
    /// The item b as a subtask takes it.
    fn of(b: &Blocker) -> Park {
        Park {
            reason: b.reason.clone(),
            question: b.question.clone(),
            context: b.context.clone(),
            on: b.on.clone().unwrap_or_default(),
            later: b.later.clone(),
            root: root_of(&b.id).to_string(),
            ..Park::default()
        }
    }
}

/// The loops of cards that can't finish before each other (card #570).
#[derive(Debug, Clone, Default)]
pub struct OrderLoops {
    rings: Vec<Vec<String>>,
    of: HashMap<String, usize>,
}

impl OrderLoops {
    /// The loop card `id` is in, in wait order from the lowest id for a
    /// plain ring, else by id.
    pub fn get(&self, id: &str) -> Option<&[String]> {
        self.of.get(id).map(|&i| self.rings[i].as_slice())
    }

    pub fn contains(&self, id: &str) -> bool {
        self.of.contains_key(id)
    }

    /// Every loop, once each.
    pub fn rings(&self) -> &[Vec<String>] {
        &self.rings
    }

    pub fn is_empty(&self) -> bool {
        self.rings.is_empty()
    }
}

/// What a new card waits on beyond its reason, checked before its rows.
#[derive(Debug, Default)]
pub(crate) struct NewWaits {
    after: Vec<(String, BlockedBy)>,
    park: Option<Park>,
}

impl Fold {
    // ---- order ----

    /// The cards `t` can't finish before: its waits, and its open subtasks,
    /// since a parent finishes with them.
    fn order_edges(&self, t: &Task) -> Vec<String> {
        let mut out = task_waits(t);
        for k in &t.subtasks {
            if !out.contains(k) && self.tasks.get(k).is_some_and(|kt| !is_terminal(&kt.state)) {
                out.push(k.clone());
            }
        }
        out
    }

    /// The shortest way from `from` to `to` along [`Self::order_edges`],
    /// both ends included. Every card is walked: `wait_chain`'s cap would
    /// silently pass a long chain.
    pub(crate) fn order_path(&self, from: &str, to: &str) -> Option<Vec<String>> {
        if from == to {
            return Some(vec![from.to_string()]);
        }
        let mut came: HashMap<String, String> = HashMap::new();
        let mut seen: HashSet<String> = HashSet::from([from.to_string()]);
        let mut todo: VecDeque<String> = VecDeque::from([from.to_string()]);
        while let Some(id) = todo.pop_front() {
            let Some(t) = self.tasks.get(&id) else {
                continue;
            };
            for n in self.order_edges(t) {
                if !seen.insert(n.clone()) {
                    continue;
                }
                came.insert(n.clone(), id.clone());
                if n == to {
                    let mut path = vec![n];
                    while let Some(p) = came.get(path.last().unwrap()) {
                        path.push(p.clone());
                    }
                    path.reverse();
                    return Some(path);
                }
                todo.push_back(n);
            }
        }
        None
    }

    /// Refuses `on` as a wait of card `me` when it closes a circle.
    pub(crate) fn check_edge(&self, me: &str, on: Option<&BlockedBy>) -> Result<()> {
        let Some(on) = on.filter(|on| on.kind == BLOCKED_ON_TASK) else {
            return Ok(());
        };
        if me.is_empty() || on.reference.is_empty() || on.reference == me {
            return Ok(());
        }
        let Some(path) = self.order_path(&on.reference, me) else {
            return Ok(());
        };
        let mut ids = vec![me.to_string()];
        ids.extend(path);
        Err(self.circle(&ids))
    }

    pub(crate) fn order_reaches(&self, from: &str, to: &str) -> bool {
        self.order_path(from, to).is_some()
    }

    /// Every loop along [`Self::order_edges`] among open cards, each card
    /// to its loop's ids: two devices can each write half a circle, and
    /// the fold takes both. One pass over the whole graph (Tarjan, with
    /// no recursion and no cap), so a read pays it once, not per card.
    pub(crate) fn order_loops(&self) -> OrderLoops {
        let open = |id: &str| self.tasks.get(id).filter(|t| !is_terminal(&t.state));
        let mut edges: HashMap<&str, Vec<String>> = HashMap::new();
        for t in self.tasks.values().filter(|t| !is_terminal(&t.state)) {
            let next: Vec<String> = self
                .order_edges(t)
                .into_iter()
                .filter(|n| open(n).is_some())
                .collect();
            if !next.is_empty() {
                edges.insert(t.id.as_str(), next);
            }
        }
        let mut index: HashMap<&str, (usize, usize)> = HashMap::new();
        let mut on_stack: HashSet<&str> = HashSet::new();
        let mut stack: Vec<&str> = Vec::new();
        let mut out = OrderLoops::default();
        let mut starts: Vec<&str> = edges.keys().copied().collect();
        starts.sort();
        for start in starts {
            if index.contains_key(start) {
                continue;
            }
            // (card, next edge to look at)
            let mut work: Vec<(&str, usize)> = vec![(start, 0)];
            while let Some(&(v, i)) = work.last() {
                if i == 0 && !index.contains_key(v) {
                    let n = index.len();
                    index.insert(v, (n, n));
                    stack.push(v);
                    on_stack.insert(v);
                }
                let next = edges.get(v).map(Vec::as_slice).unwrap_or_default();
                if let Some(w) = next.get(i) {
                    work.last_mut().unwrap().1 += 1;
                    let Some(w) = self.tasks.get_key_value(w).map(|(k, _)| k.as_str()) else {
                        continue;
                    };
                    match index.get(w) {
                        None => work.push((w, 0)),
                        Some(&(wi, _)) if on_stack.contains(w) => {
                            let low = &mut index.get_mut(v).unwrap().1;
                            *low = (*low).min(wi);
                        }
                        _ => {}
                    }
                    continue;
                }
                work.pop();
                let (vi, vlow) = index[v];
                if let Some(&(u, _)) = work.last() {
                    let low = &mut index.get_mut(u).unwrap().1;
                    *low = (*low).min(vlow);
                }
                if vi != vlow {
                    continue;
                }
                let mut ring: Vec<String> = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack.remove(w);
                    ring.push(w.to_string());
                    if w == v {
                        break;
                    }
                }
                if ring.len() < 2 {
                    continue;
                }
                let inside: HashMap<String, Vec<String>> = ring
                    .iter()
                    .map(|id| {
                        (
                            id.clone(),
                            edges.get(id.as_str()).cloned().unwrap_or_default(),
                        )
                    })
                    .collect();
                let ring = ring_order(ring, &inside);
                for id in &ring {
                    out.of.insert(id.clone(), out.rings.len());
                }
                out.rings.push(ring);
            }
        }
        out
    }

    /// The refusal for the circle `ids` would make, first card last again.
    pub(crate) fn circle(&self, ids: &[String]) -> Error {
        let names: Vec<String> = ids.iter().map(|id| self.ref_of(id)).collect();
        circle_of(&names)
    }

    /// Whether a new subtask of `parent` may wait on `k`: its parent can't
    /// finish before it, so `k` must not need the parent.
    fn check_new_subtask(&self, parent: &str, k: &str) -> Result<()> {
        if parent.is_empty() {
            return Ok(());
        }
        let Some(path) = self.order_path(k, parent) else {
            return Ok(());
        };
        let mut names = vec![self.ref_of(parent), "the new subtask".to_string()];
        names.extend(path.iter().map(|id| self.ref_of(id)));
        Err(circle_of(&names))
    }

    /// Whether `k` may become a subtask of `parent`: a card that needs its
    /// new parent makes a circle, and a held parent holds it too.
    pub(crate) fn check_adopt(&self, parent: &str, k: &str) -> Result<()> {
        let on_it = self.tasks.get(k).is_some_and(held_by_someone);
        self.adopt_check(parent, k, on_it)
    }

    /// [`Self::check_adopt`] for an ask closed as a card under `parent`
    /// that goes out with nobody on it.
    pub(crate) fn check_ask_adopt(&self, parent: &str, k: &str) -> Result<()> {
        self.adopt_check(parent, k, false)
    }

    fn adopt_check(&self, parent: &str, k: &str, on_it: bool) -> Result<()> {
        if let Some(path) = self.order_path(k, parent) {
            let mut ids = vec![parent.to_string()];
            ids.extend(path);
            return Err(self.circle(&ids));
        }
        let Some(item) = self.tasks.get(parent).and_then(|p| p.parked()) else {
            return Ok(());
        };
        let kt = &self.tasks[k];
        // A hold until k itself leaves k free (finding 7).
        if item.on.as_ref().is_some_and(|on| on.reference == k) {
            return Ok(());
        }
        if on_it {
            return Err(bad_state(format!(
                "{} is held, and {} is Not yet: it can't take a card someone is on",
                kt.short_ref(),
                self.ref_of(parent)
            )));
        }
        if kt.parked().is_some() {
            return Ok(());
        }
        self.check_parkable(kt)?;
        self.check_room(kt)?;
        // The fold would merge the hold into that item and drop its wait.
        if let Some(i) = open_with_question(kt, &item.question) {
            return Err(invalid(format!(
                "{} has {} with the words of {}'s hold ({}): say one of them apart first",
                kt.short_ref(),
                kt.blockers[i].id,
                self.ref_of(parent),
                quote(&item.question)
            )));
        }
        if let Some(Back::Card(_)) = item.back() {
            self.check_blocker(item.on.as_ref(), k)?;
        }
        Ok(())
    }

    /// Puts parent's hold on `k`, a subtask just made or adopted: a parked
    /// parent doesn't hold its subtasks, real rows do.
    pub(crate) fn inherit_park(&mut self, by: &str, parent: &str, k: &str) -> Result<()> {
        let Some(item) = self.tasks.get(parent).and_then(|p| p.parked()).cloned() else {
            return Ok(());
        };
        let Some(kt) = self.tasks.get(k).cloned() else {
            return Ok(());
        };
        let its_own = item.on.as_ref().is_some_and(|on| on.reference == k);
        let clash = open_with_question(&kt, &item.question).is_some();
        if held_by_someone(&kt) || kt.parked().is_some() || its_own || clash {
            return Ok(());
        }
        self.write_park(by, k, &kt.note, &Park::of(&item))?;
        self.end_pending_spawns(by, k, HELD_SPAWN)
    }

    /// The sweep's part (finding 27): an old device's subtask of a held
    /// parent has no hold, so its origin would hand it out. This device
    /// puts the parent's hold on it, as ordinary rows, where that row
    /// counts. Only a subtask never held, with nobody on it: one a person
    /// freed stays free, even once its closed hold is trimmed.
    pub(crate) fn hold_old_subtasks(&mut self) -> Result<()> {
        let mut todo: Vec<(String, Park)> = Vec::new();
        for p in self.tasks.values() {
            let Some(item) = p.parked() else {
                continue;
            };
            for k in p.subtasks.iter().filter_map(|id| self.tasks.get(id)) {
                let free = !is_terminal(&k.state)
                    && k.open_ask().is_none()
                    && !held_by_someone(k)
                    && k.assigned.is_empty()
                    && !k.was_held
                    && open_with_question(k, &item.question).is_none()
                    && item.on.as_ref().is_none_or(|on| on.reference != k.id);
                if !free
                    || !self.may_cancel(k, &self.dev, None)
                    || self.check_parkable(k).is_err()
                    || self.check_room(k).is_err()
                {
                    continue;
                }
                if let Some(Back::Card(_)) = item.back()
                    && self.check_blocker(item.on.as_ref(), &k.id).is_err()
                {
                    continue;
                }
                todo.push((k.id.clone(), Park::of(item)));
            }
        }
        for (id, park) in todo {
            let note = self.tasks[&id].note.clone();
            self.write_park(SWEEPER, &id, &note, &park)?;
        }
        Ok(())
    }

    /// The sweep's part (round two, finding 8): a subtask's copy of its
    /// parent's hold ends once that hold has, though it was made on a
    /// device that didn't see the last change. A parent whose closed item
    /// was trimmed away leaves the copy, the safe way.
    pub(crate) fn end_kid_holds(&mut self) -> Result<()> {
        let mut todo: Vec<(String, String)> = Vec::new();
        for k in self.tasks.values().filter(|k| !is_terminal(&k.state)) {
            let Some(p) = self.tasks.get(&k.parent) else {
                continue;
            };
            for b in k
                .blockers
                .iter()
                .filter(|b| b.open() && !b.later.is_empty())
            {
                let r = root_of(&b.id);
                let of_r = |x: &&Blocker| !x.later.is_empty() && root_of(&x.id) == r;
                let ended = !r.is_empty()
                    && p.blockers.iter().any(|x| of_r(&x))
                    && !p.blockers.iter().filter(of_r).any(Blocker::open);
                if ended && self.may_sweep_close(k, b) {
                    todo.push((k.id.clone(), b.id.clone()));
                }
            }
        }
        for (id, bid) in todo {
            self.close_items(SWEEPER, &id, &[bid], "its parent's hold ended")?;
        }
        Ok(())
    }

    /// The sweep's part (finding 23): ends held cards' requests that a
    /// park on another person's device couldn't.
    pub(crate) fn end_held_spawns(&mut self) -> Result<()> {
        let ids: Vec<String> = self
            .tasks
            .values()
            .filter(|t| t.parked().is_some() && t.spawns.iter().any(|s| s.pending()))
            .map(|t| t.id.clone())
            .collect();
        for id in ids {
            self.end_pending_spawns(SWEEPER, &id, HELD_SPAWN)?;
        }
        Ok(())
    }

    /// The sweep's part (round two, finding 10): an agent that took a
    /// held card, or a step on it, on a device that hadn't seen the hold
    /// yet is taken off it. Only from its person's device, where those
    /// rows count, and told so.
    pub(crate) fn take_off_held(&mut self, out: &mut Vec<Notice>) -> Result<()> {
        let mine = |f: &Fold, k: &str| !k.is_empty() && !f.is_person(k) && f.may_write(k, &f.dev);
        let todo: Vec<Task> = self
            .tasks
            .values()
            .filter(|t| t.parked().is_some())
            .filter(|t| {
                t.holders.iter().any(|k| mine(self, k))
                    || t.steps.iter().any(|s| !s.done && mine(self, &s.owner))
            })
            .cloned()
            .collect();
        for t in todo {
            let mut told: Vec<String> = Vec::new();
            let steps: Vec<&Step> = t
                .steps
                .iter()
                .filter(|s| !s.done && mine(self, &s.owner))
                .collect();
            let holders: Vec<&String> = t.holders.iter().filter(|k| mine(self, k)).collect();
            for s in steps {
                // A sweeper's step row doesn't count on a board: the agent's
                // own does.
                let by = if self.on_board {
                    s.owner.as_str()
                } else {
                    SWEEPER
                };
                let at = self.stamp();
                self.write(
                    by,
                    kinds::TASK_STEP,
                    &TaskStepRow {
                        task_id: t.id.clone(),
                        step_id: s.id.clone(),
                        owner: Some(String::new()),
                        by: by.into(),
                        at,
                        ..TaskStepRow::default()
                    },
                )?;
                told.push(s.owner.clone());
            }
            for who in holders {
                let at = self.stamp();
                self.write(
                    SWEEPER,
                    kinds::TASK_LEFT,
                    &TaskMemberRow {
                        task_id: t.id.clone(),
                        who: who.clone(),
                        by: SWEEPER.into(),
                        at,
                        reason: LEFT_PARKED.into(),
                        ..TaskMemberRow::default()
                    },
                )?;
                told.push(who.clone());
            }
            told.sort();
            told.dedup();
            out.push(Notice {
                kind: NOTICE_PARKED.into(),
                task_id: t.id.clone(),
                reference: format!("task:{}", t.id),
                title: t.title.clone(),
                by: SWEEPER.into(),
                to: told,
                text: format!(
                    "{} ({}) is held for now, so you are off it: stop work on it, and leave where you got to on the card for whoever picks it up.{}",
                    quote(&t.title),
                    t.refs(),
                    deeper(&t.id)
                ),
                wake: true,
                ..Notice::default()
            });
        }
        Ok(())
    }

    // ---- holding a card ----

    /// Whether a card may be held: what stays Needs you stays, because
    /// raising here would swallow it.
    fn check_parkable(&self, t: &Task) -> Result<()> {
        if t.state == INPUT_REQUIRED {
            return Err(bad_state(format!(
                "{} waits on the person's input: that goes first",
                t.short_ref()
            )));
        }
        if let Some(b) = t
            .blockers
            .iter()
            .find(|b| b.open() && b.later.is_empty() && for_person(b))
        {
            return Err(bad_state(format!(
                "{} has {} open for the person ({}): that goes first",
                t.short_ref(),
                b.id,
                b.question
            )));
        }
        Ok(())
    }

    fn check_room(&self, t: &Task) -> Result<()> {
        if count_open(t) >= MAX_OPEN_BLOCKERS {
            return Err(invalid(format!(
                "task {} already has {MAX_OPEN_BLOCKERS} open blockers",
                t.id
            )));
        }
        Ok(())
    }

    /// Validates and normalizes `back` for card `me` (`""` while it is
    /// being made).
    fn check_back(&self, back: Back, me: &str) -> Result<Back> {
        match back {
            Back::Card(r) => {
                let id = self.task_ref_locked(&r)?;
                let on = Back::Card(id).on();
                let on = self.check_blocker(Some(&on), me)?.unwrap_or_default();
                Ok(Back::Card(on.reference))
            }
            Back::Date(d) => {
                if !gotime::after(&d, &self.stamp()) {
                    return Err(invalid(format!(
                        "{} has passed: a card held until then would come back at once",
                        day_of(&d)
                    )));
                }
                Ok(Back::Date(d))
            }
            // A version wait's own checks, a board's refusal among them.
            // The row itself waits on Irori: older windows don't know
            // `on: version`, and a version park is raised when one lags.
            Back::Version(r) => {
                let on = BlockedBy {
                    kind: BLOCKED_ON_VERSION.into(),
                    reference: r,
                };
                let on = self.check_blocker(Some(&on), me)?.unwrap_or_default();
                Ok(Back::Version(on.reference))
            }
            Back::Word => Ok(Back::Word),
        }
    }

    /// How the card says when it comes back.
    fn back_line(&self, back: &Back) -> String {
        match back {
            Back::Card(id) => format!("Comes back when {} finishes", self.ref_of(id)),
            Back::Date(d) => format!("Comes back on {}", day_of(d)),
            Back::Version(r) => {
                let (min, devs) = parse_version_wait(r);
                format!("Comes back when {min} runs on {}", devs.join(", "))
            }
            Back::Word => "Comes back when the person says so".into(),
        }
    }

    fn park_item(&self, why: &str, back: &Back) -> Park {
        let reason = format!("{NOT_YET}: {why}");
        let (question, more) = block_question(&reason);
        let line = self.back_line(back);
        let context = if more.is_empty() {
            line
        } else {
            format!("{more}\n{line}")
        };
        Park {
            reason,
            question,
            context,
            on: back.on(),
            later: back.encode(),
            ..Park::default()
        }
    }

    /// Writes item p on card `id` as a new item, with an id no other
    /// device picks. Never a replace: one crossing a close of the same id
    /// leaves the card blocked with nothing open. Not reason-only: a later
    /// plain block would replace that. Returns the item's id.
    pub(crate) fn write_park(
        &mut self,
        by: &str,
        id: &str,
        note: &str,
        p: &Park,
    ) -> Result<String> {
        let bid = match p.root.as_str() {
            "" => self.fresh_blocker_id(id),
            r => format!("{}-{r}", self.fresh_blocker_id(id)),
        };
        let at = self.stamp();
        self.write(
            by,
            kinds::TASK_STATUS,
            &TaskStateRow {
                task_id: id.into(),
                state: BLOCKED.into(),
                note: note.into(),
                reason: p.reason.clone(),
                blocked_by: Some(p.on.clone()),
                blocker: Some(BlockerRow {
                    id: bid.clone(),
                    question: p.question.clone(),
                    context: p.context.clone(),
                    on: Some(p.on.clone()),
                    later: p.later.clone(),
                    links: p.links.clone(),
                    attachments: p.attachments.clone(),
                    ..BlockerRow::default()
                }),
                by: by.into(),
                at,
                ..TaskStateRow::default()
            },
        )?;
        Ok(bid)
    }

    /// Takes everyone off card t, after its blocker row: `leave` writes
    /// SUBMITTED only from WORKING, so it stays held. A kept holder would
    /// come back silently as WORKING, and a step's owner holds it too.
    fn drop_holders(&mut self, by: &str, t: &Task) -> Result<()> {
        for s in t.steps.iter().filter(|s| !s.done && !s.owner.is_empty()) {
            let at = self.stamp();
            self.write(
                by,
                kinds::TASK_STEP,
                &TaskStepRow {
                    task_id: t.id.clone(),
                    step_id: s.id.clone(),
                    owner: Some(String::new()),
                    by: by.into(),
                    at,
                    ..TaskStepRow::default()
                },
            )?;
        }
        for who in &t.holders {
            let at = self.stamp();
            self.write(
                by,
                kinds::TASK_LEFT,
                &TaskMemberRow {
                    task_id: t.id.clone(),
                    who: who.clone(),
                    by: by.into(),
                    at,
                    reason: LEFT_PARKED.into(),
                    ..TaskMemberRow::default()
                },
            )?;
        }
        Ok(())
    }

    /// The subtasks a hold on t goes on. One that took one of t's `old`
    /// holds gets the new one; one with a hold of its own keeps it.
    fn kids_to_park(
        &self,
        t: &Task,
        old: &[Blocker],
        item: &Park,
        back: &Back,
    ) -> Result<Vec<Task>> {
        let mut out = Vec::new();
        for k in t.subtasks.iter().filter_map(|id| self.tasks.get(id)) {
            if is_terminal(&k.state) {
                continue;
            }
            // A hold until this very subtask leaves it free (finding 7).
            if matches!(back, Back::Card(c) if *c == k.id) {
                continue;
            }
            if held_by_someone(k) {
                return Err(bad_state(format!(
                    "subtask {} of {} is held: it finishes first, or its holder leaves it",
                    k.short_ref(),
                    t.short_ref()
                )));
            }
            let took = |b: &Blocker| old.iter().any(|o| same_hold(b, o));
            let same = k.blockers.iter().any(|b| b.open() && took(b));
            if !same {
                if k.parked().is_some() {
                    continue;
                }
                self.check_parkable(k)?;
                self.check_room(k)?;
            }
            // The fold would merge the hold into that item and drop its wait.
            if let Some(b) = k
                .blockers
                .iter()
                .find(|b| b.open() && b.question == item.question && !took(b))
            {
                return Err(invalid(format!(
                    "subtask {} of {} has {} with the hold's words ({}): say it another way",
                    k.short_ref(),
                    t.short_ref(),
                    b.id,
                    quote(&item.question)
                )));
            }
            if matches!(back, Back::Card(_)) {
                self.check_blocker(Some(&back.on()), &k.id)?;
            }
            out.push(k.clone());
        }
        Ok(out)
    }

    /// Holds card `id` until `back`. On a held card this changes the hold,
    /// on the same item; a hold on the person's word changes only by a
    /// person. Parking a parent puts the same item on its open subtasks.
    pub(crate) fn park(
        &mut self,
        by: &str,
        id: &str,
        why: &str,
        back: Back,
    ) -> Result<(Task, Vec<Notice>)> {
        let by = caller(by)?;
        let why = trim_space(why).to_string();
        check_reason(&why, true)?;
        let t = self.open_task(id)?.clone();
        if t.open_ask().is_some() {
            return Err(super::asks::ask_refusal(
                &t,
                "an ask isn't held: ask_close it with outcome card, then hold that card",
            ));
        }
        let person = self.is_person(&by);
        // Every hold: two made apart are changed together.
        let old = Self::parks_of(&t);
        if old.is_empty() {
            self.check_parkable(&t)?;
            self.check_room(&t)?;
        } else if !person && old.iter().any(|o| o.back() == Some(Back::Word)) {
            return Err(word_refusal(&t));
        }
        let on_it = t.holders.iter().chain(&t.co_holders);
        if !person && on_it.clone().any(|k| *k != by) {
            return Err(not_allowed(format!(
                "{} is held: only its holder or a person may hold it back",
                t.short_ref()
            )));
        }
        let back = self.check_back(back, id)?;
        let mut item = self.park_item(&why, &back);
        if let Some(o) = old.first() {
            item.root = root_of(&o.id).to_string();
        }
        if let Some(b) = t
            .blockers
            .iter()
            .find(|b| b.open() && b.question == item.question && !old.iter().any(|o| o.id == b.id))
        {
            return Err(invalid(format!(
                "{} already has {} with that question",
                t.short_ref(),
                b.id
            )));
        }
        let kids = self.kids_to_park(&t, &old, &item, &back)?;
        let told: Vec<String> = on_it
            .filter(|k| **k != by && !self.is_person(k) && *k != SWEEPER)
            .cloned()
            .collect();

        // A change raises the new item, then closes the old ones: a
        // replace crossing the old item's close would leave nothing open.
        let answer = format!("held again: {}", self.back_line(&back));
        self.swap_holds(&by, &t, &old, &item, &answer)?;
        self.drop_holders(&by, &t)?;
        self.end_pending_spawns(&by, id, HELD_SPAWN)?;
        // A subtask the new hold leaves free (finding 7) loses the old one.
        let mut free = t.clone();
        free.subtasks.retain(|s| kids.iter().all(|k| k.id != *s));
        self.close_kid_parks(&by, &free, &old, &answer)?;
        for k in kids {
            let took: Vec<Blocker> = Self::parks_of(&k)
                .into_iter()
                .filter(|b| old.iter().any(|o| same_hold(b, o)))
                .collect();
            self.swap_holds(&by, &k, &took, &item, &answer)?;
            self.end_pending_spawns(&by, &k.id, HELD_SPAWN)?;
        }

        let nt = self.tasks[id].clone();
        let mut ns = Vec::new();
        if !told.is_empty() {
            ns.push(Notice {
                kind: NOTICE_PARKED.into(),
                task_id: id.into(),
                reference: format!("task:{id}"),
                title: t.title.clone(),
                by: by.clone(),
                to: told,
                text: format!(
                    "{} held {} ({}) for now: {why}. {}. You are off it: stop work on it, and leave where you got to on the card for whoever picks it up.{}",
                    self.named(&by),
                    quote(&t.title),
                    t.refs(),
                    self.back_line(&back),
                    deeper(id)
                ),
                wake: true,
                ..Notice::default()
            });
        }
        Ok((nt, ns))
    }

    /// Revise of hold b on t: closes it and raises the same hold with
    /// question q and context cx under a new id, on t and on the subtasks
    /// that took it, so bringing t back still finds them (finding 15).
    pub(crate) fn reword_hold(
        &mut self,
        by: &str,
        t: &Task,
        b: &Blocker,
        q: String,
        cx: String,
    ) -> Result<(Task, Blocker)> {
        if let Some(p) = self.tasks.get(&t.parent)
            && Self::parks_of(p).iter().any(|o| same_hold(b, o))
        {
            return Err(invalid(format!(
                "{} took this hold from {}: change it there",
                t.short_ref(),
                p.short_ref()
            )));
        }
        if q != b.question && open_with_question(t, &q).is_some() {
            return Err(invalid(format!(
                "{} already has an item with that question",
                t.short_ref()
            )));
        }
        let kids: Vec<Task> = t
            .subtasks
            .iter()
            .filter_map(|id| self.tasks.get(id))
            .filter(|k| k.blockers.iter().any(|x| x.open() && same_hold(x, b)))
            .cloned()
            .collect();
        for k in &kids {
            if q != b.question && open_with_question(k, &q).is_some() {
                return Err(invalid(format!(
                    "its subtask {} already has an item with that question",
                    k.short_ref()
                )));
            }
        }
        let p = Park {
            reason: q.clone(),
            question: q,
            context: cx,
            on: b.on.clone().unwrap_or_default(),
            later: b.later.clone(),
            links: b.links.clone(),
            attachments: b.attachments.clone(),
            root: root_of(&b.id).to_string(),
        };
        let bid = self.swap_holds(by, t, std::slice::from_ref(b), &p, "reworded")?;
        for k in &kids {
            let took: Vec<Blocker> = Self::parks_of(k)
                .into_iter()
                .filter(|x| same_hold(x, b))
                .collect();
            self.swap_holds(by, k, &took, &p, "reworded")?;
        }
        let nt = self.tasks[&t.id].clone();
        let out = nt
            .blockers
            .iter()
            .find(|x| x.id == bid)
            .cloned()
            .unwrap_or_default();
        Ok((nt, out))
    }

    /// Closes items of card `id` one by one: `blocker.closed` names no item,
    /// so a late one would close a newer hold. The last open one says
    /// where the card goes.
    pub(crate) fn close_items(
        &mut self,
        by: &str,
        id: &str,
        bids: &[String],
        answer: &str,
    ) -> Result<()> {
        for bid in bids {
            let t = &self.tasks[id];
            let restored = if count_open(t) == 1 {
                self.restore_for(t).to_string()
            } else {
                String::new()
            };
            for x in std::iter::once(bid.clone()).chain(merged_into(t, bid)) {
                let at = self.stamp();
                self.write(
                    by,
                    kinds::BLOCKER_RESOLVED,
                    &BlockerResolvedRow {
                        task_id: id.into(),
                        blocker_id: x,
                        answer: answer.into(),
                        option: -1,
                        closed: true,
                        restored: restored.clone(),
                        by: by.into(),
                        at,
                        ..BlockerResolvedRow::default()
                    },
                )?;
            }
        }
        Ok(())
    }

    /// Puts hold p on card t in place of its holds `old`: the new item is
    /// raised before the old ones close, so a device that has only part
    /// of this never sees the card free. One with p's own words is renamed
    /// first, or the raise would merge into it. Returns the new item's id.
    fn swap_holds(
        &mut self,
        by: &str,
        t: &Task,
        old: &[Blocker],
        p: &Park,
        answer: &str,
    ) -> Result<String> {
        for o in old.iter().filter(|o| o.question == p.question) {
            let at = self.stamp();
            let q = cut(&format!("{} (changed)", o.question), MAX_QUESTION);
            self.write(
                by,
                kinds::TASK_STATUS,
                &TaskStateRow {
                    task_id: t.id.clone(),
                    state: BLOCKED.into(),
                    note: t.note.clone(),
                    reason: q.clone(),
                    blocked_by: o.on.clone(),
                    blocker: Some(BlockerRow {
                        id: o.id.clone(),
                        question: q,
                        context: o.context.clone(),
                        on: o.on.clone(),
                        later: o.later.clone(),
                        replace: true,
                        ..BlockerRow::default()
                    }),
                    by: by.into(),
                    at,
                    ..TaskStateRow::default()
                },
            )?;
        }
        let bid = self.write_park(by, &t.id, &t.note, p)?;
        let ids: Vec<String> = old.iter().map(|o| o.id.clone()).collect();
        self.close_items(by, &t.id, &ids, answer)?;
        Ok(bid)
    }

    /// Closes what t's subtasks took from its `items`: a hold on the
    /// person's word has nothing else to end it there.
    pub(crate) fn close_kid_parks(
        &mut self,
        by: &str,
        t: &Task,
        items: &[Blocker],
        answer: &str,
    ) -> Result<()> {
        for id in &t.subtasks {
            let Some(k) = self.tasks.get(id) else {
                continue;
            };
            let bids: Vec<String> = k
                .blockers
                .iter()
                .filter(|b| b.open() && items.iter().any(|o| same_hold(b, o)))
                .map(|b| b.id.clone())
                .collect();
            self.close_items(by, id, &bids, answer)?;
        }
        Ok(())
    }

    fn parks_of(t: &Task) -> Vec<Blocker> {
        t.blockers
            .iter()
            .filter(|b| b.open() && !b.later.is_empty())
            .cloned()
            .collect()
    }

    /// Ends card `id`'s hold, if it has one, with `answer` as what ended it.
    pub(crate) fn end_park(&mut self, by: &str, id: &str, answer: &str) -> Result<()> {
        let t = self.tasks[id].clone();
        let items = Self::parks_of(&t);
        let bids: Vec<String> = items.iter().map(|b| b.id.clone()).collect();
        self.close_items(by, id, &bids, answer)?;
        self.close_kid_parks(by, &t, &items, answer)
    }

    /// Why `by` may not take held card t. A person may: that is their word,
    /// and [`Self::end_park`] runs before the row that takes it.
    pub(crate) fn parked_take_refusal(&self, by: &str, t: &Task) -> Result<()> {
        if t.parked().is_some() && !self.is_person(by) {
            return Err(parked_refusal(t, "nobody takes it for now"));
        }
        Ok(())
    }

    /// A new step owner on held card t (finding 10): refused for an agent,
    /// as claim is. A person's is their word, as assign is: the caller
    /// ends the hold before its row.
    pub(crate) fn parked_owner_refusal(&self, by: &str, t: &Task) -> Result<()> {
        if t.parked().is_some() && !self.is_person(by) {
            return Err(parked_refusal(t, "nobody is given a step on it for now"));
        }
        Ok(())
    }

    /// Brings card `id` back now. A hold on the person's word is theirs to
    /// end; anyone may end the others early.
    pub(crate) fn bring_back(
        &mut self,
        by: &str,
        id: &str,
        note: &str,
    ) -> Result<(Task, Vec<Notice>)> {
        let by = caller(by)?;
        let note = trim_space(note).to_string();
        check_reason(&note, false)?;
        let t = self.open_task(id)?.clone();
        let items = Self::parks_of(&t);
        if items.is_empty() {
            return Err(bad_state(format!("{} isn't held", t.short_ref())));
        }
        if !self.is_person(&by) && items.iter().any(|b| b.back() == Some(Back::Word)) {
            return Err(word_refusal(&t));
        }
        self.end_park(&by, id, &cmp_str(&note, "brought back"))?;
        let nt = self.tasks[id].clone();
        let mut ns = Vec::new();
        let to = self.recipients(&nt, &by);
        if !to.is_empty() {
            ns.push(self.back_notice(&by, &nt, &note, to));
        }
        Ok((nt, ns))
    }

    /// `board_unblock` with no item on a held card: agents are refused, and
    /// a person's closes every open item, one by one.
    pub(crate) fn unblock_parked(
        &mut self,
        by: &str,
        id: &str,
        note: &str,
    ) -> Result<(Task, Notice)> {
        let t = self.tasks[id].clone();
        if self.is_agent_key(by) {
            return Err(parked_refusal(&t, "an agent doesn't unblock it"));
        }
        let answer = cmp_str(note, "brought back");
        let items = Self::parks_of(&t);
        let bids: Vec<String> = t
            .blockers
            .iter()
            .filter(|b| b.open())
            .map(|b| b.id.clone())
            .collect();
        self.close_items(by, id, &bids, &answer)?;
        self.close_kid_parks(by, &t, &items, &answer)?;
        let nt = self.tasks[id].clone();
        let to = self.recipients(&nt, by);
        let n = self.back_notice(by, &nt, note, to);
        Ok((nt, n))
    }

    fn back_notice(&self, by: &str, t: &Task, note: &str, to: Vec<String>) -> Notice {
        let mut b = format!(
            "{} brought {} ({}) back",
            self.named(by),
            quote(&t.title),
            t.refs()
        );
        if !note.is_empty() {
            b.push_str(&format!(": {note}"));
        }
        b.push_str(&format!(". It is {} again.", t.state));
        b.push_str(&deeper(&t.id));
        Notice {
            kind: NOTICE_UNBLOCKED.into(),
            task_id: t.id.clone(),
            reference: format!("task:{}", t.id),
            title: t.title.clone(),
            by: by.into(),
            to,
            text: b,
            ..Notice::default()
        }
    }

    /// Whether this device's close of item b would count: a sweeper's
    /// row on a board folds only from a device of the raiser's person, the
    /// creator's person or an owner. Any other device would write a dead
    /// row every sweep.
    pub(crate) fn may_sweep_close(&self, t: &Task, b: &Blocker) -> bool {
        !self.on_board
            || self.may_write(&b.raised_by, &self.dev)
            || self.may_cancel(t, &self.dev, None)
    }

    // ---- after ----

    /// Writes one "after" wait on card `id`.
    fn write_after(
        &mut self,
        by: &str,
        id: &str,
        note: &str,
        question: &str,
        on: &BlockedBy,
    ) -> Result<()> {
        let bid = self.fresh_blocker_id(id);
        let at = self.stamp();
        self.write(
            by,
            kinds::TASK_STATUS,
            &TaskStateRow {
                task_id: id.into(),
                state: BLOCKED.into(),
                note: note.into(),
                reason: question.into(),
                blocked_by: Some(on.clone()),
                blocker: Some(BlockerRow {
                    id: bid,
                    question: question.into(),
                    on: Some(on.clone()),
                    ..BlockerRow::default()
                }),
                by: by.into(),
                at,
                ..TaskStateRow::default()
            },
        )
    }

    /// Puts card `id` after `cards`, keeping its holder as `board_block`
    /// does. The whole list is checked before the first row; a card it is
    /// already after (a plain after item) is left as it is.
    pub(crate) fn add_after(&mut self, by: &str, id: &str, cards: &[String]) -> Result<Task> {
        let by = caller(by)?;
        let t = self.open_task(id)?.clone();
        if t.open_ask().is_some() {
            return Err(super::asks::ask_refusal(
                &t,
                "an ask has no order: ask_close it with outcome card first",
            ));
        }
        if cards.is_empty() {
            return Err(invalid("give the cards it comes after"));
        }
        let have = after_items(&t);
        let mut waits: Vec<(String, BlockedBy)> = Vec::new();
        for r in cards {
            let k = self.task_ref_locked(r)?;
            if have.contains(&k) || waits.iter().any(|(_, on)| on.reference == k) {
                continue;
            }
            let on = Back::Card(k).on();
            let on = self.check_blocker(Some(&on), id)?.unwrap_or_default();
            let q = after_question(&self.tasks[&on.reference]);
            if open_with_question(&t, &q).is_some() || waits.iter().any(|(o, _)| *o == q) {
                return Err(invalid(format!(
                    "{} already has an item that says {}: two cards read the same, so say them apart",
                    t.short_ref(),
                    quote(&q)
                )));
            }
            waits.push((q, on));
        }
        if count_open(&t) + waits.len() > MAX_OPEN_BLOCKERS {
            return Err(invalid(format!(
                "that would make {} open blockers on {} (at most {MAX_OPEN_BLOCKERS})",
                count_open(&t) + waits.len(),
                t.short_ref()
            )));
        }
        for (q, on) in &waits {
            self.write_after(&by, id, &t.note, q, on)?;
        }
        Ok(self.tasks[id].clone())
    }

    /// Takes card `id` off its waits on `cards`, one by one; with nothing
    /// left open it goes out again. The whole list is checked first. A
    /// hold until a card isn't order: bringing it back ends that.
    pub(crate) fn remove_after(&mut self, by: &str, id: &str, cards: &[String]) -> Result<Task> {
        let by = caller(by)?;
        let t = self.open_task(id)?.clone();
        if cards.is_empty() {
            return Err(invalid("give the cards it no longer comes after"));
        }
        let mut gone: Vec<(String, Vec<String>)> = Vec::new();
        for r in cards {
            let k = self.removal_ref(r)?;
            if gone.iter().any(|(id, _)| *id == k) {
                continue;
            }
            let bids: Vec<String> = t
                .blockers
                .iter()
                .filter(|b| b.open() && b.later.is_empty() && b.options.is_empty())
                .filter(|b| {
                    b.on.as_ref()
                        .is_some_and(|on| on.kind == BLOCKED_ON_TASK && on.reference == k)
                })
                .map(|b| b.id.clone())
                .collect();
            if bids.is_empty() {
                return Err(not_found(format!(
                    "{} doesn't come after {}",
                    t.short_ref(),
                    self.ref_of(&k)
                )));
            }
            gone.push((k, bids));
        }
        for (k, bids) in &gone {
            let answer = format!("no longer after {}", self.ref_of(k));
            self.close_items(&by, id, bids, &answer)?;
        }
        Ok(self.tasks[id].clone())
    }

    // ---- a new card ----

    /// Checks all a new card `ct` waits on, before its first row: `after`,
    /// `later`, and the hold it takes from a held parent.
    pub(crate) fn new_waits(&self, ct: &CleanTask, bb: Option<&BlockedBy>) -> Result<NewWaits> {
        if ct.later.is_some() && ct.blocked_by.is_some() {
            return Err(invalid("a new card takes later or a wait, not both"));
        }
        let held = self.tasks.get(&ct.parent).and_then(|p| p.parked()).cloned();
        if ct.later.is_some() && held.is_some() {
            return Err(invalid(format!(
                "{} is Not yet, so a new subtask is too: leave later out",
                self.ref_of(&ct.parent)
            )));
        }
        if let Some(bb) = bb.filter(|bb| bb.kind == BLOCKED_ON_TASK) {
            self.check_new_subtask(&ct.parent, &bb.reference)?;
        }
        let mut out = NewWaits::default();
        // The fold merges items by question, so each needs its own.
        let mut asked: Vec<String> = Vec::new();
        if !ct.reason.is_empty() && ct.later.is_none() {
            asked.push(block_question(&ct.reason).0);
        }
        // A blocked_by is reason-only, so it isn't order: after writes its own.
        for r in &ct.after {
            let k = self.task_ref_locked(r)?;
            if out.after.iter().any(|(_, on)| on.reference == k) {
                continue;
            }
            let on = Back::Card(k).on();
            let on = self.check_blocker(Some(&on), "")?.unwrap_or_default();
            self.check_new_subtask(&ct.parent, &on.reference)?;
            let q = after_question(&self.tasks[&on.reference]);
            if asked.contains(&q) {
                return Err(same_question(&q));
            }
            asked.push(q.clone());
            out.after.push((q, on));
        }
        out.park = match (&ct.later, held) {
            (Some(back), _) => {
                let back = self.check_back(back.clone(), "")?;
                if let Back::Card(k) = &back {
                    self.check_new_subtask(&ct.parent, k)?;
                }
                Some(self.park_item(&cmp_str(&ct.reason, &ct.title), &back))
            }
            (None, Some(item)) => Some(Park::of(&item)),
            (None, None) => None,
        };
        if let Some(p) = &out.park
            && asked.contains(&p.question)
        {
            return Err(same_question(&p.question));
        }
        let plain = usize::from(!ct.reason.is_empty() && ct.later.is_none());
        let n = plain + out.after.len() + usize::from(out.park.is_some());
        if n > MAX_OPEN_BLOCKERS {
            return Err(invalid(format!(
                "that makes {n} open blockers on a new card (at most {MAX_OPEN_BLOCKERS})"
            )));
        }
        Ok(out)
    }

    /// Writes what [`Self::new_waits`] checked on the new card `id`.
    pub(crate) fn write_new_waits(&mut self, by: &str, id: &str, w: NewWaits) -> Result<()> {
        for (q, on) in &w.after {
            self.write_after(by, id, "", q, on)?;
        }
        if let Some(p) = &w.park {
            self.write_park(by, id, "", p)?;
        }
        Ok(())
    }
}

/// The cards `t` is after: open task waits that are no hold, no
/// reason-only item (a later block replaces it) and no question.
fn after_items(t: &Task) -> Vec<String> {
    after_where(t, |b| b.open())
}

fn after_where(t: &Task, keep: impl Fn(&Blocker) -> bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for on in t
        .blockers
        .iter()
        .filter(|b| keep(b) && b.later.is_empty() && !b.legacy && b.options.is_empty())
        .filter_map(|b| b.on.as_ref())
        .filter(|on| on.kind == BLOCKED_ON_TASK && !on.reference.is_empty())
    {
        if !out.contains(&on.reference) {
            out.push(on.reference.clone());
        }
    }
    out
}

/// The raises folded into card t's item `bid`: another device may have
/// one of them as the item.
pub(crate) fn merged_into(t: &Task, bid: &str) -> Vec<String> {
    t.blockers
        .iter()
        .find(|b| b.id == bid)
        .map(|b| b.merged.clone())
        .unwrap_or_default()
}

/// The refusal for two items of one new card that read the same.
fn same_question(q: &str) -> Error {
    invalid(format!(
        "two of the new card's waits read the same ({}): say them apart",
        quote(q)
    ))
}

/// The refusal that names a loop: `#1 waits on #2, which waits on #1`. A
/// long one keeps its ends, so it fits what an agent is shown.
fn circle_of(names: &[String]) -> Error {
    let mut parts: Vec<String> = names.to_vec();
    if parts.len() > MAX_CIRCLE_NAMES {
        let cut = parts.len() - MAX_CIRCLE_NAMES + 1;
        let tail = parts.split_off(MAX_CIRCLE_NAMES / 2 + cut);
        parts.truncate(MAX_CIRCLE_NAMES / 2);
        parts.push(format!("{cut} more cards"));
        parts.extend(tail);
    }
    let mut s = format!("that makes a circle: {}", parts[0]);
    for (i, p) in parts.iter().enumerate().skip(1) {
        let verb = if i == 1 {
            " waits on "
        } else {
            ", which waits on "
        };
        s.push_str(verb);
        s.push_str(p);
    }
    invalid(s)
}
