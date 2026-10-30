//! Statement checks, line numbers and the line-anchor hash
//! (I:internal/decision/lines.go).

use sha2::{Digest, Sha256};

use super::{Decision, Error, MAX_STATEMENT, MAX_WHY};
use crate::names::trim_space;

/// Trims statement and why, then checks them by rune count.
pub fn validate(d: &mut Decision) -> Result<(), Error> {
    d.statement = trim_space(&d.statement).to_string();
    d.why = trim_space(&d.why).to_string();
    if d.statement.is_empty() {
        return Err(Error::Empty);
    }
    if d.statement.chars().count() > MAX_STATEMENT || d.why.chars().count() > MAX_WHY {
        return Err(Error::TooLong);
    }
    Ok(())
}

/// Reads `"40"` or `"40-52"` (spaces allowed around `-`); `""` is (0, 0).
pub fn parse_lines(s: &str) -> Result<(i64, i64), Error> {
    let s = trim_space(s);
    if s.is_empty() {
        return Ok((0, 0));
    }
    let (a, b) = match s.split_once('-') {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    };
    let start = line_number(a).ok_or(Error::BadLines)?;
    let end = match b {
        Some(b) => line_number(b).ok_or(Error::BadLines)?,
        None => start,
    };
    if end < start {
        return Err(Error::BadLines);
    }
    Ok((start, end))
}

/// A 1-based line number made only of digits (`strconv.Atoi`, so it must
/// fit an int).
fn line_number(s: &str) -> Option<i64> {
    let s = trim_space(s);
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: i64 = s.parse().ok()?;
    (n >= 1).then_some(n)
}

/// Splits file content into normalised lines; a final newline does not
/// start another line, so empty content has none. Works on bytes: a Go
/// string may hold invalid UTF-8.
pub(crate) fn split_lines(data: &[u8]) -> Vec<Vec<u8>> {
    if data.is_empty() {
        return Vec::new();
    }
    let s = data.strip_suffix(b"\n").unwrap_or(data);
    s.split(|&b| b == b'\n').map(norm_line).collect()
}

/// Drops every `\r` and any trailing Unicode whitespace (invalid bytes
/// decode as U+FFFD, which is not space).
fn norm_line(l: &[u8]) -> Vec<u8> {
    let mut v: Vec<u8> = l.iter().copied().filter(|&b| b != b'\r').collect();
    while let Some(end) = last_rune_start(&v) {
        let (r, n) = gocompat::utf8::decode_rune(&v[end..]);
        if end + n != v.len() || !gocompat::text::is_space(r) {
            break;
        }
        v.truncate(end);
    }
    v
}

/// Where the last rune of `b` starts (Go `utf8.DecodeLastRune`'s start).
fn last_rune_start(b: &[u8]) -> Option<usize> {
    if b.is_empty() {
        return None;
    }
    let end = b.len();
    let lim = end.saturating_sub(4);
    let mut start = end - 1;
    while start > lim && (b[start] & 0xC0) == 0x80 {
        start -= 1;
    }
    let (_, n) = gocompat::utf8::decode_rune(&b[start..]);
    if start + n != end {
        // Not a full rune ending here: the last byte alone is the error rune.
        return Some(end - 1);
    }
    Some(start)
}

/// The line-anchor hash of already normalised lines.
pub(crate) fn hash_lines<L: AsRef<[u8]>>(lines: &[L]) -> String {
    let mut h = Sha256::new();
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            h.update(b"\n");
        }
        h.update(l.as_ref());
    }
    hex::encode(h.finalize())
}
