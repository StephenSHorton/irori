//! Embedded home-server relay: iroh-relay's server module run in-process.
//!
//! TCP N serves the relay over HTTPS (`TlsConfig.https_bind_addr`), UDP N serves
//! QUIC address discovery, and the mandatory plain-HTTP captive-portal listener
//! goes to an ephemeral loopback port nobody forwards (mesh.md §5.4).

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result, anyhow};
use iroh::{EndpointId, RelayUrl};
use iroh_relay::server::{
    Access, AccessControl, CertConfig, ClientRequest, QuicConfig, RelayConfig, Server,
    ServerConfig, TlsConfig,
};

use crate::identity::DeviceIdentity;

/// Per-EndpointId admission, shared by the relay (`AccessControl`) and the
/// endpoint (`after_handshake`). In the real build this is `CheckDevice`
/// over the Chain; here it is a plain set.
#[derive(Debug, Clone, Default)]
pub struct AllowList(Arc<RwLock<HashSet<EndpointId>>>);

impl AllowList {
    pub fn new(ids: impl IntoIterator<Item = EndpointId>) -> Self {
        Self(Arc::new(RwLock::new(ids.into_iter().collect())))
    }
    pub fn allow(&self, id: EndpointId) {
        self.0.write().expect("lock").insert(id);
    }
    pub fn contains(&self, id: &EndpointId) -> bool {
        self.0.read().expect("lock").contains(id)
    }
}

impl AccessControl for AllowList {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        let id = request.endpoint_id();
        if self.contains(&id) {
            Access::Allow
        } else {
            Access::Deny {
                reason: Some("not a certified device".into()),
            }
        }
    }
}

/// A running embedded relay.
pub struct EmbeddedRelay {
    pub server: Server,
    /// TCP (HTTPS) and UDP (QAD) port N.
    pub port: u16,
    pub bind_ip: IpAddr,
}

impl EmbeddedRelay {
    /// The relay URL for `host` (a LAN/public IP or 127.0.0.1).
    pub fn url_for(&self, host: IpAddr) -> RelayUrl {
        relay_url(host, self.port)
    }
}

pub fn relay_url(host: IpAddr, port: u16) -> RelayUrl {
    let h = match host {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    };
    format!("https://{h}:{port}/").parse().expect("relay url")
}

/// The client-side relay config for a relay on `port` (QAD on the same port).
pub fn client_relay_config(url: RelayUrl, port: u16) -> iroh::RelayConfig {
    iroh::RelayConfig::new(url, Some(iroh_relay::RelayQuicConfig::new(port)))
}

/// Spawn the relay on `bind_ip:port` (TCP and UDP). `port == 0` picks a free
/// port that is open on both TCP and UDP. Callers decide `bind_ip`; tests and the
/// CLI default use 127.0.0.1.
pub async fn spawn(
    id: &DeviceIdentity,
    bind_ip: IpAddr,
    port: u16,
    allow: AllowList,
) -> Result<EmbeddedRelay> {
    spawn_with(id, bind_ip, port, allow).await
}

/// [`spawn`] with any access rule (the Chain's: `admit::ChainAccess`).
pub async fn spawn_with<A: AccessControl + Clone>(
    id: &DeviceIdentity,
    bind_ip: IpAddr,
    port: u16,
    allow: A,
) -> Result<EmbeddedRelay> {
    let mut last_err = None;
    for _ in 0..8 {
        let p = if port == 0 { free_port(bind_ip)? } else { port };
        match try_spawn(id, bind_ip, p, allow.clone()).await {
            Ok(server) => {
                return Ok(EmbeddedRelay {
                    server,
                    port: p,
                    bind_ip,
                });
            }
            Err(e) if port == 0 => last_err = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("relay: no port")))
}

async fn try_spawn<A: AccessControl>(
    id: &DeviceIdentity,
    bind_ip: IpAddr,
    port: u16,
    allow: A,
) -> Result<Server> {
    let tls = crate::tls::relay_server_config(id, vec!["irori-relay".into()])?;
    // The plain-HTTP listener (captive portal only) never leaves loopback.
    let mut relay = RelayConfig::new(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0));
    relay.tls = Some(TlsConfig::new(
        SocketAddr::new(bind_ip, port),
        CertConfig::Manual { server_config: tls },
    ));
    relay.access = Arc::new(allow);
    let mut cfg = ServerConfig::default();
    cfg.relay = Some(relay);
    cfg.quic = Some(QuicConfig::new(SocketAddr::new(bind_ip, port)));
    Server::spawn(cfg)
        .await
        .map_err(|e| anyhow!("{e:#}"))
        .context("spawn iroh-relay")
}

fn free_port(ip: IpAddr) -> Result<u16> {
    let l = std::net::TcpListener::bind(SocketAddr::new(ip, 0))?;
    Ok(l.local_addr()?.port())
}
