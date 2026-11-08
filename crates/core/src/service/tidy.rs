//! "Tidy the board" (card #786): the Core side of Irori tidying the
//! person's own board. The sweep writes its hand-back lines and asks
//! about merged cards itself; this module holds the switch that turns
//! that on, and hears two Board changes the sweep doesn't see: a PR that
//! merged on a card nobody holds, and an agent leaving a card whose PRs
//! are all merged. Jobs go through the one assistant loop, so later jobs
//! share it. On unless the person turns it off (decision 52).

use std::sync::atomic::{AtomicBool, Ordering};

use coord::{Change, RESYNC, TASK_CHANGED};
use store::kinds;

use super::Services;
use super::call_settings::newest_switch;
use super::thread_names::{Job, NamesSet};
use crate::logf;

/// On for a new person (decision 52).
pub const TIDY_ON: bool = true;

pub(crate) struct Tidying {
    on: AtomicBool,
}

impl Tidying {
    pub(crate) fn load(st: &store::Store) -> Tidying {
        Tidying {
            on: AtomicBool::new(newest_switch(st, kinds::ASSISTANT_TIDY_SET, TIDY_ON)),
        }
    }

    pub(crate) fn on(&self) -> bool {
        self.on.load(Ordering::SeqCst)
    }

    fn set(&self, on: bool) {
        self.on.store(on, Ordering::SeqCst);
    }
}

impl Services {
    pub(crate) fn tidy_on(&self) -> bool {
        self.tidying.on()
    }

    /// The person turns "Tidy the board" on or off, from the app only.
    pub(crate) fn set_tidy_on(&self, on: bool) -> Result<(), String> {
        let row = NamesSet {
            on,
            by: format!("user:{}", self.device.id),
            at: (self.now)(),
        };
        self.store
            .append("user", kinds::ASSISTANT_TIDY_SET, &row)
            .map_err(|e| e.to_string())?;
        self.follow_tidy_on();
        Ok(())
    }

    /// An `assistant.tidy.set` row from another device, or a resync.
    pub(crate) fn follow_tidy_on(&self) {
        let on = newest_switch(&self.store, kinds::ASSISTANT_TIDY_SET, TIDY_ON);
        self.tidying.set(on);
        self.coord.set_tidy(on);
    }

    /// A Board change the merged-work job cares about: a PR's new state,
    /// or someone leaving (a task.left carries `holder`). The job's own
    /// writes re-fire changes with neither, and its record stops a repeat.
    /// One device acts: the one that hands the card out.
    pub(crate) fn tidy_hear(&self, ch: &Change) {
        if !self.tidy_on() {
            return;
        }
        if ch.kind == RESYNC {
            // Changes were dropped, and a merged PR's row never comes again.
            for id in self.coord.merged_unasked() {
                self.names_queue_job(Job::Tidy(id));
            }
            return;
        }
        if ch.kind != TASK_CHANGED || ch.task_id.is_empty() {
            return;
        }
        if ch.reason != coord::REASON_PR_STATE && ch.holder.is_empty() {
            return;
        }
        if !self.coord.hands_out_here(&ch.task_id) {
            return;
        }
        self.names_queue_job(Job::Tidy(ch.task_id.clone()));
    }

    /// Runs one tidy job: asks about the card if it still qualifies.
    pub(crate) fn tidy_run(&self, id: &str) {
        match self.coord.flag_merged(id) {
            Ok(true) => logf(format_args!("assistant: asked if {id} is done")),
            Ok(false) => {}
            Err(e) => logf(format_args!("assistant: tidying {id}: {e}")),
        }
    }
}
