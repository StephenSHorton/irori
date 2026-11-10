//! Live tasks on cards (card #1249, docs/design/live-tasks.md §5.3, §6):
//! the live load of this device's runs on its own `runs:changed` event,
//! and Stop.
//!
//! The run facts (command, start, end, Stopped) ride the cards
//! (`CardSummary.runs`, joined in [`crate::cards`]) and move with
//! `board:changed`. The CPU and memory the Core's sampler reads every 2 s
//! go out here instead, the `meters:changed` pattern, so a reading never
//! rebuilds the card lists: every [`EVERY`] while any run of this device
//! is going, each time the full set of live runs, and one empty set when
//! the last one ends.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use specta::Type;
use sys::cancel::Cancel;

use crate::Window;

pub const EVENT_RUNS_CHANGED: &str = "runs:changed";

/// How often the loads go out.
pub const EVERY: Duration = Duration::from_secs(2);

/// One live run's latest reading.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunLoad {
    /// The agent's key.
    pub key: String,
    /// The claude task id.
    pub run: String,
    /// CPU in cores over its tree; none before the second sample.
    pub cores: Option<f32>,
    pub mem_bytes: u64,
    /// When it was read, Unix ms (0: not yet).
    pub at: i64,
    /// This device's name.
    pub device: String,
}

impl Window {
    /// Every run of this device still going, with its latest reading.
    pub fn run_loads(&self) -> Vec<RunLoad> {
        let me = self.core.device_id().to_string();
        let live: Vec<_> = self
            .core
            .runs()
            .into_iter()
            .filter(|r| r.running() && r.key.split_once('/').is_some_and(|(d, _)| d == me))
            .collect();
        if live.is_empty() {
            return Vec::new();
        }
        let device = self.core.device_labels().remove(&me).unwrap_or_default();
        live.into_iter()
            .map(|r| RunLoad {
                cores: (r.sampled_ms > 0).then_some(r.cpu_cores),
                mem_bytes: r.mem_bytes,
                at: r.sampled_ms,
                device: device.clone(),
                key: r.key,
                run: r.task_id,
            })
            .collect()
    }

    /// The person's Stop on one of this device's runs: it reads Stopped
    /// and its process tree ends. No tool or route reaches this.
    pub fn runs_stop(&self, key: &str, task_id: &str) -> Result<(), String> {
        self.core.stop_run(crate::trim(key), crate::trim(task_id))
    }
}

/// Starts the loop that sends `runs:changed`.
pub(crate) fn start(w: Arc<Window>, stop: Cancel) -> Option<std::thread::JoinHandle<()>> {
    let spawned = std::thread::Builder::new()
        .name("irori-window-runs".into())
        .spawn(move || {
            let mut sent_any = false;
            while !stop.sleep(EVERY) {
                let loads = w.run_loads();
                // The page replaces its readings with each set: once empty
                // is enough to clear the last chip.
                if loads.is_empty() && !sent_any {
                    continue;
                }
                sent_any = !loads.is_empty();
                w.emit(EVENT_RUNS_CHANGED, &loads);
            }
        });
    match spawned {
        Ok(h) => Some(h),
        Err(e) => {
            irori_core::log(&format!("runs: no loads thread: {e}"));
            None
        }
    }
}
