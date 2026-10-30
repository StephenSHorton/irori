//! The invite end to end on an in-memory connection: two made-up people,
//! each with a real identity and real board stores on disk, the inviter's
//! board created the way the Core creates one. The net layer (card #43)
//! hands both sides the same kind of `Conn`; tests/iroh.rs runs this over
//! iroh on 127.0.0.1 once it does.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use gocompat::time::GoTime;
use irori_identity::{
    BoardSecrets, BoardVerifier, DeviceJson, FileKeystore, Identity, MemberPin, ROLE_DESKTOP, kinds,
};
use irori_invite::flow::{STATE_DONE, STATE_FAILED, STATE_WORDS};
use irori_invite::{Guest, Host, InviteBoards, Timing, bundle::person_actor};
use irori_meshseam::{BoxFut, Conn, ConnError, Stream, StreamHandler};
use store::{OpenOptions, Store};
use tokio::sync::{Mutex as AsyncMutex, mpsc};

const BOARD: &str = "0123456789abcdef0123456789abcdef";

fn now() -> GoTime {
    GoTime::from_system(SystemTime::now())
}

/// One half of an in-memory connection.
struct MemConn {
    peer: String,
    tx: mpsc::UnboundedSender<Vec<u8>>,
    rx: AsyncMutex<mpsc::UnboundedReceiver<Vec<u8>>>,
    binding: String,
    closed: Mutex<Option<String>>,
}

fn mem_pair(a: &str, b: &str) -> (Arc<MemConn>, Arc<MemConn>) {
    let (ta, ra) = mpsc::unbounded_channel();
    let (tb, rb) = mpsc::unbounded_channel();
    let binding = "aa:bb:cc".to_string();
    let mk = |peer: &str, tx, rx| {
        Arc::new(MemConn {
            peer: peer.into(),
            tx,
            rx: AsyncMutex::new(rx),
            binding: binding.clone(),
            closed: Mutex::new(None),
        })
    };
    (mk(b, ta, rb), mk(a, tb, ra))
}

impl Conn for MemConn {
    fn id(&self) -> &str {
        &self.peer
    }
    fn pub_key(&self) -> [u8; 32] {
        [0; 32]
    }
    fn epoch(&self) -> i64 {
        0
    }
    fn send<'a>(&'a self, msg: &'a [u8]) -> BoxFut<'a, Result<(), ConnError>> {
        Box::pin(async move {
            self.tx
                .send(msg.to_vec())
                .map_err(|_| ConnError::Transport("gone".into()))
        })
    }
    fn recv(&self) -> BoxFut<'_, Result<Vec<u8>, ConnError>> {
        Box::pin(async move {
            self.rx
                .lock()
                .await
                .recv()
                .await
                .ok_or_else(|| ConnError::Remote("closed".into()))
        })
    }
    fn open_stream<'a>(&'a self, _label: &'a str) -> BoxFut<'a, Result<Stream, ConnError>> {
        Box::pin(async { Err(ConnError::Local("no streams".into())) })
    }
    fn on_stream(&self, _h: StreamHandler) {}
    fn done(&self) -> BoxFut<'static, ()> {
        Box::pin(std::future::pending())
    }
    fn err(&self) -> Option<ConnError> {
        None
    }
    fn close(&self, reason: &str) {
        *self.closed.lock().unwrap() = Some(reason.into());
    }
    fn path(&self) -> String {
        "direct".into()
    }
    fn rtt(&self) -> Duration {
        Duration::ZERO
    }
    fn fingerprints(&self) -> (String, String) {
        (self.binding.clone(), self.binding.clone())
    }
}

/// One open board: its log, Verifier and secrets.
type Open = (Arc<Store>, Arc<BoardVerifier>, Arc<BoardSecrets>);

/// The Core's boards for one device, as `irori_core::Core` opens them.
struct Boards {
    dir: std::path::PathBuf,
    ident: Arc<Identity>,
    open: Mutex<BTreeMap<String, Open>>,
    pins: Mutex<BTreeMap<String, String>>,
}

impl InviteBoards for Boards {
    fn open_board(&self, id: &str, creator: Option<MemberPin>) -> Result<(), String> {
        let mut g = self.open.lock().unwrap();
        if g.contains_key(id) {
            return Ok(());
        }
        let pins: Vec<MemberPin> = creator.into_iter().collect();
        let ver = Arc::new(BoardVerifier::new_for(id, &pins));
        std::fs::create_dir_all(self.dir.join("boards")).map_err(|e| e.to_string())?;
        let st = Store::open_with(
            self.dir.join(format!("boards/{id}.db")),
            &self.ident.device_id(),
            Arc::new(store::Clock::with_now(Box::new(SystemTime::now))),
            OpenOptions {
                signer: Some(self.ident.clone()),
                verifier: Some(ver.clone()),
                board: true,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
        let sec = BoardSecrets::open(
            &self.dir.join(format!("boards/{id}.secrets")),
            id,
            Box::new(FileKeystore),
        )
        .map_err(|e| e.to_string())?;
        g.insert(id.into(), (Arc::new(st), ver, Arc::new(sec)));
        Ok(())
    }
    fn board_store(&self, id: &str) -> Option<Arc<Store>> {
        self.open.lock().unwrap().get(id).map(|b| b.0.clone())
    }
    fn board_verifier(&self, id: &str) -> Option<Arc<BoardVerifier>> {
        self.open.lock().unwrap().get(id).map(|b| b.1.clone())
    }
    fn board_secrets(&self, id: &str) -> Option<Arc<BoardSecrets>> {
        self.open.lock().unwrap().get(id).map(|b| b.2.clone())
    }
    fn pinned_key(&self, person_id: &str) -> Option<String> {
        self.pins.lock().unwrap().get(person_id).cloned()
    }
}

struct Person {
    _tmp: tempfile::TempDir,
    ident: Arc<Identity>,
    boards: Arc<Boards>,
}

fn person(name: &str) -> Person {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let mut dv = DeviceJson::load_or_create(&dir).unwrap();
    dv.name = format!("{name}'s laptop");
    dv.save(&dir).unwrap();
    // A made-up person named `name`: load once to make the keys, then
    // write person.json again under the name.
    let first = Identity::load(&dir, dv.clone(), Some(Box::new(FileKeystore))).unwrap();
    let mut p = first.current();
    let seed = first.person_seed(p.epoch).unwrap();
    p.name = name.into();
    drop(first);
    irori_identity::person::write_person_file_for_tests(
        &dir.join(irori_identity::PERSON_FILE),
        &FileKeystore,
        &p,
        *seed,
        &[],
    )
    .unwrap();
    let ident = Arc::new(Identity::load(&dir, dv, Some(Box::new(FileKeystore))).unwrap());
    assert_eq!(ident.current().name, name);
    let boards = Arc::new(Boards {
        dir,
        ident: ident.clone(),
        open: Mutex::default(),
        pins: Mutex::default(),
    });
    Person {
        _tmp: tmp,
        ident,
        boards,
    }
}

/// The board as the Go Core's CreateBoard writes it: the creator's
/// member.* rows first (so board.created's device signature checks against
/// the root's self-cert), then board.created, then epoch 1's secret.
fn create_board(p: &Person) {
    let pin = p.ident.board_pin();
    p.boards.open_board(BOARD, Some(pin.clone())).unwrap();
    let st = p.boards.board_store(BOARD).unwrap();
    let actor = person_actor(&pin.person_id);
    let cert = p
        .ident
        .member_cert_for(
            pin.person_epoch,
            &p.ident.device_id(),
            &hex::encode(p.ident.device_pub()),
            ROLE_DESKTOP,
            now(),
        )
        .unwrap();
    st.append(&actor, kinds::MEMBER_DEVICE_ADDED, &cert)
        .unwrap();
    let prof = p.ident.member_profile_row(pin.person_epoch).unwrap();
    st.append(&actor, kinds::MEMBER_PROFILE, &prof).unwrap();
    let bc = p
        .ident
        .board_created_row(BOARD, "MLG & co", "mlg", &pin, now())
        .unwrap();
    st.append(&actor, kinds::BOARD_CREATED, &bc).unwrap();
    st.append(
        &actor,
        "task.created",
        &serde_json::json!({"taskId": "t1", "title": "a card before the invite"}),
    )
    .unwrap();
    p.boards
        .board_secrets(BOARD)
        .unwrap()
        .mint(1, &p.ident.device_id())
        .unwrap();
    assert!(p.boards.board_verifier(BOARD).unwrap().membership().created);
}

fn quick() -> Timing {
    Timing {
        step: Duration::from_secs(10),
        words: Duration::from_secs(10),
    }
}

async fn wait_state(
    mut rx: tokio::sync::watch::Receiver<irori_invite::Status>,
    want: &str,
) -> irori_invite::Status {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let st = rx.borrow_and_update().clone();
            if st.state == want || st.state == STATE_FAILED {
                return st;
            }
            rx.changed().await.unwrap();
        }
    })
    .await
    .expect("state")
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invitee_joins_from_a_link_with_the_words() {
    let stephen = person("Stephen");
    let blake = person("Blake");
    create_board(&stephen);

    let host = Arc::new(
        Host::new(
            stephen.ident.clone(),
            stephen.boards.clone(),
            BOARD,
            quick(),
        )
        .unwrap(),
    );
    let link = host
        .ticket(&"11".repeat(32), "", "", vec!["127.0.0.1:5000".into()])
        .to_link();
    let guest =
        Arc::new(Guest::new(blake.ident.clone(), blake.boards.clone(), &link, quick()).unwrap());
    assert_eq!(
        guest.room().key,
        host.room().key,
        "both derive one room from the code"
    );

    let (a, b) = mem_pair(&stephen.ident.device_id(), &blake.ident.device_id());
    let h = tokio::spawn({
        let host = host.clone();
        async move { host.run(a).await }
    });
    let g = tokio::spawn({
        let guest = guest.clone();
        async move { guest.run(b).await }
    });

    let gst = wait_state(guest.subscribe(), STATE_WORDS).await;
    let hst = wait_state(host.subscribe(), STATE_WORDS).await;
    assert_eq!(gst.words.len(), 5, "{gst:?}");
    assert_eq!(gst.peer_name, "Stephen");
    assert_eq!(hst.peer_name, "Blake");
    // The fingerprints read out on the call match across the screens.
    assert_eq!(gst.peer_fingerprint, hst.my_fingerprint);
    assert_eq!(hst.peer_fingerprint, gst.my_fingerprint);

    assert_eq!(host.confirm("wrong words entirely here now"), Ok(false));
    assert_eq!(host.status().misses, 1);
    assert_eq!(host.confirm(&gst.words.join(" ").to_uppercase()), Ok(true));

    h.await.unwrap().unwrap();
    g.await.unwrap().unwrap();
    assert_eq!(host.status().state, STATE_DONE);
    assert_eq!(guest.status().state, STATE_DONE);

    // Both logs hold both people, the invitee as a member.
    for p in [&stephen, &blake] {
        let m = p.boards.board_verifier(BOARD).unwrap().membership();
        assert!(m.owner(&stephen.ident.current().id));
        assert!(m.active(&blake.ident.current().id));
        assert!(!m.owner(&blake.ident.current().id));
        let v = p.boards.board_verifier(BOARD).unwrap();
        assert!(
            v.cert(&blake.ident.device_id()).is_some(),
            "Blake's device counts"
        );
        assert!(v.cert(&stephen.ident.device_id()).is_some());
    }
    // The card made before the invite came along, and the secret matches.
    let bst = blake.boards.board_store(BOARD).unwrap();
    let cards = bst
        .list(&store::Filter {
            kinds: vec!["task.created".into()],
            limit: 10,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(cards.len(), 1);
    let (s1, m1) = stephen
        .boards
        .board_secrets(BOARD)
        .unwrap()
        .secret(1)
        .unwrap();
    let (s2, m2) = blake
        .boards
        .board_secrets(BOARD)
        .unwrap()
        .secret(1)
        .unwrap();
    assert_eq!((&*s1, m1), (&*s2, m2));
}

#[tokio::test(flavor = "multi_thread")]
async fn two_misses_end_it_and_nothing_is_written() {
    let stephen = person("Stephen");
    let blake = person("Blake");
    create_board(&stephen);
    let host = Arc::new(
        Host::new(
            stephen.ident.clone(),
            stephen.boards.clone(),
            BOARD,
            quick(),
        )
        .unwrap(),
    );
    let link = host.ticket(&"11".repeat(32), "", "", vec![]).to_link();
    let guest =
        Arc::new(Guest::new(blake.ident.clone(), blake.boards.clone(), &link, quick()).unwrap());
    let (a, b) = mem_pair("a", "b");
    let h = tokio::spawn({
        let host = host.clone();
        async move { host.run(a).await }
    });
    let g = tokio::spawn({
        let guest = guest.clone();
        async move { guest.run(b).await }
    });
    wait_state(host.subscribe(), STATE_WORDS).await;
    assert_eq!(host.confirm("one two three four five"), Ok(false));
    assert!(host.confirm("one two three four five").is_err());
    assert!(h.await.unwrap().is_err());
    let e = g.await.unwrap().unwrap_err();
    assert!(e.0.contains("words"), "{e}");
    assert!(
        blake.boards.board_store(BOARD).is_none(),
        "the board opened"
    );
    let m = stephen.boards.board_verifier(BOARD).unwrap().membership();
    assert!(!m.active(&blake.ident.current().id), "Blake was added");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_naming_someone_else_stops_the_invitee() {
    let stephen = person("Stephen");
    let mallory = person("Mallory");
    let blake = person("Blake");
    create_board(&stephen);
    let host = Arc::new(
        Host::new(
            stephen.ident.clone(),
            stephen.boards.clone(),
            BOARD,
            quick(),
        )
        .unwrap(),
    );
    // Mallory forwards Stephen's code in a link with her own fingerprint:
    // Blake's device sees Stephen's key on the other end and stops.
    let mut t = host.ticket(&"11".repeat(32), "", "", vec![]);
    t.fingerprint = irori_invite::code::person_fingerprint(
        &hex::decode(mallory.ident.current().pub_hex).unwrap(),
    );
    let guest = Arc::new(
        Guest::new(
            blake.ident.clone(),
            blake.boards.clone(),
            &t.to_link(),
            quick(),
        )
        .unwrap(),
    );
    let (a, b) = mem_pair("a", "b");
    let h = tokio::spawn({
        let host = host.clone();
        async move { host.run(a).await }
    });
    let e = guest.run(b).await.unwrap_err();
    assert!(e.0.contains("isn't the person the link names"), "{e}");
    assert!(h.await.unwrap().is_err());
    assert!(guest.status().words.is_empty(), "words shown");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_code_proves_nothing() {
    let stephen = person("Stephen");
    let blake = person("Blake");
    create_board(&stephen);
    let host = Arc::new(
        Host::new(
            stephen.ident.clone(),
            stephen.boards.clone(),
            BOARD,
            quick(),
        )
        .unwrap(),
    );
    let mut t = host.ticket(&"11".repeat(32), "", "", vec![]);
    t.code = irori_pair::new_code();
    let guest = Arc::new(
        Guest::new(
            blake.ident.clone(),
            blake.boards.clone(),
            &t.to_link(),
            quick(),
        )
        .unwrap(),
    );
    let (a, b) = mem_pair("a", "b");
    let h = tokio::spawn({
        let host = host.clone();
        async move { host.run(a).await }
    });
    let e = guest.run(b).await.unwrap_err();
    assert!(e.0.contains("code"), "{e}");
    assert!(h.await.unwrap().is_err());
    assert!(host.status().state == STATE_FAILED);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_guest_ends_the_invite() {
    let stephen = person("Stephen");
    let blake = person("Blake");
    create_board(&stephen);
    let host = Arc::new(
        Host::new(
            stephen.ident.clone(),
            stephen.boards.clone(),
            BOARD,
            quick(),
        )
        .unwrap(),
    );
    let link = host.ticket(&"11".repeat(32), "", "", vec![]).to_link();
    let guest =
        Arc::new(Guest::new(blake.ident.clone(), blake.boards.clone(), &link, quick()).unwrap());
    let (a, b) = mem_pair("a", "b");
    let h = tokio::spawn({
        let host = host.clone();
        async move { host.run(a).await }
    });
    let g = tokio::spawn({
        let guest = guest.clone();
        async move { guest.run(b).await }
    });
    wait_state(host.subscribe(), STATE_WORDS).await;
    let (a2, _b2) = mem_pair("a", "c");
    let e = host.run(a2).await.unwrap_err();
    assert!(e.0.contains("Someone else"), "{e}");
    assert!(h.await.unwrap().is_err());
    assert!(g.await.unwrap().is_err());
    assert!(host.confirm("anything").is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_different_pinned_key_stops_both() {
    let stephen = person("Stephen");
    let blake = person("Blake");
    create_board(&stephen);
    // Another of Stephen's boards pins "Blake" under another key.
    stephen
        .boards
        .pins
        .lock()
        .unwrap()
        .insert(blake.ident.current().id, "33".repeat(32));
    let host = Arc::new(
        Host::new(
            stephen.ident.clone(),
            stephen.boards.clone(),
            BOARD,
            quick(),
        )
        .unwrap(),
    );
    let link = host.ticket(&"11".repeat(32), "", "", vec![]).to_link();
    let guest =
        Arc::new(Guest::new(blake.ident.clone(), blake.boards.clone(), &link, quick()).unwrap());
    let (a, b) = mem_pair("a", "b");
    let h = tokio::spawn({
        let host = host.clone();
        async move { host.run(a).await }
    });
    let ge = guest.run(b).await.unwrap_err();
    let he = h.await.unwrap().unwrap_err();
    assert!(he.0.contains("different key"), "{he}");
    assert!(
        ge.0.contains("different key"),
        "the guest is told why: {ge}"
    );
}

#[test]
fn only_an_owner_opens_an_invite() {
    let blake = person("Blake");
    assert!(Host::new(blake.ident.clone(), blake.boards.clone(), BOARD, quick()).is_err());
}
