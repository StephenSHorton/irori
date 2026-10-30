//! The Go core tests' attach rig (I:internal/core/v02_test.go newRig,
//! evStream): a Core over a scratch data dir and a fake Claude dir holding
//! two live sessions, alpha (A) and beta (B), with the file keystore,
//! served by the real `irori_api::Server` (what `irori serve` hosts) on
//! 127.0.0.1:0 with its lockfile written. Copied from
//! crates/core/tests/common, which is not a library.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use irori_core::{Core, Options};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// The API server over the rig's Core. Dropping it stops the listener.
pub struct Host {
    pub port: u16,
    pub server: irori_api::Server,
    stop: CancellationToken,
}

impl Host {
    /// Serves `core` on 127.0.0.1:0 on the current runtime, with the Core's
    /// tools, and writes its lockfile; a nonzero `keepalive` replaces 15 s.
    pub async fn start(core: Arc<Core>, keepalive: Duration) -> Host {
        let server = irori_api::Server::new(core.clone(), core.token())
            .with_tools(Arc::new(irori_api::CoreTools(core.clone())));
        if !keepalive.is_zero() {
            server.set_keepalive(keepalive);
        }
        let ln = irori_api::listen().await.unwrap();
        let port = ln.local_addr().unwrap().port();
        core.write_lockfile(port).expect("write the lockfile");
        let stop = CancellationToken::new();
        {
            let (srv, stop) = (server.clone(), stop.clone());
            tokio::spawn(async move { srv.serve(ln, stop).await });
        }
        Host { port, server, stop }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

pub const SESS_A: &str = "aaaaaaaa-1111-4111-8111-000000000001";
pub const SESS_B: &str = "bbbbbbbb-2222-4222-8222-000000000002";

pub struct Rig {
    pub core: Arc<Core>,
    pub host: Host,
    pub data: PathBuf,
    pub claude: PathBuf,
    pub key_a: String,
    pub key_b: String,
    _dirs: Vec<tempfile::TempDir>,
}

fn proc_start_field() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    let st = sys::proc::start_time(i64::from(std::process::id()))?;
    Some(sys::proc::filetime_ticks(st).to_string())
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// A live session file for this process.
pub fn write_session(dir: &Path, file: &str, id: &str, name: &str, cwd: &Path) {
    let mut m = serde_json::json!({
        "pid": std::process::id(), "sessionId": id, "cwd": cwd.to_string_lossy(), "name": name,
        "status": "idle", "kind": "bg", "entrypoint": "cli", "version": "2.1.311",
        "startedAt": unix_ms(), "updatedAt": unix_ms(),
    });
    if let Some(ps) = proc_start_field() {
        m["procStart"] = ps.into();
    }
    std::fs::write(dir.join(file), serde_json::to_vec(&m).unwrap()).unwrap();
}

pub fn options(data: &Path, claude: &Path) -> Options {
    Options {
        data_dir: data.to_path_buf(),
        claude_dir: claude.to_path_buf(),
        version: "0.2.0-test".into(),
        headless: true,
        keystore: Some(Box::new(irori_identity::keystore::FileKeystore)),
        ..Options::default()
    }
}

impl Rig {
    pub async fn new() -> Rig {
        Rig::with(Duration::ZERO, |_| {}).await
    }

    /// The rig with a keepalive (zero: 15 s) and `edit` applied to the
    /// Core's options.
    pub async fn with(keepalive: Duration, edit: impl FnOnce(&mut Options)) -> Rig {
        let (d, c, w) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let sessions = c.path().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        write_session(&sessions, "900001.json", SESS_A, "alpha", w.path());
        write_session(&sessions, "900002.json", SESS_B, "beta", w.path());
        let mut o = options(d.path(), c.path());
        edit(&mut o);
        let core = Arc::new(tokio::task::block_in_place(|| Core::start(o)).unwrap());
        let me = core.device_id().to_string();
        let (key_a, key_b) = (format!("{me}/{SESS_A}"), format!("{me}/{SESS_B}"));
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let n = core
                .local_agents()
                .iter()
                .filter(|c| c.key == key_a || c.key == key_b)
                .count();
            if n == 2 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "presence never listed both sessions"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let host = Host::start(core.clone(), keepalive).await;
        Rig {
            core,
            host,
            data: d.path().to_path_buf(),
            claude: c.path().to_path_buf(),
            key_a,
            key_b,
            _dirs: vec![d, c, w],
        }
    }

    pub fn token(&self) -> String {
        self.core.token().to_string()
    }

    /// One request: its status, headers and whole body.
    pub async fn request(
        &self,
        method: &str,
        path: &str,
        hdr: &[(&str, &str)],
        body: Option<Value>,
    ) -> (u16, hyper::HeaderMap, Vec<u8>) {
        let resp = send(self.host.port, method, path, &self.token(), hdr, body).await;
        let status = resp.status().as_u16();
        let headers = resp.headers().clone();
        let b = resp
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        (status, headers, b)
    }

    pub async fn do_(
        &self,
        method: &str,
        path: &str,
        hdr: &[(&str, &str)],
        body: Option<Value>,
    ) -> (u16, String) {
        let (s, _, b) = self.request(method, path, hdr, body).await;
        (s, String::from_utf8(b).unwrap())
    }

    /// Opens `/v1/attach?<q>`: the stream, or the refusal's status.
    pub async fn open(&self, q: &str) -> Result<EvStream, u16> {
        let resp = send(
            self.host.port,
            "GET",
            &format!("/v1/attach?{q}"),
            &self.token(),
            &[],
            None,
        )
        .await;
        if resp.status() != 200 {
            return Err(resp.status().as_u16());
        }
        let (tx, rx) = mpsc::channel(256);
        let mut body = resp.into_body();
        let task = tokio::spawn(async move {
            let mut buf = Vec::new();
            while let Some(Ok(f)) = body.frame().await {
                let Ok(d) = f.into_data() else { continue };
                buf.extend_from_slice(&d);
                while let Some(i) = buf.iter().position(|&c| c == b'\n') {
                    let line: Vec<u8> = buf.drain(..=i).collect();
                    let line = String::from_utf8(line).unwrap();
                    let t = line.trim();
                    if t.is_empty() {
                        continue;
                    }
                    let v: Value =
                        serde_json::from_str(t).unwrap_or_else(|e| panic!("bad line {t:?}: {e}"));
                    if tx.send(v).await.is_err() {
                        return;
                    }
                }
            }
        });
        Ok(EvStream { rx, task })
    }

    /// A mod stream naming `mod_version`, and its hello.
    pub async fn attach_mod(&self, session: &str, mod_version: &str) -> (EvStream, Value) {
        let mut s = self
            .open(&format!("session={session}&mod={mod_version}"))
            .await
            .unwrap_or_else(|c| panic!("attach {session}: status {c}"));
        let hello = s.next("hello").await;
        assert!(
            !str_of(&hello, "attachSecret").is_empty()
                && hello["api"]["token"] == self.token().as_str()
                && str_of(&hello["api"], "url").starts_with("http://127.0.0.1:"),
            "hello {hello}"
        );
        (s, hello)
    }

    /// Go `rig.attach`: a mod stream at mod=0.2.0-test (before appends).
    pub async fn attach(&self, session: &str) -> (EvStream, Value) {
        self.attach_mod(session, "0.2.0-test").await
    }

    /// Go `rig.control`: a deliveries report that must answer `{"ok":true}`.
    pub async fn control(&self, secret: &str, what: &str, token: &str, turn: &str) {
        let (code, raw) = self
            .do_(
                "POST",
                &format!("/v1/deliveries/{what}"),
                &[(attach::HEADER_ATTACH, secret)],
                Some(serde_json::json!({"token": token, "turnId": turn})),
            )
            .await;
        assert!(
            code == 200 && raw.contains("\"ok\":true"),
            "{what}: {code} {raw}"
        );
    }

    pub fn state(&self, id: &str) -> String {
        self.core
            .inbox()
            .get(id)
            .unwrap_or_else(|| panic!("no delivery {id}"))
            .state
    }

    pub fn sender(&self, key: &str, name: &str) -> inbox::Sender {
        inbox::Sender::new(key, name, "claude")
    }
}

/// An HTTP/1.1 request to the host.
pub async fn send(
    port: u16,
    method: &str,
    path: &str,
    token: &str,
    hdr: &[(&str, &str)],
    body: Option<Value>,
) -> hyper::Response<hyper::body::Incoming> {
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let (mut s, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tcp))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let mut req = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("Host", format!("127.0.0.1:{port}"))
        .header("Authorization", format!("Bearer {token}"));
    for (k, v) in hdr {
        req = req.header(*k, *v);
    }
    let raw = body
        .map(|b| serde_json::to_vec(&b).unwrap())
        .unwrap_or_default();
    s.send_request(req.body(Full::new(Bytes::from(raw))).unwrap())
        .await
        .unwrap()
}

pub fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

pub fn strs(v: &Value, k: &str) -> Vec<String> {
    v[k].as_array()
        .map(|a| {
            a.iter()
                .map(|x| x.as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// An open attach stream read in the background.
pub struct EvStream {
    rx: mpsc::Receiver<Value>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for EvStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl EvStream {
    /// The next event of type `t`, skipping others.
    pub async fn next(&mut self, t: &str) -> Value {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            match tokio::time::timeout_at(deadline, self.rx.recv()).await {
                Err(_) => panic!("no {t} event"),
                Ok(None) => panic!("stream closed waiting for {t}"),
                Ok(Some(ev)) if ev["type"] == t => return ev,
                Ok(Some(_)) => {}
            }
        }
    }

    /// No event of type `t` within `d`.
    pub async fn none(&mut self, t: &str, d: Duration) {
        let deadline = tokio::time::Instant::now() + d;
        loop {
            match tokio::time::timeout_at(deadline, self.rx.recv()).await {
                Err(_) | Ok(None) => return,
                Ok(Some(ev)) if ev["type"] == t => panic!("unexpected {t}: {ev}"),
                Ok(Some(_)) => {}
            }
        }
    }

    /// Every event within `d`.
    pub async fn collect(&mut self, d: Duration) -> Vec<Value> {
        let deadline = tokio::time::Instant::now() + d;
        let mut out = vec![];
        while let Ok(Some(ev)) = tokio::time::timeout_at(deadline, self.rx.recv()).await {
            out.push(ev);
        }
        out
    }

    /// The stream closes, after an event of type `t` (empty: any).
    pub async fn ended(&mut self, t: &str) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut seen = t.is_empty();
        loop {
            match tokio::time::timeout_at(deadline, self.rx.recv()).await {
                Err(_) => panic!("stream did not end"),
                Ok(None) => {
                    assert!(seen, "stream ended without {t}");
                    return;
                }
                Ok(Some(ev)) => {
                    if ev["type"] == t {
                        seen = true;
                    }
                }
            }
        }
    }

    /// Closes the client side.
    pub fn close(self) {}
}
