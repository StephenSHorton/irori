//! The Board as a search source for `task:` and `comment:` refs
//! (I:internal/coord/board_search.go, board.go's SearchKinds/SearchDocs/
//! SearchShow; coord.md §1.1, §3.1).
//!
//! A task's doc has Body = its short ref, objective, done criteria, note,
//! reason and card text (steps, open blocker items), Agent = Holder,
//! Updated = UpdatedAt (or the time its short ref moved); a comment's has
//! Title "comment on <task title>", Body = its text, Agent = its author if
//! an agent. Show renders a task as its fields, links, holders, blockers
//! and its newest [`SHOW_COMMENTS`] comments (Related: `comment:<id>` for
//! older ones); a comment as its text with Related `["task:<id>"]`.

use std::fmt::Write as _;

use gocompat::quote::quote;
use gocompat::time::GoTime;
use search::{Doc, Kind, MAX_RELATED, MAX_TITLE, ShowOptions, Shown, make_ref};

use super::Coord;
use super::fold::Fold;
use super::short::parse_short;
use super::types::*;
use super::util::{cmp_str, cut};

/// SearchDocs' limit when none is given.
pub const DEFAULT_DOC_PAGE: usize = 500;
/// The newest comments a task's Show renders.
pub const SHOW_COMMENTS: usize = 10;
/// Runes of each comment in a task's Show.
pub const SHOW_COMMENT_CUT: usize = 600;

/// Go's `FileRef.Describe`, through blob's.
fn describe(f: &FileRef) -> String {
    blob::FileRef {
        sha256: f.sha256.clone(),
        name: f.name.clone(),
        size: f.size,
        mime: f.mime.clone(),
        device: f.device.clone(),
    }
    .describe()
}

/// `joinNonEmpty`.
fn join_non_empty(sep: &str, parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(sep)
}

/// `cardText`: what search finds of a task's steps and open blocker items.
fn card_text(t: &Task) -> String {
    let mut parts: Vec<&str> = t.steps.iter().map(|s| s.text.as_str()).collect();
    for b in &t.blockers {
        if b.open() && b.question != t.reason {
            parts.push(&b.question);
        }
        if b.open() && !b.context.is_empty() {
            parts.push(&b.context);
        }
    }
    parts.join("\n")
}

impl Fold {
    fn search_docs(&self, since: &GoTime, limit: usize) -> Vec<Doc> {
        let limit = if limit == 0 { DEFAULT_DOC_PAGE } else { limit };
        let mut out = vec![];
        for t in self.tasks.values() {
            if crate::gotime::before(&t.updated_at, since) && !self.moved_since(&t.id, since) {
                continue;
            }
            let short = t.short_ref();
            let card = card_text(t);
            out.push(Doc {
                r#ref: make_ref(Kind::Task, &t.id),
                kind: Kind::Task,
                at: t.created_at,
                updated: t.updated_at,
                title: t.title.clone(),
                body: join_non_empty(
                    "\n",
                    &[
                        &short,
                        &t.objective,
                        &t.done_criteria,
                        &t.note,
                        &t.reason,
                        &card,
                    ],
                ),
                agent: t.holder.clone(),
                repo_key: t.repo_key.clone(),
                readers: vec![],
            });
        }
        for cm in self.comments.values() {
            // An edit (card #105) indexes the comment again.
            let updated = if cm.edited_at.is_zero() {
                cm.at
            } else {
                cm.edited_at
            };
            if crate::gotime::before(&updated, since) {
                continue;
            }
            let t = self.tasks.get(&cm.task_id);
            out.push(Doc {
                r#ref: make_ref(Kind::Comment, &cm.id),
                kind: Kind::Comment,
                at: cm.at,
                updated,
                title: format!("comment on {}", t.map(|t| t.title.as_str()).unwrap_or("")),
                body: cm.text.clone(),
                agent: if self.is_owner(&cm.by) {
                    String::new()
                } else {
                    cm.by.clone()
                },
                repo_key: t.map(|t| t.repo_key.clone()).unwrap_or_default(),
                readers: vec![],
            });
        }
        out.sort_by(|a, b| {
            crate::gotime::cmp(&a.updated, &b.updated).then_with(|| a.r#ref.cmp(&b.r#ref))
        });
        out.truncate(limit);
        out
    }

    fn search_show(&self, r: &str, o: &ShowOptions) -> Result<Shown, search::Error> {
        let (kind, mut id) = search::parse_ref(r)?;
        let (full, title, related) = match kind {
            Kind::Task => {
                let mut t = self.tasks.get(&id);
                if t.is_none()
                    && let Some(short) = parse_short(&id)
                {
                    t = self.short_idx.get(&short).and_then(|i| self.tasks.get(i));
                    if let Some(t) = t {
                        id = t.id.clone();
                    } else if let Some(e) = self.ambiguous(&short) {
                        return Err(search::Error::Other(e.to_string()));
                    }
                }
                let Some(t) = t else {
                    return Err(search::Error::NotFoundRef(r.to_string()));
                };
                let (full, related) = self.render_task(t);
                (full, cut(&t.title, MAX_TITLE), related)
            }
            Kind::Comment => {
                let Some(cm) = self.comments.get(&id) else {
                    return Err(search::Error::NotFoundRef(r.to_string()));
                };
                let t = self.tasks.get(&cm.task_id).cloned().unwrap_or_default();
                let mut b = format!(
                    "Comment by {} on {} ({}), {}",
                    cm.by,
                    quote(&t.title),
                    t.refs(),
                    cm.at.format_rfc3339()
                );
                if cm.wake {
                    b.push_str(", wake");
                }
                b.push_str("\n\n");
                b.push_str(&cm.text);
                b.push('\n');
                for f in &cm.attachments {
                    let _ = writeln!(
                        b,
                        "attached: {} (fetch it with attachment_get)",
                        describe(f)
                    );
                }
                (
                    b,
                    cut(&format!("comment on {}", t.title), MAX_TITLE),
                    vec![make_ref(Kind::Task, &t.id)],
                )
            }
            _ => {
                return Err(search::Error::NotFoundRef(format!(
                    "coord has no {kind} refs"
                )));
            }
        };
        let (text, more, next) = search::page(&full, o);
        Ok(Shown {
            r#ref: make_ref(kind, &id),
            kind,
            title,
            text: text.to_string(),
            more,
            next,
            related,
        })
    }

    /// Keys joined, each the roster knows followed by its device's
    /// presence (card #372): "key (device online)", "key (device offline
    /// since …)".
    pub(crate) fn with_devices(&self, keys: &[String]) -> String {
        let agents = self.roster.as_ref().map(|r| r.agents()).unwrap_or_default();
        let now = self.stamp();
        keys.iter()
            .map(|k| match agents.iter().find(|a| &a.key == k) {
                Some(a) => format!(
                    "{k} (device {})",
                    crate::roster::device_phrase(
                        &a.device_state,
                        &a.device_seen,
                        &a.device_via,
                        &now
                    )
                ),
                None => k.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// `renderTask`: a task's Show text and Related refs.
    pub(crate) fn render_task(&self, t: &Task) -> (String, Vec<String>) {
        let mut b = String::new();
        let line = |b: &mut String, label: &str, v: &str| {
            if !v.is_empty() {
                let _ = writeln!(b, "{label}: {v}");
            }
        };
        let _ = writeln!(
            b,
            "Task {} {} (task:{})",
            t.short_ref(),
            quote(&t.title),
            t.id
        );
        if !t.short_was.is_empty() {
            let w = &t.short_was;
            let _ = writeln!(
                b,
                "renumbered: it was #{w}, until cards made on devices that were apart turned out to have #{w} too; #{w} alone now names none of them"
            );
        }
        let mut state = effective_state(t).to_string();
        if let Some(r) = &t.rollup {
            let _ = write!(state, " (from its subtasks: {}/{} done)", r.done, r.total);
        }
        if t.shared {
            state.push_str(", shared");
        }
        line(&mut b, "state", &state);
        line(&mut b, "side", &t.side);
        line(&mut b, "waits", &self.side_wait(t));
        line(&mut b, "ask", &t.ask_line());
        if let Some(p) = self.tasks.get(&t.parent) {
            line(
                &mut b,
                "part of",
                &format!("{} {}", p.short_ref(), quote(&p.title)),
            );
        }
        if !t.subtasks.is_empty() {
            let mut ks = String::new();
            for id in &t.subtasks {
                if let Some(k) = self.tasks.get(id) {
                    let who = if k.holders.is_empty() {
                        "nobody on it".to_string()
                    } else {
                        k.holders.join(", ")
                    };
                    let (done, total) = k.progress();
                    let steps = if total > 0 {
                        format!(", steps {done}/{total}")
                    } else {
                        String::new()
                    };
                    let _ = write!(
                        ks,
                        "\n- {} [{}] {}: {who}{steps}",
                        k.short_ref(),
                        k.state,
                        quote(&k.title)
                    );
                }
            }
            line(&mut b, "subtasks", &ks);
        }
        line(&mut b, "repo", &t.repo_key);
        line(&mut b, "holders", &self.with_devices(&t.holders));
        line(&mut b, "assigned, not claimed yet", &t.assigned.join(", "));
        for (to, note) in &t.handoffs {
            line(&mut b, &format!("hand-off note for {to}"), note);
        }
        line(
            &mut b,
            "created",
            &format!("{} at {}", t.created_by, t.created_at.format_rfc3339()),
        );
        line(&mut b, "updated", &t.updated_at.format_rfc3339());
        if t.state == BLOCKED {
            let mut bl = t.reason.clone();
            if let Some(bb) = &t.blocked_by {
                let mut on = bb.kind.clone();
                if !bb.reference.is_empty() {
                    on.push(' ');
                    on.push_str(&bb.reference);
                }
                let _ = write!(bl, " (on {on})");
            }
            line(&mut b, "blocked", &bl);
        }
        // Gone watchers are left out (card #485).
        let rs = self.roster_status();
        let ws: Vec<&str> = t
            .watchers
            .iter()
            .filter(|w| !self.key_gone_in(w, &rs))
            .map(String::as_str)
            .collect();
        line(&mut b, "watching", &ws.join(", "));
        // Card #155: the thread it came from, and a thread's cards.
        if !t.from.is_empty() {
            line(&mut b, "from the thread", &self.short_of(&t.from));
        }
        let made: Vec<String> = t.made.iter().map(|id| self.short_of(id)).collect();
        line(&mut b, "cards from this thread", &made.join(", "));
        // Waits (card #87): who waits on this card, and what its holders
        // wait on.
        let on_it: Vec<String> = self
            .waits
            .values()
            .filter(|w| w.task_id == t.id)
            .map(|w| format!("{} until {}", w.by, super::waits::until_text(&w.until)))
            .collect();
        line(&mut b, "waited on by", &on_it.join(", "));
        let theirs: Vec<String> = self
            .waits
            .values()
            .filter(|w| t.holders.contains(&w.by) && !is_terminal(&t.state))
            .map(|w| {
                let r = self
                    .tasks
                    .get(&w.task_id)
                    .map(|x| x.short_ref())
                    .unwrap_or_default();
                format!("{} waits on {}", w.by, super::waits::wait_line(w, &r))
            })
            .collect();
        line(&mut b, "waiting", &theirs.join("; "));
        line(&mut b, "scope", &t.scope.join(", "));
        line(&mut b, "note", &t.note);
        if !t.steps.is_empty() {
            let (d, n) = t.progress();
            if t.rollup.is_some() {
                // A parent's progress is its subtasks'.
                b.push_str("\nsteps done before the split:\n");
            } else {
                let _ = writeln!(b, "\nsteps ({d}/{n} done):");
            }
            for s in &t.steps {
                let mark = if s.done {
                    "[x]"
                } else if s.doing {
                    "[>]"
                } else {
                    "[ ]"
                };
                let _ = writeln!(b, "- {mark} {} {}", s.id, s.text);
            }
        }
        if !t.blockers.is_empty() {
            b.push_str("\nblockers:\n");
            for bl in &t.blockers {
                let state = match &bl.resolved {
                    Some(r) if r.closed => {
                        let mut s = format!("closed by {}", r.by);
                        // Card #481: why, then the person's answer.
                        if !r.note.is_empty() {
                            let _ = write!(s, ": {}; the answer", r.note);
                        }
                        if !r.answer.is_empty() {
                            let _ = write!(s, ": {}", r.answer);
                        }
                        s
                    }
                    Some(r) if !r.note.is_empty() => {
                        format!("resolved by {}: {}; the answer: {}", r.by, r.note, r.answer)
                    }
                    Some(r) => format!("answered by {}: {}", r.by, r.answer),
                    None if bl.status == BLOCKER_ANSWERED => format!(
                        "answered, waiting on {} to resolve or reply",
                        cmp_str(&bl.sent_to, &bl.raised_by)
                    ),
                    None if bl.status == BLOCKER_REPLIED => {
                        "replied, back with the person".to_string()
                    }
                    None => "open".to_string(),
                };
                let _ = writeln!(
                    b,
                    "- {} ({state}) {}, raised by {} at {}",
                    bl.id,
                    bl.question,
                    bl.raised_by,
                    bl.at.format_rfc3339()
                );
                if !bl.context.is_empty() {
                    let _ = writeln!(b, "  context: {}", bl.context);
                }
                if bl.open() && !bl.missing.is_empty() {
                    let _ = writeln!(b, "  missing: {}", bl.missing.join("; "));
                }
                for l in &bl.links {
                    let _ = writeln!(b, "  link: {} ({})", l.label, l.reference);
                }
                for f in &bl.attachments {
                    let _ = writeln!(b, "  file: {}", describe(f));
                }
                if bl.open() && bl.options_aside && bl.done_check {
                    let _ = writeln!(
                        b,
                        "  Not done: the person said why; answer them in its thread (board_comment blocker_id), then set completed again with proof that says what changed"
                    );
                } else if bl.open() && bl.options_aside {
                    let _ = writeln!(
                        b,
                        "  options set aside: the person answered in their own words; revise with board_block blocker_id for new ones (the earlier ones follow)"
                    );
                }
                for (i, o) in bl.options.iter().enumerate() {
                    if o.recommended {
                        let _ = write!(
                            b,
                            "  option {}: {} (recommended by {})",
                            i + 1,
                            o.answer,
                            o.by
                        );
                    } else {
                        let _ = write!(b, "  option {}: {} (from {})", i + 1, o.answer, o.by);
                    }
                    if !o.why.is_empty() {
                        let _ = write!(b, ": {}", o.why);
                    }
                    if let Some(l) = &o.link {
                        let _ = write!(b, " [link: {}]", l.reference);
                    }
                    b.push('\n');
                }
            }
        }
        if !t.objective.is_empty() {
            let _ = writeln!(b, "\nobjective:\n{}", t.objective);
        }
        if !t.done_criteria.is_empty() {
            let _ = writeln!(b, "\ndone when:\n{}", t.done_criteria);
        }
        let reviews = self.reviews_of(&t.id);
        if !self.on_board && !reviews.is_empty() {
            b.push_str("\nreviews:\n");
            for r in &reviews {
                let state = match r.state.as_str() {
                    super::review::VERDICT_CHANGES => "changes asked",
                    s => s,
                };
                let _ = write!(b, "- {} (asked by {}): {state}", r.reviewer, r.from);
                if !r.pr.is_empty() {
                    let _ = write!(b, " [{}]", r.pr);
                }
                if !r.comment_id.is_empty() {
                    let _ = write!(b, " (comment:{})", r.comment_id);
                }
                b.push('\n');
            }
        }
        if !t.links.is_empty() {
            b.push_str("\nlinks:\n");
            let heard = self.pr_states.get(&t.id);
            for l in &t.links {
                let _ = write!(b, "- {}: {}", l.label, l.reference);
                // Card #208: how GitHub last said a linked PR stands.
                if let Some(s) = heard.and_then(|m| m.get(&l.reference)) {
                    if !s.error.is_empty() {
                        let _ = write!(b, " (GitHub not asked: {})", s.error);
                    } else {
                        let _ = write!(b, " (GitHub: {}", s.state.to_lowercase());
                        if !s.review.is_empty() {
                            let _ = write!(b, ", {}", s.review.to_lowercase().replace('_', " "));
                        }
                        b.push(')');
                    }
                }
                b.push('\n');
            }
        }
        // Card #555: what the person says on a subtask stays in view on its
        // parent, so a slice doesn't carry on with an old premise.
        let subs = self.said_on_subtasks(&t.id);
        if !subs.is_empty() {
            b.push_str("\nthe person, on its subtasks (newest; show a comment for all of it):\n");
            for s in subs {
                let _ = writeln!(b, "- {} comment:{}: {}", s.card, s.comment_id, s.text);
            }
        }
        let mut related = vec![];
        if let Some(bb) = &t.blocked_by
            && t.state == BLOCKED
            && bb.kind == BLOCKED_ON_TASK
        {
            related.push(make_ref(Kind::Task, &bb.reference));
        }
        let ids: &[String] = self.by_task.get(&t.id).map(Vec::as_slice).unwrap_or(&[]);
        if !ids.is_empty() {
            let shown = &ids[ids.len().saturating_sub(SHOW_COMMENTS)..];
            let older = &ids[..ids.len() - shown.len()];
            let _ = write!(b, "\ncomments ({}", ids.len());
            if !older.is_empty() {
                let _ = write!(b, ", newest {} shown; Related has older ones", shown.len());
            }
            b.push_str("):\n");
            for id in shown {
                let Some(cm) = self.comments.get(id) else {
                    continue;
                };
                let mut text = cut(&cm.text, SHOW_COMMENT_CUT);
                if text != cm.text {
                    let _ = write!(text, " (show comment:{} for all of it)", cm.id);
                }
                if cm.held {
                    text = "(not let through the wake policy yet; text not on the card)".into();
                }
                if !cm.attachments.is_empty() {
                    let _ = write!(
                        text,
                        " [{} attached: show comment:{}]",
                        cm.attachments.len(),
                        cm.id
                    );
                }
                if cm.files > 0 {
                    let _ = write!(text, " [{} files on msg:{}]", cm.files, cm.msg_id);
                }
                let mut who = cm.by.clone();
                if !cm.msg_id.is_empty() {
                    // A message about the card: who sent it to whom.
                    let _ = write!(who, " -> {}", cm.to);
                }
                if cm.gone {
                    text.push_str(" (recipient gone, left on the card)");
                }
                if !cm.verdict.is_empty() {
                    let _ = write!(who, " (review: {})", cm.verdict);
                }
                if !cm.blocker_id.is_empty() {
                    // The person's answer on a blocker, or an agent's reply on it.
                    let _ = write!(who, " on {}", cm.blocker_id);
                    if cm.pick > 0 {
                        let _ = write!(text, " (option {})", cm.pick);
                    }
                    if !cm.missing.is_empty() {
                        let _ = write!(text, " [missing: {}]", cm.missing.join("; "));
                    }
                }
                let _ = writeln!(b, "- {} {who}: {text}", cm.at.format_rfc3339());
            }
            for id in older.iter().rev() {
                if related.len() >= MAX_RELATED {
                    break;
                }
                related.push(make_ref(Kind::Comment, id));
            }
        }
        (b, related)
    }
}

impl search::Source for Coord {
    fn search_kinds(&self) -> Vec<Kind> {
        vec![Kind::Task, Kind::Comment]
    }

    fn search_docs(&self, since: GoTime, limit: usize) -> Result<Vec<Doc>, search::Error> {
        Ok(self.lock().search_docs(&since, limit))
    }

    fn search_show(&self, r: &str, o: &ShowOptions) -> Result<Shown, search::Error> {
        self.lock().search_show(r, o)
    }
}
