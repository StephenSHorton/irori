//! The Go core tests' rig (I:internal/core/v02_test.go newRig): a Core over
//! a scratch data dir and a fake Claude dir holding two live sessions,
//! alpha (A) and beta (B), with the file keystore. No network: nothing
//! here opens a socket.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use irori_core::{Core, Options};

pub const SESS_A: &str = "aaaaaaaa-1111-4111-8111-000000000001";
pub const SESS_B: &str = "bbbbbbbb-2222-4222-8222-000000000002";

pub struct Rig {
    pub core: Core,
    pub data: PathBuf,
    pub claude: PathBuf,
    pub key_a: String,
    pub key_b: String,
    _dirs: Vec<tempfile::TempDir>,
}

/// This process's start as Claude Code writes `procStart` (Windows only).
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
    pub fn new() -> Rig {
        Rig::with(|_| {})
    }

    /// The rig with `edit` applied to the Core's options first.
    pub fn with(edit: impl FnOnce(&mut Options)) -> Rig {
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
        let core = Core::start(o).unwrap();
        let me = core.device_id().to_string();
        let r = Rig {
            key_a: format!("{me}/{SESS_A}"),
            key_b: format!("{me}/{SESS_B}"),
            data: d.path().to_path_buf(),
            claude: c.path().to_path_buf(),
            core,
            _dirs: vec![d, c, w],
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let n = r
                .core
                .local_agents()
                .iter()
                .filter(|c| c.key == r.key_a || c.key == r.key_b)
                .count();
            if n == 2 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "presence never listed both sessions: {:?}",
                r.core.local_agents()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        r
    }

    /// The Board notices delivered to `key`.
    pub fn notices_to(&self, key: &str) -> Vec<inbox::Delivery> {
        self.core
            .inbox()
            .for_agent(key, true, 0)
            .into_iter()
            .filter(|d| d.message.notice)
            .collect()
    }
}

/// Waits until `ok` holds, or fails after `secs` seconds.
pub fn wait_for(secs: u64, what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}
