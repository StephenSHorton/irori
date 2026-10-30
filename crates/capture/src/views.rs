//! I:internal/capture/views.go: Timeline, Edits, Commits, Commit, Repos,
//! PRs. View JSON follows I:internal/capture/capture.go:174-214.

use std::collections::BTreeMap;

use gocompat::json::RawJson;
use gocompat::time::GoTime;
use serde::Serialize;
use serde::ser::SerializeStruct;

use crate::gotime;
use crate::noise::strip_noise;
use crate::roots::norm_root;
use crate::rows::*;
use crate::service::{Capture, CommitRec, Fold};

/// A timeline entry's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Turn,
    Summary,
    Commit,
    Pr,
}

/// A turn with its edits.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnView {
    pub row: TurnRow,
    /// The in-memory open turn of a live session.
    pub open: bool,
    pub edits: Vec<EditRow>,
}

/// An embedded struct's JSON with more fields after it: `base` is an
/// object, each extra is a `(name, marshaled value)`. Only meaningful under
/// `gocompat::json::marshal`, which writes a `RawJson` through.
pub(crate) fn splice<E: serde::ser::Error>(
    base: &impl Serialize,
    extra: Vec<(&str, Vec<u8>)>,
) -> Result<RawJson, E> {
    let mut b = gocompat::json::marshal(base).map_err(E::custom)?;
    b.pop(); // the closing '}'
    for (k, v) in extra {
        if b.len() > 1 {
            b.push(b',');
        }
        b.extend_from_slice(gocompat::json::quote_string(k, true).as_bytes());
        b.push(b':');
        b.extend_from_slice(&v);
    }
    b.push(b'}');
    Ok(RawJson(b))
}

pub(crate) fn mj<E: serde::ser::Error>(v: &impl Serialize) -> Result<Vec<u8>, E> {
    gocompat::json::marshal(v).map_err(E::custom)
}

/// TurnRow's fields, then `open` and `edits` (Go embeds the row; its id wins).
impl Serialize for TurnView {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut extra = Vec::new();
        if self.open {
            extra.push(("open", b"true".to_vec()));
        }
        if !self.edits.is_empty() {
            extra.push(("edits", mj::<S::Error>(&self.edits)?));
        }
        splice::<S::Error>(&self.row, extra)?.serialize(s)
    }
}

/// CommitRow's fields, then `link`.
impl Serialize for CommitView {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut extra = Vec::new();
        if let Some(l) = &self.link {
            extra.push(("link", mj::<S::Error>(l)?));
        }
        splice::<S::Error>(&self.row, extra)?.serialize(s)
    }
}

impl Serialize for Entry {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("Entry", 6)?;
        st.serialize_field("kind", &self.kind)?;
        st.serialize_field("at", &self.at)?;
        if let Some(t) = &self.turn {
            st.serialize_field("turn", t)?;
        }
        if let Some(v) = &self.summary {
            st.serialize_field("summary", v)?;
        }
        if let Some(v) = &self.commit {
            st.serialize_field("commit", v)?;
        }
        if let Some(v) = &self.pr {
            st.serialize_field("pr", v)?;
        }
        st.end()
    }
}

/// `{key, entries, more}`; entries is always an array.
impl Serialize for Timeline {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("Timeline", 3)?;
        st.serialize_field("key", &self.key)?;
        st.serialize_field("entries", &self.entries)?;
        st.serialize_field("more", &self.more)?;
        st.end()
    }
}

/// A commit with its newest link.
#[derive(Debug, Clone, PartialEq)]
pub struct CommitView {
    pub row: CommitRow,
    pub link: Option<CommitLink>,
}

/// One timeline line; exactly one field is set.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub kind: EntryKind,
    pub at: GoTime,
    pub turn: Option<TurnView>,
    pub summary: Option<SummaryRow>,
    pub commit: Option<CommitView>,
    pub pr: Option<ledger::Entry>,
}

/// One agent's history, newest first.
#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    pub key: String,
    pub entries: Vec<Entry>,
    /// Older entries exist (page with `before` = the last entry's at).
    pub more: bool,
}

/// Selects a timeline page.
#[derive(Debug, Clone, Default)]
pub struct TimelineQuery {
    /// Agent key; required.
    pub key: String,
    /// Zero: newest.
    pub before: GoTime,
    /// Turns per page; default 20, max 100.
    pub limit: usize,
    /// Include the session's subagent and workflow-agent turns.
    pub subagents: bool,
}

/// Narrows [`Capture::edits`]; empty fields mean any.
#[derive(Debug, Clone, Default)]
pub struct EditQuery {
    pub repo_key: String,
    /// Repo-relative, exact.
    pub path: String,
    pub key: String,
    /// Default 50, max 500.
    pub limit: usize,
}

impl CommitRec {
    fn view(&self) -> CommitView {
        CommitView {
            row: self.row.clone(),
            link: self.link.clone(),
        }
    }
}

fn turn_edits(f: &Fold, key: &str, agent_id: &str, turn_id: &str) -> Vec<EditRow> {
    f.edits_by_turn
        .get(&turn_key(key, agent_id, turn_id))
        .map(|ix| ix.iter().map(|&i| f.edit_list[i].clone()).collect())
        .unwrap_or_default()
}

/// A directory's root and key from what is already known (no git run).
pub(crate) fn cached_repo(f: &Fold, dir: &str) -> (String, String) {
    if dir.is_empty() {
        return (String::new(), String::new());
    }
    let d = norm_root(dir);
    let sep = crate::gopath::SEPARATOR;
    let mut best = RepoSeen::default();
    for (n, r) in &f.repo_roots {
        if (d == *n || d.starts_with(&format!("{n}{sep}"))) && r.root.len() > best.root.len() {
            best = r.clone();
        }
    }
    (best.root, best.repo_key)
}

impl Capture {
    /// A page of one agent's history: the open turn first when the session
    /// is live, closed turns with their edits, summaries, PR ledger
    /// entries, and commits the agent committed or is linked to.
    pub fn timeline(&self, q: &TimelineQuery) -> Timeline {
        let inner = &self.inner;
        let limit = if q.limit == 0 { 20 } else { q.limit.min(100) };
        let mut out = Timeline {
            key: q.key.clone(),
            entries: Vec::new(),
            more: false,
        };
        if q.key.is_empty() {
            return out;
        }
        let live = q.before.is_zero() && inner.live_cards().values().any(|c| c.key == q.key);
        let before = |t: &GoTime| q.before.is_zero() || gotime::before(t, &q.before);
        let now = inner.now();
        let f = inner.read();
        if live {
            let mut opens = Vec::new();
            for (k, t) in &f.open {
                let (key, agent) = k.split_once('|').unwrap_or((k, ""));
                if key != q.key || (!agent.is_empty() && !q.subagents) {
                    continue;
                }
                let (root, rk) = cached_repo(&f, &t.cwd);
                let mut clean = t.clone();
                clean.edit_paths =
                    strip_noise(&f.noise_paths, &q.key, &t.agent_id, &t.id, &t.edit_paths);
                let tv = TurnView {
                    row: TurnRow {
                        id: turn_row_id(&q.key, &t.agent_id, &t.id),
                        key: q.key.clone(),
                        repo_key: rk,
                        root: root.clone(),
                        turn_id: t.id.clone(),
                        turn: inner.scrub_turn(clean, &root),
                    },
                    open: true,
                    edits: turn_edits(&f, &q.key, &t.agent_id, &t.id),
                };
                opens.push(entry_turn(t.started_at, tv));
            }
            opens.sort_by(|a, b| gotime::cmp(&b.at, &a.at));
            out.entries.extend(opens);
        }
        let mut entries = Vec::new();
        for t in f.turns.get(&q.key).map(Vec::as_slice).unwrap_or(&[]) {
            if (!t.turn.agent_id.is_empty() && !q.subagents) || !before(&t.turn.started_at) {
                continue;
            }
            let tv = TurnView {
                row: t.clone(),
                open: false,
                edits: turn_edits(&f, &t.key, &t.turn.agent_id, &t.turn.id),
            };
            entries.push(entry_turn(t.turn.started_at, tv));
        }
        for s in f.summaries.get(&q.key).map(Vec::as_slice).unwrap_or(&[]) {
            if (!s.summary.agent_id.is_empty() && !q.subagents) || !before(&s.summary.at) {
                continue;
            }
            entries.push(Entry {
                kind: EntryKind::Summary,
                at: s.summary.at,
                turn: None,
                summary: Some(s.clone()),
                commit: None,
                pr: None,
            });
        }
        for e in f.ledger.for_session(&q.key, &now) {
            if !before(&e.at) {
                continue;
            }
            entries.push(Entry {
                kind: EntryKind::Pr,
                at: e.at,
                turn: None,
                summary: None,
                commit: None,
                pr: Some(e),
            });
        }
        let mut recs: Vec<&CommitRec> = f.commit_map.values().collect();
        recs.sort_by_key(|r| r.order);
        for rec in recs {
            let Some(l) = &rec.link else { continue };
            if !before(&rec.row.commit.at) {
                continue;
            }
            if l.committer.as_ref().is_some_and(|c| c.key == q.key) || l.sessions.contains(&q.key) {
                entries.push(Entry {
                    kind: EntryKind::Commit,
                    at: rec.row.commit.at,
                    turn: None,
                    summary: None,
                    commit: Some(rec.view()),
                    pr: None,
                });
            }
        }
        entries.sort_by(|a, b| gotime::cmp(&b.at, &a.at));
        let mut turns = 0;
        for e in entries {
            if e.kind == EntryKind::Turn {
                if turns == limit {
                    out.more = true;
                    break;
                }
                turns += 1;
            }
            out.entries.push(e);
        }
        out
    }

    /// Captured edits, newest first.
    pub fn edits(&self, q: &EditQuery) -> Vec<EditRow> {
        let limit = if q.limit == 0 { 50 } else { q.limit.min(500) };
        let path = q.path.replace('\\', "/");
        let f = self.inner.read();
        let mut hits: Vec<&EditRow> = f
            .edit_list
            .iter()
            .filter(|e| {
                (q.repo_key.is_empty() || e.repo_key == q.repo_key)
                    && (path.is_empty() || e.path == path)
                    && (q.key.is_empty() || e.key == q.key)
            })
            .collect();
        hits.sort_by(|a, b| gotime::cmp(&b.at, &a.at));
        hits.truncate(limit);
        hits.into_iter().cloned().collect()
    }

    /// A repo's commits seen ("" = all), newest committer time first, then
    /// newest seen, with their newest link.
    pub fn commits(&self, repo_key: &str, limit: usize) -> Vec<CommitView> {
        let limit = if limit == 0 { 50 } else { limit.min(500) };
        let f = self.inner.read();
        let mut recs: Vec<&CommitRec> = f
            .commit_map
            .values()
            .filter(|r| repo_key.is_empty() || r.row.repo_key == repo_key)
            .collect();
        recs.sort_by(|a, b| {
            gotime::cmp(&b.row.commit.at, &a.row.commit.at)
                .then_with(|| gotime::cmp(&b.seen_at, &a.seen_at))
                .then_with(|| b.order.cmp(&a.order))
        });
        recs.truncate(limit);
        recs.into_iter().map(CommitRec::view).collect()
    }

    /// One commit by sha (any repo); a unique prefix of 7+ hex digits works.
    pub fn commit(&self, sha: &str) -> Option<CommitView> {
        let sha = gocompat::text::to_lower_str(crate::jsonx::trim_space(sha));
        if sha.len() < 7 {
            return None;
        }
        let f = self.inner.read();
        let mut hit: Option<&CommitRec> = None;
        for rec in f.commit_map.values() {
            let s = gocompat::text::to_lower_str(&rec.row.commit.sha);
            if s == sha {
                return Some(rec.view());
            }
            if s.starts_with(&sha) {
                if let Some(h) = hit
                    && !gocompat::text::equal_fold(&h.row.commit.sha, &s)
                {
                    return None; // ambiguous
                }
                hit = Some(rec);
            }
        }
        hit.map(CommitRec::view)
    }

    /// The repo keys seen, with their known roots (sorted).
    pub fn repos(&self) -> BTreeMap<String, Vec<String>> {
        let f = self.inner.read();
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for r in f.repo_roots.values() {
            out.entry(r.repo_key.clone())
                .or_default()
                .push(r.root.clone());
        }
        for roots in out.values_mut() {
            roots.sort();
        }
        out
    }

    /// A stored patch or summary.
    pub fn blob(&self, id: &str) -> Result<Vec<u8>, String> {
        self.inner.opts.blobs.get(id)
    }

    /// The agent keys owning a PR (ledger key), best first.
    pub fn pr_owners(&self, pr_key: &str) -> Vec<String> {
        let now = self.inner.now();
        let mut out: Vec<String> = Vec::new();
        for e in self.inner.read().ledger.owners(pr_key, &now) {
            if !out.contains(&e.session) {
                out.push(e.session);
            }
        }
        out
    }

    /// Ledger entries for a repo ("owner/name"; "" = all).
    pub fn prs(&self, repo: &str) -> Vec<ledger::Entry> {
        let now = self.inner.now();
        self.inner.read().ledger.for_repo(repo, &now)
    }

    /// Every turn row folded for an agent key, in log order (for replay
    /// comparisons and the search port).
    pub fn turn_rows(&self, key: &str) -> Vec<TurnRow> {
        self.inner
            .read()
            .turns
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    /// Every edit row folded, in log order.
    pub fn edit_rows(&self) -> Vec<EditRow> {
        self.inner.read().edit_list.clone()
    }

    /// Every summary row folded for an agent key, in log order.
    pub fn summary_rows(&self, key: &str) -> Vec<SummaryRow> {
        self.inner
            .read()
            .summaries
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    /// The commit commands kept for committer matching.
    pub fn commit_cmds(&self) -> Vec<CommitCmdRow> {
        self.inner.read().cmds.clone()
    }

    /// The uncommitted edit ids of a repo (the link candidates' pool).
    pub fn pool(&self, repo_key: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .inner
            .read()
            .pool
            .get(repo_key)
            .map(|p| p.iter().cloned().collect())
            .unwrap_or_default();
        v.sort();
        v
    }
}

fn entry_turn(at: GoTime, tv: TurnView) -> Entry {
    Entry {
        kind: EntryKind::Turn,
        at,
        turn: Some(tv),
        summary: None,
        commit: None,
        pr: None,
    }
}
