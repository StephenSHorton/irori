//! The two huddle switches (card #1053, docs/design/call-agents.md §12):
//! "Let my assistant add notes from what I say" and "Let my assistant chair
//! huddles I start". Private `{on, by, at}` rows like "Weigh new cards";
//! on unless the person turned them off (decision 52), newest wins, set in
//! the app only.

use std::sync::atomic::{AtomicBool, Ordering};

use store::{Store, kinds};

use super::Services;
use super::call_settings::newest_switch;
use super::thread_names::NamesSet;

/// On for a new person.
pub const HUDDLE_SWITCH_ON: bool = true;

/// The switches as last read from the log.
pub(crate) struct HuddleSwitches {
    notes: AtomicBool,
    chair: AtomicBool,
}

impl HuddleSwitches {
    pub(crate) fn load(st: &Store) -> HuddleSwitches {
        HuddleSwitches {
            notes: AtomicBool::new(newest_switch(
                st,
                kinds::ASSISTANT_HUDDLE_NOTES_SET,
                HUDDLE_SWITCH_ON,
            )),
            chair: AtomicBool::new(newest_switch(
                st,
                kinds::ASSISTANT_HUDDLE_CHAIR_SET,
                HUDDLE_SWITCH_ON,
            )),
        }
    }
}

impl Services {
    /// "Let my assistant add notes from what I say": off stops its own
    /// digest (L3); being called, its person's questions and routed
    /// questions still work.
    pub(crate) fn huddle_notes_on(&self) -> bool {
        self.huddle_switches.notes.load(Ordering::SeqCst)
    }

    /// "Let my assistant chair huddles I start": off passes the chair to the
    /// next person's assistant.
    pub(crate) fn huddle_chair_on(&self) -> bool {
        self.huddle_switches.chair.load(Ordering::SeqCst)
    }

    fn set_huddle_switch(&self, kind: &str, on: bool) -> Result<(), String> {
        let row = NamesSet {
            on,
            by: format!("user:{}", self.device.id),
            at: (self.now)(),
        };
        self.store
            .append("user", kind, &row)
            .map_err(|e| e.to_string())?;
        self.follow_huddle_switches();
        Ok(())
    }

    pub(crate) fn set_huddle_notes_on(&self, on: bool) -> Result<(), String> {
        self.set_huddle_switch(kinds::ASSISTANT_HUDDLE_NOTES_SET, on)
    }

    pub(crate) fn set_huddle_chair_on(&self, on: bool) -> Result<(), String> {
        self.set_huddle_switch(kinds::ASSISTANT_HUDDLE_CHAIR_SET, on)
    }

    /// A switch row from another device, or a resync: reads both again.
    pub(crate) fn follow_huddle_switches(&self) {
        let notes = newest_switch(
            &self.store,
            kinds::ASSISTANT_HUDDLE_NOTES_SET,
            HUDDLE_SWITCH_ON,
        );
        let chair = newest_switch(
            &self.store,
            kinds::ASSISTANT_HUDDLE_CHAIR_SET,
            HUDDLE_SWITCH_ON,
        );
        self.huddle_switches.notes.store(notes, Ordering::SeqCst);
        self.huddle_switches.chair.store(chair, Ordering::SeqCst);
    }
}
