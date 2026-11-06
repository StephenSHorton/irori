//! Agents starting and managing agents (card #575, GitHub #66): the limit
//! behind "My agents may start agents on my cards", and what the
//! `agent_start` and `agent_end` tools may do.
//!
//! The limit is the person's, set in the app. It rides on a `spawn.cap` row
//! under a reserved device name, so it reaches all their devices with no
//! new row kind, and only `actor == "user"` rows count. No tool writes it.
//!
//! One pool counts the whole chain: every live agent an agent asked for,
//! and everything those agents went on to cause, by posting a card or
//! splitting one for helpers (`Fold::in_chain`). A start the person asks
//! for never counts and is never held. Other agents' cards are hand-out as
//! before: the board switch and device caps hold them.
//!
//! An agent's start has less than the person's Start: it never moves a card
//! that waits on someone, never overrides the crash stop, and never gives
//! the yes a card given to the person needs. It leaves `spawn_now`, which
//! crosses side pins and skips the retry wait, to the person.
//!
//! Two computers asking in the same instant can each see the last slot.
//! Every request is checked again in order where it starts, so once the
//! rows meet the later one is refused; with no server, agents started in
//! the gap before they meet can pass the limit by that many until one ends.

use std::collections::HashMap;

use coord::{SPAWN_FAILED, SpawnHold, Task};
use gocompat::time::GoTime;
use irori_identity::owner::device_of;
use store::kinds;

use super::Services;
use super::spawn::SpawnCap;
use super::spawn_why::Why;
use crate::logf;

const SWEEPER: &str = "irori";

/// The reserved `spawn.cap` device the person's limit rides on. Device ids
/// are hex, so it names no device, and an older Irori only stores it.
pub const AGENT_STARTS_ROW: &str = "agents";
/// On for a new person (Stephen, #575 b1).
pub const AGENT_STARTS_ON: bool = true;
/// The maximum for a new person (Stephen, #575 b1).
pub const AGENT_STARTS_MAX: i64 = 100;
/// The most the app accepts.
pub const AGENT_STARTS_CEILING: i64 = 1000;
/// How long an ask this device can't serve itself holds its slot here: the
/// device that serves it counts it from its own request by then.
const REMOTE_ASK_HOLD_MS: i64 = 10 * 60 * 1000;

/// "My agents may start agents on my cards" and its maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentStarts {
    pub on: bool,
    pub max: i64,
}

impl Default for AgentStarts {
    fn default() -> AgentStarts {
        AgentStarts {
            on: AGENT_STARTS_ON,
            max: AGENT_STARTS_MAX,
        }
    }
}

/// An agent's ask for a card, holding its slot until its request is written.
#[derive(Debug, Clone)]
pub(crate) struct AgentAsk {
    /// The card's request generation when it was asked.
    generation: i64,
    at: GoTime,
}

/// What a tool reads of the limit. Using it is all an agent can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentStartsUse {
    pub on: bool,
    pub max: i64,
    pub used: i64,
}

impl Services {
    pub(crate) fn agent_starts(&self) -> AgentStarts {
        *self
            .spawn
            .agent_starts
            .read()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// The person sets the limit, from the app only: no tool reaches this.
    pub(crate) fn set_agent_starts(&self, on: bool, max: i64) -> Result<(), String> {
        if !(0..=AGENT_STARTS_CEILING).contains(&max) {
            return Err(format!(
                "the maximum is a number from 0 to {AGENT_STARTS_CEILING}"
            ));
        }
        let row = SpawnCap {
            device: AGENT_STARTS_ROW.into(),
            cap: max,
            on: Some(on),
            by: format!("user:{}", self.device.id),
            at: (self.now)(),
        };
        let rows = self.spawn.rows.lock().unwrap_or_else(|e| e.into_inner());
        self.store
            .append("user", kinds::SPAWN_CAP, &row)
            .map_err(|e| e.to_string())?;
        *self
            .spawn
            .agent_starts
            .write()
            .unwrap_or_else(|e| e.into_inner()) = AgentStarts { on, max };
        drop(rows);
        self.asks_kick.kick();
        Ok(())
    }

    /// The counted requests still starting, in every log, oldest first.
    fn counted_starting(&self) -> Vec<(GoTime, String)> {
        let mut v = self.coord.counted_pending();
        for b in self.shown_boards() {
            v.extend(b.coord().counted_pending());
        }
        v.sort_by_key(|(at, id)| (at.unix_milli_value(), id.clone()));
        v
    }

    /// Agents counted now, with `asks` held, ahead of card `except`: an ask
    /// waits only on older asks, so a lowered maximum can't leave asks all
    /// waiting on each other with nothing running.
    fn used_with(&self, asks: &HashMap<String, AgentAsk>, except: &str) -> i64 {
        let starting = self.counted_starting();
        let order = |id: &str, a: &AgentAsk| (a.at.unix_milli_value(), id.to_string());
        let mine = asks.get(except).map(|a| order(except, a));
        let waiting = asks
            .iter()
            .filter(|(id, _)| id.as_str() != except && !starting.iter().any(|(_, c)| c == *id))
            .filter(|(id, a)| mine.as_ref().is_none_or(|m| order(id, a) < *m))
            .count();
        let starting = starting.iter().filter(|(_, c)| c != except).count();
        (self.coord.counted_live().len() + starting + waiting) as i64
    }

    fn agent_starts_used(&self, except: &str) -> i64 {
        let asks = self
            .spawn
            .agent_asks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        self.used_with(&asks, except)
    }

    pub(crate) fn agent_starts_use(&self) -> AgentStartsUse {
        let s = self.agent_starts();
        AgentStartsUse {
            on: s.on,
            max: s.max,
            used: self.agent_starts_used(""),
        }
    }

    /// Whether card t's next agent counts toward the limit.
    pub(crate) fn start_counts(&self, co: &coord::Coord, t: &Task) -> bool {
        let person = self
            .spawn
            .person_asks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&t.id);
        // A card given to the person starts only on their own yes.
        if person || co.given_to_me(t) {
            return false;
        }
        let asked = self
            .spawn
            .agent_asks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&t.id);
        // An agent has another key in each shared board's log, so its mark
        // is read from the person's own log, which holds every agent's.
        asked
            || co.in_chain(t)
            || (t.spawn_request().is_none()
                && self
                    .coord
                    .spawned(&self.private_key(&t.created_by))
                    .is_some_and(|m| m.counted))
    }

    /// What the limit says about card t's agent, if it holds it back.
    pub(crate) fn limit_holds(&self, co: &coord::Coord, t: &Task) -> Option<Why> {
        if !self.start_counts(co, t) {
            return None;
        }
        let s = self.agent_starts();
        if !s.on {
            return Some(Why::AgentStartsOff);
        }
        let used = self.agent_starts_used(&t.id);
        (used >= s.max).then_some(Why::AgentStartsFull { used, max: s.max })
    }

    /// Card `id`'s request is written: its ask is in the log now.
    pub(crate) fn settle_ask(&self, id: &str) {
        self.spawn
            .agent_asks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        self.spawn
            .person_asks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    /// The person asked for card `id`'s agent: it doesn't count.
    pub(crate) fn note_person_ask(&self, id: &str) {
        self.spawn
            .agent_asks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        self.spawn
            .person_asks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string());
    }

    fn ask_for(&self, t: &Task) -> AgentAsk {
        AgentAsk {
            generation: t.spawn_request().map_or(0, |s| s.generation),
            at: (self.now)(),
        }
    }

    /// An agent's ask read from its row: another device's, or this one's
    /// after a restart. One already held keeps its place.
    pub(crate) fn note_agent_ask(&self, t: &Task) {
        let ask = self.ask_for(t);
        self.spawn
            .agent_asks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(t.id.clone())
            .or_insert(ask);
    }

    /// Lets go of asks nothing will come of: the card has an agent, closed
    /// or moved on, or another device serves it and has had time to.
    pub(crate) fn prune_agent_asks(&self) {
        let now = (self.now)();
        let mut asks = self
            .spawn
            .agent_asks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        asks.retain(|id, a| {
            let Ok((_, co)) = self.route(id) else {
                return false;
            };
            let Some(t) = co.get_task(id) else {
                return false;
            };
            co.spawn_hold(id) == SpawnHold::Clear
                && t.spawn_request().map_or(0, |s| s.generation) == a.generation
                && (co.hands_out_here(id)
                    || now.unix_milli_value() - a.at.unix_milli_value() < REMOTE_ASK_HOLD_MS)
        });
        drop(asks);
        // A person's ask served some other way must not excuse a later
        // agent's start.
        self.spawn
            .person_asks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|id| {
                self.route(id).is_ok_and(|(_, co)| {
                    co.hands_out_here(id) && co.spawn_hold(id) == SpawnHold::Clear
                })
            });
    }

    /// Checked again where a counted request starts: it goes only if it
    /// fits, oldest request first, so two devices can't both take the last
    /// slot once their rows have met. Ok(true) when it was refused.
    pub(crate) fn refuse_over_limit(
        &self,
        co: &coord::Coord,
        t: &Task,
        generation: i64,
    ) -> Result<bool, String> {
        let counted = co
            .get_task(&t.id)
            .and_then(|c| c.spawn_request().cloned())
            .is_some_and(|s| s.generation == generation && s.counted);
        if !counted {
            return Ok(false);
        }
        let s = self.agent_starts();
        let ahead = self
            .counted_starting()
            .iter()
            .take_while(|(_, c)| c != &t.id)
            .count() as i64;
        let used = self.coord.counted_live().len() as i64 + ahead;
        let why = if !s.on {
            Why::AgentStartsOff
        } else if used >= s.max {
            Why::AgentStartsFull { used, max: s.max }
        } else {
            return Ok(false);
        };
        co.end_spawn(SWEEPER, &t.id, generation, SPAWN_FAILED, &why.to_string())
            .map_err(|e| e.to_string())?;
        Ok(true)
    }

    /// Whether `key` is an agent on one of the person's own devices.
    fn own_agent(&self, key: &str) -> bool {
        let dev = device_of(key);
        !dev.is_empty() && self.chain.is_owner(&format!("user:{dev}"))
    }

    /// Agent `caller` asks for an agent on card `id`: the twin of the
    /// person's Start, for their own cards, inside their limit. `Err` says
    /// why not; `Ok` says what happens next.
    pub(crate) fn agent_start(&self, caller: &str, id: &str) -> Result<String, String> {
        if !self.own_agent(caller) {
            return Err("only the person's own agents start agents here".into());
        }
        let (_, co) = self.route(id).map_err(|e| e.to_string())?;
        let t = co.get_task(id).ok_or_else(|| format!("no card {id}"))?;
        let r = t.short_ref();
        let s = self.agent_starts();
        if !s.on {
            return Err(Why::AgentStartsOff.to_string());
        }
        // A card given to the person waits for their own yes. It's theirs
        // to give, in Needs you, and no agent gives it for them (#572).
        if co.given_to_me(&t) {
            return Err(format!(
                "{r} was given to the person by someone else, so it starts only on their own yes in Needs you; an agent can't say it for them"
            ));
        }
        let why = self.why_no_agent(id, true);
        match &why {
            None if t.holders.is_empty() => return Err(format!("{r} doesn't want an agent now")),
            // Given to another member: theirs to start, on their own yes.
            None if t.given.is_some() => {
                return Err(format!(
                    "{r} was given to another member, so it starts only on their own yes"
                ));
            }
            None if t.holders.iter().all(|h| co.is_any_person(h)) => {
                return Err(format!("{r} is held by a person, not waiting for an agent"));
            }
            None => return Err(format!("{r} has an agent on it")),
            Some(Why::Starting { device }) => {
                return Ok(format!("{r} already has an agent starting on {device}."));
            }
            Some(
                Why::NoShow { .. }
                | Why::NextPass
                | Why::AtCap { .. }
                | Why::Elsewhere { .. }
                | Why::NoDevice { .. }
                | Why::NoComputer(_)
                | Why::AgentStartsFull { .. },
            ) => {}
            Some(w) => return Err(w.to_string()),
        }
        // One lock over the count and the ask, so two agents asking at once
        // can't both take the last slot here.
        let mut asks = self
            .spawn
            .agent_asks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let person = self
            .spawn
            .person_asks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(id);
        if person || asks.contains_key(id) {
            return Ok(format!("{r} is already asked for."));
        }
        let used = self.used_with(&asks, "");
        if used >= s.max {
            return Err(Why::AgentStartsFull { used, max: s.max }.to_string());
        }
        asks.insert(id.to_string(), self.ask_for(&t));
        drop(asks);
        // Written even when this device serves it, so a restart keeps the
        // ask and its count. No spawn_now: that is the person's override.
        if let Err(e) = self.request_agent_start(id) {
            self.settle_ask(id);
            return Err(e);
        }
        self.asks_kick.kick();
        logf(format_args!("spawn {r}: {caller} asked for an agent"));
        let next = match &why {
            Some(
                w @ (Why::AtCap { .. }
                | Why::NoDevice { .. }
                | Why::NoComputer(_)
                | Why::Elsewhere { .. }),
            ) => {
                format!(" {w}.")
            }
            _ => String::new(),
        };
        Ok(format!(
            "Asked for an agent on {r}: {} of the person's {} in use.{next}",
            used + 1,
            s.max
        ))
    }

    /// Agent `caller` ends agent `key`, with the hand-off the person's End
    /// runs. Only agents Irori started on the person's own devices. The
    /// card it was on.
    pub(crate) fn agent_end(&self, caller: &str, key: &str) -> Result<String, String> {
        if !self.own_agent(caller) {
            return Err("only the person's own agents end agents here".into());
        }
        if !self.agent_starts().on {
            return Err(Why::AgentStartsOff.to_string());
        }
        if key == caller {
            return Err("that's you: board_leave your card instead".into());
        }
        if !self.own_agent(key) {
            return Err("that agent is another person's".into());
        }
        let Some(m) = self.coord.spawned(key) else {
            return Err("Irori didn't start that agent, so only the person ends it".into());
        };
        if !self.coord.spawned_live(key) {
            return Err("that agent has already gone".into());
        }
        self.request_end(key)?;
        logf(format_args!("spawn: {caller} ended {key}"));
        Ok(m.task_id)
    }
}
