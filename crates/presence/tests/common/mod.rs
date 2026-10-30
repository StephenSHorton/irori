//! Shared helpers for the presence tests (Go presence_test.go,
//! events_test.go, v02_test.go).
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use presence::{Change, ChangeKind, FsEvent, FsSource, Options, Subscription, WatchError, Watcher};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub const FIXTURE_ID: &str = "0e1f2a3b-4c5d-4e6f-8a9b-0c1d2e3f4a5b";
pub const FIXTURE_ID2: &str = "5b4a3f2e-1d0c-4b9a-8f6e-5d4c3b2a1f0e";

pub fn testdata() -> PathBuf {
    manifest_dir().join("testdata")
}

/// Records every path the watcher opened.
#[derive(Clone, Default)]
pub struct Opens(pub Arc<Mutex<Vec<String>>>);

impl Opens {
    pub fn count(&self, suffix: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.ends_with(suffix))
            .count()
    }

    pub fn paths(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// Go `testWatcher`'s options: device "dev1", interval 10 ms, opens
/// recorded, and every pid live when `all_live`.
pub fn test_opts(dir: &Path, all_live: bool) -> (Options, Opens) {
    let o = Opens::default();
    let rec = o.clone();
    let mut opts = Options {
        claude_dir: dir.to_string_lossy().into_owned(),
        device_id: "dev1".into(),
        interval: Duration::from_millis(10),
        open: Some(Arc::new(move |p: &Path| {
            rec.0.lock().unwrap().push(p.to_string_lossy().into_owned());
            std::fs::File::open(p)
        })),
        ..Options::default()
    };
    if all_live {
        opts.live = Some(Arc::new(|_, _| true));
    }
    (opts, o)
}

pub fn test_watcher(dir: &Path, all_live: bool) -> (Watcher, Opens) {
    let (opts, o) = test_opts(dir, all_live);
    (Watcher::new(opts), o)
}

/// Go `writeSession`: a synthetic `<pid>.json` in dir/sessions.
pub fn write_session(dir: &Path, fields: Value) -> PathBuf {
    std::fs::create_dir_all(dir.join("sessions")).unwrap();
    let pid = match &fields["pid"] {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    };
    let p = dir.join("sessions").join(format!("{pid}.json"));
    std::fs::write(&p, serde_json::to_vec(&fields).unwrap()).unwrap();
    p
}

pub fn mode_row(mode: &str) -> String {
    format!(
        "{}\n",
        json!({"type": "permission-mode", "permissionMode": mode, "sessionId": "s"})
    )
}

pub fn assistant_row(ts: &str, text: &str) -> String {
    format!(
        "{{\"type\":\"assistant\",\"timestamp\":{},\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":{}}}]}}}}\n",
        json!(ts),
        json!(text)
    )
}

pub fn append_file(p: &Path, s: &str) {
    use std::io::Write;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).unwrap();
    }
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(p)
        .unwrap();
    f.write_all(s.as_bytes()).unwrap();
}

pub fn drain(sub: &Subscription) -> Vec<Change> {
    let mut out = Vec::new();
    while let gocompat::sub::TryRecv::Item(c) = sub.try_recv() {
        out.push(c);
    }
    out
}

/// Go `await`: the first change matching `ok` within 2 s.
pub async fn await_change(sub: &Subscription, what: &str, ok: impl Fn(&Change) -> bool) -> Change {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        match tokio::time::timeout_at(deadline, sub.recv_async()).await {
            Ok(Some(c)) if ok(&c) => return c,
            Ok(Some(_)) => {}
            Ok(None) => panic!("subscription ended waiting for {what}"),
            Err(_) => panic!("no {what} within 2s"),
        }
    }
}

pub fn is(kind: ChangeKind, sid: &str) -> impl Fn(&Change) -> bool + '_ {
    move |c: &Change| c.kind == kind && c.card.session_id == sid
}

/// Go `runWatcher`: runs `w` until the returned guard drops, and checks
/// run returns within 5 s of cancel.
pub struct Running {
    pub cancel: CancellationToken,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl Running {
    pub async fn stop(mut self) {
        self.cancel.cancel();
        let h = self.handle.take().unwrap();
        tokio::time::timeout(Duration::from_secs(5), h)
            .await
            .expect("run did not return after cancel")
            .unwrap();
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

pub fn run_watcher(w: &Watcher) -> Running {
    let cancel = CancellationToken::new();
    let (w2, c2) = (w.clone(), cancel.clone());
    let handle = tokio::spawn(async move { w2.run(c2).await });
    Running {
        cancel,
        handle: Some(handle),
    }
}

/// One fake event source the test drives by hand (Go `fakeSource`).
pub struct FakeSource {
    pub tx: Mutex<Option<mpsc::Sender<FsEvent>>>,
    rx: Mutex<Option<mpsc::Receiver<FsEvent>>>,
    pub closed: Arc<AtomicBool>,
}

impl FakeSource {
    fn new() -> Arc<FakeSource> {
        let (tx, rx) = mpsc::channel(16);
        Arc::new(FakeSource {
            tx: Mutex::new(Some(tx)),
            rx: Mutex::new(Some(rx)),
            closed: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn send(&self, e: FsEvent) {
        self.tx
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .try_send(e)
            .unwrap();
    }

    /// Go `close(dead.ch)`: the watcher dies.
    pub fn kill(&self) {
        self.tx.lock().unwrap().take();
    }
}

/// Hands out fake sources by dir name ("sessions", "projects") and counts
/// the calls (Go `fakeWatch`).
#[derive(Clone)]
pub struct FakeWatch {
    pub srcs: Arc<HashMap<String, Arc<FakeSource>>>,
    pub calls: Arc<Mutex<HashMap<String, usize>>>,
    pub err: Arc<Mutex<Option<WatchError>>>,
}

impl FakeWatch {
    pub fn new() -> FakeWatch {
        let mut srcs = HashMap::new();
        srcs.insert("sessions".to_string(), FakeSource::new());
        srcs.insert("projects".to_string(), FakeSource::new());
        FakeWatch {
            srcs: Arc::new(srcs),
            calls: Arc::default(),
            err: Arc::default(),
        }
    }

    pub fn count(&self, name: &str) -> usize {
        self.calls.lock().unwrap().get(name).copied().unwrap_or(0)
    }

    pub fn src(&self, name: &str) -> &FakeSource {
        &self.srcs[name]
    }

    pub fn install(&self, opts: &mut Options) {
        let fw = self.clone();
        opts.watch = Some(Arc::new(move |root: &Path, _rec: bool| {
            let name = root.file_name().unwrap().to_string_lossy().into_owned();
            *fw.calls.lock().unwrap().entry(name.clone()).or_default() += 1;
            if let Some(e) = fw.err.lock().unwrap().clone() {
                return Err(e);
            }
            let src = &fw.srcs[&name];
            let Some(rx) = src.rx.lock().unwrap().take() else {
                return Err(WatchError::Other("source already handed out".into()));
            };
            let closed = src.closed.clone();
            Ok(FsSource::new(rx, move || {
                closed.store(true, Ordering::SeqCst)
            }))
        }));
    }
}

/// Go `eventWatcher`'s options: only the given intervals as clocks, settle
/// 10 ms, no exit waits.
pub fn event_opts(dir: &Path, interval: Duration, safety: Duration) -> (Options, Opens) {
    let (mut o, opens) = test_opts(dir, true);
    o.interval = interval;
    o.safety_interval = safety;
    o.settle = Duration::from_millis(10);
    o.watch_exit = Some(Arc::new(|_, _| None));
    (o, opens)
}

/// A real file watcher for tests, by polling: every 5 ms it walks `root`
/// and reports each file whose size or mtime moved, or that appeared or
/// went. It stands in for sys::fswatch until core wires that in.
pub fn polling_watch() -> presence::WatchFn {
    Arc::new(|root: &Path, recursive: bool| {
        if !root.is_dir() {
            return Err(WatchError::Other("no such dir".into()));
        }
        let (tx, rx) = mpsc::channel(1024);
        let stop = Arc::new(AtomicBool::new(false));
        let (root, s2) = (root.to_path_buf(), stop.clone());
        let mut seen = snapshot(&root, recursive);
        let t = std::thread::spawn(move || {
            while !s2.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
                let now = snapshot(&root, recursive);
                for (p, v) in &now {
                    if seen.get(p) != Some(v) {
                        let _ = tx.blocking_send(FsEvent {
                            path: p.clone(),
                            overflow: false,
                        });
                    }
                }
                for p in seen.keys() {
                    if !now.contains_key(p) {
                        let _ = tx.blocking_send(FsEvent {
                            path: p.clone(),
                            overflow: false,
                        });
                    }
                }
                seen = now;
            }
        });
        Ok(FsSource::new(rx, move || {
            stop.store(true, Ordering::SeqCst);
            let _ = t.join();
        }))
    })
}

fn snapshot(root: &Path, recursive: bool) -> HashMap<String, (u64, Option<std::time::SystemTime>)> {
    let mut out = HashMap::new();
    let Ok(rd) = std::fs::read_dir(root) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if recursive {
                out.extend(snapshot(&p, true));
            }
        } else if let Ok(m) = e.metadata() {
            out.insert(
                p.to_string_lossy().into_owned(),
                (m.len(), m.modified().ok()),
            );
        }
    }
    out
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
