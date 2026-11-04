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
            t.elapsed() < Duration::from_secs(20),
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

#[test]
fn a_card_terminal_runs_a_shell_and_outlives_its_view() {
    let dir = tempfile::tempdir().unwrap();
    let terms = Terms::new();
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
    let dir = tempfile::tempdir().unwrap();
    let terms = Terms::new();
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
