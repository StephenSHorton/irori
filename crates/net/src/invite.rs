//! The invite room on `irori/invite/1`: the hooks the invites part (card
//! #44) builds on (docs/design/boards2-transport.md §5).
//!
//! INTERFACE ONLY (card #43). The inviter opens an invite guest window
//! ([`crate::guest::GuestWindows::open_invite_window`]) and sets an
//! [`InviteHandler`]; the invitee dials [`IrohNet::dial_invite`] with the
//! ticket's [`InviteAddr`], presenting the window's
//! [`crate::guest::invite_token`] to the inviter's relay. Both sides get a
//! plain [`Conn`]: `send`/`recv` carry the invite protocol (person proofs,
//! bundle), and `fingerprints()` is the session EKM the five words bind to
//! (as pairing, architecture §3.4). The ticket's encoding is the invites
//! part's.

use std::net::SocketAddr;
use std::sync::Arc;

use iroh::{EndpointId, RelayUrl};
use irori_meshseam::{Conn, Secret, TransportError, UNSUPPORTED};

use crate::transport::IrohNet;

/// The invite-open proof's label (first message, as the board-open proof,
/// keyed by the invite room key from the code).
pub const INVITE_OPEN_LABEL: &[u8] = b"irori/invite-open/1";

/// Where the invitee dials: the inviter's endpoint, its home relay and that
/// relay's pin (the server's devicePub), and direct addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteAddr {
    pub id: EndpointId,
    pub relay: Option<RelayUrl>,
    /// QAD UDP port of `relay` (Irori serves it on the HTTPS port number).
    pub relay_quic_port: u16,
    pub relay_pub: Option<EndpointId>,
    pub direct: Vec<SocketAddr>,
}

/// Called on the inviter with each connection that proved the open window's
/// invite room key. The first one wins; the handler closes the window.
pub type InviteHandler = Arc<dyn Fn(Arc<dyn Conn>) + Send + Sync>;

impl IrohNet {
    /// Accept `irori/invite/1` connections for `room` (`None` stops). The
    /// endpoint admits the ALPN only while an invite window is open.
    pub fn set_invite_handler(
        &self,
        _room: Option<Secret>,
        _h: Option<InviteHandler>,
    ) -> Result<(), TransportError> {
        Err(TransportError(UNSUPPORTED.into()))
    }

    /// The invitee's side: dial the inviter as a relay guest (token from
    /// `room.key`), prove the room key, and return the connection.
    pub async fn dial_invite(
        &self,
        _to: &InviteAddr,
        _room: &Secret,
    ) -> Result<Arc<dyn Conn>, TransportError> {
        Err(TransportError(UNSUPPORTED.into()))
    }
}
