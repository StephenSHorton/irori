//! Person-to-person DMs on the mesh (card #538, docs/design/talk-dms.md §2):
//! the `dm` frame, its hooks and `send_to` on board connections, and
//! `dm.secret` on the private mesh. Call ringing rides the same connections
//! and hooks the same way (`call` frames, `send_call`, card #654). Nothing
//! here opens a socket.

use crate::common;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{Dev, eventually, never};
use ed25519_dalek::SigningKey;
use irori_identity::*;
use mesh::boardmesh::{
    BoardMesh, BoardMeshOpts, CALLS_PROTOCOL, CallFrame, CallHooks, DMS_PROTOCOL, DmFrame, DmHooks,
    LivePeer, MemberDevice, OwnDevicesFn, open_board_secret,
};
use mesh::frames::{FRAME_BOARD_SECRET, FRAME_DM_SECRET, FRAME_HELLO, PROTOCOL};
use mesh::ownboards::OwnBoards;
use mesh::{Conn, FakeNet, Frame, Intervals, MSG_PRESENCE, Presence, Room, RoomConfig, Secret};
use mesh::{Transport, decode_frame, mesh_secret};
use serde::Serialize;
use serde_json::json;
use store::Store;
use tokio::sync::mpsc;

/// The space both people share.
const S: &str = "00112233445566778899aabbccddeeff";
/// The hidden space a hand-off names.
const DM: &str = "ffeeddccbbaa99887766554433221100";
/// S's secret, which every device of the test holds.
const SECRET: [u8; 32] = [5; 32];

fn rand_key() -> SigningKey {
    let mut s = [0u8; 32];
    getrandom::fill(&mut s).unwrap();
    SigningKey::from_bytes(&s)
}

fn unix(secs: i64) -> gocompat::time::GoTime {
    gocompat::time::GoTime::unix(secs, 0)
}

struct P {
    pid: String,
    root: String,
    key: SigningKey,
}

impl P {
    fn new(root: &str) -> P {
        let key = rand_key();
        P {
            pid: person_id_of(&key.verifying_key().to_bytes()),
            root: root.into(),
            key,
        }
    }

    fn pin(&self) -> MemberPin {
        MemberPin {
            person_id: self.pid.clone(),
            person_pub: pub_hex(&self.key),
            root_device_id: self.root.clone(),
            person_epoch: 1,
        }
    }
}

/// S's identity rows: `alice` made it, and every other person is a member
/// with one device, named as its root.
struct Space {
    alice: P,
    people: Vec<P>,
    devs: BTreeMap<String, SigningKey>,
    rows: Vec<Row>,
    seq: BTreeMap<String, i64>,
    hlc: u64,
}

impl Space {
    fn new(others: &[&str]) -> Space {
        let mut s = Space {
            alice: P::new("a1"),
            people: others.iter().map(|d| P::new(d)).collect(),
            devs: BTreeMap::new(),
            rows: Vec::new(),
            seq: BTreeMap::new(),
            hlc: 0,
        };
        s.cert(None, "a1");
        let mut created = BoardCreated {
            name: "mlg".into(),
            tag: "mlg".into(),
            creator: s.alice.pin(),
            at: unix(1),
            ..Default::default()
        };
        created.person_sig = sign_person(&s.alice.key, &created).unwrap();
        let actor = format!("person:{}", s.alice.pid);
        s.row("a1", &actor, kinds::BOARD_CREATED, &created);
        for i in 0..s.people.len() {
            let pin = s.people[i].pin();
            let mut add = BoardMemberAdded {
                person_id: pin.person_id,
                person_pub: pin.person_pub,
                root_device_id: pin.root_device_id.clone(),
                person_epoch: 1,
                name: "x".into(),
                role: "member".into(),
                by: s.alice.pid.clone(),
                by_epoch: 1,
                at: unix(2),
                person_sig: String::new(),
            };
            add.person_sig = sign_person(&s.alice.key, &add).unwrap();
            s.row("a1", &actor, kinds::BOARD_MEMBER_ADDED, &add);
            s.cert(Some(i), &pin.root_device_id);
        }
        s
    }

    fn row<T: Serialize>(&mut self, writer: &str, actor: &str, kind: &str, payload: &T) {
        *self.seq.entry(writer.into()).or_default() += 1;
        self.hlc += 1;
        let mut r = Row {
            device_id: writer.into(),
            seq: self.seq[writer],
            hlc: self.hlc,
            actor: actor.into(),
            kind: kind.into(),
            payload: gocompat::json::marshal(payload).unwrap(),
            sig: Vec::new(),
        };
        r.sign(&self.devs[writer]);
        self.rows.push(r);
    }

    /// The cert of `dev`, of person `who` (`None`: alice).
    fn cert(&mut self, who: Option<usize>, dev: &str) {
        let k = rand_key();
        self.devs.insert(dev.into(), k.clone());
        let p = who.map_or(&self.alice, |i| &self.people[i]);
        let c = MemberCert {
            person_id: p.pid.clone(),
            device_id: dev.into(),
            device_pub: pub_hex(&k),
            role: ROLE_DESKTOP.into(),
            person_epoch: 1,
            at: unix(1),
        };
        let a = MemberDeviceAdded {
            person_sig: sign_person(&p.key, &c).unwrap(),
            cert: c,
        };
        self.row(dev, "irori", kinds::MEMBER_DEVICE_ADDED, &a);
    }

    fn verifier(&self) -> Arc<BoardVerifier> {
        let v = BoardVerifier::new(&[self.alice.pin()]);
        for r in &self.rows {
            v.observe(r.as_ref());
        }
        Arc::new(v)
    }

    fn device_pub(&self, dev: &str) -> [u8; 32] {
        self.devs[dev].verifying_key().to_bytes()
    }

    fn person_of(&self, dev: &str) -> String {
        if dev == "a1" {
            return self.alice.pid.clone();
        }
        let p = self.people.iter().find(|p| p.root == dev).unwrap();
        p.pid.clone()
    }
}

/// What a device's hooks heard.
#[derive(Default)]
struct Heard {
    hellos: Mutex<Vec<(String, LivePeer)>>,
    ticks: AtomicUsize,
    frames: Mutex<Vec<(String, LivePeer, DmFrame)>>,
    call_hellos: Mutex<Vec<(String, LivePeer)>>,
    calls: Mutex<Vec<(String, LivePeer, CallFrame)>>,
}

impl DmHooks for Heard {
    fn hello(&self, via: &str, peer: &LivePeer) {
        let mut g = self.hellos.lock().unwrap();
        g.push((via.to_string(), peer.clone()));
    }

    fn tick(&self, _via: &str, _peer: &LivePeer) {
        self.ticks.fetch_add(1, Ordering::SeqCst);
    }

    fn frame(&self, via: &str, peer: &LivePeer, frame: DmFrame) {
        let mut g = self.frames.lock().unwrap();
        g.push((via.to_string(), peer.clone(), frame));
    }
}

impl CallHooks for Heard {
    fn hello(&self, via: &str, peer: &LivePeer) {
        let mut g = self.call_hellos.lock().unwrap();
        g.push((via.to_string(), peer.clone()));
    }

    fn frame(&self, via: &str, peer: &LivePeer, frame: CallFrame) {
        let mut g = self.calls.lock().unwrap();
        g.push((via.to_string(), peer.clone(), frame));
    }
}

impl Heard {
    fn frame_count(&self) -> usize {
        self.frames.lock().unwrap().len()
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

/// A device of S with its mesh running.
struct Member {
    mesh: BoardMesh,
    heard: Arc<Heard>,
    secrets: Arc<BoardSecrets>,
    log: Arc<Store>,
    // Last: fields drop in order, and Windows can't delete an open file.
    _tmp: tempfile::TempDir,
}

impl Drop for Member {
    fn drop(&mut self) {
        self.mesh.close();
        // The mesh's tasks may still hold the log: close it so its files can go.
        let _ = self.log.close();
    }
}

fn secrets_at(dir: &std::path::Path, board: &str) -> Arc<BoardSecrets> {
    let path = dir.join(format!("{board}.secrets"));
    Arc::new(BoardSecrets::open(&path, board, Box::new(FileKeystore)).unwrap())
}

async fn member(net: &FakeNet, s: &Space, dev: &str, own: Option<OwnDevicesFn>) -> Member {
    let tmp = tempfile::tempdir().unwrap();
    let log = Arc::new(
        Store::open_with(
            tmp.path().join("board.db"),
            dev,
            Arc::new(store::Clock::new()),
            store::OpenOptions::default(),
        )
        .unwrap(),
    );
    let secrets = secrets_at(tmp.path(), S);
    secrets.add(1, "a1", &SECRET).unwrap();
    let heard = Arc::new(Heard::default());
    let m = BoardMesh::new(BoardMeshOpts {
        board_id: S.into(),
        self_id: dev.into(),
        person_id: s.person_of(dev),
        core_version: "0.12.0-test".into(),
        log: log.clone(),
        verifier: s.verifier(),
        secrets: secrets.clone(),
        transport: Arc::new(net.keyed(s.device_pub(dev))),
        own_devices: own,
        files: None,
        dms: Some(heard.clone()),
        calls: Some(heard.clone()),
    });
    m.set_intervals(Intervals {
        presence: Duration::from_millis(40),
        ping: Duration::from_millis(60),
        vector: Duration::from_millis(50),
        ..Default::default()
    });
    m.start().await.unwrap();
    Member {
        mesh: m,
        heard,
        secrets,
        log,
        _tmp: tmp,
    }
}

/// A device on S's room that speaks frames by hand: an older build, or one
/// saying things a real one wouldn't.
struct Raw {
    _room: Arc<dyn Room>,
    conn: Arc<dyn Conn>,
}

async fn raw(net: &FakeNet, m: &Member, key: [u8; 32], id: &str, to: &str) -> Raw {
    let t = m.secrets.topics().unwrap().remove(0);
    let secret = Secret {
        key: t.key,
        topic: t.topic.clone(),
        epoch: t.epoch,
    };
    let (room, mut conns) = raw_join(net, secret, key, id).await;
    room.connect(to);
    let conn = tokio::time::timeout(Duration::from_secs(5), conns.recv())
        .await
        .expect("the raw peer never connected")
        .unwrap();
    Raw { _room: room, conn }
}

async fn raw_join(
    net: &FakeNet,
    s: Secret,
    key: [u8; 32],
    id: &str,
) -> (Arc<dyn Room>, mpsc::Receiver<Arc<dyn Conn>>) {
    let (tx, rx) = mpsc::channel(4);
    let mut cfg = RoomConfig::new(id);
    cfg.on_peer = Arc::new(move |c: Arc<dyn Conn>| {
        let _ = tx.try_send(c);
    });
    (net.keyed(key).join(s, cfg).await.unwrap(), rx)
}

/// A board hello as `dev` writes it; `extra` adds what a newer build says.
fn hello(dev: &str, person: &str, extra: serde_json::Value) -> Vec<u8> {
    let mut v = json!({
        "type": "hello", "v": 1, "deviceId": dev, "personId": person,
        "boardId": S, "boardEpoch": 1, "coreVersion": "0.10.2", "schema": 1,
        "vector": {}, "wallMs": 0,
    });
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    serde_json::to_vec(&v).unwrap()
}

fn ask() -> DmFrame {
    DmFrame {
        board_id: DM.into(),
        step: "ask".into(),
        body: json!({}),
    }
}

fn live_ids(m: &Member) -> Vec<String> {
    let peers = m.mesh.live_peers();
    peers.into_iter().map(|p| p.device_id).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn send_to_reaches_only_that_peer() {
    let net = FakeNet::new();
    let s = Space::new(&["b1", "c1"]);
    let a = member(&net, &s, "a1", None).await;
    let b = member(&net, &s, "b1", None).await;
    let c = member(&net, &s, "c1", None).await;
    eventually("a hears b and c", || live_ids(&a) == ["b1", "c1"]).await;
    eventually("b and c hear a", || {
        live_ids(&b).contains(&"a1".to_string()) && live_ids(&c).contains(&"a1".to_string())
    })
    .await;
    let peers = a.mesh.live_peers();
    assert_eq!(peers[0].dms, DMS_PROTOCOL);
    assert_eq!(peers[0].person_id, s.person_of("b1"));
    assert_eq!(peers[0].device_pub, hex::encode(s.device_pub("b1")));

    a.mesh.send_to("b1", ask()).await.unwrap();
    eventually("b gets the frame", || b.heard.frame_count() == 1).await;
    {
        let got = b.heard.frames.lock().unwrap();
        let (via, peer, f) = &got[0];
        assert_eq!(via, S, "the hook names the connection's board");
        assert_eq!(f, &ask(), "the frame keeps naming the hidden space");
        assert_eq!(peer.device_id, "a1");
        assert_eq!(peer.device_pub, hex::encode(s.device_pub("a1")));
        assert_eq!(peer.person_id, s.person_of("a1"));
    }
    never("c gets b's frame", || c.heard.frame_count() > 0).await;
    assert_eq!(a.heard.frame_count(), 0);
    assert!(a.mesh.send_to("nobody", ask()).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hellos_and_vector_ticks_reach_the_hooks() {
    let net = FakeNet::new();
    let s = Space::new(&["b1"]);
    let a = member(&net, &s, "a1", None).await;
    let b = member(&net, &s, "b1", None).await;
    eventually("a hears b's hello", || {
        a.heard.hellos.lock().unwrap().len() == 1
    })
    .await;
    {
        let got = a.heard.hellos.lock().unwrap();
        assert_eq!(got[0].0, S);
        assert_eq!(got[0].1.device_id, "b1");
        assert_eq!(got[0].1.dms, DMS_PROTOCOL);
    }
    eventually("ticks go on after the one hello", || {
        a.heard.ticks.load(Ordering::SeqCst) >= 3 && b.heard.ticks.load(Ordering::SeqCst) >= 3
    })
    .await;
    assert_eq!(a.heard.hellos.lock().unwrap().len(), 1);
}

/// `an_old_peer_gets_no_ask`, at the mesh: its hello still reaches the hook,
/// so the window can say it needs updating, and `send_to` refuses it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_that_doesnt_say_dms_is_never_asked() {
    let net = FakeNet::new();
    let s = Space::new(&["d1"]);
    let a = member(&net, &s, "a1", None).await;
    let d = raw(&net, &a, s.device_pub("d1"), "d1", "a1").await;
    let said = hello("d1", &s.person_of("d1"), json!({"spaces": 1, "spawns": 1}));
    d.conn.send(&said).await.unwrap();
    eventually("a hears d", || live_ids(&a) == ["d1"]).await;
    assert_eq!(a.mesh.live_peers()[0].dms, 0);
    eventually("the hook hears d", || {
        let g = a.heard.hellos.lock().unwrap();
        g.len() == 1 && g[0].1.dms == 0
    })
    .await;
    let err = a.mesh.send_to("d1", ask()).await.unwrap_err();
    assert!(err.contains("DM hand-off"), "{err}");
}

/// A frame this build can't read is skipped and the connection goes on, and
/// the hook names the certified person, not the one the hello claims.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unknown_wire_type_is_skipped() {
    let net = FakeNet::new();
    let s = Space::new(&["d1"]);
    let a = member(&net, &s, "a1", None).await;
    let d = raw(&net, &a, s.device_pub("d1"), "d1", "a1").await;
    let said = hello("d1", &s.alice.pid, json!({"spaces": 1, "dms": 1}));
    d.conn.send(&said).await.unwrap();
    for skipped in [
        json!({"type": "later.frame", "boardId": DM}),
        json!({"type": "dm", "step": "ask"}),
    ] {
        let b = serde_json::to_vec(&skipped).unwrap();
        d.conn.send(&b).await.unwrap();
    }
    let good = json!({"type": "dm", "boardId": DM, "step": "pin", "body": {"pin": 1}});
    d.conn
        .send(&serde_json::to_vec(&good).unwrap())
        .await
        .unwrap();
    eventually("the good frame lands", || a.heard.frame_count() == 1).await;
    let got = a.heard.frames.lock().unwrap();
    let (_, peer, f) = &got[0];
    assert_eq!(f.step, "pin");
    assert_eq!(f.body, json!({"pin": 1}));
    assert_eq!(peer.device_id, "d1");
    assert_eq!(peer.person_id, s.person_of("d1"), "the hello said alice");
    assert!(d.conn.err().is_none(), "the connection stayed up");
}

/// One of this person's devices is let in before S certifies it, but it has
/// no cert to stand on in a hand-off.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_s_doesnt_count_has_no_say_in_a_hand_off() {
    let net = FakeNet::new();
    let s = Space::new(&["b1"]);
    let key = rand_key().verifying_key().to_bytes();
    let own: OwnDevicesFn = {
        let person = s.alice.pid.clone();
        Arc::new(move || {
            vec![MemberDevice {
                person_id: person.clone(),
                device_id: "a2".into(),
                device_pub: hex::encode(key),
            }]
        })
    };
    let a = member(&net, &s, "a1", Some(own)).await;
    let d = raw(&net, &a, key, "a2", "a1").await;
    let said = hello("a2", &s.alice.pid, json!({"spaces": 1, "dms": 1}));
    d.conn.send(&said).await.unwrap();
    eventually("a hears its own device", || live_ids(&a) == ["a2"]).await;
    assert_eq!(a.mesh.live_peers()[0].person_id, s.alice.pid);
    let f = serde_json::to_vec(&json!({"type": "dm", "boardId": DM, "step": "ask"})).unwrap();
    d.conn.send(&f).await.unwrap();
    never("its frame is heard", || a.heard.frame_count() > 0).await;
    assert!(a.mesh.send_to("a2", ask()).await.is_err());
}

fn ring() -> CallFrame {
    CallFrame {
        space_id: S.into(),
        call_id: "call-1".into(),
        op: "ring".into(),
        conv_id: "general".into(),
        epoch: 1,
        devices: vec!["a1".into()],
        at_ms: 1,
    }
}

/// Card #654: a call frame reaches the receiver's call hooks alone, named
/// by the connection's board, and both hellos say calls.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn send_call_reaches_the_call_hooks() {
    let net = FakeNet::new();
    let s = Space::new(&["b1", "c1"]);
    let a = member(&net, &s, "a1", None).await;
    let b = member(&net, &s, "b1", None).await;
    let c = member(&net, &s, "c1", None).await;
    eventually("a hears b and c", || live_ids(&a) == ["b1", "c1"]).await;
    assert!(
        a.mesh
            .live_peers()
            .iter()
            .all(|p| p.calls == CALLS_PROTOCOL)
    );
    eventually("b's call hooks hear a's hello", || {
        let g = b.heard.call_hellos.lock().unwrap();
        g.iter()
            .any(|(via, p)| via == S && p.device_id == "a1" && p.calls == CALLS_PROTOCOL)
    })
    .await;

    a.mesh.send_call("b1", ring()).await.unwrap();
    eventually("b gets the call", || b.heard.call_count() == 1).await;
    {
        let got = b.heard.calls.lock().unwrap();
        let (via, peer, f) = &got[0];
        assert_eq!(via, S, "the hook names the connection's board");
        assert_eq!(f, &ring());
        assert_eq!(peer.device_id, "a1");
        assert_eq!(peer.person_id, s.person_of("a1"));
    }
    assert_eq!(b.heard.frame_count(), 0, "a call isn't a dm");
    never("c gets b's call", || c.heard.call_count() > 0).await;
    assert!(a.mesh.send_call("nobody", ring()).await.is_err());
}

/// A call frame about another space is dropped; one with an op this build
/// doesn't know still reaches the hooks, and the connection stays up.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_frame_for_another_space_is_dropped() {
    let net = FakeNet::new();
    let s = Space::new(&["d1"]);
    let a = member(&net, &s, "a1", None).await;
    let d = raw(&net, &a, s.device_pub("d1"), "d1", "a1").await;
    let said = hello("d1", &s.person_of("d1"), json!({"spaces": 1, "calls": 1}));
    d.conn.send(&said).await.unwrap();
    eventually("a hears d", || live_ids(&a) == ["d1"]).await;
    for f in [
        json!({"type": "call", "spaceId": DM, "callId": "x", "op": "ring"}),
        json!({"type": "call", "spaceId": S, "callId": "y", "op": "mute", "devices": ["d1"]}),
    ] {
        d.conn.send(&serde_json::to_vec(&f).unwrap()).await.unwrap();
    }
    eventually("the unknown op lands", || a.heard.call_count() == 1).await;
    let got = a.heard.calls.lock().unwrap();
    let (via, peer, f) = &got[0];
    assert_eq!(via, S);
    assert_eq!(f.call_id, "y", "the other space's frame never came");
    assert_eq!(f.op, "mute");
    assert_eq!(f.devices, ["d1"]);
    assert_eq!(peer.device_id, "d1");
    assert_eq!(peer.calls, CALLS_PROTOCOL);
    assert!(d.conn.err().is_none(), "the connection stayed up");
}

/// A device whose hello doesn't say calls is still heard, and never rung.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_that_doesnt_say_calls_is_never_rung() {
    let net = FakeNet::new();
    let s = Space::new(&["d1"]);
    let a = member(&net, &s, "a1", None).await;
    let d = raw(&net, &a, s.device_pub("d1"), "d1", "a1").await;
    let said = hello("d1", &s.person_of("d1"), json!({"spaces": 1, "dms": 1}));
    d.conn.send(&said).await.unwrap();
    eventually("a hears d", || live_ids(&a) == ["d1"]).await;
    assert_eq!(a.mesh.live_peers()[0].calls, 0);
    eventually("the call hooks hear d", || {
        let g = a.heard.call_hellos.lock().unwrap();
        g.len() == 1 && g[0].1.calls == 0
    })
    .await;
    let err = a.mesh.send_call("d1", ring()).await.unwrap_err();
    assert!(err.contains("too old for calls"), "{err}");
}

/// The keys a call connection is proved with are the joined epochs' topic
/// keys: here epoch 1's alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_topic_keys_are_the_joined_epochs() {
    let net = FakeNet::new();
    let s = Space::new(&[]);
    let a = member(&net, &s, "a1", None).await;
    let t = a.secrets.topics().unwrap().remove(0);
    assert_eq!(a.mesh.newest_key(), Some((1, t.key)));
    assert_eq!(a.mesh.topic_keys(), [(1, t.key)]);
}

/// The boards a stub Core holds open.
#[derive(Default)]
struct Own {
    open: Mutex<Vec<(String, Arc<BoardSecrets>)>>,
    dms: Mutex<BTreeSet<String>>,
    vouched: Mutex<Vec<(String, String)>>,
}

impl OwnBoards for Own {
    fn open_boards(&self) -> Vec<(String, Arc<BoardSecrets>)> {
        self.open.lock().unwrap().clone()
    }

    fn vouched(&self, board_id: &str, maker: &str) {
        let mut g = self.vouched.lock().unwrap();
        g.push((board_id.to_string(), maker.to_string()));
    }

    fn is_dm(&self, board_id: &str) -> bool {
        self.dms.lock().unwrap().contains(board_id)
    }
}

/// An older own device sees `dm.secret` as a frame it doesn't know, and no
/// `board.secret` for the hidden space, so it can't open that log.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dm_spaces_secret_never_goes_as_board_secret() {
    let net = FakeNet::new();
    let (mut pc, mut mac) = (Dev::new(&net, "pc"), Dev::new(&net, "mac"));
    mac.join(&pc).await;
    mac.learn(&pc);
    pc.start();

    let tmp = tempfile::tempdir().unwrap();
    let (plain, hidden) = (secrets_at(tmp.path(), S), secrets_at(tmp.path(), DM));
    plain.mint(1, &pc.id()).unwrap();
    hidden.mint(1, &pc.id()).unwrap();
    let own = Arc::new(Own::default());
    own.dms.lock().unwrap().insert(DM.to_string());
    *own.open.lock().unwrap() = vec![(S.into(), plain), (DM.into(), hidden.clone())];
    let node = pc.node.clone().unwrap();
    node.set_own_boards(Some(own));

    // The mac as an older build would be: it speaks hello, then only reads.
    let pid = pc.ident.current().id;
    let secret = mesh_secret(&pc.seed(1), &pid, 1);
    let key = secret.key;
    let (room, mut conns) = raw_join(&net, secret, mac.ident.device_pub(), &mac.id()).await;
    let pres = gocompat::json::marshal(&Presence {
        device_id: mac.id(),
        epoch: 1,
        core_version: String::new(),
    })
    .unwrap();
    let _ = room.send("", MSG_PRESENCE, serde_json::from_slice(&pres).unwrap());
    room.connect(&pc.id());
    let c = tokio::time::timeout(Duration::from_secs(5), conns.recv())
        .await
        .expect("the mac never connected")
        .unwrap();
    let said = Frame {
        v: PROTOCOL,
        device_id: mac.id(),
        person_id: pid.clone(),
        epoch: 1,
        schema: store::schema_version(),
        wall_ms: 1,
        ..Frame::of_type(FRAME_HELLO)
    };
    c.send(&said.encode()).await.unwrap();

    let mut got: BTreeMap<(String, String), Frame> = BTreeMap::new();
    let _ = tokio::time::timeout(Duration::from_millis(600), async {
        while let Ok(b) = c.recv().await {
            let f = decode_frame(&b).unwrap();
            if !f.board_id.is_empty() {
                got.insert((f.typ.clone(), f.board_id.clone()), f);
            }
        }
    })
    .await;
    let keys: Vec<_> = got.keys().cloned().collect();
    let want = [
        (FRAME_BOARD_SECRET.to_string(), S.to_string()),
        (FRAME_DM_SECRET.to_string(), DM.to_string()),
    ];
    assert_eq!(keys, want, "sorted: board.secret comes first");
    let f = &got[&(FRAME_DM_SECRET.to_string(), DM.to_string())];
    let opened = open_board_secret(&key, DM, f.epoch, &f.sealed, &mac.id()).unwrap();
    assert_eq!(opened, *hidden.secret(1).unwrap().0);
    room.close();
}

/// `dm.secret` can come before the marker opens the space here: it waits
/// like a `board.secret`, and the sender's word vouches for the maker.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dm_secret_waits_for_its_space() {
    let net = FakeNet::new();
    let (mut pc, mut mac) = (Dev::new(&net, "pc"), Dev::new(&net, "mac"));
    mac.join(&pc).await;
    mac.learn(&pc);

    let (pc_dir, mac_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (theirs, ours) = (
        secrets_at(pc_dir.path(), DM),
        secrets_at(mac_dir.path(), DM),
    );
    theirs.mint(1, &pc.id()).unwrap();
    let on_pc = Arc::new(Own::default());
    on_pc.dms.lock().unwrap().insert(DM.to_string());
    *on_pc.open.lock().unwrap() = vec![(DM.into(), theirs.clone())];
    let on_mac = Arc::new(Own::default());
    on_mac.dms.lock().unwrap().insert(DM.to_string());

    pc.start();
    mac.start();
    pc.node
        .as_ref()
        .unwrap()
        .set_own_boards(Some(on_pc.clone()));
    mac.node
        .as_ref()
        .unwrap()
        .set_own_boards(Some(on_mac.clone()));
    eventually("the two connect", || net.connected(&pc.id(), &mac.id())).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        on_mac.vouched.lock().unwrap().is_empty(),
        "nothing is taken before the space opens"
    );

    *on_mac.open.lock().unwrap() = vec![(DM.into(), ours.clone())];
    mac.node.as_ref().unwrap().boards_changed();
    eventually("the secret lands", || ours.epoch() == 1).await;
    assert_eq!(ours.secret(1).unwrap().0, theirs.secret(1).unwrap().0);
    assert_eq!(
        *on_mac.vouched.lock().unwrap(),
        [(DM.to_string(), pc.id())],
        "the maker was vouched for before the secret went in"
    );
}
