//! Boards stage 2 gossip on loopback (card #43; boards2-transport.md §5):
//! a board's iroh-gossip topic between two people's devices, gated by
//! `MemberAdmit` over `HeldBoards`. An announcement from a member that is
//! ahead of the local log kicks a pull; a device no board here counts
//! can't join the topic; a member of another board held here gets through
//! the endpoint gate but its deliveries are dropped on this board's topic.
//! Fixture keys only; every socket is 127.0.0.1.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use irori_identity::{BOARD_SECRET_SIZE, BoardTopic, Chain, derive_board_topic};
use irori_net::admit::ChainAdmit;
use irori_net::board::MemberAdmit;
use irori_net::boardadmit::{HeldBoards, MemberKeys};
use irori_net::endpoint::IpMode;
use irori_net::gossip::{Announcement, BoardChannel};
use irori_net::transport::{IrohNet, NetConfig, PeerAddr};

const WAIT: Duration = Duration::from_secs(20);
const BOARD: &str = "0123456789abcdef0123456789abcdef";
const OTHER: &str = "fedcba9876543210fedcba9876543210";

/// One device: its own person (an empty Chain: no other device of its
/// own), the boards it holds, and its endpoint.
struct Dev {
    id: String,
    key: iroh::SecretKey,
    held: HeldBoards,
    net: IrohNet,
}

impl Dev {
    fn new(id: &str) -> Dev {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).unwrap();
        let key = iroh::SecretKey::from_bytes(&seed);
        let held = HeldBoards::default();
        let admit = Arc::new(MemberAdmit {
            chain: ChainAdmit {
                chain: Arc::new(Chain::new(&format!("person-{id}"), id, id)),
                self_id: id.into(),
            },
            boards: Arc::new(held.clone()),
        });
        let mut cfg = NetConfig::new(id, key.clone(), admit);
        cfg.ip = IpMode::Loopback;
        cfg.connect_timeout = Duration::from_secs(10);
        Dev {
            id: id.into(),
            key,
            held,
            net: IrohNet::new(cfg),
        }
    }

    /// Holds `board` counting `members` (device id, key).
    fn hold(&self, board: &str, members: &[&Dev]) {
        let m = MemberKeys::default();
        for d in members {
            m.add(&d.id, &d.key.public());
        }
        self.held.hold(board, Arc::new(m));
    }

    fn addr(&self) -> PeerAddr {
        PeerAddr {
            device_id: self.id.clone(),
            id: self.key.public(),
            relay: None,
            direct: self.net.endpoint().unwrap().bound_sockets(),
        }
    }

    async fn channel(&self, topic: &BoardTopic, board: &str, boot: &[&Dev]) -> BoardChannel {
        let boot: Vec<String> = boot.iter().map(|d| d.id.clone()).collect();
        self.net
            .board_channel_gated(board, topic, &boot, Some(self.held.gossip_accept(board)))
            .await
            .unwrap()
    }
}

async fn introduce(devs: &[&Dev]) {
    for d in devs {
        d.net.bind().await.unwrap();
        for a in d.net.endpoint().unwrap().bound_sockets() {
            assert!(a.ip().is_loopback(), "non-loopback socket {a}");
        }
    }
    let addrs: Vec<PeerAddr> = devs.iter().map(|d| d.addr()).collect();
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

fn topic(board: &str) -> BoardTopic {
    let mut secret = [0u8; BOARD_SECRET_SIZE];
    getrandom::fill(&mut secret).unwrap();
    derive_board_topic(board, 1, &secret).unwrap()
}

fn vector(n: i64) -> BTreeMap<String, i64> {
    BTreeMap::from([("stephenpc".to_string(), n)])
}

/// Counts kicks; `have` is the local vector the channel compares with.
struct Kicks {
    n: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<Announcement>>>,
    have: Arc<Mutex<BTreeMap<String, i64>>>,
}

impl Kicks {
    fn on(ch: &mut BoardChannel) -> Kicks {
        let k = Kicks {
            n: Arc::default(),
            seen: Arc::default(),
            have: Arc::default(),
        };
        let (n, seen, have) = (k.n.clone(), k.seen.clone(), k.have.clone());
        ch.kicks(
            Arc::new(move || have.lock().unwrap().clone()),
            Arc::new(move |a: &Announcement| {
                seen.lock().unwrap().push(a.clone());
                n.fetch_add(1, Ordering::SeqCst);
            }),
        );
        k
    }

    fn count(&self) -> usize {
        self.n.load(Ordering::SeqCst)
    }
}

/// Announces `v` on `ch` until `done()` or `WAIT` (the topic needs a moment
/// to form). Each round's vector differs so gossip doesn't de-duplicate it.
async fn announce_until(ch: &BoardChannel, base: i64, mut done: impl FnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + WAIT;
    let mut n = base;
    while tokio::time::Instant::now() < deadline {
        ch.announce(vector(n)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        if done() {
            return true;
        }
        n += 1;
    }
    false
}

/// Stephen's PC and Blake's PC share a board; a stranger who somehow has the
/// topic bootstraps from both. Stephen's announcement kicks Blake's pull;
/// the stranger never hears it and never gets a kick out of Blake.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_members_announcement_kicks_a_pull_and_a_stranger_cannot_join() {
    let (s, b, x) = (
        Dev::new("stephenpc"),
        Dev::new("blakepc0"),
        Dev::new("strangr0"),
    );
    s.hold(BOARD, &[&s, &b]);
    b.hold(BOARD, &[&s, &b]);
    x.hold(BOARD, &[&s, &b, &x]); // the stranger thinks it belongs
    introduce(&[&s, &b, &x]).await;
    let t = topic(BOARD);
    let ch_s = s.channel(&t, BOARD, &[&b]).await;
    let mut ch_b = b.channel(&t, BOARD, &[&s]).await;
    let mut ch_x = x.channel(&t, BOARD, &[&s, &b]).await;
    let kicks = Kicks::on(&mut ch_b);
    let mut heard_x = Vec::new();

    assert!(
        announce_until(&ch_s, 1, || kicks.count() > 0).await,
        "blake's pull was never kicked"
    );
    let a = kicks.seen.lock().unwrap()[0].clone();
    assert_eq!((a.board_id.as_str(), a.epoch), (BOARD, 1));

    // Caught up: an announcement that isn't ahead kicks nothing.
    *kicks.have.lock().unwrap() = vector(1_000);
    let before = kicks.count();
    for n in 0..5 {
        ch_s.announce(vector(500 + n)).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(kicks.count(), before, "a stale announcement kicked a pull");
    // Ahead again: it kicks.
    assert!(
        announce_until(&ch_s, 2_000, || kicks.count() > before).await,
        "an ahead announcement didn't kick"
    );

    // The stranger: refused by both gates, it hears nothing, and what it
    // announces kicks nothing.
    let before = kicks.count();
    *kicks.have.lock().unwrap() = BTreeMap::new();
    for n in 0..10 {
        ch_x.announce(vector(10_000 + n)).await.unwrap();
        ch_s.announce(vector(20_000 + n)).await.unwrap();
        if let Ok(Some(a)) = tokio::time::timeout(Duration::from_millis(150), ch_x.next()).await {
            heard_x.push(a);
        }
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    while let Ok(Some(a)) = tokio::time::timeout(Duration::from_millis(50), ch_x.next()).await {
        heard_x.push(a);
    }
    assert!(heard_x.is_empty(), "the stranger heard {heard_x:?}");
    assert!(
        kicks
            .seen
            .lock()
            .unwrap()
            .iter()
            .skip(before)
            .all(|a| a.vector.values().all(|n| *n >= 20_000)),
        "the stranger's announcement kicked blake"
    );
    drop((ch_s, ch_b, ch_x));
    for d in [&s, &b, &x] {
        d.net.close().await;
    }
}

/// A member of another board held here passes the endpoint gate (it is a
/// member device somewhere), but this board's topic drops what it delivers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_member_of_another_board_is_not_heard_on_this_one() {
    let (s, c) = (Dev::new("stephenpc"), Dev::new("carapc00"));
    s.hold(BOARD, &[&s]);
    s.hold(OTHER, &[&s, &c]);
    c.hold(BOARD, &[&s, &c]);
    introduce(&[&s, &c]).await;
    let t = topic(BOARD);
    let mut ch_s = s.channel(&t, BOARD, &[&c]).await;
    let ch_c = c.channel(&t, BOARD, &[&s]).await;
    let kicks = Kicks::on(&mut ch_s);
    for n in 0..15 {
        ch_c.announce(vector(1 + n)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(kicks.count(), 0, "a member of another board kicked a pull");

    // Once Cara is counted on this board too, her announcements count.
    let m = MemberKeys::default();
    m.add(&s.id, &s.key.public());
    m.add(&c.id, &c.key.public());
    s.held.hold(BOARD, Arc::new(m));
    assert!(
        announce_until(&ch_c, 100, || kicks.count() > 0).await,
        "a member's announcement was dropped"
    );
    drop((ch_s, ch_c));
    for d in [&s, &c] {
        d.net.close().await;
    }
}
