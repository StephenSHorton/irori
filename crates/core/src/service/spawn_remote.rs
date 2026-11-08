//! Spawned agents across the person's devices (docs/design/spawned-agents.md
//! §7, §9, decision 7 (a); card #455).
//!
//! The device that hands a card out (the one it was made on) picks the
//! device its agent starts on and writes the `task.spawn` request naming
//! it. Each device publishes what it can start agents in (`spawn.host`:
//! its checkouts, which of them Claude Code trusts, its OS), only when that
//! changes, so the choice can see the others; a device that never published
//! one (an older Irori, the phone, a home server) is never picked. The
//! named device starts the agent only for its own person's card and only
//! while the request is the card's newest and fresh
//! ([`coord::Coord::spawns_here`]); one that comes back online with an older
//! request starts nothing, because the card's device has moved on to the
//! next one after the grace. If two sessions start anyway, the one bound to
//! the card stays and the other is ended.
//!
//! Three person-only requests travel the private log too: End for an agent
//! another of their devices runs, "start an agent now" from a device that
//! doesn't hand the card out (with the person's words for its brief), and
//! "trust this repo" for the device the agent must start on, which only
//! asks the person in that device's own window. None of them is written by
//! a tool, and each is acted on only by the device it is for, only when its
//! writer is the person on one of their own devices.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use coord::roster::Agents;
use coord::{SPAWN_CANCELED, SPAWN_FAILED, Spawn, SpawnAsk, Task};
use gocompat::time::GoTime;
use irori_identity::owner::device_of;
use serde::{Deserialize, Serialize};
use store::{Event, Filter, kinds};

use super::Services;
use super::spawn::{BriefMore, NOT_TRUSTED_PREFIX, Spawner, build_brief, spawn_permissions};
use crate::logf;

const SWEEPER: &str = "irori";

/// How often a device looks again at what it can start agents in.
pub const HOST_EVERY: Duration = Duration::from_secs(60);

/// One repo a device can start agents in.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostRepo {
    pub repo_key: String,
    pub folder: String,
    #[serde(default)]
    pub trusted: bool,
}

/// A `spawn.host` row: what device `device` can start agents in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnHost {
    pub device: String,
    /// `std::env::consts::OS` there: "windows", "macos", "linux".
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub repos: Vec<HostRepo>,
    /// It can't run agents (card #541): a phone, or a computer without
    /// Claude Code. Older rows have no such field and could.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_agents: bool,
    /// [`coord::HOST_MOVES`] from an Irori that reads `ask.moved` and
    /// stands in for a phone (card #541); always written, so a row without
    /// it is an older Irori's, which is never picked to take phone work.
    #[serde(default)]
    pub moves: i64,
    #[serde(default)]
    pub at: GoTime,
    /// A Claude usage limit holds starts on this device until then (card
    /// #450): it isn't asked to start anything meanwhile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_until: Option<GoTime>,
}

impl SpawnHost {
    /// "pc" or "mac", as a card's `needs:` hint names them; "" otherwise.
    pub fn machine(&self) -> &'static str {
        match self.os.as_str() {
            "windows" => "pc",
            "macos" => "mac",
            _ => "",
        }
    }

    pub fn repo(&self, repo_key: &str) -> Option<&HostRepo> {
        self.repos
            .iter()
            .find(|r| coord::canonical_repo_key(&r.repo_key) == coord::canonical_repo_key(repo_key))
    }
}

/// What a `spawn.host` row last said: the OS, repos, no_agents, moves and
/// usage hold.
type HostShown = (String, Vec<HostRepo>, bool, i64, Option<GoTime>);

/// A `spawn.end.requested` row.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EndRequest {
    pub key: String,
    #[serde(default)]
    pub task_id: String,
    /// The mark's generation the person saw: an older one is ignored
    /// (compared by generation, not by two devices' clocks).
    #[serde(default, rename = "gen")]
    pub generation: i64,
    pub by: String,
    #[serde(default)]
    pub at_device: String,
    #[serde(default)]
    pub at: GoTime,
}

/// A `spawn.start.requested` row.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRequest {
    pub task_id: String,
    /// The card's newest request the person saw: a newer one serves it.
    #[serde(default, rename = "gen")]
    pub generation: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// An agent asked, not the person (card #575): the Core wrote the row
    /// after its own checks, and the start counts toward their limit.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub agent: bool,
    pub by: String,
    #[serde(default)]
    pub at_device: String,
    #[serde(default)]
    pub at: GoTime,
}

/// A `trust.requested` row.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustRequest {
    pub task_id: String,
    #[serde(rename = "gen")]
    pub generation: i64,
    pub device: String,
    #[serde(default)]
    pub repo_key: String,
    #[serde(default)]
    pub folder: String,
    pub by: String,
    #[serde(default)]
    pub at_device: String,
    #[serde(default)]
    pub at: GoTime,
}

/// What the Services keep for spawning across devices.
#[derive(Default)]
pub(crate) struct RemoteState {
    /// When this device last looked at what it can start agents in, and
    /// what it last published.
    host_at: Mutex<Option<GoTime>>,
    host_last: Mutex<Option<HostShown>>,
    /// Devices asked to start a card's agent, and when: the next request
    /// names another device first (§9 "the next device that has the repo").
    tried: Mutex<HashMap<String, HashMap<String, GoTime>>>,
    /// Requests started on this device, by (card, generation): the name it
    /// was started with. Dropped once bound, or ended when superseded.
    started: Mutex<HashMap<(String, i64), String>>,
    /// Board requests this device started, for a given card whose yes went
    /// before its agent showed (card #573).
    pub(super) board_started: Mutex<std::collections::HashSet<(String, i64)>>,
    /// Request rows (end, start, trust) already acted on, by (device, seq).
    seen: Mutex<HashSet<(String, i64)>>,
    /// Cards another device asked this one to trust the repo of, and which
    /// device asked.
    trust_asked: Mutex<HashMap<String, String>>,
    /// The requests naming this device its log held when the Core started
    /// (before any sync): never started (one may already have been, before
    /// the restart, and one from before an absence is the card's device's
    /// to replace with the next generation).
    held_at_start: Mutex<Option<HashSet<(String, i64)>>>,
    /// Cards the person asked for that this device sent to another one, by
    /// the request's generation (card #576): writing the request spent
    /// `spawn_now`, so if a usage limit stops it there, this puts it back.
    /// In memory, as `spawn_now` is.
    asked_elsewhere: Mutex<HashMap<String, i64>>,
}

fn at_least(now: &GoTime, at: &GoTime, d: Duration) -> bool {
    i128::from(now.unix_nano()) - i128::from(at.unix_nano()) >= d.as_nanos() as i128
}

/// The cap a device starts with when the person hasn't set one (§6): 6 on
/// Windows, 12 elsewhere, by that device's OS.
fn default_cap_for(os: &str) -> i64 {
    if os == "windows" { 6 } else { 12 }
}

impl Services {
    // ---- What this device can start agents in ------------------------

    /// Publishes what this device can start agents in, when it changed
    /// since it last did (never on a timer: card #467's row flood).
    pub(crate) fn publish_host(&self, sp: Option<&Spawner>, ready: bool) {
        let now = (self.now)();
        {
            let mut at = self
                .spawn
                .remote
                .host_at
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if at.as_ref().is_some_and(|a| !at_least(&now, a, HOST_EVERY)) {
                return;
            }
            *at = Some(now);
        }
        let os = std::env::consts::OS.to_string();
        let no_agents = !ready;
        let mut repos: Vec<HostRepo> = self
            .capture
            .repos()
            .into_keys()
            .filter_map(|k| {
                let sp = sp?;
                let folder = self.checkout_here(&k)?;
                let trusted = (sp.trusted)(Path::new(&folder));
                Some(HostRepo {
                    repo_key: coord::canonical_repo_key(&k),
                    folder,
                    trusted,
                })
            })
            .collect();
        repos.sort();
        repos.dedup_by(|a, b| a.repo_key == b.repo_key);
        let mut last = self
            .spawn
            .remote
            .host_last
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if last.is_none() {
            // After a restart: what this device published before.
            *last = self
                .hosts()
                .remove(&self.device.id)
                .map(|h| (h.os, h.repos, h.no_agents, h.moves, h.held_until));
        }
        let moves = coord::HOST_MOVES;
        let held = self.usage_held();
        if last.as_ref() == Some(&(os.clone(), repos.clone(), no_agents, moves, held)) {
            return;
        }
        let row = SpawnHost {
            device: self.device.id.clone(),
            os: os.clone(),
            repos: repos.clone(),
            no_agents,
            moves,
            at: now,
            held_until: held,
        };
        match self.store.append(SWEEPER, kinds::SPAWN_HOST, &row) {
            Ok(_) => *last = Some((os, repos, no_agents, moves, held)),
            Err(e) => logf(format_args!("spawn host: {e}")),
        }
    }

    /// Looks again at what this device can start agents in at the next
    /// sweep (the person trusted a repo, or the spawner changed).
    pub(crate) fn republish_host(&self) {
        *self
            .spawn
            .remote
            .host_at
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// The newest `spawn.host` of each of the person's devices, by device.
    pub(crate) fn hosts(&self) -> HashMap<String, SpawnHost> {
        let evs = self
            .store
            .list(&Filter {
                kinds: vec![kinds::SPAWN_HOST.into()],
                desc: true,
                limit: store::MAX_LIST_LIMIT,
                ..Filter::default()
            })
            .unwrap_or_default();
        let mut out: HashMap<String, SpawnHost> = HashMap::new();
        for ev in evs {
            if ev.actor != SWEEPER || out.contains_key(&ev.device_id) {
                continue;
            }
            let Ok(h) = serde_json::from_slice::<SpawnHost>(&ev.payload) else {
                continue;
            };
            // Only a device's own row about itself, from one of the
            // person's devices.
            if h.device != ev.device_id || !self.chain.is_owner(&format!("user:{}", h.device)) {
                continue;
            }
            out.insert(ev.device_id.clone(), h);
        }
        out
    }

    /// Device `dev`'s cap: the person's, else its OS's default.
    pub(crate) fn cap_on(&self, dev: &str, host: Option<&SpawnHost>) -> i64 {
        if let Some(c) = self
            .spawn
            .caps
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(dev)
        {
            return *c;
        }
        if dev == self.device.id {
            return super::spawn::default_cap();
        }
        default_cap_for(host.map(|h| h.os.as_str()).unwrap_or(""))
    }

    /// Spawned agents live on device `dev` and requests starting there.
    pub(crate) fn spawned_on(&self, dev: &str) -> i64 {
        if dev == self.device.id {
            return self.spawned_here();
        }
        let live = self
            .roster
            .agents()
            .into_iter()
            .filter(|a| {
                let d = if a.device_id.is_empty() {
                    device_of(&a.key)
                } else {
                    a.device_id.as_str()
                };
                d == dev && a.status != "gone" && self.coord.is_spawned(&a.key)
            })
            .count() as i64;
        let starting = self
            .coord
            .list_tasks(&coord::TaskFilter {
                open: true,
                ..coord::TaskFilter::default()
            })
            .iter()
            .filter(|t| {
                t.spawn_request()
                    .is_some_and(|s| s.pending() && s.device == dev)
            })
            .count() as i64;
        live + starting
    }

    // ---- Choosing the device (decision 7 (a)) -------------------------

    /// The device card t's next agent starts on, or None to wait (§9,
    /// decision 7 (a)): among the person's devices that published a
    /// checkout of its repo and aren't offline, the one the card needs
    /// (`needs:mac`/`needs:pc`); then one with the repo trusted; one below
    /// its cap; the one with the card's own checkout (its Where); the PC,
    /// which stays on; this device. A device asked lately for this card
    /// goes last, so the next request names the next device; a card that
    /// needs a machine never moves off it. Asks and helpers start here,
    /// and so does a card nobody published a checkout of. A card that
    /// names no repo starts here too, in the ask home (card #600); on a
    /// device with no checkout of any repo it waits here, unasked. Never a
    /// device that can't run agents (card #541): this one waits instead,
    /// and is left out of the choice.
    pub(crate) fn spawn_device(&self, t: &Task) -> Option<String> {
        let me = self.device.id.clone();
        // Pause agents here (card #665): nothing is started here, and a
        // card only this device would run waits.
        let here = self.can_run_here() && !self.agents_paused();
        if t.open_ask().is_some() || (t.spawn && !t.parent.is_empty()) || t.repo_key.is_empty() {
            return here.then_some(me);
        }
        let hosts = self.hosts();
        let cant = self.cant_run_devices();
        let need = coord::machine_needed(t);
        let rows = self.mesh.node.devices(false);
        // (may start agents, is online)
        let state = |d: &str| -> (bool, bool) {
            match rows.iter().find(|r| r.device_id == d) {
                Some(r) => (
                    !r.revoked
                        && (r.role.is_empty() || r.role == irori_identity::objects::ROLE_DESKTOP),
                    r.this || r.state != "offline",
                ),
                None => (d == me, d == me),
            }
        };
        let now = (self.now)();
        let tried = self
            .spawn
            .remote
            .tried
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&t.id)
            .cloned()
            .unwrap_or_default();
        let lately = |d: &str| {
            tried
                .get(d)
                .is_some_and(|at| !at_least(&now, at, coord::SPAWN_RETRY))
        };
        let where_dev = t
            .where_
            .as_ref()
            .map(|w| device_of(&w.who).to_string())
            .unwrap_or_default();
        let mut cands: Vec<(Vec<bool>, String)> = Vec::new();
        let mut waits_for_machine = false;
        let mut remote_full = false;
        let asked_now = self.coord.spawn_asked_now(&t.id);
        for (dev, h) in &hosts {
            let Some(r) = h.repo(&t.repo_key) else {
                continue;
            };
            let (usable, online) = state(dev);
            if !usable || cant.contains(dev) {
                continue;
            }
            // Pause agents on that device (card #665): never asked; the card
            // waits for it, or starts on another device that has its repo.
            if self.agents_paused_on(dev) {
                waits_for_machine = true;
                continue;
            }
            // Only a device that runs the card's side (its device.sides
            // pin; no pin runs every side), unless the person asked for
            // this agent now (card #495's rule, across devices).
            if !asked_now && !self.coord.runs_side(dev, &t.side) {
                continue;
            }
            if !need.is_empty() && h.machine() != need {
                continue;
            }
            // Offline: the card waits for it (its request moves to it once
            // it is back) rather than start where there is no copy.
            if !online {
                waits_for_machine = true;
                continue;
            }
            let held = *dev != me && h.held_until.is_some_and(|u| u.secs > now.secs);
            let free = !held && self.spawned_on(dev) < self.cap_on(dev, Some(h));
            // A full device elsewhere isn't asked: the card waits for a
            // free slot rather than for a request that goes stale.
            if !free && *dev != me {
                remote_full = true;
                continue;
            }
            // Its newest request failed on this device for a reason its
            // host row still shows (the repo untrusted, or no copy there):
            // not asked again until that changes or the person asks
            // (card #467's rule, across devices).
            if *dev != me && !asked_now && failed_for_reason(t, dev, r) {
                continue;
            }
            let key = vec![
                need.is_empty() && lately(dev),
                !r.trusted,
                !free,
                *dev != where_dev,
                h.machine() != "pc",
                *dev != me,
            ];
            cands.push((key, dev.clone()));
        }
        cands.sort();
        let Some((_, dev)) = cands.into_iter().next() else {
            if waits_for_machine || remote_full {
                return None;
            }
            // Its newest request failed on another device that is the only
            // one with the repo: it waits there, as on one device.
            if t.spawn_request()
                .is_some_and(|s| s.op == SPAWN_FAILED && s.device != me)
                && !asked_now
                && hosts
                    .values()
                    .any(|h| h.repo(&t.repo_key).is_some() && h.device != me)
                && hosts.get(&me).is_none_or(|h| h.repo(&t.repo_key).is_none())
            {
                return None;
            }
            // Only computers without Claude Code have the repo: it waits
            // for them, and says so, rather than fail here for no copy.
            if !self.repo_waits_on(t).1.is_empty() {
                return None;
            }
            return here.then_some(me);
        };
        // The best device was asked lately (it failed, or never answered
        // and the request went stale): wait for the retry rather than ask
        // it again every grace (card #467's row flood).
        if dev != me && lately(&dev) && !asked_now {
            return None;
        }
        Some(dev)
    }

    pub(crate) fn note_tried(&self, id: &str, dev: &str) {
        self.spawn
            .remote
            .tried
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(id.to_string())
            .or_default()
            .insert(dev.to_string(), (self.now)());
    }

    /// Asks device `dev` to start card t's agent: the request names it,
    /// and that device does the rest ([`Self::start_requested`]).
    pub(crate) fn request_on(&self, t: &Task, dev: &str) -> Result<bool, String> {
        // Read before the request spends it. A stale request of his that is
        // asked again stays his.
        let asked = self.coord.spawn_asked_now(&t.id)
            || self
                .spawn
                .remote
                .asked_elsewhere
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(&t.id);
        let generation = t.spawn_request().map_or(0, |s| s.generation) + 1;
        let name = self.spawn_name(t, generation);
        let setting = self.spawn_setting("");
        let comments = self.coord.comments(&t.id).unwrap_or_default();
        let named = |k: &str| {
            let n = self.roster.name_of(k);
            if n.is_empty() { k.to_string() } else { n }
        };
        let brief = build_brief(
            t,
            &comments,
            &named,
            &|_| None,
            None,
            None,
            BriefMore::default(),
        );
        let perms = spawn_permissions(&setting, "", &t.repo_key, &brief);
        // The person's words for its brief go with the request: the device
        // that starts it has no other way to them (card #455).
        let note = self
            .spawn
            .answers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&t.id)
            .cloned()
            .unwrap_or_default();
        let name_logged = name.clone();
        let ask = SpawnAsk {
            task_id: t.id.clone(),
            device: dev.to_string(),
            repo_key: t.repo_key.clone(),
            permission_mode: perms.mode,
            name,
            // By the card's weight (card #963); none is normal.
            model: setting.pick_for(t).0,
            effort: setting.pick_for(t).1,
            weight: t.run_level().to_string(),
            advisor: super::spawn::advisor_in(std::path::Path::new(&self.presence.claude_dir())),
            note,
            counted: self.start_counts(&self.coord, t),
            others: brief.others_text,
            ..SpawnAsk::default()
        };
        let out = self
            .coord
            .request_spawn(SWEEPER, ask)
            .map_err(|e| e.to_string())?;
        if let Some(g) = out {
            let on = format!("device {dev}");
            super::spawn::log_asked(t, t.spawn_request(), g, &name_logged, &on, &(self.now)());
            self.settle_ask(&t.id);
            self.note_tried(&t.id, dev);
            self.spawn
                .answers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&t.id);
            if asked {
                self.spawn
                    .remote
                    .asked_elsewhere
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(t.id.clone(), g);
            }
        }
        Ok(out.is_some())
    }

    /// The person's "Start an agent" still stands after a usage limit
    /// (card #576): a request of his that ended on one on another device
    /// asks again here, as a local start does. Run before the queue is
    /// walked. One bound, failed otherwise or replaced is forgotten.
    pub(crate) fn renew_asks_sent_elsewhere(&self) {
        self.spawn
            .remote
            .asked_elsewhere
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|id, g| {
                let t = self.coord.get_task(id);
                let Some(s) = t
                    .as_ref()
                    .and_then(|t| t.spawn_request())
                    .filter(|s| s.generation == *g)
                else {
                    return false;
                };
                if coord::usage_limited(s) {
                    self.coord.spawn_now(id);
                    return false;
                }
                s.pending()
            });
    }

    // ---- The device a request names ----------------------------------

    /// Starts the agents another of the person's devices asked this one
    /// for, under this device's cap. The count started.
    pub(crate) fn start_requested(&self, sp: &Spawner, running: &mut i64) -> usize {
        let cap = self.spawn_cap(&self.device.id);
        let mut n = 0;
        let names: HashSet<String> = self
            .presence
            .snapshot()
            .into_iter()
            .filter(|a| a.status != "gone")
            .map(|a| a.name)
            .collect();
        let here = self.coord.spawns_here();
        // Each requested name is held here too, so no terminal session
        // started meanwhile is given it (and then bound as the agent).
        let now = (self.now)();
        for (t, s) in &here {
            self.names.set_entry(
                &format!("{}{}/{}", crate::namer::SPAWN_PREFIX, t.id, s.generation),
                &s.name,
                now,
            );
        }
        let held = self
            .spawn
            .remote
            .held_at_start
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_default();
        let todo: Vec<(Task, Spawn)> = here
            .into_iter()
            .filter(|(t, s)| {
                !self.was_started(&t.id, s.generation)
                    && !names.contains(&s.name)
                    && !held.contains(&(t.id.clone(), s.generation))
            })
            .collect();
        // `running` already counts these as starting here.
        *running -= todo.len() as i64;
        for (t, s) in todo {
            if *running >= cap {
                *running += 1;
                continue;
            }
            // A usage limit holds starts here (card #450): the request goes
            // back to the queue, not left to go stale. The asking device
            // sees the hold on this device's host row and asks elsewhere.
            if let Some(until) = self.usage_held() {
                let why = super::spawn_limits::usage_reason(&until);
                logf(format_args!(
                    "spawn {} gen {}: held by a usage limit until {}",
                    t.short_ref(),
                    s.generation,
                    until.format_rfc3339()
                ));
                if let Err(e) =
                    self.coord
                        .end_spawn(SWEEPER, &t.id, s.generation, coord::SPAWN_FAILED, &why)
                {
                    logf(format_args!("spawn {}: {e}", t.short_ref()));
                }
                continue;
            }
            match self.start_here(sp, &t, s.generation, &s.name, None, false) {
                Ok(true) => {
                    *running += 1;
                    n += 1;
                }
                Ok(false) => {}
                Err(e) => logf(format_args!("spawn {}: {e}", t.short_ref())),
            }
        }
        n
    }

    /// Notes the requests naming this device its log holds now, as the
    /// Core starts: none of them is ever started here.
    pub(crate) fn hold_requests_at_start(&self) {
        let held = self
            .coord
            .spawns_here()
            .into_iter()
            .map(|(t, s)| (t.id, s.generation))
            .collect();
        *self
            .spawn
            .remote
            .held_at_start
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(held);
    }

    pub(crate) fn was_started(&self, id: &str, generation: i64) -> bool {
        self.spawn
            .remote
            .started
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&(id.to_string(), generation))
    }

    pub(crate) fn note_started_as(&self, id: &str, generation: i64, name: &str) {
        self.spawn
            .remote
            .started
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((id.to_string(), generation), name.to_string());
    }

    /// Sessions this device started for a request that is no longer the
    /// card's (a newer generation named another device, or it was
    /// canceled) and never bound: ended, so the card keeps one agent
    /// (§9 "if two sessions start anyway").
    pub(crate) fn end_superseded(&self, sp: &Spawner) {
        self.end_lost_ties(sp);
        let started: Vec<((String, i64), String)> = self
            .spawn
            .remote
            .started
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if started.is_empty() {
            return;
        }
        let live = self.presence.snapshot();
        for ((id, generation), name) in started {
            let card = self.coord.get_task(&id);
            let current = card.as_ref().and_then(|t| t.spawn_request().cloned());
            // Held while it was starting (card #570): the session may show
            // up yet, so it is watched for as long as the card is held.
            let held = card.as_ref().is_some_and(|t| t.parked().is_some())
                && current
                    .as_ref()
                    .is_some_and(|s| s.generation == generation && s.op == SPAWN_CANCELED);
            let mine: Option<&Spawn> = current
                .as_ref()
                .filter(|s| s.generation == generation && s.op == coord::SPAWN_REQUESTED)
                // Another computer took the same generation (card #541).
                .filter(|s| s.device.is_empty() || s.device == self.device.id);
            let session = live
                .iter()
                .find(|a| a.name == name && a.status != "gone")
                .map(|a| a.key.clone());
            match (mine, session) {
                // Still the card's and not bound yet: wait for presence.
                (Some(s), _) if s.key.is_empty() => continue,
                // Bound: nothing more to watch.
                (Some(_), _) => {}
                // No longer the card's and running unbound: end it.
                (None, Some(key)) if !self.coord.is_spawned(&key) => {
                    let sess = key.rsplit_once('/').map_or(key.as_str(), |(_, s)| s);
                    match (sp.end)(sess) {
                        Ok(_) => logf(format_args!(
                            "spawn {id}: ended {key}, started for request {generation} which is no longer the card's"
                        )),
                        Err(e) => {
                            logf(format_args!("spawn {id}: end superseded {key}: {e}"));
                            continue;
                        }
                    }
                }
                (None, Some(_)) => {}
                (None, None) if held => continue,
                // Failed or canceled before it showed: nothing runs.
                (None, None) => {}
            }
            self.spawn
                .remote
                .started
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&(id, generation));
        }
    }

    /// This device's live, bound agents whose card's request for the same
    /// generation is now another computer's (card #541: two took one card
    /// at once, and the lowest id won): ended, so the card keeps one. Read
    /// from the marks, not from what this run started, so it holds after a
    /// restart too. Only the request that is still starting wins: if it
    /// failed, the agent that did start keeps the card.
    fn end_lost_ties(&self, sp: &Spawner) {
        let me = self.device.id.clone();
        for a in self.presence.snapshot() {
            if a.device_id != me || a.status == "gone" {
                continue;
            }
            let Some(m) = self.coord.spawned(&a.key) else {
                continue;
            };
            if m.device != me || !m.step.is_empty() || !m.subtask.is_empty() {
                continue;
            }
            let co = if m.board.is_empty() {
                self.coord.clone()
            } else {
                match self.board(&m.board) {
                    Ok(b) => b.coord().clone(),
                    // A board it can't read now says nothing.
                    Err(_) => continue,
                }
            };
            let Some(t) = co.get_task(&m.task_id) else {
                continue;
            };
            let lost = !coord::is_terminal(&t.state)
                && t.spawn_request().is_some_and(|s| {
                    s.generation == m.generation
                        && s.op == coord::SPAWN_REQUESTED
                        && !s.device.is_empty()
                        && s.device != me
                });
            if !lost {
                continue;
            }
            let sess = a.key.rsplit_once('/').map_or(a.key.as_str(), |(_, s)| s);
            match (sp.end)(sess) {
                Ok(_) => logf(format_args!(
                    "spawn {}: ended {}, another computer took request {}",
                    m.task_id, a.key, m.generation
                )),
                Err(e) => logf(format_args!("spawn {}: end {}: {e}", m.task_id, a.key)),
            }
        }
    }

    // ---- End, start-now and trust requests -----------------------------

    /// Request rows of `kind` this device hasn't acted on yet, oldest
    /// first.
    fn unseen_requests(&self, kind: &str) -> Vec<Event> {
        let evs = self
            .store
            .list(&Filter {
                kinds: vec![kind.into()],
                desc: true,
                limit: 200,
                ..Filter::default()
            })
            .unwrap_or_default();
        let seen = self
            .spawn
            .remote
            .seen
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut v: Vec<Event> = evs
            .into_iter()
            .filter(|ev| !seen.contains(&(ev.device_id.clone(), ev.seq)))
            .collect();
        v.reverse();
        v
    }

    fn mark_seen(&self, ev: &Event) {
        self.spawn
            .remote
            .seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((ev.device_id.clone(), ev.seq));
    }

    /// Whether request row ev, naming `by`, came from the person on one of
    /// their own devices (§7): actor `user`, `by` exactly the writing
    /// device's person key, and a device of their chain. Anything else is
    /// ignored and logged.
    fn by_person(&self, ev: &Event, by: &str) -> bool {
        let ok =
            ev.actor == "user" && by == format!("user:{}", ev.device_id) && self.chain.is_owner(by);
        if !ok {
            logf(format_args!(
                "spawn: ignored {} {}#{} by {by:?} (actor {:?}): not the person on one of their devices",
                ev.kind, ev.device_id, ev.seq, ev.actor
            ));
        }
        ok
    }

    /// The person's End on spawned agent `key`, from any of their devices:
    /// ended here when it runs here, else asked of the device it runs on.
    pub(crate) fn request_end(&self, key: &str) -> Result<(), String> {
        let m = self
            .coord
            .spawned(key)
            .ok_or_else(|| format!("{key} wasn't started by Irori"))?;
        let dev = device_of(key);
        if dev.is_empty() || dev == self.device.id {
            return self.end_spawned(key);
        }
        let row = EndRequest {
            key: key.into(),
            generation: m.generation,
            task_id: m.task_id,
            by: format!("user:{}", self.device.id),
            at_device: self.device.id.clone(),
            at: (self.now)(),
        };
        self.store
            .append("user", kinds::SPAWN_END_REQUESTED, &row)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Ends the agents this device runs that the person asked another of
    /// their devices to end.
    pub(crate) fn serve_end_requests(&self) {
        let me = self.device.id.clone();
        for ev in self.unseen_requests(kinds::SPAWN_END_REQUESTED) {
            self.mark_seen(&ev);
            let Ok(r) = serde_json::from_slice::<EndRequest>(&ev.payload) else {
                continue;
            };
            if device_of(&r.key) != me || ev.device_id == me || !self.by_person(&ev, &r.by) {
                continue;
            }
            let Some(m) = self.coord.spawned(&r.key) else {
                continue;
            };
            if (!r.task_id.is_empty() && r.task_id != m.task_id)
                || self.coord.ended_by_irori(&r.key)
                || r.generation != m.generation
            {
                continue;
            }
            if let Err(e) = self.end_spawned(&r.key) {
                logf(format_args!(
                    "spawn: end {} for {}: {e}",
                    r.key, ev.device_id
                ));
            }
        }
    }

    /// The person asks for card `id`'s agent now from a device that
    /// doesn't hand it out: the card's own device acts on it.
    pub(crate) fn request_start(&self, id: &str, note: &str) -> Result<(), String> {
        self.write_start_request(id, note, false)
    }

    /// An agent's ask, passed to the device that hands the card out, after
    /// [`Services::agent_start`]'s checks (card #575).
    pub(crate) fn request_agent_start(&self, id: &str) -> Result<(), String> {
        self.write_start_request(id, "", true)
    }

    fn write_start_request(&self, id: &str, note: &str, agent: bool) -> Result<(), String> {
        // The card's own log: it may be a shared board's (card #575).
        let generation = self
            .route(id)
            .ok()
            .and_then(|(_, co)| co.get_task(id))
            .and_then(|t| t.spawn_request().map(|s| s.generation))
            .unwrap_or(0);
        let row = StartRequest {
            task_id: id.into(),
            generation,
            note: note.trim().to_string(),
            agent,
            by: format!("user:{}", self.device.id),
            at_device: self.device.id.clone(),
            at: (self.now)(),
        };
        self.store
            .append("user", kinds::SPAWN_START_REQUESTED, &row)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Start-now requests for cards this device hands out.
    pub(crate) fn serve_start_requests(&self) {
        for ev in self.unseen_requests(kinds::SPAWN_START_REQUESTED) {
            let Ok(r) = serde_json::from_slice::<StartRequest>(&ev.payload) else {
                self.mark_seen(&ev);
                continue;
            };
            let Some((co, t)) = self
                .route(&r.task_id)
                .ok()
                .and_then(|(_, co)| co.get_task(&r.task_id).map(|t| (co, t)))
            else {
                // The card hasn't reached this device yet: next sweep.
                continue;
            };
            self.mark_seen(&ev);
            // An agent's ask is read here too, for its count after a restart.
            if (ev.device_id == self.device.id && !r.agent)
                || !co.hands_out_here(&t.id)
                || !self.by_person(&ev, &r.by)
            {
                continue;
            }
            // Asked before the card's newest request: already served.
            if t.spawn_request()
                .is_some_and(|s| s.generation > r.generation)
            {
                continue;
            }
            if !r.note.is_empty() {
                self.set_brief_answer(&t.id, &r.note);
            }
            if r.agent {
                self.note_agent_ask(&t);
            } else {
                self.note_person_ask(&t.id);
                co.spawn_now(&t.id);
                self.release_usage_hold();
            }
            self.asks_kick.kick();
        }
    }

    /// The person, on another device, asks the device card `id`'s agent
    /// must start on to trust its repo there: only when the card's newest
    /// request failed there as not trusted. The device's name.
    pub(crate) fn request_trust(&self, id: &str) -> Result<String, String> {
        let t = self
            .coord
            .get_task(id)
            .ok_or_else(|| format!("no card {id}"))?;
        let s = t
            .spawn_request()
            .filter(|s| s.op == SPAWN_FAILED && s.reason.starts_with(NOT_TRUSTED_PREFIX))
            .ok_or_else(|| format!("{} isn't waiting on trust", t.short_ref()))?;
        if s.device == self.device.id {
            return Err("its agent starts on this computer: trust it here".into());
        }
        let row = TrustRequest {
            task_id: t.id.clone(),
            generation: s.generation,
            device: s.device.clone(),
            repo_key: s.repo_key.clone(),
            folder: not_trusted_folder(&s.reason),
            by: format!("user:{}", self.device.id),
            at_device: self.device.id.clone(),
            at: (self.now)(),
        };
        self.store
            .append("user", kinds::TRUST_REQUESTED, &row)
            .map_err(|e| e.to_string())?;
        Ok(s.device.clone())
    }

    /// Trust requests for this device: the person is asked in this
    /// device's own window (a notification that opens the card, whose
    /// Needs you row has Trust and start). Nothing here writes trust.
    pub(crate) fn serve_trust_requests(&self) {
        let me = self.device.id.clone();
        for ev in self.unseen_requests(kinds::TRUST_REQUESTED) {
            let Ok(r) = serde_json::from_slice::<TrustRequest>(&ev.payload) else {
                self.mark_seen(&ev);
                continue;
            };
            let Some(t) = self.coord.get_task(&r.task_id) else {
                continue;
            };
            self.mark_seen(&ev);
            if r.device != me || ev.device_id == me || !self.by_person(&ev, &r.by) {
                continue;
            }
            let waits = t.spawn_request().is_some_and(|s| {
                s.generation == r.generation
                    && s.device == me
                    && s.op == SPAWN_FAILED
                    && s.reason.starts_with(NOT_TRUSTED_PREFIX)
            });
            if waits {
                self.ask_trust_here(&t, &r, &ev.device_id);
            }
        }
    }

    fn ask_trust_here(&self, t: &Task, r: &TrustRequest, from: &str) {
        let from_name = self
            .device_labels()
            .remove(from)
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "Another of your devices".into());
        let what = if r.repo_key.is_empty() {
            r.folder.clone()
        } else {
            coord::repo_name(&r.repo_key)
        };
        let n = self.notifier.0.clone();
        let id = t.id.clone();
        let shown = self.notifier.0.post(notify::Note {
            id: format!("trust-{}-{}", t.id, r.generation),
            title: format!("Trust {what} for agents here?"),
            body: format!(
                "{from_name} asked: {}'s agent can't start on this computer until Claude Code trusts {}. Open the card to choose.",
                t.short_ref(),
                r.folder
            ),
            sticky: true,
            on_click: Some(std::sync::Arc::new(move || {
                n.open(notify::Target {
                    tab: notify::TAB_BOARD.into(),
                    ref_: id.clone(),
                })
            })),
            ..notify::Note::default()
        });
        self.spawn
            .remote
            .trust_asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(t.id.clone(), from.to_string());
        logf(format_args!(
            "spawn {}: {from} asked to trust {} here (shown: {shown})",
            t.short_ref(),
            r.folder
        ));
    }

    /// The device that asked this one to trust card `id`'s repo, if one
    /// did (the window says so beside Trust and start).
    pub(crate) fn trust_asked_by(&self, id: &str) -> Option<String> {
        self.spawn
            .remote
            .trust_asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }
}

/// Whether card t's newest request failed on device `dev` for a reason
/// its host row `r` (its copy of the repo) still shows: not trusted
/// there and still untrusted.
fn failed_for_reason(t: &Task, dev: &str, r: &HostRepo) -> bool {
    t.spawn_request().is_some_and(|s| {
        s.op == SPAWN_FAILED
            && s.device == dev
            && s.reason.starts_with(NOT_TRUSTED_PREFIX)
            && !r.trusted
    })
}

/// The folder a "not trusted: <folder> on <device>" reason names.
pub fn not_trusted_folder(reason: &str) -> String {
    let rest = reason.strip_prefix(NOT_TRUSTED_PREFIX).unwrap_or(reason);
    match rest.rfind(" on ") {
        Some(i) => rest[..i].to_string(),
        None => rest.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_a_not_trusted_reason_names() {
        assert_eq!(
            not_trusted_folder("not trusted: /Users/s/projects/irori on Stephen's Mac"),
            "/Users/s/projects/irori"
        );
        assert_eq!(not_trusted_folder("not trusted: C:\\r on pc"), "C:\\r");
    }

    #[test]
    fn machines_and_caps_by_os() {
        let h = |os: &str| SpawnHost {
            os: os.into(),
            ..SpawnHost::default()
        };
        assert_eq!(h("windows").machine(), "pc");
        assert_eq!(h("macos").machine(), "mac");
        assert_eq!(h("linux").machine(), "");
        assert_eq!(default_cap_for("windows"), 6);
        assert_eq!(default_cap_for("macos"), 12);
    }
}
