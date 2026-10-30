//! I:internal/capture/why.go: Why (what is behind a path and line),
//! RecordDecision and Decisions (capture.md §1, §2.8).

use std::collections::HashMap;

use coord::decision::{self, AnchorState, Decision, Filter, Listed, Party, Source};
use serde::Serialize;
use sys::cancel::Cancel;

use crate::gopath;
use crate::roots::{dir_exists, norm_root, rel_to};
use crate::rows::*;
use crate::service::{Capture, Fold};
use crate::views::{CommitView, mj, splice};
use crate::{GIT_TIMEOUT, gotime};

/// Most edits and turns in a [`WhyResult`].
pub const MAX_WHY: usize = 10;

/// Why, RecordDecision and Decisions errors, with Go's texts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Go `ErrNoRepo`.
    NoRepo,
    /// A decision rule refused it.
    Decision(decision::Error),
    /// Go `fmt.Errorf("%w: %s", err, id)` for a refused supersede.
    DecisionFor(decision::Error, String),
    /// Anything else (an append failure), with its text.
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoRepo => f.write_str("capture: not inside a known git repo"),
            Error::Decision(e) => write!(f, "{e}"),
            Error::DecisionFor(e, id) => write!(f, "{e}: {id}"),
            Error::Other(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for Error {}

impl From<decision::Error> for Error {
    fn from(e: decision::Error) -> Error {
        Error::Decision(e)
    }
}

/// Asks what is behind a path.
#[derive(Debug, Clone, Default)]
pub struct WhyQuery {
    /// Absolute, or relative to the root / the caller's cwd.
    pub path: String,
    /// Optional, 1-based.
    pub line: i64,
    /// Optional; else from `cwd`.
    pub repo_key: String,
    /// The caller's cwd, to resolve the repo and relative paths.
    pub cwd: String,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// Answers a [`WhyQuery`].
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct WhyResult {
    #[serde(rename = "repoKey")]
    pub repo_key: String,
    pub root: String,
    /// Repo-relative.
    pub path: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub line: i64,
    /// The commit that last changed `line`; "" if uncommitted or no line.
    #[serde(rename = "blameSha", skip_serializing_if = "String::is_empty")]
    pub blame_sha: String,
    /// That commit, if seen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<CommitView>,
    /// Newest first, at most [`MAX_WHY`]; always an array.
    pub edits: Vec<EditRow>,
    /// The turns behind the commit's link or behind the edits.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub turns: Vec<TurnRow>,
    /// Anchored at the path (overlapping the line when given).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<DecisionView>,
}

/// A decision with its anchor re-checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionView {
    pub listed: Listed,
    /// None when unanchored.
    pub status: Option<decision::Status>,
}

/// Listed's fields, then `status`.
impl Serialize for DecisionView {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut extra = Vec::new();
        if let Some(st) = &self.status {
            extra.push(("status", mj::<S::Error>(st)?));
        }
        splice::<S::Error>(&self.listed, extra)?.serialize(s)
    }
}

/// A new decision before it is anchored.
#[derive(Debug, Clone, Default)]
pub struct DecisionInput {
    pub by: Party,
    /// "user" or "untrusted".
    pub trust: String,
    pub statement: String,
    pub why: String,
    /// Optional; absolute or relative to `cwd`.
    pub path: String,
    /// Optional, "40" or "40-52".
    pub lines: String,
    /// The caller's cwd.
    pub cwd: String,
    pub supersedes: Vec<String>,
    pub source: Option<Source>,
}

/// An anchor state's Go string.
pub(crate) fn anchor_state(s: AnchorState) -> &'static str {
    match s {
        AnchorState::Ok => "ok",
        AnchorState::Moved => "moved",
        AnchorState::Changed => "changed",
        AnchorState::Gone => "gone",
        AnchorState::Unknown => "unknown",
    }
}

fn file_exists(p: &str) -> bool {
    std::fs::metadata(p).is_ok_and(|m| !m.is_dir())
}

/// Which root resolver a lookup uses: `repo_for` (logs repo.seen for a new
/// root) on writes, `known_root` on reads.
#[derive(Clone, Copy)]
enum Resolve {
    Write,
    Read,
}

/// Located path: root, repo key, absolute and repo-relative forms.
struct Located {
    root: String,
    rk: String,
    abs: String,
    rel: String,
}

impl Capture {
    fn resolve(&self, how: Resolve, cancel: &Cancel, dir: &str) -> (String, String) {
        match how {
            Resolve::Write => self.inner.repo_for(cancel, dir),
            Resolve::Read => self.known_root(cancel, dir),
        }
    }

    /// `repo_for` for reads: a directory's worktree root and repo key only
    /// when capture has already seen that root, or a live agent works in
    /// it, else ("", ""). It logs nothing and starts no reflog watch.
    pub(crate) fn known_root(&self, cancel: &Cancel, dir: &str) -> (String, String) {
        let inner = &self.inner;
        let root = inner.roots.for_dir(cancel, dir);
        if root.is_empty() {
            return (String::new(), String::new());
        }
        let n = norm_root(&root);
        if let Some(r) = inner.read().repo_roots.get(&n) {
            let rk = r.repo_key.clone();
            return (root, rk);
        }
        for card in inner.live_cards().values() {
            if card.cwd.is_empty() {
                continue;
            }
            let cr = inner.roots.for_dir(cancel, &card.cwd);
            if !cr.is_empty() && norm_root(&cr) == n {
                let rk = (inner.opts.repo_key)(&root);
                if !rk.is_empty() {
                    return (root, rk);
                }
            }
        }
        (String::new(), String::new())
    }

    /// Resolves a path (absolute, or relative to cwd or to repo_key's root)
    /// to its worktree root, repo key, absolute and repo-relative forms.
    fn locate(
        &self,
        how: Resolve,
        cancel: &Cancel,
        path: &str,
        cwd: &str,
        repo_key: &str,
    ) -> Result<Located, Error> {
        let path = crate::jsonx::trim_space(path);
        if path.is_empty() {
            return Err(Error::NoRepo);
        }
        let (root, rk, abs);
        if gopath::is_abs(&gopath::from_slash(path)) {
            abs = gopath::clean(&gopath::from_slash(path));
            (root, rk) = self.resolve(how, cancel, &gopath::dir(&abs));
        } else {
            let mut cwd_root = String::new();
            let (mut r, mut k) = (String::new(), String::new());
            if !cwd.is_empty() {
                (cwd_root, k) = self.resolve(how, cancel, cwd);
                r = cwd_root.clone();
            }
            if !repo_key.is_empty() && k != repo_key {
                r = self.root_of(how, cancel, repo_key);
                k = repo_key.to_string();
            }
            if r.is_empty() {
                return Err(Error::NoRepo);
            }
            // Relative to the caller's cwd when that is in the same worktree
            // and the file is there, else to the root.
            let mut a = gopath::join(&[r.as_str(), &gopath::from_slash(path)]);
            if !cwd_root.is_empty() && norm_root(&cwd_root) == norm_root(&r) {
                let p = gopath::join(&[cwd, &gopath::from_slash(path)]);
                if file_exists(&p) || !file_exists(&a) {
                    a = p;
                }
            }
            (root, rk, abs) = (r, k, a);
        }
        if root.is_empty() {
            return Err(Error::NoRepo);
        }
        let rel = rel_to(&root, &abs).ok_or(Error::NoRepo)?;
        Ok(Located { root, rk, abs, rel })
    }

    /// An existing worktree root for a repo key: a live card's, else any
    /// seen root still on disk.
    fn root_of(&self, how: Resolve, cancel: &Cancel, repo_key: &str) -> String {
        for card in self.inner.live_cards().values() {
            if card.cwd.is_empty() || (!card.repo_key.is_empty() && card.repo_key != repo_key) {
                continue;
            }
            let (root, rk) = self.resolve(how, cancel, &card.cwd);
            if !root.is_empty() && rk == repo_key {
                return root;
            }
        }
        let mut roots: Vec<String> = self
            .inner
            .read()
            .repo_roots
            .values()
            .filter(|r| r.repo_key == repo_key)
            .map(|r| r.root.clone())
            .collect();
        roots.sort();
        roots
            .into_iter()
            .find(|r| dir_exists(r))
            .unwrap_or_default()
    }

    /// The root of a live card in `repo_key`, else "".
    pub(crate) fn live_root_of(&self, repo_key: &str) -> String {
        if repo_key.is_empty() {
            return String::new();
        }
        let cancel = Cancel::new();
        for card in self.inner.live_cards().values() {
            if card.cwd.is_empty() || (!card.repo_key.is_empty() && card.repo_key != repo_key) {
                continue;
            }
            let (root, rk) = self.known_root(&cancel, &card.cwd);
            if !root.is_empty() && rk == repo_key {
                return root;
            }
        }
        String::new()
    }

    /// Re-checks a decision's anchor against its recorded root if that
    /// still exists, else `root` ("" gives Unknown). It reads files, so no
    /// fold lock may be held.
    pub(crate) fn check_decision(&self, d: Listed, root: &str) -> DecisionView {
        let Some(a) = &d.decision.anchor else {
            return DecisionView {
                listed: d,
                status: None,
            };
        };
        let r = if !a.root.is_empty() && dir_exists(&a.root) {
            a.root.as_str()
        } else if !root.is_empty() && dir_exists(root) {
            root
        } else {
            ""
        };
        let st = decision::check(a, r);
        DecisionView {
            listed: d,
            status: Some(st),
        }
    }

    /// What is behind a path (and line): git blame, the commit's link,
    /// recent edits, the turns behind them, and decisions anchored there.
    /// It is a read: a path outside the repos capture has already seen (or
    /// that a live agent works in) is [`Error::NoRepo`], and the query
    /// never logs a repo or starts watching one. Git runs are bounded by
    /// [`GIT_TIMEOUT`].
    pub fn why(&self, q: &WhyQuery) -> Result<WhyResult, Error> {
        let cancel = Cancel::new();
        let loc = self.locate(Resolve::Read, &cancel, &q.path, &q.cwd, &q.repo_key)?;
        let (root, rk, rel) = (loc.root, loc.rk, loc.rel);
        let mut res = WhyResult {
            repo_key: rk.clone(),
            root: root.clone(),
            path: rel.clone(),
            line: q.line,
            ..WhyResult::default()
        };
        let mut uncommitted = true;
        if q.line > 0 {
            let n = q.line.to_string();
            let range = format!("{n},{n}");
            if let Ok(out) = self.inner.opts.git.run(
                &cancel,
                GIT_TIMEOUT,
                &root,
                &["blame", "-L", &range, "--porcelain", "--", &rel],
            ) {
                let t = crate::jsonx::trim_space(&out);
                let sha = t.split(' ').next().unwrap_or("");
                if !sha.is_empty() && !crate::gitlink::zero_sha(sha) {
                    res.blame_sha = sha.to_string();
                    uncommitted = false;
                }
            }
        }

        {
            let f = self.inner.read();
            let mut turn_keys: Vec<String> = Vec::new();
            let mut add_turn = |key: &str, agent: &str, turn: &str| {
                let k = turn_key(key, agent, turn);
                if !turn_keys.contains(&k) {
                    turn_keys.push(k);
                }
            };
            if !res.blame_sha.is_empty() {
                if let Some(rec) = f.commit_map.get(&commit_key(&rk, &res.blame_sha)) {
                    let v = CommitView {
                        row: rec.row.clone(),
                        link: rec.link.clone(),
                    };
                    if let Some(l) = &v.link {
                        for fl in l.files() {
                            if fl.path != rel {
                                continue;
                            }
                            for id in fl.edits.iter().rev() {
                                if res.edits.len() >= MAX_WHY {
                                    break;
                                }
                                if let Some(e) = f.edit(id) {
                                    res.edits.push(e.clone());
                                }
                            }
                        }
                        for e in &res.edits {
                            add_turn(&e.key, &e.agent_id, &e.turn_id);
                        }
                        if res.edits.is_empty() {
                            // No edit of this file: the turns behind the
                            // commit as a whole.
                            for k in &l.sessions {
                                for t in f.turns.get(k).map(Vec::as_slice).unwrap_or(&[]) {
                                    if l.turns.contains(&t.turn.id) {
                                        add_turn(&t.key, &t.turn.agent_id, &t.turn.id);
                                    }
                                }
                            }
                        }
                        if let Some(c) = &l.committer {
                            add_turn(&c.key, "", &c.turn_id);
                        }
                    }
                    res.commit = Some(v);
                }
            } else if uncommitted && q.line > 0 {
                let es: Vec<&EditRow> = f
                    .pool
                    .get(&rk)
                    .map(|p| {
                        p.iter()
                            .filter_map(|id| f.edit(id))
                            .filter(|e| e.path == rel)
                            .collect()
                    })
                    .unwrap_or_default();
                res.edits = newest(es);
                for e in &res.edits {
                    add_turn(&e.key, &e.agent_id, &e.turn_id);
                }
            } else {
                let es: Vec<&EditRow> = f
                    .edit_list
                    .iter()
                    .filter(|e| e.repo_key == rk && e.path == rel)
                    .collect();
                res.edits = newest(es);
                for e in &res.edits {
                    add_turn(&e.key, &e.agent_id, &e.turn_id);
                }
            }
            for tk in &turn_keys {
                if res.turns.len() >= MAX_WHY {
                    break;
                }
                if let Some(t) = find_turn(&f, tk) {
                    res.turns.push(t.clone());
                }
            }
        }
        // Anchor checks read files: done without holding the fold lock.
        let list = self.inner.read().dview.list(&Filter {
            repo_key: rk.clone(),
            path: rel.clone(),
            limit: 500,
            ..Filter::default()
        });
        for d in list {
            if let Some(a) = &d.decision.anchor
                && q.line > 0
                && a.start > 0
                && (q.line < a.start || q.line > a.end)
            {
                continue;
            }
            res.decisions.push(self.check_decision(d, &root));
        }
        Ok(res)
    }

    /// Validates, anchors (against the root of `cwd`'s repo, with HEAD's
    /// sha) and appends a decision. Supersedes are checked with
    /// `View::can_supersede`.
    pub fn record_decision(&self, input: &DecisionInput) -> Result<Decision, Error> {
        let cancel = Cancel::new();
        let mut d = Decision {
            by: input.by.clone(),
            statement: input.statement.clone(),
            why: input.why.clone(),
            supersedes: input.supersedes.clone(),
            source: input.source.clone(),
            trust: if input.trust == "user" {
                "user"
            } else {
                "untrusted"
            }
            .into(),
            ..Decision::default()
        };
        decision::validate(&mut d)?;
        for id in &d.supersedes {
            self.inner
                .read()
                .dview
                .can_supersede(&d.by, id)
                .map_err(|e| Error::DecisionFor(e, id.clone()))?;
        }
        if !crate::jsonx::trim_space(&input.path).is_empty() {
            let (start, end) = decision::parse_lines(&input.lines)?;
            let loc = self.locate(Resolve::Write, &cancel, &input.path, &input.cwd, "")?;
            let head = self
                .inner
                .opts
                .git
                .run(&cancel, GIT_TIMEOUT, &loc.root, &["rev-parse", "HEAD"])
                .unwrap_or_default(); // no commits yet
            let a = decision::make_anchor(
                &loc.root,
                &loc.abs,
                start,
                end,
                &loc.rk,
                crate::jsonx::trim_space(&head),
            )?;
            d.anchor = Some(a);
        }
        let now = self.inner.now().utc();
        let mut rnd = [0u8; 3];
        let _ = getrandom::fill(&mut rnd);
        d.id = format!("d-{}-{}", decision_stamp(&now), hex::encode(rnd));
        d.at = now;
        let actor = if d.by.kind == "user" {
            "user".to_string()
        } else {
            agent_actor(&d.by.key)
        };
        self.inner
            .emit(&actor, KIND_DECISION, &d)
            .map_err(Error::Other)?;
        Ok(d)
    }

    /// Decisions with their anchors re-checked against the recorded root if
    /// it exists, else a live card's root for the repo, else Unknown.
    pub fn decisions(&self, f: &Filter) -> Vec<DecisionView> {
        let list = self.inner.read().dview.list(f);
        // Fallback roots per repo key, resolved once.
        let mut fallback: HashMap<String, String> = HashMap::new();
        for d in &list {
            let Some(a) = &d.decision.anchor else {
                continue;
            };
            if !a.root.is_empty() && dir_exists(&a.root) {
                continue;
            }
            if !fallback.contains_key(&a.repo_key) {
                let r = self.live_root_of(&a.repo_key);
                fallback.insert(a.repo_key.clone(), r);
            }
        }
        list.into_iter()
            .map(|d| {
                let root = d
                    .decision
                    .anchor
                    .as_ref()
                    .and_then(|a| fallback.get(&a.repo_key).cloned())
                    .unwrap_or_default();
                self.check_decision(d, &root)
            })
            .collect()
    }
}

/// Go `t.Format("20060102T150405.000000")` (UTC already).
fn decision_stamp(t: &gocompat::time::GoTime) -> String {
    let (y, mo, d, h, mi, s) = t.civil();
    format!(
        "{y:04}{mo:02}{d:02}T{h:02}{mi:02}{s:02}.{:06}",
        t.nanos / 1000
    )
}

/// Sorts edits newest first and keeps [`MAX_WHY`].
fn newest(mut es: Vec<&EditRow>) -> Vec<EditRow> {
    es.sort_by(|a, b| gotime::cmp(&b.at, &a.at));
    es.into_iter().take(MAX_WHY).cloned().collect()
}

/// A turn by turn key.
fn find_turn<'a>(f: &'a Fold, tk: &str) -> Option<&'a TurnRow> {
    let (key, rest) = tk.split_once('|').unwrap_or((tk, ""));
    let (agent, turn) = rest.split_once('|').unwrap_or((rest, ""));
    f.turns
        .get(key)?
        .iter()
        .find(|t| t.turn.id == turn && (agent.is_empty() || t.turn.agent_id == agent))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_stamp_format() {
        let t = gocompat::time::GoTime::unix(1_793_091_845, 123_456_789);
        assert_eq!(decision_stamp(&t), "20261027T090405.123456");
    }
}
