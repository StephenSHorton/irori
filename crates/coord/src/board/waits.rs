//! Waits (card #87; a subset of docs/design/waits-signals.md): an agent
//! asks to be woken once when a card reaches a state, or when a branch
//! moves. Task waits match here, in the fold (a state change while the
//! wait is open) and in the sweep (a state that already holds, or held
//! while this device was off). Branch waits are checked by the core,
//! which reads the refs and calls `fire_wait`: the coord never reads git.
//! A wait on a shared board's card (card #574) works the same way: it lives
//! in the waiter's own log, not the board's, so the board's other members
//! never see it and an older Irori there has nothing new to read, and the
//! core reads the card from the board's coord each sweep.
//! A wait fires from its waiter's own device only: the `wait.fired` row
//! first, then a wake, so a crash between the two can't wake twice.
//! The sweep cancels a wait whose waiter's session is gone, or that is a
//! day old.

use std::time::Duration;

use gocompat::quote::quote;
use gocompat::time::GoTime;
use store::{Event, kinds};

use super::error::{Result, invalid, not_allowed, not_found};
use super::fold::Fold;
use super::rows::*;
use super::types::*;
use super::util::{SWEEPER, caller, check_key, check_text, new_id};
use crate::gotime;

/// At most this many open waits per agent.
pub const MAX_WAITS: usize = 20;
/// A wait nobody fired lapses after this.
pub const WAIT_TTL: Duration = Duration::from_secs(24 * 3600);
const MAX_WAIT_NOTE: usize = 500;

/// `until` values for a task wait.
pub const UNTIL_CHANGED: &str = "changed";
pub const UNTIL_DONE: &str = "done";
pub const UNTIL_BLOCKED: &str = "blocked";
pub const UNTIL_UNBLOCKED: &str = "unblocked";
const UNTIL_STATES: &[&str] = &[
    UNTIL_CHANGED,
    UNTIL_DONE,
    UNTIL_BLOCKED,
    UNTIL_UNBLOCKED,
    COMPLETED,
    FAILED,
    CANCELED,
    WORKING,
    SUBMITTED,
];

/// Why Irori canceled a wait.
pub const WAIT_GONE: &str = "gone";
pub const WAIT_EXPIRED: &str = "expired";
pub const WAIT_CARD_GONE: &str = "card_gone";

fn is_blocked(s: &str) -> bool {
    s == BLOCKED || s == INPUT_REQUIRED
}

/// Whether one `until` value matches a move from `was` to `now`.
fn matches(until: &str, was: &str, now: &str) -> bool {
    match until {
        UNTIL_CHANGED => was != now,
        UNTIL_DONE => is_terminal(now),
        UNTIL_BLOCKED => is_blocked(now),
        UNTIL_UNBLOCKED => is_blocked(was) && !is_blocked(now),
        s => s == now,
    }
}

/// Whether one `until` value already holds in state `now` (only the
/// values that name a state; "changed" and "unblocked" need a move).
fn holds(until: &str, now: &str) -> bool {
    match until {
        UNTIL_CHANGED | UNTIL_UNBLOCKED => false,
        u => matches(u, "", now),
    }
}

/// The until list as one phrase: "done or blocked".
pub fn until_text(until: &[String]) -> String {
    if until.is_empty() {
        return UNTIL_CHANGED.into();
    }
    until.join(" or ")
}

/// A short line for a wait: "#12 until done" or "branch main of org/app".
pub fn wait_line(w: &Wait, card_ref: &str) -> String {
    if !w.task_id.is_empty() {
        let mut s = format!("{card_ref} until {}", until_text(&w.until));
        if !w.note.is_empty() {
            s.push_str(&format!(" ({})", w.note));
        }
        return s;
    }
    let mut s = format!("branch {} moves", w.branch);
    if !w.repo.is_empty() {
        s = format!("branch {} of {} moves", w.branch, w.repo);
    }
    if !w.note.is_empty() {
        s.push_str(&format!(" ({})", w.note));
    }
    s
}

impl Fold {
    // ---- writes ----

    /// Registers a wait for `by`; it fires once. A task wait whose state
    /// already holds fires at the next sweep.
    pub(crate) fn create_wait(&mut self, by: &str, spec: WaitSpec) -> Result<Wait> {
        let by = caller(by)?;
        if self.is_owner(&by) || by == SWEEPER {
            return Err(invalid("only agents wait; people get notifications"));
        }
        check_key(&by)?;
        check_text("note", &spec.note, 0, MAX_WAIT_NOTE)?;
        let task = !spec.task_id.is_empty();
        let branch = !spec.branch.is_empty();
        if task == branch {
            return Err(invalid("wait on a card (task_id) or a branch, not both"));
        }
        let mut until: Vec<String> = Vec::new();
        let mut from_state = String::new();
        if task {
            let (own, state) = match &spec.on_board {
                Some(c) => (c.own, c.state.clone()),
                None => {
                    let t = self.any_task(&spec.task_id)?;
                    (t.holders.len() == 1 && t.holders[0] == by, t.state.clone())
                }
            };
            if own {
                return Err(invalid(
                    "that is your own card: you'll know when it changes. Wait on someone else's card",
                ));
            }
            for u in &spec.until {
                let u = u.trim().to_lowercase();
                if u.is_empty() {
                    continue;
                }
                if !UNTIL_STATES.contains(&u.as_str()) {
                    return Err(invalid(format!(
                        "until {}: use changed, done, blocked, unblocked, completed, failed, canceled, working or submitted",
                        quote(&u)
                    )));
                }
                if !until.contains(&u) {
                    until.push(u);
                }
            }
            if until.is_empty() {
                until.push(UNTIL_CHANGED.into());
            }
            from_state = state;
        } else if spec.refs.is_empty() {
            return Err(invalid(
                "a branch wait needs the branch's refs as they are now",
            ));
        }
        let mine = self.waits.values().filter(|w| w.by == by).count();
        if mine >= MAX_WAITS {
            return Err(not_allowed(format!(
                "you have {MAX_WAITS} open waits; cancel one with wait_cancel first"
            )));
        }
        let at = self.stamp();
        let id = new_id(&at);
        let on_board = spec.on_board.unwrap_or_default();
        self.write(
            &by,
            kinds::WAIT_CREATED,
            &WaitCreatedRow {
                id: id.clone(),
                by: by.clone(),
                task_id: spec.task_id,
                until,
                from_state,
                repo: spec.repo,
                root: spec.root,
                branch: spec.branch,
                refs: spec.refs,
                note: spec.note,
                at,
                expires: gotime::add(&at, WAIT_TTL),
                board: on_board.board,
                card: on_board.card,
                title: on_board.title,
            },
        )?;
        self.waits
            .get(&id)
            .cloned()
            .ok_or_else(|| not_found(format!("wait {id}")))
    }

    /// Cancels one of `by`'s open waits.
    pub(crate) fn cancel_wait(&mut self, by: &str, id: &str) -> Result<Wait> {
        let by = caller(by)?;
        let Some(w) = self.waits.get(id).cloned() else {
            return Err(not_found(format!("no open wait {id}")));
        };
        if w.by != by && !self.is_owner(&by) {
            return Err(not_allowed("that wait is someone else's"));
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::WAIT_CANCELED,
            &WaitCanceledRow {
                id: id.into(),
                reason: String::new(),
                by: by.clone(),
                at,
            },
        )?;
        Ok(w)
    }

    /// Queues a wait to fire with `what` (the core, for a branch that
    /// moved). Unknown or already-fired waits are ignored.
    pub(crate) fn fire_wait(&mut self, id: &str, what: &str) {
        if self.waits.contains_key(id) && !self.waitq.iter().any(|(q, _)| q == id) {
            self.waitq.push((id.into(), what.into()));
        }
    }

    /// Whether board wait `id` should fire now the card is in state `cur`
    /// (card #574), as the sweep decides for a card in this log: `Some`
    /// with the state it moved from. The core reads the state and calls
    /// `fire_wait`, since this log has no such card.
    pub(crate) fn board_wait_moved(&mut self, id: &str, cur: &str) -> Option<String> {
        let w = self.waits.get(id)?;
        let seen = self
            .wait_seen
            .get(id)
            .cloned()
            .unwrap_or_else(|| w.from_state.clone());
        let moved = seen != cur && w.until.iter().any(|u| matches(u, &seen, cur));
        let fire = moved || w.until.iter().any(|u| holds(u, cur));
        self.wait_seen.insert(id.into(), cur.into());
        fire.then_some(seen)
    }

    /// Board wait `id`'s card is gone from its board: the sweep ends the
    /// wait and says so.
    pub(crate) fn board_wait_lost(&mut self, id: &str) {
        if self.waits.contains_key(id) {
            self.waits_lost.insert(id.into());
        }
    }

    // ---- reads ----

    /// Open waits, oldest first; `by` empty for every waiter.
    pub(crate) fn waits_of(&self, by: &str) -> Vec<Wait> {
        let mut out: Vec<Wait> = self
            .waits
            .values()
            .filter(|w| by.is_empty() || w.by == by)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Open branch waits whose waiter lives on this device: the core
    /// checks their refs.
    pub(crate) fn branch_waits(&self) -> Vec<Wait> {
        self.waits_of("")
            .into_iter()
            .filter(|w| !w.branch.is_empty() && self.local_key(&w.by))
            .collect()
    }

    /// Open waits on shared boards' cards whose waiter lives on this
    /// device: the core reads those cards.
    pub(crate) fn board_waits(&self) -> Vec<Wait> {
        self.waits_of("")
            .into_iter()
            .filter(|w| !w.board.is_empty() && self.local_key(&w.by))
            .collect()
    }

    /// "#12 “Title” is now completed (was working): note", for a wait in
    /// another log (card #574).
    pub(crate) fn wait_what(&self, id: &str, was: &str, now: &str) -> String {
        self.state_what(id, was, now)
    }

    // ---- fold ----

    pub(crate) fn fold_wait(&mut self, ev: &Event) {
        match ev.kind.as_str() {
            kinds::WAIT_CREATED => {
                let Some(p) = row::<WaitCreatedRow>(&ev.payload) else {
                    return;
                };
                if p.id.is_empty() || p.by.is_empty() || self.waits_done.contains(&p.id) {
                    return;
                }
                let w = Wait {
                    id: p.id.clone(),
                    by: p.by,
                    task_id: p.task_id,
                    until: p.until,
                    from_state: p.from_state,
                    repo: p.repo,
                    root: p.root,
                    branch: p.branch,
                    refs: p.refs,
                    note: p.note,
                    at: p.at,
                    expires: p.expires,
                    board: p.board,
                    card: p.card,
                    title: p.title,
                };
                if !self.replaying
                    && let Some(t) = self.tasks.get(&w.task_id)
                    && w.until.iter().any(|u| holds(u, &t.state))
                {
                    let what = self.state_what(&w.task_id, "", &t.state.clone());
                    self.waitq.push((w.id.clone(), what));
                }
                if !w.task_id.is_empty() {
                    self.wait_seen.insert(w.id.clone(), w.from_state.clone());
                }
                self.waits.insert(p.id, w);
            }
            kinds::WAIT_FIRED => {
                if let Some(p) = row::<WaitFiredRow>(&ev.payload) {
                    self.waits.remove(&p.id);
                    self.wait_seen.remove(&p.id);
                    self.waits_lost.remove(&p.id);
                    self.waits_done.insert(p.id);
                }
            }
            kinds::WAIT_CANCELED => {
                if let Some(p) = row::<WaitCanceledRow>(&ev.payload) {
                    self.waits.remove(&p.id);
                    self.wait_seen.remove(&p.id);
                    self.waits_lost.remove(&p.id);
                    self.waits_done.insert(p.id);
                }
            }
            _ => {}
        }
    }

    /// After a task's state moved from `was` to `now`: queue the waits it
    /// matches (live rows only; replay leaves it to the sweep).
    pub(crate) fn waits_on_status(&mut self, id: &str, was: &str, now: &str) {
        if self.replaying || self.waits.is_empty() {
            return;
        }
        let on: Vec<Wait> = self
            .waits
            .values()
            .filter(|w| w.task_id == id)
            .cloned()
            .collect();
        for w in &on {
            self.wait_seen.insert(w.id.clone(), now.to_string());
        }
        let hit: Vec<String> = on
            .iter()
            .filter(|w| w.until.iter().any(|u| matches(u, was, now)))
            .map(|w| w.id.clone())
            .collect();
        if hit.is_empty() {
            return;
        }
        let what = self.state_what(id, was, now);
        for w in hit {
            if !self.waitq.iter().any(|(q, _)| *q == w) {
                self.waitq.push((w, what.clone()));
            }
        }
    }

    /// "#12 “Title” is now completed (was working): note".
    fn state_what(&self, id: &str, was: &str, now: &str) -> String {
        let Some(t) = self.tasks.get(id) else {
            return format!("task:{id} is now {now}");
        };
        let mut s = format!("{} {} is now {now}", t.refs(), quote(&t.title));
        if !was.is_empty() && was != now {
            s.push_str(&format!(" (was {was})"));
        }
        if !t.note.is_empty() {
            s.push_str(&format!(": {}", t.note));
        }
        s
    }

    // ---- sweep ----

    /// Lapses waits whose waiter is gone or that expired, catches task
    /// waits whose state holds now, then fires what is queued: the
    /// wait.fired row, then a wake to the waiter. Only waits whose waiter
    /// lives on this device.
    pub(crate) fn wait_notices(&mut self, now: &GoTime, out: &mut Vec<Notice>) -> Result<()> {
        let open: Vec<Wait> = self
            .waits
            .values()
            .filter(|w| self.local_key(&w.by))
            .cloned()
            .collect();
        for w in &open {
            let reason = if self.gone(&w.by, now) {
                WAIT_GONE
            } else if !w.expires.is_zero() && gotime::before(&w.expires, now) {
                WAIT_EXPIRED
            } else if self.waits_lost.contains(&w.id)
                || (w.board.is_empty()
                    && !w.task_id.is_empty()
                    && !self.tasks.contains_key(&w.task_id))
            {
                WAIT_CARD_GONE
            } else {
                ""
            };
            if !reason.is_empty() {
                self.write(
                    SWEEPER,
                    kinds::WAIT_CANCELED,
                    &WaitCanceledRow {
                        id: w.id.clone(),
                        reason: reason.into(),
                        by: SWEEPER.into(),
                        at: *now,
                    },
                )?;
                if reason != WAIT_GONE {
                    // Nothing happened: say so at its next turn, no wake.
                    let mut n = self.wait_notice(
                        w,
                        &format!(
                            "Your wait on {} ended without firing: {}.",
                            self.wait_target(w),
                            if reason == WAIT_EXPIRED {
                                "it was a day old"
                            } else if w.board.is_empty() {
                                "the card was deleted"
                            } else {
                                "the card was withdrawn, or you left its board"
                            }
                        ),
                    );
                    n.wake = false;
                    out.push(n);
                }
                continue;
            }
            // A board card's state is the core's to read (card #574).
            if !w.board.is_empty() {
                continue;
            }
            let Some(cur) = self.tasks.get(&w.task_id).map(|t| t.state.clone()) else {
                continue;
            };
            let seen = self
                .wait_seen
                .get(&w.id)
                .cloned()
                .unwrap_or_else(|| w.from_state.clone());
            let moved = seen != cur && w.until.iter().any(|u| matches(u, &seen, &cur));
            self.wait_seen.insert(w.id.clone(), cur.clone());
            if (moved || w.until.iter().any(|u| holds(u, &cur)))
                && !self.waitq.iter().any(|(q, _)| *q == w.id)
            {
                let what = self.state_what(&w.task_id, &seen, &cur);
                self.waitq.push((w.id.clone(), what));
            }
        }
        let q = std::mem::take(&mut self.waitq);
        let mut later = Vec::new();
        for (id, what) in q {
            let Some(w) = self.waits.get(&id).cloned() else {
                continue;
            };
            if !self.local_key(&w.by) {
                later.push((id, what));
                continue;
            }
            self.write(
                SWEEPER,
                kinds::WAIT_FIRED,
                &WaitFiredRow {
                    id: id.clone(),
                    what: what.clone(),
                    by: SWEEPER.into(),
                    at: *now,
                },
            )?;
            let text = format!(
                "Your wait went off: {what}.\nYou waited on {}.{}",
                self.wait_target(&w),
                if w.task_id.is_empty() {
                    String::new()
                } else {
                    format!(" show task:{} for the card.", w.task_id)
                }
            );
            out.push(self.wait_notice(&w, &text));
        }
        self.waitq = later;
        Ok(())
    }

    fn wait_target(&self, w: &Wait) -> String {
        let r = self
            .tasks
            .get(&w.task_id)
            .map(|t| t.refs())
            .or_else(|| (!w.card.is_empty()).then(|| w.card.clone()))
            .unwrap_or_else(|| format!("task:{}", w.task_id));
        wait_line(w, &r)
    }

    fn wait_notice(&self, w: &Wait, text: &str) -> Notice {
        let title = self
            .tasks
            .get(&w.task_id)
            .map(|t| t.title.clone())
            .or_else(|| (!w.title.is_empty()).then(|| w.title.clone()))
            .unwrap_or_else(|| format!("branch {}", w.branch));
        Notice {
            kind: NOTICE_WAIT.into(),
            task_id: w.task_id.clone(),
            reference: if w.task_id.is_empty() {
                String::new()
            } else {
                format!("task:{}", w.task_id)
            },
            title,
            to: vec![w.by.clone()],
            by: SWEEPER.into(),
            text: text.into(),
            wake: true,
            ..Notice::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn until_matches() {
        assert!(matches(UNTIL_CHANGED, WORKING, BLOCKED));
        assert!(!matches(UNTIL_CHANGED, WORKING, WORKING));
        assert!(matches(UNTIL_DONE, WORKING, COMPLETED));
        assert!(matches(UNTIL_DONE, WORKING, CANCELED));
        assert!(!matches(UNTIL_DONE, WORKING, BLOCKED));
        assert!(matches(UNTIL_BLOCKED, WORKING, INPUT_REQUIRED));
        assert!(matches(UNTIL_UNBLOCKED, BLOCKED, WORKING));
        assert!(!matches(UNTIL_UNBLOCKED, WORKING, COMPLETED));
        assert!(matches(FAILED, WORKING, FAILED));
        assert!(holds(UNTIL_DONE, COMPLETED));
        assert!(!holds(UNTIL_CHANGED, COMPLETED));
        assert!(!holds(UNTIL_UNBLOCKED, WORKING));
    }
}
