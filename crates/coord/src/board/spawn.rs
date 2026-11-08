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

use super::error::{Result, bad_state, not_allowed};
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
/// How long a card whose spawn failed as not trusted waits before Irori
/// looks again (card #699): the person trusting the folder anywhere (in
/// Claude Code, or Settings) starts its agent within this. Looking costs
/// no row: until the folder is trusted the Core's gate writes nothing
/// (card #467), so #464's rows every sweep can't come back.
pub const SPAWN_RETRY_UNTRUSTED: Duration = Duration::from_secs(60);
/// A spawn failure's reason when the device's Claude Code doesn't trust
/// the card's folder: `not trusted: <folder> on <device>`.
pub const NOT_TRUSTED_PREFIX: &str = "not trusted: ";

/// How long failed request `s` waits before the next (card #699): a
/// not-trusted failure is looked at again soon, as that costs no row.
pub fn retry_after(s: &Spawn) -> Duration {
    if s.reason.starts_with(NOT_TRUSTED_PREFIX) {
        SPAWN_RETRY_UNTRUSTED
    } else {
        SPAWN_RETRY
    }
}

/// Spawned agents of one card that may crash in a row before its spawning
/// stops and the person is asked (§2 "Crash", card #447).
pub const MAX_SPAWN_CRASHES: u32 = 2;

/// The question the person is asked then.
pub const CRASHED_QUESTION: &str = "Its agent crashed twice in a row: start another?";

/// The person's way on from it (their own words go into the next brief).
pub const CRASHED_START: &str = "Start another agent";

/// Whether b is Irori's crash question.
pub fn is_crash_question(b: &Blocker) -> bool {
    b.raised_by == SWEEPER && is_crash_text(&b.question)
}

/// Whether `q` is [`crash_question`]'s, at any limit (card #576).
pub(crate) fn is_crash_text(q: &str) -> bool {
    q.starts_with("Its agent crashed") && q.ends_with(": start another?")
}

/// The question at a crash limit of `n`: [`CRASHED_QUESTION`] at the
/// default, so rows and older devices read it as before.
pub fn crash_question(n: u32) -> String {
    match n {
        0 | 1 => "Its agent crashed: start another?".into(),
        MAX_SPAWN_CRASHES => CRASHED_QUESTION.into(),
        n => format!("Its agent crashed {n} times in a row: start another?"),
    }
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
    /// The weight it runs at (card #966); empty on older rows.
    pub weight: String,
    /// Whether the advisor was on where it was asked for (card #966).
    pub advisor: bool,
    pub permission_mode: String,
    pub name: String,
    pub reason: String,
    /// The person's words for the agent's brief (card #455).
    pub note: String,
    /// It counts toward "My agents may start agents" (card #575).
    pub counted: bool,
    /// Its agent reads another person's text (card #1075).
    pub others: bool,
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
    /// The weight it runs at, and whether the advisor is on (card #966).
    pub weight: String,
    pub advisor: bool,
    pub permission_mode: String,
    pub name: String,
    /// The person's words for the agent's brief, for the device that
    /// starts it (card #455).
    pub note: String,
    /// It counts toward "My agents may start agents" (card #575).
    pub counted: bool,
    /// Its agent reads another person's text (card #1075): its brief
    /// quotes it, or its card is on a shared board.
    pub others: bool,
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
    /// A thread whose last agent Irori ended: it waits for the person.
    Resting,
    /// On a side this device doesn't run: why, in the card's words.
    OffSide(String),
    /// An unmarked subtask of a card with a lead.
    ForLead,
    /// An agent that had read another person's text made or changed it
    /// (card #1075): it waits on the person's "OK to start an agent on
    /// this?".
    NeedsOk,
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
        // One person serves a card: who it was given to, else its author.
        // Their devices alone write its spawn rows, so the author's write
        // nothing for a card they gave away. The yes isn't checked here:
        // only their own devices pass this, and the asking device checks
        // it before it starts or binds anything (card #573).
        let serves = self.serving_person(t);
        !serves.is_empty() && serves == dp(&ev.device_id) && (by == SWEEPER || kp(by) == serves)
    }

    pub(crate) fn fold_spawn(&mut self, ev: &Event, at: &dyn Fn(GoTime) -> GoTime) {
        let Some(p) = row::<TaskSpawnRow>(&ev.payload) else {
            return;
        };
        if !self.spawn_writer_ok(ev, &p.by, &p.task_id) {
            self.ignore(&ev.device_id);
            return;
        }
        if p.op == SPAWN_REQUESTED
            && p.step.is_empty()
            && p.subtask.is_empty()
            && !p.weight.is_empty()
        {
            self.runs.entry(p.task_id.clone()).or_default().insert(
                p.generation,
                super::runs::Run {
                    weight: p.weight.clone(),
                    model: p.model.clone(),
                    effort: p.effort.clone(),
                    advisor: p.advisor,
                },
            );
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
            weight: p.weight,
            advisor: p.advisor,
            permission_mode: p.permission_mode,
            name: p.name,
            reason: p.reason,
            note: p.note,
            counted: p.counted,
            others: p.others,
            by: p.by,
            at: at(p.at),
            key: String::new(),
        };
        match t.spawns.iter_mut().find(|o| o.part() == s.part()) {
            // Only a newer generation moves a request on.
            Some(o) if s.generation > o.generation => *o = s,
            // Its own generation's rows from the same device may end it;
            // a cancel (the person's) ends the generation, whichever
            // computer's request it named.
            Some(o)
                if s.generation == o.generation
                    && s.op != SPAWN_REQUESTED
                    && o.op != SPAWN_CANCELED
                    && (s.op == SPAWN_CANCELED || same_device(&s.device, &o.device)) =>
            {
                // A request's end doesn't repeat that it counts (card #575).
                *o = Spawn {
                    key: o.key.clone(),
                    counted: s.counted || o.counted,
                    others: s.others || o.others,
                    ..s
                };
            }
            // Two computers took one card at once (card #541): the lowest
            // device id's request wins over anything of the other's in
            // that generation, so every device ends with the same request
            // whatever order the rows arrive in. The other one's session
            // ends as superseded.
            Some(o)
                if s.generation == o.generation
                    && s.op == SPAWN_REQUESTED
                    && o.op != SPAWN_CANCELED
                    && !same_device(&s.device, &o.device)
                    && s.device < o.device =>
            {
                *o = s;
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
        // Only on the request of the device that started it: the losing
        // computer's agent never binds the winner's (card #541).
        if let Some(s) = t.spawns.iter_mut().find(|s| {
            s.generation == p.generation
                && s.step == p.step
                && s.subtask == p.subtask
                && same_device(&s.device, &p.device)
        }) {
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
                weight: a.weight,
                advisor: a.advisor,
                permission_mode: a.permission_mode,
                name: a.name,
                reason: String::new(),
                note: a.note,
                counted: a.counted,
                others: a.others,
                by,
                at: now,
            },
        )?;
        Ok(Some(generation))
    }

    /// Ends request `generation` of card `id` as failed because this device
    /// couldn't start it (card #541): Ok(false), writing nothing, when the
    /// request is now another computer's, which this failure isn't about.
    pub(crate) fn fail_own_spawn(
        &mut self,
        id: &str,
        generation: i64,
        reason: &str,
    ) -> Result<bool> {
        let theirs = self.tasks.get(id).is_some_and(|t| {
            t.spawns.iter().any(|s| {
                s.part() == ("", "")
                    && s.generation == generation
                    && !same_device(&s.device, &self.dev)
            })
        });
        if theirs {
            return Ok(false);
        }
        self.end_spawn(SWEEPER, id, ("", ""), generation, SPAWN_FAILED, reason)
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
                // So a session started under that name can still be found
                // and ended (card #570).
                name: s.name,
                reason: reason.into(),
                by,
                at: now,
                ..TaskSpawnRow::default()
            },
        )?;
        Ok(true)
    }

    /// Whether a cancel `by` writes here for card t counts: on a board
    /// only from the device of the person serving it, who it was given to
    /// else its author (see spawn_writer_ok, card #573).
    fn may_end_spawn(&self, by: &str, t: &Task) -> bool {
        if !self.on_board {
            return true;
        }
        let (Some(kp), Some(dp)) = (&self.key_person, &self.dev_person) else {
            return false;
        };
        let serves = self.serving_person(t);
        !serves.is_empty() && serves == dp(&self.dev) && (by == SWEEPER || kp(by) == serves)
    }

    /// Ends every request of card `id` still waiting for its agent (card
    /// #570), where this device's cancel counts. Anywhere else the serving
    /// person's device ends it: bind_spawn and the sweep both do.
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
        // Another computer's request won this generation (card #541).
        if !same_device(&s.device, &self.dev) {
            return Ok(false);
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

    /// Agent `key`, started for card `id`, began its first turn (card #699):
    /// it takes the card up as itself, so the card stops reading "Offered,
    /// not taken up yet" while it works. Only the card's own agent (not a
    /// step's or subtask's), only while the card is still offered to it,
    /// and not on an ask (its take lock hands that over). Writing only once
    /// the assignment is folded keeps log order: the assigned row can never
    /// land after the claim and offer the card again. Ok(true) when the
    /// claim was written.
    pub(crate) fn take_up_started(&mut self, key: &str, id: &str) -> Result<bool> {
        let Some(m) = self.spawned_for(key) else {
            return Ok(false);
        };
        if m.task_id != id || !m.step.is_empty() || !m.subtask.is_empty() {
            return Ok(false);
        }
        let Some(t) = self.tasks.get(id) else {
            return Ok(false);
        };
        // A held card is taken by nobody (parked_take_refusal): its agent
        // is ended, not made its holder.
        if is_terminal(&t.state)
            || t.parked().is_some()
            || t.open_ask().is_some()
            || !contains(&t.assigned, key)
        {
            return Ok(false);
        }
        let now = self.stamp();
        self.write(
            key,
            kinds::TASK_CLAIMED,
            &TaskClaimedRow {
                task_id: id.into(),
                by: key.into(),
                at: now,
            },
        )?;
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
            // An agent's own thread starts one only on its person's post
            // (card #631), so its opener's mark doesn't carry.
            None => {
                t.ask.as_ref().is_none_or(|a| a.opener.is_empty())
                    && self.spawned.get(&t.created_by).is_some_and(|m| m.counted)
            }
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
        if s.key.is_empty()
            || !self.ended_by_irori.contains(&s.key)
            || self.usage_limited.contains(&s.key)
        {
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

    /// Whether this device's person may have it start or bind card t's
    /// request s (§7, card #455): always on the private log, which only
    /// their own devices write; on a shared board only for a card they
    /// serve, asked for by them (or Irori on their device), and on a card
    /// given to them only a request made since their yes, on the device
    /// that asked them (card #573).
    pub(crate) fn mine_to_start(&self, t: &Task, s: &Spawn) -> bool {
        if !self.on_board {
            return true;
        }
        let Some(kp) = &self.key_person else {
            return false;
        };
        let me = kp(&self.person);
        if me.is_empty() || self.serving_person(t) != me || !(s.by == SWEEPER || kp(&s.by) == me) {
            return false;
        }
        match &t.given {
            None => true,
            Some(g) => {
                g.asker == self.dev
                    && t.given_said
                        .as_ref()
                        .is_some_and(|y| y.yes && s.generation > y.generation)
            }
        }
    }

    /// The person whose Irori serves card t on a shared board: who it was
    /// given to, else its author. Only theirs may start its agent.
    pub(crate) fn serving_person(&self, t: &Task) -> String {
        if let Some(g) = &t.given {
            return g.person.clone();
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

    /// The person a `task.assigned` row gives a card to: a `person:` key,
    /// another person's, written from a device of the person it names as
    /// the giver, who made or owns the card. Nobody gives away a card that
    /// isn't theirs, nor gives one that isn't given to the person who
    /// serves it already; giving it again starts its question over.
    pub(crate) fn gave_to_person(&self, ev: &Event, p: &TaskAssignedRow) -> Option<String> {
        let (true, Some(kp), Some(dp)) = (self.on_board, &self.key_person, &self.dev_person) else {
            return None;
        };
        let t = self.tasks.get(&p.task_id)?;
        let (to, by) = (kp(&p.to), kp(&p.by));
        (p.to.starts_with(kinds::PERSON_ACTOR_PREFIX)
            && !to.is_empty()
            && !by.is_empty()
            && to != by
            && by == dp(&ev.device_id)
            && self.may_give(t, &by)
            && (t.given.is_some() || self.serving_person(t) != to))
            .then_some(to)
    }

    /// The persons whose card t is: who made it and who owns it.
    fn card_persons(&self, t: &Task) -> [String; 2] {
        [self.person_of(&t.created_by), self.owner_person(t)]
    }

    /// Whether `person` may give card t to someone: it is theirs.
    pub(crate) fn may_give(&self, t: &Task, person: &str) -> bool {
        !person.is_empty() && self.card_persons(t).iter().any(|p| p == person)
    }

    /// Whether a `task.assigned` row is the card's author taking it back:
    /// to their own `person:` key, from their own device.
    pub(crate) fn takes_back(&self, ev: &Event, p: &TaskAssignedRow) -> bool {
        let (Some(kp), Some(dp)) = (&self.key_person, &self.dev_person) else {
            return false;
        };
        let me = dp(&ev.device_id);
        !me.is_empty()
            && p.to.starts_with(kinds::PERSON_ACTOR_PREFIX)
            && kp(&p.to) == me
            && kp(&p.by) == me
            && self.tasks.get(&p.task_id).is_some_and(|t| {
                t.given.as_ref().is_some_and(|g| g.person != me) && kp(&t.created_by) == me
            })
    }

    /// Whether someone other than card `id`'s given person wrote ev, a
    /// change to what its agent's brief says. Decided by the device that
    /// wrote it as well as the `by` it claims: a row anyone can write
    /// names whoever its writer likes.
    pub(crate) fn edited_by_another(&self, ev: &Event, id: &str, by: &str) -> bool {
        let Some(g) = self.tasks.get(id).and_then(|t| t.given.as_ref()) else {
            return false;
        };
        let (Some(kp), Some(dp)) = (&self.key_person, &self.dev_person) else {
            return true;
        };
        kp(by) != g.person || dp(&ev.device_id) != g.person
    }

    /// Whether an assignment to `to`, already on card t, is someone else
    /// giving it again once it has been asked about: after a Not now, or
    /// when the device that asked is gone. It starts over, and a device
    /// that can start it asks.
    pub(crate) fn gives_again(&self, t: &Task, by: &str, to: &str) -> bool {
        t.given
            .as_ref()
            .is_some_and(|g| g.key == to && (t.given_said.is_some() || !g.asker.is_empty()))
            && self.person_of(by) != self.person_of(to)
    }

    /// A card with an agent on it, or one starting, can't be given: its
    /// agent would be orphaned on its person's device. A parent card's
    /// agents are its parts' (card #573).
    pub(crate) fn check_give(&self, t: &Task, by: &str, to: &str) -> Result<()> {
        if !self.on_board
            || !to.starts_with(kinds::PERSON_ACTOR_PREFIX)
            || self.person_of(by) == self.person_of(to)
        {
            return Ok(());
        }
        if !self.may_give(t, &self.person_of(by)) {
            return Err(not_allowed(format!(
                "{} isn't yours to give: only the person who made or owns it gives it to someone",
                t.short_ref()
            )));
        }
        if t.given.is_none() && self.serving_person(t) == self.person_of(to) {
            return Err(bad_state(format!(
                "{} is already theirs: assign it to one of their agents instead",
                t.short_ref()
            )));
        }
        if !t.subtasks.is_empty() {
            return Err(bad_state(format!(
                "{} is split into parts: give a part, not the whole card",
                t.short_ref()
            )));
        }
        let agent = t
            .holders
            .iter()
            .any(|h| !h.starts_with(kinds::PERSON_ACTOR_PREFIX));
        let now = self.stamp();
        if agent
            || t.spawn_request()
                .is_some_and(|s| s.pending() && !s.stale(&now))
        {
            return Err(bad_state(format!(
                "{} has an agent on it: end it before giving the card to someone",
                t.short_ref()
            )));
        }
        Ok(())
    }

    /// What a `blocker.resolved` row says on the card's "OK to start an
    /// agent on this?", if it is one and the given person wrote it as
    /// themselves (`person`: never their agent): yes on the question
    /// they raised. Anyone else closing it says nothing, so they are asked
    /// again.
    pub(crate) fn given_say(&self, p: &BlockerResolvedRow, person: bool) -> Option<GivenSaid> {
        let t = self.tasks.get(&p.task_id)?;
        let b = t.blockers.iter().find(|b| b.id == p.blocker_id)?;
        if b.question != GIVEN_QUESTION || !b.open() {
            return None;
        }
        let g = t.given.as_ref()?;
        let who = g.person.clone();
        let kp = self.key_person.as_ref()?;
        if !person || kp(&p.by) != who || g.question != p.blocker_id {
            return None;
        }
        Some(GivenSaid {
            yes: !p.closed && p.answer == GIVEN_START,
            generation: t.spawn_request().map_or(0, |s| s.generation),
            person: who,
        })
    }

    /// Notes the given person raising the question on card t, as
    /// themselves from their own device: no agent's or other member's row
    /// makes a question their yes can stand on. True when it is a second
    /// row for the question the card already has: it changes nothing, and
    /// the lowest device of the two acts.
    pub(crate) fn given_raised(&self, t: &mut Task, p: &TaskStateRow, ev: &Event) -> bool {
        let Some(br) = p.blocker.as_ref().filter(|b| b.question == GIVEN_QUESTION) else {
            return false;
        };
        let mine = t.given.as_ref().is_some_and(|g| g.key == p.by);
        if !mine || !self.person_row(ev, &p.by) {
            return false;
        }
        let known = t.blockers.iter().any(|b| b.id == br.id);
        let Some(g) = t.given.as_mut() else {
            return false;
        };
        // The lowest device that raised it asks, whatever has been said:
        // every device works that out the same from the same rows, so one
        // yes can't have two askers.
        if known {
            if ev.device_id < g.asker {
                g.asker = ev.device_id.clone();
            }
            return true;
        }
        g.asker = ev.device_id.clone();
        g.question = br.id.clone();
        t.given_said = None;
        false
    }

    /// Whether ev waits beside the row that lets it in: rows come device
    /// by device, and a store batch holds one device's rows ahead of
    /// another's. Every row of a card that isn't folded yet waits for it,
    /// so they all fold in the order they came. On a card never given,
    /// the given person's rows wait for the give: their spawn rows, the
    /// bind's assignment, the question and an answer on a blocker not
    /// raised yet. Once a card has been given or taken back, nothing
    /// waits: rows from someone who no longer serves it are ignored, not
    /// kept for a later give.
    pub(crate) fn ahead_of_give(&self, ev: &Event) -> bool {
        if !self.on_board || ev.kind == kinds::TASK_CREATED {
            return false;
        }
        let Some(id) = row::<TaskIdRow>(&ev.payload).filter(|p| !p.task_id.is_empty()) else {
            return false;
        };
        let Some(t) = self.tasks.get(&id.task_id) else {
            return !self.deleted.contains(&id.task_id);
        };
        if t.given.is_some() || self.given_ever.contains(&t.id) {
            return false;
        }
        let (Some(kp), Some(dp)) = (&self.key_person, &self.dev_person) else {
            return false;
        };
        let writer = dp(&ev.device_id);
        if writer.is_empty() || kp(&t.created_by) == writer {
            return false;
        }
        match ev.kind.as_str() {
            kinds::TASK_SPAWN | kinds::TASK_SPAWNED => true,
            kinds::TASK_ASSIGNED => ev.actor == SWEEPER,
            kinds::TASK_STATUS => row::<TaskStateRow>(&ev.payload)
                .is_some_and(|p| p.blocker.is_some_and(|b| b.question == GIVEN_QUESTION)),
            // Only a blocker not raised yet: one already trimmed folds as
            // nothing, as it always has.
            kinds::BLOCKER_RESOLVED => row::<BlockerResolvedRow>(&ev.payload).is_some_and(|p| {
                !t.blockers.iter().any(|b| b.id == p.blocker_id)
                    && super::cards::seq_of(&p.blocker_id, "b")
                        > self.blocker_seq.get(&t.id).copied().unwrap_or(0)
            }),
            _ => false,
        }
    }

    /// Whether ev waits behind an earlier row of its device about the same
    /// card that waits for the give: one device's rows keep their order,
    /// so an agent's leave can't fold before the bind that put it on.
    pub(crate) fn behind_held(&self, ev: &Event, earlier: &[Event]) -> bool {
        // The card's own row is what the others wait for.
        if !self.on_board || ev.kind == kinds::TASK_CREATED {
            return false;
        }
        let Some(id) = row::<TaskIdRow>(&ev.payload).filter(|p| !p.task_id.is_empty()) else {
            return false;
        };
        earlier.iter().any(|a| {
            a.device_id == ev.device_id
                && row::<TaskIdRow>(&a.payload).is_some_and(|p| p.task_id == id.task_id)
                && self.ahead_of_give(a)
        })
    }

    /// Whether card t, given to someone, is free for an agent: only people
    /// on it, and nothing else in the way.
    fn given_free(&self, t: &Task) -> bool {
        t.given.is_some()
            && (t.state == SUBMITTED || t.state == WORKING)
            && !t.withdrawn
            && t.open_ask().is_none()
            && !super::offers::waits_on_work(t)
            && t.holders
                .iter()
                .all(|h| h.starts_with(kinds::PERSON_ACTOR_PREFIX))
    }

    /// Why the agent their yes asked for failed to start, if it did.
    pub(crate) fn given_failed(t: &Task) -> Option<String> {
        let y = t.given_said.as_ref().filter(|y| y.yes)?;
        t.spawn_request()
            .filter(|s| s.op == SPAWN_FAILED && s.generation > y.generation)
            .map(|s| s.reason.clone())
    }

    /// Whether the card's yes still has an agent to start: one yes starts
    /// one agent, and a later one is asked for again.
    pub(crate) fn yes_unspent(t: &Task) -> bool {
        t.given_said.as_ref().is_some_and(|y| {
            y.yes
                && t.spawn_request()
                    .is_none_or(|s| s.generation <= y.generation || s.key.is_empty())
        }) && Self::given_failed(t).is_none()
    }

    /// Whether card t was given to someone who said yes, and may have its
    /// agent now.
    pub(crate) fn given_starts(&self, t: &Task) -> bool {
        self.given_free(t) && Self::yes_unspent(t)
    }

    /// The cards given to this device's person that wait on their say:
    /// not asked yet, or asked and their yes's agent went with the card
    /// unfinished or couldn't start. Once asked, only the device that
    /// asked asks again. A Not now stays until it's given again or they
    /// ask for it on the card.
    pub(crate) fn given_to_ask(&self) -> Vec<Task> {
        let now = self.stamp();
        let mut v: Vec<Task> = self
            .tasks
            .values()
            .filter(|t| self.mine_to_serve(t) && self.given_free(t))
            .filter(|t| t.spawn_crashes < self.crash_limit())
            .filter(|t| {
                t.spawn_request()
                    .is_none_or(|s| !s.pending() || s.stale(&now))
            })
            .filter(|t| {
                t.given
                    .as_ref()
                    .is_some_and(|g| g.asker.is_empty() || g.asker == self.dev)
            })
            .filter(|t| t.given_said.as_ref().is_none_or(|y| y.yes) && !Self::yes_unspent(t))
            .cloned()
            .collect();
        v.sort_by(|a, b| gotime::cmp(&a.created_at, &b.created_at).then_with(|| a.id.cmp(&b.id)));
        v
    }

    /// The given cards with a yes this device starts now: the device that
    /// asked, so one yes never starts two agents, wherever it was said.
    pub(crate) fn given_to_spawn(&self) -> Vec<Task> {
        let now = self.stamp();
        let mut v: Vec<Task> = self
            .tasks
            .values()
            .filter(|t| self.mine_to_serve(t) && self.given_starts(t))
            // A closed board's stray card starts nothing (card #690).
            .filter(|t| !self.board_closed(&t.board))
            .filter(|t| t.given.as_ref().is_some_and(|g| g.asker == self.dev))
            .filter(|t| self.runs_here(t))
            .filter(|t| t.spawn_crashes < self.crash_limit())
            .filter(|t| self.spawn_due(t, &now))
            .cloned()
            .collect();
        v.sort_by(|a, b| gotime::cmp(&a.created_at, &b.created_at).then_with(|| a.id.cmp(&b.id)));
        v
    }

    /// Whether card t, on the private log, waits for the person's OK
    /// before Irori starts an agent for it (card #1075): an agent that had
    /// read another person's text made or changed it, so it is a request
    /// from a stranger in all but name (decision 50), and nothing starts
    /// until the person says yes. Their own "Start an agent" on the card
    /// is that yes. One yes starts one agent.
    pub(crate) fn marked_waits(&self, t: &Task) -> bool {
        !self.on_board
            && t.others
            && !Self::yes_unspent(t)
            && !self.spawn_now.contains(&t.id)
    }

    /// The marked cards that would get an agent but for the person's OK
    /// and have not been asked yet, or whose last yes is spent. A Not now
    /// stays until they ask for an agent on the card themselves.
    pub(crate) fn marked_to_ask(&self) -> Vec<Task> {
        self.spawn_candidates(true)
            .into_iter()
            .filter(|t| t.given_said.as_ref().is_none_or(|y| y.yes))
            .filter(|t| {
                !t.blockers
                    .iter()
                    .any(|b| b.open() && b.question == GIVEN_QUESTION)
            })
            .collect()
    }

    /// Asks the person "OK to start an agent on this?" about marked card
    /// `id`, in their own name (card #1075).
    pub(crate) fn ask_marked(&mut self, by: &str, id: &str, context: &str) -> Result<()> {
        let t = self.open_task(id)?.clone();
        if !t.others || self.on_board {
            return Ok(());
        }
        let now = self.stamp();
        let opt = |answer: &str, why: &str| BlockerOption {
            answer: answer.into(),
            why: why.into(),
            by: by.to_string(),
            at: now,
            recommended: false,
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
                    "It starts under your own settings and cap, never on Bypass.",
                ),
                opt(GIVEN_NOT_NOW, "The card stays yours, with no agent."),
            ],
            Some(BlockedBy {
                kind: BLOCKED_ON_PERSON.into(),
                reference: String::new(),
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

    /// What a `blocker.resolved` row says on a marked card's "OK to start
    /// an agent on this?": the person's own yes or no (card #1075). An
    /// agent closing it says nothing, so they are asked again.
    pub(crate) fn marked_say(&self, p: &BlockerResolvedRow, person: bool) -> Option<GivenSaid> {
        if self.on_board || !person {
            return None;
        }
        let t = self.tasks.get(&p.task_id).filter(|t| t.others)?;
        let b = t.blockers.iter().find(|b| b.id == p.blocker_id)?;
        if b.question != GIVEN_QUESTION || !b.open() {
            return None;
        }
        Some(GivenSaid {
            yes: !p.closed && p.answer == GIVEN_START,
            generation: t.spawn_request().map_or(0, |s| s.generation),
            person: String::new(),
        })
    }

    /// Marks card `id` when ev, a private-log row of an agent that had read
    /// another person's text, says so (card #1075). Sticky.
    pub(crate) fn note_others(&mut self, ev: &Event, id: &str) {
        if self.on_board || !super::carries_others(&ev.kind) {
            return;
        }
        let said = serde_json::from_slice::<serde_json::Value>(&ev.payload)
            .ok()
            .and_then(|v| v.get("others").and_then(serde_json::Value::as_bool))
            .unwrap_or(false);
        if said && let Some(t) = self.tasks.get_mut(id) {
            t.others = true;
        }
    }

    /// The agents whose spawn row says they read another person's text.
    pub(crate) fn spawned_others(&self) -> Vec<String> {
        self.tasks
            .values()
            .flat_map(|t| t.spawns.iter())
            .filter(|s| s.others && !s.key.is_empty())
            .map(|s| s.key.clone())
            .collect()
    }

    /// Asks the person about card `id` given to them, in their own name:
    /// a person's row, so every member's fold takes it from their device.
    pub(crate) fn ask_given(&mut self, by: &str, id: &str, context: &str) -> Result<()> {
        let t = self.open_task(id)?.clone();
        let Some(p) = t.given.as_ref().map(|g| g.person.clone()) else {
            return Ok(());
        };
        let now = self.stamp();
        let opt = |answer: &str, why: &str| BlockerOption {
            answer: answer.into(),
            why: why.into(),
            by: by.to_string(),
            at: now,
            recommended: false,
            link: None,
        };
        self.raise_locked(
            by,
            &t,
            GIVEN_QUESTION,
            GIVEN_QUESTION,
            context,
            vec![
                opt(GIVEN_START, "It starts under your own settings and cap."),
                opt(GIVEN_NOT_NOW, "The card stays yours, with no agent."),
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
                    || usage_limited(s)
                    || self.spawn_now.contains(&t.id)
                    || gotime::at_least(now, &s.at, retry_after(s))
                    || gotime::before(&s.at, &t.updated_at))
        })
    }

    /// Whether card t is this device's person's to start agents for: every
    /// card on the private log; on a shared board, one they made. A
    /// teammate's card waits on the teammate's computers, not theirs.
    pub(crate) fn persons_card(&self, t: &Task) -> bool {
        if !self.on_board {
            return true;
        }
        let Some(kp) = &self.key_person else {
            return false;
        };
        let me = kp(&self.person);
        !me.is_empty() && kp(&t.created_by) == me
    }

    /// Whether card t waits for an agent Irori starts for it, wherever it
    /// would start (card #541): nobody has it, none is starting, and it's
    /// no agent's turn to end. Unlike [`Self::ask_needs_agent`] it doesn't
    /// ask who owns an ask or whether the hand-out ran, so the device the
    /// person posted on can tell them. A request past its grace isn't
    /// starting: the device it named went away.
    pub(crate) fn waits_for_agent(&self, t: &Task) -> bool {
        self.spawning_on() && self.would_wait_for_agent(t)
    }

    /// [`Self::waits_for_agent`] without this device's own spawning switch:
    /// another computer goes by its own (card #995).
    pub(crate) fn would_wait_for_agent(&self, t: &Task) -> bool {
        let now = self.stamp();
        if !self.persons_card(t)
            || t.spawn_request()
                .is_some_and(|s| s.pending() && !s.stale(&now))
            || t.spawn_crashes >= self.crash_limit()
            || self.ask_resting(t)
            || self.waits_for_lead(t)
        {
            return false;
        }
        match t.open_ask() {
            Some(a) => {
                (!self.on_board || a.thread)
                    && !is_terminal(&t.state)
                    && !t.withdrawn
                    && !t.shared
                    && a.taker.is_empty()
                    && a.reason != super::threads::TAKE_ANSWERED
                    && (a.thread || a.mentions.is_empty())
                    && (!a.thread || a.wake)
                    && !self.agent_thread_rests(t)
            }
            None => super::offers::needs_agent(t) || self.needs_lead(t),
        }
    }

    /// The cards this device hands out that need an agent and have none
    /// starting, oldest first; none while the board doesn't spawn.
    /// See [`SpawnHold`]. Sides are judged on the device that hands the
    /// card out, as its sweep will, whichever device asks.
    pub(crate) fn spawn_hold(&self, t: &Task) -> SpawnHold {
        if is_terminal(&t.state) || t.withdrawn {
            return SpawnHold::NoNeed;
        }
        let unheld = t.holders.is_empty() && t.assigned.is_empty();
        if unheld && t.subtasks.is_empty() && t.spawn_crashes >= self.crash_limit() {
            return SpawnHold::Crashed;
        }
        if self.ask_resting(t) {
            return SpawnHold::Resting;
        }
        let wanted = super::offers::needs_agent(t) || self.needs_lead(t) || self.ask_needs_agent(t);
        if !wanted {
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
        let origin = self.card_origin(t);
        let asked_there = origin == self.dev && self.spawn_now.contains(&t.id);
        if !origin.is_empty() && !self.sides.runs(&origin, &t.side) && !asked_there {
            return SpawnHold::OffSide(format!(
                "no device here runs {}",
                crate::sides::clean(&t.side)
            ));
        }
        if self.waits_for_lead(t) {
            return SpawnHold::ForLead;
        }
        if self.marked_waits(t) {
            return SpawnHold::NeedsOk;
        }
        SpawnHold::Clear
    }

    /// The device whose sweep starts card t's agent: an open ask's owner,
    /// else the device that hands it out. `""` when none does.
    pub(crate) fn card_origin(&self, t: &Task) -> String {
        if let Some(a) = t.open_ask() {
            return a.owner.clone();
        }
        if self.on_board {
            return self.origin.get(&t.id).cloned().unwrap_or_default();
        }
        owner::device_of(&t.created_by).to_string()
    }

    pub(crate) fn cards_to_spawn(&self) -> Vec<Task> {
        self.spawn_candidates(false)
    }

    /// [`Fold::cards_to_spawn`], or (`marked`) the cards it leaves out
    /// because they wait on the person's OK (card #1075).
    fn spawn_candidates(&self, marked: bool) -> Vec<Task> {
        if !self.spawning_on() {
            return Vec::new();
        }
        let now = self.stamp();
        let mut v: Vec<Task> = self
            .tasks
            .values()
            .filter(|t| {
                ((super::offers::needs_agent(t) || self.needs_lead(t)) && self.spawns_out(t))
                    || self.ask_needs_agent(t)
            })
            .filter(|t| self.marked_waits(t) == marked)
            // Only on a device that runs the card's side (card #495),
            // unless the person asked for an agent on it here.
            .filter(|t| self.runs_here(t) || self.spawn_now.contains(&t.id))
            // A card on a closed board starts no agent (card #690): an
            // older device can still put one there.
            .filter(|t| !self.board_closed(&t.board))
            .filter(|t| t.spawn_crashes < self.crash_limit())
            .filter(|t| !self.ask_resting(t))
            .filter(|t| !self.waits_for_lead(t))
            .filter(|t| self.spawn_due(t, &now))
            .cloned()
            .collect();
        v.sort_by(|a, b| {
            self.queue_class(a)
                .cmp(&self.queue_class(b))
                .then_with(|| gotime::cmp(&a.created_at, &b.created_at))
                .then_with(|| a.id.cmp(&b.id))
        });
        v
    }

    /// Where card t goes in the spawn queue (§6, card #450): first what
    /// the person asked for (a card or ask they made, which New card on a
    /// spawning board gives "a fresh agent", or their "Start an agent"),
    /// then a lead's helper parts, then the rest (agents' cards) by age.
    /// A start an agent asked for counts toward the person's limit and
    /// never goes first, even on their own card (card #575).
    pub(crate) fn queue_class(&self, t: &Task) -> u8 {
        if self.spawn_now.contains(&t.id)
            || (t.parent.is_empty() && self.is_owner(&t.created_by) && !self.in_chain(t))
            // An agent's thread its person wrote in is theirs (card #631).
            || (t.ask.as_ref().is_some_and(|a| a.thread && !a.opener.is_empty())
                && !self.agent_thread_rests(t))
        {
            0
        } else if t.spawn && !t.parent.is_empty() {
            1
        } else {
            2
        }
    }

    /// The spawned_for marks written at or after `since` (card #450:
    /// today's count), with their agents' keys.
    pub(crate) fn spawned_since(&self, since: &GoTime) -> Vec<(String, Spawned)> {
        let mut v: Vec<(String, Spawned)> = self
            .spawned
            .iter()
            .filter(|(_, s)| !gotime::before(&s.at, since))
            .map(|(k, s)| (k.clone(), s.clone()))
            .collect();
        v.sort_by(|a, b| gotime::cmp(&a.1.at, &b.1.at).then_with(|| a.0.cmp(&b.0)));
        v
    }
}

/// Whether two spawn rows' devices are the same one; a row from before
/// devices were named matches any.
fn same_device(a: &str, b: &str) -> bool {
    a.is_empty() || b.is_empty() || a == b
}

impl Fold {
    /// How many of a card's agents may crash in a row before Irori stops
    /// and asks the person (card #576): the board's setting, else
    /// [`MAX_SPAWN_CRASHES`].
    pub(crate) fn crash_limit(&self) -> u32 {
        self.crash_limit
            .as_ref()
            .map(|f| f())
            .filter(|n| *n >= 1)
            .map_or(MAX_SPAWN_CRASHES, |n| n as u32)
    }
}

/// How the reason on a request that a Claude usage limit stopped starts
/// (card #450): it isn't a failure, so it waits for the limit's reset (the
/// Core's hold), not [`SPAWN_RETRY`], and the card reads "queued: usage
/// limit" rather than "can't start".
pub const USAGE_LIMIT: &str = "usage limit";

/// Whether request s ended on a Claude usage limit.
pub fn usage_limited(s: &Spawn) -> bool {
    s.op == SPAWN_FAILED && s.reason.starts_with(USAGE_LIMIT)
}

/// The `agent.gone` reason Irori writes for an agent it ended on a usage
/// limit (card #450): ended by Irori, so no crash, and an ask it served
/// starts again after the reset instead of waiting for the person.
pub const GONE_USAGE_LIMIT: &str = "usage limit";
