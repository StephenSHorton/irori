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
use super::fold::Fold;
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

impl Fold {
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
            Some(w) => w.level == level && w.reason == reason,
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
            },
        )?;
        let mut t = t;
        t.weight = self.weight_of(id);
        Ok((t, true))
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
        let w = Weight {
            level: p.level,
            reason: p.reason,
            set_by: set_by.into(),
            by: p.by,
            at: p.at,
        };
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
