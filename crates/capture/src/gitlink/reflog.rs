//! I:internal/capture/gitlink/reflog.go.

use std::time::Duration;

use gocompat::time::GoTime;
use sys::cancel::Cancel;

use super::{Git, GitError, MAX_WALK, ReflogEntry};
use crate::gopath;

/// The directory holding a worktree's own logs/HEAD: `<root>/.git` when it
/// is a directory, else the `gitdir: <path>` in the `.git` file (relative
/// paths resolve against root; a UTF-8 BOM is skipped).
pub fn git_dir(root: &str) -> std::io::Result<String> {
    let dot = gopath::join(&[root, ".git"]);
    let md = std::fs::metadata(&dot)?;
    if md.is_dir() {
        return Ok(dot);
    }
    let data = std::fs::read(&dot)?;
    let data = data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&data);
    let text = String::from_utf8_lossy(data);
    let line = text.split('\n').next().unwrap_or("");
    let line = crate::jsonx::trim_space(line);
    let bad = |m: &str| std::io::Error::other(format!("gitlink: {dot} {m}"));
    let Some(p) = line.strip_prefix("gitdir:") else {
        return Err(bad("has no gitdir line"));
    };
    let p = crate::jsonx::trim_space(p);
    if p.is_empty() {
        return Err(bad("has an empty gitdir"));
    }
    let mut p = gopath::from_slash(p);
    if !gopath::is_abs(&p) {
        p = gopath::join(&[root, &p]);
    }
    Ok(gopath::clean(&p))
}

/// Parses the complete lines of `data` (format
/// `<old> <new> <name> <<email>> <unix> <+hhmm>\t<message>`), skipping
/// malformed ones, and returns how many bytes it consumed (through the last
/// `\n`; a trailing partial line is left for next time).
pub fn parse_reflog(data: &[u8]) -> (Vec<ReflogEntry>, usize) {
    let Some(end) = data.iter().rposition(|&c| c == b'\n') else {
        return (Vec::new(), 0);
    };
    let text = String::from_utf8_lossy(&data[..end]);
    let out = text
        .split('\n')
        .filter_map(|l| parse_line(l.strip_suffix('\r').unwrap_or(l)))
        .collect();
    (out, end + 1)
}

/// `"<old> <new> <name> <<email>> <unix> <tz>\t<msg>"`. Git writes no tab
/// when the message is empty.
fn parse_line(line: &str) -> Option<ReflogEntry> {
    let (head, msg) = line.split_once('\t').unwrap_or((line, ""));
    let (old, rest) = head.split_once(' ')?;
    if !is_sha(old) {
        return None;
    }
    let (nw, rest) = rest.split_once(' ')?;
    if !is_sha(nw) || nw.len() != old.len() {
        return None;
    }
    let lt = rest.find('<')?;
    let gt = rest.rfind('>')?;
    if gt < lt {
        return None;
    }
    let name = crate::jsonx::trim_space(&rest[..lt]).to_string();
    let email = rest[lt + 1..gt].to_string();
    let fields: Vec<&str> = rest[gt + 1..].split_whitespace().collect();
    if fields.len() != 2 {
        return None;
    }
    let unix: i64 = fields[0].parse().ok()?;
    let off = parse_tz(fields[1])?;
    Some(ReflogEntry {
        old: old.to_ascii_lowercase(),
        new: nw.to_ascii_lowercase(),
        name,
        email,
        at: GoTime::unix(unix, 0).with_offset(off),
        message: msg.to_string(),
    })
}

/// git's `+hhmm` / `-hhmm` offset, in seconds east of UTC.
fn parse_tz(s: &str) -> Option<i32> {
    let b = s.as_bytes();
    if b.len() != 5 || (b[0] != b'+' && b[0] != b'-') {
        return None;
    }
    // Go strconv.Atoi on the two-byte fields: a sign is accepted there too.
    let hh: i32 = atoi(&s[1..3])?;
    let mm: i32 = atoi(&s[3..5])?;
    if mm > 59 {
        return None;
    }
    let off = hh * 3600 + mm * 60;
    Some(if b[0] == b'-' { -off } else { off })
}

/// Go `strconv.Atoi` for a short field.
fn atoi(s: &str) -> Option<i32> {
    let b = s.as_bytes();
    let digits = match b.first() {
        Some(b'+') | Some(b'-') => &b[1..],
        _ => b,
    };
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    s.parse().ok()
}

/// A full SHA-1 (40) or SHA-256 (64) hex object id.
pub(crate) fn is_sha(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|c| c.is_ascii_hexdigit())
}

/// An all-zero object id (any length).
pub fn zero_sha(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c == b'0')
}

/// The commits a reflog movement brought into HEAD that `seen` doesn't
/// hold, oldest first: `git rev-list --reverse --max-count=200 <old>..<new>`
/// (just `<new>` when old is zero or that fails). `skipped` counts commits
/// past the cap (`rev-list --count`). A movement whose new is already seen
/// returns nothing without running git.
pub fn new_commits(
    g: &dyn Git,
    cancel: &Cancel,
    timeout: Duration,
    root: &str,
    e: &ReflogEntry,
    seen: &dyn Fn(&str) -> bool,
) -> Result<(Vec<String>, usize), GitError> {
    if !is_sha(&e.new) || (!e.old.is_empty() && !zero_sha(&e.old) && !is_sha(&e.old)) {
        return Err(GitError::Other(
            "gitlink: reflog entry has a malformed object id".into(),
        ));
    }
    if zero_sha(&e.new) || seen(&e.new) {
        return Ok((Vec::new(), 0));
    }
    let mut ranges = vec![e.new.clone()];
    if !e.old.is_empty() && !zero_sha(&e.old) {
        ranges = vec![format!("{}..{}", e.old, e.new), e.new.clone()];
    }
    let max = format!("--max-count={MAX_WALK}");
    let mut last: Result<String, GitError> = Err(GitError::Other(String::new()));
    let mut rng = String::new();
    for r in &ranges {
        rng = r.clone();
        last = g.run(
            cancel,
            timeout,
            root,
            &["rev-list", "--reverse", &max, r, "--"],
        );
        if last.is_ok() || cancel.is_canceled() {
            break;
        }
    }
    let out = last?;
    let listed: Vec<&str> = out.split_whitespace().collect();
    let mut skipped = 0;
    if listed.len() >= MAX_WALK {
        let n = g.run(cancel, timeout, root, &["rev-list", "--count", &rng, "--"])?;
        let t = crate::jsonx::trim_space(&n);
        let total: usize = t.parse().map_err(|_| {
            GitError::Other(format!(
                "gitlink: rev-list --count: {}",
                gocompat::quote::quote(t)
            ))
        })?;
        if total > listed.len() {
            skipped = total - listed.len();
        }
    }
    let shas = listed
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| !seen(s))
        .collect();
    Ok((shas, skipped))
}
