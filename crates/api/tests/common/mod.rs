//! A Core over a scratch data dir and a fake Claude dir holding two live
//! sessions, alpha (A) and beta (B), with the file keystore (Go's core
//! newRig), served by an api::Server on 127.0.0.1:0. Nothing else listens.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use irori_api::Server;
use irori_api::client::Client;
use irori_core::{Core, Options};
use tokio_util::sync::CancellationToken;

pub const SESS_A: &str = "aaaaaaaa-1111-4111-8111-000000000001";
pub const SESS_B: &str = "bbbbbbbb-2222-4222-8222-000000000002";

pub struct Rig {
    pub core: Arc<Core>,
    pub server: Server,
    pub client: Client,
    pub port: u16,
    pub token: String,
    pub data: PathBuf,
    pub claude: PathBuf,
    pub cwd: PathBuf,
    pub key_a: String,
    pub key_b: String,
    pub rt: tokio::runtime::Runtime,
    stop: CancellationToken,
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

/// A live session file for this process, active at `ms` (Unix ms). The
/// rig's two sessions share one `ms`, so the roster orders them by key as
/// the Go golden recorded, not by which file was written a millisecond
/// later.
pub fn write_session(dir: &Path, file: &str, id: &str, name: &str, cwd: &Path, ms: i64) {
    let mut m = serde_json::json!({
        "pid": std::process::id(), "sessionId": id, "cwd": cwd.to_string_lossy(), "name": name,
        "status": "idle", "kind": "bg", "entrypoint": "cli", "version": "2.1.311",
        "startedAt": ms, "updatedAt": ms,
    });
    if let Some(ps) = proc_start_field() {
        m["procStart"] = ps.into();
    }
    std::fs::write(dir.join(file), serde_json::to_vec(&m).unwrap()).unwrap();
}

impl Rig {
    pub fn new() -> Rig {
        Rig::build(None)
    }

    /// The rig after `seed` ran on a first Core over the same data dir and
    /// that Core closed: rows capture folds only on replay (Go's fixtures
    /// write them before capture.New) are in the log when the served Core
    /// starts.
    pub fn seeded(seed: impl FnOnce(&Core)) -> Rig {
        Rig::build(Some(Box::new(seed)))
    }

    #[allow(clippy::type_complexity)]
    fn build(seed: Option<Box<dyn FnOnce(&Core) + '_>>) -> Rig {
        let (d, c, w) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let sessions = c.path().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let now = unix_ms();
        write_session(&sessions, "900001.json", SESS_A, "alpha", w.path(), now);
        write_session(&sessions, "900002.json", SESS_B, "beta", w.path(), now);
        let start = || {
            Core::start(Options {
                data_dir: d.path().to_path_buf(),
                claude_dir: c.path().to_path_buf(),
                version: "0.2.0-test".into(),
                headless: true,
                keystore: Some(Box::new(irori_identity::keystore::FileKeystore)),
                ..Options::default()
            })
            .unwrap()
        };
        if let Some(seed) = seed {
            let first = start();
            seed(&first);
            first.close().unwrap();
        }
        let core = Arc::new(start());
        let me = core.device_id().to_string();
        let (key_a, key_b) = (format!("{me}/{SESS_A}"), format!("{me}/{SESS_B}"));
        // Presence lists them within a scan or two; a loaded machine (a
        // parallel build) has taken over 10 s.
        let deadline = Instant::now() + Duration::from_secs(30);
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
            std::thread::sleep(Duration::from_millis(50));
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let token = core.token().to_string();
        let server = Server::new(core.clone(), &token)
            .with_tools(Arc::new(irori_api::CoreTools(core.clone())));
        let ln = rt.block_on(irori_api::listen()).unwrap();
        let port = ln.local_addr().unwrap().port();
        let stop = CancellationToken::new();
        {
            let (srv, stop) = (server.clone(), stop.clone());
            rt.spawn(async move { srv.serve(ln, stop).await });
        }
        Rig {
            client: Client::at(port, &token),
            core,
            server,
            port,
            token,
            data: d.path().to_path_buf(),
            claude: c.path().to_path_buf(),
            cwd: w.path().to_path_buf(),
            key_a,
            key_b,
            rt,
            stop,
            _dirs: vec![d, c, w],
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.stop.cancel();
        std::thread::sleep(Duration::from_millis(50));
        let _ = self.core.close();
    }
}
