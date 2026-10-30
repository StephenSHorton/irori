//! `irori-mesh`: the sync protocol above the transport seam (mesh.md §0-§3,
//! §6; I:internal/mesh except roomnet).
//!
//! | Module | Go | What |
//! |---|---|---|
//! | [`transport`] | `mesh.go` | the seam, `irori-meshseam`'s `Transport` / `Room` / `Conn` (which `irori-net` implements on iroh), and the Node's [`PeerConn`] view of a connection |
//! | [`fake`] | `fake.go` | FakeNet, the in-memory seam transport the suite runs on |
//! | [`secret`] | `mesh.go`, `secret.go` | `MeshSecret` and `person.secret` sealing (frozen, goldens) |
//! | [`frames`] | `frames.go` | sync frames and rows on the wire (frozen, goldens) |
//! | [`node`] | `node.go` | [`Node`]: epoch rooms, revoke, rotation and its tie-break, secret hand-off |
//! | `session` | `session.go` | sync on one connection: hello, batches, acks, held rows, restore and fork checks |
//! | [`files`] | `files.go` | attachment offers and fetches over `blob/` streams |
//! | [`boards`] | `boards.go` | board-log pulls over `board/` streams |
//! | [`boardmesh`] | (stage 2) | one board's mesh on `irori/board/1`: hello, pulls, `board.secret` |
//! | [`devices`] | `devices.go`, `node.go` | liveness, `Status`, device rows and their warnings |
//! | [`servers`] | `servers.go` | the `node.addr` fold and `hub-addrs.json` |
//! | [`peers`] | `peers.go` | `mesh-peers.json` |
//! | [`relay`] | `relay.go` | `relay.json` (no meaning under iroh, D4; kept readable) |
//!
//! The Node is async (tokio). Its fold state sits behind std mutexes that
//! are never held across an `.await`; heavy store work (`missing`,
//! `insert_batch`) runs on blocking threads.

extern crate irori_identity as identity;

pub mod boardmesh;
pub mod boards;
pub mod devices;
pub mod fake;
pub mod files;
pub mod frames;
mod fsutil;
pub mod node;
pub mod peers;
pub mod relay;
pub mod secret;
pub mod servers;
mod session;
pub mod transport;

pub use boards::BOARD_LABEL_PREFIX;
pub use devices::{DeviceRow, PeerState, Status};
pub use fake::FakeNet;
pub use files::{BLOB_LABEL_PREFIX, FetchError, FileOffer};
pub use frames::{Frame, Row, batches, decode_frame};
pub use node::{
    ERR_NOT_PEER, Intervals, MSG_LEAVE, MSG_PRESENCE, Node, Options, Presence, STATE_ERROR,
    STATE_OFF, STATE_ON, STATE_REMOVED, check_device,
};
pub use peers::PEERS_FILE;
pub use relay::{Relay, RelaySetter, RelayStatus, load_relay, save_relay, scrub_urls};
pub use secret::{MESH_APP, Secret, mesh_info, mesh_secret, open_secret, seal_secret};
pub use servers::{LAN_PORT, NodeAddr, PublicAddr, Server, Servers};
pub use transport::{
    BoxFut, ByteStream, Conn, ConnError, ERR_REFUSED, Envelope, PeerConn, Refused, Room,
    RoomConfig, Stream, Transport, TransportError,
};
