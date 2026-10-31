//! Boards stage 2 through two Cores (card #43): two made-up people's Cores,
//! each its own first-run person on loopback iroh, open one shared board on
//! the network (`Core::open_board_net`) and its rows move both ways on
//! `irori/board/1`, while their private meshes never connect. The board's
//! people and their keys are made up for the test; the board logs are
//! signed by each Core's own device key. No LAN lookup, nothing off
//! 127.0.0.1.
//!
//! The second test runs the whole stage 2 path: one member's home server
//! (on 127.0.0.1, reachable "from outside" at 127.0.0.1) writes its
//! `board.server` row, the other member's Core reaches it through that
//! relay, which admits it only as a member device of a board held there
//! (B1), and an announcement on the board's gossip topic makes the other
//! Core pull over `irori/board/1`.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use ed25519_dalek::SigningKey;
use irori_core::{BoardParts, Core, devices};
use irori_identity::membership::BOARD_MEMBER;
use irori_identity::{
    BoardCreated, BoardMemberAdded, BoardSecrets, BoardVerifier, FileKeystore, MemberCert,
    MemberDeviceAdded, MemberPin, MemberProfile, ROLE_DESKTOP, kinds, person_id_of, sign_person,
};
use irori_net::endpoint::IpMode;
use irori_net::transport::IrohNet;
use store::{Event, Store};

const BOARD: &str = "fedcba9876543210fedcba9876543210";

fn wait(what: &str, ok: impl FnMut() -> bool) {
    wait_for(what, Duration::from_secs(30), ok)
}

fn wait_for(what: &str, limit: Duration, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(start.elapsed() < limit, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn now() -> gocompat::time::GoTime {
    gocompat::time::GoTime::from_system(std::time::SystemTime::now())
}

fn rand_key() -> SigningKey {
    let mut s = [0u8; 32];
    getrandom::fill(&mut s).unwrap();
    SigningKey::from_bytes(&s)
}

/// A Core on loopback iroh with no LAN lookup.
fn core(data: &std::path::Path, claude: &std::path::Path) -> Core {
    core_with(data, claude, false, None)
}

/// [`core`] with board gossip on or off and a home server edit.
fn core_with(
    data: &std::path::Path,
    claude: &std::path::Path,
    gossip: bool,
    home: Option<irori_core::HomeEdit>,
) -> Core {
    let mut o = common::options(data, claude);
    o.pair_net = Some(Arc::new(move |ident, chain, guests| {
        let mut nc = devices::default_net_config(ident, chain, guests);
        nc.ip = IpMode::Loopback;
        nc.lan_config = None;
        nc.gossip = gossip;
        IrohNet::new(nc)
    }));
    o.home = home;
    Core::start(o).unwrap()
}

/// A made-up board member: its person key, rooted at a Core's device.
struct Member {
    key: SigningKey,
    pid: String,
    dev: String,
    dev_pub: String,
}

impl Member {
    fn of(c: &Core) -> Member {
        let key = rand_key();
        Member {
            pid: person_id_of(&key.verifying_key().to_bytes()),
            key,
            dev: c.device_id().to_string(),
            dev_pub: hex::encode(c.identity().device_pub()),
        }
    }

    fn pin(&self) -> MemberPin {
        MemberPin {
            person_id: self.pid.clone(),
            person_pub: hex::encode(self.key.verifying_key().to_bytes()),
            root_device_id: self.dev.clone(),
            person_epoch: 1,
        }
    }

    fn actor(&self) -> String {
        format!("person:{}", self.pid)
    }
}

struct Board {
    log: Arc<Store>,
    verifier: Arc<BoardVerifier>,
    secrets: Arc<BoardSecrets>,
}

fn open_board(c: &Core, dir: &std::path::Path, creator: &MemberPin) -> Board {
    let verifier = Arc::new(BoardVerifier::new_for(BOARD, std::slice::from_ref(creator)));
    let log = Arc::new(
        Store::open_with(
            dir.join(format!("{BOARD}.db")),
            c.device_id(),
            Arc::new(store::Clock::new()),
            store::OpenOptions {
                signer: Some(c.identity().clone()),
                verifier: Some(verifier.clone()),
                board: true,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let secrets = Arc::new(
        BoardSecrets::open(
            &dir.join(format!("{BOARD}.secrets")),
            BOARD,
            Box::new(FileKeystore),
        )
        .unwrap(),
    );
    Board {
        log,
        verifier,
        secrets,
    }
}

fn append<T: serde::Serialize>(st: &Store, actor: &str, kind: &str, v: &T) -> Event {
    st.append(
        actor,
        kind,
        &gocompat::json::RawJson(gocompat::json::marshal(v).unwrap()),
    )
    .unwrap()
}

fn self_cert(st: &Store, m: &Member) -> Event {
    let cert = MemberCert {
        person_id: m.pid.clone(),
        device_id: m.dev.clone(),
        device_pub: m.dev_pub.clone(),
        role: ROLE_DESKTOP.into(),
        person_epoch: 1,
        at: now(),
    };
    let a = MemberDeviceAdded {
        person_sig: sign_person(&m.key, &cert).unwrap(),
        cert,
    };
    append(st, "irori", kinds::MEMBER_DEVICE_ADDED, &a)
}

fn note(st: &Store, m: &Member, name: &str) -> Event {
    let p = MemberProfile {
        person_id: m.pid.clone(),
        name: name.into(),
        person_epoch: 1,
        person_sig: String::new(),
    };
    append(st, "irori", kinds::MEMBER_PROFILE, &p)
}

fn has(st: &Store, ev: &Event) -> bool {
    st.list(&store::Filter {
        device_id: ev.device_id.clone(),
        after_seq: ev.seq - 1,
        limit: 1,
        ..Default::default()
    })
    .is_ok_and(|evs| evs.len() == 1 && store::row_hash(&evs[0]) == store::row_hash(ev))
}

/// Alice (on Core `a`) creates the board and adds Bob (on Core `b`); each
/// side's log and secrets hold what the invite bundle would carry.
fn shared_board(
    a: &Core,
    b: &Core,
    dir_a: &std::path::Path,
    dir_b: &std::path::Path,
) -> (Member, Member, Board, Board) {
    let (alice, bob) = (Member::of(a), Member::of(b));
    let creator = alice.pin();
    let ba = open_board(a, dir_a, &creator);
    let bb = open_board(b, dir_b, &creator);
    let a_cert = self_cert(&ba.log, &alice);
    let mut created = BoardCreated {
        board_id: BOARD.into(),
        name: "shared".into(),
        tag: "shared".into(),
        creator: alice.pin(),
        at: now(),
        person_sig: String::new(),
    };
    created.person_sig = sign_person(&alice.key, &created).unwrap();
    let created = append(&ba.log, &alice.actor(), kinds::BOARD_CREATED, &created);
    let pin = bob.pin();
    let mut added = BoardMemberAdded {
        person_id: pin.person_id,
        person_pub: pin.person_pub,
        root_device_id: pin.root_device_id,
        person_epoch: 1,
        name: "bob".into(),
        role: BOARD_MEMBER.into(),
        by: alice.pid.clone(),
        by_epoch: 1,
        at: now(),
        person_sig: String::new(),
    };
    added.person_sig = sign_person(&alice.key, &added).unwrap();
    let added = append(&ba.log, &alice.actor(), kinds::BOARD_MEMBER_ADDED, &added);
    let b_cert = self_cert(&bb.log, &bob);
    // What the invite bundle carries.
    bb.log.insert_batch(&[a_cert, created, added]).1.unwrap();
    ba.log.insert_batch(&[b_cert]).1.unwrap();
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).unwrap();
    ba.secrets.add(1, &alice.dev, &secret).unwrap();
    bb.secrets.add(1, &alice.dev, &secret).unwrap();
    (alice, bob, ba, bb)
}

fn board_parts(m: &Member, b: &Board) -> BoardParts {
    BoardParts {
        board_id: BOARD.into(),
        person_id: m.pid.clone(),
        log: b.log.clone(),
        verifier: b.verifier.clone(),
        secrets: b.secrets.clone(),
    }
}

#[test]
fn two_peoples_cores_sync_a_shared_board_on_its_own_connection() {
    let dirs: Vec<tempfile::TempDir> = (0..6).map(|_| tempfile::tempdir().unwrap()).collect();
    let a = core(dirs[0].path(), dirs[1].path());
    let b = core(dirs[2].path(), dirs[3].path());
    assert_ne!(a.person().id, b.person().id, "two people");

    let (alice, bob, ba, bb) = shared_board(&a, &b, dirs[4].path(), dirs[5].path());
    let (ra, rb) = (a.pairing_runtime().clone(), b.pairing_runtime().clone());
    ra.block_on(a.open_board_net(board_parts(&alice, &ba)))
        .unwrap();
    rb.block_on(b.open_board_net(board_parts(&bob, &bb)))
        .unwrap();
    // The invite ticket's direct addresses.
    let addrs = |c: &Core| c.net().endpoint().expect("bound").bound_sockets();
    for x in addrs(&a).iter().chain(addrs(&b).iter()) {
        assert!(x.ip().is_loopback(), "non-loopback socket {x}");
    }
    a.board_addr_hint(b.device_id(), addrs(&b));
    b.board_addr_hint(a.device_id(), addrs(&a));

    let peer = |c: &Core, id: &str| c.net().board_conns(BOARD).iter().any(|x| x.peer == id);
    wait("the two Cores connect on the board", || {
        peer(&a, b.device_id()) && peer(&b, a.device_id())
    });
    let from_a = note(&ba.log, &alice, "from alice");
    a.kick_board(BOARD);
    wait("bob's Core pulls alice's row", || has(&bb.log, &from_a));
    let from_b = note(&bb.log, &bob, "from bob");
    b.kick_board(BOARD);
    wait("alice's Core pulls bob's row", || has(&ba.log, &from_b));
    // Nothing on the private mesh, and no board row in a private log.
    assert!(a.net().conns().is_empty() && b.net().conns().is_empty());
    for (c, ev) in [(&a, &from_b), (&b, &from_a)] {
        let private = c
            .store()
            .list(&store::Filter {
                kinds: vec![kinds::MEMBER_PROFILE.into()],
                ..Default::default()
            })
            .unwrap();
        assert!(
            !private.iter().any(|e| e.device_id == ev.device_id),
            "a board row reached a private log"
        );
    }

    a.close_board_net(BOARD);
    wait("closing the board drops its connections", || {
        a.net().board_conns(BOARD).is_empty()
    });
    a.close().unwrap();
    b.close().unwrap();
}

/// The stage 2 path end to end on loopback, with board gossip on: Alice's
/// home server runs (on 127.0.0.1, "reachable from outside" at 127.0.0.1),
/// and her Core writes its `board.server` row into the board she opened.
/// Bob's Core, which has that row and no address for Alice at all, dials
/// her through her relay, and the relay admits Bob's device only as a
/// member device of the board (B1). Then Alice writes without kicking her
/// mesh and an announcement goes out on the board's gossip topic from her
/// endpoint: Bob's board mesh, which joined that topic itself, hears it
/// and pulls the row over `irori/board/1`, well before the board
/// connection's own vector tick (30 s) could have.
///
/// The announcement is sent on a second subscription to the topic on
/// Alice's endpoint (`IrohNet::board_channel_ids`), not by her board
/// mesh's announcer: the only Core lever for that, `Core::kick_board`,
/// also wakes her pull loops, whose vector frame on the board connection
/// would make Bob pull by itself. So this proves Bob's side (his mesh's
/// gossip join, its bootstrap from member keys across the relay, the
/// neighbour gate, announcement to pull) and that Alice's endpoint is on
/// the topic; that her announcer fires on a kick is the mesh crate's to
/// prove, not this test's.
#[test]
fn a_gossip_announcement_pulls_through_a_members_relay() {
    let dirs: Vec<tempfile::TempDir> = (0..6).map(|_| tempfile::tempdir().unwrap()).collect();
    let a = core_with(
        dirs[0].path(),
        dirs[1].path(),
        true,
        Some(Arc::new(|hc: &mut home::Config| {
            hc.bind_ip = Some(std::net::Ipv4Addr::LOCALHOST.into());
            hc.v6_addrs = Some(Arc::new(Vec::new));
            // The board here is opened by hand (open_board_net), not through
            // the Core's board set that `wanted` counts (#63, covered by
            // board_servers.rs): run the server regardless.
            hc.wanted = Some(Arc::new(|| true));
        })),
    );
    let b = core_with(dirs[2].path(), dirs[3].path(), true, None);
    assert_ne!(a.person().id, b.person().id, "two people");
    // On, reachable from outside at a manual address: no port mapping.
    a.set_home_server(true, true, "127.0.0.1").unwrap();
    let (alice, bob, ba, bb) = shared_board(&a, &b, dirs[4].path(), dirs[5].path());

    let (ra, rb) = (a.pairing_runtime().clone(), b.pairing_runtime().clone());
    ra.block_on(a.open_board_net(board_parts(&alice, &ba)))
        .unwrap();
    // Once the server runs, the Core writes the board.server row.
    let home = a.home().expect("alice's home server");
    wait("alice's home server runs", || home.relay_port().is_some());
    let port = i64::from(home.relay_port().unwrap());
    let row = || {
        mesh::boardmesh::board_servers(&ba.log, &ba.verifier)
            .unwrap()
            .into_iter()
            .find(|s| s.device_id == a.device_id())
    };
    wait("alice's Core writes its board.server row", || {
        row().is_some_and(|s| s.public.is_some())
    });
    // An inviter's server is up before the invite goes out: Alice's own
    // endpoint is on her relay, so what Bob sends through it reaches her.
    let since = Instant::now();
    while !a
        .net()
        .home_relays()
        .iter()
        .any(|(u, up)| *up && u.port() == Some(port as u16))
    {
        assert!(
            since.elapsed() < Duration::from_secs(30),
            "timed out waiting for alice's endpoint on her own relay (port {port}): home relays {:?}, relay clients {:?}, tcp connect {:?}",
            a.net().home_relays(),
            home.relay_clients(),
            std::net::TcpStream::connect_timeout(
                &std::net::SocketAddr::from(([127, 0, 0, 1], port as u16)),
                Duration::from_secs(1)
            )
            .map(|_| ()),
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let sv = row().unwrap();
    let public = sv.public.as_ref().unwrap();
    assert_eq!(
        (sv.port, public.ip.as_str(), public.tcp),
        (port, "127.0.0.1", port)
    );
    // The invite bundle carries the board's rows so far, the server's too.
    let rows = ba.log.list(&store::Filter::default()).unwrap();
    bb.log.insert_batch(&rows).1.unwrap();
    let warm = note(&ba.log, &alice, "before bob opens");

    rb.block_on(b.open_board_net(board_parts(&bob, &bb)))
        .unwrap();
    for x in a
        .net()
        .endpoint()
        .expect("bound")
        .bound_sockets()
        .iter()
        .chain(b.net().endpoint().expect("bound").bound_sockets().iter())
    {
        assert!(x.ip().is_loopback(), "non-loopback socket {x}");
    }
    let peer = |c: &Core, id: &str| c.net().board_conns(BOARD).iter().any(|x| x.peer == id);
    let t0 = Instant::now();
    while !(peer(&a, b.device_id()) && peer(&b, a.device_id())) {
        assert!(
            t0.elapsed() < Duration::from_secs(30),
            "timed out waiting for the two Cores to connect on the board: dial errors {:?} / {:?}, alice's relays {:?}, bob's {:?}, on alice's relay {:?}",
            a.net().dial_errors(),
            b.net().dial_errors(),
            a.net().home_relays(),
            b.net().home_relays(),
            home.relay_clients(),
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let connected = Instant::now();
    // Bob's device is on Alice's relay, admitted as a board member only.
    let bob_key = iroh::EndpointId::from_bytes(&b.identity().device_pub()).unwrap();
    wait("alice's relay admits bob's device", || {
        home.relay_clients().contains(&bob_key)
    });
    assert!(
        a.certified_devices()
            .iter()
            .all(|d| d.cert.device_id != b.device_id()),
        "bob's device is not alice's own"
    );

    // Catch-up done: Bob's first pull brought the row written before he
    // opened (no kick, no announcement), so both pull loops are idle,
    // waiting on their tick or a wake, from here on.
    wait("bob's Core pulls the row written before he opened", || {
        has(&bb.log, &warm)
    });
    // Quiet: a row written with no kick and no announcement stays on
    // Alice's side, so the connection alone does not move it before the
    // tick.
    let quiet = note(&ba.log, &alice, "quiet");
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        !has(&bb.log, &quiet),
        "a row moved with no announcement: the connection is not idle"
    );
    // Alice's endpoint announces on the epoch's topic. Gossip drops a
    // repeat of the same message, so each round writes a new row (a new
    // vector) until the topic has formed; the first announcement that
    // lands makes Bob pull, bringing the quiet row too.
    let topic = ba.secrets.topics().unwrap().remove(0);
    let ch = ra
        .block_on(
            a.net()
                .board_channel_ids(BOARD, &topic, vec![bob_key], None),
        )
        .unwrap();
    let mut round = 0;
    wait_for(
        "an announcement makes bob's Core pull",
        Duration::from_secs(20),
        || {
            if has(&bb.log, &quiet) {
                return true;
            }
            if round < 40 {
                note(&ba.log, &alice, &format!("gossip {round}"));
                round += 1;
                ra.block_on(ch.announce(ba.log.vector().unwrap())).unwrap();
                std::thread::sleep(Duration::from_millis(400));
            }
            false
        },
    );
    assert!(
        connected.elapsed() < Duration::from_secs(29),
        "the pull may have been the vector tick's, not the announcement's"
    );
    // Nothing on the private mesh.
    assert!(a.net().conns().is_empty() && b.net().conns().is_empty());

    drop(ch);
    a.close_board_net(BOARD);
    wait("closing the board drops its connections", || {
        a.net().board_conns(BOARD).is_empty()
    });
    a.close().unwrap();
    b.close().unwrap();
}
