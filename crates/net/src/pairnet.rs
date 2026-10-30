//! The pairing room on iroh (M7, architecture §3.4 D5, mesh.md §5.4): the
//! epoch-0 room of `irori_meshseam`, on ALPN `irori/pair/1`.
//!
//! go-room carried pairing on a signaling bus: B broadcast `pair.hello`,
//! A pinned the first joiner and offered a connection. iroh has no bus, and
//! B knows neither A's address nor its id, so the room is turned round:
//!
//! - **Finding A.** A's room is a LAN lookup room under the code-derived
//!   key ([`PairLan`]), so A announces its EndpointAddr sealed with it; B,
//!   in the same room, hears A ([`irori_lan::Config::on_room`]). Away from
//!   home B types the home server's `host:port`: it reads the server's
//!   device key from its relay certificate (the relay TLS is on the device
//!   key, D3; trust on first use, the words and typed key decide), joins the
//!   relay as a guest with the window's token ([`crate::guest`]), and asks
//!   the server for A. **The bridge**: a server holding a member's window
//!   announces that member's room on its LAN, naming its own relay and key
//!   (an [`irori_lan::Proxy`]), so B at home with only the code reaches an
//!   away A through it.
//! - **Hello.** `Room::send` of a hello is a dial of every candidate on
//!   `irori/pair/1`. The first stream (label `pair`) carries
//!   `{"v":1,"t":"hello","deviceId","typ","body","tag","proof"}`, `proof` =
//!   hex(HMAC-SHA256(room key, "irori/pair-open/1" || EKM32 || role)), EKM =
//!   `EXPORTER-irori-pair`, role 0 from the dialer and 1 from the acceptor.
//!   The acceptor hands the hello to `on_msg` and holds the connection.
//! - **Offer.** `Room::connect(peer)` on the acceptor takes up a held
//!   connection: `allow` and `accept` run, then `{"v":1,"t":"ok","deviceId",
//!   "proof"}` goes back and both sides report the connection to `on_peer`.
//!   `Room::send(peer, typ)` to a held connection closes it with `typ` as the
//!   reason (`pair.busy`), which the dialer reports to its `on_msg`.
//! - **Guest windows.** A room joined with `guest_ttl` opens a window on
//!   this device's relay and, over `{"t":"guest","topic","key","ttlMs"}`, on
//!   every home server this device knows; the server keeps it while that
//!   connection lives.
//!
//! The endpoint gate admits unknown ids on `irori/pair/1` only while a
//! pairing room is open here or a guest window is (`irori/mesh/1` never).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use iroh::endpoint::{Connection, RecvStream, SendStream, VarInt};
use iroh::{EndpointAddr, EndpointId, RelayUrl};
use irori_meshseam::{
    ConnError, Envelope, Room, RoomConfig, Secret, TransportError, read_frame, reason, write_frame,
};
use serde::{Deserialize, Serialize};

use crate::guest::{GuestWindows, guest_token, room_tag};
use crate::transport::{self, Inner, PAIR_ALPN, RoomState};

const OPEN_LABEL: &[u8] = b"irori/pair-open/1";
const EKM_PAIR: &[u8] = b"EXPORTER-irori-pair";
const STREAM_LABEL: &[u8] = b"pair";
const ROLE_DIAL: u8 = 0;
const ROLE_ACCEPT: u8 = 1;
const MSG_MAX: usize = 64 << 10;
/// How long an acceptor holds a hello for `connect` (A takes it up at once).
const HOLD: Duration = Duration::from_secs(30);
/// How long a dialer waits for the acceptor's answer.
const ANSWER: Duration = Duration::from_secs(40);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// No Debug: a guest message carries the room key.
#[derive(Default, Serialize, Deserialize)]
struct OpenMsg {
    v: u32,
    t: String,
    #[serde(rename = "deviceId", default, skip_serializing_if = "String::is_empty")]
    device_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    typ: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    body: serde_json::Value,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    tag: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    proof: String,
    // guest
    #[serde(default, skip_serializing_if = "String::is_empty")]
    topic: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    key: String,
    #[serde(rename = "ttlMs", default, skip_serializing_if = "is_zero")]
    ttl_ms: u64,
    /// `invite` for a board invite window (card #44); empty for pairing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    kind: String,
    // at: where the room's owner is
    #[serde(default, skip_serializing_if = "String::is_empty")]
    id: String,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

// The open proof and the exporter are the mesh transport's, under the
// pairing room's labels.
fn proof(key: &[u8; 32], ekm: &[u8; 32], role: u8) -> String {
    transport::open_proof(OPEN_LABEL, key, ekm, role)
}

fn proof_ok(key: &[u8; 32], ekm: &[u8; 32], role: u8, got: &str) -> bool {
    transport::open_proof_ok(OPEN_LABEL, key, ekm, role, got)
}

fn ekm(conn: &Connection) -> Result<[u8; 32], ConnError> {
    transport::ekm(conn, EKM_PAIR)
}

fn close(conn: &Connection, why: &str) {
    conn.close(VarInt::from_u32(reason::code(why)), why.as_bytes());
}

async fn write_msg(s: &mut SendStream, m: &OpenMsg) -> Result<(), String> {
    let b = serde_json::to_vec(m).map_err(|e| e.to_string())?;
    write_frame(s, &b, MSG_MAX).await.map_err(|e| e.to_string())
}

async fn read_msg(r: &mut RecvStream) -> Result<OpenMsg, String> {
    let b = read_frame(r, MSG_MAX, false)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("stream ended")?;
    serde_json::from_slice(&b).map_err(|e| e.to_string())
}

/// Where the room's other side may be.
#[derive(Debug, Clone)]
struct Cand {
    id: EndpointId,
    relay: Option<RelayUrl>,
    direct: Vec<SocketAddr>,
}

impl Cand {
    fn addr(&self) -> EndpointAddr {
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

/// A hello held for `connect`.
struct Held {
    conn: Connection,
    send: SendStream,
    recv: RecvStream,
}

/// This device's open pairing room.
pub(crate) struct PairRoom {
    pub(crate) state: Arc<RoomState>,
    held: Mutex<HashMap<String, Held>>,
    cands: Mutex<BTreeMap<EndpointId, Cand>>,
    dialing: Mutex<HashSet<EndpointId>>,
    /// The last hello sent, redialed to candidates heard later.
    hello: Mutex<Option<(String, serde_json::Value)>>,
    window: Mutex<Option<u64>>,
    regs: Mutex<Vec<Connection>>,
    /// Relays joined as a guest and server keys pinned for this pairing:
    /// removed when it closes, unless the mesh's address book names them
    /// (a paired device reaches its servers again through `Servers` hints).
    guest: Mutex<Vec<(RelayUrl, [u8; 32])>>,
    closed: AtomicBool,
}

impl PairRoom {
    fn key(&self) -> &[u8; 32] {
        &self.state.secret.key
    }

    fn cfg(&self) -> &RoomConfig {
        &self.state.cfg
    }

    fn add_cand(&self, c: Cand) -> bool {
        let mut g = lock(&self.cands);
        let fresh = !g.contains_key(&c.id);
        g.insert(c.id, c);
        fresh
    }
}

type SinkFn = Arc<dyn Fn(EndpointId, &irori_lan::Heard) + Send + Sync>;

/// The LAN side of pairing, shared between the LAN lookup (created first)
/// and the transport: the open pairing room as a LAN room, what was heard
/// in it, and the bridge's announcements for guest windows.
#[derive(Clone, Default)]
pub struct PairLan {
    st: Arc<Mutex<LanState>>,
    guests: GuestWindows,
}

#[derive(Default)]
struct LanState {
    room: Option<irori_lan::Room>,
    sink: Option<SinkFn>,
    /// This server's relay as the bridge names it, and its key (hex).
    relay: Option<(RelayUrl, String)>,
    lan: Option<irori_lan::LanLookup>,
}

impl PairLan {
    /// A PairLan whose bridge announces `guests`' windows.
    pub fn new(guests: GuestWindows) -> PairLan {
        PairLan {
            st: Arc::default(),
            guests,
        }
    }

    /// Hooks pairing into a LAN lookup config before it starts: the open
    /// pairing room is added to `rooms`, what is heard in it reaches the
    /// room, and guest windows are announced for their owners.
    pub fn wire(&self, cfg: &mut irori_lan::Config) {
        let base = cfg.rooms.clone();
        let st = self.st.clone();
        cfg.rooms = Arc::new(move || {
            let mut v = base();
            if let Some(r) = lock(&st).room.clone() {
                v.push(r);
            }
            v
        });
        let st = self.st.clone();
        cfg.on_room = Some(Arc::new(move |topic, id, h| {
            let sink = {
                let g = lock(&st);
                match &g.room {
                    Some(r) if r.topic == topic => g.sink.clone(),
                    _ => None,
                }
            };
            if let Some(f) = sink {
                f(id, h);
            }
        }));
        let st = self.st.clone();
        let guests = self.guests.clone();
        cfg.proxies = Some(Arc::new(move || {
            let Some((url, key)) = lock(&st).relay.clone() else {
                return vec![];
            };
            guests
                .open()
                .into_iter()
                .map(|w| irori_lan::Proxy {
                    room: irori_lan::Room {
                        topic: w.secret.topic.clone(),
                        key: w.secret.key,
                    },
                    id: w.owner,
                    relay: Some(url.clone()),
                    relay_key: Some(key.clone()),
                })
                .collect()
        }));
    }

    /// The lookup the config was started as, so a room opening announces
    /// at once.
    pub fn attach(&self, lan: irori_lan::LanLookup) {
        lock(&self.st).lan = Some(lan);
    }

    /// This server's relay as the bridge names it (`None` when it serves
    /// none): its LAN URL and its device key.
    pub fn set_relay(&self, relay: Option<(RelayUrl, EndpointId)>) {
        lock(&self.st).relay = relay.map(|(u, k)| (u, hex::encode(k.as_bytes())));
        self.kick();
    }

    /// The relay [`PairLan::set_relay`] last named: its URL and its key
    /// (hex).
    pub fn relay(&self) -> Option<(RelayUrl, String)> {
        lock(&self.st).relay.clone()
    }

    fn kick(&self) {
        let lan = lock(&self.st).lan.clone();
        if let Some(l) = lan {
            l.announce_now();
        }
    }

    fn open(&self, s: &Secret, sink: SinkFn) {
        {
            let mut g = lock(&self.st);
            g.room = Some(irori_lan::Room {
                topic: s.topic.clone(),
                key: s.key,
            });
            g.sink = Some(sink);
        }
        self.kick();
    }

    fn close(&self, topic: &str) {
        let mut g = lock(&self.st);
        if g.room.as_ref().is_some_and(|r| r.topic == topic) {
            g.room = None;
            g.sink = None;
        }
    }
}

/// Go `HostPort`-style `host:port` to a socket address (IP literals, or a
/// name resolved once).
async fn resolve(hub: &str) -> Result<SocketAddr, String> {
    if let Ok(a) = hub.parse::<SocketAddr>() {
        return Ok(a);
    }
    tokio::net::lookup_host(hub)
        .await
        .map_err(|e| format!("{hub}: {e}"))?
        .next()
        .ok_or_else(|| format!("{hub}: no address"))
}

fn relay_url_for(a: SocketAddr) -> RelayUrl {
    crate::relay::relay_url(a.ip(), a.port())
}

impl Inner {
    /// `Transport::join` of an epoch-0 room.
    pub(crate) async fn join_pair(
        self: &Arc<Self>,
        s: Secret,
        cfg: RoomConfig,
    ) -> Result<Arc<dyn Room>, TransportError> {
        self.bind().await?;
        let ttl = cfg.guest_ttl;
        let hub = cfg.hub_addr.clone();
        let room = Arc::new(PairRoom {
            state: Arc::new(RoomState::new(s.clone(), cfg)),
            held: Mutex::default(),
            cands: Mutex::default(),
            dialing: Mutex::default(),
            hello: Mutex::default(),
            window: Mutex::default(),
            regs: Mutex::default(),
            guest: Mutex::default(),
            closed: AtomicBool::new(false),
        });
        let old = lock(&self.pair).replace(room.clone());
        if let Some(old) = old {
            self.close_pair(&old);
        }
        self.pair_open.store(true, Ordering::SeqCst);
        let me = Arc::downgrade(self);
        let wr = Arc::downgrade(&room);
        self.cfg.pair_lan.open(
            &s,
            Arc::new(move |id, h| {
                let (Some(net), Some(room)) = (me.upgrade(), wr.upgrade()) else {
                    return;
                };
                net.heard_in_pair(&room, id, h);
            }),
        );
        if let Some(ttl) = ttl {
            // This device's own relay, and every home server it knows.
            *lock(&room.window) =
                self.cfg
                    .guests
                    .open_window(self.cfg.key.public(), s.clone(), ttl);
            self.cfg.pair_lan.kick();
            let servers = lock(&self.relays).clone();
            for sv in servers {
                if sv.server_pub == self.cfg.key.public() {
                    continue;
                }
                let (net, room, s) = (self.clone(), room.clone(), s.clone());
                tokio::spawn(async move { net.register_guest(&room, sv, &s, ttl).await });
            }
        }
        if let Some(hub) = hub.filter(|h| !h.is_empty()) {
            let (net, room) = (self.clone(), room.clone());
            tokio::spawn(async move {
                if let Err(e) = net.via_hub(&room, &hub).await {
                    let mut errs = lock(&net.dial_errors);
                    errs.insert(hub, e);
                }
            });
        }
        let on_connect = room.cfg().on_connect.clone();
        tokio::spawn(async move { on_connect() });
        Ok(Arc::new(IrohPairRoom {
            net: Arc::downgrade(self),
            room,
        }) as Arc<dyn Room>)
    }

    pub(crate) fn close_pair(&self, room: &Arc<PairRoom>) {
        if room.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        {
            let mut g = lock(&self.pair);
            if g.as_ref().is_some_and(|r| Arc::ptr_eq(r, room)) {
                *g = None;
                self.pair_open.store(false, Ordering::SeqCst);
            }
        }
        self.cfg.pair_lan.close(&room.state.secret.topic);
        if let Some(w) = lock(&room.window).take() {
            self.cfg.guests.close_window(w);
        }
        for c in lock(&room.regs).drain(..) {
            close(&c, reason::LEAVING);
        }
        for (_, h) in lock(&room.held).drain() {
            close(&h.conn, reason::LEAVING);
        }
        self.close_room(&room.state, reason::LEAVING);
        self.drop_guest_relays(room);
    }

    /// Takes out what this pairing added to the endpoint: its guest relays
    /// and its trust-on-first-use pins, except those the mesh's own relays
    /// (`set_servers`) still name.
    fn drop_guest_relays(&self, room: &Arc<PairRoom>) {
        let added: Vec<(RelayUrl, [u8; 32])> = lock(&room.guest).drain(..).collect();
        if added.is_empty() {
            return;
        }
        let mesh = lock(&self.relays).clone();
        let mut gone = Vec::new();
        for (url, key) in added {
            if !mesh.iter().any(|r| r.server_pub.as_bytes() == &key) {
                self.pins.remove(&key);
            }
            if !mesh.iter().any(|r| r.url == url) && !gone.contains(&url) {
                gone.push(url);
            }
        }
        // The mesh's book may be feeding the same server in right now (the
        // Pairer's hint to `Servers` races this close): look again at the
        // moment of removal, so a relay `set_servers` just inserted stays.
        if let Some(ep) = self.ep.get().cloned() {
            let relays = self.relays.clone();
            tokio::spawn(async move {
                for url in gone {
                    if lock(&relays).iter().any(|r| r.url == url) {
                        continue;
                    }
                    ep.remove_relay(&url).await;
                }
            });
        }
    }

    fn heard_in_pair(self: &Arc<Self>, room: &Arc<PairRoom>, id: EndpointId, h: &irori_lan::Heard) {
        if id == self.cfg.key.public() {
            return;
        }
        if let (Some(r), Some(k)) = (&h.relay, &h.relay_key) {
            // The bridge: another device's room, carried by a home server.
            if let Ok(kb) = hex::decode(k)
                && let Ok(kb) = <[u8; 32]>::try_from(kb.as_slice())
            {
                self.pins.add(kb);
                lock(&room.guest).push((r.clone(), kb));
                let (net, url, key) = (self.clone(), r.clone(), room.state.secret.key);
                tokio::spawn(async move { net.insert_guest_relay(url, &key).await });
            }
        }
        let c = Cand {
            id,
            relay: h.relay.clone(),
            direct: h.addrs.clone(),
        };
        self.lookup.set_endpoint_info(c.addr());
        if room.add_cand(c.clone()) {
            let hello = lock(&room.hello).clone();
            if let Some((typ, body)) = hello {
                self.dial_pair(room, c, typ, body);
            }
        }
    }

    async fn insert_guest_relay(self: &Arc<Self>, url: RelayUrl, key: &[u8; 32]) {
        let Ok(ep) = self.bind().await else { return };
        let port = url.port().unwrap_or(443);
        let cfg =
            crate::relay::client_relay_config(url.clone(), port).with_auth_token(guest_token(key));
        ep.insert_relay(url, Arc::new(cfg)).await;
    }

    /// B away from home with a server's `host:port`: its key from the relay
    /// certificate, the relay joined as a guest, and the server itself the
    /// first candidate (it answers for the member whose window it holds).
    async fn via_hub(self: &Arc<Self>, room: &Arc<PairRoom>, hub: &str) -> Result<(), String> {
        let addr = resolve(hub).await?;
        let key = crate::tls::probe_server_key(addr).await?;
        self.pins.add(key);
        let url = relay_url_for(addr);
        lock(&room.guest).push((url.clone(), key));
        self.insert_guest_relay(url.clone(), room.key()).await;
        let id = EndpointId::from_bytes(&key).map_err(|e| e.to_string())?;
        let c = Cand {
            id,
            relay: Some(url),
            direct: vec![],
        };
        self.lookup.set_endpoint_info(c.addr());
        if room.add_cand(c.clone()) {
            let hello = lock(&room.hello).clone();
            if let Some((typ, body)) = hello {
                self.dial_pair(room, c, typ, body);
            }
        }
        Ok(())
    }

    /// A member opens its window on a home server.
    async fn register_guest(
        self: &Arc<Self>,
        room: &Arc<PairRoom>,
        sv: crate::endpoint::RelaySpec,
        s: &Secret,
        ttl: Duration,
    ) {
        let Some((conn, streams)) = self.open_guest_on(sv, s, ttl, "").await else {
            return;
        };
        if room.closed.load(Ordering::SeqCst) {
            close(&conn, reason::LEAVING);
            return;
        }
        lock(&room.regs).push(conn.clone());
        // Held for as long as the window: the server closes it when this
        // connection goes.
        let _keep = streams;
        conn.closed().await;
    }

    /// Opens a guest window of `kind` (`""` pairing, `"invite"`) on the home
    /// server `sv`. The window lives while the returned connection (and its
    /// streams) does.
    pub(crate) async fn open_guest_on(
        self: &Arc<Self>,
        sv: crate::endpoint::RelaySpec,
        s: &Secret,
        ttl: Duration,
        kind: &str,
    ) -> Option<(Connection, (SendStream, RecvStream))> {
        let ep = self.bind().await.ok()?;
        let addr = EndpointAddr::new(sv.server_pub).with_relay_url(sv.url.clone());
        let Ok(Ok(conn)) =
            tokio::time::timeout(self.cfg.connect_timeout, ep.connect(addr, PAIR_ALPN)).await
        else {
            return None;
        };
        let res = async {
            let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
            write_frame(&mut send, STREAM_LABEL, 255)
                .await
                .map_err(|e| e.to_string())?;
            write_msg(
                &mut send,
                &OpenMsg {
                    v: 1,
                    t: "guest".into(),
                    device_id: self.cfg.self_id.clone(),
                    topic: s.topic.clone(),
                    key: hex::encode(s.key),
                    ttl_ms: ttl.as_millis() as u64,
                    kind: kind.into(),
                    ..Default::default()
                },
            )
            .await?;
            let r = read_msg(&mut recv).await?;
            if r.t != "ok" {
                return Err("refused".to_string());
            }
            Ok::<_, String>((send, recv))
        };
        match tokio::time::timeout(self.cfg.open_timeout, res).await {
            Ok(Ok(streams)) => Some((conn, streams)),
            _ => {
                close(&conn, reason::PROTOCOL);
                None
            }
        }
    }

    /// Dials one candidate with the hello.
    fn dial_pair(
        self: &Arc<Self>,
        room: &Arc<PairRoom>,
        c: Cand,
        typ: String,
        body: serde_json::Value,
    ) {
        if room.closed.load(Ordering::SeqCst) || !lock(&room.dialing).insert(c.id) {
            return;
        }
        let (me, room) = (self.clone(), room.clone());
        tokio::spawn(async move {
            let res = me.dial_pair_once(&room, &c, &typ, &body, 0).await;
            lock(&room.dialing).remove(&c.id);
            if let Err(e) = res {
                lock(&me.dial_errors).insert(c.id.to_string(), e.to_string());
            }
        });
    }

    async fn dial_pair_once(
        self: &Arc<Self>,
        room: &Arc<PairRoom>,
        c: &Cand,
        typ: &str,
        body: &serde_json::Value,
        depth: u8,
    ) -> Result<(), ConnError> {
        let ep = self.bind().await.map_err(|e| ConnError::Transport(e.0))?;
        let conn = tokio::time::timeout(self.cfg.connect_timeout, ep.connect(c.addr(), PAIR_ALPN))
            .await
            .map_err(|_| ConnError::Transport(reason::TIMEOUT.into()))?
            .map_err(|e| ConnError::Transport(format!("connect: {e:#}")))?;
        let k = ekm(&conn)?;
        let (mut send, mut recv) = conn
            .open_bi()
            .await
            .map_err(|e| ConnError::Transport(e.to_string()))?;
        let hello = OpenMsg {
            v: 1,
            t: "hello".into(),
            device_id: self.cfg.self_id.clone(),
            typ: typ.into(),
            body: body.clone(),
            tag: room_tag(&room.state.secret.topic),
            proof: proof(room.key(), &k, ROLE_DIAL),
            ..Default::default()
        };
        let sent = async {
            write_frame(&mut send, STREAM_LABEL, 255)
                .await
                .map_err(|e| e.to_string())?;
            write_msg(&mut send, &hello).await
        };
        if let Err(e) = sent.await {
            close(&conn, reason::PROTOCOL);
            return Err(ConnError::Transport(e));
        }
        let reply = tokio::select! {
            r = tokio::time::timeout(ANSWER, read_msg(&mut recv)) => r,
            _ = wait_closed(&room.closed) => {
                close(&conn, reason::LEAVING);
                return Ok(());
            }
        };
        let reply = match reply {
            Err(_) => {
                close(&conn, reason::TIMEOUT);
                return Err(ConnError::Local(reason::TIMEOUT.into()));
            }
            Ok(Err(e)) => {
                // Closed before an answer: a busy room says so in the reason.
                if let Some(iroh::endpoint::ConnectionError::ApplicationClosed(a)) =
                    conn.close_reason()
                {
                    let why = String::from_utf8_lossy(&a.reason).into_owned();
                    if why.starts_with("pair.") {
                        (room.cfg().on_msg)(Envelope {
                            from: c.id.to_string(),
                            typ: why.clone(),
                            body: serde_json::json!({}),
                        });
                    }
                    return Err(ConnError::Remote(why));
                }
                return Err(ConnError::Transport(e));
            }
            Ok(Ok(r)) => r,
        };
        match reply.t.as_str() {
            "at" if depth == 0 => {
                // A server answering for the member whose window it holds.
                close(&conn, reason::LEAVING);
                let id: EndpointId = reply
                    .id
                    .parse()
                    .map_err(|_| ConnError::Local(reason::PROTOCOL.into()))?;
                let next = Cand {
                    id,
                    relay: c.relay.clone(),
                    direct: vec![],
                };
                self.lookup.set_endpoint_info(next.addr());
                room.add_cand(next.clone());
                Box::pin(self.dial_pair_once(room, &next, typ, body, 1)).await
            }
            "ok" if proof_ok(room.key(), &k, ROLE_ACCEPT, &reply.proof) => {
                let peer = reply.device_id;
                let pubkey = *conn.remote_id().as_bytes();
                if peer.is_empty()
                    || peer == self.cfg.self_id
                    || !(room.cfg().allow)(&peer)
                    || (room.cfg().accept)(&peer, &pubkey).is_err()
                {
                    close(&conn, reason::REFUSED);
                    return Err(ConnError::Local(reason::REFUSED.into()));
                }
                let me = self.cfg.self_id.clone();
                self.setup(conn, &room.state, peer, me, send, recv)
            }
            _ => {
                close(&conn, reason::EPOCH);
                Err(ConnError::Local(reason::EPOCH.into()))
            }
        }
    }

    /// An incoming `irori/pair/1` connection.
    pub(crate) async fn handle_pair(self: &Arc<Self>, conn: Connection) {
        let first = async {
            let (send, mut recv) = conn.accept_bi().await.map_err(|e| e.to_string())?;
            let label = read_frame(&mut recv, 255, false)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("no label")?;
            if label != STREAM_LABEL {
                return Err("first stream is not pair".to_string());
            }
            let m = read_msg(&mut recv).await?;
            Ok::<_, String>((send, recv, m))
        };
        let (mut send, recv, m) = match tokio::time::timeout(self.cfg.open_timeout, first).await {
            Ok(Ok(x)) => x,
            Ok(Err(_)) => return close(&conn, reason::PROTOCOL),
            Err(_) => return close(&conn, reason::TIMEOUT),
        };
        if m.v != 1 {
            return close(&conn, reason::VERSION);
        }
        match m.t.as_str() {
            "guest" => self.serve_guest(conn, send, recv, m).await,
            "hello" => {
                let room = lock(&self.pair).clone();
                let Ok(k) = ekm(&conn) else {
                    return close(&conn, reason::PROTOCOL);
                };
                if let Some(room) = room
                    && proof_ok(room.key(), &k, ROLE_DIAL, &m.proof)
                {
                    return self.hold(&room, conn, send, recv, m);
                }
                if let Some(w) = self.cfg.guests.by_tag(&m.tag)
                    && w.owner != self.cfg.key.public()
                {
                    let at = OpenMsg {
                        v: 1,
                        t: "at".into(),
                        id: w.owner.to_string(),
                        ..Default::default()
                    };
                    if write_msg(&mut send, &at).await.is_ok() {
                        let _ = send.finish();
                        let _ = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
                    }
                    return;
                }
                close(&conn, reason::EPOCH)
            }
            _ => close(&conn, reason::PROTOCOL),
        }
    }

    fn hold(
        self: &Arc<Self>,
        room: &Arc<PairRoom>,
        conn: Connection,
        send: SendStream,
        recv: RecvStream,
        m: OpenMsg,
    ) {
        let peer = m.device_id.clone();
        if peer.is_empty() || peer == self.cfg.self_id || m.typ.is_empty() {
            return close(&conn, reason::REFUSED);
        }
        // The first key to hold a device id keeps it while its connection
        // lives (Go refuses a second key for a pinned peer): another holder
        // of the code can't knock the real joiner off by naming its id. The
        // same key reconnecting replaces its old connection.
        let old = {
            let mut g = lock(&room.held);
            if let Some(h) = g.get(&peer)
                && h.conn.remote_id() != conn.remote_id()
                && h.conn.close_reason().is_none()
            {
                drop(g);
                return close(&conn, reason::REFUSED);
            }
            g.insert(
                peer.clone(),
                Held {
                    conn: conn.clone(),
                    send,
                    recv,
                },
            )
        };
        if let Some(old) = old {
            close(&old.conn, reason::REPLACED);
        }
        let wr = Arc::downgrade(room);
        let p2 = peer.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep(HOLD) => {}
                _ = conn.closed() => {}
            }
            if let Some(room) = wr.upgrade() {
                let mut g = lock(&room.held);
                if g.get(&p2)
                    .is_some_and(|h| h.conn.stable_id() == conn.stable_id())
                {
                    g.remove(&p2);
                    close(&conn, reason::TIMEOUT);
                }
            }
        });
        (room.cfg().on_msg)(Envelope {
            from: peer,
            typ: m.typ,
            body: m.body,
        });
    }

    /// `connect(peer)` in a pairing room: takes up a held hello.
    fn take_held(self: &Arc<Self>, room: &Arc<PairRoom>, peer: &str) {
        let Some(h) = lock(&room.held).remove(peer) else {
            return;
        };
        let (me, room, peer) = (self.clone(), room.clone(), peer.to_string());
        tokio::spawn(async move {
            let Held {
                conn,
                mut send,
                recv,
            } = h;
            let pubkey = *conn.remote_id().as_bytes();
            if room.closed.load(Ordering::SeqCst)
                || !(room.cfg().allow)(&peer)
                || (room.cfg().accept)(&peer, &pubkey).is_err()
            {
                return close(&conn, reason::REFUSED);
            }
            let Ok(k) = ekm(&conn) else {
                return close(&conn, reason::PROTOCOL);
            };
            let ok = OpenMsg {
                v: 1,
                t: "ok".into(),
                device_id: me.cfg.self_id.clone(),
                proof: proof(room.key(), &k, ROLE_ACCEPT),
                ..Default::default()
            };
            if write_msg(&mut send, &ok).await.is_err() {
                return close(&conn, reason::PROTOCOL);
            }
            let dialer = peer.clone();
            let _ = me.setup(conn, &room.state, peer, dialer, send, recv);
        });
    }

    /// A member's guest window on this device's relay, while its
    /// connection lives.
    async fn serve_guest(
        self: &Arc<Self>,
        conn: Connection,
        mut send: SendStream,
        _recv: RecvStream,
        m: OpenMsg,
    ) {
        let owner = conn.remote_id();
        if !self.cfg.admit.admits(crate::transport::MESH_ALPN, &owner) {
            return close(&conn, reason::REFUSED);
        }
        let Ok(kb) = hex::decode(&m.key) else {
            return close(&conn, reason::PROTOCOL);
        };
        let Ok(key) = <[u8; 32]>::try_from(kb.as_slice()) else {
            return close(&conn, reason::PROTOCOL);
        };
        let secret = Secret {
            key,
            topic: m.topic,
            epoch: 0,
        };
        let (id, ttl) = match m.kind.as_str() {
            "" => {
                let ttl = Duration::from_millis(m.ttl_ms).min(crate::guest::MAX_TTL);
                (self.cfg.guests.open_window(owner, secret, ttl), ttl)
            }
            "invite" => {
                let ttl = Duration::from_millis(m.ttl_ms).min(crate::guest::INVITE_MAX_TTL);
                let id = self.cfg.guests.open_invite_window(owner, secret, ttl);
                (id.ok(), ttl)
            }
            _ => (None, Duration::ZERO),
        };
        let Some(id) = id else {
            return close(&conn, reason::REFUSED);
        };
        self.cfg.pair_lan.kick();
        let ok = OpenMsg {
            v: 1,
            t: "ok".into(),
            ..Default::default()
        };
        if write_msg(&mut send, &ok).await.is_ok() {
            tokio::select! {
                _ = conn.closed() => {}
                _ = tokio::time::sleep(ttl) => close(&conn, reason::TIMEOUT),
            }
        }
        self.cfg.guests.close_window(id);
        self.cfg.pair_lan.kick();
    }
}

async fn wait_closed(flag: &AtomicBool) {
    while !flag.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The pairing room as the Pairer sees it.
pub(crate) struct IrohPairRoom {
    net: Weak<Inner>,
    room: Arc<PairRoom>,
}

impl Room for IrohPairRoom {
    fn send(&self, to: &str, typ: &str, body: serde_json::Value) -> Result<(), TransportError> {
        let Some(net) = self.net.upgrade() else {
            return Err(TransportError("mesh: transport closed".into()));
        };
        if self.room.closed.load(Ordering::SeqCst) {
            return Err(TransportError("mesh: room closed".into()));
        }
        if to.is_empty() {
            // A hello: a dial of every candidate not yet connected.
            *lock(&self.room.hello) = Some((typ.to_string(), body.clone()));
            let cands: Vec<Cand> = lock(&self.room.cands).values().cloned().collect();
            for c in cands {
                net.dial_pair(&self.room, c, typ.to_string(), body.clone());
            }
            return Ok(());
        }
        // To a held hello: the answer is the close reason (`pair.busy`).
        if let Some(h) = lock(&self.room.held).remove(to) {
            close(&h.conn, typ);
        }
        Ok(())
    }

    fn connect(&self, peer: &str) {
        if let Some(net) = self.net.upgrade() {
            net.take_held(&self.room, peer);
        }
    }

    fn close(&self) {
        if let Some(net) = self.net.upgrade() {
            net.close_pair(&self.room);
        }
    }
}
