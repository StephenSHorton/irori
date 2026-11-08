//! Subtasks, one level deep, and pins (I:internal/coord/subtasks.go,
//! pins.go; coord.md §3.10).

use std::collections::{BTreeMap, HashMap};

use gocompat::quote::quote;
use store::{Event, kinds};

use super::error::{Error, Result, bad_state, invalid, not_allowed, not_found, subtask};
use super::fold::{Fold, ParentLink, PinRec};
use super::rows::*;
use super::short::short_num;
use super::tasks::clean_new;
use super::types::*;
use crate::names::trim_space;

/// One split caps this many subtasks.
const MAX_SUBTASKS: usize = 20;

/// The first open item on t that is a call for the person: unanswered,
/// and not a wait on other work (a card, a lease, an agent, something
/// outside) unless it offers choices.
fn first_call_item(t: &Task) -> Option<&Blocker> {
    t.blockers.iter().find(|b| {
        b.open()
            && b.status != BLOCKER_ANSWERED
            && (!b.options.is_empty()
                || b.on
                    .as_ref()
                    .is_none_or(|on| !super::offers::on_work(&on.kind)))
    })
}

/// Whether blocked card t waits on a call from the person: an open call,
/// or blocked with no item at all (its reason is the question).
fn asks_a_call(t: &Task) -> bool {
    first_call_item(t).is_some() || !t.blockers.iter().any(|b| b.open())
}

fn first_call(t: &Task) -> String {
    first_call_item(t)
        .map(|b| b.question.clone())
        .unwrap_or_else(|| t.reason.clone())
}

/// A short ref's number; a card without one sorts last.
fn short_int(short: &str) -> i64 {
    super::resource::go_atoi(short_num(short)).unwrap_or(1 << 30)
}

impl Fold {
    /// Records a link row.
    pub(crate) fn fold_parent(&mut self, child: &str, parent: &str, ev: &Event) {
        if child.is_empty() {
            return;
        }
        self.links_moved = true;
        self.links.push(ParentLink {
            child: child.into(),
            parent: parent.into(),
            at: RowPos::of(ev),
        });
    }

    /// Whether id is an ask still open at position `at`.
    fn ask_at(&self, id: &str, at: &RowPos) -> bool {
        let Some(a) = self.tasks.get(id).and_then(|t| t.ask.as_ref()) else {
            return false;
        };
        if a.is_open() {
            return true;
        }
        at.before(&self.asks.closed_at.get(id).cloned().unwrap_or_default())
    }

    /// Derives every card's Parent and Subtasks from the link rows, then
    /// each parent's Rollup.
    pub(crate) fn relink(&mut self) {
        self.links_moved = false;
        self.links.sort_by(|a, b| a.at.cmp(&b.at));
        let mut parent_of: HashMap<String, String> = HashMap::new();
        let mut kids: HashMap<String, i64> = HashMap::new();
        for l in &self.links {
            let p = &l.parent;
            if !p.is_empty()
                && (*p == l.child
                    || !self.tasks.contains_key(&l.child)
                    || !self.tasks.contains_key(p)
                    || parent_of.get(p).is_some_and(|x| !x.is_empty())
                    || kids.get(&l.child).copied().unwrap_or(0) > 0
                    || self.ask_at(&l.child, &l.at)
                    || self.ask_at(p, &l.at))
            {
                continue;
            }
            if let Some(old) = parent_of.remove(&l.child) {
                *kids.entry(old).or_default() -= 1;
            }
            if !p.is_empty() {
                parent_of.insert(l.child.clone(), p.clone());
                *kids.entry(p.clone()).or_default() += 1;
            }
        }
        let mut subs: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (child, p) in &parent_of {
            subs.entry(p.clone()).or_default().push(child.clone());
        }
        let ids: Vec<String> = self.tasks.keys().cloned().collect();
        for id in ids {
            let mut ks = subs.remove(&id).unwrap_or_default();
            ks.sort_by(|a, b| {
                let (sa, sb) = (&self.tasks[a].short, &self.tasks[b].short);
                short_int(sa)
                    .cmp(&short_int(sb))
                    .then_with(|| sa.cmp(sb))
                    .then_with(|| a.cmp(b))
            });
            let rollup = if ks.is_empty() {
                None
            } else {
                Some(self.rollup(&self.tasks[&id], &ks))
            };
            let new_parent = parent_of.get(&id).cloned().unwrap_or_default();
            // A subtask runs on its parent's side (card #494), also when
            // it was adopted or an ask was made part of a card; out of its
            // parent it has its own again.
            let side = match self.tasks.get(&new_parent) {
                Some(p) if !new_parent.is_empty() => p.own_side.clone(),
                _ => self.tasks[&id].own_side.clone(),
            };
            let t = self.tasks.get_mut(&id).expect("listed");
            let changed =
                t.parent != new_parent || t.subtasks != ks || t.rollup != rollup || t.side != side;
            t.side = side;
            super::offers::index_side(&self.stale, t);
            t.parent = new_parent;
            t.subtasks = ks;
            t.rollup = rollup;
            if !self.replaying && changed {
                self.note(Change {
                    kind: TASK_CHANGED.into(),
                    task_id: id,
                    ..Change::default()
                });
            }
        }
    }

    /// A parent's progress and state from its subtasks.
    fn rollup(&self, t: &Task, ks: &[String]) -> Rollup {
        let mut r = Rollup::default();
        let (mut ended, mut completed, mut failed) = (0, 0, 0);
        let (mut blocked, mut input, mut working, mut started) = (false, false, false, false);
        let (mut reviewing, mut not_reviewing) = (0, 0);
        for id in ks {
            let k = &self.tasks[id];
            match k.state.as_str() {
                COMPLETED => {
                    r.done += 1;
                    completed += 1;
                    ended += 1;
                }
                CANCELED => {
                    ended += 1;
                    continue;
                }
                FAILED => {
                    failed += 1;
                    ended += 1;
                }
                // Held as Not yet, it needs no call (card #570).
                BLOCKED if super::later::only_held(k) => {}
                BLOCKED => {
                    blocked = true;
                    // The part the parent's "needs a call" names: one with
                    // a question for the person, not one waiting on work,
                    // such as a helper's part on its lead (card #557).
                    if r.blocked.is_empty() && asks_a_call(k) {
                        r.blocked = id.clone();
                        r.blocked_question = first_call(k);
                    }
                }
                INPUT_REQUIRED => input = true,
                WORKING => working = true,
                _ => {}
            }
            // Card #191: the parent is in review when every open subtask is.
            if k.state == IN_REVIEW {
                reviewing += 1;
            } else if !is_terminal(&k.state) {
                not_reviewing += 1;
            }
            if k.state != SUBMITTED && !super::later::only_held(k) {
                started = true;
            }
            r.total += 1;
        }
        let all = ended == ks.len();
        // The parent's own wait comes before "every part is done" (card
        // #557): a lead's "Can I merge?" keeps the card open, listed and
        // before the person until it is answered and acted on, and keeps
        // it where a fresh lead can be started and bound for that answer.
        let own_wait = t.state == BLOCKED || t.state == INPUT_REQUIRED;
        r.state = if is_terminal(&t.state) || (all && own_wait) {
            t.state.clone()
        } else if all && completed > 0 {
            COMPLETED.into()
        } else if all && failed > 0 {
            FAILED.into()
        } else if all {
            CANCELED.into()
        } else if t.state == BLOCKED || blocked {
            if t.state == BLOCKED {
                r.blocked.clear();
                r.blocked_question.clear();
            }
            BLOCKED.into()
        } else if input || t.state == INPUT_REQUIRED {
            INPUT_REQUIRED.into()
        } else if reviewing > 0 && not_reviewing == 0 {
            IN_REVIEW.into()
        } else if working || started {
            WORKING.into()
        } else {
            SUBMITTED.into()
        };
        r
    }

    /// Whether `parent` may take a new or adopted subtask `child` (`""`
    /// for a card not made yet).
    pub(crate) fn check_parent(&self, parent_id: &str, child: &str) -> Result<()> {
        let Some(p) = self.tasks.get(parent_id) else {
            return Err(not_found(format!("parent {}", quote(parent_id))));
        };
        if parent_id == child {
            return Err(subtask("a card can't be its own subtask"));
        }
        if p.open_ask().is_some() {
            return Err(subtask(format!(
                "{} is an open ask; ask_claim it and ask_close it as a card first",
                p.short_ref()
            )));
        }
        if !p.parent.is_empty() {
            let pr = self.ref_of(&p.parent);
            return Err(subtask(format!(
                "{} is itself a subtask of {pr}, and subtasks are one level; add a sibling with board_create parent={pr}",
                p.short_ref()
            )));
        }
        if is_terminal(&p.state) {
            return Err(bad_state(format!("{} is {}", p.short_ref(), p.state)));
        }
        if child.is_empty() {
            return Ok(());
        }
        let Some(k) = self.tasks.get(child) else {
            return Err(not_found(format!("task {}", quote(child))));
        };
        if k.open_ask().is_some() {
            return Err(subtask(format!(
                "{} is an open ask; ask_close it as a card (with parent) instead",
                k.short_ref()
            )));
        }
        if is_terminal(&k.state) {
            return Err(bad_state(format!(
                "{} is {}; only cards still in play can be adopted",
                k.short_ref(),
                k.state
            )));
        }
        if !k.subtasks.is_empty() {
            return Err(subtask(format!(
                "{} has subtasks of its own, and subtasks are one level",
                k.short_ref()
            )));
        }
        if k.parent == parent_id {
            return Err(invalid(format!(
                "{} is already a subtask of {}",
                k.short_ref(),
                p.short_ref()
            )));
        }
        Ok(())
    }

    /// id's short ref, or its task: ref.
    pub(crate) fn ref_of(&self, id: &str) -> String {
        self.short_of(id)
    }

    /// set_task_state's rule for a parent; Ok(true) cascades a cancel.
    pub(crate) fn parent_state_refusal(&self, t: &Task, s: &str) -> Result<bool> {
        if t.subtasks.is_empty() {
            return Ok(false);
        }
        match s {
            CANCELED => Ok(true),
            COMPLETED => {
                let open: Vec<String> = t
                    .subtasks
                    .iter()
                    .filter_map(|id| self.tasks.get(id))
                    .filter(|k| !is_terminal(&k.state))
                    .map(|k| format!("{} ({})", k.short_ref(), k.state))
                    .collect();
                if !open.is_empty() {
                    return Err(subtask(format!(
                        "{} still has open subtasks: {}. Finish or cancel them; the parent completes with them",
                        t.short_ref(),
                        open.join(", ")
                    )));
                }
                Ok(false)
            }
            _ => Err(subtask(format!(
                "{} is a parent: its state follows its subtasks (only canceled or completed can be set)",
                t.short_ref()
            ))),
        }
    }

    /// Turns card id into a parent.
    pub(crate) fn split(
        &mut self,
        by: &str,
        id: &str,
        subs: Vec<NewSubtask>,
        adopt: Vec<String>,
    ) -> Result<(Task, Vec<Task>)> {
        let by = super::util::caller(by)?;
        if subs.is_empty() && adopt.is_empty() {
            return Err(invalid("give subtasks or cards to adopt"));
        }
        if subs.len() + adopt.len() > MAX_SUBTASKS {
            return Err(invalid(format!(
                "at most {MAX_SUBTASKS} subtasks in one split"
            )));
        }
        let t = self.open_task(id)?.clone();
        self.check_parent(id, "")?;
        // A closed board takes no card, adopted ones included (card #690).
        if self.board_closed(&t.board) {
            return Err(self.closed_refusal(&t.board));
        }
        let may_spawn = self.may_spawn_by(&t, &by);
        let mut placed: Vec<String> = Vec::new();
        let mut made = Vec::with_capacity(subs.len());
        let mut marks = Vec::with_capacity(subs.len());
        for s in &subs {
            // A helper's part (spawned agents §3): asked for here, or a
            // marked step moving to it.
            let mut mark = s.spawn && may_spawn;
            let mut nt = NewTask {
                title: s.title.clone(),
                objective: s.objective.clone(),
                done_criteria: s.done_criteria.clone(),
                weight: s.weight.clone(),
                weight_reason: s.weight_reason.clone(),
                ..NewTask::default()
            };
            for raw in &s.steps {
                let txt = trim_space(raw).to_string();
                if let Some(old) = t.steps.iter().find(|o| o.id == txt) {
                    if old.done {
                        return Err(invalid(format!(
                            "step {txt} is done; done steps stay on the parent"
                        )));
                    }
                    if placed.contains(&txt) {
                        return Err(invalid(format!("step {txt} is given to two subtasks")));
                    }
                    placed.push(txt.clone());
                    mark |= old.spawn;
                    nt.steps.push(Step {
                        text: old.text.clone(),
                        doing: old.doing,
                        ..Step::default()
                    });
                    continue;
                }
                nt.steps.push(Step {
                    text: txt,
                    ..Step::default()
                });
            }
            made.push(nt);
            marks.push(mark);
        }
        let unplaced: Vec<String> = t
            .steps
            .iter()
            .filter(|s| !s.done && !placed.contains(&s.id))
            .map(|s| format!("{} {}", s.id, quote(&s.text)))
            .collect();
        if !unplaced.is_empty() {
            return Err(invalid(format!(
                "every open step must go to a subtask; not placed: {}",
                unplaced.join(", ")
            )));
        }
        let mut cleaned = Vec::with_capacity(made.len());
        for (i, nt) in made.iter().enumerate() {
            let mut ct =
                clean_new(&by, nt).map_err(|e: Error| e.prefixed(format!("subtask {}", i + 1)))?;
            // Every subtask's weight is checked before the first is made.
            self.check_new_weight(&by, &ct.weight, &ct.weight_reason)
                .map_err(|e: Error| e.prefixed(format!("subtask {}", i + 1)))?;
            ct.parent = id.into();
            ct.spawn = marks[i];
            // A part runs on its parent's side (card #494).
            ct.side = t.side.clone();
            cleaned.push(ct);
        }
        let mut adopt_ids: Vec<String> = Vec::new();
        for a in &adopt {
            let k = self.task_ref_locked(a)?;
            self.check_parent(id, &k)?;
            self.check_adopt(id, &k)?;
            // Adopting moves a card to its parent's side: an agent does it
            // only to a card it may take (card #496).
            if let Some(kt) = self.tasks.get(&k).cloned() {
                self.check_side_take(&by, &kt)?;
            }
            if !adopt_ids.contains(&k) {
                adopt_ids.push(k);
            }
        }
        let mut out = Vec::new();
        for ct in cleaned {
            out.push(self.create_locked(&by, ct)?);
        }
        for k in adopt_ids {
            let on = self.tasks.get(&k).map(|x| x.board.clone());
            let at = self.stamp();
            self.write(
                &by,
                kinds::TASK_PARENT,
                &TaskParentRow {
                    task_id: k.clone(),
                    parent: id.into(),
                    by: by.clone(),
                    at,
                },
            )?;
            // A subtask sits on its parent's board (card #563).
            if on.is_some_and(|b| b != t.board) {
                let (num, at) = (self.next_short(&t.board), self.stamp());
                self.write(
                    &by,
                    self.ask_kind(kinds::TASK_EDITED),
                    &TaskEditedRow {
                        task_id: k.clone(),
                        board: Some(t.board.clone()),
                        num,
                        by: by.clone(),
                        at,
                        ..TaskEditedRow::default()
                    },
                )?;
            }
            self.inherit_park(&by, id, &k)?;
        }
        if !placed.is_empty() {
            let keep: Vec<Step> = t.steps.iter().filter(|s| s.done).cloned().collect();
            let at = self.stamp();
            self.write(
                &by,
                kinds::TASK_STEPS,
                &TaskStepsRow {
                    task_id: id.into(),
                    steps: keep,
                    by: by.clone(),
                    at,
                },
            )?;
        }
        Ok((self.tasks[id].clone(), out))
    }

    // ---- pins ----

    /// A task.pinned row: only the person's, only after the card's current
    /// pin row.
    pub(crate) fn fold_pin(&mut self, ev: &Event) {
        let Some(p) = row::<TaskPinnedRow>(&ev.payload) else {
            return;
        };
        if p.task_id.is_empty() {
            return;
        }
        if ev.actor != "user" || !self.is_owner(&p.by) {
            self.ignore(&ev.device_id);
            return;
        }
        let at = RowPos::of(ev);
        if let Some(old) = self.pins.get(&p.task_id)
            && !old.at.before(&at)
        {
            return;
        }
        self.pins.insert(
            p.task_id.clone(),
            PinRec {
                pinned: p.pinned,
                rank: p.rank,
                at,
            },
        );
        if let Some(mut t) = self.tasks.remove(&p.task_id) {
            self.sync_pin(&mut t);
            self.tasks.insert(p.task_id.clone(), t);
            self.note(Change {
                kind: TASK_CHANGED.into(),
                task_id: p.task_id,
                ..Change::default()
            });
        } else if p.task_id.starts_with(BOARD_PIN_PREFIX) {
            // A shared board's pin (card #135) is no card: say so, or a pin
            // from another of the person's devices never reaches the
            // sidebar until a restart (card #233).
            self.note(Change {
                kind: BOARD_PIN_CHANGED.into(),
                task_id: p.task_id,
                ..Change::default()
            });
        }
    }

    /// Folds task.archived (card #138): the person's only; the newest row
    /// for a card wins. The id needn't be a card here: a shared board's
    /// card is cleared through the private log.
    pub(crate) fn fold_archived(&mut self, ev: &Event) {
        let Some(p) = row::<TaskArchivedRow>(&ev.payload) else {
            return;
        };
        if p.task_id.is_empty() {
            return;
        }
        if ev.actor != "user" || !self.is_owner(&p.by) {
            self.ignore(&ev.device_id);
            return;
        }
        let at = RowPos::of(ev);
        if let Some((_, old)) = self.archived.get(&p.task_id)
            && !old.before(&at)
        {
            return;
        }
        self.archived.insert(p.task_id.clone(), (p.archived, at));
        if self.tasks.contains_key(&p.task_id) {
            self.note(Change {
                kind: TASK_CHANGED.into(),
                task_id: p.task_id,
                ..Change::default()
            });
        }
    }

    /// The person clears done cards from the window, or brings them back
    /// (card #138). A card of this log must be closed to be cleared; an id
    /// this log doesn't hold is a shared board's card, which the caller
    /// checked. Returns how many rows it wrote.
    pub(crate) fn archive(&mut self, by: &str, ids: &[String], archived: bool) -> Result<usize> {
        let by = trim_space(by).to_string();
        if !self.is_owner(&by) {
            return Err(not_allowed("only the person clears cards"));
        }
        if self.on_board {
            return Err(invalid("cards are cleared in the private log"));
        }
        for id in ids {
            if id.is_empty() {
                return Err(invalid("an empty card id"));
            }
            if archived
                && let Some(t) = self.tasks.get(id)
                && !is_terminal(effective_state(t))
            {
                return Err(invalid(format!(
                    "{} isn't closed: only done cards are cleared",
                    t.short_ref()
                )));
            }
        }
        let mut n = 0;
        for id in ids {
            if self.archived.get(id).map(|(a, _)| *a).unwrap_or(false) == archived {
                continue;
            }
            let at = self.stamp();
            self.write(
                &by,
                kinds::TASK_ARCHIVED,
                &TaskArchivedRow {
                    task_id: id.clone(),
                    archived,
                    by: by.clone(),
                    at,
                },
            )?;
            n += 1;
        }
        Ok(n)
    }

    /// Folds task.paused (card #207): the person's only; the newest row for
    /// a card wins.
    pub(crate) fn fold_paused(&mut self, ev: &Event) {
        let Some(p) = row::<TaskPausedRow>(&ev.payload) else {
            return;
        };
        if p.task_id.is_empty() {
            return;
        }
        if ev.actor != "user" || !self.is_owner(&p.by) {
            self.ignore(&ev.device_id);
            return;
        }
        let at = RowPos::of(ev);
        if let Some((_, old)) = self.paused.get(&p.task_id)
            && !old.before(&at)
        {
            return;
        }
        self.paused.insert(p.task_id.clone(), (p.paused, at));
        if let Some(t) = self.tasks.get_mut(&p.task_id) {
            t.paused = p.paused;
        }
        if self.tasks.contains_key(&p.task_id) {
            self.note(Change {
                kind: TASK_CHANGED.into(),
                task_id: p.task_id,
                ..Change::default()
            });
        }
    }

    /// The person pauses a card, or resumes it (card #207). Only this
    /// log's open cards; a shared board's card isn't paused (its holders
    /// may be another person's). Returns the card, and whether the row was
    /// written (false: it already was so).
    pub(crate) fn pause(
        &mut self,
        by: &str,
        id: &str,
        paused: bool,
        note: &str,
    ) -> Result<(Task, bool)> {
        let by = trim_space(by).to_string();
        if !self.is_owner(&by) {
            return Err(not_allowed("only the person pauses a card"));
        }
        if self.on_board {
            return Err(invalid("a shared space's card can't be paused"));
        }
        let Some(t) = self.tasks.get(id).cloned() else {
            return Err(not_found(format!("task {}", quote(id))));
        };
        if paused && is_terminal(effective_state(&t)) {
            return Err(invalid(format!(
                "{} is closed: nothing to pause",
                t.short_ref()
            )));
        }
        if self.is_paused(id) == paused {
            return Ok((t, false));
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_PAUSED,
            &TaskPausedRow {
                task_id: id.to_string(),
                paused,
                note: trim_space(note).to_string(),
                by: by.clone(),
                at,
            },
        )?;
        Ok((t, true))
    }

    /// Whether the person paused the card (card #207).
    pub(crate) fn is_paused(&self, id: &str) -> bool {
        self.paused.get(id).map(|(p, _)| *p).unwrap_or(false)
    }

    /// The ids the person paused (card #207).
    pub(crate) fn paused_ids(&self) -> std::collections::HashSet<String> {
        self.paused
            .iter()
            .filter(|(_, (p, _))| *p)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// The ids the person cleared (card #138).
    pub(crate) fn archived_ids(&self) -> std::collections::HashSet<String> {
        self.archived
            .iter()
            .filter(|(_, (a, _))| *a)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Folds task.deleted (card #57): the person's only. The card leaves
    /// the fold for good, whatever arrives about it later, and its
    /// subtasks stand alone (relink drops the link to a missing parent).
    pub(crate) fn fold_deleted(&mut self, ev: &Event) {
        let Some(p) = row::<TaskDeletedRow>(&ev.payload) else {
            return;
        };
        if p.task_id.is_empty() {
            return;
        }
        if ev.actor != "user" || !self.is_owner(&p.by) {
            self.ignore(&ev.device_id);
            return;
        }
        self.deleted.insert(p.task_id.clone());
        self.placed.remove(&p.task_id);
        self.links_moved = true;
        if self.tasks.remove(&p.task_id).is_some() {
            self.order.remove(&p.task_id);
            self.pins.remove(&p.task_id);
            self.note(Change {
                kind: TASK_CHANGED.into(),
                task_id: p.task_id,
                ..Change::default()
            });
        }
    }

    /// The person deletes a card: it leaves the Board on every device, and
    /// the signed log keeps it. Only the person; any card, in any state.
    pub(crate) fn delete_task(&mut self, by: &str, id: &str) -> Result<Task> {
        let by = trim_space(by).to_string();
        if !self.is_owner(&by) {
            return Err(not_allowed("only the person deletes cards"));
        }
        if self.on_board {
            return Err(invalid(
                "a card in a shared space is withdrawn, not deleted",
            ));
        }
        let Some(t) = self.tasks.get(id).cloned() else {
            return Err(not_found(format!("task {}", quote(id))));
        };
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_DELETED,
            &TaskDeletedRow {
                task_id: id.into(),
                by: by.clone(),
                at,
            },
        )?;
        Ok(t)
    }

    /// Copies the card's pin onto it.
    pub(crate) fn sync_pin(&self, t: &mut Task) {
        let r = self.pins.get(&t.id).cloned().unwrap_or_default();
        t.pinned = r.pinned;
        t.pin_rank = if r.pinned { r.rank } else { 0.0 };
        t.paused = self.is_paused(&t.id);
        t.weight = self.weight_of(&t.id);
        if let Some(s) = self.weights.get(&t.id) {
            s.sync_marks(t);
        }
    }

    pub(crate) fn pin(
        &mut self,
        by: &str,
        id: &str,
        pinned: bool,
        before: &str,
        mv: bool,
    ) -> Result<Task> {
        let by = trim_space(by).to_string();
        if !self.is_owner(&by) {
            return Err(not_allowed("only the person pins cards"));
        }
        let Some(t) = self.tasks.get(id) else {
            return Err(not_found(format!("task {}", quote(id))));
        };
        if !mv && t.pinned == pinned {
            return Ok(t.clone());
        }
        let rank = if pinned {
            self.pin_rank(id, before, mv)?
        } else {
            0.0
        };
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_PINNED,
            &TaskPinnedRow {
                task_id: id.into(),
                pinned,
                rank,
                by: by.clone(),
                at,
            },
        )?;
        Ok(self.tasks[id].clone())
    }

    /// Pins a card this log doesn't hold (card #161): a shared board's
    /// card, pinned by the person in their own log, so it orders with
    /// their private pins and follows them to their devices, and no other
    /// member sees it. Only the private log takes these.
    pub(crate) fn pin_ref(
        &mut self,
        by: &str,
        id: &str,
        pinned: bool,
        before: &str,
        mv: bool,
    ) -> Result<()> {
        let by = trim_space(by).to_string();
        if !self.is_owner(&by) {
            return Err(not_allowed("only the person pins cards"));
        }
        if self.on_board {
            return Err(invalid("pins live in the person's own log"));
        }
        if trim_space(id).is_empty() {
            return Err(invalid("pin which card?"));
        }
        let now = self.pins.get(id).is_some_and(|r| r.pinned);
        if !mv && now == pinned {
            return Ok(());
        }
        let rank = if pinned {
            self.pin_rank(id, before, mv)?
        } else {
            0.0
        };
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_PINNED,
            &TaskPinnedRow {
                task_id: id.into(),
                pinned,
                rank,
                by: by.clone(),
                at,
            },
        )
    }

    /// Every pinned card id with its rank, held cards or not.
    pub(crate) fn pinned_ranks(&self) -> HashMap<String, f64> {
        self.pins
            .iter()
            .filter(|(_, r)| r.pinned)
            .map(|(id, r)| (id.clone(), r.rank))
            .collect()
    }

    /// Where id goes among the pinned cards: last, or just before `before`.
    fn pin_rank(&self, id: &str, before: &str, mv: bool) -> Result<f64> {
        let mut list: Vec<(&String, f64)> = self
            .pins
            .iter()
            .filter(|(tid, r)| r.pinned && tid.as_str() != id)
            .map(|(tid, r)| (tid, r.rank))
            .collect();
        list.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(b.0)));
        if !mv || before.is_empty() {
            return Ok(list.last().map_or(1.0, |l| l.1 + 1.0));
        }
        let Some(i) = list.iter().position(|p| p.0 == before) else {
            return Err(invalid(format!("{before} isn't pinned")));
        };
        if i == 0 {
            return Ok(list[0].1 - 1.0);
        }
        Ok((list[i - 1].1 + list[i].1) / 2.0)
    }
}
