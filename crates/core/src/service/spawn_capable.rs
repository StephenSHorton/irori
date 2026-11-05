//! Which devices can run agents, and what the person is told when none can
//! (card #541).
//!
//! A device runs agents when it has a spawner and the `claude` command: a
//! phone never does, and neither does a computer without Claude Code. Each
//! device says so on its `spawn.host` row; the phone's `os` says so for
//! rows from before that. Work that lands on a device that can't is a
//! computer's: an ask moves to one ([`coord::Coord::move_asks`]) and a card
//! is taken up by one standing in, so the waiting needs nothing from a
//! sleeping phone and starts by itself when a computer is back.

use std::collections::HashSet;
use std::time::Duration;

use coord::AgentDevices;
use coord::notifications as history;
use gocompat::time::GoTime;
use irori_identity::objects::ROLE_DESKTOP;
use irori_identity::owner::device_of;

use super::Services;
use crate::logf;

/// `std::env::consts::OS` and `device.started`'s `os` on a phone.
const ANDROID: &str = "android";

/// How long work waits for a computer to take it before the person is told:
/// a connected computer starts it well inside this.
pub const NO_AGENT_AFTER: Duration = Duration::from_secs(30);

/// A "no agent can start" notification's id for card `id`.
pub fn no_agent_id(id: &str) -> String {
    format!("{}{id}", notify::NO_AGENT_PREFIX)
}

/// A device's name for the person.
fn device_name(labels: &std::collections::BTreeMap<String, String>, id: &str) -> String {
    labels
        .get(id)
        .filter(|n| !n.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| "a computer".to_string())
}

/// `a`, `a and b`, `a, b and c`.
fn and_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// What the person reads when no computer can start an agent now: why, by
/// device, and what happens next. Never a question: there's nothing to
/// choose. `offline`: computers that would run one; `no_claude`: computers
/// without Claude Code.
pub fn no_agent_text(offline: &[String], no_claude: &[String]) -> String {
    let (off, none) = (and_list(offline), and_list(no_claude));
    let verb = |n: &[String]| if n.len() == 1 { "is" } else { "are" };
    match (offline.is_empty(), no_claude.is_empty()) {
        (true, true) => {
            "No agent can start: none of your devices is a computer that runs agents. It starts by itself once one is.".into()
        }
        (false, true) => format!(
            "No agent can start: {off} {} offline. It starts by itself when {} back.",
            verb(offline),
            if offline.len() == 1 {
                "it's"
            } else {
                "one is"
            }
        ),
        (true, false) => format!(
            "No agent can start: Claude Code isn't installed on {none}. It starts by itself once it is."
        ),
        (false, false) => format!(
            "No agent can start: {off} {} offline, and Claude Code isn't installed on {none}. It starts by itself when one of them can."
            , verb(offline)
        ),
    }
}

impl Services {
    /// Whether this device can run agents now.
    pub(crate) fn can_run_here(&self) -> bool {
        self.spawn
            .spawner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|s| s.ready.as_ref().is_none_or(|f| f()))
    }

    /// Reads again which devices can run agents, for the coords' hand-out
    /// (which can't read the store under its lock) and the words the person
    /// sees. Computers are ranked: this one or a connected one first, then
    /// one seen lately, then an offline one, the lowest id first, so two
    /// connected computers agree on which of them takes phone work.
    pub(crate) fn refresh_agent_devices(&self) {
        let me = self.device.id.clone();
        let here = self.can_run_here();
        let hosts = self.hosts();
        let labels = self.device_labels();
        let rows = self.mesh.node.devices(false);
        let mut cant: HashSet<String> = HashSet::new();
        // (cannot run, tier, id)
        let mut ranked: Vec<(bool, u8, String)> = Vec::new();
        let mut seen_me = false;
        for r in rows.iter().filter(|r| !r.revoked) {
            let id = &r.device_id;
            seen_me |= *id == me;
            let host = hosts.get(id);
            let phone = r.os == ANDROID || host.is_some_and(|h| h.os == ANDROID);
            let computer = r.role.is_empty() || r.role == ROLE_DESKTOP;
            if phone || !computer {
                cant.insert(id.clone());
                continue;
            }
            // A device that never published one (an older Irori) is never
            // picked, as in spawn_device.
            if host.is_none() && *id != me {
                continue;
            }
            let no = if *id == me {
                !here
            } else {
                host.is_some_and(|h| h.no_agents)
            };
            if no {
                cant.insert(id.clone());
            }
            let tier = match r.state.as_str() {
                _ if r.this || *id == me => 0,
                "connected" => 0,
                "recent" => 1,
                _ => 2,
            };
            ranked.push((no, tier, id.clone()));
        }
        if !seen_me {
            if here {
                ranked.push((false, 0, me.clone()));
            } else {
                cant.insert(me.clone());
            }
        }
        ranked.sort();
        let name = |id: &str| device_name(&labels, id);
        let can_now = ranked.iter().any(|(no, tier, _)| !no && *tier == 0);
        let text = if can_now {
            String::new()
        } else {
            let offline: Vec<String> = ranked
                .iter()
                .filter(|(no, _, _)| !no)
                .map(|(_, _, d)| name(d))
                .collect();
            let no_claude: Vec<String> = ranked
                .iter()
                .filter(|(no, _, _)| *no)
                .map(|(_, _, d)| name(d))
                .collect();
            no_agent_text(&offline, &no_claude)
        };
        *self.spawn.agents.write().unwrap_or_else(|e| e.into_inner()) = AgentDevices {
            homes: ranked.into_iter().map(|(_, _, d)| d).collect(),
            cant,
        };
        *self
            .spawn
            .no_agent
            .write()
            .unwrap_or_else(|e| e.into_inner()) = text;
    }

    /// What the person reads while no computer can start an agent; empty
    /// while one can.
    pub(crate) fn no_agent_line(&self) -> String {
        self.spawn
            .no_agent
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Names of the computers without Claude Code that card t waits on:
    /// they alone have its repo. Empty when one that can run agents has it,
    /// or when nobody published it, which is a folder question.
    pub(crate) fn repo_waits_on(&self, t: &coord::Task) -> Vec<String> {
        if t.repo_key.is_empty() || t.open_ask().is_some() {
            return Vec::new();
        }
        let cant = self.cant_run_devices();
        let need = coord::machine_needed(t);
        let hosts = self.hosts();
        let holders: Vec<&String> = hosts
            .iter()
            .filter(|(_, h)| h.repo(&t.repo_key).is_some())
            .filter(|(_, h)| need.is_empty() || h.machine() == need)
            .map(|(d, _)| d)
            .collect();
        if holders.iter().any(|d| !cant.contains(*d)) {
            return Vec::new();
        }
        let labels = self.device_labels();
        let mut names: Vec<String> = holders
            .into_iter()
            .map(|d| device_name(&labels, d))
            .collect();
        names.sort();
        names
    }

    /// Why no agent can start for card t now; empty when one can.
    pub(crate) fn no_agent_for(&self, t: &coord::Task) -> String {
        if !self.coord.waits_for_agent(t) {
            return String::new();
        }
        let line = self.no_agent_line();
        if !line.is_empty() {
            return line;
        }
        let names = self.repo_waits_on(t);
        if names.is_empty() {
            return String::new();
        }
        no_agent_text(&[], &names)
    }

    /// The devices that can't run agents, for the window's words.
    pub(crate) fn cant_run_devices(&self) -> HashSet<String> {
        self.spawn
            .agents
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .cant
            .clone()
    }

    /// Hands asks this device can't serve to a computer.
    pub(crate) fn move_asks(&self) {
        if let Err(e) = self.coord.move_asks() {
            logf(format_args!("spawn: moving asks: {e}"));
        }
        for b in self.shown_boards() {
            if let Err(e) = b.coord().move_asks() {
                logf(format_args!("spawn: moving asks on {}: {e}", b.id));
            }
        }
    }

    /// Tells the person, once, about each thing they posted here that no
    /// agent can start for, and takes it back when that's over. Runs on the
    /// device the work was posted on or is owned by: with every computer
    /// off, the phone is the only one awake to say it. The row reaches their
    /// other devices' history; it settles by itself ([`Self::resolved`]).
    pub(crate) fn say_no_agent(&self) {
        let me = self.device.id.clone();
        let now = (self.now)();
        let open = self.coord.list_tasks(&coord::TaskFilter {
            open: true,
            ..coord::TaskFilter::default()
        });
        let mut told = self
            .spawn
            .no_agent_told
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let waiting: Vec<(&coord::Task, String)> = open
            .iter()
            .filter(|t| {
                device_of(&t.created_by) == me || t.open_ask().is_some_and(|a| a.owner == me)
            })
            .map(|t| (t, self.no_agent_for(t)))
            .filter(|(_, line)| !line.is_empty())
            .collect();
        let still: HashSet<&str> = waiting.iter().map(|(t, _)| t.id.as_str()).collect();
        told.retain(|id| {
            let keep = still.contains(id.as_str());
            if !keep {
                self.notifier.0.clear(&no_agent_id(id));
            }
            keep
        });
        for (t, text) in waiting {
            if told.contains(&t.id) || !waited(&now, &t.created_at) {
                continue;
            }
            told.insert(t.id.clone());
            // "Nobody took it" would say someone still might.
            self.notifier.0.clear(&notify::unclaimed_id(&t.id));
            let (n, target) = (self.notifier.0.clone(), format!("task:{}", t.id));
            let title = format!("No agent for {}", t.short_ref());
            self.notifier.0.post(notify::Note {
                id: no_agent_id(&t.id),
                title: title.clone(),
                body: text.clone(),
                sticky: true,
                on_click: Some(std::sync::Arc::new(move || {
                    n.open(notify::Target {
                        tab: notify::TAB_BOARD.into(),
                        ref_: target.clone(),
                    })
                })),
                about: Some(history::PostedRow {
                    kind: history::KIND_NO_AGENT.into(),
                    title,
                    body: text.clone(),
                    task_id: t.id.clone(),
                    // The same card can wait again another day.
                    instance: (now.unix_nano() / 3_600_000_000_000).to_string(),
                    at: now,
                    ..history::PostedRow::default()
                }),
                ..notify::Note::default()
            });
        }
    }
}

fn waited(now: &GoTime, since: &GoTime) -> bool {
    i128::from(now.unix_nano()) - i128::from(since.unix_nano()) >= NO_AGENT_AFTER.as_nanos() as i128
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn the_words_name_the_devices_and_what_happens_next() {
        assert_eq!(
            no_agent_text(&s(&["Mac"]), &[]),
            "No agent can start: Mac is offline. It starts by itself when it's back."
        );
        assert_eq!(
            no_agent_text(&s(&["Mac", "PC"]), &[]),
            "No agent can start: Mac and PC are offline. It starts by itself when one is back."
        );
        assert_eq!(
            no_agent_text(&[], &s(&["Mac"])),
            "No agent can start: Claude Code isn't installed on Mac. It starts by itself once it is."
        );
        assert_eq!(
            no_agent_text(&s(&["PC"]), &s(&["Mac"])),
            "No agent can start: PC is offline, and Claude Code isn't installed on Mac. It starts by itself when one of them can."
        );
    }

    #[test]
    fn the_words_are_never_a_question_or_about_a_repo() {
        for t in [
            no_agent_text(&[], &[]),
            no_agent_text(&s(&["Mac"]), &[]),
            no_agent_text(&[], &s(&["Mac"])),
            no_agent_text(&s(&["PC"]), &s(&["Mac"])),
        ] {
            assert!(!t.contains('?'), "{t}");
            assert!(!t.contains("repo") && !t.contains("folder"), "{t}");
            assert!(!crate::spawn_stop::no_folder_reason(&t), "{t}");
        }
    }
}
