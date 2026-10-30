//! I:internal/capture/roots.go: directories to worktree roots, cached.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gocompat::time::GoTime;
use sys::cancel::Cancel;

use crate::gitlink::{Git, GitError};
use crate::gopath;
use crate::{GIT_TIMEOUT, NowFn, gotime};

/// How long "not in a repo" is cached for a directory.
pub const NEGATIVE_TTL: Duration = Duration::from_secs(60);

#[derive(Clone)]
struct Entry {
    root: String,
    /// `None`: forever.
    expires: Option<GoTime>,
}

/// Resolves directories to worktree roots with `git rev-parse
/// --show-toplevel`, cached per directory: positives forever, "not a repo"
/// for a minute, a timeout not at all.
pub(crate) struct RootCache {
    git: Arc<dyn Git>,
    now: NowFn,
    dir: Mutex<HashMap<String, Entry>>,
}

impl RootCache {
    pub fn new(git: Arc<dyn Git>, now: NowFn) -> RootCache {
        RootCache {
            git,
            now,
            dir: Mutex::new(HashMap::new()),
        }
    }

    /// The worktree root holding `dir` ("" when none). `dir` need not
    /// exist: its nearest existing ancestor is asked.
    pub fn for_dir(&self, cancel: &Cancel, dir: &str) -> String {
        if dir.is_empty() || !gopath::is_abs(dir) {
            return String::new();
        }
        let mut dir = gopath::clean(dir);
        loop {
            if std::fs::metadata(&dir).is_ok_and(|m| m.is_dir()) {
                break;
            }
            let up = gopath::dir(&dir);
            if up == dir {
                return String::new();
            }
            dir = up;
        }
        let k = norm_root(&dir);
        let now = (self.now)();
        if let Some(e) = self
            .dir
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&k)
            .cloned()
            && e.expires.is_none_or(|x| gotime::before(&now, &x))
        {
            return e.root;
        }
        let out = self
            .git
            .run(cancel, GIT_TIMEOUT, &dir, &["rev-parse", "--show-toplevel"]);
        let root = match &out {
            Ok(s) => crate::jsonx::trim_space(s).to_string(),
            Err(_) => String::new(),
        };
        if out.is_err() || root.is_empty() {
            if cancel.is_canceled() || matches!(out, Err(GitError::Timeout | GitError::Canceled)) {
                return String::new(); // don't cache a timeout
            }
            self.dir.lock().unwrap_or_else(|p| p.into_inner()).insert(
                k,
                Entry {
                    root: String::new(),
                    expires: Some(gotime::add(&now, NEGATIVE_TTL)),
                },
            );
            return String::new();
        }
        let root = gopath::clean(&gopath::from_slash(&root));
        self.dir.lock().unwrap_or_else(|p| p.into_inner()).insert(
            k,
            Entry {
                root: root.clone(),
                expires: None,
            },
        );
        root
    }

    /// The worktree root holding a file.
    pub fn for_file(&self, cancel: &Cancel, path: &str) -> String {
        if path.is_empty() || !gopath::is_abs(path) {
            return String::new();
        }
        self.for_dir(cancel, &gopath::dir(&gopath::clean(path)))
    }
}

/// A comparable form of a directory: cleaned, OS separators, lowercase on
/// Windows. Persisted as the cursor file's `reflogs` keys.
pub fn norm_root(p: &str) -> String {
    let p = gopath::clean(&gopath::from_slash(p));
    if cfg!(windows) {
        return gocompat::text::to_lower_str(&p);
    }
    p
}

/// `path` relative to `root` with forward slashes, when inside it.
pub fn rel_to(root: &str, path: &str) -> Option<String> {
    if root.is_empty() || path.is_empty() {
        return None;
    }
    let rel = gopath::rel(
        &gopath::clean(root),
        &gopath::clean(&gopath::from_slash(path)),
    )
    .ok()?;
    let sep = gopath::SEPARATOR;
    if rel == "." || rel == ".." || rel.starts_with(&format!("..{sep}")) || gopath::is_abs(&rel) {
        return None;
    }
    Some(gopath::to_slash(&rel))
}

/// An existing directory.
pub(crate) fn dir_exists(p: &str) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.is_dir())
}
