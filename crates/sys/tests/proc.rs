//! Ports of I:internal/proc/proc_test.go, wait_test.go and
//! filetime_windows_test.go.

use std::process::{Child, Command};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use sys::cancel::Cancel;
use sys::proc::{self, WaitError};

#[test]
fn self_is_alive() {
    assert!(proc::alive(i64::from(std::process::id())));
    assert!(!proc::alive(0) && !proc::alive(-1));
}

/// The test harness's own parent (cargo) is running, and is not us.
#[test]
fn parent_is_alive() {
    let pp = proc::parent_pid();
    assert!(pp > 0 && pp != i64::from(std::process::id()), "parent {pp}");
    assert!(proc::alive(pp));
}

#[test]
fn self_start_time() {
    let Some(st) = proc::start_time(i64::from(std::process::id())) else {
        return; // not supported on this OS
    };
    let age = SystemTime::now()
        .duration_since(st)
        .expect("start time in the future");
    assert!(age < Duration::from_secs(3600), "{age:?}");
}

/// procStart in Claude Code's session files is FILETIME ticks.
#[cfg(windows)]
#[test]
fn filetime_matches_claude_units() {
    let st = proc::start_time(i64::from(std::process::id())).expect("start time on windows");
    let ticks = proc::filetime_ticks(st);
    assert_eq!(ticks.to_string().len(), 18, "{ticks}");
    let back = proc::from_filetime_ticks(ticks);
    let d = st.duration_since(back).unwrap_or_else(|e| e.duration());
    assert!(d < Duration::from_micros(1));
}

/// The child the wait test spawns: sleeps IRORI_PROC_SLEEP_MS and exits.
/// It does nothing in a normal run.
#[test]
fn helper_sleep() {
    if let Ok(ms) = std::env::var("IRORI_PROC_SLEEP_MS") {
        std::thread::sleep(Duration::from_millis(ms.parse().unwrap()));
        std::process::exit(0);
    }
}

struct Kid(Child);

impl Drop for Kid {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn child(ms: u64) -> Kid {
    let exe = std::env::current_exe().unwrap();
    Kid(Command::new(exe)
        .args(["--exact", "helper_sleep", "--nocapture"])
        .env("IRORI_PROC_SLEEP_MS", ms.to_string())
        .spawn()
        .unwrap())
}

fn wait_async(
    cancel: Cancel,
    pid: i64,
    by: Option<SystemTime>,
) -> mpsc::Receiver<Result<(), WaitError>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(proc::wait_exit(&cancel, pid, by));
    });
    rx
}

#[test]
fn wait_exit() {
    let me = i64::from(std::process::id());
    let probe = proc::wait_exit(
        &Cancel::new(),
        me,
        Some(SystemTime::now() - Duration::from_secs(3600)),
    );
    if probe == Err(WaitError::CantWait) {
        return; // not supported on this OS
    }

    // Exits by itself.
    let c = child(300);
    let got = wait_async(Cancel::new(), i64::from(c.0.id()), Some(SystemTime::now()));
    assert_eq!(got.recv_timeout(Duration::from_secs(5)).unwrap(), Ok(()));

    // Killed.
    let mut c = child(3_600_000);
    let got = wait_async(Cancel::new(), i64::from(c.0.id()), Some(SystemTime::now()));
    std::thread::sleep(Duration::from_millis(200));
    c.0.kill().unwrap();
    assert_eq!(got.recv_timeout(Duration::from_secs(3)).unwrap(), Ok(()));

    // Canceled.
    let c = child(3_600_000);
    let cancel = Cancel::new();
    let got = wait_async(cancel.clone(), i64::from(c.0.id()), Some(SystemTime::now()));
    std::thread::sleep(Duration::from_millis(200));
    cancel.cancel();
    assert_eq!(
        got.recv_timeout(Duration::from_secs(3)).unwrap(),
        Err(WaitError::Canceled)
    );
    drop(c);

    // Reused pid: the live child started after the bound, so it can't be
    // the process meant, and that one is gone.
    let before = SystemTime::now() - Duration::from_secs(60);
    let c = child(3_600_000);
    let got = wait_async(Cancel::new(), i64::from(c.0.id()), Some(before));
    assert_eq!(got.recv_timeout(Duration::from_secs(2)).unwrap(), Ok(()));
    drop(c);
}

/// sysctl reads another user's process (launchd, root's pid 1), which
/// `proc_pidinfo` refuses.
#[cfg(target_os = "macos")]
#[test]
fn macos_start_time_of_another_user() {
    let st = proc::start_time(1).expect("launchd's start time");
    assert!(st < SystemTime::now());
    assert!(proc::start_time(i64::from(u32::MAX)).is_none());
}

/// On macOS an exited child its parent hasn't reaped yet is gone, so the
/// liveness check agrees with the exit wait.
#[cfg(target_os = "macos")]
#[test]
fn macos_zombie_is_gone() {
    let mut c = Command::new("/usr/bin/true").spawn().unwrap();
    let pid = i64::from(c.id());
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while proc::alive(pid) {
        assert!(std::time::Instant::now() < deadline, "zombie still alive");
        std::thread::sleep(Duration::from_millis(10));
    }
    // Still unreaped: wait collects it now.
    assert!(c.try_wait().unwrap().is_some());
    assert_eq!(
        proc::wait_exit(&Cancel::new(), pid, None),
        Ok(()),
        "a reaped pid's wait returns at once"
    );
}
