//! Voice calls on loopback iroh (#654): `irori/call/1` between two made-up
//! people in one space, each endpoint gated by `MemberAdmit` over a fixed
//! set of member devices. Every socket is on 127.0.0.1.
//!
//! - a dial opens one connection, both sides get it, and a datagram goes
//!   each way;
//! - two dials at once leave one connection (`replaced`), and two dials
//!   from one end open only one (the second is turned away, and a redial
//!   of a live peer leaves it be);
//! - a wrong key, an unknown call and a malformed call id are refused;
//! - a non-member is refused at the endpoint;
//! - `leave`, `drop_device` and `recheck` close the peer's connection,
//!   even one whose open was in flight when they ran.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ed25519_dalek::SigningKey;
use iroh::EndpointId;
use iroh::endpoint::{Connection, ConnectionError};
use irori_identity::{Chain, person_id_of};
use irori_meshseam::{MAX_LABEL, reason, write_frame};
use irori_net::admit::ChainAdmit;
use irori_net::board::{BoardAdmit, MemberAdmit};
use irori_net::call::{
    CALL_ALPN, CALL_LABEL, CALL_OPEN_V, CallConn, CallHandle, CallJoin, CallOpenMsg, EKM_CALL,
    call_open_proof,
};
use irori_net::endpoint::IpMode;
use irori_net::transport::{IrohNet, NetConfig, PeerAddr};

const SPACE: &str = "0123456789abcdef0123456789abcdef";
const CALL: &str = "aaaaaaaaaaaaaaaabbbbbbbbbbbbbbbb";
const OTHER_CALL: &str = "ccccccccccccccccdddddddddddddddd";
const WAIT: Duration = Duration::from_secs(20);

fn rand_key() -> SigningKey {
    let mut s = [0u8; 32];
    getrandom::fill(&mut s).unwrap();
    SigningKey::from_bytes(&s)
}

async fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        if start.elapsed() > WAIT {
            panic!("timed out waiting for {what}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// The space's member devices as the endpoint gate sees them.
#[derive(Debug, Default)]
struct FixedBoards(Mutex<Vec<EndpointId>>);

impl BoardAdmit for FixedBoards {
    fn board_device(&self, board_id: &str, id: &EndpointId) -> Option<String> {
        (board_id == SPACE && self.0.lock().unwrap().contains(id)).then(|| SPACE.to_string())
    }
    fn boards_of(&self, id: &EndpointId) -> Vec<String> {
        if self.0.lock().unwrap().contains(id) {
            vec![SPACE.to_string()]
        } else {
            Vec::new()
        }
    }
}

/// One device: its endpoint, the members its gate counts, the devices its
/// calls take, and the peer connections its calls handed over.
struct Dev {
    id: String,
    key: SigningKey,
    net: IrohNet,
    members: Arc<FixedBoards>,
    takes: Arc<Mutex<HashMap<String, EndpointId>>>,
    got: Arc<Mutex<Vec<CallConn>>>,
}

impl Dev {
    fn new(id: &str) -> Dev {
        let key = rand_key();
        let person = rand_key();
        let pid = person_id_of(&person.verifying_key().to_bytes());
        let chain = Arc::new(Chain::new(&pid, id, id));
        let members = Arc::new(FixedBoards::default());
        let admit = Arc::new(MemberAdmit {
            chain: ChainAdmit {
                chain,
                self_id: id.to_string(),
            },
            boards: members.clone(),
        });
        let mut cfg = NetConfig::new(id, iroh::SecretKey::from_bytes(&key.to_bytes()), admit);
        cfg.ip = IpMode::Loopback;
        cfg.gossip = false;
        cfg.dial_grace = Duration::from_millis(300);
        cfg.connect_timeout = Duration::from_secs(10);
        Dev {
            id: id.to_string(),
            key,
            net: IrohNet::new(cfg),
            members,
            takes: Arc::default(),
            got: Arc::default(),
        }
    }

    fn endpoint_id(&self) -> EndpointId {
        EndpointId::from_bytes(&self.key.verifying_key().to_bytes()).unwrap()
    }

    async fn peer_addr(&self) -> PeerAddr {
        let ep = self.net.bind().await.unwrap();
        let direct = ep.bound_sockets();
        for a in &direct {
            assert!(a.ip().is_loopback(), "non-loopback socket {a}");
        }
        PeerAddr {
            device_id: self.id.clone(),
            id: self.endpoint_id(),
            relay: None,
            direct,
        }
    }

    /// Counts `other` as a member device at the gate, takes it in calls,
    /// and books its address under the space.
    async fn meet(&self, other: &Dev) {
        self.members.0.lock().unwrap().push(other.endpoint_id());
        self.takes
            .lock()
            .unwrap()
            .insert(other.id.clone(), other.endpoint_id());
        self.net
            .set_board_peers(SPACE, vec![other.peer_addr().await]);
    }

    async fn join(&self, call: &str, key: [u8; 32]) -> CallHandle {
        self.join_with(call, key, Arc::new(AtomicBool::new(false)))
            .await
    }

    /// Joins `call`; while `slow` is set, each `accept` answers and then
    /// stalls half a second (holding a handshake open), once.
    async fn join_with(&self, call: &str, key: [u8; 32], slow: Arc<AtomicBool>) -> CallHandle {
        let takes = self.takes.clone();
        let got = self.got.clone();
        self.net
            .call_join(CallJoin {
                call_id: call.into(),
                space_id: SPACE.into(),
                keys: vec![(1, key)],
                accept: Arc::new(move |peer, id| {
                    let ok = takes.lock().unwrap().get(peer) == Some(id);
                    if slow.swap(false, Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    ok
                }),
                on_peer: Arc::new(move |c| got.lock().unwrap().push(c)),
            })
            .await
            .unwrap()
    }

    fn got(&self) -> usize {
        self.got.lock().unwrap().len()
    }

    /// The connection of the `i`th peer handed over.
    fn conn(&self, i: usize) -> Connection {
        self.got.lock().unwrap()[i].conn.clone()
    }

    fn live(&self) -> Vec<Connection> {
        self.got
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.conn.close_reason().is_none())
            .map(|c| c.conn.clone())
            .collect()
    }
}

/// Two members of the space who know each other.
async fn pair() -> (Dev, Dev) {
    let (a, b) = (Dev::new("a-alice-pc"), Dev::new("b-bob-mac"));
    a.meet(&b).await;
    b.meet(&a).await;
    (a, b)
}

fn key() -> [u8; 32] {
    let mut k = [0u8; 32];
    getrandom::fill(&mut k).unwrap();
    k
}

/// The reason the peer closed `conn` with.
async fn closed_by_peer(conn: &Connection) -> String {
    match tokio::time::timeout(WAIT, conn.closed()).await {
        Ok(ConnectionError::ApplicationClosed(c)) => String::from_utf8_lossy(&c.reason).into(),
        Ok(other) => panic!("closed otherwise: {other:?}"),
        Err(_) => panic!("never closed"),
    }
}

/// Alice dials Bob on `CALL` and both get the connection.
async fn connected() -> (Dev, Dev, CallHandle, CallHandle) {
    let (a, b) = pair().await;
    let k = key();
    let ha = a.join(CALL, k).await;
    let hb = b.join(CALL, k).await;
    a.net.call_dial(CALL, &b.id).await.unwrap();
    eventually("both sides get the peer", || a.got() == 1 && b.got() == 1).await;
    assert_eq!(a.got.lock().unwrap()[0].peer, b.id);
    assert_eq!(b.got.lock().unwrap()[0].peer, a.id);
    (a, b, ha, hb)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dial_opens_the_call_and_datagrams_go_both_ways() {
    let (a, b, ha, hb) = connected().await;
    assert_eq!(ha.peers(), vec![b.id.clone()]);
    assert_eq!(hb.peers(), vec![a.id.clone()]);
    let (ca, cb) = (a.conn(0), b.conn(0));
    assert!(ca.max_datagram_size().is_some());

    ca.send_datagram(b"from alice".to_vec().into()).unwrap();
    let got = tokio::time::timeout(WAIT, cb.read_datagram())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&got[..], b"from alice");
    cb.send_datagram(b"from bob".to_vec().into()).unwrap();
    let got = tokio::time::timeout(WAIT, ca.read_datagram())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&got[..], b"from bob");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_dials_at_once_leave_one_connection() {
    let (a, b) = pair().await;
    let k = key();
    let ha = a.join(CALL, k).await;
    let hb = b.join(CALL, k).await;
    let (ra, rb) = tokio::join!(a.net.call_dial(CALL, &b.id), b.net.call_dial(CALL, &a.id));
    // One of the two may lose to the other (`replaced`); at least one opens.
    assert!(ra.is_ok() || rb.is_ok(), "{ra:?} {rb:?}");
    eventually("one live connection each side", || {
        a.live().len() == 1 && b.live().len() == 1
    })
    .await;
    assert_eq!(ha.peers(), vec![b.id.clone()]);
    assert_eq!(hb.peers(), vec![a.id.clone()]);
    // The two live ends are one connection: a datagram crosses it.
    let (ca, cb) = (a.live().remove(0), b.live().remove(0));
    ca.send_datagram(b"one".to_vec().into()).unwrap();
    let got = tokio::time::timeout(WAIT, cb.read_datagram())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&got[..], b"one");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_dials_from_one_end_open_one_connection() {
    let (a, b) = pair().await;
    let k = key();
    let ha = a.join(CALL, k).await;
    let hb = b.join(CALL, k).await;
    let (r1, r2) = tokio::join!(a.net.call_dial(CALL, &b.id), a.net.call_dial(CALL, &b.id));
    // One dials; the other is turned away while it does.
    assert!(r1.is_ok() != r2.is_ok(), "{r1:?} {r2:?}");
    let err = r1.err().or(r2.err()).unwrap();
    assert!(err.contains("already dialing"), "{err}");
    eventually("both sides get the peer", || a.got() == 1 && b.got() == 1).await;
    // A redial of a live peer leaves its connection be.
    a.net.call_dial(CALL, &b.id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!((a.got(), b.got()), (1, 1));
    assert_eq!((a.live().len(), b.live().len()), (1, 1));
    assert_eq!(ha.peers(), vec![b.id.clone()]);
    assert_eq!(hb.peers(), vec![a.id.clone()]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drop_device_reaches_an_open_in_flight() {
    let (a, b) = pair().await;
    let k = key();
    let ha = a.join(CALL, k).await;
    let slow = Arc::new(AtomicBool::new(true));
    let _hb = b.join_with(CALL, k, slow.clone()).await;
    let dial = tokio::spawn({
        let net = a.net.clone();
        let id = b.id.clone();
        async move { net.call_dial(CALL, &id).await }
    });
    // Bob's `accept` has taken Alice and stalls; drop her meanwhile.
    eventually("bob's accept runs", || !slow.load(Ordering::SeqCst)).await;
    b.net.drop_device(&a.id, reason::REVOKED);
    let _ = dial.await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(b.got(), 0, "bob took a dropped device");
    eventually("alice has no live peer", || ha.peers().is_empty()).await;
    for c in a.live() {
        assert_eq!(closed_by_peer(&c).await, reason::REVOKED);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recheck_reaches_an_open_in_flight() {
    let (a, b) = pair().await;
    let k = key();
    let ha = a.join(CALL, k).await;
    let slow = Arc::new(AtomicBool::new(true));
    let _hb = b.join_with(CALL, k, slow.clone()).await;
    let dial = tokio::spawn({
        let net = a.net.clone();
        let id = b.id.clone();
        async move { net.call_dial(CALL, &id).await }
    });
    eventually("bob's accept runs", || !slow.load(Ordering::SeqCst)).await;
    // Bob's call stops taking Alice while her open is in flight.
    b.takes.lock().unwrap().remove(&a.id);
    b.net.recheck();
    let _ = dial.await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(b.got(), 0, "bob took a revoked device");
    eventually("alice has no live peer", || ha.peers().is_empty()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wrong_key_is_refused() {
    let (a, b) = pair().await;
    let _ha = a.join(CALL, key()).await;
    let _hb = b.join(CALL, key()).await;
    let err = a.net.call_dial(CALL, &b.id).await.unwrap_err();
    assert!(err.contains(reason::REFUSED), "{err}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!((a.got(), b.got()), (0, 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unknown_call_is_refused() {
    let (a, b) = pair().await;
    let k = key();
    let _ha = a.join(CALL, k).await;
    let _hb = b.join(OTHER_CALL, k).await;
    let err = a.net.call_dial(CALL, &b.id).await.unwrap_err();
    assert!(err.contains(reason::REFUSED), "{err}");
    // Not joined here at all: nothing to dial with.
    assert!(a.net.call_dial(OTHER_CALL, &b.id).await.is_err());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!((a.got(), b.got()), (0, 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_malformed_call_id_is_refused() {
    let (a, b) = pair().await;
    let k = key();
    let upper = CALL.to_uppercase();
    let join = |id: &str| CallJoin {
        call_id: id.into(),
        space_id: SPACE.into(),
        keys: vec![(1, k)],
        accept: Arc::new(|_, _| true),
        on_peer: Arc::new(|_| {}),
    };
    for bad in [upper.as_str(), "abc", "", &format!("{CALL}0")] {
        assert!(a.net.call_join(join(bad)).await.is_err(), "joined {bad:?}");
        assert!(a.net.call_dial(bad, &b.id).await.is_err(), "dialed {bad:?}");
    }

    // On the wire: an open message naming a malformed id, with a proof
    // under it, is refused before any reply. No call here can have such an
    // id (`call_join` refuses it), so this refusal is also the unknown-call
    // one; the acceptor's own format check is defensive, and `is_call_id`
    // is unit-tested in `call.rs`.
    let _hb = b.join(CALL, k).await;
    let ep = a.net.bind().await.unwrap();
    let pb = b.peer_addr().await;
    let mut addr = iroh::EndpointAddr::new(pb.id);
    for d in pb.direct {
        addr = addr.with_ip_addr(d);
    }
    let conn = ep.connect(addr, CALL_ALPN).await.unwrap();
    let mut out = [0u8; 32];
    conn.export_keying_material(&mut out, EKM_CALL, b"")
        .unwrap();
    let (mut send, _recv) = conn.open_bi().await.unwrap();
    write_frame(&mut send, CALL_LABEL.as_bytes(), MAX_LABEL)
        .await
        .unwrap();
    let open = CallOpenMsg {
        v: CALL_OPEN_V,
        space_id: SPACE.into(),
        epoch: 1,
        call_id: upper.clone(),
        device_id: a.id.clone(),
        proof: call_open_proof(&k, &upper, &out, 0),
    };
    write_frame(&mut send, &serde_json::to_vec(&open).unwrap(), 4096)
        .await
        .unwrap();
    assert_eq!(closed_by_peer(&conn).await, reason::REFUSED);
    assert_eq!(b.got(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_non_member_is_refused_at_the_endpoint() {
    let (a, b) = pair().await;
    let carol = Dev::new("c-carol-pc");
    // Carol knows Alice (so her own gate lets the dial out); Alice's gate
    // doesn't count Carol.
    carol.meet(&a).await;
    // Alice's call would take Carol: only the endpoint gate can refuse her.
    a.takes
        .lock()
        .unwrap()
        .insert(carol.id.clone(), carol.endpoint_id());
    let k = key();
    let _ha = a.join(CALL, k).await;
    let _hc = carol.join(CALL, k).await;
    assert!(carol.net.call_dial(CALL, &a.id).await.is_err());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!((a.got(), carol.got()), (0, 0));
    drop(b);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn leave_closes_the_peers_connection() {
    let (a, b, ha, hb) = connected().await;
    let cb = b.conn(0);
    ha.leave();
    assert_eq!(closed_by_peer(&cb).await, reason::LEAVING);
    eventually("bob forgets alice", || hb.peers().is_empty()).await;
    // Left: a new dial from Bob is refused.
    let err = b.net.call_dial(CALL, &a.id).await.unwrap_err();
    assert!(err.contains(reason::REFUSED), "{err}");
    ha.leave();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dropping_the_handle_leaves() {
    let (_a, b, ha, _hb) = connected().await;
    let cb = b.conn(0);
    drop(ha);
    assert_eq!(closed_by_peer(&cb).await, reason::LEAVING);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drop_device_closes_the_call_connection() {
    let (a, b, ha, _hb) = connected().await;
    let cb = b.conn(0);
    a.net.drop_device(&b.id, reason::REVOKED);
    assert_eq!(closed_by_peer(&cb).await, reason::REVOKED);
    eventually("alice forgets bob", || ha.peers().is_empty()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recheck_closes_a_device_the_call_no_longer_takes() {
    let (a, b, ha, _hb) = connected().await;
    let cb = b.conn(0);
    a.net.recheck();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(cb.close_reason().is_none(), "recheck closed a member");
    a.takes.lock().unwrap().remove(&b.id);
    a.net.recheck();
    assert_eq!(closed_by_peer(&cb).await, reason::REVOKED);
    eventually("alice forgets bob", || ha.peers().is_empty()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drop_board_ends_the_spaces_calls() {
    let (a, b, ha, _hb) = connected().await;
    let cb = b.conn(0);
    a.net.drop_board(SPACE, reason::REVOKED).unwrap();
    assert_eq!(closed_by_peer(&cb).await, reason::REVOKED);
    eventually("alice forgets bob", || ha.peers().is_empty()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closing_the_net_ends_its_calls() {
    let (a, b, _ha, hb) = connected().await;
    let cb = b.conn(0);
    a.net.close().await;
    assert_eq!(closed_by_peer(&cb).await, reason::SHUTTING_DOWN);
    eventually("bob forgets alice", || hb.peers().is_empty()).await;
}
