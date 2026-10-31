//! Process liveness and exit waits (Go `live_*.go`, `exit_*.go` and the
//! parts of `internal/proc` they use; capture.md §4, mesh.md §4).
//!
//! Windows is built in. macOS goes through `sys::proc`: sysctl liveness
//! and start time, and a kqueue exit wait. Other Unix: pid only, as Go.

use crate::{ExitFn, LiveFn};
use std::sync::Arc;

/// How far a start time may drift from Claude's procStart and still be
/// the same process: Windows FILETIME ticks (1 s); macOS seconds (2 s).
#[cfg(windows)]
pub const PROC_START_SLACK: u64 = 10_000_000;
#[cfg(not(windows))]
pub const PROC_START_SLACK: u64 = 2;

/// The default liveness check.
pub fn default_live() -> LiveFn {
    Arc::new(live)
}

/// The default exit wait, where this OS has one.
pub fn default_watch_exit() -> Option<ExitFn> {
    #[cfg(windows)]
    {
        Some(Arc::new(win::watch_exit))
    }
    #[cfg(target_os = "macos")]
    {
        Some(Arc::new(mac::watch_exit))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

/// Go `live(pid, procStart)`: pid runs and, when the session recorded a
/// procStart, is the same process that wrote it (not a reused pid).
#[cfg(windows)]
fn live(pid: i64, proc_start: u64) -> bool {
    if !win::alive(pid) {
        return false;
    }
    if proc_start == 0 {
        return true;
    }
    let Some(ticks) = win::start_ticks(pid) else {
        // Alive but unreadable (another user's process): trust Alive.
        return true;
    };
    ticks.abs_diff(proc_start) <= PROC_START_SLACK
}

/// macOS: procStart is Unix seconds from `ps -o lstart=`; larger values
/// are some other unit and trusted like an absent one.
#[cfg(target_os = "macos")]
fn live(pid: i64, proc_start: u64) -> bool {
    if !mac::alive(pid) {
        return false;
    }
    if proc_start == 0 || proc_start > 1 << 36 {
        return true;
    }
    let Some(st) = mac::start_secs(pid) else {
        return true;
    };
    st.abs_diff(proc_start) <= PROC_START_SLACK
}

/// Elsewhere procStart's unit is not known, so pid reuse goes undetected.
#[cfg(all(unix, not(target_os = "macos")))]
fn live(pid: i64, _: u64) -> bool {
    if pid <= 0 || pid > i64::from(i32::MAX) {
        return false;
    }
    // SAFETY: kill with signal 0 only checks the pid.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// FILETIME ticks of a process's start (Go `FiletimeTicks(StartTime(pid))`).
#[cfg(windows)]
pub fn process_start_ticks(pid: i64) -> Option<u64> {
    win::start_ticks(pid)
}
/// Unix seconds of a process's start, where the OS tells.
#[cfg(not(windows))]
pub fn process_start_ticks(pid: i64) -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        mac::start_secs(pid)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        None
    }
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, FILETIME, GetLastError, HANDLE, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Threading::{
        CreateEventW, GetExitCodeProcess, GetProcessTimes, INFINITE, OpenProcess,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, SetEvent, WaitForMultipleObjects,
    };

    const STILL_ACTIVE: u32 = 259;

    fn pid32(pid: i64) -> Option<u32> {
        (pid > 0 && pid <= i64::from(u32::MAX)).then_some(pid as u32)
    }

    /// Go `proc.Alive`.
    pub(super) fn alive(pid: i64) -> bool {
        let Some(pid) = pid32(pid) else {
            return false;
        };
        // SAFETY: handle checked and closed below.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                // Access denied means it exists but belongs to someone else.
                return GetLastError() == ERROR_ACCESS_DENIED;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code) != 0;
            CloseHandle(h);
            ok && code == STILL_ACTIVE
        }
    }

    /// The creation FILETIME of a process.
    pub(super) fn start_ticks(pid: i64) -> Option<u64> {
        let pid = pid32(pid)?;
        // SAFETY: handle checked and closed below; out-params are ours.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut c: FILETIME = std::mem::zeroed();
            let mut e: FILETIME = std::mem::zeroed();
            let mut k: FILETIME = std::mem::zeroed();
            let mut u: FILETIME = std::mem::zeroed();
            let ok = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) != 0;
            CloseHandle(h);
            ok.then(|| (u64::from(c.dwHighDateTime) << 32) | u64::from(c.dwLowDateTime))
        }
    }

    /// A HANDLE moved to the wait thread.
    struct Sendable(HANDLE);
    // SAFETY: kernel handles may be used from any thread.
    unsafe impl Send for Sendable {}

    /// Go `watchExit` (exit_windows.go): OpenProcess(SYNCHRONIZE) and a
    /// stop event, waited on together by one thread.
    pub(super) fn watch_exit(
        pid: i64,
        on_exit: Box<dyn Fn() + Send + Sync>,
    ) -> Option<Box<dyn FnOnce() + Send>> {
        let pid = pid32(pid)?;
        // SAFETY: handles checked; each closed exactly once below.
        unsafe {
            let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if h.is_null() {
                return None;
            }
            let ev = CreateEventW(std::ptr::null(), 1, 0, std::ptr::null());
            if ev.is_null() {
                CloseHandle(h);
                return None;
            }
            let (hs, evs) = (Sendable(h), Sendable(ev));
            let t = std::thread::Builder::new()
                .name("presence-exit".into())
                .spawn(move || {
                    let (hs, evs) = (hs, evs);
                    let hs_arr = [hs.0, evs.0];
                    let w = WaitForMultipleObjects(2, hs_arr.as_ptr(), 0, INFINITE);
                    CloseHandle(hs.0);
                    if w == WAIT_OBJECT_0 {
                        on_exit();
                    }
                })
                .ok();
            let Some(t) = t else {
                CloseHandle(h);
                CloseHandle(ev);
                return None;
            };
            let evs = Sendable(ev);
            Some(Box::new(move || {
                let evs = evs;
                SetEvent(evs.0);
                let _ = t.join();
                CloseHandle(evs.0);
            }))
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::time::UNIX_EPOCH;

    use sys::cancel::Cancel;

    /// Go `proc.Alive` on macOS: a zombie counts as gone.
    pub(super) fn alive(pid: i64) -> bool {
        sys::proc::alive(pid)
    }

    /// The process start in Unix seconds (Go truncates to the second).
    pub(super) fn start_secs(pid: i64) -> Option<u64> {
        let st = sys::proc::start_time(pid)?;
        Some(st.duration_since(UNIX_EPOCH).ok()?.as_secs())
    }

    /// Go `watchExit` (exit_darwin.go): a kqueue NOTE_EXIT wait on a
    /// thread, which calls on_exit when the process ends. None when it is
    /// already gone; the safety rescan covers that, and a wait the kernel
    /// refuses.
    pub(super) fn watch_exit(
        pid: i64,
        on_exit: Box<dyn Fn() + Send + Sync>,
    ) -> Option<Box<dyn FnOnce() + Send>> {
        if !sys::proc::alive(pid) {
            return None;
        }
        let cancel = Cancel::new();
        let c2 = cancel.clone();
        let t = std::thread::Builder::new()
            .name("presence-exit".into())
            .spawn(move || {
                if sys::proc::wait_exit(&c2, pid, None).is_ok() && !c2.is_canceled() {
                    on_exit();
                }
            })
            .ok()?;
        Some(Box::new(move || {
            cancel.cancel();
            let _ = t.join();
        }))
    }
}
