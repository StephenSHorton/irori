//! Spawned agents for cards on shared boards (docs/design/spawned-agents.md
//! §7, §9 "Shared boards"; card #455).
//!
//! Only cards this person's own device made get an agent, and only this
//! person's own agents: each member's Irori serves their own cards under
//! their own switch. The request and the mark live in the board's log (its
//! other members see the card starting and never offer the agent their
//! cards); the agent is put on the board (its `agent:<cardId>`, marked as
//! started by Irori on its agent card) before the card is assigned to it.
//! The brief leaves other members' lines out and bypass is never passed on
//! a shared board (§7). A board card's agent starts on the device that
//! made the card.
//!
//! A card another person gave this person (assigned to their `person:` key,
//! card #573) is the one exception: Irori stops and asks them in Needs you,
//! whatever the switch says. Their yes starts it as their own card would,
//! on the device that asked, wherever they said it; one yes, one agent.

use std::collections::BTreeMap;
use std::sync::Arc;

use coord::roster::Agents;
use coord::{Coord, SPAWN_CANCELED, Spawn, SpawnAsk, Task, TaskFilter};
use irori_identity::owner::device_of;

use super::Services;
use super::boards::BoardLog;
use super::spawn::{BriefMore, Spawner, build_brief, spawn_permissions};
use crate::logf;

/// Why a held card's starting agent was called off.
const HELD: &str = "the card is Not yet";

const SWEEPER: &str = "irori";

/// How much of what a given card asks its question quotes.
const GIVEN_CONTEXT_CHARS: usize = 400;

type NameFn = Box<dyn Fn(&str) -> String + Send + Sync>;
type OthersFn = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// The log a card's agent is started from: the private board's, or a
/// shared board's.
pub(crate) struct SpawnLog {
    /// `""` for the private board.
    pub board_id: String,
    pub co: Arc<Coord>,
    /// A key's name for the brief.
    pub named: NameFn,
    /// Another person's name for a key that isn't this person's or their
    /// agent's; None for this person's own.
    pub others: OthersFn,
}

impl SpawnLog {
    pub(super) fn private(s: &Services) -> SpawnLog {
        let roster = s.roster.clone();
        SpawnLog {
            board_id: String::new(),
            co: s.coord.clone(),
            named: Box::new(move |k: &str| {
                let n = roster.name_of(k);
                if n.is_empty() { k.to_string() } else { n }
            }),
            others: Box::new(|_| None),
        }
    }

    pub(super) fn board(s: &Services, b: &Arc<BoardLog>) -> SpawnLog {
        let names: Arc<BTreeMap<String, String>> = Arc::new(s.space_names());
        let me = s.person_id();
        let verifier = b.verifier.clone();
        let roster = s.roster.clone();
        let n2 = names.clone();
        SpawnLog {
            board_id: b.id.clone(),
            co: b.coord().clone(),
            named: Box::new(move |k: &str| {
                if let Some(n) = n2.get(k) {
                    return n.clone();
                }
                let n = roster.name_of(k);
                if n.is_empty() { k.to_string() } else { n }
            }),
            others: Box::new(move |by: &str| {
                if by == SWEEPER {
                    return None;
                }
                let p = verifier.key_person(by);
                if p == me {
                    return None;
                }
                Some(
                    names
                        .get(by)
                        .cloned()
                        .unwrap_or_else(|| "another member".into()),
                )
            }),
        }
    }
}

impl Services {
    /// The shared boards' spawning pass: bind started agents, then start
    /// agents for this person's own cards on boards whose switch is on,
    /// under this device's cap.
    pub(crate) fn sweep_board_spawns(&self, sp: &Spawner, cap: i64, running: &mut i64) {
        for b in self.visible_boards() {
            self.bind_board_spawns(sp, &b);
            let co = b.coord().clone();
            // A card given to this person: never by the switch, which is
            // for cards they made; only on their yes (card #573).
            for t in co.given_to_spawn() {
                if *running >= cap {
                    break;
                }
                match self.spawn_board_card(sp, &b, &t) {
                    Ok(true) => *running += 1,
                    Ok(false) => {}
                    Err(e) => logf(format_args!("spawn {} on {}: {e}", t.short_ref(), b.id)),
                }
            }
            if !co.spawning() {
                continue;
            }
            for t in co.cards_to_spawn() {
                // A usage limit holds starts here on every board (card
                // #450), and the cap counts them all.
                if *running >= cap || self.usage_held().is_some() {
                    break;
                }
                // A plain board ask is served by the members' idle agents,
                // as before; a board thread gets its own agent on the
                // poster's device (card #512).
                if t.open_ask().is_some_and(|a| !a.thread) {
                    continue;
                }
                match self.spawn_board_card(sp, &b, &t) {
                    Ok(true) => *running += 1,
                    Ok(false) => {}
                    Err(e) => logf(format_args!("spawn {} on {}: {e}", t.short_ref(), b.id)),
                }
            }
        }
    }

    /// The question's context: who gave it, what it asks, and why the
    /// last yes didn't start one if it didn't.
    fn given_context(&self, log: &SpawnLog, t: &Task) -> String {
        let giver = t
            .given
            .as_ref()
            .map(|g| (log.named)(&g.by))
            .unwrap_or_else(|| "Another member".into());
        let asks = if t.objective.trim().is_empty() {
            &t.title
        } else {
            &t.objective
        };
        let cut: String = asks.chars().take(GIVEN_CONTEXT_CHARS).collect();
        let more = if cut.len() < asks.len() {
            "… (open the card for the rest)"
        } else {
            ""
        };
        let failed = coord::Coord::given_failed(t)
            .map(|why| {
                format!(
                    "The last try couldn't start: {why}. Check its repo is on this computer and \
                     trusted in Settings › Agents, then start it again.\n\n"
                )
            })
            .unwrap_or_default();
        format!(
            "{failed}{giver} gave you this card. It asks: {cut}{more}\n\nIrori stopped here \
             because someone else wrote it. An agent would run on this computer, under your own \
             settings and cap, and treat the card as {giver}'s request."
        )
    }

    /// Stops at each card another person gave this person: it asks them
    /// in Needs you, with who gave it and what it asks (card #573). One of
    /// their devices asks, the one that would start the agent, and only it
    /// starts and binds one. Runs even while paused: it starts nothing.
    pub(crate) fn ask_given_cards(&self) {
        for b in self.visible_boards() {
            let co = b.coord();
            let log = SpawnLog::board(self, &b);
            let me = self.person_actor();
            for t in co.given_to_ask() {
                let first = t.given.as_ref().is_some_and(|g| g.asker.is_empty());
                if first && self.spawn_device(&t).is_none_or(|d| d != self.device.id) {
                    continue;
                }
                let context = self.given_context(&log, &t);
                if let Err(e) = co.ask_given(&me, &t.id, &context) {
                    logf(format_args!("given card {}: {e}", t.short_ref()));
                }
            }
        }
    }

    /// The person asks for an agent on a card given to them after their
    /// Not now: the question comes back on this device, which then starts
    /// it on their yes. False when `id` is no such card.
    pub(crate) fn ask_given_again(&self, id: &str) -> Result<bool, String> {
        for b in self.visible_boards() {
            let co = b.coord();
            let Some(t) = co.get_task(id) else {
                continue;
            };
            if !co.mine_to_serve(&t) || t.given_said.as_ref().is_none_or(|y| y.yes) {
                return Ok(false);
            }
            // The device that asks is the one that starts it: a phone, or a
            // computer without its repo, can't take that on.
            let runs = self
                .spawn
                .spawner
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some();
            if !runs || self.spawn_device(&t).is_none_or(|d| d != self.device.id) {
                return Err(format!(
                    "{} can't start on this device: press Start an agent on the computer with its repo",
                    t.short_ref()
                ));
            }
            let log = SpawnLog::board(self, &b);
            let context = self.given_context(&log, &t);
            co.ask_given(&self.person_actor(), id, &context)
                .map_err(|e| e.to_string())?;
            return Ok(true);
        }
        Ok(false)
    }

    fn spawn_board_card(&self, sp: &Spawner, b: &Arc<BoardLog>, t: &Task) -> Result<bool, String> {
        // An agent's start past the person's limit waits (card #575).
        if self.limit_holds(b.coord(), t).is_some() {
            return Ok(false);
        }
        // Only a card this person serves (they made it, or it was given to
        // them, and they said yes); a thread, only one they started.
        if !b.coord().mine_to_serve(t) {
            return Ok(false);
        }
        // Every member device must read the spawn rows first: an older
        // Irori refuses them and its sync of this board would stall.
        if !self.board_spawns_ready(b) {
            return Ok(false);
        }
        let log = SpawnLog::board(self, b);
        let me = self.device.id.clone();
        let generation = t.spawn_request().map_or(0, |s| s.generation) + 1;
        // A board thread names no repo (a board ask never does): its agent
        // starts where a private thread's does (card #512), and so does a
        // board card that names none (card #600).
        let (repo_key, folder) = if t.repo_key.is_empty() {
            match self.repo_home() {
                Some((k, f)) => (k, Some(f)),
                None => (String::new(), None),
            }
        } else {
            (t.repo_key.clone(), self.checkout_here(&t.repo_key))
        };
        // No copy here, or not trusted, and its last request failed for
        // that: it waits (card #467's rule) until that changes or the
        // person asks again.
        // A yes is the person asking now: a card that can't start says so
        // again and asks again, rather than waiting in silence (card #573).
        let asked_now = log.co.spawn_asked_now(&t.id) || t.given.is_some();
        let failed_here = |no_folder: bool| {
            t.spawn_request().is_some_and(|s| {
                s.op == coord::SPAWN_FAILED
                    && s.device == me
                    && super::spawn_stop::no_folder_reason(&s.reason) == no_folder
                    && (no_folder || s.reason.starts_with(super::spawn::NOT_TRUSTED_PREFIX))
            })
        };
        if !asked_now
            && (folder.is_none() && failed_here(true)
                || folder
                    .as_ref()
                    .is_some_and(|f| failed_here(false) && !(sp.trusted)(std::path::Path::new(f))))
        {
            return Ok(false);
        }
        let name = self.spawn_name(t, generation);
        let comments = log.co.comments(&t.id).unwrap_or_default();
        let named = |k: &str| (log.named)(k);
        let others = |k: &str| (log.others)(k);
        let brief = build_brief(
            t,
            &comments,
            &named,
            &others,
            None,
            None,
            BriefMore::default(),
        );
        let setting = self.spawn_setting(&b.id);
        let perms = spawn_permissions(&setting, &b.id, &repo_key, &brief);
        let ask = SpawnAsk {
            task_id: t.id.clone(),
            device: me,
            repo_key,
            folder: folder.clone().unwrap_or_default(),
            permission_mode: perms.mode,
            name: name.clone(),
            // By the card's weight (card #963); none is normal.
            model: setting.pick_for(t).0,
            effort: setting.pick_for(t).1,
            weight: t.run_level().to_string(),
            advisor: super::spawn::advisor_in(std::path::Path::new(&self.presence.claude_dir())),
            counted: self.start_counts(&log.co, t),
            ..SpawnAsk::default()
        };
        let Some(g) = log
            .co
            .request_spawn(SWEEPER, ask)
            .map_err(|e| e.to_string())?
        else {
            return Ok(false);
        };
        super::spawn::log_asked(t, t.spawn_request(), g, &name, "this device", &(self.now)());
        self.settle_ask(&t.id);
        if t.given.is_some() {
            self.spawn
                .remote
                .board_started
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert((t.id.clone(), g));
        }
        self.start_in(sp, &log, t, g, &name, Some(folder), asked_now)
    }

    /// Binds each board card's agent started here once presence lists it:
    /// the mark in the person's own log first (so their own hand-out and
    /// the agent card know Irori started it), then the agent put on the
    /// board (`card_for`, marked on its agent card), then the board's mark
    /// and the card assigned to its `agent:<cardId>` (§9).
    fn bind_board_spawns(&self, sp: &Spawner, b: &Arc<BoardLog>) {
        if !self.board_spawns_ready(b) {
            return;
        }
        let me = self.device.id.as_str();
        let co = b.coord();
        let live = self.presence.snapshot();
        let open = co.list_tasks(&TaskFilter {
            open: true,
            ..TaskFilter::default()
        });
        for t in open {
            // Only a card this person serves and their own request:
            // another member's request naming this device never binds (or
            // marks) a session here (§7).
            if !co.mine_to_serve(&t) {
                continue;
            }
            // Pending, or ended because the card was held while its agent
            // started (card #570): that session still goes.
            let held = t.parked().is_some();
            let Some(s) = t
                .spawn_request()
                .filter(|s| {
                    (s.pending() || (held && s.op == SPAWN_CANCELED && s.reason == HELD))
                        && s.device == me
                })
                .cloned()
            else {
                continue;
            };
            if !co.mine_to_start(&t, &s) {
                if s.pending() {
                    self.drop_given_request(sp, b, &t, &s, &live);
                }
                continue;
            }
            let Some(a) = live.iter().find(|a| {
                !s.name.is_empty()
                    && a.name == s.name
                    && a.status != "gone"
                    && !co.is_spawned(&a.key)
            }) else {
                continue;
            };
            // A held card gets no agent (card #570): its request ends and
            // the session started for it goes, unmarked.
            if held {
                self.end_held_start(sp, co, &t, &s, &a.key);
                continue;
            }
            if let Err(e) =
                self.coord
                    .mark_spawned_elsewhere(&a.key, &t.id, s.generation, me, &b.id, s.counted)
            {
                logf(format_args!("spawn {}: mark {}: {e}", t.short_ref(), a.key));
                continue;
            }
            let key = match self.board_key(&b.id, &a.key) {
                Ok(k) => k,
                Err(e) => {
                    logf(format_args!(
                        "spawn {}: card for {}: {e}",
                        t.short_ref(),
                        a.key
                    ));
                    continue;
                }
            };
            match co.bind_spawn(&t.id, s.generation, &key) {
                Ok(true) => logf(format_args!(
                    "spawn {} gen {}: {} showed {}s after it was asked; bound to {key}",
                    t.short_ref(),
                    s.generation,
                    s.name,
                    super::spawn::secs_since(&(self.now)(), &s.at)
                )),
                Ok(false) => {}
                Err(e) => logf(format_args!("spawn {}: bind {key}: {e}", t.short_ref())),
            }
        }
    }

    /// Ends held card t's request `generation` and session `key`, started
    /// for it before it was held.
    fn end_held_start(&self, sp: &Spawner, co: &Coord, t: &Task, s: &Spawn, key: &str) {
        if s.pending()
            && let Err(e) = co.end_spawn(SWEEPER, &t.id, s.generation, SPAWN_CANCELED, HELD)
        {
            logf(format_args!("spawn {}: end request: {e}", t.short_ref()));
            return;
        }
        self.spawn
            .remote
            .board_started
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(t.id.clone(), s.generation));
        let sess = key.rsplit_once('/').map_or(key, |(_, s)| s);
        match (sp.end)(sess) {
            Ok(_) => logf(format_args!(
                "spawn {}: ended {key}, the card is held",
                t.short_ref()
            )),
            Err(e) => logf(format_args!("spawn {}: end {key}: {e}", t.short_ref())),
        }
    }

    /// A request on a card given to this person that their yes no longer
    /// covers: the card changed, or it was never theirs. It ends, so the
    /// card can be asked about again, and a session this device started
    /// for it goes. Never a session it didn't start: the name came on a row.
    fn drop_given_request(
        &self,
        sp: &Spawner,
        b: &Arc<BoardLog>,
        t: &Task,
        s: &coord::Spawn,
        live: &[presence::AgentCard],
    ) {
        if t.given.is_none() {
            return;
        }
        let why = "the yes for this agent no longer stands";
        if let Err(e) =
            b.coord()
                .end_spawn(SWEEPER, &t.id, s.generation, coord::SPAWN_CANCELED, why)
        {
            logf(format_args!("spawn {}: cancel: {e}", t.short_ref()));
            return;
        }
        let started = self
            .spawn
            .remote
            .board_started
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(t.id.clone(), s.generation));
        if !started {
            return;
        }
        if let Some(a) = live.iter().find(|a| a.name == s.name && a.status != "gone") {
            let sess = a.key.rsplit_once('/').map_or(a.key.as_str(), |(_, x)| x);
            if let Err(e) = (sp.end)(sess) {
                logf(format_args!("spawn {}: end {}: {e}", t.short_ref(), a.key));
            }
        }
    }

    /// The card spawned agent `key` (a session) was started for, from
    /// whichever log holds it: the private board's, or a shared board's,
    /// with the agent's key there (`agent:<cardId>` on a board).
    pub(crate) fn spawn_card_of(&self, key: &str) -> Option<(Arc<Coord>, Task, String)> {
        let m = self.coord.spawned(key)?;
        if m.board.is_empty() {
            let t = self.coord.get_task(&m.task_id)?;
            return Some((self.coord.clone(), t, key.to_string()));
        }
        // The board the mark names, only; its card there must be this
        // person's, and the agent already on that board (no card minted).
        if device_of(key) != self.device.id {
            return None;
        }
        let b = self.board(&m.board).ok()?;
        let t = b.coord().get_task(&m.task_id)?;
        if !b.coord().mine_to_serve(&t) {
            return None;
        }
        let id = super::boards::lock(&b.maps).keys.get(key).cloned()?;
        Some((
            b.coord().clone(),
            t,
            format!("{}{id}", store::kinds::AGENT_ACTOR_PREFIX),
        ))
    }

    /// The person's Start an agent on their own waiting card or thread on
    /// a shared board (card #576): it ends a usage hold here, so the card
    /// tries now. The window offers it on the card's waiting line, on the
    /// device that made the card, which is the device its agent starts on;
    /// the card is waiting for an agent already, and takes no note.
    pub(crate) fn start_board_agent(&self, id: &str) -> Result<(), String> {
        let (b, t) = self
            .shown_boards()
            .into_iter()
            .find_map(|b| {
                let t = b.coord().get_task(id)?;
                Some((b, t))
            })
            .ok_or_else(|| format!("no card {id}"))?;
        let co = b.coord();
        if b.verifier.key_person(&t.created_by) != self.person_id() {
            return Err(format!("{} is another member's card", t.short_ref()));
        }
        if !t.holders.is_empty() {
            return Err(format!("{} has an agent on it", t.short_ref()));
        }
        if !co.hands_out_here(id) {
            return Err(format!(
                "{} starts on the computer that made it: start it there",
                t.short_ref()
            ));
        }
        co.spawn_now(id);
        self.release_usage_hold();
        self.asks_kick.kick();
        Ok(())
    }

    /// Whether every member device of board b reads the spawn rows
    /// (`spawns` in its board hello, card #455). No mesh for it (stopped,
    /// or no network): not ready.
    pub(crate) fn board_spawns_ready(&self, b: &BoardLog) -> bool {
        self.mesh
            .board(&b.id)
            .is_some_and(|m| m.not_ready_for_spawns().is_empty())
    }
}
