//! Offers: a card an agent posts unheld goes to one free agent at a time
//! (card #82, research 07 case G), the way an ask does (#91, decision 38).
//! Pull, not push: the Core knows who is free, so Irori hands the card out
//! instead of the poster picking someone off the roster.
//!
//! The card goes to the free agent idle longest, which takes it with
//! board_join. It moves on only on an event, never on time: that agent
//! gives it back (board_leave, or skipping the delivery), its turn ends
//! without taking it, or it goes away. With nobody free it waits, and goes
//! out when an agent goes idle. The poster's device runs the hand-out and
//! writes its `task.offered` rows. An agent busy on a working card (or
//! owning open steps on one) is told no when it tries to take a second,
//! wherever that working card is: the private board or a shared one
//! (card #422, [`super::Options::busy_elsewhere`]).

use std::collections::{HashMap, HashSet};

use gocompat::quote::quote;
use gocompat::time::GoTime;
use irori_identity::owner;
use store::{Event, kinds};

use super::asks::Handed;
use super::error::{Error, ErrorKind, Result, bad_state};
use super::fold::{Fold, Touch, engaged};
use super::rows::*;
use super::types::*;
use super::util::{SWEEPER, caller, contains, cut, deeper};
use crate::gotime;
use crate::roster::AgentCard;

/// What an offer notice quotes of the card's objective.
const MAX_OFFER_NOTICE: usize = 600;

/// Irori handed it to `to`.
pub const OFFER_HANDED: &str = "handed";
/// The agent it went to gave it back or ended its turn without taking it.
pub const OFFER_PASSED: &str = "passed";
/// Nobody is free: it waits for the next agent to go idle.
pub const OFFER_WAITING: &str = "waiting";

/// Idle agents and the open work nobody has been handed (card #341), for
/// Home's line: a work shortage reads differently from a dispatch gap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdleReport {
    /// Agents idle on an online device with no working card in hand.
    pub idle: usize,
    /// Unheld cards this device hands out that went to nobody yet, the
    /// oldest first.
    pub unoffered: Vec<Unoffered>,
}

/// One unheld card nobody has been handed, and why.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Unoffered {
    pub task_id: String,
    pub short: String,
    pub title: String,
    pub why: String,
}

/// Whether card t is one Irori hands out now: open, unheld, unassigned,
/// private, not an open ask and not a parent, and not waiting on other work
/// (card #397).
pub(crate) fn offerable(t: &Task) -> bool {
    needs_agent(t) && !t.spawn_pending()
}

/// [`offerable`] before its spawn check (card #446): the card needs an
/// agent, whether one is starting for it or not.
pub(crate) fn needs_agent(t: &Task) -> bool {
    t.state == SUBMITTED
        && t.holders.is_empty()
        && t.assigned.is_empty()
        && !t.shared
        && !t.withdrawn
        // A PR card is where a PR's review meets, never work (merge desk
        // §12.2): handing it out would start an agent for every PR.
        && !pr_card(t)
        // A review card is given to its reviewer and starts on their yes
        // (or their switch), never from the author's hand-out (§12.10).
        && t.review_of.is_empty()
        // An ask made into a card is a card from then on (card #454).
        && t.open_ask().is_none()
        && t.subtasks.is_empty()
        && !waits_on_work(t)
        // A given card is its person's: their yes starts its agent, and
        // nobody else's hand-out reaches it (card #573).
        && t.given.is_none()
}

impl Fold {
    /// Whether ask t needs an agent Irori starts for it (card #470): open,
    /// not withdrawn, handed out here (this device owns its take lock),
    /// nobody has it, and the hand-out ran and found nobody free
    /// (`unclaimed`), so an idle agent with "Offer it cards" on still takes
    /// it first. A plain ask that names agents goes to them alone. A thread
    /// goes out at once, whoever it names (card #512: they are woken, and
    /// nobody waits on them), so it is unclaimed as soon as the hand-out
    /// finds nobody free; one with "Agents answer now" off is never handed
    /// out, so never unclaimed. A private thread about a shared board
    /// (`a.board`, card #185) lives in the person's own log, so it spawns
    /// too (card #454), and so does a thread in a shared board's own log,
    /// on the poster's device (the ask's owner; card #512, as a board card
    /// spawns on the device that made it, #455). Plain board asks still go
    /// to the members' idle agents only.
    pub(crate) fn ask_needs_agent(&self, t: &Task) -> bool {
        let Some(a) = t.open_ask() else {
            return false;
        };
        (!self.on_board || a.thread)
            && !is_terminal(&t.state)
            && !t.withdrawn
            && !t.shared
            && a.owner == self.dev
            && a.taker.is_empty()
            && a.unclaimed
            && (a.thread || a.mentions.is_empty())
            && (!a.thread || a.wake)
            // An agent's own thread its person hasn't written in (card
            // #631): another agent's words never start one.
            && !self.agent_thread_rests(t)
    }
}

/// Whether t has an open blocker on other work: another task, a lease, an
/// agent or something outside (cards #397, #423). Nobody can start it
/// until that is done, so Irori doesn't hand it out (an open ask can't be
/// blocked). A task or lease blocker closes by itself when the work is
/// done, the others when someone resolves them, and the card then goes out
/// once, as if just posted.
pub(crate) fn waits_on_work(t: &Task) -> bool {
    t.blockers
        .iter()
        .any(|b| b.open() && b.on.as_ref().is_some_and(|on| on_work(&on.kind)))
}

/// Whether a blocker of this kind waits on work rather than a person.
pub(crate) fn on_work(kind: &str) -> bool {
    matches!(
        kind,
        BLOCKED_ON_TASK
            | BLOCKED_ON_LEASE
            | BLOCKED_ON_AGENT
            | BLOCKED_ON_EXTERNAL
            | BLOCKED_ON_VERSION
    )
}

/// When t's last blocker on other work was resolved, if it has one.
fn work_unblocked_at(t: &Task) -> Option<GoTime> {
    t.blockers
        .iter()
        .filter(|b| b.on.as_ref().is_some_and(|on| on_work(&on.kind)))
        .filter_map(|b| b.resolved.as_ref().map(|r| r.at))
        .max_by(gotime::cmp)
}

impl Task {
    /// Irori's hand-out while it applies: the card is still unheld.
    pub fn live_offer(&self) -> Option<&Offer> {
        self.offer.as_ref().filter(|_| offerable(self))
    }
}

impl Fold {
    /// The working card `key` already has in hand, if any: one it holds
    /// and has taken up, or owns open steps on (card #86), open, not
    /// shared, and not a parent (a parent's state follows its subtasks,
    /// and its holder runs them).
    pub(crate) fn busy_with(&self, key: &str, except: &str) -> Option<&Task> {
        let mut busy: Vec<&Task> = self
            .tasks
            .values()
            .filter(|t| {
                t.id != except
                    && t.state == WORKING
                    && !t.shared
                    && t.subtasks.is_empty()
                    && t.open_ask().is_none()
                    && (engaged(t, key) || contains(&t.co_holders, key))
            })
            .collect();
        busy.sort_by(|a, b| a.id.cmp(&b.id));
        busy.into_iter().next()
    }

    /// How a busy card reads in a refusal: `tag#n "title"`.
    pub(crate) fn busy_text(t: &Task) -> String {
        format!("{} {}", t.short_ref(), quote(&t.title))
    }

    /// The working card `key` holds in another of the Core's logs (card
    /// #422), by [`super::Options::busy_elsewhere`].
    pub(crate) fn busy_elsewhere(&self, key: &str) -> Option<String> {
        self.busy_elsewhere.as_ref().and_then(|f| f(key))
    }

    /// Whether `key` has a working card in hand here or in another log.
    pub(crate) fn busy_anywhere(&self, key: &str) -> bool {
        self.busy_with(key, "").is_some() || self.busy_elsewhere(key).is_some()
    }

    /// Refuses `by` taking card t while it already works another one. A
    /// shared card, a card someone assigned to `by`, and a person are let
    /// through.
    pub(crate) fn check_free(&self, by: &str, t: &Task) -> Result<()> {
        if self.is_owner(by) {
            return Ok(());
        }
        // An agent started for a card takes only that card and its
        // subtasks (card #446).
        if let Some(s) = self.spawned_for(by)
            && !self.spawned_may_take(by, t)
        {
            return Err(Error::raw(
                ErrorKind::NotAllowed,
                format!(
                    "coord: you were started for {} and take only that card: leave {} for another agent.",
                    self.ref_of(&s.task_id),
                    t.short_ref()
                ),
            ));
        }
        if t.shared || contains(&t.assigned, by) {
            return Ok(());
        }
        // A working card in another log (card #422) counts as one here.
        let held = match self.busy_with(by, &t.id) {
            Some(b) => Fold::busy_text(b),
            None => match self.busy_elsewhere(by) {
                Some(d) => d,
                None => return Ok(()),
            },
        };
        Err(Error::raw(
            ErrorKind::NotAllowed,
            format!(
                "coord: you already hold {held} and it is still working: one card at a time. Finish it (board_status completed) or hand it back (board_leave) first; until then leave {} for a free agent.",
                t.short_ref()
            ),
        ))
    }

    /// Keeps the inbox's view of card `id`'s hand-out current, so a
    /// hand-out notice to an agent it no longer goes to closes before it
    /// wakes that agent (the Stale hook never takes coord's lock).
    pub(crate) fn sync_offer_stale(&self, id: &str) {
        let Some(t) = self.tasks.get(id).filter(|t| t.offer.is_some()) else {
            return;
        };
        let v = match t.live_offer() {
            Some(o) => (true, o.to.clone()),
            None if is_terminal(&t.state) => (false, "the card closed".to_string()),
            None if !t.holders.is_empty() => (false, format!("taken by {}", t.holders.join(", "))),
            None if !t.assigned.is_empty() => {
                (false, format!("assigned to {}", t.assigned.join(", ")))
            }
            None => (false, "it is no longer handed out".to_string()),
        };
        let mut s = self.stale.write().unwrap_or_else(|e| e.into_inner());
        s.offers.insert(id.to_string(), v);
    }

    /// The agents an unheld card other than `id` is handed to now: one
    /// thing at a time.
    pub(crate) fn offered_elsewhere(&self, id: &str) -> Vec<String> {
        self.tasks
            .values()
            .filter(|t| t.id != id)
            .filter_map(|t| t.live_offer())
            .filter(|o| !o.to.is_empty())
            .map(|o| o.to.clone())
            .collect()
    }

    /// The agents among `roster` free to take card t now: live, idle, in
    /// t's repo when both have one, holding no working card, handed no
    /// other card or ask, and not in `skip`. Those on the machine the card
    /// needs come first (card #340, see `machine_rank`), then the one idle
    /// longest.
    pub(crate) fn free_for(&self, t: &Task, roster: &[AgentCard], skip: &[String]) -> Vec<String> {
        let elsewhere = self.handed_elsewhere(&t.id);
        let needs = needs_machine(t);
        let repo_devs = repo_devices(t, roster);
        let mut out: Vec<String> = Vec::new();
        let mut since: HashMap<String, GoTime> = HashMap::new();
        let mut rank: HashMap<String, u8> = HashMap::new();
        for a in roster {
            if a.key.is_empty()
                || a.key == SWEEPER
                || self.is_owner(&a.key)
                || a.status != "idle"
                || a.device_state == crate::roster::OFFLINE
            {
                continue;
            }
            let dev = device_of(a);
            if let Some(r) = &self.revoked
                && !dev.is_empty()
                && r(dev)
            {
                continue;
            }
            if !t.repo_key.is_empty()
                && !a.repo_key.is_empty()
                && !ledger::same_repo_key(&a.repo_key, &t.repo_key)
            {
                continue;
            }
            // Only an agent on a device that runs the card's side (card #495).
            if !self.sides.runs(dev, &t.side) {
                continue;
            }
            if skip.contains(&a.key)
                || out.contains(&a.key)
                || elsewhere.contains(&a.key)
                || self.busy_anywhere(&a.key)
                || !self.may_hand_to(&a.key, t)
            {
                continue;
            }
            out.push(a.key.clone());
            since.insert(a.key.clone(), a.updated_at);
            rank.insert(a.key.clone(), machine_rank(needs, &repo_devs, a, dev));
        }
        out.sort_by(|x, y| {
            rank[x]
                .cmp(&rank[y])
                .then_with(|| gotime::cmp(&since[x], &since[y]))
                .then_with(|| x.cmp(y))
        });
        out
    }

    /// What Home says about idle agents and open work (card #341): how
    /// many agents are idle with nothing in hand, and each unheld card
    /// Irori hands out that nobody has been handed yet, with why. Read
    /// only: the hand-out stays [`Fold::sweep_offers`]'s.
    pub(crate) fn idle_report(&self, agents: &[AgentCard]) -> IdleReport {
        let idle: Vec<&AgentCard> = agents
            .iter()
            .filter(|a| {
                !a.key.is_empty()
                    && a.key != SWEEPER
                    && !self.is_owner(&a.key)
                    && a.status == "idle"
                    && a.device_state != crate::roster::OFFLINE
                    && !self.busy_anywhere(&a.key)
                    && self.takes_handout(&a.key)
            })
            .collect();
        let mut cards: Vec<&Task> = self
            .tasks
            .values()
            .filter(|t| offerable(t) && self.hands_out(t) && self.runs_here(t))
            .filter(|t| t.offer.as_ref().is_none_or(|o| o.to.is_empty()))
            .collect();
        cards.sort_by(|a, b| {
            gotime::cmp(&a.created_at, &b.created_at).then_with(|| a.id.cmp(&b.id))
        });
        let unoffered = cards
            .into_iter()
            .map(|t| {
                let heard = t
                    .offer
                    .as_ref()
                    .map(|o| o.heard.clone())
                    .unwrap_or_default();
                let mut skip = heard.clone();
                skip.push(t.created_by.clone());
                let pool = self.offer_pool(t, agents);
                let why = if idle.is_empty() {
                    // Idle agents the person didn't offer cards to are left
                    // out of idle (card #453).
                    "no agent is free".to_string()
                } else if !self.free_for(t, &pool, &skip).is_empty() {
                    "a free agent is there: the next sweep hands it out".to_string()
                } else if !t.repo_key.is_empty()
                    && idle.iter().all(|a| {
                        !a.repo_key.is_empty() && !ledger::same_repo_key(&a.repo_key, &t.repo_key)
                    })
                {
                    format!("no free agent works in {}", ledger::repo_name(&t.repo_key))
                } else if idle.iter().all(|a| heard.contains(&a.key)) {
                    "every free agent passed it".to_string()
                } else {
                    "the free agents were handed other work".to_string()
                };
                Unoffered {
                    task_id: t.id.clone(),
                    short: t.short_ref(),
                    title: t.title.clone(),
                    why,
                }
            })
            .collect();
        IdleReport {
            idle: idle.len(),
            unoffered,
        }
    }

    /// `by` puts its new card `id` up for a free agent: Irori hands it to
    /// the one idle longest at once, or it waits for the next to go idle.
    /// The notice is for that agent (its `to` empty when nobody is free).
    pub(crate) fn offer_card(
        &mut self,
        by: &str,
        id: &str,
        roster: &[AgentCard],
    ) -> Result<Notice> {
        let by = caller(by)?;
        let t = self.open_task(id)?.clone();
        if !offerable(&t) || self.on_board {
            return Err(bad_state(format!(
                "{} isn't an unheld card Irori can hand out",
                t.short_ref()
            )));
        }
        let now = self.stamp();
        let skip = vec![by.clone()];
        // On a board with spawning on, Irori starts an agent for the card
        // (cards_to_spawn) and the sweep hands nothing out there: handing
        // it to an idle agent too would put two on it (card #453).
        // A card on a side this device doesn't run waits (card #495).
        let free = if self.spawning_on() || !self.runs_here(&t) {
            Vec::new()
        } else {
            self.free_for(&t, roster, &skip)
        };
        let row = match free.first() {
            Some(pick) => TaskOfferedRow {
                to: pick.clone(),
                reason: OFFER_HANDED.into(),
                heard: vec![pick.clone()],
                ..TaskOfferedRow::default()
            },
            None => TaskOfferedRow {
                reason: OFFER_WAITING.into(),
                ..TaskOfferedRow::default()
            },
        };
        self.write_offer(id, &by, now, row)?;
        let t = self.tasks[id].clone();
        Ok(match free.first() {
            Some(pick) => offer_notice(&t, pick, &by, ""),
            None => Notice::default(),
        })
    }

    /// `who` gives back card `id` Irori handed it: it goes to the next
    /// free agent. Ok(false) when it wasn't handed to `who`.
    pub(crate) fn pass_offer(&mut self, who: &str, id: &str) -> Result<bool> {
        let Some(o) = self.tasks.get(id).and_then(|t| t.live_offer()).cloned() else {
            return Ok(false);
        };
        if o.to != who {
            return Ok(false);
        }
        let now = self.stamp();
        self.write_offer(
            id,
            who,
            now,
            TaskOfferedRow {
                reason: OFFER_PASSED.into(),
                was: who.into(),
                heard: o.heard,
                ..TaskOfferedRow::default()
            },
        )?;
        Ok(true)
    }

    fn write_offer(
        &mut self,
        id: &str,
        by: &str,
        now: GoTime,
        mut r: TaskOfferedRow,
    ) -> Result<()> {
        r.task_id = id.into();
        r.by = by.into();
        r.since = now;
        r.at = now;
        let actor = if by == SWEEPER {
            SWEEPER.to_string()
        } else {
            self.actor_for(by)
        };
        self.write_as(&actor, kinds::TASK_OFFERED, &r)
    }

    pub(crate) fn fold_offered(&mut self, ev: &Event, at: &dyn Fn(GoTime) -> GoTime) {
        let Some(p) = row::<TaskOfferedRow>(&ev.payload) else {
            return;
        };
        // On a board only the device that made the card hands it out (card
        // #174), whoever the row names as its writer; anyone it went to may
        // still give it back.
        if self.on_board
            && p.reason != OFFER_PASSED
            && self.origin.get(&p.task_id) != Some(&ev.device_id)
        {
            return;
        }
        let Some(t) = self.tasks.get_mut(&p.task_id) else {
            return;
        };
        let was = t.offer.as_ref().map(|o| o.to.clone()).unwrap_or_default();
        t.offer = Some(Offer {
            to: p.to.clone(),
            reason: p.reason.clone(),
            since: at(p.since),
            heard: p.heard.clone(),
        });
        let ti = Touch::of(t);
        let when = at(p.at);
        let gone = if p.was.is_empty() { was } else { p.was.clone() };
        match p.reason.as_str() {
            OFFER_HANDED if !p.to.is_empty() => {
                if !p.was.is_empty() {
                    self.touch(ev, &ti, when, &p.was, "passed it on", false, Vec::new());
                }
                let what = format!("handed it to {}", self.named(&p.to));
                self.touch(ev, &ti, when, SWEEPER, &what, false, Vec::new());
            }
            OFFER_PASSED if !gone.is_empty() => {
                self.touch(ev, &ti, when, &gone, "passed it on", false, Vec::new());
            }
            OFFER_WAITING if !gone.is_empty() => {
                let what = "passed it on; no agent is free, so it waits for the next one that takes offered cards";
                self.touch(ev, &ti, when, &gone, what, false, Vec::new());
            }
            OFFER_WAITING => {
                let what = "no agent is free: it waits for the next one that takes offered cards";
                self.touch(ev, &ti, when, SWEEPER, what, false, Vec::new());
            }
            _ => {}
        }
        self.note(Change {
            kind: TASK_CHANGED.into(),
            task_id: p.task_id,
            ..Change::default()
        });
    }

    /// Whether this device runs card t's hand-out: on the private board,
    /// a card its agents posted; on a shared board, a card it made (card
    /// #174).
    pub(crate) fn hands_out(&self, t: &Task) -> bool {
        if self.on_board {
            return self.origin.get(&t.id) == Some(&self.dev);
        }
        owner::device_of(&t.created_by) == self.dev
    }

    /// The device that made card t.
    fn made_on(&self, t: &Task) -> String {
        if self.on_board {
            return self.origin.get(&t.id).cloned().unwrap_or_default();
        }
        owner::device_of(&t.created_by).to_string()
    }

    /// Whether device `dev` can run agents (card #541).
    pub(crate) fn can_run(&self, dev: &str) -> bool {
        self.agent_devices
            .as_ref()
            .is_none_or(|f| !f().cant.contains(dev))
    }

    /// Whether device `dev` has been away for at least `d` (card #541).
    pub(crate) fn away_for(&self, dev: &str, d: std::time::Duration) -> bool {
        let Some(f) = self.agent_devices.as_ref() else {
            return false;
        };
        let a = f();
        if !a.away.contains(dev) {
            return false;
        }
        let now = i128::from(self.stamp().unix_nano());
        a.away_since
            .get(dev)
            .is_some_and(|s| now - i128::from(*s) >= d.as_nanos() as i128)
    }

    /// Whether device `dev`'s Irori reads `ask.moved` (card #541); every
    /// device when nothing says (tests without a sweep).
    pub(crate) fn reads_moves(&self, dev: &str) -> bool {
        self.agent_devices
            .as_ref()
            .is_none_or(|f| f().moves.contains(dev))
    }

    /// The device agent work on `side` goes to when it lands on one that
    /// can't run agents: the first of the person's computers that runs
    /// the side (card #541).
    pub(crate) fn agent_home(&self, side: &str) -> Option<String> {
        let d = self.agent_devices.as_ref()?();
        d.homes
            .into_iter()
            .find(|h| self.sides.runs(h, side) && self.same_person_dev(h, &self.dev))
    }

    /// Whether device `dev` isn't connected now (card #686).
    pub(crate) fn is_away(&self, dev: &str) -> bool {
        self.agent_devices
            .as_ref()
            .is_some_and(|f| f().away.contains(dev))
    }

    /// Whether device `dev` has no free agent slot now (card #686): at its
    /// cap, held by a usage limit, or with its agents paused.
    pub(crate) fn is_full(&self, dev: &str) -> bool {
        self.agent_devices
            .as_ref()
            .is_some_and(|f| f().full.contains(dev))
    }

    /// How many more agents device `dev` can start now (card #686); no
    /// limit when nothing says.
    pub(crate) fn free_slots(&self, dev: &str) -> i64 {
        let Some(f) = self.agent_devices.as_ref() else {
            return i64::MAX;
        };
        let d = f();
        if d.full.contains(dev) {
            return 0;
        }
        d.free.get(dev).copied().unwrap_or(i64::MAX)
    }

    /// The computer an ask on `side` goes to when its owner `owner` is
    /// full (card #686): the first of the person's computers, in
    /// [`Self::agent_home`]'s order, that is connected, runs the side and
    /// has a free slot. Only it takes the ask, so two computers with room
    /// don't both write a move. None when every computer is full.
    pub(crate) fn room_home(&self, side: &str, owner: &str) -> Option<String> {
        let d = self.agent_devices.as_ref()?();
        d.homes.into_iter().find(|h| {
            h != owner
                && !d.full.contains(h)
                && !d.away.contains(h)
                && !d.cant.contains(h)
                && self.sides.runs(h, side)
                && self.same_person_dev(h, &self.dev)
        })
    }

    /// Whether this device starts the agent of a card made on a device
    /// that can't run agents (card #541). No device is asked to: a phone
    /// asleep hands nothing out, so a computer does it for the phone, and
    /// the first computer back takes up what waited.
    fn stands_in(&self, t: &Task) -> bool {
        let made = self.made_on(t);
        !made.is_empty()
            && made != self.dev
            && !self.can_run(&made)
            && self.same_person_dev(&made, &self.dev)
            && self.agent_home(&t.side).as_deref() == Some(self.dev.as_str())
            // The phone's hand-out already gave it to an idle agent.
            && t.live_offer().is_none()
    }

    /// Whether this device starts card t's agent (card #541): the ask's
    /// owner, the device that made the card when it can run agents, or
    /// the computer standing in for it.
    pub(crate) fn spawns_out(&self, t: &Task) -> bool {
        if let Some(a) = t.open_ask() {
            return a.owner == self.dev;
        }
        (self.hands_out(t) && self.can_run(&self.dev)) || self.stands_in(t)
    }

    /// Whether this device names the person's thread t (card #782): the
    /// ask's owner (with no namer, as on the phone, it names nothing),
    /// else the computer standing in for an owner that can't run Claude
    /// (a thread posted on the phone). One device, so it isn't named
    /// twice. Whether the owner starts agents now doesn't matter: naming
    /// isn't an agent.
    pub(crate) fn names_out(&self, t: &Task) -> bool {
        let Some(a) = t.ask.as_ref() else {
            return false;
        };
        if a.owner == self.dev {
            return true;
        }
        !a.owner.is_empty()
            && !self.can_run(&a.owner)
            && self.same_person_dev(&a.owner, &self.dev)
            && self.agent_home(&t.side).as_deref() == Some(self.dev.as_str())
    }

    /// Whether this device runs card t's side (card #495): a card with no
    /// side, or a device with no pin, runs anywhere. A card this device
    /// hands out on a side it doesn't run waits: nothing here spawns it or
    /// hands it out, and it is never handed to another device instead.
    pub(crate) fn runs_here(&self, t: &Task) -> bool {
        self.sides.runs(&self.dev, &t.side)
    }

    /// Copies the sides each device runs where the inbox's off-side hold
    /// reads them (card #495).
    pub(crate) fn index_runs(&self) {
        let mut s = self.stale.write().unwrap_or_else(|e| e.into_inner());
        s.runs = self.sides.clone();
    }

    /// The inbox's off-side hold (card #495): whether a notice of `kind`
    /// about card `id` to `to` is one Irori picked on its own for an agent
    /// on a device that doesn't run the card's side. A hand-out always is;
    /// an ask unless the person named `to` in it, and on a thread only the
    /// agent it was handed to. Runs under its own lock (the inbox's is held).
    /// Card t as it is shown: with [`Fold::side_wait`] set.
    pub(crate) fn shown(&self, t: &Task) -> Task {
        let mut t = t.clone();
        t.side_wait = self.side_wait(&t);
        t
    }

    /// Why card t waits on its side (card #495), or "" when it doesn't:
    /// it needs an agent, none is starting for it, and the device that
    /// hands it out doesn't run its side. Read when shown, so a pin that
    /// changes clears it at once. An ask never says so: its owner may be
    /// another device, and its agents are any device's that run the side.
    pub(crate) fn side_wait(&self, t: &Task) -> String {
        if self.on_board
            || crate::sides::clean(&t.side).is_empty()
            || !needs_agent(t)
            || t.spawn_pending()
            || self.spawn_now.contains(&t.id)
        {
            return String::new();
        }
        let dev = owner::device_of(&t.created_by);
        if dev.is_empty() || self.sides.runs(dev, &t.side) {
            return String::new();
        }
        format!("no device here runs {}", crate::sides::clean(&t.side))
    }

    /// The agents card t may go to (card #174): on a shared board, only
    /// those of the member who made it. Irori never hands one person's
    /// card to another person's agents (0.11.1: their own background
    /// agents serve them); an agent told to take one still can.
    fn offer_pool(&self, t: &Task, agents: &[AgentCard]) -> Vec<AgentCard> {
        // A spawned agent serves only the card it was started for (§6).
        let agents: Vec<AgentCard> = agents
            .iter()
            .filter(|a| self.may_hand_to(&a.key, t))
            .cloned()
            .collect();
        let (true, Some(kp)) = (self.on_board, &self.key_person) else {
            return agents;
        };
        let author = kp(&t.created_by);
        if author.is_empty() {
            return Vec::new();
        }
        agents
            .iter()
            .filter(|a| kp(&a.key) == author)
            .cloned()
            .collect()
    }

    /// How many unheld cards this device hands out have nobody among
    /// `agents` free to take them now (card #337): on a shared board the
    /// Core then puts that many of its person's idle agents on the board.
    pub(crate) fn unserved_offers(&self, agents: &[AgentCard]) -> usize {
        self.tasks
            .values()
            .filter(|t| {
                let waiting = match t.live_offer() {
                    Some(o) => o.to.is_empty(),
                    None => t.offer.is_none() && offerable(t),
                };
                waiting && self.hands_out(t) && self.runs_here(t)
            })
            .filter(|t| {
                let mut skip = t
                    .offer
                    .as_ref()
                    .map(|o| o.heard.clone())
                    .unwrap_or_default();
                skip.push(t.created_by.clone());
                let pool = self.offer_pool(t, agents);
                self.free_for(t, &pool, &skip).is_empty()
            })
            .count()
    }

    /// What this device does for the unheld cards its agents posted (on a
    /// shared board, the cards it made): hands each on when the agent it
    /// went to gave it back, ended its turn without taking it, or went
    /// away, and hands out the ones waiting when an agent is free. Never
    /// by time waited.
    pub(crate) fn sweep_offers(&mut self, agents: &[AgentCard]) -> (Vec<Notice>, Result<()>) {
        let mut out = Vec::new();
        let in_roster: HashMap<&str, &AgentCard> =
            agents.iter().map(|a| (a.key.as_str(), a)).collect();
        // Cards posted here that never had a hand-out (posted before 0.7.1,
        // put back on the Board by Irori, or with their offer row lost) go
        // out too (card #110), as if just posted.
        // On a spawning board a card that needs an agent gets its own
        // (card #446): the Core asks for it, nothing is handed out here.
        let spawning = self.spawning_on();
        let mut ids: Vec<(GoTime, String)> = self
            .tasks
            .values()
            .filter(|t| {
                !spawning
                    && (t.live_offer().is_some() || (t.offer.is_none() && offerable(t)))
                    && self.hands_out(t)
                    // On a side this device doesn't run (card #495) a card
                    // gets no new pick; one already handed out is still
                    // moved on when its agent ends or goes, so that agent
                    // is free for other work.
                    && (self.runs_here(t) || t.live_offer().is_some_and(|o| !o.to.is_empty()))
            })
            .map(|t| (t.created_at, t.id.clone()))
            .collect();
        // Oldest card first: it gets the agent idle longest.
        ids.sort_by(|a, b| gotime::cmp(&a.0, &b.0).then_with(|| a.1.cmp(&b.1)));
        let res = (|| -> Result<()> {
            for (_, id) in &ids {
                let t = &self.tasks[id];
                // A hand-out from before its last holder let it go (card
                // #338) is stale: the card goes out as if just posted,
                // never back to the one who let it go.
                let fresh = || Offer {
                    heard: t.let_go.iter().map(|(k, _)| k.clone()).collect(),
                    ..Offer::default()
                };
                let o = match t.live_offer() {
                    Some(o) if stale_offer(t, o) => fresh(),
                    Some(o) => o.clone(),
                    None if t.offer.is_none() && offerable(t) => fresh(),
                    None => continue,
                };
                let let_go = if o.to.is_empty() && o.reason.is_empty() && t.let_go.is_some() {
                    if t.returned.is_some() {
                        "the agent working it went away"
                    } else {
                        "the agent working it gave it back"
                    }
                } else {
                    ""
                };
                let mut heard = o.heard.clone();
                let (was, why) = if !o.to.is_empty() {
                    match self.turn_state(&o.to, &o.since, &in_roster) {
                        Handed::Waits | Handed::Touched => continue,
                        Handed::Ended => (
                            o.to.clone(),
                            "the agent it went to ended its turn without taking it",
                        ),
                        Handed::Gone => (o.to.clone(), "the agent it went to went away"),
                    }
                } else if o.reason == OFFER_PASSED {
                    (String::new(), "the agent it went to gave it back")
                } else {
                    (String::new(), let_go)
                };
                if !was.is_empty() && !heard.contains(&was) {
                    heard.push(was.clone());
                }
                let t = self.tasks[id].clone();
                let mut skip = heard.clone();
                skip.push(t.created_by.clone());
                let pool = self.offer_pool(&t, agents);
                let free = if self.runs_here(&t) {
                    self.free_for(&t, &pool, &skip)
                } else {
                    Vec::new()
                };
                let now = self.stamp();
                if let Some(pick) = free.first() {
                    heard.push(pick.clone());
                    self.write_offer(
                        id,
                        SWEEPER,
                        now,
                        TaskOfferedRow {
                            to: pick.clone(),
                            was,
                            reason: OFFER_HANDED.into(),
                            heard,
                            ..TaskOfferedRow::default()
                        },
                    )?;
                    let t = self.tasks[id].clone();
                    out.push(offer_notice(&t, pick, &t.created_by, why));
                    continue;
                }
                // Nobody free: it waits, and goes out the moment an agent
                // goes idle (the next sweep, kicked by the roster change).
                if o.to.is_empty() && o.reason == OFFER_WAITING {
                    continue;
                }
                self.write_offer(
                    id,
                    SWEEPER,
                    now,
                    TaskOfferedRow {
                        was,
                        reason: OFFER_WAITING.into(),
                        heard,
                        ..TaskOfferedRow::default()
                    },
                )?;
            }
            Ok(())
        })();
        (out, res)
    }
}

/// The machine card t asks for (card #340): `needs:mac` or `needs:pc`
/// (`needs:windows` too) in its title, objective or done-when. None when
/// it names neither, or both.
fn needs_machine(t: &Task) -> Option<Machine> {
    let text = format!("{} {} {}", t.title, t.objective, t.done_criteria).to_lowercase();
    let mac = text.contains("needs:mac");
    let pc = text.contains("needs:pc") || text.contains("needs:windows");
    match (mac, pc) {
        (true, false) => Some(Machine::Mac),
        (false, true) => Some(Machine::Pc),
        _ => None,
    }
}

/// The machine card t asks for by its `needs:` hint (card #455, for
/// choosing the device its agent starts on): "mac", "pc", or "" for none.
pub fn machine_needed(t: &Task) -> &'static str {
    match needs_machine(t) {
        Some(Machine::Mac) => "mac",
        Some(Machine::Pc) => "pc",
        None => "",
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Machine {
    Mac,
    Pc,
}

/// The machine an agent runs on, from its working directory's shape: a
/// drive letter or UNC path is a PC, /Users or /Volumes a Mac.
fn machine_of(cwd: &str) -> Option<Machine> {
    let b = cwd.as_bytes();
    if cwd.starts_with("\\\\") || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':') {
        Some(Machine::Pc)
    } else if cwd.starts_with("/Users/") || cwd.starts_with("/Volumes/") {
        Some(Machine::Mac)
    } else {
        None
    }
}

/// The devices card t's repo is known on: those with an agent working in
/// it, and the one where the card was last worked (its `where`). Most
/// agents start outside a repo, so this is often just the latter, or none.
fn repo_devices(t: &Task, roster: &[AgentCard]) -> HashSet<String> {
    let mut out = HashSet::new();
    if t.repo_key.is_empty() {
        return out;
    }
    for a in roster {
        if ledger::same_repo_key(&a.repo_key, &t.repo_key) {
            out.insert(device_of(a).to_string());
        }
    }
    if let Some(w) = &t.where_
        && !w.who.is_empty()
        && !(w.worktree.is_empty() && w.cwd.is_empty())
    {
        out.insert(owner::device_of(&w.who).to_string());
    }
    out.remove("");
    out
}

fn device_of(a: &AgentCard) -> &str {
    if a.device_id.is_empty() {
        owner::device_of(&a.key)
    } else {
        a.device_id.as_str()
    }
}

/// How well agent `a` on device `dev` fits the machine card t needs: 0 on
/// it, 1 unknown, 2 on the other one. A `needs:` hint wins; without one,
/// a device where the card's repo is known beats the rest.
fn machine_rank(
    needs: Option<Machine>,
    repo_devs: &HashSet<String>,
    a: &AgentCard,
    dev: &str,
) -> u8 {
    if let Some(n) = needs {
        return match machine_of(&a.cwd) {
            Some(m) if m == n => 0,
            Some(_) => 2,
            None => 1,
        };
    }
    if repo_devs.is_empty() || repo_devs.contains(dev) {
        0
    } else {
        1
    }
}

/// Whether hand-out o of card t came before its last holder let it go,
/// or before the other work it waited on was done (card #397: the agents
/// it reached while it couldn't be started don't count as having passed).
fn stale_offer(t: &Task, o: &Offer) -> bool {
    t.let_go
        .as_ref()
        .is_some_and(|(_, at)| gotime::before(&o.since, at))
        || work_unblocked_at(t).is_some_and(|at| gotime::before(&o.since, &at))
}

/// The notice handing unheld card t to agent `to`.
fn offer_notice(t: &Task, to: &str, poster: &str, why: &str) -> Notice {
    let mut b = format!(
        "{poster} put up a card, and Irori handed it to you{}: {} ({})",
        if why.is_empty() {
            String::new()
        } else {
            format!(" ({why})")
        },
        quote(&t.title),
        t.refs()
    );
    if !t.objective.is_empty() {
        b.push_str(&format!(":\n{}", cut(&t.objective, MAX_OFFER_NOTICE)));
    } else {
        b.push('.');
    }
    if !t.done_criteria.is_empty() {
        b.push_str(&format!(
            "\nDone when: {}",
            cut(&t.done_criteria, MAX_OFFER_NOTICE)
        ));
    }
    b.push_str(&t.handover_lines());
    b.push_str(&format!(
        "\nIt is yours to take: board_join {} and start. If you can't take it on now, board_leave {} and it goes to the next free agent; ending your turn without taking it does the same.",
        t.short_ref(),
        t.short_ref()
    ));
    b.push_str(&deeper(&t.id));
    Notice {
        kind: NOTICE_OFFER.into(),
        task_id: t.id.clone(),
        reference: format!("task:{}", t.id),
        title: t.title.clone(),
        by: poster.to_string(),
        to: vec![to.to_string()],
        // A free agent is idle, and an idle agent gets no next turn until
        // someone types in its window: an offer that doesn't wake reaches
        // nobody (as with asks, #69).
        wake: true,
        text: b,
        ..Notice::default()
    }
}

/// Puts card t's side where the inbox's off-side hold reads it (card
/// #495); a card on no side leaves it.
pub(crate) fn index_side(stale: &std::sync::RwLock<super::fold::StaleMap>, t: &Task) {
    let mut s = stale.write().unwrap_or_else(|e| e.into_inner());
    if t.side.is_empty() {
        s.sides.remove(&t.id);
        return;
    }
    let (named, thread) = match &t.ask {
        Some(a) => (a.mentions.iter().map(|m| m.key.clone()).collect(), a.thread),
        None => (Vec::new(), false),
    };
    let c = super::fold::SideCard {
        side: t.side.clone(),
        named,
        thread,
    };
    if s.sides.get(&t.id) != Some(&c) {
        s.sides.insert(t.id.clone(), c);
    }
}

/// The inbox's off-side hold over the index it reads (card #495): whether
/// a notice of `kind` about card `id` to `to` is one Irori picked on its
/// own for an agent on a device that doesn't run the card's side. A
/// hand-out or a review request always is; an ask unless the person named
/// `to` in it, and on a thread only the agent it was handed to.
pub(crate) fn off_side_in(s: &super::fold::StaleMap, kind: &str, id: &str, to: &str) -> bool {
    let Some(c) = s.sides.get(id) else {
        return false;
    };
    let dev = owner::device_of(to);
    if dev.is_empty() || s.runs.runs(dev, &c.side) {
        return false;
    }
    match kind {
        NOTICE_OFFER | super::review::NOTICE_REVIEW => true,
        NOTICE_ASK => {
            !c.named.iter().any(|n| n == to)
                && (!c.thread || s.stale.get(id).is_some_and(|t| t == to))
        }
        _ => false,
    }
}
