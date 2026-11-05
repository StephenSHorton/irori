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

use std::collections::BTreeMap;
use std::sync::Arc;

use coord::roster::Agents;
use coord::{Coord, SpawnAsk, Task, TaskFilter};
use irori_identity::owner::device_of;

use super::Services;
use super::boards::BoardLog;
use super::spawn::{BriefMore, Spawner, build_brief, spawn_permissions};
use crate::logf;

const SWEEPER: &str = "irori";

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
        for b in self.shown_boards() {
            self.bind_board_spawns(&b);
            let co = b.coord().clone();
            if !co.spawning() {
                continue;
            }
            for t in co.cards_to_spawn() {
                if *running >= cap {
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

    fn spawn_board_card(&self, sp: &Spawner, b: &Arc<BoardLog>, t: &Task) -> Result<bool, String> {
        // Only a card this person made (its device hands it out, and its
        // author is this person); a thread, only one this person started.
        if b.verifier.key_person(&t.created_by) != self.person_id() {
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
        // starts where a private thread's does (card #512).
        let (repo_key, folder) = if t.repo_key.is_empty() && t.open_ask().is_some() {
            match self.ask_home() {
                Some((k, f)) => (k, Some(f)),
                None => (String::new(), None),
            }
        } else {
            (t.repo_key.clone(), self.checkout_here(&t.repo_key))
        };
        // No copy here, or not trusted, and its last request failed for
        // that: it waits (card #467's rule) until that changes or the
        // person asks again.
        let asked_now = log.co.spawn_asked_now(&t.id);
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
            model: setting.model().into(),
            effort: setting.effort().into(),
            ..SpawnAsk::default()
        };
        let Some(g) = log
            .co
            .request_spawn(SWEEPER, ask)
            .map_err(|e| e.to_string())?
        else {
            return Ok(false);
        };
        self.start_in(sp, &log, t, g, &name, Some(folder))
    }

    /// Binds each board card's agent started here once presence lists it:
    /// the mark in the person's own log first (so their own hand-out and
    /// the agent card know Irori started it), then the agent put on the
    /// board (`card_for`, marked on its agent card), then the board's mark
    /// and the card assigned to its `agent:<cardId>` (§9).
    fn bind_board_spawns(&self, b: &Arc<BoardLog>) {
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
            // Only this person's own card and request: another member's
            // request naming this device never binds (or marks) a session
            // here (§7).
            if b.verifier.key_person(&t.created_by) != self.person_id() {
                continue;
            }
            let Some(s) = t
                .spawn_request()
                .filter(|s| {
                    s.pending()
                        && s.device == me
                        && (s.by == SWEEPER || b.verifier.key_person(&s.by) == self.person_id())
                })
                .cloned()
            else {
                continue;
            };
            let Some(a) = live.iter().find(|a| {
                !s.name.is_empty()
                    && a.name == s.name
                    && a.status != "gone"
                    && !co.is_spawned(&a.key)
            }) else {
                continue;
            };
            if let Err(e) =
                self.coord
                    .mark_spawned_elsewhere(&a.key, &t.id, s.generation, me, &b.id)
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
            if let Err(e) = co.bind_spawn(&t.id, s.generation, &key) {
                logf(format_args!("spawn {}: bind {key}: {e}", t.short_ref()));
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
        if b.verifier.key_person(&t.created_by) != self.person_id() {
            return None;
        }
        let id = super::boards::lock(&b.maps).keys.get(key).cloned()?;
        Some((
            b.coord().clone(),
            t,
            format!("{}{id}", store::kinds::AGENT_ACTOR_PREFIX),
        ))
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
