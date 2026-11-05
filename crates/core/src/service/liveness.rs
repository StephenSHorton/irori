//! Where an agent's device is (card #372): online, recent or offline, and
//! when it was last heard from, for this person's devices and for other
//! members' devices on shared boards alike.

use gocompat::time::GoTime;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::Services;

/// "recent" lasts this long after a device was last heard (mesh
/// `RECENT_WITHIN`).
const RECENT_WITHIN: Duration = Duration::from_secs(600);

/// One device's presence: `connected`, `recent` or `offline`
/// (coord::roster's words) and when it was last heard (zero: never).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DevicePresence {
    pub state: String,
    pub seen: GoTime,
    /// The name of the device its rows come through (card #544).
    pub via: String,
}

impl Services {
    /// Device `dev`'s presence. This device is connected now. Another of
    /// the person's devices reads the private mesh (as Settings → Devices
    /// does); any other device, a member's on a shared board, reads the
    /// board meshes: connected while some board has a live connection to
    /// it, else recent or offline by the last frame any board heard.
    pub(crate) fn device_presence(&self, dev: &str) -> DevicePresence {
        if dev.is_empty() {
            return DevicePresence {
                state: coord::roster::OFFLINE.into(),
                ..Default::default()
            };
        }
        if dev == self.device.id {
            return DevicePresence {
                state: coord::roster::CONNECTED.into(),
                seen: GoTime::from_system(SystemTime::now()),
                ..Default::default()
            };
        }
        let node = &self.mesh.node;
        if self.chain.cert(dev).is_some() {
            let r = node.device_read(dev);
            return DevicePresence {
                state: r.state.into(),
                seen: r.seen.map(GoTime::from_system).unwrap_or_default(),
                via: super::via_name(&self.chain, &self.device_names, &r.via),
            };
        }
        let mut connected = false;
        let mut at_ms = 0i64;
        for m in self.mesh.board_list() {
            connected |= m.peers().iter().any(|(_, d)| d == dev);
            for h in m.heard() {
                if h.device_id == dev {
                    at_ms = at_ms.max(h.at_ms);
                }
            }
        }
        if at_ms == 0 {
            // Nothing heard since this Core started (heard is in memory):
            // the newest row it wrote to a board here, so a restart still
            // says when it was last seen.
            for b in self.shown_boards() {
                if let Ok(evs) = b.store.list(&store::Filter {
                    device_id: dev.to_string(),
                    desc: true,
                    limit: 1,
                    ..Default::default()
                }) && let Some(ev) = evs.first()
                {
                    at_ms = at_ms.max(ev.hlc.wall_ms());
                }
            }
        }
        board_presence(connected, at_ms, SystemTime::now())
    }
}

/// Another member's agent on a shared board, for the agents tool.
#[derive(Debug, Clone, Default)]
pub struct BoardAgent {
    /// The board's tag (`mlg`), or its name when it has none.
    pub board: String,
    /// Its owner's name on the board, "" when unknown.
    pub person: String,
    pub card: coord::roster::AgentCard,
}

impl Services {
    /// Every other member's agent on the shared boards this device shows,
    /// keyed as the board writes it (`agent:<cardId>`), with its device's
    /// presence (card #372). The person's own agents are left out: the
    /// private roster lists them.
    pub(crate) fn board_others(&self) -> Vec<BoardAgent> {
        let me = self.person_id();
        let mut out = Vec::new();
        for b in self.shown_boards() {
            let mem = b.verifier.membership();
            if !mem.created {
                continue;
            }
            let label = if mem.tag.is_empty() {
                mem.name.clone()
            } else {
                mem.tag.clone()
            };
            let theirs: std::collections::HashMap<String, String> = {
                let m = super::boards::lock(&b.maps);
                m.cards
                    .iter()
                    .filter(|(id, c)| c.person_id != me && !m.by_id.contains_key(*id))
                    .map(|(id, c)| {
                        (
                            format!("{}{id}", store::kinds::AGENT_ACTOR_PREFIX),
                            c.person_id.clone(),
                        )
                    })
                    .collect()
            };
            for a in super::boards::roster_of(self, &b) {
                let Some(pid) = theirs.get(&a.key) else {
                    continue;
                };
                let person = b
                    .verifier
                    .profile(pid)
                    .or_else(|| mem.person(pid).map(|p| p.name.clone()))
                    .unwrap_or_default();
                out.push(BoardAgent {
                    board: label.clone(),
                    person,
                    card: a,
                });
            }
        }
        out
    }
}

/// A member device's presence from the board meshes: live connection,
/// and the last frame heard (Unix ms, 0 never).
pub(crate) fn board_presence(connected: bool, at_ms: i64, now: SystemTime) -> DevicePresence {
    let seen = if at_ms > 0 {
        GoTime::from_system(UNIX_EPOCH + Duration::from_millis(at_ms as u64))
    } else {
        GoTime::default()
    };
    let state = if connected {
        coord::roster::CONNECTED
    } else if at_ms > 0
        && now
            .duration_since(UNIX_EPOCH + Duration::from_millis(at_ms as u64))
            .is_ok_and(|d| d < RECENT_WITHIN)
    {
        coord::roster::RECENT
    } else {
        coord::roster::OFFLINE
    };
    DevicePresence {
        state: state.into(),
        seen,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_member_device_reads_as_its_board_meshes_heard_it() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
        let ms = |s: u64| (1_000_000 - s) as i64 * 1000;
        assert_eq!(board_presence(true, ms(5), now).state, "connected");
        assert_eq!(board_presence(false, ms(60), now).state, "recent");
        assert_eq!(board_presence(false, ms(3600), now).state, "offline");
        let never = board_presence(false, 0, now);
        assert_eq!(never.state, "offline");
        assert!(never.seen.is_zero());
        assert_eq!(
            board_presence(false, ms(60), now).seen.unix_nano(),
            ms(60) * 1_000_000
        );
    }
}
