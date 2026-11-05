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
use super::spawn::{Spawner, build_brief, spawn_permissions};
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
                // Board asks and threads are served by idle agents, as
                // before (part 10 covers the private board's).
                if t.open_ask().is_some() {
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
        // author is this person).
        if b.verifier.key_person(&t.created_by) != self.person_id() {
            return Ok(false);
        }
        let log = SpawnLog::board(self, b);
        let me = self.device.id.clone();
        let generation = t.spawn_request().map_or(0, |s| s.generation) + 1;
        let name = self.spawn_name(t, generation);
        let folder = self.checkout_here(&t.repo_key);
        let comments = log.co.comments(&t.id).unwrap_or_default();
        let named = |k: &str| (log.named)(k);
        let others = |k: &str| (log.others)(k);
        let brief = build_brief(t, &comments, &named, &others, None, None);
        let setting = self.spawn_setting(&b.id);
        let perms = spawn_permissions(&setting, &b.id, &t.repo_key, &brief);
        let ask = SpawnAsk {
            task_id: t.id.clone(),
            device: me,
            repo_key: t.repo_key.clone(),
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
        let me = self.device.id.as_str();
        let co = b.coord();
        let live = self.presence.snapshot();
        let open = co.list_tasks(&TaskFilter {
            open: true,
            ..TaskFilter::default()
        });
        for t in open {
            let Some(s) = t
                .spawn_request()
                .filter(|s| s.pending() && s.device == me)
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
            if let Err(e) = self
                .coord
                .mark_spawned_elsewhere(&a.key, &t.id, s.generation, me)
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
        if let Some(t) = self.coord.get_task(&m.task_id) {
            return Some((self.coord.clone(), t, key.to_string()));
        }
        if device_of(key) != self.device.id {
            return None;
        }
        for b in self.shown_boards() {
            if let Some(t) = b.coord().get_task(&m.task_id) {
                let k = self.board_key(&b.id, key).ok()?;
                return Some((b.coord().clone(), t, k));
            }
        }
        None
    }
}
