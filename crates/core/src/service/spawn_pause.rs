//! Pause agents (card #665): one switch that stops Irori starting or
//! restarting any agent on every one of the person's devices. New cards,
//! asks and threads, retries, the fresh agent after End or `agent_end`,
//! shared-board cards and other devices' requests all wait; agents already
//! running keep running and keep getting messages (unlike Pause all, which
//! holds wakes). Turning it off starts the waiting cards in the usual queue
//! order at the next sweep, which the switch kicks.
//!
//! It rides on a `spawn.cap` row under a reserved device name, like the
//! limit on agents' starts (card #575), so it reaches all the person's
//! devices with no new row kind; an older Irori only stores it. The person
//! sets it from the app, and their own agents with the `agents_pause` tool
//! (so "wind down the agents" is: pause, then ask each agent to stop); the
//! row's `by` says who.

use gocompat::time::GoTime;
use store::kinds;

use super::Services;
use super::spawn::SpawnCap;
use crate::logf;

/// The reserved `spawn.cap` device the pause rides on. Device ids are hex,
/// so it names no device.
pub const AGENTS_PAUSED_ROW: &str = "paused";

/// Whether agent starts are paused, since when and by whom.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentsPause {
    pub on: bool,
    /// When it was turned on (the first row of this pause); zero when off.
    pub since: GoTime,
    /// Who turned it on or off last: `user:<device>` or an agent's key.
    pub by: String,
}

/// Folds the pause rows, oldest first: the newest row with a switch wins,
/// and `since` stays at the row that first turned it on.
#[derive(Debug, Default)]
pub(crate) struct PauseFold {
    p: AgentsPause,
}

impl PauseFold {
    pub(crate) fn add(&mut self, r: &SpawnCap) {
        // A row with no switch says nothing (an older build's write).
        let Some(on) = r.on else { return };
        if on && !self.p.on {
            self.p.since = r.at;
        }
        if !on {
            self.p.since = GoTime::default();
        }
        self.p.on = on;
        self.p.by = r.by.clone();
    }

    pub(crate) fn done(self) -> AgentsPause {
        self.p
    }
}

impl Services {
    /// Whether agent starts are paused on every device.
    pub(crate) fn agents_pause(&self) -> AgentsPause {
        self.spawn
            .agents_pause
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub(crate) fn agents_paused(&self) -> bool {
        self.spawn
            .agents_pause
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .on
    }

    /// Turns Pause agents on or off for every device; `by` is
    /// `user:<device>` for the person, or the agent's key. Turning it on
    /// when it is on keeps its since. Off kicks the sweep, so the waiting
    /// cards start now in queue order.
    pub(crate) fn set_agents_paused(&self, on: bool, by: &str) -> Result<AgentsPause, String> {
        let rows = self.spawn.rows.lock().unwrap_or_else(|e| e.into_inner());
        let was = self.agents_pause();
        if was.on == on {
            return Ok(was);
        }
        let row = SpawnCap {
            device: AGENTS_PAUSED_ROW.into(),
            cap: 0,
            on: Some(on),
            by: by.into(),
            at: (self.now)(),
        };
        self.store
            .append("user", kinds::SPAWN_CAP, &row)
            .map_err(|e| e.to_string())?;
        let mut f = PauseFold { p: was };
        f.add(&row);
        let now = f.done();
        *self
            .spawn
            .agents_pause
            .write()
            .unwrap_or_else(|e| e.into_inner()) = now.clone();
        drop(rows);
        logf(format_args!(
            "spawn: {by} turned Pause agents {}",
            if on { "on" } else { "off" }
        ));
        self.republish_host();
        self.asks_kick.kick();
        Ok(now)
    }

    /// Who set the pause, for people: "the person on <device>", or the
    /// agent's name.
    pub(crate) fn pause_by_name(&self, by: &str) -> String {
        if by.is_empty() {
            return String::new();
        }
        if let Some(dev) = by.strip_prefix("user:") {
            let name = self.named_device(dev);
            return if name.is_empty() {
                "the person".into()
            } else {
                format!("the person on {name}")
            };
        }
        let name = self.agent_names.get(by);
        if name.is_empty() { by.to_string() } else { name }
    }

    /// Agent `caller` pauses or unpauses agent starts (the `agents_pause`
    /// tool): the person's own agents only.
    pub(crate) fn agent_set_paused(&self, caller: &str, on: bool) -> Result<String, String> {
        if !self.own_agent(caller) {
            return Err("only the person's own agents pause agents".into());
        }
        let p = self.set_agents_paused(on, caller)?;
        Ok(if p.on {
            format!(
                "Pause agents is on (since {}): Irori starts no agent on any of the person's devices until it is turned off. Running agents keep running; to wind down, ask each one to push, write left_off and stop.",
                p.since.format_rfc3339()
            )
        } else {
            "Pause agents is off: Irori starts the waiting cards in queue order now.".into()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(on: Option<bool>, secs: i64, by: &str) -> SpawnCap {
        SpawnCap {
            device: AGENTS_PAUSED_ROW.into(),
            cap: 0,
            on,
            by: by.into(),
            at: GoTime::unix(secs, 0),
        }
    }

    #[test]
    fn the_newest_switch_wins_and_since_is_when_it_went_on() {
        let mut f = PauseFold::default();
        f.add(&row(Some(true), 100, "user:a"));
        f.add(&row(Some(true), 200, "agent"));
        f.add(&row(None, 300, "old"));
        let p = f.done();
        assert!(p.on);
        assert_eq!(p.since.secs, 100);
        assert_eq!(p.by, "agent");

        let mut f = PauseFold::default();
        f.add(&row(Some(true), 100, "user:a"));
        f.add(&row(Some(false), 200, "user:b"));
        let p = f.done();
        assert!(!p.on);
        assert!(p.since.is_zero());
        assert_eq!(p.by, "user:b");
    }
}
