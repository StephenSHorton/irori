//! Boards stage 2 on iroh: the `irori/board/1` connection, its admission,
//! and the board servers a member dials (docs/design/boards2-transport.md,
//! architecture §3.3, v0.5-shared-boards §6).
//!
//! INTERFACE ONLY (card #43). The pure pieces (ALPNs, the open proof, the
//! ALPN-aware admission) are real; everything that moves bytes returns
//! `TransportError(UNSUPPORTED)` until the transport branch lands. Nothing
//! here is wired into the endpoint yet, so behaviour is unchanged.

use std::fmt;
use std::sync::Arc;

use iroh::EndpointId;
use irori_meshseam::{
    BoardRoom, BoardTransport, BoxFut, Room, RoomConfig, TransportError, UNSUPPORTED,
};
use serde::{Deserialize, Serialize};

use crate::admit::{Admit, ChainAdmit};
use crate::endpoint::RelaySpec;
use crate::transport::{GOSSIP_ALPN, IrohNet, MESH_ALPN, PAIR_ALPN};

/// The board connection's ALPN: one connection per (device, device, board).
pub const BOARD_ALPN: &[u8] = b"irori/board/1";
/// The invite room's ALPN (the invites part, card #44, speaks on it).
pub const INVITE_ALPN: &[u8] = b"irori/invite/1";
/// The board-open proof's label.
pub const BOARD_OPEN_LABEL: &[u8] = b"irori/board-open/1";
/// The TLS exporter label of the board-open proof's EKM32.
pub const EKM_BOARD: &[u8] = b"EXPORTER-irori-board";
/// The board-open message's version.
pub const BOARD_OPEN_V: u32 = 1;

/// The first message on a board connection's `sync` stream, each way:
/// `{v, boardId, epoch, deviceId, proof}` where
/// `proof = hex(HMAC-SHA256(BoardTopic(boardId, epoch).key,
/// "irori/board-open/1" || EKM32 || role))`, role 0 for the dialer and 1
/// for the acceptor (the mesh-open proof's shape, mesh.md §5.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardOpenMsg {
    pub v: u32,
    #[serde(rename = "boardId")]
    pub board_id: String,
    pub epoch: i64,
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub proof: String,
}

/// The board-open proof for `key` (the board topic key of the epoch named).
pub fn board_open_proof(key: &[u8; 32], ekm: &[u8; 32], role: u8) -> String {
    crate::transport::open_proof(BOARD_OPEN_LABEL, key, ekm, role)
}

/// Checks a board-open proof in constant time.
pub fn board_open_proof_ok(key: &[u8; 32], ekm: &[u8; 32], role: u8, got: &str) -> bool {
    crate::transport::open_proof_ok(BOARD_OPEN_LABEL, key, ekm, role, got)
}

/// Board membership as the endpoint and the relay see it. The Core
/// implements it over every open board's `BoardVerifier`
/// (`BoardVerifier::admitted_by_pub`).
pub trait BoardAdmit: Send + Sync + fmt::Debug {
    /// The device id of `id` when it holds a counted, uncut cert of a
    /// current member of board `board_id`, and this device holds that board.
    fn board_device(&self, board_id: &str, id: &EndpointId) -> Option<String>;
    /// The boards this device holds on which `id` is a counted, uncut
    /// member device. Empty = not a member device anywhere here.
    fn boards_of(&self, id: &EndpointId) -> Vec<String>;
}

/// [`BoardAdmit`] with no boards (single-person builds, tests).
#[derive(Debug, Clone, Default)]
pub struct NoBoards;

impl BoardAdmit for NoBoards {
    fn board_device(&self, _board_id: &str, _id: &EndpointId) -> Option<String> {
        None
    }
    fn boards_of(&self, _id: &EndpointId) -> Vec<String> {
        Vec::new()
    }
}

/// The stage 2 endpoint gate: `irori/mesh/1` and `irori/pair/1` admit only
/// this person's certified devices ([`ChainAdmit`], unchanged);
/// `irori/board/1` and `irori/gossip/1` also admit member devices counted
/// on at least one board held here. The per-board check is the board
/// room's `RoomConfig::accept` after the board-open proof.
#[derive(Clone, Debug)]
pub struct MemberAdmit {
    pub chain: ChainAdmit,
    pub boards: Arc<dyn BoardAdmit>,
}

impl Admit for MemberAdmit {
    fn admits(&self, alpn: &[u8], id: &EndpointId) -> bool {
        if alpn == MESH_ALPN || alpn == PAIR_ALPN {
            return self.chain.admits(alpn, id);
        }
        if alpn == BOARD_ALPN || alpn == GOSSIP_ALPN {
            return self.chain.admits(alpn, id) || !self.boards.boards_of(id).is_empty();
        }
        false
    }
}

/// One board's `board.server` relays, pinned to each server's devicePub
/// from its member cert (B1: the member's endpoint adds them as relays).
#[derive(Debug, Clone)]
pub struct BoardRelays {
    pub board_id: String,
    pub servers: Vec<RelaySpec>,
}

impl IrohNet {
    /// Replaces the relays fed from `board.server` rows of every board this
    /// device holds (dialled in parallel, first handshake per server kept).
    pub async fn set_board_servers(&self, _boards: Vec<BoardRelays>) -> Result<(), TransportError> {
        Err(TransportError(UNSUPPORTED.into()))
    }

    /// Closes every connection of board `board_id` (removal, leave, forget).
    pub fn drop_board(&self, _board_id: &str, _why: &str) -> Result<(), TransportError> {
        Err(TransportError(UNSUPPORTED.into()))
    }
}

impl BoardTransport for IrohNet {
    fn join_board(
        &self,
        _room: BoardRoom,
        _cfg: RoomConfig,
    ) -> BoxFut<'_, Result<Arc<dyn Room>, TransportError>> {
        Box::pin(async { Err(TransportError(UNSUPPORTED.into())) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_proof_round_trips_and_is_role_bound() {
        let (key, ekm) = ([7u8; 32], [9u8; 32]);
        let p = board_open_proof(&key, &ekm, 0);
        assert!(board_open_proof_ok(&key, &ekm, 0, &p));
        assert!(!board_open_proof_ok(&key, &ekm, 1, &p));
        assert!(!board_open_proof_ok(&[8u8; 32], &ekm, 0, &p));
    }
}
