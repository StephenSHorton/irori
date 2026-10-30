//! The boards this device holds, as admission sees them (boards stage 2,
//! card #43; docs/design/boards2-transport.md §3 and §6, B1).
//!
//! [`HeldBoards`] is the one [`BoardAdmit`]: a board id per open board and
//! that board's member check ([`BoardMembers`], the board's
//! `BoardVerifier` in the Core). The endpoint gate ([`MemberAdmit`] on
//! `irori/board/1` and `irori/gossip/1`), the home relay's access rule
//! ([`HeldBoards::relay_admit`]) and a board's gossip topic
//! ([`HeldBoards::gossip_accept`]) all read it, so a board that is closed,
//! or a member removed from it, stops being admitted everywhere at once.
//!
//! [`MemberAdmit`]: crate::board::MemberAdmit

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, RwLock};

use iroh::EndpointId;
use irori_identity::BoardVerifier;

use crate::board::BoardAdmit;

/// One board's member check by device key.
pub trait BoardMembers: Send + Sync {
    /// The device id of the counted, uncut member device whose devicePub is
    /// `device_pub_hex` (lower-case hex), or `None`.
    fn admitted_device(&self, device_pub_hex: &str) -> Option<String>;
}

impl BoardMembers for BoardVerifier {
    fn admitted_device(&self, device_pub_hex: &str) -> Option<String> {
        self.admitted_by_pub(device_pub_hex).map(|c| c.device_id)
    }
}

/// The boards this device holds and their member checks. Cheap to clone;
/// every clone sees the same boards.
#[derive(Clone, Default)]
pub struct HeldBoards(Arc<RwLock<BTreeMap<String, Arc<dyn BoardMembers>>>>);

impl fmt::Debug for HeldBoards {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("HeldBoards").field(&self.ids()).finish()
    }
}

impl HeldBoards {
    /// Holds board `board_id` with its member check (replacing an earlier one).
    pub fn hold(&self, board_id: &str, members: Arc<dyn BoardMembers>) {
        self.0
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(board_id.to_string(), members);
    }

    /// Stops holding board `board_id` (left, removed, forgotten).
    pub fn release(&self, board_id: &str) {
        self.0
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(board_id);
    }

    /// The boards held, by id.
    pub fn ids(&self) -> Vec<String> {
        self.read().keys().cloned().collect()
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<String, Arc<dyn BoardMembers>>> {
        self.0.read().unwrap_or_else(|e| e.into_inner())
    }

    /// B1: the home relay's widening, for `home::Config.board_admit`. True
    /// for a counted, uncut member device of any board held here. Which
    /// board it may reach is the endpoints' to decide (the board-open proof
    /// and that board's verifier); the relay only forwards sealed QUIC.
    pub fn relay_admit(&self) -> Arc<dyn Fn(&EndpointId) -> bool + Send + Sync> {
        let me = self.clone();
        Arc::new(move |id: &EndpointId| !me.boards_of(id).is_empty())
    }

    /// The check a board's gossip topic applies to the neighbour that
    /// delivered an announcement: a member device of *that* board (a member
    /// of another board held here passes the endpoint gate but not this).
    pub fn gossip_accept(&self, board_id: &str) -> Arc<dyn Fn(&EndpointId) -> bool + Send + Sync> {
        let (me, board) = (self.clone(), board_id.to_string());
        Arc::new(move |id: &EndpointId| me.board_device(&board, id).is_some())
    }
}

impl BoardAdmit for HeldBoards {
    fn board_device(&self, board_id: &str, id: &EndpointId) -> Option<String> {
        let m = self.read().get(board_id).cloned()?;
        m.admitted_device(&hex::encode(id.as_bytes()))
    }

    fn boards_of(&self, id: &EndpointId) -> Vec<String> {
        let key = hex::encode(id.as_bytes());
        let boards: Vec<(String, Arc<dyn BoardMembers>)> = self
            .read()
            .iter()
            .map(|(b, m)| (b.clone(), m.clone()))
            .collect();
        boards
            .into_iter()
            .filter(|(_, m)| m.admitted_device(&key).is_some())
            .map(|(b, _)| b)
            .collect()
    }
}

/// A fixed member list by device key (tests, and boards whose verifier is
/// not open yet).
#[derive(Debug, Clone, Default)]
pub struct MemberKeys(pub Arc<RwLock<BTreeMap<String, String>>>);

impl MemberKeys {
    /// Counts device `device_id` with key `id` as a member.
    pub fn add(&self, device_id: &str, id: &EndpointId) {
        self.0
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(hex::encode(id.as_bytes()), device_id.to_string());
    }

    /// Drops the device with key `id` (revoked, removed, left).
    pub fn remove(&self, id: &EndpointId) {
        self.0
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&hex::encode(id.as_bytes()));
    }
}

impl BoardMembers for MemberKeys {
    fn admitted_device(&self, device_pub_hex: &str) -> Option<String> {
        self.0
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(device_pub_hex)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admit::{Admit, ChainAdmit};
    use crate::board::{BOARD_ALPN, MemberAdmit};
    use crate::transport::{GOSSIP_ALPN, MESH_ALPN, PAIR_ALPN};

    fn key(b: u8) -> EndpointId {
        iroh::SecretKey::from_bytes(&[b; 32]).public()
    }

    #[test]
    fn held_boards_admit_per_board_and_follow_removal() {
        let held = HeldBoards::default();
        let (a, b) = (MemberKeys::default(), MemberKeys::default());
        a.add("blake-pc", &key(1));
        b.add("cara-pc", &key(2));
        held.hold("boardA", Arc::new(a.clone()));
        held.hold("boardB", Arc::new(b));
        assert_eq!(held.boards_of(&key(1)), vec!["boardA".to_string()]);
        assert_eq!(
            held.board_device("boardA", &key(1)).as_deref(),
            Some("blake-pc")
        );
        assert_eq!(
            held.board_device("boardB", &key(1)),
            None,
            "A's member on B"
        );
        assert!(held.boards_of(&key(3)).is_empty());
        let relay = held.relay_admit();
        assert!(relay(&key(1)) && relay(&key(2)) && !relay(&key(3)));
        let gossip_a = held.gossip_accept("boardA");
        assert!(gossip_a(&key(1)) && !gossip_a(&key(2)));
        a.remove(&key(1));
        assert!(!relay(&key(1)) && !gossip_a(&key(1)));
        held.release("boardB");
        assert!(!relay(&key(2)));
    }

    #[test]
    fn member_admit_lets_board_members_onto_board_and_gossip_only() {
        let chain = Arc::new(irori_identity::Chain::new("p", "root", "root"));
        let held = HeldBoards::default();
        let m = MemberKeys::default();
        m.add("blake-pc", &key(1));
        held.hold("boardA", Arc::new(m));
        let gate = MemberAdmit {
            chain: ChainAdmit {
                chain,
                self_id: "root".into(),
            },
            boards: Arc::new(held),
        };
        for alpn in [BOARD_ALPN, GOSSIP_ALPN] {
            assert!(gate.admits(alpn, &key(1)));
            assert!(!gate.admits(alpn, &key(2)));
        }
        for alpn in [MESH_ALPN, PAIR_ALPN, b"irori/other/1".as_slice()] {
            assert!(!gate.admits(alpn, &key(1)), "a member on {alpn:?}");
        }
    }
}
