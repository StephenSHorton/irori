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

use gocompat::time::GoTime;

use super::MAX_OBJECTIVE;
use super::error::Result;
use super::fold::Fold;
use super::types::*;
use super::util::{SWEEPER, cut};

/// A card's agents at once, lead and helpers, when neither the card nor
/// the board says (decision 12).
pub const DEFAULT_AGENTS_PER_CARD: i64 = 4;
/// The most one card's own "Up to N" may ask for: the Mac's device cap
/// (decision 11). The board's Agents per card isn't held to it.
pub const MAX_AGENTS_PER_CARD: i64 = 12;

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
            && self.spawning_on()
            && self.tasks.get(&t.parent).is_some_and(|p| {
                !is_terminal(&p.state)
                    && p.holders.is_empty()
                    && !p.shared
                    && p.spawn_crashes < self.crash_limit()
            })
    }

    /// Whether led card t lost its lead and has work for the next: nobody
    /// is on it or starting, and a helper's part says it is done. Not before
    /// that: a fresh lead with nothing to merge would only idle out again.
    /// Nor while t waits on anything but its own parts: on the person, Irori
    /// would end the new lead at once and start another, so it comes with
    /// their answer. A wait on a part only closes through a lead.
    pub(crate) fn needs_lead(&self, t: &Task) -> bool {
        let on_a_part = |b: &Blocker| {
            b.on.as_ref()
                .is_some_and(|on| on.kind == BLOCKED_ON_TASK && t.subtasks.contains(&on.reference))
        };
        t.assigned.is_empty()
            && !t.withdrawn
            && t.open_ask().is_none()
            && t.blockers
                .iter()
                .filter(|b| b.open())
                .all(|b| b.status == BLOCKER_ANSWERED || on_a_part(b))
            && t.subtasks
                .iter()
                .filter_map(|id| self.tasks.get(id))
                .any(|k| {
                    !is_terminal(&k.state)
                        && self.lead_coming(k)
                        && k.blockers
                            .iter()
                            .any(|b| b.open() && b.question == PART_QUESTION)
                })
    }

    /// How many agents card t may have at once: its own Up to N, else the
    /// board's Agents per card, else [`DEFAULT_AGENTS_PER_CARD`].
    pub(crate) fn agent_limit(&self, t: &Task) -> usize {
        let board = self
            .agents_per_card
            .as_ref()
            .map(|f| f())
            .filter(|n| *n >= 1)
            .unwrap_or(DEFAULT_AGENTS_PER_CARD);
        // The board's number is the person's, from Settings › Agents, and
        // has its own bound there (card #576).
        let n = if t.agents >= 1 {
            t.agents.min(MAX_AGENTS_PER_CARD)
        } else {
            board
        };
        n.max(1) as usize
    }

    /// The agents on card t now, and those starting: its holders and step
    /// owners, and the holders of its open marked subtasks, or a pending
    /// request for one with nobody on it yet.
    pub(crate) fn agents_on(&self, t: &Task) -> usize {
        let now = self.stamp();
        let mut keys: Vec<&str> = t
            .holders
            .iter()
            .chain(&t.co_holders)
            .map(String::as_str)
            .collect();
        let mut starting = 0;
        for k in t.subtasks.iter().filter_map(|id| self.tasks.get(id)) {
            if !k.spawn || is_terminal(&k.state) {
                continue;
            }
            if k.holders.is_empty() {
                if k.spawn_request()
                    .is_some_and(|s| s.pending() && !s.stale(&now))
                {
                    starting += 1;
                }
                continue;
            }
            keys.extend(k.holders.iter().map(String::as_str));
        }
        keys.sort_unstable();
        keys.dedup();
        keys.len() + starting
    }

    /// Of `cards` (oldest first), the marked subtasks that would take a
    /// parent past its agent limit are left out: they queue on the card
    /// until a helper there finishes (§3, "Limits").
    pub(crate) fn within_limits(&self, cards: Vec<Task>) -> Vec<Task> {
        let mut used: std::collections::HashMap<String, usize> = Default::default();
        cards
            .into_iter()
            .filter(|k| {
                if !k.spawn {
                    return true;
                }
                let Some(p) = self.tasks.get(&k.parent).filter(|_| !k.parent.is_empty()) else {
                    return true;
                };
                let n = used
                    .entry(p.id.clone())
                    .or_insert_with(|| self.agents_on(p));
                if *n >= self.agent_limit(p) {
                    return false;
                }
                *n += 1;
                true
            })
            .collect()
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
