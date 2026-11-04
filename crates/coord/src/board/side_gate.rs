//! An agent keeps to its device's sides (card #496, work-personal-separation
//! §7.5): on its own it never claims, joins or assigns a card on a side its
//! device doesn't run. The person's own moves go through, and so does an
//! agent the person asked.
//!
//! "The person asked" is recognised on the card, from what the log already
//! holds:
//! - the person is the actor (`user:<device>` on any of their devices);
//! - Irori itself (its sweep, a spawn's bind): its picks keep to the side
//!   by themselves (#495);
//! - the agent is assigned the card: only the person or Irori can put an
//!   agent on a card across the line, since an agent's own assign is
//!   checked here too;
//! - the agent was started for the card (Start an agent, or the spawn
//!   sweep) or for its parent;
//! - the person @-mentioned the agent on the card, or sent it a message
//!   that names the card.
//!
//! A message from the person that names no card leaves nothing on the
//! card, so it doesn't count: the agent asks them to assign it instead.

use irori_identity::owner;

use super::error::{Error, ErrorKind, Result};
use super::fold::Fold;
use super::types::Task;
use super::util::{SWEEPER, contains};

/// The part of every side refusal that says how the line is crossed;
/// tools and tests look for it.
pub const ASK_THE_PERSON: &str = "ask the person";

impl Fold {
    /// Refuses agent `by` taking card t (claim or join) when its device
    /// doesn't run t's side and the person didn't ask it to.
    pub(crate) fn check_side_take(&self, by: &str, t: &Task) -> Result<()> {
        if self.free_of_sides(by) || self.person_asked(by, t) {
            return Ok(());
        }
        let dev = owner::device_of(by);
        if self.sides.runs(dev, &t.side) {
            return Ok(());
        }
        Err(self.side_refusal(t, dev, "your device", "they can assign it to you from the app, start an agent on it, or @-mention you on it"))
    }

    /// Refuses agent `by` assigning card t to `to` when `to`'s device
    /// doesn't run t's side, unless the person asked `by` to (an agent
    /// asked can hand it on: the person's word is on the card).
    pub(crate) fn check_side_assign(&self, by: &str, t: &Task, to: &str) -> Result<()> {
        if self.free_of_sides(by) {
            return Ok(());
        }
        let dev = owner::device_of(to);
        if self.sides.runs(dev, &t.side) {
            return Ok(());
        }
        if to == by && self.person_asked(by, t) {
            return Ok(());
        }
        let whose = if to == by {
            "your device".to_string()
        } else {
            format!("{to}'s device")
        };
        Err(self.side_refusal(t, dev, &whose, "they can assign it from the app"))
    }

    /// The person, and Irori itself, aren't held to a side here.
    fn free_of_sides(&self, by: &str) -> bool {
        by == SWEEPER || self.is_owner(by)
    }

    /// Whether the card shows the person asked agent `key` to take it.
    pub(crate) fn person_asked(&self, key: &str, t: &Task) -> bool {
        if contains(&t.assigned, key) {
            return true;
        }
        if self
            .spawned_for(key)
            .is_some_and(|s| s.task_id == t.id || (!t.parent.is_empty() && t.parent == s.task_id))
        {
            return true;
        }
        let Some(ids) = self.by_task.get(&t.id) else {
            return false;
        };
        ids.iter().filter_map(|id| self.comments.get(id)).any(|c| {
            c.person
                && !c.gone
                && (c.mentions.iter().any(|m| m.key == key)
                    || (c.to == key && !c.msg_id.is_empty()))
        })
    }

    fn side_refusal(&self, t: &Task, dev: &str, whose: &str, how: &str) -> Error {
        let runs = self
            .sides
            .of(dev)
            .map(|s| s.join(" and "))
            .unwrap_or_default();
        Error::raw(
            ErrorKind::NotAllowed,
            format!(
                "coord: {} is on the {} side, and {whose} runs only {runs}. Cards on another side are only read here, never taken, unless the person asks: {ASK_THE_PERSON} ({how}).",
                t.short_ref(),
                t.side,
            ),
        )
    }
}
