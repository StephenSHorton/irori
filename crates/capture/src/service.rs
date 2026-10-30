//! I:internal/capture/service.go: the fold over the log, replay, emit,
//! subscriptions and status.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Instant;

use gocompat::sub::Queue;
use gocompat::time::GoTime;
use serde::Serialize;
use store::{Cursor, Event, Filter, MAX_LIST_LIMIT, Store};
use sys::cancel::Cancel;

use crate::cursors::CursorFile;
use crate::find::{
    Sources, commit_doc, decision_doc, edit_doc, merge_owner, pr_doc, sort_owners, turn_doc,
};
use crate::index::Index;
use crate::noise::Dropped;
use crate::repos::ReflogState;
use crate::roots::{RootCache, norm_root};
use crate::rows::*;
use crate::tailer::TFile;
use crate::transcript::Turn;
use crate::{AgentCard, CMD_AGE, Options, Roster, gotime};

/// Subscriber queue capacity; a full queue drops the change.
pub const SUB_BUFFER: usize = 64;

/// What moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Turn,
    Edit,
    Commit,
    Decision,
    Pr,
    /// An in-memory open turn grew (not logged).
    Open,
}

/// Tells the UI to refresh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub kind: ChangeKind,
    /// Agent key.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub key: String,
    #[serde(rename = "repoKey", skip_serializing_if = "String::is_empty")]
    pub repo_key: String,
}

impl Change {
    fn new(kind: ChangeKind, key: &str, repo_key: &str) -> Change {
        Change {
            kind,
            key: key.into(),
            repo_key: repo_key.into(),
        }
    }
}

/// Capture's own health.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    /// Transcript files tracked.
    pub files: i64,
    /// Unread bytes across them.
    pub backlog: i64,
    /// Worktree roots watched.
    pub repos: i64,
    #[serde(rename = "lastScan")]
    pub last_scan: GoTime,
    /// The first pass hasn't finished.
    pub backfilling: bool,
    #[serde(rename = "lastError", skip_serializing_if = "String::is_empty")]
    pub last_error: String,
    /// Heuristic edits discarded as noise since this process started.
    pub dropped: Dropped,
}

/// One commit seen, with its newest link.
#[derive(Debug, Clone)]
pub(crate) struct CommitRec {
    pub row: CommitRow,
    pub link: Option<CommitLink>,
    pub seen_at: GoTime,
    /// Fold order, so ties between equal times sort the same every run.
    pub order: usize,
}

/// The fold: everything replay and emit build. Behind the RwLock.
#[derive(Default)]
pub(crate) struct Fold {
    pub ids: HashSet<String>,
    pub turns: HashMap<String, Vec<TurnRow>>,
    pub edit_list: Vec<EditRow>,
    pub edit_by_id: HashMap<String, usize>,
    pub edits_by_turn: HashMap<String, Vec<usize>>,
    pub pool: HashMap<String, HashSet<String>>,
    pub linked: HashSet<String>,
    pub linked_with: HashMap<String, Vec<String>>,
    pub summaries: HashMap<String, Vec<SummaryRow>>,
    pub cmds: Vec<CommitCmdRow>,
    pub ledger: ledger::Ledger,
    pub repo_roots: HashMap<String, RepoSeen>,
    pub root_users: HashMap<String, HashSet<String>>,
    pub commit_map: HashMap<String, CommitRec>,
    /// The raw decision.recorded payloads, oldest first.
    pub decisions: Vec<Vec<u8>>,
    /// The decisions fold (Go `dview`).
    pub dview: coord::decision::View,
    /// TurnRow.id -> (key, index into `turns[key]`).
    pub turn_by_id: HashMap<String, (String, usize)>,
    /// Ledger key -> every entry, best first (the ledger's view ages out).
    pub pr_all: HashMap<String, Vec<ledger::Entry>>,
    pub open: HashMap<String, Turn>,
    pub first_size: HashMap<String, i64>,
    pub backlog: i64,
    pub files: i64,
    pub repos: i64,
    pub last_scan: GoTime,
    pub backfilling: bool,
    pub last_err: String,
    pub dropped: Dropped,
    pub noise_paths: HashMap<String, HashMap<String, bool>>,
}

impl Fold {
    pub fn edit(&self, id: &str) -> Option<&EditRow> {
        self.edit_by_id.get(id).map(|&i| &self.edit_list[i])
    }

    /// A closed turn by its row id.
    pub fn turn_by_id(&self, id: &str) -> Option<&TurnRow> {
        let (k, i) = self.turn_by_id.get(id)?;
        self.turns.get(k).and_then(|v| v.get(*i))
    }

    /// Upserts one of capture's own docs. An index failure never fails the
    /// fold: it is recorded for Status.
    fn index(&mut self, idx: &Index, d: &search::Doc) {
        if let Err(e) = idx.put(d) {
            self.last_err = format!("capture: search index: {e}");
        }
    }
}

/// What only the run loop touches (Go: owned by the run goroutine).
pub(crate) struct RunState {
    pub cursors: Option<CursorFile>,
    pub files: HashMap<String, TFile>,
    pub reflogs: HashMap<String, ReflogState>,
    pub new_edits: Vec<String>,
    pub first_pass: bool,
    pub repo_deadline: Instant,
}

pub(crate) struct Inner {
    pub st: Arc<Store>,
    pub roster: Option<Arc<dyn Roster>>,
    pub opts: Options,
    pub roots: RootCache,
    pub fold: RwLock<Fold>,
    pub subs: Mutex<Vec<Arc<Queue<Change>>>>,
    pub relink_mu: Mutex<()>,
    pub run: Mutex<RunState>,
    pub ignores: Mutex<HashMap<String, bool>>,
    /// The search index (search.rs); written under the fold's write lock.
    pub idx: Index,
    /// The registered outside sources; the mutex also serializes pulls.
    pub srcs: Mutex<Sources>,
}

/// The capture service. Cloning shares one service; safe for concurrent
/// use.
#[derive(Clone)]
pub struct Capture {
    pub(crate) inner: Arc<Inner>,
}

/// A subscription to [`Change`]s; [`Subscription::cancel`] (or drop) ends
/// it and closes its queue.
pub struct Subscription {
    q: Arc<Queue<Change>>,
    owner: std::sync::Weak<Inner>,
}

impl Subscription {
    /// The next change; `None` once canceled and drained.
    pub fn recv(&self) -> Option<Change> {
        self.q.recv()
    }

    /// The next change within `d`; `Err` on timeout.
    pub fn recv_timeout(
        &self,
        d: std::time::Duration,
    ) -> Result<Option<Change>, sys::fswatch::Timeout> {
        self.q.recv_timeout(d).map_err(|()| sys::fswatch::Timeout)
    }

    /// The queue itself (for async readers: `recv_async`).
    pub fn queue(&self) -> &Queue<Change> {
        &self.q
    }

    pub fn cancel(&self) {
        if let Some(inner) = self.owner.upgrade() {
            lock(&inner.subs).retain(|s| !Arc::ptr_eq(s, &self.q));
        }
        self.q.close();
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Inner {
    pub fn read(&self) -> RwLockReadGuard<'_, Fold> {
        self.fold.read().unwrap_or_else(|p| p.into_inner())
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, Fold> {
        self.fold.write().unwrap_or_else(|p| p.into_inner())
    }

    pub fn now(&self) -> GoTime {
        (self.opts.now)()
    }

    pub fn has(&self, id: &str) -> bool {
        self.read().ids.contains(id)
    }

    pub fn set_err(&self, err: impl std::fmt::Display) {
        self.write().last_err = err.to_string();
    }

    /// Live roster cards by session id ("gone" ones and cards without a
    /// session left out).
    pub fn live_cards(&self) -> HashMap<String, AgentCard> {
        let mut out = HashMap::new();
        if let Some(r) = &self.roster {
            for card in r.agents() {
                if card.status != "gone" && !card.session_id.is_empty() {
                    out.insert(card.session_id.clone(), card);
                }
            }
        }
        out
    }

    /// Appends one row, folds it, then tells subscribers.
    pub fn emit<P: Serialize>(&self, actor: &str, kind: &str, payload: &P) -> Result<(), String> {
        let ev = self
            .st
            .append(actor, kind, payload)
            .map_err(|e| format!("capture: append {kind}: {e}"))?;
        let ch = {
            let mut f = self.write();
            apply(&mut f, &self.idx, &ev)?
        };
        if let Some(ch) = ch {
            self.notify(ch);
        }
        Ok(())
    }

    pub fn notify(&self, ch: Change) {
        for s in lock(&self.subs).iter() {
            s.push(ch.clone());
        }
    }
}

/// Folds one row; returns the change to announce. A payload that doesn't
/// decode is an error (replay stops on it, as Go's does).
pub(crate) fn apply(c: &mut Fold, idx: &Index, ev: &Event) -> Result<Option<Change>, String> {
    let err = |e: gocompat::json::Error| e.to_string();
    match ev.kind.as_str() {
        KIND_TURN => {
            let r = decode_turn_row(&ev.payload).map_err(err)?;
            if c.ids.contains(&r.id) {
                return Ok(None);
            }
            c.ids.insert(r.id.clone());
            let ch = Change::new(ChangeKind::Turn, &r.key, &r.repo_key);
            let doc = turn_doc(&r);
            let list = c.turns.entry(r.key.clone()).or_default();
            c.turn_by_id
                .insert(r.id.clone(), (r.key.clone(), list.len()));
            list.push(r);
            c.index(idx, &doc);
            Ok(Some(ch))
        }
        KIND_EDIT => {
            let r = decode_edit_row(&ev.payload).map_err(err)?;
            if c.edit_by_id.contains_key(&r.id) {
                return Ok(None);
            }
            c.ids.insert(r.id.clone());
            let i = c.edit_list.len();
            c.edit_by_id.insert(r.id.clone(), i);
            c.edits_by_turn
                .entry(turn_key(&r.key, &r.agent_id, &r.turn_id))
                .or_default()
                .push(i);
            if !r.repo_key.is_empty() && !c.linked.contains(&r.id) {
                c.pool
                    .entry(r.repo_key.clone())
                    .or_default()
                    .insert(r.id.clone());
            }
            let ch = Change::new(ChangeKind::Edit, &r.key, &r.repo_key);
            let doc = edit_doc(&r);
            c.edit_list.push(r);
            c.index(idx, &doc);
            Ok(Some(ch))
        }
        KIND_SUMMARY => {
            let r = decode_summary_row(&ev.payload).map_err(err)?;
            if c.ids.contains(&r.id) {
                return Ok(None);
            }
            c.ids.insert(r.id.clone());
            let ch = Change::new(ChangeKind::Turn, &r.key, "");
            c.summaries.entry(r.key.clone()).or_default().push(r);
            Ok(Some(ch))
        }
        KIND_COMMIT_CMD => {
            let r = decode_commit_cmd_row(&ev.payload).map_err(err)?;
            c.ids.insert(cmd_id(&r.key, &r.tool_use_id));
            c.cmds.push(r);
            Ok(None)
        }
        KIND_PR => {
            let e = ledger::decode_entry(&ev.payload).map_err(err)?;
            c.ids.insert(pr_id(&e));
            let ch = Change::new(ChangeKind::Pr, &e.session, &ledger::repo_key(&e.repo));
            if !c.ledger.add(e.clone()) {
                return Ok(None);
            }
            let mut es = c.pr_all.remove(&e.key).unwrap_or_default();
            merge_owner(&mut es, e.clone());
            sort_owners(&mut es);
            let doc = pr_doc(&es);
            c.pr_all.insert(e.key, es);
            c.index(idx, &doc);
            Ok(Some(ch))
        }
        KIND_REPO_SEEN => {
            let r = decode_repo_seen(&ev.payload).map_err(err)?;
            if r.root.is_empty() {
                return Ok(None);
            }
            c.repo_roots.insert(norm_root(&r.root), r);
            Ok(None)
        }
        KIND_COMMIT_SEEN => {
            let r = decode_commit_row(&ev.payload).map_err(err)?;
            let k = commit_key(&r.repo_key, &r.commit.sha);
            if c.commit_map.contains_key(&k) {
                return Ok(None);
            }
            let ch = Change::new(ChangeKind::Commit, "", &r.repo_key);
            let rec = CommitRec {
                row: r,
                link: None,
                seen_at: GoTime::unix_milli(ev.hlc.wall_ms()),
                order: c.commit_map.len(),
            };
            let doc = commit_doc(&rec);
            c.commit_map.insert(k, rec);
            c.index(idx, &doc);
            Ok(Some(ch))
        }
        KIND_COMMIT_LINK => {
            let l = decode_commit_link(&ev.payload).map_err(err)?;
            let k = commit_key(&l.repo_key, &l.sha);
            let n = c.commit_map.len();
            c.commit_map.entry(k).or_insert_with(|| {
                // A link always follows its commit.seen; keep it anyway.
                CommitRec {
                    row: CommitRow {
                        repo_key: l.repo_key.clone(),
                        commit: crate::gitlink::Commit {
                            sha: l.sha.clone(),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    link: None,
                    seen_at: GoTime::unix_milli(ev.hlc.wall_ms()),
                    order: n,
                }
            });
            for f in l.files() {
                for id in &f.edits {
                    c.linked.insert(id.clone());
                    c.linked_with.insert(id.clone(), f.edits.clone());
                    if let Some(p) = c.pool.get_mut(&l.repo_key) {
                        p.remove(id);
                    }
                }
            }
            let ch = Change::new(ChangeKind::Commit, "", &l.repo_key);
            let k = commit_key(&l.repo_key, &l.sha);
            let doc = c.commit_map.get_mut(&k).map(|rec| {
                rec.link = Some(l);
                commit_doc(rec)
            });
            if let Some(doc) = doc {
                c.index(idx, &doc);
            }
            Ok(Some(ch))
        }
        KIND_DECISION => {
            let d = decode_decision_ref(&ev.payload).map_err(err)?;
            // Folded already: capture's own emit races the Core's follow of
            // the same row (Capture::follow_decision).
            if !d.id.is_empty() && c.dview.get(&d.id).is_some() {
                return Ok(None);
            }
            let full = coord::gojson::decode::<coord::decision::Decision>(&ev.payload)
                .ok_or_else(|| "decision: payload does not decode".to_string())?;
            c.decisions.push(ev.payload.clone());
            c.dview.add(full);
            if let Some(got) = c.dview.get(&d.id) {
                c.index(idx, &decision_doc(&got));
            }
            Ok(Some(Change::new(
                ChangeKind::Decision,
                &d.by_key,
                &d.repo_key,
            )))
        }
        _ => Ok(None),
    }
}

impl Capture {
    /// Replays the log (ids already held, uncommitted edits, commits seen
    /// and linked, ledger, decisions, roots seen) and returns a ready
    /// service. Nothing is read from disk until [`Capture::run`] or a pass.
    pub fn new(
        st: Arc<Store>,
        roster: Option<Arc<dyn Roster>>,
        opts: Options,
    ) -> Result<Capture, String> {
        let roots = RootCache::new(opts.git.clone(), opts.now.clone());
        let inner = Inner {
            st,
            roster,
            roots,
            fold: RwLock::new(Fold {
                backfilling: true,
                ..Fold::default()
            }),
            subs: Mutex::new(Vec::new()),
            relink_mu: Mutex::new(()),
            run: Mutex::new(RunState {
                cursors: None,
                files: HashMap::new(),
                reflogs: HashMap::new(),
                new_edits: Vec::new(),
                first_pass: true,
                repo_deadline: Instant::now(),
            }),
            ignores: Mutex::new(HashMap::new()),
            idx: Index::open()?,
            srcs: Mutex::new(Sources::default()),
            opts,
        };
        let c = Capture {
            inner: Arc::new(inner),
        };
        c.replay()?;
        Ok(c)
    }

    /// Folds every capture row, oldest first, paging past the list cap by
    /// the `(hlc, device, seq)` cursor.
    fn replay(&self) -> Result<(), String> {
        let inner = &self.inner;
        let mut f = inner.write();
        inner.idx.begin();
        let res = Self::replay_rows(inner, &mut f);
        if let Err(e) = inner.idx.commit() {
            f.last_err = format!("capture: search index: {e}");
        }
        res
    }

    fn replay_rows(inner: &Inner, f: &mut Fold) -> Result<(), String> {
        let mut after = Cursor::default();
        loop {
            let evs = inner
                .st
                .list(&Filter {
                    kinds: REPLAY_KINDS.iter().map(|s| s.to_string()).collect(),
                    after: after.clone(),
                    limit: MAX_LIST_LIMIT,
                    ..Filter::default()
                })
                .map_err(|e| format!("capture: replay: {e}"))?;
            for ev in &evs {
                apply(f, &inner.idx, ev).map_err(|e| {
                    format!(
                        "capture: replay {} {}/{}: {e}",
                        ev.kind, ev.device_id, ev.seq
                    )
                })?;
                after = ev.cursor();
            }
            if (evs.len() as i64) < MAX_LIST_LIMIT {
                break;
            }
        }
        // Commit commands only matter near a new commit.
        let cut = gotime::sub(&inner.now(), CMD_AGE);
        f.cmds.retain(|cmd| gotime::after(&cmd.at, &cut));
        Ok(())
    }

    /// Changes until the subscription is canceled (buffered; a full buffer
    /// drops).
    pub fn subscribe(&self) -> Subscription {
        let q = Arc::new(Queue::new(SUB_BUFFER));
        lock(&self.inner.subs).push(q.clone());
        Subscription {
            q,
            owner: Arc::downgrade(&self.inner),
        }
    }

    /// Capture's health.
    pub fn status(&self) -> Status {
        let f = self.inner.read();
        Status {
            files: f.files,
            backlog: f.backlog,
            repos: f.repos,
            last_scan: f.last_scan,
            backfilling: f.backfilling,
            last_error: f.last_err.clone(),
            dropped: f.dropped,
        }
    }

    /// The raw decision.recorded payloads replayed or appended, oldest
    /// first (tools; the fold itself is [`Capture::decisions`]).
    pub fn decision_payloads(&self) -> Vec<Vec<u8>> {
        self.inner.read().decisions.clone()
    }

    /// Folds one `decision.recorded` row another device wrote, as the
    /// store delivers it (the Core's follow loop), and tells subscribers.
    /// Go folds only the rows replayed at start and its own; this also
    /// follows the mesh's. A row already folded (this device's own emit)
    /// is skipped; any other kind is ignored.
    pub fn follow_decision(&self, ev: &Event) -> Result<(), String> {
        if ev.kind != KIND_DECISION {
            return Ok(());
        }
        let ch = {
            let mut f = self.inner.write();
            apply(&mut f, &self.inner.idx, ev)?
        };
        if let Some(ch) = ch {
            self.inner.notify(ch);
        }
        Ok(())
    }

    /// After a store resync: the decisions (raw payloads and the view)
    /// rebuilt from the log and swapped in whole, so rows a revoke's cut
    /// moved away leave them. Their search-index rows stay until restart.
    pub fn refold_decisions(&self) -> Result<(), String> {
        let mut rows = Vec::new();
        let mut after = Cursor::default();
        loop {
            let evs = self
                .inner
                .st
                .list(&Filter {
                    kinds: vec![KIND_DECISION.into()],
                    after: after.clone(),
                    limit: MAX_LIST_LIMIT,
                    ..Filter::default()
                })
                .map_err(|e| format!("capture: refold decisions: {e}"))?;
            if let Some(ev) = evs.last() {
                after = ev.cursor();
            }
            let n = evs.len() as i64;
            rows.extend(evs);
            if n < MAX_LIST_LIMIT {
                break;
            }
        }
        let mut f = self.inner.write();
        f.decisions.clear();
        f.dview = coord::decision::View::new();
        for ev in &rows {
            apply(&mut f, &self.inner.idx, ev)?;
        }
        drop(f);
        self.inner.notify(Change::new(ChangeKind::Decision, "", ""));
        Ok(())
    }

    /// Every row id capture holds (turn, edit, summary ids and the
    /// in-memory commit-command and PR identities).
    pub fn has_id(&self, id: &str) -> bool {
        self.inner.has(id)
    }

    /// One tail pass over every transcript (the run loop's, exposed for
    /// tests and tools).
    pub fn tail_pass(&self, cancel: &Cancel) {
        let mut rs = lock(&self.inner.run);
        self.inner.tail_pass(&mut rs, cancel);
    }

    /// One repo pass over every known root's reflog.
    pub fn repo_pass(&self, cancel: &Cancel) {
        let mut rs = lock(&self.inner.run);
        self.inner.repo_pass(&mut rs, cancel);
    }

    /// Tails transcripts and watches repos until `cancel` is canceled.
    pub fn run(&self, cancel: &Cancel) {
        crate::watch::run(&self.inner, cancel);
    }

    /// The fold's normRoot for a root (exposed for the cursor-file tests).
    pub fn norm_root(root: &str) -> String {
        norm_root(root)
    }
}

/// The paths the tailer and the repo pass take, reachable without them
/// (tools and tests).
impl Capture {
    /// Appends one row of a capture kind and folds it, as the tailer and
    /// the repo pass do (`actor` is "irori", "user" or "agent:<key>").
    pub fn emit_row<P: Serialize>(
        &self,
        actor: &str,
        kind: &str,
        payload: &P,
    ) -> Result<(), String> {
        self.inner.emit(actor, kind, payload)
    }

    /// Late committer matching for one commit command (the tailer calls it
    /// when a commit command's result arrives).
    pub fn relink_committer(&self, cmd: &CommitCmdRow) -> Result<(), String> {
        self.inner.relink_committer(cmd)
    }

    /// The open turns held in memory.
    pub fn open_turns(&self) -> usize {
        self.inner.read().open.len()
    }

    /// Test hook: sets (or with `None` drops) the in-memory open turn of
    /// `key` / `agent_id`, as the tailer holds one between a prompt and
    /// its close.
    #[doc(hidden)]
    pub fn set_open_turn(&self, key: &str, agent_id: &str, t: Option<Turn>) {
        let k = open_key(key, agent_id);
        let mut f = self.inner.write();
        match t {
            Some(t) => {
                f.open.insert(k, t);
            }
            None => {
                f.open.remove(&k);
            }
        }
    }

    /// Test hook: replaces a commit's newest link in memory (no row).
    #[doc(hidden)]
    pub fn set_commit_link(&self, repo_key: &str, sha: &str, link: Option<CommitLink>) {
        if let Some(rec) = self
            .inner
            .write()
            .commit_map
            .get_mut(&commit_key(repo_key, sha))
        {
            rec.link = link;
        }
    }
}
