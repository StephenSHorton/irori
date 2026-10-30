//! The log rows capture writes and replays (capture.md §2.3), with Go's
//! JSON field names, order and omitempty rules, and Go-style decoding
//! (case-insensitive keys, last duplicate wins, a type error fails the row).
//!
//! Marshal them with `gocompat::json::marshal` (the store does) for Go's
//! bytes. A `GoTime` field is always written, even under Go's `omitempty`
//! (Go never omits a struct); an `Option<Vec<_>>` is a Go slice that may be
//! nil (`null`).

use gocompat::json::{self, Out, Schema};
use gocompat::time::GoTime;
use serde::Serialize;
use serde::ser::SerializeStruct;

use crate::gitlink::{Commit, FileChange, FileLink};
use crate::jsonx::{out_bool, out_int, out_items, out_str, out_strings, out_time};
use crate::transcript::{Summary, ToolCall, Turn};

/// Log kinds capture writes and replays. Payloads only ever gain fields.
pub const KIND_TURN: &str = "capture.turn";
pub const KIND_EDIT: &str = "capture.edit";
pub const KIND_SUMMARY: &str = "capture.summary";
pub const KIND_COMMIT_CMD: &str = "capture.commitcmd";
pub const KIND_PR: &str = "pr.ledger";
pub const KIND_REPO_SEEN: &str = "repo.seen";
pub const KIND_COMMIT_SEEN: &str = "commit.seen";
pub const KIND_COMMIT_LINK: &str = "commit.linked";
pub const KIND_DECISION: &str = "decision.recorded";

/// Every kind replay folds, in Go's order.
pub const REPLAY_KINDS: &[&str] = &[
    KIND_TURN,
    KIND_EDIT,
    KIND_SUMMARY,
    KIND_COMMIT_CMD,
    KIND_PR,
    KIND_REPO_SEEN,
    KIND_COMMIT_SEEN,
    KIND_COMMIT_LINK,
    KIND_DECISION,
];

fn is_false(b: &bool) -> bool {
    !*b
}

/// One capture.turn payload: a closed turn. Its `id` hides the turn's own
/// id in JSON, which travels as `turnId` (`turn.id` is filled from it on
/// replay). Prompt, reply and tool targets are scrubbed; edit paths are
/// repo-relative (forward slashes) when under `root`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TurnRow {
    /// `t-` + hash(key|turnId), or key|agentId|turnId for a subagent's turn.
    pub id: String,
    /// The agent key `<device>/<sessionId>`.
    pub key: String,
    pub repo_key: String,
    pub root: String,
    pub turn_id: String,
    pub turn: Turn,
}

impl Serialize for ToolCall {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("ToolCall", 6)?;
        st.serialize_field("id", &self.id)?;
        st.serialize_field("name", &self.name)?;
        if !self.target.is_empty() {
            st.serialize_field("target", &self.target)?;
        }
        if self.is_error {
            st.serialize_field("isError", &true)?;
        }
        if self.done {
            st.serialize_field("done", &true)?;
        }
        st.serialize_field("at", &self.at)?;
        st.end()
    }
}

impl Serialize for TurnRow {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let t = &self.turn;
        let mut st = s.serialize_struct("TurnRow", 20)?;
        st.serialize_field("id", &self.id)?;
        st.serialize_field("key", &self.key)?;
        if !self.repo_key.is_empty() {
            st.serialize_field("repoKey", &self.repo_key)?;
        }
        if !self.root.is_empty() {
            st.serialize_field("root", &self.root)?;
        }
        st.serialize_field("turnId", &self.turn_id)?;
        st.serialize_field("sessionId", &t.session_id)?;
        if !t.agent_id.is_empty() {
            st.serialize_field("agentId", &t.agent_id)?;
        }
        st.serialize_field("origin", &t.origin)?;
        if !t.origin_name.is_empty() {
            st.serialize_field("originName", &t.origin_name)?;
        }
        st.serialize_field("prompt", &t.prompt)?;
        if !t.reply.is_empty() {
            st.serialize_field("reply", &t.reply)?;
        }
        if !t.cwd.is_empty() {
            st.serialize_field("cwd", &t.cwd)?;
        }
        if !t.branch.is_empty() {
            st.serialize_field("branch", &t.branch)?;
        }
        st.serialize_field("startedAt", &t.started_at)?;
        st.serialize_field("endedAt", &t.ended_at)?;
        if t.interrupted {
            st.serialize_field("interrupted", &true)?;
        }
        if !t.tools.is_empty() {
            st.serialize_field("tools", &t.tools)?;
        }
        if t.tools_more != 0 {
            st.serialize_field("toolsMore", &t.tools_more)?;
        }
        if !t.edit_paths.is_empty() {
            st.serialize_field("editPaths", &t.edit_paths)?;
        }
        if !t.end_reason.is_empty() {
            st.serialize_field("endReason", &t.end_reason)?;
        }
        st.end()
    }
}

fn tool_call_schema() -> Schema {
    Schema::object(vec![
        ("id", Schema::String),
        ("name", Schema::String),
        ("target", Schema::String),
        ("isError", Schema::Bool),
        ("done", Schema::Bool),
        ("at", Schema::Time),
    ])
}

fn turn_row_schema() -> Schema {
    Schema::object(vec![
        ("id", Schema::String),
        ("key", Schema::String),
        ("repoKey", Schema::String),
        ("root", Schema::String),
        ("turnId", Schema::String),
        ("sessionId", Schema::String),
        ("agentId", Schema::String),
        ("origin", Schema::String),
        ("originName", Schema::String),
        ("prompt", Schema::String),
        ("reply", Schema::String),
        ("cwd", Schema::String),
        ("branch", Schema::String),
        ("startedAt", Schema::Time),
        ("endedAt", Schema::Time),
        ("interrupted", Schema::Bool),
        ("tools", Schema::Slice(Box::new(tool_call_schema()))),
        ("toolsMore", Schema::Int(64)),
        ("editPaths", Schema::Slice(Box::new(Schema::String))),
        ("endReason", Schema::String),
    ])
}

/// Go `json.Unmarshal` of a capture.turn payload, then `Turn.ID = TurnID`.
pub fn decode_turn_row(payload: &[u8]) -> Result<TurnRow, json::Error> {
    let o = json::unmarshal(payload, &turn_row_schema())?;
    let s = |n: &str| out_str(o.get(n));
    let tools = out_items(o.get("tools"))
        .iter()
        .map(|t| ToolCall {
            id: out_str(t.get("id")),
            name: out_str(t.get("name")),
            target: out_str(t.get("target")),
            is_error: out_bool(t.get("isError")),
            done: out_bool(t.get("done")),
            at: out_time(t.get("at")),
        })
        .collect();
    let turn_id = s("turnId");
    Ok(TurnRow {
        id: s("id"),
        key: s("key"),
        repo_key: s("repoKey"),
        root: s("root"),
        turn: Turn {
            id: turn_id.clone(),
            session_id: s("sessionId"),
            agent_id: s("agentId"),
            origin: s("origin"),
            origin_name: s("originName"),
            prompt: s("prompt"),
            reply: s("reply"),
            cwd: s("cwd"),
            branch: s("branch"),
            started_at: out_time(o.get("startedAt")),
            ended_at: out_time(o.get("endedAt")),
            interrupted: out_bool(o.get("interrupted")),
            tools,
            tools_more: out_int(o.get("toolsMore")),
            edit_paths: out_strings(o.get("editPaths")).unwrap_or_default(),
            end_reason: s("endReason"),
        },
        turn_id,
    })
}

/// One capture.edit payload.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct EditRow {
    /// `e-` + hash(key|toolUseId|path as the transcript wrote it).
    pub id: String,
    pub key: String,
    #[serde(rename = "agentId", skip_serializing_if = "String::is_empty")]
    pub agent_id: String,
    #[serde(rename = "turnId")]
    pub turn_id: String,
    #[serde(rename = "toolUseId")]
    pub tool_use_id: String,
    pub tool: String,
    #[serde(rename = "repoKey", skip_serializing_if = "String::is_empty")]
    pub repo_key: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub root: String,
    /// Repo-relative with forward slashes inside a repo, else the cleaned
    /// absolute native path.
    pub path: String,
    #[serde(skip_serializing_if = "is_false")]
    pub create: bool,
    #[serde(rename = "beforeBlob", skip_serializing_if = "String::is_empty")]
    pub before_blob: String,
    #[serde(rename = "afterBlob", skip_serializing_if = "String::is_empty")]
    pub after_blob: String,
    #[serde(rename = "afterBlobLF", skip_serializing_if = "String::is_empty")]
    pub after_blob_lf: String,
    /// The blob id of the scrubbed unified diff; "" when skipped.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub patch: String,
    pub adds: i64,
    pub dels: i64,
    pub fidelity: String,
    pub at: GoTime,
}

/// Go `json.Unmarshal` of a capture.edit payload.
pub fn decode_edit_row(payload: &[u8]) -> Result<EditRow, json::Error> {
    let o = json::unmarshal(
        payload,
        &Schema::object(vec![
            ("id", Schema::String),
            ("key", Schema::String),
            ("agentId", Schema::String),
            ("turnId", Schema::String),
            ("toolUseId", Schema::String),
            ("tool", Schema::String),
            ("repoKey", Schema::String),
            ("root", Schema::String),
            ("path", Schema::String),
            ("create", Schema::Bool),
            ("beforeBlob", Schema::String),
            ("afterBlob", Schema::String),
            ("afterBlobLF", Schema::String),
            ("patch", Schema::String),
            ("adds", Schema::Int(64)),
            ("dels", Schema::Int(64)),
            ("fidelity", Schema::String),
            ("at", Schema::Time),
        ]),
    )?;
    let s = |n: &str| out_str(o.get(n));
    Ok(EditRow {
        id: s("id"),
        key: s("key"),
        agent_id: s("agentId"),
        turn_id: s("turnId"),
        tool_use_id: s("toolUseId"),
        tool: s("tool"),
        repo_key: s("repoKey"),
        root: s("root"),
        path: s("path"),
        create: out_bool(o.get("create")),
        before_blob: s("beforeBlob"),
        after_blob: s("afterBlob"),
        after_blob_lf: s("afterBlobLF"),
        patch: s("patch"),
        adds: out_int(o.get("adds")),
        dels: out_int(o.get("dels")),
        fidelity: s("fidelity"),
        at: out_time(o.get("at")),
    })
}

/// One capture.summary payload.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SummaryRow {
    /// `s-` + hash(key|agentId|kind|rowUuid, else the scrubbed text).
    pub id: String,
    pub key: String,
    /// `text` scrubbed; `full` is never logged.
    pub summary: Summary,
    /// The blob id of the whole text when it was clipped.
    pub full_blob: String,
}

impl Serialize for SummaryRow {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let m = &self.summary;
        let mut st = s.serialize_struct("SummaryRow", 9)?;
        st.serialize_field("id", &self.id)?;
        st.serialize_field("key", &self.key)?;
        st.serialize_field("sessionId", &m.session_id)?;
        if !m.agent_id.is_empty() {
            st.serialize_field("agentId", &m.agent_id)?;
        }
        st.serialize_field("kind", &m.kind)?;
        st.serialize_field("text", &m.text)?;
        if !m.row_uuid.is_empty() {
            st.serialize_field("rowUuid", &m.row_uuid)?;
        }
        st.serialize_field("at", &m.at)?;
        if !self.full_blob.is_empty() {
            st.serialize_field("fullBlob", &self.full_blob)?;
        }
        st.end()
    }
}

/// Go `json.Unmarshal` of a capture.summary payload.
pub fn decode_summary_row(payload: &[u8]) -> Result<SummaryRow, json::Error> {
    let o = json::unmarshal(
        payload,
        &Schema::object(vec![
            ("id", Schema::String),
            ("key", Schema::String),
            ("sessionId", Schema::String),
            ("agentId", Schema::String),
            ("kind", Schema::String),
            ("text", Schema::String),
            ("rowUuid", Schema::String),
            ("at", Schema::Time),
            ("fullBlob", Schema::String),
        ]),
    )?;
    let s = |n: &str| out_str(o.get(n));
    Ok(SummaryRow {
        id: s("id"),
        key: s("key"),
        summary: Summary {
            session_id: s("sessionId"),
            agent_id: s("agentId"),
            kind: s("kind"),
            text: s("text"),
            full: String::new(),
            row_uuid: s("rowUuid"),
            at: out_time(o.get("at")),
        },
        full_blob: s("fullBlob"),
    })
}

/// One capture.commitcmd payload: a Bash call that may have committed.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct CommitCmdRow {
    pub key: String,
    #[serde(rename = "agentId", skip_serializing_if = "String::is_empty")]
    pub agent_id: String,
    #[serde(rename = "turnId")]
    pub turn_id: String,
    #[serde(rename = "toolUseId")]
    pub tool_use_id: String,
    pub cwd: String,
    /// Scrubbed, the first line, at most 300 runes.
    pub command: String,
    pub at: GoTime,
    /// The tool call's start (Go's omitempty still writes a zero time).
    #[serde(rename = "startedAt")]
    pub started_at: GoTime,
    /// Where git really ran (not cleaned); "" in old rows.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub dir: String,
}

/// Go `json.Unmarshal` of a capture.commitcmd payload.
pub fn decode_commit_cmd_row(payload: &[u8]) -> Result<CommitCmdRow, json::Error> {
    let o = json::unmarshal(
        payload,
        &Schema::object(vec![
            ("key", Schema::String),
            ("agentId", Schema::String),
            ("turnId", Schema::String),
            ("toolUseId", Schema::String),
            ("cwd", Schema::String),
            ("command", Schema::String),
            ("at", Schema::Time),
            ("startedAt", Schema::Time),
            ("dir", Schema::String),
        ]),
    )?;
    let s = |n: &str| out_str(o.get(n));
    Ok(CommitCmdRow {
        key: s("key"),
        agent_id: s("agentId"),
        turn_id: s("turnId"),
        tool_use_id: s("toolUseId"),
        cwd: s("cwd"),
        command: s("command"),
        at: out_time(o.get("at")),
        started_at: out_time(o.get("startedAt")),
        dir: s("dir"),
    })
}

/// One commit.seen payload.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CommitRow {
    pub repo_key: String,
    pub root: String,
    /// The reflog message up to ':' ("commit", "merge v0.2-talk", ...).
    pub op: String,
    pub branch: String,
    pub commit: Commit,
}

impl Serialize for CommitRow {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let c = &self.commit;
        let mut st = s.serialize_struct("CommitRow", 11)?;
        st.serialize_field("repoKey", &self.repo_key)?;
        st.serialize_field("root", &self.root)?;
        st.serialize_field("op", &self.op)?;
        if !self.branch.is_empty() {
            st.serialize_field("branch", &self.branch)?;
        }
        st.serialize_field("sha", &c.sha)?;
        if !c.parents.is_empty() {
            st.serialize_field("parents", &c.parents)?;
        }
        st.serialize_field("at", &c.at)?;
        st.serialize_field("author", &c.author)?;
        st.serialize_field("subject", &c.subject)?;
        st.serialize_field("files", &c.files)?;
        if c.files_omitted != 0 {
            st.serialize_field("filesOmitted", &c.files_omitted)?;
        }
        st.end()
    }
}

fn file_change_of(o: &Out) -> FileChange {
    FileChange {
        path: out_str(o.get("path")),
        blob: out_str(o.get("blob")),
        status: out_str(o.get("status")),
    }
}

/// Go `json.Unmarshal` of a commit.seen payload.
pub fn decode_commit_row(payload: &[u8]) -> Result<CommitRow, json::Error> {
    let fc = Schema::object(vec![
        ("path", Schema::String),
        ("blob", Schema::String),
        ("status", Schema::String),
    ]);
    let o = json::unmarshal(
        payload,
        &Schema::object(vec![
            ("repoKey", Schema::String),
            ("root", Schema::String),
            ("op", Schema::String),
            ("branch", Schema::String),
            ("sha", Schema::String),
            ("parents", Schema::Slice(Box::new(Schema::String))),
            ("at", Schema::Time),
            ("author", Schema::String),
            ("subject", Schema::String),
            ("files", Schema::Slice(Box::new(fc))),
            ("filesOmitted", Schema::Int(64)),
        ]),
    )?;
    let s = |n: &str| out_str(o.get(n));
    let files = match o.get("files") {
        Some(Out::Slice(Some(v))) => Some(v.iter().map(file_change_of).collect()),
        _ => None,
    };
    Ok(CommitRow {
        repo_key: s("repoKey"),
        root: s("root"),
        op: s("op"),
        branch: s("branch"),
        commit: Commit {
            sha: s("sha"),
            parents: out_strings(o.get("parents")).unwrap_or_default(),
            at: out_time(o.get("at")),
            author: s("author"),
            subject: s("subject"),
            files,
            files_omitted: out_int(o.get("filesOmitted")),
        },
    })
}

/// Who ran the command that made a commit.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Committer {
    pub key: String,
    #[serde(rename = "turnId")]
    pub turn_id: String,
}

/// One commit.linked payload; the newest per sha wins.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct CommitLink {
    #[serde(rename = "repoKey")]
    pub repo_key: String,
    pub sha: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub committer: Option<Committer>,
    /// `None` is Go's nil slice (`null`); links capture writes always have one.
    pub files: Option<Vec<FileLink>>,
    /// Agent keys behind linked edits.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<String>,
    /// Transcript turn ids, in `sessions` order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub turns: Vec<String>,
    pub at: GoTime,
}

impl CommitLink {
    /// The files, empty when nil.
    pub fn files(&self) -> &[FileLink] {
        self.files.as_deref().unwrap_or(&[])
    }
}

/// Go `json.Unmarshal` of a commit.linked payload.
pub fn decode_commit_link(payload: &[u8]) -> Result<CommitLink, json::Error> {
    let fl = Schema::object(vec![
        ("path", Schema::String),
        ("link", Schema::String),
        ("edits", Schema::Slice(Box::new(Schema::String))),
    ]);
    let o = json::unmarshal(
        payload,
        &Schema::object(vec![
            ("repoKey", Schema::String),
            ("sha", Schema::String),
            (
                "committer",
                Schema::Ptr(Box::new(Schema::object(vec![
                    ("key", Schema::String),
                    ("turnId", Schema::String),
                ]))),
            ),
            ("files", Schema::Slice(Box::new(fl))),
            ("sessions", Schema::Slice(Box::new(Schema::String))),
            ("turns", Schema::Slice(Box::new(Schema::String))),
            ("at", Schema::Time),
        ]),
    )?;
    let committer = match o.get("committer") {
        Some(Out::Ptr(Some(c))) => Some(Committer {
            key: out_str(c.get("key")),
            turn_id: out_str(c.get("turnId")),
        }),
        _ => None,
    };
    let files = match o.get("files") {
        Some(Out::Slice(Some(v))) => Some(
            v.iter()
                .map(|f| FileLink {
                    path: out_str(f.get("path")),
                    link: out_str(f.get("link")),
                    edits: out_strings(f.get("edits")).unwrap_or_default(),
                })
                .collect(),
        ),
        _ => None,
    };
    Ok(CommitLink {
        repo_key: out_str(o.get("repoKey")),
        sha: out_str(o.get("sha")),
        committer,
        files,
        sessions: out_strings(o.get("sessions")).unwrap_or_default(),
        turns: out_strings(o.get("turns")).unwrap_or_default(),
        at: out_time(o.get("at")),
    })
}

/// The repo.seen payload.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct RepoSeen {
    #[serde(rename = "repoKey")]
    pub repo_key: String,
    pub root: String,
    pub at: GoTime,
}

/// Go `json.Unmarshal` of a repo.seen payload.
pub fn decode_repo_seen(payload: &[u8]) -> Result<RepoSeen, json::Error> {
    let o = json::unmarshal(
        payload,
        &Schema::object(vec![
            ("repoKey", Schema::String),
            ("root", Schema::String),
            ("at", Schema::Time),
        ]),
    )?;
    Ok(RepoSeen {
        repo_key: out_str(o.get("repoKey")),
        root: out_str(o.get("root")),
        at: out_time(o.get("at")),
    })
}

/// The fields of a decision.recorded payload capture's fold reads; the
/// decision area owns the full type.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DecisionRef {
    pub id: String,
    pub by_key: String,
    pub repo_key: String,
}

/// Decodes the decision fields capture needs (`id`, `by.key`,
/// `anchor.repoKey`); other fields are not checked here.
pub fn decode_decision_ref(payload: &[u8]) -> Result<DecisionRef, json::Error> {
    let o = json::unmarshal(
        payload,
        &Schema::object(vec![
            ("id", Schema::String),
            ("by", Schema::object(vec![("key", Schema::String)])),
            (
                "anchor",
                Schema::Ptr(Box::new(Schema::object(vec![("repoKey", Schema::String)]))),
            ),
        ]),
    )?;
    Ok(DecisionRef {
        id: out_str(o.get("id")),
        by_key: out_str(o.get("by").and_then(|b| b.get("key"))),
        repo_key: out_str(o.get("anchor").and_then(|a| a.get("repoKey"))),
    })
}

/// `prefix` + the first 20 hex digits of sha256(parts joined by "|").
pub fn hash_id(prefix: &str, parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let sum = Sha256::digest(parts.join("|").as_bytes());
    format!("{prefix}{}", &hex::encode(sum)[..20])
}

/// A turn's row id; a subagent's turns hash its agent id in too.
pub fn turn_row_id(key: &str, agent_id: &str, turn_id: &str) -> String {
    if !agent_id.is_empty() {
        return hash_id("t-", &[key, agent_id, turn_id]);
    }
    hash_id("t-", &[key, turn_id])
}

/// An edit's row id, over the path exactly as the transcript wrote it.
pub fn edit_row_id(key: &str, tool_use_id: &str, path: &str) -> String {
    hash_id("e-", &[key, tool_use_id, path])
}

/// A summary's row id (`agent_id` may be "" and still takes its slot).
pub fn summary_row_id(key: &str, agent_id: &str, kind: &str, uuid_or_text: &str) -> String {
    hash_id("s-", &[key, agent_id, kind, uuid_or_text])
}

/// The in-memory identity of a commit command (never logged).
pub fn cmd_id(key: &str, tool_use_id: &str) -> String {
    format!("c|{key}|{tool_use_id}")
}

/// The in-memory identity of a ledger entry (never logged).
pub fn pr_id(e: &ledger::Entry) -> String {
    format!("p|{}|{}|{}", e.key, e.session, e.role.as_str())
}

/// The commit map key.
pub fn commit_key(repo_key: &str, sha: &str) -> String {
    format!("{repo_key}|{}", gocompat::text::to_lower_str(sha))
}

pub(crate) fn turn_key(key: &str, agent_id: &str, turn_id: &str) -> String {
    format!("{key}|{agent_id}|{turn_id}")
}

pub(crate) fn open_key(key: &str, agent_id: &str) -> String {
    format!("{key}|{agent_id}")
}

pub(crate) fn agent_actor(key: &str) -> String {
    format!("agent:{key}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_cmd_writes_zero_started_at() {
        let r = CommitCmdRow {
            key: "d/s".into(),
            at: GoTime::unix(1, 0),
            ..CommitCmdRow::default()
        };
        let b = gocompat::json::marshal(&r).unwrap();
        let s = String::from_utf8(b).unwrap();
        assert!(s.contains(r#""startedAt":"0001-01-01T00:00:00Z""#), "{s}");
        assert!(!s.contains("dir"), "{s}");
    }
}
