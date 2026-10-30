//! The single-line rules and the private-key blocks (Go `scrubber.go`).

use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::marker;

fn go(p: &str) -> Regex {
    gocompat::regex::from_go(p).unwrap_or_else(|e| panic!("scrub pattern {p:?}: {e}"))
}

// Go source texts (`regexp.MustCompile` arguments, as `String()` prints them).
const JWT: &str = r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]*";
const ANTHROPIC: &str = r"\bsk-ant-[A-Za-z0-9_-]{20,}";
const OPENAI: &str = r"\bsk-[A-Za-z0-9_-]{20,}";
const GITHUB: &str = r"\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{22,})";
const SLACK: &str = r"\bxox[abprse]-[A-Za-z0-9-]{10,}";
const AWS: &str = r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b";
const GOOGLE: &str = r"\bAIza[A-Za-z0-9_-]{35}";
const BEARER: &str = r"(?i)\bbearer[ \t]+([A-Za-z0-9_\-.~+/]+=*)";
const AUTHORIZATION: &str = r#"(?i)\bauthorization(?:\\?["'])?[ \t]*[:=][ \t]*(?:\\?["'])?(?:basic|token|digest|negotiate)[ \t]+([A-Za-z0-9_\-.~+/]+=*)"#;
const URL_PASSWORD: &str =
    r#"\b[A-Za-z][A-Za-z0-9+.-]*://(?:\[redacted:[a-z-]+\]|[^\s:/@"'<>\[\]]*):([^\s/"'<>]+)@"#;
const ASSIGNMENT: &str = concat!(
    r#"(?i)(?:token|secret|password|passwd|api[_-]?key|authorization)[A-Za-z0-9_.\-]*(?:\\?["'])?[ \t]*(===|!==|==|!=|:=|=>|[:=])[ \t]*"#,
    r#"(?:\\"((?:[^"\\\r\n]|\\[^"\r\n])+?)\\""#,
    r#"|\\'((?:[^'\\\r\n]|\\[^'\r\n])+?)\\'"#,
    r#"|"((?:\\[^\r\n]|[^"\\\r\n])+)""#,
    r#"|'((?:\\[^\r\n]|[^'\\\r\n])+)'"#,
    r#"|`((?:\\[^\r\n]|[^`\\\r\n])+)`"#,
    r#"|([^\s"'`]+))"#,
);
const QUERY_PARAM: &str = r"&[A-Za-z0-9_.\-\[\]]+=";
const CALL_OR_INDEX: &str = r"^[A-Za-z_$][A-Za-z0-9_$.]*[(\[<]";
const DOTTED_NAME: &str = r"^[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)+$";
const SHELL_VAR: &str = r"^(?:\$[A-Z_][A-Z0-9_]*|%[A-Za-z_][A-Za-z0-9_]*%)$";
const SCRIPT_VAR: &str = r"^\$[A-Za-z_][A-Za-z0-9_]*(?:->|\[|\.)";
const IDENTIFIER: &str = r"^[A-Za-z_][A-Za-z0-9_]*$";
const PEM_BEGIN: &str = r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY( BLOCK)?-----";
const PEM_END: &str = r"-----END [A-Z0-9 ]*PRIVATE KEY( BLOCK)?-----";

/// Every Go pattern text by its Go variable name, for the golden check.
pub fn go_patterns() -> Vec<(&'static str, String)> {
    let mut v: Vec<(&'static str, String)> = [
        ("rules[0]", JWT),
        ("rules[1]", ANTHROPIC),
        ("rules[2]", OPENAI),
        ("rules[3]", GITHUB),
        ("rules[4]", SLACK),
        ("rules[5]", AWS),
        ("rules[6]", GOOGLE),
        ("rules[7]", BEARER),
        ("rules[8]", AUTHORIZATION),
        ("rules[9]", URL_PASSWORD),
        ("rules[10]", ASSIGNMENT),
        ("assignmentRE", ASSIGNMENT),
        ("queryParam", QUERY_PARAM),
        ("callOrIndex", CALL_OR_INDEX),
        ("dottedName", DOTTED_NAME),
        ("shellVar", SHELL_VAR),
        ("scriptVar", SCRIPT_VAR),
        ("identifier", IDENTIFIER),
        ("pemBegin", PEM_BEGIN),
        ("pemEnd", PEM_END),
    ]
    .into_iter()
    .map(|(n, p)| (n, p.to_string()))
    .collect();
    v.extend(crate::strict::go_patterns());
    v
}

type Pick = fn(&str, &Captures) -> Option<(usize, usize)>;

/// One single-line pattern. `pick` returns the span to replace (often just
/// the value group), or None to leave the match alone.
struct Rule {
    kind: &'static str,
    re: Regex,
    /// A literal that must appear in the line, case-sensitive.
    gate: &'static str,
    /// At least one must appear, ignoring ASCII case.
    lower_gates: &'static [&'static str],
    pick: Pick,
}

fn whole(_: &str, m: &Captures) -> Option<(usize, usize)> {
    let g = m.get(0)?;
    Some((g.start(), g.end()))
}

fn group(m: &Captures, i: usize) -> Option<(usize, usize)> {
    m.get(i).map(|g| (g.start(), g.end()))
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    vec![
        Rule {
            kind: "jwt",
            re: go(JWT),
            gate: "eyJ",
            lower_gates: &[],
            pick: whole,
        },
        Rule {
            kind: "anthropic",
            re: go(ANTHROPIC),
            gate: "sk-ant-",
            lower_gates: &[],
            pick: whole,
        },
        Rule {
            // sk-… with at least one digit: real keys are random, prose like
            // "sk-some-long-hyphenated-words" is not.
            kind: "openai",
            re: go(OPENAI),
            gate: "sk-",
            lower_gates: &[],
            pick: |s, m| {
                let (lo, hi) = whole(s, m)?;
                s[lo + 3..hi]
                    .bytes()
                    .any(|b| b.is_ascii_digit())
                    .then_some((lo, hi))
            },
        },
        Rule {
            kind: "github",
            re: go(GITHUB),
            gate: "_",
            lower_gates: &[],
            pick: whole,
        },
        Rule {
            kind: "slack",
            re: go(SLACK),
            gate: "xox",
            lower_gates: &[],
            pick: whole,
        },
        Rule {
            kind: "aws",
            re: go(AWS),
            gate: "A",
            lower_gates: &[],
            pick: whole,
        },
        Rule {
            kind: "google",
            re: go(GOOGLE),
            gate: "AIza",
            lower_gates: &[],
            pick: whole,
        },
        Rule {
            // Keep the word Bearer, redact the credential. A short or
            // all-letter word ("bearer instruments") is prose.
            kind: "bearer",
            re: go(BEARER),
            gate: "",
            lower_gates: &["bearer"],
            pick: |s, m| {
                let (lo, hi) = group(m, 1)?;
                credential(&s[lo..hi]).then_some((lo, hi))
            },
        },
        Rule {
            // Authorization: Basic|Token|Digest <credential>.
            kind: "bearer",
            re: go(AUTHORIZATION),
            gate: "",
            lower_gates: &["authorization"],
            pick: |_, m| {
                let (lo, hi) = group(m, 1)?;
                (hi - lo >= 8).then_some((lo, hi))
            },
        },
        Rule {
            // scheme://user:password@host. Any password length counts here.
            kind: "assignment",
            re: go(URL_PASSWORD),
            gate: "://",
            lower_gates: &[],
            pick: |s, m| {
                let (lo, hi) = group(m, 1)?;
                (!s[lo..hi].starts_with("[redacted:")).then_some((lo, hi))
            },
        },
        Rule {
            kind: "assignment",
            re: go(ASSIGNMENT),
            gate: "",
            lower_gates: &[
                "token",
                "secret",
                "password",
                "passwd",
                "apikey",
                "api_key",
                "api-key",
                "authorization",
            ],
            pick: pick_assignment,
        },
    ]
});

static QUERY_PARAM_RE: LazyLock<Regex> = LazyLock::new(|| go(QUERY_PARAM));
static CALL_OR_INDEX_RE: LazyLock<Regex> = LazyLock::new(|| go(CALL_OR_INDEX));
static DOTTED_NAME_RE: LazyLock<Regex> = LazyLock::new(|| go(DOTTED_NAME));
static SHELL_VAR_RE: LazyLock<Regex> = LazyLock::new(|| go(SHELL_VAR));
static SCRIPT_VAR_RE: LazyLock<Regex> = LazyLock::new(|| go(SCRIPT_VAR));
static IDENTIFIER_RE: LazyLock<Regex> = LazyLock::new(|| go(IDENTIFIER));
static PEM_BEGIN_RE: LazyLock<Regex> = LazyLock::new(|| go(PEM_BEGIN));
static PEM_END_RE: LazyLock<Regex> = LazyLock::new(|| go(PEM_END));

/// Go `pickAssignment`: a value of 8 or more characters. A comparison
/// (`password == x`) counts only a quoted literal. An unquoted value loses
/// trailing punctuation and any following query parameter, and is left alone
/// when it reads as code.
fn pick_assignment(s: &str, m: &Captures) -> Option<(usize, usize)> {
    for g in 2..=6 {
        if let Some((lo, hi)) = group(m, g) {
            let v = &s[lo..hi];
            return (v.len() >= 8 && !v.contains("[redacted:")).then_some((lo, hi));
        }
    }
    let sep = m.get(1)?.as_str();
    if sep.starts_with('!') || sep.starts_with("==") {
        return None;
    }
    let (lo, hi) = group(m, 7)?;
    let v = trim_value(&s[lo..hi]);
    let hi = lo + v.len();
    if v.len() < 8 || v.contains("[redacted:") || code(v) {
        return None;
    }
    Some((lo, hi))
}

/// Go `trimValue`: cut at the next query parameter, then drop closing
/// punctuation and a trailing backslash.
fn trim_value(v: &str) -> &str {
    let v = match QUERY_PARAM_RE.find(v) {
        Some(m) => &v[..m.start()],
        None => v,
    };
    v.trim_end_matches([';', ',', ')', '}', ']', '>', '\\'])
}

/// Go `code`: an unquoted value that reads as code rather than a literal.
fn code(v: &str) -> bool {
    if v == "undefined"
        || CALL_OR_INDEX_RE.is_match(v)
        || v.starts_with("${")
        || v.starts_with("$(")
        || SHELL_VAR_RE.is_match(v)
        || SCRIPT_VAR_RE.is_match(v)
    {
        return true;
    }
    let name = v.strip_prefix('*').unwrap_or(v);
    let deref = name.len() != v.len();
    if !DOTTED_NAME_RE.is_match(name) && !(deref && IDENTIFIER_RE.is_match(name)) {
        return false;
    }
    names_secret(name)
}

/// Go `namesSecret`.
fn names_secret(name: &str) -> bool {
    if contains_any_fold(
        name,
        &[
            "token",
            "secret",
            "passw",
            "apikey",
            "api_key",
            "auth",
            "credential",
        ],
    ) {
        return true;
    }
    name.split('.').any(|seg| {
        gocompat::text::equal_fold(seg, "env") || gocompat::text::equal_fold(seg, "environ")
    })
}

/// Go `credential`: 8+ characters and either not all letters or long.
fn credential(v: &str) -> bool {
    if v.len() < 8 {
        return false;
    }
    if v.len() >= 16 {
        return true;
    }
    v.bytes().any(|c| !c.is_ascii_alphabetic())
}

/// Go `containsAnyFold`: whether `s` holds any of `words` (lowercase
/// ASCII), ignoring ASCII case. Ported loop for loop, including its
/// `| 0x20` folding (so `_` only matches itself through the second test).
pub fn contains_any_fold(s: &str, words: &[&str]) -> bool {
    let s = s.as_bytes();
    for i in 0..s.len() {
        let c = s[i] | 0x20;
        for w in words {
            let w = w.as_bytes();
            if c != w[0] || s.len() - i < w.len() {
                continue;
            }
            let mut j = 1;
            while j < w.len() && (s[i + j] | 0x20 == w[j] || w[j] == s[i + j]) {
                j += 1;
            }
            if j == w.len() {
                return true;
            }
        }
    }
    false
}

/// Go `scrubLines`: the rules on each line that passes their gates.
pub fn scrub_lines(s: &str) -> String {
    let mut out: Option<String> = None;
    let mut pos = 0;
    while pos < s.len() {
        let rest = &s[pos..];
        let (line, nl) = match rest.find('\n') {
            Some(n) => (&rest[..n], "\n"),
            None => (rest, ""),
        };
        let got = scrub_line(line);
        if out.is_none() && got != line {
            let mut b = String::with_capacity(s.len() + 64);
            b.push_str(&s[..pos]);
            out = Some(b);
        }
        if let Some(b) = out.as_mut() {
            b.push_str(&got);
            b.push_str(nl);
        }
        pos += line.len() + nl.len();
    }
    out.unwrap_or_else(|| s.to_string())
}

fn scrub_line(line: &str) -> String {
    let mut line = line.to_string();
    for r in RULES.iter() {
        if !r.gate.is_empty() && !line.contains(r.gate) {
            continue;
        }
        if !r.lower_gates.is_empty() && !contains_any_fold(&line, r.lower_gates) {
            continue;
        }
        if let Some(next) = apply(&line, r) {
            line = next;
        }
    }
    line
}

/// Go `apply`: one rule over `s`; None when nothing changed.
fn apply(s: &str, r: &Rule) -> Option<String> {
    let mut b = String::new();
    let mut last = 0;
    for m in r.re.captures_iter(s) {
        let Some((lo, hi)) = (r.pick)(s, &m) else {
            continue;
        };
        if lo < last || hi <= lo {
            continue;
        }
        b.push_str(&s[last..lo]);
        b.push_str(&marker(r.kind, &s[lo..hi]));
        last = hi;
    }
    if last == 0 {
        return None;
    }
    b.push_str(&s[last..]);
    Some(b)
}

/// Go `scrubPrivateKeys`: each BEGIN … PRIVATE KEY … END block becomes the
/// marker on its first line, and each later line keeps only its leading
/// diff/indent prefix. A block with no END runs to the end of the text.
pub fn scrub_private_keys(s: &str) -> String {
    if !s.contains("PRIVATE KEY") {
        return s.to_string();
    }
    let mut b = String::new();
    let mut last = 0;
    while let Some(loc) = PEM_BEGIN_RE.find(&s[last..]) {
        let start = last + loc.start();
        let after = last + loc.end();
        let end = match PEM_END_RE.find(&s[after..]) {
            Some(e) => after + e.end(),
            None => s.len(),
        };
        b.push_str(&s[last..start]);
        b.push_str("[redacted:private-key]");
        let mut block = &s[start..end];
        while let Some(i) = block.find(['\r', '\n']) {
            let mut j = i + 1;
            if block.as_bytes()[i] == b'\r' && j < block.len() && block.as_bytes()[j] == b'\n' {
                j += 1;
            }
            b.push_str(&block[i..j]);
            block = &block[j..];
            b.push_str(line_prefix(block));
        }
        last = end;
    }
    if last == 0 {
        return s.to_string();
    }
    b.push_str(&s[last..]);
    b
}

/// Go `linePrefix`: one optional diff marker (+, - or space, but not the
/// first dash of "-----END") and then any indentation.
fn line_prefix(line: &str) -> &str {
    let l = line.as_bytes();
    let mut i = 0;
    let bare_end = line.starts_with("-----") && !line.starts_with("------");
    if i < l.len() && (l[i] == b'+' || l[i] == b' ' || l[i] == b'-' && !bare_end) {
        i += 1;
    }
    while i < l.len() && (l[i] == b' ' || l[i] == b'\t') {
        i += 1;
    }
    &line[..i]
}
