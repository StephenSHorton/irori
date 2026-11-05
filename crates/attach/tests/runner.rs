//! `irori attach`'s loop against the real API server (I:internal/core/
//! v02_test.go TestAttachCommand), run in-process as Go does.

mod common;

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use attach::runner::{self, Options};
use common::{Rig, SESS_A, SESS_B};
use inbox::{Address, addr};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

/// The runner's stdout, kept.
#[derive(Clone, Default)]
struct Out(Arc<Mutex<Vec<u8>>>);

impl Write for Out {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Out {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }

    /// Every line, each one JSON with a newline (nothing else reaches
    /// stdout).
    fn events(&self) -> Vec<Value> {
        let t = self.text();
        assert!(t.is_empty() || t.ends_with('\n'), "a partial line: {t:?}");
        t.lines()
            .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("stdout line {l:?}: {e}")))
            .collect()
    }

    /// The `n`th event of type `t` (1-based), waiting up to 10 s.
    async fn wait(&self, t: &str, n: usize) -> Value {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let seen: Vec<Value> = self
                .events()
                .into_iter()
                .filter(|e| e["type"] == t)
                .collect();
            if seen.len() >= n {
                return seen[n - 1].clone();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "runner: no {t} #{n} in {}",
                self.text()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

struct Proc {
    out: Out,
    stop: CancellationToken,
    done: tokio::task::JoinHandle<Result<(), String>>,
}

impl Proc {
    async fn exited(self) {
        let r = tokio::time::timeout(Duration::from_secs(10), self.done)
            .await
            .expect("runner did not exit")
            .unwrap();
        r.expect("runner");
    }
}

fn start_runner(data: &Path, session: &str) -> Proc {
    start_runner_with(data, session, |_| {})
}

fn start_runner_with(data: &Path, session: &str, edit: impl FnOnce(&mut Options)) -> Proc {
    let out = Out::default();
    let stop = CancellationToken::new();
    let mut o = Options {
        session: session.into(),
        mod_version: "0.2.0-test".into(),
        data_dir: data.to_path_buf(),
        stdout: Box::new(out.clone()),
        cancel: stop.clone(),
        min_backoff: Duration::from_millis(50),
        max_backoff: Duration::from_millis(200),
        ..Options::default()
    };
    edit(&mut o);
    let done = tokio::spawn(runner::run(o));
    Proc { out, stop, done }
}

#[tokio::test(flavor = "multi_thread")]
async fn attach_command() {
    // Not running yet: one error line, then it attaches once Irori starts.
    let early = tempfile::tempdir().unwrap();
    let p0 = start_runner(early.path(), SESS_B);
    let ev = p0.out.wait("error", 1).await;
    assert!(
        ev["message"]
            .as_str()
            .unwrap()
            .contains("Irori isn't running"),
        "error {ev}"
    );
    tokio::time::sleep(Duration::from_millis(300)).await; // several retries
    let n = p0
        .out
        .events()
        .iter()
        .filter(|e| e["type"] == "error")
        .count();
    assert_eq!(n, 1, "error lines during one outage");
    assert_eq!(
        p0.out.text(),
        "{\"type\":\"error\",\"message\":\"Irori isn't running, so messages can't reach this session. It attaches by itself once Irori starts.\"}\n"
    );
    p0.stop.cancel();
    p0.exited().await;

    let r = Rig::new().await;
    let p1 = start_runner(&r.data, SESS_B);
    let hello = p1.out.wait("hello", 1).await;
    assert_eq!(hello["agentKey"], r.key_b.as_str(), "hello {hello}");
    r.core
        .inbox()
        .send(
            &r.sender(&r.key_a, "alpha"),
            &Address::new(addr::AGENT, "beta"),
            "through the pipe",
            "",
        )
        .unwrap();
    let d = p1.out.wait("deliver", 1).await;
    assert!(
        d["prompt"].as_str().unwrap().contains("through the pipe"),
        "deliver {d}"
    );

    // A second attach for the same session supersedes the first, which
    // exits cleanly.
    let p2 = start_runner(&r.data, SESS_B);
    p2.out.wait("hello", 1).await;
    p1.out.wait("superseded", 1).await;
    p1.exited().await;

    // Canceling (the parent died) ends it.
    p2.stop.cancel();
    p2.exited().await;
}

/// GitHub #57: a superseded copy waits in standby. It takes nothing from the
/// live attach, says nothing while it waits, and attaches within seconds of
/// that attach going.
#[tokio::test(flavor = "multi_thread")]
async fn standby_waits_for_the_session_to_come_free() {
    let r = Rig::new().await;
    let holder = start_runner(&r.data, SESS_B);
    holder.out.wait("hello", 1).await;
    let standby = start_runner_with(&r.data, SESS_B, |o| {
        o.standby = true;
        o.standby_poll = Duration::from_millis(50);
    });
    tokio::time::sleep(Duration::from_millis(500)).await; // several asks
    assert_eq!(standby.out.text(), "", "standby while the session is held");
    assert!(
        !holder.out.events().iter().any(|e| e["type"] == "superseded"),
        "standby took the session: {}",
        holder.out.text()
    );

    // The holder goes, as a process that dies does.
    let gone = std::time::Instant::now();
    holder.stop.cancel();
    holder.exited().await;
    standby.out.wait("hello", 1).await;
    let took = gone.elapsed();
    assert!(took < Duration::from_secs(3), "took over after {took:?}");
    assert!(
        !standby.out.events().iter().any(|e| e["type"] == "error"),
        "{}",
        standby.out.text()
    );
    standby.stop.cancel();
    standby.exited().await;
}

/// GitHub #57: two copies that both wait never kick each other; the first
/// to ask takes a free session and the other goes on waiting.
#[tokio::test(flavor = "multi_thread")]
async fn two_standby_copies_settle_on_one() {
    let r = Rig::new().await;
    let quick = |o: &mut Options| {
        o.standby = true;
        o.standby_poll = Duration::from_millis(50);
    };
    let a = start_runner_with(&r.data, SESS_B, quick);
    a.out.wait("hello", 1).await;
    let b = start_runner_with(&r.data, SESS_B, quick);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(b.out.text(), "", "the second copy waits");
    assert!(
        !a.out.events().iter().any(|e| e["type"] == "superseded"),
        "{}",
        a.out.text()
    );
    a.stop.cancel();
    a.exited().await;
    b.out.wait("hello", 1).await;
    b.stop.cancel();
    b.exited().await;
}

/// GitHub #57: a copy that came back through standby attaches to the new
/// Core after a restart, like any other.
#[tokio::test(flavor = "multi_thread")]
async fn standby_survives_the_core_restarting() {
    let r = Rig::new().await;
    let holder = start_runner(&r.data, SESS_B);
    holder.out.wait("hello", 1).await;
    let standby = start_runner_with(&r.data, SESS_B, |o| {
        o.standby = true;
        o.standby_poll = Duration::from_millis(50);
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    holder.stop.cancel();
    holder.exited().await;
    let first = standby.out.wait("hello", 1).await;
    let _r = r.restart().await;
    let second = standby.out.wait("hello", 2).await;
    assert_ne!(first["attachSecret"], second["attachSecret"]);
    standby.stop.cancel();
    standby.exited().await;
}

/// A session presence doesn't list yet is a 404: retried quietly (no
/// error line), and attached once its card appears.
#[tokio::test(flavor = "multi_thread")]
async fn not_on_roster_yet_retries_quietly() {
    let r = Rig::new().await;
    let late = "cccccccc-3333-4333-8333-000000000003";
    let p = start_runner(&r.data, late);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        p.out.text(),
        "",
        "nothing on stdout while the card is missing"
    );
    let cwd = tempfile::tempdir().unwrap();
    common::write_session(
        &r.claude.join("sessions"),
        "900003.json",
        late,
        "gamma",
        cwd.path(),
    );
    let hello = p.out.wait("hello", 1).await;
    assert_eq!(hello["name"], "gamma");
    p.stop.cancel();
    p.exited().await;
}

/// The Core going away mid-stream: the runner says the stream dropped,
/// reconnects, and says Irori isn't running once per outage.
#[tokio::test(flavor = "multi_thread")]
async fn reconnects_after_the_core_stops() {
    let r = Rig::new().await;
    let p = start_runner(&r.data, SESS_A);
    p.out.wait("hello", 1).await;
    let data = r.data.clone();
    drop(r); // the host stops and the Core closes (its lockfile goes)
    p.out.wait("error", 2).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let errs: Vec<Value> = p
        .out
        .events()
        .into_iter()
        .filter(|e| e["type"] == "error")
        .map(|e| e["message"].clone())
        .collect();
    // The stream's secret died, then Irori is down: each said once.
    assert_eq!(
        errs,
        vec![
            Value::from(runner::DROPPED),
            Value::from(runner::NOT_RUNNING)
        ]
    );
    assert!(!data.join("irori.json").exists());
    p.stop.cancel();
    p.exited().await;
}

/// An app restart (or update) under an attached session (card #111): the
/// runner attaches to the new Core by itself, and the new hello's secret is
/// the one the new Core takes.
#[tokio::test(flavor = "multi_thread")]
async fn reattaches_after_the_app_restarts() {
    let r = Rig::new().await;
    let p = start_runner(&r.data, SESS_A);
    let first = p.out.wait("hello", 1).await;
    let r = r.restart().await;
    let second = p.out.wait("hello", 2).await;
    // Between the two, the mod heard its secret died.
    let evs = p.out.events();
    let h2 = evs.iter().rposition(|e| e["type"] == "hello").unwrap();
    let first_err = evs.iter().position(|e| e["type"] == "error").unwrap();
    assert!(first_err < h2);
    assert_eq!(evs[first_err]["message"], runner::DROPPED);
    assert_ne!(first["attachSecret"], second["attachSecret"]);
    assert_eq!(
        second["api"]["url"],
        format!("http://127.0.0.1:{}", r.host.port)
    );
    let old = first["attachSecret"].as_str().unwrap();
    let new = second["attachSecret"].as_str().unwrap();
    assert!(!r.host.server.is_attach_secret(old));
    assert!(r.host.server.is_attach_secret(new));
    p.stop.cancel();
    p.exited().await;
}

/// A stream that goes silent (the Core lost it without closing it; no
/// keepalive either) is dropped after the idle timeout and attached again,
/// instead of holding the session forever (card #111).
#[tokio::test(flavor = "multi_thread")]
async fn a_silent_stream_is_reattached() {
    let r = Rig::with(Duration::from_secs(3600), |_| {}).await;
    let p = start_runner_with(&r.data, SESS_A, |o| {
        o.idle_timeout = Duration::from_millis(300)
    });
    let first = p.out.wait("hello", 1).await;
    let dropped = p.out.wait("error", 1).await;
    assert_eq!(dropped["message"], runner::DROPPED);
    let second = p.out.wait("hello", 2).await;
    assert!(
        r.host
            .server
            .is_attach_secret(second["attachSecret"].as_str().unwrap())
    );
    assert!(
        !r.host
            .server
            .is_attach_secret(first["attachSecret"].as_str().unwrap())
    );
    p.stop.cancel();
    p.exited().await;
}

/// A session that stays off the roster is said once, not left on a silent
/// "connecting" (card #111).
#[tokio::test(flavor = "multi_thread")]
async fn off_the_roster_too_long_is_said_once() {
    let r = Rig::new().await;
    let late = "dddddddd-4444-4444-8444-000000000004";
    let p = start_runner_with(&r.data, late, |o| {
        o.roster_grace = Duration::from_millis(300)
    });
    let err = p.out.wait("error", 1).await;
    assert_eq!(err["message"], runner::not_on_roster(late));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(p.out.events().len(), 1, "{}", p.out.text());
    p.stop.cancel();
    p.exited().await;
}
