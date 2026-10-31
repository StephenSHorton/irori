//! Ports of I:internal/fswatch/fswatch_test.go and fswatch_windows_test.go
//! (the coalescer's own tests are unit tests in src/fswatch/coalesce.rs).

use std::path::Path;
use std::time::{Duration, Instant};

use sys::cancel::Cancel;
use sys::fswatch::{self, Error, Watcher};

fn p(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Waits for an event naming `path` (or an Overflow).
fn wait_for(w: &Watcher, path: &str) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match w.recv_timeout(left) {
            Ok(Some(e)) if e.path == path || e.overflow => return,
            Ok(Some(_)) => {}
            Ok(None) => panic!("watcher closed"),
            Err(_) => panic!("no event for {path}"),
        }
    }
}

fn start(root: &str, recursive: bool, cancel: &Cancel) -> Option<Watcher> {
    match fswatch::watch(cancel, root, recursive, Duration::ZERO) {
        Err(Error::Unsupported) => None,
        Err(e) => panic!("{e}"),
        Ok(w) => Some(w),
    }
}

#[test]
fn watch_sees_create_append_and_nested() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::path::absolute(dir.path()).unwrap();
    let Some(w) = start(&p(&root), true, &Cancel::new()) else {
        return;
    };
    let f = root.join("a.jsonl");
    std::fs::write(&f, "one\n").unwrap();
    wait_for(&w, &p(&f));

    // An append that opens and closes the file, as Claude Code and git do.
    {
        use std::io::Write;
        let mut h = std::fs::OpenOptions::new().append(true).open(&f).unwrap();
        h.write_all(b"two\n").unwrap();
    }
    wait_for(&w, &p(&f));

    let sub = root.join("s").join("subagents");
    std::fs::create_dir_all(&sub).unwrap();
    let g = sub.join("agent-x.jsonl");
    std::fs::write(&g, "x\n").unwrap();
    wait_for(&w, &p(&g));
    w.close();
}

#[test]
fn close_and_context_stop() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = Cancel::new();
    let Some(w) = start(&p(dir.path()), false, &cancel) else {
        return;
    };
    cancel.cancel();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match w.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(None) => break,
            Ok(Some(_)) => {}
            Err(_) => panic!("events not closed after cancel"),
        }
    }
    w.close(); // idempotent after the token stopped it
    w.close();
}

/// A non-recursive watch reports root's direct children and not the files
/// below them.
#[test]
fn non_recursive_skips_subtree() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::path::absolute(dir.path()).unwrap();
    let sub = root.join("sub");
    std::fs::create_dir(&sub).unwrap();
    let Some(w) = start(&p(&root), false, &Cancel::new()) else {
        return;
    };
    let deep = p(&sub.join("deep.txt"));
    std::fs::write(&deep, "x").unwrap();
    let top = p(&root.join("top.txt"));
    std::fs::write(&top, "x").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match w.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Some(e)) => {
                assert_ne!(e.path, deep, "non-recursive watch reported {deep}");
                if e.path == top {
                    break;
                }
            }
            Ok(None) => panic!("watcher closed"),
            Err(_) => panic!("no event for {top}"),
        }
    }
    w.close();
}

/// Close joins the watcher's threads (Go: no goroutine left behind), and the
/// queue then reads as closed.
#[test]
fn close_joins_after_events() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::path::absolute(dir.path()).unwrap();
    let Some(w) = start(&p(&root), true, &Cancel::new()) else {
        return;
    };
    let f = p(&root.join("a.txt"));
    std::fs::write(&f, "x").unwrap();
    wait_for(&w, &f);
    let t = Instant::now();
    w.close();
    assert!(t.elapsed() < Duration::from_secs(3));
    loop {
        match w.recv_timeout(Duration::from_secs(1)) {
            Ok(None) => break,
            Ok(Some(_)) => {}
            Err(_) => panic!("queue not closed after Close"),
        }
    }
}

/// A watcher that stopped on its own (its directory went away) has closed
/// its handles; stopping it again must not signal whatever event object
/// Windows handed those handle values to next.
#[cfg(windows)]
#[test]
fn stop_after_self_close_signals_nothing() {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    for how in ["Close", "ctx"] {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("w");
        std::fs::create_dir(&dir).unwrap();
        let cancel = Cancel::new();
        let w = start(&p(&dir), true, &cancel).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match w.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(None) => break,
                Ok(Some(_)) => {}
                Err(_) => panic!("watcher on a removed directory didn't stop ({how})"),
            }
        }
        // SAFETY: unnamed events, closed below.
        let probes: Vec<_> = (0..8)
            .map(|_| unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) })
            .collect();
        if how == "Close" {
            w.close();
        } else {
            cancel.cancel();
            std::thread::sleep(Duration::from_millis(100));
            w.close();
        }
        for (i, h) in probes.iter().enumerate() {
            // SAFETY: a live event handle.
            let r = unsafe { WaitForSingleObject(*h, 0) };
            assert_eq!(r, WAIT_TIMEOUT, "probe {i} was signalled ({how})");
            // SAFETY: closed once.
            unsafe { CloseHandle(*h) };
        }
    }
}

/// The Mac has a watcher (FSEvents), events come back under the root as
/// given (the tempdir is under /var, which FSEvents reports as
/// /private/var), and removing the root reports an Overflow.
#[cfg(target_os = "macos")]
#[test]
fn macos_fsevents_root_as_given_and_root_removed() {
    let base = tempfile::tempdir().unwrap();
    let dir = std::path::absolute(base.path()).unwrap().join("w");
    std::fs::create_dir(&dir).unwrap();
    let w = start(&p(&dir), true, &Cancel::new()).expect("FSEvents on macOS");
    let f = p(&dir.join("x.txt"));
    std::fs::write(&f, "x").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match w.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Some(e)) if e.path == f => break,
            Ok(Some(e)) => assert!(e.path.starts_with(&p(&dir)), "{e:?}"),
            Ok(None) => panic!("watcher closed"),
            Err(_) => panic!("no event for {f}"),
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match w.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Some(e)) if e.overflow => {
                assert_eq!(e.path, p(&dir));
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) => panic!("watcher closed"),
            Err(_) => panic!("no Overflow for the removed root"),
        }
    }
    w.close();
}
