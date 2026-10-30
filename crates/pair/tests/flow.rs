//! pair_test.go's nine flow tests (identity.md §7.2 "flow tests") on
//! FakeNet with real stores, keys, chains and mesh Nodes.

#![allow(clippy::field_reassign_with_default)]

mod common;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use common::{Dev, eventually};
use gocompat::time::GoTime;
use irori_meshseam as seam;
use irori_pair::wire::{Frame, Hello, WirePerson};
use irori_pair::*;
use mesh::FakeNet;
use store::Filter;

const Z: Duration = Duration::ZERO;

async fn pair_up(a: &Dev, b: &Dev) {
    let st = a.pair.add(irori_identity::ROLE_DESKTOP).await.unwrap();
    assert!(!st.code.is_empty(), "add status {st:?}");
    b.pair.join(&st.code.to_lowercase()).await.unwrap();
    eventually("b shows words", || b.state() == STATE_WORDS).await;
    eventually("a asks for the words", || a.state() == STATE_CONFIRM).await;
    let j = a.pair.status().joiner.expect("joiner");
    assert_eq!(j.device_id, b.id());
    assert_eq!(j.name, b.name);
    assert_eq!(j.fingerprint, fingerprint(&b.ident.device_pub()));
    a.pair
        .confirm(&b.pair.status().words.join(" "))
        .await
        .unwrap();
    eventually("a waits for its key on b", || a.state() == STATE_GRANTED).await;
    assert!(
        a.chain.cert(&b.id()).is_none(),
        "a certified b before b's person typed a's key"
    );
    b.pair
        .confirm_key(&a.pair.status().fingerprint)
        .await
        .unwrap();
    eventually("a done", || a.state() == STATE_DONE).await;
    eventually("b done", || b.state() == STATE_DONE).await;
}

fn secret(code: &str) -> seam::Secret {
    room_secret(code).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pair_copies_the_person_and_syncs() {
    let net = FakeNet::new();
    let a = Dev::new(&net, "pc", Z);
    let mut b = Dev::new(&net, "mac", Z);
    let before =
        b.st.append(
            "irori",
            "device.started",
            &serde_json::json!({"name": "mac"}),
        )
        .unwrap();
    let st = a.pair.add(irori_identity::ROLE_NODE).await.unwrap();
    b.pair.join(&st.code).await.unwrap();
    eventually("b shows words", || b.state() == STATE_WORDS).await;
    eventually("a asks for the words", || a.state() == STATE_CONFIRM).await;
    assert!(
        a.pair.status().code.is_empty(),
        "the code still shows once a device used it"
    );
    let words = b.pair.status().words;
    assert_eq!(
        a.pair
            .confirm("aardvark adroitness aardvark adroitness aardvark")
            .await,
        Err(Error::WrongWords)
    );
    let s = a.pair.status();
    assert!(
        s.state == STATE_CONFIRM && s.misses == 1,
        "one miss closed the window: {s:?}"
    );
    // B's key check can come first: the grant then goes through at once.
    assert_eq!(
        b.pair.confirm_key("0000 0000 0000 0000").await,
        Err(Error::WrongKey)
    );
    b.pair
        .confirm_key(
            &fingerprint(&a.ident.device_pub())
                .replace(' ', "-")
                .to_uppercase(),
        )
        .await
        .unwrap();
    a.pair.confirm(&words.join(", ")).await.unwrap();
    eventually("both done", || {
        a.state() == STATE_DONE && b.state() == STATE_DONE
    })
    .await;

    let (pa, pb) = (a.ident.current(), b.ident.current());
    assert_eq!(pb.id, pa.id);
    assert_eq!(pb.root_device_id, a.id());
    assert_eq!(pb.epoch, pa.epoch);
    assert_eq!(pb.pub_hex, pa.pub_hex);
    let dv = irori_identity::DeviceJson::load_or_create(&b.dir).unwrap();
    assert_eq!(dv.person_id, pa.id, "b's device.json names another person");
    let cert = a.chain.cert(&b.id()).expect("a's cert for b");
    assert_eq!(cert.role, irori_identity::ROLE_NODE);
    assert_eq!(cert.label, "mac");
    assert!(
        b.chain.cert(&b.id()).is_some(),
        "b doesn't hold its own cert"
    );
    assert!(
        net.members(&secret(&st.code).topic).is_empty(),
        "someone is still in the pairing room"
    );
    // Both reconnect on the mesh topic, and b's earlier rows reach a.
    eventually("b's earlier row on a", || a.held(&before)).await;
    eventually("a's cert for b stored on b", || {
        b.st.list(&Filter {
            kinds: vec![irori_identity::kinds::PERSON_DEVICE_ADDED.into()],
            device_id: a.id(),
            ..Filter::default()
        })
        .map(|e| e.len() >= 2)
        .unwrap_or(false)
    })
    .await;
    assert!(
        b.dir.join(ROWS_FILE).exists(),
        "b keeps the pairing rows until restart"
    );
    // B restarted: the rows are held now, so the file goes.
    b.close().await;
    b.open(Z);
    assert!(
        !b.dir.join(ROWS_FILE).exists(),
        "rows file after the rows were stored"
    );
    assert_eq!(b.pair.join(&new_code()).await.unwrap_err(), Error::InMesh);
}

/// B paired while A's net was down: B restarts before any sync and still
/// holds its cert (the saved rows), so its net layer starts and finds A.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pairing_survives_a_restart_before_the_first_sync() {
    let net = FakeNet::new();
    let mut a = Dev::with(&net, "pc", Z, true);
    let mut b = Dev::new(&net, "mac", Z);
    pair_up(&a, &b).await;
    a.close().await;
    b.close().await;
    b.open(Z);
    let held =
        b.st.list(&Filter {
            device_id: a.id(),
            ..Filter::default()
        })
        .unwrap();
    assert!(
        held.is_empty(),
        "b holds {} of a's rows: the test needs no sync before the restart",
        held.len()
    );
    a.no_net = false;
    assert!(
        b.chain.cert(&b.id()).is_some(),
        "after a restart b lost its cert"
    );
    eventually("b's node wants a peer", || b.node.has_peer()).await;
    a.open(Z);
    eventually("a and b connect", || net.connected(&a.id(), &b.id())).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn second_joiner_gets_busy_and_the_window_closes() {
    let net = FakeNet::new();
    let (a, b, c) = (
        Dev::new(&net, "pc", Z),
        Dev::new(&net, "mac", Z),
        Dev::new(&net, "thief", Z),
    );
    let st = a.pair.add(irori_identity::ROLE_DESKTOP).await.unwrap();
    b.pair.join(&st.code).await.unwrap();
    eventually("a asks for the words", || a.state() == STATE_CONFIRM).await;
    c.pair.join(&st.code).await.unwrap();
    eventually("the second joiner hears busy", || {
        c.state() == STATE_CODE_IN_USE
    })
    .await;
    eventually("a's window closes", || a.state() == STATE_FAILED).await;
    assert_eq!(a.pair.status().error, ERR_TEXT_SOMEONE_ELSE);
    assert!(
        c.pair.status().words.is_empty(),
        "the second joiner got words"
    );
    eventually("b hears the window closed", || b.state() == STATE_FAILED).await;
    assert!(a.chain.cert(&b.id()).is_none(), "b was certified");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wrong_words_twice_close_the_window() {
    let net = FakeNet::new();
    let (a, b) = (Dev::new(&net, "pc", Z), Dev::new(&net, "mac", Z));
    let st = a.pair.add(irori_identity::ROLE_DESKTOP).await.unwrap();
    let _ = b.pair.join(&st.code).await;
    eventually("a asks for the words", || a.state() == STATE_CONFIRM).await;
    for _ in 0..MAX_MISSES {
        assert_eq!(a.pair.confirm("wrong words").await, Err(Error::WrongWords));
    }
    let s = a.pair.status();
    assert!(
        s.state == STATE_FAILED && s.error == ERR_TEXT_SOMEONE_ELSE,
        "after two misses: {s:?}"
    );
    eventually("b closes too", || b.state() == STATE_FAILED).await;
    assert_eq!(
        a.pair.confirm(&sas(&[0u8; 32], &[]).join(" ")).await,
        Err(Error::NotOpen)
    );
}

/// A forger's room on the same FakeNet: every handler is the test's.
async fn forger_room(
    net: &FakeNet,
    code: &str,
    self_id: &str,
    pub_key: [u8; 32],
    offer: bool,
    conns: tokio::sync::mpsc::UnboundedSender<Arc<dyn seam::Conn>>,
    on_msg: Option<seam::MsgFn>,
) -> Arc<dyn seam::Room> {
    let t = net.keyed(pub_key);
    let mut cfg = seam::RoomConfig::new(self_id);
    if offer {
        cfg.offerer = Some(Arc::new(|_| true));
    }
    cfg.on_peer = Arc::new(move |c| {
        let _ = conns.send(c);
    });
    if let Some(m) = on_msg {
        cfg.on_msg = m;
    }
    seam::Transport::join(&t, secret(code), cfg).await.unwrap()
}

fn rand_key() -> [u8; 32] {
    let mut k = [0u8; 32];
    getrandom::fill(&mut k).unwrap();
    k
}

/// A code holder that can't prove it (a forger speaking frames by hand)
/// closes the window: the HMAC failure.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bad_proof_closes_the_window() {
    let net = FakeNet::new();
    let a = Dev::new(&net, "pc", Z);
    let st = a.pair.add(irori_identity::ROLE_DESKTOP).await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let r = forger_room(&net, &st.code, "forger", rand_key(), false, tx, None).await;
    let hello = serde_json::to_value(Hello {
        device_id: "forger".into(),
        name: "x".into(),
        os: "y".into(),
    })
    .unwrap();
    r.send("", MSG_HELLO, hello).unwrap();
    let c = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("a never connected to the joiner")
        .unwrap();
    let mut bad = Frame::default();
    bad.typ = "pair.proof".into();
    bad.proof = Some(vec![0u8; 32]);
    c.send(&serde_json::to_vec(&bad).unwrap()).await.unwrap();
    eventually("a's window closes", || a.state() == STATE_FAILED).await;
    assert_eq!(a.pair.status().error, ERR_TEXT_SOMEONE_ELSE);
    r.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_window_expires() {
    let net = FakeNet::new();
    let a = Dev::new(&net, "pc", Duration::from_millis(150));
    let st = a.pair.add(irori_identity::ROLE_DESKTOP).await.unwrap();
    eventually("the window expires", || a.state() == STATE_EXPIRED).await;
    assert!(
        net.members(&secret(&st.code).topic).is_empty(),
        "a stayed in the pairing room"
    );
    // The full window from here: each Add derives its room key (scrypt)
    // before it checks for an open window.
    a.pair.set_window(Z);
    a.pair
        .add(irori_identity::ROLE_DESKTOP)
        .await
        .expect("a new window after expiry");
    assert_eq!(
        a.pair.add(irori_identity::ROLE_DESKTOP).await.unwrap_err(),
        Error::Busy
    );
}

/// A revoked device can't come back through a pairing room: A ignores its
/// hello, so nothing connects.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_revoked_device_cant_pair_again() {
    let net = FakeNet::new();
    let (a, b) = (Dev::new(&net, "pc", Z), Dev::new(&net, "mac", Z));
    pair_up(&a, &b).await;
    a.node.revoke(&b.id(), "lost").await.unwrap();
    let st = a.pair.add(irori_identity::ROLE_DESKTOP).await.unwrap();
    // b is in a mesh (its own revoke aside), so its Join is refused anyway;
    // speak its hello by hand to show A ignores it.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let r = forger_room(
        &net,
        &st.code,
        &b.id(),
        b.ident.device_pub(),
        false,
        tx,
        None,
    )
    .await;
    let hello = serde_json::to_value(Hello {
        device_id: b.id(),
        name: "mac".into(),
        os: String::new(),
    })
    .unwrap();
    for _ in 0..5 {
        let _ = r.send("", MSG_HELLO, hello.clone());
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(300), rx.recv())
            .await
            .is_err(),
        "a connected to a revoked device in the pairing room"
    );
    let s = a.pair.status();
    assert_eq!(
        s.state, STATE_WAITING,
        "a's window after a revoked hello: {s:?}"
    );
    r.close();
}

/// Someone else with A's code reaches B first, posing as A: it proves the
/// code, and hands B a person of its own that certifies B. B holds the
/// grant until the person types A's key, which the impostor's handshake
/// can't prove, so B never adopts.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_code_holder_posing_as_a_cant_hand_b_a_person() {
    let net = FakeNet::new();
    let (a, b, thief) = (
        Dev::new(&net, "pc", Z),
        Dev::new(&net, "mac", Z),
        Dev::new(&net, "thief", Z),
    );
    let st = a.pair.add(irori_identity::ROLE_DESKTOP).await.unwrap();
    a.pair.cancel(); // the real A waits elsewhere; only the thief answers
    let sec = secret(&st.code);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let room: Arc<std::sync::OnceLock<Arc<dyn seam::Room>>> = Arc::default();
    let room2 = room.clone();
    let on_msg: seam::MsgFn = Arc::new(move |env: seam::Envelope| {
        if env.typ == MSG_HELLO
            && let Some(r) = room2.get()
        {
            r.connect(&env.from);
        }
    });
    let r = forger_room(
        &net,
        &st.code,
        &thief.id(),
        thief.ident.device_pub(),
        true,
        tx,
        Some(on_msg),
    )
    .await;
    let _ = room.set(r.clone());
    let before = b.ident.current();
    b.pair.join(&st.code).await.unwrap();
    let c = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("b never connected to the thief")
        .unwrap();
    let (local, remote) = c.fingerprints();
    let inf = info(
        &thief.ident.device_pub(),
        &c.pub_key(),
        &local,
        &remote,
        &thief.id(),
        &b.id(),
    );
    let mut pf = Frame::default();
    pf.typ = "pair.proof".into();
    pf.proof = Some(proof(&sec.key, b'A', &inf));
    pf.name = "pc".into();
    c.send(&serde_json::to_vec(&pf).unwrap()).await.unwrap();
    eventually("b shows words", || b.state() == STATE_WORDS).await;
    let add = thief
        .ident
        .cert_for(
            &b.id(),
            &hex::encode(b.ident.device_pub()),
            "mac",
            irori_identity::ROLE_DESKTOP,
            GoTime::from_system(SystemTime::now()),
        )
        .unwrap();
    thief
        .st
        .append("irori", irori_identity::kinds::PERSON_DEVICE_ADDED, &add)
        .unwrap();
    let rows: Vec<_> = thief
        .st
        .list(&Filter {
            kinds: vec!["person.".into()],
            limit: 1000,
            ..Filter::default()
        })
        .unwrap()
        .iter()
        .map(irori_pair::wire::WireRow::of)
        .collect();
    let cur = thief.ident.current();
    let mut g = Frame::default();
    g.typ = "pair.grant".into();
    g.person = Some(WirePerson::from(&cur));
    g.seed = Some(thief.ident.person_seed(cur.epoch).unwrap().to_vec());
    g.rows = rows;
    c.send(&serde_json::to_vec(&g).unwrap()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let s = b.pair.status();
    assert!(
        s.state == STATE_WORDS && b.ident.current().id == before.id,
        "b took a grant before its key was typed: {s:?}"
    );
    // The person types the real A's key, twice: the window closes.
    for _ in 0..MAX_MISSES {
        assert_eq!(
            b.pair.confirm_key(&st.fingerprint).await,
            Err(Error::WrongKey)
        );
    }
    let s = b.pair.status();
    assert!(
        s.state == STATE_FAILED && s.error == ERR_TEXT_NOT_THAT_KEY,
        "after two misses: {s:?}"
    );
    assert_eq!(
        b.ident.current().id,
        before.id,
        "b adopted the thief's person"
    );
    r.close();
}

/// The person mistypes A's key twice on B: the window closes on both, and A
/// wrote no cert for B, so B can simply pair again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wrong_key_twice_leaves_no_cert() {
    let net = FakeNet::new();
    let (a, b) = (Dev::new(&net, "pc", Z), Dev::new(&net, "mac", Z));
    let st = a.pair.add(irori_identity::ROLE_DESKTOP).await.unwrap();
    b.pair.join(&st.code).await.unwrap();
    eventually("b shows words", || b.state() == STATE_WORDS).await;
    eventually("a asks for the words", || a.state() == STATE_CONFIRM).await;
    a.pair
        .confirm(&b.pair.status().words.join(" "))
        .await
        .unwrap();
    for _ in 0..MAX_MISSES {
        assert_eq!(
            b.pair.confirm_key("0000 0000 0000 0000").await,
            Err(Error::WrongKey)
        );
    }
    eventually("a's window closes", || a.state() == STATE_FAILED).await;
    assert!(
        a.chain.cert(&b.id()).is_none(),
        "a certified b although b never took the pairing"
    );
    pair_up(&a, &b).await;
}
