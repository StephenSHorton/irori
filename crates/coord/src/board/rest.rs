//! Letting go as an agent rests (agent-continuity §6, card #1215).
//!
//! Today's end lets a spawned agent's work go passively: once its key is
//! gone, `return_gone`, `release_gone` and `taker_gone` take it off. A
//! resting agent is never gone (it keeps its name and its count), so none
//! of those fire for it: the rest writes each let-go itself, before its
//! process stops.

use gocompat::time::GoTime;
use store::kinds;

use super::Coord;
use super::error::Result;
use super::fold::{Fold, LEFT_RESTED, NOBODY_NOTE, REASON_RELEASED};
use super::rows::{LeaseReleasedRow, TaskMemberRow, TaskStateRow};
use super::types::{SUBMITTED, WORKING, is_terminal};
use super::util::SWEEPER;
use crate::gotime;

impl Fold {
    /// Lets go of everything `key` holds as it rests: it leaves each open
    /// card it holds or was assigned (`task.left` reason `rested`: no crash,
    /// nobody told), gives up the asks and threads it took, passes the
    /// offers it was handed, and releases its leases. A private working card
    /// it was the last on goes up again, as a leave does; one whose every
    /// question the person answered goes back on the Board. Returns the
    /// cards it left, oldest first. Only this device's own agents, and a
    /// no-op for anything it no longer holds.
    pub(crate) fn rest_leave(&mut self, key: &str) -> Result<Vec<String>> {
        if key.is_empty() || !self.local_key(key) || !self.may_write(key, &self.dev) {
            return Ok(Vec::new());
        }
        let now = self.stamp();
        let mut ids: Vec<(GoTime, String)> = self
            .tasks
            .values()
            .filter(|t| !is_terminal(&t.state))
            .filter(|t| {
                t.holders.iter().any(|h| h == key)
                    || t.assigned.iter().any(|h| h == key)
                    || t.ask.as_ref().is_some_and(|a| a.taker == key)
                    || t.live_offer().is_some_and(|o| o.to == key)
            })
            .map(|t| (t.created_at, t.id.clone()))
            .collect();
        ids.sort_by(|a, b| gotime::cmp(&a.0, &b.0).then_with(|| a.1.cmp(&b.1)));
        let mut left = Vec::new();
        for (_, id) in ids {
            let t = self.tasks[&id].clone();
            if let Some(a) = t.open_ask() {
                if a.taker == key {
                    // Given back as a leave gives it back; the ask owner's
                    // duties hand it on, and no fresh agent starts for it
                    // until the person writes (`ask_waits_for_person`).
                    self.ask_release(key, &id)?;
                    left.push(id);
                }
                continue;
            }
            let holds = t.holders.iter().any(|h| h == key);
            let assigned = t.assigned.iter().any(|h| h == key);
            if !holds && !assigned {
                // Handed it and never took it: the next free agent gets it.
                self.pass_offer(key, &id)?;
                continue;
            }
            self.write(
                SWEEPER,
                kinds::TASK_LEFT,
                &TaskMemberRow {
                    task_id: id.clone(),
                    who: key.into(),
                    by: SWEEPER.into(),
                    at: now,
                    reason: LEFT_RESTED.into(),
                    person: false,
                },
            )?;
            left.push(id.clone());
            let t = &self.tasks[&id];
            if !holds || !t.holders.is_empty() {
                continue;
            }
            if t.state == WORKING && t.subtasks.is_empty() && !t.shared {
                let note = format!("{NOBODY_NOTE}: {} is resting", self.named(key));
                self.write(
                    SWEEPER,
                    kinds::TASK_STATUS,
                    &TaskStateRow {
                        task_id: id.clone(),
                        state: SUBMITTED.into(),
                        note,
                        by: SWEEPER.into(),
                        at: now,
                        ..TaskStateRow::default()
                    },
                )?;
            } else if t.all_answered() {
                // Everything it waited on is answered and it rests before
                // acting on it: the card goes back on the Board.
                self.back_on_board(
                    SWEEPER,
                    &id,
                    "the person answered and the agent working it is resting",
                )?;
            }
        }
        // Its leases (a slot, a PR claim, a board mirror): nobody sweeps a
        // resting holder's leases away, so the rest releases them now.
        let held: Vec<(String, String)> = self
            .held
            .values()
            .filter(|l| {
                l.lease.holder == key
                    && gotime::after(&l.lease.until, &now)
                    && self.lease_local(l)
            })
            .map(|l| (l.lease.resource.clone(), l.lease.holder.clone()))
            .collect();
        for (resource, holder) in held {
            self.write(
                SWEEPER,
                kinds::LEASE_RELEASED,
                &LeaseReleasedRow {
                    resource,
                    holder,
                    reason: REASON_RELEASED.into(),
                    by: SWEEPER.into(),
                    at: now,
                },
            )?;
        }
        Ok(left)
    }
}

impl Coord {
    /// See [`Fold::rest_leave`]: everything resting agent `key` holds is
    /// let go, before its process stops. Returns the cards it left.
    pub fn rest_leave(&self, key: &str) -> Result<Vec<String>> {
        self.lock().rest_leave(key)
    }
}
