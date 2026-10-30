//! Port of Go `internal/presence` (mesh.md §3.4, surface.md §2.13): which
//! coding agents are running on this device.
//!
//! Source: Claude Code's `~/.claude/sessions/<pid>.json` files (one per
//! live session) plus a tail of each session's transcript
//! `~/.claude/projects/<slug>/<sessionId>.jsonl` for the last activity,
//! title and a short last-reply snippet.
//!
//! Rules:
//! - Never open, read, log or replicate the `<pid>.<hash>.key` files next
//!   to the session json; they are secrets. Only `*.json` whose stem is all
//!   digits counts ([`Options::open`] is the seam the tests use to prove it).
//! - Tolerate unknown and missing fields; the format is Claude Code's.
//! - A session file whose pid is dead (or reused, checked through procStart
//!   where the OS allows) is a gone agent, not a live one.
//! - Read only the transcript tail (last 256 KiB), never the whole file.
//!
//! The OS hooks Go keeps as unexported fields are [`Options`] fields here:
//! `watch` (file events; none means polling every interval, Go's path when
//! fswatch is unsupported), `watch_exit` and `live` (process exit and
//! liveness; Windows built in), `open` and `now`.

mod localtime;
mod os;
mod run;
mod session;
mod transcript;
mod watcher;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gocompat::time::GoTime;
use serde::{Deserialize, Serialize};

pub use localtime::{local, now_local};
pub use run::{FsEvent, FsSource, WatchError};
pub use transcript::slug;
pub use watcher::{Subscription, Watcher};

/// Name sources ([`AgentCard::name_source`]): a name someone chose.
pub const SOURCE_USER: &str = "user";
/// The friendly name Irori gave a session nobody named.
pub const SOURCE_IRORI: &str = "irori";

fn is_empty(s: &str) -> bool {
    s.is_empty()
}

/// What the roster shows for one session (Go `AgentCard`; JSON field order
/// and `omitempty` as Go marshals it).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentCard {
    /// "<deviceId>/<sessionId>".
    pub key: String,
    #[serde(rename = "deviceId")]
    pub device_id: String,
    /// "claude".
    pub agent: String,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub pid: i64,
    /// The session name, else derived.
    pub name: String,
    /// The newest custom-title / ai-title from the transcript.
    pub title: String,
    /// Where `name` came from: "user", "irori", or "" (an ai-title or the
    /// folder).
    #[serde(rename = "nameSource", skip_serializing_if = "is_empty")]
    pub name_source: String,
    /// The transcript's newest ai-title; stays on this device.
    #[serde(rename = "aiTitle", skip_serializing_if = "is_empty")]
    pub ai_title: String,
    pub cwd: String,
    /// As Claude Code reports: "busy", "idle", ...; "gone" when dead.
    pub status: String,
    /// "interactive", "bg", ...
    #[serde(rename = "sessionKind")]
    pub session_kind: String,
    /// "cli", "desktop", ...
    pub entrypoint: String,
    pub version: String,
    /// Never null in JSON.
    #[serde(rename = "peerFeatures", deserialize_with = "null_as_empty")]
    pub peer_features: Vec<String>,
    #[serde(rename = "startedAt")]
    pub started_at: GoTime,
    /// statusUpdatedAt/updatedAt from the session file.
    #[serde(rename = "updatedAt")]
    pub updated_at: GoTime,
    /// The newest transcript row timestamp.
    #[serde(rename = "lastActivity")]
    pub last_activity: GoTime,
    /// The last assistant prose, at most 240 runes, one line.
    #[serde(rename = "lastText")]
    pub last_text: String,
    /// Absolute path, "" if not found.
    pub transcript: String,
    /// Options::repo_key(cwd); "" outside git.
    #[serde(rename = "repoKey")]
    pub repo_key: String,
    /// The newest mode row's permissionMode; "" is unknown. Consumers test
    /// == "bypassPermissions", never a fixed list.
    #[serde(rename = "permissionMode")]
    pub permission_mode: String,
    /// Set on another device's card in the mesh roster: "connected",
    /// "recent" or "offline". "" on this device's own.
    #[serde(rename = "deviceState", skip_serializing_if = "is_empty")]
    pub device_state: String,
}

fn null_as_empty<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Ok(Option::<Vec<String>>::deserialize(d)?.unwrap_or_default())
}

/// What happened to a card.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    #[default]
    Added,
    Updated,
    Removed,
}

/// One roster update.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub kind: ChangeKind,
    pub card: AgentCard,
}

/// The friendly names Irori gives sessions (coord's names implement it).
pub trait Namer: Send + Sync {
    /// The name already given to session `id`, or "".
    fn assigned(&self, session_id: &str) -> String;
    /// Whether session `id` was ever given `name` (now or before a clash
    /// moved it).
    fn gave(&self, session_id: &str, name: &str) -> bool;
    /// Gives session `id` a name if it has none, and returns it.
    fn assign(&self, session_id: &str) -> String;
}

/// File events for `root` (recursive or not): Go `fswatch.Watch`.
pub type WatchFn = Arc<dyn Fn(&Path, bool) -> Result<FsSource, WatchError> + Send + Sync>;
/// Calls `on_exit` once `pid`'s process ends; returns the stop handle, or
/// None when the process can't be waited on (the safety rescan covers it).
/// Stop ends the wait without calling `on_exit` and waits for it.
pub type ExitFn =
    Arc<dyn Fn(i64, Box<dyn Fn() + Send + Sync>) -> Option<Box<dyn FnOnce() + Send>> + Send + Sync>;
/// Whether `pid` runs and is the process that wrote `proc_start`.
pub type LiveFn = Arc<dyn Fn(i64, u64) -> bool + Send + Sync>;
/// How every file's content is opened.
pub type OpenFn = Arc<dyn Fn(&Path) -> std::io::Result<std::fs::File> + Send + Sync>;
/// The clock.
pub type NowFn = Arc<dyn Fn() -> GoTime + Send + Sync>;
/// Maps a cwd to its repo key (repo.Resolver.Key).
pub type RepoKeyFn = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// Configures a [`Watcher`] (Go `Options`, plus the OS hooks Go keeps as
/// unexported fields).
#[derive(Clone, Default)]
pub struct Options {
    /// e.g. `~/.claude`.
    pub claude_dir: String,
    pub device_id: String,
    /// The rescan period while file events are unavailable; zero is 2 s.
    pub interval: Duration,
    /// The slow full rescan backing up file and exit events; zero is 30 s.
    pub safety_interval: Duration,
    /// The clock (removed cards' updatedAt); None is the local time now.
    pub now: Option<NowFn>,
    /// Maps a cwd to its repo key; None leaves repoKey empty.
    pub repo_key: Option<RepoKeyFn>,
    /// Friendly names; None keeps the old fallback (ai-title, else folder).
    pub names: Option<Arc<dyn Namer>>,
    /// File events (core passes sys::fswatch); None polls every interval.
    pub watch: Option<WatchFn>,
    /// Process-exit waits; None is this OS's (Windows only so far).
    pub watch_exit: Option<ExitFn>,
    /// Liveness; None is this OS's.
    pub live: Option<LiveFn>,
    /// How files are opened; None is `File::open`.
    pub open: Option<OpenFn>,
    /// Gathers a burst of file events into one scan; zero is 100 ms.
    pub settle: Duration,
    /// How much of a transcript's end is read; zero is 256 KiB.
    #[doc(hidden)]
    pub tail_bytes: u64,
    /// The one-time backward look for a permission mode; zero is 8 MiB.
    #[doc(hidden)]
    pub mode_scan_bytes: u64,
}

impl std::fmt::Debug for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Options")
            .field("claude_dir", &self.claude_dir)
            .field("device_id", &self.device_id)
            .field("interval", &self.interval)
            .field("safety_interval", &self.safety_interval)
            .finish_non_exhaustive()
    }
}

/// The pieces of Go's unexported API the ported tests and goldens reach.
#[doc(hidden)]
pub mod internal {
    pub use crate::os::{PROC_START_SLACK, default_live, default_watch_exit, process_start_ticks};
    pub use crate::session::{Session, is_session_file, parse_lstart, read_session};
    pub use crate::transcript::{Tail, one_line, read_tail, scan_mode};
    pub use crate::watcher::{changed, display_name};
}
