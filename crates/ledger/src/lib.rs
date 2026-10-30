//! The PR authorship ledger and PR reference parsing, a port of
//! I:internal/ledger (store.md §8). It is pure: the capture service feeds it
//! entries folded from the log and asks it who owns a PR.
//!
//! The Go package's files map one to one: [`keys`] (keys.go), [`store`]
//! (store.go), [`refs`] (refs.go) and [`bash`] (bash.go). Go's RE2 classes
//! `\w \d \s \b` are ASCII, so every pattern is built with
//! [`gocompat::regex::from_go`]; in particular `\s` never matches `\v`
//! (store.md §7.12).

mod bash;
mod keys;
mod refs;
mod store;

use std::borrow::Cow;
use std::fmt;

use gocompat::json::{self, Out, Schema};
use gocompat::time::GoTime;
use serde::Serialize;

pub use bash::{BashRef, BashResult, from_bash};
pub use keys::{key, repo_from_key, repo_key};
pub use refs::{Ref, RefOptions, extract_refs, parse_target};
pub use store::Ledger;

/// Ranks an owner. Go's `Role` is a string type, so a decoded entry may
/// carry any role; only the three below are ranked, and [`Ledger::add`]
/// ignores the rest. Rank order (tsukumo's code, not its comment): author 0,
/// touched 1, reviewer 2.
// No Ord: string order is not rank order; compare with `rank`.
#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub struct Role(Cow<'static, str>);

impl Role {
    pub const AUTHOR: Role = Role(Cow::Borrowed("author"));
    pub const TOUCHED: Role = Role(Cow::Borrowed("touched"));
    pub const REVIEWER: Role = Role(Cow::Borrowed("reviewer"));

    /// Any role string, as Go's `Role("...")` conversion.
    pub fn new(s: impl Into<String>) -> Role {
        Role(Cow::Owned(s.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The rank of a known role (`roleRank` in store.go), `None` otherwise.
    pub fn rank(&self) -> Option<u8> {
        match self.as_str() {
            "author" => Some(0),
            "touched" => Some(1),
            "reviewer" => Some(2),
            _ => None,
        }
    }

    /// Go's `roleRank[r]` map read: 0 for an unknown role.
    pub(crate) fn rank_or_zero(&self) -> u8 {
        self.rank().unwrap_or(0)
    }
}

impl fmt::Debug for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Role {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// How long an entry stays in the view: 45 × 24 h, in seconds.
pub const KEEP_SECS: i64 = 45 * 24 * 3600;

/// One ledger row: a session's relation to a PR. JSON field order and names
/// are Go's (ledger.go:25-34); marshal it with [`gocompat::json::marshal`]
/// for Go's bytes.
///
/// There is deliberately no `Deserialize`: Go reads `pr.ledger` payloads with
/// `json.Unmarshal` (case-insensitive keys, last duplicate wins), which
/// [`decode_entry`] reproduces.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Entry {
    /// [`key`]`(repo, number)`.
    pub key: String,
    /// `"owner/name"` as first seen (case kept).
    pub repo: String,
    pub number: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// Agent key `"<device>/<sessionId>"`.
    pub session: String,
    pub role: Role,
    /// `"pr-link"`, `"gh"` or `"gitOperation"`.
    pub source: String,
    pub at: GoTime,
}

fn entry_schema() -> Schema {
    Schema::object(vec![
        ("key", Schema::String),
        ("repo", Schema::String),
        ("number", Schema::Int(64)),
        ("url", Schema::String),
        ("session", Schema::String),
        ("role", Schema::String),
        ("source", Schema::String),
        ("at", Schema::Time),
    ])
}

/// `json.Unmarshal(payload, &e)` into a zero `ledger.Entry`, as capture
/// reads `pr.ledger` rows (I:internal/capture/service.go:293). Any decode
/// error fails the whole value.
pub fn decode_entry(payload: &[u8]) -> Result<Entry, json::Error> {
    let out = json::unmarshal(payload, &entry_schema())?;
    let int = |name: &str| match out.get(name) {
        Some(Out::Int(n)) => *n,
        _ => 0,
    };
    let at = match out.get("at") {
        Some(Out::Time(t)) => *t,
        _ => GoTime::ZERO,
    };
    Ok(Entry {
        key: out.get_str("key").to_string(),
        repo: out.get_str("repo").to_string(),
        number: int("number"),
        url: out.get_str("url").to_string(),
        session: out.get_str("session").to_string(),
        role: Role::new(out.get_str("role")),
        source: out.get_str("source").to_string(),
        at,
    })
}

/// Seconds from Go's internal epoch (year 1) to the Unix epoch.
const UNIX_TO_INTERNAL: i128 = 62_135_596_800;

/// A time as Go compares it: internal seconds since year 1, then nanoseconds.
pub(crate) fn instant(t: &GoTime) -> (i128, u32) {
    (i128::from(t.secs) + UNIX_TO_INTERNAL, t.nanos)
}

/// `now.Add(-Keep)` as an instant. Go's `addSec` saturates at ±(2^63-1).
pub(crate) fn keep_cutoff(now: &GoTime) -> (i128, u32) {
    let (s, n) = instant(now);
    let max = i128::from(i64::MAX);
    ((s - i128::from(KEEP_SECS)).clamp(-max, max), n)
}
