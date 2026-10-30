//! `irori-net`: below the mesh seam (architecture §3, mesh.md §5).
//!
//! M1 spike contents:
//! - [`identity`]: `irori-identity` re-exported, plus the iroh key of a device (D2).
//! - [`tls`]: relay TLS on the device key, pinned to the certified devicePub (D3).
//! - [`relay`]: the embedded iroh-relay server with per-EndpointId access control.
//! - [`endpoint`]: an iroh Endpoint keyed by the device seed (D2), built from
//!   `Builder::empty()` only, never a preset.
//! - [`path`]: relay vs direct and RTT from a connection.
//! - [`fromdb`]: the Mac side's lookup of the PC's relay address and devicePub in `irori.db`.

//!
//! M5 adds the mesh transport:
//! - [`transport`]: `irori_meshseam::Transport` on iroh (epoch-open proof,
//!   who-dials, `replaced`, labelled streams, path and RTT).
//! - [`admit`]: the Chain's admission (`CheckDevice`) for the endpoint gate
//!   and the home relay's access control.
//! - [`gossip`]: per-board iroh-gossip topics, announcements only (D13).
//!
//! M7 adds pairing:
//! - [`pairnet`]: the epoch-0 pairing room on `irori/pair/1`, found on the
//!   LAN by the code-derived key, through a typed server, or across the
//!   home/away bridge.
//! - [`guest`]: relay guest windows (tokens from the room key).
//!
//! Boards stage 2 (card #43, interface first; docs/design/boards2-transport.md):
//! - [`board`]: `irori/board/1`, the board-open proof, member admission,
//!   `board.server` relays, `BoardTransport` on IrohNet.
//! - [`invite`]: `irori/invite/1` hooks for the invites part.
//! - [`boardadmit`]: the boards held here as admission sees them (endpoint
//!   gate, board gossip, the home relay's B1 widening).

pub mod admit;
pub mod board;
pub mod boardadmit;
pub mod book;
pub mod dns;
pub mod endpoint;
pub mod fromdb;
pub mod gossip;
pub mod guest;
pub mod identity;
pub mod invite;
pub mod pairnet;
pub mod path;
pub mod relay;
pub mod tls;
pub mod transport;

/// The spike's ALPN. The real build uses `irori/mesh/1` and `irori/pair/1`.
pub const SPIKE_ALPN: &[u8] = b"irori/spike/1";
