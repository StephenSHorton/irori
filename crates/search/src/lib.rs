//! Port of Go `internal/search` (capture.md §1, §2.7): the types shared by
//! Irori's search and show tools. There is no index here: capture owns the
//! index and the dispatch, and every service whose rows should be found
//! implements [`Source`]. Keeping the types here means coord and inbox
//! implement `Source` without depending on capture.
//!
//! Progressive disclosure: an agent asks for the overview, then a short
//! list of [`Hit`]s (one line each, with a ref), then Show(ref) for one
//! item's detail. Every result is capped and says how to go deeper.
//!
//! JSON: [`Hit`] and [`Shown`] serialize as Go's structs do (field order,
//! `omitempty`, times as RFC3339Nano); encode them with
//! [`gocompat::json::marshal`] for Go's escaping.

use gocompat::time::GoTime;
use serde::{Deserialize, Serialize};

/// Search and show errors. `BadRef` and `NotFound` carry Go's texts, which
/// tools print.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Go `ErrBadRef`.
    #[error("search: not a ref (kind:id, e.g. task:01J... or pr:owner/repo#12)")]
    BadRef,
    /// Go `ErrNotFound`.
    #[error("search: nothing has that ref")]
    NotFound,
    /// Go's `fmt.Errorf("%w: %s", ErrNotFound, detail)`: NotFound naming
    /// what was asked.
    #[error("search: nothing has that ref: {0}")]
    NotFoundRef(String),
    /// Any other failure a source reports, with its text.
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Go `errors.Is(err, ErrNotFound)`.
    pub fn is_not_found(&self) -> bool {
        matches!(self, Error::NotFound | Error::NotFoundRef(_))
    }
}

/// A ref's prefix: what sort of thing it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// capture.TurnRow.ID ("t-...").
    Turn,
    /// capture.EditRow.ID ("e-...").
    Edit,
    /// Full sha, or a unique prefix of at least 7 hex.
    Commit,
    /// inbox.Message.ID.
    Msg,
    /// coord.Task.ID.
    Task,
    /// coord.Comment.ID.
    Comment,
    /// Ledger key: "pr:<owner/name>#<n>".
    #[serde(rename = "pr")]
    Pr,
    /// decision.Decision.ID.
    Decision,
    /// Show-only (never indexed): "agent:<device>/<session>".
    Agent,
}

impl Kind {
    /// Go's string value.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Turn => "turn",
            Kind::Edit => "edit",
            Kind::Commit => "commit",
            Kind::Msg => "msg",
            Kind::Task => "task",
            Kind::Comment => "comment",
            Kind::Pr => "pr",
            Kind::Decision => "decision",
            Kind::Agent => "agent",
        }
    }

    /// The kind named exactly `s` (Go compares kinds as plain strings).
    pub fn from_name(s: &str) -> Option<Kind> {
        KINDS.iter().copied().find(|k| k.as_str() == s)
    }
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Every ref kind, in the order tools print them (Go `Kinds`).
pub const KINDS: [Kind; 9] = [
    Kind::Turn,
    Kind::Edit,
    Kind::Commit,
    Kind::Msg,
    Kind::Task,
    Kind::Comment,
    Kind::Pr,
    Kind::Decision,
    Kind::Agent,
];

/// Hits per Search by default.
pub const DEFAULT_LIMIT: i64 = 10;
/// Most hits per Search.
pub const MAX_LIMIT: i64 = 50;
/// Runes in Hit.Title / Shown.Title (one line).
pub const MAX_TITLE: usize = 120;
/// Runes in Hit.Snippet (one line, match marked with [ ]).
pub const MAX_SNIPPET: usize = 240;
/// Bytes of Shown.Text by default.
pub const DEFAULT_SHOW: i64 = 4000;
/// Most bytes of Shown.Text.
pub const MAX_SHOW: i64 = 6000;
/// Most refs in Shown.Related.
pub const MAX_RELATED: usize = 12;

/// Go `Ref`: "kind:id".
pub fn make_ref(k: Kind, id: &str) -> String {
    format!("{}:{id}", k.as_str())
}

/// Go `ParseRef`: a ref's kind and id. The id is everything after the
/// first ':' (a pr ref keeps "owner/name#n"; an agent ref keeps its slash).
/// Surrounding spaces are trimmed; the kind is case-insensitive.
pub fn parse_ref(r: &str) -> Result<(Kind, String), Error> {
    let r = r.trim_matches(gocompat::text::is_space);
    let Some((k, id)) = r.split_once(':') else {
        return Err(Error::BadRef);
    };
    if id.is_empty() {
        return Err(Error::BadRef);
    }
    let kind = Kind::from_name(&gocompat::text::to_lower_str(k)).ok_or(Error::BadRef)?;
    let mut id = id.to_string();
    match kind {
        Kind::Commit => {
            let lower = gocompat::text::to_lower_str(&id);
            if id.len() < 7
                || id.len() > 40
                || !lower
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(Error::BadRef);
            }
            id = lower;
        }
        Kind::Pr => {
            let Some((repo, n)) = id.split_once('#') else {
                return Err(Error::BadRef);
            };
            if !repo.contains('/') || n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
                return Err(Error::BadRef);
            }
        }
        _ => {}
    }
    if id.contains([' ', '\t', '\r', '\n']) {
        return Err(Error::BadRef);
    }
    Ok((kind, id))
}

/// One searchable item as a [`Source`] hands it to the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doc {
    /// "task:<id>", ...
    pub r#ref: String,
    /// Ref's kind.
    pub kind: Kind,
    /// When it happened (shown, and Query.before compares it).
    pub at: GoTime,
    /// When it last changed (Source::search_docs pages on it); zero = at.
    pub updated: GoTime,
    /// One line; the index trims it to MAX_TITLE.
    pub title: String,
    /// Indexed text; scrubbed already.
    pub body: String,
    /// Agent key it belongs to, if any.
    pub agent: String,
    pub repo_key: String,
    /// When set, the only agent keys that may find or show it (a message:
    /// its sender and recipients). Empty: every viewer.
    pub readers: Vec<String>,
}

/// Selects hits. Zero values mean any.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// Words; every word must match (prefix match on the last); "" lists
    /// newest.
    pub text: String,
    /// Restrict to these kinds.
    pub kinds: Vec<Kind>,
    /// Restrict to one repo.
    pub repo_key: String,
    /// Restrict to one agent key.
    pub agent: String,
    /// Only items with At strictly before this (zero: no filter).
    pub before: GoTime,
    /// Skip that many hits: page by passing the number already shown.
    pub offset: i64,
    /// Default DEFAULT_LIMIT, max MAX_LIMIT.
    pub limit: i64,
    /// Who is asking: "" is the person's view (everything), else an agent
    /// key, and docs whose readers leave it out are skipped.
    pub viewer: String,
}

/// One search result: one line plus a ref to Show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hit {
    #[serde(rename = "ref")]
    pub r#ref: String,
    pub kind: Kind,
    pub at: GoTime,
    pub title: String,
    pub snippet: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent: String,
    #[serde(rename = "repoKey", default, skip_serializing_if = "String::is_empty")]
    pub repo_key: String,
}

/// Pages Shown.text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShowOptions {
    /// Byte offset into the rendered text (snapped back to a rune start).
    pub offset: i64,
    /// Bytes; default DEFAULT_SHOW, max MAX_SHOW.
    pub limit: i64,
    /// Who is asking, as in Query. An item the viewer may not read is
    /// NotFound.
    pub viewer: String,
}

/// Go `CanRead`: the person ("") always, an agent when readers is empty or
/// names it.
pub fn can_read<S: AsRef<str>>(viewer: &str, readers: &[S]) -> bool {
    viewer.is_empty() || readers.is_empty() || readers.iter().any(|r| r.as_ref() == viewer)
}

fn is_zero_i64(n: &i64) -> bool {
    *n == 0
}

/// One item's detail, rendered as text for a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shown {
    #[serde(rename = "ref")]
    pub r#ref: String,
    pub kind: Kind,
    pub title: String,
    /// The page of the rendering, cut on a rune boundary.
    pub text: String,
    /// More text after this page.
    pub more: bool,
    /// The offset for the next page when more.
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub next: i64,
    /// Refs worth showing next, at most MAX_RELATED.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
}

/// A service whose items can be found and shown (Go `Source`). coord
/// implements it for task and comment, inbox for msg; capture implements
/// its own kinds internally and registers the others with `add_source`.
/// Go's `ctx` is dropped: sources answer from their in-memory folds.
pub trait Source: Send + Sync {
    /// The ref kinds this source owns.
    fn search_kinds(&self) -> Vec<Kind>;
    /// Docs whose Updated (or At) is at or after `since`, oldest first, at
    /// most `limit`. The index calls it with the newest Updated it has
    /// seen, so a doc at the boundary comes back again; the index upserts
    /// by ref. A changed doc is returned again with its new Updated.
    fn search_docs(&self, since: GoTime, limit: usize) -> Result<Vec<Doc>, Error>;
    /// Renders one of its refs (NotFound when it has none, or when
    /// `o.viewer` may not read it).
    fn search_show(&self, r: &str, o: &ShowOptions) -> Result<Shown, Error>;
}

/// Go `Page`: cut a rendering for Show. It applies `o`'s defaults and caps,
/// snaps to rune boundaries, and returns (text, more, next).
pub fn page<'a>(full: &'a str, o: &ShowOptions) -> (&'a str, bool, i64) {
    let mut limit = o.limit;
    if limit <= 0 {
        limit = DEFAULT_SHOW;
    }
    let limit = limit.min(MAX_SHOW) as usize;
    let len = full.len();
    let mut off = o.offset.clamp(0, len as i64) as usize;
    while off > 0 && off < len && !full.is_char_boundary(off) {
        off -= 1;
    }
    let mut end = len.min(off + limit);
    while end > off && end < len && !full.is_char_boundary(end) {
        end -= 1;
    }
    if end < len {
        (&full[off..end], true, end as i64)
    } else {
        (&full[off..end], false, 0)
    }
}
