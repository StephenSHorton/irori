//! Boards stage 2 on iroh: the `irori/board/1` connection, its admission,
//! and the board servers a member dials (docs/design/boards2-transport.md,
//! architecture §3.3, v0.5-shared-boards §6).
//!
//! The endpoint offers `irori/board/1` and routes it to [`Inner::handle_board`];
//! [`BoardTransport::join_board`] opens a board room per (board, epoch);
//! [`BoardMembers`] is the Core's [`BoardAdmit`] over its open boards'
//! verifiers. `set_board_servers` (B1 relays) is the gossip-relay half's.

use std::fmt;
use std::sync::Arc;

use iroh::EndpointId;
use iroh::endpoint::Connection;
use irori_identity::BoardVerifier;
use irori_meshseam::{
    BoardRoom, BoardTransport, BoxFut, ConnError, MAX_LABEL, Room, RoomConfig, SYNC_LABEL,
    TransportError, read_frame, reason, write_frame,
};
use serde::{Deserialize, Serialize};

use crate::admit::{Admit, ChainAdmit};
use crate::endpoint::RelaySpec;
use crate::transport::{
    ConnInfo, GOSSIP_ALPN, Inner, IrohNet, IrohRoom, MESH_ALPN, OPEN_MAX, PAIR_ALPN, PeerAddr,
    ROLE_ACCEPT, ROLE_DIAL, RoomState, close_conn, closed_error, ekm,
};

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
    /// device holds (B1). Each server's TLS key is pinned (the relay's TLS
    /// is on its device key, D3); the relay is not added to the endpoint's
    /// relay map, so another person's server never becomes this device's
    /// home relay. iroh opens a relay connection on demand when a member
    /// device's address names it (`set_board_peers` with `relay` set), and
    /// the server's relay admits this device as a member of a board it holds
    /// (`home::Config.board_admit`). Pins no board and no home relay names
    /// any more are dropped.
    pub async fn set_board_servers(&self, boards: Vec<BoardRelays>) -> Result<(), TransportError> {
        let inner = self.inner();
        let keys: Vec<[u8; 32]> = boards
            .iter()
            .flat_map(|b| b.servers.iter().map(|s| *s.server_pub.as_bytes()))
            .collect();
        for k in &keys {
            inner.pins.add(*k);
        }
        let old = std::mem::replace(
            &mut *inner.board_relays.lock().expect("board relays"),
            boards,
        );
        let mesh = inner.relays.lock().expect("relays").clone();
        for o in old.iter().flat_map(|b| b.servers.iter()) {
            let k = o.server_pub.as_bytes();
            if !keys.contains(k) && !mesh.iter().any(|m| m.server_pub == o.server_pub) {
                inner.pins.remove(k);
            }
        }
        Ok(())
    }

    /// The `board.server` relays as last fed, for status lines and tests.
    pub fn board_servers(&self) -> Vec<BoardRelays> {
        self.inner()
            .board_relays
            .lock()
            .expect("board relays")
            .clone()
    }

    /// Closes every connection of board `board_id` (removal, leave, forget)
    /// with `why`, leaves its rooms and forgets its member devices.
    pub fn drop_board(&self, board_id: &str, why: &str) -> Result<(), TransportError> {
        let inner = self.inner();
        let rooms: Vec<_> = inner
            .board_rooms_snapshot()
            .into_iter()
            .filter(|r| r.board.as_deref() == Some(board_id))
            .collect();
        for r in rooms {
            inner.close_room(&r, why);
        }
        inner
            .board_book
            .lock()
            .expect("board book")
            .remove(board_id);
        Ok(())
    }

    /// Board `board_id`'s member devices and where to dial them (B1: from
    /// the board's counted certs, `board.server` rows and invite hints). A
    /// device known by its id alone leaves what the lookups hold for it.
    /// Kept apart from [`IrohNet::set_peers`]' private book: a board peer is
    /// never dialled on `irori/mesh/1`.
    pub fn set_board_peers(&self, board_id: &str, peers: Vec<PeerAddr>) {
        let inner = self.inner();
        let mut m = std::collections::BTreeMap::new();
        for p in peers {
            if p.relay.is_some() || !p.direct.is_empty() {
                inner.lookup.set_endpoint_info(p.endpoint_addr());
            }
            m.insert(p.device_id.clone(), p);
        }
        inner
            .board_book
            .lock()
            .expect("board book")
            .insert(board_id.to_string(), m);
    }

    /// Live board connections of `board_id`, by epoch then peer.
    pub fn board_conns(&self, board_id: &str) -> Vec<ConnInfo> {
        let mut out = Vec::new();
        for room in self.inner().board_rooms_snapshot() {
            if room.board.as_deref() != Some(board_id) {
                continue;
            }
            out.extend(room.conn_infos());
        }
        out.sort_by(|a, b| (a.epoch, &a.peer).cmp(&(b.epoch, &b.peer)));
        out
    }

    /// The board rooms open here, as (board, epoch).
    pub fn board_rooms(&self) -> Vec<(String, i64)> {
        let mut v: Vec<(String, i64)> = self
            .inner()
            .board_rooms
            .lock()
            .expect("board rooms")
            .keys()
            .cloned()
            .collect();
        v.sort();
        v
    }
}

impl BoardTransport for IrohNet {
    fn join_board(
        &self,
        room: BoardRoom,
        cfg: RoomConfig,
    ) -> BoxFut<'_, Result<Arc<dyn Room>, TransportError>> {
        Box::pin(async move {
            if room.secret.epoch < 1 || room.board_id.is_empty() {
                return Err(TransportError(
                    "boards2: a board room needs a board and an epoch".into(),
                ));
            }
            let inner = self.inner();
            inner.bind().await?;
            let mut st = RoomState::new(room.secret, cfg);
            st.board = Some(room.board_id.clone());
            let state = Arc::new(st);
            let old = inner
                .board_rooms
                .lock()
                .expect("board rooms")
                .insert((room.board_id, state.secret.epoch), state.clone());
            if let Some(old) = old {
                // The epoch's secret changed (the lower maker won): the old
                // room goes.
                inner.close_room(&old, reason::LEAVING);
            }
            let on_connect = state.cfg.on_connect.clone();
            tokio::spawn(async move { on_connect() });
            Ok(Arc::new(IrohRoom {
                net: Arc::downgrade(inner),
                state,
            }) as Arc<dyn Room>)
        })
    }
}

impl Inner {
    /// The board-open exchange, dialer side: write our [`BoardOpenMsg`] on
    /// `sync`, read the acceptor's, check its proof (role 1) and that the
    /// device it names is `peer` and admitted by the board room.
    pub(crate) async fn dial_board(
        self: &Arc<Self>,
        conn: Connection,
        room: &Arc<RoomState>,
        board: &str,
        peer: &str,
        addr: &PeerAddr,
    ) -> Result<(), ConnError> {
        let epoch = room.secret.epoch;
        let k = ekm(&conn, EKM_BOARD)?;
        let hs = async {
            let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
            write_frame(&mut send, SYNC_LABEL.as_bytes(), MAX_LABEL)
                .await
                .map_err(|e| e.to_string())?;
            let open = BoardOpenMsg {
                v: BOARD_OPEN_V,
                board_id: board.to_string(),
                epoch,
                device_id: self.cfg.self_id.clone(),
                proof: board_open_proof(&room.secret.key, &k, ROLE_DIAL),
            };
            let b = serde_json::to_vec(&open).expect("open msg");
            write_frame(&mut send, &b, OPEN_MAX)
                .await
                .map_err(|e| e.to_string())?;
            let reply = read_frame(&mut recv, OPEN_MAX, false)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "no board-open reply".to_string())?;
            let reply: BoardOpenMsg = serde_json::from_slice(&reply).map_err(|e| e.to_string())?;
            Ok::<_, String>((send, recv, reply))
        };
        let (send, recv, reply) = match tokio::time::timeout(self.cfg.open_timeout, hs).await {
            Err(_) => {
                close_conn(&conn, reason::TIMEOUT);
                return Err(ConnError::Local(reason::TIMEOUT.into()));
            }
            Ok(Err(e)) => {
                let err = closed_error(&conn, e);
                close_conn(&conn, reason::PROTOCOL);
                return Err(err);
            }
            Ok(Ok(x)) => x,
        };
        if reply.v != BOARD_OPEN_V {
            close_conn(&conn, reason::VERSION);
            return Err(ConnError::Local(reason::VERSION.into()));
        }
        if reply.board_id != board
            || reply.epoch != epoch
            || !board_open_proof_ok(&room.secret.key, &k, ROLE_ACCEPT, &reply.proof)
            || reply.device_id != peer
            || (room.cfg.accept)(peer, addr.id.as_bytes()).is_err()
        {
            close_conn(&conn, reason::REFUSED);
            return Err(ConnError::Local(reason::REFUSED.into()));
        }
        self.setup(
            conn,
            room,
            peer.to_string(),
            self.cfg.self_id.clone(),
            send,
            recv,
        )
    }

    /// The board-open exchange, acceptor side. The endpoint gate already let
    /// the key in as a member device somewhere here; this is the per-board
    /// check: the room for (board, epoch), the proof of its key, and the
    /// board room's `accept` (that board's verifier). Any failure closes
    /// `refused` (`version` for an unknown open version) before a frame.
    pub(crate) async fn handle_board(self: &Arc<Self>, conn: Connection) {
        let hs = async {
            let (send, mut recv) = conn.accept_bi().await.map_err(|e| e.to_string())?;
            let label = read_frame(&mut recv, MAX_LABEL, false)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "no label".to_string())?;
            if label != SYNC_LABEL.as_bytes() {
                return Err("first stream is not sync".to_string());
            }
            let open = read_frame(&mut recv, OPEN_MAX, false)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "no board-open message".to_string())?;
            let open: BoardOpenMsg = serde_json::from_slice(&open).map_err(|e| e.to_string())?;
            Ok::<_, String>((send, recv, open))
        };
        let (mut send, recv, open) = match tokio::time::timeout(self.cfg.open_timeout, hs).await {
            Ok(Ok(x)) => x,
            Ok(Err(_)) => return close_conn(&conn, reason::PROTOCOL),
            Err(_) => return close_conn(&conn, reason::TIMEOUT),
        };
        if open.v != BOARD_OPEN_V {
            return close_conn(&conn, reason::VERSION);
        }
        let room = self
            .board_rooms
            .lock()
            .expect("board rooms")
            .get(&(open.board_id.clone(), open.epoch))
            .cloned();
        let Some(room) = room else {
            return close_conn(&conn, reason::REFUSED);
        };
        let Ok(k) = ekm(&conn, EKM_BOARD) else {
            return close_conn(&conn, reason::PROTOCOL);
        };
        if !board_open_proof_ok(&room.secret.key, &k, ROLE_DIAL, &open.proof) {
            return close_conn(&conn, reason::REFUSED);
        }
        let peer = open.device_id;
        let pubkey = *conn.remote_id().as_bytes();
        if peer == self.cfg.self_id
            || (room.cfg.accept)(&peer, &pubkey).is_err()
            || !(room.cfg.allow)(&peer)
        {
            return close_conn(&conn, reason::REFUSED);
        }
        let reply = BoardOpenMsg {
            v: BOARD_OPEN_V,
            board_id: open.board_id,
            epoch: open.epoch,
            device_id: self.cfg.self_id.clone(),
            proof: board_open_proof(&room.secret.key, &k, ROLE_ACCEPT),
        };
        let b = serde_json::to_vec(&reply).expect("open msg");
        if write_frame(&mut send, &b, OPEN_MAX).await.is_err() {
            return close_conn(&conn, reason::PROTOCOL);
        }
        let _ = self.setup(conn, &room, peer.clone(), peer, send, recv);
    }
}

/// The Core's [`BoardAdmit`]: every open board's verifier, by board id.
/// Interior-mutable, so boards opened after the endpoint bound count at its
/// gate (which holds this from bind on). It is the one registry
/// (boards2-transport.md §9): it keeps its boards in a
/// [`HeldBoards`](crate::boardadmit::HeldBoards), so the endpoint gate, the
/// home relay ([`BoardMembers::relay_admit`]) and a board's gossip topic
/// ([`BoardMembers::gossip_accept`]) all read the same boards.
#[derive(Default)]
pub struct BoardMembers {
    held: crate::boardadmit::HeldBoards,
}

impl fmt::Debug for BoardMembers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoardMembers")
            .field("boards", &self.ids())
            .finish()
    }
}

impl BoardMembers {
    pub fn new() -> Arc<BoardMembers> {
        Arc::new(BoardMembers::default())
    }

    /// Counts board `board_id`'s members from now on.
    pub fn add(&self, board_id: &str, verifier: Arc<BoardVerifier>) {
        self.held.hold(board_id, verifier);
    }

    /// Stops counting board `board_id` (left, removed, forgotten).
    pub fn remove(&self, board_id: &str) {
        self.held.release(board_id);
    }

    /// The boards counted.
    pub fn ids(&self) -> Vec<String> {
        self.held.ids()
    }

    /// The registry itself.
    pub fn held(&self) -> &crate::boardadmit::HeldBoards {
        &self.held
    }

    /// B1: the home relay's widening (`home::Config.board_admit`): a
    /// counted, uncut member device of any board held here.
    pub fn relay_admit(&self) -> Arc<dyn Fn(&EndpointId) -> bool + Send + Sync> {
        self.held.relay_admit()
    }

    /// A board's gossip topic's neighbour check: a member device of that
    /// board.
    pub fn gossip_accept(&self, board_id: &str) -> Arc<dyn Fn(&EndpointId) -> bool + Send + Sync> {
        self.held.gossip_accept(board_id)
    }
}

impl BoardAdmit for BoardMembers {
    fn board_device(&self, board_id: &str, id: &EndpointId) -> Option<String> {
        self.held.board_device(board_id, id)
    }

    fn boards_of(&self, id: &EndpointId) -> Vec<String> {
        self.held.boards_of(id)
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
