//! Boards stage 2 on loopback iroh (docs/design/boards2-transport.md §8.6,
//! card #43): two made-up people share a board over `irori/board/1`, each
//! device's endpoint gated by `MemberAdmit` over its open boards, and a
//! third person who is no member. Every socket is on 127.0.0.1.
//!
//! - a board's rows move both ways on the board connection;
//! - a member device holding the wrong board key is refused before a frame;
//! - a non-member is refused at the endpoint, even holding the board key;
//! - a newer board epoch reaches the other member on `board.secret`;
//! - a removed member's connection is cut and its redials are refused;
//! - the private mesh (`irori/mesh/1`) refuses the other person's device.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ed25519_dalek::{Signer as _, SigningKey};
use gocompat::json::RawJson;
use irori_identity::membership::{BOARD_MEMBER, BOARD_OWNER};
use irori_identity::{
    BoardCreated, BoardMemberAdded, BoardMemberRemoved, BoardSecrets, BoardVerifier, Chain,
    FileKeystore, MemberCert, MemberDeviceAdded, MemberPin, MemberProfile, ROLE_DESKTOP, kinds,
    person_id_of, sign_person,
};
use irori_meshseam::{BoardTransport as _, RoomConfig, Transport as _, mesh_secret};
use irori_net::admit::ChainAdmit;
use irori_net::board::{BoardMembers, MemberAdmit};
use irori_net::endpoint::IpMode;
use irori_net::transport::{IrohNet, NetConfig, PeerAddr};
use mesh::boardmesh::{BoardMesh, BoardMeshOpts};
use store::{Event, Store};

const BOARD: &str = "0123456789abcdef0123456789abcdef";
const WAIT: Duration = Duration::from_secs(20);

fn rand_key() -> SigningKey {
    let mut s = [0u8; 32];
    getrandom::fill(&mut s).unwrap();
    SigningKey::from_bytes(&s)
}

fn now() -> gocompat::time::GoTime {
    gocompat::time::GoTime::from_system(std::time::SystemTime::now())
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

async fn never(what: &str, within: Duration, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < within {
        assert!(!f(), "{what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

struct KeySigner(SigningKey);

impl store::Signer for KeySigner {
    fn sign_row(&self, msg: &[u8]) -> store::Result<Vec<u8>> {
        Ok(self.0.sign(msg).to_bytes().to_vec())
    }
}

/// A made-up person: a person key and a root device.
struct Person {
    key: SigningKey,
    pid: String,
    root: String,
}

impl Person {
    fn new(root: &str) -> Person {
        let key = rand_key();
        Person {
            pid: person_id_of(&key.verifying_key().to_bytes()),
            key,
            root: root.into(),
        }
    }

    fn pin(&self) -> MemberPin {
        MemberPin {
            person_id: self.pid.clone(),
            person_pub: hex::encode(self.key.verifying_key().to_bytes()),
            root_device_id: self.root.clone(),
            person_epoch: 1,
        }
    }
}

/// One person's root device holding the board: its board log, verifier,
/// secrets, endpoint and board mesh.
struct Dev {
    id: String,
    person: Person,
    key: SigningKey,
    log: Arc<Store>,
    verifier: Arc<BoardVerifier>,
    secrets: Arc<BoardSecrets>,
    members: Arc<BoardMembers>,
    net: IrohNet,
    mesh: Option<BoardMesh>,
    dir: tempfile::TempDir,
}

impl Dev {
    fn new(root: &str, creator: &MemberPin) -> Dev {
        Self::with_person(Person::new(root), creator)
    }

    fn with_person(person: Person, creator: &MemberPin) -> Dev {
        Self::build(person, creator, false)
    }

    /// `open_gate`: this device's own endpoint gate lets everyone in, so a
    /// refusal seen here is the other side's.
    fn build(person: Person, creator: &MemberPin, open_gate: bool) -> Dev {
        let dir = tempfile::tempdir().unwrap();
        let key = rand_key();
        let id = person.root.clone();
        let verifier = Arc::new(BoardVerifier::new_for(BOARD, std::slice::from_ref(creator)));
        let log = Arc::new(
            Store::open_with(
                dir.path().join(format!("{BOARD}.db")),
                &id,
                Arc::new(store::Clock::new()),
                store::OpenOptions {
                    signer: Some(Arc::new(KeySigner(key.clone()))),
                    verifier: Some(verifier.clone()),
                    board: true,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let secrets = Arc::new(
            BoardSecrets::open(
                &dir.path().join(format!("{BOARD}.secrets")),
                BOARD,
                Box::new(FileKeystore),
            )
            .unwrap(),
        );
        let members = BoardMembers::new();
        // The private chain knows only this person (no other devices).
        let chain = Arc::new(Chain::new(&person.pid, &person.root, &id));
        let admit: Arc<dyn irori_net::admit::Admit> = if open_gate {
            Arc::new(irori_net::admit::AdmitAll)
        } else {
            Arc::new(MemberAdmit {
                chain: ChainAdmit {
                    chain,
                    self_id: id.clone(),
                },
                boards: members.clone(),
            })
        };
        let mut cfg = NetConfig::new(
            id.clone(),
            iroh::SecretKey::from_bytes(&key.to_bytes()),
            admit,
        );
        cfg.ip = IpMode::Loopback;
        cfg.gossip = false;
        cfg.dial_grace = Duration::from_millis(300);
        cfg.connect_timeout = Duration::from_secs(10);
        Dev {
            id,
            person,
            key,
            log,
            verifier,
            secrets,
            members,
            net: IrohNet::new(cfg),
            mesh: None,
            dir,
        }
    }

    fn pub_hex(&self) -> String {
        hex::encode(self.key.verifying_key().to_bytes())
    }

    fn append<T: serde::Serialize>(&self, actor: &str, kind: &str, v: &T) -> Event {
        self.log
            .append(actor, kind, &RawJson(gocompat::json::marshal(v).unwrap()))
            .unwrap()
    }

    /// This root device's member cert, signed by its person.
    fn self_cert(&self) -> Event {
        let cert = MemberCert {
            person_id: self.person.pid.clone(),
            device_id: self.id.clone(),
            device_pub: self.pub_hex(),
            role: ROLE_DESKTOP.into(),
            person_epoch: 1,
            at: now(),
        };
        let a = MemberDeviceAdded {
            person_sig: sign_person(&self.person.key, &cert).unwrap(),
            cert,
        };
        self.append("irori", kinds::MEMBER_DEVICE_ADDED, &a)
    }

    fn actor(&self) -> String {
        format!("person:{}", self.person.pid)
    }

    fn create_board(&self) -> Event {
        let mut r = BoardCreated {
            board_id: BOARD.into(),
            name: "mlg".into(),
            tag: "mlg".into(),
            creator: self.person.pin(),
            at: now(),
            person_sig: String::new(),
        };
        r.person_sig = sign_person(&self.person.key, &r).unwrap();
        self.append(&self.actor(), kinds::BOARD_CREATED, &r)
    }

    fn add_member(&self, who: &Person) -> Event {
        let pin = who.pin();
        let mut r = BoardMemberAdded {
            person_id: pin.person_id,
            person_pub: pin.person_pub,
            root_device_id: pin.root_device_id,
            person_epoch: 1,
            name: "bob".into(),
            role: BOARD_MEMBER.into(),
            by: self.person.pid.clone(),
            by_epoch: 1,
            at: now(),
            person_sig: String::new(),
        };
        r.person_sig = sign_person(&self.person.key, &r).unwrap();
        self.append(&self.actor(), kinds::BOARD_MEMBER_ADDED, &r)
    }

    fn remove_member(&self, who: &Person, cut: &[(&str, i64)]) -> Event {
        let mut r = BoardMemberRemoved {
            person_id: who.pid.clone(),
            by: self.person.pid.clone(),
            cut: Some(cut.iter().map(|(k, v)| (k.to_string(), *v)).collect()),
            at: now(),
            person_epoch: 1,
            ..Default::default()
        };
        r.person_sig = sign_person(&self.person.key, &r).unwrap();
        self.append(&self.actor(), kinds::BOARD_MEMBER_REMOVED, &r)
    }

    /// A plain board row (a member.profile) to watch move.
    fn note(&self, name: &str) -> Event {
        let p = MemberProfile {
            person_id: self.person.pid.clone(),
            name: name.into(),
            person_epoch: 1,
            person_sig: String::new(),
        };
        self.append("irori", kinds::MEMBER_PROFILE, &p)
    }

    fn take(&self, evs: &[Event]) {
        let (n, r) = self.log.insert_batch(evs);
        r.unwrap();
        assert_eq!(n, evs.len());
    }

    fn has(&self, ev: &Event) -> bool {
        self.log
            .list(&store::Filter {
                device_id: ev.device_id.clone(),
                after_seq: ev.seq - 1,
                limit: 1,
                ..Default::default()
            })
            .is_ok_and(|evs| evs.len() == 1 && store::row_hash(&evs[0]) == store::row_hash(ev))
    }

    async fn peer_addr(&self) -> PeerAddr {
        let ep = self.net.bind().await.unwrap();
        let direct = ep.bound_sockets();
        for a in &direct {
            assert!(a.ip().is_loopback(), "non-loopback socket {a}");
        }
        PeerAddr {
            device_id: self.id.clone(),
            id: iroh::EndpointId::from_bytes(&self.key.verifying_key().to_bytes()).unwrap(),
            relay: None,
            direct,
        }
    }

    /// Opens the board here: counted at the endpoint gate, its mesh started.
    async fn open(&mut self) {
        self.members.add(BOARD, self.verifier.clone());
        let m = BoardMesh::new(BoardMeshOpts {
            board_id: BOARD.into(),
            self_id: self.id.clone(),
            person_id: self.person.pid.clone(),
            core_version: "0.6.0-test".into(),
            log: self.log.clone(),
            verifier: self.verifier.clone(),
            secrets: self.secrets.clone(),
            transport: Arc::new(self.net.clone()),
            own_devices: None,
        });
        m.set_intervals(mesh::Intervals {
            presence: Duration::from_millis(200),
            ping: Duration::from_millis(500),
            vector: Duration::from_millis(300),
            ..Default::default()
        });
        m.start().await.unwrap();
        self.mesh = Some(m);
    }

    fn mesh(&self) -> &BoardMesh {
        self.mesh.as_ref().unwrap()
    }

    fn connected_to(&self, peer: &str) -> bool {
        self.net.board_conns(BOARD).iter().any(|c| c.peer == peer)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_people_share_a_board_and_only_its_members_get_in() {
    // Alice creates the board and adds Bob. Carol is no member; she even
    // holds the board key (leaked), and is still refused.
    let alice_p = Person::new("a-alice-pc");
    let creator = alice_p.pin();
    let mut alice = Dev::with_person(alice_p, &creator);
    let mut bob = Dev::new("b-bob-mac", &creator);
    let mut carol = Dev::new("c-carol-pc", &creator);

    let a_cert = alice.self_cert();
    let created = alice.create_board();
    let added = alice.add_member(&bob.person);
    let b_cert = bob.self_cert();
    // The invite bundle's job: each member starts with the membership rows.
    bob.take(&[a_cert.clone(), created.clone(), added.clone()]);
    alice.take(std::slice::from_ref(&b_cert));
    carol.take(&[a_cert, created, added, b_cert]);
    assert!(alice.verifier.admitted_by_pub(&bob.pub_hex()).is_some());
    assert!(bob.verifier.admitted_by_pub(&alice.pub_hex()).is_some());
    assert!(alice.verifier.admitted_by_pub(&carol.pub_hex()).is_none());
    let roles: Vec<String> = alice
        .verifier
        .membership()
        .members
        .iter()
        .map(|m| m.role.clone())
        .collect();
    assert!(roles.contains(&BOARD_OWNER.to_string()), "{roles:?}");

    // Epoch 1's secret: Alice's; Bob first holds a wrong one.
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).unwrap();
    alice.secrets.add(1, &alice.id, &secret).unwrap();
    carol.secrets.add(1, &alice.id, &secret).unwrap();
    bob.secrets.add(1, &bob.id, &[7u8; 32]).unwrap();

    let (pa, pb, pc) = (
        alice.peer_addr().await,
        bob.peer_addr().await,
        carol.peer_addr().await,
    );
    alice
        .net
        .set_board_peers(BOARD, vec![pb.clone(), pc.clone()]);
    bob.net.set_board_peers(BOARD, vec![pa.clone(), pc.clone()]);
    carol
        .net
        .set_board_peers(BOARD, vec![pa.clone(), pb.clone()]);
    alice.open().await;
    bob.open().await;
    carol.open().await;
    assert_eq!(alice.mesh().member_devices().len(), 1, "alice dials bob");

    // A wrong board key: refused before any frame, both ways.
    eventually("bob's dial with the wrong key fails", || {
        bob.net.dial_errors().contains_key(&alice.id)
    })
    .await;
    never(
        "a connection opened under the wrong board key",
        Duration::from_millis(800),
        || alice.connected_to(&bob.id) || bob.connected_to(&alice.id),
    )
    .await;

    // Bob gets the right secret (Alice's, the lower maker, counts): his
    // room is rejoined under the right key and the two connect.
    bob.secrets.add(1, &alice.id, &secret).unwrap();
    bob.mesh().recheck();
    eventually("alice and bob connect on the board", || {
        alice.connected_to(&bob.id) && bob.connected_to(&alice.id)
    })
    .await;

    // Rows both ways.
    let from_alice = alice.note("from alice");
    alice.mesh().kick();
    eventually("bob pulls alice's row", || bob.has(&from_alice)).await;
    let from_bob = bob.note("from bob");
    bob.mesh().kick();
    eventually("alice pulls bob's row", || alice.has(&from_bob)).await;

    // Carol, no member, is refused at the endpoint; nothing reaches her.
    eventually("carol's dials are refused", || {
        carol.net.dial_errors().contains_key(&alice.id)
    })
    .await;
    never("a non-member connected", Duration::from_millis(800), || {
        alice.connected_to(&carol.id)
            || bob.connected_to(&carol.id)
            || !carol.net.board_conns(BOARD).is_empty()
    })
    .await;
    assert!(!carol.has(&from_alice) && !carol.has(&from_bob));

    // A newer epoch reaches Bob on board.secret, sealed under epoch 1.
    alice.secrets.mint(2, &alice.id).unwrap();
    alice.mesh().recheck();
    eventually("bob holds epoch 2", || bob.secrets.epoch() == 2).await;
    assert_eq!(
        bob.secrets.secret(2).map(|(s, m)| (s.to_vec(), m)),
        alice.secrets.secret(2).map(|(s, m)| (s.to_vec(), m)),
    );
    eventually("alice and bob meet on epoch 2", || {
        alice
            .net
            .board_conns(BOARD)
            .iter()
            .any(|c| c.peer == bob.id && c.epoch == 2)
    })
    .await;
    assert!(carol.secrets.epoch() == 1, "carol got a newer secret");

    // Alice removes Bob: his connections are cut, and his redials refused.
    let bob_seq = bob.log.vector().unwrap().get(&bob.id).copied().unwrap_or(0);
    alice.remove_member(&bob.person, &[(&bob.id, bob_seq)]);
    assert!(alice.verifier.admitted_by_pub(&bob.pub_hex()).is_none());
    alice.mesh().recheck();
    eventually("bob's connections are cut", || !alice.connected_to(&bob.id)).await;
    never(
        "a removed member reconnected",
        Duration::from_millis(1500),
        || alice.connected_to(&bob.id),
    )
    .await;
    let after = alice.note("after bob left");
    alice.mesh().kick();
    never(
        "a removed member pulled a row",
        Duration::from_millis(800),
        || bob.has(&after),
    )
    .await;

    for d in [&alice, &bob, &carol] {
        d.mesh().close();
        d.net.close().await;
    }
    let _ = (&alice.dir, &bob.dir, &carol.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_a_removal_the_old_topic_closes_once_every_member_moved() {
    // Alice, Bob and Carol share a board on epoch 1. Alice removes Bob and
    // rotates to epoch 2 while Carol is offline: epoch 1's room stays open
    // for Carol. When Carol comes back and says hello on epoch 2, Alice and
    // Carol settle and leave epoch 1; Bob, who never got epoch 2, has no
    // topic left in common with them and no new rows.
    let alice_p = Person::new("a-alice-pc");
    let creator = alice_p.pin();
    let mut alice = Dev::with_person(alice_p, &creator);
    let mut bob = Dev::new("b-bob-mac", &creator);
    let mut carol = Dev::new("c-carol-pc", &creator);

    let a_cert = alice.self_cert();
    let created = alice.create_board();
    let added_b = alice.add_member(&bob.person);
    let added_c = alice.add_member(&carol.person);
    let b_cert = bob.self_cert();
    let c_cert = carol.self_cert();
    bob.take(&[
        a_cert.clone(),
        created.clone(),
        added_b.clone(),
        added_c.clone(),
    ]);
    bob.take(std::slice::from_ref(&c_cert));
    carol.take(&[a_cert, created, added_b, added_c]);
    carol.take(std::slice::from_ref(&b_cert));
    alice.take(&[b_cert, c_cert]);
    assert!(alice.verifier.admitted_by_pub(&bob.pub_hex()).is_some());
    assert!(alice.verifier.admitted_by_pub(&carol.pub_hex()).is_some());

    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).unwrap();
    for d in [&alice, &bob, &carol] {
        d.secrets.add(1, &alice.id, &secret).unwrap();
    }
    let (pa, pb, pc) = (
        alice.peer_addr().await,
        bob.peer_addr().await,
        carol.peer_addr().await,
    );
    alice
        .net
        .set_board_peers(BOARD, vec![pb.clone(), pc.clone()]);
    bob.net.set_board_peers(BOARD, vec![pa.clone(), pc.clone()]);
    carol
        .net
        .set_board_peers(BOARD, vec![pa.clone(), pb.clone()]);
    alice.open().await;
    bob.open().await;
    eventually("alice and bob connect on epoch 1", || {
        alice.connected_to(&bob.id) && bob.connected_to(&alice.id)
    })
    .await;
    let before = alice.note("before");
    alice.mesh().kick();
    eventually("bob pulls alice's row", || bob.has(&before)).await;

    // Carol is offline. Alice removes Bob and rotates.
    let bob_seq = bob.log.vector().unwrap().get(&bob.id).copied().unwrap_or(0);
    alice.remove_member(&bob.person, &[(&bob.id, bob_seq)]);
    alice.secrets.mint(2, &alice.id).unwrap();
    alice.mesh().recheck();
    eventually("alice joins epoch 2", || {
        alice.mesh().epochs() == vec![1, 2]
    })
    .await;
    eventually("bob's connections are cut", || {
        !alice.connected_to(&bob.id) && bob.net.board_conns(BOARD).is_empty()
    })
    .await;
    // Not every remaining member has moved: epoch 1 stays open for Carol.
    never(
        "alice settled before carol said hello on epoch 2",
        Duration::from_millis(1000),
        || alice.secrets.topics().unwrap().len() != 2 || alice.mesh().epochs() != vec![1, 2],
    )
    .await;
    assert_eq!(alice.secrets.settled(), 0);

    // Carol comes back holding Alice's rows (the removal among them) and
    // only epoch 1.
    let rows = alice.log.list(&store::Filter::default()).unwrap();
    let (_, r) = carol.log.insert_batch(&rows);
    r.unwrap();
    assert!(carol.verifier.admitted_by_pub(&bob.pub_hex()).is_none());
    carol.open().await;
    eventually("carol gets epoch 2 on board.secret", || {
        carol.secrets.epoch() == 2
    })
    .await;
    eventually("alice and carol settle on epoch 2 and leave epoch 1", || {
        alice.secrets.settled() == 2
            && carol.secrets.settled() == 2
            && alice.mesh().epochs() == vec![2]
            && carol.mesh().epochs() == vec![2]
            && !alice.net.board_rooms().iter().any(|(_, e)| *e == 1)
            && !carol.net.board_rooms().iter().any(|(_, e)| *e == 1)
    })
    .await;
    assert_eq!(alice.secrets.topics().unwrap().len(), 1);
    assert_eq!(alice.secrets.topics().unwrap()[0].epoch, 2);
    assert!(alice.secrets.secret(1).is_none(), "epoch 1's secret is gone");
    assert_eq!(alice.mesh().peer_epochs().get(&carol.id), Some(&2));
    assert!(
        alice
            .net
            .board_conns(BOARD)
            .iter()
            .all(|c| c.epoch == 2),
        "{:?}",
        alice.net.board_conns(BOARD)
    );

    // Bob never got epoch 2, and nobody else is on epoch 1 any more.
    assert_eq!(bob.secrets.epoch(), 1);
    assert!(bob.secrets.secret(2).is_none());
    let after = alice.note("after the rotation");
    alice.mesh().kick();
    eventually("carol pulls the new row", || carol.has(&after)).await;
    never(
        "the removed member reached the board",
        Duration::from_millis(1500),
        || {
            bob.has(&after)
                || !bob.net.board_conns(BOARD).is_empty()
                || alice.connected_to(&bob.id)
                || carol.connected_to(&bob.id)
        },
    )
    .await;

    for d in [&alice, &bob, &carol] {
        d.mesh().close();
        d.net.close().await;
    }
    let _ = (&alice.dir, &bob.dir, &carol.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_private_mesh_refuses_another_persons_device() {
    let alice_p = Person::new("a-alice-pc");
    let creator = alice_p.pin();
    let mut alice = Dev::with_person(alice_p, &creator);
    // Bob's own gate is open: every refusal below is Alice's endpoint.
    let bob = Dev::build(Person::new("b-bob-mac"), &creator, true);
    let a_cert = alice.self_cert();
    let created = alice.create_board();
    let added = alice.add_member(&bob.person);
    let b_cert = bob.self_cert();
    bob.take(&[a_cert, created, added]);
    alice.take(&[b_cert]);
    // Bob is a member device of a board Alice holds, so her endpoint lets
    // him in on irori/board/1 — and on nothing private.
    alice.open_counted_only();
    let pa = alice.peer_addr().await;

    // Both in a private mesh room under the same key: the handshake gate
    // still refuses, since Bob's device is not one of Alice's.
    let s = mesh_secret(&[5u8; 32], "shared-for-test", 1);
    let room_a = alice
        .net
        .join(s.clone(), RoomConfig::new(alice.id.clone()))
        .await
        .unwrap();
    bob.net.set_peers(vec![pa.clone()]);
    let room_b = bob
        .net
        .join(s, RoomConfig::new(bob.id.clone()))
        .await
        .unwrap();
    room_b.connect(&alice.id);
    eventually("bob's private dial fails", || {
        bob.net.dial_errors().contains_key(&alice.id)
    })
    .await;
    let err = bob.net.dial_errors()[&alice.id].clone();
    assert!(err.contains("refused"), "{err}");
    assert!(alice.net.conns().is_empty() && bob.net.conns().is_empty());

    // The same key on irori/board/1 is let through the gate (the board
    // room's own checks then decide): a member device somewhere here.
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).unwrap();
    let topic = irori_identity::derive_board_topic(BOARD, 1, &secret).unwrap();
    let mut cfg = RoomConfig::new(alice.id.clone());
    {
        let v = alice.verifier.clone();
        cfg.accept = Arc::new(move |_peer: &str, pk: &[u8; 32]| {
            v.admitted_by_pub(&hex::encode(pk))
                .map(|_| ())
                .ok_or(irori_meshseam::Refused)
        });
    }
    let board_a = alice
        .net
        .join_board(
            irori_meshseam::BoardRoom {
                board_id: BOARD.into(),
                secret: irori_meshseam::Secret {
                    key: topic.key,
                    topic: topic.topic.clone(),
                    epoch: 1,
                },
            },
            cfg,
        )
        .await
        .unwrap();
    bob.net.set_board_peers(BOARD, vec![pa]);
    let board_b = bob
        .net
        .join_board(
            irori_meshseam::BoardRoom {
                board_id: BOARD.into(),
                secret: irori_meshseam::Secret {
                    key: topic.key,
                    topic: topic.topic,
                    epoch: 1,
                },
            },
            RoomConfig::new(bob.id.clone()),
        )
        .await
        .unwrap();
    board_b.connect(&alice.id);
    let t = Instant::now();
    while !alice.connected_to(&bob.id) {
        assert!(
            t.elapsed() < WAIT,
            "bob never reached alice on the board connection: {:?}",
            bob.net.dial_errors()
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(alice.net.conns().is_empty(), "nothing on irori/mesh/1");
    for r in [&room_a, &room_b, &board_a, &board_b] {
        r.close();
    }
    alice.net.close().await;
    bob.net.close().await;
}

impl Dev {
    /// Counts the board at the endpoint gate without starting a mesh.
    fn open_counted_only(&mut self) {
        self.members.add(BOARD, self.verifier.clone());
    }
}
