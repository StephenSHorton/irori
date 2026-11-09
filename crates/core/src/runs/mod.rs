//! Live tasks on cards (card #1249, docs/design/live-tasks.md): the runs
//! an agent starts, as this device knows them.
//!
//! A run is a background shell (`bash`, PowerShell included), a background
//! Agent or Workflow, or a foreground shell call (`fg`). The Irori mod
//! posts each start and end to `POST /v1/runs` ([`RunReport`]); the
//! tasks-folder scan ([`scan`]) finds the runs of sessions with no mod;
//! the sampler ([`sampler`]) meters each run's process tree; the fold
//! ([`fold`]) keeps other devices' summaries. All of them keep their rows
//! in one [`Runs`] registry on the Core's services, saved as `runs.json`
//! in the data dir.
//!
//! Runs are keyed by agent key and Claude's task id ([`RunId`]), never by
//! card: the window joins them to the cards their agents hold on read.
//!
//! The module sits in `crates/core/src/runs/` but is declared from
//! `service.rs` (re-exported as `crate::runs`), so its files see the
//! Core's services as `service.rs`'s own submodules do. The split is so
//! the slices of card #1249 never touch the same file: this file and
//! `registry.rs` are the registry and the API's way in, `scan.rs`,
//! `sampler.rs` and `fold.rs` each start their own loop from [`start`].

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{Core, Services};
use crate::events::CoreEvent;

pub(crate) mod fold;
mod registry;
pub(crate) mod sampler;
pub(crate) mod scan;

pub use registry::{
    ENDED_KEEP_MS, FG_KEEP, FG_STALE_MS, FILE, MAX_CMD, MAX_OUTPUT, MAX_PER_KEY, MAX_RUNNING,
    MAX_TASK_ID, Runs,
};

/// One run: an agent's key and the task id Claude gave it (`toolUseId`
/// for a foreground call, which has none).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunId {
    pub key: String,
    pub task_id: String,
}

impl RunId {
    pub fn new(key: impl Into<String>, task_id: impl Into<String>) -> RunId {
        RunId {
            key: key.into(),
            task_id: task_id.into(),
        }
    }
}

/// What kind of run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunKind {
    /// A background shell run: Bash or PowerShell (`b*` task files).
    Bash,
    /// A background Agent (`a*`).
    Agent,
    /// A background Workflow (`w*`).
    Workflow,
    /// A foreground shell call: time and CPU, no stream, no tab, no chip.
    Fg,
}

impl RunKind {
    /// `bash`, `agent`, `workflow` or `fg`.
    pub fn parse(s: &str) -> Option<RunKind> {
        Some(match s {
            "bash" => RunKind::Bash,
            "agent" => RunKind::Agent,
            "workflow" => RunKind::Workflow,
            "fg" => RunKind::Fg,
            _ => return None,
        })
    }
}

/// One run as this device keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveRun {
    pub key: String,
    pub task_id: String,
    pub kind: RunKind,
    /// The command (a workflow's name, an agent's description), scrubbed
    /// and at most [`MAX_CMD`] chars.
    #[serde(default)]
    pub cmd: String,
    /// When it started, Unix ms.
    pub started_ms: i64,
    /// The task's output file, as the mod reported it (never rebuilt, and
    /// never a kill target); `None` when no report named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_path: Option<PathBuf>,
    /// The tool call that started a background run, when the report named
    /// it: that call's foreground reports are then not a separate row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    /// When it ended, Unix ms; `None` while it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_ms: Option<i64>,
    /// Its exit code, when its end said one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    /// The person pressed Stop on it: whatever end follows reads Stopped.
    #[serde(default)]
    pub stopped_by_person: bool,
    /// Its process tree as last sampled: `(pid, start time)`.
    #[serde(default)]
    pub members: Vec<(u32, u64)>,
    #[serde(default)]
    pub cpu_cores: f32,
    #[serde(default)]
    pub mem_bytes: u64,
    #[serde(default)]
    pub peak_cores: f32,
    #[serde(default)]
    pub peak_mem: u64,
    /// When it was last sampled, Unix ms; 0 for never.
    #[serde(default)]
    pub sampled_ms: i64,
}

impl LiveRun {
    pub fn id(&self) -> RunId {
        RunId::new(&self.key, &self.task_id)
    }

    /// Whether it is still going.
    pub fn running(&self) -> bool {
        self.ended_ms.is_none()
    }

    /// A row with no end and no samples yet.
    pub fn new(key: &str, task_id: &str, kind: RunKind, started_ms: i64) -> LiveRun {
        LiveRun {
            key: key.into(),
            task_id: task_id.into(),
            kind,
            cmd: String::new(),
            started_ms,
            output_path: None,
            tool_use_id: None,
            ended_ms: None,
            exit: None,
            stopped_by_person: false,
            members: Vec::new(),
            cpu_cores: 0.0,
            mem_bytes: 0,
            peak_cores: 0.0,
            peak_mem: 0,
            sampled_ms: 0,
        }
    }
}

/// One report from the mod (`POST /v1/runs`, design §2.1). Every field is
/// optional on the wire; [`Runs::record`] says which an event needs. The
/// agent is the caller's attach key, never the body.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    /// `start`, `end`, `fg-start` or `fg`.
    #[serde(default)]
    pub event: String,
    /// `bash`, `agent`, `workflow` or `fg`.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub tool_use_id: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// A workflow's name: its label when there is no command.
    #[serde(default)]
    pub workflow_name: Option<String>,
    #[serde(default)]
    pub output_file: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// The mod's clock, Unix ms; 0 or absent for "now".
    #[serde(default)]
    pub started_at: i64,
    #[serde(default)]
    pub ended_at: i64,
    /// `completed`, `failed` or `killed` on an `end`.
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub interrupted: bool,
    #[serde(default)]
    pub is_error: bool,
}

/// Starts the runs' loops: the tasks-folder scan, the sampler and the mesh
/// fold, each in its own file. Called once from `Core::start` after the
/// loops are up; the threads go through `Core::spawn`, so `Core::close`
/// waits for them.
pub(crate) fn start(core: &Core) {
    scan::start(core);
    sampler::start(core);
    fold::start(core);
}

impl Services {
    /// Tells the window these agents' runs changed ([`CoreEvent::Runs`]).
    /// Every slice publishes through here.
    pub(crate) fn publish_runs(&self, keys: Vec<String>) {
        if !keys.is_empty() {
            self.hub.publish(CoreEvent::Runs { keys });
        }
    }
}
