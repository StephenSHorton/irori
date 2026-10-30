//! Devices (M7; I:devices_service.go, internal/core/core.go:284-293,
//! reset.go): the pairing flow the Core owns, and the Devices window's
//! calls with Go's texts.
//!
//! Wiring notes:
//! - The Pairer and the mesh Node share one [`IrohNet`] keyed by the device
//!   key ([`Options::pair_net`](crate::Options::pair_net), or
//!   [`default_net`]): one endpoint serves `irori/mesh/1` and
//!   `irori/pair/1`. The Pairer's `kick` is the Node's and its `servers` the
//!   mesh's `Servers` (crate::meshnode). The default transport carries the
//!   sealed LAN lookup on UDP 38419 ([`default_net_config`]), started only
//!   when the endpoint binds (the mesh's first join or a pairing room, Go
//!   roomnet `start`), with the open epochs' rooms, pairing wired through
//!   `PairLan::wire` and its `on_heard` feeding `Servers::heard`.
//! - The home server shares the pairing's guest windows (its relay admits
//!   their tokens) and runs while one is open (Go `Wanted`).
//! - The window seam (`crates/window` `seams::Pairing`) maps onto
//!   [`add_device`], [`join_with_code`], [`confirm_words`], [`confirm_key`],
//!   [`Core::pairing`]'s `status()` and `cancel()`; `Mesh::remove_device`
//!   onto [`Core::remove_device`]; Reset is [`Core::request_reset`] and a
//!   quit, as Go's `ResetDevice`.
//! - The loopback API's join-side routes (Go routes_pair.go) are
//!   `irori_api::pair` over [`Core::pairing`] and [`Core::pairing_runtime`];
//!   `irori pair join` (app `pair_cmd`) drives them, or the Pairer of a Core
//!   it starts itself.

use std::sync::Arc;
use std::time::Duration;

use irori_identity::{Chain, Identity};
use irori_net::guest::GuestWindows;
use irori_net::transport::IrohNet;
use irori_pair::{Error, Pairer, Status};

use crate::Core;

/// Builds the iroh transport the mesh and pairing run on, from this
/// device's identity and Chain and the guest windows the home server shares
/// (tests: loopback).
pub type PairNetFn =
    Arc<dyn Fn(&Arc<Identity>, &Arc<Chain>, &GuestWindows) -> IrohNet + Send + Sync>;

/// Removes a device (Go `Mesh().Revoke(ctx, id, reason)`): the mesh Node's
/// revoke, which writes the revoke and rotates the person key.
pub type RevokeFn = Arc<
    dyn Fn(
            String,
            String,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>
        + Send
        + Sync,
>;

/// The reason Remove device records (Go devices_service.go).
pub const REMOVED_REASON: &str = "removed in Settings";

/// The default transport: iroh on the device key, every IPv4 interface,
/// the Chain's gate; nothing binds until the mesh joins (a second device)
/// or a pairing window opens.
pub(crate) fn default_net(
    ident: &Arc<Identity>,
    chain: &Arc<Chain>,
    guests: &GuestWindows,
) -> IrohNet {
    IrohNet::new(default_net_config(ident, chain, guests))
}

/// The default transport's settings, for [`Options::pair_net`]
/// (crate::Options::pair_net) in tests that keep everything but the
/// sockets (loopback IP, a loopback LAN lookup): no public relay, STUN,
/// DNS or n0 lookup (D3/D13), and the LAN lookup on UDP 38419 (dev builds:
/// plus `IRORI_DEV_PORT_OFFSET`) to start at the first bind.
pub fn default_net_config(
    ident: &Arc<Identity>,
    chain: &Arc<Chain>,
    guests: &GuestWindows,
) -> irori_net::transport::NetConfig {
    let key = iroh::SecretKey::from_bytes(&ident.expose_device_seed());
    let mut lan = irori_lan::Config::new(key.public(), irori_net::book::lan_rooms(ident.clone()));
    lan.logf = Some(Arc::new(|l: &str| crate::log(l)));
    let admit = Arc::new(irori_net::admit::ChainAdmit {
        chain: chain.clone(),
        self_id: ident.device_id(),
    });
    let mut cfg = irori_net::transport::NetConfig::new(ident.device_id(), key, admit);
    cfg.ip = irori_net::endpoint::IpMode::AnyV4;
    cfg.guests = guests.clone();
    cfg.pair_lan = irori_net::pairnet::PairLan::new(guests.clone());
    // Dev builds move the port with IRORI_DEV_PORT_OFFSET (architecture
    // §3.2); a bad value turns the lookup off rather than leave it on Go's.
    match irori_lan::dev_lan_port() {
        Ok(port) => {
            lan.port = port;
            cfg.lan_config = Some(lan);
        }
        Err(e) => crate::log(&format!("lan: off: {e}")),
    }
    cfg
}

/// Go `AddDevice`: opens a pairing window; `role` is what the new device's
/// cert says (`desktop` or `node`).
pub async fn add_device(core: &Core, role: &str) -> Result<Status, String> {
    core.pairing().add(role).await.map_err(|e| e.to_string())
}

/// Go `JoinWithCode`: enters a code from another device; `server` is the
/// home server's address, typed only when joining from away from home.
pub async fn join_with_code(core: &Core, code: &str, server: &str) -> Result<Status, String> {
    let server = server.trim();
    let server = if server.is_empty() {
        String::new()
    } else {
        home::hostport::host_port(server, 0).map_err(|_| {
            "the home server address is a host and port, like 203.0.113.7:24817".to_string()
        })?
    };
    core.pairing()
        .join_via(code, &server)
        .await
        .map_err(|e| e.to_string())
}

/// Go `ConfirmWords`: the words the new device shows, typed on this one.
pub async fn confirm_words(core: &Core, words: &str) -> Result<(), String> {
    match core.pairing().confirm(words).await {
        Err(Error::WrongWords) => Err("those aren't the words the other device shows".into()),
        r => r.map_err(|e| e.to_string()),
    }
}

/// Go `ConfirmKey`: on the joining device, the key the other device shows
/// beside its code.
pub async fn confirm_key(core: &Core, key: &str) -> Result<(), String> {
    match core.pairing().confirm_key(key).await {
        Err(Error::WrongKey) => Err("that isn't the key the other device shows".into()),
        r => r.map_err(|e| e.to_string()),
    }
}

/// How long a test's pairing window stays open (`Options::pair_window`).
pub(crate) fn window_or_default(d: Duration) -> Duration {
    if d.is_zero() { irori_pair::WINDOW } else { d }
}

/// The Core's Pairer and what it needs alive.
pub(crate) struct Pairing {
    pub pairer: Pairer,
    pub guests: GuestWindows,
    pub handle: tokio::runtime::Handle,
    /// The runtime pairing and its transport run on when the Core was given
    /// none; shut down at close.
    pub own_rt: std::sync::Mutex<Option<tokio::runtime::Runtime>>,
}

impl Pairing {
    pub(crate) fn shutdown(&self) {
        self.pairer.cancel();
        let rt = self.own_rt.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(rt) = rt {
            rt.shutdown_background();
        }
    }
}
