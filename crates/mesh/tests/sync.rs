//! I:internal/mesh/sync_test.go on FakeNet (mesh.md §6).

mod common;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use common::{Dev, converged, eventually, never, trio};
use ed25519_dalek::{Signer as _, SigningKey};
use irori_identity::{Cert, DeviceAdded, objects::canonical};
use mesh::frames::{FRAME_HELLO, FRAME_PERSON_SECRET, FRAME_ROWS, FRAME_VECTOR, PROTOCOL};
use mesh::{
    Conn, DeviceRow, FakeNet, Frame, MSG_PRESENCE, Presence, Room, RoomConfig, Row, STATE_OFF,
    Secret, Transport, decode_frame, mesh_secret,
};
use serde_json::json;
use store::{Event, Filter, Timestamp};
use tokio::sync::mpsc;

fn key() -> SigningKey {
    let mut s = [0u8; 32];
    getrandom::fill(&mut s).unwrap();
    SigningKey::from_bytes(&s)
}

/// Joins `s` as an arbitrary device and hands back its connections: a
/// forger with no cert, speaking frames by hand.
async fn raw_peer(
    net: &FakeNet,
    s: Secret,
    self_id: &str,
) -> (Arc<dyn Room>, mpsc::Receiver<Arc<dyn Conn>>) {
    let (tx, rx) = mpsc::channel(4);
    // Admits and allows everyone.
    let mut cfg = RoomConfig::new(self_id);
    cfg.on_peer = Arc::new(move |c: Arc<dyn Conn>| {
        let _ = tx.try_send(c);
    });
    let net = net.keyed(key().verifying_key().to_bytes());
    (net.join(s, cfg).await.unwrap(), rx)
}

async fn send_frame(c: &Arc<dyn Conn>, f: &Frame) {
    c.send(&f.encode()).await.unwrap();
}

async fn recv_frame(c: &Arc<dyn Conn>, typ: &str, within: Duration) -> Result<Frame, String> {
    tokio::time::timeout(within, async {
        loop {
            let b = c.recv().await.map_err(|e| e.to_string())?;
            let f = decode_frame(&b)?;
            if f.typ == typ {
                return Ok(f);
            }
        }
    })
    .await
    .map_err(|_| "timeout".to_string())?
}

fn unix_ms() -> i64 {
    hlc::unix_milli(SystemTime::now())
}

fn seen(d: &Dev, tag: &str) -> serde_json::Value {
    json!({"key": format!("{}/{tag}", d.id())})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_verifier_reset_on_a_dead_context_keeps_the_certs() {
    let net = FakeNet::new();
    let (a, mut b) = (Dev::new(&net, "pc"), Dev::new(&net, "mac"));
    b.join(&a).await;
    a.st.reload_verifier();
    assert!(a.chain.cert(&b.id()).is_some(), "the reset lost the certs");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_device_opens_nothing() {
    let net = FakeNet::new();
    let mut a = Dev::new(&net, "pc");
    a.append(store::kinds::DEVICE_STARTED, json!({"name": "pc"}));
    a.start();
    never("a lone device joins a topic", || net.joins() > 0).await;
    let st = a.node().status();
    assert!(
        st.state == STATE_OFF && st.topics == 0,
        "lone device status {st:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pairing_starts_the_net_and_earlier_rows_replicate() {
    let net = FakeNet::new();
    let (mut a, mut b) = (Dev::new(&net, "pc"), Dev::new(&net, "mac"));
    let before = b.append(store::kinds::AGENT_SEEN, seen(&b, "s1"));
    a.append(store::kinds::DEVICE_STARTED, json!({"name": "pc"}));
    a.start();
    never(
        "the root joins a topic before it has a second device",
        || net.joins() > 0,
    )
    .await;
    b.join(&a).await;
    a.node().kick();
    b.start();
    eventually("a and b converge", || converged(&[&a, &b])).await;
    assert!(
        a.has(&before),
        "b's row from before pairing did not replicate"
    );
    let cert = a.chain.cert(&b.id()).expect("a's cert for b");
    assert_eq!(cert.device_pub, hex::encode(b.ident.device_pub()));
    let ea = a.append(store::kinds::AGENT_SEEN, seen(&a, "s2"));
    let eb = b.append(store::kinds::AGENT_SEEN, seen(&b, "s3"));
    eventually("live rows cross", || a.has(&eb) && b.has(&ea)).await;
    eventually("b is connected", || a.peer(&b.id()).state == "connected").await;
    let rows = a.node().devices(false);
    assert!(
        rows.len() == 2 && rows[1].last_seen.is_some(),
        "a's devices: {rows:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_kinds_never_cross() {
    let net = FakeNet::new();
    let (mut a, mut b) = (Dev::new(&net, "pc"), Dev::new(&net, "mac"));
    b.join(&a).await;
    a.st.append_local("user", store::kinds::POLICY_SET, &json!({"paused": true}))
        .unwrap();
    a.start();
    b.start();
    eventually("converge", || converged(&[&a, &b])).await;
    let local =
        b.st.list(&Filter {
            local: true,
            ..Filter::default()
        })
        .unwrap();
    assert!(local.is_empty(), "b's local rows: {local:?}");
    let evs =
        b.st.list(&Filter {
            kinds: vec![store::kinds::POLICY_SET.into()],
            ..Filter::default()
        })
        .unwrap();
    assert!(evs.is_empty(), "a policy row reached b");
}

fn signed(k: &SigningKey, mut ev: Event) -> Event {
    ev.sig = k.sign(&store::signing_bytes(&ev)).to_bytes().to_vec();
    ev
}

/// A forger connects to a's topic as a device a doesn't know. With the
/// cert check on it never connects; with it off nothing it sends is
/// stored (both layers hold).
async fn forged_rows_refused(trust: bool) {
    let net = FakeNet::new();
    let (mut a, mut b) = (Dev::new(&net, "pc"), Dev::new(&net, "mac"));
    b.join(&a).await;
    a.trust = trust;
    a.start();
    b.start();
    eventually("converge", || converged(&[&a, &b])).await;
    let pid = a.ident.current().id;
    let secret = mesh_secret(&a.seed(1), &pid, 1);
    let (room, mut conns) = raw_peer(&net, secret, "ffffffffforger").await;
    let pres = gocompat::json::marshal(&Presence {
        device_id: "ffffffffforger".into(),
        epoch: 1,
        core_version: String::new(),
    })
    .unwrap();
    let _ = room.send("", MSG_PRESENCE, serde_json::from_slice(&pres).unwrap());
    room.connect(&a.id());
    if !trust {
        never("the forger connects", || !conns.is_empty()).await;
        room.close();
        return;
    }
    let c = tokio::time::timeout(Duration::from_secs(5), conns.recv())
        .await
        .expect("forger never connected with the cert check off")
        .unwrap();
    send_frame(
        &c,
        &Frame {
            v: PROTOCOL,
            device_id: "ffffffffforger".into(),
            person_id: pid.clone(),
            epoch: 1,
            schema: store::schema_version(),
            wall_ms: unix_ms(),
            ..Frame::of_type(FRAME_HELLO)
        },
    )
    .await;
    let fkey = key();
    let now = Timestamp((unix_ms() as u64) << 16);
    let bv = a.vector().get(&b.id()).copied().unwrap_or(0);
    let ev = |d: &str, seq: i64, h: u64, actor: &str, kind: &str, p: &[u8]| Event {
        device_id: d.into(),
        seq,
        hlc: Timestamp(now.0 + h),
        actor: actor.into(),
        kind: kind.into(),
        payload: p.to_vec(),
        sig: Vec::new(),
    };
    let mut forged = vec![
        // its own row: no cert, held and never stored
        signed(
            &fkey,
            ev("ffffffffforger", 1, 0, "user", "message.sent", b"{}"),
        ),
        // b's next row with the wrong key: bad signature
        signed(&fkey, ev(&b.id(), bv + 1, 1, "user", "message.sent", b"{}")),
        // a local kind: refused before any signature check
        signed(
            &fkey,
            ev(
                "eeeeeeeeother",
                1,
                2,
                "user",
                store::kinds::POLICY_SET,
                b"{}",
            ),
        ),
    ];
    // A cert for itself, signed by a person key it made up.
    let pkey = key();
    let cert = Cert {
        person_id: pid.clone(),
        device_id: "ffffffffforger".into(),
        device_pub: hex::encode(fkey.verifying_key().to_bytes()),
        role: "desktop".into(),
        ..Cert::default()
    };
    let msg = canonical(&cert).unwrap();
    let add = DeviceAdded {
        cert,
        person_pub: hex::encode(pkey.verifying_key().to_bytes()),
        person_sig: hex::encode(pkey.sign(&msg).to_bytes()),
        person_epoch: 0,
    };
    let p = gocompat::json::marshal(&add).unwrap();
    forged.push(signed(
        &fkey,
        ev(
            "ffffffffforger",
            2,
            3,
            "irori",
            irori_identity::kinds::PERSON_DEVICE_ADDED,
            &p,
        ),
    ));
    let rows: Vec<Row> = forged.iter().map(Row::of).collect();
    send_frame(
        &c,
        &Frame {
            events: rows.clone(),
            ..Frame::of_type(FRAME_ROWS)
        },
    )
    .await;
    send_frame(
        &c,
        &Frame {
            events: rows,
            loose: true,
            ..Frame::of_type(FRAME_ROWS)
        },
    )
    .await;
    recv_frame(&c, FRAME_VECTOR, Duration::from_secs(5))
        .await
        .unwrap();
    // The bad signature and the local kind.
    eventually("the refusals are counted", || {
        refused_of(&a, "ffffffffforger") >= 2
    })
    .await;
    never("a forged row is stored", || {
        let v = a.vector();
        v.contains_key("ffffffffforger")
            || v.contains_key("eeeeeeeeother")
            || v.get(&b.id()).copied().unwrap_or(0) > bv
    })
    .await;
    assert!(
        a.chain.cert("ffffffffforger").is_none(),
        "the forger got certified"
    );
    // The person secret never goes to it, even with its epoch behind.
    assert!(
        recv_frame(&c, FRAME_PERSON_SECRET, Duration::from_millis(200))
            .await
            .is_err(),
        "the forger was sent the person secret"
    );
    room.close();
}

/// The forger is no certified device, so Status lists no row for it; its
/// refusals are read from the Node's own count.
fn refused_of(a: &Dev, dev: &str) -> i64 {
    a.node().refused_count(dev)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forged_rows_are_refused_with_the_cert_check_on() {
    forged_rows_refused(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forged_rows_are_refused_even_with_the_cert_check_off() {
    forged_rows_refused(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn catch_up_through_the_node_with_the_mac_gone() {
    let net = FakeNet::new();
    let (mut pc, mut mac, nd) = trio(&net).await;
    pc.stop().await; // the PC is off
    let rows: Vec<Event> = (0..20)
        .map(|i| {
            mac.append(
                store::kinds::AGENT_SEEN,
                json!({"key": format!("{}/s", mac.id()), "i": i}),
            )
        })
        .collect();
    eventually("the node holds the Mac's rows", || {
        nd.has(rows.last().unwrap())
    })
    .await;
    mac.stop().await; // the Mac sleeps
    pc.start();
    eventually("the PC catches up from the node", || {
        pc.has(rows.last().unwrap())
    })
    .await;
    assert!(
        !net.connected(&pc.id(), &mac.id()),
        "the PC reached the Mac, which was asleep"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn backfill_in_batches() {
    let net = FakeNet::new();
    let (mut a, mut b) = (Dev::new(&net, "pc"), Dev::new(&net, "mac"));
    let big = "x".repeat(4000);
    for i in 0..1300 {
        a.append(store::kinds::CAPTURE_TURN, json!({"i": i, "text": big}));
    }
    b.join(&a).await;
    a.start();
    b.start();
    eventually("the backfill lands", || converged(&[&a, &b])).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reconnect_after_a_drop() {
    let net = FakeNet::new();
    let (mut a, mut b) = (Dev::new(&net, "pc"), Dev::new(&net, "mac"));
    b.join(&a).await;
    a.start();
    b.start();
    eventually("converge", || converged(&[&a, &b])).await;
    net.partition(&a.id(), &b.id());
    let eb = b.append(store::kinds::AGENT_SEEN, seen(&b, "late"));
    never("a row crosses a partition", || a.has(&eb)).await;
    net.heal(&a.id(), &b.id());
    eventually("they reconnect and sync", || a.has(&eb)).await;
    net.drop_link(&a.id(), &b.id());
    let ea = a.append(store::kinds::AGENT_SEEN, seen(&a, "late"));
    eventually("they reconnect after a drop", || b.has(&ea)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_converges_and_rotates_the_key() {
    let net = FakeNet::new();
    let (pc, mac, nd) = trio(&net).await;
    // The Mac writes rows the node hears and the PC doesn't.
    net.isolate(&pc.id());
    let cut_seq = pc.vector().get(&mac.id()).copied().unwrap_or(0);
    let extra: Vec<Event> = (0..3)
        .map(|i| {
            mac.append(
                store::kinds::AGENT_SEEN,
                json!({"key": format!("{}/x", mac.id()), "i": i}),
            )
        })
        .collect();
    eventually("the node hears the Mac's extra rows", || nd.has(&extra[2])).await;
    net.partition(&nd.id(), &mac.id()); // and the Mac goes away
    let seed1 = pc.seed(1);
    pc.node().revoke(&mac.id(), "lost").await.unwrap();
    assert_eq!(
        pc.ident.current().epoch,
        2,
        "the PC's person key did not rotate"
    );
    net.heal(&pc.id(), &nd.id());
    eventually("the node takes the revoke and the new key", || {
        nd.chain.cut(&mac.id()).is_some()
            && nd.ident.current().epoch == 2
            && nd.ident.current().pub_hex == pc.ident.current().pub_hex
    })
    .await;
    eventually(
        "the node's extra rows move out and vectors converge",
        || nd.vector().get(&mac.id()).copied().unwrap_or(0) == cut_seq && converged(&[&pc, &nd]),
    )
    .await;
    let moved = nd
        .st
        .refused(100)
        .unwrap()
        .iter()
        .filter(|ev| ev.device_id == mac.id())
        .count();
    assert_eq!(
        moved,
        extra.len(),
        "node refused_events holds the Mac's rows"
    );
    // Both are on the new topic only: no device is behind.
    let cur = mesh_secret(&pc.seed(2), &pc.ident.current().id, 2).topic;
    eventually("the old topic closes", || {
        let tp = net.topics(&pc.id());
        tp.len() == 1 && tp[0] == cur && net.topics(&nd.id()).len() == 1
    })
    .await;
    // The Mac can't come back: not on the old topic, not on a reconnect.
    net.heal_all();
    never("the revoked Mac connects", || {
        net.connected(&pc.id(), &mac.id()) || net.connected(&nd.id(), &mac.id())
    })
    .await;
    let old = mesh_secret(&seed1, &pc.ident.current().id, 1);
    let (r, conns) = raw_peer(&net, old, &format!("{}x", mac.id())).await;
    never("anyone answers on the old topic", || !conns.is_empty()).await;
    r.close();
}

fn mac_row(pc: &Dev, mac: &str) -> DeviceRow {
    pc.node()
        .devices(false)
        .into_iter()
        .find(|r| r.device_id == mac)
        .unwrap_or_default()
}

/// The node is off when the PC revokes the Mac. The PC keeps the epoch-1
/// topic open until the node has the new key, which it gets there.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_offline_device_gets_the_new_key_on_the_previous_topic() {
    let net = FakeNet::new();
    let (pc, mut mac, mut nd) = trio(&net).await;
    nd.stop().await;
    mac.stop().await;
    pc.node().revoke(&mac.id(), "lost").await.unwrap();
    eventually("the PC keeps both topics open", || {
        net.topics(&pc.id()).len() == 2
    })
    .await;
    let w = mac_row(&pc, &mac.id()).warning;
    assert_eq!(w, "Removal pending: seen by 1 of 2 devices.");
    nd.start();
    eventually("the removal is seen by both", || {
        mac_row(&pc, &mac.id()).warning == "Removed."
    })
    .await;
    eventually("the node adopts epoch 2", || nd.ident.current().epoch == 2).await;
    eventually("both close the old topic", || {
        net.topics(&pc.id()).len() == 1 && net.topics(&nd.id()).len() == 1
    })
    .await;
    let e = pc.append(store::kinds::AGENT_SEEN, seen(&pc, "after"));
    eventually("sync runs on the new topic", || nd.has(&e)).await;
    // The node keeps the new key across a restart.
    nd.reopen().await;
    assert_eq!(
        nd.ident.current().epoch,
        2,
        "the adopted key did not survive a restart"
    );
}

/// The PC and the node can't reach each other, and the person removes the
/// lost Mac from both. Each rotates to epoch 2 with its own key. When they
/// meet again the rotation row of the lower device id wins on both.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_devices_that_revoke_apart_settle_on_one_key() {
    let net = FakeNet::new();
    let (pc, mut mac, nd) = trio(&net).await;
    mac.stop().await;
    net.partition(&pc.id(), &nd.id());
    pc.node().revoke(&mac.id(), "lost").await.unwrap();
    nd.node().revoke(&mac.id(), "lost").await.unwrap();
    assert_ne!(
        pc.ident.current().pub_hex,
        nd.ident.current().pub_hex,
        "the two rotations made one key"
    );
    let winner = if nd.id() < pc.id() { &nd } else { &pc };
    let want = winner.ident.current().pub_hex;
    net.heal(&pc.id(), &nd.id());
    eventually("both hold the winning key", || {
        pc.ident.current().pub_hex == want
            && nd.ident.current().pub_hex == want
            && pc.ident.current().epoch == 2
            && nd.ident.current().epoch == 2
    })
    .await;
    let topic = mesh_secret(&winner.seed(2), &pc.ident.current().id, 2).topic;
    eventually("both sit on the one epoch-2 topic only", || {
        let (a, b) = (net.topics(&pc.id()), net.topics(&nd.id()));
        a.len() == 1 && b.len() == 1 && a[0] == topic && b[0] == topic
    })
    .await;
    let e = pc.append(store::kinds::AGENT_SEEN, seen(&pc, "after"));
    eventually("sync runs on it", || nd.has(&e)).await;
}

/// The node is off through two revokes: it is still on epoch 1, and the PC
/// keeps every older key, so the node jumps straight to epoch 3.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_offline_through_two_revokes_catches_up() {
    let net = FakeNet::new();
    let (pc, mut mac, mut nd) = trio(&net).await;
    let mut x = Dev::new(&net, "x");
    x.join(&pc).await;
    x.start();
    eventually("x syncs", || converged(&[&pc, &x])).await;
    eventually("the node hears x's cert", || {
        nd.chain.cert(&x.id()).is_some()
    })
    .await;
    nd.stop().await;
    mac.stop().await;
    pc.node().revoke(&x.id(), "lost").await.unwrap();
    pc.node().revoke(&mac.id(), "lost").await.unwrap();
    assert_eq!(pc.ident.current().epoch, 3);
    assert_eq!(
        nd.ident.current().epoch,
        1,
        "the node moved while it was off"
    );
    nd.start();
    eventually("the node adopts epoch 3", || {
        nd.ident.current().epoch == 3 && nd.ident.current().pub_hex == pc.ident.current().pub_hex
    })
    .await;
    eventually("both close the older topics and forget their keys", || {
        net.topics(&pc.id()).len() == 1
            && net.topics(&nd.id()).len() == 1
            && pc.ident.older_epochs().is_empty()
    })
    .await;
    let e = pc.append(store::kinds::AGENT_SEEN, seen(&pc, "after"));
    eventually("sync runs on epoch 3", || nd.has(&e)).await;
}

/// After a restart the PC still knows the node has the new key, so it
/// doesn't reopen the old topic while the node is away.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn known_epochs_survive_a_restart() {
    let net = FakeNet::new();
    let (mut pc, mut mac, mut nd) = trio(&net).await;
    mac.stop().await;
    pc.node().revoke(&mac.id(), "lost").await.unwrap();
    eventually("the node adopts epoch 2 and the old topic closes", || {
        nd.ident.current().epoch == 2 && net.topics(&pc.id()).len() == 1
    })
    .await;
    nd.stop().await;
    pc.stop().await;
    pc.start();
    eventually("the PC is back on its topic", || {
        net.topics(&pc.id()).len() == 1
    })
    .await;
    never("the PC reopens the epoch-1 topic", || {
        net.topics(&pc.id()).len() > 1
    })
    .await;
}

async fn restored_pair() -> (FakeNet, Dev, Dev, tempfile::TempDir) {
    let net = FakeNet::new();
    let (mut a, mut b) = (Dev::new(&net, "pc"), Dev::new(&net, "mac"));
    b.join(&a).await;
    a.start();
    b.start();
    eventually("converge", || converged(&[&a, &b])).await;
    b.close().await;
    let backup = tempfile::tempdir().unwrap();
    Dev::copy_dir(&b.dir, backup.path());
    b.open();
    b.start();
    (net, a, b, backup)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restored_device_gets_its_own_rows_back() {
    let (_net, a, mut b, backup) = restored_pair().await;
    let later: Vec<Event> = (0..5)
        .map(|i| {
            b.append(
                store::kinds::AGENT_SEEN,
                json!({"key": format!("{}/later", b.id()), "i": i}),
            )
        })
        .collect();
    eventually("a has b's later rows", || a.has(&later[4])).await;
    // b's data dir is restored from the backup.
    b.close().await;
    Dev::copy_dir(backup.path(), &b.dir);
    b.open();
    assert!(
        b.vector().get(&b.id()).copied().unwrap_or(0) < later[4].seq,
        "the backup already held the later rows"
    );
    b.start();
    eventually("b gets its own rows back", || b.has(&later[4])).await;
    let mut ev = None;
    eventually("b can append again", || {
        ev =
            b.st.append("irori", store::kinds::AGENT_SEEN, &seen(&b, "new"))
                .ok();
        ev.is_some()
    })
    .await;
    let ev = ev.unwrap();
    assert_eq!(ev.seq, later[4].seq + 1, "b's next row");
    eventually("a takes it", || a.has(&ev)).await;
    let p = a.peer(&b.id());
    assert!(p.stopped.is_empty(), "a flagged b: {p:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restored_device_that_wrote_over_its_rows_is_a_fork() {
    let (_net, a, mut b, backup) = restored_pair().await;
    let later = b.append(store::kinds::AGENT_SEEN, seen(&b, "later"));
    eventually("a has it", || a.has(&later)).await;
    b.close().await;
    Dev::copy_dir(backup.path(), &b.dir);
    b.open();
    let other = b.append(store::kinds::AGENT_SEEN, seen(&b, "other")); // same seq, before syncing
    assert_eq!(other.seq, later.seq);
    b.start();
    eventually("a flags b as signing two histories", || {
        !a.peer(&b.id()).stopped.is_empty()
    })
    .await;
    assert!(!a.st.forks().unwrap().is_empty(), "no fork recorded on a");
}

/// The restored device wrote fewer rows than it had sent before: the
/// check on its last row finds the fork, on both sides, and nothing is
/// spliced.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restored_device_that_wrote_fewer_over_its_rows_is_a_fork() {
    let (_net, a, mut b, backup) = restored_pair().await;
    let later: Vec<Event> = (0..3)
        .map(|i| {
            b.append(
                store::kinds::AGENT_SEEN,
                json!({"key": format!("{}/later", b.id()), "i": i}),
            )
        })
        .collect();
    eventually("a has them", || a.has(&later[2])).await;
    b.close().await;
    Dev::copy_dir(backup.path(), &b.dir);
    b.open();
    let other = b.append(store::kinds::AGENT_SEEN, seen(&b, "other"));
    assert_eq!(other.seq, later[0].seq);
    b.start();
    eventually("b finds it signed two histories", || {
        b.node().stopped_reason(&b.id()).is_some()
    })
    .await;
    eventually("a flags b", || !a.peer(&b.id()).stopped.is_empty()).await;
    assert!(
        !b.has(&later[1]) && !b.has(&later[2]),
        "b's old rows were stacked on its new ones"
    );
    assert!(!b.st.forks().unwrap().is_empty(), "no fork recorded on b");
    // b isn't left restoring for good: it writes.
    b.st.append("irori", store::kinds::AGENT_SEEN, &seen(&b, "after"))
        .expect("append after the fork");
}

/// A hello that claims more of this device's rows than it holds, with no
/// row of its own key to prove it, doesn't hold its appends.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_forged_vector_does_not_hold_appends() {
    let net = FakeNet::new();
    let (mut a, mut b) = (Dev::new(&net, "pc"), Dev::new(&net, "mac"));
    b.join(&a).await;
    a.trust = true;
    a.start();
    b.start();
    eventually("converge", || converged(&[&a, &b])).await;
    let pid = a.ident.current().id;
    let (room, mut conns) = raw_peer(&net, mesh_secret(&a.seed(1), &pid, 1), "ffffffffthief").await;
    room.connect(&a.id());
    let c = tokio::time::timeout(Duration::from_secs(5), conns.recv())
        .await
        .expect("never connected")
        .unwrap();
    // A row "of a's" signed by another key, at the claimed seq.
    let fake = signed(
        &key(),
        Event {
            device_id: a.id(),
            seq: 1 << 40,
            hlc: Timestamp((unix_ms() as u64) << 16),
            actor: "irori".into(),
            kind: store::kinds::AGENT_SEEN.into(),
            payload: b"{}".to_vec(),
            sig: Vec::new(),
        },
    );
    send_frame(
        &c,
        &Frame {
            v: PROTOCOL,
            device_id: "ffffffffthief".into(),
            person_id: pid,
            epoch: 1,
            schema: store::schema_version(),
            wall_ms: unix_ms(),
            vector: [(a.id(), 1i64 << 40)].into_iter().collect(),
            top: store::row_hash(&fake),
            top_row: Some(Box::new(Row::of(&fake))),
            ..Frame::of_type(FRAME_HELLO)
        },
    )
    .await;
    recv_frame(&c, FRAME_VECTOR, Duration::from_secs(5))
        .await
        .unwrap();
    a.st.append("irori", store::kinds::AGENT_SEEN, &seen(&a, "x"))
        .expect("append after a forged claim");
    let st = a.node().status();
    assert!(st.restoring.is_empty(), "a says it is restoring: {st:?}");
    // And a real hold never blocks a revoke.
    a.st.hold_appends(1 << 40);
    a.node()
        .revoke(&b.id(), "lost")
        .await
        .expect("revoke while appends are held");
    room.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rows_from_a_clock_ten_minutes_ahead_wait_then_land() {
    let net = FakeNet::new();
    let (mut a, mut b) = (Dev::new(&net, "pc"), Dev::new(&net, "mac"));
    b.join(&a).await;
    a.start();
    b.start();
    eventually("converge", || converged(&[&a, &b])).await;
    b.clk.set(Duration::from_secs(600));
    let ahead = b.append(store::kinds::AGENT_SEEN, seen(&b, "ahead"));
    never("a stores a row 10 minutes ahead", || a.has(&ahead)).await;
    eventually("a shows b's rows waiting", || a.peer(&b.id()).ahead).await;
    a.clk.set(Duration::from_secs(600)); // a's wall time catches up
    eventually("the row lands", || a.has(&ahead)).await;
}

#[test]
fn batches_keep_the_limits() {
    let ev = |seq: i64, p: Vec<u8>| Event {
        device_id: "d".into(),
        seq,
        payload: p,
        ..Event::default()
    };
    let evs: Vec<Event> = (1..=1200).map(|i| ev(i, br#"{"a":1}"#.to_vec())).collect();
    let fr = mesh::batches(&evs, false);
    assert!(
        fr.len() == 3 && fr[0].events.len() == 500 && fr[2].events.len() == 200,
        "row cap: {} frames",
        fr.len()
    );
    let big: Vec<Event> = (1..=10).map(|i| ev(i, vec![0; 300 << 10])).collect();
    for f in mesh::batches(&big, false) {
        let b = f.encode();
        assert!(
            f.events.len() <= 1 || b.len() <= (1 << 20) + (64 << 10),
            "frame of {} rows is {} bytes",
            f.events.len(),
            b.len()
        );
    }
    // The payload travels as the stored bytes.
    let b = Frame {
        events: vec![Row::of(&Event {
            device_id: "d".into(),
            seq: 1,
            payload: br#"{"b": 2 }"#.to_vec(),
            sig: vec![1, 2],
            ..Event::default()
        })],
        ..Frame::of_type(FRAME_ROWS)
    }
    .encode();
    let f = decode_frame(&b).unwrap();
    assert_eq!(f.events[0].event().payload, br#"{"b": 2 }"#);
    assert_eq!(f.events[0].event().sig[1], 2);
}

#[test]
fn mesh_secret_moves_with_the_epoch() {
    let seed = [0u8; 32];
    let (a, b) = (mesh_secret(&seed, "p", 1), mesh_secret(&seed, "p", 2));
    assert!(a.topic != b.topic && a.key != b.key && a.topic.starts_with("irori/mesh/1/"));
    assert_eq!(mesh_secret(&seed, "p", 1), a);
    assert_ne!(mesh_secret(&seed, "q", 1).topic, a.topic);
}

#[test]
fn person_secret_sealing() {
    let mut old = [0u8; 32];
    old[0] = 7;
    let nw = b"0123456789abcdef0123456789abcdef";
    let sealed = mesh::seal_secret(&old, nw, "p", 2, "dev-b").unwrap();
    assert_eq!(
        mesh::open_secret(&old, &sealed, "p", 2, "dev-b").unwrap(),
        nw
    );
    assert!(
        mesh::open_secret(&old, &sealed, "p", 2, "dev-c").is_err(),
        "another device opened it"
    );
    assert!(
        mesh::open_secret(&old, &sealed, "p", 3, "dev-b").is_err(),
        "it opened for another epoch"
    );
}

#[test]
fn relay_setting() {
    let dir = tempfile::tempdir().unwrap();
    let r = mesh::load_relay(dir.path()).unwrap();
    assert!(r.direct(), "default relay {r:?}");
    assert!(
        mesh::save_relay(
            dir.path(),
            &mesh::Relay {
                credential_url: "http://example.com/cred".into(),
                ..Default::default()
            }
        )
        .is_err(),
        "saved a plain-http credential URL"
    );
    let want = mesh::Relay {
        credential_url: "https://relay.example/cred".into(),
        turn_urls: vec!["turns:relay.example:5349".into()],
    };
    mesh::save_relay(dir.path(), &want).unwrap();
    assert_eq!(mesh::load_relay(dir.path()).unwrap(), want);
    mesh::save_relay(dir.path(), &mesh::Relay::default()).unwrap();
    assert!(
        !dir.path().join(mesh::relay::RELAY_FILE).exists(),
        "Direct only left relay.json behind"
    );
}

#[test]
fn scrub_urls_cuts_queries() {
    assert_eq!(
        mesh::scrub_urls("fetch https://relay.example/cred?key=abc failed"),
        "fetch https://relay.example/cred?… failed"
    );
}
