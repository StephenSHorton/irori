//! The real `irori attach` binary, spawned the way the Irori Claude Code
//! mod spawns it (mod/hooks/register.tsx: `[iroriPath, "attach",
//! "--session", id, "--cwd", cwd, "--mod-version", "0.4.1"]`, stdin
//! closed, stdout and stderr piped), against a Rust Core on a scratch
//! `IRORI_HOME` served by the real `irori_api::Server`, with its lockfile,
//! as `irori serve` hosts it. This replaces the Go mod's `mod/dev/fakecore`
//! + `fakeirori` live checks below the mod.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

/// The API server over `core`, as `irori serve` runs it. Dropping it stops
/// the listener.
struct Host {
    port: u16,
    stop: CancellationToken,
}

impl Host {
    async fn start(core: Arc<irori_core::Core>) -> Host {
        let server = irori_api::Server::new(core.clone(), core.token())
            .with_tools(Arc::new(irori_api::CoreTools(core.clone())));
        let ln = irori_api::listen().await.unwrap();
        let port = ln.local_addr().unwrap().port();
        core.write_lockfile(port).expect("write the lockfile");
        let stop = CancellationToken::new();
        let s = stop.clone();
        tokio::spawn(async move { server.serve(ln, s).await });
        Host { port, stop }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

const SESS_A: &str = "aaaaaaaa-1111-4111-8111-000000000001";
const SESS_B: &str = "bbbbbbbb-2222-4222-8222-000000000002";

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn write_session(dir: &Path, file: &str, id: &str, name: &str, cwd: &Path) {
    let mut m = serde_json::json!({
        "pid": std::process::id(), "sessionId": id, "cwd": cwd.to_string_lossy(), "name": name,
        "status": "idle", "kind": "bg", "entrypoint": "cli", "version": "2.1.311",
        "startedAt": unix_ms(), "updatedAt": unix_ms(),
    });
    if cfg!(windows)
        && let Some(st) = sys::proc::start_time(i64::from(std::process::id()))
    {
        m["procStart"] = sys::proc::filetime_ticks(st).to_string().into();
    }
    std::fs::write(dir.join(file), serde_json::to_vec(&m).unwrap()).unwrap();
}

/// One spawned `irori attach`: its stdout lines as they come.
struct Exe {
    child: Child,
    lines: mpsc::Receiver<String>,
}

impl Exe {
    fn spawn(home: &Path, session: &str) -> Exe {
        let mut child = Command::new(env!("CARGO_BIN_EXE_irori"))
            .args([
                "attach",
                "--session",
                session,
                "--cwd",
                "C:/work/api",
                "--mod-version",
                "0.4.1",
            ])
            .env("IRORI_HOME", home)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn irori attach");
        let (tx, rx) = mpsc::channel();
        let out = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for l in BufReader::new(out).lines() {
                let Ok(l) = l else { return };
                if tx.send(l).is_err() {
                    return;
                }
            }
        });
        let err = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            for l in BufReader::new(err).lines().map_while(Result::ok) {
                eprintln!("irori attach stderr: {l}");
            }
        });
        Exe { child, lines: rx }
    }

    /// The next stdout line, raw.
    fn line(&self) -> String {
        self.lines
            .recv_timeout(Duration::from_secs(15))
            .expect("a stdout line within 15 s")
    }

    /// The next line of type `t`, raw and parsed; every line on the way is
    /// JSON (nothing else reaches stdout).
    fn next(&self, t: &str) -> (String, Value) {
        loop {
            let raw = self.line();
            let v: Value = serde_json::from_str(&raw)
                .unwrap_or_else(|e| panic!("stdout line {raw:?} is not JSON: {e}"));
            if v["type"] == t {
                return (raw, v);
            }
        }
    }

    fn exit_code(&mut self) -> i32 {
        for _ in 0..150 {
            if let Some(s) = self.child.try_wait().unwrap() {
                return s.code().unwrap_or(-1);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("irori attach did not exit");
    }
}

impl Drop for Exe {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn post(port: u16, token: &str, path: &str, secret: &str, body: &str) -> (u16, String) {
    use http_body_util::{BodyExt, Full};
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let (mut s, conn) = hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(tcp))
        .await
        .unwrap();
    tokio::spawn(conn);
    let req = hyper::Request::post(path)
        .header("Host", format!("127.0.0.1:{port}"))
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .header(attach::HEADER_ATTACH, secret)
        .body(Full::new(bytes::Bytes::from(body.to_string())))
        .unwrap();
    let resp = s.send_request(req).await.unwrap();
    let code = resp.status().as_u16();
    let b = resp.into_body().collect().await.unwrap().to_bytes();
    (code, String::from_utf8(b.to_vec()).unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn the_mods_spawn_against_a_rust_core() {
    let (data, claude, work) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );

    // Before any Core: the one outage line, then quiet retries.
    let early = Exe::spawn(data.path(), SESS_B);
    let (raw, _) = early.next("error");
    assert_eq!(
        raw,
        "{\"type\":\"error\",\"message\":\"Irori isn't running, so messages can't reach this session. It attaches by itself once Irori starts.\"}"
    );

    let sessions = claude.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    write_session(&sessions, "900001.json", SESS_A, "alpha", work.path());
    write_session(&sessions, "900002.json", SESS_B, "api work", work.path());
    let core = Arc::new(
        tokio::task::block_in_place(|| {
            irori_core::Core::start(irori_core::Options {
                data_dir: data.path().to_path_buf(),
                claude_dir: claude.path().to_path_buf(),
                version: "0.2.0-test".into(),
                headless: true,
                keystore: Some(Box::new(irori_identity::keystore::FileKeystore)),
                ..irori_core::Options::default()
            })
        })
        .unwrap(),
    );
    let me = core.device_id().to_string();
    let (key_a, key_b) = (format!("{me}/{SESS_A}"), format!("{me}/{SESS_B}"));
    for _ in 0..200 {
        if core
            .local_agents()
            .iter()
            .filter(|c| c.key == key_a || c.key == key_b)
            .count()
            == 2
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let host = Host::start(core.clone()).await;

    // The early attach finds it by itself (backoff is at most 16 s; the
    // first retries are 1, 2, 4 s).
    let (_, hello) = tokio::task::block_in_place(|| early.next("hello"));
    assert_eq!(hello["agentKey"], key_b.as_str());
    assert_eq!(hello["name"], "api work");
    assert_eq!(hello["v"], 2);
    assert_eq!(
        hello["api"]["url"],
        format!("http://127.0.0.1:{}", host.port)
    );
    let secret = hello["attachSecret"].as_str().unwrap().to_string();
    assert_eq!(secret.len(), 48);
    let (raw, _) = tokio::task::block_in_place(|| early.next("inbox"));
    assert_eq!(raw, "{\"type\":\"inbox\"}");

    // A message from alpha: the deliver line exactly as the Core wrote it
    // (Go's HTML escapes kept), which the mod submits.
    let (_, d) = core
        .inbox()
        .send(
            &inbox::Sender::new(&key_a, "alpha", "claude"),
            &inbox::Address::new(inbox::addr::AGENT, "api work"),
            "hi <b> & more",
            "",
        )
        .unwrap();
    let (raw, ev) = tokio::task::block_in_place(|| early.next("deliver"));
    assert!(
        raw.contains("\\u003cirori-message from=\\\"alpha\\\""),
        "{raw}"
    );
    assert!(raw.contains("hi \\u003cb\\u003e \\u0026 more"), "{raw}");
    let token = ev["token"].as_str().unwrap().to_string();
    assert!(ev["prompt"].as_str().unwrap().starts_with(&token));
    assert_eq!(ev["deliveryIds"], serde_json::json!([d.id]));
    assert_eq!(ev["from"], "alpha");

    // The mod's reports, with its exact bodies and headers.
    let tok = gocompat::json::quote_string(&token, false);
    let tk = core.token().to_string();
    let (code, body) = post(
        host.port,
        &tk,
        "/v1/deliveries/delivered",
        &secret,
        &format!("{{\"token\":{tok}}}"),
    )
    .await;
    assert_eq!((code, body.as_str()), (200, "{\"ok\":true}\n"));
    let (code, _) = post(
        host.port,
        &tk,
        "/v1/deliveries/engaged",
        &secret,
        &format!("{{\"token\":{tok},\"turnId\":\"turn-1\"}}"),
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(core.inbox().get(&d.id).unwrap().state, inbox::st::ENGAGED);
    let (code, _) = post(
        host.port,
        &tk,
        "/v1/deliveries/turn-ended",
        &secret,
        &format!("{{\"token\":{tok},\"turnId\":\"turn-1\"}}"),
    )
    .await;
    assert_eq!(code, 200);

    // A next-turn item (the person's, mode next_turn) to a 0.4.1 mod is an
    // append.
    core.inbox()
        .send_with(
            &core.user_sender(),
            &inbox::Address::new(inbox::addr::AGENT, "api work"),
            "when you're next up",
            inbox::SendOptions::default(),
        )
        .unwrap();
    let (_, ap) = tokio::task::block_in_place(|| early.next("append"));
    assert_eq!(ap["mode"], "next_turn");
    assert!(ap["token"].as_str().unwrap().starts_with("[irori-append:"));

    // A reload: the new attach supersedes the old, which exits 0 after its
    // superseded line; the old secret stops working.
    let mut early = early;
    let second = Exe::spawn(data.path(), SESS_B);
    let (_, h2) = tokio::task::block_in_place(|| second.next("hello"));
    let (raw, _) = tokio::task::block_in_place(|| early.next("superseded"));
    assert_eq!(raw, "{\"type\":\"superseded\"}");
    assert_eq!(tokio::task::block_in_place(|| early.exit_code()), 0);
    let (code, _) = post(
        host.port,
        &tk,
        "/v1/deliveries/delivered",
        &secret,
        &format!("{{\"token\":{tok}}}"),
    )
    .await;
    assert_eq!(code, 403);
    // The unconfirmed append goes out again to the new attach, same token.
    let (_, again) = tokio::task::block_in_place(|| second.next("append"));
    assert_eq!(again["token"], ap["token"]);
    assert_ne!(h2["attachSecret"], hello["attachSecret"]);

    // A session the Core doesn't list is retried, never an error line.
    let unknown = Exe::spawn(data.path(), "dddddddd-4444-4444-8444-000000000004");
    let got = unknown.lines.recv_timeout(Duration::from_millis(1500));
    assert!(got.is_err(), "stdout stays quiet: {got:?}");

    // Flags it doesn't know end it with Go's text, exit 1.
    let out = Command::new(env!("CARGO_BIN_EXE_irori"))
        .args(["attach", "--bogus"])
        .env("IRORI_HOME", data.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("irori: flag provided but not defined: -bogus"),
        "{err}"
    );
    drop(host);
}
