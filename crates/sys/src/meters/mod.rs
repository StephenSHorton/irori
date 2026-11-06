//! This computer's hardware meters (card #599): CPU, memory, disks, network
//! and GPUs, read the way Task Manager (Windows) and Activity Monitor (macOS)
//! read them, cheaply enough to sample every few seconds.
//!
//! A [`Sampler`] holds what the OS needs between reads (a PDH query, open
//! adapter handles, the previous counters) and each [`Sampler::sample`] gives
//! one [`Sample`]. Rates (CPU %, disk and network throughput) are deltas
//! between two reads, so the first sample after [`Sampler::new`] has them as
//! `None` or zero. Every source is optional: one that fails is left out of
//! the sample and the rest still read.
//!
//! | Meter | Windows | macOS |
//! |---|---|---|
//! | CPU % | PDH `Processor Information(_Total)\% Processor Utility` | `host_processor_info` tick deltas |
//! | CPU GHz | PDH `% Processor Performance` × base MHz (`CallNtPowerInformation`) | not read (no public source on Apple Silicon) |
//! | Memory | `GlobalMemoryStatusEx` | `hw.memsize`, `host_statistics64` (active + wired + compressed) |
//! | Disks | PDH `PhysicalDisk(*)` idle time and bytes/s | IOKit `IOBlockStorageDriver` Statistics |
//! | Network | `GetIfEntry2` octets on the up Ethernet and Wi-Fi adapters | `NET_RT_IFLIST2` 64-bit byte counters |
//! | GPUs | PDH `GPU Engine(*)` / `GPU Adapter Memory(*)`, D3DKMT names, VRAM size and temperature | IOKit `IOAccelerator` PerformanceStatistics |
//!
//! Other systems (Android, iOS, Linux) have no sampler: [`SUPPORTED`] is
//! false and [`Sampler::sample`] gives an empty sample.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use self::windows as imp;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use self::macos as imp;

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    pub(super) struct Sampler;
    impl Sampler {
        pub(super) fn new() -> Sampler {
            Sampler
        }
        pub(super) fn sample(&mut self, _out: &mut super::Sample) {}
    }
}

/// Whether this OS has a sampler at all.
pub const SUPPORTED: bool = cfg!(any(windows, target_os = "macos"));

/// One read of every meter.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sample {
    pub cpu: Cpu,
    pub mem: Mem,
    /// Physical disks, in the OS's order (disk 0 first).
    pub disks: Vec<Disk>,
    pub net: Net,
    /// GPUs, the one with the most dedicated memory first.
    pub gpus: Vec<Gpu>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cpu {
    /// Busy, 0 to 100 (Task Manager's "Utilization"); `None` on the first read.
    pub pct: Option<f32>,
    /// The current speed in GHz, where the OS gives one cheaply.
    pub ghz: Option<f32>,
    /// Logical processors.
    pub cores: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mem {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Disk {
    /// The OS's name for it: "0 C:" on Windows, "disk0" on macOS.
    pub name: String,
    /// Active time, 0 to 100 (Task Manager's "Active time"; approximate on macOS).
    pub active_pct: Option<f32>,
    pub read_bytes_per_s: u64,
    pub write_bytes_per_s: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Net {
    /// Received, bits per second, summed over the up physical interfaces.
    pub rx_bits_per_s: u64,
    /// Sent, bits per second.
    pub tx_bits_per_s: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Gpu {
    /// The adapter's name, e.g. "NVIDIA GeForce RTX 5080".
    pub name: String,
    /// Shares memory with the CPU (an integrated GPU, or Apple Silicon).
    pub integrated: bool,
    /// Busy, 0 to 100: the busiest engine, as Task Manager shows it.
    pub util_pct: Option<f32>,
    pub temp_c: Option<f32>,
    pub mem_used_bytes: Option<u64>,
    pub mem_total_bytes: Option<u64>,
}

/// Holds what the OS needs between reads. Build one and keep it: the first
/// read only primes the rates.
pub struct Sampler {
    imp: imp::Sampler,
}

impl Sampler {
    pub fn new() -> Sampler {
        Sampler {
            imp: imp::Sampler::new(),
        }
    }

    /// Reads every meter now.
    pub fn sample(&mut self) -> Sample {
        let mut out = Sample::default();
        self.imp.sample(&mut out);
        out
    }
}

impl Default for Sampler {
    fn default() -> Self {
        Sampler::new()
    }
}

/// A per-second rate from two cumulative counters `secs` apart; 0 when the
/// counter went backwards (a reset or a wrap) or no time passed.
#[allow(dead_code)]
pub(crate) fn rate(prev: u64, now: u64, secs: f64) -> u64 {
    if now < prev || secs <= 0.0 {
        return 0;
    }
    ((now - prev) as f64 / secs).round() as u64
}

/// Clamps a percentage to 0..=100, dropping NaN.
#[allow(dead_code)]
pub(crate) fn clamp_pct(v: f64) -> Option<f32> {
    if v.is_nan() {
        return None;
    }
    Some(v.clamp(0.0, 100.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_handles_resets() {
        assert_eq!(rate(100, 300, 2.0), 100);
        assert_eq!(rate(300, 100, 2.0), 0);
        assert_eq!(rate(1, 2, 0.0), 0);
    }

    #[test]
    fn clamp_pct_bounds() {
        assert_eq!(clamp_pct(113.4), Some(100.0));
        assert_eq!(clamp_pct(-1.0), Some(0.0));
        assert_eq!(clamp_pct(f64::NAN), None);
    }

    /// Reads this machine twice and prints it (`--nocapture` to see it).
    #[test]
    fn samples_this_machine() {
        let mut s = Sampler::new();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let x = s.sample();
        println!("{x:#?}");
        if SUPPORTED {
            assert!(x.mem.total_bytes > 0);
            assert!(x.mem.used_bytes <= x.mem.total_bytes);
            assert!(x.cpu.cores > 0);
            assert!(x.cpu.pct.is_some());
        }
    }
}
