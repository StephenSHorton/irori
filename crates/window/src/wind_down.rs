//! Wind down (card #220, a switch since card #787,
//! docs/design/wind-down.md): the person asks every agent to save its
//! place, hand back its cards and stop, and Irori stops starting agents on
//! every one of their devices (Pause agents) until the person turns it off.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use specta::Type;

use crate::Window;

/// windDown:changed: the switch or a run's status moved.
pub const EVENT_WIND_DOWN: &str = "windDown:changed";
/// A run's scope: Wind down, every one of the person's devices.
pub const SCOPE_ALL: &str = "all";
/// A run's scope: Pause agents on this device, its graceful stop (card
/// #787).
pub const SCOPE_DEVICE: &str = "device";

/// What a wind-down says (the wake's preamble carries the steps).
pub const WIND_DOWN_TEXT: &str = "Wind down: save your place, hand back your cards, release your leases and reply \"wound down\". Then stop until I resume you.";
/// The Stop a straggler's row sends.
pub const WIND_DOWN_STOP_TEXT: &str = "Wind down now: commit and push your work, post a hand-off and leave your cards, release your leases and reply \"wound down\".";

/// Row states.
pub mod row {
    /// Not reached yet (queued).
    pub const WAITING: &str = "waiting";
    /// Delivered or engaged: it is winding down.
    pub const WINDING: &str = "winding";
    /// It answered.
    pub const DONE: &str = "done";
    /// Closed without an answer (skipped, expired, its session ended).
    pub const NO_ANSWER: &str = "no_answer";
    /// Wind down was turned off before it reached the agent (card #787).
    pub const CANCELED: &str = "canceled";
}

/// One agent in a run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
pub struct WindDownRow {
    pub key: String,
    pub name: String,
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub device: String,
    #[serde(rename = "deliveryId")]
    pub delivery_id: String,
    /// [`row`]'s states.
    pub state: String,
    /// Its answer (the cards it left), once it answered.
    pub reply: String,
    /// Why it couldn't be sent, when it couldn't.
    pub error: String,
}

/// The switch and this device's latest run (empty `id`: no run here).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
pub struct WindDownStatus {
    /// The switch (card #787): Wind down holds agent starts on at least one
    /// of the person's devices. Read from the replicated Pause agents rows,
    /// so every device sees it, and it outlives a restart.
    pub on: bool,
    /// When it went on, in ms since the epoch (0 when off).
    #[specta(type = f64)]
    pub since: i64,
    /// The devices Wind down holds, by name.
    pub devices: Vec<String>,
    pub id: String,
    pub scope: String,
    /// No agent is still waiting or winding down.
    pub finished: bool,
    /// The run was turned off (card #787).
    pub off: bool,
    pub rows: Vec<WindDownRow>,
}

#[derive(Debug, Default)]
pub(crate) struct Run {
    st: WindDownStatus,
}

/// The run, and the switch as last told (so a change from another device
/// is told once).
#[derive(Default)]
pub(crate) struct RunSlotInner {
    run: Option<Run>,
    told: Option<bool>,
    listeners: Vec<SwitchFn>,
}

pub(crate) type RunSlot = Mutex<RunSlotInner>;

/// Takes the switch (on or off) whenever it moves (the tray).
pub type SwitchFn = Arc<dyn Fn(bool) + Send + Sync>;

/// A delivery state (and its note) as a row state.
pub fn row_state(delivery_state: &str, note: &str) -> &'static str {
    use inbox::st;
    match delivery_state {
        st::QUEUED | st::HELD => row::WAITING,
        st::DELIVERED | st::ENGAGED | st::DEFERRED => row::WINDING,
        st::ANSWERED => row::DONE,
        st::SKIPPED if note == inbox::NOTE_WIND_DOWN_OFF => row::CANCELED,
        _ => row::NO_ANSWER,
    }
}

/// Every row has ended (done, no answer or canceled) and at least one was
/// sent.
pub fn finished(rows: &[WindDownRow]) -> bool {
    rows.iter().any(|r| !r.delivery_id.is_empty())
        && rows
            .iter()
            .all(|r| r.state == row::DONE || r.state == row::NO_ANSWER || r.state == row::CANCELED)
}

impl Window {
    /// Wind down on (WindDownService.All): holds agent starts on every one
    /// of the person's devices at once, then sends a wind-down to every
    /// live agent on all of them, as a run that replaces any earlier one.
    /// There is no Wind down for one device (card #787): that is Pause
    /// agents. A device already paused stays as it was.
    pub fn wind_down_all(&self) -> Result<WindDownStatus, String> {
        let here = self.core.device_id().to_string();
        let mut agents = self.core.mesh_agents();
        agents.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.key.cmp(&b.key)));
        agents.dedup_by(|a, b| a.key == b.key);
        // Stop starts first, so no fresh agent takes a card the wound-down
        // ones leave, on any device (card #787).
        let mut devs = vec![here];
        devs.extend(self.core.agent_devices());
        devs.extend(agents.iter().map(|a| a.device_id.clone()));
        devs.sort();
        devs.dedup();
        self.core.wind_down_pause(&devs)?;
        self.start_run(SCOPE_ALL, agents);
        Ok(self.wind_down_told())
    }

    /// The shared graceful stop (card #787): a wind-down to each of
    /// `agents`, as the run that replaces any earlier one. Wind down sends
    /// it to every device's agents, Pause agents to this device's.
    fn start_run(&self, scope: &str, agents: Vec<coord::roster::AgentCard>) {
        let me = self.core.user_sender();
        let ib = self.core.inbox();
        // An agent already winding down (Pause here, then Pause all) keeps
        // its wind-down: the row follows it, no second one is sent.
        let open: std::collections::HashMap<String, ::inbox::Delivery> = ib
            .open_wind_downs()
            .into_iter()
            .map(|d| (d.to.clone(), d))
            .collect();
        let mut run = Run {
            st: WindDownStatus {
                id: format!(
                    "wd_{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or_default()
                ),
                scope: scope.into(),
                ..WindDownStatus::default()
            },
        };
        for a in agents {
            let to = self.core.private_key(&a.key);
            let mut r = WindDownRow {
                key: to.clone(),
                name: a.name.clone(),
                device: self.core.device_label(&a.device_id),
                device_id: a.device_id.clone(),
                state: row::WAITING.into(),
                ..WindDownRow::default()
            };
            if let Some(d) = open.get(&to) {
                r.delivery_id = d.id.clone();
                r.state = row_state(&d.state, &d.note).into();
                run.st.rows.push(r);
                continue;
            }
            let o = ::inbox::SendOptions {
                mode: ::inbox::mode::WAKE.into(),
                reason: ::inbox::REASON_WIND_DOWN.into(),
                ..::inbox::SendOptions::default()
            };
            match ib.send_with(
                &me,
                &::inbox::Address::new(::inbox::addr::AGENT, &to),
                WIND_DOWN_TEXT,
                o,
            ) {
                Ok((_, d)) => {
                    r.delivery_id = d.id;
                    r.state = row_state(&d.state, &d.note).into();
                }
                Err(e) => {
                    r.state = row::NO_ANSWER.into();
                    r.error = e.to_string();
                }
            }
            run.st.rows.push(r);
        }
        self.wind_down.lock().unwrap_or_else(|e| e.into_inner()).run = Some(run);
    }

    /// Closes the wind-downs not reached yet whose agent's device starts
    /// agents again (no pause holds it), every device's (card #787).
    fn cancel_unpaused(&self) -> Vec<String> {
        let ib = self.core.inbox();
        let by = self.core.user_sender().key;
        let mut errs = vec![];
        for d in ib.open_wind_downs() {
            let dev = irori_identity::owner::device_of(&d.to);
            // Its own pause, or Stop everywhere's, still holds it.
            if self.core.agents_paused_on(dev) {
                continue;
            }
            if let Err(e) = ib.cancel_wind_down(&d.id, &by) {
                errs.push(e.to_string());
            }
        }
        errs
    }

    /// Wind down off (WindDownService.Off, card #787), at any time: the
    /// wind-downs that haven't reached their agents close (every device's),
    /// the pauses Wind down set are lifted, and the cards the agents left
    /// start again in queue order. Agents already winding down finish.
    pub fn wind_down_off(&self) -> Result<WindDownStatus, String> {
        let mut errs = vec![];
        if let Err(e) = self.core.wind_down_lift() {
            errs.push(e);
        }
        // A device still paused (by the person, or its own Pause agents)
        // keeps its graceful stop going.
        errs.extend(self.cancel_unpaused());
        if let Some(run) = self
            .wind_down
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .run
            .as_mut()
            && run.st.scope == SCOPE_ALL
        {
            run.st.off = true;
        }
        let st = self.wind_down_told();
        if errs.is_empty() {
            Ok(st)
        } else {
            Err(format!("wind down off: {}", errs.join("; ")))
        }
    }

    /// Pause agents from the app (card #787): a graceful stop of this
    /// device. On holds starts here and winds this device's agents down
    /// (their cards may start on the person's other devices); off lifts it
    /// and closes the wind-downs not reached yet. The `agents_pause` tool
    /// only holds starts.
    pub(crate) fn pause_here(&self, on: bool) -> Result<(), String> {
        let was = self.core.agents_pause().on;
        self.core.set_agents_paused(on)?;
        if on && !was {
            let here = self.core.device_id().to_string();
            let mut agents: Vec<_> = self
                .core
                .mesh_agents()
                .into_iter()
                .filter(|a| a.device_id == here)
                .collect();
            agents.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.key.cmp(&b.key)));
            agents.dedup_by(|a, b| a.key == b.key);
            if !agents.is_empty() {
                self.start_run(SCOPE_DEVICE, agents);
            }
        }
        if !on {
            let errs = self.cancel_unpaused();
            if let Some(run) = self
                .wind_down
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .run
                .as_mut()
                && run.st.scope == SCOPE_DEVICE
            {
                run.st.off = true;
            }
            if !errs.is_empty() {
                self.wind_down_told();
                return Err(format!("pause agents off: {}", errs.join("; ")));
            }
        }
        self.wind_down_told();
        Ok(())
    }

    /// The switch, read from Stop everywhere's own pause and every
    /// device's Pause agents (card #787).
    fn wind_down_switch(&self, st: &mut WindDownStatus) {
        use irori_core::spawn_pause::{EVERYWHERE, by_wind_down};
        let mut pauses = self.core.agents_pauses();
        let all = pauses.remove(EVERYWHERE).filter(|p| p.on);
        let mut on: Vec<(String, irori_core::spawn_pause::AgentsPause)> = pauses
            .into_iter()
            .filter(|(_, p)| p.on && by_wind_down(&p.by))
            .collect();
        on.sort_by(|a, b| a.0.cmp(&b.0));
        // Its own pause is the switch; a device's alone is from an Irori
        // before it (or a lift that half-failed), still lifted by Off.
        st.on = all.is_some() || !on.is_empty();
        st.since = all
            .iter()
            .chain(on.iter().map(|(_, p)| p))
            .map(|p| p.since.unix_milli_value())
            .min()
            .unwrap_or(0);
        st.devices = on
            .iter()
            .map(|(d, _)| {
                let n = self.core.device_label(d);
                if n.is_empty() { d.clone() } else { n }
            })
            .collect();
    }

    /// WindDownService.Status: the switch, and the current run read fresh
    /// from its deliveries.
    pub fn wind_down_status(&self) -> WindDownStatus {
        let ib = self.core.inbox();
        let mut st = {
            let mut slot = self.wind_down.lock().unwrap_or_else(|e| e.into_inner());
            match slot.run.as_mut() {
                None => WindDownStatus::default(),
                Some(run) => {
                    for r in run.st.rows.iter_mut() {
                        if r.delivery_id.is_empty() {
                            continue;
                        }
                        let Some(d) = ib.get(&r.delivery_id) else {
                            continue;
                        };
                        r.state = row_state(&d.state, &d.note).into();
                        // The agent's "wound down" is kept on the
                        // wind-down's own delivery (card #307), never sent
                        // to the person as a message.
                        if r.state == row::DONE {
                            r.reply = d.note.clone();
                        }
                    }
                    run.st.finished = run.st.rows.is_empty() || finished(&run.st.rows);
                    run.st.clone()
                }
            }
        };
        self.wind_down_switch(&mut st);
        st
    }

    /// The status, told to the window and the switch's listeners.
    fn wind_down_told(&self) -> WindDownStatus {
        let st = self.wind_down_status();
        self.emit(EVENT_WIND_DOWN, &st);
        let fns = {
            let mut slot = self.wind_down.lock().unwrap_or_else(|e| e.into_inner());
            slot.told = Some(st.on);
            slot.listeners.clone()
        };
        for f in fns {
            f(st.on);
        }
        st
    }

    /// `f` gets the switch now and whenever it moves (the tray).
    pub fn on_wind_down(&self, f: SwitchFn) {
        f(self.wind_down_status().on);
        self.wind_down
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .listeners
            .push(f);
    }

    /// Re-reads the switch and the run after a change, or now and then
    /// (another device may turn the switch), and tells the window when a
    /// run is going or the switch moved.
    pub(crate) fn wind_down_tick(&self) {
        let (running, told) = {
            let slot = self.wind_down.lock().unwrap_or_else(|e| e.into_inner());
            (slot.run.as_ref().is_some_and(|r| !r.st.finished), slot.told)
        };
        let mut st = WindDownStatus::default();
        self.wind_down_switch(&mut st);
        if running || told != Some(st.on) {
            self.wind_down_told();
        }
    }

    /// Stop on a straggler's row: cuts its running turn with the
    /// wind-down's steps.
    /// The Stop is itself a wind-down (mode stop, reason wind_down, card
    /// #307), so its "wound down" reply also stays out of the person's
    /// messages; the row follows it from then on.
    pub fn wind_down_stop(&self, key: &str) -> Result<(), String> {
        let key = crate::trim(key);
        if key.is_empty() {
            return Err("stop whom? no agent key".into());
        }
        let to = self.core.private_key(key);
        let (_, d) = self
            .core
            .inbox()
            .send_with(
                &self.core.user_sender(),
                &::inbox::Address::new(::inbox::addr::AGENT, &to),
                WIND_DOWN_STOP_TEXT,
                ::inbox::SendOptions {
                    mode: ::inbox::mode::STOP.into(),
                    reason: ::inbox::REASON_WIND_DOWN.into(),
                    ..::inbox::SendOptions::default()
                },
            )
            .map_err(|e| e.to_string())?;
        if let Some(run) = self
            .wind_down
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .run
            .as_mut()
            && let Some(r) = run.st.rows.iter_mut().find(|r| r.key == to)
        {
            r.delivery_id = d.id;
            r.state = row_state(&d.state, &d.note).into();
            r.error.clear();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(state: &str) -> WindDownRow {
        WindDownRow {
            delivery_id: "d".into(),
            state: state.into(),
            ..WindDownRow::default()
        }
    }

    #[test]
    fn states() {
        assert_eq!(row_state("queued", ""), row::WAITING);
        assert_eq!(row_state("held", ""), row::WAITING);
        assert_eq!(row_state("delivered", ""), row::WINDING);
        assert_eq!(row_state("engaged", ""), row::WINDING);
        assert_eq!(row_state("deferred", ""), row::WINDING);
        assert_eq!(row_state("answered", ""), row::DONE);
        assert_eq!(row_state("skipped", ""), row::NO_ANSWER);
        assert_eq!(
            row_state("skipped", inbox::NOTE_RECIPIENT_GONE),
            row::NO_ANSWER
        );
        assert_eq!(
            row_state("skipped", inbox::NOTE_WIND_DOWN_OFF),
            row::CANCELED
        );
    }

    #[test]
    fn finished_only_when_every_row_ended() {
        assert!(!finished(&[]));
        assert!(!finished(&[r(row::DONE), r(row::WINDING)]));
        assert!(!finished(&[r(row::WAITING)]));
        assert!(finished(&[r(row::DONE), r(row::NO_ANSWER)]));
        assert!(finished(&[r(row::CANCELED), r(row::DONE)]));
        let unsent = WindDownRow {
            state: row::NO_ANSWER.into(),
            ..WindDownRow::default()
        };
        assert!(!finished(std::slice::from_ref(&unsent)), "nothing was sent");
        assert!(finished(&[unsent, r(row::DONE)]));
    }
}
