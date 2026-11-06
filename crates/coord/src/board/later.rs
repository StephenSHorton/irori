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
use super::wait_chain::task_waits;
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

/// Whether open item b is the person's to see: a question, a done check,
/// an ownership change or a PR to merge.
fn for_person(b: &Blocker) -> bool {
    b.done_check
        || b.transfer
        || !b.options.is_empty()
        || b.merge_wait().is_some()
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

/// Whether two items are the same hold, one on a parent and one a subtask
/// took from it.
fn same_hold(a: &Blocker, b: &Blocker) -> bool {
    a.later == b.later && a.question == b.question
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
            ..Park::default()
        }
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

    /// Whether t's waits lead back to t: two devices can each write half a
    /// circle, and the fold takes both.
    pub(crate) fn order_loop(&self, t: &Task) -> bool {
        self.order_edges(t)
            .iter()
            .any(|n| self.order_reaches(n, &t.id))
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
        if held_by_someone(&kt) || kt.parked().is_some() || its_own {
            return Ok(());
        }
        self.write_park(by, k, &kt.note, &Park::of(&item))?;
        self.end_pending_spawns(by, k, HELD_SPAWN)
    }

    /// The sweep's part (finding 27): an old device's subtask of a held
    /// parent has no hold, so its origin would hand it out. This device
    /// puts the parent's hold on it, as ordinary rows, where that row
    /// counts. Only a subtask never held, with nobody on it: one a person
    /// freed stays free.
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
                    && k.blockers.iter().all(|b| b.later.is_empty())
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
        let bid = self.fresh_blocker_id(id);
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

    /// The subtasks a hold on t goes on. One that took t's `old` hold
    /// gets the new one; one with a hold of its own keeps it.
    fn kids_to_park(&self, t: &Task, old: Option<&Blocker>, back: &Back) -> Result<Vec<Task>> {
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
            let same = old.is_some_and(|o| k.blockers.iter().any(|b| b.open() && same_hold(b, o)));
            if !same {
                if k.parked().is_some() {
                    continue;
                }
                self.check_parkable(k)?;
                self.check_room(k)?;
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
        let old = t.parked().cloned();
        match &old {
            Some(o) if o.back() == Some(Back::Word) && !person => return Err(word_refusal(&t)),
            Some(_) => {}
            None => {
                self.check_parkable(&t)?;
                self.check_room(&t)?;
            }
        }
        let on_it = t.holders.iter().chain(&t.co_holders);
        if !person && on_it.clone().any(|k| *k != by) {
            return Err(not_allowed(format!(
                "{} is held: only its holder or a person may hold it back",
                t.short_ref()
            )));
        }
        let back = self.check_back(back, id)?;
        let item = self.park_item(&why, &back);
        if let Some(i) = open_with_question(&t, &item.question)
            && old.as_ref().is_none_or(|o| o.id != t.blockers[i].id)
        {
            return Err(invalid(format!(
                "{} already has {} with that question",
                t.short_ref(),
                t.blockers[i].id
            )));
        }
        let kids = self.kids_to_park(&t, old.as_ref(), &back)?;
        let told: Vec<String> = on_it
            .filter(|k| **k != by && !self.is_person(k) && *k != SWEEPER)
            .cloned()
            .collect();

        // A change closes the old item and raises a new one: a replace
        // crossing the old item's close would leave nothing open.
        if let Some(o) = &old {
            let answer = format!("held again: {}", self.back_line(&back));
            self.close_items(&by, id, std::slice::from_ref(&o.id), &answer)?;
            self.close_kid_parks(&by, &t, std::slice::from_ref(o), &answer)?;
        }
        self.write_park(&by, id, &t.note, &item)?;
        self.drop_holders(&by, &t)?;
        self.end_pending_spawns(&by, id, HELD_SPAWN)?;
        for k in kids {
            self.write_park(&by, &k.id, &k.note, &item)?;
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
        };
        self.close_items(by, &t.id, std::slice::from_ref(&b.id), "reworded")?;
        self.close_kid_parks(by, t, std::slice::from_ref(b), "reworded")?;
        let bid = self.write_park(by, &t.id, &t.note, &p)?;
        for k in &kids {
            self.write_park(by, &k.id, &k.note, &p)?;
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
            let at = self.stamp();
            self.write(
                by,
                kinds::BLOCKER_RESOLVED,
                &BlockerResolvedRow {
                    task_id: id.into(),
                    blocker_id: bid.clone(),
                    answer: answer.into(),
                    option: -1,
                    closed: true,
                    restored,
                    by: by.into(),
                    at,
                    ..BlockerResolvedRow::default()
                },
            )?;
        }
        Ok(())
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
    /// does. The whole list is checked before the first row; a card it
    /// already waits on is left as it is.
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
        let have = task_waits(&t);
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
            let k = self.task_ref_locked(r)?;
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
        for r in &ct.after {
            let k = self.task_ref_locked(r)?;
            if bb.is_some_and(|bb| bb.reference == k)
                || out.after.iter().any(|(_, on)| on.reference == k)
            {
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
