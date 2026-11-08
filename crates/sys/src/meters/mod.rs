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
//! | Memory | `GlobalMemoryStatusEx` | `hw.memsize`, `host_statistics64` (app + wired + compressed, as Activity Monitor) |
//! | Disks | PDH `PhysicalDisk(*)` idle time and bytes/s | IOKit `IOBlockStorageDriver` Statistics (disk images left out) |
//! | Network | `GetIfEntry2` octets on the up Ethernet and Wi-Fi adapters | `NET_RT_IFLIST2` 64-bit byte counters |
//! | GPUs | PDH `GPU Engine(*)` / `GPU Adapter Memory(*)`, D3DKMT names, VRAM size and temperature | IOKit `IOAccelerator` PerformanceStatistics |
//!
//! [`quick`] is the cheap one-shot: free disk for a folder, available memory
//! and CPU busy, with no [`Sampler`], no window thread and no shared state.
//! Disk reads on every unix (`statfs` on macOS, `statvfs` elsewhere); memory
//! and CPU only where there is a sampler (Windows fills them in later). A
//! source that fails is `None`, never "all free".
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
    pub(super) fn cpu_now(_window: std::time::Duration) -> Option<super::Cpu> {
        None
    }
    pub(super) fn mem_now() -> Option<super::Mem> {
        None
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

/// Free and total space on a volume.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Space {
    /// What a normal user can still write (not root's reserve).
    pub free_bytes: u64,
    pub total_bytes: u64,
}

/// A cheap one-shot read of this device's room; each part is `None` when it
/// could not be read here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Quick {
    /// The volume holding the path asked about.
    pub disk: Option<Space>,
    /// Used memory as [`Mem`]; `None` when the read failed (a zero total or
    /// zero used counts as failed).
    pub mem: Option<Mem>,
    /// Busy over about a second.
    pub cpu: Option<Cpu>,
}

/// How long [`quick`] watches the CPU.
const QUICK_CPU_WINDOW: std::time::Duration = std::time::Duration::from_secs(1);

/// Reads free disk for `path` (its nearest existing parent when it doesn't
/// exist yet), memory and CPU now. Takes about a second, for the CPU window.
pub fn quick(path: &std::path::Path) -> Quick {
    let cpu = std::thread::scope(|s| {
        let h = s.spawn(|| imp::cpu_now(QUICK_CPU_WINDOW));
        let disk = space(path);
        let mem = imp::mem_now().filter(|m| m.total_bytes > 0 && m.used_bytes > 0);
        (h.join().ok().flatten(), disk, mem)
    });
    Quick {
        disk: cpu.1,
        mem: cpu.2,
        cpu: cpu.0,
    }
}

/// The nearest ancestor of `path` (itself first) that exists; a relative
/// path with nothing above it resolves to ".".
pub fn existing_ancestor(path: &std::path::Path) -> std::path::PathBuf {
    for a in path.ancestors() {
        if a.as_os_str().is_empty() {
            break;
        }
        if a.exists() {
            return a.to_path_buf();
        }
    }
    if path.is_relative() {
        ".".into()
    } else {
        path.ancestors().last().unwrap_or(path).to_path_buf()
    }
}

/// Free and total space of the volume holding `path`.
#[cfg(unix)]
pub fn space(path: &std::path::Path) -> Option<Space> {
    use std::os::unix::ffi::OsStrExt;
    if path.as_os_str().as_bytes().contains(&0) {
        return None;
    }
    let c = std::ffi::CString::new(existing_ancestor(path).as_os_str().as_bytes()).ok()?;
    #[cfg(target_os = "macos")]
    {
        // statvfs's block counts are 32-bit on the Mac; statfs's are 64.
        // SAFETY: a zeroed plain-data struct filled by statfs.
        let mut st: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: c is NUL-terminated and st is valid.
        if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        let bs = st.f_bsize as u64;
        space_from(st.f_bavail as u64, st.f_blocks as u64, bs)
    }
    #[cfg(not(target_os = "macos"))]
    {
        // SAFETY: a zeroed plain-data struct filled by statvfs.
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: c is NUL-terminated and st is valid.
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        #[allow(clippy::unnecessary_cast)]
        let (avail, blocks, bs) = (st.f_bavail as u64, st.f_blocks as u64, st.f_frsize as u64);
        space_from(avail, blocks, bs)
    }
}

/// Filled in by card B.
#[cfg(not(unix))]
pub fn space(_path: &std::path::Path) -> Option<Space> {
    None
}

#[cfg(unix)]
fn space_from(avail: u64, blocks: u64, block_size: u64) -> Option<Space> {
    let total = blocks.checked_mul(block_size)?;
    if total == 0 {
        return None;
    }
    Some(Space {
        free_bytes: avail.saturating_mul(block_size).min(total),
        total_bytes: total,
    })
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
    fn missing_path_resolves_to_its_parent() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("a").join("b");
        assert_eq!(existing_ancestor(&missing), dir.path());
        assert_eq!(existing_ancestor(dir.path()), dir.path());
    }

    #[cfg(unix)]
    #[test]
    fn quick_reads_this_machine() {
        let dir = tempfile::tempdir().unwrap();
        let q = quick(&dir.path().join("not").join("yet"));
        let d = q.disk.expect("disk");
        assert!(d.total_bytes > 0 && d.free_bytes <= d.total_bytes);
        assert_eq!(d.total_bytes, space(dir.path()).unwrap().total_bytes);
        if SUPPORTED {
            let m = q.mem.expect("mem");
            assert!(m.used_bytes > 0 && m.used_bytes <= m.total_bytes);
            let c = q.cpu.expect("cpu");
            assert!(c.cores > 0 && c.pct.is_some());
        } else {
            assert!(q.mem.is_none() && q.cpu.is_none());
        }
        // Interior NUL: an error, not a panic.
        assert!(space(std::path::Path::new("a\0b")).is_none());
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
