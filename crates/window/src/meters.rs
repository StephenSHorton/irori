//! This computer's hardware meters in the top bar (card #599): a sampler
//! thread reads [`sys::meters`] every [`EVERY`], keeps the last [`KEEP`]
//! samples for the panel's graphs, and emits each one as `meters:changed`.
//! `meters_history` gives the kept samples, so a reloaded page draws its
//! graphs at once.
//!
//! Desktop only: on a system with no sampler (the phone) the thread never
//! starts and the history stays empty.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use specta::Type;
use sys::cancel::Cancel;
use sys::meters;

use crate::Window;

pub const EVENT_METERS_CHANGED: &str = "meters:changed";

/// How often the meters are read: Task Manager's "Normal" is 1 s; 2 s keeps
/// the GPU engine counters (hundreds of instances) cheap.
pub const EVERY: Duration = Duration::from_secs(2);
/// Samples kept for the graphs: two minutes.
pub const KEEP: usize = 60;

/// One read of every meter, as the window draws it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MetersSample {
    /// When it was read, Unix milliseconds.
    pub at_ms: i64,
    pub cpu: MetersCpu,
    pub mem: MetersMem,
    pub disks: Vec<MetersDisk>,
    pub net: MetersNet,
    pub gpus: Vec<MetersGpu>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MetersCpu {
    /// Busy, 0 to 100; null on the first read.
    pub pct: Option<f32>,
    pub ghz: Option<f32>,
    pub cores: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MetersMem {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MetersDisk {
    pub name: String,
    pub active_pct: Option<f32>,
    pub read_bytes_per_s: u64,
    pub write_bytes_per_s: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MetersNet {
    pub rx_bits_per_s: u64,
    pub tx_bits_per_s: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MetersGpu {
    pub name: String,
    pub integrated: bool,
    pub util_pct: Option<f32>,
    pub temp_c: Option<f32>,
    pub mem_used_bytes: Option<u64>,
    pub mem_total_bytes: Option<u64>,
}

impl MetersSample {
    pub fn from_sys(at_ms: i64, s: meters::Sample) -> MetersSample {
        MetersSample {
            at_ms,
            cpu: MetersCpu {
                pct: s.cpu.pct,
                ghz: s.cpu.ghz,
                cores: s.cpu.cores,
            },
            mem: MetersMem {
                used_bytes: s.mem.used_bytes,
                total_bytes: s.mem.total_bytes,
            },
            disks: s
                .disks
                .into_iter()
                .map(|d| MetersDisk {
                    name: d.name,
                    active_pct: d.active_pct,
                    read_bytes_per_s: d.read_bytes_per_s,
                    write_bytes_per_s: d.write_bytes_per_s,
                })
                .collect(),
            net: MetersNet {
                rx_bits_per_s: s.net.rx_bits_per_s,
                tx_bits_per_s: s.net.tx_bits_per_s,
            },
            gpus: s
                .gpus
                .into_iter()
                .map(|g| MetersGpu {
                    name: g.name,
                    integrated: g.integrated,
                    util_pct: g.util_pct,
                    temp_c: g.temp_c,
                    mem_used_bytes: g.mem_used_bytes,
                    mem_total_bytes: g.mem_total_bytes,
                })
                .collect(),
        }
    }
}

/// The last [`KEEP`] samples, oldest first.
#[derive(Default)]
pub struct History(Mutex<VecDeque<MetersSample>>);

impl History {
    pub fn push(&self, s: MetersSample) {
        let mut q = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() == KEEP {
            q.pop_front();
        }
        q.push_back(s);
    }

    pub fn all(&self) -> Vec<MetersSample> {
        let q = self.0.lock().unwrap_or_else(|e| e.into_inner());
        q.iter().cloned().collect()
    }
}

impl Window {
    /// The kept samples, oldest first; empty on a system with no meters.
    pub fn meters_history(&self) -> Vec<MetersSample> {
        self.meters.all()
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Starts the sampler thread, or nothing where this OS has no meters.
pub(crate) fn start(w: Arc<Window>, stop: Cancel) -> Option<std::thread::JoinHandle<()>> {
    if !meters::SUPPORTED {
        return None;
    }
    let spawned = std::thread::Builder::new()
        .name("irori-window-meters".into())
        .spawn(move || {
            // Built on this thread: opening the counters takes a moment,
            // and the first read only primes the rates.
            let mut sampler = meters::Sampler::new();
            while !stop.sleep(EVERY) {
                let s = MetersSample::from_sys(now_ms(), sampler.sample());
                w.meters.push(s.clone());
                w.emit(EVENT_METERS_CHANGED, &s);
            }
        });
    match spawned {
        Ok(h) => Some(h),
        Err(e) => {
            irori_core::log(&format!("meters: no sampler thread: {e}"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_keeps_the_last_samples() {
        let h = History::default();
        for i in 0..(KEEP as i64 + 5) {
            h.push(MetersSample {
                at_ms: i,
                ..MetersSample::default()
            });
        }
        let all = h.all();
        assert_eq!(all.len(), KEEP);
        assert_eq!(all[0].at_ms, 5);
        assert_eq!(all[KEEP - 1].at_ms, KEEP as i64 + 4);
    }
}
