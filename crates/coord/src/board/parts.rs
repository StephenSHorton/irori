//! Several agents on one card (docs/design/spawned-agents.md §3, part 7):
//! the `spawn` mark on a card's steps and on the subtasks a split makes,
//! who may set it, and who closes a helper's part.
//!
//! A mark asks Irori for a helper on that part. Only the card's owner's
//! person, or one of their agents, may set it: on a shared board anyone
//! may edit a card's steps (card #320), and a mark from someone else would
//! start an agent on the owner's machine. Every member's fold applies the
//! same rule, by the writing device, so a row written straight to the log
//! gets no further than a call does.
//!
//! A subtask split off marked is a helper's part. Its agent's completed
//! goes to the parent's holder (the lead), not to the person: a card split
//! four ways asks the person once, on the parent.
//!
//! A lead can go while its helpers work: Claude Code's background sessions
//! drop off about an hour after going idle, and a lead waiting on queued
//! helpers is idle. Then a part that finishes waits for the next lead, and
//! Irori starts one for the card ([`Fold::needs_lead`]), so the helpers'
//! branches still get merged and the person is still asked once (#451).
//! A lead can also go after every part is in, with its "Can I merge?"
//! unanswered: the person's answer then stays open and brings the next
//! lead, which acts on it (#557).

use gocompat::time::GoTime;

use super::MAX_OBJECTIVE;
use super::error::Result;
use super::fold::Fold;
use super::types::*;
use super::util::{SWEEPER, cut};

/// The thread line's tail when a row's marks were set aside.
pub(crate) const SPAWN_REFUSED: &str =
    "its spawn marks were set aside: only the card's owner or their agents ask for helpers";

/// A helper's question to its lead when it says its part is done.
pub const PART_QUESTION: &str = "Part done: merge it and accept?";

/// Who a part's wait on Irori is shown waiting on once no lead is coming.
pub(crate) const NO_LEAD: &str = "its lead";

/// The context when the helper gave no proof.
const PART_CONTEXT: &str = "The helper says its part is done. Merge its branch into the card's branch, then accept with board_status completed on this subtask, or say what is missing on its thread.";

impl Fold {
    /// Whether a row written by `device` may set spawn marks on card t.
    /// Off a board every row is the person's own.
    pub(crate) fn may_spawn_on(&self, t: &Task, device: &str) -> bool {
        if !self.on_board {
            return true;
        }
        let p = self.writer_person(device);
        !p.is_empty() && p == self.owner_person(t)
    }

    /// Whether caller `by` may set spawn marks on card t now.
    pub(crate) fn may_spawn_by(&self, t: &Task, by: &str) -> bool {
        if !self.on_board {
            return true;
        }
        let p = self.person_of(by);
        !p.is_empty() && p == self.owner_person(t)
    }

    /// The lead of helper part t: its open parent's holders. Empty when t
    /// isn't a marked part or nobody leads it, and then t's completed
    /// works as any card's.
    pub(crate) fn lead_of(&self, t: &Task) -> Vec<String> {
        if !t.spawn || t.parent.is_empty() {
            return Vec::new();
        }
        self.tasks
            .get(&t.parent)
            .filter(|p| !is_terminal(&p.state))
            .map(|p| p.holders.clone())
            .unwrap_or_default()
    }

    /// Whether helper part t has no lead now but Irori would start one:
    /// its open parent has no holder on a board that spawns. Off a spawning
    /// board nobody would come, so t's completed works as any card's.
    pub(crate) fn lead_coming(&self, t: &Task) -> bool {
        t.spawn
            && !t.parent.is_empty()
            && self.tasks.get(&t.parent).is_some_and(|p| self.lead_due(p))
    }

    /// Who helper part t's "Part done" waits on now (card #557): Some with
    /// its parent's holders while a lead is on it, Some(empty) while Irori
    /// would start one, None when t isn't a part or nobody is coming (the
    /// wait is then stranded and says so).
    pub(crate) fn part_lead(&self, t: &Task) -> Option<Vec<String>> {
        let on = self.lead_of(t);
        if !on.is_empty() {
            return Some(on);
        }
        self.lead_coming(t).then(Vec::new)
    }

    /// Whether the person's answer on led card t brings it a fresh lead
    /// (card #557): t has parts and nobody on it or handed it, and Irori
    /// would start a lead. The window then leaves their answer open, for
    /// that lead to read and act on, instead of closing it with nobody to
    /// act; the lead starts once every open item is answered.
    pub(crate) fn answer_brings_lead(&self, t: &Task) -> bool {
        !t.subtasks.is_empty()
            && self.lead_due(t)
            && t.co_holders.is_empty()
            && t.assigned.is_empty()
            && !t.withdrawn
            && t.open_ask().is_none()
    }

    /// Whether Irori would start a lead for parent p once there is work for
    /// one: p is open, nobody holds it, it is private, the board spawns and
    /// its agents haven't crashed past the limit.
    fn lead_due(&self, p: &Task) -> bool {
        self.spawning_on()
            && !is_terminal(&p.state)
            && p.holders.is_empty()
            && !p.shared
            && p.spawn_crashes < self.crash_limit()
    }

    /// Whether led card t lost its lead and has work for the next: nobody
    /// is on it or starting, and a helper's part says it is done, or the
    /// person answered on t itself (card #557: their "Merge it" on the
    /// lead's "Can I merge?" once every part was accepted, after the lead
    /// went). Not before that: a fresh lead with nothing to do would only
    /// idle out again. Nor while t waits on anything but its own parts: on
    /// the person, Irori would end the new lead at once and start another,
    /// so it comes with their answer. A wait on a part only closes through
    /// a lead.
    pub(crate) fn needs_lead(&self, t: &Task) -> bool {
        let on_a_part = |b: &Blocker| {
            b.on.as_ref()
                .is_some_and(|on| on.kind == BLOCKED_ON_TASK && t.subtasks.contains(&on.reference))
        };
        let part_done = || {
            t.subtasks
                .iter()
                .filter_map(|id| self.tasks.get(id))
                .any(|k| {
                    !is_terminal(&k.state)
                        && self.lead_coming(k)
                        && k.blockers
                            .iter()
                            .any(|b| b.open() && b.question == PART_QUESTION)
                })
        };
        // The person's answer on the parent waits on an agent, and with no
        // lead on it nobody else would act on it: back_on_board leaves a
        // parent's state to its subtasks, and needs_agent refuses one.
        let answered = || {
            self.answer_brings_lead(t)
                && t.blockers
                    .iter()
                    .any(|b| b.open() && b.status == BLOCKER_ANSWERED)
        };
        !t.subtasks.is_empty()
            && t.assigned.is_empty()
            && !t.withdrawn
            && t.open_ask().is_none()
            && t.parked().is_none()
            && t.blockers
                .iter()
                .filter(|b| b.open())
                .all(|b| b.status == BLOCKER_ANSWERED || on_a_part(b))
            && (part_done() || answered())
    }

    /// An unmarked subtask of a led card (§3): it isn't spawned on its own,
    /// since the lead asks for the helpers it wants (a mark) or assigns it.
    pub(crate) fn waits_for_lead(&self, t: &Task) -> bool {
        !t.spawn
            && !t.parent.is_empty()
            && self
                .tasks
                .get(&t.parent)
                .is_some_and(|p| !is_terminal(&p.state) && !p.holders.is_empty())
    }

    /// A helper's completed on its part: a question for the lead on the
    /// subtask, with the helper's proof, in place of the person's done
    /// check. An earlier one still open is closed first. An empty `lead`
    /// is the lead Irori will start: it waits on Irori until then.
    pub(crate) fn ask_lead(
        &mut self,
        by: &str,
        t: &Task,
        lead: &str,
        note: &str,
        files: Vec<FileRef>,
    ) -> Result<Task> {
        let earlier: Vec<String> = t
            .blockers
            .iter()
            .filter(|b| b.open() && b.question == PART_QUESTION)
            .map(|b| b.id.clone())
            .collect();
        for bid in earlier {
            self.answer(
                by,
                &t.id,
                &bid,
                BlockerAnswer {
                    text: "asked again: ready for the lead".into(),
                    resolve: true,
                    ..BlockerAnswer::default()
                },
            )?;
        }
        let t = self.open_task(&t.id)?.clone();
        let cx = if note.is_empty() {
            PART_CONTEXT.to_string()
        } else {
            cut(note, MAX_OBJECTIVE)
        };
        let now: GoTime = self.stamp();
        let bb = Some(BlockedBy {
            kind: BLOCKED_ON_AGENT.into(),
            reference: if lead.is_empty() { SWEEPER } else { lead }.into(),
        });
        let (nt, _) = self.raise_locked(
            by,
            &t,
            PART_QUESTION,
            PART_QUESTION,
            &cx,
            Vec::new(),
            bb,
            false,
            now,
            Vec::new(),
            files,
            false,
            false,
        )?;
        Ok(nt)
    }
}

/// Keeps `was`'s marks on `steps`, by step id, for a writer who may not
/// set them: it neither adds a mark nor drops one. True when it changed
/// any.
pub(crate) fn keep_marks(was: &[Step], steps: &mut [Step]) -> bool {
    let mut changed = false;
    for s in steps.iter_mut() {
        let old = was.iter().any(|o| o.id == s.id && o.spawn);
        if s.spawn != old {
            s.spawn = old;
            changed = true;
        }
    }
    changed
}
