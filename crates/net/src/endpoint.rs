//! The iroh Endpoint keyed by the device seed (D2).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::{Result, anyhow, bail};
use iroh::endpoint::{
    AfterHandshakeOutcome, Connection, EndpointHooks, NetReportConfig, PortmapperConfig, VarInt,
};
use iroh::{Endpoint, EndpointId, RelayMap, RelayMode, RelayUrl};

use crate::identity::{DeviceIdentity, IrohKey as _};
use crate::relay::AllowList;

/// Which IP sockets the endpoint binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpMode {
    /// No IP transports: every packet goes through the relay.
    RelayOnly,
    /// One UDP socket on 127.0.0.1 (tests and the default CLI).
    Loopback,
    /// One UDP socket on 0.0.0.0 (the CLI's `--public`, real LAN direct paths).
    AnyV4,
}

/// One relay this endpoint may use, pinned to its server's device key.
#[derive(Debug, Clone)]
pub struct RelaySpec {
    pub url: RelayUrl,
    /// QAD UDP port; Irori serves it on the same number as the HTTPS port.
    pub quic_port: u16,
    pub server_pub: EndpointId,
}

#[derive(Debug, Clone)]
pub struct EndpointOpts {
    pub ip: IpMode,
    /// Empty means `RelayMode::Custom(RelayMap::empty())`, so relays can be
    /// added after bind with `Endpoint::insert_relay`.
    pub relays: Vec<RelaySpec>,
    /// Admission in `after_handshake`; `None` admits anyone (dial-only test peers).
    pub allow: Option<AllowList>,
    pub alpns: Vec<Vec<u8>>,
    /// Extra relay server keys to pin, for relays added later with `insert_relay`.
    pub extra_pins: Vec<EndpointId>,
}

impl EndpointOpts {
    pub fn new(ip: IpMode) -> Self {
        Self {
            ip,
            relays: vec![],
            allow: None,
            alpns: vec![crate::SPIKE_ALPN.to_vec()],
            extra_pins: vec![],
        }
    }
}

/// Refuses every peer not in the allow list right after the TLS handshake,
/// before any stream is read (mesh.md §5.2, `CheckDevice`).
#[derive(Debug)]
struct AllowHook(AllowList);

impl EndpointHooks for AllowHook {
    async fn after_handshake(&self, conn: &Connection) -> AfterHandshakeOutcome {
        if self.0.contains(&conn.remote_id()) {
            AfterHandshakeOutcome::Accept
        } else {
            AfterHandshakeOutcome::Reject {
                error_code: VarInt::from_u32(1),
                reason: b"refused".to_vec(),
            }
        }
    }
}

/// Build and bind the endpoint. Never `presets::N0`, never `RelayMode::Default`.
pub async fn bind(id: &DeviceIdentity, opts: EndpointOpts) -> Result<Endpoint> {
    let mut nr = NetReportConfig::default();
    nr.captive_portal_check = false;
    let mut b = iroh::endpoint::Builder::empty()
        .crypto_provider(crate::tls::provider())
        .secret_key(id.secret_key())
        .alpns(opts.alpns.clone())
        .portmapper_config(PortmapperConfig::Disabled)
        .net_report_config(nr)
        .dns_resolver(crate::dns::LocalDns::resolver(crate::dns::DnsLog::default()))
        .clear_ip_transports();
    match opts.ip {
        IpMode::RelayOnly => {}
        IpMode::Loopback => {
            b = b
                .bind_addr(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
                .map_err(|e| anyhow!("bind addr: {e}"))?;
        }
        IpMode::AnyV4 => {
            b = b
                .bind_addr(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0))
                .map_err(|e| anyhow!("bind addr: {e}"))?;
        }
    }
    let mut pins: Vec<EndpointId> = opts.relays.iter().map(|r| r.server_pub).collect();
    pins.extend(opts.extra_pins.iter().copied());
    // Always pin: even with no relay, nothing may fall back to the webpki roots.
    b = b.ca_tls_config(crate::tls::pinned_ca_config(&pins));
    if opts.relays.is_empty() {
        if opts.ip == IpMode::RelayOnly {
            bail!("relay-only endpoint needs a relay");
        }
        // Not RelayMode::Disabled: Disabled drops the relay transport, so a
        // later insert_relay has nothing to run on. An empty Custom map binds
        // and accepts insert_relay (tests/loopback.rs, mesh.md §8).
        b = b.relay_mode(RelayMode::Custom(RelayMap::empty()));
    } else {
        let map = RelayMap::from_iter(
            opts.relays
                .iter()
                .map(|r| crate::relay::client_relay_config(r.url.clone(), r.quic_port)),
        );
        b = b.relay_mode(RelayMode::Custom(map));
    }
    if let Some(allow) = opts.allow {
        b = b.hooks(AllowHook(allow));
    }
    b.bind().await.map_err(|e| anyhow!("bind endpoint: {e:#}"))
}

/// Serve the spike echo: every bi stream's bytes come back, then `finish()`.
pub async fn serve_echo(ep: Endpoint, on_conn: impl Fn(&Connection) + Send + Sync + 'static) {
    let on_conn = std::sync::Arc::new(on_conn);
    while let Some(incoming) = ep.accept().await {
        let on_conn = on_conn.clone();
        tokio::spawn(async move {
            let Ok(accepting) = incoming.accept() else {
                return;
            };
            let Ok(conn) = accepting.await else {
                return;
            };
            on_conn(&conn);
            while let Ok((mut send, mut recv)) = conn.accept_bi().await {
                tokio::spawn(async move {
                    if let Ok(buf) = recv.read_to_end(1 << 20).await {
                        let _ = send.write_all(&buf).await;
                        let _ = send.finish();
                        let _ = send.stopped().await;
                    }
                });
            }
        });
    }
}

/// One echo round trip on a fresh bi stream.
pub async fn echo_once(conn: &Connection, msg: &[u8]) -> Result<Vec<u8>> {
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(msg).await?;
    send.finish()?;
    let back = recv.read_to_end(1 << 20).await?;
    Ok(back)
}

/// Parse an EndpointId (hex devicePub, or iroh's base32).
pub fn parse_id(s: &str) -> Result<EndpointId> {
    s.trim()
        .parse()
        .map_err(|e| anyhow!("endpoint id {s:?}: {e}"))
}
