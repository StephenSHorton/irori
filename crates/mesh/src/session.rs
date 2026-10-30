//! Sync on one connection (mesh.md §3.1 "Session algorithm";
//! I:internal/mesh/session.go). Its state is keyed on the Conn, not the
//! device id: a replaced connection is a new session.
//!
//! Tasks run it: the reader applies frames as they arrive, and the writer
//! sends hello, then control frames (vector, pong, person.secret) ahead of
//! rows, one rows batch at a time: the next goes once the peer's ack vector
//! shows the last one applied. Streams the peer opens are served on their
//! own tasks, and the board loop pulls board logs.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use identity::Person;
use store::{ErrorKind, Event, Filter, MAX_LIST_LIMIT, Vector};
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::frames::{
    FRAME_HELLO, FRAME_PERSON_SECRET, FRAME_PING, FRAME_PONG, FRAME_ROW_AT, FRAME_ROWS,
    FRAME_VECTOR, Frame, MAX_BATCH_ROWS, PROTOCOL, Row, batches, decode_frame,
};
use crate::node::{FORKED_SELF, HELD_BYTES_CAP, HELD_ROWS_CAP, MIN_PEER_SCHEMA, Node, within};
use crate::transport::PeerConn;

/// The control queue's size: when full it drops (logged).
const CONTROL_QUEUE: usize = 64;

/// Rows refused only for "no cert yet", per device, up to the caps.
#[derive(Debug, Default)]
pub(crate) struct Held {
    by_dev: BTreeMap<String, BTreeMap<i64, Event>>,
    rows: usize,
    bytes: usize,
}

impl Held {
    /// Go `hold`: a duplicate seq doesn't count; past a cap the rest is
    /// dropped (asked for again by vector later).
    pub(crate) fn hold(&mut self, dev: &str, evs: &[Event]) {
        let m = self.by_dev.entry(dev.to_string()).or_default();
        for ev in evs {
            if m.contains_key(&ev.seq) {
                continue;
            }
            let size = ev.payload.len() + ev.sig.len();
            if self.rows >= HELD_ROWS_CAP || self.bytes + size > HELD_BYTES_CAP {
                break;
            }
            m.insert(ev.seq, ev.clone());
            self.rows += 1;
            self.bytes += size;
        }
        if m.is_empty() {
            self.by_dev.remove(dev);
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.rows
    }

    fn has(&self, dev: &str) -> bool {
        self.by_dev.get(dev).is_some_and(|m| !m.is_empty())
    }
}

#[derive(Default)]
struct SessState {
    hello_recv: bool,
    p_epoch: i64,
    p_pub: String,
    sent_key: String,
    peer_vec: Vector,
    sent_loose: bool,
    inflight: bool,
    batch: i64,
    sent_at: Option<SystemTime>,
    extent: BTreeMap<String, (i64, i64)>,
    stuck: BTreeSet<String>,
    held: Held,
    own_check: i64,
    ended: bool,
}

/// Go `session`.
pub(crate) struct Session {
    pub(crate) n: Arc<Node>,
    /// The room's epoch.
    pub(crate) epoch: i64,
    pub(crate) conn: Arc<PeerConn>,
    ctl_tx: mpsc::Sender<Frame>,
    ctl_rx: tokio::sync::Mutex<mpsc::Receiver<Frame>>,
    wake: Notify,
    st: Mutex<SessState>,
    serving: Mutex<Vec<JoinHandle<()>>>,
}

fn get(v: &Vector, d: &str) -> i64 {
    v.get(d).copied().unwrap_or(0)
}

/// Store work on a blocking thread (architecture §2).
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    match tokio::task::spawn_blocking(f).await {
        Ok(v) => v,
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
}

impl Session {
    pub(crate) fn new(n: Arc<Node>, epoch: i64, conn: Arc<PeerConn>) -> Arc<Session> {
        let (ctl_tx, ctl_rx) = mpsc::channel(CONTROL_QUEUE);
        Arc::new(Session {
            n,
            epoch,
            conn,
            ctl_tx,
            ctl_rx: tokio::sync::Mutex::new(ctl_rx),
            wake: Notify::new(),
            st: Mutex::new(SessState::default()),
            serving: Mutex::new(Vec::new()),
        })
    }

    fn lock(&self) -> MutexGuard<'_, SessState> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn poke(&self) {
        self.wake.notify_one();
    }

    /// Queues a frame for the writer. A full queue drops it: vectors and
    /// pongs repeat, and the queue only fills when the connection is stuck.
    pub(crate) fn control(&self, f: Frame) {
        if self.ctl_tx.try_send(f.clone()).is_err() {
            self.n.log(&format!(
                "mesh: {}: control queue full, dropped {}",
                self.conn.id(),
                f.typ
            ));
        }
    }

    pub(crate) fn hello_done(&self) -> bool {
        self.lock().hello_recv
    }

    /// The epoch and person key the peer's hello carried.
    pub(crate) fn peer_key(&self) -> (i64, String) {
        let g = self.lock();
        (g.p_epoch, g.p_pub.clone())
    }

    /// Records that `cur`'s secret goes to the peer on this connection;
    /// false if it already went.
    pub(crate) fn mark_secret(&self, cur: &Person) -> bool {
        let k = format!("{}/{}", cur.epoch, cur.pub_hex);
        let mut g = self.lock();
        if g.sent_key == k {
            return false;
        }
        g.sent_key = k;
        true
    }

    pub(crate) fn held_count(&self) -> usize {
        self.lock().held.count()
    }

    pub(crate) async fn run(self: &Arc<Self>, parent: CancellationToken) {
        let ctx = parent.child_token();
        let peer = self.conn.id();
        // Close the connection when the Node stops; end when it goes down.
        {
            let (ctx, parent, conn) = (ctx.clone(), parent.clone(), self.conn.clone());
            let done = conn.done();
            self.n.spawn_task(async move {
                tokio::select! {
                    _ = done.cancelled() => {}
                    _ = ctx.cancelled() => {
                        if parent.is_cancelled() {
                            conn.close("shutting down");
                        }
                    }
                }
                ctx.cancel();
            });
        }
        // Streams the peer opens: each is served on its own task, ended with
        // the session.
        let accept = {
            let (s, ctx, peer) = (self.clone(), ctx.clone(), peer.clone());
            self.n.spawn_task(async move {
                loop {
                    let next = tokio::select! {
                        _ = ctx.cancelled() => return,
                        r = s.conn.accept_stream() => r,
                    };
                    let Ok((label, st)) = next else { return };
                    if s.lock().ended {
                        continue;
                    }
                    let (n, ctx, peer) = (s.n.clone(), ctx.clone(), peer.clone());
                    let h = s.n.spawn_task(async move {
                        tokio::select! {
                            _ = ctx.cancelled() => {}
                            _ = n.serve_stream(&peer, &label, st) => {}
                        }
                    });
                    let mut sv = s.serving.lock().unwrap_or_else(|e| e.into_inner());
                    sv.retain(|t| !t.is_finished());
                    sv.push(h);
                }
            })
        };
        let read = {
            let (s, ctx) = (self.clone(), ctx.clone());
            self.n.spawn_task(async move {
                tokio::select! {
                    _ = ctx.cancelled() => {}
                    _ = s.read() => {}
                }
                ctx.cancel();
            })
        };
        tokio::select! {
            _ = ctx.cancelled() => {}
            _ = self.write() => {}
        }
        ctx.cancel();
        let _ = read.await;
        let _ = accept.await;
        self.lock().ended = true;
        let serving = std::mem::take(&mut *self.serving.lock().unwrap_or_else(|e| e.into_inner()));
        for h in serving {
            let _ = h.await;
        }
    }

    async fn send(&self, f: &Frame) -> Result<(), String> {
        self.conn.send(f.encode()).await
    }

    async fn write(&self) {
        let hello = self.n.hello(&self.conn.id());
        if self.send(&hello).await.is_err() {
            self.conn.close("hello");
            return;
        }
        let mut ctl = self.ctl_rx.lock().await;
        let start = tokio::time::Instant::now();
        let mut ping = tokio::time::interval_at(start + self.n.iv.ping, self.n.iv.ping);
        let mut tick = tokio::time::interval_at(start + self.n.iv.vector, self.n.iv.vector);
        loop {
            if let Ok(f) = ctl.try_recv() {
                if self.send(&f).await.is_err() {
                    return;
                }
                continue;
            }
            let frames = self.next().await;
            for f in &frames {
                if self.send(f).await.is_err() {
                    return;
                }
            }
            if !frames.is_empty() {
                continue;
            }
            tokio::select! {
                f = ctl.recv() => {
                    let Some(f) = f else { return };
                    if self.send(&f).await.is_err() {
                        return;
                    }
                }
                _ = self.wake.notified() => {}
                _ = ping.tick() => {
                    let f = Frame { t: hlc::unix_milli(self.n.now()), ..Frame::of_type(FRAME_PING) };
                    if self.send(&f).await.is_err() {
                        return;
                    }
                }
                _ = tick.tick() => {
                    if self.tick().await.is_err() {
                        return;
                    }
                }
            }
        }
    }

    /// Go `tick`: the periodic vector; retries devices the peer didn't take
    /// and gives up on an ack that never came.
    async fn tick(&self) -> Result<(), String> {
        let now = self.n.now();
        {
            let mut g = self.lock();
            g.stuck.clear();
            if g.inflight
                && g.sent_at
                    .is_some_and(|t| !within(now, t, 2 * self.n.iv.vector))
            {
                g.inflight = false;
                g.extent.clear();
            }
        }
        let Ok(v) = self.n.o.store.vector() else {
            return Ok(());
        };
        self.send(&Frame {
            vector: v,
            ..Frame::of_type(FRAME_VECTOR)
        })
        .await
    }

    /// Go `next`: the loose rows once after hello, then the next batch of
    /// Missing(peer's vector) when none is in flight.
    async fn next(&self) -> Vec<Frame> {
        let (first, mut v) = {
            let mut g = self.lock();
            if !g.hello_recv {
                return Vec::new();
            }
            if !g.sent_loose {
                g.sent_loose = true;
                (true, g.peer_vec.clone())
            } else if g.inflight {
                return Vec::new();
            } else {
                let mut v = g.peer_vec.clone();
                for d in &g.stuck {
                    v.insert(d.clone(), i64::MAX);
                }
                (false, v)
            }
        };
        if first {
            return self.loose(v).await;
        }
        if self.n.is_stopped(&self.n.self_id) {
            v.insert(self.n.self_id.clone(), i64::MAX); // forked: don't spread its rows
        }
        let st = self.n.o.store.clone();
        let evs = match blocking(move || st.missing(&v, MAX_BATCH_ROWS as i64)).await {
            Ok(evs) if !evs.is_empty() => evs,
            _ => return Vec::new(),
        };
        let mut f = batches(&evs, false).swap_remove(0);
        let mut ext: BTreeMap<String, (i64, i64)> = BTreeMap::new();
        for r in &f.events {
            let e = ext.entry(r.d.clone()).or_insert((r.s, r.s));
            e.1 = r.s;
        }
        let now = self.n.now();
        let mut g = self.lock();
        g.batch += 1;
        f.batch = g.batch;
        g.inflight = true;
        g.sent_at = Some(now);
        g.extent = ext;
        vec![f]
    }

    /// Go `loose`: every person.* row the peer lacks, honoured revokes kept
    /// in refused_events, and fork rows.
    async fn loose(&self, peer: Vector) -> Vec<Frame> {
        let st = self.n.o.store.clone();
        blocking(move || {
            let mut out = Vec::new();
            let mut f = Filter {
                kinds: vec!["person.".into()],
                limit: MAX_LIST_LIMIT,
                ..Filter::default()
            };
            loop {
                let evs = match st.list(&f) {
                    Ok(evs) if !evs.is_empty() => evs,
                    _ => break,
                };
                for ev in &evs {
                    if ev.seq > get(&peer, &ev.device_id) {
                        out.push(ev.clone());
                    }
                }
                f.after = evs.last().map(Event::cursor).unwrap_or_default();
            }
            if let Ok(rf) = st.refused(MAX_LIST_LIMIT) {
                out.extend(rf.into_iter().filter(|ev| {
                    ev.kind == identity::kinds::PERSON_DEVICE_REVOKED && !ev.sig.is_empty()
                }));
            }
            if let Ok(forks) = st.forks() {
                out.extend(
                    forks
                        .into_iter()
                        .filter(|fk| fk.reason == store::FORK_TWO_ROWS && !fk.other.sig.is_empty())
                        .map(|fk| fk.other),
                );
            }
            batches(&out, true)
        })
        .await
    }

    async fn read(self: &Arc<Self>) {
        loop {
            let Ok(b) = self.conn.recv().await else {
                return;
            };
            let f = match decode_frame(&b) {
                Ok(f) => f,
                Err(_) => {
                    self.conn.close("protocol");
                    return;
                }
            };
            if f.typ != FRAME_HELLO && !self.hello_done() {
                self.conn.close("protocol: frame before hello");
                return;
            }
            match f.typ.as_str() {
                FRAME_HELLO => {
                    if !self.on_hello(&f).await {
                        return;
                    }
                }
                FRAME_ROWS => self.on_rows(f).await,
                FRAME_VECTOR => self.on_vector(&f),
                FRAME_PING => {
                    self.n.alive(&self.conn.id());
                    self.control(Frame {
                        t: f.t,
                        ..Frame::of_type(FRAME_PONG)
                    });
                }
                FRAME_PONG => self.n.alive(&self.conn.id()),
                FRAME_PERSON_SECRET => {
                    if self.n.certified(&self.conn.id()) {
                        self.n.take_secret(&f);
                    }
                }
                FRAME_ROW_AT => {
                    // Only the asker's own rows: it is checking its history.
                    if f.device_id == self.conn.id()
                        && let Some(ev) = self.n.row_at(&f.device_id, f.seq)
                    {
                        self.control(Frame {
                            events: vec![Row::of(&ev)],
                            loose: true,
                            ..Frame::of_type(FRAME_ROWS)
                        });
                    }
                }
                _ => {} // a newer peer's frame: ignored
            }
        }
    }

    /// Go `onHello`: checks the peer and records its vector and epoch.
    async fn on_hello(self: &Arc<Self>, f: &Frame) -> bool {
        let n = &self.n;
        let peer = self.conn.id();
        if self.hello_done() {
            return true;
        }
        if f.v != PROTOCOL {
            self.conn.close("version");
            return false;
        }
        if f.device_id != peer || f.person_id != n.o.ident.current().id {
            self.conn.close("person");
            return false;
        }
        if f.schema < MIN_PEER_SCHEMA {
            n.lock().info(&peer).schema_old = true;
            self.conn.close("version");
            return false;
        }
        if !n.o.trust_any_peer {
            match n.o.chain.cert(&peer) {
                Some(c) if hex::encode(self.conn.pub_key()) == c.device_pub => {}
                _ => {
                    self.conn.close("key");
                    return false;
                }
            }
        }
        let now = n.now();
        let moved = {
            let mut g = n.lock();
            let k = g.info(&peer);
            let moved = k.epoch != f.epoch || k.pub_hex != f.person_pub;
            k.epoch = f.epoch;
            k.pub_hex = f.person_pub.clone();
            k.last_sync = Some(now);
            k.last_ping = Some(now);
            let now_ns = now
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_nanos() as i64)
                .unwrap_or(0);
            k.skew = f.wall_ms.saturating_mul(1_000_000).saturating_sub(now_ns);
            k.schema_old = false;
            moved
        };
        if moved {
            n.save_peers();
        }
        let mine = n.o.store.vector().unwrap_or_default();
        // The peer's copy of my latest row it holds must be the row I hold
        // at that seq; if not, this data dir was restored and wrote over
        // rows it had already sent: a fork.
        let (at, have) = (get(&f.vector, &n.self_id), get(&mine, &n.self_id));
        if !f.top.is_empty()
            && at > 0
            && at <= have
            && let Some(ev) = n.row_at(&n.self_id, at)
            && store::row_hash(&ev) != f.top
        {
            n.stop(&n.self_id, FORKED_SELF);
            self.control(Frame {
                events: vec![Row::of(&ev)],
                loose: true,
                ..Frame::of_type(FRAME_ROWS)
            });
        }
        // My own rows coming back: only a claim the peer proves with my own
        // signed row holds my appends.
        if at > have && !n.is_stopped(&n.self_id) {
            if !n.proves_own(f.top_row.as_deref(), at, &f.top) {
                n.lock().info(&peer).refused += 1;
                n.log(&format!(
                    "mesh: {peer} claims {at} of this device's rows without the row to prove it; ignored"
                ));
            } else {
                if have > 0 {
                    // The rows after mine are only mine if the peer's copy
                    // of my last row is the one I hold.
                    self.lock().own_check = have;
                    self.control(Frame {
                        device_id: n.self_id.clone(),
                        seq: have,
                        ..Frame::of_type(FRAME_ROW_AT)
                    });
                }
                n.o.store.hold_appends(at);
                n.restoring_from(&peer, at);
                n.log(&format!(
                    "mesh: restoring this device's rows up to {at} from {peer}"
                ));
            }
        }
        {
            let mut g = self.lock();
            g.hello_recv = true;
            g.p_epoch = f.epoch;
            g.p_pub = f.person_pub.clone();
            g.peer_vec = f.vector.clone();
        }
        n.note_vector(&peer, &f.vector);
        n.connected(&peer);
        n.send_secret(self); // if it is behind, or rotated apart
        n.kick(); // its epoch may close or open a room
        self.poke();
        true
    }

    fn on_vector(&self, f: &Frame) {
        let merged = {
            let mut g = self.lock();
            let old = g.peer_vec.clone();
            let mut merged = old.clone();
            for (d, v) in &f.vector {
                if *v > get(&merged, d) {
                    merged.insert(d.clone(), *v);
                }
            }
            g.stuck.retain(|d| get(&merged, d) <= get(&old, d));
            // An ack for an earlier batch says nothing about the one in
            // flight.
            if f.ack && g.inflight && (f.batch == 0 || f.batch == g.batch) {
                let ext = std::mem::take(&mut g.extent);
                for (d, (first, last)) in ext {
                    let got = get(&f.vector, &d);
                    if got < last && got >= first - 1 {
                        // The peer refused or held part of it: skip that
                        // device until it moves or the next tick.
                        g.stuck.insert(d);
                    }
                }
                g.inflight = false;
            }
            g.peer_vec = merged.clone();
            merged
        };
        self.n.note_vector(&self.conn.id(), &merged);
        self.poke();
    }

    /// Go `onRows`: applies a rows frame, then acks with this vector.
    async fn on_rows(self: &Arc<Self>, f: Frame) {
        if f.loose {
            // Not an ack: the sender's batch in flight is another frame.
            self.on_loose(f.events).await;
            self.vector(false, 0);
            return;
        }
        let mut order: Vec<String> = Vec::new();
        let mut runs: BTreeMap<String, Vec<Event>> = BTreeMap::new();
        for r in &f.events {
            if !runs.contains_key(&r.d) {
                order.push(r.d.clone());
            }
            runs.entry(r.d.clone()).or_default().push(r.event());
        }
        for dev in order {
            let evs = runs.remove(&dev).unwrap_or_default();
            if self.n.is_stopped(&dev) {
                continue;
            }
            if self.lock().held.has(&dev) || (dev == self.n.self_id && self.checking_own()) {
                self.lock().held.hold(&dev, &evs);
                continue;
            }
            let st = self.n.o.store.clone();
            let (evs, (applied, r)) = blocking(move || {
                let r = st.insert_batch(&evs);
                (evs, r)
            })
            .await;
            if let Err(e) = r {
                self.refusal(&dev, &evs[applied..], &e).await;
            }
        }
        self.n.synced(&self.conn.id());
        self.vector(true, f.batch);
    }

    /// Queues this device's vector; an ack answers a rows frame, which the
    /// sender's next batch waits for, and echoes its batch number.
    fn vector(&self, ack: bool, batch: i64) {
        let Ok(v) = self.n.o.store.vector() else {
            return;
        };
        self.control(Frame {
            vector: v,
            ack,
            batch,
            ..Frame::of_type(FRAME_VECTOR)
        });
    }

    /// Go `refusal`: the rest of a device's run after Insert refused
    /// `rest[0]`.
    async fn refusal(&self, dev: &str, rest: &[Event], err: &store::Error) {
        let n = &self.n;
        let self_id = n.self_id.as_str();
        match err.kind() {
            ErrorKind::NoCert => self.lock().held.hold(dev, rest),
            ErrorKind::Ahead => {
                let now = n.now();
                n.lock().ahead.insert(dev.to_string(), now);
            }
            ErrorKind::Fork | ErrorKind::HlcOrder if dev == self_id => {
                // My own rows from before a restore don't follow the ones I
                // wrote since: I signed two histories. My copy goes to the
                // peer too.
                n.stop(self_id, FORKED_SELF);
                let seq = rest.first().map(|e| e.seq).unwrap_or(0);
                let ev = n
                    .row_at(self_id, seq)
                    .or_else(|| n.row_at(self_id, seq - 1)); // an HLC refusal: the row before differs
                if let Some(ev) = ev {
                    self.control(Frame {
                        events: vec![Row::of(&ev)],
                        loose: true,
                        ..Frame::of_type(FRAME_ROWS)
                    });
                }
            }
            ErrorKind::Fork => n.stop(dev, "signed two different histories"),
            ErrorKind::HlcOrder => n.stop(dev, "its clock went backwards"),
            ErrorKind::Gap | ErrorKind::Closed => {}
            _ => {
                n.lock().info(&self.conn.id()).refused += 1;
                n.log(&format!(
                    "mesh: refused {dev}/{} from {}: {err}",
                    rest.first().map(|e| e.seq).unwrap_or(0),
                    self.conn.id()
                ));
            }
        }
    }

    /// Go `onLoose`: inserts each loose row on its own; a person row whose
    /// device's earlier rows aren't held yet is learned instead.
    async fn on_loose(self: &Arc<Self>, rows: Vec<Row>) {
        let n = &self.n;
        let mut learn: Vec<Event> = Vec::new();
        for r in rows {
            let ev = r.event();
            if n.is_stopped(&ev.device_id) {
                continue;
            }
            let st = n.o.store.clone();
            let (ev, res) = blocking(move || {
                let r = st.insert(&ev);
                (ev, r)
            })
            .await;
            if ev.device_id == n.self_id && res.is_ok() && self.pass_own_check(ev.seq) {
                // The peer's copy of my last row is mine: the rows after it
                // are my own history, held until now.
                let s = self.clone();
                n.spawn_task(async move { s.retry_held().await });
            }
            match res {
                Ok(()) => {}
                Err(e)
                    if store::is_person_kind(&ev.kind)
                        && (e.is(ErrorKind::Gap) || e.is(ErrorKind::NoCert)) =>
                {
                    learn.push(ev)
                }
                Err(e) if e.is(ErrorKind::NoCert) || e.is(ErrorKind::Gap) => {
                    // A fork row for a device not certified here, or ahead of
                    // what is held: it comes again.
                }
                Err(e) => {
                    self.refusal(&ev.device_id.clone(), std::slice::from_ref(&ev), &e)
                        .await
                }
            }
        }
        let mut learned = false;
        while !learn.is_empty() {
            let mut rest = Vec::new();
            for ev in &learn {
                match n.o.chain.learn(ev.into()) {
                    Ok(()) => learned = true,
                    Err(identity::LearnError::Verify(identity::VerifyError::NoCert(_))) => {
                        rest.push(ev.clone())
                    }
                    Err(_) => n.lock().info(&self.conn.id()).refused += 1,
                }
            }
            if rest.len() == learn.len() {
                break;
            }
            learn = rest;
        }
        if learned {
            n.kick();
        }
    }

    fn checking_own(&self) -> bool {
        self.lock().own_check > 0
    }

    /// Ends the own-row check if `seq` is the row it asked for.
    fn pass_own_check(&self, seq: i64) -> bool {
        let mut g = self.lock();
        if g.own_check == 0 || g.own_check != seq {
            return false;
        }
        g.own_check = 0;
        true
    }

    /// Go `retryHeld`: inserts held rows again, after a cert may have
    /// arrived.
    pub(crate) async fn retry_held(&self) {
        let held = {
            let mut g = self.lock();
            if g.held.by_dev.is_empty() {
                return;
            }
            let mut held = std::mem::take(&mut g.held);
            if g.own_check > 0
                && let Some(own) = held.by_dev.remove(&self.n.self_id)
            {
                // Still checking: my own rows stay held.
                let evs: Vec<Event> = own.into_values().collect();
                g.held.hold(&self.n.self_id, &evs);
            }
            held
        };
        let mut any = false;
        for (dev, m) in held.by_dev {
            if self.n.is_stopped(&dev) {
                continue;
            }
            let evs: Vec<Event> = m.into_values().collect(); // by seq
            let st = self.n.o.store.clone();
            let (evs, (applied, r)) = blocking(move || {
                let r = st.insert_batch(&evs);
                (evs, r)
            })
            .await;
            if applied > 0 {
                any = true;
            }
            if let Err(e) = r {
                self.refusal(&dev, &evs[applied..], &e).await;
            }
        }
        if any {
            self.vector(false, 0);
        }
    }
}

impl Node {
    /// Go `noteVector`: keeps the peer's newest vector, for "Removal
    /// pending".
    pub(crate) fn note_vector(&self, peer: &str, v: &Vector) {
        self.lock().info(peer).vec = v.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Go `TestHeldRowsAreCapped`.
    #[test]
    fn held_rows_are_capped() {
        let evs: Vec<Event> = (0..HELD_ROWS_CAP + 500)
            .map(|i| Event {
                device_id: "x".into(),
                seq: i as i64 + 1,
                payload: b"{}".to_vec(),
                ..Event::default()
            })
            .collect();
        let mut h = Held::default();
        h.hold("x", &evs);
        h.hold("x", &evs[..10]); // already held: not counted twice
        assert_eq!(h.count(), HELD_ROWS_CAP);
        let big = Event {
            device_id: "y".into(),
            seq: 1,
            payload: vec![0; HELD_BYTES_CAP + 1],
            ..Event::default()
        };
        let mut h2 = Held::default();
        h2.hold("y", &[big]);
        assert_eq!(h2.count(), 0, "a row past the byte cap was held");
    }
}
