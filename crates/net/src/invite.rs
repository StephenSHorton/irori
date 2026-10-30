//! The invite room on `irori/invite/1`: the hooks the invites part (card
//! #44) builds on (docs/design/boards2-transport.md §5).
//!
//! The inviter sets an [`InviteHandler`] with [`IrohNet::set_invite_handler`]:
//! that opens an invite guest window
//! ([`crate::guest::GuestWindows::open_invite_window`]) on this device's
//! relay and on every home server it knows (the pairing `guest` message
//! with `kind: "invite"`), and the endpoint gate admits ids it doesn't know
//! on `irori/invite/1` while the handler is set. The invitee dials
//! [`IrohNet::dial_invite`] with the ticket's [`InviteAddr`], joining the
//! inviter's relay as a guest with the window's
//! [`crate::guest::invite_token`]. Both sides get a plain [`Conn`]:
//! `send`/`recv` carry the invite protocol (person proofs, bundle), and
//! `fingerprints()` is the session EKM the five words bind to (as pairing,
//! architecture §3.4). The ticket's encoding is the invites part's.
//!
//! The open exchange is the board-open's shape: the dialer opens the first
//! bi stream, label `sync`, and writes `{"v":1,"deviceId","proof"}`,
//! `proof = hex(HMAC-SHA256(invite room key, "irori/invite-open/1" ||
//! EKM32 || role))`, EKM = `EXPORTER-irori-invite`, role 0 from the dialer
//! and 1 from the acceptor, which answers in kind. The first dialer that
//! proves the key takes the invite: the gate closes to new ones at once,
//! and the windows close when that connection ends.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl};
use irori_meshseam::{
    Conn, ConnError, MAX_LABEL, RoomConfig, SYNC_LABEL, Secret, TransportError, read_frame, reason,
    write_frame,
};
use serde::{Deserialize, Serialize};

use crate::board::INVITE_ALPN;
use crate::guest::{INVITE_MAX_TTL, invite_token};
use crate::transport::{
    Inner, IrohNet, OPEN_MAX, ROLE_ACCEPT, ROLE_DIAL, RoomState, close_conn, closed_error, ekm,
    open_proof, open_proof_ok,
};

/// The invite-open proof's label (first message, as the board-open proof,
/// keyed by the invite room key from the code).
pub const INVITE_OPEN_LABEL: &[u8] = b"irori/invite-open/1";
/// The TLS exporter label of the invite-open proof's EKM32.
pub const EKM_INVITE: &[u8] = b"EXPORTER-irori-invite";
/// The invite-open message's version.
pub const INVITE_OPEN_V: u32 = 1;

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

#[derive(Serialize, Deserialize)]
struct InviteOpenMsg {
    v: u32,
    #[serde(rename = "deviceId")]
    device_id: String,
    proof: String,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The open invite on the inviter: its room, handler and windows.
pub(crate) struct InviteSlot {
    secret: Secret,
    handler: InviteHandler,
    /// The window on this device's own relay.
    window: Option<u64>,
    /// Connections holding windows on home servers.
    regs: Mutex<Vec<Connection>>,
    closed: AtomicBool,
}

impl IrohNet {
    /// Accept `irori/invite/1` connections for `room` (`None` stops). The
    /// endpoint admits the ALPN only while an invite window is open. Call
    /// it inside a tokio runtime: the endpoint binds and the home servers'
    /// windows open in the background.
    pub fn set_invite_handler(
        &self,
        room: Option<Secret>,
        h: Option<InviteHandler>,
    ) -> Result<(), TransportError> {
        let inner = self.inner().clone();
        inner.clear_invite();
        let (Some(room), Some(h)) = (room, h) else {
            return Ok(());
        };
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|_| TransportError("invite: no async runtime".into()))?;
        let window = inner.cfg.guests.open_invite_window(
            inner.cfg.key.public(),
            room.clone(),
            INVITE_MAX_TTL,
        )?;
        let slot = Arc::new(InviteSlot {
            secret: room,
            handler: h,
            window: Some(window),
            regs: Mutex::default(),
            closed: AtomicBool::new(false),
        });
        let old = lock(&inner.invite).replace(slot.clone());
        if let Some(old) = old {
            inner.close_slot(&old);
        }
        inner.invite_open.store(true, Ordering::SeqCst);
        rt.spawn(async move {
            if inner.bind().await.is_err() {
                return;
            }
            let servers = inner.relays.lock().expect("relays").clone();
            for sv in servers {
                if sv.server_pub == inner.cfg.key.public() {
                    continue;
                }
                let (net, slot) = (inner.clone(), slot.clone());
                tokio::spawn(async move {
                    let Some((conn, streams)) = net
                        .open_guest_on(sv, &slot.secret, INVITE_MAX_TTL, "invite")
                        .await
                    else {
                        return;
                    };
                    {
                        let mut regs = lock(&slot.regs);
                        if slot.closed.load(Ordering::SeqCst) {
                            close_conn(&conn, reason::LEAVING);
                            return;
                        }
                        regs.push(conn.clone());
                    }
                    // Held for as long as the window.
                    let _keep = streams;
                    conn.closed().await;
                });
            }
        });
        Ok(())
    }

    /// Where an invitee can dial this device (the ticket's fields): its
    /// EndpointId, its home relay with that server's key and QAD port, and
    /// the direct addresses iroh knows. Binds the endpoint.
    pub async fn invite_addr(&self) -> Result<InviteAddr, TransportError> {
        let inner = self.inner().clone();
        let ep = inner.bind().await?;
        let a = ep.addr();
        let relay = a.relay_urls().next().cloned();
        let spec = relay.as_ref().and_then(|u| {
            let relays = inner.relays.lock().expect("relays");
            relays.iter().find(|r| &r.url == u).cloned()
        });
        let mut direct: Vec<SocketAddr> = a.ip_addrs().copied().collect();
        if direct.is_empty() {
            // Before iroh has found its addresses: the sockets it bound,
            // when they name an interface.
            direct = ep
                .bound_sockets()
                .into_iter()
                .filter(|s| !s.ip().is_unspecified())
                .collect();
        }
        Ok(InviteAddr {
            id: ep.id(),
            relay_quic_port: spec.as_ref().map(|s| s.quic_port).unwrap_or(0),
            relay_pub: spec.map(|s| s.server_pub),
            relay,
            direct,
        })
    }

    /// The invitee's side: dial the inviter as a relay guest (token from
    /// `room.key`), prove the room key, and return the connection.
    pub async fn dial_invite(
        &self,
        to: &InviteAddr,
        room: &Secret,
    ) -> Result<Arc<dyn Conn>, TransportError> {
        let inner = self.inner().clone();
        let ep = inner.bind().await?;
        let mut addr = EndpointAddr::new(to.id);
        // The inviter's relay, joined as a guest unless the mesh already
        // uses it; taken out again when the invite connection ends.
        let mut guest: Option<(RelayUrl, Option<[u8; 32]>)> = None;
        if let Some(url) = &to.relay {
            let mesh = inner.relays.lock().expect("relays").clone();
            if !mesh.iter().any(|r| &r.url == url) {
                let pin = to.relay_pub.map(|k| *k.as_bytes());
                if let Some(k) = pin {
                    inner.pins.add(k);
                }
                let port = match to.relay_quic_port {
                    0 => url.port().unwrap_or(443),
                    p => p,
                };
                let cfg = crate::relay::client_relay_config(url.clone(), port)
                    .with_auth_token(invite_token(&room.key));
                ep.insert_relay(url.clone(), Arc::new(cfg)).await;
                guest = Some((url.clone(), pin));
            }
            addr = addr.with_relay_url(url.clone());
        }
        for d in &to.direct {
            addr = addr.with_ip_addr(*d);
        }
        lock(&inner.invite_dials).insert(to.id);
        let res = inner.dial_invite_once(&ep, addr, room).await;
        lock(&inner.invite_dials).remove(&to.id);
        match res {
            Ok(c) => {
                if let Some(g) = guest {
                    let (net, done) = (inner.clone(), c.done());
                    tokio::spawn(async move {
                        done.await;
                        net.drop_invite_relay(&ep, g).await;
                    });
                }
                Ok(c)
            }
            Err(e) => {
                if let Some(g) = guest {
                    inner.drop_invite_relay(&ep, g).await;
                }
                Err(TransportError(e.to_string()))
            }
        }
    }
}

/// A one-connection room for the invite connection: `setup` hands the
/// connection to `on_peer`.
fn invite_room(secret: &Secret, self_id: &str, on_peer: InviteHandler) -> Arc<RoomState> {
    let mut cfg = RoomConfig::new(self_id);
    cfg.on_peer = on_peer;
    Arc::new(RoomState::new(secret.clone(), cfg))
}

impl Inner {
    /// Takes down the open invite, if any.
    pub(crate) fn clear_invite(&self) {
        let old = lock(&self.invite).take();
        self.invite_open.store(false, Ordering::SeqCst);
        if let Some(s) = old {
            self.close_slot(&s);
        }
    }

    fn close_slot(&self, s: &InviteSlot) {
        s.closed.store(true, Ordering::SeqCst);
        if let Some(w) = s.window {
            self.cfg.guests.close_window(w);
        }
        for c in lock(&s.regs).drain(..) {
            close_conn(&c, reason::LEAVING);
        }
    }

    async fn drop_invite_relay(&self, ep: &Endpoint, (url, pin): (RelayUrl, Option<[u8; 32]>)) {
        let mesh = self.relays.lock().expect("relays").clone();
        if let Some(k) = pin
            && !mesh.iter().any(|r| r.server_pub.as_bytes() == &k)
        {
            self.pins.remove(&k);
        }
        if !mesh.iter().any(|r| r.url == url) {
            ep.remove_relay(&url).await;
        }
    }

    async fn dial_invite_once(
        self: &Arc<Self>,
        ep: &Endpoint,
        addr: EndpointAddr,
        room: &Secret,
    ) -> Result<Arc<dyn Conn>, ConnError> {
        let conn = tokio::time::timeout(self.cfg.connect_timeout, ep.connect(addr, INVITE_ALPN))
            .await
            .map_err(|_| ConnError::Transport(reason::TIMEOUT.into()))?
            .map_err(|e| ConnError::Transport(format!("connect: {e:#}")))?;
        let k = ekm(&conn, EKM_INVITE)?;
        let hs = async {
            let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
            write_frame(&mut send, SYNC_LABEL.as_bytes(), MAX_LABEL)
                .await
                .map_err(|e| e.to_string())?;
            write_open(&mut send, &self.cfg.self_id, &room.key, &k, ROLE_DIAL).await?;
            let reply = read_open(&mut recv).await?;
            Ok::<_, String>((send, recv, reply))
        };
        let (send, recv, reply) = match tokio::time::timeout(self.cfg.open_timeout, hs).await {
            Err(_) => {
                close_conn(&conn, reason::TIMEOUT);
                return Err(ConnError::Local(reason::TIMEOUT.into()));
            }
            Ok(Err(e)) => {
                let err = closed_error(&conn, e);
                close_conn(&conn, reason::PROTOCOL);
                return Err(err);
            }
            Ok(Ok(x)) => x,
        };
        if reply.v != INVITE_OPEN_V {
            close_conn(&conn, reason::VERSION);
            return Err(ConnError::Local(reason::VERSION.into()));
        }
        if reply.device_id.is_empty()
            || reply.device_id == self.cfg.self_id
            || !open_proof_ok(INVITE_OPEN_LABEL, &room.key, &k, ROLE_ACCEPT, &reply.proof)
        {
            close_conn(&conn, reason::REFUSED);
            return Err(ConnError::Local(reason::REFUSED.into()));
        }
        let got: Arc<Mutex<Option<Arc<dyn Conn>>>> = Arc::default();
        let g = got.clone();
        let room = invite_room(
            room,
            &self.cfg.self_id,
            Arc::new(move |c| *lock(&g) = Some(c)),
        );
        let peer = reply.device_id;
        self.setup(conn, &room, peer, self.cfg.self_id.clone(), send, recv)?;
        lock(&got)
            .take()
            .ok_or_else(|| ConnError::Local(reason::LEAVING.into()))
    }

    /// The invite-open exchange, acceptor side.
    pub(crate) async fn handle_invite(self: &Arc<Self>, conn: Connection) {
        let Some(slot) = lock(&self.invite).clone() else {
            return close_conn(&conn, reason::REFUSED);
        };
        let hs = async {
            let (send, mut recv) = conn.accept_bi().await.map_err(|e| e.to_string())?;
            let label = read_frame(&mut recv, MAX_LABEL, false)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "no label".to_string())?;
            if label != SYNC_LABEL.as_bytes() {
                return Err("first stream is not sync".to_string());
            }
            let open = read_open(&mut recv).await?;
            Ok::<_, String>((send, recv, open))
        };
        let (mut send, recv, open) = match tokio::time::timeout(self.cfg.open_timeout, hs).await {
            Ok(Ok(x)) => x,
            Ok(Err(_)) => return close_conn(&conn, reason::PROTOCOL),
            Err(_) => return close_conn(&conn, reason::TIMEOUT),
        };
        if open.v != INVITE_OPEN_V {
            return close_conn(&conn, reason::VERSION);
        }
        let Ok(k) = ekm(&conn, EKM_INVITE) else {
            return close_conn(&conn, reason::PROTOCOL);
        };
        let peer = open.device_id;
        if peer.is_empty()
            || peer == self.cfg.self_id
            || !open_proof_ok(
                INVITE_OPEN_LABEL,
                &slot.secret.key,
                &k,
                ROLE_DIAL,
                &open.proof,
            )
        {
            return close_conn(&conn, reason::REFUSED);
        }
        // The first that proves the key takes the invite.
        {
            let mut g = lock(&self.invite);
            if !g.as_ref().is_some_and(|s| Arc::ptr_eq(s, &slot)) {
                return close_conn(&conn, reason::REFUSED);
            }
            *g = None;
            self.invite_open.store(false, Ordering::SeqCst);
        }
        if write_open(
            &mut send,
            &self.cfg.self_id,
            &slot.secret.key,
            &k,
            ROLE_ACCEPT,
        )
        .await
        .is_err()
        {
            self.close_slot(&slot);
            return close_conn(&conn, reason::PROTOCOL);
        }
        let room = invite_room(&slot.secret, &self.cfg.self_id, slot.handler.clone());
        let done = conn.clone();
        let _ = self.setup(conn, &room, peer.clone(), peer, send, recv);
        // The windows stay while the invitee may still be on the relay path.
        let net = self.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = done.closed() => {}
                _ = tokio::time::sleep(INVITE_MAX_TTL) => {}
            }
            net.close_slot(&slot);
        });
    }
}

async fn write_open(
    s: &mut SendStream,
    self_id: &str,
    key: &[u8; 32],
    ekm: &[u8; 32],
    role: u8,
) -> Result<(), String> {
    let m = InviteOpenMsg {
        v: INVITE_OPEN_V,
        device_id: self_id.to_string(),
        proof: open_proof(INVITE_OPEN_LABEL, key, ekm, role),
    };
    let b = serde_json::to_vec(&m).expect("open msg");
    write_frame(s, &b, OPEN_MAX)
        .await
        .map_err(|e| e.to_string())
}

async fn read_open(r: &mut RecvStream) -> Result<InviteOpenMsg, String> {
    let b = read_frame(r, OPEN_MAX, false)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no invite-open message".to_string())?;
    serde_json::from_slice(&b).map_err(|e| e.to_string())
}
