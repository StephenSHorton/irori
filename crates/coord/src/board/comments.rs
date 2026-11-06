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
    Fold, LEFT_FINISHED, LEFT_GONE, LEFT_REMOVED, RefusalRec, engaged, is_holder, may_record,
};
use super::links::parse_link;
use super::resource::{RESOURCE_PATH, Resource, conflicts, normalize_resource, parse_resource};
use super::rows::*;
use super::types::*;
use super::util::{
    MAX_NOTICE_FIELD, SWEEPER, args, caller, check_key, check_text, contains, cut, deeper,
    multiline, new_id, runes,
};
use super::{
    DEFAULT_DONE_CARD, MAX_ATTACHMENTS, MAX_COMMENT, MAX_DONE_CARD, MAX_HANDOFF_NOTE,
    MAX_PERSON_TEXT, MAX_REASON, MAX_REF, REOPEN_COMMENT_NOTE,
};
use crate::gotime;
use crate::names::trim_space;

/// Why the sweep closes an answered item whose agent left (card #481).
const ANSWERED_LEFT: &str = "the agent it went to left the card";

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

/// Checks text the person typed: at most [`MAX_PERSON_TEXT`] bytes.
pub(crate) fn check_person_text(what: &str, s: &str, min: usize) -> Result<()> {
    if runes(s) < min {
        return Err(invalid(format!("{what} is empty")));
    }
    if s.len() > MAX_PERSON_TEXT {
        return Err(invalid(format!(
            "{what} longer than {} KiB",
            MAX_PERSON_TEXT >> 10
        )));
    }
    Ok(())
}

/// Trims and checks a comment's caller, text and files. The person's
/// comment may be up to [`MAX_PERSON_TEXT`] bytes; an agent's stays at
/// [`MAX_COMMENT`] characters.
pub(crate) fn check_comment(
    by: &str,
    text: &str,
    files: &[FileRef],
    person: bool,
) -> Result<(String, String)> {
    let by = caller(by)?;
    let text = trim_space(text).to_string();
    if text.is_empty() {
        return Err(invalid("an empty comment"));
    }
    if person {
        check_person_text("comment", &text, 1)?;
    } else if runes(&text) > MAX_COMMENT {
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
                // One lease, or several by space (card #339): it waits until
                // none of them is held.
                let now = self.stamp();
                let mut names = Vec::new();
                for r in out.reference.split_whitespace() {
                    let n = normalize_resource(r)
                        .map_err(|e| invalid(format!("blocked on a lease: {e}")))?;
                    if !self.lease_held(&n, &now) {
                        return Err(invalid(format!(
                            "nobody leases {n}; block on external or a person instead"
                        )));
                    }
                    if !names.contains(&n) {
                        names.push(n);
                    }
                }
                if names.is_empty() {
                    return Err(invalid("blocked on a lease needs the resource"));
                }
                out.reference = names.join(" ");
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
                if let Some(w) = work_done(o) {
                    return Err(invalid(format!("{} is already {w}", o.short_ref())));
                }
            }
            BLOCKED_ON_PERSON => {
                // On a shared board a question can be for another member
                // (card #264): their person:<id>.
                let member = self.on_board
                    && out.reference.starts_with(kinds::PERSON_ACTOR_PREFIX)
                    && !self.person_of(&out.reference).is_empty();
                if !out.reference.is_empty() && !member && !self.is_owner(&out.reference) {
                    return Err(invalid("blocked on a person needs \"\" or a user: key"));
                }
            }
            BLOCKED_ON_AGENT => {
                if out.reference.is_empty() || self.is_owner(&out.reference) {
                    return Err(invalid("blocked on an agent needs its key"));
                }
            }
            BLOCKED_ON_EXTERNAL => {
                // An external wait that names leases held now ("a free
                // ww/dolphin-slot-1..4 lease") waits on those leases, so it
                // closes when the last of them goes (card #339). On a board
                // the leases are this person's private ones, which never
                // show on the board: the wait stays external there and
                // their own devices judge it (outside_wait_gone).
                let names = self.leases_named_in(&out.reference, &self.stamp());
                if !names.is_empty() {
                    out.kind = BLOCKED_ON_LEASE.into();
                    out.reference = names.join(" ");
                }
            }
            BLOCKED_ON_VERSION => {
                // A board's log keeps no device.started rows (card #485):
                // without a hook to the person's own, it could never close.
                if self.on_board && self.device_version.is_none() {
                    return Err(invalid(
                        "a version wait goes on a private card; in a shared space, wait on external or the person",
                    ));
                }
                out.reference = self.check_version_wait(&out.reference)?;
            }
            _ => {
                return Err(invalid(format!(
                    "blocker kind {} (lease|task|person|agent|external|version)",
                    quote(&bb.kind)
                )));
            }
        }
        Ok(Some(out))
    }

    // ---- assign, join, leave, share ----

    pub(crate) fn assign_notice(
        &mut self,
        by: &str,
        id: &str,
        to: &str,
        note: &str,
    ) -> Result<(Task, Notice)> {
        let note = trim_space(note).to_string();
        check_text("the hand-off note", &note, 0, MAX_HANDOFF_NOTE)?;
        let (t, wrote) = self.assign(by, id, to, &note)?;
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
            // An idle assignee is woken; a busy one hears at its next turn.
            wake: self.idle(&to),
            ..Notice::default()
        };
        // A person isn't a mailbox: their Irori asks them (card #573).
        if !self.is_owner(&to) && to != by && !to.starts_with(kinds::PERSON_ACTOR_PREFIX) {
            n.to = vec![to.clone()];
        }
        let mut b = format!(
            "{} assigned you a task: {} ({}).",
            self.named(&by),
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
        b.push_str(&t.handover_lines());
        if !note.is_empty() {
            b.push_str(&format!(
                "\nHand-off note: {}",
                cut(&note, MAX_NOTICE_FIELD)
            ));
        }
        if contains(&t.assigned, &to) {
            b.push_str("\nClaim it to start.");
        }
        b.push_str(&deeper(&t.id));
        n.text = b;
        Ok((t, n))
    }

    /// Whether the roster shows `key` live and idle; unknown is not.
    pub(crate) fn idle(&self, key: &str) -> bool {
        self.roster.as_ref().is_some_and(|r| {
            r.agents()
                .iter()
                .any(|a| a.key == key && a.status == "idle")
        })
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
        // The person may add anyone; an agent joining on its own keeps to
        // its device's sides (card #496).
        if by == who {
            self.check_side_take(&who, t)?;
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
            // A card Irori handed `who` and it hasn't taken: it goes to
            // the next free agent (card #82).
            if self.pass_offer(&who, id)? {
                return Ok(self.tasks[id].clone());
            }
            return Err(not_found(format!("{who} isn't on task {id}")));
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_LEFT,
            &TaskMemberRow {
                task_id: id.into(),
                who: who.clone(),
                by: by.clone(),
                at,
                ..TaskMemberRow::default()
            },
        )?;
        // The last holder of a private working card handed it back (card
        // #338): it goes up again, and Irori hands it to the next free
        // agent. A shared card keeps its state for whoever joins next.
        let t = &self.tasks[id];
        if t.holders.is_empty() && t.state == WORKING && t.subtasks.is_empty() && !t.shared {
            let note = format!("{}: {} left it", super::fold::NOBODY_NOTE, self.named(&who));
            self.write(
                SWEEPER,
                kinds::TASK_STATUS,
                &TaskStateRow {
                    task_id: id.into(),
                    state: SUBMITTED.into(),
                    note,
                    by: SWEEPER.into(),
                    at,
                    ..TaskStateRow::default()
                },
            )?;
        }
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
            return Err(invalid("taking a member off is for a shared space"));
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
            // A card given to them comes back though nobody of theirs holds
            // it now (card #573).
            if let Some(g) = t.given.as_ref().filter(|g| !keys.contains(&g.key)) {
                keys.push(g.key.clone());
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
                        note: format!(
                            "{}: the person whose agent worked it left the space",
                            super::fold::NOBODY_NOTE
                        ),
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
            false,
            false,
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
        let restored = self.restore_for(&t);
        self.write(
            by,
            kinds::BLOCKER_CLOSED,
            &BlockerClosedRow {
                task_id: id.into(),
                by: by.into(),
                note: note.into(),
                restored: restored.into(),
                at: now,
                // What this device sees open, and so closes (card #635).
                blocker_ids: t
                    .blockers
                    .iter()
                    .filter(|b| b.open())
                    .map(|b| b.id.clone())
                    .collect(),
            },
        )?;
        let nt = self.tasks[id].clone();
        let who = if by == SWEEPER {
            "Irori".to_string()
        } else {
            self.named(by)
        };
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
            .chain(&t.co_holders)
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
        self.add_comment_to(by, id, text, wake, files, "")
    }

    /// [`Fold::add_comment`] addressed to one key on the card (the row's
    /// `to`), as a lease release request is to the lease's holder (§5.4):
    /// the bridge carries it to that agent only.
    pub(crate) fn add_comment_to(
        &mut self,
        by: &str,
        id: &str,
        text: &str,
        wake: bool,
        files: Vec<FileRef>,
        to: &str,
    ) -> Result<(Comment, Notice)> {
        self.add_comment_full(by, id, text, wake, files, to, &[])
    }

    /// [`Fold::add_comment_to`] with the comment's mentions (checked).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_comment_full(
        &mut self,
        by: &str,
        id: &str,
        text: &str,
        wake: bool,
        files: Vec<FileRef>,
        to: &str,
        mentions: &[Mention],
    ) -> Result<(Comment, Notice)> {
        let (by, text) = check_comment(by, text, &files, self.is_owner(trim_space(by)))?;
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
                to: to.into(),
                mentions: mentions.to_vec(),
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
                "{} commented on {} ({}):\n{text}{}",
                self.named(&by),
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
        for h in &t.co_holders {
            if ok(h, &answer, &watch) {
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn person_comment(
        &mut self,
        by: &str,
        id: &str,
        text: &str,
        wake: bool,
        mentions: &[Mention],
        files: Vec<FileRef>,
        around: Option<&dyn Fn(&str) -> bool>,
    ) -> Result<(Comment, Vec<Notice>)> {
        // The files go on the comment and with every notice about it, so
        // whoever hears it can fetch them with attachment_get (card #124).
        let (cm, mut ns) =
            self.person_comment_notices(by, id, text, wake, mentions, &files, around)?;
        if !files.is_empty() {
            for n in &mut ns {
                n.attachments = files.clone();
            }
        }
        Ok((cm, ns))
    }

    #[allow(clippy::too_many_arguments)]
    fn person_comment_notices(
        &mut self,
        by: &str,
        id: &str,
        text: &str,
        wake: bool,
        mentions: &[Mention],
        files: &[FileRef],
        around: Option<&dyn Fn(&str) -> bool>,
    ) -> Result<(Comment, Vec<Notice>)> {
        let (by, text) = check_comment(by, text, files, self.is_owner(trim_space(by)))?;
        if !self.is_owner(&by) {
            return Err(invalid("only the person comments from the app"));
        }
        let ms = self.check_mentions(&by, mentions)?;
        let mentioned = |k: &str| ms.iter().any(|m| m.key == k);
        let live_mention = ms.iter().any(|m| !m.gone);
        let t = self.any_task(id)?.clone();
        // A thread with "Agents answer now" off (card #391) wakes nobody,
        // here or through another member's bridge, which reads the row.
        let wake = wake && !t.open_ask().is_some_and(|a| a.thread && !a.wake);
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
        // Who answers is decided before the mentioned leave the lists: a
        // mentioned holder still answers it, so watchers aren't woken in
        // its place.
        let answerers = answer.clone();
        answer.retain(|k| !mentioned(k));
        watch.retain(|k| !mentioned(k));
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
                unheard: answerers.is_empty() && watch.is_empty() && !live_mention,
                mentions: ms.clone(),
                attachments: files.to_vec(),
                ..TaskCommentRow::default()
            },
        )?;
        let mn = self.mention_notice(&by, id, &ms, &text, &format!("comment:{cid}"), &was)?;
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
        if let Some(a) = t.open_ask().filter(|a| a.thread) {
            let quiet = !wake || !a.wake;
            let mut ns = self.thread_follow_up(&t, &by, &text, &cid, wake, around)?;
            // Agents answer now off (card #391): an agent it names is told
            // at its next turn, not woken.
            ns.extend(mn.into_iter().map(|mut n| {
                n.wake &= !quiet;
                n
            }));
            return Ok((cm, ns));
        }
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
            let mut ns = vec![a];
            ns.extend(mn);
            return Ok((cm, ns));
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
            w.wake = wake && answerers.is_empty();
            w.watching = true;
            w.text = format!("{head}, a card you watch:\n{text}\n");
            if !answerers.is_empty() {
                w.text.push_str(&format!(
                    "{} answers it; answer on the card only if it asks you.",
                    answerers.join(", ")
                ));
            } else {
                w.text.push_str("Nobody else is on it: if it asks something you can answer, answer on the card with board_comment.");
            }
            w.text.push_str(&deeper(id));
            ns.push(w);
        }
        ns.extend(mn);
        Ok((cm, ns))
    }

    /// Appends a task.comment row as actor; returns the comment it folded to.
    /// On a board the row keeps only mentions in board form (`agent:<cardId>`,
    /// `person:<personId>`), which every member's Core resolves (card #177;
    /// board.md "Mentions"): a private key never goes in a board log.
    pub(crate) fn write_comment(&mut self, actor: &str, mut r: TaskCommentRow) -> Result<Comment> {
        if self.on_board {
            r.mentions.retain(|m| super::mention::board_form(&m.key));
        }
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
        } else if self.is_owner(&from) && text.len() <= MAX_PERSON_TEXT {
            // The person's own message goes on the thread whole (card #265).
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
                mentions: m.mentions.clone(),
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

    /// The live leases text names (card #339), from `names`: each by its
    /// whole name, or a numbered one (`ww/dolphin-slot-3`) by its family
    /// (`ww/dolphin-slot-1..4` when 3 is in range, `ww/dolphin-slot-N`,
    /// `ww/dolphin-slot-*`). At most what fits a blocker's ref.
    fn named_of(text: &str, names: impl IntoIterator<Item = String>) -> Vec<String> {
        let mut names: Vec<String> = names.into_iter().filter(|n| names_lease(text, n)).collect();
        names.sort();
        names.dedup();
        let mut out: Vec<String> = Vec::new();
        let mut len = 0;
        for n in names {
            let add = runes(&n) + usize::from(!out.is_empty());
            if len + add > MAX_REF {
                break;
            }
            len += add;
            out.push(n);
        }
        out
    }

    /// The leases of this log held now that text names.
    pub(crate) fn leases_named_in(&self, text: &str, now: &GoTime) -> Vec<String> {
        let live = self
            .held
            .values()
            .filter(|l| gotime::after(&l.lease.until, now))
            .map(|l| l.lease.resource.clone());
        Self::named_of(text, live.collect::<Vec<_>>())
    }

    /// On a board, the private leases of this person (see
    /// [`super::Options::outside_leases`]) held now that text names.
    fn outside_named_in(&self, text: &str) -> Vec<String> {
        match &self.outside_leases {
            Some(f) if self.on_board => Self::named_of(text, f()),
            _ => Vec::new(),
        }
    }

    /// Whether blocker b is one of this person's agents' on a board, so
    /// this device sees the private leases it may wait on (card #339).
    pub(crate) fn own_on_board(&self, b: &Blocker) -> bool {
        let mine = self.person_of(&self.person);
        self.on_board && !mine.is_empty() && self.person_of(&b.raised_by) == mine
    }

    /// Notes an external wait on a board that names this person's private
    /// leases held now (card #339): from then on the sweep closes it once
    /// none of them is held. Kept in memory on this device; a restart
    /// notes it again while one of its leases is still held.
    pub(crate) fn note_outside_wait(&self, task_id: &str, b: &Blocker) {
        let Some(on) = b.on.as_ref() else { return };
        if on.kind != BLOCKED_ON_EXTERNAL || !b.open() || !self.own_on_board(b) {
            return;
        }
        if !self.outside_named_in(&on.reference).is_empty() {
            self.outside_waits
                .borrow_mut()
                .insert(format!("{task_id}/{}", b.id));
        }
    }

    /// Whether blocker b's external wait on private leases is over: it
    /// named some held then (noted), and names none held now.
    fn outside_wait_gone(&self, task_id: &str, b: &Blocker) -> bool {
        let Some(on) = b.on.as_ref() else {
            return false;
        };
        if !self.own_on_board(b) {
            return false;
        }
        let key = format!("{task_id}/{}", b.id);
        if !self.outside_named_in(&on.reference).is_empty() {
            self.outside_waits.borrow_mut().insert(key);
            return false;
        }
        self.outside_waits.borrow().contains(&key)
    }

    /// Whether open item b of t was answered and waits on an agent that
    /// left the card since (card #481): nobody is left to act on the
    /// answer. Only an agent that held the card and left counts, not one
    /// that raised it from outside; and only this device of the raiser's
    /// person closes it, so the row folds everywhere (card #242).
    fn answered_left(&self, t: &Task, b: &Blocker) -> bool {
        // A done check or an ownership change is the person's to close,
        // never Irori's.
        if !b.open() || b.status != BLOCKER_ANSWERED || b.done_check || b.transfer {
            return false;
        }
        if !b.sent_to.is_empty() && b.sent_to != b.raised_by {
            return false;
        }
        let tgt = &b.raised_by;
        !tgt.is_empty()
            && tgt != SWEEPER
            && !self.is_owner(tgt)
            && contains(&t.left, tgt)
            && !contains(&t.holders, tgt)
            && !contains(&t.co_holders, tgt)
            && !contains(&t.assigned, tgt)
            && self.may_write(tgt, &self.dev)
    }

    /// The open items of a blocked task whose lease or task went away, or
    /// whose answer went to an agent that left the card.
    pub(crate) fn gone_away(&self, t: &Task, now: &GoTime) -> (Vec<String>, Vec<String>) {
        let mut ids = Vec::new();
        let mut why = Vec::new();
        if t.state != BLOCKED {
            return (ids, why);
        }
        for b in &t.blockers {
            if self.answered_left(t, b) {
                ids.push(b.id.clone());
                why.push(ANSWERED_LEFT.to_string());
                continue;
            }
            let Some(on) = b.on.as_ref().filter(|_| b.open()) else {
                continue;
            };
            match on.kind.as_str() {
                BLOCKED_ON_LEASE => {
                    let names: Vec<&str> = on.reference.split_whitespace().collect();
                    if !names.is_empty() && !names.iter().any(|n| self.lease_held(n, now)) {
                        ids.push(b.id.clone());
                        why.push(if names.len() > 1 {
                            "leases released".to_string()
                        } else {
                            "lease released".to_string()
                        });
                    }
                }
                BLOCKED_ON_TASK => {
                    if let Some(o) = self.tasks.get(&on.reference)
                        && let Some(w) = work_done(o)
                    {
                        ids.push(b.id.clone());
                        why.push(w);
                    }
                }
                BLOCKED_ON_VERSION if self.version_behind(&on.reference).is_empty() => {
                    let (min, devs) = super::version_wait::parse_version_wait(&on.reference);
                    ids.push(b.id.clone());
                    why.push(if devs.len() > 1 {
                        format!("{min} or later running on all {} devices", devs.len())
                    } else {
                        format!("{min} or later running")
                    });
                }
                BLOCKED_ON_EXTERNAL if self.outside_wait_gone(&t.id, b) => {
                    ids.push(b.id.clone());
                    why.push("leases released".to_string());
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
        let rs = self.roster_status();
        for t in self.tasks.values() {
            if !f.repo_key.is_empty() && !ledger::same_repo_key(&t.repo_key, &f.repo_key) {
                continue;
            }
            let mut card = Card {
                task: self.shown(t),
                overlaps: self.overlaps_of(t, &now),
                stranded: self.stranded_in(t, &rs),
                gone_waits: self.gone_waits_in(t, &rs),
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
                for r in on.reference.split_whitespace() {
                    let Ok(res) = parse_resource(r) else { continue };
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
            } else if self.needs_you(t, &viewer, agent_viewer, &rs) {
                v.needs_you.push(card);
            } else if state == BLOCKED && !t.all_answered() {
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
                && !ledger::same_repo_key(&l.res.repo_key, &f.repo_key)
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
        let dev = match irori_identity::owner::device_of(&viewer) {
            "" => self.dev.as_str(),
            d => d,
        };
        v.side_line = crate::sides::agent_line(self.sides.of(dev), &self.side_names());
        v
    }

    /// The NeedsYou rule (coord.md §3.15).
    fn needs_you(
        &self,
        t: &Task,
        viewer: &str,
        agent_viewer: bool,
        rs: &super::stranded::RosterStatus,
    ) -> bool {
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
        // Card #509: an external wait on a GitHub PR waits on the person's
        // merge (agents waited on theirs for days, unseen), until the PR is
        // known merged or closed. On a board, only their own agents' waits.
        if (viewer.is_empty() || self.is_owner(viewer)) && !self.merge_waits(t).is_empty() {
            return true;
        }
        // Card #520: a wait on an agent that is gone, or an outside wait its
        // gone agent raised, on a card nobody holds: nothing ends it but
        // the person (Hand to an agent, or stop the card).
        if (viewer.is_empty() || self.is_owner(viewer)) && !self.gone_waits_in(t, rs).is_empty() {
            return true;
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
        self.return_unheld_prs()?;
        self.tell_not_answering(out)?;
        self.tell_idle_holders(out);
        let todo: Vec<String> = self
            .tasks
            .values()
            .filter(|t| !self.gone_away(t, &now).0.is_empty())
            .map(|t| t.id.clone())
            .collect();
        for id in todo {
            let t = self.tasks[&id].clone();
            let (ids, why) = self.gone_away(&t, &now);
            // An answered item keeps the person's answer, so it goes one
            // by one, never through the close of the whole card.
            let left: Vec<bool> = ids
                .iter()
                .map(|bid| {
                    t.blockers
                        .iter()
                        .find(|b| &b.id == bid)
                        .is_some_and(|b| self.answered_left(&t, b))
                })
                .collect();
            if ids.len() == count_open(&t) && !left.contains(&true) {
                out.push(self.close_blocker(SWEEPER, &id, &why[0], now)?);
                continue;
            }
            // An agent that left the card (card #481) isn't told.
            let raisers: Vec<String> = t
                .blockers
                .iter()
                .filter(|b| ids.contains(&b.id) && !self.answered_left(&t, b))
                .map(|b| b.raised_by.clone())
                .collect();
            for (i, bid) in ids.iter().enumerate() {
                // The person's answer stands, with why Irori closed it.
                let (answer, option, note) = match left[i]
                    .then(|| super::cards::last_answer(self, &t, bid))
                    .flatten()
                {
                    Some(a) => (a.text.clone(), a.pick - 1, why[i].clone()),
                    None => (why[i].clone(), -1, String::new()),
                };
                self.write(
                    SWEEPER,
                    kinds::BLOCKER_RESOLVED,
                    &BlockerResolvedRow {
                        task_id: id.clone(),
                        blocker_id: bid.clone(),
                        answer,
                        option,
                        note,
                        by: SWEEPER.into(),
                        closed: true,
                        at: now,
                        ..BlockerResolvedRow::default()
                    },
                )?;
            }
            // Nobody holds it (its spawned agent went while it waited on
            // other work, card #474) and what is still open the person
            // answered: it goes back on the Board, as when its agent goes
            // after his answer (return_gone), and a fresh agent reads it.
            let held = &self.tasks[&id];
            if held.holders.is_empty() && held.assigned.is_empty() && held.all_answered() {
                self.back_on_board(SWEEPER, &id, "what it waited on went away")?;
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
        self.wait_notices(&now, out)?;
        Ok(())
    }

    /// Says once on each open card a not-answering agent of this device
    /// holds that it isn't answering (card #110), so the person sees it on
    /// the card, and tells the card's poster when that is another live
    /// agent. Told again only after it answered in between.
    fn tell_not_answering(&mut self, out: &mut Vec<Notice>) -> Result<()> {
        // Private Board only: a shared board's fold takes no comment from
        // Irori, and this device's roster says nothing about its members.
        if self.on_board {
            return Ok(());
        }
        let mut mute = HashSet::new();
        let mut live = HashSet::new();
        if let Some(r) = &self.roster {
            for a in r.agents() {
                if a.status == crate::roster::NOT_ANSWERING && self.local_key(&a.key) {
                    mute.insert(a.key);
                } else if crate::roster::answers(&a.status) {
                    live.insert(a.key);
                }
            }
        }
        self.mute_told.retain(|(k, _)| mute.contains(k));
        if mute.is_empty() {
            return Ok(());
        }
        let mut todo: Vec<(String, String)> = Vec::new();
        for t in self.tasks.values() {
            if is_terminal(&t.state) || t.open_ask().is_some() {
                continue;
            }
            for h in &t.holders {
                let pair = (h.clone(), t.id.clone());
                if mute.contains(h) && !self.mute_told.contains(&pair) {
                    todo.push(pair);
                }
            }
        }
        todo.sort();
        let mins = self.grace.as_secs().div_ceil(60);
        let back = self.holder_grace.as_secs().div_ceil(60);
        for (h, id) in todo {
            let name = self.named(&h);
            let text = format!(
                "{name} isn't answering: a wake has waited on it unread, though its session still runs. Irori hands it no new cards or asks. Any agent can take this card over with board_join after {mins} minutes; after {back} it goes back on the Board."
            );
            let at = self.stamp();
            self.write_comment(
                SWEEPER,
                TaskCommentRow {
                    task_id: id.clone(),
                    comment_id: new_id(&at),
                    by: SWEEPER.into(),
                    text: text.clone(),
                    at,
                    ..TaskCommentRow::default()
                },
            )?;
            self.mute_told.insert((h.clone(), id.clone()));
            let t = self.tasks[&id].clone();
            let poster = t.created_by.clone();
            if poster != h && !self.is_owner(&poster) && live.contains(&poster) {
                out.push(Notice {
                    kind: NOTICE_RETURNED.into(),
                    task_id: id.clone(),
                    reference: format!("task:{id}"),
                    title: t.title.clone(),
                    by: SWEEPER.into(),
                    to: vec![poster],
                    text: format!(
                        "{name}, which holds your card {} ({}), isn't answering. Take it over with board_join after {mins} minutes, or leave it: after {back} it goes back on the Board.{}",
                        quote(&t.title),
                        t.refs(),
                        deeper(&id)
                    ),
                    ..Notice::default()
                });
            }
        }
        Ok(())
    }

    /// Takes each holder of this device gone for HolderGrace off its cards.
    fn return_gone(&mut self, now: &GoTime, out: &mut Vec<Notice>) -> Result<()> {
        let mut live = HashSet::new();
        let mut mute = HashSet::new();
        // Agents whose session ended (card #338): wound down or closed, not
        // merely missing for a restart. They come off their cards after
        // ENDED_GRACE (time to --resume), not the holder grace.
        let mut ended: HashSet<String> = self.gone_rows.keys().cloned().collect();
        // An agent Irori ended (card #446) is ended once it is off the
        // roster, even when its agent.gone came while presence still
        // listed it.
        ended.extend(self.ended_by_irori.iter().cloned());
        if let Some(r) = &self.roster {
            for a in r.agents() {
                if crate::roster::answers(&a.status) {
                    ended.remove(&a.key);
                    live.insert(a.key);
                } else if a.status == crate::roster::NOT_ANSWERING {
                    mute.insert(a.key);
                } else if a.status == "gone" {
                    ended.insert(a.key);
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
                // Only a holder the fold lets this device take off: on a
                // board an `agent:<card>` key names no device, so
                // local_key alone holds for every member's agents, and a
                // task.left for another person's agent folds to nothing
                // and was written again every sweep (card #242).
                let grace = if ended.contains(&h) {
                    super::ENDED_GRACE
                } else {
                    self.holder_grace
                };
                if !gotime::at_least(now, &first, grace)
                    || self.is_owner(&h)
                    || !self.local_key(&h)
                    || !self.may_write(&h, &self.dev)
                {
                    continue;
                }
                let t = self.tasks[&id].clone();
                let last = t.holders.len() == 1;
                let went = if mute.contains(&h) {
                    "stopped answering"
                } else {
                    "went away"
                };
                let creator = t.created_by.clone();
                // An agent Irori started for the card (card #446) finished:
                // nobody is told to take it.
                let finished = self.spawned_for(&h).is_some() && self.ended_by_irori.contains(&h);
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
                        reason: if finished { LEFT_FINISHED } else { LEFT_GONE }.into(),
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
                            note: format!(
                                "{}: the agent working it {went}",
                                super::fold::NOBODY_NOTE
                            ),
                            by: SWEEPER.into(),
                            at: *now,
                            ..TaskStateRow::default()
                        },
                    )?;
                } else if self.tasks[&id].all_answered() {
                    // The person already answered everything it waited on (a
                    // "Not done", a blocker answer) and its agent went before
                    // acting on it: nobody is left to, so it goes back on the
                    // Board (card #447).
                    self.back_on_board(
                        SWEEPER,
                        &id,
                        &format!("the person answered and the agent working it {went}"),
                    )?;
                }
                self.stop_crashed_spawns(&id, now)?;
                if !person && !finished {
                    out.push(Notice {
                        kind: NOTICE_RETURNED.into(),
                        task_id: id.clone(),
                        reference: format!("task:{id}"),
                        title: t.title.clone(),
                        by: SWEEPER.into(),
                        to: vec![creator],
                        text: format!(
                            "Nobody is on {} ({}) any more: {} {went}, and it's open again. Take it with board_join, hand it on with board_assign, or close it with board_status.{}",
                            quote(&t.title),
                            t.refs(),
                            self.named(&h),
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

impl Fold {
    /// On a card whose spawned agents crashed as many times in a row as
    /// the crash limit allows, stops its spawning by asking the person (§2 "Crash", card
    /// #447): a card that kills its agent doesn't loop. Their answer puts
    /// it back on the Board (back_on_board) and clears the run, even one
    /// sent as a resolve, as the window does (card #540). Only on the
    /// device that hands the card out: the limit is each device's own
    /// (#492) and the count is shared, so its number governs the card
    /// wherever the agent ran, and the Core's sweep asks there (card #576).
    pub(crate) fn stop_crashed_spawns(&mut self, id: &str, now: &GoTime) -> Result<()> {
        let Some(t) = self.tasks.get(id).cloned() else {
            return Ok(());
        };
        let limit = self.crash_limit();
        if t.spawn_crashes < limit
            || is_terminal(&t.state)
            || !self.hands_out(&t)
            || t.blockers
                .iter()
                .any(|b| b.open() && super::spawn::is_crash_text(&b.question))
        {
            return Ok(());
        }
        let last = match limit {
            1 => "The last agent".to_string(),
            2 => "The last two agents".to_string(),
            n => format!("The last {n} agents"),
        };
        self.raise_locked(
            SWEEPER,
            &t,
            "",
            &super::spawn::crash_question(limit),
            &format!("{last} started for this card went away before finishing. Irori won't start another until you say so: start one, or say in your own words what to try, and it goes in the next agent's brief."),
            vec![BlockerOption {
                answer: super::spawn::CRASHED_START.into(),
                why: "A fresh agent picks it up from Where and Left off.".into(),
                by: SWEEPER.into(),
                at: *now,
                recommended: true,
                link: None,
            }],
            None,
            false,
            *now,
            Vec::new(),
            Vec::new(),
            false,
            false,
        )?;
        Ok(())
    }
}

/// Whether text names lease `name` (card #339): the whole name as a token,
/// or, for a numbered name (`ww/dolphin-slot-3`), its family:
/// `ww/dolphin-slot-N` (or `n`, `*`), or a range holding it (`-1..4`).
pub(crate) fn names_lease(text: &str, name: &str) -> bool {
    fn name_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | ':')
    }
    let starts_token = |i: usize| i == 0 || !text[..i].chars().next_back().is_some_and(name_char);
    for (i, _) in text.match_indices(name) {
        let rest = &text[i + name.len()..];
        let mut it = rest.chars();
        let ends = match it.next() {
            None => true,
            // A sentence's full stop ends it; "x.y" goes on.
            Some('.') => !it.next().is_some_and(|c| c.is_ascii_alphanumeric()),
            Some(c) => !name_char(c),
        };
        if starts_token(i) && ends {
            return true;
        }
    }
    let Some(dash) = name.rfind('-') else {
        return false;
    };
    let num = &name[dash + 1..];
    if num.is_empty() || !num.bytes().all(|b| b.is_ascii_digit()) || dash == 0 {
        return false;
    }
    let (stem, n) = (&name[..=dash], num.parse::<u64>().ok());
    text.match_indices(stem).any(|(i, _)| {
        if !starts_token(i) {
            return false;
        }
        let rest = &text[i + stem.len()..];
        let after = |r: &str| !r.chars().next().is_some_and(name_char);
        if let Some(r) = rest.strip_prefix(['N', 'n', '*']) {
            return after(r);
        }
        // A range: 1..4 (or 1-4 is not one: that is a name).
        let lo_len = rest.bytes().take_while(u8::is_ascii_digit).count();
        let Some(r) = rest[lo_len..].strip_prefix("..") else {
            return false;
        };
        let hi_len = r.bytes().take_while(u8::is_ascii_digit).count();
        let (Ok(lo), Ok(hi)) = (rest[..lo_len].parse::<u64>(), r[..hi_len].parse::<u64>()) else {
            return false;
        };
        after(&r[hi_len..]) && n.is_some_and(|n| lo <= n && n <= hi)
    })
}

/// Whether a card another waits on has finished its work (card #347): it
/// ended, it's in review, or an agent said it's done and it waits on the
/// person's Done. A wait on it is over; what's left is the person's.
/// Says how, for the closing note.
fn work_done(o: &Task) -> Option<String> {
    if is_terminal(&o.state) {
        return Some(format!("task {}", o.state));
    }
    if o.state == IN_REVIEW {
        return Some("task in review".into());
    }
    o.blockers
        .iter()
        .any(|b| b.open() && b.done_check)
        .then(|| "task done, waiting on Done".into())
}
