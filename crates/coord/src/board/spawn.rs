//! Spawned agents for cards (docs/design/spawned-agents.md §2, §6, §7;
//! card #446): the `task.spawn` request rows, keyed by card (and part)
//! with a `generation`, and the `task.spawned` mark that binds an agent to the
//! card it was started for.
//!
//! A pending request keeps its card out of hand-out ([`super::offers`]'s
//! `offerable`), so repeated kicks never start two agents and no free
//! agent is handed the card while its own starts. A marked agent is never
//! offered another card: every place that picks an idle agent asks
//! [`Fold::spawned_for`]. The mark outlives the agent (card #452 reads it
//! after `agent.gone`).

use std::time::Duration;

use gocompat::time::GoTime;
use irori_identity::owner;
use store::{Event, kinds};

use super::error::{Result, bad_state};
use super::fold::Fold;
use super::later::HELD_SPAWN;
use super::rows::*;
use super::types::*;
use super::util::{SWEEPER, caller, contains};
use crate::gotime;

/// A request's ops.
pub const SPAWN_REQUESTED: &str = "requested";
pub const SPAWN_FAILED: &str = "failed";
pub const SPAWN_CANCELED: &str = "canceled";

/// How long a request waits for its agent to show in presence before the
/// next request may replace it, so a spawn that died before presence saw
/// it doesn't hold its card forever (the next ask is generation + 1).
pub const SPAWN_GRACE: Duration = Duration::from_secs(180);
/// How long a card whose spawn failed (not trusted, no folder, the spawner
/// refused) waits before Irori asks again, unless the card changes first
/// (card #464): every sweep retrying wrote two replicated rows each 5s.
pub const SPAWN_RETRY: Duration = Duration::from_secs(600);

/// Spawned agents of one card that may crash in a row before its spawning
/// stops and the person is asked (§2 "Crash", card #447).
pub const MAX_SPAWN_CRASHES: u32 = 2;

/// The question the person is asked then.
pub const CRASHED_QUESTION: &str = "Its agent crashed twice in a row: start another?";

/// The person's way on from it (their own words go into the next brief).
pub const CRASHED_START: &str = "Start another agent";

/// Whether b is Irori's crash question.
pub fn is_crash_question(b: &Blocker) -> bool {
    b.raised_by == SWEEPER && b.question == CRASHED_QUESTION
}

/// The permission modes a spawn may pass (§7). One is always passed.
pub const PERMISSION_MODES: [&str; 4] = ["bypassPermissions", "acceptEdits", "default", "plan"];

/// A card's newest spawn request for one part (the card itself when
/// `step` and `subtask` are empty).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spawn {
    pub generation: i64,
    pub step: String,
    pub subtask: String,
    pub op: String,
    pub device: String,
    pub repo_key: String,
    pub folder: String,
    pub worktree: String,
    pub model: String,
    pub effort: String,
    pub permission_mode: String,
    pub name: String,
    pub reason: String,
    /// The person's words for the agent's brief (card #455).
    pub note: String,
    pub by: String,
    pub at: GoTime,
    /// The agent bound to this request, once presence listed it.
    pub key: String,
}

impl Spawn {
    /// Waiting for its agent: requested and not bound yet.
    pub fn pending(&self) -> bool {
        self.op == SPAWN_REQUESTED && self.key.is_empty()
    }

    /// Pending past [`SPAWN_GRACE`] at `now`: a new request may replace it.
    pub fn stale(&self, now: &GoTime) -> bool {
        self.pending() && gotime::at_least(now, &self.at, SPAWN_GRACE)
    }

    fn part(&self) -> (&str, &str) {
        (&self.step, &self.subtask)
    }
}

/// The mark on an agent started for a card.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spawned {
    pub task_id: String,
    pub generation: i64,
    pub step: String,
    pub subtask: String,
    pub device: String,
    /// The shared board whose card it is, for a mark kept in the person's
    /// own log for a board card's agent (card #455); empty otherwise.
    pub board: String,
    pub at: GoTime,
}

/// What the Core asks for when it starts an agent for a card.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpawnAsk {
    pub task_id: String,
    pub step: String,
    pub subtask: String,
    pub device: String,
    pub repo_key: String,
    pub folder: String,
    pub worktree: String,
    pub model: String,
    pub effort: String,
    pub permission_mode: String,
    pub name: String,
    /// The person's words for the agent's brief, for the device that
    /// starts it (card #455).
    pub note: String,
}

impl Task {
    /// The card's own spawn request (no step or subtask), if any.
    pub fn spawn_request(&self) -> Option<&Spawn> {
        self.spawns
            .iter()
            .find(|s| s.step.is_empty() && s.subtask.is_empty())
    }

    /// Whether the card's own agent is starting.
    pub fn spawn_pending(&self) -> bool {
        self.spawn_request().is_some_and(Spawn::pending)
    }
}

impl Fold {
    /// The card agent `key` was started for, if it was spawned; kept
    /// after the agent goes.
    pub(crate) fn spawned_for(&self, key: &str) -> Option<&Spawned> {
        self.spawned.get(key)
    }

    /// Whether `key` may be handed card t: never a spawned agent, except
    /// its own card and that card's subtasks (§6).
    /// Whether Irori may hand agent `key` work nobody gave it (card #453):
    /// not a spawned agent (it serves only its card), and a terminal agent
    /// only with the person's "Offer it cards" on.
    pub(crate) fn takes_handout(&self, key: &str) -> bool {
        self.spawned_for(key).is_none() && self.offers.as_ref().is_none_or(|f| f(key))
    }

    /// Whether Irori may hand card t to agent `key`: a spawned agent only
    /// its own card and its subtasks, any other agent per
    /// [`Fold::takes_handout`].
    pub(crate) fn may_hand_to(&self, key: &str, t: &Task) -> bool {
        match self.spawned_for(key) {
            Some(s) => s.task_id == t.id || t.parent == s.task_id,
            None => self.offers.as_ref().is_none_or(|f| f(key)),
        }
    }

    /// Also a card it made itself: the merge card that lands its work
    /// with other ready work (card #509).
    pub(crate) fn spawned_may_take(&self, key: &str, t: &Task) -> bool {
        match self.spawned_for(key) {
            None => true,
            Some(s) => s.task_id == t.id || t.parent == s.task_id || t.created_by == key,
        }
    }

    /// Whether a `task.spawn`/`task.spawned` row came from someone who may
    /// start an agent here (§7): Irori's own sweep, the person, or one of
    /// the person's agents writing from its own device; on a shared board
    /// only the card's author's person.
    pub(crate) fn spawn_writer_ok(&self, ev: &Event, by: &str, task_id: &str) -> bool {
        if by.is_empty() {
            return false;
        }
        let who_ok = if ev.actor == SWEEPER {
            by == SWEEPER
        } else if self.is_owner(by) {
            self.person_actor(ev)
        } else {
            owner::may_write(by, &ev.device_id)
        };
        if !who_ok {
            return false;
        }
        if !self.on_board {
            return true;
        }
        let (Some(kp), Some(dp)) = (&self.key_person, &self.dev_person) else {
            return false;
        };
        let Some(t) = self.tasks.get(task_id) else {
            return false;
        };
        let author = kp(&t.created_by);
        !author.is_empty() && author == dp(&ev.device_id) && (by == SWEEPER || kp(by) == author)
    }

    pub(crate) fn fold_spawn(&mut self, ev: &Event, at: &dyn Fn(GoTime) -> GoTime) {
        let Some(p) = row::<TaskSpawnRow>(&ev.payload) else {
            return;
        };
        if !self.spawn_writer_ok(ev, &p.by, &p.task_id) {
            self.ignore(&ev.device_id);
            return;
        }
        let Some(t) = self.tasks.get_mut(&p.task_id) else {
            return;
        };
        let s = Spawn {
            generation: p.generation,
            step: p.step,
            subtask: p.subtask,
            op: p.op,
            device: p.device,
            repo_key: ledger::canonical_repo_key(&p.repo_key),
            folder: p.folder,
            worktree: p.worktree,
            model: p.model,
            effort: p.effort,
            permission_mode: p.permission_mode,
            name: p.name,
            reason: p.reason,
            note: p.note,
            by: p.by,
            at: at(p.at),
            key: String::new(),
        };
        match t.spawns.iter_mut().find(|o| o.part() == s.part()) {
            // Only a newer generation moves a request on; its own generation may end it.
            Some(o)
                if s.generation > o.generation
                    || (s.generation == o.generation && s.op != SPAWN_REQUESTED) =>
            {
                let key = if s.generation == o.generation {
                    o.key.clone()
                } else {
                    String::new()
                };
                *o = Spawn { key, ..s };
            }
            Some(_) => return,
            None => t.spawns.push(s),
        }
        let holder = t.holder.clone();
        self.note(Change {
            kind: TASK_CHANGED.into(),
            task_id: p.task_id,
            holder,
            ..Change::default()
        });
    }

    pub(crate) fn fold_spawned(&mut self, ev: &Event, at: &dyn Fn(GoTime) -> GoTime) {
        let Some(p) = row::<TaskSpawnedRow>(&ev.payload) else {
            return;
        };
        if p.key.is_empty() || !self.spawn_writer_ok(ev, &p.by, &p.task_id) {
            self.ignore(&ev.device_id);
            return;
        }
        // An agent is started for one card, once: a later mark never moves
        // it to another card. On a shared board the agent must be the card
        // author's own: another member's Irori can't mark this person's
        // agent as started for their card, which would let their hand-out
        // reach it (card #455).
        if self
            .spawned
            .get(&p.key)
            .is_some_and(|m| m.task_id != p.task_id)
        {
            self.ignore(&ev.device_id);
            return;
        }
        if self.on_board {
            let Some(kp) = &self.key_person else {
                return;
            };
            let author = self
                .tasks
                .get(&p.task_id)
                .map(|t| kp(&t.created_by))
                .unwrap_or_default();
            if author.is_empty() || kp(&p.key) != author {
                self.ignore(&ev.device_id);
                return;
            }
        }
        self.spawned_keys
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(p.key.clone());
        self.spawned.insert(
            p.key.clone(),
            Spawned {
                task_id: p.task_id.clone(),
                generation: p.generation,
                step: p.step.clone(),
                subtask: p.subtask.clone(),
                device: p.device.clone(),
                board: p.board.clone(),
                at: at(p.at),
            },
        );
        let Some(t) = self.tasks.get_mut(&p.task_id) else {
            return;
        };
        if let Some(s) = t
            .spawns
            .iter_mut()
            .find(|s| s.generation == p.generation && s.step == p.step && s.subtask == p.subtask)
        {
            s.key = p.key.clone();
        }
        let holder = t.holder.clone();
        self.note(Change {
            kind: TASK_CHANGED.into(),
            task_id: p.task_id,
            holder,
            ..Change::default()
        });
    }

    /// Writes a request for card `a.task_id`'s agent, the card's next
    /// generation. Ok(None) when one is pending already (no double spawn under
    /// repeated kicks) or the card no longer needs an agent.
    pub(crate) fn request_spawn(&mut self, by: &str, a: SpawnAsk) -> Result<Option<i64>> {
        let by = caller(by)?;
        if a.step.is_empty() && a.subtask.is_empty() {
            self.spawn_now.remove(&a.task_id);
        }
        if !PERMISSION_MODES.contains(&a.permission_mode.as_str()) {
            return Err(bad_state(format!(
                "a spawn needs an explicit permission mode, not {:?}",
                a.permission_mode
            )));
        }
        let now = self.stamp();
        let t = self.open_task(&a.task_id)?;
        let mine = t
            .spawns
            .iter()
            .find(|s| s.step == a.step && s.subtask == a.subtask);
        if mine.is_some_and(|s| s.pending() && !s.stale(&now)) {
            return Ok(None);
        }
        let card = a.step.is_empty() && a.subtask.is_empty();
        if card && !super::offers::needs_agent(t) && !self.ask_needs_agent(t) && !self.needs_lead(t)
        {
            return Ok(None);
        }
        let generation = mine.map(|s| s.generation).unwrap_or(0) + 1;
        let actor = if by == SWEEPER {
            SWEEPER.to_string()
        } else {
            self.actor_for(&by)
        };
        self.write_as(
            &actor,
            kinds::TASK_SPAWN,
            &TaskSpawnRow {
                task_id: a.task_id,
                generation,
                step: a.step,
                subtask: a.subtask,
                op: SPAWN_REQUESTED.into(),
                device: a.device,
                repo_key: a.repo_key,
                folder: a.folder,
                worktree: a.worktree,
                model: a.model,
                effort: a.effort,
                permission_mode: a.permission_mode,
                name: a.name,
                reason: String::new(),
                note: a.note,
                by,
                at: now,
            },
        )?;
        Ok(Some(generation))
    }

    /// Ends request `generation` for card `id`'s part as failed or canceled, with
    /// the reason the card shows. Ok(false) when it isn't the newest.
    pub(crate) fn end_spawn(
        &mut self,
        by: &str,
        id: &str,
        part: (&str, &str),
        generation: i64,
        op: &str,
        reason: &str,
    ) -> Result<bool> {
        let by = caller(by)?;
        if op != SPAWN_FAILED && op != SPAWN_CANCELED {
            return Err(bad_state(format!("{op:?} doesn't end a spawn")));
        }
        let t = self.open_task(id)?;
        let Some(s) = t
            .spawns
            .iter()
            .find(|s| s.part() == part && s.generation == generation)
            .cloned()
        else {
            return Ok(false);
        };
        let actor = if by == SWEEPER {
            SWEEPER.to_string()
        } else {
            self.actor_for(&by)
        };
        let now = self.stamp();
        self.write_as(
            &actor,
            kinds::TASK_SPAWN,
            &TaskSpawnRow {
                task_id: id.into(),
                generation,
                step: s.step,
                subtask: s.subtask,
                op: op.into(),
                device: s.device,
                permission_mode: s.permission_mode,
                reason: reason.into(),
                by,
                at: now,
                ..TaskSpawnRow::default()
            },
        )?;
        Ok(true)
    }

    /// Whether a cancel `by` writes here for card t counts: on a board
    /// only from the author's person's device (see spawn_writer_ok).
    fn may_end_spawn(&self, by: &str, t: &Task) -> bool {
        if !self.on_board {
            return true;
        }
        let (Some(kp), Some(dp)) = (&self.key_person, &self.dev_person) else {
            return false;
        };
        let author = kp(&t.created_by);
        !author.is_empty() && author == dp(&self.dev) && (by == SWEEPER || kp(by) == author)
    }

    /// Ends every request of card `id` still waiting for its agent (card
    /// #570), where this device's cancel counts. Anywhere else the
    /// author's device ends it: bind_spawn and the sweep both do.
    pub(crate) fn end_pending_spawns(&mut self, by: &str, id: &str, reason: &str) -> Result<()> {
        let by = if self.may_end_spawn(by, &self.tasks[id]) {
            by
        } else if self.may_end_spawn(SWEEPER, &self.tasks[id]) {
            SWEEPER
        } else {
            return Ok(());
        };
        let pending: Vec<Spawn> = self.tasks[id]
            .spawns
            .iter()
            .filter(|s| s.pending())
            .cloned()
            .collect();
        for s in pending {
            self.end_spawn(by, id, s.part(), s.generation, SPAWN_CANCELED, reason)?;
        }
        Ok(())
    }

    /// Binds agent `key` to request `generation` of card `id` once presence
    /// lists it: the spawned_for mark, then the card assigned to it (which
    /// keeps it out of hand-out and lets `check_free` through, §2).
    pub(crate) fn bind_spawn(
        &mut self,
        id: &str,
        generation: i64,
        part: (&str, &str),
        key: &str,
    ) -> Result<bool> {
        let t = self.open_task(id)?;
        let Some(s) = t
            .spawns
            .iter()
            .find(|s| s.part() == part && s.generation == generation)
            .cloned()
        else {
            return Ok(false);
        };
        if s.op != SPAWN_REQUESTED || !s.key.is_empty() {
            return Ok(s.key == key);
        }
        // A held card gets no agent (card #570): its request ends here.
        if t.parked().is_some() {
            self.end_pending_spawns(SWEEPER, id, HELD_SPAWN)?;
            return Ok(false);
        }
        let now = self.stamp();
        self.write_as(
            SWEEPER,
            kinds::TASK_SPAWNED,
            &TaskSpawnedRow {
                task_id: id.into(),
                key: key.into(),
                generation,
                step: s.step.clone(),
                subtask: s.subtask.clone(),
                device: s.device.clone(),
                by: SWEEPER.into(),
                at: now,
                ..TaskSpawnedRow::default()
            },
        )?;
        if s.step.is_empty() && s.subtask.is_empty() {
            let t = &self.tasks[id];
            // An ask (card #470): its take lock hands it over, not an
            // assignment (ask_gate holds that back on an open ask). Taken,
            // not handed: a fresh agent's first turn may end before it
            // replies, and only its going away moves the ask on.
            if let Some(a) = t.open_ask() {
                // A thread another agent answered while this one started
                // (card #512: nobody waits on its named agents) stays
                // answered: taking it would reopen it for a second answer.
                let answered = a.thread && a.reason == super::threads::TAKE_ANSWERED;
                if a.taker.is_empty() && a.owner == self.dev && !answered {
                    let mut heard = a.heard.clone();
                    if !heard.iter().any(|h| h == key) {
                        heard.push(key.to_string());
                    }
                    self.write_take(
                        id,
                        TakeStateRow {
                            taker: key.into(),
                            reason: super::asks::TAKE_TAKEN.into(),
                            told: heard.len() as i64,
                            heard,
                            since: now,
                            ..TakeStateRow::default()
                        },
                    )?;
                }
                return Ok(true);
            }
            if !contains(&t.holders, key) {
                self.write_as(
                    SWEEPER,
                    kinds::TASK_ASSIGNED,
                    &TaskAssignedRow {
                        task_id: id.into(),
                        to: key.into(),
                        by: SWEEPER.into(),
                        at: now,
                        note: String::new(),
                    },
                )?;
            }
        }
        Ok(true)
    }

    /// Marks agent `key` as started by Irori for card `task_id` of another
    /// log (a shared board's, card #455), in this one: the person's own
    /// hand-out never offers it a card here, and their devices know it as
    /// spawned. The card isn't in this log; the mark is all it carries.
    pub(crate) fn mark_spawned_elsewhere(
        &mut self,
        key: &str,
        task_id: &str,
        generation: i64,
        device: &str,
        board: &str,
    ) -> Result<()> {
        if self.spawned.get(key).is_some_and(|m| m.task_id == task_id) {
            return Ok(());
        }
        let now = self.stamp();
        self.write_as(
            SWEEPER,
            kinds::TASK_SPAWNED,
            &TaskSpawnedRow {
                task_id: task_id.into(),
                key: key.into(),
                generation,
                device: device.into(),
                board: board.into(),
                by: SWEEPER.into(),
                at: now,
                ..TaskSpawnedRow::default()
            },
        )?;
        Ok(())
    }

    /// Whether this log spawns an agent per card (the board's switch).
    pub(crate) fn spawning_on(&self) -> bool {
        self.spawning.as_ref().is_some_and(|f| f())
    }

    /// Whether ask t waits for the person before Irori starts another
    /// agent for it (card #470): Irori ended the last one it started (it
    /// went quiet, or the app restarted under it), and the person hasn't
    /// written on it or asked again since. Without this an ask nobody
    /// answers would get a fresh agent every idle limit.
    pub(crate) fn ask_resting(&self, t: &Task) -> bool {
        let Some(a) = t.open_ask() else {
            return false;
        };
        let Some(s) = t.spawn_request() else {
            return false;
        };
        if s.key.is_empty() || !self.ended_by_irori.contains(&s.key) {
            return false;
        }
        if a.reason == super::asks::TAKE_REOPENED {
            return false;
        }
        !self
            .comments
            .values()
            .any(|c| c.task_id == t.id && self.is_person(&c.by) && !gotime::before(&c.at, &s.at))
    }

    /// The requests naming this device that it may start an agent for now
    /// (card #455): the card's newest, still requested and unbound, for a
    /// card that still needs an agent. Whether a request went stale is the
    /// card's own device's call, by its own clock: it then writes the next
    /// generation, which supersedes this one here (and the Core never
    /// starts a request it already held when it started). On a shared board only cards this
    /// device's own person made, asked for by them or Irori on one of
    /// their devices: another member's request naming this device, even on
    /// their own card, is never acted on (§7). Oldest first.
    pub(crate) fn spawns_here(&self) -> Vec<(Task, Spawn)> {
        let mut v: Vec<(Task, Spawn)> = self
            .tasks
            .values()
            .filter(|t| {
                super::offers::needs_agent(t) || self.ask_needs_agent(t) || self.needs_lead(t)
            })
            .filter_map(|t| {
                let s = t.spawn_request()?;
                (s.pending() && s.device == self.dev && self.mine_to_start(t, s))
                    .then(|| (t.clone(), s.clone()))
            })
            .collect();
        v.sort_by(|a, b| gotime::cmp(&a.1.at, &b.1.at).then_with(|| a.0.id.cmp(&b.0.id)));
        v
    }

    /// Whether this device's person may have it start card t's request s
    /// (§7, card #455): always on the private log, which only their own
    /// devices write; on a shared board only for a card they made, asked
    /// for by them (or Irori on their device).
    pub(crate) fn mine_to_start(&self, t: &Task, s: &Spawn) -> bool {
        if !self.on_board {
            return true;
        }
        let Some(kp) = &self.key_person else {
            return false;
        };
        let me = kp(&self.person);
        !me.is_empty() && kp(&t.created_by) == me && (s.by == SWEEPER || kp(&s.by) == me)
    }

    /// The cards this device hands out that need an agent and have none
    /// starting, oldest first; none while the board doesn't spawn.
    pub(crate) fn cards_to_spawn(&self) -> Vec<Task> {
        if !self.spawning_on() {
            return Vec::new();
        }
        let now = self.stamp();
        let mut v: Vec<Task> = self
            .tasks
            .values()
            .filter(|t| {
                ((super::offers::needs_agent(t) || self.needs_lead(t)) && self.hands_out(t))
                    || self.ask_needs_agent(t)
            })
            // Only on a device that runs the card's side (card #495),
            // unless the person asked for an agent on it here.
            .filter(|t| self.runs_here(t) || self.spawn_now.contains(&t.id))
            .filter(|t| t.spawn_crashes < MAX_SPAWN_CRASHES)
            .filter(|t| !self.ask_resting(t))
            .filter(|t| !self.waits_for_lead(t))
            .filter(|t| {
                t.spawn_request()
                    .is_none_or(|s| !s.pending() || s.stale(&now))
            })
            .filter(|t| {
                t.spawn_request().is_none_or(|s| {
                    s.op != SPAWN_FAILED
                        || s.device != self.dev
                        || self.spawn_now.contains(&t.id)
                        || gotime::at_least(&now, &s.at, SPAWN_RETRY)
                        || gotime::before(&s.at, &t.updated_at)
                })
            })
            .cloned()
            .collect();
        v.sort_by(|a, b| gotime::cmp(&a.created_at, &b.created_at).then_with(|| a.id.cmp(&b.id)));
        self.within_limits(v)
    }
}
