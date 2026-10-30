//! Small helpers the Board shares (Go's cut, cmpStr, deeper, callers ...).

use gocompat::quote::quote;
use gocompat::time::GoTime;

use super::error::{Error, invalid};
use crate::names::trim_space;

/// Irori itself as a caller.
pub const SWEEPER: &str = "irori";

/// Runes in an objective or done criteria in a notice.
pub const MAX_NOTICE_FIELD: usize = 600;

pub fn runes(s: &str) -> usize {
    s.chars().count()
}

/// `s` cut to `n` runes, marking the cut with `…`.
pub fn cut(s: &str, n: usize) -> String {
    if runes(s) <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn cmp_str(s: &str, def: &str) -> String {
    if s.is_empty() { def } else { s }.to_string()
}

pub fn cmp_or(t: GoTime, def: GoTime) -> GoTime {
    if t.is_zero() { def } else { t }
}

pub fn contains(xs: &[String], x: &str) -> bool {
    xs.iter().any(|v| v == x)
}

pub fn remove_str(xs: &[String], x: &str) -> Vec<String> {
    xs.iter().filter(|v| *v != x).cloned().collect()
}

/// `"\nshow task:<id> for the card, its comments and links."`
pub fn deeper(id: &str) -> String {
    format!("\nshow task:{id} for the card, its comments and links.")
}

/// `strings.Join(strings.Fields(s), " ")`.
pub fn collapse_space(s: &str) -> String {
    s.split(gocompat::text::is_space)
        .filter(|f| !f.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Checks a text's rune count.
pub fn check_text(what: &str, s: &str, min: usize, max: usize) -> Result<(), Error> {
    let n = runes(s);
    if n < min {
        return Err(invalid(format!("{what} is empty")));
    }
    if n > max {
        return Err(invalid(format!("{what} longer than {max} characters")));
    }
    Ok(())
}

/// Trims the caller; empty is Invalid.
pub fn caller(by: &str) -> Result<String, Error> {
    let by = trim_space(by);
    if by.is_empty() {
        return Err(invalid("no caller"));
    }
    Ok(by.to_string())
}

/// Trims further arguments; an empty one is Invalid.
pub fn args(more: &[&str]) -> Result<Vec<String>, Error> {
    more.iter()
        .map(|m| {
            let m = trim_space(m);
            if m.is_empty() {
                Err(invalid("empty argument"))
            } else {
                Ok(m.to_string())
            }
        })
        .collect()
}

/// A key that becomes a holder: one line, at most MAX_KEY runes, not Irori.
pub fn check_key(k: &str) -> Result<(), Error> {
    if k == SWEEPER {
        return Err(invalid(format!("{} is Irori itself", quote(k))));
    }
    if runes(k) > super::MAX_KEY || k.chars().any(gocompat::text::is_control) {
        return Err(invalid(format!(
            "a key longer than {} characters or with control characters",
            super::MAX_KEY
        )));
    }
    Ok(())
}

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A ULID-style id: 48 bits of milliseconds then 80 random bits, 26
/// Crockford base32 characters.
pub fn new_id(t: &GoTime) -> String {
    let mut b = [0u8; 16];
    let ms = t.unix_milli_value() as u64;
    for (i, byte) in b.iter_mut().take(6).enumerate() {
        *byte = (ms >> (8 * (5 - i))) as u8;
    }
    let _ = getrandom::fill(&mut b[6..]);
    let v = u128::from_be_bytes(b);
    let mut out = [0u8; 26];
    for (i, c) in out.iter_mut().enumerate() {
        let shift = 5 * (25 - i);
        *c = CROCKFORD[((v >> shift) & 31) as usize];
    }
    String::from_utf8(out.to_vec()).expect("ascii")
}

/// `strings.ContainsAny(s, "\r\n")`.
pub fn multiline(s: &str) -> bool {
    s.contains(['\r', '\n'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_id_sortable() {
        // glob_test.go TestNewIDSortable.
        let t = GoTime::parse_rfc3339("2026-10-26T12:00:00Z").unwrap();
        let a = new_id(&t);
        let b = new_id(&GoTime::unix(t.secs, 1));
        let c = new_id(&GoTime::unix(t.secs, 2_000_000));
        assert_eq!(a.len(), 26);
        assert_ne!(a, b);
        assert!(a[..10] < c[..10], "{a} {c}");
        assert!(a.bytes().all(|x| CROCKFORD.contains(&x)));
        // The first character carries 3 bits.
        assert!(a.as_bytes()[0] <= b'7');
    }

    #[test]
    fn test_cut() {
        assert_eq!(cut("héllo", 5), "héllo");
        assert_eq!(cut("héllo", 4), "hél…");
    }
}
