//! "Is this process still the one I think it is?", a port of
//! I:internal/proc (capture.md §4).
//!
//! | | Windows | other Unix |
//! |---|---|---|
//! | [`alive`] | `OpenProcess(QUERY_LIMITED)`; access denied = alive; exit code 259 | `kill(pid, 0)`, EPERM = alive |
//! | [`start_time`] | `GetProcessTimes` creation time | `None` |
//! | [`wait_exit`] | `WaitForMultipleObjects([process, stop event])` | [`WaitError::CantWait`] |
//!
//! macOS's sysctl `kinfo_proc` start time and zombie check and its kqueue
//! `NOTE_EXIT` wait are not ported yet: on macOS this behaves as other Unix
//! (callers poll [`alive`]).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::cancel::Cancel;

/// Why [`wait_exit`] returned without the process exiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitError {
    /// Go `ErrCantWait`: no way to wait on this pid here; poll [`alive`].
    CantWait,
    /// The token was canceled first (Go returns `ctx.Err()`).
    Canceled,
}

impl std::fmt::Display for WaitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            WaitError::CantWait => "proc: can't wait on this process",
            WaitError::Canceled => "context canceled",
        })
    }
}

impl std::error::Error for WaitError {}

/// Whether a process with `pid` exists. A process we may not signal still
/// counts; so does a zombie off macOS.
pub fn alive(pid: i64) -> bool {
    if pid <= 0 || pid > i64::from(u32::MAX) {
        return false;
    }
    imp::alive(pid)
}

/// When `pid` started, if the OS can tell cheaply.
pub fn start_time(pid: i64) -> Option<SystemTime> {
    if pid <= 0 || pid > i64::from(u32::MAX) {
        return None;
    }
    imp::start_time(pid)
}

/// Blocks until `pid` exits (`Ok`), or the token is canceled. Returns `Ok`
/// at once when `pid` is not running, or when the process now at `pid`
/// started after `started_by` (the pid was reused).
pub fn wait_exit(
    cancel: &Cancel,
    pid: i64,
    started_by: Option<SystemTime>,
) -> Result<(), WaitError> {
    if pid <= 0 {
        return Ok(());
    }
    if pid > i64::from(u32::MAX) {
        return Err(WaitError::CantWait);
    }
    imp::wait_exit(cancel, pid, started_by)
}

/// This process's parent pid (Go `os.Getppid`); 0 when it can't be told.
/// Windows never reparents: the pid stays the one that started us, even
/// after that process exits.
pub fn parent_pid() -> i64 {
    imp::parent_pid()
}

/// 100 ns ticks from 1601-01-01 to 1970-01-01.
const EPOCH_TICKS: u64 = 116_444_736_000_000_000;

/// `t` as a Windows FILETIME (100 ns ticks since 1601): the unit Claude Code
/// writes as `procStart` in its session files on Windows.
pub fn filetime_ticks(t: SystemTime) -> u64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => EPOCH_TICKS + (d.as_nanos() / 100) as u64,
        Err(e) => EPOCH_TICKS.saturating_sub((e.duration().as_nanos() / 100) as u64),
    }
}

/// The inverse of [`filetime_ticks`].
pub fn from_filetime_ticks(ticks: u64) -> SystemTime {
    if ticks >= EPOCH_TICKS {
        UNIX_EPOCH + Duration::from_nanos((ticks - EPOCH_TICKS).saturating_mul(100))
    } else {
        UNIX_EPOCH - Duration::from_nanos((EPOCH_TICKS - ticks).saturating_mul(100))
    }
}

#[cfg(windows)]
mod imp {
    use std::sync::{Arc, Mutex};
    use std::time::SystemTime;

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, FILETIME, GetLastError, HANDLE,
        WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Threading::{
        CreateEventW, GetExitCodeProcess, GetProcessTimes, INFINITE, OpenProcess,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, SetEvent, WaitForMultipleObjects,
    };

    use super::{WaitError, from_filetime_ticks};
    use crate::cancel::Cancel;

    const STILL_ACTIVE: u32 = 259;

    pub fn parent_pid() -> i64 {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        };
        let me = std::process::id();
        // SAFETY: a process snapshot; no pointer arguments.
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snap == INVALID_HANDLE_VALUE || snap.is_null() {
            return 0;
        }
        let snap = Handle(snap);
        // SAFETY: PROCESSENTRY32W is plain data; dwSize is set before use.
        let mut e: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        // SAFETY: a live snapshot handle and an entry with dwSize set.
        let mut ok = unsafe { Process32FirstW(snap.0, &mut e) } != 0;
        while ok {
            if e.th32ProcessID == me {
                return i64::from(e.th32ParentProcessID);
            }
            // SAFETY: as above.
            ok = unsafe { Process32NextW(snap.0, &mut e) } != 0;
        }
        0
    }

    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful Open/Create call.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    fn open(access: u32, pid: i64) -> Result<Handle, u32> {
        // SAFETY: OpenProcess has no pointer arguments.
        let h = unsafe { OpenProcess(access, 0, pid as u32) };
        if h.is_null() {
            // SAFETY: reads the calling thread's last error.
            return Err(unsafe { GetLastError() });
        }
        Ok(Handle(h))
    }

    fn created(h: &Handle) -> Option<SystemTime> {
        let z = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut c, mut e, mut k, mut u) = (z, z, z, z);
        // SAFETY: out-parameters on the stack, a live process handle.
        if unsafe { GetProcessTimes(h.0, &mut c, &mut e, &mut k, &mut u) } == 0 {
            return None;
        }
        let ticks = (u64::from(c.dwHighDateTime) << 32) | u64::from(c.dwLowDateTime);
        Some(from_filetime_ticks(ticks))
    }

    pub fn alive(pid: i64) -> bool {
        match open(PROCESS_QUERY_LIMITED_INFORMATION, pid) {
            Err(code) => code == ERROR_ACCESS_DENIED,
            Ok(h) => {
                let mut code = 0u32;
                // SAFETY: out-parameter, live handle.
                if unsafe { GetExitCodeProcess(h.0, &mut code) } == 0 {
                    return false;
                }
                code == STILL_ACTIVE
            }
        }
    }

    pub fn start_time(pid: i64) -> Option<SystemTime> {
        let h = open(PROCESS_QUERY_LIMITED_INFORMATION, pid).ok()?;
        created(&h)
    }

    struct SendHandle(HANDLE);
    // SAFETY: an event handle may be signalled from any thread.
    unsafe impl Send for SendHandle {}

    pub fn wait_exit(
        cancel: &Cancel,
        pid: i64,
        started_by: Option<SystemTime>,
    ) -> Result<(), WaitError> {
        let h = match open(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, pid) {
            Ok(h) => h,
            Err(ERROR_INVALID_PARAMETER) => return Ok(()), // no process has this pid
            Err(_) => return Err(WaitError::CantWait),
        };
        if let Some(by) = started_by
            && let Some(st) = created(&h)
            && st > by
        {
            return Ok(()); // pid reused: the process we meant is gone
        }
        // SAFETY: an unnamed manual-reset event.
        let stop = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if stop.is_null() {
            return Err(WaitError::CantWait);
        }
        let stop = Handle(stop);
        // The callback may run on another thread after this function has
        // returned; it signals the event only while the handle is open.
        let shared = Arc::new(Mutex::new(Some(SendHandle(stop.0))));
        let s2 = shared.clone();
        let reg = cancel.on_cancel(move || {
            if let Some(h) = s2.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
                // SAFETY: the handle is open while it is held here.
                unsafe {
                    SetEvent(h.0);
                }
            }
        });
        let handles = [h.0, stop.0];
        // SAFETY: two live handles.
        let r = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
        drop(reg);
        shared.lock().unwrap_or_else(|p| p.into_inner()).take();
        if r == WAIT_OBJECT_0 {
            return Ok(());
        }
        if r == WAIT_OBJECT_0 + 1 {
            return Err(WaitError::Canceled);
        }
        Err(WaitError::CantWait)
    }
}

#[cfg(unix)]
mod imp {
    use std::time::SystemTime;

    use super::WaitError;
    use crate::cancel::Cancel;

    pub fn parent_pid() -> i64 {
        i64::from(std::os::unix::process::parent_id())
    }

    pub fn alive(pid: i64) -> bool {
        // SAFETY: kill with signal 0 only checks.
        let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
        r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    pub fn start_time(_: i64) -> Option<SystemTime> {
        None
    }

    pub fn wait_exit(_: &Cancel, _: i64, _: Option<SystemTime>) -> Result<(), WaitError> {
        Err(WaitError::CantWait)
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    use std::time::SystemTime;

    use super::WaitError;
    use crate::cancel::Cancel;

    pub fn parent_pid() -> i64 {
        0
    }

    pub fn alive(_: i64) -> bool {
        false
    }

    pub fn start_time(_: i64) -> Option<SystemTime> {
        None
    }

    pub fn wait_exit(_: &Cancel, _: i64, _: Option<SystemTime>) -> Result<(), WaitError> {
        Err(WaitError::CantWait)
    }
}
