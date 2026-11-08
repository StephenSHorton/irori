//! Who is online on a shared board (card #371): each member's presence
//! for the board page's People panel, from what this device sees of their
//! devices on the board, and how many of their agents on it are working or
//! free.
//!
//! - **online**: a board connection to one of their devices is up (this
//!   person is always online here).
//! - **recent**: one of their devices was heard from, or wrote a row,
//!   within [`mesh::node::RECENT_WITHIN`].
//! - **offline**: neither; `last_seen` says when, if this device knows.
//!
//! [`member_presence`] is pure; [`Services::board_presence`] gathers its
//! input from the board, its mesh and the endpoint.

use std::collections::{BTreeMap, BTreeSet};

use store::kinds::AGENT_ACTOR_PREFIX;

use super::Services;
use super::boards::BoardLog;

/// Online: a board connection to one of their devices is up.
pub const PRESENCE_ONLINE: &str = "online";
/// Recent: heard from within [`mesh::node::RECENT_WITHIN`].
pub const PRESENCE_RECENT: &str = "recent";
/// Offline: not heard from lately.
pub const PRESENCE_OFFLINE: &str = "offline";

/// What [`member_presence`] reads.
#[derive(Debug, Clone, Default)]
pub struct PresenceInput {
    /// The person at this device.
    pub me: String,
    /// The members to report, in order.
    pub people: Vec<String>,
    /// The person each member device is certified to.
    pub device_person: BTreeMap<String, String>,
    /// Devices with a board connection up now.
    pub connected: BTreeSet<String>,
    /// Unix ms of each device's last frame on the board (this run only).
    pub heard_ms: BTreeMap<String, i64>,
    /// Unix ms of each device's newest row in the board log (survives a
    /// restart, where `heard_ms` does not).
    pub wrote_ms: BTreeMap<String, i64>,
    /// The board's agents: their person and roster status.
    pub agents: Vec<(String, String)>,
    pub now_ms: i64,
}

/// One member's presence.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MemberPresence {
    pub person_id: String,
    /// [`PRESENCE_ONLINE`], [`PRESENCE_RECENT`] or [`PRESENCE_OFFLINE`].
    pub presence: String,
    /// Unix ms they were last seen, 0 when unknown (and for this person).
    pub last_seen_ms: i64,
    /// Their agents on the board that are working (not idle, not gone).
    pub working: i64,
    /// Their agents on the board that are idle.
    pub free: i64,
}

/// Each of `i.people`'s presence, in order.
pub fn member_presence(i: &PresenceInput) -> Vec<MemberPresence> {
    let recent_ms = mesh::node::RECENT_WITHIN.as_millis() as i64;
    i.people
        .iter()
        .map(|p| {
            let devs: Vec<&String> = i
                .device_person
                .iter()
                .filter(|(_, q)| *q == p)
                .map(|(d, _)| d)
                .collect();
            let last_seen_ms = devs
                .iter()
                .flat_map(|d| [i.heard_ms.get(*d), i.wrote_ms.get(*d)])
                .flatten()
                .copied()
                .max()
                .unwrap_or(0);
            let presence = if *p == i.me || devs.iter().any(|d| i.connected.contains(*d)) {
                PRESENCE_ONLINE
            } else if last_seen_ms > 0 && i.now_ms - last_seen_ms <= recent_ms {
                PRESENCE_RECENT
            } else {
                PRESENCE_OFFLINE
            };
            let (mut working, mut free) = (0, 0);
            for (q, status) in &i.agents {
                if q != p || status == "gone" {
                    continue;
                }
                if status == "idle" {
                    free += 1;
                } else {
                    working += 1;
                }
            }
            MemberPresence {
                person_id: p.clone(),
                presence: presence.into(),
                last_seen_ms: if *p == i.me { 0 } else { last_seen_ms },
                working,
                free,
            }
        })
        .collect()
}

impl Services {
    /// `people`'s presence on board `b` ([`member_presence`]).
    pub(crate) fn board_presence(&self, b: &BoardLog, people: &[String]) -> Vec<MemberPresence> {
        let me = self.person_id();
        let mut device_person = BTreeMap::new();
        for p in people {
            for d in b.verifier.devices(p) {
                device_person.insert(d.cert.device_id, p.clone());
            }
        }
        // Each device as device_presence sees it (card #372): the private
        // mesh for this person's devices, every board's connections and
        // frames for a member's, the newest board row after a restart.
        let (mut connected, mut heard_ms) = (BTreeSet::new(), BTreeMap::new());
        for (d, p) in &device_person {
            if *p == me {
                continue;
            }
            let dp = self.device_presence(d);
            if dp.state == coord::roster::CONNECTED {
                connected.insert(d.clone());
            }
            if !dp.seen.is_zero() {
                heard_ms.insert(d.clone(), dp.seen.unix_nano() / 1_000_000);
            }
        }
        let wrote_ms = BTreeMap::new();
        let agents = {
            let roster = super::boards::roster_of(self, b);
            let m = super::boards::lock(&b.maps);
            roster
                .into_iter()
                .filter_map(|a| {
                    let id = a.key.strip_prefix(AGENT_ACTOR_PREFIX)?;
                    let card = m.cards.get(id)?;
                    // The Office Assistant is not a working agent.
                    if card.kind == coord::roster::ASSISTANT_KIND {
                        return None;
                    }
                    Some((card.person_id.clone(), a.status))
                })
                .collect()
        };
        member_presence(&PresenceInput {
            me,
            people: people.to_vec(),
            device_person,
            connected,
            heard_ms,
            wrote_ms,
            agents,
            now_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 100_000_000;
    const MIN: i64 = 60_000;

    fn base() -> PresenceInput {
        PresenceInput {
            me: "pAlice".into(),
            people: vec!["pAlice".into(), "pBlake".into(), "pCasey".into()],
            device_person: [
                ("a1", "pAlice"),
                ("b1", "pBlake"),
                ("b2", "pBlake"),
                ("c1", "pCasey"),
            ]
            .into_iter()
            .map(|(a, b)| (a.into(), b.into()))
            .collect(),
            now_ms: NOW,
            ..Default::default()
        }
    }

    fn of<'a>(out: &'a [MemberPresence], p: &str) -> &'a MemberPresence {
        out.iter().find(|m| m.person_id == p).unwrap()
    }

    #[test]
    fn this_person_is_online_and_nobody_heard_is_offline() {
        let out = member_presence(&base());
        assert_eq!(out.len(), 3);
        assert_eq!(of(&out, "pAlice").presence, PRESENCE_ONLINE);
        assert_eq!(of(&out, "pAlice").last_seen_ms, 0);
        assert_eq!(of(&out, "pBlake").presence, PRESENCE_OFFLINE);
        assert_eq!(of(&out, "pBlake").last_seen_ms, 0);
    }

    #[test]
    fn any_connected_device_makes_them_online() {
        let mut i = base();
        i.connected.insert("b2".into());
        i.heard_ms.insert("b2".into(), NOW - 2_000);
        let out = member_presence(&i);
        assert_eq!(of(&out, "pBlake").presence, PRESENCE_ONLINE);
        assert_eq!(of(&out, "pBlake").last_seen_ms, NOW - 2_000);
        assert_eq!(of(&out, "pCasey").presence, PRESENCE_OFFLINE);
    }

    #[test]
    fn heard_within_ten_minutes_is_recent_older_is_offline() {
        let mut i = base();
        i.heard_ms.insert("b1".into(), NOW - 5 * MIN);
        i.heard_ms.insert("c1".into(), NOW - 2 * 60 * MIN);
        let out = member_presence(&i);
        assert_eq!(of(&out, "pBlake").presence, PRESENCE_RECENT);
        assert_eq!(of(&out, "pCasey").presence, PRESENCE_OFFLINE);
        assert_eq!(of(&out, "pCasey").last_seen_ms, NOW - 2 * 60 * MIN);
    }

    #[test]
    fn last_seen_is_the_newest_of_heard_and_wrote_over_their_devices() {
        let mut i = base();
        i.heard_ms.insert("b1".into(), NOW - 30 * MIN);
        i.wrote_ms.insert("b2".into(), NOW - 4 * MIN);
        let out = member_presence(&i);
        assert_eq!(of(&out, "pBlake").last_seen_ms, NOW - 4 * MIN);
        assert_eq!(of(&out, "pBlake").presence, PRESENCE_RECENT);
    }

    #[test]
    fn agents_count_as_working_or_free_per_person_and_gone_ones_not_at_all() {
        let mut i = base();
        i.agents = [
            ("pAlice", "active"),
            ("pAlice", "idle"),
            ("pAlice", "gone"),
            ("pBlake", "busy"),
            ("pBlake", "shell"),
            ("pBlake", "idle"),
        ]
        .into_iter()
        .map(|(a, b)| (a.into(), b.into()))
        .collect();
        let out = member_presence(&i);
        assert_eq!(
            (of(&out, "pAlice").working, of(&out, "pAlice").free),
            (1, 1)
        );
        assert_eq!(
            (of(&out, "pBlake").working, of(&out, "pBlake").free),
            (2, 1)
        );
        assert_eq!(
            (of(&out, "pCasey").working, of(&out, "pCasey").free),
            (0, 0)
        );
    }
}
