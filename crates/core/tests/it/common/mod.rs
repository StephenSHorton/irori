//! The Go core tests' rig (I:internal/core/v02_test.go newRig): a Core over
//! a scratch data dir and a fake Claude dir holding two live sessions,
//! alpha (A) and beta (B), with the file keystore. No network: nothing
//! here opens a socket.
#![allow(dead_code)]

pub mod chat;
pub mod mesh;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use irori_core::{Core, Options};

pub mod pair;

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

/// Runs git in `dir` and gives its output; a failure fails the test.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A main checkout at `dir` with one commit pushed to a bare origin beside
/// it, ignoring `/target` as a Rust repo does.
pub fn git_repo(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let origin = dir.with_extension("origin.git");
    std::fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "-q", "--bare", "-b", "main"]);
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["remote", "add", "origin", &origin.to_string_lossy()]);
    std::fs::write(dir.join(".gitignore"), "/target\n/target.irori-clearing\n").unwrap();
    std::fs::write(dir.join("src.txt"), "one\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "one"]);
    git(dir, &["push", "-q", "origin", "main"]);
    dir.to_path_buf()
}

/// A tree Irori made in `main`: `.claude/worktrees/<name>` on its own branch.
pub fn git_tree(main: &Path, name: &str) -> PathBuf {
    let w = main.join(".claude/worktrees").join(name);
    git(
        main,
        &["worktree", "add", "-q", "-b", name, &w.to_string_lossy()],
    );
    w
}

/// Gives worktree `w` a cargo `target/`, its lock file included.
pub fn git_target(w: &Path) {
    std::fs::create_dir_all(w.join("target/debug/deps")).unwrap();
    std::fs::write(
        w.join("target/CACHEDIR.TAG"),
        "Signature: 8a477f597d28d172789f06886806bc55\n",
    )
    .unwrap();
    std::fs::write(w.join("target/debug/.cargo-lock"), "").unwrap();
    std::fs::write(w.join("target/debug/deps/libx.rlib"), vec![0u8; 4096]).unwrap();
}

/// This process's start as Claude Code writes `procStart` (Windows only).
fn proc_start_field() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    let st = sys::proc::start_time(i64::from(std::process::id()))?;
    Some(sys::proc::filetime_ticks(st).to_string())
}

pub fn unix_ms() -> i64 {
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

    /// The rig with its Core stopped and started again on the same data
    /// and sessions, `edit` applied to the new Core's options: an app
    /// restart (card #1215).
    pub fn restarted(self, edit: impl FnOnce(&mut Options)) -> Rig {
        let Rig {
            core,
            data,
            claude,
            key_a,
            key_b,
            _dirs,
        } = self;
        core.close().unwrap();
        let mut o = options(&data, &claude);
        edit(&mut o);
        let core = Core::start(o).unwrap();
        Rig {
            core,
            data,
            claude,
            key_a,
            key_b,
            _dirs,
        }
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
        // A long deadline: it returns as soon as presence lists both, and
        // a Mac busy with other builds can take well over 10 s (#402).
        let deadline = Instant::now() + Duration::from_secs(60);
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

/// Waits until `ok` holds, or fails after `secs` seconds, and never
/// sooner than 60: it returns as soon as `ok` holds, and a Mac busy with
/// other builds can be many seconds late (#402).
pub fn wait_for(secs: u64, what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs.max(60));
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}
