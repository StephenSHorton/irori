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
use super::fold::{Fold, is_holder};
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

/// What a person is asked about a card another person gave them (card
/// #573): Irori stops there, and nothing starts until they say yes.
pub const GIVEN_QUESTION: &str = "OK to start an agent on this?";

/// The answer that starts it.
pub const GIVEN_START: &str = "Start an agent";

/// The answer that leaves the card theirs, with no agent.
pub const GIVEN_NOT_NOW: &str = "Not now";

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
    /// It counts toward "My agents may start agents" (card #575).
    pub counted: bool,
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
    /// It counts toward "My agents may start agents" (card #575), and so
    /// does whatever it goes on to start.
    pub counted: bool,
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
    /// It counts toward "My agents may start agents" (card #575).
    pub counted: bool,
}

/// What the card itself says about getting an agent (card #574): what
/// [`Fold::cards_to_spawn`] looks at, as a reason. The core adds the switch,
/// the devices and the cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnHold {
    /// Nothing on the card is in the way.
    Clear,
    /// It doesn't want an agent: done, withdrawn, held, offered, a parent.
    NoNeed,
    /// A shared card: agents join it themselves.
    Joinable,
    /// It waits on a blocker.
    Blocked,
    /// Its agents crashed twice in a row.
    Crashed,
    /// On a side this device doesn't run: why, in the card's words.
    OffSide(String),
    /// An unmarked subtask of a card with a lead.
    ForLead,
    /// Its parent has this many agents, its most.
    AtLimit(usize),
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
    pub(crate) fn spawn_writer_ok(&self, ev: &Event, by: &str, task_id: &str, op: &str) -> bool {
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
        // One person serves a card: its author, or the person it was given
        // to, and then only on their yes (card #573). The author's devices
        // write nothing for a card they gave away.
        let serves = self.serving_person(t);
        !serves.is_empty()
            && serves == dp(&ev.device_id)
            && (by == SWEEPER || kp(by) == serves)
            && (op != SPAWN_REQUESTED || self.given_person(t).is_none() || self.yes_unspent(t))
    }

    pub(crate) fn fold_spawn(&mut self, ev: &Event, at: &dyn Fn(GoTime) -> GoTime) {
        let Some(p) = row::<TaskSpawnRow>(&ev.payload) else {
            return;
        };
        if !self.spawn_writer_ok(ev, &p.by, &p.task_id, &p.op) {
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
            counted: p.counted,
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
                let same = s.generation == o.generation;
                let key = if same { o.key.clone() } else { String::new() };
                // A request's end doesn't repeat that it counts (card #575).
                let counted = s.counted || (same && o.counted);
                *o = Spawn { key, counted, ..s };
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
        if p.key.is_empty() || !self.spawn_writer_ok(ev, &p.by, &p.task_id, "") {
            self.ignore(&ev.device_id);
            return;
        }
        // An agent is started for one card, once: a later mark never moves
        // it to another card. On a shared board the agent must be the card's
        // own person's (its author, or who it was given to, card #573):
        // another member's Irori can't mark this person's agent as started
        // for their card, which would let their hand-out reach it (card
        // #455).
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
            let serves = self
                .tasks
                .get(&p.task_id)
                .map(|t| self.serving_person(t))
                .unwrap_or_default();
            if serves.is_empty() || kp(&p.key) != serves {
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
                counted: p.counted,
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
        if card
            && !super::offers::needs_agent(t)
            && !self.ask_needs_agent(t)
            && !self.needs_lead(t)
            && !self.given_starts(t)
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
                counted: a.counted,
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
                counted: s.counted,
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
        counted: bool,
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
                counted,
                by: SWEEPER.into(),
                at: now,
                ..TaskSpawnedRow::default()
            },
        )?;
        Ok(())
    }

    /// Whether card t's next agent counts toward "My agents may start
    /// agents" (card #575) with no new ask: the card's last request did,
    /// or an agent that counts made the card. That is the chain: an agent
    /// started by an agent can't get past the limit by posting a card, or
    /// by splitting one for helpers.
    pub(crate) fn in_chain(&self, t: &Task) -> bool {
        match t.spawn_request() {
            // A request that didn't count was the person's Start, which
            // holds through a retry; a crash they answered is theirs too.
            Some(s) => s.counted && t.person_restart != s.generation,
            None => self.spawned.get(&t.created_by).is_some_and(|m| m.counted),
        }
    }

    /// The agents that count toward the limit and haven't gone, by key:
    /// on any of the person's devices, since marks replicate. An agent on
    /// a device that's off still counts, so the count never reads low.
    pub(crate) fn counted_live(&self) -> Vec<String> {
        let rs = self.roster_status();
        let mut v: Vec<String> = self
            .spawned
            .iter()
            .filter(|(k, m)| m.counted && !self.key_gone_in(k, &rs))
            .map(|(k, _)| k.clone())
            .collect();
        v.sort();
        v
    }

    /// Whether agent `key`, marked as started by Irori, hasn't gone.
    pub(crate) fn spawned_live(&self, key: &str) -> bool {
        self.spawned.contains_key(key) && !self.key_gone(key)
    }

    /// This person's counted requests of this log still waiting for their
    /// agent, oldest first: (when, card id). Another member's on a shared
    /// board are their limit's, and one past its grace holds no slot.
    pub(crate) fn counted_pending(&self) -> Vec<(GoTime, String)> {
        let now = self.stamp();
        let mut v: Vec<(GoTime, String)> = self
            .tasks
            .values()
            .filter(|t| !is_terminal(&t.state))
            .flat_map(|t| t.spawns.iter().map(move |s| (t, s)))
            .filter(|(t, s)| s.counted && s.pending() && !s.stale(&now) && self.mine_to_start(t, s))
            .map(|(t, s)| (s.at, t.id.clone()))
            .collect();
        v.sort_by(|a, b| gotime::cmp(&a.0, &b.0).then_with(|| a.1.cmp(&b.1)));
        v
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
                super::offers::needs_agent(t)
                    || self.ask_needs_agent(t)
                    || self.needs_lead(t)
                    || self.given_starts(t)
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
    /// devices write; on a shared board only for a card they serve, asked
    /// for by them (or Irori on their device), and on a card given to them
    /// only a request made since their yes (card #573).
    pub(crate) fn mine_to_start(&self, t: &Task, s: &Spawn) -> bool {
        if !self.on_board {
            return true;
        }
        let Some(kp) = &self.key_person else {
            return false;
        };
        let me = kp(&self.person);
        !me.is_empty()
            && self.serving_person(t) == me
            && (s.by == SWEEPER || kp(&s.by) == me)
            && (self.given_person(t).is_none()
                || t.given_said
                    .as_ref()
                    .is_some_and(|y| y.yes && !gotime::before(&s.at, &y.at)))
    }

    /// The person card t was given to (card #573): a member another
    /// person assigned it to, who still has it. None off a board, and for
    /// a card someone assigned to themselves.
    pub(crate) fn given_person(&self, t: &Task) -> Option<String> {
        let kp = self.key_person.as_ref().filter(|_| self.on_board)?;
        t.given.iter().find_map(|(to, by)| {
            let p = kp(to);
            (!p.is_empty() && kp(by) != p && is_holder(t, to)).then_some(p)
        })
    }

    /// The person whose Irori serves card t on a shared board: who it was
    /// given to, else its author. Only theirs may start its agent.
    pub(crate) fn serving_person(&self, t: &Task) -> String {
        if let Some(p) = self.given_person(t) {
            return p;
        }
        self.key_person
            .as_ref()
            .map(|kp| kp(&t.created_by))
            .unwrap_or_default()
    }

    /// Whether this device's person serves card t.
    pub(crate) fn mine_to_serve(&self, t: &Task) -> bool {
        let Some(kp) = &self.key_person else {
            return false;
        };
        let me = kp(&self.person);
        !me.is_empty() && self.serving_person(t) == me
    }

    /// Whether a person's `task.assigned` row gives a card to another
    /// person: to a `person:` key, by someone else, written from a device
    /// of the person it names as the giver.
    pub(crate) fn gave_to_person(&self, ev: &Event, p: &TaskAssignedRow) -> bool {
        let (true, Some(kp), Some(dp)) = (self.on_board, &self.key_person, &self.dev_person) else {
            return false;
        };
        let (to, by) = (kp(&p.to), kp(&p.by));
        p.to.starts_with(kinds::PERSON_ACTOR_PREFIX)
            && !to.is_empty()
            && !by.is_empty()
            && to != by
            && by == dp(&ev.device_id)
    }

    /// What a `blocker.resolved` row says on the card's "OK to start an
    /// agent on this?", if it is one: yes only from the person it was
    /// given to, as themselves (`person`: never an agent's unblock), on
    /// the question they raised. Any other end to it is a no, so it isn't
    /// asked again.
    pub(crate) fn given_say(
        &self,
        ev: &Event,
        p: &BlockerResolvedRow,
        person: bool,
        at: GoTime,
    ) -> Option<GivenSaid> {
        let t = self.tasks.get(&p.task_id)?;
        let b = t.blockers.iter().find(|b| b.id == p.blocker_id)?;
        if b.question != GIVEN_QUESTION || !b.open() {
            return None;
        }
        let who = self.given_person(t)?;
        let kp = self.key_person.as_ref()?;
        Some(GivenSaid {
            yes: person
                && !p.closed
                && p.answer == GIVEN_START
                && kp(&p.by) == who
                && kp(&b.raised_by) == who,
            person: who,
            device: ev.device_id.clone(),
            at,
        })
    }

    /// Whether card t, given to someone, is free for an agent: nobody but
    /// people on it, and nothing else in the way.
    fn given_free(&self, t: &Task) -> bool {
        // Their answer on the card takes it up, so it reads working with
        // only them on it.
        self.given_person(t).is_some()
            && (t.state == SUBMITTED || t.state == WORKING)
            && !t.withdrawn
            && t.open_ask().is_none()
            && t.subtasks.is_empty()
            && !super::offers::waits_on_work(t)
            && t.holders
                .iter()
                .all(|h| h.starts_with(kinds::PERSON_ACTOR_PREFIX))
    }

    /// Whether the card's yes hasn't started an agent yet. One yes starts
    /// one agent: a later one is asked for again.
    pub(crate) fn yes_unspent(&self, t: &Task) -> bool {
        t.given_said.as_ref().is_some_and(|y| {
            y.yes
                && Some(&y.person) == self.given_person(t).as_ref()
                && t.spawn_request()
                    .is_none_or(|s| s.key.is_empty() || gotime::before(&s.at, &y.at))
        })
    }

    /// Whether card t was given to someone who said yes, and may have its
    /// agent now.
    pub(crate) fn given_starts(&self, t: &Task) -> bool {
        self.given_free(t) && self.yes_unspent(t)
    }

    /// The cards given to this device's person that wait on their say:
    /// not asked yet, or asked and the agent their yes started has gone
    /// with the card unfinished. A Not now stays until it's given again.
    pub(crate) fn given_to_ask(&self) -> Vec<Task> {
        let mut v: Vec<Task> = self
            .tasks
            .values()
            .filter(|t| self.mine_to_serve(t) && self.given_free(t))
            .filter(|t| t.spawn_crashes < MAX_SPAWN_CRASHES)
            .filter(|t| t.given_said.as_ref().is_none_or(|y| y.yes) && !self.yes_unspent(t))
            .cloned()
            .collect();
        v.sort_by(|a, b| gotime::cmp(&a.created_at, &b.created_at).then_with(|| a.id.cmp(&b.id)));
        v
    }

    /// The given cards this device starts an agent for now: the person's
    /// yes, said here, or said on a device that hasn't started it after
    /// [`SPAWN_GRACE`] (a phone runs no agents).
    pub(crate) fn given_to_spawn(&self) -> Vec<Task> {
        let now = self.stamp();
        let mut v: Vec<Task> = self
            .tasks
            .values()
            .filter(|t| self.mine_to_serve(t) && self.given_starts(t))
            .filter(|t| self.runs_here(t))
            .filter(|t| t.spawn_crashes < MAX_SPAWN_CRASHES)
            .filter(|t| {
                t.given_said.as_ref().is_some_and(|y| {
                    y.device == self.dev || gotime::at_least(&now, &y.at, SPAWN_GRACE)
                })
            })
            .filter(|t| self.spawn_due(t, &now))
            .cloned()
            .collect();
        v.sort_by(|a, b| gotime::cmp(&a.created_at, &b.created_at).then_with(|| a.id.cmp(&b.id)));
        v
    }

    /// Asks the person about card `id` given to them, in their own name:
    /// a person's row, so every member's fold takes it from their device.
    pub(crate) fn ask_given(&mut self, by: &str, id: &str, context: &str) -> Result<()> {
        let t = self.open_task(id)?.clone();
        let Some(p) = self.given_person(&t) else {
            return Ok(());
        };
        let now = self.stamp();
        let opt = |answer: &str, why: &str, recommended: bool| BlockerOption {
            answer: answer.into(),
            why: why.into(),
            by: by.to_string(),
            at: now,
            recommended,
            link: None,
        };
        self.raise_locked(
            by,
            &t,
            GIVEN_QUESTION,
            GIVEN_QUESTION,
            context,
            vec![
                opt(
                    GIVEN_START,
                    "It starts here, under your own settings and cap.",
                    false,
                ),
                opt(GIVEN_NOT_NOW, "The card stays yours, with no agent.", false),
            ],
            Some(BlockedBy {
                kind: BLOCKED_ON_PERSON.into(),
                reference: format!("{}{p}", kinds::PERSON_ACTOR_PREFIX),
            }),
            false,
            now,
            Vec::new(),
            Vec::new(),
            false,
            false,
        )?;
        Ok(())
    }

    /// Whether card t's last request lets a new one go now: none is
    /// pending, and a failed one here has waited or the card changed.
    fn spawn_due(&self, t: &Task, now: &GoTime) -> bool {
        t.spawn_request().is_none_or(|s| {
            (!s.pending() || s.stale(now))
                && (s.op != SPAWN_FAILED
                    || s.device != self.dev
                    || self.spawn_now.contains(&t.id)
                    || gotime::at_least(now, &s.at, SPAWN_RETRY)
                    || gotime::before(&s.at, &t.updated_at))
        })
    }

    /// The cards this device hands out that need an agent and have none
    /// starting, oldest first; none while the board doesn't spawn.
    /// See [`SpawnHold`]; the order is the order the card clears them in.
    pub(crate) fn spawn_hold(&self, t: &Task) -> SpawnHold {
        if is_terminal(&t.state) || t.withdrawn {
            return SpawnHold::NoNeed;
        }
        let wanted = super::offers::needs_agent(t) || self.needs_lead(t) || self.ask_needs_agent(t);
        if !wanted {
            let unheld = t.holders.is_empty() && t.assigned.is_empty();
            return if !unheld || !t.subtasks.is_empty() || t.open_ask().is_some() {
                SpawnHold::NoNeed
            } else if t.shared {
                SpawnHold::Joinable
            } else if t.state == BLOCKED
                || t.state == INPUT_REQUIRED
                || super::offers::waits_on_work(t)
            {
                SpawnHold::Blocked
            } else {
                SpawnHold::NoNeed
            };
        }
        if t.spawn_crashes >= MAX_SPAWN_CRASHES {
            return SpawnHold::Crashed;
        }
        if !self.runs_here(t) && !self.spawn_now.contains(&t.id) {
            let why = self.side_wait(t);
            return SpawnHold::OffSide(if why.is_empty() {
                "it's on a side no device here runs".into()
            } else {
                why
            });
        }
        if self.ask_resting(t) {
            return SpawnHold::NoNeed;
        }
        if self.waits_for_lead(t) {
            return SpawnHold::ForLead;
        }
        if self.within_limits(vec![t.clone()]).is_empty() {
            let limit = self.tasks.get(&t.parent).map_or(0, |p| self.agent_limit(p));
            return SpawnHold::AtLimit(limit);
        }
        SpawnHold::Clear
    }

    /// The device that hands card t out, so whose sweep starts its agent.
    pub(crate) fn card_origin(&self, t: &Task) -> String {
        if self.on_board {
            return self.origin.get(&t.id).cloned().unwrap_or_default();
        }
        owner::device_of(&t.created_by).to_string()
    }

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
            .filter(|t| self.spawn_due(t, &now))
            .cloned()
            .collect();
        v.sort_by(|a, b| gotime::cmp(&a.created_at, &b.created_at).then_with(|| a.id.cmp(&b.id)));
        self.within_limits(v)
    }
}
