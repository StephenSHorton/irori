//! Decision records with code anchors that are re-checked when read
//! (I:internal/decision, coord.md §2.5, §3.14, §3.17). Pure apart from
//! reading files under a worktree root; the capture service folds
//! `decision.recorded` rows into a [`View`] and appends new ones.

mod anchor;
mod lines;
mod view;

#[cfg(test)]
mod tests;

use gocompat::time::GoTime;

pub use anchor::{check, make_anchor};
pub use lines::{parse_lines, validate};
pub use view::{Filter, Listed, View};

use crate::gojson::omit;

pub const MAX_STATEMENT: usize = 500;
pub const MAX_WHY: usize = 2000;
pub const CONTEXT_LINES: usize = 2;

/// The decision errors, with Go's texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("decision: statement is empty")]
    Empty,
    #[error("decision: statement or why is too long")]
    TooLong,
    #[error("decision: lines must look like \"40\" or \"40-52\"")]
    BadLines,
    #[error("decision: path must be inside the repo")]
    BadPath,
    #[error("decision: no such file")]
    NoFile,
    #[error("decision: lines are past the end of the file")]
    Range,
    #[error("decision: only its author or the person can supersede a decision")]
    Supersede,
    #[error("decision: no such decision")]
    NoDecision,
}

crate::go_struct! {
    /// Who recorded a decision (inbox's sender shape).
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Party {
        pub key: String = "key",
        pub name: String = "name" "omit",
        /// `claude`, `grok`, `user`.
        pub kind: String = "kind" "omit",
    }
}

crate::go_struct! {
    /// Ties a decision to code.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Anchor {
        pub repo_key: String = "repoKey",
        /// The worktree root it was recorded in.
        pub root: String = "root",
        /// Repo-relative, forward slashes.
        pub path: String = "path",
        /// HEAD when recorded.
        pub sha: String = "sha" "omit",
        /// 1-based, inclusive; 0 = the whole file.
        pub start: i64 = "start" "omit",
        pub end: i64 = "end" "omit",
        pub hash: String = "hash",
        /// Up to CONTEXT_LINES lines before, then after.
        pub context: Vec<String> = "context" "omit",
    }
}

crate::go_struct! {
    /// What a decision was marked from in the GUI.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Source {
        pub message_id: String = "messageId" "omit",
        pub turn_id: String = "turnId" "omit",
    }
}

crate::go_struct! {
    /// One `decision.recorded` payload.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Decision {
        pub id: String = "id",
        pub by: Party = "by",
        pub statement: String = "statement",
        pub why: String = "why" "omit",
        pub anchor: Option<Anchor> = "anchor" "omit",
        pub supersedes: Vec<String> = "supersedes" "omit",
        pub source: Option<Source> = "source" "omit",
        /// `user` from the GUI, else `untrusted`.
        pub trust: String = "trust",
        pub at: GoTime = "at",
    }
}

/// The result of re-checking an anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum AnchorState {
    /// The lines are where they were.
    #[serde(rename = "ok")]
    Ok,
    /// The same lines are elsewhere in the file.
    #[serde(rename = "moved")]
    Moved,
    /// The file is there, the lines are not.
    #[serde(rename = "changed")]
    Changed,
    /// The file is missing.
    #[serde(rename = "gone")]
    Gone,
    /// No worktree to check against.
    #[serde(rename = "unknown")]
    Unknown,
}

/// An anchor re-check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Status {
    pub state: AnchorState,
    /// The current range when Ok or Moved.
    #[serde(skip_serializing_if = "omit")]
    pub start: i64,
    #[serde(skip_serializing_if = "omit")]
    pub end: i64,
}

impl Status {
    pub(crate) fn of(state: AnchorState) -> Status {
        Status {
            state,
            start: 0,
            end: 0,
        }
    }
}

/// Git object ids for file contents (I:internal/gitsha).
pub mod gitsha {
    use sha1::{Digest, Sha1};

    /// The git blob id (SHA-1, lowercase hex) of `data`.
    pub fn blob(data: &[u8]) -> String {
        let mut h = Sha1::new();
        h.update(format!("blob {}\0", data.len()).as_bytes());
        h.update(data);
        hex::encode(h.finalize())
    }

    /// `data` with every CRLF turned into LF.
    pub fn lf(data: &[u8]) -> std::borrow::Cow<'_, [u8]> {
        if !data.windows(2).any(|w| w == b"\r\n") {
            return std::borrow::Cow::Borrowed(data);
        }
        let mut out = Vec::with_capacity(data.len());
        let mut i = 0;
        while i < data.len() {
            if data[i] == b'\r' && data.get(i + 1) == Some(&b'\n') {
                i += 1;
                continue;
            }
            out.push(data[i]);
            i += 1;
        }
        std::borrow::Cow::Owned(out)
    }

    /// `blob(data)` and, when it differs, `blob(lf(data))` (`""` when data
    /// has no CRLF).
    pub fn pair(data: &[u8]) -> (String, String) {
        let raw = blob(data);
        let l = lf(data);
        let lf = if l.len() != data.len() {
            blob(&l)
        } else {
            String::new()
        };
        (raw, lf)
    }
}
