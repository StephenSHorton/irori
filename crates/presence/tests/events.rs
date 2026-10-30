//! Ports of I:internal/presence/events_test.go: the run loop on file
//! events, overflows, dead and unsupported watchers, the safety timer, and
//! a clean stop.

mod common;

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
    // Run starts its watchers before the first scan, so once this session
    // is Added every later write is reported.
    write_session(dir, json!({"pid": 40, "sessionId": "sentinel"}));
    let (mut o, opens) = event_opts(dir, HOUR, HOUR);
    o.watch = Some(polling_watch());
    let w = Watcher::new(o);
    let sub = w.subscribe();
    let r = run_watcher(&w);
    await_change(&sub, "Added sentinel", is(ChangeKind::Added, "sentinel")).await;

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
    r.stop().await;
    assert!(fw.src("sessions").closed.load(Ordering::SeqCst));
    assert!(fw.src("projects").closed.load(Ordering::SeqCst));
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(stopped.load(Ordering::SeqCst), 1);
}

/// The Windows exit wait itself: its thread ends on stop without calling
/// on_exit, and on exit when the process ends.
#[cfg(windows)]
#[test]
fn windows_exit_wait_stops_and_fires() {
    // Our own process never exits during the test: stop must end the wait.
    let w = Watcher::new(presence::Options::default());
    drop(w);
    let fired = Arc::new(AtomicBool::new(false));
    let f2 = fired.clone();
    let exit = presence::internal::default_watch_exit().expect("windows exit wait");
    let stop = exit(
        i64::from(std::process::id()),
        Box::new(move || f2.store(true, Ordering::SeqCst)),
    )
    .expect("wait on self");
    stop();
    assert!(!fired.load(Ordering::SeqCst));
}
