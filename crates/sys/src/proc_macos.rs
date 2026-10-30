//! I:internal/proc/proc_darwin.go and wait_darwin.go: the start time and
//! zombie state from sysctl `kern.proc.pid` (readable for any user's
//! process, unlike `proc_pidinfo`), and the exit wait on a kqueue.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::WaitError;
use crate::cancel::Cancel;

/// `sizeof(struct kinfo_proc)`, the same on arm64 and x86_64.
const KINFO_PROC_SIZE: usize = 648;
/// Offsets into it: `kp_proc.p_starttime` (a timeval), `kp_proc.p_stat`
/// and `kp_proc.p_pid`.
const OFF_START: usize = 0;
const OFF_STAT: usize = 36;
const OFF_PID: usize = 40;
/// `p_stat` of a process that has exited and not been reaped (SZOMB).
const SZOMB: u8 = 5;

/// The EVFILT_USER event that ends a wait when its token is canceled.
const WAKE_IDENT: usize = 1;

pub fn parent_pid() -> i64 {
    i64::from(std::os::unix::process::parent_id())
}

/// A pid that fits `pid_t` (a larger one would turn negative and name a
/// process group).
fn fits(pid: i64) -> bool {
    pid > 0 && pid <= i64::from(libc::pid_t::MAX)
}

/// The pid's `kinfo_proc`, when the kernel has a process at it.
fn kinfo(pid: i64) -> Option<[u8; KINFO_PROC_SIZE]> {
    if !fits(pid) {
        return None;
    }
    let mut mib = [
        libc::CTL_KERN,
        libc::KERN_PROC,
        libc::KERN_PROC_PID,
        pid as libc::c_int,
    ];
    let mut buf = [0u8; KINFO_PROC_SIZE];
    let mut n = KINFO_PROC_SIZE;
    // SAFETY: the kernel writes at most n bytes into buf.
    let r = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as libc::c_uint,
            buf.as_mut_ptr().cast(),
            &mut n,
            std::ptr::null_mut(),
            0,
        )
    };
    let at = |o: usize| i32::from_ne_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]);
    (r == 0 && n == KINFO_PROC_SIZE && i64::from(at(OFF_PID)) == pid).then_some(buf)
}

/// kill(pid, 0), except that a zombie counts as gone: it has exited and
/// only its parent has yet to collect it.
pub fn alive(pid: i64) -> bool {
    if !fits(pid) {
        return false;
    }
    // SAFETY: kill with signal 0 only checks.
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if r != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) {
        return false;
    }
    !kinfo(pid).is_some_and(|k| k[OFF_STAT] == SZOMB)
}

pub fn start_time(pid: i64) -> Option<SystemTime> {
    let k = kinfo(pid)?;
    let b = &k[OFF_START..];
    let sec = i64::from_ne_bytes(b[..8].try_into().ok()?);
    let usec = i32::from_ne_bytes(b[8..12].try_into().ok()?);
    if sec <= 0 && usec == 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_secs(sec as u64) + Duration::from_micros(usec as u64))
}

fn kevent(ident: usize, filter: i16, flags: u16, fflags: u32) -> libc::kevent {
    libc::kevent {
        ident,
        filter,
        flags,
        fflags,
        data: 0,
        udata: std::ptr::null_mut(),
    }
}

/// Registers or triggers `ev` on kq.
fn change(kq: libc::c_int, ev: &libc::kevent) -> std::io::Result<()> {
    // SAFETY: one change, no event list.
    let r = unsafe { libc::kevent(kq, ev, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
    if r < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

struct Kq(libc::c_int);

impl Drop for Kq {
    fn drop(&mut self) {
        // SAFETY: the descriptor came from kqueue() and is closed once.
        unsafe {
            libc::close(self.0);
        }
    }
}

/// Waits on a kqueue for EVFILT_PROC NOTE_EXIT on pid, with an EVFILT_USER
/// event the token triggers. A kqueue registration is by pid, not a handle,
/// so the start-time check runs after it: a process at pid that started
/// after `started_by` is a reuse, and the one meant is gone.
pub fn wait_exit(
    cancel: &Cancel,
    pid: i64,
    started_by: Option<SystemTime>,
) -> Result<(), WaitError> {
    if !fits(pid) {
        return Err(WaitError::CantWait);
    }
    // SAFETY: no arguments; the descriptor is owned by Kq.
    let fd = unsafe { libc::kqueue() };
    if fd < 0 {
        return Err(WaitError::CantWait);
    }
    let kq = Kq(fd);
    let wake = kevent(
        WAKE_IDENT,
        libc::EVFILT_USER,
        libc::EV_ADD | libc::EV_CLEAR,
        0,
    );
    if change(kq.0, &wake).is_err() {
        return Err(WaitError::CantWait);
    }
    let exit = kevent(
        pid as usize,
        libc::EVFILT_PROC,
        libc::EV_ADD | libc::EV_ONESHOT,
        libc::NOTE_EXIT,
    );
    if let Err(e) = change(kq.0, &exit) {
        if e.raw_os_error() == Some(libc::ESRCH) {
            return Ok(()); // no process has this pid
        }
        return Err(WaitError::CantWait);
    }
    if let Some(by) = started_by
        && let Some(st) = start_time(pid)
        && st > by
    {
        return Ok(()); // pid reused: the process we meant is gone
    }

    // The callback may run on another thread; it touches kq only while the
    // descriptor is in the slot, which is emptied before kq closes.
    let slot = Arc::new(Mutex::new(Some(kq.0)));
    let s2 = slot.clone();
    let reg = cancel.on_cancel(move || {
        if let Some(fd) = *s2.lock().unwrap_or_else(|p| p.into_inner()) {
            let _ = change(
                fd,
                &kevent(WAKE_IDENT, libc::EVFILT_USER, 0, libc::NOTE_TRIGGER),
            );
        }
    });
    let r = loop {
        let mut evs = [kevent(0, 0, 0, 0), kevent(0, 0, 0, 0)];
        // SAFETY: no changes; up to two events into evs.
        let n = unsafe {
            libc::kevent(
                kq.0,
                std::ptr::null(),
                0,
                evs.as_mut_ptr(),
                2,
                std::ptr::null(),
            )
        };
        if n < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break Err(WaitError::CantWait);
        }
        if evs[..n as usize]
            .iter()
            .any(|e| e.filter == libc::EVFILT_PROC)
        {
            break Ok(());
        }
        if cancel.is_canceled() {
            break Err(WaitError::Canceled);
        }
    };
    drop(reg);
    slot.lock().unwrap_or_else(|p| p.into_inner()).take();
    r
}
