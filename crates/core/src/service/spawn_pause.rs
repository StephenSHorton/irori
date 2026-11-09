//! Pause agents (card #665): a switch per device that stops Irori starting
//! or restarting any agent on that device. New cards, asks and threads,
//! retries, the fresh agent after End or `agent_end`, shared-board cards and
//! other devices' requests all wait; agents already running keep running and
//! keep getting messages (unlike Pause all, which holds wakes). Stephen's
//! pick (#665 b2): each computer has its own switch.
//!
//! It rides on a `spawn.cap` row under a reserved device name,
//! `paused:<device id>`, like the limit on agents' starts (card #575), so it
//! reaches all the person's devices with no new row kind: the person's other
//! devices read it too and stop asking a paused device to start their
//! cards, and a paused device's own cards may start on another device that
//! can run them. An older Irori only stores the row. Turning it off kicks
//! the sweep, so the waiting cards start in the usual queue order.
//!
//! The person sets it from the app, and their own agents with the
//! `agents_pause` tool, for the agent's own device (so "wind down the
//! agents" is: pause, then ask each agent to stop); the row's `by` says
//! who.

use std::collections::HashMap;

use gocompat::time::GoTime;
use irori_identity::owner::device_of;
use store::kinds;

use super::Services;
use super::spawn::SpawnCap;
use crate::logf;

/// The prefix of the reserved `spawn.cap` device the pause rides on:
/// `paused:<device id>`. Device ids are hex, so it names no device.
pub const AGENTS_PAUSED_PREFIX: &str = "paused:";

/// The `by` of a pause Wind down set (card #787): `winddown:<device id>`,
/// the device whose person turned Wind down on. Off lifts only these, so a
/// device paused before (`user:…` or an agent's key) stays paused.
pub const WIND_DOWN_BY_PREFIX: &str = "winddown:";

/// The reserved device Stop everywhere's own pause rides on (card #787):
/// `paused:everywhere`. Device ids are hex, so it names no device. While it
/// is on, no device starts agents, whatever its own pause says (an agent's
/// `agents_pause` off, a device that joins later), and it is the switch
/// itself, on even when every device was paused before. Stop everywhere
/// still pauses each device too, so an older Irori holds its starts.
pub const EVERYWHERE: &str = "everywhere";

/// Whether a pause's `by` is Wind down's.
pub fn by_wind_down(by: &str) -> bool {
    by.starts_with(WIND_DOWN_BY_PREFIX)
}

/// Whether agent starts are paused on a device, since when and by whom.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentsPause {
    pub on: bool,
    /// When it was turned on (the first row of this pause); zero when off.
    pub since: GoTime,
    /// Who turned it on or off last: `user:<device>` or an agent's key.
    pub by: String,
}

/// Folds the pause rows, oldest first, per device: the newest row with a
/// switch wins, and `since` stays at the row that first turned it on.
#[derive(Debug, Default)]
pub(crate) struct PauseFold {
    p: HashMap<String, AgentsPause>,
}

/// The device a pause row is about, if it is one.
pub(crate) fn paused_device(row_device: &str) -> Option<&str> {
    row_device
        .strip_prefix(AGENTS_PAUSED_PREFIX)
        .filter(|d| !d.is_empty())
}

fn fold(p: &mut AgentsPause, r: &SpawnCap) {
    // A row with no switch says nothing.
    let Some(on) = r.on else { return };
    if on && !p.on {
        p.since = r.at;
    }
    if !on {
        p.since = GoTime::default();
    }
    p.on = on;
    p.by = r.by.clone();
}

impl PauseFold {
    pub(crate) fn add(&mut self, device: &str, r: &SpawnCap) {
        fold(self.p.entry(device.to_string()).or_default(), r);
    }

    pub(crate) fn done(self) -> HashMap<String, AgentsPause> {
        self.p
    }
}

impl Services {
    /// Pause agents on device `dev`.
    pub(crate) fn agents_pause_on(&self, dev: &str) -> AgentsPause {
        self.spawn
            .agents_pause
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(dev)
            .cloned()
            .unwrap_or_default()
    }

    /// Whether device `dev` starts no agents now: its own pause, or Stop
    /// everywhere's.
    pub(crate) fn agents_paused_on(&self, dev: &str) -> bool {
        let m = self
            .spawn
            .agents_pause
            .read()
            .unwrap_or_else(|e| e.into_inner());
        m.get(dev).is_some_and(|p| p.on) || m.get(EVERYWHERE).is_some_and(|p| p.on)
    }

    /// What holds device `dev`'s starts: its own pause when on, else Stop
    /// everywhere's (off when neither is on).
    pub(crate) fn pause_holding(&self, dev: &str) -> AgentsPause {
        let p = self.agents_pause_on(dev);
        if p.on {
            return p;
        }
        let e = self.agents_pause_on(EVERYWHERE);
        if e.on { e } else { p }
    }

    /// Pause agents on this device.
    pub(crate) fn agents_pause(&self) -> AgentsPause {
        self.agents_pause_on(&self.device.id)
    }

    /// Whether this device starts no agents now.
    pub(crate) fn agents_paused(&self) -> bool {
        self.agents_paused_on(&self.device.id)
    }

    /// Turns Pause agents on or off for device `dev`; `by` is
    /// `user:<device>` for the person, or the agent's key. Turning it on
    /// when it is on keeps its since. Off kicks the sweep, so the waiting
    /// cards start now in queue order.
    pub(crate) fn set_agents_paused(
        &self,
        dev: &str,
        on: bool,
        by: &str,
    ) -> Result<AgentsPause, String> {
        if dev.is_empty() {
            return Err("spawn: no device given".into());
        }
        let rows = self.spawn.rows.lock().unwrap_or_else(|e| e.into_inner());
        let mut p = self.agents_pause_on(dev);
        if p.on == on {
            return Ok(p);
        }
        let row = SpawnCap {
            device: format!("{AGENTS_PAUSED_PREFIX}{dev}"),
            cap: 0,
            on: Some(on),
            by: by.into(),
            at: (self.now)(),
        };
        self.store
            .append("user", kinds::SPAWN_CAP, &row)
            .map_err(|e| e.to_string())?;
        fold(&mut p, &row);
        self.spawn
            .agents_pause
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(dev.to_string(), p.clone());
        drop(rows);
        logf(format_args!(
            "spawn: {by} turned Pause agents {} on {dev}",
            if on { "on" } else { "off" }
        ));
        self.asks_kick.kick();
        Ok(p)
    }

    /// Who set the pause, for people: "the person on <device>", or the
    /// agent's name. A removed device is not named (card #1240).
    pub(crate) fn pause_by_name(&self, by: &str) -> String {
        if by.is_empty() {
            return String::new();
        }
        let named = |dev: &str| {
            if self.chain.cut(dev).is_some() {
                String::new()
            } else {
                self.named_device(dev)
            }
        };
        if let Some(dev) = by.strip_prefix(WIND_DOWN_BY_PREFIX) {
            let name = named(dev);
            return if name.is_empty() {
                "Stop everywhere".into()
            } else {
                format!("Stop everywhere on {name}")
            };
        }
        if let Some(dev) = by.strip_prefix("user:") {
            let name = named(dev);
            return if name.is_empty() {
                "the person".into()
            } else {
                format!("the person on {name}")
            };
        }
        let name = self.agent_names.get(by);
        if name.is_empty() {
            by.to_string()
        } else {
            name
        }
    }

    /// Every device's Pause agents, by device id. A removed device's is
    /// left out (card #1240): it neither counts nor is named.
    pub(crate) fn agents_pauses(&self) -> HashMap<String, AgentsPause> {
        let mut out = self
            .spawn
            .agents_pause
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        out.retain(|d, _| self.chain.cut(d).is_none());
        out
    }

    /// Stop everywhere on (card #787): its own pause ([`EVERYWHERE`]),
    /// which holds every device, then Pause agents on each of `devs`, by
    /// Stop everywhere from this device. A device already paused keeps its
    /// pause and who set it, so Off leaves it paused. Says which devices it
    /// paused.
    pub(crate) fn wind_down_pause(&self, devs: &[String]) -> Result<Vec<String>, String> {
        let by = format!("{WIND_DOWN_BY_PREFIX}{}", self.device.id);
        self.set_agents_paused(EVERYWHERE, true, &by)?;
        let mut paused = vec![];
        for dev in devs {
            if dev.is_empty() || dev == EVERYWHERE || self.agents_pause_on(dev).on {
                continue;
            }
            self.set_agents_paused(dev, true, &by)?;
            paused.push(dev.clone());
        }
        Ok(paused)
    }

    /// Lifts the old inbox Pause all on this device (card #1196, GitHub
    /// #305). It left the app with card #787 (and the mod's `/irori pause`
    /// and the API's way on with this card), but old Wind downs turned it
    /// on when they finished, so a device could keep it with nothing in
    /// the top bar saying so: no agent started or was woken here, and the
    /// card's line pointed at a switch the person no longer has. Stop here
    /// off, Start everywhere and each start lift it, as the person, the
    /// way Resume wakes does, so the wakes it held go out again. `why`
    /// goes in the log.
    pub(crate) fn lift_old_pause_all(&self, why: &str) {
        let mut p = self.inbox.policy();
        if !p.paused {
            return;
        }
        p.paused = false;
        match self.inbox.set_policy(p, &self.user_sender().key) {
            Ok(()) => {
                logf(format_args!(
                    "spawn: lifted the old Pause all on this device ({why})"
                ));
                self.asks_kick.kick();
            }
            Err(e) => logf(format_args!(
                "spawn: couldn't lift the old Pause all ({why}): {e}"
            )),
        }
    }

    /// Stop everywhere off (card #787): lifts every pause it set, on any
    /// of the person's devices, and only those, then its own. Says which
    /// devices it lifted.
    pub(crate) fn wind_down_lift(&self) -> Result<Vec<String>, String> {
        let by = format!("user:{}", self.device.id);
        let mut devs: Vec<String> = self
            .agents_pauses()
            .into_iter()
            .filter(|(d, p)| d != EVERYWHERE && p.on && by_wind_down(&p.by))
            .map(|(d, _)| d)
            .collect();
        devs.sort();
        for dev in &devs {
            self.set_agents_paused(dev, false, &by)?;
        }
        self.set_agents_paused(EVERYWHERE, false, &by)?;
        self.lift_old_pause_all("Start everywhere");
        Ok(devs)
    }

    /// Agent `caller` pauses or unpauses agent starts on its own device
    /// (the `agents_pause` tool): the person's own agents only.
    pub(crate) fn agent_set_paused(&self, caller: &str, on: bool) -> Result<String, String> {
        if !self.own_agent(caller) {
            return Err("only the person's own agents pause agents".into());
        }
        let dev = device_of(caller).to_string();
        // Stop everywhere is the person's switch: only they lift it, and
        // an agent's off would let its device start cards while the
        // person's switch still reads on (card #787).
        if !on && self.agents_pause_on(EVERYWHERE).on {
            return Err("Stop everywhere is on: the person turns it off (Start everywhere); agents can't start work again while it is on".into());
        }
        let name = self.named_device(&dev);
        let on_dev = if name.is_empty() {
            "your device".to_string()
        } else {
            name
        };
        let p = self.set_agents_paused(&dev, on, caller)?;
        Ok(if p.on {
            format!(
                "Pause agents is on for {on_dev} (since {}): Irori starts no agent there until it is turned off; the person's other devices don't ask it. Running agents keep running; to wind down, ask each one to push, write left_off and stop.",
                p.since.format_rfc3339()
            )
        } else {
            format!(
                "Pause agents is off for {on_dev}: Irori starts the waiting cards there in queue order now."
            )
        })
    }

    /// What the tool reads: Pause agents on agent `caller`'s device.
    pub(crate) fn agent_pause_of(&self, caller: &str) -> (String, AgentsPause) {
        let dev = device_of(caller);
        let dev = if dev.is_empty() {
            self.device.id.as_str()
        } else {
            dev
        };
        (self.named_device(dev), self.pause_holding(dev))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(dev: &str, on: Option<bool>, secs: i64, by: &str) -> SpawnCap {
        SpawnCap {
            device: format!("{AGENTS_PAUSED_PREFIX}{dev}"),
            cap: 0,
            on,
            by: by.into(),
            at: GoTime::unix(secs, 0),
        }
    }

    fn folded(rows: &[SpawnCap]) -> HashMap<String, AgentsPause> {
        let mut f = PauseFold::default();
        for r in rows {
            f.add(paused_device(&r.device).unwrap(), r);
        }
        f.done()
    }

    #[test]
    fn each_device_has_its_own_switch_and_since_is_when_it_went_on() {
        let p = folded(&[
            row("mac", Some(true), 100, "user:mac"),
            row("pc", Some(true), 150, "user:pc"),
            row("mac", Some(true), 200, "agent"),
            row("mac", None, 300, "old"),
            row("pc", Some(false), 400, "user:mac"),
        ]);
        let mac = &p["mac"];
        assert!(mac.on);
        assert_eq!(mac.since.secs, 100);
        assert_eq!(mac.by, "agent");
        let pc = &p["pc"];
        assert!(!pc.on);
        assert!(pc.since.is_zero());
        assert_eq!(pc.by, "user:mac");
    }

    #[test]
    fn only_a_winddown_by_is_wind_downs() {
        assert!(by_wind_down("winddown:mac"));
        assert!(!by_wind_down("user:mac"));
        assert!(!by_wind_down("abc/def"));
    }

    #[test]
    fn only_a_named_device_is_a_pause_row() {
        assert_eq!(paused_device("paused:abc"), Some("abc"));
        assert_eq!(paused_device("paused:"), None);
        assert_eq!(paused_device("agents"), None);
        assert_eq!(paused_device("abc"), None);
    }
}
