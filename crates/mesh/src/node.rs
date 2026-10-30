//! The Core side of the mesh (mesh.md §1.1, §3.1; I:internal/mesh/node.go):
//! it starts the net layer once the person has a second device, keeps a
//! room per open epoch, runs sync on every connection, revokes, rotates and
//! hands the person secret on, and reports liveness and status.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, SystemTime};

use blob::Files;
use ed25519_dalek::SigningKey;
use identity::{Chain, Identity, Person};
use serde::Serialize;
use store::{Event, Filter, MAX_LIST_LIMIT, Store, Vector};
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::frames::{FRAME_HELLO, FRAME_PERSON_SECRET, Frame, PROTOCOL, Row};
use crate::peers::{PeerEpoch, load_peers, save_peers};
use crate::secret::{mesh_secret, open_secret, seal_secret};
use crate::session::Session;
use crate::transport::{
    Conn, ERR_REFUSED, Envelope, PeerConn, Refused, Room, RoomConfig, Transport,
};

/// The oldest store schema a peer may run and still sync.
pub const MIN_PEER_SCHEMA: i64 = 3;

/// Envelope types on a room.
pub const MSG_PRESENCE: &str = "presence";
pub const MSG_LEAVE: &str = "leave";

/// The presence envelope's body. It never logs a row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Presence {
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub epoch: i64,
    #[serde(rename = "coreVersion")]
    pub core_version: String,
}

/// The mesh's timers; zero values take the design's defaults. Tests
/// shorten them.
#[derive(Debug, Clone, Copy, Default)]
pub struct Intervals {
    /// Presence and connect attempts (30 s).
    pub presence: Duration,
    /// Ping on each connection (15 s).
    pub ping: Duration,
    /// Unsolicited vector, and retrying stalled devices (30 s).
    pub vector: Duration,
    /// Reconnect after a drop: from 1 s ...
    pub backoff_min: Duration,
    /// ... up to 60 s.
    pub backoff_max: Duration,
}

impl Intervals {
    pub(crate) fn with_defaults(mut self) -> Intervals {
        let def = |d: &mut Duration, v: u64| {
            if d.is_zero() {
                *d = Duration::from_secs(v);
            }
        };
        def(&mut self.presence, 30);
        def(&mut self.ping, 15);
        def(&mut self.vector, 30);
        def(&mut self.backoff_min, 1);
        def(&mut self.backoff_max, 60);
        self
    }
}

/// Held-row caps per peer: rows refused only for "no cert yet" wait in
/// memory up to these, and past them are dropped and asked for again.
pub const HELD_ROWS_CAP: usize = 2000;
pub const HELD_BYTES_CAP: usize = 8 << 20;

/// Liveness windows.
pub const CONNECTED_WITHIN: Duration = Duration::from_secs(45);
pub const RECENT_WITHIN: Duration = Duration::from_secs(600);

/// Net states.
pub const STATE_OFF: &str = "off";
pub const STATE_ON: &str = "on";
pub const STATE_REMOVED: &str = "removed";
pub const STATE_ERROR: &str = "error";

/// Go `ErrNotPeer`: Revoke was given this device or one not certified.
pub const ERR_NOT_PEER: &str = "mesh: not one of this person's other devices";

/// Why this device's own stream stops when it signed two histories.
pub(crate) const FORKED_SELF: &str =
    "this device signed two different histories (restored from a copy?); reset it";

/// The bound on person secrets held waiting for their rotation row.
const MAX_PENDING: usize = 8;

pub type LogFn = Arc<dyn Fn(&str) + Send + Sync>;
pub type NowFn = Arc<dyn Fn() -> SystemTime + Send + Sync>;
pub type IgnoredFn = Arc<dyn Fn() -> BTreeMap<String, i64> + Send + Sync>;

/// Go `Options`.
#[derive(Clone)]
pub struct Options {
    pub store: Arc<Store>,
    pub ident: Arc<Identity>,
    pub chain: Arc<Chain>,
    pub net: Arc<dyn Transport>,
    pub version: String,
    /// The attachment store blob streams serve from and fetch into; `None`
    /// serves nothing.
    pub files: Option<Arc<Files>>,
    /// Where `mesh-peers.json` lives; `None` keeps it in memory only.
    pub data_dir: Option<PathBuf>,
    pub intervals: Intervals,
    /// Turns off the handshake's cert check and hello's key check (tests
    /// only). Secrets still go only to certified devices.
    pub trust_any_peer: bool,
    /// Per writing device, the rows the folds ignored for the writer rules.
    pub ignored: Option<IgnoredFn>,
    pub logf: Option<LogFn>,
    pub now: Option<NowFn>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PeerInfo {
    pub epoch: i64,
    pub pub_hex: String,
    pub last_ping: Option<SystemTime>,
    pub last_sync: Option<SystemTime>,
    pub last_presence: Option<SystemTime>,
    /// Nanoseconds, the peer's wall clock minus this one's.
    pub skew: i64,
    pub vec: Vector,
    pub refused: i64,
    pub schema_old: bool,
}

struct PendingSecret {
    epoch: i64,
    seed: zeroize::Zeroizing<Vec<u8>>,
}

#[derive(Default)]
struct RetryState {
    backoff: Duration,
    timer: Option<JoinHandle<()>>,
}

pub(crate) struct State {
    cancel: Option<CancellationToken>,
    rt: Option<Handle>,
    state: &'static str,
    detail: String,
    rooms: BTreeMap<i64, Arc<dyn Room>>,
    topics: BTreeMap<i64, String>,
    sessions: Vec<Arc<Session>>,
    tasks: Vec<JoinHandle<()>>,
    pub(crate) known: BTreeMap<String, PeerInfo>,
    stopped: BTreeMap<String, String>,
    pub(crate) ahead: BTreeMap<String, SystemTime>,
    pending: Vec<PendingSecret>,
    restoring: String,
    restore_to: i64,
    retry: BTreeMap<String, RetryState>,
}

impl State {
    pub(crate) fn info(&mut self, dev: &str) -> &mut PeerInfo {
        self.known.entry(dev.to_string()).or_default()
    }
}

/// Go `Node`.
pub struct Node {
    pub(crate) o: Options,
    pub(crate) iv: Intervals,
    pub(crate) self_id: String,
    now: NowFn,
    logf: LogFn,
    kick: Notify,
    room_mu: tokio::sync::Mutex<()>,
    peers_mu: Mutex<()>,
    pub(crate) st: Mutex<State>,
    pub(crate) fetching: Mutex<BTreeMap<String, Arc<tokio::sync::Semaphore>>>,
    /// The shared boards open here, for `board.secret` (crate::ownboards).
    pub(crate) own_boards: Mutex<Option<Arc<dyn crate::ownboards::OwnBoards>>>,
    pub(crate) board_pending: Mutex<BTreeMap<String, crate::ownboards::PendingBoardSecret>>,
    me: Weak<Node>,
}

/// Go `time.Since(t) < d` for a time that may be ahead of now.
pub(crate) fn within(now: SystemTime, t: SystemTime, d: Duration) -> bool {
    now.duration_since(t).unwrap_or(Duration::ZERO) < d
}

impl Node {
    /// Go `New`: a Node; [`Node::run`] starts it.
    pub fn new(o: Options) -> Arc<Node> {
        let now = o
            .now
            .clone()
            .unwrap_or_else(|| Arc::new(SystemTime::now) as NowFn);
        let logf = o
            .logf
            .clone()
            .unwrap_or_else(|| Arc::new(|_: &str| {}) as LogFn);
        let mut known = BTreeMap::new();
        for (id, e) in load_peers(o.data_dir.as_deref()) {
            known.insert(
                id,
                PeerInfo {
                    epoch: e.epoch,
                    pub_hex: e.person_pub,
                    ..PeerInfo::default()
                },
            );
        }
        Arc::new_cyclic(|me| Node {
            iv: o.intervals.with_defaults(),
            self_id: o.ident.device_id(),
            now,
            logf,
            kick: Notify::new(),
            room_mu: tokio::sync::Mutex::new(()),
            peers_mu: Mutex::new(()),
            st: Mutex::new(State {
                cancel: None,
                rt: None,
                state: STATE_OFF,
                detail: String::new(),
                rooms: BTreeMap::new(),
                topics: BTreeMap::new(),
                sessions: Vec::new(),
                tasks: Vec::new(),
                known,
                stopped: BTreeMap::new(),
                ahead: BTreeMap::new(),
                pending: Vec::new(),
                restoring: String::new(),
                restore_to: 0,
                retry: BTreeMap::new(),
            }),
            fetching: Mutex::new(BTreeMap::new()),
            own_boards: Mutex::new(None),
            board_pending: Mutex::new(BTreeMap::new()),
            me: me.clone(),
            o,
        })
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn now(&self) -> SystemTime {
        (self.now)()
    }

    pub(crate) fn log(&self, msg: &str) {
        (self.logf)(msg)
    }

    fn arc(&self) -> Option<Arc<Node>> {
        self.me.upgrade()
    }

    /// This device's id.
    pub fn self_id(&self) -> &str {
        &self.self_id
    }

    /// Go `Kick`: re-reads the cert fold now.
    pub fn kick(&self) {
        self.kick.notify_one();
    }

    /// Go `HasPeer`: the person has another certified, unrevoked device.
    pub fn has_peer(&self) -> bool {
        self.o
            .chain
            .devices()
            .iter()
            .any(|d| d.cert.device_id != self.self_id && !d.revoked)
    }

    /// Go `Run`: waits until the person has a second device, then runs the
    /// net layer until `cancel`. It returns after leaving every room.
    pub async fn run(self: &Arc<Self>, cancel: CancellationToken) {
        let sub = self.o.store.subscribe();
        {
            let mut g = self.lock();
            g.cancel = Some(cancel.clone());
            g.rt = Some(Handle::current());
        }
        if let Ok(forks) = self.o.store.forks() {
            let mut g = self.lock();
            for f in forks {
                g.stopped
                    .insert(f.stored.device_id, "signed two different histories".into());
            }
        }
        while !self.has_peer() || self.is_removed() {
            if self.is_removed() {
                self.set_state(STATE_REMOVED, "");
            }
            tokio::select! {
                _ = cancel.cancelled() => return,
                ev = sub.recv_async() => if ev.is_none() { return },
                _ = self.kick.notified() => {}
            }
        }
        self.set_state(STATE_ON, "");
        self.person_changed().await;
        let start = tokio::time::Instant::now() + self.iv.presence;
        let mut pres = tokio::time::interval_at(start, self.iv.presence);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                ev = sub.recv_async() => {
                    let Some(ev) = ev else { break };
                    self.wake_all();
                    if ev.seq == 0 || store::is_person_kind(&ev.kind) {
                        self.person_changed().await;
                    }
                }
                _ = self.kick.notified() => self.person_changed().await,
                _ = pres.tick() => {
                    self.ensure_rooms().await; // retries a failed join
                    self.announce();
                    self.dial_all();
                }
            }
        }
        self.shutdown().await;
    }

    fn set_state(&self, state: &'static str, detail: &str) {
        let mut g = self.lock();
        g.state = state;
        g.detail = detail.to_string();
    }

    pub(crate) fn running(&self) -> Option<CancellationToken> {
        let g = self.lock();
        match &g.cancel {
            Some(c) if !c.is_cancelled() && g.state == STATE_ON => Some(c.clone()),
            _ => None,
        }
    }

    fn spawn<F: std::future::Future<Output = ()> + Send + 'static>(&self, f: F) -> JoinHandle<()> {
        match self.lock().rt.clone() {
            Some(rt) => rt.spawn(f),
            None => tokio::spawn(f),
        }
    }

    pub(crate) fn is_removed(&self) -> bool {
        self.o.chain.cut(&self.self_id).is_some()
    }

    /// A device with a valid, unrevoked cert for the person.
    pub(crate) fn certified(&self, dev: &str) -> bool {
        dev != self.self_id && self.o.chain.cert(dev).is_some() && self.o.chain.cut(dev).is_none()
    }

    /// Every room's Accept: only a certified, unrevoked device whose
    /// handshake proved the key its cert names.
    fn accept(&self, peer: &str, pub_key: &[u8; 32]) -> Result<(), &'static str> {
        if peer == self.self_id {
            return Err(ERR_REFUSED);
        }
        if self.o.trust_any_peer {
            return Ok(());
        }
        check_device(&self.o.chain, &self.self_id, peer, pub_key)
    }

    /// The Allow of the room for `epoch`: certified devices only, and on a
    /// previous epoch's room only devices still behind.
    pub(crate) fn allow(&self, epoch: i64, peer: &str) -> bool {
        if peer == self.self_id || self.is_removed() {
            return false;
        }
        if !self.o.trust_any_peer && !self.certified(peer) {
            return false;
        }
        let cur = self.o.ident.current();
        if epoch < cur.epoch && has_current(self.lock().known.get(peer), &cur) {
            return false;
        }
        true
    }

    /// Go `personChanged`: a removed device stops, revoked peers are
    /// dropped, a waiting person.secret may now be adopted, a peer still
    /// behind gets the current key, the epoch rooms are opened or closed
    /// and held rows are retried.
    pub(crate) async fn person_changed(&self) {
        if self.is_removed() {
            self.set_state(STATE_REMOVED, "");
            self.close_rooms().await;
            return;
        }
        self.adopt_pending();
        let cur = self.o.ident.current().epoch;
        for s in self.session_list() {
            let peer = s.conn.id();
            if !self.o.trust_any_peer && !self.certified(&peer) {
                s.conn.close("revoked");
            } else if s.epoch < cur && !self.allow(s.epoch, &peer) {
                s.conn.close("epoch"); // both have the new epoch: the new room only
            } else if s.hello_done() {
                self.send_secret(&s);
                self.send_board_secrets(&s);
            }
        }
        self.ensure_rooms().await;
        for s in self.session_list() {
            s.retry_held().await;
        }
    }

    /// Go `ensureRooms`: keeps the current epoch's room open, and every
    /// older epoch a certified device is still behind on.
    pub(crate) async fn ensure_rooms(&self) {
        let _g = self.room_mu.lock().await;
        if self.running().is_none() {
            return;
        }
        let cur = self.o.ident.current();
        let want = self.want_epochs(&cur);
        if want.len() == 1
            && let Err(e) = self.o.ident.forget(cur.epoch)
        {
            self.log(&format!("mesh: forget older person keys: {e}"));
        }
        let mut topics = BTreeMap::new();
        for e in &want {
            if let Some(seed) = self.o.ident.person_seed(*e) {
                topics.insert(*e, mesh_secret(&seed[..], &cur.id, *e).topic);
            }
        }
        let (closing, opening) = {
            let mut g = self.lock();
            let mut closing = Vec::new();
            let stale: Vec<i64> = g
                .rooms
                .keys()
                .copied()
                .filter(|e| !topics.contains_key(e) || g.topics.get(e) != topics.get(e))
                .collect();
            for e in stale {
                if let Some(r) = g.rooms.remove(&e) {
                    closing.push(r);
                }
            }
            let opening: Vec<i64> = topics
                .keys()
                .copied()
                .filter(|e| !g.rooms.contains_key(e))
                .collect();
            (closing, opening)
        };
        for r in closing {
            let _ = r.send("", MSG_LEAVE, serde_json::json!({}));
            r.close();
        }
        for e in opening {
            self.join(e).await;
        }
    }

    /// Go `wantEpochs`.
    fn want_epochs(&self, cur: &Person) -> BTreeSet<i64> {
        let mut want = BTreeSet::from([cur.epoch]);
        let held: BTreeSet<i64> = self.o.ident.older_epochs().into_iter().collect();
        let prev = self.o.ident.prev_epoch();
        let devices = self.o.chain.devices();
        let g = self.lock();
        for d in devices {
            let id = &d.cert.device_id;
            if *id == self.self_id || d.revoked {
                continue;
            }
            let k = g.known.get(id);
            if has_current(k, cur) {
                continue;
            }
            match k {
                Some(k) if k.epoch < cur.epoch && held.contains(&k.epoch) => {
                    want.insert(k.epoch);
                }
                _ if prev > 0 => {
                    want.insert(prev);
                }
                _ => {}
            }
        }
        want
    }

    async fn join(&self, epoch: i64) {
        let Some(seed) = self.o.ident.person_seed(epoch) else {
            return;
        };
        let s = mesh_secret(&seed[..], &self.o.ident.current().id, epoch);
        let topic = s.topic.clone();
        let w = self.me.clone();
        let w2 = w.clone();
        let w3 = w.clone();
        let w4 = w.clone();
        let w5 = w.clone();
        // No key or signer: the transport is keyed by the device key and its
        // handshake proves it (D2).
        let cfg = RoomConfig {
            self_id: self.self_id.clone(),
            accept: Arc::new(move |peer: &str, key: &[u8; 32]| match w.upgrade() {
                Some(n) => n.accept(peer, key).map_err(|_| Refused),
                None => Err(Refused),
            }),
            allow: Arc::new(move |peer: &str| w2.upgrade().is_some_and(|n| n.allow(epoch, peer))),
            offerer: None,
            on_msg: Arc::new(move |env: Envelope| {
                if let Some(n) = w3.upgrade() {
                    n.on_msg(epoch, env);
                }
            }),
            on_connect: Arc::new(move || {
                if let Some(n) = w4.upgrade() {
                    n.announce_in(epoch);
                }
            }),
            on_peer: Arc::new(move |c: Arc<dyn Conn>| {
                if let Some(n) = w5.upgrade() {
                    n.on_peer(epoch, PeerConn::new(c));
                }
            }),
            guest_ttl: None,
            hub_addr: None,
        };
        let r = match self.o.net.join(s, cfg).await {
            Ok(r) => r,
            Err(e) => {
                // Retried on the next presence tick; the app works alone.
                self.log(&format!("mesh: join epoch {epoch}: {e}"));
                self.lock().detail = format!("Can't open the network: {e}");
                return;
            }
        };
        {
            let mut g = self.lock();
            if g.rooms.contains_key(&epoch) || g.state != STATE_ON {
                drop(g);
                r.close();
                return;
            }
            g.rooms.insert(epoch, r);
            g.topics.insert(epoch, topic);
            g.detail.clear();
        }
        self.announce_in(epoch);
        self.dial_in(epoch);
    }

    fn room(&self, epoch: i64) -> Option<Arc<dyn Room>> {
        self.lock().rooms.get(&epoch).cloned()
    }

    fn presence(&self) -> Presence {
        Presence {
            device_id: self.self_id.clone(),
            epoch: self.o.ident.current().epoch,
            core_version: self.o.version.clone(),
        }
    }

    fn announce_in(&self, epoch: i64) {
        if let Some(r) = self.room(epoch) {
            let body = gocompat::json::marshal(&self.presence()).unwrap_or_default();
            let body = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
            let _ = r.send("", MSG_PRESENCE, body);
        }
    }

    fn announce(&self) {
        let eps: Vec<i64> = self.lock().rooms.keys().copied().collect();
        for e in eps {
            self.announce_in(e);
        }
    }

    /// Asks the room of `epoch` for a connection to every allowed device
    /// with none: under iroh there is no presence bus (mesh.md §5.3), so
    /// this is how peers meet. `connect` is idempotent.
    fn dial_in(&self, epoch: i64) {
        let Some(r) = self.room(epoch) else { return };
        for d in self.o.chain.devices() {
            let id = d.cert.device_id;
            if !d.revoked && id != self.self_id && !self.connected_to(&id) && self.allow(epoch, &id)
            {
                r.connect(&id);
            }
        }
    }

    fn dial_all(&self) {
        let eps: Vec<i64> = self.lock().rooms.keys().copied().collect();
        for e in eps {
            self.dial_in(e);
        }
    }

    /// Presence: each one from an allowed device is a Connect.
    fn on_msg(&self, epoch: i64, env: Envelope) {
        match env.typ.as_str() {
            MSG_PRESENCE => {
                let now = self.now();
                self.lock().info(&env.from).last_presence = Some(now);
                if self.allow(epoch, &env.from)
                    && let Some(r) = self.room(epoch)
                {
                    r.connect(&env.from);
                }
            }
            MSG_LEAVE => {
                if let Some(k) = self.lock().known.get_mut(&env.from) {
                    k.last_presence = None;
                }
            }
            _ => {}
        }
    }

    fn on_peer(&self, epoch: i64, c: Arc<PeerConn>) {
        let Some(me) = self.arc() else {
            c.close("not running");
            return;
        };
        let peer = c.id();
        let mut g = self.lock();
        let cancel = match &g.cancel {
            Some(cx) if !cx.is_cancelled() && g.state == STATE_ON && peer != self.self_id => {
                cx.clone()
            }
            _ => {
                drop(g);
                c.close("not running");
                return;
            }
        };
        let s = Session::new(me.clone(), epoch, c.clone());
        g.sessions.push(s.clone());
        let rt = g.rt.clone();
        let task = async move {
            s.run(cancel).await;
            me.lock().sessions.retain(|x| !Arc::ptr_eq(x, &s));
            me.schedule_reconnect(&peer);
        };
        let h = match rt {
            Some(rt) => rt.spawn(task),
            None => tokio::spawn(task),
        };
        g.tasks.retain(|t| !t.is_finished());
        g.tasks.push(h);
    }

    /// Go `scheduleReconnect`: tries `peer` again after a drop, 1 s, 2 s,
    /// 4 s ... up to 60 s apart.
    fn schedule_reconnect(&self, peer: &str) {
        let Some(me) = self.arc() else { return };
        let mut g = self.lock();
        if !matches!(&g.cancel, Some(c) if !c.is_cancelled()) || g.state != STATE_ON {
            return;
        }
        let (min, max) = (self.iv.backoff_min, self.iv.backoff_max);
        let rs = g.retry.entry(peer.to_string()).or_default();
        if rs.timer.is_some() {
            return;
        }
        rs.backoff = if rs.backoff.is_zero() {
            min
        } else {
            (rs.backoff * 2).min(max)
        };
        let wait = rs.backoff;
        let key = peer.to_string();
        let peer = peer.to_string();
        let task = async move {
            tokio::time::sleep(wait).await;
            let rooms: Vec<(i64, Arc<dyn Room>)> = {
                let mut g = me.lock();
                if let Some(rs) = g.retry.get_mut(&peer) {
                    rs.timer = None;
                }
                g.rooms.iter().map(|(e, r)| (*e, r.clone())).collect()
            };
            if me.connected_to(&peer) {
                return;
            }
            for (e, r) in &rooms {
                if me.allow(*e, &peer) {
                    r.connect(&peer);
                }
            }
            if !me.connected_to(&peer) && rooms.iter().any(|(e, _)| me.allow(*e, &peer)) {
                me.schedule_reconnect(&peer);
            }
        };
        let rt = g.rt.clone();
        let h = match rt {
            Some(rt) => rt.spawn(task),
            None => tokio::spawn(task),
        };
        if let Some(rs) = g.retry.get_mut(&key) {
            rs.timer = Some(h);
        }
    }

    pub(crate) fn connected_to(&self, peer: &str) -> bool {
        self.session_list()
            .iter()
            .any(|s| s.conn.id() == peer && s.hello_done())
    }

    /// Go `connected`: `peer` is reachable again; its backoff starts over.
    pub(crate) fn connected(&self, peer: &str) {
        if let Some(rs) = self.lock().retry.remove(peer)
            && let Some(t) = rs.timer
        {
            t.abort();
        }
    }

    pub(crate) fn session_list(&self) -> Vec<Arc<Session>> {
        self.lock().sessions.clone()
    }

    fn wake_all(&self) {
        for s in self.session_list() {
            s.poke();
        }
    }

    async fn close_rooms(&self) {
        let _g = self.room_mu.lock().await;
        let rooms = {
            let mut g = self.lock();
            for (_, rs) in std::mem::take(&mut g.retry) {
                if let Some(t) = rs.timer {
                    t.abort();
                }
            }
            std::mem::take(&mut g.rooms)
        };
        for r in rooms.into_values() {
            let _ = r.send("", MSG_LEAVE, serde_json::json!({}));
            r.close();
        }
        for s in self.session_list() {
            s.conn.close("leaving");
        }
    }

    /// Leaves every room and waits for the sessions to end, so nothing
    /// writes to the store after Run returns.
    async fn shutdown(&self) {
        {
            let mut g = self.lock();
            if g.state == STATE_ON {
                g.state = STATE_OFF;
            }
        }
        self.close_rooms().await;
        let tasks = std::mem::take(&mut self.lock().tasks);
        let all = async {
            for t in tasks {
                let _ = t.await;
            }
        };
        if tokio::time::timeout(Duration::from_secs(5), all)
            .await
            .is_err()
        {
            self.log("mesh: sessions did not stop in time");
        }
    }

    /// This device's first frame on a connection.
    pub(crate) fn hello(&self, peer: &str) -> Frame {
        let v = self.o.store.vector().unwrap_or_default();
        let cur = self.o.ident.current();
        let mut f = Frame {
            v: PROTOCOL,
            device_id: self.self_id.clone(),
            person_id: cur.id,
            epoch: cur.epoch,
            person_pub: cur.pub_hex,
            core_version: self.o.version.clone(),
            schema: store::schema_version(),
            wall_ms: hlc::unix_milli(self.now()),
            ..Frame::of_type(FRAME_HELLO)
        };
        let at = v.get(peer).copied().unwrap_or(0);
        if let Some(ev) = self.row_at(peer, at) {
            f.top = store::row_hash(&ev);
            f.top_row = Some(Box::new(Row::of(&ev)));
        }
        f.vector = v;
        f
    }

    /// `dev`'s row at `seq`, if held.
    pub(crate) fn row_at(&self, dev: &str, seq: i64) -> Option<Event> {
        self.try_row_at(dev, seq).ok().flatten()
    }

    /// `row_at`, with the store's error (Revoke must not write a cut with no
    /// hash because a list failed).
    fn try_row_at(&self, dev: &str, seq: i64) -> store::Result<Option<Event>> {
        if seq < 1 {
            return Ok(None);
        }
        let evs = self.o.store.list(&Filter {
            device_id: dev.into(),
            after_seq: seq - 1,
            limit: 1,
            ..Filter::default()
        })?;
        Ok((evs.len() == 1 && evs[0].seq == seq).then(|| evs[0].clone()))
    }

    /// A ping or pong from `peer`.
    pub(crate) fn alive(&self, peer: &str) {
        let now = self.now();
        self.lock().info(peer).last_ping = Some(now);
    }

    /// Rows applied from `peer`.
    pub(crate) fn synced(&self, peer: &str) {
        let now = self.now();
        let mut g = self.lock();
        let k = g.info(peer);
        k.last_sync = Some(now);
        k.last_ping = Some(now);
    }

    /// Go `stop`: `dev`'s stream stops on this device. When `dev` is this
    /// device, a hold on its appends is lifted: they would never come back.
    pub(crate) fn stop(&self, dev: &str, why: &str) {
        {
            let mut g = self.lock();
            if !g.stopped.contains_key(dev) {
                g.stopped.insert(dev.to_string(), why.to_string());
                drop(g);
                self.log(&format!("mesh: {dev}'s stream stopped: {why}"));
                g = self.lock();
            }
            if dev == self.self_id {
                g.restoring.clear();
                g.restore_to = 0;
            }
        }
        if dev == self.self_id {
            self.o.store.hold_appends(0);
        }
    }

    /// Go `provesOwn`: `r` is this device's own row at `seq`, signed by
    /// this device's key.
    pub(crate) fn proves_own(&self, r: Option<&Row>, seq: i64, top: &str) -> bool {
        let Some(r) = r else { return false };
        if r.d != self.self_id || r.s != seq {
            return false;
        }
        let ev = r.event();
        if !top.is_empty() && store::row_hash(&ev) != top {
            return false;
        }
        verify(
            &self.o.ident.device_pub(),
            &store::signing_bytes(&ev),
            &ev.sig,
        )
    }

    pub(crate) fn is_stopped(&self, dev: &str) -> bool {
        self.lock().stopped.contains_key(dev)
    }

    /// Go `sendSecret`: hands the peer the current person secret when its
    /// hello carried an older epoch, or this epoch with another key. Only to
    /// certified devices, only the winning rotation's key.
    pub(crate) fn send_secret(&self, s: &Session) {
        let peer = s.conn.id();
        if !self.certified(&peer) {
            return;
        }
        let (peer_epoch, peer_pub) = s.peer_key();
        let cur = self.o.ident.current();
        let from = if peer_epoch < cur.epoch {
            peer_epoch
        } else if peer_epoch == cur.epoch && !peer_pub.is_empty() && peer_pub != cur.pub_hex {
            self.o.ident.prev_epoch() // the last epoch both held one key for
        } else {
            return;
        };
        if let Some(win) = self.rotation_pub(cur.epoch)
            && win != cur.pub_hex
        {
            return; // this device lost: it takes the winner's key first
        }
        let old = match self.o.ident.person_seed(from) {
            Some(old) if from >= 1 => old,
            _ => {
                self.log(&format!(
                    "mesh: {peer} is on epoch {peer_epoch}, which this device no longer holds"
                ));
                return;
            }
        };
        if !s.mark_secret(&cur) {
            return; // already sent on this connection
        }
        let Some(seed) = self.o.ident.person_seed(cur.epoch) else {
            return;
        };
        match seal_secret(&old[..], &seed[..], &cur.id, cur.epoch, &peer) {
            Ok(sealed) => s.control(Frame {
                epoch: cur.epoch,
                from,
                sealed,
                ..Frame::of_type(FRAME_PERSON_SECRET)
            }),
            Err(e) => self.log(&format!("mesh: seal person secret: {e}")),
        }
    }

    /// Go `takeSecret`: opens a person.secret frame; it is adopted once the
    /// matching rotation row is held and wins.
    pub(crate) fn take_secret(&self, f: &Frame) {
        let cur = self.o.ident.current();
        if f.epoch < cur.epoch {
            return;
        }
        let from = if f.from == 0 { cur.epoch } else { f.from };
        let Some(old) = self.o.ident.person_seed(from) else {
            return;
        };
        let seed = match open_secret(&old[..], &f.sealed, &cur.id, f.epoch, &self.self_id) {
            Ok(s) => zeroize::Zeroizing::new(s),
            Err(e) => {
                self.log(e);
                return;
            }
        };
        {
            let mut g = self.lock();
            g.pending.push(PendingSecret {
                epoch: f.epoch,
                seed,
            });
            let n = g.pending.len();
            if n > MAX_PENDING {
                g.pending.drain(..n - MAX_PENDING);
            }
        }
        self.kick();
    }

    /// Go `adoptPending`.
    fn adopt_pending(&self) {
        let pend = std::mem::take(&mut self.lock().pending);
        let mut keep = Vec::new();
        for p in pend {
            let cur = self.o.ident.current();
            let Ok(seed) = <[u8; 32]>::try_from(&p.seed[..]) else {
                continue;
            };
            if p.epoch < cur.epoch {
                continue;
            }
            let pub_hex = hex::encode(SigningKey::from_bytes(&seed).verifying_key().to_bytes());
            if p.epoch == cur.epoch && pub_hex == cur.pub_hex {
                continue; // already this device's key
            }
            match self.rotation_pub(p.epoch) {
                Some(win) if win == pub_hex => {}
                _ => {
                    keep.push(p);
                    continue;
                }
            }
            let r = if p.epoch == cur.epoch {
                self.o.ident.replace_secret(p.epoch, &seed, &pub_hex)
            } else {
                self.o.ident.adopt_secret(p.epoch, &seed, &pub_hex)
            };
            match r {
                Ok(()) => self.log(&format!("mesh: adopted person epoch {}", p.epoch)),
                Err(e) => self.log(&format!("mesh: adopt person secret: {e}")),
            }
        }
        if !keep.is_empty() {
            let mut g = self.lock();
            keep.append(&mut g.pending);
            let n = keep.len();
            if n > MAX_PENDING {
                keep.drain(..n - MAX_PENDING);
            }
            g.pending = keep;
        }
    }

    /// Go `rotationPub`: the person key of `epoch` the winning held
    /// person.key.rotated row names: the lowest writer id, then the lowest
    /// seq, among writers ever certified (or self).
    pub(crate) fn rotation_pub(&self, epoch: i64) -> Option<String> {
        let pid = self.o.ident.current().id;
        let mut f = Filter {
            kinds: vec![identity::kinds::PERSON_KEY_ROTATED.into()],
            limit: MAX_LIST_LIMIT,
            ..Filter::default()
        };
        let mut best: Option<(String, i64, String)> = None;
        loop {
            let evs = match self.o.store.list(&f) {
                Ok(evs) if !evs.is_empty() => evs,
                _ => break,
            };
            for ev in &evs {
                let Some(r) = identity::objects::decode::<identity::KeyRotated>(&ev.payload) else {
                    continue;
                };
                if r.person_id != pid || r.epoch != epoch || !self.ever_certified(&ev.device_id) {
                    continue;
                }
                let better = match &best {
                    None => true,
                    Some((d, s, _)) => ev.device_id < *d || (ev.device_id == *d && ev.seq < *s),
                };
                if better {
                    best = Some((ev.device_id.clone(), ev.seq, r.person_pub.clone()));
                }
            }
            if (evs.len() as i64) < f.limit {
                break;
            }
            f.after = evs.last().map(Event::cursor).unwrap_or_default();
        }
        best.map(|(_, _, p)| p)
    }

    fn ever_certified(&self, dev: &str) -> bool {
        dev == self.self_id || self.o.chain.cert(dev).is_some()
    }

    /// Go `Revoke`: writes person.device.revoked with the cut at the highest
    /// of `dev`'s rows held here, rotates the person key, drops `dev`'s
    /// connections and hands the new secret to every connected device.
    pub async fn revoke(&self, dev: &str, reason: &str) -> Result<(), String> {
        if dev == self.self_id || self.o.chain.cert(dev).is_none() {
            return Err(ERR_NOT_PEER.into());
        }
        if self.o.chain.cut(dev).is_some() {
            return Ok(());
        }
        let vec = self.o.store.vector().map_err(|e| e.to_string())?;
        let cut = vec.get(dev).copied().unwrap_or(0);
        // Go returns List's error here rather than revoke with no hash.
        let hash = self
            .try_row_at(dev, cut)
            .map_err(|e| e.to_string())?
            .map(|ev| store::row_hash(&ev))
            .unwrap_or_default();
        let at = gocompat::time::GoTime::from_system(self.now());
        let rev = self
            .o
            .ident
            .revoke_of(dev, cut, &hash, reason, at)
            .map_err(|e| e.to_string())?;
        self.o
            .store
            .append("irori", identity::kinds::PERSON_DEVICE_REVOKED, &rev)
            .map_err(|e| e.to_string())?;
        let rot = self
            .o
            .ident
            .rotate(at)
            .map_err(|e| format!("mesh: rotate the person key: {e}"))?;
        self.o
            .store
            .append("irori", identity::kinds::PERSON_KEY_ROTATED, &rot)
            .map_err(|e| format!("mesh: record the rotation: {e}"))?;
        self.person_changed().await; // drops dev, and hands the new key on
        Ok(())
    }

    /// Saves every known device's epoch and key, best effort.
    pub(crate) fn save_peers(&self) {
        let Some(dir) = &self.o.data_dir else { return };
        let out: BTreeMap<String, PeerEpoch> = self
            .lock()
            .known
            .iter()
            .filter(|(_, k)| k.epoch > 0)
            .map(|(id, k)| {
                (
                    id.clone(),
                    PeerEpoch {
                        epoch: k.epoch,
                        person_pub: k.pub_hex.clone(),
                    },
                )
            })
            .collect();
        let _g = self.peers_mu.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = save_peers(dir, &out) {
            self.log(&format!("mesh: save {}: {e}", crate::peers::PEERS_FILE));
        }
    }

    /// Holds appends for a restore from `peer` up to `at`.
    pub(crate) fn restoring_from(&self, peer: &str, at: i64) {
        let mut g = self.lock();
        if at > g.restore_to {
            g.restoring = peer.to_string();
            g.restore_to = at;
        }
    }

    pub(crate) fn status_parts(&self) -> (String, String, usize, String, i64) {
        let g = self.lock();
        (
            g.state.to_string(),
            g.detail.clone(),
            g.rooms.len(),
            g.restoring.clone(),
            g.restore_to,
        )
    }

    /// How many of `dev`'s rows (or rows it sent) were refused here, kept
    /// for any device, certified or not.
    pub fn refused_count(&self, dev: &str) -> i64 {
        self.lock().known.get(dev).map(|k| k.refused).unwrap_or(0)
    }

    /// Why `dev`'s stream stopped here, if it did.
    pub fn stopped_reason(&self, dev: &str) -> Option<String> {
        self.lock().stopped.get(dev).cloned()
    }

    pub(crate) fn stopped_map(&self) -> BTreeMap<String, String> {
        self.lock().stopped.clone()
    }

    /// The spawned-task entry for session-owned work.
    pub(crate) fn spawn_task<F: std::future::Future<Output = ()> + Send + 'static>(
        &self,
        f: F,
    ) -> JoinHandle<()> {
        self.spawn(f)
    }
}

/// Go `hasCurrent`: the device's last hello carried the current epoch with
/// this device's key for it (or an unknown key), or a newer epoch.
fn has_current(k: Option<&PeerInfo>, cur: &Person) -> bool {
    let Some(k) = k else { return false };
    k.epoch > cur.epoch
        || (k.epoch == cur.epoch && (k.pub_hex.is_empty() || k.pub_hex == cur.pub_hex))
}

/// Go `CheckDevice` ([`Chain::check_device`], the one rule for who gets
/// in) with the mesh's refusal.
pub fn check_device(
    ch: &Chain,
    self_id: &str,
    id: &str,
    pub_key: &[u8; 32],
) -> Result<(), &'static str> {
    if ch.check_device(self_id, id, pub_key) {
        Ok(())
    } else {
        Err(ERR_REFUSED)
    }
}

/// Go `ed25519.Verify` (non-strict, as identity.md §6 settles).
pub(crate) fn verify(pub_key: &[u8; 32], msg: &[u8], sig: &[u8]) -> bool {
    identity::objects::ed25519_verify(pub_key, msg, sig)
}
