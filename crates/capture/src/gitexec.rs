//! Running the git binary the way gitlink.Exec and repo's runGit do
//! (capture.md §2.5, §7 trap 11): found on PATH (with PATHEXT on Windows,
//! never in the current directory), no shell, no console window, the
//! repo-selecting `GIT_*` variables removed, a timeout that kills the
//! process, and a 500 ms wait for its pipes after that. A run that exits by
//! itself waits for its pipes until its timeout (at least 500 ms): unread
//! output is a timeout, never an empty success.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use sys::cancel::Cancel;

/// Variables that point git at one repository regardless of `-C`.
pub const REPO_ENV_VARS: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CEILING_DIRECTORIES",
    "GIT_DISCOVERY_ACROSS_FILESYSTEM",
    "GIT_NAMESPACE",
];

/// Go `WaitDelay`: how long the pipes may stay open after the process ends
/// or is killed.
const WAIT_DELAY: Duration = Duration::from_millis(500);

/// How one git run failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    /// The timeout passed; the process was killed.
    Timeout,
    /// The token was canceled; the process was killed.
    Canceled,
    /// git ran and exited non-zero (`code` is -1 when it was signalled).
    Exit { code: i32, stderr: String },
    /// git could not be started (not on PATH, ...).
    Spawn(String),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::Timeout => f.write_str("context deadline exceeded"),
            ExecError::Canceled => f.write_str("context canceled"),
            ExecError::Exit { code, .. } => write!(f, "exit status {code}"),
            ExecError::Spawn(e) => f.write_str(e),
        }
    }
}

/// Go `sameEnvName`: Windows ignores case.
fn same_env_name(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        gocompat::text::equal_fold(a, b)
    } else {
        a == b
    }
}

/// Go `gitEnv`: `env` without the repo-selecting git variables.
pub fn git_env(env: Vec<(String, String)>) -> Vec<(String, String)> {
    env.into_iter()
        .filter(|(k, _)| !REPO_ENV_VARS.iter().any(|v| same_env_name(k, v)))
        .collect()
}

/// Go `exec.LookPath("git")`, cached: the first PATH entry holding git (an
/// absolute entry; relative ones, the current directory included, are
/// skipped), trying PATHEXT's extensions on Windows.
pub fn git_path() -> Option<PathBuf> {
    static GIT: OnceLock<Option<PathBuf>> = OnceLock::new();
    GIT.get_or_init(|| look_path("git")).clone()
}

fn look_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| e.to_ascii_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        if !dir.is_absolute() {
            continue;
        }
        for ext in &exts {
            let p = dir.join(format!("{name}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Runs git with `args` (the caller adds `-C` if it wants one) and returns
/// its stdout bytes, untouched. `extra_env` is appended to the filtered
/// environment.
pub fn run(
    cancel: &Cancel,
    timeout: Duration,
    args: &[String],
    extra_env: &[(&str, &str)],
) -> Result<(Vec<u8>, Vec<u8>), ExecError> {
    if cancel.is_canceled() {
        return Err(ExecError::Canceled);
    }
    let Some(git) = git_path() else {
        return Err(ExecError::Spawn(
            "exec: \"git\": executable file not found in %PATH%".into(),
        ));
    };
    let mut cmd = Command::new(git);
    cmd.args(args)
        .env_clear()
        .envs(git_env(
            std::env::vars_os()
                .map(|(k, v)| {
                    (
                        k.to_string_lossy().into_owned(),
                        v.to_string_lossy().into_owned(),
                    )
                })
                .collect(),
        ))
        .envs(extra_env.iter().map(|(k, v)| (*k, *v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashes up for a GUI parent.
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| ExecError::Spawn(e.to_string()))?;
    let pipe = |r: Option<Box<dyn Read + Send>>| {
        let (tx, rx) = mpsc::channel();
        if let Some(mut r) = r {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = r.read_to_end(&mut buf);
                let _ = tx.send(buf);
            });
        } else {
            let _ = tx.send(Vec::new());
        }
        rx
    };
    let out_rx = pipe(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let err_rx = pipe(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let deadline = Instant::now() + timeout;
    let mut killed: Option<ExecError> = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {}
            Err(_) => break None,
        }
        if cancel.is_canceled() {
            killed = Some(ExecError::Canceled);
        } else if Instant::now() >= deadline {
            killed = Some(ExecError::Timeout);
        }
        if killed.is_some() {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        cancel.sleep(Duration::from_millis(2));
    };
    if let Some(e) = killed {
        let _ = out_rx.recv_timeout(WAIT_DELAY);
        let _ = err_rx.recv_timeout(WAIT_DELAY);
        return Err(e);
    }
    // git exited by itself. Its output is in the pipes, but the reader
    // threads may not have run yet: on a loaded machine a fresh thread can
    // wait more than WAIT_DELAY for a core. Waiting only WAIT_DELAY here
    // and taking what was read returned a successful run with empty stdout
    // (a rev-list with no commits, a repo with no root), so the readers get
    // the rest of the run's own timeout, and at least WAIT_DELAY (a
    // grandchild holding a pipe open is what this bound is for). Output
    // still unread then is a timeout, never an empty success.
    let pipes_by = deadline.max(Instant::now() + WAIT_DELAY);
    let wait = || pipes_by.saturating_duration_since(Instant::now());
    let (Ok(stdout), Ok(stderr)) = (out_rx.recv_timeout(wait()), err_rx.recv_timeout(wait()))
    else {
        return Err(ExecError::Timeout);
    };
    let Some(status) = status else {
        return Err(ExecError::Spawn("wait failed".into()));
    };
    if !status.success() {
        return Err(ExecError::Exit {
            code: status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        });
    }
    Ok((stdout, stderr))
}
