//! The loopback iroh suite for the mesh transport (architecture §6.5,
//! mesh.md §6 "roomnet ... port every one of these to iroh"), with real
//! identities, Chains and signed logs from the identity test harness, an
//! embedded relay on 127.0.0.1, and every socket on loopback.
//!
//! These are the transport's half of the roomnet list: who connects, who is
//! refused, which connection stays, the 8 MiB cap, streams, blobs, board
//! pulls and gossip, relays. Rows move with [`MiniSync`], a test-only vector
//! exchange that only drives traffic. The protocol itself (the real mesh
//! Node's session: hello, rotation and its tie-break over `person.secret`,
//! restore's `provesOwn`, the fork's `row.at`) runs on `IrohNet` in
//! `node_iroh.rs`.

#[path = "../../identity/tests/support/mod.rs"]
mod support;

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use iroh::Watcher as _;
use irori_identity::{BOARD_SECRET_SIZE, DeviceIdentity, Row, derive_board_topic, kinds};
use irori_meshseam::{
    Conn, ConnError, MAX_MESSAGE, Room, RoomConfig, Stream, Transport, mesh_secret, read_frame,
    reason, write_frame,
};
use irori_net::admit::{ChainAccess, ChainAdmit, check_device};
use irori_net::endpoint::{IpMode, RelaySpec};
use irori_net::identity::IrohKey as _;
use irori_net::relay::{self, EmbeddedRelay};
use irori_net::transport::{IrohNet, NetConfig, PeerAddr, finish_with};
use support::{Node, copy_rows};
use tokio::io::AsyncReadExt as _;

const LO: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

async fn wait_until(what: &str, within: Duration, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        if start.elapsed() > within {
            panic!("timed out waiting for {what}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

const WAIT: Duration = Duration::from_secs(20);

// ---- MiniSync: a test-only stand-in for the mesh session ----

fn row_json(r: &Row) -> serde_json::Value {
    serde_json::json!({
        "d": r.device_id, "s": r.seq, "h": r.hlc, "a": r.actor, "k": r.kind,
        "p": B64.encode(&r.payload), "sig": B64.encode(&r.sig),
    })
}

fn json_row(v: &serde_json::Value) -> Row {
    Row {
        device_id: v["d"].as_str().unwrap().into(),
        seq: v["s"].as_i64().unwrap(),
        hlc: v["h"].as_u64().unwrap(),
        actor: v["a"].as_str().unwrap().into(),
        kind: v["k"].as_str().unwrap().into(),
        payload: B64.decode(v["p"].as_str().unwrap()).unwrap(),
        sig: B64.decode(v["sig"].as_str().unwrap()).unwrap(),
    }
}

/// One device under test: its identity, Chain and log, its transport, the
/// rooms it joined and the connections it saw.
struct Dev {
    node: Arc<Mutex<Node>>,
    id: String,
    net: IrohNet,
    rooms: Mutex<BTreeMap<i64, Arc<dyn Room>>>,
    conns: Arc<Mutex<Vec<Arc<dyn Conn>>>>,
    refusals: Arc<Mutex<Vec<support::StoreErr>>>,
    batches: Arc<AtomicUsize>,
    _dir: tempfile::TempDir,
}

impl Dev {
    fn new(join: Option<&Dev>, ip: IpMode) -> Dev {
        let dir = tempfile::tempdir().unwrap();
        let node = match join {
            Some(j) => Node::new(dir.path(), Some(&j.node.lock().unwrap())),
            None => Node::new(dir.path(), None),
        };
        Self::from_node(node, dir, ip)
    }

    fn from_node(node: Node, dir: tempfile::TempDir, ip: IpMode) -> Dev {
        Self::from_shared(Arc::new(Mutex::new(node)), dir, ip, None)
    }

    /// `admit` replaces the Chain gate (a stranger that would let anyone in).
    fn from_shared(
        node: Arc<Mutex<Node>>,
        dir: tempfile::TempDir,
        ip: IpMode,
        admit: Option<Arc<dyn irori_net::admit::Admit>>,
    ) -> Dev {
        let (id, key, chain) = {
            let n = node.lock().unwrap();
            (
                n.dev(),
                iroh::SecretKey::from_bytes(&n.id.expose_device_seed()),
                n.chain.clone(),
            )
        };
        let admit = admit.unwrap_or_else(|| {
            Arc::new(ChainAdmit {
                chain,
                self_id: id.clone(),
            })
        });
        let mut cfg = NetConfig::new(id.clone(), key, admit);
        cfg.ip = ip;
        cfg.dial_grace = Duration::from_millis(300);
        cfg.connect_timeout = Duration::from_secs(10);
        Dev {
            node,
            id,
            net: IrohNet::new(cfg),
            rooms: Mutex::new(BTreeMap::new()),
            conns: Arc::new(Mutex::new(Vec::new())),
            refusals: Arc::new(Mutex::new(Vec::new())),
            batches: Arc::new(AtomicUsize::new(0)),
            _dir: dir,
        }
    }

    fn pubkey(&self) -> [u8; 32] {
        self.node.lock().unwrap().id.device_pub()
    }

    fn device_identity(&self) -> DeviceIdentity {
        DeviceIdentity::from_seed_for_tests(*self.node.lock().unwrap().id.expose_device_seed())
    }

    fn vector(&self) -> BTreeMap<String, i64> {
        self.node.lock().unwrap().st.vector()
    }

    fn append(&self, n: usize, size: usize) {
        let node = self.node.lock().unwrap();
        for i in 0..n {
            let body = serde_json::json!({"i": i, "pad": "x".repeat(size)});
            node.st.append("agent:test", "task.commented", &body);
        }
    }

    fn secret(&self, epoch: i64) -> irori_meshseam::Secret {
        let node = self.node.lock().unwrap();
        let seed = node.id.person_seed(epoch).expect("person seed");
        mesh_secret(&seed[..], &node.id.current().id, epoch)
    }

    fn room_config(&self) -> RoomConfig {
        let mut cfg = RoomConfig::new(self.id.clone());
        let node = self.node.clone();
        let me = self.id.clone();
        cfg.accept = Arc::new(move |peer, pubkey| {
            let chain = node.lock().unwrap().chain.clone();
            check_device(&chain, &me, peer, pubkey)
        });
        let (node, conns, refusals, batches, net) = (
            self.node.clone(),
            self.conns.clone(),
            self.refusals.clone(),
            self.batches.clone(),
            self.net.clone(),
        );
        cfg.on_peer = Arc::new(move |c: Arc<dyn Conn>| {
            conns.lock().unwrap().push(c.clone());
            tokio::spawn(MiniSync::run(
                node.clone(),
                c,
                refusals.clone(),
                batches.clone(),
                net.clone(),
            ));
        });
        cfg
    }

    async fn join(&self, epoch: i64) -> Arc<dyn Room> {
        self.join_with(self.secret(epoch), self.room_config()).await
    }

    async fn join_with(&self, s: irori_meshseam::Secret, cfg: RoomConfig) -> Arc<dyn Room> {
        let epoch = s.epoch;
        let room = self.net.join(s, cfg).await.unwrap();
        self.rooms.lock().unwrap().insert(epoch, room.clone());
        room
    }

    fn room(&self, epoch: i64) -> Arc<dyn Room> {
        self.rooms.lock().unwrap()[&epoch].clone()
    }

    fn live_to(&self, peer: &str) -> Vec<Arc<dyn Conn>> {
        self.conns
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.id() == peer && c.err().is_none())
            .cloned()
            .collect()
    }

    fn peer_addr(&self, relay: Option<&EmbeddedRelay>) -> PeerAddr {
        let direct = match self.net.endpoint() {
            Some(ep) => ep.bound_sockets(),
            None => vec![],
        };
        PeerAddr {
            device_id: self.id.clone(),
            id: iroh::EndpointId::from_bytes(&self.pubkey()).unwrap(),
            relay: relay.map(|r| r.url_for(LO)),
            direct,
        }
    }
}

fn assert_loopback(d: &Dev) {
    if let Some(ep) = d.net.endpoint() {
        for a in ep.bound_sockets() {
            assert!(a.ip().is_loopback(), "non-loopback socket {a}");
        }
    }
}

/// A person with `n` devices: the first is the root and certifies the rest;
/// every log holds every cert.
fn person(n: usize, ip: IpMode) -> Vec<Dev> {
    let a = Dev::new(None, ip);
    let mut devs = vec![a];
    for _ in 1..n {
        let d = Dev::new(Some(&devs[0]), ip);
        devs[0]
            .node
            .lock()
            .unwrap()
            .certify(&d.node.lock().unwrap());
        devs.push(d);
    }
    for i in 0..n {
        for j in 0..n {
            if i != j {
                copy_rows(&devs[i].node.lock().unwrap(), &devs[j].node.lock().unwrap()).unwrap();
            }
        }
    }
    devs
}

/// Bind every device, then give each the others' addresses.
async fn introduce(devs: &[&Dev], relay: Option<&EmbeddedRelay>) {
    for d in devs {
        d.net.bind().await.unwrap();
        assert_loopback(d);
    }
    let addrs: Vec<PeerAddr> = devs.iter().map(|d| d.peer_addr(relay)).collect();
    for d in devs {
        d.net.set_peers(
            addrs
                .iter()
                .filter(|a| a.device_id != d.id)
                .cloned()
                .collect(),
        );
    }
}

async fn home_relay(server: &Dev) -> (EmbeddedRelay, RelaySpec) {
    let chain = server.node.lock().unwrap().chain.clone();
    let ident = server.device_identity();
    let access = ChainAccess {
        admit: ChainAdmit {
            chain,
            self_id: server.id.clone(),
        },
        self_pub: ident.endpoint_id(),
    };
    let r = relay::spawn_with(&ident, LO, 0, access).await.unwrap();
    assert!(r.server.https_addr().unwrap().ip().is_loopback());
    assert!(r.server.quic_addr().unwrap().ip().is_loopback());
    let spec = RelaySpec {
        url: r.url_for(LO),
        quic_port: r.port,
        server_pub: server.device_identity().endpoint_id(),
    };
    (r, spec)
}

struct MiniSync;

impl MiniSync {
    async fn run(
        node: Arc<Mutex<Node>>,
        c: Arc<dyn Conn>,
        refusals: Arc<Mutex<Vec<support::StoreErr>>>,
        batches: Arc<AtomicUsize>,
        net: IrohNet,
    ) {
        let vec_msg = |node: &Arc<Mutex<Node>>| {
            let v = node.lock().unwrap().st.vector();
            serde_json::to_vec(&serde_json::json!({"t": "v", "v": v})).unwrap()
        };
        if c.send(&vec_msg(&node)).await.is_err() {
            return;
        }
        let ticker = {
            let (node, c) = (node.clone(), c.clone());
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    if c.send(&vec_msg(&node)).await.is_err() {
                        return;
                    }
                }
            })
        };
        while let Ok(m) = c.recv().await {
            let v: serde_json::Value = serde_json::from_slice(&m).unwrap();
            match v["t"].as_str().unwrap() {
                "v" => {
                    let have: BTreeMap<String, i64> =
                        serde_json::from_value(v["v"].clone()).unwrap();
                    let missing = node.lock().unwrap().st.missing(&have);
                    for chunk in missing.chunks(500) {
                        let rows: Vec<_> = chunk.iter().map(row_json).collect();
                        let msg =
                            serde_json::to_vec(&serde_json::json!({"t": "r", "e": rows})).unwrap();
                        if c.send(&msg).await.is_err() {
                            ticker.abort();
                            return;
                        }
                    }
                }
                "r" => {
                    batches.fetch_add(1, Ordering::SeqCst);
                    let rows: Vec<Row> = v["e"].as_array().unwrap().iter().map(json_row).collect();
                    let person_rows = {
                        let node = node.lock().unwrap();
                        let mut person: Vec<&Row> = rows
                            .iter()
                            .filter(|r| kinds::is_person_kind(&r.kind))
                            .collect();
                        let mut progress = true;
                        while progress {
                            progress = false;
                            person.retain(|r| {
                                let ok = node.chain.learn(r.as_ref()).is_ok();
                                progress |= ok;
                                !ok
                            });
                        }
                        for r in &rows {
                            if let Err(e) = node.st.insert(r) {
                                refusals.lock().unwrap().push(e);
                            }
                        }
                        rows.iter().any(|r| kinds::is_person_kind(&r.kind))
                    };
                    if person_rows {
                        // What the Node does on every person row: drop the
                        // connections the Chain no longer admits.
                        net.recheck();
                    }
                }
                _ => {}
            }
        }
        ticker.abort();
    }
}

fn caught_up(devs: &[&Dev]) -> bool {
    let v0 = devs[0].vector();
    devs.iter().all(|d| d.vector() == v0)
}

// ---- tests ----

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_opens_before_the_first_join() {
    let devs = person(2, IpMode::Loopback);
    assert!(devs[0].net.endpoint().is_none());
    assert!(devs[0].net.rooms().is_empty());
    devs[0].join(1).await;
    assert!(devs[0].net.endpoint().is_some());
    assert_loopback(&devs[0]);
    assert_eq!(devs[0].net.rooms(), vec![1]);
    devs[0].net.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_nodes_sync_through_the_home_relay_alone() {
    let devs = person(2, IpMode::RelayOnly);
    let (a, b) = (&devs[0], &devs[1]);
    // A is the home server: its relay, pinned to A's device key.
    let (relay, spec) = home_relay(a).await;
    for d in [a, b] {
        d.net.set_servers(vec![spec.clone()]).await;
    }
    a.append(30, 100);
    b.append(20, 100);
    a.join(1).await;
    b.join(1).await;
    introduce(&[a, b], Some(&relay)).await;
    for d in [a, b] {
        let ep = d.net.endpoint().unwrap().clone();
        wait_until("home relay", WAIT, || {
            ep.home_relay_status()
                .get()
                .iter()
                .any(|s| s.is_connected())
        })
        .await;
    }
    b.room(1)
        .send("", irori_meshseam::MSG_PRESENCE, serde_json::json!({}))
        .unwrap();
    a.room(1)
        .send("", irori_meshseam::MSG_PRESENCE, serde_json::json!({}))
        .unwrap();
    wait_until("rows both ways", WAIT, || caught_up(&[a, b])).await;
    let c = a.live_to(&b.id).pop().unwrap();
    assert_eq!(c.path(), "relay");
    assert_eq!(c.epoch(), 1);
    let (fa, fb) = c.fingerprints();
    let cb = b.live_to(&a.id).pop().unwrap();
    assert_eq!(
        (fa.clone(), fb),
        cb.fingerprints(),
        "EKM differs between the ends"
    );
    assert_eq!(fa.len(), 32 * 3 - 1);
    // Live rows keep flowing.
    b.append(5, 10);
    wait_until("live rows", WAIT, || caught_up(&[a, b])).await;
    assert_eq!(a.vector()[&b.id], b.vector()[&b.id]);
    for d in [a, b] {
        d.net.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn backfill_in_batches_on_a_direct_path() {
    let devs = person(2, IpMode::Loopback);
    let (a, b) = (&devs[0], &devs[1]);
    a.append(1300, 4000);
    a.join(1).await;
    b.join(1).await;
    introduce(&[a, b], None).await;
    a.room(1).connect(&b.id);
    b.room(1).connect(&a.id);
    wait_until("1300 rows", Duration::from_secs(60), || caught_up(&[a, b])).await;
    assert!(
        b.batches.load(Ordering::SeqCst) >= 3,
        "1300 rows came in fewer than 3 batches"
    );
    let c = b.live_to(&a.id).pop().unwrap();
    assert_eq!(c.path(), "direct");
    assert!(c.rtt() < Duration::from_secs(1));
    for d in [a, b] {
        d.net.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_third_node_carries_history_while_the_first_is_gone() {
    let devs = person(3, IpMode::Loopback);
    let (a, b, c) = (&devs[0], &devs[1], &devs[2]);
    a.append(40, 50);
    for d in [a, b, c] {
        d.join(1).await;
    }
    introduce(&[a, b, c], None).await;
    // A meets only C.
    a.room(1).connect(&c.id);
    c.room(1).connect(&a.id);
    wait_until("A to C", WAIT, || caught_up(&[a, c])).await;
    let a_seq = a.vector()[&a.id];
    assert!(b.vector().get(&a.id) < Some(&a_seq));
    a.net.close().await;
    wait_until("A's close reaches C", WAIT, || c.live_to(&a.id).is_empty()).await;
    let gone = c
        .conns
        .lock()
        .unwrap()
        .iter()
        .find(|x| x.id() == a.id)
        .unwrap()
        .err()
        .unwrap();
    assert_eq!(gone, ConnError::Remote(reason::SHUTTING_DOWN.into()));
    assert!(!gone.retry());
    // B gets A's rows from C.
    b.room(1).connect(&c.id);
    c.room(1).connect(&b.id);
    wait_until("B from C", WAIT, || b.vector().get(&a.id) == Some(&a_seq)).await;
    for d in [b, c] {
        d.net.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn duplicate_connections_keep_the_one_the_lower_id_dialed() {
    let devs = person(2, IpMode::Loopback);
    let (a, b) = (&devs[0], &devs[1]);
    // Both sides offer, so both dial at once.
    for d in [a, b] {
        let mut cfg = d.room_config();
        cfg.offerer = Some(Arc::new(|_| true));
        d.join_with(d.secret(1), cfg).await;
    }
    introduce(&[a, b], None).await;
    a.room(1).connect(&b.id);
    b.room(1).connect(&a.id);
    let lower = std::cmp::min(a.id.clone(), b.id.clone());
    wait_until("one connection", WAIT, || {
        let v = a.net.conns();
        v.len() == 1 && v[0].dialer == lower && b.net.conns().len() == 1
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (ca, cb) = (a.net.conns(), b.net.conns());
    assert_eq!((ca.len(), cb.len()), (1, 1));
    assert_eq!(ca[0].dialer, lower);
    assert_eq!(cb[0].dialer, lower);
    // Every other connection that came up was closed `replaced`, and is not retried.
    for d in [a, b] {
        for c in d.conns.lock().unwrap().iter() {
            if let Some(e) = c.err() {
                assert!(e.replaced(), "{e}");
                assert!(!e.retry());
            }
        }
    }
    for d in [a, b] {
        d.net.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_uncertified_device_is_refused_before_any_stream() {
    let devs = person(1, IpMode::Loopback);
    let a = &devs[0];
    // A stranger: its own person, with A's address. It allows everyone.
    let stranger = {
        let dir = tempfile::tempdir().unwrap();
        let node = Node::new(dir.path(), None);
        Dev::from_shared(
            Arc::new(Mutex::new(node)),
            dir,
            IpMode::Loopback,
            Some(Arc::new(irori_net::admit::AdmitAll)),
        )
    };
    let mut cfg = RoomConfig::new(stranger.id.clone());
    cfg.offerer = Some(Arc::new(|_| true));
    let s = a.secret(1); // even holding the epoch key does not help
    stranger.join_with(s, cfg).await;
    a.join(1).await;
    introduce(&[a, &stranger], None).await;
    stranger.room(1).connect(&a.id);
    wait_until("a dial error", WAIT, || {
        stranger.net.dial_errors().contains_key(&a.id)
    })
    .await;
    let e = stranger.net.dial_errors()[&a.id].clone();
    assert!(e.contains(reason::REFUSED), "{e}");
    assert!(a.net.conns().is_empty());
    for d in [a, &stranger] {
        d.net.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_closes_the_live_connection_and_refuses_the_redial() {
    let devs = person(3, IpMode::Loopback);
    let (a, b, c) = (&devs[0], &devs[1], &devs[2]);
    for d in [a, b, c] {
        d.join(1).await;
    }
    introduce(&[a, b, c], None).await;
    for (x, y) in [(a, b), (a, c), (b, c)] {
        x.room(1).connect(&y.id);
        y.room(1).connect(&x.id);
    }
    wait_until("full mesh", WAIT, || {
        a.net.conns().len() == 2 && b.net.conns().len() == 2 && c.net.conns().len() == 2
    })
    .await;
    // A revokes C at its current seq; the revoke reaches B by sync.
    let cut = c.vector().get(&c.id).copied().unwrap_or(0);
    a.node.lock().unwrap().revoke(&c.id, cut);
    a.net.recheck();
    wait_until("C dropped everywhere", WAIT, || {
        a.live_to(&c.id).is_empty() && b.live_to(&c.id).is_empty()
    })
    .await;
    let on_c: Vec<ConnError> = c
        .conns
        .lock()
        .unwrap()
        .iter()
        .filter_map(|x| x.err())
        .collect();
    assert!(
        on_c.iter()
            .all(|e| e.reason() == reason::REVOKED && !e.retry()),
        "{on_c:?}"
    );
    assert!(b.node.lock().unwrap().chain.cut(&c.id).is_some());
    // C's redial is refused at the handshake.
    c.room(1).connect(&a.id);
    c.room(1).connect(&b.id);
    wait_until("redials refused", WAIT, || {
        let e = c.net.dial_errors();
        e.get(&a.id).is_some_and(|x| x.contains(reason::REFUSED))
            && e.get(&b.id).is_some_and(|x| x.contains(reason::REFUSED))
    })
    .await;
    assert_eq!(a.net.conns().len(), 1);
    for d in [a, b, c] {
        d.net.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sync_messages_keep_the_8_mib_cap() {
    let devs = person(2, IpMode::Loopback);
    let (a, b) = (&devs[0], &devs[1]);
    let got = Arc::new(Mutex::new(Vec::<usize>::new()));
    let peers = Arc::new(Mutex::new(Vec::<Arc<dyn Conn>>::new()));
    for d in [a, b] {
        let mut cfg = d.room_config();
        let (got, peers) = (got.clone(), peers.clone());
        cfg.on_peer = Arc::new(move |c: Arc<dyn Conn>| {
            peers.lock().unwrap().push(c.clone());
            let got = got.clone();
            tokio::spawn(async move {
                while let Ok(m) = c.recv().await {
                    got.lock().unwrap().push(m.len());
                }
            });
        });
        d.join_with(d.secret(1), cfg).await;
    }
    introduce(&[a, b], None).await;
    a.room(1).connect(&b.id);
    wait_until("connected", WAIT, || peers.lock().unwrap().len() == 2).await;
    let c = peers
        .lock()
        .unwrap()
        .iter()
        .find(|c| c.id() == b.id)
        .unwrap()
        .clone();
    assert!(c.send(&vec![1u8; MAX_MESSAGE + 1]).await.is_err());
    c.send(&vec![2u8; MAX_MESSAGE]).await.unwrap();
    c.send(b"").await.unwrap();
    wait_until("both messages", WAIT, || got.lock().unwrap().len() == 2).await;
    assert_eq!(*got.lock().unwrap(), vec![MAX_MESSAGE, 0]);
    assert!(c.err().is_none());
    for d in [a, b] {
        d.net.close().await;
    }
}

// ---- blobs: `blob/<16hex>` streams (files.go, mesh.md §2.3) ----

/// Serve `blob/…` requests from `held` (sha256 hex → bytes); a hash not
/// held is refused with zero bytes.
fn serve_blobs(c: &Arc<dyn Conn>, held: Arc<BTreeMap<String, Vec<u8>>>) {
    c.on_stream(Arc::new(move |mut s: Stream| {
        let held = held.clone();
        tokio::spawn(async move {
            if !s.label.starts_with("blob/") {
                return;
            }
            let Ok(Some(req)) = read_frame(&mut s.recv, 4096, false).await else {
                return;
            };
            let req: serde_json::Value = serde_json::from_slice(&req).unwrap();
            let sha = req["sha256"].as_str().unwrap_or("");
            let Some(data) = held.get(sha) else {
                let _ = finish_with(&mut s, &[]).await;
                return;
            };
            let off = (req["offset"].as_u64().unwrap() as usize).min(data.len());
            let len = req["len"].as_u64().unwrap() as usize;
            let end = if len == 0 {
                data.len()
            } else {
                (off + len).min(data.len())
            };
            for chunk in data[off..end].chunks(16 << 10) {
                use tokio::io::AsyncWriteExt as _;
                if s.send.write_all(chunk).await.is_err() {
                    return;
                }
            }
            let _ = finish_with(&mut s, &[]).await;
        });
    }));
}

async fn fetch_blob(c: &Arc<dyn Conn>, sha: &str, offset: u64, len: u64) -> Vec<u8> {
    let mut id = [0u8; 8];
    getrandom::fill(&mut id).unwrap();
    let mut s = c
        .open_stream(&format!("blob/{}", hex::encode(id)))
        .await
        .unwrap();
    let req =
        serde_json::json!({"id": hex::encode(id), "sha256": sha, "offset": offset, "len": len});
    write_frame(&mut s.send, &serde_json::to_vec(&req).unwrap(), 4096)
        .await
        .unwrap();
    use tokio::io::AsyncWriteExt as _;
    s.send.shutdown().await.unwrap();
    let mut out = Vec::new();
    s.recv.read_to_end(&mut out).await.unwrap();
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blobs_cross_with_resume_and_zero_byte_refusal() {
    let devs = person(2, IpMode::Loopback);
    let (a, b) = (&devs[0], &devs[1]);
    let mut data = vec![0u8; 300_000];
    getrandom::fill(&mut data).unwrap();
    let sha = {
        use sha2::Digest as _;
        hex::encode(sha2::Sha256::digest(&data))
    };
    let held = Arc::new(BTreeMap::from([(sha.clone(), data.clone())]));
    let peers = Arc::new(Mutex::new(Vec::<Arc<dyn Conn>>::new()));
    for d in [a, b] {
        let mut cfg = d.room_config();
        let (peers, held, me) = (peers.clone(), held.clone(), d.id.clone());
        let a_id = a.id.clone();
        cfg.on_peer = Arc::new(move |c: Arc<dyn Conn>| {
            if me == a_id {
                serve_blobs(&c, held.clone());
            }
            peers.lock().unwrap().push(c);
        });
        d.join_with(d.secret(1), cfg).await;
    }
    introduce(&[a, b], None).await;
    b.room(1).connect(&a.id);
    a.room(1).connect(&b.id);
    wait_until("connected", WAIT, || peers.lock().unwrap().len() == 2).await;
    let to_a = peers
        .lock()
        .unwrap()
        .iter()
        .find(|c| c.id() == a.id)
        .unwrap()
        .clone();
    // Whole file, then a resume from an offset, then a bounded range.
    assert_eq!(fetch_blob(&to_a, &sha, 0, 0).await, data);
    assert_eq!(fetch_blob(&to_a, &sha, 200_000, 0).await, data[200_000..]);
    assert_eq!(fetch_blob(&to_a, &sha, 10, 5).await, data[10..15]);
    // A hash A does not hold: zero bytes.
    assert!(fetch_blob(&to_a, &"0".repeat(64), 0, 0).await.is_empty());
    // More than 4 fetches at once queue on the outgoing slots and all finish.
    let mut set = tokio::task::JoinSet::new();
    for _ in 0..10 {
        let (c, sha) = (to_a.clone(), sha.clone());
        set.spawn(async move { fetch_blob(&c, &sha, 0, 1000).await.len() });
    }
    while let Some(n) = set.join_next().await {
        assert_eq!(n.unwrap(), 1000);
    }
    for d in [a, b] {
        d.net.close().await;
    }
}

// ---- boards stage 1: `board/<id>` pulls, announced on gossip ----

const BOARD_PULL_ROWS: usize = 2000;

/// Serve `board/<id>` pulls from `rows` (stage 1: the board log moves
/// inside the private mesh connection, boards.go).
fn serve_boards(c: &Arc<dyn Conn>, board: String, rows: Arc<Vec<serde_json::Value>>) {
    c.on_stream(Arc::new(move |mut s: Stream| {
        let (board, rows) = (board.clone(), rows.clone());
        tokio::spawn(async move {
            if s.label != format!("board/{board}") {
                let _ = finish_with(&mut s, &[]).await;
                return;
            }
            let Ok(Some(req)) = read_frame(&mut s.recv, 1 << 20, false).await else {
                return;
            };
            let req: serde_json::Value = serde_json::from_slice(&req).unwrap();
            let have = req["vector"]["dev"].as_i64().unwrap_or(0) as usize;
            let missing: Vec<_> = rows
                .iter()
                .skip(have)
                .take(BOARD_PULL_ROWS)
                .cloned()
                .collect();
            for chunk in missing.chunks(500) {
                let b = serde_json::to_vec(&serde_json::json!({"events": chunk})).unwrap();
                if write_frame(&mut s.send, &b, 1 << 20).await.is_err() {
                    return;
                }
            }
            let _ = finish_with(&mut s, &[]).await;
        });
    }));
}

async fn pull_board(c: &Arc<dyn Conn>, board: &str, have: usize) -> Vec<serde_json::Value> {
    let mut s = c.open_stream(&format!("board/{board}")).await.unwrap();
    let req = serde_json::json!({"vector": {"dev": have}});
    write_frame(&mut s.send, &serde_json::to_vec(&req).unwrap(), 1 << 20)
        .await
        .unwrap();
    use tokio::io::AsyncWriteExt as _;
    s.send.shutdown().await.unwrap();
    let mut out = Vec::new();
    while let Some(chunk) = read_frame(&mut s.recv, 1 << 20, false).await.unwrap() {
        let v: serde_json::Value = serde_json::from_slice(&chunk).unwrap();
        out.extend(v["events"].as_array().unwrap().iter().cloned());
        if out.len() >= BOARD_PULL_ROWS {
            break;
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn boards_pull_in_chunks_after_a_gossip_announcement_across_three_nodes() {
    let devs = person(3, IpMode::Loopback);
    let (a, b, c) = (&devs[0], &devs[1], &devs[2]);
    let board = "0123456789abcdef0123456789abcdef".to_string();
    let rows: Arc<Vec<serde_json::Value>> = Arc::new(
        (0..2600)
            .map(|i| serde_json::json!({"d": "dev", "s": i + 1, "k": "task.created", "p": "e30="}))
            .collect(),
    );
    let peers = Arc::new(Mutex::new(Vec::<(String, Arc<dyn Conn>)>::new()));
    for d in [a, b, c] {
        let mut cfg = d.room_config();
        let (peers, rows, board, me, a_id) = (
            peers.clone(),
            rows.clone(),
            board.clone(),
            d.id.clone(),
            a.id.clone(),
        );
        cfg.on_peer = Arc::new(move |conn: Arc<dyn Conn>| {
            if me == a_id {
                serve_boards(&conn, board.clone(), rows.clone());
            }
            peers.lock().unwrap().push((me.clone(), conn));
        });
        d.join_with(d.secret(1), cfg).await;
    }
    introduce(&[a, b, c], None).await;
    // Every device subscribes to the board's gossip topic; C knows only B.
    let mut secret = [0u8; BOARD_SECRET_SIZE];
    getrandom::fill(&mut secret).unwrap();
    let topic = derive_board_topic(&board, 1, &secret).unwrap();
    let ch_a = a
        .net
        .board_channel(&board, &topic, std::slice::from_ref(&b.id))
        .await
        .unwrap();
    let mut ch_b = b
        .net
        .board_channel(&board, &topic, std::slice::from_ref(&a.id))
        .await
        .unwrap();
    let mut ch_c = c
        .net
        .board_channel(&board, &topic, std::slice::from_ref(&b.id))
        .await
        .unwrap();
    // A announces until C hears it (the topic needs a moment to form).
    let vector = BTreeMap::from([("dev".to_string(), rows.len() as i64)]);
    let heard = tokio::time::timeout(WAIT, async {
        loop {
            ch_a.announce(vector.clone()).await.unwrap();
            if let Ok(Some(x)) = tokio::time::timeout(Duration::from_millis(300), ch_c.next()).await
            {
                break x;
            }
        }
    })
    .await
    .expect("C heard A's announcement");
    assert_eq!(heard.board_id, board);
    assert_eq!(heard.vector, vector);
    assert!(ch_b.next().await.is_some());
    // C pulls from A over the mesh: 2000 rows per pull, in chunks.
    c.room(1).connect(&a.id);
    a.room(1).connect(&c.id);
    wait_until("C to A", WAIT, || {
        peers
            .lock()
            .unwrap()
            .iter()
            .any(|(me, x)| *me == c.id && x.id() == a.id)
    })
    .await;
    let to_a = peers
        .lock()
        .unwrap()
        .iter()
        .find(|(me, x)| *me == c.id && x.id() == a.id)
        .unwrap()
        .1
        .clone();
    let first = pull_board(&to_a, &board, 0).await;
    assert_eq!(first.len(), BOARD_PULL_ROWS);
    let rest = pull_board(&to_a, &board, first.len()).await;
    assert_eq!(first.len() + rest.len(), rows.len());
    assert_eq!(rest.last().unwrap()["s"], rows.len());
    // Another board's label is refused with zero bytes.
    assert!(pull_board(&to_a, &"f".repeat(32), 0).await.is_empty());
    // A device holding a different board secret computes another topic.
    let other = derive_board_topic(&board, 1, &[9u8; BOARD_SECRET_SIZE]).unwrap();
    assert_ne!(
        irori_net::gossip::topic_id(&other),
        irori_net::gossip::topic_id(&topic)
    );
    drop((ch_a, ch_b, ch_c));
    for d in [a, b, c] {
        d.net.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_relay_learned_after_bind_is_pinned_and_carries_the_mesh() {
    let devs = person(2, IpMode::RelayOnly);
    let (a, b) = (&devs[0], &devs[1]);
    a.join(1).await;
    b.join(1).await;
    // Bound with no server known; the relay comes from a node.addr row later.
    let (relay, spec) = home_relay(a).await;
    for d in [a, b] {
        assert!(
            d.net
                .endpoint()
                .unwrap()
                .home_relay_status()
                .get()
                .is_empty()
        );
        d.net.set_servers(vec![spec.clone()]).await;
    }
    introduce(&[a, b], Some(&relay)).await;
    for d in [a, b] {
        let ep = d.net.endpoint().unwrap().clone();
        wait_until("relay after bind", WAIT, || {
            ep.home_relay_status()
                .get()
                .iter()
                .any(|s| s.is_connected())
        })
        .await;
    }
    b.room(1).connect(&a.id);
    a.room(1).connect(&b.id);
    a.append(3, 10);
    wait_until("sync via the late relay", WAIT, || caught_up(&[a, b])).await;
    assert_eq!(b.live_to(&a.id).pop().unwrap().path(), "relay");
    for d in [a, b] {
        d.net.close().await;
    }
}

#[test]
fn the_endpoint_ids_are_the_device_keys() {
    let devs = person(1, IpMode::Loopback);
    let d = &devs[0];
    assert_eq!(d.device_identity().endpoint_id().as_bytes(), &d.pubkey());
    let _unused: SocketAddr = SocketAddr::new(LO, 0);
}
