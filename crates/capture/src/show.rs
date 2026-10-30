//! I:internal/capture/show.go: Show renderings of capture's own refs
//! (turn, edit, commit, pr, decision, agent). Agents read this text, so
//! every line is Go's byte for byte (capture.md §2.8).

use std::collections::HashMap;
use std::fmt::Write as _;

use gocompat::time::GoTime;
use search::{Error, Kind, MAX_RELATED, MAX_TITLE, ShowOptions, Shown};

use crate::find::{edit_title, hide_messages};
use crate::handle::{clip_runes, first_line};
use crate::index::one_line;
use crate::overview::{AgentLine, OVERVIEW_LINE_MAX, card_name};
use crate::roots::dir_exists;
use crate::rows::*;
use crate::service::{Capture, CommitRec, Fold};
use crate::views::{TimelineQuery, cached_repo};
use crate::why::anchor_state;

/// One ref drawn in full, before paging.
pub(crate) struct Rendering {
    r#ref: String,
    kind: Kind,
    title: String,
    text: String,
    related: Vec<String>,
}

impl Rendering {
    fn new(r: String, kind: Kind, title: String) -> Rendering {
        Rendering {
            r#ref: r,
            kind,
            title,
            text: String::new(),
            related: Vec::new(),
        }
    }

    fn line(&mut self, s: &str) {
        self.text.push_str(s);
        self.text.push('\n');
    }

    /// A labelled multi-line section, skipped when empty.
    fn block(&mut self, label: &str, body: &str) {
        let body = crate::jsonx::trim_space(body);
        if body.is_empty() {
            return;
        }
        let _ = write!(self.text, "\n{label}:\n{body}\n");
    }

    /// Adds a ref once, up to MAX_RELATED.
    fn relate(&mut self, r: &str) {
        if r.is_empty()
            || r == self.r#ref
            || self.related.iter().any(|x| x == r)
            || self.related.len() >= MAX_RELATED
        {
            return;
        }
        self.related.push(r.to_string());
    }

    pub(crate) fn page(self, o: &ShowOptions) -> Shown {
        let (text, more, next) = search::page(&self.text, o);
        Shown {
            r#ref: self.r#ref,
            kind: self.kind,
            title: clip_runes(&one_line(&self.title), MAX_TITLE),
            text: text.to_string(),
            more,
            next,
            related: self.related,
        }
    }
}

/// UTC RFC3339 seconds, or `-` for the zero time.
pub(crate) fn stamp(t: &GoTime) -> String {
    if t.is_zero() {
        return "-".into();
    }
    t.utc().format_rfc3339()
}

pub(crate) fn turn_ref(id: &str) -> String {
    search::make_ref(Kind::Turn, id)
}
pub(crate) fn edit_ref(id: &str) -> String {
    search::make_ref(Kind::Edit, id)
}
pub(crate) fn commit_ref(sha: &str) -> String {
    search::make_ref(Kind::Commit, &gocompat::text::to_lower_str(sha))
}
pub(crate) fn agent_ref(key: &str) -> String {
    search::make_ref(Kind::Agent, key)
}

/// The turn row of (key, transcript turn id), any agent.
pub(crate) fn turn_of<'a>(
    f: &'a Fold,
    key: &str,
    agent_id: &str,
    turn_id: &str,
) -> Option<&'a TurnRow> {
    if let Some(t) = f.turn_by_id(&turn_row_id(key, agent_id, turn_id)) {
        return Some(t);
    }
    f.turns.get(key)?.iter().find(|t| t.turn.id == turn_id)
}

/// The commits whose newest link names any of the edit ids, or whose
/// committer is (key, turn_id) when key is set, newest first.
fn commits_taking<'a>(
    f: &'a Fold,
    edits: &[String],
    key: &str,
    turn_id: &str,
) -> Vec<&'a CommitRec> {
    let mut out: Vec<&CommitRec> = f
        .commit_map
        .values()
        .filter(|rec| {
            let Some(l) = &rec.link else { return false };
            let by = !key.is_empty()
                && l.committer
                    .as_ref()
                    .is_some_and(|c| c.key == key && c.turn_id == turn_id);
            by || l
                .files()
                .iter()
                .any(|fl| fl.edits.iter().any(|id| edits.contains(id)))
        })
        .collect();
    out.sort_by(|a, b| {
        crate::gotime::cmp(&b.row.commit.at, &a.row.commit.at)
            .then_with(|| a.row.commit.sha.cmp(&b.row.commit.sha))
    });
    out
}

/// Go `strconv.Atoi` on digits: the value, clamped on overflow (its error
/// is ignored by the caller, as Go's is).
fn atoi(s: &str) -> i64 {
    s.parse::<i64>()
        .unwrap_or(if s.is_empty() { 0 } else { i64::MAX })
}

/// The ledger key of a pr ref's id ("owner/name#n").
fn pr_key(id: &str) -> String {
    let (repo, n) = id.split_once('#').unwrap_or((id, ""));
    ledger::key(repo, atoi(n))
}

impl Capture {
    /// Draws one of capture's own refs for `viewer` ("" is the person).
    pub(crate) fn render(&self, kind: Kind, id: &str, viewer: &str) -> Result<Rendering, Error> {
        match kind {
            Kind::Turn => self.render_turn(id, viewer),
            Kind::Edit => self.render_edit(id),
            Kind::Commit => self.render_commit(id),
            Kind::Pr => self.render_pr(id),
            Kind::Decision => self.render_decision(id),
            Kind::Agent => self.render_agent(id, viewer),
            _ => Err(Error::NotFound),
        }
    }

    /// A turn by its row id: a closed one, else a live session's open turn.
    fn shown_turn(&self, f: &Fold, id: &str) -> Option<TurnRow> {
        if let Some(t) = f.turn_by_id(id) {
            return Some(t.clone());
        }
        for (k, t) in &f.open {
            let key = k.split_once('|').map_or(k.as_str(), |(a, _)| a);
            if turn_row_id(key, &t.agent_id, &t.id) == id {
                let (root, rk) = cached_repo(f, &t.cwd);
                return Some(TurnRow {
                    id: id.to_string(),
                    key: key.to_string(),
                    repo_key: rk,
                    turn_id: t.id.clone(),
                    turn: self.inner.scrub_turn(t.clone(), &root),
                    root,
                });
            }
        }
        None
    }

    fn render_turn(&self, id: &str, viewer: &str) -> Result<Rendering, Error> {
        let f = self.inner.read();
        let mut t = self.shown_turn(&f, id).ok_or(Error::NotFound)?;
        t.turn.prompt = hide_messages(&t.turn.prompt, viewer, &t.key);
        let mut r = Rendering::new(turn_ref(id), Kind::Turn, first_line(&t.turn.prompt));
        let mut who = t.key.clone();
        if !t.turn.agent_id.is_empty() {
            who += &format!(" (subagent {})", t.turn.agent_id);
        }
        r.line(&format!("turn {} by {who}", r.r#ref));
        if !t.repo_key.is_empty() || !t.turn.branch.is_empty() {
            r.line(&format!("repo: {}  branch: {}", t.repo_key, t.turn.branch));
        }
        let state = if t.turn.ended_at.is_zero() {
            "open".to_string()
        } else if t.turn.interrupted {
            format!("ended {} (interrupted)", stamp(&t.turn.ended_at))
        } else {
            format!("ended {}", stamp(&t.turn.ended_at))
        };
        r.line(&format!(
            "started {}, {state}; origin {}",
            stamp(&t.turn.started_at),
            t.turn.origin
        ));
        r.block("prompt", &t.turn.prompt);
        r.block("reply", &t.turn.reply);
        if !t.turn.tools.is_empty() {
            let mut b = String::new();
            for tc in &t.turn.tools {
                b.push_str("- ");
                b.push_str(&tc.name);
                if !tc.target.is_empty() {
                    b.push(' ');
                    b.push_str(&one_line(&tc.target));
                }
                if tc.is_error {
                    b.push_str(" (error)");
                }
                b.push('\n');
            }
            if t.turn.tools_more > 0 {
                let _ = writeln!(b, "... {} more tool calls", t.turn.tools_more);
            }
            r.block("tools", &b);
        }
        let edits: Vec<&EditRow> = f
            .edits_by_turn
            .get(&turn_key(&t.key, &t.turn.agent_id, &t.turn.id))
            .map(|ix| ix.iter().map(|&i| &f.edit_list[i]).collect())
            .unwrap_or_default();
        let mut ids = Vec::new();
        if !edits.is_empty() {
            let mut b = String::new();
            for e in &edits {
                ids.push(e.id.clone());
                let _ = writeln!(b, "- {} {}", edit_ref(&e.id), edit_title(e));
            }
            r.block("edits", &b);
        }
        let committer = if t.turn.agent_id.is_empty() {
            t.key.as_str()
        } else {
            ""
        };
        let commits = commits_taking(&f, &ids, committer, &t.turn.id);
        if !commits.is_empty() {
            let mut b = String::new();
            for rec in &commits {
                let _ = writeln!(
                    b,
                    "- {} {}",
                    commit_ref(&rec.row.commit.sha),
                    one_line(&rec.row.commit.subject)
                );
            }
            r.block("commits", &b);
        }
        for e in &edits {
            r.relate(&edit_ref(&e.id));
        }
        for rec in &commits {
            r.relate(&commit_ref(&rec.row.commit.sha));
        }
        r.relate(&agent_ref(&t.key));
        Ok(r)
    }

    fn render_edit(&self, id: &str) -> Result<Rendering, Error> {
        let (row, tref, crefs) = {
            let f = self.inner.read();
            let row = f.edit(id).ok_or(Error::NotFound)?.clone();
            let tref = turn_of(&f, &row.key, &row.agent_id, &row.turn_id)
                .map(|t| turn_ref(&t.id))
                .unwrap_or_default();
            let crefs: Vec<String> = commits_taking(&f, &[id.to_string()], "", "")
                .iter()
                .map(|rec| commit_ref(&rec.row.commit.sha))
                .collect();
            (row, tref, crefs)
        };
        let mut r = Rendering::new(edit_ref(id), Kind::Edit, edit_title(&row));
        r.line(&format!("edit {} by {}", r.r#ref, row.key));
        r.line(&edit_title(&row));
        if !row.repo_key.is_empty() {
            r.line(&format!("repo: {}", row.repo_key));
        }
        let created = if row.create { ", created the file" } else { "" };
        r.line(&format!(
            "at {}, fidelity {}{created}",
            stamp(&row.at),
            row.fidelity
        ));
        if !tref.is_empty() {
            r.line(&format!("turn: {tref}"));
        }
        for cr in &crefs {
            r.line(&format!("committed in: {cr}"));
        }
        if row.patch.is_empty() {
            r.block("patch", "(no patch stored)");
        } else {
            match self.inner.opts.blobs.get(&row.patch) {
                Ok(b) => r.block("patch", &String::from_utf8_lossy(&b)),
                Err(e) => r.block("patch", &format!("(patch unreadable: {e})")),
            }
        }
        r.relate(&tref);
        for cr in &crefs {
            r.relate(cr);
        }
        Ok(r)
    }

    fn render_commit(&self, sha: &str) -> Result<Rendering, Error> {
        let v = self.commit(sha).ok_or(Error::NotFound)?;
        let c = &v.row.commit;
        let mut r = Rendering::new(commit_ref(&c.sha), Kind::Commit, c.subject.clone());
        r.line(&format!("commit {} in {}", c.sha, v.row.repo_key));
        r.line(&format!("subject: {}", one_line(&c.subject)));
        r.line(&format!("author: {}, at {}", c.author, stamp(&c.at)));
        if !v.row.branch.is_empty() || !v.row.op.is_empty() {
            r.line(&format!("branch: {}  op: {}", v.row.branch, v.row.op));
        }
        let mut links: HashMap<&str, &str> = HashMap::new();
        if let Some(l) = &v.link {
            for fl in l.files() {
                links.insert(&fl.path, &fl.link);
            }
            if let Some(cm) = &l.committer {
                r.line(&format!("committed by: {}", cm.key));
            }
        }
        let files = c.files.as_deref().unwrap_or(&[]);
        if !files.is_empty() {
            let mut b = String::new();
            for fc in files {
                let _ = write!(b, "- {} {}", fc.status, fc.path);
                if let Some(l) = links.get(fc.path.as_str())
                    && !l.is_empty()
                {
                    let _ = write!(b, " ({l})");
                }
                b.push('\n');
            }
            if c.files_omitted > 0 {
                let _ = writeln!(b, "... {} more files", c.files_omitted);
            }
            r.block("files", &b);
        }
        let Some(l) = &v.link else {
            r.line("\nnot linked to any agent's edits yet");
            return Ok(r);
        };
        let f = self.inner.read();
        if let Some(cm) = &l.committer
            && let Some(t) = turn_of(&f, &cm.key, "", &cm.turn_id)
        {
            r.relate(&turn_ref(&t.id));
        }
        for fl in l.files() {
            for id in &fl.edits {
                if let Some(e) = f.edit(id)
                    && let Some(t) = turn_of(&f, &e.key, &e.agent_id, &e.turn_id)
                {
                    r.relate(&turn_ref(&t.id));
                }
            }
        }
        if !r.related.is_empty() {
            let body = format!("- {}", r.related.join("\n- "));
            r.block("turns", &body);
        }
        Ok(r)
    }

    fn render_pr(&self, id: &str) -> Result<Rendering, Error> {
        let key = pr_key(id);
        let es = self
            .inner
            .read()
            .pr_all
            .get(&key)
            .cloned()
            .unwrap_or_default();
        let Some(best) = es.first() else {
            return Err(Error::NotFound);
        };
        let mut r = Rendering::new(
            key.clone(),
            Kind::Pr,
            format!("PR {}#{}", best.repo, best.number),
        );
        r.line(&format!("PR {}#{}", best.repo, best.number));
        if let Some(e) = es.iter().find(|e| !e.url.is_empty()) {
            r.line(&format!("url: {}", e.url));
        }
        let mut b = String::new();
        for e in &es {
            let _ = writeln!(
                b,
                "- {} {} via {} at {}",
                agent_ref(&e.session),
                e.role.as_str(),
                e.source,
                stamp(&e.at)
            );
            r.relate(&agent_ref(&e.session));
        }
        r.block("who", &b);
        Ok(r)
    }

    fn render_decision(&self, id: &str) -> Result<Rendering, Error> {
        let (d, listed) = {
            let f = self.inner.read();
            let d = f.dview.get(id).ok_or(Error::NotFound)?;
            let listed = f
                .dview
                .list(&coord::decision::Filter {
                    all: true,
                    limit: 500,
                    ..Default::default()
                })
                .into_iter()
                .find(|l| l.decision.id == id)
                .unwrap_or_else(|| coord::decision::Listed {
                    decision: d.clone(),
                    superseded_by: Vec::new(),
                });
            (d, listed)
        };
        let mut root = String::new();
        if let Some(a) = &d.anchor
            && (a.root.is_empty() || !dir_exists(&a.root))
        {
            root = self.live_root_of(&a.repo_key);
        }
        let v = self.check_decision(listed.clone(), &root);
        let mut r = Rendering::new(
            search::make_ref(Kind::Decision, id),
            Kind::Decision,
            d.statement.clone(),
        );
        let by = if d.by.name.is_empty() {
            &d.by.key
        } else {
            &d.by.name
        };
        r.line(&format!(
            "decision {} by {by} ({}), at {}",
            r.r#ref,
            d.trust,
            stamp(&d.at)
        ));
        r.block("statement", &d.statement);
        r.block("why", &d.why);
        if let Some(a) = &d.anchor {
            let lines = if a.start > 0 {
                format!(":{}-{}", a.start, a.end)
            } else {
                String::new()
            };
            r.text.push('\n');
            r.line(&format!(
                "anchor: {} {}{lines} (recorded at {})",
                a.repo_key, a.path, a.sha
            ));
            if let Some(st) = &v.status {
                let mut now = anchor_state(st.state).to_string();
                if st.start > 0 && (st.start != a.start || st.end != a.end) {
                    now += &format!(", now at {}-{}", st.start, st.end);
                }
                r.line(&format!("anchor status: {now}"));
            }
        }
        for s in &d.supersedes {
            let sr = search::make_ref(Kind::Decision, s);
            r.line(&format!("supersedes: {sr}"));
            r.relate(&sr);
        }
        for s in &listed.superseded_by {
            let sr = search::make_ref(Kind::Decision, s);
            r.line(&format!("superseded by: {sr}"));
            r.relate(&sr);
        }
        if let Some(src) = &d.source
            && !src.message_id.is_empty()
        {
            r.relate(&search::make_ref(Kind::Msg, &src.message_id));
        }
        Ok(r)
    }

    fn render_agent(&self, key: &str, viewer: &str) -> Result<Rendering, Error> {
        let card = self
            .inner
            .live_cards()
            .into_values()
            .find(|cd| cd.key == key)
            .map(|cd| AgentLine {
                name: card_name(&cd.name, &cd.title, &cd.key),
                key: cd.key,
                status: cd.status,
                repo_key: cd.repo_key,
                doing: Default::default(),
            });
        let tl = self.timeline(&TimelineQuery {
            key: key.to_string(),
            limit: 20,
            ..Default::default()
        });
        if card.is_none() && tl.entries.is_empty() {
            return Err(Error::NotFound);
        }
        let mut r = Rendering::new(agent_ref(key), Kind::Agent, key.to_string());
        match &card {
            Some(c) => {
                r.title = c.name.clone();
                r.line(&format!(
                    "agent {} ({key}), {}, repo {}",
                    c.name, c.status, c.repo_key
                ));
            }
            None => r.line(&format!("agent {key} (not live)")),
        }
        r.text.push('\n');
        for e in &tl.entries {
            if let Some(t) = &e.turn {
                let rf = turn_ref(&t.row.id);
                let open = if t.open { " (open)" } else { "" };
                r.line(&format!(
                    "{} {rf}{open} {}",
                    stamp(&e.at),
                    clip_runes(
                        &first_line(&hide_messages(&t.row.turn.prompt, viewer, key)),
                        OVERVIEW_LINE_MAX
                    )
                ));
                r.relate(&rf);
            } else if let Some(c) = &e.commit {
                let rf = commit_ref(&c.row.commit.sha);
                r.line(&format!(
                    "{} {rf} {}",
                    stamp(&e.at),
                    clip_runes(&one_line(&c.row.commit.subject), OVERVIEW_LINE_MAX)
                ));
                r.relate(&rf);
            } else if let Some(p) = &e.pr {
                r.line(&format!("{} {} {}", stamp(&e.at), p.key, p.role.as_str()));
                r.relate(&p.key);
            } else if let Some(s) = &e.summary {
                r.line(&format!(
                    "{} summary {}",
                    stamp(&e.at),
                    clip_runes(&one_line(&s.summary.text), OVERVIEW_LINE_MAX)
                ));
            }
        }
        if tl.more {
            r.line(&format!("\nolder: search with agent {key}"));
        }
        Ok(r)
    }
}
