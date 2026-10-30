//! The Board's calls: assign, join, block, comments, messages, links,
//! refusals, the grouped view and the sweep (I:internal/coord/board_impl.go;
//! coord.md §3.5, §3.7, §3.10, §3.15).

use std::collections::{BTreeMap, HashSet};

use gocompat::quote::quote;
use gocompat::time::GoTime;
use store::kinds;

use super::cards::count_open;
use super::error::{Result, bad_state, invalid, not_allowed, not_found};
use super::fold::{
    Fold, LEFT_GONE, LEFT_REMOVED, RefusalRec, engaged, is_holder, may_record, restore_state,
};
use super::links::parse_link;
use super::resource::{RESOURCE_PATH, Resource, conflicts, normalize_resource, parse_resource};
use super::rows::*;
use super::types::*;
use super::util::{
    MAX_NOTICE_FIELD, SWEEPER, args, caller, check_key, contains, cut, deeper, multiline, new_id,
    runes,
};
use super::{
    DEFAULT_DONE_CARD, MAX_ATTACHMENTS, MAX_COMMENT, MAX_DONE_CARD, MAX_REASON, MAX_REF,
    REOPEN_COMMENT_NOTE,
};
use crate::gotime;
use crate::names::trim_space;

/// Validates a block reason or unblock note (trimmed).
pub(crate) fn check_reason(s: &str, required: bool) -> Result<()> {
    if required && s.is_empty() {
        return Err(invalid("a blocked task needs a reason"));
    }
    if runes(s) > MAX_REASON {
        return Err(invalid(format!("longer than {MAX_REASON} characters")));
    }
    Ok(())
}

/// Trims and checks a comment's caller, text and files.
pub(crate) fn check_comment(by: &str, text: &str, files: &[FileRef]) -> Result<(String, String)> {
    let by = caller(by)?;
    let text = trim_space(text).to_string();
    if text.is_empty() {
        return Err(invalid("an empty comment"));
    }
    if runes(&text) > MAX_COMMENT {
        return Err(invalid(format!(
            "comment longer than {MAX_COMMENT} characters"
        )));
    }
    if files.len() > MAX_ATTACHMENTS {
        return Err(invalid(format!("at most {MAX_ATTACHMENTS} attachments")));
    }
    Ok((by, text))
}

impl Fold {
    /// Validates and normalizes what blocks task `me` (`""` while it is
    /// being created).
    pub(crate) fn check_blocker(
        &self,
        bb: Option<&BlockedBy>,
        me: &str,
    ) -> Result<Option<BlockedBy>> {
        let Some(bb) = bb else {
            return Ok(None);
        };
        let mut out = BlockedBy {
            kind: bb.kind.clone(),
            reference: trim_space(&bb.reference).to_string(),
        };
        if runes(&out.reference) > MAX_REF || multiline(&out.reference) {
            return Err(invalid(format!(
                "blocker ref longer than {MAX_REF} characters or on several lines"
            )));
        }
        match out.kind.as_str() {
            BLOCKED_ON_LEASE => {
                let n = normalize_resource(&out.reference)
                    .map_err(|e| invalid(format!("blocked on a lease: {e}")))?;
                if !self.lease_held(&n, &self.stamp()) {
                    return Err(invalid(format!(
                        "nobody leases {n}; block on external or a person instead"
                    )));
                }
                out.reference = n;
            }
            BLOCKED_ON_TASK => {
                if out.reference.is_empty() || out.reference == me {
                    return Err(invalid("blocked on a task needs another task's id"));
                }
                let Some(o) = self.tasks.get(&out.reference) else {
                    return Err(not_found(format!(
                        "blocked on task {}",
                        quote(&out.reference)
                    )));
                };
                if is_terminal(&o.state) {
                    return Err(invalid(format!(
                        "task {} is already {}",
                        out.reference, o.state
                    )));
                }
            }
            BLOCKED_ON_PERSON => {
                if !out.reference.is_empty() && !self.is_owner(&out.reference) {
                    return Err(invalid("blocked on a person needs \"\" or a user: key"));
                }
            }
            BLOCKED_ON_AGENT => {
                if out.reference.is_empty() || self.is_owner(&out.reference) {
                    return Err(invalid("blocked on an agent needs its key"));
                }
            }
            BLOCKED_ON_EXTERNAL => {}
            _ => {
                return Err(invalid(format!(
                    "blocker kind {} (lease|task|person|agent|external)",
                    quote(&bb.kind)
                )));
            }
        }
        Ok(Some(out))
    }

    // ---- assign, join, leave, share ----

    pub(crate) fn assign_notice(&mut self, by: &str, id: &str, to: &str) -> Result<(Task, Notice)> {
        let (t, wrote) = self.assign(by, id, to, true)?;
        if !wrote {
            return Ok((t, Notice::default()));
        }
        let by = trim_space(by).to_string();
        let to = trim_space(to).to_string();
        let mut n = Notice {
            kind: NOTICE_ASSIGNED.into(),
            task_id: t.id.clone(),
            reference: format!("task:{}", t.id),
            title: t.title.clone(),
            by: by.clone(),
            ..Notice::default()
        };
        if !self.is_owner(&to) && to != by {
            n.to = vec![to.clone()];
        }
        let mut b = format!(
            "{by} assigned you a task: {} ({}).",
            quote(&t.title),
            t.refs()
        );
        if !t.objective.is_empty() {
            b.push_str(&format!(
                "\nObjective: {}",
                cut(&t.objective, MAX_NOTICE_FIELD)
            ));
        }
        if !t.done_criteria.is_empty() {
            b.push_str(&format!(
                "\nDone when: {}",
                cut(&t.done_criteria, MAX_NOTICE_FIELD)
            ));
        }
        if contains(&t.assigned, &to) {
            b.push_str("\nClaim it to start.");
        }
        b.push_str(&deeper(&t.id));
        n.text = b;
        Ok((t, n))
    }

    pub(crate) fn join(&mut self, by: &str, id: &str, who: &str) -> Result<Task> {
        let by = caller(by)?;
        let who = args(&[who])?.remove(0);
        check_key(&who)?;
        if by != who && !self.is_owner(&by) {
            return Err(not_allowed(format!("only {who} or a person may add {who}")));
        }
        let t = self.open_task(id)?;
        if t.open_ask().is_some() {
            return Err(super::asks::ask_refusal(t, "one agent takes it"));
        }
        if !t.shared {
            return Err(not_allowed(format!(
                "task {id} isn't shared; claim it instead"
            )));
        }
        if engaged(t, &who) && t.state != SUBMITTED {
            return Ok(t.clone());
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_JOINED,
            &TaskMemberRow {
                task_id: id.into(),
                who,
                by: by.clone(),
                at,
                ..TaskMemberRow::default()
            },
        )?;
        Ok(self.tasks[id].clone())
    }

    pub(crate) fn leave(&mut self, by: &str, id: &str, who: &str) -> Result<Task> {
        let by = caller(by)?;
        let who = args(&[who])?.remove(0);
        if by != who && !self.is_owner(&by) {
            return Err(not_allowed(format!(
                "only {who} or a person may take {who} off"
            )));
        }
        let t = self.open_task(id)?;
        if let Some(a) = t.open_ask() {
            if a.taker != who {
                return Err(not_found(format!(
                    "{who} hasn't taken the ask {}",
                    t.short_ref()
                )));
            }
            self.ask_release(&who, id)?;
            return Ok(self.tasks[id].clone());
        }
        if !is_holder(t, &who) {
            return Err(not_found(format!("{who} isn't on task {id}")));
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_LEFT,
            &TaskMemberRow {
                task_id: id.into(),
                who,
                by: by.clone(),
                at,
                ..TaskMemberRow::default()
            },
        )?;
        Ok(self.tasks[id].clone())
    }

    /// Takes every key of board member `person_id` (their agents and the
    /// person) off the open cards they hold or are assigned (v0.5 §4.3,
    /// the remover's cleanup), as the person at this device. A card left
    /// with nobody on it goes back to submitted. Returns how many came off.
    pub(crate) fn take_off_person(&mut self, by: &str, person_id: &str) -> Result<usize> {
        let by = caller(by)?;
        if !self.is_owner(&by) {
            return Err(not_allowed("only the person takes another member off"));
        }
        let Some(kp) = self.key_person.clone().filter(|_| self.on_board) else {
            return Err(invalid("taking a member off is for a shared board"));
        };
        if person_id.is_empty() {
            return Ok(0);
        }
        let mut n = 0;
        let ids: Vec<String> = self
            .tasks
            .iter()
            .filter(|(_, t)| !is_terminal(&t.state))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            let t = self.tasks[&id].clone();
            let mut keys: Vec<String> = t.holders.clone();
            for a in &t.assigned {
                if !keys.contains(a) {
                    keys.push(a.clone());
                }
            }
            for who in keys.into_iter().filter(|k| kp(k) == person_id) {
                let at = self.stamp();
                self.write(
                    &by,
                    kinds::TASK_LEFT,
                    &TaskMemberRow {
                        task_id: id.clone(),
                        who,
                        by: by.clone(),
                        at,
                        reason: LEFT_REMOVED.into(),
                        ..TaskMemberRow::default()
                    },
                )?;
                n += 1;
            }
            let now = &self.tasks[&id];
            if t.state == WORKING && now.holders.is_empty() && now.state == WORKING {
                let at = self.stamp();
                self.write(
                    &by,
                    kinds::TASK_STATUS,
                    &TaskStateRow {
                        task_id: id.clone(),
                        state: SUBMITTED.into(),
                        note: "nobody is on it: the person whose agent worked it left the board"
                            .into(),
                        by: by.clone(),
                        at,
                        ..TaskStateRow::default()
                    },
                )?;
            }
        }
        Ok(n)
    }

    pub(crate) fn set_shared(&mut self, by: &str, id: &str, shared: bool) -> Result<Task> {
        let by = caller(by)?;
        let t = self.open_task(id)?;
        if !self.may_manage(t, &by) {
            return Err(not_allowed(format!("share task {id}")));
        }
        if t.shared == shared {
            return Ok(t.clone());
        }
        if !shared && t.holders.len() > 1 {
            return Err(invalid(format!(
                "{} hold task {id}; they leave first",
                t.holders.len()
            )));
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_SHARED,
            &TaskSharedRow {
                task_id: id.into(),
                shared,
                by: by.clone(),
                at,
            },
        )?;
        Ok(self.tasks[id].clone())
    }

    // ---- block, unblock ----

    pub(crate) fn block(
        &mut self,
        by: &str,
        id: &str,
        reason: &str,
        bb: Option<BlockedBy>,
    ) -> Result<Task> {
        let by = caller(by)?;
        let reason = trim_space(reason).to_string();
        check_reason(&reason, true)?;
        let t = self.open_task(id)?.clone();
        if !self.may_manage(&t, &by) {
            return Err(not_allowed(format!("block task {id}")));
        }
        let (q, cx) = super::cards::block_question(&reason);
        let now = self.stamp();
        let (out, _) = self.raise_locked(
            &by,
            &t,
            &reason,
            &q,
            &cx,
            Vec::new(),
            bb,
            true,
            now,
            Vec::new(),
            Vec::new(),
        )?;
        Ok(out)
    }

    pub(crate) fn unblock(&mut self, by: &str, id: &str, note: &str) -> Result<(Task, Notice)> {
        let by = caller(by)?;
        let note = trim_space(note).to_string();
        check_reason(&note, false)?;
        let t = self.open_task(id)?;
        if t.state != BLOCKED {
            return Err(bad_state(format!("task {id} is {}, not blocked", t.state)));
        }
        let now = self.stamp();
        let n = self.close_blocker(&by, id, &note, now)?;
        Ok((self.tasks[id].clone(), n))
    }

    /// Writes blocker.closed for a blocked task and builds the notice.
    pub(crate) fn close_blocker(
        &mut self,
        by: &str,
        id: &str,
        note: &str,
        now: GoTime,
    ) -> Result<Notice> {
        let t = self.tasks[id].clone();
        let restored = restore_state(&t);
        self.write(
            by,
            kinds::BLOCKER_CLOSED,
            &BlockerClosedRow {
                task_id: id.into(),
                by: by.into(),
                note: note.into(),
                restored: restored.into(),
                at: now,
            },
        )?;
        let nt = self.tasks[id].clone();
        let who = if by == SWEEPER { "Irori" } else { by };
        let mut b = format!("{who} unblocked {} ({})", quote(&t.title), t.refs());
        if !note.is_empty() {
            b.push_str(&format!(": {note}"));
        }
        b.push_str(&format!(
            ". It was blocked on: {}. It is {restored} again.",
            cut(&t.reason, MAX_NOTICE_FIELD)
        ));
        b.push_str(&deeper(id));
        Ok(Notice {
            kind: NOTICE_UNBLOCKED.into(),
            task_id: id.into(),
            reference: format!("task:{id}"),
            title: t.title.clone(),
            by: by.into(),
            to: self.recipients(&nt, by),
            text: b,
            ..Notice::default()
        })
    }

    /// Who hears about t: its agent holders other than by; with none, its
    /// creator if that is an agent other than by.
    pub(crate) fn recipients(&self, t: &Task, by: &str) -> Vec<String> {
        let mut to: Vec<String> = t
            .holders
            .iter()
            .filter(|h| *h != by && !self.is_owner(h) && *h != SWEEPER)
            .cloned()
            .collect();
        let c = &t.created_by;
        if t.holders.is_empty() && c != by && !c.is_empty() && !self.is_owner(c) && c != SWEEPER {
            to = vec![c.clone()];
        }
        to
    }

    // ---- comments ----

    pub(crate) fn add_comment(
        &mut self,
        by: &str,
        id: &str,
        text: &str,
        wake: bool,
        files: Vec<FileRef>,
    ) -> Result<(Comment, Notice)> {
        let (by, text) = check_comment(by, text, &files)?;
        let t = self.any_task(id)?.clone();
        let at = self.stamp();
        let cid = new_id(&at);
        let actor = self.actor_for(&by);
        let cm = self.write_comment(
            &actor,
            TaskCommentRow {
                task_id: id.into(),
                comment_id: cid.clone(),
                by: by.clone(),
                text: text.clone(),
                wake,
                at,
                attachments: files.clone(),
                ..TaskCommentRow::default()
            },
        )?;
        let nt = self.tasks[id].clone();
        let n = Notice {
            kind: NOTICE_COMMENT.into(),
            task_id: id.into(),
            reference: format!("comment:{cid}"),
            title: t.title.clone(),
            by: by.clone(),
            to: self.recipients(&nt, &by),
            wake,
            attachments: files,
            text: format!(
                "{by} commented on {} ({}):\n{text}{}",
                quote(&t.title),
                t.refs(),
                deeper(id)
            ),
            ..Notice::default()
        };
        Ok((cm, n))
    }

    /// Who hears the person's comment on t: answerers, then watchers.
    fn hearers(
        &self,
        t: &Task,
        by: &str,
        around: Option<&dyn Fn(&str) -> bool>,
    ) -> (Vec<String>, Vec<String>) {
        let mut answer: Vec<String> = Vec::new();
        let mut watch: Vec<String> = Vec::new();
        let ok = |k: &str, answer: &[String], watch: &[String]| {
            !k.is_empty()
                && k != by
                && k != SWEEPER
                && !self.is_owner(k)
                && !contains(answer, k)
                && !contains(watch, k)
                && around.is_none_or(|f| f(k))
        };
        for h in &t.holders {
            if !contains(&t.assigned, h) && ok(h, &answer, &watch) {
                answer.push(h.clone());
            }
        }
        for h in &t.holders {
            if contains(&t.assigned, h) && ok(h, &answer, &watch) {
                answer.push(h.clone());
            }
        }
        if answer.is_empty() && ok(&t.created_by, &answer, &watch) {
            answer = vec![t.created_by.clone()];
        }
        for w in &t.watchers {
            if ok(w, &answer, &watch) {
                watch.push(w.clone());
            }
        }
        (answer, watch)
    }

    pub(crate) fn person_comment(
        &mut self,
        by: &str,
        id: &str,
        text: &str,
        wake: bool,
        around: Option<&dyn Fn(&str) -> bool>,
    ) -> Result<(Comment, Vec<Notice>)> {
        let (by, text) = check_comment(by, text, &[])?;
        if !self.is_owner(&by) {
            return Err(invalid("only the person comments from the app"));
        }
        let t = self.any_task(id)?.clone();
        let (mut answer, mut watch) = self.hearers(&t, &by, around);
        let mut room = false;
        if let Some(a) = t.open_ask()
            && a.taker.is_empty()
        {
            room = true;
            answer = a
                .heard
                .iter()
                .filter(|k| **k != by && around.is_none_or(|f| f(k)))
                .cloned()
                .collect();
            watch.clear();
        }
        let mut was = String::new();
        if t.holders
            .iter()
            .any(|h| !self.is_owner(h) && around.is_none_or(|f| f(h)))
            && self.reopen_refusal(&t).is_ok()
        {
            was = t.state.clone();
            self.reopen_locked(&by, id, REOPEN_COMMENT_NOTE)?;
        }
        let at = self.stamp();
        let cid = new_id(&at);
        let actor = self.actor_for(&by);
        let cm = self.write_comment(
            &actor,
            TaskCommentRow {
                task_id: id.into(),
                comment_id: cid.clone(),
                by: by.clone(),
                text: text.clone(),
                wake,
                at,
                unheard: answer.is_empty() && watch.is_empty(),
                ..TaskCommentRow::default()
            },
        )?;
        let t = self.tasks[id].clone();
        let mut head = format!("The person commented on {} ({})", quote(&t.title), t.refs());
        if !was.is_empty() {
            head.push_str(&format!(", which was {was} and is open again"));
        }
        let base = Notice {
            kind: NOTICE_COMMENT.into(),
            task_id: id.into(),
            reference: format!("comment:{cid}"),
            title: t.title.clone(),
            by: by.clone(),
            ..Notice::default()
        };
        if room && !answer.is_empty() {
            let mut a = base;
            a.kind = NOTICE_ASK_COMMENT.into();
            a.reference = format!("task:{id}");
            a.to = answer;
            a.wake = wake;
            a.person = true;
            a.text = format!(
                "{head}, the ask nobody has taken yet:\n{text}\nIf you can take it on now, ask_claim {}. If you're busy, leave it.{}",
                t.short_ref(),
                deeper(id)
            );
            return Ok((cm, vec![a]));
        }
        let mut ns = Vec::new();
        if !answer.is_empty() {
            let mut a = base.clone();
            a.to = answer.clone();
            a.wake = wake;
            a.text = format!("{head}:\n{text}\nAnswer it on the card with board_comment.");
            if answer.len() > 1 {
                a.text
                    .push_str(" One answer is enough: read the card first.");
            }
            a.text.push_str(&deeper(id));
            ns.push(a);
        }
        if !watch.is_empty() {
            let mut w = base;
            w.to = watch;
            w.wake = wake && answer.is_empty();
            w.watching = true;
            w.text = format!("{head}, a card you watch:\n{text}\n");
            if !answer.is_empty() {
                w.text.push_str(&format!(
                    "{} answers it; answer on the card only if it asks you.",
                    answer.join(", ")
                ));
            } else {
                w.text.push_str("Nobody else is on it: if it asks something you can answer, answer on the card with board_comment.");
            }
            w.text.push_str(&deeper(id));
            ns.push(w);
        }
        Ok((cm, ns))
    }

    /// Appends a task.comment row as actor; returns the comment it folded to.
    pub(crate) fn write_comment(&mut self, actor: &str, r: TaskCommentRow) -> Result<Comment> {
        let cid = r.comment_id.clone();
        self.write_as(actor, kinds::TASK_COMMENT, &r)?;
        self.comments
            .get(&cid)
            .cloned()
            .ok_or_else(|| invalid(format!("comment {cid} did not fold")))
    }

    /// Puts an inbox message on its card's thread.
    pub(crate) fn record_message(&mut self, m: MessageEntry) -> Result<Comment> {
        let from = caller(&m.from)?;
        let id = args(&[&m.task_id])?.remove(0);
        let to = trim_space(&m.to).to_string();
        if !m.msg_id.starts_with("msg_") || m.msg_id.len() == 4 {
            return Err(invalid(format!("message id {}", quote(&m.msg_id))));
        }
        let mut text = trim_space(&m.text).to_string();
        let held = m.held && !m.fill;
        if held {
            text.clear();
        } else if runes(&text) > MAX_COMMENT {
            let full = format!("… (full: msg:{})", m.msg_id);
            let keep = MAX_COMMENT - runes(&full);
            text = text.chars().take(keep).collect::<String>() + &full;
        }
        let mut actor = self.actor_for(&from);
        if m.writer == SWEEPER {
            actor = SWEEPER.into();
        }
        let t = self.any_task(&id)?.clone();
        if let Some(cm) = self.comments.get(&m.msg_id)
            && (m.gone && cm.gone || m.fill && !cm.held || !m.gone && !m.fill && cm.primary)
        {
            return Ok(cm.clone());
        }
        if !may_record(&from, &to, m.gone || m.fill, &self.dev) {
            return Err(not_allowed(format!(
                "{from}'s message is recorded from its own device"
            )));
        }
        let at = self.stamp();
        self.write_comment(
            &actor,
            TaskCommentRow {
                task_id: t.id.clone(),
                comment_id: m.msg_id.clone(),
                msg_id: m.msg_id.clone(),
                by: from,
                to,
                text,
                files: m.files.max(0),
                held,
                gone: m.gone,
                fill: m.fill,
                at,
                ..TaskCommentRow::default()
            },
        )
    }

    /// The ref on the earliest comment that carries sha, else the earliest
    /// blocker.
    pub(crate) fn comment_attachment(&self, sha: &str) -> Option<FileRef> {
        let mut got: Option<(FileRef, GoTime, String)> = None;
        for cm in self.comments.values() {
            if let Some((_, at, cid)) = &got
                && (gotime::after(&cm.at, at) || cm.at.equal(at) && cm.id > *cid)
            {
                continue;
            }
            if let Some(f) = cm.attachments.iter().find(|f| f.sha256 == sha) {
                got = Some((f.clone(), cm.at, cm.id.clone()));
            }
        }
        if let Some((f, _, _)) = got {
            return Some(f);
        }
        let mut best: Option<(FileRef, GoTime, String)> = None;
        for t in self.tasks.values() {
            for b in &t.blockers {
                let k = format!("{}/{}", t.id, b.id);
                if let Some((_, at, key)) = &best
                    && (gotime::after(&b.at, at) || b.at.equal(at) && k > *key)
                {
                    continue;
                }
                if let Some(f) = b.attachments.iter().find(|f| f.sha256 == sha) {
                    best = Some((f.clone(), b.at, k));
                }
            }
        }
        best.map(|(f, _, _)| f)
    }

    pub(crate) fn list_comments(&self, id: &str) -> Option<Vec<Comment>> {
        self.tasks.get(id)?;
        Some(
            self.by_task
                .get(id)
                .into_iter()
                .flatten()
                .filter_map(|c| self.comments.get(c).cloned())
                .collect(),
        )
    }

    // ---- links ----

    pub(crate) fn add_link(&mut self, by: &str, id: &str, link: &str) -> Result<Task> {
        let by = caller(by)?;
        let l = parse_link(link)?;
        let t = self.any_task(id)?;
        if t.links.iter().any(|x| x.reference == l.reference) {
            return Ok(t.clone());
        }
        if t.links.len() >= super::links::MAX_LINKS {
            return Err(invalid(format!(
                "task {id} already has {} links",
                super::links::MAX_LINKS
            )));
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_LINKED,
            &TaskLinkedRow {
                task_id: id.into(),
                link: l,
                by: by.clone(),
                at,
                ..TaskLinkedRow::default()
            },
        )?;
        Ok(self.tasks[id].clone())
    }

    pub(crate) fn remove_link(&mut self, by: &str, id: &str, r: &str) -> Result<Task> {
        let by = caller(by)?;
        let r = trim_space(r).to_string();
        let t = self.any_task(id)?;
        if !self.may_manage(t, &by) {
            return Err(not_allowed(format!("remove a link from task {id}")));
        }
        let mut i = t.links.iter().position(|l| l.reference == r);
        if i.is_none()
            && !r.is_empty()
            && let Ok(l) = parse_link(&r)
        {
            i = t.links.iter().position(|x| x.reference == l.reference);
        }
        let Some(i) = i else {
            return Err(not_found(format!("task {id} has no link {}", quote(&r))));
        };
        let link = t.links[i].clone();
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_LINKED,
            &TaskLinkedRow {
                task_id: id.into(),
                link,
                removed: true,
                by: by.clone(),
                at,
            },
        )?;
        Ok(self.tasks[id].clone())
    }

    // ---- refusals ----

    fn refusal_active(&self, r: &RefusalRec, now: &GoTime) -> bool {
        self.held.values().any(|l| {
            l.lease.holder == r.r.held_by
                && gotime::after(&l.lease.until, now)
                && !gotime::after(&l.lease.acquired_at, &r.r.at)
                && conflicts(&l.res, &r.res)
        })
    }

    pub(crate) fn gc_refusals(&mut self, now: &GoTime) {
        let rs = std::mem::take(&mut self.refusals);
        self.refusals = rs
            .into_iter()
            .filter(|r| self.refusal_active(r, now))
            .collect();
    }

    /// Logs a refused Acquire: one row per holder in the way, skipping an
    /// active duplicate.
    pub(crate) fn refuse(
        &mut self,
        by: &str,
        res: &Resource,
        victims: &[(String, String, GoTime)],
        now: GoTime,
    ) -> Result<()> {
        let mut done = HashSet::new();
        for (resource, holder, until) in victims {
            if !done.insert(holder.clone()) {
                continue;
            }
            let dup = self.refusals.iter().any(|r| {
                r.r.resource == res.name
                    && r.r.by == by
                    && r.r.held_by == *holder
                    && self.refusal_active(r, &now)
            });
            if dup {
                continue;
            }
            self.write(
                by,
                kinds::LEASE_REFUSED,
                &Refusal {
                    resource: res.name.clone(),
                    by: by.into(),
                    held_by: holder.clone(),
                    held: resource.clone(),
                    until: *until,
                    at: now,
                    conflict: false,
                },
            )?;
        }
        Ok(())
    }

    /// Active refusals, newest first.
    pub(crate) fn refusals_list(&self) -> Vec<Refusal> {
        let mut out: Vec<Refusal> = self.refusals.iter().rev().map(|r| r.r.clone()).collect();
        out.sort_by(|a, b| gotime::cmp(&b.at, &a.at));
        out
    }

    // ---- board ----

    /// Whether an unexpired lease conflicts with resource `name`.
    pub(crate) fn lease_held(&self, name: &str, now: &GoTime) -> bool {
        let res = parse_resource(name).unwrap_or_else(|_| Resource::named(name));
        self.held
            .values()
            .any(|l| gotime::after(&l.lease.until, now) && conflicts(&l.res, &res))
    }

    /// The open items of a blocked task whose lease or task went away.
    pub(crate) fn gone_away(&self, t: &Task, now: &GoTime) -> (Vec<String>, Vec<String>) {
        let mut ids = Vec::new();
        let mut why = Vec::new();
        if t.state != BLOCKED {
            return (ids, why);
        }
        for b in &t.blockers {
            let Some(on) = b.on.as_ref().filter(|_| b.open()) else {
                continue;
            };
            match on.kind.as_str() {
                BLOCKED_ON_LEASE => {
                    if !self.lease_held(&on.reference, now) {
                        ids.push(b.id.clone());
                        why.push("lease released".to_string());
                    }
                }
                BLOCKED_ON_TASK => {
                    if let Some(o) = self.tasks.get(&on.reference)
                        && is_terminal(&o.state)
                    {
                        ids.push(b.id.clone());
                        why.push(format!("task {}", o.state));
                    }
                }
                _ => {}
            }
        }
        (ids, why)
    }

    fn overlaps_of(&self, t: &Task, now: &GoTime) -> Vec<String> {
        if !t.shared || t.holders.len() < 2 {
            return Vec::new();
        }
        let mine: Vec<_> = self
            .held
            .values()
            .filter(|l| gotime::after(&l.lease.until, now) && is_holder(t, &l.lease.holder))
            .collect();
        let mut out = Vec::new();
        for (i, a) in mine.iter().enumerate() {
            for b in &mine[i + 1..] {
                if a.lease.holder != b.lease.holder && conflicts(&a.res, &b.res) {
                    out.push(format!("{} ~ {}", a.lease.resource, b.lease.resource));
                }
            }
        }
        out
    }

    pub(crate) fn board(&mut self, f: &BoardFilter) -> BoardView {
        let now = self.stamp();
        self.prune(&now);
        let mut v = BoardView::default();
        let limit = if f.done_limit <= 0 {
            DEFAULT_DONE_CARD
        } else {
            f.done_limit
        }
        .min(MAX_DONE_CARD);
        let viewer = trim_space(&f.viewer).to_string();
        let agent_viewer = !viewer.is_empty() && !self.is_owner(&viewer);
        let mut blocks: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for t in self.tasks.values() {
            if !f.repo_key.is_empty() && t.repo_key != f.repo_key {
                continue;
            }
            let mut card = Card {
                task: t.clone(),
                overlaps: self.overlaps_of(t, &now),
                ..Card::default()
            };
            if let Some(last) = self.by_task.get(&t.id).and_then(|ids| ids.last()) {
                card.last_comment = self.comments.get(last).cloned();
            }
            let (ids, _) = self.gone_away(t, &now);
            if !ids.is_empty() {
                if ids.len() == count_open(t) {
                    card.resolved = true;
                } else {
                    card.gone = ids;
                }
            }
            for bl in &t.blockers {
                let Some(on) = bl.on.as_ref() else { continue };
                if t.state != BLOCKED || !bl.open() || on.kind != BLOCKED_ON_LEASE {
                    continue;
                }
                if let Ok(res) = parse_resource(&on.reference) {
                    for (name, l) in &self.held {
                        if gotime::after(&l.lease.until, &now) && conflicts(&l.res, &res) {
                            let b = blocks.entry(name.clone()).or_default();
                            if !b.contains(&t.id) {
                                b.push(t.id.clone());
                            }
                        }
                    }
                }
            }
            let state = effective_state(t);
            if is_terminal(state) {
                v.done.push(card);
            } else if self.needs_you(t, &viewer, agent_viewer) {
                v.needs_you.push(card);
            } else if state == BLOCKED {
                v.blocked.push(card);
            } else if !t.holders.is_empty() {
                v.working.push(card);
            } else {
                v.open.push(card);
            }
        }
        for g in [
            &mut v.needs_you,
            &mut v.blocked,
            &mut v.working,
            &mut v.open,
            &mut v.done,
        ] {
            g.sort_by(|a, b| {
                gotime::cmp(&b.task.updated_at, &a.task.updated_at)
                    .then_with(|| b.task.id.cmp(&a.task.id))
            });
        }
        v.done_total = v.done.len() as i64;
        v.done.truncate(limit as usize);
        let mut refused: BTreeMap<String, Vec<Refusal>> = BTreeMap::new();
        for r in self.refusals.iter().rev() {
            for (name, l) in &self.held {
                if l.lease.holder == r.r.held_by
                    && gotime::after(&l.lease.until, &now)
                    && !gotime::after(&l.lease.acquired_at, &r.r.at)
                    && conflicts(&l.res, &r.res)
                {
                    refused.entry(name.clone()).or_default().push(r.r.clone());
                }
            }
        }
        for (name, l) in &self.held {
            let bl = blocks.get(name).cloned().unwrap_or_default();
            let rf = refused.get(name).cloned().unwrap_or_default();
            if !gotime::after(&l.lease.until, &now) || bl.is_empty() && rf.is_empty() {
                continue;
            }
            if !f.repo_key.is_empty()
                && bl.is_empty()
                && l.res.kind == RESOURCE_PATH
                && l.res.repo_key != f.repo_key
            {
                continue;
            }
            let mut lc = LeaseCard {
                lease: l.lease.clone(),
                refused: rf,
                blocks: bl,
            };
            lc.blocks.sort();
            lc.refused.sort_by(|a, b| gotime::cmp(&b.at, &a.at));
            v.leases.push(lc);
        }
        v.leases.sort_by(|a, b| {
            gotime::cmp(&a.lease.until, &b.lease.until)
                .then_with(|| a.lease.resource.cmp(&b.lease.resource))
        });
        v
    }

    /// The NeedsYou rule (coord.md §3.15).
    fn needs_you(&self, t: &Task, viewer: &str, agent_viewer: bool) -> bool {
        let mut ons: Vec<&BlockedBy> = Vec::new();
        let mut asks: Vec<&BlockedBy> = Vec::new();
        if t.state == BLOCKED {
            for b in &t.blockers {
                if let Some(on) = b.on.as_ref().filter(|_| b.open()) {
                    ons.push(on);
                    if b.status != BLOCKER_ANSWERED {
                        asks.push(on);
                    }
                }
            }
        }
        if agent_viewer {
            if t.state != BLOCKED && is_holder(t, viewer) && !engaged(t, viewer) {
                return true;
            }
            if ons
                .iter()
                .any(|bb| bb.kind == BLOCKED_ON_AGENT && bb.reference == viewer)
            {
                return true;
            }
            return t.blockers.iter().any(|b| {
                b.open()
                    && b.status == BLOCKER_ANSWERED
                    && (b.sent_to == viewer || b.sent_to.is_empty() && b.raised_by == viewer)
            });
        }
        if t.state == INPUT_REQUIRED {
            return true;
        }
        for bb in asks {
            if bb.kind == BLOCKED_ON_PERSON
                && (viewer.is_empty()
                    || bb.reference.is_empty()
                    || bb.reference == viewer
                    || self.is_owner(&bb.reference) && self.is_owner(viewer))
            {
                return true;
            }
        }
        if viewer.is_empty() {
            return false;
        }
        if self.is_owner(viewer) {
            return t.holders.iter().any(|h| self.is_owner(h));
        }
        is_holder(t, viewer)
    }

    // ---- sweep ----

    /// Returns what it built before any failed write, with the failure
    /// (Go returns both): a notice for a row already written still goes.
    pub(crate) fn sweep_notices(&mut self) -> (Vec<Notice>, Result<()>) {
        let now = self.stamp();
        if let Err(e) = self.release_gone(&now) {
            return (Vec::new(), Err(e));
        }
        let mut out = Vec::new();
        let r = self.sweep_rest(&now, &mut out);
        (out, r)
    }

    fn sweep_rest(&mut self, now: &GoTime, out: &mut Vec<Notice>) -> Result<()> {
        let now = *now;
        self.return_gone(&now, out)?;
        let todo: Vec<String> = self
            .tasks
            .values()
            .filter(|t| !self.gone_away(t, &now).0.is_empty())
            .map(|t| t.id.clone())
            .collect();
        for id in todo {
            let t = self.tasks[&id].clone();
            let (ids, why) = self.gone_away(&t, &now);
            if ids.len() == count_open(&t) {
                out.push(self.close_blocker(SWEEPER, &id, &why[0], now)?);
                continue;
            }
            let raisers: Vec<String> = t
                .blockers
                .iter()
                .filter(|b| ids.contains(&b.id))
                .map(|b| b.raised_by.clone())
                .collect();
            for (i, bid) in ids.iter().enumerate() {
                self.write(
                    SWEEPER,
                    kinds::BLOCKER_RESOLVED,
                    &BlockerResolvedRow {
                        task_id: id.clone(),
                        blocker_id: bid.clone(),
                        answer: why[i].clone(),
                        option: -1,
                        by: SWEEPER.into(),
                        closed: true,
                        at: now,
                        ..BlockerResolvedRow::default()
                    },
                )?;
            }
            let nt = self.tasks[&id].clone();
            let mut n = Notice {
                kind: NOTICE_ANSWERED.into(),
                task_id: id.clone(),
                reference: format!("task:{id}"),
                title: t.title.clone(),
                by: SWEEPER.into(),
                ..Notice::default()
            };
            for r in raisers.into_iter().chain(self.recipients(&nt, SWEEPER)) {
                if !r.is_empty() && !self.is_owner(&r) && r != SWEEPER && !n.to.contains(&r) {
                    n.to.push(r);
                }
            }
            let items: Vec<String> = ids
                .iter()
                .zip(&why)
                .map(|(i, w)| format!("{i} ({w})"))
                .collect();
            n.text = format!(
                "Irori closed {} on {} ({}): what it waited on went away. {} more blockers are still open; board_unblock with blocker_id answers one.{}",
                items.join(", "),
                quote(&nt.title),
                nt.refs(),
                count_open(&nt),
                deeper(&id)
            );
            if !n.to.is_empty() {
                out.push(n);
            }
        }
        out.extend(self.watch_notices());
        Ok(())
    }

    /// Takes each holder of this device gone for HolderGrace off its cards.
    fn return_gone(&mut self, now: &GoTime, out: &mut Vec<Notice>) -> Result<()> {
        let mut live = HashSet::new();
        if let Some(r) = &self.roster {
            for a in r.agents() {
                if a.status != "gone" {
                    live.insert(a.key);
                }
            }
        }
        let ids: Vec<String> = self
            .tasks
            .iter()
            .filter(|(_, t)| {
                !is_terminal(&t.state) && t.open_ask().is_none() && !t.holders.is_empty()
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            let holders = self.tasks[&id].holders.clone();
            for h in holders {
                let Some(first) = self.first_absent.get(&h).copied() else {
                    continue;
                };
                if !gotime::at_least(now, &first, self.holder_grace)
                    || self.is_owner(&h)
                    || !self.local_key(&h)
                {
                    continue;
                }
                let t = self.tasks[&id].clone();
                let last = t.holders.len() == 1;
                let creator = t.created_by.clone();
                let person =
                    last && (self.is_owner(&creator) || creator == h || !live.contains(&creator));
                self.write(
                    SWEEPER,
                    kinds::TASK_LEFT,
                    &TaskMemberRow {
                        task_id: id.clone(),
                        who: h.clone(),
                        by: SWEEPER.into(),
                        at: *now,
                        reason: LEFT_GONE.into(),
                        person,
                    },
                )?;
                if !last {
                    continue;
                }
                if self.tasks[&id].state == WORKING {
                    self.write(
                        SWEEPER,
                        kinds::TASK_STATUS,
                        &TaskStateRow {
                            task_id: id.clone(),
                            state: SUBMITTED.into(),
                            note: "nobody is on it: the agent working it went away".into(),
                            by: SWEEPER.into(),
                            at: *now,
                            ..TaskStateRow::default()
                        },
                    )?;
                }
                if !person {
                    out.push(Notice {
                        kind: NOTICE_RETURNED.into(),
                        task_id: id.clone(),
                        reference: format!("task:{id}"),
                        title: t.title.clone(),
                        by: SWEEPER.into(),
                        to: vec![creator],
                        text: format!(
                            "Nobody is on {} ({}) any more: {h} went away, and it's open again. Take it with board_join, hand it on with board_assign, or close it with board_status.{}",
                            quote(&t.title),
                            t.refs(),
                            deeper(&id)
                        ),
                        ..Notice::default()
                    });
                }
            }
        }
        Ok(())
    }
}
