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

#[cfg(test)]
mod tests {
    #[test]
    fn busy_loop_shows_cpu_time() {
        let meter = super::Meter::start();
        let mut x = 0u64;
        while meter.wall().as_millis() < 50 {
            x = x.wrapping_mul(31).wrapping_add(7);
            std::hint::black_box(x);
        }
        assert!(meter.cpu().as_millis() >= 20, "cpu {:?}", meter.cpu());
        assert!(meter.percent_of_one_core() > 30.0);
    }
}
