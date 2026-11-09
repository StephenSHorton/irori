//! Rest instead of end (agent-continuity §6, §9, A.4; card #1215).
//!
//! Where Irori used to end a spawned agent whose work settled, it now
//! lets it rest: the agent writes its hand-off, lets go of its cards,
//! takes and leases, `agent.rest` is written and its process stops with
//! `claude stop` alone, so its job record and session stay. Its name stays
//! too, and every device's roster shows it `resting`. It retires for good
//! (`agent.retire`, then today's `agent.gone` "finished", stop and `rm`)
//! past the context budget or compacted, after a quiet day, when more
//! than [`MAX_RESTING`] rest here, on End or Stop, when a "Not done" lifts
//! its card, and, until agents can be woken (card #1216), whenever
//! something comes for it (§7's fallback, [`coord::RESTING_WAKES`]).
//!
//! One map holds the resting agents (`SpawnState::resting`): the local
//! roster lists them from it, coord reads their rest times through its
//! `resting` option, and it is rebuilt from this device's own rows at
//! start with the roster's own fold ([`coord::roster::resting_of`]).
//! Every change to it happens under `SpawnState::rest_lock`, whichever
//! thread makes it (the sweep, the presence recorder, the person's End).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use coord::roster::{
    AgentCard, REST_DROPPED, REST_IDLE, REST_SLOT, RESTING, RETIRE_BUDGET, RETIRE_COMPACTED,
    RETIRE_CROWDED, RETIRE_ENDED, RETIRE_QUIET, RETIRE_WOKEN, RestRow, RetireRow,
};
use coord::{Notice, Task};
use gocompat::time::GoTime;
use store::kinds;

use super::Services;
use super::spawn::{PROPOSE_NOTE, at_least, before, session_of};
use super::spawn_pause::by_wind_down;
use crate::logf;

/// Irori's own actor on the rows and comments it writes.
const SWEEPER: &str = "irori";

/// `life` counts an agent's processes: 1 at spawn, one more with each wake
/// (card #1216). Until agents can be woken every agent is on its first.
pub const FIRST_LIFE: i64 = 1;

/// A resting agent nothing came for in this long retires (§9, "quiet").
pub const QUIET_FOR: Duration = Duration::from_secs(24 * 3600);

/// At most this many agents rest on a device: when one more rests, the one
/// quiet longest retires (§9, "crowded"). Fixed, not a setting.
pub const MAX_RESTING: usize = 20;

/// The resting agents of this device, by key: each one's last card, with
/// status [`RESTING`] and `status_since` when it rested.
pub(crate) type RestingMap = Arc<RwLock<HashMap<String, AgentCard>>>;

/// What an agent in `SpawnState::ending` is being stopped for. A key with
/// none is ended as before (a shared board's agent, or a spawner that
/// can't stop a job without removing it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ending {
    Rest(&'static str),
    Retire(&'static str),
}

/// The closing comment's shape (§12), asked for at every rest.
pub const HANDOFF_SHAPE: &str = "Done: <what is finished, with the commit or PR>\nLearned: <what the next agent needs to know, or \"nothing new\">\nOpen: <what is left, or \"nothing\">";

/// What Irori writes on a resting agent's items when it rested without a
/// hand-off: its rest wait ran out, or Claude Code dropped its session.
pub const RESTED_NO_HANDOFF: &str = "The agent Irori started for this rested without a hand-off: read the thread and Where for what it did.";

/// The same, for a session Claude Code dropped while it was idle (card
/// #498: about an hour after it went idle).
pub const DROPPED_NO_HANDOFF: &str = "The agent Irori started for this rested without a hand-off: Claude Code stopped its session while it was idle. Read the thread and Where for what it did.";

/// The notice kind of the rest's hand-off ask.
pub const REST_NOTICE: &str = "spawn.rest";

/// The resting agents to retire as crowded: while more than `max` rest,
/// the one quiet longest (oldest `at`, then the lower key) goes.
pub(crate) fn crowded(resting: &[(String, GoTime)], max: usize) -> Vec<String> {
    if resting.len() <= max {
        return Vec::new();
    }
    let mut v: Vec<&(String, GoTime)> = resting.iter().collect();
    v.sort_by(|a, b| {
        a.1.unix_nano()
            .cmp(&b.1.unix_nano())
            .then_with(|| a.0.cmp(&b.0))
    });
    v.into_iter()
        .take(resting.len() - max)
        .map(|(k, _)| k.clone())
        .collect()
}

/// The card a resting agent shows, from its live one as it rests.
pub(crate) fn resting_card(mut c: AgentCard, at: GoTime) -> AgentCard {
    c.status = RESTING.into();
    c.status_since = at;
    c.updated_at = at;
    c.pid = 0;
    c.last_text.clear();
    c.last_tool.clear();
    c.waiting_for.clear();
    c
}

impl Services {
    /// Whether spawned agent `key`'s current life rests.
    pub(crate) fn resting(&self, key: &str) -> bool {
        self.spawn
            .resting
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(key)
    }

    /// The resting agents, with when each rested.
    pub(crate) fn resting_keys(&self) -> Vec<(String, GoTime)> {
        let mut v: Vec<(String, GoTime)> = self
            .spawn
            .resting
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(k, c)| (k.clone(), c.status_since))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// Whether the installed spawner can stop a job and keep it. Without
    /// one an agent can't rest: it ends as before.
    pub(crate) fn can_rest(&self) -> bool {
        self.spawn
            .spawner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|s| s.stop.is_some())
    }

    /// The person's Stop holds this device (Stop here, or Stop everywhere):
    /// its agents retire, never rest (decision 60). An agent's own
    /// `agents_pause` only holds starts.
    pub(crate) fn stop_holds(&self) -> bool {
        let p = self.pause_holding(&self.device.id);
        p.on && (p.by.starts_with("user:") || by_wind_down(&p.by))
    }

    /// Why agent `a` retires rather than rests at an item boundary (§5):
    /// past the person's budget, compacted, or reuse turned off ("Always
    /// start a fresh agent"). A cold cache is no reason.
    pub(crate) fn retire_reason(&self, a: &presence::AgentCard) -> Option<&'static str> {
        let budget = self.spawn_setting("").ctx_budget();
        if a.compacted {
            return Some(RETIRE_COMPACTED);
        }
        if budget < 0 {
            return Some(RETIRE_BUDGET);
        }
        let budget = if budget == 0 {
            coord::DEFAULT_CTX_BUDGET
        } else {
            budget as u64
        };
        (a.ctx > budget).then_some(RETIRE_BUDGET)
    }

    /// An agent whose item settled (§6's table): it retires when the
    /// person's Stop holds it or it is past its budget, else it rests.
    pub(crate) fn settle_agent(&self, a: &presence::AgentCard) {
        if self.stop_holds() || self.winding_down(&a.key) {
            self.begin_retire(&a.key, RETIRE_ENDED);
        } else if let Some(r) = self.retire_reason(a) {
            self.begin_retire(&a.key, r);
        } else {
            self.begin_rest(&a.key, REST_IDLE);
        }
    }

    fn set_ending(&self, key: &str, kind: Ending) -> bool {
        let now = (self.now)();
        let mut m = self.spawn.ending.lock().unwrap_or_else(|e| e.into_inner());
        if m.contains_key(key) {
            return false;
        }
        // A retire asked a moment ago (End, a lift) wins over a rest.
        if matches!(self.ending_kind(key), Some(Ending::Retire(_))) {
            return false;
        }
        m.insert(key.to_string(), now);
        self.spawn
            .end_kind
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_string(), kind);
        true
    }

    /// What `key` is being stopped for, if anything.
    pub(crate) fn ending_kind(&self, key: &str) -> Option<Ending> {
        self.spawn
            .end_kind
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .copied()
    }

    /// Asks awake agent `key` for its hand-off before it rests (§6): every
    /// rest pays this one turn, since a resting agent may never wake. It
    /// rests once the hand-off is on its items, or after `HANDOFF_WAIT`.
    /// With no spawner that can stop and keep a job it ends as before.
    pub(crate) fn begin_rest(&self, key: &str, reason: &'static str) {
        if !self.can_rest() {
            self.begin_end(key);
            return;
        }
        if self.resting(key) || !self.set_ending(key, Ending::Rest(reason)) {
            return;
        }
        let Some(m) = self.coord.spawned(key) else {
            return;
        };
        let t = self.spawn_card_of(key).map(|(_, t, _)| t);
        let short = t.as_ref().map(|t| t.short_ref()).unwrap_or_default();
        let held = t.as_ref().is_some_and(|t| t.parked().is_some());
        let keep = "you keep your name while you rest, and Irori lets go of what you hold";
        let text = match t.as_ref().and_then(|t| t.ask.as_ref()) {
            None if held => format!(
                "Irori is letting you rest: {short} was put aside as Not yet. It is not done: it comes back later, and an agent picks it up then. Stop work on it now. Before you rest, leave where you got to on {short}: Where (commit and push), Left off (board_step left_off) and a closing comment (board_comment) in this shape:\n{HANDOFF_SHAPE}\nDon't set it completed. {PROPOSE_NOTE} Then stop: {keep}."
            ),
            Some(a) if a.thread => format!(
                "Irori is letting you rest: the thread {short} is resolved or has gone quiet. Say in the thread what the person should still know (board_comment {short}), in this shape:\n{HANDOFF_SHAPE}\n{PROPOSE_NOTE} Then stop: {keep}."
            ),
            Some(a)
                if a.outcome
                    .as_ref()
                    .is_none_or(|o| o.kind != coord::OUTCOME_CARD) =>
            {
                format!(
                    "Irori is letting you rest: your part in the ask {short} is done (it was closed, went to another agent, or has gone quiet). Leave a closing comment on {short} (board_comment) with where it stands, in this shape:\n{HANDOFF_SHAPE}\n{PROPOSE_NOTE} Then stop: {keep}."
                )
            }
            _ => format!(
                "Irori is letting you rest: your work on {short} is done for now. Before you rest, put your hand-off on {short}: Where (commit and push), Left off (board_step left_off) and a closing comment (board_comment) in this shape:\n{HANDOFF_SHAPE}\n{PROPOSE_NOTE} Then stop: {keep}."
            ),
        };
        let n = Notice {
            kind: REST_NOTICE.into(),
            task_id: m.current().to_string(),
            reference: format!("task:{}", m.current()),
            by: SWEEPER.into(),
            to: vec![key.to_string()],
            wake: true,
            text,
            ..Notice::default()
        };
        if let Some(e) = crate::notice::notice_err(&self.route_notice(&inbox::system_sender(), &n))
        {
            logf(format_args!("spawn rest {key}: {e}"));
        }
    }

    /// Asks awake agent `key` for its hand-off before it retires (§9):
    /// today's end, with `agent.retire` written before its `agent.gone`.
    pub(crate) fn begin_retire(&self, key: &str, reason: &'static str) {
        if self.resting(key) {
            if let Err(e) = self.retire_resting(key, reason) {
                logf(format_args!("spawn retire {key}: {e}"));
            }
            return;
        }
        {
            // Under `ending`'s lock, as set_ending is. A rest still waiting
            // for its hand-off becomes this retire (the person's End or a
            // lift during it): its wait goes on, and it retires instead.
            let _ending = self.spawn.ending.lock().unwrap_or_else(|e| e.into_inner());
            let mut kinds = self
                .spawn
                .end_kind
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if !matches!(kinds.get(key), Some(Ending::Retire(_))) {
                kinds.insert(key.to_string(), Ending::Retire(reason));
            }
        }
        self.begin_end(key);
    }

    /// The hand-off the rest asked for is on its items: a comment by it, or
    /// a done-check or part proof it raised, since the rest began.
    pub(crate) fn rested_handoff(&self, key: &str, since: &GoTime) -> bool {
        let Some(m) = self.coord.spawned(key) else {
            return false;
        };
        let Some((co, t, key)) = self.spawn_card_of(key) else {
            return false;
        };
        let since = if before(since, &m.current_since()) {
            m.current_since()
        } else {
            *since
        };
        if t.blockers.iter().any(|b| {
            (b.done_check || b.question == coord::PART_QUESTION)
                && b.raised_by == key
                && !before(&b.at, &since)
        }) {
            return true;
        }
        co.comments(m.current())
            .unwrap_or_default()
            .iter()
            .any(|c| c.by == key && !before(&c.at, &since))
    }

    /// The rest itself (§6, steps 2 and 3), once per life: the "rested
    /// without a hand-off" line when it gave none, everything it holds let
    /// go, `agent.rest`, then `claude stop` alone. Rows first, so the
    /// presence drop that follows the stop is never recorded as a crash.
    pub(crate) fn rest_now(
        &self,
        key: &str,
        given: bool,
        reason: &'static str,
    ) -> Result<(), String> {
        let _g = self
            .spawn
            .rest_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.resting(key) || self.coord.ended_by_irori(key) {
            return Ok(());
        }
        // Turned into a retire after the sweep read it.
        if let Some(Ending::Retire(r)) = self.ending_kind(key) {
            drop(_g);
            return self.retire_now(key, given, r);
        }
        let live = self.presence.snapshot().into_iter().find(|a| a.key == key);
        self.rest_rows(key, live.map(super::roster_card), reason, given)?;
        self.close_rest_notices(key);
        self.stop_job(key);
        Ok(())
    }

    /// The rows of a rest, under `rest_lock`: the line when it gave no
    /// hand-off, the let-go, the resting map, then `agent.rest`.
    fn rest_rows(
        &self,
        key: &str,
        card: Option<AgentCard>,
        reason: &'static str,
        given: bool,
    ) -> Result<(), String> {
        let now = (self.now)();
        let current = self.coord.spawned(key).map(|m| m.current().to_string());
        let left = self.coord.rest_leave(key).map_err(|e| e.to_string())?;
        if !given {
            let line = if reason == REST_DROPPED {
                DROPPED_NO_HANDOFF
            } else {
                RESTED_NO_HANDOFF
            };
            let mut on: Vec<String> = current.into_iter().collect();
            for id in left {
                if !on.contains(&id) {
                    on.push(id);
                }
            }
            for id in on {
                if self.coord.get_task(&id).is_some()
                    && let Err(e) = self.coord.add_comment(SWEEPER, &id, line, false)
                {
                    logf(format_args!("spawn rest {key}: {e}"));
                }
            }
        }
        let mut card = card.unwrap_or_else(|| AgentCard {
            key: key.to_string(),
            device_id: self.device.id.clone(),
            agent: "claude".into(),
            session_id: session_of(key).to_string(),
            name: self.agent_names.get(key),
            name_source: "user".into(),
            ..AgentCard::default()
        });
        if card.name.is_empty() {
            card.name = self.agent_names.get(key);
        }
        let card = resting_card(card, now);
        let row = RestRow {
            key: key.to_string(),
            life: FIRST_LIFE,
            at: now,
            reason: reason.into(),
            ctx: card.ctx,
            ctx_at: card.ctx_at,
            compacted: card.compacted,
            name: card.name.clone(),
            by: SWEEPER.into(),
        };
        // In the map before the row: a reader that sees `agent.rest` (or the
        // presence drop after it) already finds the agent resting.
        self.spawn
            .resting
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_string(), card);
        if let Err(e) = self.store.append("irori", kinds::AGENT_REST, &row) {
            self.spawn
                .resting
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .remove(key);
            return Err(e.to_string());
        }
        self.coord.mark_rested(key, now);
        self.hub.publish(crate::events::CoreEvent::Roster);
        Ok(())
    }

    /// Closes the rest's own hand-off ask and any end ask still open to
    /// `key`: a resting agent never reads them, and they must not count as
    /// something that came for it.
    fn close_rest_notices(&self, key: &str) {
        for d in self.inbox.for_agent(key, false, usize::MAX) {
            let r = d.message.reason.as_str();
            if (r == REST_NOTICE || r == "spawn.end" || inbox::is_wind_down(&d.message))
                && let Err(e) = self.inbox.skip(key, &d.id, inbox::NOTE_RECIPIENT_GONE)
            {
                logf(format_args!("spawn rest {key}: close {r}: {e}"));
            }
        }
    }

    /// `claude stop` for a resting agent, never `rm`: the job record keeps
    /// the flags a wake applies again. `claude stop` can return before the
    /// process exits, so the agent is checked on later sweeps until Claude
    /// Code no longer lists it live.
    pub(crate) fn stop_job(&self, key: &str) {
        let stop = self
            .spawn
            .spawner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|s| s.stop.clone());
        let Some(stop) = stop else {
            return;
        };
        if let Err(e) = stop(session_of(key)) {
            logf(format_args!("spawn rest {key}: claude stop: {e}"));
        }
        self.spawn
            .stopping
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_string(), (self.now)());
    }

    /// Resting agents whose stop isn't confirmed yet: one Claude Code (or
    /// presence, with no lister) still lists live is stopped again, after
    /// `END_RETRY`; one it doesn't is done.
    fn confirm_stops(&self) {
        let keys: Vec<(String, GoTime)> = self
            .spawn
            .stopping
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(k, at)| (k.clone(), *at))
            .collect();
        if keys.is_empty() {
            return;
        }
        let list = self
            .spawn
            .spawner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|s| s.list.clone());
        let listed = list.map(|l| l());
        let now = (self.now)();
        for (k, at) in keys {
            if !self.resting(&k) {
                self.spawn
                    .stopping
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&k);
                continue;
            }
            let sess = session_of(&k);
            let live = match &listed {
                Some(Ok(agents)) => agents.iter().any(|a| a.session_id == sess && a.live),
                // Can't tell yet: ask again next time.
                Some(Err(_)) => continue,
                None => self.presence.snapshot().iter().any(|a| a.key == k),
            };
            if !live {
                self.spawn
                    .stopping
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&k);
                continue;
            }
            if at_least(&now, &at, super::spawn::END_RETRY) {
                self.stop_job(&k);
            }
        }
    }

    /// Retires awake agent `key` once its hand-off is in (or the wait ran
    /// out): `agent.retire`, then today's end.
    pub(crate) fn retire_now(
        &self,
        key: &str,
        given: bool,
        reason: &'static str,
    ) -> Result<(), String> {
        // The row and the end under one lock, so two sweeps never write
        // either twice.
        let _g = self
            .spawn
            .rest_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !self.coord.ended_by_irori(key) {
            self.retire_row(key, reason)?;
        }
        self.end_now(key, given)
    }

    fn retire_row(&self, key: &str, reason: &'static str) -> Result<(), String> {
        let row = RetireRow {
            key: key.to_string(),
            life: FIRST_LIFE,
            at: (self.now)(),
            reason: reason.into(),
            by: SWEEPER.into(),
        };
        self.store
            .append("irori", kinds::AGENT_RETIRE, &row)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Retires resting agent `key` straight away (§6: its hand-off was
    /// written when it rested): `agent.retire`, `agent.gone` "finished",
    /// then `claude rm` (its process is already stopped). No notice: one
    /// would only be something that came for it.
    pub(crate) fn retire_resting(&self, key: &str, reason: &'static str) -> Result<(), String> {
        {
            let _g = self
                .spawn
                .rest_lock
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if !self.resting(key) {
                return Ok(());
            }
            if !self.coord.ended_by_irori(key) {
                self.retire_row(key, reason)?;
            }
            // Its `agent.gone` (in end_now) is written before it leaves the
            // map, so no reader finds it neither resting nor ended.
            let r = self.end_now(key, true);
            self.spawn
                .resting
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .remove(key);
            self.spawn
                .stopping
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(key);
            self.hub.publish(crate::events::CoreEvent::Roster);
            // Notices nobody will read again; plain messages take the gone
            // path (onto its card, or to the person) as for any agent that
            // finished.
            for d in self.inbox.for_agent(key, false, usize::MAX) {
                if d.message.notice
                    && let Err(e) = self.inbox.skip(key, &d.id, inbox::NOTE_RECIPIENT_GONE)
                {
                    logf(format_args!("spawn retire {key}: {e}"));
                }
            }
            if let Err(e) = r {
                logf(format_args!("spawn retire {key}: {e}"));
            }
        }
        self.spawn
            .removed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_string());
        Ok(())
    }

    /// The pass over the resting set (§9), in each sweep: the let-go of
    /// agents Claude Code dropped, stops not confirmed yet, and the retires
    /// presence can't see (a resting agent has no session): Stop, what
    /// came for it (§7's fallback), a quiet day, too many resting.
    pub(crate) fn sweep_resting(&self) {
        self.reconcile_resting();
        self.let_go_dropped();
        self.confirm_stops();
        let keys = self.resting_keys();
        if keys.is_empty() {
            return;
        }
        if self.stop_holds() {
            for (k, _) in &keys {
                if let Err(e) = self.retire_resting(k, RETIRE_ENDED) {
                    logf(format_args!("spawn retire {k}: {e}"));
                }
            }
            return;
        }
        let now = (self.now)();
        let mut left: Vec<(String, GoTime)> = Vec::new();
        for (k, at) in keys {
            if !coord::RESTING_WAKES && self.came_for(&k) {
                if let Err(e) = self.retire_resting(&k, RETIRE_WOKEN) {
                    logf(format_args!("spawn retire {k}: {e}"));
                }
                continue;
            }
            let last = self.last_for(&k, at);
            if at_least(&now, &last, QUIET_FOR) {
                if let Err(e) = self.retire_resting(&k, RETIRE_QUIET) {
                    logf(format_args!("spawn retire {k}: {e}"));
                }
                continue;
            }
            left.push((k, last));
        }
        for k in crowded(&left, MAX_RESTING) {
            if let Err(e) = self.retire_resting(&k, RETIRE_CROWDED) {
                logf(format_args!("spawn retire {k}: {e}"));
            }
        }
    }

    /// Something that would wake resting agent `key` is waiting for it
    /// (§7): an open delivery that wakes, from the person, from Irori about
    /// its items, or from one of the person's own agents while "My agents
    /// may start agents" is on. Never another member's text, which waits
    /// for its next turn as on any agent.
    fn came_for(&self, key: &str) -> bool {
        self.inbox
            .for_agent(key, false, usize::MAX)
            .iter()
            .any(|d| {
                if d.mode == inbox::mode::NEXT_TURN || !d.hold.is_empty() {
                    return false;
                }
                let r = d.message.reason.as_str();
                if r == REST_NOTICE || r == "spawn.end" {
                    return false;
                }
                let from = &d.message.from;
                match from.kind.as_str() {
                    "user" | "irori" => true,
                    "claude" => self.own_agent(&from.key) && self.agent_starts().on,
                    _ => false,
                }
            })
    }

    /// When anything was last for resting agent `key`: its rest, the newest
    /// message to it, or the newest comment on an item it served.
    fn last_for(&self, key: &str, rested: GoTime) -> GoTime {
        let mut last = rested;
        let mut later = |at: GoTime| {
            if before(&last, &at) {
                last = at;
            }
        };
        if let Some(d) = self.inbox.for_agent(key, true, 1).first() {
            later(d.created_at);
        }
        if let Some(m) = self.coord.spawned(key) {
            let mut ids: Vec<String> = vec![m.task_id.clone()];
            ids.extend(m.items.iter().map(|i| i.task_id.clone()));
            ids.sort();
            ids.dedup();
            for id in ids {
                if let Some(c) = self
                    .coord
                    .comments(&id)
                    .unwrap_or_default()
                    .iter()
                    .max_by_key(|c| c.at.unix_nano())
                {
                    later(c.at);
                }
            }
        }
        last
    }

    /// Once per run: everything a resting agent still holds is let go. A
    /// dropped rest queues its let-go in memory, and Irori may quit before
    /// the sweep writes it; nothing else takes cards from a resting agent,
    /// since it is never gone. A no-op for one that let go already.
    fn reconcile_resting(&self) {
        use std::sync::atomic::Ordering;
        if self.spawn.rest_reconciled.swap(true, Ordering::SeqCst) {
            return;
        }
        for (k, _) in self.resting_keys() {
            let _g = self
                .spawn
                .rest_lock
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Err(e) = self.coord.rest_leave(&k) {
                logf(format_args!("spawn rest {k}: let go after a start: {e}"));
                self.spawn.rest_reconciled.store(false, Ordering::SeqCst);
            }
        }
    }

    /// Whether Removed key `key` is a rest, decided on the presence thread
    /// (§6's last row): nothing for a life that already rests or is
    /// resting now; a rest when Claude Code dropped an idle spawned agent
    /// of this device nothing waited on; `false` (a crash, as before) for
    /// anything else. A dropped rest writes its `agent.rest` here; its
    /// let-go waits for the next sweep, which owns the coord's writes.
    pub(crate) fn removed_rests(&self, key: &str, was: Option<&presence::AgentCard>) -> bool {
        let _g = self
            .spawn
            .rest_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.resting(key) {
            return true;
        }
        if matches!(self.ending_kind(key), Some(Ending::Rest(_))) {
            // Mid-rest: the rest finishes on the sweep and writes its rows.
            return true;
        }
        if !self.dropped_rests(key, was) {
            return false;
        }
        let card = was.cloned().map(super::roster_card);
        if let Err(e) = self.rest_rows_dropped(key, card) {
            logf(format_args!("spawn rest {key}: {e}"));
            return false;
        }
        true
    }

    /// Whether a drop of `key` rests: a spawned agent of this device's own
    /// log, not being ended, that was idle and that nothing waits on (no
    /// submitted or working card of its, no take). An agent whose prompt
    /// never got in (card #677) still holds its card assigned and
    /// submitted, or its ask's take, so it stays a crash.
    fn dropped_rests(&self, key: &str, was: Option<&presence::AgentCard>) -> bool {
        let Some(was) = was else {
            return false;
        };
        if was.status != "idle" {
            return false;
        }
        if irori_identity::owner::device_of(key) != self.device.id {
            return false;
        }
        let Some(m) = self.coord.spawned(key) else {
            return false;
        };
        if !m.board.is_empty()
            || self.coord.ended_by_irori(key)
            || self
                .spawn
                .ending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(key)
            || self
                .spawn
                .removed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(key)
        {
            return false;
        }
        !self.open_cards().iter().any(|t| waits_on(t, key))
    }

    /// The rows of a dropped rest (the presence thread): no hand-off, the
    /// map and `agent.rest`; the let-go is queued for the sweep.
    fn rest_rows_dropped(&self, key: &str, card: Option<AgentCard>) -> Result<(), String> {
        let now = (self.now)();
        let mut card = card.unwrap_or_default();
        if card.key.is_empty() {
            card.key = key.to_string();
            card.device_id = self.device.id.clone();
            card.agent = "claude".into();
            card.session_id = session_of(key).to_string();
        }
        if card.name.is_empty() {
            card.name = self.agent_names.get(key);
        }
        let card = resting_card(card, now);
        let row = RestRow {
            key: key.to_string(),
            life: FIRST_LIFE,
            at: now,
            reason: REST_DROPPED.into(),
            ctx: card.ctx,
            ctx_at: card.ctx_at,
            compacted: card.compacted,
            name: card.name.clone(),
            by: SWEEPER.into(),
        };
        self.spawn
            .resting
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_string(), card);
        if let Err(e) = self.store.append("irori", kinds::AGENT_REST, &row) {
            self.spawn
                .resting
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .remove(key);
            return Err(e.to_string());
        }
        self.coord.mark_rested(key, now);
        self.spawn
            .dropped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(key.to_string());
        self.hub.publish(crate::events::CoreEvent::Roster);
        Ok(())
    }

    /// The let-go of agents Claude Code dropped while idle (queued by the
    /// presence thread): their cards, takes and leases, and the "rested
    /// without a hand-off" line on their items.
    fn let_go_dropped(&self) {
        let keys: Vec<String> =
            std::mem::take(&mut *self.spawn.dropped.lock().unwrap_or_else(|e| e.into_inner()));
        for k in keys {
            let _g = self
                .spawn
                .rest_lock
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if !self.resting(&k) {
                continue;
            }
            let current = self.coord.spawned(&k).map(|m| m.current().to_string());
            let left = match self.coord.rest_leave(&k) {
                Ok(l) => l,
                Err(e) => {
                    logf(format_args!("spawn rest {k}: let go: {e}"));
                    self.spawn
                        .dropped
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(k.clone());
                    continue;
                }
            };
            let mut on: Vec<String> = current.into_iter().collect();
            for id in left {
                if !on.contains(&id) {
                    on.push(id);
                }
            }
            for id in on {
                if self.coord.get_task(&id).is_some()
                    && let Err(e) = self
                        .coord
                        .add_comment(SWEEPER, &id, DROPPED_NO_HANDOFF, false)
                {
                    logf(format_args!("spawn rest {k}: {e}"));
                }
            }
            self.close_rest_notices(&k);
        }
    }

    /// A session presence lists under a resting key. One that started after
    /// the rest is the person's Revive or a `claude --resume` by hand: its
    /// process is back, so it no longer rests (the `agent.seen` its Added
    /// writes says so on every device), and this returns true. One that
    /// started before is the rested life's own process, still exiting (a
    /// restart came before its stop took): it is stopped again and stays
    /// resting, and this returns false.
    pub(crate) fn rest_ended_by_session(&self, card: &presence::AgentCard, stop_old: bool) -> bool {
        let _g = self
            .spawn
            .rest_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let rested = self
            .spawn
            .resting
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&card.key)
            .map(|c| c.status_since);
        let Some(at) = rested else {
            return true;
        };
        if card.started_at.is_zero() || card.started_at.unix_nano() <= at.unix_nano() {
            drop(_g);
            if stop_old {
                self.stop_job(&card.key);
            }
            return false;
        }
        self.spawn
            .resting
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&card.key);
        self.hub.publish(crate::events::CoreEvent::Roster);
        true
    }

    /// §7's fallback, while resting agents can't be woken: a fresh agent
    /// started for card t, which a resting agent served (it is that agent's
    /// item, made from its thread, or the card it let go as it rested), so
    /// the resting one retires: the successor has its work now.
    pub(crate) fn retire_for_start(&self, t: &Task) {
        if coord::RESTING_WAKES {
            return;
        }
        let rested_by = t.rested.as_ref().map(|r| r.was.clone()).unwrap_or_default();
        for (k, _) in self.resting_keys() {
            let served = k == rested_by
                || self
                    .coord
                    .spawned(&k)
                    .is_some_and(|m| m.serves(&t.id) || (!t.from.is_empty() && m.serves(&t.from)));
            if served && let Err(e) = self.retire_resting(&k, RETIRE_WOKEN) {
                logf(format_args!("spawn retire {k}: {e}"));
            }
        }
    }

    /// A start queued for a slot (§6, "a queued start needs its slot"):
    /// for ask or thread t, whose hand-out ran and found nobody, the idle
    /// spawned agent idle longest that holds nothing rests. Never one that
    /// is a candidate for t or holds an offer, and no more rests in flight
    /// than starts waiting (`waiting`, counted by the caller this sweep).
    /// A card's hand-out can't be told from "never ran" on a board that
    /// starts agents until card #1217, so cards don't take a slot yet.
    pub(crate) fn slot_rest(&self, t: &Task, waiting: usize) {
        if !self.can_rest()
            || !t
                .open_ask()
                .is_some_and(|a| a.taker.is_empty() && a.unclaimed)
        {
            return;
        }
        let in_flight = self
            .spawn
            .end_kind
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|k| **k == Ending::Rest(REST_SLOT))
            .count();
        if in_flight >= waiting {
            return;
        }
        let me = self.device.id.clone();
        let open = self.open_cards();
        let mut idle: Vec<presence::AgentCard> = self
            .presence
            .snapshot()
            .into_iter()
            .filter(|a| {
                a.status == "idle"
                    && irori_identity::owner::device_of(&a.key) == me
                    && !self.resting(&a.key)
                    && !self
                        .spawn
                        .ending
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .contains_key(&a.key)
            })
            .filter(|a| {
                self.coord.spawned(&a.key).is_some_and(|m| {
                    m.board.is_empty()
                        && !m.serves(&t.id)
                        && (t.from.is_empty() || !m.serves(&t.from))
                })
            })
            .filter(|a| !open.iter().any(|c| holds_any(c, &a.key)))
            .collect();
        idle.sort_by(|a, b| {
            a.status_since
                .unix_nano()
                .cmp(&b.status_since.unix_nano())
                .then_with(|| a.key.cmp(&b.key))
        });
        if let Some(a) = idle.first() {
            self.begin_rest(&a.key, REST_SLOT);
        }
    }

    /// The brief and settings files to keep (card #1215): `<card>-<gen>`
    /// of each spawned agent of this device that runs or rests. A plain
    /// wake reads the spawn's settings file again at its path, and fails
    /// before it starts if the file is gone.
    pub(crate) fn kept_brief_stems(&self) -> HashSet<String> {
        let me = self.device.id.clone();
        let mut keys: Vec<String> = self.resting_keys().into_iter().map(|(k, _)| k).collect();
        keys.extend(
            self.presence
                .snapshot()
                .into_iter()
                .filter(|a| a.status != "gone")
                .map(|a| a.key),
        );
        keys.into_iter()
            .filter(|k| irori_identity::owner::device_of(k) == me)
            .filter_map(|k| self.coord.spawned(&k))
            .map(|m| format!("{}-{}", m.task_id, m.generation))
            .collect()
    }
}

/// Whether card t still waits on agent `key`: it holds or was given a card
/// that is submitted (its prompt may never have got in) or working, or it
/// has an ask or thread's take.
fn waits_on(t: &Task, key: &str) -> bool {
    if let Some(a) = t.open_ask() {
        return a.taker == key;
    }
    (t.holders.iter().any(|h| h == key) || t.assigned.iter().any(|h| h == key))
        && (t.state == coord::SUBMITTED || t.state == coord::WORKING)
}

/// Whether `key` holds anything on open card t: it holds it, co-holds it,
/// was given it, took its ask, or was offered it.
fn holds_any(t: &Task, key: &str) -> bool {
    t.holders.iter().any(|h| h == key)
        || t.co_holders.iter().any(|h| h == key)
        || t.assigned.iter().any(|h| h == key)
        || t.ask.as_ref().is_some_and(|a| a.taker == key)
        || t.live_offer().is_some_and(|o| o.to == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: i64) -> GoTime {
        GoTime::unix(s, 0)
    }

    #[test]
    fn crowded_retires_the_quiet_longest_past_the_cap() {
        let v: Vec<(String, GoTime)> = (0..23)
            .map(|i| (format!("dev/k{i:02}"), at(1000 - i)))
            .collect();
        // 23 resting, 20 kept: the three quiet longest (the lowest times).
        assert_eq!(
            crowded(&v, MAX_RESTING),
            vec!["dev/k22", "dev/k21", "dev/k20"]
        );
        assert!(crowded(&v[..20], MAX_RESTING).is_empty());
        // A tie goes by key.
        let tie = vec![
            ("dev/b".to_string(), at(5)),
            ("dev/a".to_string(), at(5)),
            ("dev/c".to_string(), at(9)),
        ];
        assert_eq!(crowded(&tie, 2), vec!["dev/a"]);
    }

    #[test]
    fn a_resting_card_says_so_and_drops_its_live_bits() {
        let c = resting_card(
            AgentCard {
                key: "d/s".into(),
                status: "idle".into(),
                pid: 42,
                last_text: "hi".into(),
                ctx: 1234,
                ..AgentCard::default()
            },
            at(77),
        );
        assert_eq!(c.status, RESTING);
        assert_eq!(c.status_since, at(77));
        assert_eq!(c.pid, 0);
        assert!(c.last_text.is_empty());
        assert_eq!(c.ctx, 1234);
    }
}
