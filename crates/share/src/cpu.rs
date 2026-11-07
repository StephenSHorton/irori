//! This process's CPU time (user + system), for measuring what a share costs.

use std::time::Duration;

/// CPU time this process has used so far, across all its threads.
#[cfg(unix)]
pub fn process_time() -> Duration {
    // SAFETY: getrusage fills the struct we hand it and touches nothing else.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        return Duration::ZERO;
    }
    let tv = |t: libc::timeval| {
        Duration::from_secs(t.tv_sec as u64) + Duration::from_micros(t.tv_usec as u64)
    };
    tv(usage.ru_utime) + tv(usage.ru_stime)
}

/// CPU time this process has used so far, across all its threads.
#[cfg(windows)]
pub fn process_time() -> Duration {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
    // SAFETY: the pseudo-handle needs no closing, and the call only writes
    // the four FILETIMEs.
    if unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) } == 0 {
        return Duration::ZERO;
    }
    let ticks = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
    // FILETIME counts 100 ns ticks.
    Duration::from_nanos((ticks(k) + ticks(u)) * 100)
}

/// How many logical CPUs this machine has, to put CPU time in context.
pub fn cores() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// Measures CPU use between two points.
pub struct Meter {
    wall: std::time::Instant,
    cpu: Duration,
}

impl Meter {
    pub fn start() -> Self {
        Self {
            wall: std::time::Instant::now(),
            cpu: process_time(),
        }
    }

    /// CPU time over wall time since `start`, as a percent of one core
    /// (200 means two cores busy).
    pub fn percent_of_one_core(&self) -> f64 {
        let wall = self.wall.elapsed().as_secs_f64();
        if wall <= 0.0 {
            return 0.0;
        }
        (process_time() - self.cpu).as_secs_f64() / wall * 100.0
    }

    pub fn cpu(&self) -> Duration {
        process_time() - self.cpu
    }

    pub fn wall(&self) -> Duration {
        self.wall.elapsed()
    }
}

/// CPU used by helper processes this one drives. On the Mac, VideoToolbox's
/// hardware encoder runs in a `VTEncoderXPCService` process launched for its
/// client, not in ours, so [`Meter`] alone misses it.
#[cfg(target_os = "macos")]
pub struct ServiceMeter {
    name: &'static str,
    /// Each matching process's CPU time when the meter started.
    before: std::collections::HashMap<i32, Duration>,
}

#[cfg(target_os = "macos")]
impl ServiceMeter {
    /// Start counting the CPU of processes called `name`; ones that start
    /// later count from zero. An instance shared with another client would
    /// count that client's work too: run the bench with nothing else encoding.
    pub fn start(name: &'static str) -> Self {
        let before = pids_named(name)
            .into_iter()
            .filter_map(|p| Some((p, pid_cpu(p)?)))
            .collect();
        Self { name, before }
    }

    /// CPU those processes used since `start` (exited ones drop out, so read
    /// it while the encoder is still open).
    pub fn cpu(&self) -> Duration {
        pids_named(self.name)
            .into_iter()
            .filter_map(|p| {
                let now = pid_cpu(p)?;
                Some(now.saturating_sub(self.before.get(&p).copied().unwrap_or_default()))
            })
            .sum()
    }
}

#[cfg(target_os = "macos")]
fn pids_named(name: &str) -> Vec<i32> {
    // SAFETY: proc_listallpids fills at most the buffer it is given and
    // returns how many it wrote; proc_name writes at most `len` bytes.
    unsafe {
        let n = libc::proc_listallpids(std::ptr::null_mut(), 0);
        if n <= 0 {
            return Vec::new();
        }
        let mut pids = vec![0i32; n as usize + 64];
        let bytes = (pids.len() * std::mem::size_of::<i32>()) as libc::c_int;
        let got = libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes);
        pids.truncate(got.max(0) as usize);
        pids.retain(|&p| {
            let mut buf = [0u8; 256];
            let len = libc::proc_name(p, buf.as_mut_ptr().cast(), buf.len() as u32);
            len > 0 && &buf[..len as usize] == name.as_bytes()
        });
        pids
    }
}

#[cfg(target_os = "macos")]
// libc marks mach_timebase_info deprecated in favour of the mach2 crate;
// the call is the same.
#[allow(deprecated)]
fn pid_cpu(pid: i32) -> Option<Duration> {
    // SAFETY: proc_pid_rusage writes one rusage_info_v2 into the struct we
    // pass; mach_timebase_info fills the struct we pass.
    unsafe {
        let mut info: libc::rusage_info_v2 = std::mem::zeroed();
        let ptr = (&mut info as *mut libc::rusage_info_v2).cast();
        if libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V2, ptr) != 0 {
            return None;
        }
        // ri_*_time are in Mach absolute time units (not ns on Apple silicon).
        let mut tb = libc::mach_timebase_info { numer: 0, denom: 0 };
        libc::mach_timebase_info(&mut tb);
        if tb.denom == 0 {
            return None;
        }
        let ticks = u128::from(info.ri_user_time) + u128::from(info.ri_system_time);
        let ns = ticks * u128::from(tb.numer) / u128::from(tb.denom);
        Some(Duration::from_nanos(ns as u64))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn busy_loop_shows_cpu_time() {
        // A fixed amount of CPU work, not a fixed wall time: on a loaded
        // machine it only takes longer. The deadline catches a clock that
        // never moves.
        let meter = super::Meter::start();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut x = 0u64;
        while meter.cpu() < std::time::Duration::from_millis(20) {
            assert!(
                std::time::Instant::now() < deadline,
                "cpu time never advanced: {:?}",
                meter.cpu()
            );
            for _ in 0..10_000 {
                x = x.wrapping_mul(31).wrapping_add(7);
                std::hint::black_box(x);
            }
        }
        assert!(meter.percent_of_one_core() > 0.0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn service_meter_reads_another_process() {
        // This test process stands in for a service: pid_cpu must read it.
        let me = std::process::id() as i32;
        assert!(super::pid_cpu(me).is_some());
    }
}
