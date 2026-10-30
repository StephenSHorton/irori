//! I:internal/capture/transcript/target.go: tool-call labels, clipping and
//! unified diffs.

use gocompat::json::Value;
use gocompat::text::is_space;

use super::MAX_TARGET;
use super::source::same_prefix;
use crate::jsonx::lenient_strings;

/// The input fields Target and the edit builders read.
#[derive(Debug, Clone, Default)]
pub(crate) struct ToolInput {
    pub file_path: String,
    pub notebook_path: String,
    pub command: String,
    pub pattern: String,
    pub url: String,
    pub query: String,
    pub description: String,
}

/// Go `decodeInput`: a partial or odd input still labels what it can.
pub(crate) fn decode_input(input: Option<&[u8]>) -> ToolInput {
    let Some(raw) = input.filter(|r| !r.is_empty()) else {
        return ToolInput::default();
    };
    let Ok(v) = gocompat::json::parse(raw) else {
        return ToolInput::default();
    };
    decode_input_value(&v)
}

pub(crate) fn decode_input_value(v: &Value) -> ToolInput {
    let f = lenient_strings(
        v,
        &[
            "file_path",
            "notebook_path",
            "command",
            "pattern",
            "url",
            "query",
            "description",
        ],
    );
    let mut it = f.into_iter();
    let mut n = || it.next().unwrap_or_default();
    ToolInput {
        file_path: n(),
        notebook_path: n(),
        command: n(),
        pattern: n(),
        url: n(),
        query: n(),
        description: n(),
    }
}

/// Labels a tool call from its name, raw input and the row's cwd: the path
/// for Read/Edit/Write/NotebookEdit (cwd-relative with forward slashes when
/// under cwd), the first line of a Bash or PowerShell command, the pattern
/// for Grep/Glob, the url for WebFetch, the query for WebSearch, the
/// description for Agent/Task, else "". Clipped to [`MAX_TARGET`] runes.
pub fn target(name: &str, input: Option<&[u8]>, cwd: &str) -> String {
    let i = decode_input(input);
    let s = match name {
        "Read" | "Edit" | "Write" => rel_path(&i.file_path, cwd),
        "NotebookEdit" => rel_path(&i.notebook_path, cwd),
        "Bash" | "PowerShell" => first_line(&i.command),
        "Grep" | "Glob" => i.pattern,
        "WebFetch" => i.url,
        "WebSearch" => i.query,
        "Agent" | "Task" => i.description,
        _ => String::new(),
    };
    clip(&s, MAX_TARGET)
}

/// `p` relative to `cwd` with forward slashes when under it, else `p`.
pub(crate) fn rel_path(p: &str, cwd: &str) -> String {
    if p.is_empty() || cwd.is_empty() {
        return p.to_string();
    }
    let fp = p.replace('\\', "/");
    let fc_full = cwd.replace('\\', "/");
    let fc = fc_full.trim_end_matches('/');
    let (pb, cb) = (fp.as_bytes(), fc.as_bytes());
    if fc.is_empty() || pb.len() <= cb.len() + 1 || pb[cb.len()] != b'/' {
        return p.to_string();
    }
    if !same_prefix(&pb[..cb.len()], cb) {
        return p.to_string();
    }
    fp[cb.len()..].trim_start_matches('/').to_string()
}

/// The first non-blank line of `s`, trimmed.
pub(crate) fn first_line(s: &str) -> String {
    let s = crate::jsonx::trim_space(s);
    let s = match s.find('\n') {
        Some(i) => &s[..i],
        None => s,
    };
    crate::jsonx::trim_space(s).to_string()
}

/// At most `n` runes of `s`, ending in "…" when it cut. The cut backs up to
/// whitespace within the last n/2 runes, so a word straddling the cap (a
/// token the scrubber would only match whole) is dropped, not halved.
pub fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    if n == 0 {
        return String::new();
    }
    let r: Vec<char> = s.chars().collect();
    let mut cut = n - 1;
    if !is_space(r[cut]) {
        let lo = cut as isize - (n / 2) as isize;
        let mut i = cut as isize - 1;
        while i >= lo && i > 0 {
            if is_space(r[i as usize]) {
                cut = i as usize;
                break;
            }
            i -= 1;
        }
    }
    let mut out: String = r[..cut].iter().collect();
    out.push('…');
    out
}

/// At most `n` bytes of `s` without splitting a rune.
pub fn clip_bytes(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut i = n;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    s[..i].to_string()
}

/// One structuredPatch entry.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hunk {
    pub old_start: i64,
    pub old_lines: i64,
    pub new_start: i64,
    pub new_lines: i64,
    pub lines: Vec<String>,
}

/// Renders a structuredPatch for `p` as a unified diff
/// (`--- a/<p>\n+++ b/<p>\n@@ -s,l +s,l @@\n` + lines), with the added and
/// removed line counts.
pub fn unified_diff(p: &str, hunks: &[Hunk]) -> (String, i64, i64) {
    if hunks.is_empty() {
        return (String::new(), 0, 0);
    }
    let p = p.replace('\\', "/");
    let mut b = String::new();
    let (mut adds, mut dels) = (0, 0);
    b.push_str(&format!("--- a/{p}\n+++ b/{p}\n"));
    for h in hunks {
        b.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            h.old_start, h.old_lines, h.new_start, h.new_lines
        ));
        for l in &h.lines {
            if l.starts_with('+') {
                adds += 1;
            } else if l.starts_with('-') {
                dels += 1;
            }
            b.push_str(l);
            b.push('\n');
        }
    }
    (b, adds, dels)
}
