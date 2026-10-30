//! A working directory's repo_key, the same on every device: a port of
//! I:internal/repo (capture.md §0 I8).
//!
//! `remote:<host>/<owner>/<name>` from the normalized origin remote, else
//! `root:<sha>` from the repository's smallest root commit, else "" when the
//! directory isn't in a git repo. Keys are cached per directory and per
//! toplevel for five minutes ("" included); a git run that times out is
//! never cached.

mod normalize;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gocompat::time::GoTime;
use sys::cancel::Cancel;

pub use normalize::normalize_remote;

use crate::gitexec::{self, ExecError};
use crate::gotime;

/// One git run's limit.
pub const GIT_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a key stays cached.
pub const CACHE_TTL: Duration = Duration::from_secs(5 * 60);

/// Runs git with `args` within `timeout` and returns its trimmed stdout.
pub type RunFn = Arc<dyn Fn(Duration, &[String]) -> Result<String, ExecError> + Send + Sync>;
/// The clock.
pub type NowFn = Arc<dyn Fn() -> GoTime + Send + Sync>;

#[derive(Clone)]
struct Entry {
    key: String,
    expires: GoTime,
}

/// Caches keys per directory. Safe for concurrent use.
pub struct Resolver {
    run: RunFn,
    now: NowFn,
    ttl: Duration,
    timeout: Duration,
    by_dir: Mutex<HashMap<String, Entry>>,
    by_root: Mutex<HashMap<String, Entry>>,
}

impl Default for Resolver {
    fn default() -> Self {
        Resolver::new()
    }
}

/// Go `runGit`: git from PATH, no shell, no console window, stdout trimmed.
pub fn run_git(timeout: Duration, args: &[String]) -> Result<String, ExecError> {
    let (out, _) = gitexec::run(
        &Cancel::new(),
        timeout,
        args,
        &[("GIT_TERMINAL_PROMPT", "0"), ("GIT_OPTIONAL_LOCKS", "0")],
    )?;
    Ok(crate::jsonx::trim_space(&String::from_utf8_lossy(&out)).to_string())
}

enum GitErr {
    Timeout,
    Failed,
}

impl Resolver {
    /// A resolver over the real git binary and clock.
    pub fn new() -> Resolver {
        Resolver::with(Arc::new(run_git), Arc::new(gotime::now))
    }

    /// A resolver over an injected runner and clock (tests).
    pub fn with(run: RunFn, now: NowFn) -> Resolver {
        Resolver {
            run,
            now,
            ttl: CACHE_TTL,
            timeout: GIT_TIMEOUT,
            by_dir: Mutex::new(HashMap::new()),
            by_root: Mutex::new(HashMap::new()),
        }
    }

    /// Sets the per-run git timeout (tests).
    pub fn set_timeout(&mut self, d: Duration) {
        self.timeout = d;
    }

    /// The repo_key for `dir` (any directory inside a repo works).
    pub fn key(&self, dir: &str) -> String {
        if dir.is_empty() {
            return String::new();
        }
        // Cache by absolute path: a relative dir means something else once
        // the process cwd moves.
        let Ok(dir) = gocompat::filepath::abs(dir) else {
            return String::new();
        };
        if let Some(k) = self.cached(&self.by_dir, &dir) {
            return k;
        }
        let root = match self.git(&["-C", &dir, "rev-parse", "--show-toplevel"]) {
            Ok(r) if !r.is_empty() => r,
            Err(GitErr::Timeout) => return String::new(),
            _ => {
                self.store(&self.by_dir, &dir, "");
                return String::new();
            }
        };
        let root = gocompat::filepath::clean(&root);
        if let Some(k) = self.cached(&self.by_root, &root) {
            self.store(&self.by_dir, &dir, &k);
            return k;
        }
        let Ok(k) = self.resolve_root(&root) else {
            return String::new();
        };
        self.store(&self.by_root, &root, &k);
        self.store(&self.by_dir, &dir, &k);
        k
    }

    /// The key for a toplevel: the normalized origin, else the smallest root
    /// commit, else "". Only a timeout is an error.
    fn resolve_root(&self, root: &str) -> Result<String, ()> {
        match self.git(&["-C", root, "remote", "get-url", "origin"]) {
            Err(GitErr::Timeout) => return Err(()),
            Ok(url) => {
                let n = normalize_remote(&url);
                if !n.is_empty() {
                    return Ok(format!("remote:{n}"));
                }
            }
            Err(GitErr::Failed) => {}
        }
        match self.git(&["-C", root, "rev-list", "--max-parents=0", "HEAD"]) {
            Err(GitErr::Timeout) => Err(()),
            Err(GitErr::Failed) => Ok(String::new()),
            Ok(out) => {
                let sha = smallest_sha(&out);
                Ok(if sha.is_empty() {
                    String::new()
                } else {
                    format!("root:{sha}")
                })
            }
        }
    }

    fn git(&self, args: &[&str]) -> Result<String, GitErr> {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        match (self.run)(self.timeout, &args) {
            Ok(s) => Ok(s),
            Err(ExecError::Timeout) => Err(GitErr::Timeout),
            Err(_) => Err(GitErr::Failed),
        }
    }

    fn cached(&self, m: &Mutex<HashMap<String, Entry>>, k: &str) -> Option<String> {
        let mut m = m.lock().unwrap_or_else(|p| p.into_inner());
        let e = m.get(k)?.clone();
        if !gotime::before(&(self.now)(), &e.expires) {
            m.remove(k);
            return None;
        }
        Some(e.key)
    }

    fn store(&self, m: &Mutex<HashMap<String, Entry>>, k: &str, key: &str) {
        let expires = gotime::add(&(self.now)(), self.ttl);
        m.lock().unwrap_or_else(|p| p.into_inner()).insert(
            k.to_string(),
            Entry {
                key: key.to_string(),
                expires,
            },
        );
    }
}

/// The lexicographically smallest line, lowercased, so a repo with several
/// root commits keys the same everywhere.
pub fn smallest_sha(out: &str) -> String {
    let mut best = String::new();
    for line in out.split('\n') {
        let s = gocompat::text::to_lower_str(crate::jsonx::trim_space(line));
        if s.is_empty() {
            continue;
        }
        if best.is_empty() || s < best {
            best = s;
        }
    }
    best
}
