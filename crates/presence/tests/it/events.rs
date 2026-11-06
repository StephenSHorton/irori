//! Ports of I:internal/presence/events_test.go: the run loop on file
//! events, overflows, dead and unsupported watchers, the safety timer, and
//! a clean stop.

use crate::common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use common::*;
use presence::{ChangeKind, FsEvent, WatchError, Watcher};
use serde_json::json;

const HOUR: Duration = Duration::from_secs(3600);

/// With both intervals at an hour, only file events can move the roster.
/// The watcher is a polling stand-in for sys::fswatch (common::polling_watch).
#[tokio::test(flavor = "multi_thread")]
async fn event_path_reacts_without_rescan() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let cwd = r"C:\work\events";
    let id_a = "0b1c2d3e-0000-4000-8000-0000000000e1";
    let id_b = "0b1c2d3e-0000-4000-8000-0000000000e2";
    let tp = dir
        .join("projects")
        .join(presence::slug(cwd))
        .join(format!("{id_a}.jsonl"));
    std::fs::create_dir_all(tp.parent().unwrap()).unwrap();
    std::fs::create_dir_all(dir.join("sessions")).unwrap();
    // Once the watchers are up every later write is reported.
    write_session(dir, json!({"pid": 40, "sessionId": "sentinel"}));
    let (mut o, opens) = event_opts(dir, HOUR, HOUR);
    o.watch = Some(polling_watch());
    let w = Watcher::new(o);
    let sub = w.subscribe();
    let r = run_watcher(&w);
    await_change(&sub, "Added sentinel", is(ChangeKind::Added, "sentinel")).await;
    await_events(&w, &opens, 1).await;

    // A session appears, transcript already there.
    append_file(&tp, &assistant_row("2026-10-26T10:00:00Z", "one"));
    write_session(
        dir,
        json!({"pid": 41, "sessionId": id_a, "cwd": cwd, "status": "busy"}),
    );
    await_change(&sub, "Added A", |c| {
        c.kind == ChangeKind::Added && c.card.session_id == id_a && c.card.last_text == "one"
    })
    .await;

    // The transcript grows: only A's tail is reread, no session file.
    let jsons = opens.count(".json");
    append_file(&tp, &assistant_row("2026-10-26T10:00:05Z", "two"));
    await_change(&sub, "Updated A", |c| {
        c.kind == ChangeKind::Updated && c.card.session_id == id_a && c.card.last_text == "two"
    })
    .await;
    assert_eq!(
        opens.count(".json"),
        jsons,
        "a transcript change reread session files"
    );

    // A session whose transcript shows up after its session file.
    let p_b = write_session(dir, json!({"pid": 42, "sessionId": id_b, "cwd": cwd}));
    await_change(&sub, "Added B", |c| {
        c.kind == ChangeKind::Added && c.card.session_id == id_b && c.card.transcript.is_empty()
    })
    .await;
    append_file(
        &dir.join("projects")
            .join(presence::slug(cwd))
            .join(format!("{id_b}.jsonl")),
        &assistant_row("2026-10-26T10:01:00Z", "late"),
    );
    await_change(&sub, "Updated B transcript", |c| {
        c.kind == ChangeKind::Updated
            && c.card.session_id == id_b
            && c.card.last_text == "late"
            && !c.card.transcript.is_empty()
    })
    .await;

    std::fs::remove_file(&p_b).unwrap();
    await_change(&sub, "Removed B", |c| {
        c.kind == ChangeKind::Removed && c.card.session_id == id_b && c.card.status == "gone"
    })
    .await;
    r.stop().await;
}

/// Overflow means events were lost: everything is rescanned at once.
#[tokio::test(flavor = "multi_thread")]
async fn overflow_forces_full_scan() {
    for root in ["sessions", "projects"] {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path();
        let fw = FakeWatch::new();
        let (mut o, _) = event_opts(dir, HOUR, HOUR);
        fw.install(&mut o);
        write_session(dir, json!({"pid": 41, "sessionId": "first"}));
        let w = Watcher::new(o);
        let sub = w.subscribe();
        let r = run_watcher(&w);
        await_change(&sub, "Added first", is(ChangeKind::Added, "first")).await;

        write_session(dir, json!({"pid": 42, "sessionId": "second"}));
        fw.src(root).send(FsEvent {
            path: dir.join(root).to_string_lossy().into_owned(),
            overflow: true,
        });
        await_change(&sub, "Added second", is(ChangeKind::Added, "second")).await;
        r.stop().await;
    }
}

/// Events for files that aren't session files (the .key secrets, temp
/// files) don't trigger a scan.
#[tokio::test(flavor = "multi_thread")]
async fn key_file_event_ignored() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let fw = FakeWatch::new();
    let (mut o, opens) = event_opts(dir, HOUR, HOUR);
    fw.install(&mut o);
    write_session(dir, json!({"pid": 41, "sessionId": "only"}));
    let w = Watcher::new(o);
    let sub = w.subscribe();
    let r = run_watcher(&w);
    await_change(&sub, "Added", |c| c.kind == ChangeKind::Added).await;
    await_events(&w, &opens, 1).await;
    let jsons = opens.count(".json");
    fw.src("sessions").send(FsEvent {
        path: dir
            .join("sessions")
            .join("41.abcdef.key")
            .to_string_lossy()
            .into_owned(),
        overflow: false,
    });
    fw.src("projects").send(FsEvent {
        path: dir
            .join("projects")
            .join("x")
            .join("only")
            .join("subagents")
            .join("agent-1.jsonl")
            .to_string_lossy()
            .into_owned(),
        overflow: false,
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(opens.count(".json"), jsons, "non-session events rescanned");
    r.stop().await;
}

/// A watcher that dies hands the roster back to Interval, and Run keeps
/// trying to start it again.
#[tokio::test(flavor = "multi_thread")]
async fn dead_watcher_falls_back_to_interval() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let fw = FakeWatch::new();
    let (mut o, _) = event_opts(dir, Duration::from_millis(20), HOUR);
    fw.install(&mut o);
    write_session(dir, json!({"pid": 41, "sessionId": "first"}));
    let w = Watcher::new(o);
    let sub = w.subscribe();
    let r = run_watcher(&w);
    await_change(&sub, "Added first", |c| c.kind == ChangeKind::Added).await;
    await_watching(&w).await;

    *fw.err.lock().unwrap() = Some(WatchError::Other("dir gone".into()));
    fw.src("sessions").kill();

    write_session(dir, json!({"pid": 42, "sessionId": "second"}));
    await_change(
        &sub,
        "Added second by interval",
        is(ChangeKind::Added, "second"),
    )
    .await;
    assert!(
        fw.src("sessions").closed.load(Ordering::SeqCst),
        "dead watcher not closed"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while fw.count("sessions") < 3 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        fw.count("sessions") >= 3,
        "watcher restarted {} times, want retries",
        fw.count("sessions") - 1
    );
    r.stop().await;
}

/// Without file events the roster polls every Interval and never asks
/// again.
#[tokio::test(flavor = "multi_thread")]
async fn unsupported_keeps_interval() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let fw = FakeWatch::new();
    *fw.err.lock().unwrap() = Some(WatchError::Unsupported);
    let (mut o, _) = event_opts(dir, Duration::from_millis(10), HOUR);
    fw.install(&mut o);
    let w = Watcher::new(o);
    let sub = w.subscribe();
    let r = run_watcher(&w);
    tokio::time::sleep(Duration::from_millis(30)).await;
    write_session(dir, json!({"pid": 41, "sessionId": "polled"}));
    await_change(&sub, "Added", |c| c.kind == ChangeKind::Added).await;
    assert_eq!(
        (fw.count("sessions"), fw.count("projects")),
        (1, 0),
        "watch calls"
    );
    r.stop().await;
}

/// No watch hook at all (core before sys::fswatch is wired) is the same
/// polling path.
#[tokio::test(flavor = "multi_thread")]
async fn no_watch_hook_polls() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let (o, _) = event_opts(dir, Duration::from_millis(10), HOUR);
    let w = Watcher::new(o);
    let sub = w.subscribe();
    let r = run_watcher(&w);
    tokio::time::sleep(Duration::from_millis(30)).await;
    write_session(dir, json!({"pid": 41, "sessionId": "polled"}));
    await_change(&sub, "Added", |c| c.kind == ChangeKind::Added).await;
    r.stop().await;
}

/// A process that dies without touching its session file is caught by the
/// safety rescan.
#[tokio::test(flavor = "multi_thread")]
async fn liveness_on_safety_timer() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let fw = FakeWatch::new();
    let (mut o, _) = event_opts(dir, HOUR, Duration::from_millis(50));
    fw.install(&mut o);
    let alive = Arc::new(AtomicBool::new(true));
    let a2 = alive.clone();
    o.live = Some(Arc::new(move |_, _| a2.load(Ordering::SeqCst)));
    write_session(dir, json!({"pid": 41, "sessionId": "dies"}));
    let w = Watcher::new(o);
    let sub = w.subscribe();
    let r = run_watcher(&w);
    await_change(&sub, "Added", |c| c.kind == ChangeKind::Added).await;
    alive.store(false, Ordering::SeqCst);
    await_change(&sub, "Removed", is(ChangeKind::Removed, "dies")).await;
    r.stop().await;
}

/// Run leaves nothing behind: it returns after cancel, having stopped every
/// file watcher and exit wait.
#[tokio::test(flavor = "multi_thread")]
async fn run_stops_everything() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    for d in ["sessions", "projects"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    write_session(
        dir,
        json!({"pid": std::process::id(), "sessionId": "me", "cwd": dir.to_string_lossy()}),
    );
    let fw = FakeWatch::new();
    let (mut o, _) = test_opts(dir, false);
    fw.install(&mut o);
    // Count exit waits started and stopped, around the OS's own.
    let started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let stopped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (s1, s2) = (started.clone(), stopped.clone());
    o.watch_exit = Some(Arc::new(move |_pid, _on_exit| {
        s1.fetch_add(1, Ordering::SeqCst);
        let s2 = s2.clone();
        Some(Box::new(move || {
            s2.fetch_add(1, Ordering::SeqCst);
        }))
    }));
    let w = Watcher::new(o);
    let sub = w.subscribe();
    let r = run_watcher(&w);
    await_change(&sub, "Added", |c| c.kind == ChangeKind::Added).await;
    await_watching(&w).await;
    r.stop().await;
    assert!(fw.src("sessions").closed.load(Ordering::SeqCst));
    assert!(fw.src("projects").closed.load(Ordering::SeqCst));
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(stopped.load(Ordering::SeqCst), 1);
}

/// The Windows exit wait itself: its thread ends on stop without calling
/// on_exit, and on exit when the process ends.
#[cfg(any(windows, target_os = "macos"))]
#[test]
fn os_exit_wait_stops_and_fires() {
    // Our own process never exits during the test: stop must end the wait.
    let w = Watcher::new(presence::Options::default());
    drop(w);
    let fired = Arc::new(AtomicBool::new(false));
    let f2 = fired.clone();
    let exit = presence::internal::default_watch_exit().expect("this OS's exit wait");
    let stop = exit(
        i64::from(std::process::id()),
        Box::new(move || f2.store(true, Ordering::SeqCst)),
    )
    .expect("wait on self");
    stop();
    assert!(!fired.load(Ordering::SeqCst));

    #[cfg(target_os = "macos")]
    {
        // A child that exits fires on_exit at once, before it is reaped.
        let mut c = std::process::Command::new("/bin/sleep")
            .arg("0.3")
            .spawn()
            .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let stop = exit(
            i64::from(c.id()),
            Box::new(move || {
                let _ = tx.send(());
            }),
        )
        .expect("wait on the child");
        rx.recv_timeout(Duration::from_secs(5))
            .expect("on_exit after the child exited");
        stop();
        c.wait().unwrap();
        // An exited pid has nothing to wait on.
        assert!(exit(i64::from(c.id()), Box::new(|| {})).is_none());
    }
}

/// Card #433: a session whose file was unreadable when last scanned (so
/// it isn't listed) and then writes its transcript is read again in full,
/// without waiting for a session-file event or the safety scan.
#[tokio::test(flavor = "multi_thread")]
async fn transcript_of_an_unlisted_session_rescans() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let cwd = r"C:\work\unlisted";
    let id = "0b1c2d3e-0000-4000-8000-0000000004a3";
    std::fs::create_dir_all(dir.join("sessions")).unwrap();
    write_session(dir, json!({"pid": 40, "sessionId": "sentinel"}));
    let p = dir.join("sessions").join("43.json");
    std::fs::write(&p, b"{\"pid\":").unwrap();
    let (mut o, opens) = event_opts(dir, HOUR, HOUR);
    let fw = FakeWatch::new();
    fw.install(&mut o);
    let w = Watcher::new(o);
    let sub = w.subscribe();
    let r = run_watcher(&w);
    await_change(&sub, "Added sentinel", is(ChangeKind::Added, "sentinel")).await;
    await_events(&w, &opens, 2).await;
    assert!(!w.snapshot().iter().any(|c| c.session_id == id));

    // The file reads again, but no session-file event arrives; only the
    // transcript moves.
    write_session(dir, json!({"pid": 43, "sessionId": id, "cwd": cwd}));
    let tp = dir
        .join("projects")
        .join(presence::slug(cwd))
        .join(format!("{id}.jsonl"));
    append_file(&tp, &assistant_row("2026-10-26T10:00:00Z", "still here"));
    fw.src("projects").send(FsEvent {
        path: tp.to_string_lossy().into_owned(),
        overflow: false,
    });
    await_change(&sub, "Added unlisted", is(ChangeKind::Added, id)).await;
    r.stop().await;
}

/// Card #601: on a busy Mac starting FSEvents took up to 100 s, and the
/// roster stayed empty until it was up. The first scan doesn't wait for
/// the watchers; until they're up the roster polls, and once they are it
/// reads everything again.
#[tokio::test(flavor = "multi_thread")]
async fn slow_watcher_start_does_not_hold_the_roster() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    std::fs::create_dir_all(dir.join("sessions")).unwrap();
    write_session(dir, json!({"pid": 41, "sessionId": "first"}));
    let (mut o, _) = event_opts(dir, Duration::from_millis(20), HOUR);
    let fw = FakeWatch::new();
    fw.install(&mut o);
    let gate = Arc::new(std::sync::Barrier::new(2));
    let inner = o.watch.take().unwrap();
    let g = gate.clone();
    o.watch = Some(Arc::new(move |root: &std::path::Path, rec: bool| {
        if root.ends_with("sessions") {
            g.wait(); // held until the test lets it go
        }
        inner(root, rec)
    }));
    let w = Watcher::new(o);
    let sub = w.subscribe();
    let r = run_watcher(&w);
    await_change(&sub, "Added first", is(ChangeKind::Added, "first")).await;
    assert!(!w.watching());

    // Still starting: a new session shows by polling.
    write_session(dir, json!({"pid": 42, "sessionId": "second"}));
    await_change(&sub, "Added second", is(ChangeKind::Added, "second")).await;

    let g = gate.clone();
    tokio::task::spawn_blocking(move || g.wait()).await.unwrap();
    await_watching(&w).await;
    r.stop().await;
    assert!(fw.src("sessions").closed.load(Ordering::SeqCst));
}

/// Card #601: Run ending doesn't wait out a watcher start that hangs; the
/// watchers it hands back later stop at once.
#[tokio::test(flavor = "multi_thread")]
async fn stop_is_prompt_while_a_watcher_starts() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    write_session(dir, json!({"pid": 41, "sessionId": "only"}));
    let (mut o, _) = event_opts(dir, HOUR, HOUR);
    let fw = FakeWatch::new();
    fw.install(&mut o);
    let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
    let go_rx = std::sync::Mutex::new(go_rx);
    let inner = o.watch.take().unwrap();
    o.watch = Some(Arc::new(move |root: &std::path::Path, rec: bool| {
        let _ = go_rx.lock().unwrap().recv();
        inner(root, rec)
    }));
    let w = Watcher::new(o);
    let sub = w.subscribe();
    let r = run_watcher(&w);
    await_change(&sub, "Added", |c| c.kind == ChangeKind::Added).await;
    let t0 = Instant::now();
    r.stop().await;
    assert!(
        t0.elapsed() < Duration::from_secs(3),
        "stop took {:?}",
        t0.elapsed()
    );
    assert!(!w.watching());

    // The start ends after Run did: what it opened is stopped.
    go_tx.send(()).unwrap();
    go_tx.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !(fw.src("sessions").closed.load(Ordering::SeqCst)
        && fw.src("projects").closed.load(Ordering::SeqCst))
    {
        assert!(Instant::now() < deadline, "late watchers never stopped");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
