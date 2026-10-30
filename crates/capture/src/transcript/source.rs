//! I:internal/capture/transcript/source.go: which file a path is, and which
//! lines never need decoding.

use gocompat::text::{equal_fold_bytes, to_lower_str};

use crate::gopath::slash_clean;

/// The file's session id (from `projects/<slug>/<sessionId>.jsonl`) and,
/// for subagent files
/// (`projects/<slug>/<sessionId>/subagents/**/agent-<agentId>.jsonl`), the
/// agent id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Source {
    pub session_id: String,
    /// "" for the main conversation.
    pub agent_id: String,
}

/// Derives the Source from a path under a projects dir; `None` for paths
/// of neither shape. Ids must match `[A-Za-z0-9_-]+`. Both slash styles are
/// accepted; the prefix compares case-insensitively when it looks like a
/// Windows path.
pub fn source_for(projects_dir: &str, p: &str) -> Option<Source> {
    let base_c = clean_slash(projects_dir);
    let base = base_c.trim_end_matches('/');
    let full = clean_slash(p);
    let (bb, fb) = (base.as_bytes(), full.as_bytes());
    if base.is_empty() || fb.len() <= bb.len() + 1 || fb[bb.len()] != b'/' {
        return None;
    }
    if !same_prefix(&fb[..bb.len()], bb) {
        return None;
    }
    let parts: Vec<&str> = full[bb.len() + 1..].split('/').collect();
    if parts
        .iter()
        .any(|s| s.is_empty() || *s == "." || *s == "..")
    {
        return None;
    }
    if parts.len() == 2 {
        let sid = parts[1].strip_suffix(".jsonl")?;
        if !valid_id(sid) {
            return None;
        }
        return Some(Source {
            session_id: sid.into(),
            agent_id: String::new(),
        });
    }
    if parts.len() >= 4 && parts[2] == "subagents" {
        let sid = parts[1];
        let name = parts[parts.len() - 1];
        let id = name.strip_prefix("agent-")?.strip_suffix(".jsonl")?;
        if !valid_id(sid) || !valid_id(id) {
            return None;
        }
        return Some(Source {
            session_id: sid.into(),
            agent_id: id.into(),
        });
    }
    None
}

/// `[A-Za-z0-9_-]+`.
fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

/// Backslashes to slashes, then `path.Clean`; "" stays "".
pub(crate) fn clean_slash(p: &str) -> String {
    if p.is_empty() {
        return String::new();
    }
    slash_clean(&p.replace('\\', "/"))
}

/// Two slash paths, compared ignoring case for Windows-looking ones.
pub(crate) fn same_prefix(a: &[u8], b: &[u8]) -> bool {
    if windowsish(a) || windowsish(b) {
        return equal_fold_bytes(a, b);
    }
    a == b
}

/// A drive-letter path ("C:/..." or "C:\...") or a UNC path.
pub(crate) fn windowsish(p: &[u8]) -> bool {
    if p.len() >= 2 && p[1] == b':' && p[0].is_ascii_alphabetic() {
        return true;
    }
    p.starts_with(b"\\\\") || p.starts_with(b"//")
}

/// A path for comparison: forward slashes, lower case for Windows-looking
/// paths.
pub(crate) fn path_key(p: &str) -> String {
    let k = clean_slash(p);
    if windowsish(k.as_bytes()) {
        return to_lower_str(&k);
    }
    k
}

const API_REQUEST: &[u8] = br#"{"type":"api-request"#;

/// Blank lines and api-request rows (by byte prefix, allowing leading
/// spaces and tabs).
pub fn skip_line(line: &[u8]) -> bool {
    let start = line
        .iter()
        .position(|&c| c != b' ' && c != b'\t')
        .unwrap_or(line.len());
    let t = &line[start..];
    if crate::jsonx::trim_space_bytes(t).is_empty() {
        return true;
    }
    t.starts_with(API_REQUEST)
}
