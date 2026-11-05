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

use std::fmt;

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
    /// The device that would start it can't run agents (card #541's words).
    CantRun { device: String },
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
    /// It's on a side no device here runs: the card's own words.
    OffSide(String),
    /// A subtask of a card with a lead, with no mark of its own.
    ForLead,
    /// The card has its most agents.
    AtLimit { limit: usize },
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
    /// Nothing that has its repo can take it now.
    NoDevice { devices: Vec<String> },
    /// Another of the person's devices hands this card out.
    Elsewhere { device: String },
    /// "My agents may start agents" is off, and an agent caused this start.
    AgentStartsOff,
    /// That limit is used up.
    AgentStartsFull { used: i64, max: i64 },
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
            Why::CantRun { device } => write!(f, "{device} can't run agents"),
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
                "a mark on a step starts nothing by itself; board_split the step into a subtask and its mark asks for a helper"
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
            Why::OffSide(why) => write!(f, "{why}"),
            Why::ForLead => write!(
                f,
                "it's a subtask of a card with a lead: the lead marks it spawn or assigns it"
            ),
            Why::AtLimit { limit } => {
                write!(
                    f,
                    "its parent has its most agents ({limit}); it starts when one ends"
                )
            }
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
            Why::NextPass => write!(f, "Irori starts one at its next pass"),
        }
    }
}

/// Whether an open step of t asks for a helper.
fn step_marked(t: &Task) -> bool {
    t.steps.iter().any(|s| s.spawn && !s.done)
}

impl Services {
    /// A device's name for the person; `""` when this device has none.
    fn named_device(&self, dev: &str) -> String {
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

    /// Whether `dev` is a phone or another device that never runs agents.
    fn never_runs(&self, dev: &str) -> bool {
        self.mesh
            .node
            .devices(false)
            .iter()
            .find(|r| r.device_id == dev)
            .is_some_and(|r| !r.role.is_empty() && r.role != ROLE_DESKTOP)
    }

    /// The device that starts card t, and what stands in its way there.
    fn device_why(&self, t: &Task, origin: &str, private: bool) -> Why {
        let me = self.device.id.clone();
        if !origin.is_empty() && origin != me {
            let device = self.named_device(origin);
            return if self.never_runs(origin) {
                Why::CantRun { device }
            } else {
                Why::Elsewhere { device }
            };
        }
        if self.inbox.policy().paused {
            return Why::Paused {
                device: self.this_device_name(),
            };
        }
        let started = self
            .spawn
            .spawner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        if !started {
            return Why::CantRun {
                device: self.this_device_name(),
            };
        }
        let dev = if private {
            self.spawn_device(t)
        } else {
            Some(me.clone())
        };
        let Some(dev) = dev else {
            return self.no_device_why(t);
        };
        let hosts = self.hosts();
        let (running, cap) = if dev == me {
            (self.spawned_here(), self.spawn_cap(&me))
        } else {
            (self.spawned_on(&dev), self.cap_on(&dev, hosts.get(&dev)))
        };
        if running >= cap {
            return Why::AtCap {
                device: self.named_device(&dev),
                running,
                cap,
            };
        }
        Why::NextPass
    }

    /// `spawn_device` found nowhere to start card t now: a device full, or
    /// those with its repo away.
    fn no_device_why(&self, t: &Task) -> Why {
        let hosts = self.hosts();
        let mut names = Vec::new();
        for (dev, h) in hosts.iter().filter(|(_, h)| h.repo(&t.repo_key).is_some()) {
            let (running, cap) = (self.spawned_on(dev), self.cap_on(dev, Some(h)));
            if running >= cap {
                return Why::AtCap {
                    device: self.named_device(dev),
                    running,
                    cap,
                };
            }
            names.push(self.named_device(dev));
        }
        names.sort();
        Why::NoDevice { devices: names }
    }

    /// What card `id` waits on for an agent; `None` when it doesn't.
    /// `asked`: the caller just asked for one, so even what the board's
    /// list leaves out is said. On its own a card says only when it has a
    /// mark or a request, or is on a board that starts agents: someone
    /// else's card or a board with the switch off is ordinary there.
    pub fn why_no_agent(&self, id: &str, asked: bool) -> Option<Why> {
        let (board_id, co) = self.route(id).ok()?;
        let t = co.get_task(id)?;
        let hold = co.spawn_hold(id);
        let private = board_id.is_empty();
        let request = t.spawn_request();
        let marked = t.spawn || step_marked(&t);
        let asked = asked || marked || request.is_some();
        if hold == SpawnHold::NoNeed {
            return (!t.holders.is_empty() && step_marked(&t) && t.subtasks.is_empty())
                .then_some(Why::StepOnly);
        }
        if let Some(s) = request.filter(|s| s.pending()) {
            let device = self.named_device(&s.device);
            return Some(if s.stale(&(self.now)()) {
                Why::NoShow { device }
            } else {
                Why::Starting { device }
            });
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
        if let Some(w) = self.limit_holds(&co, &t) {
            return Some(w);
        }
        if !co.spawning() {
            return asked.then_some(Why::SwitchOff);
        }
        let why = match hold {
            SpawnHold::Joinable => Why::Joinable,
            SpawnHold::Crashed => Why::Crashed,
            SpawnHold::OffSide(why) => Why::OffSide(why),
            SpawnHold::ForLead => Why::ForLead,
            SpawnHold::AtLimit(limit) => Why::AtLimit { limit },
            SpawnHold::Blocked | SpawnHold::Clear | SpawnHold::NoNeed => {
                match request.filter(|s| s.op == SPAWN_FAILED) {
                    Some(s) => self.failed_why(&t, s),
                    None if hold == SpawnHold::Blocked => Why::Blocked,
                    None => self.clear_why(&t, board.as_deref(), &co, private),
                }
            }
        };
        (asked || why != Why::NextPass).then_some(why)
    }

    /// Nothing on the card itself stands in the way: what's left is the
    /// board's members and the device.
    fn clear_why(
        &self,
        t: &Task,
        board: Option<&super::boards::BoardLog>,
        co: &coord::Coord,
        private: bool,
    ) -> Why {
        if let Some(b) = board
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
            return Why::OlderMember { people };
        }
        self.device_why(t, &co.card_origin(&t.id), private)
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
            (Why::CantRun { device: dev() }, "Mac can't run agents"),
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
            (Why::ForLead, "lead"),
            (Why::AtLimit { limit: 4 }, "(4)"),
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
