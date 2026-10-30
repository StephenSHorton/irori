//! The LAN lookup on loopback: every socket binds 127.0.0.1 with a
//! non-default port, and announcements go unicast to the other test socket
//! instead of a broadcast (architecture §6.5). Ports go-room's behavioural
//! LAN tests (R:lan_test.go): sealed rooms, tag windows, junk and replay
//! limits, rate limit, the port-taken retry, plus an iroh dial by id alone.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use iroh::{EndpointAddr, EndpointId, RelayUrl, SecretKey};
use irori_lan::{Announcement, Config, LanLookup, Room, seal};

fn free_udp() -> SocketAddr {
    let s = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    s.local_addr().unwrap()
}

fn key() -> SecretKey {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).unwrap();
    SecretKey::from_bytes(&b)
}

fn room(k: u8) -> Room {
    Room {
        topic: format!("irori/mesh/1/{k:032x}"),
        key: [k; 32],
    }
}

fn cfg(id: EndpointId, bind: SocketAddr, to: SocketAddr, rooms: Vec<Room>) -> Config {
    let mut c = Config::new(id, Arc::new(move || rooms.clone()));
    c.port = bind.port();
    c.bind = Some(bind);
    c.targets = Some(Arc::new(move || vec![to]));
    c.every = Duration::from_millis(100);
    c.retry = Duration::from_millis(100);
    c
}

async fn eventually(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..300 {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for: {what}");
}

#[tokio::test]
async fn two_devices_hear_each_other_and_bye_forgets() {
    let (a_addr, b_addr) = (free_udp(), free_udp());
    let (a_id, b_id) = (key().public(), key().public());
    let heard_by_b: Arc<Mutex<Vec<(EndpointId, SocketAddr)>>> = Arc::default();
    let a = LanLookup::start(cfg(a_id, a_addr, b_addr, vec![room(1)]));
    let mut bc = cfg(b_id, b_addr, a_addr, vec![room(1)]);
    let h = heard_by_b.clone();
    bc.on_heard = Some(Arc::new(move |id, from| h.lock().unwrap().push((id, from))));
    let b = LanLookup::start(bc);
    let relay: RelayUrl = "https://127.0.0.1:25817/".parse().unwrap();
    a.set_self(vec!["127.0.0.1:4242".parse().unwrap()], Some(relay.clone()));
    b.set_self(vec!["127.0.0.1:4343".parse().unwrap()], None);
    eventually("b hears a", || b.heard(&a_id).is_some()).await;
    eventually("a hears b", || a.heard(&b_id).is_some()).await;
    let got = b.heard(&a_id).unwrap();
    assert_eq!(
        got.addrs,
        vec!["127.0.0.1:4242".parse::<SocketAddr>().unwrap()]
    );
    assert_eq!(got.relay, Some(relay));
    assert_eq!(got.from, a_addr);
    assert!(
        heard_by_b
            .lock()
            .unwrap()
            .iter()
            .all(|(id, from)| *id == a_id && *from == a_addr)
    );
    assert!(b.heard(&b_id).is_none(), "a device never lists itself");
    a.close().await;
    eventually("bye forgets a", || b.heard(&a_id).is_none()).await;
    b.close().await;
}

/// A bye in the same millisecond as the announcement before it (a tick and
/// the close both ready when the task runs, as on a loaded machine) still
/// forgets: announcements carry strictly increasing times, so the replay
/// guard never takes the bye for a replay. a's clock stands still here,
/// making every send the same millisecond.
#[tokio::test]
async fn bye_in_the_same_millisecond_forgets() {
    let (a_addr, b_addr) = (free_udp(), free_udp());
    let (a_id, b_id) = (key().public(), key().public());
    let frozen = SystemTime::now();
    let mut ac = cfg(a_id, a_addr, b_addr, vec![room(1)]);
    ac.now = Some(Arc::new(move || frozen));
    let a = LanLookup::start(ac);
    let b = LanLookup::start(cfg(b_id, b_addr, a_addr, vec![room(1)]));
    a.set_self(vec!["127.0.0.1:4242".parse().unwrap()], None);
    eventually("b hears a", || b.heard(&a_id).is_some()).await;
    tokio::time::sleep(Duration::from_millis(250)).await; // a few ticks
    a.close().await;
    eventually("bye forgets a", || b.heard(&a_id).is_none()).await;
    b.close().await;
}

#[tokio::test]
async fn another_person_hears_nothing() {
    let (a_addr, b_addr) = (free_udp(), free_udp());
    let (a_id, b_id) = (key().public(), key().public());
    let a = LanLookup::start(cfg(a_id, a_addr, b_addr, vec![room(1)]));
    let b = LanLookup::start(cfg(b_id, b_addr, a_addr, vec![room(2)]));
    a.set_self(vec!["127.0.0.1:1".parse().unwrap()], None);
    b.set_self(vec!["127.0.0.1:2".parse().unwrap()], None);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(b.heard(&a_id).is_none() && a.heard(&b_id).is_none());
    // A second open epoch on b's side meets a's room.
    a.close().await;
    b.close().await;
}

#[tokio::test]
async fn old_announcements_and_junk_are_dropped() {
    let b_addr = free_udp();
    let b_id = key().public();
    let b = LanLookup::start(cfg(b_id, b_addr, free_udp(), vec![room(1)]));
    eventually("bound", || b.local_addr().is_some()).await;
    let raw = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let ms = now.as_millis() as i64;
    let mk = |id: EndpointId, at: i64| Announcement {
        v: 1,
        id: id.to_string(),
        addrs: vec!["127.0.0.1:9".into()],
        relay: None,
        relay_key: None,
        at,
        bye: false,
    };
    // Three minutes old: past the 2-minute age.
    let stale = key().public();
    raw.send_to(&seal(&room(1), &mk(stale, ms - 180_000), ms / 1000), b_addr)
        .unwrap();
    // Junk with the right magic, a truncated box, and a wrong version.
    raw.send_to(b"IR1aaaaaaaajunkjunkjunkjunkjunkjunkjunkjunkjunk", b_addr)
        .unwrap();
    let v2 = key().public();
    let mut a2 = mk(v2, ms);
    a2.v = 2;
    raw.send_to(&seal(&room(1), &a2, ms / 1000), b_addr)
        .unwrap();
    // A fresh one lands.
    let fresh = key().public();
    raw.send_to(&seal(&room(1), &mk(fresh, ms), ms / 1000), b_addr)
        .unwrap();
    eventually("the fresh announcement", || b.heard(&fresh).is_some()).await;
    assert!(b.heard(&stale).is_none() && b.heard(&v2).is_none());
    b.close().await;
}

#[tokio::test]
async fn opens_are_rate_limited() {
    let b_addr = free_udp();
    let mut c = cfg(key().public(), b_addr, free_udp(), vec![room(1)]);
    c.max_rate = 3;
    // A fixed clock: every datagram falls in one rate second.
    let fixed = SystemTime::now();
    c.now = Some(Arc::new(move || fixed));
    let n = Arc::new(AtomicUsize::new(0));
    let nn = n.clone();
    c.on_heard = Some(Arc::new(move |_, _| {
        nn.fetch_add(1, Ordering::SeqCst);
    }));
    let b = LanLookup::start(c);
    eventually("bound", || b.local_addr().is_some()).await;
    let ms = fixed
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let raw = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    for _ in 0..10 {
        let a = Announcement {
            v: 1,
            id: key().public().to_string(),
            addrs: vec![],
            relay: None,
            relay_key: None,
            at: ms,
            bye: false,
        };
        raw.send_to(&seal(&room(1), &a, ms / 1000), b_addr).unwrap();
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(n.load(Ordering::SeqCst), 3);
    b.close().await;
}

#[tokio::test]
async fn a_taken_port_is_retried() {
    let busy = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let at = busy.local_addr().unwrap();
    let b = LanLookup::start(cfg(key().public(), at, free_udp(), vec![room(1)]));
    eventually("the error line", || !b.error_line().is_empty()).await;
    assert!(
        b.error_line().starts_with(&format!(
            "Irori can't use UDP port {} on this device",
            at.port()
        )),
        "{}",
        b.error_line()
    );
    drop(busy);
    eventually("it opens once the port is free", || {
        b.local_addr() == Some(at)
    })
    .await;
    assert_eq!(b.error_line(), "");
    b.close().await;
}

#[tokio::test]
async fn nowhere_to_send_says_so() {
    let mut c = cfg(key().public(), free_udp(), free_udp(), vec![room(1)]);
    c.targets = Some(Arc::new(Vec::new));
    let b = LanLookup::start(c);
    eventually("bound", || b.local_addr().is_some()).await;
    assert!(
        b.error_line()
            .contains("Windows has this network as Public")
    );
    b.close().await;
}

/// An iroh endpoint dials another by its EndpointId alone: the address
/// comes from the LAN lookup (the endpoint's own publish feeds the
/// announcement). Loopback IP only, no relay.
#[tokio::test]
async fn iroh_dials_by_id_through_the_lan_lookup() {
    const ALPN: &[u8] = b"irori/lan-test/1";
    let (ka, kb) = (key(), key());
    let (a_addr, b_addr) = (free_udp(), free_udp());
    let rooms = vec![room(7)];
    let la = LanLookup::start(cfg(ka.public(), a_addr, b_addr, rooms.clone()));
    let lb = LanLookup::start(cfg(kb.public(), b_addr, a_addr, rooms));
    let ep = |k: SecretKey, l: LanLookup| async move {
        iroh::endpoint::Builder::empty()
            .crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .secret_key(k)
            .alpns(vec![ALPN.to_vec()])
            .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
            .clear_ip_transports()
            .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .unwrap()
            .relay_mode(iroh::RelayMode::Disabled)
            .address_lookup(l)
            .bind()
            .await
            .unwrap()
    };
    let a = ep(ka.clone(), la.clone()).await;
    let b = ep(kb, lb.clone()).await;
    eventually("b hears a", || lb.heard(&ka.public()).is_some()).await;
    let server = tokio::spawn(async move {
        let inc = a.accept().await.unwrap();
        let conn = inc.accept().unwrap().await.unwrap();
        let (mut s, mut r) = conn.accept_bi().await.unwrap();
        let got = r.read_to_end(64).await.unwrap();
        s.write_all(&got).await.unwrap();
        s.finish().unwrap();
        let _ = s.stopped().await;
        a
    });
    let conn = tokio::time::timeout(
        Duration::from_secs(10),
        b.connect(EndpointAddr::new(ka.public()), ALPN),
    )
    .await
    .expect("connect in time")
    .expect("connect");
    let (mut s, mut r) = conn.open_bi().await.unwrap();
    s.write_all(b"hello").await.unwrap();
    s.finish().unwrap();
    assert_eq!(r.read_to_end(64).await.unwrap(), b"hello");
    let a = server.await.unwrap();
    conn.close(0u32.into(), b"done");
    b.close().await;
    a.close().await;
    la.close().await;
    lb.close().await;
}

/// R:lan_test.go `TestLANReplayDoesntMoveAMember`: a captured announcement
/// sent again from another address within the age window neither moves the
/// device (its `from`, what `on_heard` dials first) nor, as a replayed bye,
/// makes this device forget it.
#[tokio::test]
async fn a_replay_doesnt_move_a_member() {
    let b_addr = free_udp();
    let b_id = key().public();
    let heard: Arc<Mutex<Vec<(EndpointId, SocketAddr)>>> = Arc::default();
    let mut bc = cfg(b_id, b_addr, free_udp(), vec![room(1)]);
    let h = heard.clone();
    bc.on_heard = Some(Arc::new(move |id, from| h.lock().unwrap().push((id, from))));
    let b = LanLookup::start(bc);
    eventually("bound", || b.local_addr().is_some()).await;
    let real = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let attacker = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let a_id = key().public();
    let mk = |at: i64, bye: bool| Announcement {
        v: 1,
        id: a_id.to_string(),
        addrs: if bye {
            vec![]
        } else {
            vec!["127.0.0.1:9".into()]
        },
        relay: None,
        relay_key: None,
        at,
        bye,
    };
    let first = seal(&room(1), &mk(ms, false), ms / 1000);
    real.send_to(&first, b_addr).unwrap();
    eventually("b hears a", || b.heard(&a_id).is_some()).await;
    let real_from = real.local_addr().unwrap();
    assert_eq!(b.heard(&a_id).unwrap().from, real_from);

    // The same bytes from the attacker: dropped.
    attacker.send_to(&first, b_addr).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(b.heard(&a_id).unwrap().from, real_from, "a replay moved a");
    assert!(
        heard
            .lock()
            .unwrap()
            .iter()
            .all(|(_, from)| *from == real_from),
        "on_heard was told the attacker's address"
    );

    // A real bye, then a real comeback; a replay of the bye forgets nothing.
    let bye = seal(&room(1), &mk(ms + 1, true), ms / 1000);
    real.send_to(&bye, b_addr).unwrap();
    eventually("the bye forgets a", || b.heard(&a_id).is_none()).await;
    attacker.send_to(&first, b_addr).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(b.heard(&a_id).is_none(), "a replay brought a back");
    real.send_to(&seal(&room(1), &mk(ms + 2, false), ms / 1000), b_addr)
        .unwrap();
    eventually("a is back", || b.heard(&a_id).is_some()).await;
    attacker.send_to(&bye, b_addr).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(b.heard(&a_id).is_some(), "a replayed bye forgot a");
    b.close().await;
}
