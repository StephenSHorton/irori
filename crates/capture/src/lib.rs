//! `irori-capture`: the capture service, a port of I:internal/capture with
//! its `transcript` and `gitlink` packages, plus I:internal/repo and
//! I:internal/gitsha (capture.md).
//!
//! It tails Claude Code transcripts (`<ClaudeDir>/projects/**.jsonl`) into
//! log rows (turns, edits, summaries, commit commands, PR ledger rows),
//! watches each known worktree's `logs/HEAD`, imports commits and links
//! them to edits. Its state is a fold over the event log, rebuilt at
//! [`Capture::new`]; every new fact is appended first, then folded.
//! Transcript offsets live in `<DataDir>/capture-cursors.json`.
//!
//! What this crate leaves to its callers (the core wires them):
//! - [`Scrubber`] (I:internal/scrub) and [`Blobs`] (I:internal/blob): the
//!   secret scrubber and the patch/summary blob store are other crates;
//! - [`Roster`]: presence's live cards;
//! - the other areas' search sources: the core registers coord's and the
//!   inbox's [`search::Source`]s with [`Capture::add_source`] after
//!   [`Capture::new`], and builds the [`OverviewInput`] lines from them.
//!
//! Capture owns the FTS5 search index (in memory, rebuilt from the fold at
//! `new`), [`Capture::search`] and [`Capture::show`] over its own kinds and
//! the registered sources, [`Capture::overview`], [`Capture::why`],
//! [`Capture::record_decision`] and [`Capture::decisions`] (capture.md §1,
//! §2.7, §2.8); [`Capture::subscribe`] reports every change. Show, why and
//! record_decision run their git commands with a fresh token, each run
//! bounded by [`GIT_TIMEOUT`].
//!
//! The API is synchronous, like the store's. [`Capture::run`] blocks until
//! its token is canceled: the core runs it on its own thread.

pub mod cursors;
pub mod gitexec;
pub mod gitlink;
pub mod gitsha;
pub mod gopath;
pub mod gotime;
mod jsonx;
pub mod repo;
pub mod rows;
mod service;
pub mod transcript;

mod cmddir;
mod find;
mod handle;
mod index;
mod noise;
mod overview;
mod repos;
mod roots;
mod show;
mod tailer;
mod views;
mod watch;
mod why;

use std::sync::Arc;
use std::time::Duration;

use gocompat::time::GoTime;

pub use cmddir::command_dir;
pub use find::{HIDDEN_MESSAGE, OWN_KINDS, PULL_PAGE, hide_messages};
pub use handle::{MAX_COMMAND, clip_runes};
pub use noise::{Dropped, edit_path_key, is_artifact};
pub use overview::{
    AgentLine, Line, OVERVIEW_AGENTS, OVERVIEW_LINE_MAX, OVERVIEW_LINES, Overview, OverviewInput,
};
pub use repos::MAX_REFLOG_TRIES;
pub use roots::{norm_root, rel_to};
pub use rows::*;
pub use service::{Capture, Change, ChangeKind, Status};
pub use views::{CommitView, EditQuery, Entry, EntryKind, Timeline, TimelineQuery, TurnView};
pub use why::{DecisionInput, DecisionView, Error, MAX_WHY, WhyQuery, WhyResult};

/// Option defaults (Go's zero values).
pub const DEFAULT_SCAN_INTERVAL: Duration = Duration::from_secs(3);
pub const DEFAULT_REPO_INTERVAL: Duration = Duration::from_secs(5);
pub const DEFAULT_SAFETY_INTERVAL: Duration = Duration::from_secs(60);
pub const DEFAULT_QUIET_AFTER: Duration = Duration::from_secs(120);
pub const DEFAULT_BACKFILL_WINDOW: Duration = Duration::from_secs(72 * 3600);
pub const DEFAULT_BACKFILL_MAX_BYTES: i64 = 64 << 20;
/// One repo pass's budget (Go `repoBudget`).
pub const DEFAULT_REPO_BUDGET: Duration = Duration::from_millis(1500);

/// An uncommitted edit older than this is no longer a link candidate.
pub const POOL_AGE: Duration = Duration::from_secs(30 * 24 * 3600);
/// Commit commands older than this can't be a new commit's committer.
pub const CMD_AGE: Duration = Duration::from_secs(24 * 3600);
/// Commits seen this recently are linked again when matching edits arrive.
pub const RELINK_AGE: Duration = Duration::from_secs(3600);
/// An edit may be stamped this far after the (whole-second) commit time.
pub const COMMIT_SLACK: Duration = Duration::from_secs(2);
/// One git run's limit.
pub const GIT_TIMEOUT: Duration = Duration::from_secs(15);

/// The secret scrubber capture applies before anything is logged
/// (I:internal/scrub: `Scrubber.String` and the package's `SkipFile`).
pub trait Scrubber: Send + Sync {
    /// The text with secrets replaced by markers (line count kept).
    fn string(&self, s: &str) -> String;
    /// Whether a file's patch must never be stored (`.env`, keys, ...).
    fn skip_file(&self, path: &str) -> bool;
}

/// The patch and summary blob store (I:internal/blob `Store`).
pub trait Blobs: Send + Sync {
    /// Stores `data` and returns its id (sha256 hex of the stored bytes).
    fn put(&self, data: &[u8]) -> Result<String, String>;
    /// A stored blob's bytes.
    fn get(&self, id: &str) -> Result<Vec<u8>, String>;
}

/// The roster fields capture reads from a presence card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentCard {
    /// `<deviceId>/<sessionId>`.
    pub key: String,
    pub session_id: String,
    pub cwd: String,
    /// "busy", "idle", "gone", ...; "gone" cards are ignored.
    pub status: String,
    pub name: String,
    pub title: String,
    pub repo_key: String,
}

/// Presence as capture reads it: live cards give the sessions to tail
/// first and the roots to watch.
pub trait Roster: Send + Sync {
    fn agents(&self) -> Vec<AgentCard>;
}

/// The clock.
pub type NowFn = Arc<dyn Fn() -> GoTime + Send + Sync>;
/// Maps a worktree root to its repo_key ([`repo::Resolver::key`]).
pub type RepoKeyFn = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// What [`Capture::new`] needs. [`Options::new`] fills Go's defaults.
#[derive(Clone)]
pub struct Options {
    /// `~/.claude`; transcripts are under `<claude_dir>/projects`.
    pub claude_dir: String,
    pub data_dir: String,
    pub device_id: String,
    pub now: NowFn,
    pub git: Arc<dyn gitlink::Git>,
    pub repo_key: RepoKeyFn,
    pub scrub: Arc<dyn Scrubber>,
    pub blobs: Arc<dyn Blobs>,
    /// The user's home, for `cd ~` in commit commands (Go
    /// `os.UserHomeDir`); "" leaves `~` alone.
    pub home: String,
    pub scan_interval: Duration,
    pub repo_interval: Duration,
    pub safety_interval: Duration,
    pub quiet_after: Duration,
    pub backfill_window: Duration,
    pub backfill_max_bytes: i64,
    pub repo_budget: Duration,
}

impl Options {
    /// Options with Go's defaults: the system clock, the git binary, a
    /// fresh [`repo::Resolver`], the home directory from the environment.
    pub fn new(
        claude_dir: &str,
        data_dir: &str,
        device_id: &str,
        scrub: Arc<dyn Scrubber>,
        blobs: Arc<dyn Blobs>,
    ) -> Options {
        let resolver = Arc::new(repo::Resolver::new());
        Options {
            claude_dir: claude_dir.into(),
            data_dir: data_dir.into(),
            device_id: device_id.into(),
            now: Arc::new(gotime::now),
            git: Arc::new(gitlink::Exec),
            repo_key: Arc::new(move |d: &str| resolver.key(d)),
            scrub,
            blobs,
            home: user_home_dir(),
            scan_interval: DEFAULT_SCAN_INTERVAL,
            repo_interval: DEFAULT_REPO_INTERVAL,
            safety_interval: DEFAULT_SAFETY_INTERVAL,
            quiet_after: DEFAULT_QUIET_AFTER,
            backfill_window: DEFAULT_BACKFILL_WINDOW,
            backfill_max_bytes: DEFAULT_BACKFILL_MAX_BYTES,
            repo_budget: DEFAULT_REPO_BUDGET,
        }
    }
}

/// Go `os.UserHomeDir`: `%USERPROFILE%` on Windows, `$HOME` elsewhere.
pub fn user_home_dir() -> String {
    let v = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var(v).unwrap_or_default()
}

/// A scrubber that changes nothing and skips no file (tests only: the
/// core must wire the real one).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoScrub;

impl Scrubber for NoScrub {
    fn string(&self, s: &str) -> String {
        s.to_string()
    }
    fn skip_file(&self, _: &str) -> bool {
        false
    }
}

/// An in-memory blob store keyed by sha256 of the bytes (tests and tools;
/// no truncation, no gzip).
#[derive(Default)]
pub struct MemBlobs {
    m: std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>,
}

impl Blobs for MemBlobs {
    fn put(&self, data: &[u8]) -> Result<String, String> {
        use sha2::{Digest, Sha256};
        let id = hex::encode(Sha256::digest(data));
        self.m
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.clone(), data.to_vec());
        Ok(id)
    }
    fn get(&self, id: &str) -> Result<Vec<u8>, String> {
        self.m
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
            .ok_or_else(|| "blob: not found".to_string())
    }
}
