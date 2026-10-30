//! I:internal/capture/repos.go: reflog passes, commit import and linking.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::time::Instant;

use gocompat::text::equal_fold;
use gocompat::time::GoTime;
use sys::cancel::Cancel;

use crate::gitlink::{self, EditRef, FileLink, LINK_EXACT, ReflogEntry};
use crate::gopath;
use crate::gotime;
use crate::roots::norm_root;
use crate::rows::*;
use crate::service::{CommitRec, Fold, Inner, RunState, lock};
use crate::transcript::HEURISTIC;
use crate::{COMMIT_SLACK, GIT_TIMEOUT, POOL_AGE, RELINK_AGE};

/// One pass reads at most this much of a logs/HEAD.
pub const MAX_REFLOG_READ: i64 = 4 << 20;
/// Repo passes that try a failing reflog line before it is skipped.
pub const MAX_REFLOG_TRIES: u32 = 3;

/// One watched worktree root.
#[derive(Debug, Clone)]
pub(crate) struct ReflogState {
    pub root: String,
    pub repo_key: String,
    pub log_path: String,
    pub offset: i64,
    /// Failed imports of the line at `offset`.
    pub fails: u32,
}

/// Why a reflog import stopped.
enum ImportErr {
    Budget,
    Canceled,
    Failed(String),
}

impl Inner {
    /// Checks every known root's reflog and imports new commits.
    pub(crate) fn repo_pass(&self, rs: &mut RunState, cancel: &Cancel) {
        self.load_cursors(rs);
        rs.repo_deadline = Instant::now() + self.opts.repo_budget;
        self.note_live_roots(cancel);
        let mut roots: Vec<RepoSeen> = self.read().repo_roots.values().cloned().collect();
        roots.sort_by(|a, b| a.root.cmp(&b.root));
        for r in &roots {
            if cancel.is_canceled() {
                return;
            }
            self.watch_root(rs, cancel, r);
            if Instant::now() > rs.repo_deadline {
                break;
            }
        }
        self.save_cursors(rs);
        self.link_unlinked();
    }

    /// Notes the worktree roots live agents work in.
    pub(crate) fn note_live_roots(&self, cancel: &Cancel) {
        for card in self.live_cards().values() {
            if !card.cwd.is_empty() {
                let (root, _) = self.repo_for(cancel, &card.cwd);
                self.use_root(&root, &card.session_id);
            }
        }
    }

    /// Reads a root's new reflog lines.
    pub(crate) fn watch_root(&self, rs: &mut RunState, cancel: &Cancel, r: &RepoSeen) {
        let n = norm_root(&r.root);
        if !rs.reflogs.contains_key(&n) {
            let Ok(gd) = gitlink::git_dir(&r.root) else {
                return; // a removed worktree; it stays listed in Repos
            };
            let mut st = ReflogState {
                root: r.root.clone(),
                repo_key: r.repo_key.clone(),
                log_path: gopath::join(&[&gd, "logs", "HEAD"]),
                offset: 0,
                fails: 0,
            };
            let cf = rs.cursors.as_mut().expect("cursors loaded");
            if let Some(&off) = cf.reflogs.get(&n) {
                st.offset = off;
            } else {
                // First sight: only movements since the root was first seen.
                let first = self.read().first_size.get(&n).copied();
                if let Ok(m) = std::fs::metadata(&st.log_path) {
                    let size = m.len() as i64;
                    st.offset = size;
                    if let Some(first) = first
                        && first <= st.offset
                    {
                        st.offset = first;
                    }
                    // Commits made since the root's oldest captured edit may
                    // already be in the log: start at the first movement
                    // after it (never past the backfill window).
                    if let Some(mut since) = self.oldest_edit_at(&n) {
                        let floor = gotime::sub(&self.now(), self.opts.backfill_window);
                        if gotime::before(&since, &floor) {
                            since = floor;
                        }
                        if let Some(off) = reflog_offset_since(
                            &st.log_path,
                            size,
                            &gotime::truncate_second(&since),
                        ) && off < st.offset
                        {
                            st.offset = off;
                        }
                    }
                }
                cf.set_reflog(&n, st.offset);
            }
            rs.reflogs.insert(n.clone(), st);
            self.write().repos = rs.reflogs.len() as i64;
        }
        let mut st = rs.reflogs.get(&n).cloned().expect("inserted");
        self.read_reflog(rs, cancel, &n, &mut st);
        rs.reflogs.insert(n, st);
    }

    fn read_reflog(&self, rs: &mut RunState, cancel: &Cancel, n: &str, st: &mut ReflogState) {
        let Ok(m) = std::fs::metadata(&st.log_path) else {
            return;
        };
        let size = m.len() as i64;
        if size < st.offset {
            // Rewritten (reflog expire): carry on from its end.
            st.offset = size;
            cursors(rs).set_reflog(n, size);
            return;
        }
        if size == st.offset {
            return;
        }
        let data = match read_at(
            &st.log_path,
            st.offset,
            (size - st.offset).min(MAX_REFLOG_READ),
        ) {
            Ok(d) => d,
            Err(e) => {
                self.set_err(e);
                return;
            }
        };
        let mut branch = String::new();
        let mut rest: &[u8] = &data;
        while let Some(i) = rest.iter().position(|&c| c == b'\n') {
            let line = &rest[..=i];
            rest = &rest[i + 1..];
            let (entries, _) = gitlink::parse_reflog(line);
            let mut line_err: Option<String> = None;
            for e in &entries {
                if branch.is_empty() {
                    branch = self.branch(cancel, &st.root);
                }
                match self.import_entry(rs, cancel, st, e, &branch) {
                    Ok(()) => {}
                    Err(ImportErr::Budget | ImportErr::Canceled) => return,
                    Err(ImportErr::Failed(m)) => {
                        if cancel.is_canceled() {
                            return;
                        }
                        line_err = Some(m);
                    }
                }
            }
            if let Some(m) = line_err {
                self.set_err(&m);
                st.fails += 1;
                let base = gopath::base(&st.root);
                if st.fails < MAX_REFLOG_TRIES {
                    // Keep the offset: the next pass tries the line again.
                    eprintln!("capture: reflog {base}: {m} (will retry)");
                    return;
                }
                eprintln!(
                    "capture: reflog {base}: {m} (skipped after {} tries)",
                    st.fails
                );
            }
            st.fails = 0;
            st.offset += line.len() as i64;
            cursors(rs).set_reflog(n, st.offset);
            // Lines that log nothing still cost a rev-list each.
            if Instant::now() > rs.repo_deadline {
                return;
            }
        }
    }

    /// The root's current branch, "-" when detached or unknown.
    fn branch(&self, cancel: &Cancel, root: &str) -> String {
        match self.opts.git.run(
            cancel,
            GIT_TIMEOUT,
            root,
            &["rev-parse", "--abbrev-ref", "HEAD"],
        ) {
            Ok(out) => {
                let b = crate::jsonx::trim_space(&out);
                if b == "HEAD" {
                    "-".into()
                } else {
                    b.to_string()
                }
            }
            Err(_) => "-".into(),
        }
    }

    /// Logs and links the commits one reflog movement brought in.
    fn import_entry(
        &self,
        rs: &mut RunState,
        cancel: &Cancel,
        st: &ReflogState,
        e: &ReflogEntry,
        branch: &str,
    ) -> Result<(), ImportErr> {
        let seen = |sha: &str| {
            self.read()
                .commit_map
                .contains_key(&commit_key(&st.repo_key, sha))
        };
        let (shas, skipped) =
            gitlink::new_commits(&*self.opts.git, cancel, GIT_TIMEOUT, &st.root, e, &seen)
                .map_err(|e| ImportErr::Failed(e.to_string()))?;
        if skipped > 0 {
            eprintln!(
                "capture: {}: {skipped} commits past the {}-commit walk cap were not imported",
                gopath::base(&st.root),
                gitlink::MAX_WALK
            );
        }
        let branch = if branch == "-" { "" } else { branch };
        let op = e.message.split(':').next().unwrap_or("");
        let mut first_err: Option<String> = None;
        for sha in shas {
            if seen(&sha) {
                continue;
            }
            let mut com =
                match gitlink::describe(&*self.opts.git, cancel, GIT_TIMEOUT, &st.root, &sha) {
                    Ok(c) => c,
                    Err(err) => {
                        if cancel.is_canceled() {
                            return Err(ImportErr::Canceled);
                        }
                        // One unreadable commit doesn't stop the walk.
                        first_err.get_or_insert(err.to_string());
                        continue;
                    }
                };
            com.subject = self.opts.scrub.string(&com.subject);
            let row = CommitRow {
                repo_key: st.repo_key.clone(),
                root: st.root.clone(),
                op: crate::jsonx::trim_space(op).to_string(),
                branch: branch.to_string(),
                commit: com,
            };
            self.emit("irori", KIND_COMMIT_SEEN, &row)
                .map_err(ImportErr::Failed)?;
            self.link_commit(&row).map_err(ImportErr::Failed)?;
            // Checked after a commit is in, so every pass makes progress.
            if Instant::now() > rs.repo_deadline {
                return Err(ImportErr::Budget);
            }
        }
        match first_err {
            Some(e) => Err(ImportErr::Failed(e)),
            None => Ok(()),
        }
    }

    /// The edits a commit can link: the repo's uncommitted edits plus, for a
    /// path whose committed blob an already linked edit produced, that edit
    /// and its link-mates.
    pub(crate) fn candidates(&self, f: &Fold, row: &CommitRow) -> Vec<EditRef> {
        let cut = gotime::sub(&self.now(), POOL_AGE);
        let limit = gotime::add(&row.commit.at, COMMIT_SLACK);
        let usable = |e: &EditRow| {
            !gotime::before(&e.at, &cut)
                && (row.commit.at.is_zero() || !gotime::after(&e.at, &limit))
        };
        let mut out: Vec<EditRef> = Vec::new();
        let mut have = HashSet::new();
        let mut add = |e: &EditRow, out: &mut Vec<EditRef>| {
            if !have.insert(e.id.clone()) {
                return;
            }
            out.push(EditRef {
                id: e.id.clone(),
                root: e.root.clone(),
                path: e.path.clone(),
                after_blob: e.after_blob.clone(),
                after_blob_lf: e.after_blob_lf.clone(),
                heuristic: e.fidelity == HEURISTIC,
                at: e.at,
            });
        };
        if let Some(pool) = f.pool.get(&row.repo_key) {
            for id in pool {
                if let Some(e) = f.edit(id)
                    && usable(e)
                {
                    add(e, &mut out);
                }
            }
        }
        let mut blobs: HashMap<&str, &str> = HashMap::new();
        for fc in row.commit.files.as_deref().unwrap_or(&[]) {
            if !fc.blob.is_empty() {
                blobs.insert(&fc.path, &fc.blob);
            }
        }
        if !blobs.is_empty() {
            for e in &f.edit_list {
                if e.repo_key != row.repo_key
                    || !f.linked.contains(&e.id)
                    || e.fidelity == HEURISTIC
                    || !usable(e)
                {
                    continue;
                }
                let Some(b) = blobs.get(e.path.as_str()) else {
                    continue;
                };
                if !(equal_fold(&e.after_blob, b)
                    || (!e.after_blob_lf.is_empty() && equal_fold(&e.after_blob_lf, b)))
                {
                    continue;
                }
                add(e, &mut out);
                for id in f.linked_with.get(&e.id).map(Vec::as_slice).unwrap_or(&[]) {
                    if let Some(r) = f.edit(id)
                        && usable(r)
                        && r.path == e.path
                    {
                        add(r, &mut out);
                    }
                }
            }
        }
        out.sort_by(|a, b| gotime::cmp(&a.at, &b.at).then_with(|| a.id.cmp(&b.id)));
        out
    }

    /// Classifies a commit's files against the uncommitted edits and logs
    /// commit.linked.
    pub(crate) fn link_commit(&self, row: &CommitRow) -> Result<(), String> {
        let (cands, cmds) = {
            let f = self.read();
            let cands = self.candidates(&f, row);
            let cmds: Vec<_> = f
                .cmds
                .iter()
                .filter(|c| c.spans(&row.commit.at))
                .map(|c| c.commit_ref(&self.opts.home))
                .collect();
            (cands, cmds)
        };
        let mut link = CommitLink {
            repo_key: row.repo_key.clone(),
            sha: row.commit.sha.clone(),
            committer: None,
            files: Some(gitlink::classify(&row.commit, &row.root, &cands)),
            sessions: Vec::new(),
            turns: Vec::new(),
            at: self.now(),
        };
        if let Some(who) = gitlink::committer(&row.commit, &row.root, &cmds) {
            link.committer = Some(Committer {
                key: who.key,
                turn_id: who.turn_id,
            });
        }
        self.fill_sessions(&mut link);
        self.emit("irori", KIND_COMMIT_LINK, &link)
    }

    /// Sets a link's sessions and turns from its edits.
    fn fill_sessions(&self, l: &mut CommitLink) {
        let f = self.read();
        let mut keys: Vec<String> = Vec::new();
        let mut by_key: HashMap<String, Vec<String>> = HashMap::new();
        let mut seen_turn = HashSet::new();
        for fl in l.files() {
            for id in &fl.edits {
                let Some(e) = f.edit(id) else { continue };
                if !by_key.contains_key(&e.key) {
                    keys.push(e.key.clone());
                    by_key.insert(e.key.clone(), Vec::new());
                }
                if seen_turn.insert((e.key.clone(), e.turn_id.clone())) {
                    by_key
                        .get_mut(&e.key)
                        .expect("added")
                        .push(e.turn_id.clone());
                }
            }
        }
        l.turns = keys.iter().flat_map(|k| by_key[k].clone()).collect();
        l.sessions = keys;
    }

    /// Links commits whose commit.seen was logged but whose link never was
    /// (a crash between the two).
    pub(crate) fn link_unlinked(&self) {
        let rows: Vec<CommitRow> = self
            .read()
            .commit_map
            .values()
            .filter(|r| r.link.is_none())
            .map(|r| r.row.clone())
            .collect();
        for row in rows {
            if let Err(e) = self.link_commit(&row) {
                self.set_err(e);
                return;
            }
        }
    }

    /// Re-links recent commits when edits that arrived this pass match a
    /// committed blob their link left non-exact.
    pub(crate) fn relink(&self, rs: &mut RunState, _cancel: &Cancel) {
        let fresh_ids = std::mem::take(&mut rs.new_edits);
        if fresh_ids.is_empty() {
            return;
        }
        let cut = gotime::sub(&self.now(), RELINK_AGE);
        let jobs: Vec<(CommitRow, CommitLink)> = {
            let f = self.read();
            let fresh: Vec<&EditRow> = fresh_ids.iter().filter_map(|id| f.edit(id)).collect();
            let mut jobs = Vec::new();
            for rec in f.commit_map.values() {
                let Some(link) = &rec.link else { continue };
                if gotime::before(&rec.seen_at, &cut) {
                    continue;
                }
                let blobs: HashMap<&str, &str> = rec
                    .row
                    .commit
                    .files
                    .as_deref()
                    .unwrap_or(&[])
                    .iter()
                    .map(|fc| (fc.path.as_str(), fc.blob.as_str()))
                    .collect();
                let hit = link.files().iter().any(|fl| {
                    fl.link != LINK_EXACT
                        && fresh.iter().any(|e| {
                            let b = blobs.get(fl.path.as_str()).copied().unwrap_or("");
                            e.repo_key == rec.row.repo_key
                                && e.path == fl.path
                                && !b.is_empty()
                                && (equal_fold(&e.after_blob, b)
                                    || (!e.after_blob_lf.is_empty()
                                        && equal_fold(&e.after_blob_lf, b)))
                        })
                });
                if hit {
                    jobs.push((rec.row.clone(), link.clone()));
                }
            }
            jobs
        };
        for (row, link) in jobs {
            let next = {
                let f = self.read();
                let cands = self.candidates(&f, &row);
                let fresh = gitlink::classify(&row.commit, &row.root, &cands);
                let mut next = link.clone();
                let mut files: Vec<FileLink> = link.files().to_vec();
                let mut changed = false;
                for (i, cur) in files.iter_mut().enumerate() {
                    let Some(fr) = fresh.get(i) else { continue };
                    if fr.path != cur.path {
                        continue;
                    }
                    if cur.link != LINK_EXACT && fr.link == LINK_EXACT {
                        let mut ids: Vec<String> = cur.edits.clone();
                        ids.extend(fr.edits.iter().cloned());
                        let at = |id: &str| f.edit(id).map(|e| e.at);
                        ids.sort_by(|a, b| match (at(a), at(b)) {
                            (Some(x), Some(y)) if gotime::before(&x, &y) => {
                                std::cmp::Ordering::Less
                            }
                            (Some(x), Some(y)) if gotime::before(&y, &x) => {
                                std::cmp::Ordering::Greater
                            }
                            _ => std::cmp::Ordering::Equal,
                        });
                        ids.dedup();
                        *cur = FileLink {
                            path: cur.path.clone(),
                            link: LINK_EXACT.into(),
                            edits: ids,
                        };
                        changed = true;
                    }
                }
                if !changed {
                    continue;
                }
                next.files = Some(files);
                next
            };
            let mut next = next;
            next.at = self.now();
            self.fill_sessions(&mut next);
            if let Err(e) = self.emit("irori", KIND_COMMIT_LINK, &next) {
                self.set_err(e);
                return;
            }
        }
    }

    /// The time of the oldest captured edit under a root.
    fn oldest_edit_at(&self, n: &str) -> Option<GoTime> {
        let f = self.read();
        let mut oldest: Option<GoTime> = None;
        for e in &f.edit_list {
            if !e.root.is_empty()
                && norm_root(&e.root) == n
                && oldest.is_none_or(|o| gotime::before(&e.at, &o))
            {
                oldest = Some(e.at);
            }
        }
        oldest.filter(|t| !t.is_zero())
    }

    /// Gives a commit command that arrived after its commit was linked the
    /// commits it may have made: linked commits with no committer, under its
    /// dir and inside its span, get a new commit.linked with the committer.
    pub(crate) fn relink_committer(&self, cmd: &CommitCmdRow) -> Result<(), String> {
        let _g = lock(&self.relink_mu);
        let home = &self.opts.home;
        let r = cmd.commit_ref(home);
        let (todo, cmds): (Vec<CommitRec>, Vec<gitlink::CommitCmd>) = {
            let f = self.read();
            let todo: Vec<CommitRec> = f
                .commit_map
                .values()
                .filter(|rec| {
                    rec.link.as_ref().is_some_and(|l| l.committer.is_none())
                        && cmd.spans(&rec.row.commit.at)
                        && gitlink::committer(
                            &rec.row.commit,
                            &rec.row.root,
                            std::slice::from_ref(&r),
                        )
                        .is_some()
                })
                .cloned()
                .collect();
            let mut cmds = Vec::new();
            for rec in &todo {
                for other in &f.cmds {
                    if other.spans(&rec.row.commit.at) {
                        cmds.push(other.commit_ref(home));
                    }
                }
            }
            (todo, cmds)
        };
        let mut done = HashSet::new();
        for rec in todo {
            let l = rec.link.as_ref().expect("filtered");
            let k = commit_key(&l.repo_key, &l.sha);
            if done.contains(&k) {
                continue;
            }
            let Some(who) = gitlink::committer(&rec.row.commit, &rec.row.root, &cmds) else {
                continue;
            };
            let link = {
                let f = self.read();
                let Some(cur) = f.commit_map.get(&k).and_then(|c| c.link.as_ref()) else {
                    continue;
                };
                if cur.committer.is_some() {
                    continue;
                }
                let mut link = cur.clone();
                link.committer = Some(Committer {
                    key: who.key,
                    turn_id: who.turn_id,
                });
                if same_link(&link, cur) {
                    continue;
                }
                link
            };
            done.insert(k);
            let mut link = link;
            link.at = self.now();
            self.emit("irori", KIND_COMMIT_LINK, &link)?;
        }
        Ok(())
    }
}

fn cursors(rs: &mut RunState) -> &mut crate::cursors::CursorFile {
    rs.cursors.as_mut().expect("cursors loaded")
}

/// Two links that say the same thing, `at` aside.
fn same_link(a: &CommitLink, b: &CommitLink) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    a.at = GoTime::ZERO;
    b.at = GoTime::ZERO;
    a == b
}

fn read_at(path: &str, off: i64, n: i64) -> std::io::Result<Vec<u8>> {
    let mut fh = std::fs::File::open(path)?;
    fh.seek(SeekFrom::Start(off as u64))?;
    let mut data = Vec::with_capacity(n as usize);
    fh.take(n as u64).read_to_end(&mut data)?;
    Ok(data)
}

/// The offset of the first logs/HEAD line at or after `since`, looking only
/// at the last [`MAX_REFLOG_READ`] bytes.
pub(crate) fn reflog_offset_since(path: &str, size: i64, since: &GoTime) -> Option<i64> {
    // One byte before the window, so a window starting on a line start
    // isn't taken for a partial line.
    let start = (size - MAX_REFLOG_READ - 1).max(0);
    let data = read_at(path, start, size - start).ok()?;
    let mut off = start;
    let mut rest: &[u8] = &data;
    if start > 0 {
        let i = rest.iter().position(|&c| c == b'\n')?;
        rest = &rest[i + 1..];
        off += (i + 1) as i64;
    }
    while let Some(i) = rest.iter().position(|&c| c == b'\n') {
        let (es, _) = gitlink::parse_reflog(&rest[..=i]);
        if es.len() == 1 && !gotime::before(&es[0].at, since) {
            return Some(off);
        }
        rest = &rest[i + 1..];
        off += (i + 1) as i64;
    }
    None
}
