//! Why a card has no agent yet (card #574, GitHub #66): the one line an
//! agent reads after `spawn: true`, and in `board` and `show`.
//!
//! An overseer marked six shared cards `spawn: true`; the call succeeded and
//! nothing said why nothing started. This walks what the sweep walks
//! (`sweep_spawns`, `sweep_board_spawns`, `spawn_for`) and says the first
//! thing in the way, in the words the card's "can't start" line uses. A
//! failed request's reason is the row's own, so any refusal the sweep learns
//! shows here with no change.
//!
//! A `spawn` mark on a step starts nothing by itself: only a marked subtask
//! does, and a mark on a step moves to one with `board_split`
//! ([`Why::StepOnly`]). An unheld card gets its agent as a card, marked or
//! not.
//!
//! It reads like the sweep but doesn't run it, and the sweep has the last
//! word: the first try can still fail, and then the failed row says why.

use std::fmt;
use std::path::Path;

use coord::{SPAWN_FAILED, Spawn, SpawnHold, Task};
use irori_identity::kinds::PERSON_ACTOR_PREFIX;
use irori_identity::objects::ROLE_DESKTOP;

use super::Services;
use super::spawn::NOT_TRUSTED_PREFIX;
use super::spawn_boards::SpawnLog;
use super::spawn_remote::not_trusted_folder;
use super::spawn_stop::{no_folder_reason, no_folder_words};

/// What a card's agent is waiting on, first thing first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// A request is out and its agent hasn't shown yet.
    Starting { device: String },
    /// A request went past its grace with no agent; Irori asks again.
    NoShow { device: String },
    /// Another member's card: only their Irori starts its agent (§7).
    OthersCard { person: String },
    /// The board's switch for started agents is off.
    SwitchOff,
    /// Pause all, on the device that would start it.
    Paused { device: String },
    /// Pause agents (card #665) on the device that would start it, since
    /// when.
    AgentsPaused { device: String, since: String },
    /// The device that would start it can't run agents (card #541's words).
    CantRun { device: String },
    /// No computer can start it now: card #541's line, without its
    /// "No agent can start: " and its last period.
    NoComputer(String),
    /// A member's Irori can't read spawn rows yet.
    OlderMember { people: Vec<String> },
    /// A mark on a step of a card that has its agent.
    StepOnly,
    /// A shared card: agents join it themselves.
    Joinable,
    /// It waits on a blocker, and gets an agent once that clears.
    Blocked,
    /// Its agents crashed twice in a row.
    Crashed,
    /// A thread whose last agent Irori ended.
    Resting,
    /// It's on a side no device here runs: the card's own words.
    OffSide(String),
    /// A subtask of a card with a lead, with no mark of its own.
    ForLead,
    /// The repo isn't trusted where it would start.
    NeedsTrust { folder: String, device: String },
    /// No folder for the card: the "can't start" line's words.
    NoFolder(String),
    /// The spawner refused: the failed row's reason.
    Refused(String),
    /// The device that would start it is at its cap.
    AtCap {
        device: String,
        running: i64,
        cap: i64,
    },
    /// A thread on a device with no free slot (card #686): another of the
    /// person's computers has room and takes it.
    MovesTo { from: String, device: String },
    /// A thread whose device has no free slot, and no other computer has
    /// one either (card #686).
    AllFull,
    /// The device has room, but older cards take it first.
    Queued {
        device: String,
        running: i64,
        cap: i64,
        ahead: i64,
    },
    /// Nothing that has its repo can take it now.
    NoDevice { devices: Vec<String> },
    /// Another of the person's devices hands this card out.
    Elsewhere { device: String },
    /// "My agents may start agents" is off, and an agent caused this start.
    AgentStartsOff,
    /// That limit is used up.
    AgentStartsFull { used: i64, max: i64 },
    /// The device that hands it out was removed.
    DeviceRemoved { device: String },
    /// No device hands this card out (Irori made it, say).
    NoOrigin,
    /// Nothing in the way.
    NextPass,
}

/// `a`, `a and b`, `a, b and c`.
fn and_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl fmt::Display for Why {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Why::Starting { device } => write!(f, "one is starting on {device}"),
            Why::NoShow { device } => {
                write!(
                    f,
                    "{device} was asked to start one and none has shown up; Irori asks again"
                )
            }
            Why::OthersCard { person } => {
                write!(
                    f,
                    "{person} made this card: only their Irori starts its agent"
                )
            }
            Why::SwitchOff => write!(
                f,
                "this board doesn't start agents; the person turns that on in Settings › Agents"
            ),
            Why::Paused { device } => {
                write!(f, "Pause all is on on {device}; it starts once that's off")
            }
            Why::AgentsPaused { device, since } => write!(
                f,
                "Pause agents is on on {device} (since {since}); it starts once that's off"
            ),
            Why::CantRun { device } => write!(f, "{device} can't run agents"),
            Why::NoComputer(line) => write!(f, "{line}"),
            Why::OlderMember { people } if people.is_empty() => write!(
                f,
                "a device on this board can't read agent starts yet; it starts once that device has updated or connected"
            ),
            Why::OlderMember { people } => write!(
                f,
                "{}'s Irori can't read agent starts yet; it starts once it has updated or connected",
                and_list(people)
            ),
            Why::StepOnly => write!(
                f,
                "for its marked step: a mark on a step starts nothing by itself; board_split the step into a subtask and its mark asks for a helper"
            ),
            Why::Joinable => write!(
                f,
                "this is a shared card: agents join it themselves, and Irori doesn't start one"
            ),
            Why::Blocked => write!(f, "it's blocked; an agent starts once that clears"),
            Why::Crashed => write!(
                f,
                "its agents crashed twice in a row; the person decides on the card"
            ),
            Why::Resting => write!(
                f,
                "Irori ended its last agent; another starts when the person writes on it"
            ),
            Why::OffSide(why) => write!(f, "{why}"),
            Why::ForLead => write!(
                f,
                "it's a subtask of a card with a lead: the lead marks it spawn or assigns it"
            ),
            Why::NeedsTrust { folder, device } => write!(
                f,
                "{folder} isn't trusted on {device}; the person chooses Trust and start on the card"
            ),
            Why::NoFolder(words) => {
                write!(f, "{words}; the person chooses a folder on the card")
            }
            Why::Refused(reason) => write!(f, "{reason}"),
            Why::AtCap {
                device,
                running,
                cap,
            } => write!(
                f,
                "{device} is at its cap ({running}/{cap}); it starts when one ends"
            ),
            Why::MovesTo { from, device } => write!(
                f,
                "{from} has no free agent slot; {device} has room and takes it"
            ),
            Why::AllFull => write!(
                f,
                "every computer is at its agent limit; it starts by itself when one has room"
            ),
            Why::Queued {
                device,
                running,
                cap,
                ahead,
            } => write!(
                f,
                "{device} runs {running} of {cap}, and {ahead} older cards go first"
            ),
            Why::NoDevice { devices } if devices.is_empty() => {
                write!(f, "no device can take it now; it starts when one can")
            }
            Why::NoDevice { devices } => write!(
                f,
                "waiting for {}, which has its repo; it starts when that can take it",
                and_list(devices)
            ),
            Why::Elsewhere { device } => write!(f, "{device} starts it, not this device"),
            Why::AgentStartsOff => write!(
                f,
                "the person has turned off \"My agents may start agents\"; only they can change that, in Settings › Agents"
            ),
            Why::AgentStartsFull { used, max } => write!(
                f,
                "the person's agents have started their most agents ({used}/{max}); it starts when one ends, and only the person can change the maximum, in Settings › Agents"
            ),
            Why::DeviceRemoved { device } => write!(
                f,
                "{device} hands this card out and was removed: take it up with board_join"
            ),
            Why::NoOrigin => write!(
                f,
                "no device hands this card out: take it up with board_join"
            ),
            Why::NextPass => write!(f, "Irori starts one at its next pass"),
        }
    }
}

/// Whether the older cards ahead fill the slots a device has left.
fn queued(device: String, running: i64, cap: i64, ahead: i64) -> Option<Why> {
    (running + ahead >= cap).then_some(Why::Queued {
        device,
        running,
        cap,
        ahead,
    })
}

/// Card #541's line, fit to follow "no agent yet: ".
fn no_computer_words(line: &str) -> String {
    line.strip_prefix("No agent can start: ")
        .unwrap_or(line)
        .trim_end_matches('.')
        .to_string()
}

/// Whether an open step of t asks for a helper.
fn step_marked(t: &Task) -> bool {
    t.steps.iter().any(|s| s.spawn && !s.done)
}

impl Services {
    /// A device's name for the person; `""` when this device has none.
    pub(crate) fn named_device(&self, dev: &str) -> String {
        if dev == self.device.id {
            return self.this_device_name();
        }
        self.device_labels()
            .remove(dev)
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "another device".into())
    }

    /// What card `id`'s failed request says, in the card line's words.
    fn failed_why(&self, t: &Task, s: &Spawn) -> Why {
        let device = self.named_device(&s.device);
        if no_folder_reason(&s.reason) {
            Why::NoFolder(no_folder_words(&t.repo_key, &device))
        } else if s.reason.starts_with(NOT_TRUSTED_PREFIX) {
            Why::NeedsTrust {
                folder: not_trusted_folder(&s.reason),
                device,
            }
        } else {
            Why::Refused(s.reason.clone())
        }
    }

    /// What card t waits on for the device that starts it, when nothing on
    /// the card itself is in the way.
    fn device_why(&self, co: &coord::Coord, t: &Task, board_id: &str) -> Why {
        let private = board_id.is_empty();
        let me = self.device.id.clone();
        let origin = co.card_origin(&t.id);
        if origin.is_empty() {
            return Why::NoOrigin;
        }
        if origin != me {
            let device = self.named_device(&origin);
            let rows = self.mesh.node.devices(false);
            let row = rows.iter().find(|r| r.device_id == origin);
            return match row {
                Some(r) if r.revoked => Why::DeviceRemoved { device },
                Some(r) if !r.role.is_empty() && r.role != ROLE_DESKTOP => Why::CantRun { device },
                _ => Why::Elsewhere { device },
            };
        }
        if self.inbox.policy().paused {
            return Why::Paused {
                device: self.this_device_name(),
            };
        }
        // A thread here with no free slot (card #686): a computer with
        // room takes it; with none, it waits here and says so.
        if private
            && t.open_ask().is_some_and(|a| a.owner == me)
            && self.full_here()
        {
            if let Some(dev) = co.room_home(&t.side, &me) {
                return Why::MovesTo {
                    from: self.named_device(&me),
                    device: self.named_device(&dev),
                };
            }
            if !self.agents_paused() && !co.ask_full_wait(t).is_empty() {
                return Why::AllFull;
            }
        }
        // Pause agents here (card #665), and no other device takes it.
        let pause = self.agents_pause();
        if pause.on && self.spawn_device(t).is_none() {
            return Why::AgentsPaused {
                device: self.this_device_name(),
                since: pause.since.format_rfc3339(),
            };
        }
        let spawner = self
            .spawn
            .spawner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(sp) = spawner else {
            return Why::CantRun {
                device: self.this_device_name(),
            };
        };
        let dev = if private {
            self.spawn_device(t)
        } else {
            Some(me.clone())
        };
        let Some(dev) = dev else {
            return self.no_device_why(t);
        };
        if dev != me {
            return self.remote_why(t, &dev);
        }
        let (running, cap) = (self.spawned_here(), self.spawn_cap(&me));
        if running >= cap {
            return Why::AtCap {
                device: self.named_device(&me),
                running,
                cap,
            };
        }
        let trusted = |f: &str| (sp.trusted)(Path::new(f));
        if let Some(why) = self.folder_why(t, private, &trusted) {
            return why;
        }
        let ahead = self.ahead_here(t, board_id, &trusted);
        queued(self.named_device(&me), running, cap, ahead).unwrap_or(Why::NextPass)
    }

    /// How many older cards take one of this device's slots before card t,
    /// in the sweep's order: the private board's cards, then each shown
    /// board's (`sweep_spawns`, `sweep_board_spawns`). Only those the sweep
    /// would start here count: not a card another device starts, one that
    /// waits for its machine, or one with no folder or trust here.
    fn ahead_here(&self, t: &Task, board_id: &str, trusted: &dyn Fn(&str) -> bool) -> i64 {
        let me = self.device.id.clone();
        let mut ahead = 0;
        for k in self.coord.cards_to_spawn() {
            if board_id.is_empty() && k.id == t.id {
                return ahead;
            }
            if self.spawn_device(&k).as_deref() == Some(me.as_str())
                && self.folder_why(&k, true, trusted).is_none()
            {
                ahead += 1;
            }
        }
        if board_id.is_empty() {
            return ahead;
        }
        let person = self.person_id();
        for b in self.visible_boards() {
            let co = b.coord().clone();
            if !co.spawning() || !self.board_spawns_ready(&b) {
                continue;
            }
            for k in co.cards_to_spawn() {
                if k.id == t.id {
                    return ahead;
                }
                if k.open_ask().is_none_or(|a| a.thread)
                    && b.verifier.key_person(&k.created_by) == person
                    && self.folder_why(&k, false, trusted).is_none()
                {
                    ahead += 1;
                }
            }
            if b.id == board_id {
                break;
            }
        }
        ahead
    }

    /// What the sweep's first try here would refuse on: no folder, or one
    /// that isn't trusted. A helper's worktree is made only at its start,
    /// so its part isn't looked at. Creates nothing.
    fn folder_why(&self, t: &Task, private: bool, trusted: &dyn Fn(&str) -> bool) -> Option<Why> {
        if t.spawn && !t.parent.is_empty() {
            return None;
        }
        let folder = if private {
            self.card_folder(t)
        } else if t.repo_key.is_empty() && t.open_ask().is_some() {
            self.ask_home().map(|(_, f)| f)
        } else {
            self.checkout_here(&t.repo_key)
        };
        let device = self.this_device_name();
        match folder {
            None => Some(Why::NoFolder(no_folder_words(&t.repo_key, &device))),
            Some(folder) if !trusted(&folder) => Some(Why::NeedsTrust { folder, device }),
            Some(_) => None,
        }
    }

    /// `spawn_device` named another device: what its host row says.
    fn remote_why(&self, t: &Task, dev: &str) -> Why {
        let hosts = self.hosts();
        let host = hosts.get(dev);
        let device = self.named_device(dev);
        let (running, cap) = (self.spawned_on(dev), self.cap_on(dev, host));
        if running >= cap {
            return Why::AtCap {
                device,
                running,
                cap,
            };
        }
        match host.and_then(|h| h.repo(&t.repo_key)) {
            Some(r) if !r.trusted => Why::NeedsTrust {
                folder: r.folder.clone(),
                device,
            },
            _ => Why::NextPass,
        }
    }

    /// `spawn_device` found nowhere to start card t now, of the devices it
    /// would use: one full, or those with its repo away.
    fn no_device_why(&self, t: &Task) -> Why {
        let need = coord::machine_needed(t);
        let rows = self.mesh.node.devices(false);
        let hosts = self.hosts();
        let mut devs: Vec<_> = hosts
            .iter()
            .filter(|(_, h)| h.repo(&t.repo_key).is_some())
            .filter(|(_, h)| need.is_empty() || h.machine() == need)
            .filter(|(d, _)| self.coord.runs_side(d, &t.side))
            .filter(|(d, _)| {
                rows.iter()
                    .find(|r| r.device_id == **d)
                    .is_none_or(|r| !r.revoked && (r.role.is_empty() || r.role == ROLE_DESKTOP))
            })
            .collect();
        devs.sort_by_key(|(d, _)| d.as_str());
        let online = |d: &str| {
            rows.iter()
                .find(|r| r.device_id == d)
                .is_some_and(|r| r.this || r.state != "offline")
        };
        for (dev, h) in &devs {
            let (running, cap) = (self.spawned_on(dev), self.cap_on(dev, Some(h)));
            if online(dev) && running >= cap {
                return Why::AtCap {
                    device: self.named_device(dev),
                    running,
                    cap,
                };
            }
        }
        let mut names: Vec<String> = devs.iter().map(|(d, _)| self.named_device(d)).collect();
        names.sort();
        Why::NoDevice { devices: names }
    }

    /// What card `id` waits on for an agent; `None` when it doesn't.
    /// `asked`: the caller just asked for one, so even what the board's
    /// list leaves out is said. On its own a card says only when it has a
    /// mark or a request, or is on a board that starts agents: someone
    /// else's card or a board with the switch off is ordinary there.
    ///
    /// What no setting can change comes before what the person could
    /// change, so the line never points at a fix that does nothing.
    pub fn why_no_agent(&self, id: &str, asked: bool) -> Option<Why> {
        let (board_id, co) = self.route(id).ok()?;
        let t = co.get_task(id)?;
        if coord::is_terminal(&t.state) || t.withdrawn {
            return None;
        }
        let hold = co.spawn_hold(id);
        let private = board_id.is_empty();
        let request = t.spawn_request();
        let asked = asked || t.spawn || step_marked(&t) || request.is_some();
        // Card #541: every computer with its repo is offline or lacks
        // Claude Code, or none runs agents. Its line says which, and that
        // it starts by itself. A thread this device gave to an offline
        // computer needs no agent here, but still waits on that one.
        let no_computer = || {
            let line = self.no_agent_for(&co, &t);
            (!line.is_empty()).then(|| Why::NoComputer(no_computer_words(&line)))
        };
        if hold == SpawnHold::NoNeed {
            if step_marked(&t) && t.open_ask().is_none() {
                return Some(Why::StepOnly);
            }
            return no_computer();
        }
        let stale = request.filter(|s| s.pending() && s.stale(&(self.now)()));
        if let Some(s) = request.filter(|s| s.pending() && !s.stale(&(self.now)())) {
            return Some(Why::Starting {
                device: self.named_device(&s.device),
            });
        }
        if hold == SpawnHold::Joinable {
            return asked.then_some(Why::Joinable);
        }
        let board = if private {
            None
        } else {
            Some(self.board(&board_id).ok()?)
        };
        if let Some(b) = &board
            && let Some(person) = (SpawnLog::board(self, b).others)(&t.created_by)
        {
            return asked.then_some(Why::OthersCard { person });
        }
        let fixed = match &hold {
            SpawnHold::Crashed => Some(Why::Crashed),
            SpawnHold::Resting => Some(Why::Resting),
            SpawnHold::OffSide(why) => Some(Why::OffSide(why.clone())),
            SpawnHold::ForLead => Some(Why::ForLead),
            _ => None,
        };
        if let Some(why) = fixed {
            return (asked || co.spawning()).then_some(why);
        }
        // The person's limit is a setting: after what none can change.
        if let Some(w) = self.limit_holds(&co, &t) {
            return Some(w);
        }
        if !co.spawning() {
            return asked.then_some(Why::SwitchOff);
        }
        // Blocked before an old failure: the failure's fix (Trust and
        // start, a folder) starts nothing until the blocker clears.
        if hold == SpawnHold::Blocked {
            return Some(Why::Blocked);
        }
        // A member's older Irori before an old failure too: no agent
        // starts on this board until it updates, whatever is fixed here.
        if let Some(b) = &board
            && !self.board_spawns_ready(b)
        {
            let names = self.space_names();
            let mut people: Vec<String> = self
                .mesh
                .board(&b.id)
                .map(|m| m.not_ready_for_spawns())
                .unwrap_or_default()
                .iter()
                .filter_map(|d| names.get(&format!("{PERSON_ACTOR_PREFIX}{}", d.person_id)))
                .cloned()
                .collect();
            people.sort();
            people.dedup();
            return Some(Why::OlderMember { people });
        }
        if let Some(s) = request.filter(|s| s.op == SPAWN_FAILED) {
            return Some(self.failed_why(&t, s));
        }
        // device_why would only blame this device.
        if let Some(why) = no_computer() {
            return Some(why);
        }
        match (self.device_why(&co, &t, &board_id), stale) {
            // Asked and nothing came: Irori asks again after a wait, and
            // until then the asked device counts that request as running,
            // so a full device or nowhere to start it is about this request.
            (
                Why::NextPass | Why::AtCap { .. } | Why::Queued { .. } | Why::NoDevice { .. },
                Some(s),
            ) => Some(Why::NoShow {
                device: self.named_device(&s.device),
            }),
            (Why::NextPass, None) => asked.then_some(Why::NextPass),
            (why, _) => Some(why),
        }
    }

    /// The line for a card's agent, for the tools; `""` when there's none.
    pub(crate) fn spawn_wait(&self, id: &str, asked: bool) -> String {
        self.why_no_agent(id, asked)
            .map(|w| w.to_string())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_cards_fill_the_slots_first() {
        let q = |running, cap, ahead| queued("Mac".into(), running, cap, ahead);
        assert_eq!(q(0, 2, 1), None, "one slot left for it");
        assert!(q(1, 2, 1).is_some(), "the last slot is the older card's");
        assert!(q(0, 1, 1).is_some());
    }

    #[test]
    fn every_reason_says_what_is_in_the_way() {
        let dev = || "Mac".to_string();
        for (why, words) in [
            (Why::Starting { device: dev() }, "starting on Mac"),
            (Why::NoShow { device: dev() }, "none has shown up"),
            (
                Why::OthersCard {
                    person: "Blake".into(),
                },
                "Blake made this card",
            ),
            (Why::SwitchOff, "Settings › Agents"),
            (Why::Paused { device: dev() }, "Pause all"),
            (
                Why::AgentsPaused {
                    device: dev(),
                    since: "2026-11-07T01:00:00Z".into(),
                },
                "Pause agents is on on Mac (since 2026-11-07T01:00:00Z)",
            ),
            (Why::CantRun { device: dev() }, "Mac can't run agents"),
            (
                Why::NoComputer(no_computer_words(
                    "No agent can start: Mac is offline. It starts by itself when it's back.",
                )),
                "Mac is offline. It starts by itself when it's back",
            ),
            (
                Why::OlderMember {
                    people: vec!["Blake".into()],
                },
                "Blake's Irori",
            ),
            (Why::StepOnly, "board_split"),
            (Why::Joinable, "shared card"),
            (Why::Blocked, "blocked"),
            (Why::Crashed, "crashed twice"),
            (Why::Resting, "writes on it"),
            (
                Why::Queued {
                    device: dev(),
                    running: 1,
                    cap: 2,
                    ahead: 1,
                },
                "1 older cards go first",
            ),
            (Why::DeviceRemoved { device: dev() }, "was removed"),
            (Why::NoOrigin, "no device hands"),
            (Why::ForLead, "lead"),
            (
                Why::NeedsTrust {
                    folder: "/r".into(),
                    device: dev(),
                },
                "/r isn't trusted on Mac",
            ),
            (
                Why::NoFolder("no copy of irori on Mac".into()),
                "chooses a folder",
            ),
            (
                Why::AtCap {
                    device: dev(),
                    running: 2,
                    cap: 2,
                },
                "Mac is at its cap (2/2)",
            ),
            (
                Why::NoDevice {
                    devices: vec!["Mac".into(), "PC".into()],
                },
                "Mac and PC",
            ),
            (Why::Elsewhere { device: dev() }, "Mac starts it"),
            (Why::NextPass, "next pass"),
        ] {
            let s = why.to_string();
            assert!(s.contains(words), "{s:?} lacks {words:?}");
        }
    }
}
