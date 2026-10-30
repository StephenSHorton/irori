//! A worktree's reflog, the commits it brings in, and how they link to
//! captured edits: a port of I:internal/capture/gitlink (capture.md §2.5,
//! §3 "Linking", "Committer", "IsCommitCommand"). Git is reached through
//! the [`Git`] trait so tests can use real temp repos or fakes.

mod classify;
mod command;
mod describe;
mod reflog;

use std::time::Duration;

use gocompat::time::GoTime;
use serde::Serialize;
use sys::cancel::Cancel;

pub use classify::{classify, committer, under_root};
pub use command::{is_commit_command, split_commands};
pub use describe::{describe, parse_raw_z};
pub use reflog::{git_dir, new_commits, parse_reflog, zero_sha};

pub(crate) use reflog::is_sha;

use crate::gitexec::{self, ExecError};

/// Commits taken from one reflog movement.
pub const MAX_WALK: usize = 200;
/// Files kept per commit.
pub const MAX_FILES: usize = 500;
/// How far a commit command's time may be from the commit's committer time.
pub const COMMITTER_WINDOW: Duration = Duration::from_secs(10);

/// Link classes.
pub const LINK_EXACT: &str = "exact";
pub const LINK_PARTIAL: &str = "partial";
pub const LINK_HEURISTIC: &str = "heuristic";
pub const LINK_UNATTRIBUTED: &str = "unattributed";

/// How a git run failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// The caller's token was canceled (Go returns `ctx.Err()` bare).
    Canceled,
    /// The run's timeout passed.
    Timeout,
    /// git exited non-zero: `code`, and Go's error text
    /// (`git <sub>: exit status N: <first stderr line>`).
    Exit { code: i32, msg: String },
    /// Anything else, with its text.
    Other(String),
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::Canceled => f.write_str("context canceled"),
            GitError::Timeout => f.write_str("context deadline exceeded"),
            GitError::Exit { msg, .. } | GitError::Other(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for GitError {}

/// Runs git in `dir` and returns stdout. The real one is [`Exec`].
pub trait Git: Send + Sync {
    fn run(
        &self,
        cancel: &Cancel,
        timeout: Duration,
        dir: &str,
        args: &[&str],
    ) -> Result<String, GitError>;
}

/// The git binary: `git --no-pager [-C dir] args...` with
/// GIT_TERMINAL_PROMPT=0, GIT_OPTIONAL_LOCKS=0, no pager, the
/// repo-selecting variables removed, and no console window.
#[derive(Debug, Clone, Copy, Default)]
pub struct Exec;

impl Git for Exec {
    fn run(
        &self,
        cancel: &Cancel,
        timeout: Duration,
        dir: &str,
        args: &[&str],
    ) -> Result<String, GitError> {
        let mut full: Vec<String> = vec!["--no-pager".into()];
        if !dir.is_empty() {
            full.push("-C".into());
            full.push(dir.into());
        }
        full.extend(args.iter().map(|s| s.to_string()));
        let name = args.first().copied().unwrap_or("");
        match gitexec::run(
            cancel,
            timeout,
            &full,
            &[
                ("GIT_TERMINAL_PROMPT", "0"),
                ("GIT_OPTIONAL_LOCKS", "0"),
                ("GIT_PAGER", "cat"),
                ("PAGER", "cat"),
            ],
        ) {
            Ok((out, _)) => Ok(String::from_utf8_lossy(&out).into_owned()),
            Err(ExecError::Canceled) => Err(GitError::Canceled),
            Err(ExecError::Timeout) => Err(GitError::Timeout),
            Err(ExecError::Exit { code, stderr }) => {
                let first = crate::jsonx::trim_space(&stderr);
                let first = first.split('\n').next().unwrap_or("");
                let first = crate::jsonx::trim_space(first);
                let msg = if first.is_empty() {
                    format!("git {name}: exit status {code}")
                } else {
                    format!("git {name}: exit status {code}: {first}")
                };
                Err(GitError::Exit { code, msg })
            }
            Err(ExecError::Spawn(e)) => Err(GitError::Other(format!("git {name}: {e}"))),
        }
    }
}

/// One logs/HEAD line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReflogEntry {
    pub old: String,
    pub new: String,
    pub name: String,
    pub email: String,
    pub at: GoTime,
    pub message: String,
}

/// One path a commit changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct FileChange {
    /// Repo-relative, forward slashes.
    pub path: String,
    /// The new blob id; zeros for a deletion.
    pub blob: String,
    /// git's letter: A, M, D, T.
    pub status: String,
}

/// One commit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Commit {
    pub sha: String,
    pub parents: Vec<String>,
    /// Committer time (`%ct`), in the local zone as Go's `time.Unix` gives.
    pub at: GoTime,
    pub author: String,
    /// At most 200 runes.
    pub subject: String,
    /// At most [`MAX_FILES`]; `None` is Go's nil (a link-only record).
    pub files: Option<Vec<FileChange>>,
    pub files_omitted: i64,
}

/// An uncommitted captured edit, as linking sees it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EditRef {
    pub id: String,
    /// The worktree root the edit's file is under.
    pub root: String,
    /// Repo-relative, forward slashes.
    pub path: String,
    pub after_blob: String,
    pub after_blob_lf: String,
    pub heuristic: bool,
    pub at: GoTime,
}

/// One committed file's link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct FileLink {
    pub path: String,
    pub link: String,
    /// EditRef ids, oldest first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<String>,
}

/// A Bash call that may have made a commit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommitCmd {
    pub key: String,
    pub turn_id: String,
    pub cwd: String,
    /// When the command finished.
    pub at: GoTime,
    /// When it started; zero means `at`.
    pub start: GoTime,
}
