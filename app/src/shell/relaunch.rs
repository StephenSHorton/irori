//! Reset opens Irori again on its own (card #1095). Reset only marks the
//! data dir (`reset-pending`) and quits; the next start does the Reset. So
//! before quitting, the app starts a new copy of itself that waits for this
//! one to exit, then starts as usual and finds the mark.
//!
//! The copy is the same exe, spawned directly with this process's
//! environment, so a scratch run (`IRORI_HOME`, `CLAUDE_CONFIG_DIR`, the
//! port offset) comes back on its own copy. `open` on macOS would drop the
//! environment and come back on the live `~/.irori`. It gets no arguments,
//! so it shows its window even when this one started at login.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Set on the new copy: the pid it waits for before it starts.
pub const AFTER_ENV: &str = "IRORI_RELAUNCH_AFTER";

/// The longest the new copy waits, as the updater's swap helper does; then
/// it starts anyway (and hands off if the old one still runs).
const WAIT: Duration = Duration::from_secs(120);

/// Set once a copy is started: the quit then installs no update
/// (`install_on_exit`), which would swap the app under the copy. The copy
/// checks for the update again and offers it.
static STARTED: AtomicBool = AtomicBool::new(false);

/// Whether a copy was started to open Irori again.
pub fn started() -> bool {
    STARTED.load(Ordering::SeqCst)
}

/// Starts the new copy and returns its pid. Its stdio goes nowhere and it
/// has its own process group: it must outlive this process, which may own
/// a terminal that goes away with it.
pub fn spawn_after_exit() -> std::io::Result<u32> {
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe()?;
    let mut c = Command::new(exe);
    c.env(AFTER_ENV, std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        c.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    // Set before the spawn, so a quit already on its way can't install
    // under a copy that is starting.
    STARTED.store(true, Ordering::SeqCst);
    c.spawn().map(|child| child.id()).inspect_err(|_| {
        STARTED.store(false, Ordering::SeqCst);
    })
}

/// In the new copy: waits for the old one to exit, so its lockfile and
/// single-instance socket are gone and this launch doesn't hand off to it.
/// Takes the variable out first so nothing this process starts (agents,
/// a later Reset's copy) inherits it.
///
/// # Safety
/// Changes the environment: call it before any other thread starts.
pub unsafe fn wait_if_relaunched() {
    let Some(v) = std::env::var_os(AFTER_ENV) else {
        return;
    };
    // SAFETY: the caller's promise, no other thread yet.
    unsafe { std::env::remove_var(AFTER_ENV) };
    let Some(pid) = v.to_str().and_then(|s| s.trim().parse::<i64>().ok()) else {
        return;
    };
    let start = Instant::now();
    if wait_gone(pid, WAIT, sys::proc::alive) {
        irori_core::log(&format!(
            "irori: reopened after Reset (pid {pid} exited in {} ms)",
            start.elapsed().as_millis()
        ));
    } else {
        irori_core::log(&format!(
            "irori: reopened after Reset, but pid {pid} still runs after {} s; starting anyway",
            WAIT.as_secs()
        ));
    }
}

/// Whether `pid` went away within `limit`.
fn wait_gone(pid: i64, limit: Duration, alive: impl Fn(i64) -> bool) -> bool {
    let start = Instant::now();
    while alive(pid) {
        if start.elapsed() >= limit {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn waits_until_gone() {
        let left = Cell::new(3u32);
        let alive = |_| {
            let n = left.get();
            left.set(n.saturating_sub(1));
            n > 0
        };
        assert!(wait_gone(7, Duration::from_secs(5), alive));
        assert_eq!(left.get(), 0);
    }

    #[test]
    fn gives_up_at_the_limit() {
        assert!(!wait_gone(7, Duration::from_millis(120), |_| true));
    }

    #[test]
    fn gone_at_once() {
        assert!(wait_gone(7, Duration::ZERO, |_| false));
    }
}
