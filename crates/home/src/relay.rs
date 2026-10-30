//! The home server's embedded iroh relay (architecture §3.1, mesh.md §5.4,
//! settled by the M1 spike, mesh.md §8.1):
//!
//! - TCP N serves the relay over HTTPS (`TlsConfig.https_bind_addr`), UDP N
//!   serves QUIC address discovery, so "forward TCP and UDP port N" stays
//!   true. The mandatory plain-HTTP captive-portal listener goes to an
//!   ephemeral loopback port nobody forwards.
//! - TLS (D3): a self-signed certificate whose key *is* the device ed25519
//!   key, TLS 1.3 only. Clients pin it to the server's certified devicePub
//!   (`irori_net::tls::PinnedDeviceKey`), so `node.addr` needs no new field.
//! - Access: per EndpointId through [`Admit`]: this device, and the
//!   certified, unrevoked devices of the person (`CheckDevice` by key).
//!   [`Relay::drop_refused`] disconnects live clients that no longer pass,
//!   the iroh form of Go's `dropRevoked` (home.go:515-533).
//! - IPv6: iroh-relay binds one HTTPS address, and a `[::]` socket on
//!   Windows is v6-only, so while "Reachable from outside home" is on an
//!   extra v6-only listener on TCP N feeds the same relay service. QAD stays
//!   IPv4-only.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use iroh::EndpointId;
use iroh_relay::server::{
    Access, AccessControl, CertConfig, ClientRequest, QuicConfig, RelayConfig, Server,
    ServerConfig, TlsConfig,
};

/// Whether an endpoint may use this relay.
pub type AdmitFn = Arc<dyn Fn(&EndpointId) -> bool + Send + Sync>;
/// Whether a relay auth token opens a guest window here (M7): a pairing
/// joiner with no cert, admitted while a member's window is open.
pub type GuestFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// B1 (stage 2, card #43): whether an endpoint is a counted, uncut member
/// device of a board this server's Core holds. The relay then admits it as
/// it admits the person's own devices; which board it may reach is decided
/// at the endpoints (`irori/board/1` proof and per-board verifier), since
/// the relay only forwards sealed QUIC. Interface only: not wired yet.
pub type BoardAdmitFn = Arc<dyn Fn(&EndpointId) -> bool + Send + Sync>;

/// Stage 2: own devices or member devices of boards held here.
/// Interface only (card #43); the Server does not call it yet.
pub fn member_admit(own: AdmitFn, boards: BoardAdmitFn) -> AdmitFn {
    Arc::new(move |id: &EndpointId| own(id) || boards(id))
}

/// The reason a refused client sees (`RelayStatus::auth_denied_reason`).
pub const DENY_REASON: &str = "not a certified device";

/// The rustls server config on the device key: a self-signed certificate
/// whose key is the device ed25519 key, TLS 1.3 only (the relay's HTTPS and
/// QAD servers share it). Built by `irori_net::tls`, the one copy of the
/// relay TLS.
pub fn server_tls(seed: &[u8; 32]) -> Result<rustls::ServerConfig, String> {
    irori_net::tls::server_config_on_seed(
        seed,
        vec!["irori-relay".to_string()],
        "irori relay".to_string(),
    )
    .map_err(|e| e.to_string())
}

/// Admission plus the live connections it admitted.
#[derive(Clone)]
struct Gate {
    admit: AdmitFn,
    guest: Option<GuestFn>,
    live: Arc<Mutex<HashMap<EndpointId, usize>>>,
    /// Guests admitted by token, and the token (dropped when it closes).
    guests: Arc<Mutex<HashMap<EndpointId, String>>>,
}

impl std::fmt::Debug for Gate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Gate")
    }
}

impl Gate {
    fn live(&self) -> MutexGuard<'_, HashMap<EndpointId, usize>> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn guests(&self) -> MutexGuard<'_, HashMap<EndpointId, String>> {
        self.guests.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A guest by token: an open window's, and at most
    /// [`irori_net::guest::MAX_GUESTS`] at a time.
    fn guest_ok(&self, id: &EndpointId, token: Option<String>) -> bool {
        let (Some(f), Some(tok)) = (&self.guest, token) else {
            return false;
        };
        if !f(&tok) {
            return false;
        }
        let mut g = self.guests();
        // A guest whose window closed or expired holds no slot, even before
        // the sweep drops its connection.
        g.retain(|_, t| f(t));
        if !g.contains_key(id) && g.len() >= irori_net::guest::MAX_GUESTS {
            return false;
        }
        g.insert(*id, tok);
        true
    }

    /// Still admitted: a member, or a guest whose window is open.
    fn still(&self, id: &EndpointId) -> bool {
        if (self.admit)(id) {
            return true;
        }
        let tok = self.guests().get(id).cloned();
        matches!((&self.guest, tok), (Some(f), Some(t)) if f(&t))
    }
}

impl AccessControl for Gate {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        let id = request.endpoint_id();
        if (self.admit)(&id) || self.guest_ok(&id, request.auth_token()) {
            *self.live().entry(id).or_insert(0) += 1;
            Access::Allow
        } else {
            Access::Deny {
                reason: Some(DENY_REASON.into()),
            }
        }
    }

    fn on_disconnect(&self, id: EndpointId, _c: iroh_relay::server::ConnectionId) {
        let mut l = self.live();
        if let Some(n) = l.get_mut(&id) {
            *n -= 1;
            if *n == 0 {
                l.remove(&id);
                self.guests().remove(&id);
            }
        }
    }
}

/// A running embedded relay.
pub struct Relay {
    server: Mutex<Option<Server>>,
    gate: Gate,
    port: u16,
    /// Where TCP and UDP port N are bound.
    at: SocketAddr,
    v6: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// How long [`Relay::close`] waits for port N to come free.
const PORT_FREE_WAIT: Duration = Duration::from_secs(3);

/// Why a relay didn't start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnError {
    /// Port N couldn't be bound (taken): try another port.
    Bind(String),
    Other(String),
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpawnError::Bind(s) | SpawnError::Other(s) => f.write_str(s),
        }
    }
}

impl Relay {
    /// Starts the relay on TCP and UDP `bind_ip:port`. With `v6_too` (only
    /// for a wildcard `0.0.0.0` bind) TCP `[::]:port` is listened on too.
    pub async fn spawn(
        seed: &[u8; 32],
        bind_ip: IpAddr,
        port: u16,
        v6_too: bool,
        admit: AdmitFn,
        guest: Option<GuestFn>,
    ) -> Result<Relay, SpawnError> {
        // Bind-check both ports first: iroh-relay reports a taken UDP port
        // only through its supervisor, and a taken TCP port as a generic
        // spawn error.
        let at = SocketAddr::new(bind_ip, port);
        std::net::TcpListener::bind(at).map_err(|e| SpawnError::Bind(e.to_string()))?;
        std::net::UdpSocket::bind(at).map_err(|e| SpawnError::Bind(e.to_string()))?;
        let v6_listener = if v6_too {
            Some(v6_only_listener(port).map_err(|e| SpawnError::Bind(e.to_string()))?)
        } else {
            None
        };
        let tls = server_tls(seed).map_err(SpawnError::Other)?;
        let gate = Gate {
            admit,
            guest,
            live: Arc::default(),
            guests: Arc::default(),
        };
        let mut relay = RelayConfig::new(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0));
        relay.tls = Some(TlsConfig::new(
            at,
            CertConfig::Manual {
                server_config: tls.clone(),
            },
        ));
        relay.access = Arc::new(gate.clone());
        let mut cfg = ServerConfig::default();
        cfg.relay = Some(relay);
        cfg.quic = Some(QuicConfig::new(at));
        let server = Server::spawn(cfg)
            .await
            .map_err(|e| SpawnError::Bind(format!("{e:#}")))?;
        let v6 = match (v6_listener, server.relay_service()) {
            (Some(l), Some(svc)) => {
                let svc = svc.clone();
                let tls = iroh_relay::server::http_server::TlsConfig::new(Arc::new(tls));
                Some(tokio::spawn(async move {
                    while let Ok((stream, _)) = l.accept().await {
                        let (svc, tls) = (svc.clone(), tls.clone());
                        tokio::spawn(async move {
                            svc.handle_connection(stream, Some(tls), Duration::from_secs(30))
                                .await;
                        });
                    }
                }))
            }
            _ => None,
        };
        Ok(Relay {
            server: Mutex::new(Some(server)),
            gate,
            port,
            at,
            v6: Mutex::new(v6),
        })
    }

    /// TCP (HTTPS) and UDP (QAD) port N.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The HTTPS address the relay listens on.
    pub fn https_addr(&self) -> Option<SocketAddr> {
        lock(&self.server).as_ref().and_then(|s| s.https_addr())
    }

    /// Endpoints with a live relay connection.
    pub fn connected(&self) -> Vec<EndpointId> {
        self.gate.live().keys().copied().collect()
    }

    /// Disconnects every live client that admission now refuses (a revoked
    /// device, or a guest whose window closed); its reconnect is then
    /// denied. Returns how many were dropped.
    pub fn drop_refused(&self) -> usize {
        let g = lock(&self.server);
        let Some(svc) = g.as_ref().and_then(|s| s.relay_service()) else {
            return 0;
        };
        let mut n = 0;
        for id in self.connected() {
            if !self.gate.still(&id) && svc.clients().disconnect(id, None) {
                n += 1;
            }
        }
        n
    }

    /// Stops the relay and frees port N.
    pub async fn close(&self) {
        let t = lock(&self.v6).take();
        if let Some(t) = t {
            t.abort();
            let _ = t.await;
        }
        let s = lock(&self.server).take();
        if let Some(s) = s {
            let _ = s.shutdown().await;
        }
        // iroh-relay's shutdown returns before its tasks have let go of the
        // TCP listener or the QUIC socket (measured: 25-90 ms later under a
        // parallel build). A restart meanwhile found port N taken and moved
        // to another port, writing a node.addr row for nothing (Go's Close
        // frees it at once). Wait, bounded, until port N binds again.
        let deadline = std::time::Instant::now() + PORT_FREE_WAIT;
        while !port_free(self.at) && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

/// Whether TCP and UDP `at` can both be bound (the check binds and lets go).
fn port_free(at: SocketAddr) -> bool {
    std::net::TcpListener::bind(at).is_ok() && std::net::UdpSocket::bind(at).is_ok()
}

impl Drop for Relay {
    fn drop(&mut self) {
        if let Some(t) = lock(&self.v6).as_ref() {
            t.abort();
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn v6_only_listener(port: u16) -> std::io::Result<tokio::net::TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
    s.set_only_v6(true)?;
    s.bind(&SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port).into())?;
    s.listen(128)?;
    s.set_nonblocking(true)?;
    tokio::net::TcpListener::from_std(s.into())
}
