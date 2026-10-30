//! Port of Go `internal/scrub` (capture.md §3 "scrub", §7.3-7.4, §7.7):
//! removes secrets from text before capture stores it. It runs on every
//! excerpt, tool target, summary and patch, before hashing, so its output
//! is part of row ids: the rules here are Go's, pattern for pattern, and
//! must not be "improved" (capture.md §7.7).
//!
//! Matches become `[redacted:<kind>]` plus every `\r`/`\n` of the match, so
//! line counts never change. Kinds: anthropic, openai, github, slack, aws,
//! google, jwt, private-key, bearer, assignment, exact.
//!
//! Every pattern is the Go source text, compiled with
//! [`gocompat::regex::from_go`] so RE2's ASCII `\b` and `\s` hold
//! (`testdata/golden/scrub/patterns.json` pins the texts to Go's).
//!
//! - [`Scrubber`]: the exact values plus the shared patterns ([`string`] is
//!   a nil Go `*Scrubber`: patterns only).
//! - [`skip_file`]: paths whose patch is never stored.
//! - [`strict`]: Share to board's pass that drops absolute paths.

mod rules;
mod skipfile;
mod strict;

use std::sync::{Arc, RwLock};

use aho_corasick::{AhoCorasick, MatchKind};

pub use skipfile::skip_file;
pub use strict::{PATH_MARKER, strict, strict_dropped};

/// Removes secrets from text. Safe for concurrent use; [`Scrubber::default`]
/// is Go's zero `Scrubber` (patterns, no exact values).
#[derive(Default)]
pub struct Scrubber {
    exact: RwLock<Option<Arc<Exact>>>,
}

impl std::fmt::Debug for Scrubber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the values: they are secrets.
        let n = self.load().map_or(0, |e| e.markers.len());
        f.debug_struct("Scrubber")
            .field("exact_values", &n)
            .finish()
    }
}

/// The compiled exact-value replacer (Go `strings.NewReplacer` over the
/// values, longest first).
struct Exact {
    ac: AhoCorasick,
    markers: Vec<String>,
}

impl Scrubber {
    /// Go `New(exact...)`: a scrubber that also redacts each of `exact`
    /// (kind "exact"), e.g. Irori's own lockfile token and attach secrets.
    /// Empty and short (<8 bytes) values are ignored.
    pub fn new<S: AsRef<str>>(exact: &[S]) -> Scrubber {
        let s = Scrubber::default();
        s.set_exact(exact);
        s
    }

    /// Go `SetExact`: replaces the exact-value list.
    pub fn set_exact<S: AsRef<str>>(&self, exact: &[S]) {
        let mut vals: Vec<&str> = Vec::new();
        for v in exact {
            let v = v.as_ref();
            if v.len() < 8 || vals.contains(&v) {
                continue;
            }
            vals.push(v);
        }
        let next = if vals.is_empty() {
            None
        } else {
            // Longest first, so a value that contains another is replaced
            // whole. Go's sort.Slice is not stable, but values of one
            // length can't both match at one position, so order among
            // them never shows.
            vals.sort_by_key(|v| std::cmp::Reverse(v.len()));
            let ac = AhoCorasick::builder()
                .match_kind(MatchKind::LeftmostFirst)
                .build(&vals)
                .expect("exact values compile");
            let markers = vals.iter().map(|v| marker("exact", v)).collect();
            Some(Arc::new(Exact { ac, markers }))
        };
        *self.exact.write().unwrap_or_else(|e| e.into_inner()) = next;
    }

    fn load(&self) -> Option<Arc<Exact>> {
        self.exact.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Go `String`: `input` with every secret replaced. It never grows a
    /// line by more than the marker, and it keeps line breaks, so patch
    /// line counts don't change.
    pub fn string(&self, input: &str) -> String {
        if input.is_empty() {
            return String::new();
        }
        match self.load() {
            Some(e) => {
                let out = e.ac.replace_all(input, &e.markers);
                patterns(&out)
            }
            None => patterns(input),
        }
    }
}

/// A nil Go `*Scrubber`'s `String`: the patterns with no exact values.
pub fn string(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    patterns(input)
}

/// The private-key blocks, then each single-line rule in order.
fn patterns(input: &str) -> String {
    let out = rules::scrub_private_keys(input);
    rules::scrub_lines(&out)
}

/// Go `marker`: the kind tag plus every line break in `m`, so the line
/// count never changes.
pub(crate) fn marker(kind: &str, m: &str) -> String {
    let mut out = format!("[redacted:{kind}]");
    for b in m.bytes() {
        if b == b'\r' || b == b'\n' {
            out.push(b as char);
        }
    }
    out
}

#[doc(hidden)]
pub use rules::{contains_any_fold, go_patterns};
