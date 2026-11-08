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
use super::util::{SWEEPER, caller, check_text, collapse_space, cut, deeper, new_id, runes};
use super::{MAX_OBJECTIVE, MAX_TITLE};
use crate::gotime;
use crate::names::trim_space;
use crate::roster::AgentCard;

/// The text of the comment that carries an ask's files.
pub const ASK_FILES_NOTE: &str = "Files with the ask.";

pub const TAKE_CLAIM: &str = "claim";
pub const TAKE_RELEASE: &str = "release";
pub const TAKE_REOPEN: &str = "reopen";

pub const TAKE_POSTED: &str = "posted";
pub const TAKE_TAKEN: &str = "taken";
pub const TAKE_RELEASED: &str = "released";
pub const TAKE_GONE: &str = "gone";
pub const TAKE_REOPENED: &str = "reopened";
pub const TAKE_UNCLAIMED: &str = "unclaimed";
/// Irori handed it to one idle agent, which holds it until it acts or
/// lets it go.
pub const TAKE_HANDED: &str = "handed";
/// The agent it was handed to ended its turn without touching it.
pub const TAKE_PASSED: &str = "passed";

/// Where an ask handed to an agent stands.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Handed {
    Waits,
    Touched,
    Ended,
    Gone,
}

pub const OUTCOME_CARD: &str = "card";
pub const OUTCOME_EDIT: &str = "edit";
pub const OUTCOME_ANSWERED: &str = "answered";
pub const OUTCOME_NONE: &str = "none";

/// How long an ask may go untaken before the person hears.
pub const ASK_UNCLAIMED: Duration = super::GONE_GRACE;
/// How long ask_claim waits for the owner's answer.
pub const ASK_WAIT: Duration = Duration::from_secs(5);
/// What a thread says while every computer that could start its agent is
/// full (card #686).
pub const ALL_FULL: &str =
    "Every computer is at its agent limit. It starts by itself when one has room.";

const MAX_ASK_TITLE: usize = 80;
const MAX_ASK_NOTICE: usize = 600;
pub const MAX_OUTCOME: usize = 300;

const YOU_HAVE_IT: &str = "If it isn't clear what they want, talk it through with them on the card first. If it needs a choice only the person can make, ask_close it as a card first (it stays yours), then raise one board_block per question on that card. Otherwise ask_close it with what it became: a card, a change to another card, or nothing.";

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
}

#[derive(Debug, Clone)]
pub(crate) struct TakeMark {
    pub at: RowPos,
    pub taker: String,
}

/// How long an ask waits for an owner that's away before a connected
/// computer takes it (card #541): a link that dropped for a moment doesn't
/// move anything.
pub const ASK_AWAY: Duration = Duration::from_secs(30);

/// How long an ask that moved because its owner was full (card #686) stays
/// before it may move again for the same reason: a computer that filled up
/// between taking it and starting its agent doesn't hand it straight back.
pub const ASK_FULL_SETTLE: Duration = Duration::from_secs(60);

/// How long an ask waits on a full owner before it first moves (card
/// #686): the owner's own pass starts it at once when it has room, and a
/// computer that counted the owner's agents a moment late doesn't take it
/// from under that start.
pub const ASK_FULL_GRACE: Duration = Duration::from_secs(5);

/// How long the "Nobody took" notice holds off for a computer that looks
/// about to take the ask (card #995): well past [`ASK_FULL_GRACE`] and the
/// new owner's next sweep, and short enough that a move that never comes
/// still gets said. After a move it counts from the end of
/// [`ASK_FULL_SETTLE`], since the next move can't come sooner.
pub const ASK_ROOM_HOLD: Duration = Duration::from_secs(30);

/// How many times an ask may move within [`ASK_FULL_WINDOW`] and still
/// move because its owner is full (card #686): past this it waits where it
/// is for a free slot. A thread that goes on for days isn't held by moves
/// its earlier turns made.
pub const MAX_FULL_MOVES: usize = 3;

/// See [`MAX_FULL_MOVES`].
pub const ASK_FULL_WINDOW: Duration = Duration::from_secs(10 * 60);

/// The take lock's fold beside the tasks.
#[derive(Debug, Default)]
pub(crate) struct AskBook {
    pub reqs: HashMap<String, Vec<TakeReq>>,
    pub state_seq: HashMap<String, i64>,
    /// Asks that moved (card #541): how many times, and who has them.
    pub moved: HashMap<String, (i64, String)>,
    /// When each ask moved, by each winning row's own time, oldest first
    /// (card #686).
    pub moved_at: HashMap<String, Vec<GoTime>>,
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
/// The private take kind a board ask row stands for (§9).
pub(crate) fn take_kind(kind: &str) -> &str {
    match kind {
        kinds::BOARD_ASK_REQUESTED => kinds::TAKE_REQUESTED,
        kinds::BOARD_ASK_STATE => kinds::TAKE_STATE,
        kinds::BOARD_ASK_EDITED => kinds::TASK_EDITED,
        k => k,
    }
}

impl Fold {
    /// The kind an ask row is written as: on a board, its `board.ask.*`
    /// name, since a board log refuses the private take kinds.
    pub(crate) fn ask_kind(&self, kind: &'static str) -> &'static str {
        if !self.on_board {
            return kind;
        }
        match kind {
            kinds::TAKE_REQUESTED => kinds::BOARD_ASK_REQUESTED,
            kinds::TAKE_STATE => kinds::BOARD_ASK_STATE,
            kinds::TASK_EDITED => kinds::BOARD_ASK_EDITED,
            kinds::ASK_MOVED => kinds::BOARD_ASK_MOVED,
            k => k,
        }
    }
}

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
        // A refold rebuilds both (card #495).
        s.sides.clear();
        s.runs = crate::sides::Sides::new();
    }

    /// Sets up t's ask from its task.created row: only the person's row
    /// makes an ask, and an agent's own row a thread it opened (card #631).
    pub(crate) fn fold_ask_created(
        &self,
        t: &mut Task,
        a: Option<&AskField>,
        ev: &Event,
        by: &str,
    ) {
        let Some(a) = a else { return };
        let opener = !self.person_row(ev, by);
        if opener && !(a.thread && self.agent_own_row(ev, by)) {
            return;
        }
        t.ask = Some(Ask {
            wake: a.wake,
            repo_key: ledger::canonical_repo_key(&a.repo_key),
            // An agent's thread is its own device's: that device writes its
            // take rows, and no other device hands it out.
            owner: if !opener
                && !a.owner.is_empty()
                && self.same_person_dev(&a.owner, &ev.device_id)
            {
                a.owner.clone()
            } else {
                ev.device_id.clone()
            },
            opener: if opener {
                by.to_string()
            } else {
                String::new()
            },
            since: t.created_at,
            // A board log carries mentions in board form only (cards #177,
            // #187); anything else another device wrote is dropped here.
            mentions: if self.on_board {
                a.mentions
                    .iter()
                    .filter(|m| super::mention::board_form(&m.key))
                    .cloned()
                    .collect()
            } else {
                a.mentions.clone()
            },
            thread: a.thread,
            board: if self.on_board {
                String::new()
            } else {
                a.board.clone()
            },
            // A space's thread names its channel too (card #558).
            channel: if a.thread {
                clean_channel(&a.channel)
            } else {
                String::new()
            },
            assistant: a.thread && a.assistant,
            ..Ask::default()
        });
    }

    pub(crate) fn apply_take(&mut self, ev: &Event, at: &dyn Fn(GoTime) -> GoTime) {
        match take_kind(&ev.kind) {
            kinds::TAKE_REQUESTED => {
                let Some(p) = row::<TakeRequestedRow>(&ev.payload) else {
                    return;
                };
                if p.task_id.is_empty() || p.by.is_empty() {
                    return;
                }
                // On a board any member's agent may ask to take it, from a
                // device of its person (§9); privately, from its device.
                let ok = if self.on_board {
                    self.may_write(&p.by, &ev.device_id)
                } else {
                    owner::may_write(&p.by, &ev.device_id) && !owner::device_of(&p.by).is_empty()
                };
                if !ok {
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
                    // A former owner's row in flight as the ask moved isn't
                    // a device writing what it mustn't (card #541).
                    if !self.asks.moved.contains_key(&p.task_id) {
                        self.ignore(&ev.device_id);
                    }
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
                let was_handed = a.handed;
                // An agent Irori started for this ask went away holding it
                // (card #470): it crashed, as a card's does, unless Irori
                // ended it (finished, as LEFT_FINISHED is for a card); the
                // person asking again starts the count over.
                let gone_was = cmp_or_dev(&p.was, &was).to_string();
                if p.reason == TAKE_GONE
                    && self
                        .spawned
                        .get(&gone_was)
                        .is_some_and(|m| m.task_id == p.task_id)
                    && !self.ended_by_irori.contains(&gone_was)
                {
                    t.spawn_crashes += 1;
                } else if p.reason == TAKE_REOPENED {
                    t.spawn_crashes = 0;
                }
                a.taker = p.taker.clone();
                a.handed = p.handed && !p.taker.is_empty();
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
                let opened = t.ask.as_ref().is_some_and(|a| !a.opener.is_empty());
                let was_who = cmp_or_dev(&p.was, &was).to_string();
                if !p.taker.is_empty() && p.taker != was && p.handed {
                    let what = format!("handed it to {}", p.taker);
                    self.touch(ev, &ti, when, SWEEPER, &what, false, Vec::new());
                } else if !p.taker.is_empty() && (p.taker != was || was_handed && !p.handed) {
                    self.tasks.get_mut(&p.task_id).expect("present").updated_by = p.taker.clone();
                    self.touch(ev, &ti, when, &p.taker, "took it", true, Vec::new());
                } else if p.reason == TAKE_PASSED {
                    let what = "passed it on: its turn ended without taking it";
                    self.touch(ev, &ti, when, &was_who, what, false, Vec::new());
                } else if p.reason == TAKE_RELEASED {
                    self.touch(ev, &ti, when, &was_who, "gave it back", true, Vec::new());
                } else if p.reason == TAKE_GONE {
                    let what = format!("opened it again: {was_who} went away");
                    self.touch(ev, &ti, when, SWEEPER, &what, true, Vec::new());
                } else if p.reason == TAKE_REOPENED {
                    // An agent's thread goes out again on its person's
                    // post, not the agent's (card #631).
                    let (who, what) = if opened {
                        (SWEEPER, "opened it again: the person wrote in it")
                    } else {
                        (created_by.as_str(), "asked the room again")
                    };
                    self.touch(ev, &ti, when, who, what, true, Vec::new());
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
                // From the person's thread (card #155), not an ask it closed.
                let from_thread = self
                    .tasks
                    .get(&p.from)
                    .is_some_and(|f| f.ask.as_ref().is_some_and(|a| a.thread));
                // Before the card is looked up: a move that comes first is
                // kept, and counts once the card lands (card #563).
                let moved = p.board.as_ref().and_then(|b| {
                    self.fold_move(&p.task_id, RowPos::of(ev), b, p.num)
                        .then(|| self.moved_line(b))
                });
                let another = self.edited_by_another(ev, &p.task_id, &p.by);
                // A name the person gave stays theirs (card #779).
                let by_person = self.person_row(ev, &p.by);
                let irori_title = irori_title_row(ev, &p);
                let Some(t) = self.tasks.get_mut(&p.task_id) else {
                    return;
                };
                // What changed, from what it was: the thread is the card's log.
                let mut changed: Vec<String> = moved.into_iter().collect();
                // A move leaves the words as they were (card #563).
                let moved_lines = changed.len();
                if let Some(title) = &p.title
                    && !trim_space(title).is_empty()
                    && (by_person || !t.title_by_person)
                {
                    if by_person {
                        t.title_by_person = true;
                    }
                    // Irori's assistant names a thread (card #782): what
                    // it was is kept for Undo.
                    if *title != t.title {
                        if irori_title {
                            t.irori_titled += 1;
                            t.title_was = t.title.clone();
                        } else {
                            // Someone else renamed it: the name isn't
                            // Irori's any more, so there is nothing to undo.
                            t.title_was.clear();
                        }
                    }
                    if *title != t.title {
                        changed.push(format!(
                            "renamed it from “{}” to “{}”",
                            cut(&t.title, MAX_LOG_QUOTE),
                            cut(title, MAX_LOG_QUOTE)
                        ));
                    }
                    t.title = title.clone();
                }
                if let Some(o) = &p.objective {
                    if *o != t.objective {
                        changed.push(set_or_cleared("the objective", o));
                    }
                    t.objective = o.clone();
                }
                if let Some(d) = &p.done_criteria {
                    if *d != t.done_criteria {
                        changed.push(set_or_cleared("done when", d));
                    }
                    t.done_criteria = d.clone();
                }
                // Its repo (card #600): set, never cleared, by a row.
                if let Some(r) = &p.repo_key {
                    let r = ledger::canonical_repo_key(r);
                    if !r.is_empty() && r != t.repo_key {
                        changed.push(format!("set its repo to {}", ledger::repo_name(&r)));
                        t.repo_key = r;
                    }
                }
                // A yes is for the words they read: someone else changing
                // them asks again (card #573).
                if changed.len() > moved_lines && another && !irori_title {
                    super::cards::close_given_question(t, at(p.at));
                    t.given_said = None;
                }
                // A thread's "Agents answer now" (card #391); edit_gate let
                // only a person's row carry it.
                if let Some(on) = p.agents_now
                    && let Some(a) = t.ask.as_mut().filter(|a| a.thread)
                    && a.wake != on
                {
                    a.wake = on;
                    changed.push(format!(
                        "turned Agents answer now {}",
                        if on { "on" } else { "off" }
                    ));
                }
                // Irori's name that didn't take (the person named it
                // meanwhile, or it already read so) leaves no line, as on
                // a device that folded the person's row after it.
                if irori_title && changed.is_empty() {
                    return;
                }
                let when = at(p.at);
                t.updated_at = when;
                t.updated_by = p.by.clone();
                let mut what = if changed.is_empty() {
                    "edited the card".to_string()
                } else {
                    changed.join("; ")
                };
                if from_thread {
                    what = format!("{what} (from the thread {from_ref})");
                } else if !p.from.is_empty() {
                    let mut w = format!("changed it from the ask {from_ref}");
                    if !p.note.is_empty() {
                        w.push_str(&format!(": {}", p.note));
                    }
                    if !changed.is_empty() {
                        w.push_str(&format!("; {what}"));
                    }
                    what = w;
                }
                let ti = Touch::of(t);
                let holder = t.holder.clone();
                // Irori naming a thread wakes nobody (card #782): the line
                // is on the thread, with its Undo.
                self.touch(ev, &ti, when, &p.by, &what, !irori_title, Vec::new());
                if from_thread {
                    self.fold_made(ev, &p.from, &p.task_id, when, &p.by, "changed");
                }
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
        self.links_moved = true;
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
        match take_kind(&ev.kind) {
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
        // Irori's assistant names a thread on the person's own log (card
        // #782): a title and nothing else. A shared board takes no row from
        // Irori, so its threads aren't named.
        if irori_title_row(ev, &p) {
            if self.on_board {
                self.ignore(&ev.device_id);
                return (false, String::new());
            }
            return (true, String::new());
        }
        // Only a person turns a thread's Agents answer now (card #391).
        if p.agents_now.is_some() {
            self.ignore(&ev.device_id);
            return (false, String::new());
        }
        if !self.may_write(&p.by, &ev.device_id) {
            self.ignore(&ev.device_id);
            return (false, String::new());
        }
        let mut ask = p.from.clone();
        if ask == p.task_id {
            return (false, String::new());
        }
        // board_edit's from names the person's thread the change came from
        // (card #155): a link, not a lock. Only ask_close's edit outcome
        // names the ask being closed, whose taker gates it (card #318).
        if self
            .tasks
            .get(&ask)
            .is_some_and(|f| f.ask.as_ref().is_some_and(|a| a.thread))
        {
            ask.clear();
        }
        if ask.is_empty() {
            let Some(t) = self.tasks.get(&p.task_id) else {
                return (p.board.is_some(), String::new());
            };
            // Any agent may edit any card (card #320). A card that was an
            // ask edits like any card once the ask is closed (board_edit).
            if t.open_ask().is_none() {
                return (true, String::new());
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
        let (by, text) = super::comments::check_comment(
            by,
            &input.text,
            &input.files,
            self.is_owner(trim_space(by)),
        )?;
        if !self.is_owner(&by) {
            return Err(not_allowed("only the person asks the room"));
        }
        // On a shared board only board-form keys (agent:<cardId>,
        // person:<personId>) go in the log: every member's Core resolves
        // them (card #187, as #177 did for comments).
        let mut given = input.mentions.clone();
        if self.on_board {
            given.retain(|m| super::mention::board_form(&m.key));
        }
        let mentions = self.check_mentions(&by, &given)?;
        let repo_key = ledger::canonical_repo_key(&input.repo_key);
        if !repo_key.is_empty() {
            check_repo_key(&repo_key)?;
        }
        let at = self.stamp();
        let id = new_id(&at);
        let num = self.next_short("");
        // A device with no agents of its own (the phone) leaves the ask to
        // one of the person's computers (card #408): its roster may be
        // hours old when it posts, and it sleeps right after.
        // The side of the device it's typed on (card #494); its owner is a
        // device that runs that side (card #495).
        let side = self.new_side("", "");
        let owner = self.ask_owner(agents, &side);
        let delegated = owner != self.dev;
        self.write(
            &by,
            kinds::TASK_CREATED,
            &TaskCreatedRow {
                task_id: id.clone(),
                num,
                title: ask_title(&text),
                objective: text.clone(),
                repo_key: repo_key.clone(),
                // An ask goes only to idle agents, and an idle agent gets no
                // next turn until someone types in its window: an ask that
                // doesn't wake reaches nobody. input.wake is kept for old
                // callers and ignored. A thread's wake is its "Agents answer
                // now" (card #391): off, nobody is woken or handed it.
                ask: Some(AskField {
                    wake: !(input.thread && input.quiet),
                    repo_key: repo_key.clone(),
                    mentions: mentions.clone(),
                    thread: input.thread,
                    board: if self.on_board || !input.thread {
                        String::new()
                    } else {
                        input.board.trim().to_string()
                    },
                    owner: if delegated { owner } else { String::new() },
                    channel: if input.thread {
                        clean_channel(&input.channel)
                    } else {
                        String::new()
                    },
                    assistant: input.thread && input.assistant,
                }),
                side,
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
                    text: ASK_FILES_NOTE.into(),
                    at,
                    attachments: input.files.clone(),
                    ..TaskCommentRow::default()
                },
            )?;
        }
        let t = self.tasks[&id].clone();
        if delegated {
            // Its owner hands it out at its next sweep, kicked by this row.
            let mut n = ask_notice(&t, &[], true, "");
            n.attachments = input.files;
            return Ok((t, n));
        }
        // A thread goes first to the agents it names (card #116); with
        // none, it is handed out like any ask.
        if input.thread {
            let heard = self.thread_hearers(&text, &mentions);
            if input.quiet {
                // Agents answer now is off (card #391): the agents it names
                // hear it at their next turn, and it's never handed out.
                self.write_take(
                    &id,
                    TakeStateRow {
                        reason: TAKE_POSTED.into(),
                        told: heard.len() as i64,
                        heard: heard.clone(),
                        since: at,
                        ..TakeStateRow::default()
                    },
                )?;
                let t = self.tasks[&id].clone();
                let mut n = super::threads::thread_quiet_notice(&t, &heard);
                n.attachments = input.files;
                return Ok((t, n));
            }
            if !heard.is_empty() {
                self.write_take(
                    &id,
                    TakeStateRow {
                        reason: TAKE_POSTED.into(),
                        told: heard.len() as i64,
                        heard: heard.clone(),
                        since: at,
                        ..TakeStateRow::default()
                    },
                )?;
                let t = self.tasks[&id].clone();
                let mut n = super::threads::thread_notice(&t, &heard);
                n.attachments = input.files;
                return Ok((t, n));
            }
        }
        // Irori hands it to one idle agent, which holds it as it wakes: no
        // race on ask_claim, one turn spent. Nobody idle: it waits for one.
        let free = self.free_agents(&t, agents, &self.handed_elsewhere(&id));
        let pick: Vec<String> = free.into_iter().take(1).collect();
        self.write_take(
            &id,
            TakeStateRow {
                taker: pick.first().cloned().unwrap_or_default(),
                reason: if pick.is_empty() {
                    TAKE_UNCLAIMED.into()
                } else {
                    TAKE_HANDED.into()
                },
                handed: !pick.is_empty(),
                told: pick.len() as i64,
                heard: pick.clone(),
                since: at,
                unclaimed: pick.is_empty(),
                ..TakeStateRow::default()
            },
        )?;
        let t = self.tasks[&id].clone();
        let mut n = match pick.first() {
            Some(k) if t.ask.as_ref().is_some_and(|a| a.thread) => {
                super::threads::thread_hand_notice(&t, k, &[])
            }
            Some(k) => hand_notice(&t, k, ""),
            None => ask_notice(&t, &[], true, ""),
        };
        n.attachments = input.files;
        Ok((t, n))
    }

    /// An agent opens a thread itself (card #631): a conversation with the
    /// person, in the private log or a shared board's. It holds it from the
    /// start, on its own device, so Irori never hands it out or starts an
    /// agent for it; the agents it names hear it, woken, as for the
    /// person's thread. Once its person writes in it, it goes like their
    /// own (card #512). Returns the notice for the agents it names, if any.
    pub(crate) fn post_agent_thread(
        &mut self,
        by: &str,
        input: NewAsk,
    ) -> Result<(Task, Option<Notice>)> {
        let (by, text) = super::comments::check_comment(by, &input.text, &[], false)?;
        let mine = if self.on_board {
            self.may_write(&by, &self.dev.clone())
        } else {
            owner::may_write(&by, &self.dev) && !owner::device_of(&by).is_empty()
        };
        if by == SWEEPER
            || self.is_person(&by)
            || super::threads::names_person(&by)
            || by.starts_with("api:")
            || !mine
        {
            return Err(not_allowed(
                "only an agent on this device opens a thread here; the person starts theirs in the app",
            ));
        }
        if !input.files.is_empty() {
            return Err(invalid("a thread an agent opens carries no files"));
        }
        let mut given = input.mentions.clone();
        if self.on_board {
            given.retain(|m| super::mention::board_form(&m.key));
        }
        let mentions = self.check_mentions(&by, &given)?;
        let repo_key = ledger::canonical_repo_key(&input.repo_key);
        if !repo_key.is_empty() {
            check_repo_key(&repo_key)?;
        }
        let at = self.stamp();
        let id = new_id(&at);
        let num = self.next_short("");
        let side = self.new_side("", "");
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
                    wake: true,
                    repo_key,
                    mentions: mentions.clone(),
                    thread: true,
                    board: if self.on_board {
                        String::new()
                    } else {
                        input.board.trim().to_string()
                    },
                    owner: String::new(),
                    channel: clean_channel(&input.channel),
                    assistant: false,
                }),
                side,
                by: by.clone(),
                at,
                ..TaskCreatedRow::default()
            },
        )?;
        if !self.tasks[&id].ask.as_ref().is_some_and(|a| a.opener == by) {
            return Err(not_allowed(
                "the thread didn't open: Irori couldn't tell it was yours",
            ));
        }
        let heard: Vec<String> = self
            .thread_hearers(&text, &mentions)
            .into_iter()
            .filter(|k| *k != by)
            .collect();
        self.write_take(
            &id,
            TakeStateRow {
                taker: by.clone(),
                reason: TAKE_TAKEN.into(),
                told: heard.len() as i64,
                heard: heard.clone(),
                since: at,
                ..TakeStateRow::default()
            },
        )?;
        let t = self.tasks[&id].clone();
        let n = (!heard.is_empty()).then(|| super::threads::thread_notice(&t, &heard));
        Ok((t, n))
    }

    /// The files the person posted with ask `id` (the first comment
    /// post_ask writes), for a notice that hands it out later.
    pub(crate) fn ask_files(&self, id: &str) -> Vec<FileRef> {
        self.list_comments(id)
            .unwrap_or_default()
            .into_iter()
            .find(|c| c.text == ASK_FILES_NOTE && self.is_owner(&c.by))
            .map(|c| c.attachments)
            .unwrap_or_default()
    }

    /// The agents holding another open ask: one ask per agent at a time.
    /// An unheld card Irori handed to an agent counts too (#82).
    pub(crate) fn handed_elsewhere(&self, id: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .tasks
            .values()
            .filter(|t| t.id != id && !is_terminal(&t.state))
            .filter_map(|t| t.open_ask())
            // A thread an agent opened itself doesn't keep it from others
            // (card #631).
            .filter(|a| !a.taker.is_empty() && a.taker != a.opener)
            .map(|a| a.taker.clone())
            .collect();
        out.extend(self.offered_elsewhere(id));
        out
    }

    /// Where an ask handed to `a.taker` stands, by events only: a comment
    /// from it, its turn ending, or it going away. Never by time waited.
    fn handed_state(&self, id: &str, a: &Ask, roster: &HashMap<&str, &AgentCard>) -> Handed {
        if self
            .comments
            .values()
            .any(|c| c.task_id == id && c.by == a.taker && !gotime::before(&c.at, &a.since))
        {
            return Handed::Touched;
        }
        self.turn_state(&a.taker, &a.since, roster)
    }

    /// Where an agent something was handed to at `since` stands, by
    /// events only: its turn ended (idle again, with transcript rows and a
    /// status change since), it went away, or it still has it. Shared by
    /// asks (#91) and unheld cards (#82).
    pub(crate) fn turn_state(
        &self,
        taker: &str,
        since: &GoTime,
        roster: &HashMap<&str, &AgentCard>,
    ) -> Handed {
        let Some(card) = roster.get(taker) else {
            return Handed::Gone;
        };
        let dev = cmp_or_dev(&card.device_id, owner::device_of(&card.key));
        // Not answering (#110): its wakes go unread, so a card or ask
        // handed to it moves on as if it went away.
        if !crate::roster::answers(&card.status)
            || card.device_state == "offline"
            || self
                .revoked
                .as_ref()
                .is_some_and(|r| !dev.is_empty() && r(dev))
        {
            return Handed::Gone;
        }
        // Idle again, with transcript rows and a status change since it was
        // handed: a turn ran and ended without touching it.
        if card.status == "idle"
            && gotime::after(&card.updated_at, since)
            && gotime::after(&card.last_activity, since)
        {
            return Handed::Ended;
        }
        Handed::Waits
    }

    /// The device that hands out an ask posted here (card #408): this one
    /// when it runs agents; otherwise another of the person's devices that
    /// does, connected first, then the one with the most agents. With none
    /// known, this one, as before. Only a device that runs the ask's
    /// `side` (card #495): with none, this one, and the ask waits.
    fn ask_owner(&self, roster: &[AgentCard], side: &str) -> String {
        let dev_of = |a: &AgentCard| cmp_or_dev(&a.device_id, owner::device_of(&a.key)).to_string();
        // A spawned agent runs only its own card (card #446), and a
        // terminal agent takes asks only with "Offer it cards" on (card
        // #453): neither makes its device one that hands out asks.
        let roster: Vec<&AgentCard> = roster
            .iter()
            .filter(|a| self.takes_handout(&a.key))
            .filter(|a| self.sides.runs(&dev_of(a), side))
            .collect();
        if roster.iter().any(|a| dev_of(a) == self.dev) && self.can_run(&self.dev) {
            return self.dev.clone();
        }
        let mut devs: BTreeMap<String, (u8, usize)> = BTreeMap::new();
        for a in roster {
            let dev = dev_of(a);
            if dev.is_empty()
                || a.key == SWEEPER
                || self.is_owner(&a.key)
                || !crate::roster::answers(&a.status)
                || self.revoked.as_ref().is_some_and(|r| r(&dev))
                || !self.same_person_dev(&dev, &self.dev)
            {
                continue;
            }
            let rank = match a.device_state.as_str() {
                crate::roster::CONNECTED | "" => 0,
                crate::roster::RECENT => 1,
                _ => 2,
            };
            let e = devs.entry(dev).or_insert((rank, 0));
            e.0 = e.0.min(rank);
            e.1 += 1;
        }
        let pick = devs
            .into_iter()
            .filter(|(d, _)| self.can_run(d))
            .min_by(|(x, (rx, nx)), (y, (ry, ny))| rx.cmp(ry).then(ny.cmp(nx)).then(x.cmp(y)))
            .map_or_else(|| self.dev.clone(), |(d, _)| d);
        // A device that can't run agents never keeps an ask (card #541):
        // with no agent offered anywhere, a computer starts one.
        if self.can_run(&pick) {
            return pick;
        }
        self.agent_home(side).unwrap_or(pick)
    }

    /// Takes to this device the waiting asks whose owner can't serve them
    /// (card #541), when this is the computer such work goes to: the owner
    /// can't run agents, or has been away for [`ASK_AWAY`]. The computer
    /// pulls rather than the owner giving: a phone asleep or a computer
    /// with its lid shut writes nothing, and all the rows that follow
    /// (take lock, spawn) are this device's own, in order.
    ///
    /// An owner that is connected but full (card #686: at its cap, held by
    /// a usage limit, or paused) loses its ask to the first computer with
    /// room ([`Self::room_home`]); with none, it waits where it is. An ask
    /// moved this way settles for [`ASK_FULL_SETTLE`] and moves at most
    /// [`MAX_FULL_MOVES`] times, so it never goes back and forth.
    pub(crate) fn pull_asks(&mut self) -> Result<()> {
        if !self.can_run(&self.dev) {
            return Ok(());
        }
        let now = self.stamp();
        let mut waiting: Vec<&Task> = self
            .tasks
            .values()
            .filter(|t| self.waits_for_agent(t))
            .collect();
        waiting.sort_by(|x, y| {
            (x.created_at.unix_nano(), &x.id).cmp(&(y.created_at.unix_nano(), &y.id))
        });
        // Moves off a full owner take this device's free slots, oldest
        // first: one free slot doesn't take every waiting thread.
        let mut room = self.free_slots(&self.dev);
        let mut lost: Vec<(String, i64)> = Vec::new();
        for t in waiting {
            let Some(a) = t.open_ask() else {
                continue;
            };
            if a.owner == self.dev
                || !self.same_person_dev(&a.owner, &self.dev)
                // On a board every member's hello says so instead.
                || !(self.on_board || self.reads_moves(&a.owner))
            {
                continue;
            }
            let takes = if !self.can_run(&a.owner)
                || (self.away_for(&a.owner, ASK_AWAY)
                    && gotime::at_least(&now, &t.created_at, ASK_AWAY))
            {
                self.lost_home(&t.side, &a.owner).as_deref() == Some(self.dev.as_str())
            } else if room > 0 && self.full_move(t, &now) {
                room -= 1;
                true
            } else {
                false
            };
            if takes {
                let moves = self.asks.moved.get(&t.id).map_or(0, |m| m.0);
                lost.push((t.id.clone(), moves + 1));
            }
        }
        for (id, moves) in lost {
            let row = AskMovedRow {
                task_id: id,
                owner: self.dev.clone(),
                moves,
                at: now,
            };
            self.write_as(SWEEPER, self.ask_kind(kinds::ASK_MOVED), &row)?;
        }
        Ok(())
    }

    /// Whether this device takes ask t from `owner` because the owner is
    /// full (card #686): the owner is connected (one away is
    /// [`ASK_AWAY`]'s), this is the computer with room it goes to, and it
    /// hasn't moved too often or too lately.
    fn full_move(&self, t: &Task, now: &GoTime) -> bool {
        // The grace counts from when it last went out (a wake or reopen
        // sets since), not from when the thread was made.
        let Some(a) = t.open_ask() else {
            return false;
        };
        let since = if a.since.unix_nano() > t.created_at.unix_nano() {
            a.since
        } else {
            t.created_at
        };
        if !gotime::at_least(now, &since, ASK_FULL_GRACE) {
            return false;
        }
        if self
            .asks
            .moved_at
            .get(&t.id)
            .and_then(|v| v.last())
            .is_some_and(|at| !gotime::at_least(now, at, ASK_FULL_SETTLE))
        {
            return false;
        }
        self.full_target(t).as_deref() == Some(self.dev.as_str())
    }

    /// The computer ask t moves to because its owner is full (card #686):
    /// the owner is connected (one away is [`ASK_AWAY`]'s) and full, the
    /// ask hasn't moved [`MAX_FULL_MOVES`] times lately, and a computer has
    /// room. A short grace or settle may still hold the move a moment.
    pub(crate) fn full_target(&self, t: &Task) -> Option<String> {
        let a = t.open_ask()?;
        if !self.is_full(&a.owner) || self.is_away(&a.owner) {
            return None;
        }
        let now = self.stamp();
        let lately = self.asks.moved_at.get(&t.id).map_or(0, |v| {
            v.iter()
                .filter(|at| !gotime::at_least(&now, at, ASK_FULL_WINDOW))
                .count()
        });
        if lately >= MAX_FULL_MOVES {
            return None;
        }
        self.room_home(&t.side, &a.owner)
    }

    /// How much longer the "Nobody took" notice holds off for ask t (card
    /// #995), or None when it doesn't: a computer with room is about to
    /// take it, it just moved to one that has room, or its agent was just
    /// bound and the row saying it took the ask hasn't folded here yet.
    /// Each counts from its own moment and runs out by itself, so an ask
    /// nobody ends up taking is still said. A request still waiting for its
    /// agent is [`Task::spawn_pending`], which the notice checks itself.
    pub(crate) fn ask_notice_hold(&self, t: &Task) -> Option<Duration> {
        let a = t.open_ask()?;
        if !a.unclaimed || !a.taker.is_empty() {
            return None;
        }
        let opened = a.since.unix_nano().max(t.created_at.unix_nano());
        // From this opening only: an agent or move from an earlier one says
        // nothing about this one.
        let bound = t
            .spawn_request()
            .filter(|s| s.op == super::spawn::SPAWN_REQUESTED && !s.key.is_empty())
            .and_then(|s| self.spawned_for(&s.key))
            .map(|s| s.at.unix_nano())
            .filter(|at| *at >= opened);
        let moved = self
            .asks
            .moved_at
            .get(&t.id)
            .and_then(|v| v.last())
            .map(GoTime::unix_nano)
            .filter(|at| *at >= opened && !self.is_full(&a.owner));
        // What pull_asks needs before it would write the move. One that
        // just moved can't move again until it settles, so the hold counts
        // from then.
        let settled = self
            .asks
            .moved_at
            .get(&t.id)
            .and_then(|v| v.last())
            .map_or(0, |at| {
                at.unix_nano()
                    .saturating_add(ASK_FULL_SETTLE.as_nanos() as i64)
            });
        let target = (self.would_wait_for_agent(t)
            && (self.on_board || self.reads_moves(&a.owner))
            && self.full_target(t).is_some())
        .then_some(opened.max(settled));
        let from = [bound, moved, target].into_iter().flatten().max()?;
        let hold = self.ask_room_hold;
        let gone = self.stamp().unix_nano().saturating_sub(from).max(0) as u128;
        // A moment still ahead of this clock (the settle, or another
        // device's stamp within the skew the store allows) holds until this
        // clock reaches it, then for the bound.
        hold.checked_sub(Duration::from_nanos(gone.min(u64::MAX as u128) as u64))
            .filter(|left| !left.is_zero())
    }

    /// Where an ask whose owner can't serve it goes (card #541): the ask
    /// home, or a computer with room when that one is full (card #686).
    fn lost_home(&self, side: &str, owner: &str) -> Option<String> {
        let home = self.agent_home(side)?;
        if self.is_full(&home) {
            return self.room_home(side, owner).or(Some(home));
        }
        Some(home)
    }

    /// Why ask t waits on its full owner (card #686), or "" when it doesn't:
    /// its owner has no free slot and no other computer has one either. A
    /// computer with room takes it, so this is said only when none has.
    pub(crate) fn ask_full_wait(&self, t: &Task) -> String {
        let Some(a) = t.open_ask() else {
            return String::new();
        };
        if !self.waits_for_agent(t)
            || !self.is_full(&a.owner)
            || self.room_home(&t.side, &a.owner).is_some()
        {
            return String::new();
        }
        ALL_FULL.to_string()
    }

    /// An `ask.moved` row: a device of the same person takes the ask. The
    /// most moves wins, then the lowest device id, whatever order the rows
    /// arrive in, so two computers taking one ask at once agree on which.
    pub(crate) fn apply_ask_moved(&mut self, ev: &Event) {
        let Some(p) = row::<AskMovedRow>(&ev.payload) else {
            return;
        };
        let Some(owner) = self
            .tasks
            .get(&p.task_id)
            .and_then(|t| t.ask.as_ref())
            .map(|a| a.owner.clone())
        else {
            return;
        };
        if p.owner != ev.device_id || !self.same_person_dev(&owner, &ev.device_id) {
            self.ignore(&ev.device_id);
            return;
        }
        let wins =
            self.asks.moved.get(&p.task_id).is_none_or(|(moves, dev)| {
                p.moves > *moves || (p.moves == *moves && p.owner < *dev)
            });
        if !wins {
            return;
        }
        self.asks
            .moved
            .insert(p.task_id.clone(), (p.moves, p.owner.clone()));
        let times = self.asks.moved_at.entry(p.task_id.clone()).or_default();
        times.push(p.at);
        times.sort_by_key(|t| t.unix_nano());
        if let Some(a) = self.tasks.get_mut(&p.task_id).and_then(|t| t.ask.as_mut()) {
            a.owner = p.owner;
        }
        // Rows are ordered within one device's log: the new owner's seqs
        // start over.
        self.asks.state_seq.remove(&p.task_id);
        self.note(Change {
            kind: TASK_CHANGED.into(),
            task_id: p.task_id,
            ..Change::default()
        });
    }

    /// Whether devices a and b are the same person's: on the private log
    /// every device is; on a board, by the members' certificates.
    pub(crate) fn same_person_dev(&self, a: &str, b: &str) -> bool {
        if !self.on_board {
            return true;
        }
        match &self.dev_person {
            Some(dp) => {
                let p = dp(a);
                !p.is_empty() && p == dp(b)
            }
            None => a == b,
        }
    }

    /// The agents among `roster` that may take ask t now, the one idle
    /// longest first.
    fn free_agents(&self, t: &Task, roster: &[AgentCard], skip: &[String]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        // An ask with mentions goes to them alone, idle or not, in any repo,
        // in the order the text names them: those still in the roster and
        // not gone. Like any ask it is handed to one at a time (#91).
        if let Some(ask) = &t.ask
            && !ask.mentions.is_empty()
            && !ask.thread
        {
            for m in &ask.mentions {
                let here = roster
                    .iter()
                    .any(|a| a.key == m.key && crate::roster::answers(&a.status));
                if m.gone || !here || skip.contains(&m.key) || out.contains(&m.key) {
                    continue;
                }
                out.push(m.key.clone());
            }
            return out;
        }
        let mut since: HashMap<String, GoTime> = HashMap::new();
        for a in roster {
            let dev = cmp_or_dev(&a.device_id, owner::device_of(&a.key));
            if a.key.is_empty()
                || a.key == SWEEPER
                || self.is_owner(&a.key)
                || a.status != "idle"
                || !self.takes_handout(&a.key)
            {
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
                && !ledger::same_repo_key(&a.repo_key, &ask.repo_key)
            {
                continue;
            }
            // Only an agent on a device that runs the ask's side (card #495).
            if !self.sides.runs(dev, &t.side) {
                continue;
            }
            if skip.contains(&a.key) || out.contains(&a.key) {
                continue;
            }
            out.push(a.key.clone());
            since.insert(a.key.clone(), a.updated_at);
        }
        // Idle longest first (its status last changed earliest), then by key.
        out.sort_by(|x, y| gotime::cmp(&since[x], &since[y]).then_with(|| x.cmp(y)));
        out
    }

    /// Writes the owner's take.state for ask `id`.
    pub(crate) fn write_take(&mut self, id: &str, mut r: TakeStateRow) -> Result<()> {
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
        self.write_as(SWEEPER, self.ask_kind(kinds::TAKE_STATE), &r)
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
                if a.taker == r.by && a.handed {
                    return self.write_take(
                        id,
                        TakeStateRow {
                            taker: r.by.clone(),
                            reason: TAKE_TAKEN.into(),
                            req: Some(r.r.clone()),
                            told: a.told,
                            heard: a.heard.clone(),
                            ..TakeStateRow::default()
                        },
                    );
                }
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
                // A thread an agent opened (card #631): never handed out
                // while it rests, whatever happened to its opener.
                let gone = |f: &Fold, k: &str| f.taker_gone(k, &in_roster, &now);
                if let Some(ag) = self.agent_thread_sweep(id, &gone)? {
                    again.insert(id.clone(), ag);
                }
                if self.agent_thread_rests(&self.tasks[id]) {
                    again.remove(id);
                    if self.tasks[id]
                        .ask
                        .as_ref()
                        .is_some_and(|a| !a.taker.is_empty())
                    {
                        self.thread_sweep(id)?;
                    }
                    continue;
                }
                if self.thread_sweep(id)? {
                    continue;
                }
                let a = self.tasks[id].ask.clone().unwrap_or_default();
                if !a.taker.is_empty() && a.handed {
                    match self.handed_state(id, &a, &in_roster) {
                        Handed::Waits => continue,
                        Handed::Touched if a.thread => {
                            self.write_thread_answered(id, &a)?;
                            continue;
                        }
                        Handed::Touched => {
                            self.write_take(
                                id,
                                TakeStateRow {
                                    taker: a.taker.clone(),
                                    reason: TAKE_TAKEN.into(),
                                    told: a.told,
                                    heard: a.heard.clone(),
                                    ..TakeStateRow::default()
                                },
                            )?;
                            continue;
                        }
                        Handed::Ended => {
                            self.write_take(
                                id,
                                TakeStateRow {
                                    reason: TAKE_PASSED.into(),
                                    was: a.taker.clone(),
                                    told: a.told,
                                    heard: a.heard.clone(),
                                    since: now,
                                    ..TakeStateRow::default()
                                },
                            )?;
                            again.insert(
                                id.clone(),
                                AskAgain {
                                    skip: a.taker.clone(),
                                    why: "the agent it went to ended its turn without taking it"
                                        .into(),
                                    reason: TAKE_PASSED.into(),
                                },
                            );
                        }
                        Handed::Gone => {
                            self.write_take(
                                id,
                                TakeStateRow {
                                    reason: TAKE_GONE.into(),
                                    was: a.taker.clone(),
                                    told: a.told,
                                    heard: a.heard.clone(),
                                    since: now,
                                    ..TakeStateRow::default()
                                },
                            )?;
                            again.insert(
                                id.clone(),
                                AskAgain {
                                    skip: a.taker.clone(),
                                    why: "the agent it went to went away".into(),
                                    reason: TAKE_GONE.into(),
                                },
                            );
                        }
                    }
                } else if !a.taker.is_empty() {
                    if !self.taker_gone(&a.taker, &in_roster, &now) {
                        continue;
                    }
                    self.write_take(
                        id,
                        TakeStateRow {
                            reason: TAKE_GONE.into(),
                            was: a.taker.clone(),
                            told: a.told,
                            heard: a.heard.clone(),
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
                        },
                    );
                }
                let t = self.tasks[id].clone();
                let a = t.ask.clone().unwrap_or_default();
                if !a.taker.is_empty() {
                    continue;
                }
                // An agent Irori starts for it is on its way (card #470):
                // nobody else is handed it meanwhile, as for a card.
                if t.spawn_request()
                    .is_some_and(|s| s.pending() && !s.stale(&now))
                {
                    again.remove(id);
                    continue;
                }
                // Posted on another device and left to this one (card
                // #408): a thread goes first to the agents it names, as
                // post_ask does.
                if a.thread && a.reason.is_empty() && !again.contains_key(id) {
                    let heard = self.thread_hearers(&t.objective, &a.mentions);
                    if !heard.is_empty() {
                        self.write_take(
                            id,
                            TakeStateRow {
                                reason: TAKE_POSTED.into(),
                                told: heard.len() as i64,
                                heard: heard.clone(),
                                since: now,
                                ..TakeStateRow::default()
                            },
                        )?;
                        let t = self.tasks[id].clone();
                        let mut n = super::threads::thread_notice(&t, &heard);
                        n.attachments = self.ask_files(id);
                        out.push(n);
                        continue;
                    }
                }
                let ag = again.remove(id);
                // Asked again by the person: everyone may hear it again.
                let fresh = ag.as_ref().is_some_and(|g| g.reason == TAKE_REOPENED);
                let mut heard = if fresh { Vec::new() } else { a.heard.clone() };
                if let Some(g) = &ag
                    && !g.skip.is_empty()
                    && !heard.contains(&g.skip)
                {
                    heard.push(g.skip.clone());
                }
                // One ask per agent: an agent another ask went to isn't free.
                let mut skip = heard.clone();
                skip.extend(self.handed_elsewhere(id));
                let free = self.free_agents(&t, agents, &skip);
                if let Some(pick) = free.first() {
                    heard.push(pick.clone());
                    self.write_take(
                        id,
                        TakeStateRow {
                            taker: pick.clone(),
                            reason: TAKE_HANDED.into(),
                            handed: true,
                            told: heard.len() as i64,
                            heard,
                            since: now,
                            ..TakeStateRow::default()
                        },
                    )?;
                    let t = self.tasks[id].clone();
                    let why = ag.as_ref().map(|g| g.why.as_str()).unwrap_or_default();
                    let mut n = if a.thread {
                        super::threads::thread_hand_notice(&t, pick, &a.heard)
                    } else {
                        hand_notice(&t, pick, why)
                    };
                    // Handed later, it still brings the ask's files (#124).
                    n.attachments = self.ask_files(id);
                    out.push(n);
                    continue;
                }
                // Nobody idle to hand it to: it waits, and goes out the
                // moment an agent goes idle (the next sweep, kicked by the
                // roster change).
                if a.unclaimed && ag.is_none() {
                    continue;
                }
                // The person's "Wake them" stays a reopen, so their card
                // can say they woke them and nobody was free.
                let reason = if fresh { TAKE_REOPENED } else { TAKE_UNCLAIMED };
                self.write_take(
                    id,
                    TakeStateRow {
                        reason: reason.into(),
                        told: heard.len() as i64,
                        heard,
                        since: if ag.is_some() { now } else { a.since },
                        unclaimed: true,
                        ..TakeStateRow::default()
                    },
                )?;
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
        roster
            .get(taker)
            .is_none_or(|a| !crate::roster::answers(&a.status))
    }

    // ---- claim, close ----

    /// Writes the claim request and runs the duties; returns the request's
    /// index for the wait.
    pub(crate) fn ask_claim_start(
        &mut self,
        by: &str,
        id: &str,
    ) -> Result<std::result::Result<ClaimResult, usize>> {
        // An ask on a side this agent's device doesn't run is taken only
        // when Irori hands it over or the person asks (card #496).
        let t0 = self.open_task(id)?.clone();
        if !t0.open_ask().is_some_and(|a| a.taker == by && a.handed) {
            self.check_side_take(by, &t0)?;
        }
        let t = self.open_task(id)?;
        let Some(a) = t.open_ask() else {
            return Err(bad_state(format!("{} isn't an open ask", t.short_ref())));
        };
        if a.taker == by {
            let (handed, t) = (a.handed, t.clone());
            // Handed to it: the claim tells the owner it's taken, no wait.
            if handed {
                let at = self.stamp();
                self.write(
                    by,
                    self.ask_kind(kinds::TAKE_REQUESTED),
                    &TakeRequestedRow {
                        task_id: id.into(),
                        by: by.into(),
                        op: TAKE_CLAIM.into(),
                        at,
                    },
                )?;
                // It hears its answer here, not as a "taken" notice.
                if let Some(r) = self.asks.reqs.get_mut(id).and_then(|v| v.last_mut()) {
                    r.waiting = true;
                }
                self.ask_duties();
                if let Some(r) = self.asks.reqs.get_mut(id).and_then(|v| v.last_mut()) {
                    r.waiting = false;
                }
                self.sync_waiting(id);
            }
            let t = self.tasks.get(id).cloned().unwrap_or(t);
            let owner = t.ask.as_ref().map(|a| a.owner.clone()).unwrap_or_default();
            return Ok(Ok(ClaimResult {
                task: t,
                taker: by.into(),
                mine: true,
                pending: false,
                owner,
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
            self.ask_kind(kinds::TAKE_REQUESTED),
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
        // On a board, a thread is the member's who started it (card #134).
        if a.thread && (!person || (self.on_board && !self.started_by(&t, &a, &by))) {
            return Err(not_allowed(format!(
                "{} is the person's thread: they resolve it",
                t.short_ref()
            )));
        }
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
            // The card it becomes runs on its parent's side: an agent puts
            // it there only on a side it may take (card #496).
            if !person && let Some(pt) = self.tasks.get(&pid).cloned() {
                self.check_side_take(&by, &pt)?;
            }
            // A held parent holds the card, so one its taker keeps can't
            // go there (finding 21); and it checks circles (finding 3).
            let keeps = !a.thread && !a.taker.is_empty() && !self.is_owner(&a.taker);
            if keeps && self.tasks.get(&pid).is_some_and(|p| p.parked().is_some()) {
                return Err(bad_state(format!(
                    "{} is Not yet, and {} stays with {}: close it without a parent, or bring {} back first",
                    self.ref_of(&pid),
                    t.short_ref(),
                    a.taker,
                    self.ref_of(&pid)
                )));
            }
            self.check_ask_adopt(&pid, id)?;
            parent = pid;
        }
        let opt = |s: &str| (!s.is_empty()).then(|| s.to_string());
        match o.kind.as_str() {
            OUTCOME_CARD => {
                if title.is_empty() {
                    return Err(invalid("a card outcome needs a title"));
                }
                self.check_person_title(&by, &t, &title)?;
                self.write(
                    &by,
                    self.ask_kind(kinds::TASK_EDITED),
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
                // A thread's agent answers; a card made from the thread is a
                // new card with its own agent (§4, card #454): one still
                // holding the thread lets go first, and goes like any
                // agent whose thread closed.
                if a.thread && !taker.is_empty() && a.owner == self.dev {
                    self.write_thread_answered(id, &a)?;
                }
                // Nobody holds it (a thread, or an ask nobody took): it goes
                // out as a new card, so it gets its own agent instead of
                // sitting in working with no one on it.
                let held = !a.thread && !taker.is_empty() && !self.is_owner(&taker);
                self.write(
                    &by,
                    kinds::TASK_STATUS,
                    &TaskStateRow {
                        task_id: id.into(),
                        state: if held { WORKING } else { SUBMITTED }.into(),
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
                if held && by == taker {
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
                            parent: parent.clone(),
                            by: by.clone(),
                            at,
                        },
                    )?;
                    self.inherit_park(&by, &parent, id)?;
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
                self.check_person_title(&by, &m, &title)?;
                self.write(
                    &by,
                    self.ask_kind(kinds::TASK_EDITED),
                    &TaskEditedRow {
                        task_id: target.clone(),
                        title: opt(&title),
                        objective: opt(&obj),
                        done_criteria: opt(&done),
                        by: by.clone(),
                        from: id.into(),
                        note: note.clone(),
                        at: now,
                        ..TaskEditedRow::default()
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
            self.ask_kind(kinds::TAKE_REQUESTED),
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
            self.ask_kind(kinds::TAKE_REQUESTED),
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

/// Tells the one agent Irori handed ask t to that it holds it.
fn hand_notice(t: &Task, to: &str, why: &str) -> Notice {
    let mentioned = t.ask.as_ref().is_some_and(|a| !a.mentions.is_empty());
    let mut b = if mentioned {
        format!("The person asks you by name ({})", t.refs())
    } else {
        format!(
            "The person asks the room ({}), and Irori handed it to you",
            t.refs()
        )
    };
    if !why.is_empty() {
        b.push_str(&format!(" ({why})"));
    }
    b.push_str(&format!(":\n{}\n", cut(&t.objective, MAX_ASK_NOTICE)));
    b.push_str(&format!(
        "You hold it: no ask_claim needed. {YOU_HAVE_IT} If you can't take it on now, board_leave {} and it goes to {}; ending your turn without touching it does the same.",
        t.short_ref(),
        if mentioned {
            "the next agent it names, if any"
        } else {
            "the next idle agent"
        }
    ));
    b.push_str(&deeper(&t.id));
    Notice {
        kind: NOTICE_ASK.into(),
        task_id: t.id.clone(),
        reference: format!("task:{}", t.id),
        title: t.title.clone(),
        by: t.created_by.clone(),
        to: vec![to.into()],
        wake: true,
        text: b,
        person: true,
        ..Notice::default()
    }
}

/// Tells `to` about ask t, from the person.
fn ask_notice(t: &Task, to: &[String], wake: bool, why: &str) -> Notice {
    let mentioned = t.ask.as_ref().is_some_and(|a| !a.mentions.is_empty());
    let mut b = if mentioned {
        format!("The person asks you by name ({})", t.refs())
    } else {
        format!("The person asks the room ({})", t.refs())
    };
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

/// How long a quoted title, step or text runs in a thread line.
pub(crate) const MAX_LOG_QUOTE: usize = 80;

/// "set the objective to “…”", or "cleared the objective".
fn set_or_cleared(field: &str, to: &str) -> String {
    let to = trim_space(to);
    if to.is_empty() {
        format!("cleared {field}")
    } else {
        format!("set {field} to “{}”", cut(to, MAX_LOG_QUOTE))
    }
}

/// A thread's Talk channel (card #454): a `ch_` id, else nothing.
fn clean_channel(c: &str) -> String {
    let c = c.trim();
    if crate::talk::is_channel(c) && c.len() <= 64 && !c.contains(char::is_whitespace) {
        c.to_string()
    } else {
        String::new()
    }
}

/// Whether a task.edited row is Irori's assistant naming a thread (card
/// #782): written by Irori itself, carrying a title and nothing else.
fn irori_title_row(ev: &Event, p: &TaskEditedRow) -> bool {
    p.by == SWEEPER
        && ev.actor == SWEEPER
        && p.title.is_some()
        && p.objective.is_none()
        && p.done_criteria.is_none()
        && p.repo_key.is_none()
        && p.from.is_empty()
        && p.agents_now.is_none()
        && p.board.is_none()
}
