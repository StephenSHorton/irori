//! Boards stage 2 through two Cores (card #43): two made-up people's Cores,
//! each its own first-run person on loopback iroh, open one shared board on
//! the network (`Core::open_board_net`) and its rows move both ways on
//! `irori/board/1`, while their private meshes never connect. The board's
//! people and their keys are made up for the test; the board logs are
//! signed by each Core's own device key. No LAN lookup, nothing off
//! 127.0.0.1.

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

fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "timed out waiting for: {what}"
        );
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
    let mut o = common::options(data, claude);
    o.pair_net = Some(Arc::new(move |ident, chain, guests| {
        let mut nc = devices::default_net_config(ident, chain, guests);
        nc.ip = IpMode::Loopback;
        nc.lan_config = None;
        nc.gossip = false;
        IrohNet::new(nc)
    }));
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

#[test]
fn two_peoples_cores_sync_a_shared_board_on_its_own_connection() {
    let dirs: Vec<tempfile::TempDir> = (0..6).map(|_| tempfile::tempdir().unwrap()).collect();
    let a = core(dirs[0].path(), dirs[1].path());
    let b = core(dirs[2].path(), dirs[3].path());
    assert_ne!(a.person().id, b.person().id, "two people");

    // Alice (on Core a) creates the board and adds Bob (on Core b).
    let (alice, bob) = (Member::of(&a), Member::of(&b));
    let creator = alice.pin();
    let ba = open_board(&a, dirs[4].path(), &creator);
    let bb = open_board(&b, dirs[5].path(), &creator);
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

    let (ra, rb) = (a.pairing_runtime().clone(), b.pairing_runtime().clone());
    ra.block_on(a.open_board_net(BoardParts {
        board_id: BOARD.into(),
        person_id: alice.pid.clone(),
        log: ba.log.clone(),
        verifier: ba.verifier.clone(),
        secrets: ba.secrets.clone(),
    }))
    .unwrap();
    rb.block_on(b.open_board_net(BoardParts {
        board_id: BOARD.into(),
        person_id: bob.pid.clone(),
        log: bb.log.clone(),
        verifier: bb.verifier.clone(),
        secrets: bb.secrets.clone(),
    }))
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
