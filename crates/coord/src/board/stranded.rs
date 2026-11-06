//! Blockers left with nobody (card #485): on a card nobody holds, an item
//! whose agent is gone — the one it waits on (on = agent), or the one that
//! raised it and would hear the person's answer — can't move by itself.
//! The window marks it ("Waiting on kestrel (gone)") with a "Hand to an
//! agent" action, and the person's answer to it goes to a new agent
//! instead of to nobody.

use std::collections::HashMap;

use gocompat::quote::quote;
use store::kinds;

use super::error::{Result, bad_state, not_allowed, not_found};
use super::fold::{Fold, LEFT_GONE};
use super::rows::{TaskCommentRow, TaskMemberRow};
use super::types::*;
use super::util::SWEEPER;
use crate::names::trim_space;

/// An open blocker whose agent is gone, on a card nobody holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stranded {
    pub blocker_id: String,
    /// The gone agent: the one waited on, else the raiser.
    pub agent: String,
    /// The item waits on that agent (on = agent), rather than having
    /// been raised by it.
    pub waited_on: bool,
}

/// The roster's status by key, read once per pass; `None` without a
/// roster.
pub(crate) type RosterStatus = Option<HashMap<String, String>>;

impl Fold {
    pub(crate) fn roster_status(&self) -> RosterStatus {
        self.roster
            .as_ref()
            .map(|r| r.agents().into_iter().map(|a| (a.key, a.status)).collect())
    }

    /// Whether agent `key` is gone: its session ended here (agent.gone),
    /// Irori ended it, or the roster says gone or no longer lists it. Not
    /// the person, Irori, a run holder, or anything that isn't an agent
    /// key. On a shared board another member's agents aren't on this
    /// roster, so only a gone status or this device's own rows count there.
    pub(crate) fn key_gone_in(&self, key: &str, rs: &RosterStatus) -> bool {
        if key.is_empty()
            || !key.contains('/')
            || key == SWEEPER
            || self.is_owner(key)
            || key.starts_with(kinds::PERSON_ACTOR_PREFIX)
            || super::is_run_holder(key)
        {
            return false;
        }
        if self.gone_rows.contains_key(key) || self.ended_by_irori.contains(key) {
            return true;
        }
        match rs {
            Some(m) => match m.get(key) {
                Some(s) => s == "gone",
                None => !self.on_board || self.local_key(key),
            },
            None => false,
        }
    }

    pub(crate) fn key_gone(&self, key: &str) -> bool {
        self.key_gone_in(key, &self.roster_status())
    }

    /// The open items of `t` left with nobody: the card has no holder (nor
    /// a live agent it was handed to), and the item's agent is gone. A
    /// wait that ends by itself (a card, a lease, a version) is left out
    /// for its raiser: it closes and the card goes out again anyway.
    pub(crate) fn stranded_in(&self, t: &Task, rs: &RosterStatus) -> Vec<Stranded> {
        if is_terminal(&t.state)
            || !t.holders.is_empty()
            || !t.co_holders.is_empty()
            || t.assigned.iter().any(|k| !self.key_gone_in(k, rs))
        {
            return Vec::new();
        }
        let mut out = Vec::new();
        for b in t.blockers.iter().filter(|b| b.open()) {
            let kind = b.on.as_ref().map(|on| on.kind.as_str()).unwrap_or("");
            if kind == BLOCKED_ON_AGENT {
                let k = &b.on.as_ref().unwrap().reference;
                if self.key_gone_in(k, rs) {
                    out.push(Stranded {
                        blocker_id: b.id.clone(),
                        agent: k.clone(),
                        waited_on: true,
                    });
                }
                continue;
            }
            if matches!(
                kind,
                BLOCKED_ON_TASK | BLOCKED_ON_LEASE | BLOCKED_ON_VERSION
            ) {
                continue;
            }
            // The agent that would hear the person's answer: the one it
            // was sent to, else the raiser.
            let to = if b.sent_to.is_empty() {
                &b.raised_by
            } else {
                &b.sent_to
            };
            if self.key_gone_in(to, rs) {
                out.push(Stranded {
                    blocker_id: b.id.clone(),
                    agent: to.clone(),
                    waited_on: false,
                });
            }
        }
        out
    }

    pub(crate) fn stranded(&self, t: &Task) -> Vec<Stranded> {
        self.stranded_in(t, &self.roster_status())
    }

    /// The stranded waits of `t` that only Needs you can show (card #520):
    /// a wait on a gone agent, or an outside wait whose gone agent raised
    /// it, on a card nobody holds. A question or done check already counts
    /// as a decision, and a wait on a PR's merge as #509's merge row, so
    /// those are left out and nothing shows twice. On a shared board only
    /// the person's own agents' waits.
    pub(crate) fn gone_waits_in(&self, t: &Task, rs: &RosterStatus) -> Vec<Stranded> {
        let st = self.stranded_in(t, rs);
        if st.is_empty() {
            return st;
        }
        let merges = self.merge_waits(t);
        st.into_iter()
            .filter(|s| {
                let Some(b) = t.blockers.iter().find(|b| b.id == s.blocker_id) else {
                    return false;
                };
                if self.on_board && !self.own_on_board(b) {
                    return false;
                }
                match b.on.as_ref().map(|on| on.kind.as_str()) {
                    Some(BLOCKED_ON_AGENT) => true,
                    Some(BLOCKED_ON_EXTERNAL) => {
                        !b.merge_wait().is_some_and(|pr| merges.contains(&pr))
                    }
                    _ => false,
                }
            })
            .collect()
    }

    pub(crate) fn gone_waits(&self, t: &Task) -> Vec<Stranded> {
        // A held or finished card has none: skip the roster for it.
        if is_terminal(&t.state) || !t.holders.is_empty() || !t.co_holders.is_empty() {
            return Vec::new();
        }
        self.gone_waits_in(t, &self.roster_status())
    }

    /// Takes the gone agents a card was handed to off it (task.left
    /// "gone"), so it can go out again.
    pub(crate) fn drop_gone_assigned(&mut self, by: &str, id: &str) -> Result<()> {
        let Some(t) = self.tasks.get(id) else {
            return Ok(());
        };
        let rs = self.roster_status();
        let gone: Vec<String> = t
            .assigned
            .iter()
            .filter(|k| !t.holders.contains(k) && self.key_gone_in(k, &rs))
            .cloned()
            .collect();
        for who in gone {
            let at = self.stamp();
            self.write(
                by,
                kinds::TASK_LEFT,
                &TaskMemberRow {
                    task_id: id.into(),
                    who,
                    by: by.into(),
                    at,
                    reason: LEFT_GONE.into(),
                    person: true,
                },
            )?;
        }
        Ok(())
    }

    /// Why "Hand to an agent" can't move card `t` (what back_on_board
    /// would refuse), said before anything is written; "" when it can.
    /// Home's Needs you row (card #520) says it instead of offering it.
    pub(crate) fn hand_on_refusal(&self, t: &Task) -> &'static str {
        if t.shared || self.on_board {
            "a card in a shared space isn't handed out; take it with an agent of yours (board_join)"
        } else if t.open_ask().is_some() {
            "it has an open ask"
        } else if !t.subtasks.is_empty() {
            "it has subtasks; hand those on instead"
        } else if t.state == SUBMITTED || t.state == WORKING {
            "it is already on the Board"
        } else {
            ""
        }
    }

    /// "Hand to an agent" (card #485): the person moves a card whose items
    /// wait on gone agents back on the Board, so Irori starts an agent for
    /// it (or hands it to a free one). Its open items close as it goes, as
    /// any card put back does; the new agent's brief says what they were.
    /// Returns the card before the move and its stranded items.
    pub(crate) fn hand_on(&mut self, by: &str, id: &str) -> Result<(Task, Vec<Stranded>)> {
        let by = trim_space(by).to_string();
        if !self.is_owner(&by) {
            return Err(not_allowed("only the person hands a card to an agent"));
        }
        let Some(t) = self.tasks.get(id).cloned() else {
            return Err(not_found(format!("task {}", quote(id))));
        };
        if !t.holders.is_empty() || !t.co_holders.is_empty() {
            return Err(bad_state(format!("{} has an agent on it", t.short_ref())));
        }
        let st = self.stranded(&t);
        if st.is_empty() {
            return Err(bad_state(format!(
                "nothing on {} waits on an agent that is gone",
                t.short_ref()
            )));
        }
        let why = self.hand_on_refusal(&t);
        if !why.is_empty() {
            return Err(bad_state(format!("{}: {why}", t.short_ref())));
        }
        self.drop_gone_assigned(&by, id)?;
        let mut names: Vec<String> = Vec::new();
        for s in &st {
            let n = self.named(&s.agent);
            if !names.contains(&n) {
                names.push(n);
            }
        }
        // The items close as the card goes back: say on the card what they
        // were, so the agent that takes it reads them in its brief.
        let mut text = format!(
            "The person handed this card to a new agent: {} {} gone. What was open:",
            names.join(", "),
            if names.len() > 1 { "are" } else { "is" }
        );
        for s in &st {
            let Some(b) = t.blockers.iter().find(|b| b.id == s.blocker_id) else {
                continue;
            };
            let who = self.named(&s.agent);
            let how = if s.waited_on {
                format!("waited on {who}")
            } else {
                format!("raised by {who}")
            };
            text.push_str(&format!("\n- {} {} ({how})", b.id, quote(&b.question)));
            if !b.context.is_empty() {
                text.push_str(&format!(": {}", super::util::cut(&b.context, 600)));
            }
        }
        text.push_str("\nPick up from there: what is already done is on the card and its thread.");
        let at = self.stamp();
        self.write_comment(
            SWEEPER,
            TaskCommentRow {
                task_id: id.into(),
                comment_id: super::util::new_id(&at),
                by: SWEEPER.into(),
                text,
                at,
                ..TaskCommentRow::default()
            },
        )?;
        let moved = self.back_on_board(
            &by,
            id,
            &format!(
                "{} gone; the person handed it to an agent",
                names.join(", ")
            ),
        )?;
        if !moved {
            return Err(bad_state(format!(
                "{}: it can't go back on the Board now",
                t.short_ref()
            )));
        }
        Ok((t, st))
    }
}
