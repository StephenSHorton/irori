//! I:internal/capture/rows.go: parser items into log rows.

use sys::cancel::Cancel;

use crate::gitlink;
use crate::gitsha;
use crate::gopath;
use crate::noise::strip_noise;
use crate::roots::{norm_root, rel_to};
use crate::rows::*;
use crate::service::{Inner, RunState};
use crate::tailer::TFile;
use crate::transcript::{BashRun, Edit, Item, PrLink, Summary, Turn};

/// CommitCmdRow.command's cap, in runes.
pub const MAX_COMMAND: usize = 300;

/// At most `n` runes, ending in "…" when it cut (no whitespace backoff).
pub fn clip_runes(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n - 1).collect();
    out.push('…');
    out
}

/// The first non-blank line of `s`, trimmed.
pub(crate) fn first_line(s: &str) -> String {
    let s = crate::jsonx::trim_space(s);
    let s = s.split('\n').next().unwrap_or("");
    crate::jsonx::trim_space(s).to_string()
}

impl Inner {
    /// Turns parser items into rows; stops at the first failed append.
    pub(crate) fn handle(
        &self,
        rs: &mut RunState,
        cancel: &Cancel,
        f: &TFile,
        items: &[Item],
    ) -> Result<(), String> {
        let drop = self.drop_noise(cancel, items);
        for (i, it) in items.iter().enumerate() {
            match it {
                Item::Turn(t) => self.on_turn(cancel, f, t)?,
                Item::Edit(e) if drop.contains(&i) => self.note_path(&f.key, e, true),
                Item::Edit(e) => {
                    self.on_edit(rs, cancel, f, e)?;
                    self.note_path(&f.key, e, false);
                }
                Item::Summary(s) => self.on_summary(f, s)?,
                Item::Bash(b) => self.on_bash(cancel, f, b)?,
                Item::PrLink(l) => self.on_pr_link(f, l)?,
            }
        }
        Ok(())
    }

    /// A directory's worktree root and repo key, logging repo.seen the
    /// first time a root shows up.
    pub(crate) fn repo_for(&self, cancel: &Cancel, dir: &str) -> (String, String) {
        let root = self.roots.for_dir(cancel, dir);
        if root.is_empty() {
            return (String::new(), String::new());
        }
        let rk = self.note_root(&root);
        (root, rk)
    }

    /// A root's repo key, logging repo.seen for a new root.
    pub(crate) fn note_root(&self, root: &str) -> String {
        let n = norm_root(root);
        if let Some(r) = self.read().repo_roots.get(&n) {
            return r.repo_key.clone();
        }
        let rk = (self.opts.repo_key)(root);
        if rk.is_empty() {
            return String::new();
        }
        self.snapshot_reflog(root);
        let row = RepoSeen {
            repo_key: rk.clone(),
            root: root.to_string(),
            at: self.now(),
        };
        if let Err(e) = self.emit("irori", KIND_REPO_SEEN, &row) {
            self.set_err(e);
        }
        rk
    }

    /// Records a new root's logs/HEAD size, so its first reflog watch
    /// starts from when the root was seen.
    fn snapshot_reflog(&self, root: &str) {
        let Ok(gd) = gitlink::git_dir(root) else {
            return;
        };
        let size = match std::fs::metadata(gopath::join(&[&gd, "logs", "HEAD"])) {
            Ok(m) => m.len() as i64,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(_) => return,
        };
        self.write()
            .first_size
            .entry(norm_root(root))
            .or_insert(size);
    }

    /// Notes that session `sid` works in `root` (keeps it hot while live).
    pub(crate) fn use_root(&self, root: &str, sid: &str) {
        if root.is_empty() || sid.is_empty() {
            return;
        }
        self.write()
            .root_users
            .entry(norm_root(root))
            .or_default()
            .insert(sid.to_string());
    }

    /// Scrubs a turn's text and makes its edit paths repo-relative.
    pub(crate) fn scrub_turn(&self, mut t: Turn, root: &str) -> Turn {
        let s = &self.opts.scrub;
        t.prompt = s.string(&t.prompt);
        t.reply = s.string(&t.reply);
        for tc in &mut t.tools {
            tc.target = s.string(&tc.target);
        }
        t.edit_paths = t
            .edit_paths
            .iter()
            .map(|p| rel_to(root, p).unwrap_or_else(|| p.clone()))
            .collect();
        t
    }

    fn on_turn(&self, cancel: &Cancel, f: &TFile, t: &Turn) -> Result<(), String> {
        let id = turn_row_id(&f.key, &t.agent_id, &t.id);
        let tk = turn_key(&f.key, &t.agent_id, &t.id);
        if self.has(&id) {
            self.write().noise_paths.remove(&tk);
            return Ok(());
        }
        let (root, rk) = self.repo_for(cancel, &t.cwd);
        self.use_root(&root, &f.src.session_id);
        let mut clean = t.clone();
        clean.edit_paths = strip_noise(
            &self.read().noise_paths,
            &f.key,
            &t.agent_id,
            &t.id,
            &t.edit_paths,
        );
        let row = TurnRow {
            id,
            key: f.key.clone(),
            repo_key: rk,
            root: root.clone(),
            turn_id: t.id.clone(),
            turn: self.scrub_turn(clean, &root),
        };
        self.emit(&agent_actor(&f.key), KIND_TURN, &row)?;
        self.write().noise_paths.remove(&tk);
        Ok(())
    }

    fn on_edit(
        &self,
        rs: &mut RunState,
        cancel: &Cancel,
        f: &TFile,
        e: &Edit,
    ) -> Result<(), String> {
        let id = edit_row_id(&f.key, &e.tool_use_id, &e.path);
        if self.has(&id) {
            return Ok(());
        }
        let abs = crate::noise::edit_abs(e);
        let mut row = EditRow {
            id: id.clone(),
            key: f.key.clone(),
            agent_id: e.agent_id.clone(),
            turn_id: e.turn_id.clone(),
            tool_use_id: e.tool_use_id.clone(),
            tool: e.tool.clone(),
            path: abs.clone(),
            create: e.create,
            adds: e.adds,
            dels: e.dels,
            fidelity: e.fidelity.clone(),
            at: e.at,
            ..EditRow::default()
        };
        let root = self.roots.for_file(cancel, &abs);
        if !root.is_empty()
            && let Some(rel) = rel_to(&root, &abs)
        {
            row.root = root.clone();
            row.path = rel;
            row.repo_key = self.note_root(&root);
            self.use_root(&root, &f.src.session_id);
        }
        if let Some(b) = &e.before {
            row.before_blob = gitsha::blob(b);
        }
        if let Some(a) = &e.after {
            (row.after_blob, row.after_blob_lf) = gitsha::pair(a);
        }
        if !e.patch.is_empty() && !self.opts.scrub.skip_file(&abs) {
            row.patch = self
                .opts
                .blobs
                .put(self.opts.scrub.string(&e.patch).as_bytes())?;
        }
        self.emit(&agent_actor(&f.key), KIND_EDIT, &row)?;
        if !row.repo_key.is_empty() && self.read().edit_by_id.contains_key(&id) {
            rs.new_edits.push(id);
        }
        Ok(())
    }

    fn on_summary(&self, f: &TFile, s: &Summary) -> Result<(), String> {
        let text = self.opts.scrub.string(&s.text);
        let id_part = if s.row_uuid.is_empty() {
            text.clone()
        } else {
            s.row_uuid.clone()
        };
        let id = summary_row_id(&f.key, &s.agent_id, &s.kind, &id_part);
        if self.has(&id) {
            return Ok(());
        }
        let mut row = SummaryRow {
            id,
            key: f.key.clone(),
            summary: s.clone(),
            full_blob: String::new(),
        };
        row.summary.text = text;
        row.summary.full = String::new();
        if row.summary.at.is_zero() {
            // Title rows carry no time: stamp them with the file's activity.
            row.summary.at = f.parser.last_activity();
            if row.summary.at.is_zero() {
                row.summary.at = self.now();
            }
        }
        if !s.full.is_empty() {
            row.full_blob = self
                .opts
                .blobs
                .put(self.opts.scrub.string(&s.full).as_bytes())?;
        }
        self.emit(&agent_actor(&f.key), KIND_SUMMARY, &row)
    }

    fn on_bash(&self, cancel: &Cancel, f: &TFile, b: &BashRun) -> Result<(), String> {
        if gitlink::is_commit_command(&b.command) {
            let id = cmd_id(&f.key, &b.tool_use_id);
            if !self.has(&id) {
                let row = CommitCmdRow {
                    key: f.key.clone(),
                    agent_id: b.agent_id.clone(),
                    turn_id: b.turn_id.clone(),
                    tool_use_id: b.tool_use_id.clone(),
                    cwd: b.cwd.clone(),
                    command: clip_runes(
                        &self.opts.scrub.string(&first_line(&b.command)),
                        MAX_COMMAND,
                    ),
                    at: b.at,
                    started_at: b.started_at,
                    dir: crate::cmddir::command_dir(&b.command, &b.cwd, &self.opts.home),
                };
                self.emit(&agent_actor(&f.key), KIND_COMMIT_CMD, &row)?;
                self.relink_committer(&row)?;
            }
        }
        if b.git_operation.is_empty() && !b.command.contains("gh") {
            return Ok(()); // no PR relation possible
        }
        let (broot, rk) = self.repo_for(cancel, &b.cwd);
        self.use_root(&broot, &f.src.session_id);
        let refs = ledger::from_bash(&ledger::BashResult {
            command: b.command.clone(),
            stdout: b.stdout.clone(),
            git_operation: b.git_operation.clone(),
            default_repo: ledger::repo_from_key(&rk).to_string(),
        });
        for r in refs {
            let mut e = ledger::Entry {
                key: ledger::key(&r.r#ref.repo, r.r#ref.number),
                repo: r.r#ref.repo.clone(),
                number: r.r#ref.number,
                url: r.url.clone(),
                session: f.key.clone(),
                role: r.role.clone(),
                source: r.source.clone(),
                at: b.at,
            };
            if e.at.is_zero() {
                e.at = self.now();
            }
            if self.has(&pr_id(&e)) {
                continue;
            }
            self.emit(&agent_actor(&f.key), KIND_PR, &e)?;
        }
        Ok(())
    }

    fn on_pr_link(&self, f: &TFile, l: &PrLink) -> Result<(), String> {
        if l.repo.is_empty() || l.number <= 0 {
            return Ok(());
        }
        let mut e = ledger::Entry {
            key: ledger::key(&l.repo, l.number),
            repo: l.repo.clone(),
            number: l.number,
            url: l.url.clone(),
            session: f.key.clone(),
            role: ledger::Role::TOUCHED,
            source: "pr-link".into(),
            at: l.at,
        };
        if e.at.is_zero() {
            e.at = self.now();
        }
        if self.has(&pr_id(&e)) {
            return Ok(());
        }
        self.emit("irori", KIND_PR, &e)
    }
}
