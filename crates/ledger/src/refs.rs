//! PR references in free text and explicit PR addresses
//! (I:internal/ledger/refs.go, router.md §12).

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use gocompat::regex::from_go;
use gocompat::text;
use regex::Regex;
use serde::Serialize;

use crate::keys::{key, trim_space};

/// One PR.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize)]
pub struct Ref {
    /// `"owner/name"`.
    pub repo: String,
    pub number: i64,
}

/// [`extract_refs`]' defaults.
#[derive(Debug, Clone, Default)]
pub struct RefOptions {
    /// `"owner/name"`: for bare `#N` and `pr#N` / `pull#N`.
    pub default_repo: String,
    /// Default owner for `name#N`; else default_repo's owner.
    pub owner: String,
    /// Lowercased alias -> repo (`"name"` or `"owner/name"`).
    pub aliases: HashMap<String, String>,
}

fn re(go: &str) -> Regex {
    from_go(go).expect("ledger regex")
}

static FENCED_CODE: LazyLock<Regex> = LazyLock::new(|| re(r"```[\s\S]*?```"));
static INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| re("`[^`\\n]*`"));

// The three alternatives of tsukumo's pattern, anchored and tried in order at
// each position, so a failed lookbehind retries at the next position just as
// it does in JS.
static REF_URL: LazyLock<Regex> =
    LazyLock::new(|| re(r"^github\.com/([\w.-]+)/([\w.-]+)/pull/(\d+)"));
static REF_NAME: LazyLock<Regex> =
    LazyLock::new(|| re(r"^([A-Za-z][\w.-]*(?:/[\w.-]+)?)#(\d{1,6})\b"));
static REF_BARE: LazyLock<Regex> = LazyLock::new(|| re(r"^#(\d{2,5})\b"));

pub(crate) static TARGET_URL: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)^(?:https?://)?(?:www\.)?github\.com/([\w.-]+)/([\w.-]+)/pull/(\d+)(?:[/?#].*)?$")
});
static TARGET_NAME: LazyLock<Regex> = LazyLock::new(|| re(r"^([\w.-]+)/([\w.-]+)#(\d+)$"));

fn is_word_byte(c: u8) -> bool {
    c == b'_' || c.is_ascii_digit() || c.is_ascii_alphabetic()
}

/// Whether the byte before p is an ASCII word byte or one of extra. JS `\w`
/// is ASCII, so a non-ASCII rune never blocks; its last UTF-8 byte is >=
/// 0x80, so checking one byte is enough.
fn prev_blocks(s: &[u8], p: usize, extra: &[u8]) -> bool {
    if p == 0 {
        return false;
    }
    let c = s[p - 1];
    is_word_byte(c) || extra.contains(&c)
}

/// Whether s is a usable owner or repo name: GitHub never allows one made
/// only of dots, such as `.` or `..`.
pub(crate) fn valid_segment(s: &str) -> bool {
    !s.is_empty() && !s.trim_matches('.').is_empty()
}

/// Whether repo is `"owner/name"` with valid segments.
fn valid_repo(repo: &str) -> bool {
    match repo.split_once('/') {
        Some((o, n)) => valid_segment(o) && valid_segment(n) && !n.contains('/'),
        None => false,
    }
}

/// Go `strconv.Atoi` for a PR number; 0, overflow and junk are not PRs.
pub(crate) fn pr_number(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let digits = match b.first() {
        Some(b'+' | b'-') => &s[1..],
        _ => s,
    };
    // Atoi takes an optional sign then decimal digits only.
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: i64 = s.parse().ok()?;
    (n > 0).then_some(n)
}

/// Finds PR references in free text, in order, deduplicated by key: GitHub
/// PR URLs, `owner/name#N` and `name#N` (1-6 digits), bare `#N` (2-5 digits,
/// needs a default repo). Fenced and inline code is stripped first. RE2 has
/// no lookbehind, so the byte before a match is checked by hand: `name#N`
/// rejects a preceding `[A-Za-z0-9_/<#-]`, bare `#N` a preceding
/// `[A-Za-z0-9_/<#&@]`; after the digits the next rune must not be a word
/// character.
pub fn extract_refs(text_in: &str, o: &RefOptions) -> Vec<Ref> {
    let owner: &str = if o.owner.is_empty() {
        o.default_repo.split('/').next().unwrap_or("")
    } else {
        &o.owner
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |repo: &str, digits: &str| {
        let Some(n) = pr_number(digits) else { return };
        if !valid_repo(repo) {
            return;
        }
        if seen.insert(key(repo, n)) {
            out.push(Ref {
                repo: repo.to_string(),
                number: n,
            });
        }
    };

    // Code spans and blocks quote things; they don't name the subject.
    let prose = FENCED_CODE.replace_all(text_in, " ");
    let prose = INLINE_CODE.replace_all(&prose, " ");
    let b = prose.as_bytes();

    let mut p = 0;
    while p < b.len() {
        let c = b[p];
        // Only slice at an ASCII byte: p is then on a char boundary.
        if c == b'g'
            && let Some(m) = REF_URL.captures(&prose[p..])
        {
            add(&format!("{}/{}", &m[1], &m[2]), &m[3]);
            p += m[0].len();
            continue;
        }
        if c.is_ascii_alphabetic()
            && !prev_blocks(b, p, b"/<#-")
            && let Some(m) = REF_NAME.captures(&prose[p..])
        {
            let (name, digits) = (m.get(1).unwrap().as_str(), &m[2]);
            let lower = text::to_lower_str(name);
            if lower == "pr" || lower == "pull" {
                // "PR#1407" / "pull#1407" mean the default repo.
                add(&o.default_repo, digits);
            } else {
                let name = o.aliases.get(&lower).map(String::as_str).unwrap_or(name);
                if name.contains('/') {
                    add(name, digits);
                } else if !owner.is_empty() {
                    add(&format!("{owner}/{name}"), digits);
                }
            }
            p += m[0].len();
            continue;
        }
        if c == b'#'
            && !prev_blocks(b, p, b"/<#&@")
            && let Some(m) = REF_BARE.captures(&prose[p..])
        {
            add(&o.default_repo, &m[1]);
            p += m[0].len();
            continue;
        }
        p += 1;
    }
    out
}

/// Reads an explicit PR address: `"pr:owner/name#N"`, `"owner/name#N"` or a
/// GitHub PR URL (with or without scheme, trailing path such as /files
/// allowed).
pub fn parse_target(s: &str) -> Option<Ref> {
    let mut s = trim_space(s);
    if let Some(m) = TARGET_URL.captures(s) {
        let n = pr_number(&m[3])?;
        if valid_segment(&m[1]) && valid_segment(&m[2]) {
            return Some(Ref {
                repo: format!("{}/{}", &m[1], &m[2]),
                number: n,
            });
        }
        return None;
    }
    if s.len() >= 3 && s.as_bytes()[..3].eq_ignore_ascii_case(b"pr:") {
        s = &s[3..];
    }
    let m = TARGET_NAME.captures(s)?;
    let n = pr_number(&m[3])?;
    if valid_segment(&m[1]) && valid_segment(&m[2]) {
        return Some(Ref {
            repo: format!("{}/{}", &m[1], &m[2]),
            number: n,
        });
    }
    None
}
