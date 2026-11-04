//! Blockers left with nobody (card #485): on a card nobody holds, an item
//! whose agent is gone — the one it waits on (on = agent), or the one that
//! raised it and would hear the person's answer — can't move by itself.
//! The window marks it ("Waiting on kestrel (gone)") with a "Hand to an
//! agent" action, and the person's answer to it goes to a new agent
//! instead of to nobody.

use std::collections::HashMap;

use gocompat::quote::quote;
use store::kinds;

use super::error::{Result, bad_state, not_allowed, not_found};
use super::fold::{Fold, LEFT_GONE};
use super::util::SWEEPER;
use crate::names::trim_space;
use super::rows::TaskMemberRow;
use super::types::*;

/// An open blocker whose agent is gone, on a card nobody holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stranded {
    pub blocker_id: String,
    /// The gone agent: the one waited on, else the raiser.
    pub agent: String,
    /// The item waits on that agent (on = agent), rather than having
    /// been raised by it.
    pub waited_on: bool,
}

/// The roster's status by key, read once per pass; `None` without a
/// roster.
pub(crate) type RosterStatus = Option<HashMap<String, String>>;

impl Fold {
    pub(crate) fn roster_status(&self) -> RosterStatus {
        self.roster.as_ref().map(|r| {
            r.agents()
                .into_iter()
                .map(|a| (a.key, a.status))
                .collect()
        })
    }

    /// Whether agent `key` is gone: its session ended here (agent.gone),
    /// Irori ended it, or the roster says gone or no longer lists it. Not
    /// the person, Irori, a run holder, or anything that isn't an agent
    /// key. On a shared board another member's agents aren't on this
    /// roster, so only a gone status or this device's own rows count there.
    pub(crate) fn key_gone_in(&self, key: &str, rs: &RosterStatus) -> bool {
        if key.is_empty()
            || !key.contains('/')
            || key == SWEEPER
            || self.is_owner(key)
            || key.starts_with(kinds::PERSON_ACTOR_PREFIX)
            || super::is_run_holder(key)
        {
            return false;
        }
        if self.gone_rows.contains_key(key) || self.ended_by_irori.contains(key) {
            return true;
        }
        match rs {
            Some(m) => match m.get(key) {
                Some(s) => s == "gone",
                None => !self.on_board || self.local_key(key),
            },
            None => false,
        }
    }

    pub(crate) fn key_gone(&self, key: &str) -> bool {
        self.key_gone_in(key, &self.roster_status())
    }

    /// The open items of `t` left with nobody: the card has no holder (nor
    /// a live agent it was handed to), and the item's agent is gone. A
    /// wait that ends by itself (a card, a lease, a version) is left out
    /// for its raiser: it closes and the card goes out again anyway.
    pub(crate) fn stranded_in(&self, t: &Task, rs: &RosterStatus) -> Vec<Stranded> {
        if is_terminal(&t.state)
            || !t.holders.is_empty()
            || !t.co_holders.is_empty()
            || t.assigned.iter().any(|k| !self.key_gone_in(k, rs))
        {
            return Vec::new();
        }
        let mut out = Vec::new();
        for b in t.blockers.iter().filter(|b| b.open()) {
            let kind = b.on.as_ref().map(|on| on.kind.as_str()).unwrap_or("");
            if kind == BLOCKED_ON_AGENT {
                let k = &b.on.as_ref().unwrap().reference;
                if self.key_gone_in(k, rs) {
                    out.push(Stranded {
                        blocker_id: b.id.clone(),
                        agent: k.clone(),
                        waited_on: true,
                    });
                }
                continue;
            }
            if matches!(kind, BLOCKED_ON_TASK | BLOCKED_ON_LEASE | BLOCKED_ON_VERSION) {
                continue;
            }
            // The agent that would hear the person's answer: the one it
            // was sent to, else the raiser.
            let to = if b.sent_to.is_empty() {
                &b.raised_by
            } else {
                &b.sent_to
            };
            if self.key_gone_in(to, rs) {
                out.push(Stranded {
                    blocker_id: b.id.clone(),
                    agent: to.clone(),
                    waited_on: false,
                });
            }
        }
        out
    }

    pub(crate) fn stranded(&self, t: &Task) -> Vec<Stranded> {
        self.stranded_in(t, &self.roster_status())
    }

    /// Takes the gone agents a card was handed to off it (task.left
    /// "gone"), so it can go out again.
    pub(crate) fn drop_gone_assigned(&mut self, by: &str, id: &str) -> Result<()> {
        let Some(t) = self.tasks.get(id) else {
            return Ok(());
        };
        let rs = self.roster_status();
        let gone: Vec<String> = t
            .assigned
            .iter()
            .filter(|k| !t.holders.contains(k) && self.key_gone_in(k, &rs))
            .cloned()
            .collect();
        for who in gone {
            let at = self.stamp();
            self.write(
                by,
                kinds::TASK_LEFT,
                &TaskMemberRow {
                    task_id: id.into(),
                    who,
                    by: by.into(),
                    at,
                    reason: LEFT_GONE.into(),
                    person: true,
                },
            )?;
        }
        Ok(())
    }

    /// "Hand to an agent" (card #485): the person moves a card whose items
    /// wait on gone agents back on the Board, so Irori starts an agent for
    /// it (or hands it to a free one). Its open items close as it goes, as
    /// any card put back does; the new agent's brief says what they were.
    /// Returns the card before the move and its stranded items.
    pub(crate) fn hand_on(&mut self, by: &str, id: &str) -> Result<(Task, Vec<Stranded>)> {
        let by = trim_space(by).to_string();
        if !self.is_owner(&by) {
            return Err(not_allowed("only the person hands a card to an agent"));
        }
        let Some(t) = self.tasks.get(id).cloned() else {
            return Err(not_found(format!("task {}", quote(id))));
        };
        if !t.holders.is_empty() || !t.co_holders.is_empty() {
            return Err(bad_state(format!("{} has an agent on it", t.short_ref())));
        }
        let st = self.stranded(&t);
        if st.is_empty() {
            return Err(bad_state(format!(
                "nothing on {} waits on an agent that is gone",
                t.short_ref()
            )));
        }
        self.drop_gone_assigned(&by, id)?;
        let names: Vec<String> = st
            .iter()
            .map(|s| s.agent.clone())
            .fold(Vec::new(), |mut v, k| {
                if !v.contains(&k) {
                    v.push(k);
                }
                v
            })
            .iter()
            .map(|k| self.named(k))
            .collect();
        let moved = self.back_on_board(
            &by,
            id,
            &format!(
                "{} gone; the person handed it to an agent",
                names.join(", ")
            ),
        )?;
        if !moved {
            let why = if t.open_ask().is_some() {
                "it has an open ask"
            } else if !t.subtasks.is_empty() {
                "it has subtasks; hand those on instead"
            } else {
                "it can't go back on the Board now"
            };
            return Err(bad_state(format!("{}: {why}", t.short_ref())));
        }
        Ok((t, st))
    }
}
