//! The window's services on a scratch Core: a scratch data dir and a fake
//! Claude dir with two live sessions, the file keystore, no socket, no
//! home server.
#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use irori_core::{Core, Options};
use irori_window::{Config, Window};

pub const SESS_A: &str = "aaaaaaaa-1111-4111-8111-000000000001";
pub const SESS_B: &str = "bbbbbbbb-2222-4222-8222-000000000002";

pub struct Rig {
    pub w: Arc<Window>,
    pub key_a: String,
    pub key_b: String,
    pub data: std::path::PathBuf,
    pub _dirs: Vec<tempfile::TempDir>,
}

pub fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

pub fn write_session(dir: &Path, file: &str, id: &str, name: &str, cwd: &Path, ms: i64) {
    let mut m = serde_json::json!({
        "pid": std::process::id(), "sessionId": id, "cwd": cwd.to_string_lossy(), "name": name,
        "status": "idle", "kind": "bg", "entrypoint": "cli", "version": "2.1.311",
        "startedAt": ms, "updatedAt": ms,
    });
    if cfg!(windows)
        && let Some(st) = sys::proc::start_time(i64::from(std::process::id()))
    {
        m["procStart"] = sys::proc::filetime_ticks(st).to_string().into();
    }
    std::fs::write(dir.join(file), serde_json::to_vec(&m).unwrap()).unwrap();
}

impl Rig {
    pub fn new() -> Rig {
        Rig::with(|_| {})
    }

    /// A Rig whose Core options `edit` changes first.
    pub fn with(edit: impl FnOnce(&mut Options)) -> Rig {
        let (d, c, wd, dl) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let sessions = c.path().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let now = unix_ms();
        write_session(&sessions, "900001.json", SESS_A, "alpha", wd.path(), now);
        write_session(&sessions, "900002.json", SESS_B, "beta", wd.path(), now);
        let mut opts = Options {
            data_dir: d.path().to_path_buf(),
            claude_dir: c.path().to_path_buf(),
            version: "0.6.0-test".into(),
            headless: false,
            keystore: Some(Box::new(irori_identity::keystore::FileKeystore)),
            // The home server never runs in these tests.
            no_home: true,
            ..Options::default()
        };
        edit(&mut opts);
        let core = Arc::new(Core::start(opts).unwrap());
        let me = core.device_id().to_string();
        let (key_a, key_b) = (format!("{me}/{SESS_A}"), format!("{me}/{SESS_B}"));
        let deadline = Instant::now() + Duration::from_secs(10);
        while core
            .local_agents()
            .iter()
            .filter(|c| c.key == key_a || c.key == key_b)
            .count()
            < 2
        {
            assert!(
                Instant::now() < deadline,
                "presence never listed both sessions"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        let downloads = dl.path().to_path_buf();
        let w = Arc::new(Window::new(
            core.clone(),
            Window::default_seams(&core),
            Config {
                claude_dir: c.path().to_path_buf(),
                downloads: Box::new(move || Ok(downloads.clone())),
            },
        ));
        Rig {
            w,
            key_a,
            key_b,
            data: d.path().to_path_buf(),
            _dirs: vec![d, c, wd, dl],
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = self.w.core.close();
    }
}

/// Waits for `ok`, up to 10 s.
pub fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}
