//! A card's weight (card #963): light, normal or heavy, with one line on
//! why and who set it. Irori starts the card's agent on the model and
//! effort the board maps the weight to (Settings › Agents); no weight is
//! normal.
//!
//! Anyone working the board may weigh a card: the person, an agent, or
//! Irori's office assistant (it writes as Irori). A weight the person set
//! is theirs: only a person changes or clears it. Who set it is never read
//! from the row; the fold works it out from the writer.
//!
//! Rows can fold out of log order (a board's rows come device by device),
//! so the fold keeps two slots per card: the newest row a person wrote and
//! the newest anyone else wrote. The weight is the person's when they set
//! one; else the other's, if it came after the person's last clear. That
//! gives one answer in any order.

use gocompat::quote::quote;
use store::{Event, kinds};

use super::error::{Result, invalid, not_allowed};
use super::fold::{Fold, Touch};
use super::rows::*;
use super::types::*;
use super::util::{SWEEPER, caller, check_key};
use crate::names::trim_space;

/// The longest reason, in characters.
pub const MAX_WEIGHT_REASON: usize = 200;

/// The newest weight row of each kind of writer for one card.
#[derive(Debug, Clone, Default)]
pub(crate) struct WeightSlots {
    /// The newest row a person wrote: a weight, or a clear (level "").
    person: Option<(Weight, RowPos)>,
    /// The newest row an agent or the assistant wrote.
    other: Option<(Weight, RowPos)>,
}

impl WeightSlots {
    fn weight(&self) -> Option<Weight> {
        let set = |w: &Weight| (!w.level.is_empty()).then(|| w.clone());
        match (&self.person, &self.other) {
            (Some((p, _)), _) if !p.level.is_empty() => Some(p.clone()),
            (Some((_, pat)), Some((o, oat))) => {
                if pat.before(oat) {
                    set(o)
                } else {
                    None
                }
            }
            (None, Some((o, _))) => set(o),
            _ => None,
        }
    }

    fn put(slot: &mut Option<(Weight, RowPos)>, w: Weight, at: RowPos) -> bool {
        if let Some((_, old)) = slot
            && !old.before(&at)
        {
            return false;
        }
        *slot = Some((w, at));
        true
    }
}

/// Checks a weight level: one of [`WEIGHTS`], or "" to clear.
pub fn check_weight(level: &str) -> Result<()> {
    if level.is_empty() || WEIGHTS.contains(&level) {
        return Ok(());
    }
    Err(invalid(format!(
        "unknown weight {}: pick one of {}",
        quote(level),
        WEIGHTS.join(", ")
    )))
}

fn clean_reason(reason: &str) -> Result<String> {
    let r = super::util::collapse_space(trim_space(reason));
    if r.chars().count() > MAX_WEIGHT_REASON {
        return Err(invalid(format!(
            "a weight's reason is one line of at most {MAX_WEIGHT_REASON} characters"
        )));
    }
    Ok(r)
}

/// A new card's weight (card #964): its level, lowercased and checked,
/// and its reason, one line. ("", "") is none.
pub(crate) fn clean_new_weight(level: &str, reason: &str) -> Result<(String, String)> {
    let level = trim_space(level).to_lowercase();
    check_weight(&level)?;
    if level.is_empty() {
        return Ok((String::new(), String::new()));
    }
    Ok((level, clean_reason(reason)?))
}

impl Fold {
    /// Checks a new card's weight against who makes it (card #964): an
    /// agent gives one line on why, as with [`Self::set_weight`].
    pub(crate) fn check_new_weight(&self, by: &str, level: &str, reason: &str) -> Result<()> {
        if !level.is_empty() && reason.is_empty() && !self.is_person(by) {
            return Err(invalid("give a weight with one line on why"));
        }
        Ok(())
    }

    /// Writes a new card's weight row (card #964), right after its
    /// task.created, so it is weighed before Irori hands it out.
    pub(crate) fn write_new_weight(
        &mut self,
        by: &str,
        id: &str,
        level: &str,
        reason: &str,
    ) -> Result<()> {
        if level.is_empty() {
            return Ok(());
        }
        let at = self.stamp();
        self.write(
            by,
            kinds::TASK_WEIGHT,
            &TaskWeightRow {
                task_id: id.to_string(),
                level: level.to_string(),
                reason: reason.to_string(),
                by: by.to_string(),
                at,
                ..TaskWeightRow::default()
            },
        )
    }

    /// Weighs card `id` (card #963): `level` is light, normal or heavy, or
    /// "" to clear it. A person may always; an agent or Irori's office
    /// assistant only while the person hasn't set it, and with a reason.
    /// Returns the card and whether a row was written (false: it already
    /// was so).
    pub(crate) fn set_weight(
        &mut self,
        by: &str,
        id: &str,
        level: &str,
        reason: &str,
    ) -> Result<(Task, bool)> {
        let by = caller(by)?;
        let person = self.is_person(&by);
        if !person && by != SWEEPER {
            check_key(&by)?;
        }
        let level = trim_space(level).to_lowercase();
        check_weight(&level)?;
        let reason = clean_reason(reason)?;
        let t = self.any_task(id)?.clone();
        let now = self.weight_of(id);
        if !person {
            if let Some(w) = now.as_ref().filter(|w| w.by_person()) {
                return Err(not_allowed(format!(
                    "{}'s weight ({}) was set by the person: only they change it",
                    t.short_ref(),
                    w.level
                )));
            }
            if !level.is_empty() && reason.is_empty() {
                return Err(invalid("give a weight with one line on why"));
            }
        }
        let same = match &now {
            // The person's write over Irori's weight always counts, even
            // word for word: it makes the weight theirs (Undo, card #965).
            Some(w) => w.level == level && w.reason == reason && !(person && w.by_assistant()),
            None => level.is_empty(),
        };
        if same {
            return Ok((t, false));
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_WEIGHT,
            &TaskWeightRow {
                task_id: id.to_string(),
                level,
                reason,
                by: by.clone(),
                at,
                ..TaskWeightRow::default()
            },
        )?;
        let mut t = t;
        t.weight = self.weight_of(id);
        Ok((t, true))
    }

    /// The card's holder moves it one weight up or down (card #966): its
    /// left_off, the new weight, and it leaves, so Irori ends it and starts
    /// a fresh agent from Left off on the new weight. It moves from the
    /// level this attempt runs at, and never back the way it last moved,
    /// so a card can't bounce.
    pub(crate) fn move_weight(
        &mut self,
        by: &str,
        id: &str,
        up: bool,
        reason: &str,
        left_off: &str,
    ) -> Result<Task> {
        let by = caller(by)?;
        check_key(&by)?;
        let reason = clean_reason(reason)?;
        if reason.is_empty() {
            return Err(invalid("say in one line why it moves"));
        }
        if trim_space(left_off).is_empty() {
            return Err(invalid(
                "give left_off: what's done and what's next, for the agent that carries on",
            ));
        }
        let t = self.open_task(id)?.clone();
        if !super::fold::is_holder(&t, &by) {
            return Err(not_allowed(format!(
                "only an agent working {} moves it",
                t.short_ref()
            )));
        }
        let now = self.weight_of(id);
        if let Some(w) = now.as_ref().filter(|w| w.by_person()) {
            return Err(not_allowed(format!(
                "{}'s weight ({}) was set by the person: say so on the card and leave it to them",
                t.short_ref(),
                w.level
            )));
        }
        let (dir, back) = if up { ("up", "down") } else { ("down", "up") };
        if now.as_ref().is_some_and(|w| w.moved == back) {
            return Err(not_allowed(format!(
                "{} was moved {back} already and doesn't move back: carry on, or say why on the card",
                t.short_ref()
            )));
        }
        let at_level = WEIGHTS
            .iter()
            .position(|w| *w == t.run_level())
            .unwrap_or(1);
        let next = if up {
            WEIGHTS.get(at_level + 1)
        } else {
            at_level.checked_sub(1).and_then(|i| WEIGHTS.get(i))
        };
        let Some(level) = next else {
            return Err(invalid(format!(
                "{} is already {}: it can't move {dir}",
                t.short_ref(),
                t.run_level()
            )));
        };
        let Some(step) = t
            .steps
            .iter()
            .find(|s| s.doing)
            .or_else(|| t.steps.iter().find(|s| !s.done))
            .or(t.steps.last())
            .cloned()
        else {
            return Err(invalid(format!(
                "{} has no steps to hold left_off: give it steps (board_steps) first",
                t.short_ref()
            )));
        };
        let keep = StepUpdate {
            done: None,
            doing: Some(step.doing),
        };
        self.set_step(&by, id, &step.id, keep, left_off)?;
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_WEIGHT,
            &TaskWeightRow {
                task_id: id.to_string(),
                level: (*level).to_string(),
                reason,
                by: by.clone(),
                at,
                moved: dir.to_string(),
                ..TaskWeightRow::default()
            },
        )?;
        self.leave(&by, id, &by)
    }

    /// Irori's office assistant weighs card `id` (card #965), remembering
    /// what it replaces for the person's Undo. Never a weight the person
    /// set, and never twice: a weight it already gave stays.
    pub(crate) fn weigh_as_irori(&mut self, id: &str, level: &str, reason: &str) -> Result<Task> {
        let level = trim_space(level).to_lowercase();
        check_weight(&level)?;
        if level.is_empty() {
            return Err(invalid("Irori weighs a card light, normal or heavy"));
        }
        let reason = clean_reason(reason)?;
        if reason.is_empty() {
            return Err(invalid("give a weight with one line on why"));
        }
        let t = self.open_task(id)?.clone();
        let now = self.weight_of(id);
        // Its own row outlives the person's Undo, so once holds on any
        // device.
        let weighed = self
            .weights
            .get(id)
            .and_then(|s| s.other.as_ref())
            .is_some_and(|(w, _)| w.by_assistant());
        if weighed {
            return Err(not_allowed(format!(
                "{}: Irori weighs a card once",
                t.short_ref()
            )));
        }
        if let Some(w) = &now
            && w.by_person()
        {
            return Err(not_allowed(format!(
                "{}'s weight ({}) is the person's: Irori leaves it",
                t.short_ref(),
                w.level
            )));
        }
        let (was_level, was_reason) = now.map(|w| (w.level, w.reason)).unwrap_or_default();
        let at = self.stamp();
        self.write(
            SWEEPER,
            kinds::TASK_WEIGHT,
            &TaskWeightRow {
                task_id: id.to_string(),
                level,
                reason,
                by: SWEEPER.to_string(),
                at,
                was_level,
                was_reason,
                ..TaskWeightRow::default()
            },
        )?;
        let mut t = t;
        t.weight = self.weight_of(id);
        Ok(t)
    }

    /// The card's weight; None is normal.
    pub(crate) fn weight_of(&self, id: &str) -> Option<Weight> {
        self.weights.get(id).and_then(WeightSlots::weight)
    }

    /// Who wrote a weight row: a person, the assistant (Irori itself), or
    /// an agent writing as itself. None: nobody who may weigh a card.
    fn weigher(&self, ev: &Event, by: &str) -> Option<&'static str> {
        if self.person_row(ev, by) {
            return Some(SET_BY_PERSON);
        }
        if by == SWEEPER && ev.actor == SWEEPER {
            return Some(SET_BY_ASSISTANT);
        }
        if self.agent_own_row(ev, by) {
            return Some(SET_BY_AGENT);
        }
        None
    }

    /// Folds task.weight (card #963).
    pub(crate) fn fold_weight(&mut self, ev: &Event) {
        let Some(p) = row::<TaskWeightRow>(&ev.payload) else {
            return;
        };
        if p.task_id.is_empty() || self.deleted.contains(&p.task_id) {
            return;
        }
        if check_weight(&p.level).is_err() {
            self.ignore(&ev.device_id);
            return;
        }
        let Some(set_by) = self.weigher(ev, &p.by) else {
            self.ignore(&ev.device_id);
            return;
        };
        let moved = p.moved.clone();
        let line =
            (!p.level.is_empty() && (set_by == SET_BY_ASSISTANT || !moved.is_empty())).then(|| {
                if !moved.is_empty() {
                    format!("moved it {moved} to {}: {}", p.level, p.reason)
                } else if !p.was_level.is_empty() && p.was_level != p.level {
                    format!(
                        "changed the weight from {} to {}: {}",
                        p.was_level, p.level, p.reason
                    )
                } else {
                    format!("weighed it {}: {}", p.level, p.reason)
                }
            });
        let (by, when) = (p.by.clone(), p.at);
        let w = Weight {
            level: p.level,
            reason: p.reason,
            set_by: set_by.into(),
            by: p.by,
            at: p.at,
            was_level: p.was_level,
            was_reason: p.was_reason,
            moved: p.moved,
        };
        if let Some(t) = self.tasks.get_mut(&p.task_id) {
            let nanos = when.unix_nano();
            t.weighed_at = t.weighed_at.max(nanos);
            if !moved.is_empty() {
                t.note_retry(RETRY_MOVED, &nanos.to_string(), nanos);
            }
        }
        let slots = self.weights.entry(p.task_id.clone()).or_default();
        let slot = if set_by == SET_BY_PERSON {
            &mut slots.person
        } else {
            &mut slots.other
        };
        if !WeightSlots::put(slot, w, RowPos::of(ev)) {
            return;
        }
        let now = self.weight_of(&p.task_id);
        // Irori's weighing is a line on the card, with its Undo; it wakes
        // nobody (card #965).
        if let (Some(what), Some(t)) = (line, self.tasks.get(&p.task_id))
            && now
                .as_ref()
                .is_some_and(|w| w.by_assistant() || !w.moved.is_empty())
        {
            let ti = Touch::of(t);
            self.touch(ev, &ti, when, &by, &what, false, Vec::new());
        }
        if let Some(t) = self.tasks.get_mut(&p.task_id) {
            if t.weight == now {
                return;
            }
            t.weight = now;
            self.note(Change {
                kind: TASK_CHANGED.into(),
                task_id: p.task_id,
                ..Change::default()
            });
        }
    }
}
