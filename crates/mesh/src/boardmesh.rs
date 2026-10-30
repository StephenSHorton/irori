//! Boards stage 2 above the seam: one board mesh per board topic, the board
//! sync frames, `board.secret` and `board.server` (docs/design/
//! boards2-transport.md, v0.5-shared-boards §6.1-6.4).
//!
//! INTERFACE ONLY (card #43). Types are final; functions that do work
//! return `TransportError(UNSUPPORTED)` (or `Err`) until the transport
//! branch lands. Stage 1's `board/<id>` streams on the private mesh
//! connection ([`crate::boards`]) keep running until then, and are removed
//! by the same branch.

use std::collections::BTreeMap;
use std::sync::Arc;

use identity::{BoardSecrets, BoardVerifier};
use irori_meshseam::{BoardTransport, TransportError, UNSUPPORTED};
use serde::{Deserialize, Serialize};
use store::Store;

use crate::servers::PublicAddr;

/// The board sync protocol version a board hello carries (§6.3 "Version").
pub const BOARD_PROTOCOL: i64 = 1;

/// The frame that hands a board secret to a device: on the private mesh
/// `sync` channel to your own devices (sealed under the mesh epoch's
/// person-secret key, as `person.secret`), and after a rotation on each
/// board connection to current member devices (sealed under that
/// connection's board epoch key).
pub const FRAME_BOARD_SECRET: &str = "board.secret";

/// The kind of a board server row.
pub const KIND_BOARD_SERVER: &str = "board.server";

/// The board connection's hello (§6.4): first frame on `sync` after the
/// board-open message. A `v` or `coreVersion` the receiver refuses closes
/// the connection with reason `"version"`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardHello {
    pub v: i64,
    pub device_id: String,
    pub person_id: String,
    pub board_id: String,
    pub board_epoch: i64,
    pub core_version: String,
    pub schema: i64,
    pub vector: BTreeMap<String, i64>,
    pub wall_ms: i64,
}

/// `board.secret {boardId, boardEpoch, maker, sealed}`: `sealed` is
/// base64 `nonce(24) || secretbox(secret)`; `maker` is the device that
/// minted it (the lower-maker rule, §6.1).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardSecretFrame {
    pub board_id: String,
    pub board_epoch: i64,
    pub maker: String,
    pub sealed: String,
}

/// Seals board `board_id`'s secret for `epoch` to `recipient` under
/// `seal_key` (the mesh key on the private channel, the board topic key on
/// a board connection).
pub fn seal_board_secret(
    _seal_key: &[u8; 32],
    _board_id: &str,
    _epoch: i64,
    _secret: &[u8],
    _recipient: &str,
) -> Result<Vec<u8>, String> {
    Err(UNSUPPORTED.into())
}

/// Opens a [`seal_board_secret`] result on the recipient `self_id`.
pub fn open_board_secret(
    _seal_key: &[u8; 32],
    _board_id: &str,
    _epoch: i64,
    _sealed: &[u8],
    _self_id: &str,
) -> Result<Vec<u8>, String> {
    Err(UNSUPPORTED.into())
}

/// A `board.server` payload (B1, §6.2): written by a server device into
/// each board its person is in, only when it changes. `public: None` (JSON
/// `null`) when the server is off or outside reach is off. Never LAN or v6
/// addresses.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardServer {
    pub device_id: String,
    pub port: i64,
    pub public: Option<PublicAddr>,
}

/// The board's `board.server` fold: the newest row per server device whose
/// writer is that device and a counted member device.
pub fn board_servers(
    _log: &Store,
    _verifier: &BoardVerifier,
) -> Result<Vec<BoardServer>, TransportError> {
    Err(TransportError(UNSUPPORTED.into()))
}

/// What a [`BoardMesh`] runs on.
#[derive(Clone)]
pub struct BoardMeshOpts {
    pub board_id: String,
    pub self_id: String,
    pub person_id: String,
    pub core_version: String,
    pub log: Arc<Store>,
    pub verifier: Arc<BoardVerifier>,
    pub secrets: Arc<BoardSecrets>,
    pub transport: Arc<dyn BoardTransport>,
}

/// One board's mesh: a board room per held epoch (current and previous
/// while members move), `board/<id>` pulls on each connection, the board
/// hello, `board.secret` after rotations, and the board's gossip topic for
/// announcements. Admission is the board's verifier only.
pub struct BoardMesh {
    opts: BoardMeshOpts,
}

impl BoardMesh {
    pub fn new(opts: BoardMeshOpts) -> BoardMesh {
        BoardMesh { opts }
    }

    pub fn board_id(&self) -> &str {
        &self.opts.board_id
    }

    /// Joins the board's rooms for every epoch `secrets` holds.
    pub async fn start(&self) -> Result<(), TransportError> {
        Err(TransportError(UNSUPPORTED.into()))
    }

    /// Pulls from every connected member device now (a gossip announcement
    /// or a local write).
    pub fn kick(&self) {}

    /// Re-checks live connections against the verifier (a revoke, removal
    /// or leave folded) and closes those no longer admitted.
    pub fn recheck(&self) {}

    /// Leaves every room of the board.
    pub fn close(&self) {}
}
