//! The address book the mesh feeds [`IrohNet`] (mesh.md §5.3): home relays
//! from the mesh's `Servers` fold of `node.addr` rows, and every other
//! certified, unrevoked device as a dial target by its EndpointId (its
//! devicePub, D2).
//!
//! - Each server's every dial address (`Servers::addrs`: heard LAN IP, the
//!   row's LAN/v6/public, the last that worked, and typed `hub-addrs.json`
//!   hints) becomes a relay URL on that host and port, pinned to the
//!   server's certified key. They are one relay process, so a device homed
//!   on the LAN URL is reachable through the public one. A host name (a
//!   typed `pc.example:25817`, Go dials hub addresses by name) stays a name
//!   in the URL: the endpoint resolves it through the OS each time it
//!   connects ([`crate::dns`]), so a dynamic-DNS name that moves still
//!   works, and the pin, not the name, is what's trusted.
//! - A peer's relay is the first server URL: iroh sends to a peer's home
//!   relay by URL, and any URL of the one server reaches it. Direct
//!   addresses come from the LAN lookup and iroh's own path discovery.

use std::net::SocketAddr;

use iroh::{EndpointId, RelayUrl};
use irori_identity::{Chain, Identity};

use crate::endpoint::RelaySpec;
use crate::transport::{IrohNet, PeerAddr};

/// One home server and where to dial it (Go `Server`: id, `Addrs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAddrs {
    pub device_id: String,
    pub addrs: Vec<String>,
}

/// The relays the servers name, each pinned to the server's certified,
/// unrevoked key. A server with no cert yet, or an address that isn't
/// `host:port`, is left out.
pub fn relays_for(servers: &[ServerAddrs], chain: &Chain, self_id: &str) -> Vec<RelaySpec> {
    let mut out: Vec<RelaySpec> = Vec::new();
    for s in servers {
        if s.device_id == self_id {
            continue;
        }
        let Some(key) = certified_key(chain, &s.device_id) else {
            continue;
        };
        for a in &s.addrs {
            let Some((url, port)) = relay_of(a) else {
                continue;
            };
            if out.iter().any(|r| r.url == url) {
                continue;
            }
            out.push(RelaySpec {
                url,
                quic_port: port,
                server_pub: key,
            });
        }
    }
    out
}

/// The relay URL and port of one dial address: `ip:port`, `[v6]:port` or
/// `name:port` (the relay serves HTTPS and QAD on the same port N).
pub fn relay_of(addr: &str) -> Option<(RelayUrl, u16)> {
    let addr = addr.trim();
    if let Ok(sa) = addr.parse::<SocketAddr>() {
        if sa.port() == 0 {
            return None;
        }
        return Some((crate::relay::relay_url(sa.ip(), sa.port()), sa.port()));
    }
    let (host, port) = addr.rsplit_once(':')?;
    let port: u16 = port.parse().ok().filter(|p| *p != 0)?;
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    // A name: letters, digits, '-' and '.', as a URL host and DNS allow.
    // Anything else (a bare v6 without brackets, a path, a user) is not a
    // server address.
    if host.is_empty()
        || host.starts_with('.')
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        return None;
    }
    let url: RelayUrl = format!("https://{host}:{port}/").parse().ok()?;
    Some((url, port))
}

/// This device's own relay on `port` (it listens on every interface, so
/// loopback reaches it), pinned to its own key (Go `Net.SetLocal`: the
/// server's own hub carries its rooms).
pub fn own_relay(port: u16, key: EndpointId) -> RelaySpec {
    RelaySpec {
        url: crate::relay::relay_url(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port),
        quic_port: port,
        server_pub: key,
    }
}

/// Every other certified, unrevoked device of the Chain, reached through
/// `relay` when there is one.
pub fn peers_for(chain: &Chain, self_id: &str, relay: Option<&RelayUrl>) -> Vec<PeerAddr> {
    chain
        .devices()
        .into_iter()
        .filter(|d| !d.revoked && d.cert.device_id != self_id)
        .filter_map(|d| {
            let id = endpoint_id(&d.cert.device_pub)?;
            Some(PeerAddr {
                device_id: d.cert.device_id,
                id,
                relay: relay.cloned(),
                direct: vec![],
            })
        })
        .collect()
}

/// Feeds `net` from the servers and the Chain: call it when either changes
/// (a `node.addr` row, a person row). `own` is this device's own relay while
/// its home server runs (Go `Net.SetLocal`); it goes first. Pins of servers
/// no longer listed are removed, so a revoked home server stops being a
/// trusted relay.
pub async fn feed(net: &IrohNet, servers: &[ServerAddrs], chain: &Chain, own: Option<RelaySpec>) {
    let self_id = net.self_id().to_string();
    let mut relays = relays_for(servers, chain, &self_id);
    if let Some(o) = own {
        relays.retain(|r| r.url != o.url);
        relays.insert(0, o);
    }
    let first = relays.first().map(|r| r.url.clone());
    net.set_peers(peers_for(chain, &self_id, first.as_ref()));
    net.set_servers(relays).await;
}

fn certified_key(chain: &Chain, device_id: &str) -> Option<EndpointId> {
    let d = chain
        .devices()
        .into_iter()
        .find(|d| d.cert.device_id == device_id)?;
    if d.revoked {
        return None;
    }
    endpoint_id(&d.cert.device_pub)
}

fn endpoint_id(pub_hex: &str) -> Option<EndpointId> {
    let b: [u8; 32] = hex::decode(pub_hex).ok()?.try_into().ok()?;
    EndpointId::from_bytes(&b).ok()
}

/// The LAN lookup's rooms (`irori_lan::Config::rooms`): the mesh secret of
/// the current epoch and of every older one this device still holds, so a
/// device that hasn't rotated yet still finds this one.
pub fn lan_rooms(ident: std::sync::Arc<Identity>) -> irori_lan::RoomsFn {
    std::sync::Arc::new(move || {
        let p = ident.current();
        std::iter::once(p.epoch)
            .chain(ident.older_epochs())
            .filter_map(|e| {
                let seed = ident.person_seed(e)?;
                let s = irori_meshseam::mesh_secret(&seed[..], &p.id, e);
                Some(irori_lan::Room {
                    topic: s.topic,
                    key: s.key,
                })
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_of_ips_and_names() {
        let (u, p) = relay_of("192.168.1.20:25817").unwrap();
        assert_eq!(
            (u.to_string().as_str(), p),
            ("https://192.168.1.20:25817/", 25817)
        );
        let (u, _) = relay_of("[2001:db8::1]:25817").unwrap();
        assert_eq!(u.to_string(), "https://[2001:db8::1]:25817/");
        // A typed name stays a name (the endpoint resolves it).
        let (u, p) = relay_of("PC.example.:24817").unwrap();
        assert_eq!(
            (u.to_string().as_str(), p),
            ("https://pc.example:24817/", 24817)
        );
        let (u, _) = relay_of("localhost:5").unwrap();
        assert_eq!(u.to_string(), "https://localhost:5/");
        for bad in [
            "",
            "pc.example",
            "pc.example:0",
            "pc.example:70000",
            ":25817",
            "2001:db8::1",
            "user@pc.example:1",
            "pc.example/x:1",
            "1.2.3.4:0",
        ] {
            assert!(relay_of(bad).is_none(), "{bad:?}");
        }
    }
}
