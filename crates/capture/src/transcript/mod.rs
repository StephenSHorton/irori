//! Claude Code transcript lines into capture items, a port of
//! I:internal/capture/transcript (capture.md §2.1, §2.2). It is pure: the
//! service owns files, offsets, repos, blobs, scrubbing and the log. One
//! [`Parser`] per transcript file, fed complete lines in file order.
//!
//! Record types read (everything else is ignored, including unknown types
//! and lines that are not valid JSON): user, assistant, system (subtypes
//! turn_duration, away_summary, compact_boundary), ai-title, custom-title,
//! pr-link. Lines whose first bytes are `{"type":"api-request` are skipped
//! before decoding ([`skip_line`]). The record shapes are documented in
//! I:internal/capture/transcript/transcript.go:6-36.

mod parser;
mod result;
mod source;
mod target;

use gocompat::time::GoTime;

pub use parser::Parser;
pub use source::{Source, skip_line, source_for};
pub use target::{Hunk, clip, clip_bytes, target, unified_diff};

/// Caps (runes for text).
pub const MAX_PROMPT: usize = 2000;
pub const MAX_REPLY: usize = 1000;
pub const MAX_TARGET: usize = 160;
pub const MAX_TOOLS_SHOWN: usize = 200;
/// Bytes of Bash stdout kept on a [`BashRun`].
pub const MAX_STDOUT: usize = 4096;
/// Inline summary text; longer text is in [`Summary::full`].
pub const MAX_SUMMARY: usize = 2000;

/// Edit fidelity.
pub const EXACT: &str = "exact";
pub const HEURISTIC: &str = "heuristic";

/// One tool_use and, once it arrived, its result.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Short label, see [`target`].
    pub target: String,
    pub is_error: bool,
    /// Its result row was seen.
    pub done: bool,
    pub at: GoTime,
}

/// One prompt and everything until the next.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Turn {
    /// promptId, else the prompt row's uuid.
    pub id: String,
    pub session_id: String,
    pub agent_id: String,
    /// human, plugin, peer, task-notification, sdk, command, agent, unknown, ...
    pub origin: String,
    /// The plugin name when origin is plugin.
    pub origin_name: String,
    /// Excerpt, [`MAX_PROMPT`] runes.
    pub prompt: String,
    /// The last assistant text, [`MAX_REPLY`] runes.
    pub reply: String,
    pub cwd: String,
    pub branch: String,
    pub started_at: GoTime,
    /// Zero while open.
    pub ended_at: GoTime,
    pub interrupted: bool,
    /// The first [`MAX_TOOLS_SHOWN`].
    pub tools: Vec<ToolCall>,
    pub tools_more: i64,
    /// As the tools wrote them, in first-edit order, deduped. Empty is
    /// Go's nil.
    pub edit_paths: Vec<String>,
    /// "turn_duration", "next", "quiet", "eof".
    pub end_reason: String,
}

/// One file change by one tool call.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Edit {
    pub session_id: String,
    pub agent_id: String,
    pub turn_id: String,
    pub tool_use_id: String,
    pub row_uuid: String,
    /// Edit, Write, NotebookEdit, Bash, PowerShell.
    pub tool: String,
    /// As the tool reported it.
    pub path: String,
    pub cwd: String,
    pub create: bool,
    /// The whole file before and after, when known. Never logged: the
    /// service keeps only git blob ids.
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
    /// Unified diff, unscrubbed; "" if none.
    pub patch: String,
    pub adds: i64,
    pub dels: i64,
    pub fidelity: String,
    pub at: GoTime,
}

/// A summary-like row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Summary {
    pub session_id: String,
    pub agent_id: String,
    /// away_summary, compact_boundary, compact_summary, ai_title, custom_title.
    pub kind: String,
    /// [`MAX_SUMMARY`] runes.
    pub text: String,
    /// The whole text when longer than `text`, else "".
    pub full: String,
    pub row_uuid: String,
    /// Zero for title rows (they carry no timestamp).
    pub at: GoTime,
}

/// A pr-link row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PrLink {
    pub session_id: String,
    /// prRepository.
    pub repo: String,
    pub number: i64,
    pub url: String,
    pub at: GoTime,
}

/// A finished Bash or PowerShell call, for commit and PR detection.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BashRun {
    pub tool: String,
    pub session_id: String,
    pub agent_id: String,
    pub turn_id: String,
    pub tool_use_id: String,
    pub cwd: String,
    pub command: String,
    /// The first [`MAX_STDOUT`] bytes.
    pub stdout: String,
    /// The raw (compacted) gitOperation object; empty when none.
    pub git_operation: Vec<u8>,
    pub is_error: bool,
    /// The result row's timestamp.
    pub at: GoTime,
    /// The tool_use row's timestamp (zero when the call wasn't seen).
    pub started_at: GoTime,
}

/// One parser output.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Turn(Turn),
    Edit(Edit),
    Summary(Summary),
    PrLink(PrLink),
    Bash(BashRun),
}

// --- JSON with Go's tags (transcript.go), for tools and the goldens ---

use serde::Serialize;
use serde::ser::SerializeStruct;

impl Serialize for Turn {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("Turn", 16)?;
        st.serialize_field("id", &self.id)?;
        st.serialize_field("sessionId", &self.session_id)?;
        if !self.agent_id.is_empty() {
            st.serialize_field("agentId", &self.agent_id)?;
        }
        st.serialize_field("origin", &self.origin)?;
        if !self.origin_name.is_empty() {
            st.serialize_field("originName", &self.origin_name)?;
        }
        st.serialize_field("prompt", &self.prompt)?;
        if !self.reply.is_empty() {
            st.serialize_field("reply", &self.reply)?;
        }
        if !self.cwd.is_empty() {
            st.serialize_field("cwd", &self.cwd)?;
        }
        if !self.branch.is_empty() {
            st.serialize_field("branch", &self.branch)?;
        }
        st.serialize_field("startedAt", &self.started_at)?;
        st.serialize_field("endedAt", &self.ended_at)?;
        if self.interrupted {
            st.serialize_field("interrupted", &true)?;
        }
        if !self.tools.is_empty() {
            st.serialize_field("tools", &self.tools)?;
        }
        if self.tools_more != 0 {
            st.serialize_field("toolsMore", &self.tools_more)?;
        }
        if !self.edit_paths.is_empty() {
            st.serialize_field("editPaths", &self.edit_paths)?;
        }
        if !self.end_reason.is_empty() {
            st.serialize_field("endReason", &self.end_reason)?;
        }
        st.end()
    }
}

impl Serialize for Edit {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("Edit", 13)?;
        st.serialize_field("sessionId", &self.session_id)?;
        if !self.agent_id.is_empty() {
            st.serialize_field("agentId", &self.agent_id)?;
        }
        st.serialize_field("turnId", &self.turn_id)?;
        st.serialize_field("toolUseId", &self.tool_use_id)?;
        st.serialize_field("rowUuid", &self.row_uuid)?;
        st.serialize_field("tool", &self.tool)?;
        st.serialize_field("path", &self.path)?;
        if !self.cwd.is_empty() {
            st.serialize_field("cwd", &self.cwd)?;
        }
        if self.create {
            st.serialize_field("create", &true)?;
        }
        st.serialize_field("adds", &self.adds)?;
        st.serialize_field("dels", &self.dels)?;
        st.serialize_field("fidelity", &self.fidelity)?;
        st.serialize_field("at", &self.at)?;
        st.end()
    }
}

impl Serialize for Summary {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("Summary", 6)?;
        st.serialize_field("sessionId", &self.session_id)?;
        if !self.agent_id.is_empty() {
            st.serialize_field("agentId", &self.agent_id)?;
        }
        st.serialize_field("kind", &self.kind)?;
        st.serialize_field("text", &self.text)?;
        if !self.row_uuid.is_empty() {
            st.serialize_field("rowUuid", &self.row_uuid)?;
        }
        st.serialize_field("at", &self.at)?;
        st.end()
    }
}

impl Serialize for PrLink {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("PRLink", 5)?;
        st.serialize_field("sessionId", &self.session_id)?;
        st.serialize_field("repo", &self.repo)?;
        st.serialize_field("number", &self.number)?;
        st.serialize_field("url", &self.url)?;
        st.serialize_field("at", &self.at)?;
        st.end()
    }
}

impl Serialize for BashRun {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("BashRun", 12)?;
        st.serialize_field("tool", &self.tool)?;
        st.serialize_field("sessionId", &self.session_id)?;
        if !self.agent_id.is_empty() {
            st.serialize_field("agentId", &self.agent_id)?;
        }
        st.serialize_field("turnId", &self.turn_id)?;
        st.serialize_field("toolUseId", &self.tool_use_id)?;
        st.serialize_field("cwd", &self.cwd)?;
        st.serialize_field("command", &self.command)?;
        st.serialize_field("stdout", &self.stdout)?;
        if !self.git_operation.is_empty() {
            st.serialize_field(
                "gitOperation",
                &gocompat::json::RawJson(self.git_operation.clone()),
            )?;
        }
        if self.is_error {
            st.serialize_field("isError", &true)?;
        }
        st.serialize_field("at", &self.at)?;
        st.serialize_field("startedAt", &self.started_at)?;
        st.end()
    }
}
