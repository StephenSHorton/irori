//! PR relations a Bash call shows (I:internal/ledger/bash.go).

use std::collections::HashSet;
use std::sync::LazyLock;

use gocompat::json::{self, Out, Schema};
use gocompat::regex::from_go;
use gocompat::text;
use regex::Regex;

use crate::Role;
use crate::keys::key;
use crate::refs::{Ref, TARGET_URL, pr_number, valid_segment};

fn re(go: &str) -> Regex {
    from_go(go).expect("ledger regex")
}

// PR_URL finds a GitHub PR URL in output. The host must start the URL (no
// look-alike such as evilgithub.com) and the number must end at a word
// boundary (no pull/3abc).
static PR_URL: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)(?:^|[^\w.-])(?:https?://)?(?:www\.)?github\.com/([\w.-]+)/([\w.-]+)/pull/(\d+)\b")
});
static GH_PR_CREATE: LazyLock<Regex> = LazyLock::new(|| re(r"\bgh\s+pr\s+create\b"));
static GH_PR_REVIEW: LazyLock<Regex> = LazyLock::new(|| re(r"\bgh\s+pr\s+review(?:\s|$)"));
static DIGITS_ONLY: LazyLock<Regex> = LazyLock::new(|| re(r"^\d+$"));

/// What the transcript holds for one Bash call.
#[derive(Debug, Clone, Default)]
pub struct BashResult {
    pub command: String,
    /// May be clipped.
    pub stdout: String,
    /// Raw `toolUseResult.gitOperation` JSON; empty when there is none.
    pub git_operation: Vec<u8>,
    /// `"owner/name"` from the session's repoKey, or `""`.
    pub default_repo: String,
}

/// A PR relation a Bash call shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BashRef {
    pub r#ref: Ref,
    pub url: String,
    pub role: Role,
    /// `"gitOperation"` or `"gh"`.
    pub source: String,
}

/// Reads PR relations from one Bash call:
/// - `gitOperation.pr` with action `"created"` -> author (source
///   gitOperation);
/// - else, when the command matches `\bgh\s+pr\s+create\b`, the first
///   `github.com/<o>/<r>/pull/<n>` in stdout -> author (source gh);
/// - every `gh pr review <target>`: a URL, or a number with the repo from
///   `-R`/`--repo <o/r>`, else the default repo -> reviewer (source gh). A
///   number with no repo is skipped.
pub fn from_bash(b: &BashResult) -> Vec<BashRef> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |r: BashRef| {
        let id = format!("{}\0{}", key(&r.r#ref.repo, r.r#ref.number), r.role);
        if seen.insert(id) {
            out.push(r);
        }
    };

    if let Some((r, url)) = created_from_git_operation(b) {
        push(BashRef {
            r#ref: r,
            url,
            role: Role::AUTHOR,
            source: "gitOperation".into(),
        });
    } else if GH_PR_CREATE.is_match(&b.command)
        && let Some((r, url)) = find_pr_url(&b.stdout)
    {
        push(BashRef {
            r#ref: r,
            url,
            role: Role::AUTHOR,
            source: "gh".into(),
        });
    }

    // Each review's arguments run to the first unquoted separator; the next
    // search starts after them, so "gh pr review" quoted in a body is text.
    let mut cmd = b.command.as_str();
    while let Some(m) = GH_PR_REVIEW.find(cmd) {
        let (args, n) = shell_words(&cmd[m.end()..]);
        if let Some((r, url)) = review_target(&args, &b.default_repo) {
            push(BashRef {
                r#ref: r,
                url,
                role: Role::REVIEWER,
                source: "gh".into(),
            });
        }
        cmd = &cmd[m.end() + n..];
    }
    out
}

/// The first GitHub PR URL in s, as a Ref and its canonical
/// `https://github.com/<o>/<r>/pull/<n>` form.
fn find_pr_url(s: &str) -> Option<(Ref, String)> {
    let m = PR_URL.captures(s)?;
    if !valid_segment(&m[1]) || !valid_segment(&m[2]) {
        return None;
    }
    let n = pr_number(&m[3])?;
    Some(pr_ref(format!("{}/{}", &m[1], &m[2]), n))
}

fn pr_ref(repo: String, n: i64) -> (Ref, String) {
    let url = format!("https://github.com/{repo}/pull/{n}");
    (Ref { repo, number: n }, url)
}

fn git_operation_schema() -> Schema {
    Schema::object(vec![(
        "pr",
        Schema::Ptr(Box::new(Schema::object(vec![
            ("number", Schema::Int(64)),
            ("url", Schema::String),
            ("action", Schema::String),
        ]))),
    )])
}

/// Reads gitOperation.pr when its action is "created". The repo comes from
/// the URL, else the default repo; a URL that is not a GitHub PR is dropped.
fn created_from_git_operation(b: &BashResult) -> Option<(Ref, String)> {
    if b.git_operation.is_empty() {
        return None;
    }
    // json.Unmarshal into the gitOperation struct: any error drops it.
    let op = json::unmarshal(&b.git_operation, &git_operation_schema()).ok()?;
    let pr = match op.get("pr") {
        Some(Out::Ptr(Some(pr))) => pr,
        _ => return None,
    };
    if pr.get_str("action") != "created" {
        return None;
    }
    let mut n = match pr.get("number") {
        Some(Out::Int(n)) => *n,
        _ => 0,
    };
    let (mut repo, mut url) = (b.default_repo.clone(), String::new());
    if let Some((r, u)) = find_pr_url(pr.get_str("url")) {
        repo = r.repo;
        url = u;
        if n <= 0 {
            n = r.number;
        }
    }
    if repo.is_empty() || n <= 0 {
        return None;
    }
    Some((Ref { repo, number: n }, url))
}

/// Splits s like a shell into words, honouring '...' and "..." (with \" and
/// \\ escapes inside double quotes), up to the first unquoted ; & | or line
/// break. Outside quotes a backslash is literal, so Windows paths survive.
/// Returns the words and the bytes consumed.
fn shell_words(s: &str) -> (Vec<String>, usize) {
    // Every byte the loop looks at is ASCII, so words never split a
    // multibyte character.
    fn word(w: &mut Vec<u8>) -> String {
        String::from_utf8(std::mem::take(w)).expect("words split only at ASCII bytes")
    }
    let b = s.as_bytes();
    let mut words = Vec::new();
    let mut w: Vec<u8> = Vec::new();
    let mut in_word = false;
    let mut quote = 0u8;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if quote == b'\'' {
            if c == b'\'' {
                quote = 0;
            } else {
                w.push(c);
            }
        } else if quote == b'"' {
            if c == b'"' {
                quote = 0;
            } else if c == b'\\' && i + 1 < b.len() && (b[i + 1] == b'"' || b[i + 1] == b'\\') {
                i += 1;
                w.push(b[i]);
            } else {
                w.push(c);
            }
        } else if c == b'\'' || c == b'"' {
            quote = c;
            in_word = true;
        } else if matches!(c, b';' | b'&' | b'|' | b'\n' | b'\r') {
            if in_word {
                words.push(word(&mut w));
            }
            return (words, i);
        } else if c == b' ' || c == b'\t' {
            if in_word {
                words.push(word(&mut w));
                in_word = false;
            }
        } else {
            w.push(c);
            in_word = true;
        }
        i += 1;
    }
    if in_word {
        words.push(word(&mut w));
    }
    (words, i)
}

/// gh pr review's flags that take a value; its other flags (-a, -c, -r, -h
/// and their long forms) take none.
fn review_value_flag(f: &str) -> bool {
    matches!(f, "-b" | "--body" | "-F" | "--body-file" | "-R" | "--repo")
}

/// Reads the words after "gh pr review". The target is the first positional
/// word; flags are skipped with their values, and "--" ends them. A URL
/// names its PR; a number takes the repo from -R/--repo (the last one;
/// [HOST/]OWNER/REPO, host github.com only), else the default repo. A -R
/// value that is not a GitHub repo skips the review.
fn review_target(args: &[String], default_repo: &str) -> Option<(Ref, String)> {
    let (mut target, mut repo_flag, mut has_repo) = ("", "", false);
    let mut flags = true;
    let mut i = 0;
    while i < args.len() {
        let f = args[i].as_str();
        if !flags || f == "-" || !f.starts_with('-') {
            if target.is_empty() {
                target = f;
            }
            i += 1;
            continue;
        }
        if f == "--" {
            flags = false;
        } else if review_value_flag(f) {
            if i + 1 < args.len() {
                i += 1;
                if f == "-R" || f == "--repo" {
                    repo_flag = &args[i];
                    has_repo = true;
                }
            }
        } else if f.starts_with("--") {
            if let Some(("--repo", v)) = f.split_once('=') {
                repo_flag = v;
                has_repo = true;
            }
        } else if let Some(v) = f.strip_prefix("-R") {
            repo_flag = v;
            has_repo = true;
        }
        i += 1;
    }
    if target.is_empty() {
        return None;
    }
    if target.contains('/') {
        let m = TARGET_URL.captures(target)?;
        if !valid_segment(&m[1]) || !valid_segment(&m[2]) {
            return None;
        }
        let n = pr_number(&m[3])?;
        return Some(pr_ref(format!("{}/{}", &m[1], &m[2]), n));
    }
    if !DIGITS_ONLY.is_match(target) {
        return None;
    }
    let n = pr_number(target)?;
    let mut repo = default_repo.to_string();
    if has_repo {
        repo = repo_arg(repo_flag)?;
    }
    if repo.is_empty() {
        return None;
    }
    Some((Ref { repo, number: n }, String::new()))
}

/// Reads a -R/--repo value, [HOST/]OWNER/REPO, as "owner/name". Only
/// github.com (or www.github.com) is accepted as a host: the ledger keys are
/// github.com repos.
fn repo_arg(v: &str) -> Option<String> {
    let v = v.strip_suffix('/').unwrap_or(v);
    let mut parts: Vec<&str> = v.split('/').collect();
    if parts.len() == 3 {
        let h = text::to_lower_str(parts[0]);
        if h != "github.com" && h != "www.github.com" {
            return None;
        }
        parts.remove(0);
    }
    if parts.len() != 2 || !valid_segment(parts[0]) || !valid_segment(parts[1]) {
        return None;
    }
    Some(format!("{}/{}", parts[0], parts[1]))
}
