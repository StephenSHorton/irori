//! `irori-lan`: the sealed LAN lookup on UDP 38419 (architecture §3.1 D6,
//! mesh.md §5.3), which finds this person's devices on the home network
//! with no server, and this machine's networks ([`netif`]) for it and the
//! home server.
//!
//! It keeps go-room's privacy design (R:lan.go:14-37) with an iroh payload:
//!
//! ```text
//! "IR1" || tag(8) || nonce(24) || secretbox(json{v,id,addrs[],relay?,at,bye?}, key)
//! tag = SHA-256("irori/lan/1" || topic || u64be(unix / 300))[:8]
//! ```
//!
//! `topic` and `key` are an epoch's `MeshSecret` (mesh.md §2.4), handed in
//! through [`Config::rooms`]. The tag picks the room without trying every
//! key and changes every five minutes, so a listener can't follow a device
//! by a fixed value; receivers accept the previous, current and next window.
//! An announcement older or newer than 2 minutes is dropped, at most 200
//! tagged datagrams are opened per second, and a datagram is at most 8 KiB.
//! `id` is the sender's iroh `EndpointId` (= its certified devicePub, D2),
//! `addrs` its direct `ip:port`s and `relay` its home relay URL, so an iroh
//! dial by id alone finds it ([`LanLookup`] is an iroh `AddressLookup`).
//!
//! Announcements go out every 30 s and on change to each private subnet's
//! directed broadcast address, skipping networks Windows has on the Public
//! profile; a closing lookup sends `bye`. Tests send unicast to 127.0.0.1
//! with a non-default port ([`Config::bind`], [`Config::targets`]).
//!
//! Seams for the merge: the mesh crate supplies `rooms` (its MeshSecret of
//! every open epoch) and `on_heard` (`Servers::heard`, by device id after
//! mapping the EndpointId through the Chain); net plugs the lookup into its
//! endpoint with `Builder::address_lookup`.

pub mod netif;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crypto_secretbox::aead::{Aead, KeyInit};
use crypto_secretbox::{Nonce, XSalsa20Poly1305};
use iroh::address_lookup::{AddressLookup, EndpointData, EndpointInfo, Error as LookupError, Item};
use iroh::{EndpointId, RelayUrl, TransportAddr};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::net::UdpSocket;
use tokio::sync::{Notify, broadcast};
use tokio_util::sync::CancellationToken;

/// The LAN lookup's fixed UDP port on every device (`mesh.LANPort`,
/// I:internal/mesh/servers.go:21): a device has to know where to send
/// before it has heard anyone.
pub const LAN_PORT: u16 = 38419;

/// The dev-only env var that moves the lookup off [`LAN_PORT`]
/// (architecture §3.2), so a dev build beside Go's or the installed Irori
/// neither binds its port nor hears or answers it.
pub const DEV_PORT_OFFSET_ENV: &str = "IRORI_DEV_PORT_OFFSET";

/// The lookup's port for this build: [`LAN_PORT`] plus
/// `IRORI_DEV_PORT_OFFSET` in a debug build. A release build
/// (`debug_assertions` off) ignores the variable. `Err` for a value that is
/// not a number, or that moves the port past 65535: a dev build asked to
/// move must not quietly stay on Go's port.
pub fn dev_lan_port() -> Result<u16, String> {
    dev_lan_port_of(std::env::var_os(DEV_PORT_OFFSET_ENV))
}

/// [`dev_lan_port`] with the variable's value given.
pub fn dev_lan_port_of(v: Option<std::ffi::OsString>) -> Result<u16, String> {
    if !cfg!(debug_assertions) {
        return Ok(LAN_PORT);
    }
    let Some(v) = v.filter(|v| !v.is_empty()) else {
        return Ok(LAN_PORT);
    };
    let v = v.to_string_lossy();
    let off: u16 = v
        .trim()
        .parse()
        .map_err(|_| format!("{DEV_PORT_OFFSET_ENV}={v:?} is not a port offset"))?;
    LAN_PORT
        .checked_add(off)
        .ok_or_else(|| format!("{DEV_PORT_OFFSET_ENV}={off} moves port {LAN_PORT} past 65535"))
}
/// Bytes, header included (R:lan.go:30).
pub const MAX_DATAGRAM: usize = 8 << 10;
/// Tagged datagrams opened per second (R:lan.go:31).
pub const DEFAULT_RATE: u32 = 200;
/// The tag's domain label (go-room's was `go-room/lan/1`).
pub const TAG_LABEL: &[u8] = b"irori/lan/1";
/// Announcements further than this from now are dropped (go-room's
/// envelope `DefaultMaxAge`).
pub const MAX_AGE: Duration = Duration::from_secs(120);
/// The provenance iroh reports for addresses this lookup found.
pub const PROVENANCE: &str = "irori-lan";

const MAGIC: &[u8; 3] = b"IR1";
const TAG_LEN: usize = 8;
const HEADER: usize = MAGIC.len() + TAG_LEN;
const NONCE_LEN: usize = 24;
const BOX_OVERHEAD: usize = 16;
const WINDOW_SECS: i64 = 300;
const HEARD_CAP: usize = 256;

/// One open epoch's mesh secret: the topic picks the tag, the key seals.
#[derive(Clone)]
pub struct Room {
    pub topic: String,
    pub key: [u8; 32],
}

impl std::fmt::Debug for Room {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the key.
        f.debug_struct("Room").field("topic", &self.topic).finish()
    }
}

/// The sealed body of one datagram.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Announcement {
    pub v: i64,
    /// The sender's EndpointId, 64 hex.
    pub id: String,
    #[serde(default)]
    pub addrs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay: Option<String>,
    /// The relay server's device key, 64 hex, when the relay is not the
    /// sender's own (the home/away bridge: a home server announcing an away
    /// member's pairing room names itself as the relay, and the listener
    /// pins this key for it).
    #[serde(rename = "relayKey", default, skip_serializing_if = "Option::is_none")]
    pub relay_key: Option<String>,
    /// Unix ms.
    pub at: i64,
    /// The sender is closing its lookup: forget it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bye: bool,
}

/// A room's tag for one 300 s window (R:lan.go:184-195, relabelled).
pub fn tag(topic: &str, window: i64) -> [u8; TAG_LEN] {
    let mut h = Sha256::new();
    h.update(TAG_LABEL);
    h.update(topic.as_bytes());
    h.update((window as u64).to_be_bytes());
    let d = h.finalize();
    let mut t = [0u8; TAG_LEN];
    t.copy_from_slice(&d[..TAG_LEN]);
    t
}

/// The tag window of a unix time in seconds.
pub fn window_of(unix_secs: i64) -> i64 {
    unix_secs.div_euclid(WINDOW_SECS)
}

/// One datagram: the room's tag for `unix_secs`, a fresh nonce and the
/// sealed announcement.
pub fn seal(room: &Room, a: &Announcement, unix_secs: i64) -> Vec<u8> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).expect("getrandom");
    seal_with_nonce(room, a, unix_secs, nonce)
}

fn seal_with_nonce(room: &Room, a: &Announcement, unix_secs: i64, nonce: [u8; 24]) -> Vec<u8> {
    let plain = serde_json::to_vec(a).expect("announcement json");
    let cipher = XSalsa20Poly1305::new((&room.key).into());
    let sealed = cipher
        .encrypt(Nonce::from_slice(&nonce), plain.as_slice())
        .expect("secretbox seal");
    let mut out = Vec::with_capacity(HEADER + NONCE_LEN + sealed.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&tag(&room.topic, window_of(unix_secs)));
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    out
}

/// The rooms a datagram's tag names, in the previous, current or next
/// window; `None` when it is not an IR1 datagram of a plausible size.
pub fn rooms_for<'r>(rooms: &'r [Room], dgram: &[u8], unix_secs: i64) -> Option<Vec<&'r Room>> {
    if dgram.len() < HEADER + NONCE_LEN + BOX_OVERHEAD
        || dgram.len() > MAX_DATAGRAM
        || &dgram[..MAGIC.len()] != MAGIC
    {
        return None;
    }
    let t = &dgram[MAGIC.len()..HEADER];
    let w = window_of(unix_secs);
    Some(
        rooms
            .iter()
            .filter(|r| [w, w - 1, w + 1].iter().any(|ww| tag(&r.topic, *ww) == t))
            .collect(),
    )
}

/// Opens a datagram with one room's key; `None` when it doesn't open or
/// isn't a v1 announcement.
pub fn open(room: &Room, dgram: &[u8]) -> Option<Announcement> {
    if dgram.len() < HEADER + NONCE_LEN + BOX_OVERHEAD {
        return None;
    }
    let nonce = &dgram[HEADER..HEADER + NONCE_LEN];
    let cipher = XSalsa20Poly1305::new((&room.key).into());
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce), &dgram[HEADER + NONCE_LEN..])
        .ok()?;
    let a: Announcement = serde_json::from_slice(&plain).ok()?;
    (a.v == 1).then_some(a)
}

/// Where a device was last heard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heard {
    pub addrs: Vec<SocketAddr>,
    pub relay: Option<RelayUrl>,
    /// The relay's server key (hex) when the announcement named one.
    pub relay_key: Option<String>,
    /// The announcement's own time, unix ms.
    pub at: i64,
    /// Where the datagram came from.
    pub from: SocketAddr,
}

pub type RoomsFn = Arc<dyn Fn() -> Vec<Room> + Send + Sync>;
pub type TargetsFn = Arc<dyn Fn() -> Vec<SocketAddr> + Send + Sync>;
pub type NowFn = Arc<dyn Fn() -> SystemTime + Send + Sync>;
pub type HeardFn = Arc<dyn Fn(EndpointId, SocketAddr) + Send + Sync>;
pub type LogFn = Arc<dyn Fn(&str) + Send + Sync>;
/// Every authentic announcement with the topic of the room that opened it
/// (pairing: the code-derived room, M7).
pub type RoomHeardFn = Arc<dyn Fn(&str, EndpointId, &Heard) + Send + Sync>;
/// Announcements this device makes for another device (the home/away
/// bridge, M7), read at every send.
pub type ProxyFn = Arc<dyn Fn() -> Vec<Proxy> + Send + Sync>;

/// One announcement on another device's behalf: a home server carrying an
/// away member's pairing room on its LAN, naming its own relay.
#[derive(Clone)]
pub struct Proxy {
    pub room: Room,
    pub id: EndpointId,
    pub relay: Option<RelayUrl>,
    /// The relay server's device key, hex.
    pub relay_key: Option<String>,
}

/// Sets up a [`LanLookup`].
#[derive(Clone)]
pub struct Config {
    /// The fixed port (default [`LAN_PORT`]; the app's default transport
    /// takes [`dev_lan_port`], which adds `IRORI_DEV_PORT_OFFSET` in dev
    /// builds, architecture §3.2).
    pub port: u16,
    /// The socket address to bind. `None`: `0.0.0.0:port`, the wildcard,
    /// which is the only bind Windows delivers broadcasts to. Tests:
    /// `127.0.0.1:<port>`.
    pub bind: Option<SocketAddr>,
    /// Where announcements go. `None`: [`netif::broadcast_targets`].
    pub targets: Option<TargetsFn>,
    /// The open epochs' mesh secrets, read at every send and receive.
    pub rooms: RoomsFn,
    /// This device; its own announcements are ignored.
    pub self_id: EndpointId,
    /// How often this device announces itself (default 30 s).
    pub every: Duration,
    /// How often a port that couldn't be bound is tried again (default 30 s,
    /// roomnet's `lanRetry`).
    pub retry: Duration,
    /// Tagged datagrams opened per second (default [`DEFAULT_RATE`]).
    pub max_rate: u32,
    /// The clock for tags and ages (tests).
    pub now: Option<NowFn>,
    /// Called for every authentic announcement (mesh: `Servers.Heard`).
    pub on_heard: Option<HeardFn>,
    /// Called for every authentic announcement with its room's topic.
    pub on_room: Option<RoomHeardFn>,
    /// Announcements made for other devices (the bridge).
    pub proxies: Option<ProxyFn>,
    /// Diagnostics, never payloads or keys.
    pub logf: Option<LogFn>,
}

impl Config {
    pub fn new(self_id: EndpointId, rooms: RoomsFn) -> Self {
        Self {
            port: LAN_PORT,
            bind: None,
            targets: None,
            rooms,
            self_id,
            every: Duration::from_secs(30),
            retry: Duration::from_secs(30),
            max_rate: DEFAULT_RATE,
            now: None,
            on_heard: None,
            on_room: None,
            proxies: None,
            logf: None,
        }
    }
}

#[derive(Default)]
struct State {
    socket: Option<Arc<UdpSocket>>,
    err: String,
    started: bool,
    me: Option<(Vec<SocketAddr>, Option<RelayUrl>)>,
    heard: HashMap<EndpointId, Heard>,
    /// The newest announcement time taken per device and room: an
    /// announcement no newer is a replay (go-room drops a seen envelope id;
    /// mesh.md §6 `TestLANReplayDoesntMoveAMember`).
    newest: HashMap<(EndpointId, String), i64>,
    rate_at: i64,
    rate_n: u32,
    /// The newest `at` this device has sent (see [`Inner::stamp`]).
    sent_at: i64,
}

struct Inner {
    cfg: Config,
    st: Mutex<State>,
    kick: Notify,
    stop: CancellationToken,
    updates: broadcast::Sender<(EndpointId, Heard)>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn now(&self) -> SystemTime {
        self.cfg.now.as_ref().map_or_else(SystemTime::now, |f| f())
    }

    /// An announcement's `at`: the time in ms, but always after the last
    /// one sent. A receiver drops an announcement no newer than the newest
    /// it took (the replay guard), so a `bye` sent in the same millisecond
    /// as the announcement before it was dropped and the device stayed
    /// listed (a tick and the close both ready when the task ran, as on a
    /// loaded machine).
    fn stamp(&self, now: SystemTime) -> i64 {
        let mut st = self.lock();
        st.sent_at = unix_ms(now).max(st.sent_at + 1);
        st.sent_at
    }

    fn log(&self, msg: &str) {
        if let Some(l) = &self.cfg.logf {
            l(msg);
        }
    }

    fn targets(&self) -> Vec<SocketAddr> {
        match &self.cfg.targets {
            Some(t) => t(),
            None => netif::broadcast_targets(self.cfg.port),
        }
    }
}

fn unix_ms(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

/// The LAN lookup: one UDP socket, its announcer and its reader.
#[derive(Clone)]
pub struct LanLookup {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for LanLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LanLookup")
            .field("port", &self.inner.cfg.port)
            .finish()
    }
}

impl LanLookup {
    /// Binds the socket (retrying every `retry` while the port is taken) and
    /// starts announcing and listening. Must run inside a tokio runtime.
    pub fn start(cfg: Config) -> LanLookup {
        let (updates, _) = broadcast::channel(64);
        let inner = Arc::new(Inner {
            cfg,
            st: Mutex::new(State::default()),
            kick: Notify::new(),
            stop: CancellationToken::new(),
            updates,
            task: Mutex::new(None),
        });
        let h = tokio::spawn(run(inner.clone()));
        *inner.task.lock().unwrap_or_else(|e| e.into_inner()) = Some(h);
        LanLookup { inner }
    }

    /// What this device announces: its direct addresses and home relay.
    /// A change is announced at once.
    pub fn set_self(&self, addrs: Vec<SocketAddr>, relay: Option<RelayUrl>) {
        let changed = {
            let mut st = self.inner.lock();
            let next = Some((addrs, relay));
            let changed = st.me != next;
            st.me = next;
            changed
        };
        if changed {
            self.inner.kick.notify_one();
        }
    }

    /// Announces now (a room opened, the network changed).
    pub fn announce_now(&self) {
        self.inner.kick.notify_one();
    }

    /// Where `id` was last heard, if within [`MAX_AGE`].
    pub fn heard(&self, id: &EndpointId) -> Option<Heard> {
        let now = unix_ms(self.inner.now());
        let st = self.inner.lock();
        st.heard
            .get(id)
            .filter(|h| (now - h.at).abs() <= MAX_AGE.as_millis() as i64 * 2)
            .cloned()
    }

    /// The bound socket's address (`None` while the port is taken).
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.inner
            .lock()
            .socket
            .as_ref()
            .and_then(|s| s.local_addr().ok())
    }

    /// Why devices on this network can't find this one directly, in a
    /// sentence for Devices, or "" while it works (roomnet `LANError`,
    /// I:internal/mesh/roomnet/roomnet.go:216-234).
    pub fn error_line(&self) -> String {
        let (err, started, bound) = {
            let st = self.inner.lock();
            (st.err.clone(), st.started, st.socket.is_some())
        };
        if !err.is_empty() {
            return format!(
                "Irori can't use UDP port {} on this device, so devices on this network can't find it directly. Another program may be using it; Irori tries again on its own.",
                self.inner.cfg.port
            );
        }
        if started && bound && self.inner.targets().is_empty() {
            return "Devices on this network can't find this one directly: Windows has this network as Public, or this device isn't on a home network. In Windows Settings, open Network & internet and set your home network to Private.".into();
        }
        String::new()
    }

    /// Sends `bye`, stops and closes the socket.
    pub async fn close(&self) {
        self.inner.stop.cancel();
        let h = self
            .inner
            .task
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(h) = h {
            let _ = h.await;
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

fn bind(addr: SocketAddr) -> std::io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::for_address(addr), Type::DGRAM, Some(Protocol::UDP))?;
    s.set_broadcast(true)?;
    s.bind(&addr.into())?;
    s.set_nonblocking(true)?;
    UdpSocket::from_std(s.into())
}

async fn run(inner: Arc<Inner>) {
    let addr = inner.cfg.bind.unwrap_or(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        inner.cfg.port,
    ));
    let sock = loop {
        match bind(addr) {
            Ok(s) => break Arc::new(s),
            Err(e) => {
                let msg = e.to_string();
                let first = {
                    let mut st = inner.lock();
                    let first = st.err != msg;
                    st.err = msg.clone();
                    st.started = true;
                    first
                };
                if first {
                    inner.log(&format!("lan: the LAN lookup is off: {msg}"));
                }
                tokio::select! {
                    _ = inner.stop.cancelled() => return,
                    _ = tokio::time::sleep(inner.cfg.retry) => {}
                }
            }
        }
    };
    {
        let mut st = inner.lock();
        st.socket = Some(sock.clone());
        st.err.clear();
        st.started = true;
    }
    let reader = tokio::spawn(read(inner.clone(), sock.clone()));
    let mut tick = tokio::time::interval(inner.cfg.every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = inner.stop.cancelled() => break,
            _ = inner.kick.notified() => announce(&inner, &sock, false).await,
            _ = tick.tick() => announce(&inner, &sock, false).await,
        }
    }
    announce(&inner, &sock, true).await;
    reader.abort();
    let _ = reader.await;
    inner.lock().socket = None;
}

async fn announce(inner: &Inner, sock: &UdpSocket, bye: bool) {
    announce_proxies(inner, sock, bye).await;
    let me = inner.lock().me.clone();
    let Some((addrs, relay)) = me else { return };
    let rooms = (inner.cfg.rooms)();
    if rooms.is_empty() {
        return;
    }
    let now = inner.now();
    let a = Announcement {
        v: 1,
        id: inner.cfg.self_id.to_string(),
        addrs: if bye {
            vec![]
        } else {
            addrs.iter().map(|a| a.to_string()).collect()
        },
        relay: if bye {
            None
        } else {
            relay.as_ref().map(|r| r.to_string())
        },
        relay_key: None,
        at: inner.stamp(now),
        bye,
    };
    let secs = unix_ms(now).div_euclid(1000);
    let targets = inner.targets();
    for room in &rooms {
        let d = seal(room, &a, secs);
        if d.len() > MAX_DATAGRAM {
            inner.log("lan: an announcement doesn't fit one datagram");
            continue;
        }
        for t in &targets {
            let _ = sock.send_to(&d, t).await;
        }
    }
}

async fn announce_proxies(inner: &Inner, sock: &UdpSocket, bye: bool) {
    let Some(f) = &inner.cfg.proxies else { return };
    let list = f();
    if list.is_empty() {
        return;
    }
    let now = inner.now();
    let secs = unix_ms(now).div_euclid(1000);
    let at = inner.stamp(now);
    let targets = inner.targets();
    for p in &list {
        let a = Announcement {
            v: 1,
            id: p.id.to_string(),
            addrs: vec![],
            relay: if bye {
                None
            } else {
                p.relay.as_ref().map(|r| r.to_string())
            },
            relay_key: if bye { None } else { p.relay_key.clone() },
            at,
            bye,
        };
        let d = seal(&p.room, &a, secs);
        for t in &targets {
            let _ = sock.send_to(&d, t).await;
        }
    }
}

async fn read(inner: Arc<Inner>, sock: Arc<UdpSocket>) {
    let mut buf = vec![0u8; MAX_DATAGRAM + 1];
    let local = sock.local_addr().ok();
    let mut own = OwnIps::default();
    loop {
        let (n, from) = match sock.recv_from(&mut buf).await {
            Ok(x) => x,
            // Windows reports an ICMP port unreachable from an earlier send
            // as a read error on UDP sockets; keep reading.
            Err(_) => {
                if inner.stop.is_cancelled() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
                continue;
            }
        };
        let from = SocketAddr::new(canonical(from.ip()), from.port());
        if let Some(l) = local
            && (from == l
                || (l.ip().is_unspecified() && from.port() == l.port() && own.has(from.ip())))
        {
            continue; // our own broadcast came back
        }
        receive(&inner, &buf[..n], from);
    }
}

fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

fn receive(inner: &Inner, dgram: &[u8], from: SocketAddr) {
    let now = inner.now();
    let now_ms = unix_ms(now);
    let rooms = (inner.cfg.rooms)();
    let Some(matched) = rooms_for(&rooms, dgram, now_ms.div_euclid(1000)) else {
        return;
    };
    if matched.is_empty() || !take_rate(inner, now_ms.div_euclid(1000)) {
        return;
    }
    let Some((topic, a)) = matched
        .iter()
        .find_map(|r| open(r, dgram).map(|a| (r.topic.clone(), a)))
    else {
        return;
    };
    if (now_ms - a.at).abs() > MAX_AGE.as_millis() as i64 {
        return;
    }
    let Ok(id) = a.id.parse::<EndpointId>() else {
        return;
    };
    if id == inner.cfg.self_id {
        return;
    }
    // A captured datagram sent again, from anywhere, within the age window
    // must not move this device's address or forget it (a replayed bye).
    {
        let mut st = inner.lock();
        let key = (id, topic.clone());
        if st.newest.get(&key).is_some_and(|&t| a.at <= t) {
            return;
        }
        let horizon = now_ms - 2 * MAX_AGE.as_millis() as i64;
        if st.newest.len() >= 4 * HEARD_CAP {
            st.newest.retain(|_, t| *t >= horizon);
        }
        if st.newest.len() < 4 * HEARD_CAP {
            st.newest.insert(key, a.at);
        }
    }
    if a.bye {
        inner.lock().heard.remove(&id);
        return;
    }
    let h = Heard {
        addrs: a.addrs.iter().filter_map(|s| s.parse().ok()).collect(),
        relay: a.relay.as_deref().and_then(|r| r.parse().ok()),
        relay_key: a.relay_key.clone(),
        at: a.at,
        from,
    };
    {
        let mut st = inner.lock();
        if !st.heard.contains_key(&id) && st.heard.len() >= HEARD_CAP {
            // Forget the stalest sighting.
            if let Some(old) = st.heard.iter().min_by_key(|(_, h)| h.at).map(|(k, _)| *k) {
                st.heard.remove(&old);
            }
        }
        st.heard.insert(id, h.clone());
    }
    if let Some(f) = &inner.cfg.on_room {
        f(&topic, id, &h);
    }
    let _ = inner.updates.send((id, h));
    if let Some(f) = &inner.cfg.on_heard {
        f(id, from);
    }
}

/// Spends one of this second's opens (R:lan.go:259-270).
fn take_rate(inner: &Inner, sec: i64) -> bool {
    let mut st = inner.lock();
    if st.rate_at != sec {
        st.rate_at = sec;
        st.rate_n = 0;
    }
    if st.rate_n >= inner.cfg.max_rate {
        return false;
    }
    st.rate_n += 1;
    true
}

/// This host's interface addresses, refreshed every 30 s, to drop our own
/// broadcasts on a wildcard socket.
#[derive(Default)]
struct OwnIps {
    at: Option<std::time::Instant>,
    addrs: Vec<IpAddr>,
}

impl OwnIps {
    fn has(&mut self, ip: IpAddr) -> bool {
        if self
            .at
            .is_none_or(|t| t.elapsed() > Duration::from_secs(30))
        {
            self.addrs.clear();
            for ifc in netdev::get_interfaces() {
                self.addrs
                    .extend(ifc.ipv4.iter().map(|n| IpAddr::V4(n.addr())));
                self.addrs
                    .extend(ifc.ipv6.iter().map(|n| IpAddr::V6(n.addr())));
            }
            self.at = Some(std::time::Instant::now());
        }
        self.addrs.contains(&ip)
    }
}

fn item_of(id: EndpointId, h: &Heard) -> Item {
    let mut addrs: Vec<TransportAddr> = h.addrs.iter().copied().map(TransportAddr::Ip).collect();
    if let Some(r) = &h.relay {
        addrs.push(TransportAddr::Relay(r.clone()));
    }
    let info = EndpointInfo::from_parts(id, EndpointData::new(addrs));
    Item::new(info, PROVENANCE, Some((h.at.max(0) as u64) * 1000))
}

type ItemStream = Pin<Box<dyn n0_future::Stream<Item = Result<Item, LookupError>> + Send>>;

impl AddressLookup for LanLookup {
    /// iroh hands over this endpoint's own addresses and home relay: they
    /// are what the next announcement says.
    fn publish(&self, data: &EndpointData) {
        let addrs: Vec<SocketAddr> = data.ip_addrs().copied().collect();
        let relay = data.relay_urls().next().cloned();
        self.set_self(addrs, relay);
    }

    /// Where `id` was heard, then every later sighting of it.
    fn resolve(&self, id: EndpointId) -> Option<ItemStream> {
        let first = self.heard(&id).map(|h| item_of(id, &h));
        let rx = self.inner.updates.subscribe();
        let s = n0_future::stream::unfold((first, rx), move |(first, mut rx)| async move {
            if let Some(it) = first {
                return Some((Ok(it), (None, rx)));
            }
            loop {
                match rx.recv().await {
                    Ok((got, h)) if got == id => return Some((Ok(item_of(id, &h)), (None, rx))),
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        });
        Some(Box::pin(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_port_offset_moves_the_port_in_debug_builds_only() {
        let of = |v: Option<&str>| dev_lan_port_of(v.map(std::ffi::OsString::from));
        assert_eq!(of(None), Ok(LAN_PORT));
        assert_eq!(of(Some("")), Ok(LAN_PORT));
        if cfg!(debug_assertions) {
            assert_eq!(of(Some("100")), Ok(LAN_PORT + 100));
            assert_eq!(of(Some(" 7 ")), Ok(LAN_PORT + 7));
            assert_eq!(of(Some("0")), Ok(LAN_PORT));
            assert!(of(Some("-1")).is_err());
            assert!(of(Some("x")).is_err());
            assert!(of(Some("30000")).is_err(), "past 65535");
            assert!(of(Some("70000")).is_err());
        } else {
            assert_eq!(of(Some("100")), Ok(LAN_PORT));
            assert_eq!(of(Some("x")), Ok(LAN_PORT));
        }
    }

    fn room(topic: &str, k: u8) -> Room {
        Room {
            topic: topic.into(),
            key: [k; 32],
        }
    }

    fn ann() -> Announcement {
        Announcement {
            v: 1,
            id: "ab".repeat(32),
            addrs: vec!["127.0.0.1:1".into()],
            relay: None,
            relay_key: None,
            at: 1,
            bye: false,
        }
    }

    #[test]
    fn tag_windows() {
        let r = room("irori/mesh/1/00", 1);
        let t0 = 1_000_000_i64;
        let d = seal(&r, &ann(), t0);
        let rooms = vec![r.clone()];
        // Previous, current and next window are accepted.
        for dt in [-WINDOW_SECS, 0, WINDOW_SECS] {
            assert_eq!(rooms_for(&rooms, &d, t0 + dt).unwrap().len(), 1, "dt {dt}");
        }
        // Two windows away is not.
        for dt in [-2 * WINDOW_SECS - 1, 2 * WINDOW_SECS + 1] {
            assert!(
                rooms_for(&rooms, &d, t0 + dt).unwrap().is_empty(),
                "dt {dt}"
            );
        }
        assert_eq!(open(&r, &d), Some(ann()));
        // The tag is the label's, not go-room's.
        let mut h = Sha256::new();
        h.update(b"go-room/lan/1");
        h.update(r.topic.as_bytes());
        h.update((window_of(t0) as u64).to_be_bytes());
        assert_ne!(&h.finalize()[..8], &tag(&r.topic, window_of(t0))[..]);
    }

    #[test]
    fn another_key_or_junk_does_not_open() {
        let r = room("t", 1);
        let d = seal(&r, &ann(), 5);
        assert_eq!(open(&room("t", 2), &d), None);
        let mut bad = d.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert_eq!(open(&r, &bad), None);
        assert!(rooms_for(std::slice::from_ref(&r), b"IR1short", 5).is_none());
        let mut other = d.clone();
        other[0] = b'G';
        assert!(rooms_for(std::slice::from_ref(&r), &other, 5).is_none());
        let big = vec![0u8; MAX_DATAGRAM + 1];
        assert!(rooms_for(&[r], &big, 5).is_none());
    }

    #[test]
    fn layout() {
        let r = room("t", 3);
        let d = seal_with_nonce(&r, &ann(), 600, [7; 24]);
        assert_eq!(&d[..3], b"IR1");
        assert_eq!(&d[3..11], &tag("t", 2));
        assert_eq!(&d[11..35], &[7u8; 24]);
        let plain = serde_json::to_vec(&ann()).unwrap();
        assert_eq!(d.len(), 35 + plain.len() + 16);
        assert_eq!(
            String::from_utf8(plain).unwrap(),
            format!(
                r#"{{"v":1,"id":"{}","addrs":["127.0.0.1:1"],"at":1}}"#,
                "ab".repeat(32)
            )
        );
    }
}
