//! Room asks: the person talking to the room, with a take lock the
//! posting device owns (I:internal/coord/ask.go, coord.md §3.8).

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use gocompat::quote::quote;
use gocompat::time::GoTime;
use irori_identity::owner;
use store::{Event, kinds};

use super::error::{Error, Result, bad_state, held, invalid, not_allowed, not_found};
use super::fold::{Fold, Touch, sync_holder};
use super::resource::check_repo_key;
use super::rows::*;
use super::types::*;
use super::util::{
    SWEEPER, caller, check_text, collapse_space, contains, cut, deeper, new_id, runes,
};
use super::{MAX_OBJECTIVE, MAX_TITLE};
use crate::gotime;
use crate::names::trim_space;
use crate::roster::AgentCard;

pub const TAKE_CLAIM: &str = "claim";
pub const TAKE_RELEASE: &str = "release";
pub const TAKE_REOPEN: &str = "reopen";

pub const TAKE_POSTED: &str = "posted";
pub const TAKE_TAKEN: &str = "taken";
pub const TAKE_RELEASED: &str = "released";
pub const TAKE_GONE: &str = "gone";
pub const TAKE_REOPENED: &str = "reopened";
pub const TAKE_UNCLAIMED: &str = "unclaimed";

pub const OUTCOME_CARD: &str = "card";
pub const OUTCOME_EDIT: &str = "edit";
pub const OUTCOME_ANSWERED: &str = "answered";
pub const OUTCOME_NONE: &str = "none";

/// How long an ask may go untaken before the person hears.
pub const ASK_UNCLAIMED: Duration = super::GONE_GRACE;
/// How long ask_claim waits for the owner's answer.
pub const ASK_WAIT: Duration = Duration::from_secs(5);
const MAX_ASK_TITLE: usize = 80;
const MAX_ASK_NOTICE: usize = 600;
pub const MAX_OUTCOME: usize = 300;

const YOU_HAVE_IT: &str = "Talk it through with the person on the card first if it isn't clear what they want, then ask_close it with what it became: a card, a change to another card, or nothing.";

#[derive(Debug, Clone)]
pub(crate) struct TakeReq {
    pub r: TakeReqRef,
    pub by: String,
    pub op: String,
    pub done: bool,
    pub waiting: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AskAgain {
    pub skip: String,
    pub why: String,
    pub reason: String,
    pub wake: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct TakeMark {
    pub at: RowPos,
    pub taker: String,
}

/// The take lock's fold beside the tasks.
#[derive(Debug, Default)]
pub(crate) struct AskBook {
    pub reqs: HashMap<String, Vec<TakeReq>>,
    pub state_seq: HashMap<String, i64>,
    pub seen: HashMap<String, BTreeMap<String, i64>>,
    pub notices: Vec<Notice>,
    pub again: BTreeMap<String, AskAgain>,
    pub marks: HashMap<String, Vec<TakeMark>>,
    pub deferred: HashMap<String, Vec<Event>>,
    pub retry: Vec<String>,
    pub closed_at: HashMap<String, RowPos>,
}

impl AskBook {
    fn mark(&mut self, id: &str, at: RowPos, taker: &str) {
        let marks = self.marks.entry(id.to_string()).or_default();
        if marks.iter().any(|m| m.at == at) {
            return;
        }
        marks.push(TakeMark {
            at,
            taker: taker.into(),
        });
        self.wake(id);
    }

    fn wake(&mut self, id: &str) {
        if self.deferred.get(id).is_some_and(|d| !d.is_empty())
            && !self.retry.iter().any(|r| r == id)
        {
            self.retry.push(id.into());
        }
    }

    pub(crate) fn defer_row(&mut self, id: &str, ev: &Event) {
        let rows = self.deferred.entry(id.to_string()).or_default();
        if rows
            .iter()
            .any(|o| o.device_id == ev.device_id && o.seq == ev.seq && o.hlc == ev.hlc)
        {
            return;
        }
        rows.push(ev.clone());
    }

    fn last_seen(&mut self, id: &str, seen: &BTreeMap<String, i64>) {
        let m = self.seen.entry(id.to_string()).or_default();
        for (d, n) in seen {
            let v = m.entry(d.clone()).or_default();
            *v = (*v).max(*n);
        }
    }
}

/// A prompt's title: its first line, cut at a word to 80 characters.
pub fn ask_title(text: &str) -> String {
    let trimmed = trim_space(text);
    let line = trimmed.split_once('\n').map(|(a, _)| a).unwrap_or(trimmed);
    let line = collapse_space(line);
    if runes(&line) <= MAX_ASK_TITLE {
        return line;
    }
    let r: String = line.chars().take(MAX_ASK_TITLE - 1).collect();
    // LastIndexFunc returns a byte index (coord.md §3.18).
    if let Some((i, _)) = r
        .char_indices()
        .rev()
        .find(|(_, c)| gocompat::text::is_space(*c))
        && i > MAX_ASK_TITLE / 2
    {
        return format!("{}…", trim_space(&r[..i]));
    }
    format!("{r}…")
}

/// The error the lock gives a board tool on an open ask.
pub(crate) fn ask_refusal(t: &Task, what: &str) -> Error {
    not_allowed(format!(
        "{} is an ask: {what}; use ask_claim to take it and ask_close to end it",
        t.short_ref()
    ))
}

fn cmp_or_dev<'a>(a: &'a str, b: &'a str) -> &'a str {
    if !a.is_empty() { a } else { b }
}

impl Fold {
    fn set_stale(&self, id: &str, taker: &str, closed: bool) {
        let mut s = self.stale.write().unwrap_or_else(|e| e.into_inner());
        if closed {
            s.closed.insert(id.into());
            s.stale.remove(id);
        } else if !taker.is_empty() {
            s.stale.insert(id.into(), taker.into());
        } else {
            s.stale.remove(id);
        }
    }

    pub(crate) fn reset_stale(&self) {
        let mut s = self.stale.write().unwrap_or_else(|e| e.into_inner());
        s.stale.clear();
        s.closed.clear();
    }

    /// Sets up t's ask from its task.created row: only the person's row
    /// makes an ask.
    pub(crate) fn fold_ask_created(
        &self,
        t: &mut Task,
        a: Option<&AskField>,
        ev: &Event,
        by: &str,
    ) {
        let Some(a) = a else { return };
        if !self.person_row(ev, by) {
            return;
        }
        t.ask = Some(Ask {
            wake: a.wake,
            repo_key: a.repo_key.clone(),
            owner: ev.device_id.clone(),
            since: t.created_at,
            ..Ask::default()
        });
    }

    pub(crate) fn apply_take(&mut self, ev: &Event, at: &dyn Fn(GoTime) -> GoTime) {
        match ev.kind.as_str() {
            kinds::TAKE_REQUESTED => {
                let Some(p) = row::<TakeRequestedRow>(&ev.payload) else {
                    return;
                };
                if p.task_id.is_empty() || p.by.is_empty() {
                    return;
                }
                if !owner::may_write(&p.by, &ev.device_id) || owner::device_of(&p.by).is_empty() {
                    self.ignore(&ev.device_id);
                    return;
                }
                match p.op.as_str() {
                    TAKE_CLAIM | TAKE_RELEASE => {}
                    TAKE_REOPEN => {
                        if !self.person_row(ev, &p.by) {
                            self.ignore(&ev.device_id);
                            return;
                        }
                    }
                    _ => return,
                }
                let r = TakeReqRef {
                    device: ev.device_id.clone(),
                    seq: ev.seq,
                };
                if self
                    .asks
                    .reqs
                    .get(&p.task_id)
                    .is_some_and(|rs| rs.iter().any(|o| o.r == r))
                {
                    return;
                }
                let done = self.covered(&p.task_id, &r);
                self.asks
                    .reqs
                    .entry(p.task_id.clone())
                    .or_default()
                    .push(TakeReq {
                        r,
                        by: p.by.clone(),
                        op: p.op.clone(),
                        done,
                        waiting: false,
                    });
                if let Some(t) = self.tasks.get(&p.task_id) {
                    if p.op == TAKE_CLAIM {
                        let ti = Touch::of(t);
                        self.touch(
                            ev,
                            &ti,
                            at(p.at),
                            &p.by,
                            "asked to take it",
                            false,
                            Vec::new(),
                        );
                    }
                    self.sync_waiting(&p.task_id);
                    self.note(Change {
                        kind: TASK_CHANGED.into(),
                        task_id: p.task_id,
                        ..Change::default()
                    });
                }
            }
            kinds::TAKE_STATE => {
                let Some(p) = row::<TakeStateRow>(&ev.payload) else {
                    return;
                };
                if p.task_id.is_empty() {
                    return;
                }
                let Some(owner_dev) = self
                    .tasks
                    .get(&p.task_id)
                    .and_then(|t| t.ask.as_ref())
                    .map(|a| a.owner.clone())
                else {
                    return;
                };
                if ev.device_id != owner_dev {
                    self.ignore(&ev.device_id);
                    return;
                }
                self.asks.mark(&p.task_id, RowPos::of(ev), &p.taker);
                let closed = self.tasks[&p.task_id]
                    .ask
                    .as_ref()
                    .is_some_and(|a| a.outcome.is_some());
                if closed || ev.seq <= self.asks.state_seq.get(&p.task_id).copied().unwrap_or(0) {
                    return;
                }
                self.asks.state_seq.insert(p.task_id.clone(), ev.seq);
                self.asks.last_seen(&p.task_id, &p.seen);
                let mut t = self.tasks.remove(&p.task_id).expect("checked");
                let a = t.ask.as_mut().expect("checked");
                let was = a.taker.clone();
                a.taker = p.taker.clone();
                a.reason = p.reason.clone();
                a.told = p.told;
                a.since = at(p.since);
                a.unclaimed = p.unclaimed;
                a.heard = p.heard.clone();
                if !p.taker.is_empty() {
                    a.unclaimed = false;
                    t.holders = vec![p.taker.clone()];
                } else {
                    t.holders.clear();
                }
                t.assigned.clear();
                sync_holder(&mut t);
                let when = at(p.at);
                t.updated_at = when;
                self.tasks.insert(p.task_id.clone(), t);
                self.answer_requests(&p.task_id, &p);
                self.set_stale(&p.task_id, &p.taker, false);
                let t = &self.tasks[&p.task_id];
                let ti = Touch::of(t);
                let created_by = t.created_by.clone();
                let was_who = cmp_or_dev(&p.was, &was).to_string();
                if !p.taker.is_empty() && p.taker != was {
                    self.tasks.get_mut(&p.task_id).expect("present").updated_by = p.taker.clone();
                    self.touch(ev, &ti, when, &p.taker, "took it", true, Vec::new());
                } else if p.reason == TAKE_RELEASED {
                    self.touch(ev, &ti, when, &was_who, "gave it back", true, Vec::new());
                } else if p.reason == TAKE_GONE {
                    let what = format!("opened it again: {was_who} went away");
                    self.touch(ev, &ti, when, SWEEPER, &what, true, Vec::new());
                } else if p.reason == TAKE_REOPENED {
                    self.touch(
                        ev,
                        &ti,
                        when,
                        &created_by,
                        "asked the room again",
                        true,
                        Vec::new(),
                    );
                } else if p.reason == TAKE_UNCLAIMED && p.taker.is_empty() && was.is_empty() {
                    self.touch(
                        ev,
                        &ti,
                        when,
                        SWEEPER,
                        "nobody took it yet",
                        false,
                        Vec::new(),
                    );
                }
                let holder = self.tasks[&p.task_id].holder.clone();
                self.note(Change {
                    kind: TASK_CHANGED.into(),
                    task_id: p.task_id,
                    holder,
                    ..Change::default()
                });
            }
            kinds::TASK_EDITED => {
                let Some(p) = row::<TaskEditedRow>(&ev.payload) else {
                    return;
                };
                let from_ref = if p.from.is_empty() {
                    String::new()
                } else {
                    self.short_of(&p.from)
                };
                let Some(t) = self.tasks.get_mut(&p.task_id) else {
                    return;
                };
                if let Some(title) = &p.title
                    && !trim_space(title).is_empty()
                {
                    t.title = title.clone();
                }
                if let Some(o) = &p.objective {
                    t.objective = o.clone();
                }
                if let Some(d) = &p.done_criteria {
                    t.done_criteria = d.clone();
                }
                let when = at(p.at);
                t.updated_at = when;
                t.updated_by = p.by.clone();
                let mut what = "edited the card".to_string();
                if !p.from.is_empty() {
                    what = format!("changed it from the ask {from_ref}");
                    if !p.note.is_empty() {
                        what.push_str(&format!(": {}", p.note));
                    }
                }
                let ti = Touch::of(t);
                let holder = t.holder.clone();
                self.touch(ev, &ti, when, &p.by, &what, true, Vec::new());
                self.note(Change {
                    kind: TASK_CHANGED.into(),
                    task_id: p.task_id,
                    holder,
                    ..Change::default()
                });
            }
            _ => {}
        }
    }

    /// A card's short ref by id, or task:<id>.
    pub(crate) fn short_of(&self, id: &str) -> String {
        match self.tasks.get(id) {
            Some(t) => t.short_ref(),
            None => format!("task:{id}"),
        }
    }

    fn covered(&self, id: &str, r: &TakeReqRef) -> bool {
        self.asks
            .seen
            .get(id)
            .and_then(|s| s.get(&r.device))
            .copied()
            .unwrap_or(0)
            >= r.seq
    }

    /// Marks the requests a take.state answers, and queues the answer for
    /// this device's claimers that stopped waiting.
    fn answer_requests(&mut self, id: &str, p: &TakeStateRow) {
        let mut reqs = self.asks.reqs.remove(id).unwrap_or_default();
        for r in reqs.iter_mut() {
            if r.done {
                continue;
            }
            let mut answered = self.covered(id, &r.r) || p.req.as_ref() == Some(&r.r);
            match r.op.as_str() {
                TAKE_CLAIM => answered = answered || !p.taker.is_empty(),
                TAKE_RELEASE => answered = answered || p.taker != r.by,
                _ => {}
            }
            if !answered {
                continue;
            }
            r.done = true;
            if r.op != TAKE_CLAIM
                || r.waiting
                || self.replaying
                || r.r.device != self.dev
                || p.taker.is_empty()
            {
                continue;
            }
            let n = self.taken_notice(&self.tasks[id], &r.by, &p.taker);
            self.asks.notices.push(n);
        }
        if !reqs.is_empty() {
            self.asks.reqs.insert(id.into(), reqs);
        }
        self.sync_waiting(id);
    }

    fn taken_notice(&self, t: &Task, claimer: &str, taker: &str) -> Notice {
        let text = if taker == claimer {
            format!(
                "You have the ask {} ({}). {YOU_HAVE_IT}{}",
                quote(&t.title),
                t.refs(),
                deeper(&t.id)
            )
        } else {
            format!(
                "{taker} took the ask {} ({}) first; nothing to do unless you want to chime in (board_comment).{}",
                quote(&t.title),
                t.refs(),
                deeper(&t.id)
            )
        };
        Notice {
            kind: NOTICE_TAKEN.into(),
            task_id: t.id.clone(),
            reference: format!("task:{}", t.id),
            title: t.title.clone(),
            by: SWEEPER.into(),
            to: vec![claimer.into()],
            text,
            ..Notice::default()
        }
    }

    /// Refreshes Ask.waiting: this device's agents with an unanswered claim.
    pub(crate) fn sync_waiting(&mut self, id: &str) {
        let mut w: Vec<String> = Vec::new();
        for r in self.asks.reqs.get(id).into_iter().flatten() {
            if !r.done && r.op == TAKE_CLAIM && r.r.device == self.dev && !w.contains(&r.by) {
                w.push(r.by.clone());
            }
        }
        if let Some(a) = self.tasks.get_mut(id).and_then(|t| t.ask.as_mut()) {
            a.waiting = w;
        }
    }

    /// Ends t's ask once a task.status with an outcome (or the person's
    /// terminal state) folds at `at`.
    pub(crate) fn close_ask_fold(&mut self, t: &mut Task, o: Outcome, at: RowPos) {
        if let Some(a) = t.ask.as_mut() {
            a.outcome = Some(o);
            a.waiting.clear();
        }
        for r in self.asks.reqs.get_mut(&t.id).into_iter().flatten() {
            r.done = true;
        }
        self.asks.closed_at.insert(t.id.clone(), at);
        self.asks.wake(&t.id);
        self.set_stale(&t.id, "", true);
    }

    /// Who held ask id where a row at `at` sits.
    fn taker_at(&self, id: &str, at: &RowPos) -> String {
        self.asks
            .marks
            .get(id)
            .into_iter()
            .flatten()
            .filter(|m| m.at.before(at))
            .max_by(|a, b| a.at.cmp(&b.at))
            .map(|m| m.taker.clone())
            .unwrap_or_default()
    }

    /// The take lock in the fold: whether ev folds now, and if not which
    /// ask's next take.state or close may let it through (`""` drops it).
    pub(crate) fn ask_gate(&mut self, ev: &Event) -> (bool, String) {
        match ev.kind.as_str() {
            kinds::TASK_EDITED => return self.edit_gate(ev),
            kinds::TASK_CLAIMED
            | kinds::TASK_JOINED
            | kinds::TASK_ASSIGNED
            | kinds::TASK_STATUS => {}
            _ => return (true, String::new()),
        }
        let Some(p) = row::<GateRow>(&ev.payload) else {
            return (true, String::new());
        };
        let Some(t) = self.tasks.get(&p.task_id) else {
            return (true, String::new());
        };
        let Some(a) = &t.ask else {
            return (true, String::new());
        };
        let cur = RowPos::of(ev);
        if !a.is_open() {
            let closed = self.asks.closed_at.get(&t.id).cloned().unwrap_or_default();
            return (!cur.before(&closed), String::new());
        }
        let person = self.person_row(ev, &p.by);
        if ev.kind == kinds::TASK_STATUS {
            if p.outcome.is_some()
                && (person || !p.by.is_empty() && self.taker_at(&t.id, &cur) == p.by)
            {
                return (true, String::new());
            }
            if p.outcome.is_none() && person && is_terminal(&p.state) {
                return (true, String::new());
            }
        }
        (false, t.id.clone())
    }

    fn edit_gate(&mut self, ev: &Event) -> (bool, String) {
        let Some(p) = row::<TaskEditedRow>(&ev.payload) else {
            return (false, String::new());
        };
        if p.task_id.is_empty() || p.by.is_empty() {
            return (false, String::new());
        }
        if self.person_row(ev, &p.by) {
            return (true, String::new());
        }
        if !owner::may_write(&p.by, &ev.device_id) {
            self.ignore(&ev.device_id);
            return (false, String::new());
        }
        let mut ask = p.from.clone();
        if ask == p.task_id {
            return (false, String::new());
        }
        if ask.is_empty() {
            let Some(t) = self.tasks.get(&p.task_id) else {
                return (false, String::new());
            };
            if t.ask.is_none() {
                return (
                    p.by == t.created_by || contains(&t.holders, &p.by),
                    String::new(),
                );
            }
            ask = t.id.clone();
        }
        if self.taker_at(&ask, &RowPos::of(ev)) == p.by {
            return (true, String::new());
        }
        (false, ask)
    }

    /// Folds again the rows waiting on asks whose lock moved.
    pub(crate) fn fold_deferred(&mut self) {
        while !self.asks.retry.is_empty() {
            let id = self.asks.retry.remove(0);
            let rows = self.asks.deferred.remove(&id).unwrap_or_default();
            for ev in rows {
                self.fold(&ev);
            }
        }
    }

    // ---- posting ----

    /// The person asking the room: the ask's card, owned by this device,
    /// told to every free agent among `agents`.
    pub(crate) fn post_ask(
        &mut self,
        by: &str,
        input: NewAsk,
        agents: &[AgentCard],
    ) -> Result<(Task, Notice)> {
        let (by, text) = super::comments::check_comment(by, &input.text, &input.files)?;
        if !self.is_owner(&by) {
            return Err(not_allowed("only the person asks the room"));
        }
        let repo_key = trim_space(&input.repo_key).to_string();
        if !repo_key.is_empty() {
            check_repo_key(&repo_key)?;
        }
        let at = self.stamp();
        let id = new_id(&at);
        let num = self.next_short();
        self.write(
            &by,
            kinds::TASK_CREATED,
            &TaskCreatedRow {
                task_id: id.clone(),
                num,
                title: ask_title(&text),
                objective: text.clone(),
                repo_key: repo_key.clone(),
                ask: Some(AskField {
                    wake: input.wake,
                    repo_key: repo_key.clone(),
                }),
                by: by.clone(),
                at,
                ..TaskCreatedRow::default()
            },
        )?;
        if !input.files.is_empty() {
            let actor = self.actor_for(&by);
            self.write_comment(
                &actor,
                TaskCommentRow {
                    task_id: id.clone(),
                    comment_id: new_id(&at),
                    by: by.clone(),
                    text: "Files with the ask.".into(),
                    at,
                    attachments: input.files.clone(),
                    ..TaskCommentRow::default()
                },
            )?;
        }
        let t = self.tasks[&id].clone();
        let free = self.free_agents(&t, agents, &[]);
        self.write_take(
            &id,
            TakeStateRow {
                reason: TAKE_POSTED.into(),
                told: free.len() as i64,
                heard: free.clone(),
                since: at,
                unclaimed: free.is_empty(),
                ..TakeStateRow::default()
            },
        )?;
        let t = self.tasks[&id].clone();
        let mut n = ask_notice(&t, &free, input.wake, "");
        n.attachments = input.files;
        Ok((t, n))
    }

    /// The agents among `roster` that may take ask t now, sorted.
    fn free_agents(&self, t: &Task, roster: &[AgentCard], skip: &[String]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for a in roster {
            let dev = cmp_or_dev(&a.device_id, owner::device_of(&a.key));
            if a.key.is_empty() || a.key == SWEEPER || self.is_owner(&a.key) || a.status != "idle" {
                continue;
            }
            if a.device_state == "offline" {
                continue;
            }
            if let Some(r) = &self.revoked
                && !dev.is_empty()
                && r(dev)
            {
                continue;
            }
            if let Some(ask) = &t.ask
                && !ask.repo_key.is_empty()
                && a.repo_key != ask.repo_key
            {
                continue;
            }
            if skip.contains(&a.key) || out.contains(&a.key) {
                continue;
            }
            out.push(a.key.clone());
        }
        out.sort();
        out
    }

    /// Writes the owner's take.state for ask `id`.
    fn write_take(&mut self, id: &str, mut r: TakeStateRow) -> Result<()> {
        r.task_id = id.into();
        r.at = self.stamp();
        if r.since.is_zero()
            && let Some(a) = self.tasks.get(id).and_then(|t| t.ask.as_ref())
        {
            r.since = a.since;
        }
        if let Some(s) = self.asks.seen.get(id)
            && !s.is_empty()
        {
            r.seen = s.clone();
        }
        self.write_as(SWEEPER, kinds::TAKE_STATE, &r)
    }

    // ---- the owner's duties ----

    /// What this device does as the owner of its open asks.
    pub(crate) fn ask_duties(&mut self) {
        if self.replaying {
            return;
        }
        let mut ids: Vec<String> = self
            .asks
            .reqs
            .iter()
            .filter(|(_, rs)| !rs.is_empty())
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        let now = self.stamp();
        for id in ids {
            let ok = self.tasks.get(&id).is_some_and(|t| {
                t.open_ask().is_some_and(|a| a.owner == self.dev) && !is_terminal(&t.state)
            });
            if !ok {
                continue;
            }
            let n = self.asks.reqs.get(&id).map_or(0, Vec::len);
            for i in 0..n {
                let Some(r) = self.asks.reqs.get(&id).and_then(|v| v.get(i)).cloned() else {
                    break;
                };
                if r.done || self.covered(&id, &r.r) {
                    if let Some(x) = self.asks.reqs.get_mut(&id).and_then(|v| v.get_mut(i)) {
                        x.done = true;
                    }
                    continue;
                }
                if self.owner_act(&id, i, &r, &now).is_err() {
                    return;
                }
            }
        }
    }

    fn set_req_done(&mut self, id: &str, i: usize) {
        if let Some(x) = self.asks.reqs.get_mut(id).and_then(|v| v.get_mut(i)) {
            x.done = true;
        }
    }

    /// Answers one request on an ask this device owns.
    fn owner_act(&mut self, id: &str, i: usize, r: &TakeReq, now: &GoTime) -> Result<()> {
        let seen = self.asks.seen.entry(id.to_string()).or_default();
        let s = seen.entry(r.r.device.clone()).or_default();
        *s = (*s).max(r.r.seq);
        let a = self.tasks[id].ask.clone().unwrap_or_default();
        match r.op.as_str() {
            TAKE_CLAIM => {
                self.observe(now);
                if !a.taker.is_empty() || self.gone(&r.by, now) {
                    self.set_req_done(id, i);
                    return Ok(());
                }
                self.write_take(
                    id,
                    TakeStateRow {
                        taker: r.by.clone(),
                        reason: TAKE_TAKEN.into(),
                        req: Some(r.r.clone()),
                        told: a.told,
                        heard: a.heard.clone(),
                        ..TakeStateRow::default()
                    },
                )
            }
            TAKE_RELEASE => {
                if a.taker != r.by {
                    self.set_req_done(id, i);
                    return Ok(());
                }
                self.write_take(
                    id,
                    TakeStateRow {
                        reason: TAKE_RELEASED.into(),
                        was: r.by.clone(),
                        told: a.told,
                        heard: a.heard.clone(),
                        since: *now,
                        ..TakeStateRow::default()
                    },
                )?;
                self.asks.again.insert(
                    id.into(),
                    AskAgain {
                        skip: r.by.clone(),
                        why: "the agent that took it gave it back".into(),
                        reason: TAKE_RELEASED.into(),
                        wake: false,
                    },
                );
                Ok(())
            }
            TAKE_REOPEN => {
                if !a.taker.is_empty() {
                    self.set_req_done(id, i);
                    return Ok(());
                }
                self.asks.again.insert(
                    id.into(),
                    AskAgain {
                        wake: true,
                        why: "the person wants it picked up".into(),
                        reason: TAKE_REOPENED.into(),
                        skip: String::new(),
                    },
                );
                self.set_req_done(id, i);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// The timed part of the asks with the mesh roster.
    /// Returns what it told before any failed write, with the failure
    /// (Go returns both).
    pub(crate) fn sweep_asks(&mut self, agents: &[AgentCard]) -> (Vec<Notice>, Result<()>) {
        self.ask_duties();
        let now = self.stamp();
        self.observe(&now);
        let mut out = std::mem::take(&mut self.asks.notices);
        let mut again = std::mem::take(&mut self.asks.again);
        let in_roster: HashMap<&str, &AgentCard> =
            agents.iter().map(|a| (a.key.as_str(), a)).collect();
        let ids: Vec<String> = self
            .tasks
            .iter()
            .filter(|(_, t)| {
                t.open_ask().is_some_and(|a| a.owner == self.dev) && !is_terminal(&t.state)
            })
            .map(|(id, _)| id.clone())
            .collect();
        let res = (|| -> Result<()> {
            for id in &ids {
                let a = self.tasks[id].ask.clone().unwrap_or_default();
                if !a.taker.is_empty() {
                    if !self.taker_gone(&a.taker, &in_roster, &now) {
                        continue;
                    }
                    self.write_take(
                        id,
                        TakeStateRow {
                            reason: TAKE_GONE.into(),
                            was: a.taker.clone(),
                            since: now,
                            ..TakeStateRow::default()
                        },
                    )?;
                    again.insert(
                        id.clone(),
                        AskAgain {
                            skip: a.taker.clone(),
                            why: "the agent that took it went away".into(),
                            reason: TAKE_GONE.into(),
                            wake: false,
                        },
                    );
                }
                let t = self.tasks[id].clone();
                let a = t.ask.clone().unwrap_or_default();
                if let Some(ag) = again.get(id).cloned() {
                    if !a.taker.is_empty() {
                        continue;
                    }
                    let free = self.free_agents(&t, agents, std::slice::from_ref(&ag.skip));
                    self.write_take(
                        id,
                        TakeStateRow {
                            reason: ag.reason.clone(),
                            told: free.len() as i64,
                            heard: free.clone(),
                            since: now,
                            unclaimed: free.is_empty(),
                            ..TakeStateRow::default()
                        },
                    )?;
                    again.remove(id);
                    if !free.is_empty() {
                        let t = self.tasks[id].clone();
                        out.push(ask_notice(&t, &free, ag.wake, &ag.why));
                    }
                    continue;
                }
                let free = self.free_agents(&t, agents, &a.heard);
                let window = gotime::since(&now, &a.since) < ASK_UNCLAIMED.as_nanos() as i128;
                let unclaimed = a.unclaimed || a.told + free.len() as i64 == 0 || !window;
                if free.is_empty() && unclaimed == a.unclaimed {
                    continue;
                }
                let mut heard = a.heard.clone();
                heard.extend(free.iter().cloned());
                let mut reason = a.reason.clone();
                if unclaimed && !a.unclaimed {
                    reason = TAKE_UNCLAIMED.into();
                }
                self.write_take(
                    id,
                    TakeStateRow {
                        reason,
                        told: heard.len() as i64,
                        heard,
                        unclaimed,
                        ..TakeStateRow::default()
                    },
                )?;
                if !free.is_empty() {
                    let wake =
                        window && (a.reason == TAKE_REOPENED || a.reason == TAKE_POSTED && a.wake);
                    let t = self.tasks[id].clone();
                    out.push(ask_notice(&t, &free, wake, ""));
                }
            }
            Ok(())
        })();
        if let Err(e) = res {
            for (id, ag) in again {
                self.asks.again.entry(id).or_insert(ag);
            }
            return (out, Err(e));
        }
        // An ask whose posting device was removed: the person's close.
        let revoked: Vec<String> = self
            .tasks
            .values()
            .filter(|t| {
                t.open_ask()
                    .is_some_and(|a| self.revoked.as_ref().is_some_and(|r| r(&a.owner)))
                    && !is_terminal(&t.state)
            })
            .map(|t| t.id.clone())
            .collect();
        for id in revoked {
            let me = self.self_key();
            if self.is_owner(&me) {
                let _ = self.write(
                    &me,
                    kinds::TASK_STATUS,
                    &TaskStateRow {
                        task_id: id,
                        state: CANCELED.into(),
                        note: "posting device removed".into(),
                        outcome: Some(Outcome {
                            kind: OUTCOME_NONE.into(),
                            note: "posting device removed".into(),
                            ..Outcome::default()
                        }),
                        by: me.clone(),
                        at: now,
                        ..TaskStateRow::default()
                    },
                );
            }
        }
        (out, Ok(()))
    }

    fn taker_gone(&self, taker: &str, roster: &HashMap<&str, &AgentCard>, now: &GoTime) -> bool {
        if self.is_owner(taker) {
            return false;
        }
        if self.local_key(taker) {
            return self.gone(taker, now);
        }
        roster.get(taker).is_none_or(|a| a.status == "gone")
    }

    // ---- claim, close ----

    /// Writes the claim request and runs the duties; returns the request's
    /// index for the wait.
    pub(crate) fn ask_claim_start(
        &mut self,
        by: &str,
        id: &str,
    ) -> Result<std::result::Result<ClaimResult, usize>> {
        let t = self.open_task(id)?;
        let Some(a) = t.open_ask() else {
            return Err(bad_state(format!("{} isn't an open ask", t.short_ref())));
        };
        if a.taker == by {
            return Ok(Ok(ClaimResult {
                task: t.clone(),
                taker: by.into(),
                mine: true,
                pending: false,
                owner: a.owner.clone(),
            }));
        }
        if !a.taker.is_empty() {
            return Err(held(format!(
                "{} took {}. To add something, board_comment it",
                a.taker,
                t.short_ref()
            )));
        }
        let at = self.stamp();
        self.write(
            by,
            kinds::TAKE_REQUESTED,
            &TakeRequestedRow {
                task_id: id.into(),
                by: by.into(),
                op: TAKE_CLAIM.into(),
                at,
            },
        )?;
        let i = self.asks.reqs.get(id).map_or(0, Vec::len).saturating_sub(1);
        if let Some(r) = self.asks.reqs.get_mut(id).and_then(|v| v.get_mut(i)) {
            r.waiting = true;
        }
        self.ask_duties();
        Ok(Err(i))
    }

    /// Where a waiting claim stands; `None` while it still waits.
    pub(crate) fn ask_claim_poll(
        &mut self,
        by: &str,
        id: &str,
        req: usize,
        last: bool,
    ) -> Option<Result<ClaimResult>> {
        let t = self.tasks.get(id)?;
        let a = t.ask.clone().unwrap_or_default();
        let res = ClaimResult {
            task: t.clone(),
            owner: a.owner.clone(),
            taker: a.taker.clone(),
            mine: a.taker == by,
            pending: a.taker.is_empty(),
        };
        let done = a.outcome.is_some() || !res.pending || last;
        if done && let Some(r) = self.asks.reqs.get_mut(id).and_then(|v| v.get_mut(req)) {
            r.waiting = false;
        }
        if a.outcome.is_some() {
            return Some(Err(bad_state(format!(
                "{} closed before the owner answered",
                t.short_ref()
            ))));
        }
        if !res.pending || last {
            return Some(Ok(res));
        }
        None
    }

    pub(crate) fn ask_close(&mut self, by: &str, id: &str, o: AskOutcome) -> Result<Task> {
        let by = caller(by)?;
        let mut note = collapse_space(&o.note);
        if note.is_empty() && self.is_owner(&by) {
            note = "closed by the person".into();
        }
        check_text("the note", &note, 1, MAX_OUTCOME)?;
        let title = trim_space(&o.title).to_string();
        let obj = trim_space(&o.objective).to_string();
        let done = trim_space(&o.done_when).to_string();
        if runes(&title) > MAX_TITLE {
            return Err(invalid(format!("title longer than {MAX_TITLE} characters")));
        }
        if runes(&obj) > MAX_OBJECTIVE || runes(&done) > MAX_OBJECTIVE {
            return Err(invalid(format!(
                "objective or done criteria longer than {MAX_OBJECTIVE} characters"
            )));
        }
        let mut steps = o.steps.clone();
        super::cards::check_steps(&mut steps)?;
        let t = self.open_task(id)?.clone();
        let Some(a) = t.open_ask().cloned() else {
            return Err(bad_state(format!("{} isn't an open ask", t.short_ref())));
        };
        let person = self.is_owner(&by);
        if !person && (a.taker.is_empty() || a.taker != by) {
            if a.taker.is_empty() {
                return Err(not_allowed(format!("ask_claim {} first", t.short_ref())));
            }
            return Err(not_allowed(format!(
                "{} holds {}; only it or the person closes it",
                a.taker,
                t.short_ref()
            )));
        }
        let now = self.stamp();
        let mut parent = String::new();
        if !trim_space(&o.parent).is_empty() {
            if o.kind != OUTCOME_CARD {
                return Err(invalid("parent goes with a card outcome"));
            }
            let pid = self.task_ref_locked(&o.parent)?;
            self.check_parent(&pid, "")?;
            parent = pid;
        }
        let opt = |s: &str| (!s.is_empty()).then(|| s.to_string());
        match o.kind.as_str() {
            OUTCOME_CARD => {
                if title.is_empty() {
                    return Err(invalid("a card outcome needs a title"));
                }
                self.write(
                    &by,
                    kinds::TASK_EDITED,
                    &TaskEditedRow {
                        task_id: id.into(),
                        title: Some(title),
                        objective: opt(&obj),
                        done_criteria: opt(&done),
                        by: by.clone(),
                        at: now,
                        ..TaskEditedRow::default()
                    },
                )?;
                if !steps.is_empty() {
                    self.write(
                        &by,
                        kinds::TASK_STEPS,
                        &TaskStepsRow {
                            task_id: id.into(),
                            steps: super::cards::new_steps(&steps),
                            by: by.clone(),
                            at: now,
                        },
                    )?;
                }
                let taker = a.taker.clone();
                self.write(
                    &by,
                    kinds::TASK_STATUS,
                    &TaskStateRow {
                        task_id: id.into(),
                        state: WORKING.into(),
                        note: note.clone(),
                        by: by.clone(),
                        at: now,
                        outcome: Some(Outcome {
                            kind: OUTCOME_CARD.into(),
                            note: note.clone(),
                            ..Outcome::default()
                        }),
                        ..TaskStateRow::default()
                    },
                )?;
                if !taker.is_empty() && !self.is_owner(&taker) && by == taker {
                    self.write(
                        &by,
                        kinds::TASK_CLAIMED,
                        &TaskClaimedRow {
                            task_id: id.into(),
                            by: by.clone(),
                            at: now,
                        },
                    )?;
                }
                if !parent.is_empty() {
                    let at = self.stamp();
                    self.write(
                        &by,
                        kinds::TASK_PARENT,
                        &TaskParentRow {
                            task_id: id.into(),
                            parent,
                            by: by.clone(),
                            at,
                        },
                    )?;
                }
            }
            OUTCOME_EDIT => {
                let target = trim_space(&o.task).to_string();
                let m = match self.tasks.get(&target) {
                    Some(m) if target != id => m.clone(),
                    _ => {
                        return Err(not_found(
                            "an edit outcome needs the card it changed (task)",
                        ));
                    }
                };
                if m.open_ask().is_some() {
                    return Err(not_allowed(format!(
                        "{} is an open ask; its prompt is the person's",
                        m.short_ref()
                    )));
                }
                if is_terminal(&m.state) {
                    return Err(bad_state(format!(
                        "{} is {}; an edit outcome changes a card still in play",
                        m.short_ref(),
                        m.state
                    )));
                }
                self.write(
                    &by,
                    kinds::TASK_EDITED,
                    &TaskEditedRow {
                        task_id: target.clone(),
                        title: opt(&title),
                        objective: opt(&obj),
                        done_criteria: opt(&done),
                        by: by.clone(),
                        from: id.into(),
                        note: note.clone(),
                        at: now,
                    },
                )?;
                self.write(
                    &by,
                    kinds::TASK_STATUS,
                    &TaskStateRow {
                        task_id: id.into(),
                        state: COMPLETED.into(),
                        note: format!("Changed {}: {note}", m.short_ref()),
                        by: by.clone(),
                        at: now,
                        outcome: Some(Outcome {
                            kind: OUTCOME_EDIT.into(),
                            task: target,
                            note: note.clone(),
                        }),
                        ..TaskStateRow::default()
                    },
                )?;
            }
            OUTCOME_ANSWERED | OUTCOME_NONE => {
                let state = if o.kind == OUTCOME_ANSWERED {
                    COMPLETED
                } else {
                    CANCELED
                };
                self.write(
                    &by,
                    kinds::TASK_STATUS,
                    &TaskStateRow {
                        task_id: id.into(),
                        state: state.into(),
                        note: note.clone(),
                        by: by.clone(),
                        at: now,
                        outcome: Some(Outcome {
                            kind: o.kind.clone(),
                            note: note.clone(),
                            ..Outcome::default()
                        }),
                        ..TaskStateRow::default()
                    },
                )?;
            }
            _ => {
                return Err(invalid(format!(
                    "outcome is card, edit, answered or none, not {}",
                    quote(&o.kind)
                )));
            }
        }
        Ok(self.tasks[id].clone())
    }

    /// Gives an ask back (board_leave by its taker).
    pub(crate) fn ask_release(&mut self, by: &str, id: &str) -> Result<()> {
        let at = self.stamp();
        self.write(
            by,
            kinds::TAKE_REQUESTED,
            &TakeRequestedRow {
                task_id: id.into(),
                by: by.into(),
                op: TAKE_RELEASE.into(),
                at,
            },
        )?;
        self.ask_duties();
        Ok(())
    }

    /// The person's "Wake them" on an unclaimed ask.
    pub(crate) fn ask_again(&mut self, by: &str, id: &str) -> Result<()> {
        if !self.is_owner(by) {
            return Err(not_allowed("only the person asks again"));
        }
        let t = self.open_task(id)?;
        let Some(a) = t.open_ask() else {
            return Err(bad_state(format!("{} isn't an open ask", t.short_ref())));
        };
        if !a.taker.is_empty() {
            return Err(held(format!("{} took it", a.taker)));
        }
        let at = self.stamp();
        self.write(
            by,
            kinds::TAKE_REQUESTED,
            &TakeRequestedRow {
                task_id: id.into(),
                by: by.into(),
                op: TAKE_REOPEN.into(),
                at,
            },
        )?;
        self.ask_duties();
        Ok(())
    }
}

/// Tells `to` about ask t, from the person.
fn ask_notice(t: &Task, to: &[String], wake: bool, why: &str) -> Notice {
    let mut b = format!("The person asks the room ({})", t.refs());
    if !why.is_empty() {
        b.push_str(&format!(", again: {why}"));
    }
    b.push_str(&format!(":\n{}\n", cut(&t.objective, MAX_ASK_NOTICE)));
    b.push_str(&format!(
        "If you can take it on now, ask_claim {}: one agent takes it, and you hear if someone else got there first. If you're busy, leave it.",
        t.short_ref()
    ));
    b.push_str(&deeper(&t.id));
    Notice {
        kind: NOTICE_ASK.into(),
        task_id: t.id.clone(),
        reference: format!("task:{}", t.id),
        title: t.title.clone(),
        by: t.created_by.clone(),
        to: to.to_vec(),
        wake,
        text: b,
        person: true,
        ..Notice::default()
    }
}
