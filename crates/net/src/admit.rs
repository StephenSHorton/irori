//! Who may connect at all: the endpoint's `after_handshake` gate and the
//! relay's `AccessControl`, both from the Chain (mesh.md §5.2, §5.4).
//!
//! `check_device` is Go's `mesh.CheckDevice` (I:internal/mesh/node.go:310-322),
//! the one admission rule: a device other than self with a valid, unrevoked
//! cert for this person whose cert names the key. The rule itself is
//! `Chain::check_device` (identity), which the mesh crate's
//! `RoomConfig::accept` uses too.

use std::fmt;
use std::sync::Arc;

use iroh::EndpointId;
use irori_identity::Chain;
use irori_meshseam::Refused;

/// The endpoint-level gate, asked right after the TLS handshake proved the
/// peer's key and before any stream is read.
pub trait Admit: Send + Sync + fmt::Debug {
    /// Whether a peer holding `id` may open a connection on `alpn`.
    fn admits(&self, alpn: &[u8], id: &EndpointId) -> bool;
}

/// Go `CheckDevice` ([`Chain::check_device`]) with the seam's refusal.
pub fn check_device(
    chain: &Chain,
    self_id: &str,
    id: &str,
    pubkey: &[u8; 32],
) -> Result<(), Refused> {
    if chain.check_device(self_id, id, pubkey) {
        Ok(())
    } else {
        Err(Refused)
    }
}

/// The device id whose valid cert names `pubkey`, if one is admitted by
/// [`check_device`].
pub fn admitted_device(chain: &Chain, self_id: &str, pubkey: &[u8; 32]) -> Option<String> {
    let want = hex::encode(pubkey);
    chain
        .devices()
        .into_iter()
        .find(|d| d.cert.device_pub == want && !d.revoked)
        .map(|d| d.cert.device_id)
        .filter(|id| check_device(chain, self_id, id, pubkey).is_ok())
}

/// [`Admit`] over the Chain: certified, unrevoked devices of this person,
/// on every ALPN. Board gossip in stage 1 has the same members (all of them
/// are this person's devices); stage 2 widens it per board (§3.3).
#[derive(Clone)]
pub struct ChainAdmit {
    pub chain: Arc<Chain>,
    pub self_id: String,
}

impl fmt::Debug for ChainAdmit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChainAdmit")
            .field("self_id", &self.self_id)
            .finish_non_exhaustive()
    }
}

impl Admit for ChainAdmit {
    fn admits(&self, _alpn: &[u8], id: &EndpointId) -> bool {
        admitted_device(&self.chain, &self.self_id, id.as_bytes()).is_some()
    }
}

/// [`Admit`] that lets everyone through (tests of the layers above).
#[derive(Debug, Clone, Default)]
pub struct AdmitAll;

impl Admit for AdmitAll {
    fn admits(&self, _alpn: &[u8], _id: &EndpointId) -> bool {
        true
    }
}

impl Admit for crate::relay::AllowList {
    fn admits(&self, _alpn: &[u8], id: &EndpointId) -> bool {
        self.contains(id)
    }
}

/// The home relay's `AccessControl` from the Chain: the server itself and
/// its person's certified, unrevoked devices. Guests (pairing windows) are
/// M7 (architecture §3.3, critic finding 3). Dropping a client that is
/// already connected when it is revoked is the home crate's (`dropRevoked`).
#[derive(Clone)]
pub struct ChainAccess {
    pub admit: ChainAdmit,
    pub self_pub: EndpointId,
}

impl fmt::Debug for ChainAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChainAccess")
            .field("admit", &self.admit)
            .finish_non_exhaustive()
    }
}

impl iroh_relay::server::AccessControl for ChainAccess {
    async fn on_connect(
        &self,
        request: &iroh_relay::server::ClientRequest,
    ) -> iroh_relay::server::Access {
        let id = request.endpoint_id();
        if id == self.self_pub || self.admit.admits(b"", &id) {
            iroh_relay::server::Access::Allow
        } else {
            iroh_relay::server::Access::Deny {
                reason: Some("not a certified device".into()),
            }
        }
    }
}
