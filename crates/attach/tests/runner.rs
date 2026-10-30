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
    let out = Out::default();
    let stop = CancellationToken::new();
    let done = tokio::spawn(runner::run(Options {
        session: session.into(),
        mod_version: "0.2.0-test".into(),
        data_dir: data.to_path_buf(),
        stdout: Box::new(out.clone()),
        cancel: stop.clone(),
        min_backoff: Duration::from_millis(50),
        max_backoff: Duration::from_millis(200),
        ..Options::default()
    }));
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

/// The Core going away mid-stream: the runner reconnects, and says Irori
/// isn't running once per outage.
#[tokio::test(flavor = "multi_thread")]
async fn reconnects_after_the_core_stops() {
    let r = Rig::new().await;
    let p = start_runner(&r.data, SESS_A);
    p.out.wait("hello", 1).await;
    let data = r.data.clone();
    drop(r); // the host stops and the Core closes (its lockfile goes)
    p.out.wait("error", 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let errs = p
        .out
        .events()
        .iter()
        .filter(|e| e["type"] == "error")
        .count();
    assert_eq!(errs, 1);
    assert!(!data.join("irori.json").exists());
    p.stop.cancel();
    p.exited().await;
}
