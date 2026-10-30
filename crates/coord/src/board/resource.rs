//! Lease resources and path-glob overlap (I:internal/coord/resource.go,
//! glob.go; coord.md §3.4).
//!
//! ```text
//! path:<repoKey>:<glob>   repoKey "remote:<host>/<owner>/<name>" or "root:<sha>";
//!                         the repoKey ends at the last ':'
//! port:<n>                1..65535, no leading zeros
//! <name>                  ^[a-z0-9][a-z0-9._-]*(/[a-z0-9._-]+)*$
//! ```

use std::collections::HashMap;
use std::sync::OnceLock;

use gocompat::quote::quote;

use super::error::{Error, ErrorKind};
use crate::names::trim_space;

pub const RESOURCE_PATH: &str = "path";
pub const RESOURCE_PORT: &str = "port";
pub const RESOURCE_NAMED: &str = "named";

/// A parsed lease resource.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resource {
    pub kind: &'static str,
    /// Path only.
    pub repo_key: String,
    /// Path only, normalized.
    pub glob: String,
    /// The canonical string (also for path and port).
    pub name: String,
}

impl Resource {
    pub(crate) fn named(name: &str) -> Resource {
        Resource {
            kind: RESOURCE_NAMED,
            name: name.into(),
            ..Resource::default()
        }
    }
}

fn named_re() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"^[a-z0-9][a-z0-9._-]*(/[a-z0-9._-]+)*$").unwrap())
}

fn bad(detail: String) -> Error {
    Error::new(ErrorKind::BadResource, detail)
}

/// Parses and normalizes a lease resource; errors are BadResource.
pub fn parse_resource(s: &str) -> Result<Resource, Error> {
    let s = trim_space(s);
    if let Some(rest) = s.strip_prefix("path:") {
        let Some((kind, after)) = rest.split_once(':') else {
            return Err(bad(format!(
                "repoKey must start remote: or root: ({})",
                quote(s)
            )));
        };
        if kind != "remote" && kind != "root" {
            return Err(bad(format!(
                "repoKey must start remote: or root: ({})",
                quote(s)
            )));
        }
        let Some(i) = after.rfind(':') else {
            return Err(bad(format!(
                "missing :<glob> after the repoKey ({})",
                quote(s)
            )));
        };
        let (value, glob) = (&after[..i], &after[i + 1..]);
        let repo_key = format!("{kind}:{value}");
        check_repo_key(&repo_key)?;
        let g = normalize_glob(glob)?;
        return Ok(Resource {
            kind: RESOURCE_PATH,
            name: format!("path:{repo_key}:{g}"),
            repo_key,
            glob: g,
        });
    }
    if let Some(n) = s.strip_prefix("port:") {
        let ok = go_atoi(n).is_some_and(|p| (1..=65535).contains(&p) && p.to_string() == n);
        if !ok {
            return Err(bad(format!("port must be 1-65535 ({})", quote(s))));
        }
        return Ok(Resource {
            kind: RESOURCE_PORT,
            name: s.into(),
            ..Resource::default()
        });
    }
    if named_re().is_match(s) {
        return Ok(Resource::named(s));
    }
    Err(Error::raw(
        ErrorKind::BadResource,
        format!("{} ({})", ErrorKind::BadResource.text(), quote(s)),
    ))
}

/// `strconv.Atoi`: an optional sign then decimal digits, fitting an int.
pub(crate) fn go_atoi(s: &str) -> Option<i64> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<i64>().ok()
}

/// The canonical form of a lease resource.
pub fn normalize_resource(s: &str) -> Result<String, Error> {
    parse_resource(s).map(|r| r.name)
}

/// `root:<lowercase hex>` or `remote:<value>` with no backslash, no
/// control character and no space at either end.
pub fn check_repo_key(k: &str) -> Result<(), Error> {
    let bad_key = match k.split_once(':') {
        None => true,
        Some((_, "")) => true,
        Some(("root", v)) => !v.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
        Some(("remote", v)) => {
            trim_space(v) != v || v.contains('\\') || v.chars().any(gocompat::text::is_control)
        }
        Some(_) => true,
    };
    if bad_key {
        return Err(bad(format!("bad repoKey {}", quote(k))));
    }
    Ok(())
}

/// Validates a repo-relative POSIX glob and returns its normalized form.
pub fn normalize_glob(orig: &str) -> Result<String, Error> {
    let fail = |why: &str| bad(format!("glob {} {why}", quote(orig)));
    let mut g = trim_space(orig);
    if g.is_empty() {
        return Err(fail("is empty"));
    }
    if g.contains('\\') {
        return Err(fail("has a backslash; use /"));
    }
    if g.contains(':') {
        return Err(fail("has a ':'; use ? to match one"));
    }
    if g.chars().any(gocompat::text::is_control) {
        return Err(fail("has a control character"));
    }
    while let Some(rest) = g.strip_prefix("./") {
        g = rest;
    }
    if g.is_empty() {
        return Err(fail("is empty"));
    }
    if g.starts_with('/') {
        return Err(fail("is absolute; use a path relative to the repo root"));
    }
    let mut g = g.to_string();
    if g.ends_with('/') {
        g.push_str("**");
    }
    for seg in g.split('/') {
        match seg {
            "" => return Err(fail("has an empty segment")),
            "." | ".." => return Err(fail("has a . or .. segment")),
            _ => {}
        }
        if !gocompat::pathmatch::valid(seg) {
            return Err(fail("is malformed"));
        }
    }
    Ok(g)
}

/// Whether two parsed resources contend.
pub(crate) fn conflicts(a: &Resource, b: &Resource) -> bool {
    if a.kind == RESOURCE_PATH && b.kind == RESOURCE_PATH {
        return a.repo_key == b.repo_key && overlaps_norm(&a.glob, &b.glob);
    }
    a.name == b.name
}

/// Whether two path globs can match a common path. An unparseable glob
/// overlaps everything.
pub fn overlaps(a: &str, b: &str) -> bool {
    let (Ok(na), Ok(nb)) = (normalize_glob(a), normalize_glob(b)) else {
        return true;
    };
    overlaps_norm(&na, &nb)
}

#[derive(Debug, Clone, Default)]
struct Token {
    star: bool,
    any: bool,
    lit: u32,
    class: bool,
    neg: bool,
    ranges: Vec<(u32, u32)>,
}

pub(crate) fn overlaps_norm(a: &str, b: &str) -> bool {
    let sa: Vec<&str> = a.split('/').collect();
    let sb: Vec<&str> = b.split('/').collect();
    let (Some(ta), Some(tb)) = (tokenize_all(&sa), tokenize_all(&sb)) else {
        return true;
    };
    let mut memo: HashMap<(usize, usize), bool> = HashMap::new();
    seg_ov(0, 0, &sa, &sb, &ta, &tb, &mut memo)
}

fn seg_ov(
    i: usize,
    j: usize,
    sa: &[&str],
    sb: &[&str],
    ta: &[Vec<Token>],
    tb: &[Vec<Token>],
    memo: &mut HashMap<(usize, usize), bool>,
) -> bool {
    if let Some(v) = memo.get(&(i, j)) {
        return *v;
    }
    let r = if i == sa.len() && j == sb.len() {
        true
    } else if i < sa.len() && sa[i] == "**" {
        seg_ov(i + 1, j, sa, sb, ta, tb, memo)
            || j < sb.len() && seg_ov(i, j + 1, sa, sb, ta, tb, memo)
    } else if j < sb.len() && sb[j] == "**" {
        seg_ov(i, j + 1, sa, sb, ta, tb, memo)
            || i < sa.len() && seg_ov(i + 1, j, sa, sb, ta, tb, memo)
    } else if i == sa.len() || j == sb.len() {
        false
    } else {
        seg_overlap(&ta[i], &tb[j]) && seg_ov(i + 1, j + 1, sa, sb, ta, tb, memo)
    };
    memo.insert((i, j), r);
    r
}

fn tokenize_all(segs: &[&str]) -> Option<Vec<Vec<Token>>> {
    segs.iter()
        .map(|s| {
            if *s == "**" {
                Some(Vec::new())
            } else {
                tokenize(s)
            }
        })
        .collect()
}

fn decode(b: &[u8]) -> (u32, usize) {
    let (r, n) = gocompat::utf8::decode_rune(b);
    (r as u32, n)
}

fn tokenize(s: &str) -> Option<Vec<Token>> {
    let s = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'*' => {
                out.push(Token {
                    star: true,
                    ..Token::default()
                });
                i += 1;
            }
            b'?' => {
                out.push(Token {
                    any: true,
                    ..Token::default()
                });
                i += 1;
            }
            b'[' => {
                i += 1;
                let mut t = Token {
                    class: true,
                    ..Token::default()
                };
                if i < s.len() && s[i] == b'^' {
                    t.neg = true;
                    i += 1;
                }
                let mut closed = false;
                while i < s.len() {
                    if s[i] == b']' && !t.ranges.is_empty() {
                        closed = true;
                        i += 1;
                        break;
                    }
                    let (lo, n) = decode(&s[i..]);
                    i += n;
                    let mut hi = lo;
                    if i + 1 < s.len() && s[i] == b'-' && s[i + 1] != b']' {
                        i += 1;
                        let (h, n) = decode(&s[i..]);
                        hi = h;
                        i += n;
                    }
                    if hi < lo {
                        return None;
                    }
                    t.ranges.push((lo, hi));
                }
                if !closed {
                    return None;
                }
                out.push(t);
            }
            _ => {
                let (r, n) = decode(&s[i..]);
                out.push(Token {
                    lit: r,
                    ..Token::default()
                });
                i += n;
            }
        }
    }
    Some(out)
}

fn seg_overlap(a: &[Token], b: &[Token]) -> bool {
    fn f(
        i: usize,
        j: usize,
        a: &[Token],
        b: &[Token],
        memo: &mut HashMap<(usize, usize), bool>,
    ) -> bool {
        if let Some(v) = memo.get(&(i, j)) {
            return *v;
        }
        let r = if i == a.len() && j == b.len() {
            true
        } else if i < a.len() && a[i].star {
            f(i + 1, j, a, b, memo) || j < b.len() && f(i, j + 1, a, b, memo)
        } else if j < b.len() && b[j].star {
            f(i, j + 1, a, b, memo) || i < a.len() && f(i + 1, j, a, b, memo)
        } else if i == a.len() || j == b.len() {
            false
        } else {
            char_overlap(&a[i], &b[j]) && f(i + 1, j + 1, a, b, memo)
        };
        memo.insert((i, j), r);
        r
    }
    f(0, 0, a, b, &mut HashMap::new())
}

fn char_overlap(a: &Token, b: &Token) -> bool {
    if a.any || b.any {
        return true;
    }
    if !a.class && !b.class {
        return fold_eq(a.lit, b.lit);
    }
    let (a, b) = if !a.class { (b, a) } else { (a, b) };
    if !b.class {
        return folds(b.lit)
            .into_iter()
            .any(|c| in_ranges(&a.ranges, c) != a.neg);
    }
    match (a.neg, b.neg) {
        (true, true) => true,
        (false, false) => {
            if has_letter(&a.ranges) && has_letter(&b.ranges) {
                return true;
            }
            a.ranges
                .iter()
                .any(|x| b.ranges.iter().any(|y| x.0 <= y.1 && y.0 <= x.1))
        }
        _ => {
            let (pos, neg) = if a.neg { (b, a) } else { (a, b) };
            has_letter(&pos.ranges) || !covered(&pos.ranges, &neg.ranges)
        }
    }
}

fn fold_eq(a: u32, b: u32) -> bool {
    folds(a).contains(&b)
}

/// `r` and every rune its `unicode.SimpleFold` orbit reaches.
fn folds(r: u32) -> Vec<u32> {
    let mut out = vec![r];
    let Some(start) = char::from_u32(r) else {
        return out;
    };
    let mut c = gocompat::text::simple_fold(start);
    while c != start {
        out.push(c as u32);
        c = gocompat::text::simple_fold(c);
    }
    out
}

fn in_ranges(rs: &[(u32, u32)], c: u32) -> bool {
    rs.iter().any(|&(lo, hi)| lo <= c && c <= hi)
}

fn has_letter(rs: &[(u32, u32)]) -> bool {
    rs.iter().any(|&(lo, hi)| {
        hi > 0x7F
            || lo <= u32::from(b'z') && hi >= u32::from(b'a')
            || lo <= u32::from(b'Z') && hi >= u32::from(b'A')
    })
}

fn covered(want: &[(u32, u32)], have: &[(u32, u32)]) -> bool {
    let mut h = have.to_vec();
    h.sort_by_key(|s| s.0);
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for s in h {
        if let Some(last) = merged.last_mut()
            && s.0 <= last.1.saturating_add(1)
        {
            last.1 = last.1.max(s.1);
            continue;
        }
        merged.push(s);
    }
    want.iter()
        .all(|w| merged.iter().any(|m| m.0 <= w.0 && w.1 <= m.1))
}
