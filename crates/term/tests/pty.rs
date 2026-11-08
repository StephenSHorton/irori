//! A real shell on this machine (desktop only): open, type, read back,
//! reattach, exit, close.
#![cfg(not(any(target_os = "android", target_os = "ios")))]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use term::{OpenSpec, Scope, TermMsg, Terms, Viewer};

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(
            // A shell can take over 20s to answer on a busy PC.
            t.elapsed() < Duration::from_secs(60),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn card(id: &str, cwd: &std::path::Path) -> OpenSpec {
    OpenSpec {
        scope: Scope::Card(id.into()),
        cols: 80,
        rows: 24,
        cwd: Some(cwd.to_string_lossy().into_owned()),
        command: None,
    }
}

/// One test's shells at a time, so a test can tell its own bootstrap
/// folders from another's: they are named by pid, not by Terms.
static SERIAL: Mutex<()> = parking_lot::const_mutex(());

/// The bootstrap folders rsz-term made for this process's shells
/// (rsz-zdot-<pid>-<n>, rsz-bash-<pid>-<n>) that are still in the temp dir.
fn bootstrap_dirs() -> Vec<String> {
    let pid = std::process::id();
    let mine = [format!("rsz-zdot-{pid}-"), format!("rsz-bash-{pid}-")];
    std::fs::read_dir(std::env::temp_dir())
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| mine.iter().any(|m| n.starts_with(m)))
                .collect()
        })
        .unwrap_or_default()
}

/// A test's terminals and their folder. On drop, even when the test
/// fails, it closes every shell and waits for them to be gone: Windows
/// can't delete a folder in use, and TempDir's own drop would give up at
/// once and leave it in %TEMP% (card #556); and a shell removes its
/// bootstrap folder only as it exits, so a test binary that ends first
/// leaves rsz-zdot-* in the temp dir (card #960).
struct Scratch {
    terms: Terms,
    dir: tempfile::TempDir,
    _serial: parking_lot::MutexGuard<'static, ()>,
}

impl Scratch {
    fn new() -> Scratch {
        let serial = SERIAL.lock();
        Scratch {
            terms: Terms::new(),
            dir: tempfile::tempdir().unwrap(),
            _serial: serial,
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        self.terms.close_all();
        let settled = self.terms.settle(Duration::from_secs(20));
        let t = Instant::now();
        while std::fs::remove_dir_all(self.dir.path()).is_err()
            && t.elapsed() < Duration::from_secs(20)
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        // A panic here, during a failed test's unwind, aborts.
        if !std::thread::panicking() {
            assert!(settled, "the closed shells never exited");
            assert_eq!(
                bootstrap_dirs(),
                Vec::<String>::new(),
                "shell bootstrap folders left in temp"
            );
        }
    }
}

#[test]
fn a_card_terminal_runs_a_shell_and_outlives_its_view() {
    let s = Scratch::new();
    let (terms, dir) = (&s.terms, &s.dir);
    let info = terms.open(card("t1", dir.path())).unwrap();
    assert!(info.running);
    assert_eq!(info.cwd, dir.path().to_string_lossy());

    let frames = Arc::new(AtomicUsize::new(0));
    let msgs = Arc::new(Mutex::new(Vec::<TermMsg>::new()));
    let (f, m) = (frames.clone(), msgs.clone());
    terms
        .attach(
            info.id,
            Viewer::new(
                move |b| {
                    assert!(!b.is_empty());
                    f.fetch_add(1, Ordering::Relaxed);
                    true
                },
                move |msg| {
                    m.lock().push(msg);
                    true
                },
            ),
        )
        .unwrap();
    wait_for("a first frame", || frames.load(Ordering::Relaxed) > 0);

    // Typing reaches the shell, and it runs in the card's folder.
    terms
        .write(info.id, b"echo irori-$((40+2)); pwd\r".to_vec())
        .unwrap();
    let real = std::fs::canonicalize(dir.path()).unwrap();
    wait_for("the command's output", || {
        let s = terms.screen_text(info.id).unwrap();
        s.contains("irori-42") && s.contains(real.file_name().unwrap().to_str().unwrap())
    });

    // The view goes away; the shell keeps running, and the card still
    // lists it. Opening another gives the card a second shell.
    terms.detach(info.id);
    let listed = terms.in_scope(&Scope::Card("t1".into()));
    assert_eq!(listed.len(), 1);
    assert!(listed[0].running && listed[0].id == info.id);
    let second = terms.open(card("t1", dir.path())).unwrap();
    assert_ne!(second.id, info.id);
    assert_eq!(terms.in_scope(&Scope::Card("t1".into())).len(), 2);
    terms.close(second.id).unwrap();
    terms
        .write(info.id, b"echo still-$((1+1))\r".to_vec())
        .unwrap();
    wait_for("output after detach", || {
        terms.screen_text(info.id).unwrap().contains("still-2")
    });

    // Resizing reflows: the pane takes the new size without complaint.
    terms.resize(info.id, 40, 10).unwrap();

    // The shell exits: the view hears it, and it stays listed until closed.
    let seen = msgs.clone();
    terms
        .attach(
            info.id,
            Viewer::new(
                |_| true,
                move |m| {
                    seen.lock().push(m);
                    true
                },
            ),
        )
        .unwrap();
    terms.write(info.id, b"exit 3\r".to_vec()).unwrap();
    wait_for("the exit message", || {
        msgs.lock()
            .iter()
            .any(|m| matches!(m, TermMsg::Exit { .. }))
    });
    assert_eq!(
        terms.list(),
        vec![(info.id, Scope::Card("t1".into()), false)]
    );
    assert!(terms.write(info.id, b"x".to_vec()).is_err());

    assert!(!terms.in_scope(&Scope::Card("t1".into()))[0].running);
    terms.close(info.id).unwrap();
    assert!(terms.list().is_empty());
}

#[test]
fn a_closed_view_is_forgotten() {
    let s = Scratch::new();
    let (terms, dir) = (&s.terms, &s.dir);
    let info = terms.open(card("t2", dir.path())).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    terms
        .attach(
            info.id,
            Viewer::new(
                move |_| {
                    c.fetch_add(1, Ordering::Relaxed);
                    false
                },
                |_| false,
            ),
        )
        .unwrap();
    wait_for("a frame", || calls.load(Ordering::Relaxed) > 0);
    terms.write(info.id, b"echo more\r".to_vec()).unwrap();
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "a view that said it is gone gets nothing more"
    );
    terms.close_all();
    assert!(terms.list().is_empty());
}
