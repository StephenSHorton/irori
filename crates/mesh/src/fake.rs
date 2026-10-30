//! FakeNet (I:internal/mesh/fake.go): an in-memory seam [`Transport`] for
//! tests. Rooms are topics in a map, envelopes go to every other member in
//! order, and a connection is a pair of queues. It runs each side's Allow
//! and Accept with the other's real key, as the transport's handshake
//! would, and it can partition and drop links. Nothing here opens a socket.
//!
//! As on iroh, a transport is keyed by its device's key (D2), so each
//! device joins through its own handle, [`FakeNet::keyed`]; the handles
//! share one network.

use std::collections::BTreeMap;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll};
use std::time::Duration;

use irori_meshseam::StreamHandler;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

use crate::secret::Secret;
use crate::transport::{
    BoxFut, Conn, ConnError, Envelope, Room, RoomConfig, Transport, TransportError,
};

type Pair = (String, String);

fn pair_key(a: &str, b: &str) -> Pair {
    if a > b {
        (b.to_string(), a.to_string())
    } else {
        (a.to_string(), b.to_string())
    }
}

#[derive(Default)]
struct NetState {
    rooms: BTreeMap<String, BTreeMap<String, Arc<FakeRoom>>>,
    cut: BTreeMap<Pair, bool>,
    conns: BTreeMap<Pair, Vec<Arc<FakeConn>>>,
    joins: usize,
    frames: usize,
}

/// The in-memory network, and one device's handle on it.
#[derive(Clone, Default)]
pub struct FakeNet {
    st: Arc<Mutex<NetState>>,
    /// The device key this handle's connections prove (zero on the bare net).
    key: [u8; 32],
}

impl FakeNet {
    pub fn new() -> FakeNet {
        FakeNet::default()
    }

    /// This network as the transport of the device holding `pub_key`: the
    /// key a peer's Accept sees for its connections.
    pub fn keyed(&self, pub_key: [u8; 32]) -> FakeNet {
        FakeNet {
            st: self.st.clone(),
            key: pub_key,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, NetState> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Every join so far (a single device must make none).
    pub fn joins(&self) -> usize {
        self.lock().joins
    }

    /// Who is in `topic` right now.
    pub fn members(&self, topic: &str) -> Vec<String> {
        self.lock()
            .rooms
            .get(topic)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// The topics `self_id` is in.
    pub fn topics(&self, self_id: &str) -> Vec<String> {
        self.lock()
            .rooms
            .iter()
            .filter(|(_, m)| m.contains_key(self_id))
            .map(|(t, _)| t.clone())
            .collect()
    }

    /// Cuts every link between a and b: open connections drop and envelopes
    /// between them are lost until [`FakeNet::heal`].
    pub fn partition(&self, a: &str, b: &str) {
        let conns = {
            let mut g = self.lock();
            g.cut.insert(pair_key(a, b), true);
            g.conns.remove(&pair_key(a, b)).unwrap_or_default()
        };
        for c in conns {
            c.fail("mesh: link cut");
        }
    }

    /// Partitions `a` from every other member of any room.
    pub fn isolate(&self, a: &str) {
        for b in self.everyone() {
            if b != a {
                self.partition(a, &b);
            }
        }
    }

    /// Undoes [`FakeNet::partition`].
    pub fn heal(&self, a: &str, b: &str) {
        self.lock().cut.remove(&pair_key(a, b));
    }

    /// Undoes every partition.
    pub fn heal_all(&self) {
        self.lock().cut.clear();
    }

    /// Closes the open connections between a and b once, as a failed path
    /// would; the link stays, so they reconnect.
    pub fn drop_link(&self, a: &str, b: &str) {
        let conns = self
            .lock()
            .conns
            .remove(&pair_key(a, b))
            .unwrap_or_default();
        for c in conns {
            c.fail("mesh: connection dropped");
        }
    }

    /// Whether a and b have a live connection.
    pub fn connected(&self, a: &str, b: &str) -> bool {
        self.lock()
            .conns
            .get(&pair_key(a, b))
            .is_some_and(|l| l.iter().any(|c| !c.is_down()))
    }

    /// Every sync message sent.
    pub fn frames(&self) -> usize {
        self.lock().frames
    }

    fn everyone(&self) -> Vec<String> {
        let g = self.lock();
        let mut out: Vec<String> = Vec::new();
        for m in g.rooms.values() {
            for id in m.keys() {
                if !out.contains(id) {
                    out.push(id.clone());
                }
            }
        }
        out
    }
}

impl Transport for FakeNet {
    fn join(
        &self,
        s: Secret,
        cfg: RoomConfig,
    ) -> BoxFut<'_, Result<Arc<dyn Room>, TransportError>> {
        Box::pin(async move {
            if s.topic.is_empty() || cfg.self_id.is_empty() {
                return Err(TransportError(
                    "mesh: fake join needs a topic and a self".into(),
                ));
            }
            let (tx, mut rx) = mpsc::unbounded_channel::<Envelope>();
            let rt = Handle::current();
            let r = Arc::new(FakeRoom {
                net: self.clone(),
                topic: s.topic.clone(),
                epoch: s.epoch,
                cfg: cfg.clone(),
                tx,
                done: CancellationToken::new(),
                rt: rt.clone(),
            });
            {
                let mut g = self.lock();
                let m = g.rooms.entry(s.topic.clone()).or_default();
                if m.contains_key(&cfg.self_id) {
                    return Err(TransportError(format!(
                        "mesh: {} already joined {}",
                        cfg.self_id, s.topic
                    )));
                }
                m.insert(cfg.self_id.clone(), r.clone());
                g.joins += 1;
            }
            let (done, on_msg) = (r.done.clone(), cfg.on_msg.clone());
            rt.spawn(async move {
                loop {
                    let env = tokio::select! {
                        _ = done.cancelled() => return,
                        env = rx.recv() => env,
                    };
                    let Some(env) = env else { return };
                    on_msg(env);
                }
            });
            let f = cfg.on_connect.clone();
            rt.spawn(async move { f() });
            Ok(r as Arc<dyn Room>)
        })
    }
}

struct FakeRoom {
    /// The joining device's handle (its key).
    net: FakeNet,
    topic: String,
    epoch: i64,
    cfg: RoomConfig,
    tx: mpsc::UnboundedSender<Envelope>,
    done: CancellationToken,
    rt: Handle,
}

impl FakeRoom {
    fn is_closed(&self) -> bool {
        self.done.is_cancelled()
    }
}

impl Room for FakeRoom {
    fn send(&self, to: &str, typ: &str, body: serde_json::Value) -> Result<(), TransportError> {
        let env = Envelope {
            from: self.cfg.self_id.clone(),
            typ: typ.to_string(),
            body,
        };
        let dst: Vec<Arc<FakeRoom>> = {
            let g = self.net.lock();
            if self.is_closed() {
                return Err(TransportError("mesh: room closed".into()));
            }
            g.rooms
                .get(&self.topic)
                .map(|m| {
                    m.iter()
                        .filter(|(id, _)| {
                            **id != self.cfg.self_id
                                && (to.is_empty() || to == id.as_str())
                                && !g.cut.contains_key(&pair_key(id, &self.cfg.self_id))
                        })
                        .map(|(_, r)| r.clone())
                        .collect()
                })
                .unwrap_or_default()
        };
        for m in dst {
            let _ = m.tx.send(env.clone());
        }
        Ok(())
    }

    /// Links this room to the peer's room on the same topic when both sides
    /// allow and accept each other.
    fn connect(&self, peer: &str) {
        let me = &self.cfg.self_id;
        let k = pair_key(me, peer);
        let up = |g: &NetState| {
            g.conns
                .get(&k)
                .is_some_and(|l| l.iter().any(|c| c.topic == self.topic && !c.is_down()))
        };
        let other = {
            let g = self.net.lock();
            let other = g.rooms.get(&self.topic).and_then(|m| m.get(peer)).cloned();
            match other {
                Some(o)
                    if peer != me && !g.cut.contains_key(&k) && !self.is_closed() && !up(&g) =>
                {
                    o
                }
                _ => return,
            }
        };
        if !(self.cfg.allow)(peer) || !(other.cfg.allow)(me) {
            return;
        }
        if (self.cfg.accept)(peer, &other.net.key).is_err()
            || (other.cfg.accept)(me, &self.net.key).is_err()
        {
            return;
        }
        let (a, b) = new_pair(&self.net, self, &other);
        {
            let mut g = self.net.lock();
            // Re-check under the lock: the other side may have connected.
            if up(&g) || g.cut.contains_key(&k) || self.is_closed() || other.is_closed() {
                return;
            }
            g.conns.entry(k).or_default().extend([a.clone(), b.clone()]);
        }
        let f = self.cfg.on_peer.clone();
        self.rt.spawn(async move { f(a) });
        let f = other.cfg.on_peer.clone();
        other.rt.spawn(async move { f(b) });
    }

    fn close(&self) {
        let mine = {
            let mut g = self.net.lock();
            if self.done.is_cancelled() {
                return;
            }
            self.done.cancel();
            if let Some(m) = g.rooms.get_mut(&self.topic)
                && m.get(&self.cfg.self_id)
                    .is_some_and(|r| std::ptr::eq(Arc::as_ptr(r), self))
            {
                m.remove(&self.cfg.self_id);
                if m.is_empty() {
                    g.rooms.remove(&self.topic);
                }
            }
            let mut mine = Vec::new();
            for list in g.conns.values_mut() {
                list.retain(|c| {
                    let ours = c.topic == self.topic
                        && (c.self_id == self.cfg.self_id || c.peer == self.cfg.self_id);
                    if ours {
                        mine.push(c.clone());
                    }
                    !ours
                });
            }
            mine
        };
        for c in mine {
            c.fail("mesh: room closed");
        }
    }
}

/// The streams a peer opened and who handles them.
#[derive(Default)]
struct Streams {
    handler: Option<StreamHandler>,
    /// Streams that arrived before a handler was set.
    waiting: Vec<irori_meshseam::Stream>,
}

/// One end of a fake connection.
pub struct FakeConn {
    net: FakeNet,
    topic: String,
    epoch: i64,
    self_id: String,
    peer: String,
    pub_key: [u8; 32],
    session: String,
    other: Mutex<Weak<FakeConn>>,
    tx: mpsc::Sender<Vec<u8>>,
    rx: tokio::sync::Mutex<mpsc::Receiver<Vec<u8>>>,
    streams: Mutex<Streams>,
    done: CancellationToken,
    err: Mutex<Option<ConnError>>,
}

fn new_pair(net: &FakeNet, a: &FakeRoom, b: &FakeRoom) -> (Arc<FakeConn>, Arc<FakeConn>) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sum = Sha256::digest(format!(
        "{}|{}|{}|{nanos}",
        a.topic, a.cfg.self_id, b.cfg.self_id
    ));
    let sess = hex::encode(&sum[..8]);
    // Each end's sender feeds the other end's receiver.
    let (tx_ab, rx_ab) = mpsc::channel(1024);
    let (tx_ba, rx_ba) = mpsc::channel(1024);
    let x = Arc::new(FakeConn {
        net: net.clone(),
        topic: a.topic.clone(),
        epoch: a.epoch,
        self_id: a.cfg.self_id.clone(),
        peer: b.cfg.self_id.clone(),
        pub_key: b.net.key,
        session: sess.clone(),
        other: Mutex::new(Weak::new()),
        tx: tx_ab,
        rx: tokio::sync::Mutex::new(rx_ba),
        streams: Mutex::new(Streams::default()),
        done: CancellationToken::new(),
        err: Mutex::new(None),
    });
    let y = Arc::new(FakeConn {
        net: net.clone(),
        topic: a.topic.clone(),
        epoch: b.epoch,
        self_id: b.cfg.self_id.clone(),
        peer: a.cfg.self_id.clone(),
        pub_key: a.net.key,
        session: sess,
        other: Mutex::new(Weak::new()),
        tx: tx_ba,
        rx: tokio::sync::Mutex::new(rx_ab),
        streams: Mutex::new(Streams::default()),
        done: CancellationToken::new(),
        err: Mutex::new(None),
    });
    *x.other.lock().unwrap() = Arc::downgrade(&y);
    *y.other.lock().unwrap() = Arc::downgrade(&x);
    (x, y)
}

impl FakeConn {
    fn is_down(&self) -> bool {
        self.done.is_cancelled()
    }

    fn other(&self) -> Option<Arc<FakeConn>> {
        self.other
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .upgrade()
    }

    fn down_err(&self) -> ConnError {
        self.err
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| ConnError::Transport("mesh: connection down".into()))
    }

    /// The path failed: both ends go down with `why`.
    fn fail(&self, why: &str) {
        self.down_one(ConnError::Transport(why.to_string()));
        if let Some(o) = self.other() {
            o.down_one(ConnError::Transport(format!("mesh: peer went away: {why}")));
        }
    }

    fn down_one(&self, err: ConnError) {
        let mut g = self.err.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_none() {
            *g = Some(err);
            self.done.cancel();
        }
    }

    /// A stream the peer opened: to the handler, or held until one is set.
    fn arrive(&self, st: irori_meshseam::Stream) {
        let h = {
            let mut g = self.streams.lock().unwrap_or_else(|e| e.into_inner());
            match &g.handler {
                Some(h) => h.clone(),
                None => {
                    g.waiting.push(st);
                    return;
                }
            }
        };
        h(st);
    }
}

/// A fake stream end as the seam's two halves.
fn halves(label: &str, s: FakeStream) -> irori_meshseam::Stream {
    let (recv, send) = tokio::io::split(s);
    irori_meshseam::Stream {
        label: label.to_string(),
        send: Box::new(send),
        recv: Box::new(recv),
    }
}

impl Conn for FakeConn {
    fn id(&self) -> &str {
        &self.peer
    }

    fn pub_key(&self) -> [u8; 32] {
        self.pub_key
    }

    fn epoch(&self) -> i64 {
        self.epoch
    }

    fn send<'a>(&'a self, msg: &'a [u8]) -> BoxFut<'a, Result<(), ConnError>> {
        Box::pin(async move {
            if self.is_down() {
                return Err(self.down_err());
            }
            self.net.lock().frames += 1;
            let other_done = self.other().map(|o| o.done.clone()).unwrap_or_default();
            tokio::select! {
                r = self.tx.send(msg.to_vec()) => r.map_err(|_| self.down_err()),
                _ = self.done.cancelled() => Err(self.down_err()),
                _ = other_done.cancelled() => Err(self.down_err()),
            }
        })
    }

    fn recv(&self) -> BoxFut<'_, Result<Vec<u8>, ConnError>> {
        Box::pin(async move {
            let mut rx = self.rx.lock().await;
            if let Ok(m) = rx.try_recv() {
                return Ok(m);
            }
            tokio::select! {
                m = rx.recv() => m.ok_or_else(|| self.down_err()),
                _ = self.done.cancelled() => Err(self.down_err()),
            }
        })
    }

    fn open_stream<'a>(
        &'a self,
        label: &'a str,
    ) -> BoxFut<'a, Result<irori_meshseam::Stream, ConnError>> {
        Box::pin(async move {
            if self.is_down() {
                return Err(self.down_err());
            }
            let Some(o) = self.other() else {
                return Err(self.down_err());
            };
            let dead = CancellationToken::new();
            let (mine, theirs) = tokio::io::duplex(64 << 10);
            let (d1, d2) = (self.done.clone(), o.done.clone());
            let d = dead.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = d1.cancelled() => {}
                    _ = d2.cancelled() => {}
                }
                d.cancel();
            });
            o.arrive(halves(label, FakeStream::new(theirs, dead.clone())));
            Ok(halves(label, FakeStream::new(mine, dead)))
        })
    }

    fn on_stream(&self, h: StreamHandler) {
        let waiting = {
            let mut g = self.streams.lock().unwrap_or_else(|e| e.into_inner());
            g.handler = Some(h.clone());
            std::mem::take(&mut g.waiting)
        };
        for st in waiting {
            h(st);
        }
    }

    fn done(&self) -> BoxFut<'static, ()> {
        let t = self.done.clone();
        Box::pin(async move { t.cancelled_owned().await })
    }

    fn err(&self) -> Option<ConnError> {
        self.err.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn close(&self, reason: &str) {
        self.down_one(ConnError::Local(reason.to_string()));
        if let Some(o) = self.other() {
            o.down_one(ConnError::Remote(reason.to_string()));
        }
    }

    fn path(&self) -> String {
        "direct".into()
    }

    fn rtt(&self) -> Duration {
        Duration::from_millis(1)
    }

    fn fingerprints(&self) -> (String, String) {
        (
            format!("fake:{}:{}", self.session, self.self_id),
            format!("fake:{}:{}", self.session, self.peer),
        )
    }
}

/// An in-memory pipe end that fails once its connection is down, as a
/// stream on a dead connection would.
struct FakeStream {
    inner: DuplexStream,
    dead: Pin<Box<WaitForCancellationFutureOwned>>,
    is_dead: CancellationToken,
}

impl FakeStream {
    fn new(inner: DuplexStream, dead: CancellationToken) -> FakeStream {
        FakeStream {
            inner,
            dead: Box::pin(dead.clone().cancelled_owned()),
            is_dead: dead,
        }
    }

    fn check(&mut self, cx: &mut Context<'_>) -> Option<io::Error> {
        if self.is_dead.is_cancelled() || self.dead.as_mut().poll(cx).is_ready() {
            return Some(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "mesh: connection down",
            ));
        }
        None
    }
}

use std::future::Future as _;

impl AsyncRead for FakeStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Some(e) = self.check(cx) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for FakeStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Some(e) = self.check(cx) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
