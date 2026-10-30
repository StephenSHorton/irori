//! The mesh transport on iroh (architecture §3.1, mesh.md §5.2-5.3): the
//! `irori_meshseam::Transport` the mesh Node runs on, in place of go-room.
//!
//! - One [`Endpoint`] per process, keyed by the device seed (D2), built from
//!   `Builder::empty()` (never a preset) and bound lazily on the first
//!   [`Transport::join`] (`TestNothingOpensBeforeTheFirstJoin`).
//! - Every connection passes [`Admit`] in `after_handshake`, before any
//!   stream is read; the mesh's `accept` (Go `CheckDevice`) runs again on the
//!   epoch-open message, with the device id it names.
//! - Rooms are epochs. The dialer opens the first bi stream, label `sync`,
//!   and writes the epoch-open message; the acceptor answers with its own:
//!   `{"v":1,"epoch":E,"deviceId":id,"proof":hex(HMAC-SHA256(MeshSecret(E).key,
//!   "irori/mesh-open/1" || EKM32 || role))}`, EKM = `EXPORTER-irori-mesh`,
//!   role 0 from the dialer and 1 from the acceptor. Two refinements of
//!   mesh.md §5.2's message: `deviceId`, since an acceptor sees only a key and
//!   the mesh's admission is by device id; and the role byte, so one side
//!   cannot pass by echoing the other's proof.
//! - Streams start with `u32 BE len || label`; `sync` then carries
//!   `u32 BE len || message`, at most 8 MiB. Outgoing streams: 4 at a time;
//!   incoming: 1 + 8 concurrent bi streams.
//! - The lower device id dials. The higher one dials too after `dial_grace`
//!   with no connection (iroh has no "want" message). Of two live connections
//!   between a pair in one room, the one the lower id dialed stays and the
//!   other is closed `replaced`; of two dialed by the same side, the newer.
//! - Addresses come from outside: [`IrohNet::set_servers`] (home relays,
//!   pinned to their server's device key) and [`IrohNet::set_peers`] (each
//!   device's EndpointId, relay URL and direct addresses), both from the mesh's
//!   `Servers` fold of `node.addr` rows and the Chain's certs.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll};
use std::time::Duration;

use hmac::{Hmac, KeyInit, Mac};
use iroh::address_lookup::MemoryLookup;
use iroh::endpoint::{
    AfterHandshakeOutcome, Connection, ConnectionError, EndpointHooks, NetReportConfig,
    PortmapperConfig, QuicTransportConfig, RecvStream, SendStream, VarInt,
};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMap, RelayMode, RelayUrl, SecretKey};
use irori_meshseam::{
    BoxFut, Conn, ConnError, Envelope, MAX_LABEL, MAX_MESSAGE, MSG_PRESENCE, Room, RoomConfig,
    SYNC_LABEL, Secret, Stream, StreamHandler, Transport, TransportError, colon_hex, read_frame,
    reason, write_frame,
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::{OnceCell, OwnedSemaphorePermit, Semaphore, watch};
use tokio_util::sync::CancellationToken;

use crate::admit::Admit;
use crate::endpoint::{IpMode, RelaySpec};

/// The private mesh's ALPN (architecture §3.1).
pub const MESH_ALPN: &[u8] = b"irori/mesh/1";
/// Pairing (M7).
pub const PAIR_ALPN: &[u8] = b"irori/pair/1";
/// Board gossip (iroh-gossip, D13), Irori's own ALPN so it is gated like the mesh.
pub const GOSSIP_ALPN: &[u8] = b"irori/gossip/1";

const OPEN_LABEL: &[u8] = b"irori/mesh-open/1";
const EKM_MESH: &[u8] = b"EXPORTER-irori-mesh";
const EKM_SAS: &[u8] = b"EXPORTER-irori-sas";
pub(crate) const ROLE_DIAL: u8 = 0;
pub(crate) const ROLE_ACCEPT: u8 = 1;
pub(crate) const OPEN_MAX: usize = 4096;
/// Outgoing streams at a time per connection (go-room MaxStreams 4).
const OUT_STREAMS: usize = 4;
/// Incoming concurrent bi streams: `sync` plus 8.
const IN_STREAMS: u32 = 1 + 8;

/// Where one device can be dialed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerAddr {
    pub device_id: String,
    /// Its EndpointId: the certified devicePub (D2).
    pub id: EndpointId,
    /// Its home relay, if known.
    pub relay: Option<RelayUrl>,
    /// Direct addresses to try (LAN, v6, public; `hub-addrs.json` hints).
    pub direct: Vec<SocketAddr>,
}

impl PeerAddr {
    pub(crate) fn endpoint_addr(&self) -> EndpointAddr {
        let mut a = EndpointAddr::new(self.id);
        if let Some(r) = &self.relay {
            a = a.with_relay_url(r.clone());
        }
        for d in &self.direct {
            a = a.with_ip_addr(*d);
        }
        a
    }
}

/// How the transport binds and times its handshakes.
#[derive(Clone)]
pub struct NetConfig {
    pub self_id: String,
    /// The device key (D2).
    pub key: SecretKey,
    pub ip: IpMode,
    /// The endpoint-level gate (`after_handshake`).
    pub admit: Arc<dyn Admit>,
    /// Dial timeout (go-room's connect timeout, 30 s).
    pub connect_timeout: Duration,
    /// Time for the epoch-open exchange (go-room's auth timeout, 10 s).
    pub open_timeout: Duration,
    /// How long the higher id waits for the lower one to dial before it dials.
    pub dial_grace: Duration,
    /// Run iroh-gossip for board announcements.
    pub gossip: bool,
    /// The sealed LAN lookup on UDP 38419 (`irori_lan`, D6), added beside
    /// the address book so a dial by id alone finds a LAN peer; the endpoint
    /// publishes its own addresses and home relay to it. `None` where only
    /// the address book is wanted (the loopback tests). An already started
    /// lookup; [`NetConfig::lan_config`] is the lazy one.
    pub lan: Option<irori_lan::LanLookup>,
    /// The LAN lookup to start when the endpoint binds (the first join, a
    /// pairing room), as Go's roomnet opens its LAN carrier on the first
    /// Join (I:internal/mesh/roomnet/roomnet.go:134-163): a lone device
    /// binds no UDP 38419. [`crate::pairnet::PairLan::wire`] is applied and
    /// its `on_heard` also calls [`IrohNet::set_on_heard`]'s hook. Ignored
    /// when [`NetConfig::lan`] is set.
    pub lan_config: Option<irori_lan::Config>,
    /// Every name the endpoint's resolver is asked ([`crate::dns`]).
    pub dns_log: crate::dns::DnsLog,
    /// Guest windows on this device's relay (M7): the home server's relay
    /// admits their tokens, and pairing rooms open them.
    pub guests: crate::guest::GuestWindows,
    /// Pairing on the LAN lookup (M7): wire it into `lan`'s config before
    /// the lookup starts ([`crate::pairnet::PairLan::wire`]).
    pub pair_lan: crate::pairnet::PairLan,
    /// The boards this device holds, for a [`crate::board::MemberAdmit`]
    /// gate built over the same registry (boards stage 2): the Core adds a
    /// board's verifier when it opens the board. An `admit` that is not a
    /// `MemberAdmit` over it ignores it.
    pub boards: Arc<crate::board::BoardMembers>,
}

impl NetConfig {
    pub fn new(self_id: impl Into<String>, key: SecretKey, admit: Arc<dyn Admit>) -> NetConfig {
        NetConfig {
            self_id: self_id.into(),
            key,
            ip: IpMode::Loopback,
            admit,
            connect_timeout: Duration::from_secs(30),
            open_timeout: Duration::from_secs(10),
            dial_grace: Duration::from_secs(2),
            gossip: true,
            lan: None,
            lan_config: None,
            dns_log: crate::dns::DnsLog::default(),
            guests: crate::guest::GuestWindows::default(),
            pair_lan: crate::pairnet::PairLan::default(),
            boards: crate::board::BoardMembers::new(),
        }
    }
}

impl fmt::Debug for NetConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NetConfig")
            .field("self_id", &self.self_id)
            .field("ip", &self.ip)
            .finish_non_exhaustive()
    }
}

/// One live connection, for status lines and tests.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnInfo {
    pub epoch: i64,
    pub peer: String,
    pub dialer: String,
    pub path: String,
    pub rtt: Duration,
}

/// The iroh transport. Cheap to clone.
#[derive(Clone)]
pub struct IrohNet {
    inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) cfg: NetConfig,
    pub(crate) ep: OnceCell<Endpoint>,
    pub(crate) lookup: MemoryLookup,
    pub(crate) pins: crate::tls::PinSet,
    pub(crate) relays: Arc<Mutex<Vec<RelaySpec>>>,
    book: Mutex<BTreeMap<String, PeerAddr>>,
    rooms: Mutex<HashMap<i64, Arc<RoomState>>>,
    /// Board rooms (stage 2, `irori/board/1`), by (board, board epoch).
    pub(crate) board_rooms: Mutex<HashMap<(String, i64), Arc<RoomState>>>,
    /// Each board's member devices and where to dial them
    /// ([`IrohNet::set_board_peers`]); kept apart from the private book,
    /// which [`IrohNet::set_peers`] replaces on every refeed.
    pub(crate) board_book: Mutex<BTreeMap<String, BTreeMap<String, PeerAddr>>>,
    /// The `board.server` relays of every board held here (B1).
    pub(crate) board_relays: Mutex<Vec<crate::board::BoardRelays>>,
    /// (board or "" for the private mesh, epoch, peer) dials in flight.
    dialing: Mutex<HashSet<(String, i64, String)>>,
    pub(crate) dial_errors: Mutex<BTreeMap<String, String>>,
    pub(crate) gossip: OnceCell<iroh_gossip::net::Gossip>,
    /// The open pairing room (epoch 0), at most one (M7).
    pub(crate) pair: Mutex<Option<Arc<crate::pairnet::PairRoom>>>,
    pub(crate) pair_open: Arc<AtomicBool>,
    /// The open invite (card #44), at most one.
    pub(crate) invite: Mutex<Option<Arc<crate::invite::InviteSlot>>>,
    pub(crate) invite_open: Arc<AtomicBool>,
    /// Inviters this device is dialing on `irori/invite/1` (the gate runs on
    /// outgoing connections too).
    pub(crate) invite_dials: Arc<Mutex<HashSet<EndpointId>>>,
    /// The lookup started from `cfg.lan_config` at bind.
    lan: std::sync::OnceLock<irori_lan::LanLookup>,
    /// Late-bound `on_heard` (the mesh's `Servers::heard`, built after the
    /// transport).
    on_heard: Arc<Mutex<Option<irori_lan::HeardFn>>>,
    closed: AtomicBool,
}

pub(crate) struct RoomState {
    pub(crate) secret: Secret,
    pub(crate) cfg: RoomConfig,
    /// The board of a board room; `None` for a private mesh room.
    pub(crate) board: Option<String>,
    conns: Mutex<HashMap<String, Arc<IrohConn>>>,
    closed: AtomicBool,
}

impl RoomState {
    pub(crate) fn new(secret: Secret, cfg: RoomConfig) -> RoomState {
        RoomState {
            secret,
            cfg,
            board: None,
            conns: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
        }
    }

    /// Its live connections, for status lines and tests.
    pub(crate) fn conn_infos(&self) -> Vec<ConnInfo> {
        self.conns
            .lock()
            .expect("conns")
            .values()
            .filter(|c| !c.done.is_cancelled())
            .map(|c| ConnInfo {
                epoch: c.epoch,
                peer: c.peer.clone(),
                dialer: c.dialer.clone(),
                path: c.path(),
                rtt: c.rtt(),
            })
            .collect()
    }

    fn live(&self, peer: &str) -> bool {
        self.conns
            .lock()
            .expect("conns")
            .get(peer)
            .is_some_and(|c| !c.done.is_cancelled())
    }
}

/// The endpoint gate. On `irori/pair/1` it also admits ids it doesn't
/// know, but only while a pairing room or a pairing guest window is open
/// here (an invite window never opens pairing to strangers); on
/// `irori/invite/1`, only while an invite handler is set
/// ([`IrohNet::set_invite_handler`]) or, outgoing, to the inviter an
/// [`IrohNet::dial_invite`] is dialing.
#[derive(Debug)]
struct Gate {
    admit: Arc<dyn Admit>,
    pair_open: Arc<AtomicBool>,
    invite_open: Arc<AtomicBool>,
    invite_dials: Arc<Mutex<HashSet<EndpointId>>>,
    guests: crate::guest::GuestWindows,
}

impl EndpointHooks for Gate {
    async fn after_handshake(&self, conn: &Connection) -> AfterHandshakeOutcome {
        let pairing = conn.alpn() == PAIR_ALPN
            && (self.pair_open.load(Ordering::SeqCst)
                || self.guests.any_open_of(crate::guest::WindowKind::Pair));
        let inviting = conn.alpn() == crate::board::INVITE_ALPN
            && (self.invite_open.load(Ordering::SeqCst)
                || self
                    .invite_dials
                    .lock()
                    .expect("invite dials")
                    .contains(&conn.remote_id()));
        if pairing || inviting || self.admit.admits(conn.alpn(), &conn.remote_id()) {
            AfterHandshakeOutcome::Accept
        } else {
            AfterHandshakeOutcome::Reject {
                error_code: VarInt::from_u32(reason::code(reason::REFUSED)),
                reason: reason::REFUSED.as_bytes().to_vec(),
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
struct OpenMsg {
    v: u32,
    epoch: i64,
    #[serde(rename = "deviceId")]
    device_id: String,
    proof: String,
}

/// The session's TLS exporter bytes under `label` (the open proofs, the
/// SAS); the pairing transport uses it too.
pub(crate) fn ekm(conn: &Connection, label: &[u8]) -> Result<[u8; 32], ConnError> {
    let mut out = [0u8; 32];
    conn.export_keying_material(&mut out, label, b"")
        .map_err(|e| ConnError::Transport(format!("keying material: {e:?}")))?;
    Ok(out)
}

/// HMAC(room key, `label` || EKM || role): the open proof of the mesh's
/// epoch rooms and, under its own label, of the pairing room.
fn open_mac(label: &[u8], key: &[u8; 32], ekm: &[u8; 32], role: u8) -> Hmac<Sha256> {
    let mut m = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("any key length");
    m.update(label);
    m.update(ekm);
    m.update(&[role]);
    m
}

pub(crate) fn open_proof(label: &[u8], key: &[u8; 32], ekm: &[u8; 32], role: u8) -> String {
    hex::encode(open_mac(label, key, ekm, role).finalize().into_bytes())
}

pub(crate) fn open_proof_ok(
    label: &[u8],
    key: &[u8; 32],
    ekm: &[u8; 32],
    role: u8,
    got: &str,
) -> bool {
    let Ok(b) = hex::decode(got) else {
        return false;
    };
    open_mac(label, key, ekm, role).verify_slice(&b).is_ok()
}

fn proof(key: &[u8; 32], ekm: &[u8; 32], role: u8) -> String {
    open_proof(OPEN_LABEL, key, ekm, role)
}

fn proof_ok(key: &[u8; 32], ekm: &[u8; 32], role: u8, got: &str) -> bool {
    open_proof_ok(OPEN_LABEL, key, ekm, role, got)
}

fn conn_error(e: &ConnectionError) -> ConnError {
    match e {
        ConnectionError::ApplicationClosed(c) => {
            ConnError::Remote(String::from_utf8_lossy(&c.reason).into_owned())
        }
        ConnectionError::LocallyClosed => ConnError::Local("closed".into()),
        ConnectionError::TimedOut => ConnError::Transport("timed out".into()),
        other => ConnError::Transport(other.to_string()),
    }
}

/// The reason a connection that failed mid-handshake closed with, if any.
pub(crate) fn closed_error(conn: &Connection, fallback: impl Into<String>) -> ConnError {
    match conn.close_reason() {
        Some(e) => conn_error(&e),
        None => ConnError::Transport(fallback.into()),
    }
}

pub(crate) fn close_conn(conn: &Connection, why: &str) {
    conn.close(VarInt::from_u32(reason::code(why)), why.as_bytes());
}

impl IrohNet {
    pub fn new(cfg: NetConfig) -> IrohNet {
        IrohNet {
            inner: Arc::new(Inner {
                cfg,
                ep: OnceCell::new(),
                lookup: MemoryLookup::new(),
                pins: crate::tls::PinSet::default(),
                relays: Arc::new(Mutex::new(Vec::new())),
                book: Mutex::new(BTreeMap::new()),
                rooms: Mutex::new(HashMap::new()),
                board_rooms: Mutex::new(HashMap::new()),
                board_book: Mutex::new(BTreeMap::new()),
                board_relays: Mutex::new(Vec::new()),
                dialing: Mutex::new(HashSet::new()),
                dial_errors: Mutex::new(BTreeMap::new()),
                gossip: OnceCell::new(),
                pair: Mutex::new(None),
                pair_open: Arc::new(AtomicBool::new(false)),
                invite: Mutex::new(None),
                invite_open: Arc::new(AtomicBool::new(false)),
                invite_dials: Arc::new(Mutex::new(HashSet::new())),
                lan: std::sync::OnceLock::new(),
                on_heard: Arc::new(Mutex::new(None)),
                closed: AtomicBool::new(false),
            }),
        }
    }

    pub(crate) fn inner(&self) -> &Arc<Inner> {
        &self.inner
    }

    pub fn self_id(&self) -> &str {
        &self.inner.cfg.self_id
    }

    /// The bound endpoint, once a join has bound it.
    pub fn endpoint(&self) -> Option<&Endpoint> {
        self.inner.ep.get()
    }

    /// Bind now (a join does it on its own).
    pub async fn bind(&self) -> Result<Endpoint, TransportError> {
        self.inner.bind().await
    }

    /// The home relays to use, each pinned to its server's device key. New
    /// ones are inserted into a bound endpoint; ones no longer listed are
    /// removed, and so is the pin of a server none of them names any more
    /// (a revoked home server stops being a trusted relay certificate).
    pub async fn set_servers(&self, servers: Vec<RelaySpec>) {
        for s in &servers {
            self.inner.pins.add(*s.server_pub.as_bytes());
        }
        let old = std::mem::replace(
            &mut *self.inner.relays.lock().expect("relays"),
            servers.clone(),
        );
        let boards = self
            .inner
            .board_relays
            .lock()
            .expect("board relays")
            .clone();
        for o in &old {
            if !servers.iter().any(|s| s.server_pub == o.server_pub)
                && !boards
                    .iter()
                    .flat_map(|b| b.servers.iter())
                    .any(|s| s.server_pub == o.server_pub)
            {
                self.inner.pins.remove(o.server_pub.as_bytes());
            }
        }
        let Some(ep) = self.inner.ep.get() else {
            return;
        };
        for o in &old {
            if !servers.iter().any(|s| s.url == o.url) {
                ep.remove_relay(&o.url).await;
            }
        }
        for s in &servers {
            if !old
                .iter()
                .any(|o| o.url == s.url && o.quic_port == s.quic_port)
            {
                ep.insert_relay(
                    s.url.clone(),
                    Arc::new(crate::relay::client_relay_config(
                        s.url.clone(),
                        s.quic_port,
                    )),
                )
                .await;
            }
        }
    }

    /// The home relays as last fed ([`IrohNet::set_servers`]).
    pub fn servers(&self) -> Vec<RelaySpec> {
        self.inner.relays.lock().expect("relays").clone()
    }

    /// Stop trusting a relay server's key (a revoked home server).
    pub fn unpin(&self, server: &EndpointId) {
        self.inner.pins.remove(server.as_bytes());
    }

    /// The address book: who can be dialed and where. A peer known by its
    /// id alone (no relay, no direct address) leaves what the lookups
    /// already hold for it alone: the addresses pairing or the LAN lookup
    /// found are not wiped by a book that knows less.
    pub fn set_peers(&self, peers: Vec<PeerAddr>) {
        let mut book = self.inner.book.lock().expect("book");
        book.clear();
        for p in peers {
            if p.relay.is_some() || !p.direct.is_empty() {
                self.inner.lookup.set_endpoint_info(p.endpoint_addr());
            }
            book.insert(p.device_id.clone(), p);
        }
    }

    /// Re-run every private room's `accept` on its live connections and
    /// close the ones it now refuses with `revoked` (Go `dropRevoked`; call
    /// it on every person row). New connections from them are already
    /// refused. Board rooms are their board mesh's to recheck
    /// (`mesh::BoardMesh::recheck`), which tells the peer its last vector
    /// before closing.
    pub fn recheck(&self) {
        for room in self.inner.rooms_snapshot() {
            let conns: Vec<_> = room
                .conns
                .lock()
                .expect("conns")
                .values()
                .cloned()
                .collect();
            for c in conns {
                if (room.cfg.accept)(&c.peer, &c.pubkey).is_err() {
                    c.close(reason::REVOKED);
                }
            }
        }
    }

    /// Close every connection to `device_id` with `why`.
    pub fn drop_device(&self, device_id: &str, why: &str) {
        for room in self.inner.all_rooms() {
            let c = room.conns.lock().expect("conns").get(device_id).cloned();
            if let Some(c) = c {
                c.close(why);
            }
        }
    }

    /// Live connections, by epoch then peer.
    pub fn conns(&self) -> Vec<ConnInfo> {
        let mut out = Vec::new();
        for room in self.inner.rooms_snapshot() {
            out.extend(room.conn_infos());
        }
        out.sort_by(|a, b| (a.epoch, &a.peer).cmp(&(b.epoch, &b.peer)));
        out
    }

    /// Open rooms (Go `Status.Topics` while bound).
    pub fn rooms(&self) -> Vec<i64> {
        let mut v: Vec<i64> = self
            .inner
            .rooms
            .lock()
            .expect("rooms")
            .keys()
            .copied()
            .collect();
        v.sort();
        v
    }

    /// The last dial failure per peer, for status lines.
    pub fn dial_errors(&self) -> BTreeMap<String, String> {
        self.inner.dial_errors.lock().expect("errs").clone()
    }

    /// The endpoint's home relay now and whether it is connected (iroh
    /// keeps one home relay of the configured ones); empty before the
    /// endpoint binds or picks one.
    pub fn home_relays(&self) -> Vec<(iroh::RelayUrl, bool)> {
        use iroh::Watcher as _;
        match self.inner.ep.get() {
            Some(ep) => ep
                .home_relay_status()
                .get()
                .into_iter()
                .map(|s| (s.url().clone(), s.is_connected()))
                .collect(),
            None => Vec::new(),
        }
    }

    /// Close every room (`shutting down`) and the endpoint.
    /// The guest windows open on this device's relay (the home server's
    /// relay admits their tokens).
    pub fn guests(&self) -> &crate::guest::GuestWindows {
        &self.inner.cfg.guests
    }

    /// Whether this endpoint runs board gossip ([`NetConfig::gossip`]).
    pub fn gossip_on(&self) -> bool {
        self.inner.cfg.gossip
    }

    /// The boards counted at the endpoint gate ([`NetConfig::boards`]).
    pub fn board_members(&self) -> &Arc<crate::board::BoardMembers> {
        &self.inner.cfg.boards
    }

    /// Pairing on the LAN lookup: the bridge names this server's relay
    /// through it ([`crate::pairnet::PairLan::set_relay`]).
    pub fn pair_lan(&self) -> &crate::pairnet::PairLan {
        &self.inner.cfg.pair_lan
    }

    /// The LAN lookup, once something needed it (`None` before the first
    /// bind, or with no lookup configured).
    pub fn lan(&self) -> Option<irori_lan::LanLookup> {
        self.inner.lan()
    }

    /// Why devices on this network can't find this one directly, or ""
    /// while the lookup works or before anything started it (Go
    /// `Transport.LANError`, roomnet.go:219-234).
    pub fn lan_error(&self) -> String {
        self.lan().map(|l| l.error_line()).unwrap_or_default()
    }

    /// Called with every authentic LAN announcement's sender and source
    /// (Go roomnet's `OnHeard` → `Servers.Heard`), for the lookup started
    /// from [`NetConfig::lan_config`] (a pre-started [`NetConfig::lan`]
    /// keeps its own `on_heard`).
    pub fn set_on_heard(&self, f: irori_lan::HeardFn) {
        *self.inner.on_heard.lock().expect("on_heard") = Some(f);
    }

    /// Every name the endpoint's resolver was asked.
    pub fn dns_log(&self) -> &crate::dns::DnsLog {
        &self.inner.cfg.dns_log
    }

    /// Whether a pairing room is open on this device.
    pub fn pairing(&self) -> bool {
        self.inner.pair_open.load(Ordering::SeqCst)
    }

    pub async fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        let pr = self.inner.pair.lock().expect("pair").clone();
        if let Some(pr) = pr {
            self.inner.close_pair(&pr);
        }
        self.inner.clear_invite();
        for room in self.inner.all_rooms() {
            self.inner.close_room(&room, reason::SHUTTING_DOWN);
        }
        if let Some(g) = self.inner.gossip.get() {
            let _ = g.shutdown().await;
        }
        if let Some(ep) = self.inner.ep.get() {
            ep.close().await;
        }
        // The LAN lookup goes with the endpoint: its socket is freed and
        // nobody keeps announcing addresses that no longer answer.
        if let Some(l) = self.inner.lan() {
            l.close().await;
        }
    }
}

impl Inner {
    /// The EndpointIds of these devices: the private book's, else a board
    /// book's (another person's member device, for its board's gossip).
    pub(crate) fn endpoint_ids(&self, device_ids: &[String]) -> Vec<EndpointId> {
        let book = self.book.lock().expect("book");
        let boards = self.board_book.lock().expect("board book");
        device_ids
            .iter()
            .filter_map(|d| {
                book.get(d)
                    .or_else(|| boards.values().find_map(|b| b.get(d)))
                    .map(|p| p.id)
            })
            .collect()
    }

    fn lan(&self) -> Option<irori_lan::LanLookup> {
        self.cfg.lan.clone().or_else(|| self.lan.get().cloned())
    }

    /// Starts the configured LAN lookup, once (under the bind's `OnceCell`).
    fn start_lan(&self) -> Option<irori_lan::LanLookup> {
        if let Some(l) = &self.cfg.lan {
            return Some(l.clone());
        }
        let mut c = self.cfg.lan_config.clone()?;
        if let Some(l) = self.lan.get() {
            return Some(l.clone());
        }
        self.cfg.pair_lan.wire(&mut c);
        let base = c.on_heard.take();
        let late = self.on_heard.clone();
        c.on_heard = Some(Arc::new(move |id, from| {
            if let Some(f) = &base {
                f(id, from);
            }
            let f = late.lock().expect("on_heard").clone();
            if let Some(f) = f {
                f(id, from);
            }
        }));
        let l = irori_lan::LanLookup::start(c);
        self.cfg.pair_lan.attach(l.clone());
        let _ = self.lan.set(l.clone());
        Some(l)
    }

    fn rooms_snapshot(&self) -> Vec<Arc<RoomState>> {
        self.rooms
            .lock()
            .expect("rooms")
            .values()
            .cloned()
            .collect()
    }

    pub(crate) fn board_rooms_snapshot(&self) -> Vec<Arc<RoomState>> {
        self.board_rooms
            .lock()
            .expect("board rooms")
            .values()
            .cloned()
            .collect()
    }

    /// The private mesh's rooms and the board rooms.
    fn all_rooms(&self) -> Vec<Arc<RoomState>> {
        let mut v = self.rooms_snapshot();
        v.extend(self.board_rooms_snapshot());
        v
    }

    /// Where to dial `peer` for `room`: a board room's own book first (the
    /// other members' devices), then the private book (own devices).
    fn peer_addr(&self, room: &RoomState, peer: &str) -> Option<PeerAddr> {
        if let Some(b) = &room.board
            && let Some(p) = self
                .board_book
                .lock()
                .expect("board book")
                .get(b)
                .and_then(|m| m.get(peer))
        {
            return Some(p.clone());
        }
        self.book.lock().expect("book").get(peer).cloned()
    }

    pub(crate) async fn bind(self: &Arc<Self>) -> Result<Endpoint, TransportError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(TransportError("mesh: transport closed".into()));
        }
        let me = self.clone();
        let built: Arc<Mutex<Option<Vec<RelaySpec>>>> = Arc::default();
        let b = built.clone();
        let ep = self
            .ep
            .get_or_try_init(|| async move {
                let (ep, relays) = me.build().await?;
                *b.lock().expect("built") = Some(relays);
                if me.cfg.gossip {
                    let g = iroh_gossip::net::Gossip::builder()
                        .alpn(GOSSIP_ALPN)
                        .spawn(ep.clone());
                    let _ = me.gossip.set(g);
                }
                tokio::spawn(accept_loop(Arc::downgrade(&me), ep.clone()));
                Ok(ep)
            })
            .await
            .cloned()?;
        let built = built.lock().expect("built").take();
        if let Some(built) = built {
            // A set_servers while the endpoint was being built found no
            // endpoint and only stored its relays: bring the endpoint's
            // relay map in line with them now that it is set (so a home
            // server that came up during the bind still becomes the home
            // relay).
            self.sync_relays(&ep, &built).await;
        }
        Ok(ep)
    }

    /// Inserts the relays `set_servers` holds that `had` lacks, and removes
    /// the ones it no longer holds.
    async fn sync_relays(&self, ep: &Endpoint, had: &[RelaySpec]) {
        let now = self.relays.lock().expect("relays").clone();
        for o in had {
            if !now.iter().any(|s| s.url == o.url) {
                ep.remove_relay(&o.url).await;
            }
        }
        for s in &now {
            if !had
                .iter()
                .any(|o| o.url == s.url && o.quic_port == s.quic_port)
            {
                ep.insert_relay(
                    s.url.clone(),
                    Arc::new(crate::relay::client_relay_config(
                        s.url.clone(),
                        s.quic_port,
                    )),
                )
                .await;
            }
        }
    }

    /// Builds the endpoint; also returns the relays its relay map starts
    /// with.
    async fn build(&self) -> Result<(Endpoint, Vec<RelaySpec>), TransportError> {
        let mut nr = NetReportConfig::default();
        nr.captive_portal_check = false;
        let tc = QuicTransportConfig::builder()
            .max_concurrent_bidi_streams(VarInt::from_u32(IN_STREAMS))
            .build();
        let mut alpns = vec![
            MESH_ALPN.to_vec(),
            PAIR_ALPN.to_vec(),
            crate::board::BOARD_ALPN.to_vec(),
            crate::board::INVITE_ALPN.to_vec(),
        ];
        if self.cfg.gossip {
            alpns.push(GOSSIP_ALPN.to_vec());
        }
        let mut b = iroh::endpoint::Builder::empty()
            .crypto_provider(crate::tls::provider())
            .secret_key(self.cfg.key.clone())
            .alpns(alpns)
            .portmapper_config(PortmapperConfig::Disabled)
            .net_report_config(nr)
            .transport_config(tc)
            .address_lookup(self.lookup.clone())
            .ca_tls_config(crate::tls::shared_ca_config(self.pins.clone()))
            .dns_resolver(crate::dns::LocalDns::resolver(self.cfg.dns_log.clone()))
            .hooks(Gate {
                admit: self.cfg.admit.clone(),
                pair_open: self.pair_open.clone(),
                invite_open: self.invite_open.clone(),
                invite_dials: self.invite_dials.clone(),
                guests: self.cfg.guests.clone(),
            })
            .clear_ip_transports();
        if let Some(lan) = self.start_lan() {
            b = b.address_lookup(lan);
        }
        let bind_ip = match self.cfg.ip {
            IpMode::RelayOnly => None,
            IpMode::Loopback => Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            IpMode::AnyV4 => Some(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
        };
        if let Some(ip) = bind_ip {
            b = b
                .bind_addr(SocketAddr::new(ip, 0))
                .map_err(|e| TransportError(format!("bind addr: {e}")))?;
        }
        let relays = self.relays.lock().expect("relays").clone();
        // Never RelayMode::Disabled: it drops the relay transport, so relays
        // learned later could not be inserted (mesh.md §8.1).
        let map = if relays.is_empty() {
            RelayMap::empty()
        } else {
            RelayMap::from_iter(
                relays
                    .iter()
                    .map(|r| crate::relay::client_relay_config(r.url.clone(), r.quic_port)),
            )
        };
        b = b.relay_mode(RelayMode::Custom(map));
        let ep = b
            .bind()
            .await
            .map_err(|e| TransportError(format!("Can't open the network: {e:#}")))?;
        Ok((ep, relays))
    }

    pub(crate) fn close_room(&self, room: &Arc<RoomState>, why: &str) {
        room.closed.store(true, Ordering::SeqCst);
        if let Some(b) = &room.board {
            let key = (b.clone(), room.secret.epoch);
            let mut rooms = self.board_rooms.lock().expect("board rooms");
            if rooms.get(&key).is_some_and(|r| Arc::ptr_eq(r, room)) {
                rooms.remove(&key);
            }
        } else {
            let mut rooms = self.rooms.lock().expect("rooms");
            if rooms
                .get(&room.secret.epoch)
                .is_some_and(|r| Arc::ptr_eq(r, room))
            {
                rooms.remove(&room.secret.epoch);
            }
        }
        let conns: Vec<_> = room
            .conns
            .lock()
            .expect("conns")
            .drain()
            .map(|(_, c)| c)
            .collect();
        for c in conns {
            c.close(why);
        }
    }

    pub(crate) fn connect(self: &Arc<Self>, room: &Arc<RoomState>, peer: &str) {
        if self.closed.load(Ordering::SeqCst)
            || room.closed.load(Ordering::SeqCst)
            || peer == self.cfg.self_id
            || peer.is_empty()
            || !(room.cfg.allow)(peer)
            || room.live(peer)
        {
            return;
        }
        let key = (
            room.board.clone().unwrap_or_default(),
            room.secret.epoch,
            peer.to_string(),
        );
        if !self.dialing.lock().expect("dialing").insert(key.clone()) {
            return;
        }
        let offers = room.cfg.offers_to(peer);
        let (me, room, peer) = (self.clone(), room.clone(), peer.to_string());
        tokio::spawn(async move {
            if !offers {
                tokio::time::sleep(me.cfg.dial_grace).await;
            }
            let res = if room.live(&peer) || room.closed.load(Ordering::SeqCst) {
                Ok(())
            } else {
                me.dial(&room, &peer).await
            };
            me.dialing.lock().expect("dialing").remove(&key);
            let mut errs = me.dial_errors.lock().expect("errs");
            match res {
                Ok(()) => {
                    errs.remove(&peer);
                }
                Err(e) => {
                    errs.insert(peer, e.to_string());
                }
            }
        });
    }

    async fn dial(self: &Arc<Self>, room: &Arc<RoomState>, peer: &str) -> Result<(), ConnError> {
        let addr = self
            .peer_addr(room, peer)
            .ok_or_else(|| ConnError::Transport(format!("no address for {peer}")))?;
        let ep = self.bind().await.map_err(|e| ConnError::Transport(e.0))?;
        let alpn = if room.board.is_some() {
            crate::board::BOARD_ALPN
        } else {
            MESH_ALPN
        };
        let conn = tokio::time::timeout(
            self.cfg.connect_timeout,
            ep.connect(addr.endpoint_addr(), alpn),
        )
        .await
        .map_err(|_| ConnError::Transport(reason::TIMEOUT.into()))?
        .map_err(|e| ConnError::Transport(format!("connect: {e:#}")))?;
        if let Some(board) = &room.board {
            return self.dial_board(conn, room, board, peer, &addr).await;
        }
        let epoch = room.secret.epoch;
        let k = ekm(&conn, EKM_MESH)?;
        let hs = async {
            let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
            write_frame(&mut send, SYNC_LABEL.as_bytes(), MAX_LABEL)
                .await
                .map_err(|e| e.to_string())?;
            let open = OpenMsg {
                v: 1,
                epoch,
                device_id: self.cfg.self_id.clone(),
                proof: proof(&room.secret.key, &k, ROLE_DIAL),
            };
            let b = serde_json::to_vec(&open).expect("open msg");
            write_frame(&mut send, &b, OPEN_MAX)
                .await
                .map_err(|e| e.to_string())?;
            let reply = read_frame(&mut recv, OPEN_MAX, false)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "no epoch-open reply".to_string())?;
            let reply: OpenMsg = serde_json::from_slice(&reply).map_err(|e| e.to_string())?;
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
        if reply.v != 1
            || reply.epoch != epoch
            || !proof_ok(&room.secret.key, &k, ROLE_ACCEPT, &reply.proof)
        {
            close_conn(&conn, reason::EPOCH);
            return Err(ConnError::Local(reason::EPOCH.into()));
        }
        if reply.device_id != peer || (room.cfg.accept)(peer, addr.id.as_bytes()).is_err() {
            close_conn(&conn, reason::REFUSED);
            return Err(ConnError::Local(reason::REFUSED.into()));
        }
        self.setup(
            conn,
            room,
            peer.to_string(),
            self.cfg.self_id.clone(),
            send,
            recv,
        )
    }

    async fn handle_mesh(self: &Arc<Self>, conn: Connection) {
        let hs = async {
            let (send, mut recv) = conn.accept_bi().await.map_err(|e| e.to_string())?;
            let label = read_frame(&mut recv, MAX_LABEL, false)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "no label".to_string())?;
            if label != SYNC_LABEL.as_bytes() {
                return Err("first stream is not sync".to_string());
            }
            let open = read_frame(&mut recv, OPEN_MAX, false)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "no epoch-open message".to_string())?;
            let open: OpenMsg = serde_json::from_slice(&open).map_err(|e| e.to_string())?;
            Ok::<_, String>((send, recv, open))
        };
        let (mut send, recv, open) = match tokio::time::timeout(self.cfg.open_timeout, hs).await {
            Ok(Ok(x)) => x,
            Ok(Err(_)) => return close_conn(&conn, reason::PROTOCOL),
            Err(_) => return close_conn(&conn, reason::TIMEOUT),
        };
        if open.v != 1 {
            return close_conn(&conn, reason::VERSION);
        }
        let room = self.rooms.lock().expect("rooms").get(&open.epoch).cloned();
        let Some(room) = room else {
            return close_conn(&conn, reason::EPOCH);
        };
        let Ok(k) = ekm(&conn, EKM_MESH) else {
            return close_conn(&conn, reason::PROTOCOL);
        };
        if !proof_ok(&room.secret.key, &k, ROLE_DIAL, &open.proof) {
            return close_conn(&conn, reason::EPOCH);
        }
        let peer = open.device_id;
        let pubkey = *conn.remote_id().as_bytes();
        if peer == self.cfg.self_id
            || (room.cfg.accept)(&peer, &pubkey).is_err()
            || !(room.cfg.allow)(&peer)
        {
            return close_conn(&conn, reason::REFUSED);
        }
        let reply = OpenMsg {
            v: 1,
            epoch: open.epoch,
            device_id: self.cfg.self_id.clone(),
            proof: proof(&room.secret.key, &k, ROLE_ACCEPT),
        };
        let b = serde_json::to_vec(&reply).expect("open msg");
        if write_frame(&mut send, &b, OPEN_MAX).await.is_err() {
            return close_conn(&conn, reason::PROTOCOL);
        }
        let _ = self.setup(conn, &room, peer.clone(), peer, send, recv);
    }

    pub(crate) fn setup(
        self: &Arc<Self>,
        conn: Connection,
        room: &Arc<RoomState>,
        peer: String,
        dialer: String,
        send: SendStream,
        recv: RecvStream,
    ) -> Result<(), ConnError> {
        let sas = ekm(&conn, EKM_SAS)?;
        let (handler, _) = watch::channel(None);
        let c = Arc::new(IrohConn {
            pubkey: *conn.remote_id().as_bytes(),
            conn,
            peer: peer.clone(),
            epoch: room.secret.epoch,
            dialer,
            send: tokio::sync::Mutex::new(send),
            recv: tokio::sync::Mutex::new(recv),
            err: Mutex::new(None),
            done: CancellationToken::new(),
            handler,
            out: Arc::new(Semaphore::new(OUT_STREAMS)),
            fp: colon_hex(&sas),
        });
        {
            let mut conns = room.conns.lock().expect("conns");
            if room.closed.load(Ordering::SeqCst) {
                c.close(reason::LEAVING);
                return Err(ConnError::Local(reason::LEAVING.into()));
            }
            if let Some(old) = conns.get(&peer)
                && !old.done.is_cancelled()
            {
                let lower = std::cmp::min(self.cfg.self_id.as_str(), peer.as_str());
                let keep_new = if c.dialer == old.dialer {
                    true
                } else {
                    c.dialer == lower
                };
                if keep_new {
                    old.close(reason::REPLACED);
                } else {
                    c.close(reason::REPLACED);
                    return Err(ConnError::Local(reason::REPLACED.into()));
                }
            }
            conns.insert(peer.clone(), c.clone());
        }
        tokio::spawn(monitor(c.clone(), Arc::downgrade(room)));
        tokio::spawn(accept_streams(c.clone()));
        (room.cfg.on_msg)(Envelope {
            from: peer.clone(),
            typ: MSG_PRESENCE.into(),
            body: serde_json::json!({"deviceId": peer, "epoch": room.secret.epoch}),
        });
        (room.cfg.on_peer)(c);
        Ok(())
    }
}

async fn accept_loop(inner: Weak<Inner>, ep: Endpoint) {
    while let Some(incoming) = ep.accept().await {
        let Some(inner) = inner.upgrade() else {
            break;
        };
        tokio::spawn(async move {
            let Ok(accepting) = incoming.accept() else {
                return;
            };
            let Ok(conn) = accepting.await else {
                return;
            };
            if conn.alpn() == MESH_ALPN {
                inner.handle_mesh(conn).await;
            } else if conn.alpn() == crate::board::BOARD_ALPN {
                inner.handle_board(conn).await;
            } else if conn.alpn() == PAIR_ALPN {
                inner.handle_pair(conn).await;
            } else if conn.alpn() == crate::board::INVITE_ALPN {
                inner.handle_invite(conn).await;
            } else if conn.alpn() == GOSSIP_ALPN
                && let Some(g) = inner.gossip.get()
            {
                let _ = g.handle_connection(conn).await;
            } else {
                close_conn(&conn, reason::PROTOCOL);
            }
        });
    }
}

async fn monitor(c: Arc<IrohConn>, room: Weak<RoomState>) {
    let e = c.conn.closed().await;
    c.set_err(conn_error(&e));
    c.done.cancel();
    if let Some(room) = room.upgrade() {
        let mut conns = room.conns.lock().expect("conns");
        if conns.get(&c.peer).is_some_and(|x| Arc::ptr_eq(x, &c)) {
            conns.remove(&c.peer);
        }
    }
}

async fn accept_streams(c: Arc<IrohConn>) {
    loop {
        let next = tokio::select! {
            r = c.conn.accept_bi() => r,
            _ = c.done.cancelled() => return,
        };
        let Ok((send, mut recv)) = next else {
            return;
        };
        let c = c.clone();
        tokio::spawn(async move {
            let Ok(Some(label)) = read_frame(&mut recv, MAX_LABEL, false).await else {
                return;
            };
            let Ok(label) = String::from_utf8(label) else {
                return;
            };
            let mut rx = c.handler.subscribe();
            let h = tokio::select! {
                h = rx.wait_for(|h| h.is_some()) => match h {
                    Ok(h) => h.clone(),
                    Err(_) => return,
                },
                _ = c.done.cancelled() => return,
            };
            if let Some(h) = h {
                h(Stream {
                    label,
                    send: Box::new(send),
                    recv: Box::new(recv),
                });
            }
        });
    }
}

/// One connection to a device in one room.
pub struct IrohConn {
    conn: Connection,
    peer: String,
    pubkey: [u8; 32],
    epoch: i64,
    dialer: String,
    send: tokio::sync::Mutex<SendStream>,
    recv: tokio::sync::Mutex<RecvStream>,
    err: Mutex<Option<ConnError>>,
    done: CancellationToken,
    handler: watch::Sender<Option<StreamHandler>>,
    out: Arc<Semaphore>,
    fp: String,
}

impl IrohConn {
    fn set_err(&self, e: ConnError) {
        let mut g = self.err.lock().expect("err");
        if g.is_none() {
            *g = Some(e);
        }
    }

    fn current_err(&self, fallback: &str) -> ConnError {
        self.err
            .lock()
            .expect("err")
            .clone()
            .or_else(|| self.conn.close_reason().map(|e| conn_error(&e)))
            .unwrap_or_else(|| ConnError::Transport(fallback.to_string()))
    }

    /// Which side dialed this connection.
    pub fn dialer(&self) -> &str {
        &self.dialer
    }

    /// The iroh connection (path events, stats).
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

impl Conn for IrohConn {
    fn id(&self) -> &str {
        &self.peer
    }

    fn pub_key(&self) -> [u8; 32] {
        self.pubkey
    }

    fn epoch(&self) -> i64 {
        self.epoch
    }

    fn send<'a>(&'a self, msg: &'a [u8]) -> BoxFut<'a, Result<(), ConnError>> {
        Box::pin(async move {
            if msg.len() > MAX_MESSAGE {
                return Err(ConnError::Local(format!(
                    "{}: message of {} bytes is over the limit",
                    reason::PROTOCOL,
                    msg.len()
                )));
            }
            if self.done.is_cancelled() {
                return Err(self.current_err("closed"));
            }
            let mut s = self.send.lock().await;
            match write_frame(&mut *s, msg, MAX_MESSAGE).await {
                Ok(()) => Ok(()),
                Err(e) => Err(self.current_err(&e.to_string())),
            }
        })
    }

    fn recv(&self) -> BoxFut<'_, Result<Vec<u8>, ConnError>> {
        Box::pin(async move {
            let mut r = self.recv.lock().await;
            let res = tokio::select! {
                r = read_frame(&mut *r, MAX_MESSAGE, true) => r,
                _ = self.done.cancelled() => return Err(self.current_err("closed")),
            };
            match res {
                Ok(Some(m)) => Ok(m),
                Ok(None) => {
                    // The peer ended its sync stream: the connection is over.
                    self.close(reason::PROTOCOL);
                    Err(self.current_err("sync stream ended"))
                }
                Err(irori_meshseam::FrameError::Io(e)) => Err(self.current_err(&e.to_string())),
                Err(_) => {
                    // Over 8 MiB or cut inside a message (mesh.md §2.2).
                    self.close(reason::PROTOCOL);
                    Err(self.current_err(reason::PROTOCOL))
                }
            }
        })
    }

    fn open_stream<'a>(&'a self, label: &'a str) -> BoxFut<'a, Result<Stream, ConnError>> {
        Box::pin(async move {
            if label.is_empty() || label.len() > MAX_LABEL || label == SYNC_LABEL {
                return Err(ConnError::Local(format!("bad stream label {label:?}")));
            }
            let permit = tokio::select! {
                p = self.out.clone().acquire_owned() => p.map_err(|_| self.current_err("closed"))?,
                _ = self.done.cancelled() => return Err(self.current_err("closed")),
            };
            let (mut send, recv) = self
                .conn
                .open_bi()
                .await
                .map_err(|e| self.current_err(&e.to_string()))?;
            write_frame(&mut send, label.as_bytes(), MAX_LABEL)
                .await
                .map_err(|e| self.current_err(&e.to_string()))?;
            Ok(Stream {
                label: label.to_string(),
                send: Box::new(PermitSend {
                    s: send,
                    _permit: permit,
                }),
                recv: Box::new(recv),
            })
        })
    }

    fn on_stream(&self, h: StreamHandler) {
        self.handler.send_replace(Some(h));
    }

    fn done(&self) -> BoxFut<'static, ()> {
        let t = self.done.clone();
        Box::pin(async move { t.cancelled().await })
    }

    fn err(&self) -> Option<ConnError> {
        if !self.done.is_cancelled() {
            return None;
        }
        Some(self.current_err("closed"))
    }

    fn close(&self, why: &str) {
        self.set_err(ConnError::Local(why.to_string()));
        close_conn(&self.conn, why);
        self.done.cancel();
    }

    fn path(&self) -> String {
        crate::path::report(&self.conn).kind.to_string()
    }

    fn rtt(&self) -> Duration {
        crate::path::report(&self.conn).rtt.unwrap_or_default()
    }

    fn fingerprints(&self) -> (String, String) {
        (self.fp.clone(), self.fp.clone())
    }
}

/// An outgoing stream's send half, holding one of the 4 outgoing slots.
struct PermitSend {
    s: SendStream,
    _permit: OwnedSemaphorePermit,
}

impl AsyncWrite for PermitSend {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(&mut self.s), cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.s), cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.s), cx)
    }
}

/// One room on the iroh transport.
pub(crate) struct IrohRoom {
    pub(crate) net: Weak<Inner>,
    pub(crate) state: Arc<RoomState>,
}

impl Room for IrohRoom {
    fn send(&self, to: &str, typ: &str, _body: serde_json::Value) -> Result<(), TransportError> {
        let Some(net) = self.net.upgrade() else {
            return Err(TransportError("mesh: transport closed".into()));
        };
        // No signaling bus on iroh (mesh.md §5.3): presence is a dial of
        // every allowed peer with no connection; `leave` is the `leaving`
        // close that `close` sends.
        if typ == MSG_PRESENCE {
            let peers: Vec<String> = if !to.is_empty() {
                vec![to.to_string()]
            } else if let Some(b) = &self.state.board {
                net.board_book
                    .lock()
                    .expect("board book")
                    .get(b)
                    .map(|m| m.keys().cloned().collect())
                    .unwrap_or_default()
            } else {
                net.book.lock().expect("book").keys().cloned().collect()
            };
            for p in peers {
                net.connect(&self.state, &p);
            }
        }
        Ok(())
    }

    fn connect(&self, peer: &str) {
        if let Some(net) = self.net.upgrade() {
            net.connect(&self.state, peer);
        }
    }

    fn close(&self) {
        if let Some(net) = self.net.upgrade() {
            net.close_room(&self.state, reason::LEAVING);
        }
    }
}

impl Transport for IrohNet {
    fn join(
        &self,
        s: Secret,
        cfg: RoomConfig,
    ) -> BoxFut<'_, Result<Arc<dyn Room>, TransportError>> {
        Box::pin(async move {
            if s.epoch == 0 {
                // The pairing room (M7, crate::pairnet).
                return self.inner.join_pair(s, cfg).await;
            }
            self.inner.bind().await?;
            let state = Arc::new(RoomState::new(s, cfg));
            let old = self
                .inner
                .rooms
                .lock()
                .expect("rooms")
                .insert(state.secret.epoch, state.clone());
            if let Some(old) = old {
                // The epoch's topic changed (ReplaceSecret): its old room goes.
                self.inner.close_room(&old, reason::LEAVING);
            }
            let on_connect = state.cfg.on_connect.clone();
            tokio::spawn(async move { on_connect() });
            Ok(Arc::new(IrohRoom {
                net: Arc::downgrade(&self.inner),
                state,
            }) as Arc<dyn Room>)
        })
    }
}

/// Close the stream's send half after writing all of `b` (the blob/board
/// reply shape; a refusal is `finish_with(&[])`).
pub async fn finish_with(s: &mut Stream, b: &[u8]) -> std::io::Result<()> {
    s.send.write_all(b).await?;
    s.send.shutdown().await
}
