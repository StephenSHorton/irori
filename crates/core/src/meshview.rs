//! The running mesh as Devices and the attachments see it (Go
//! I:internal/core/home.go `RemoteServers`/`OtherServer`/`LANError`,
//! core.go `NetStatus`/`Mesh().Devices`, attachview.go `fetchRef`/
//! `fileOffline`, attachments.go `deviceNames`).
//!
//! - Device rows and their live state are the mesh Node's
//!   ([`Core::mesh_devices`], Go `Mesh().Devices`).
//! - A home server on another device is a relay of this device's iroh
//!   endpoint, not a go-room hub. iroh keeps one home relay of those it is
//!   given and reaches the others when a peer needs them, so a server is
//!   "up" when the endpoint's home relay is it and connected, or when the
//!   home relay is another server that is connected. Its clock starts when
//!   the address book first feeds it (Go `HubState.Since`).
//! - The gap from Go: iroh reports connection state for its home relay
//!   only, so a server that is not the home relay counts as up whenever
//!   the home relay is connected. Go dialed every server as a hub and could
//!   name an unreachable one that wasn't its first; here only the home
//!   relay's own failure produces Devices' `remote` line.

use std::sync::Arc;
use std::time::{Duration, Instant};

use blob::FileRef;
use irori_identity::Chain;

use crate::Core;

/// How long a home server may take to connect before Devices says it
/// can't be reached (Go `reachGrace`).
pub const REACH_GRACE: Duration = Duration::from_secs(30);

/// Bounds one fetch from another device for the window (Go
/// `core.FetchTimeout`).
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(120);

/// Go `noDevice` in `deviceNames`: no holder known.
pub const NO_CONNECTED_DEVICE: &str = "no device that is connected";

/// Another device's home server as this device reaches it (Go
/// `core.RemoteServer`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteServer {
    pub device_id: String,
    pub up: bool,
    /// Down past [`REACH_GRACE`] since this device started using it.
    pub unreachable: bool,
}

/// One server as the endpoint holds it (Go `roomnet.HubState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubState {
    pub id: String,
    pub up: bool,
    pub since: Instant,
}

/// Go `remoteServers`: a server still connecting within [`REACH_GRACE`]
/// isn't reported as unreachable.
pub fn remote_servers(hubs: &[HubState], now: Instant) -> Vec<RemoteServer> {
    hubs.iter()
        .map(|h| RemoteServer {
            device_id: h.id.clone(),
            up: h.up,
            unreachable: !h.up && now.saturating_duration_since(h.since) > REACH_GRACE,
        })
        .collect()
}

/// Whether a server whose relay URLs are `urls` is up, from the
/// endpoint's home relays (`(url, connected)`).
pub fn hub_up(urls: &[iroh::RelayUrl], home: &[(iroh::RelayUrl, bool)]) -> bool {
    let mine = home.iter().any(|(u, _)| urls.contains(u));
    if mine {
        home.iter().any(|(u, c)| *c && urls.contains(u))
    } else {
        home.iter().any(|(_, c)| *c)
    }
}

/// Why a file could not be brought to this device (Go `FileOfflineError`,
/// a deadline, anything else).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileFetchError {
    /// No device that holds it is connected: their names, joined with
    /// " or " (Go `FileOfflineError.On`).
    Offline(String),
    TimedOut,
    Other(String),
}

impl Core {
    /// Go `NetStatus`: the mesh's state and every other device.
    pub fn net_status(&self) -> mesh::Status {
        self.mesh().status()
    }

    /// Go `Mesh().Devices(detail)`: the person's devices, this one first,
    /// with their live state and warnings.
    pub fn mesh_devices(&self, detail: bool) -> Vec<mesh::DeviceRow> {
        self.mesh().devices(detail)
    }

    /// Go `Role`: this device's cert role ("desktop" or "node"; "" before
    /// it has a cert). Devices offers the home server first on a node.
    pub fn role(&self) -> String {
        self.chain()
            .cert(self.device_id())
            .map(|c| c.role)
            .unwrap_or_default()
    }

    /// Go `OtherServer`: another of the person's devices runs a home
    /// server (its node.addr row, or a pairing hint).
    pub fn other_server(&self) -> bool {
        !self.mesh_parts().servers.list().is_empty()
    }

    /// Go `LANError`: why devices on this network can't find this one
    /// directly ("" while the LAN lookup works, or without one).
    pub fn lan_error(&self) -> String {
        self.net().lan_error()
    }

    /// The other devices' home servers the endpoint uses, by id.
    pub fn hubs(&self) -> Vec<HubState> {
        let home = self.net().home_relays();
        let hubs = self
            .mesh_parts()
            .hubs
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        hubs.iter()
            .map(|(id, h)| HubState {
                id: id.clone(),
                up: hub_up(&h.urls, &home),
                since: h.since,
            })
            .collect()
    }

    /// Go `RemoteServers`: the home servers on other devices this device
    /// uses, and whether each is up.
    pub fn remote_servers(&self) -> Vec<RemoteServer> {
        remote_servers(&self.hubs(), Instant::now())
    }

    /// Go `deviceNames`: each device's label, else its id, joined with
    /// " or "; [`NO_CONNECTED_DEVICE`] for none.
    pub fn device_names(&self, ids: &[String]) -> String {
        device_names(self.chain(), ids)
    }

    /// Go `fetchRef` within [`FETCH_TIMEOUT`]: makes sure `r`'s file is on
    /// this device, fetching it from a device that holds it first; returns
    /// the device it came from ("" when it was here). Blocks on the mesh's
    /// runtime: call it off that runtime's workers.
    pub fn fetch_file_blocking(&self, r: &FileRef) -> Result<String, FileFetchError> {
        fetch_file_blocking(
            self.mesh(),
            self.pairing_runtime(),
            self.device_id(),
            self.chain(),
            r,
        )
    }
}

/// [`Core::device_names`] over the Chain.
pub(crate) fn device_names(chain: &Chain, ids: &[String]) -> String {
    if ids.is_empty() {
        return NO_CONNECTED_DEVICE.into();
    }
    ids.iter()
        .map(|id| match chain.cert(id) {
            Some(c) if !c.label.is_empty() => c.label,
            _ => id.clone(),
        })
        .collect::<Vec<_>>()
        .join(" or ")
}

/// [`Core::fetch_file_blocking`] over the Core's parts (the tools'
/// attacher reaches them without a `Core`).
pub(crate) fn fetch_file_blocking(
    node: &Arc<mesh::Node>,
    rt: &tokio::runtime::Handle,
    device_id: &str,
    chain: &Chain,
    r: &FileRef,
) -> Result<String, FileFetchError> {
    let node = node.clone();
    let res = rt.block_on(async { tokio::time::timeout(FETCH_TIMEOUT, node.fetch_file(r)).await });
    match res {
        Err(_) => Err(FileFetchError::TimedOut),
        Ok(Ok(from)) => Ok(from),
        Ok(Err(mesh::FetchError::Offline(holders))) => {
            // Go fileOffline: the attaching device when no holder is known.
            let ids = if holders.is_empty() && !r.device.is_empty() && r.device != device_id {
                vec![r.device.clone()]
            } else {
                holders
            };
            Err(FileFetchError::Offline(device_names(chain, &ids)))
        }
        Ok(Err(e)) => Err(FileFetchError::Other(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(port: u16) -> iroh::RelayUrl {
        format!("https://127.0.0.1:{port}").parse().unwrap()
    }

    // Go TestRemoteServersGrace (core/home_test.go): a hub still
    // connecting within the grace isn't unreachable; past it, it is, and an
    // up one never is.
    #[test]
    fn remote_servers_grace() {
        let now = Instant::now() + 3 * REACH_GRACE;
        let hub = |id: &str, up: bool, ago: Duration| HubState {
            id: id.into(),
            up,
            since: now - ago,
        };
        let got = remote_servers(
            &[
                hub("new", false, REACH_GRACE / 2),
                hub("old", false, 2 * REACH_GRACE),
                hub("up", true, 2 * REACH_GRACE),
            ],
            now,
        );
        assert_eq!(got.len(), 3);
        assert!(
            !got[0].unreachable && got[1].unreachable && !got[2].unreachable,
            "{got:?}"
        );
    }

    #[test]
    fn a_server_is_up_by_the_endpoints_home_relay() {
        let a = vec![url(1), url(2)];
        // No home relay picked yet.
        assert!(!hub_up(&a, &[]));
        // Its own URL is the home relay: up only while connected.
        assert!(hub_up(&a, &[(url(2), true)]));
        assert!(!hub_up(&a, &[(url(2), false)]));
        // Another server is the home relay: up while that one is.
        assert!(hub_up(&a, &[(url(3), true)]));
        assert!(!hub_up(&a, &[(url(3), false)]));
    }
}
