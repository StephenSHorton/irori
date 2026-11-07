//! The bundle's hand-off pieces for a person DM (card #538): admit with no
//! secret, the no-write rebuild, the dry fold, and join's order, role and
//! retry. Two made-up people with real identities and board stores on disk.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use gocompat::time::GoTime;
use irori_identity::membership::{BOARD_MEMBER, BOARD_OWNER};
use irori_identity::{
    BoardSecrets, BoardVerifier, DeviceJson, FileKeystore, Identity, MemberPin, ROLE_DESKTOP,
    ROLE_NODE, kinds,
};
use irori_invite::bundle::{self, all_rows, check_offer, person_actor, rebuild};
use irori_invite::{Bundle, InviteBoards};
use store::{OpenOptions, Store};

const BOARD: &str = "0123456789abcdef0123456789abcdef";

fn now() -> GoTime {
    GoTime::from_system(SystemTime::now())
}

/// One open board: its log, Verifier and secrets.
type Open = (Arc<Store>, Arc<BoardVerifier>, Arc<BoardSecrets>);

struct Boards {
    dir: PathBuf,
    ident: Arc<Identity>,
    open: Mutex<BTreeMap<String, Open>>,
    /// A board opened from now on can't save its secrets.
    break_secrets: AtomicBool,
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
        let path = self.dir.join(format!("boards/{id}.secrets"));
        let sec =
            BoardSecrets::open(&path, id, Box::new(FileKeystore)).map_err(|e| e.to_string())?;
        if self.break_secrets.load(Ordering::SeqCst) {
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        }
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
}

struct Person {
    ident: Arc<Identity>,
    boards: Arc<Boards>,
    // Last: fields drop in order, and Windows can't delete an open file.
    _tmp: tempfile::TempDir,
}

impl Person {
    fn id(&self) -> String {
        self.ident.current().id
    }
    fn store(&self) -> Arc<Store> {
        self.boards.board_store(BOARD).unwrap()
    }
    fn rows(&self) -> usize {
        all_rows(&self.store()).unwrap().len()
    }
    fn holds_nothing(&self) -> bool {
        self.boards.open.lock().unwrap().is_empty()
            && !self.boards.dir.join(format!("boards/{BOARD}.db")).exists()
            && !self
                .boards
                .dir
                .join(format!("boards/{BOARD}.secrets"))
                .exists()
    }
    fn join(&self, inviter: &Person, role: &str, b: &Bundle) -> Result<Vec<store::Event>, String> {
        bundle::join(&*self.boards, &self.ident, BOARD, &inviter.id(), role, b)
    }
}

fn person(name: &str) -> Person {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let mut dv = DeviceJson::load_or_create(&dir).unwrap();
    dv.name = format!("{name}'s laptop");
    dv.save(&dir).unwrap();
    let ident = Arc::new(Identity::load(&dir, dv, Some(Box::new(FileKeystore))).unwrap());
    let boards = Arc::new(Boards {
        dir,
        ident: ident.clone(),
        open: Mutex::default(),
        break_secrets: AtomicBool::new(false),
    });
    Person {
        _tmp: tmp,
        ident,
        boards,
    }
}

/// A two-person log as the founder writes one: `member.*`, `board.created`,
/// a row that isn't identity, and epoch 1's secret when `mint`.
fn found(p: &Person, mint: bool) {
    let pin = p.ident.board_pin();
    p.boards.open_board(BOARD, Some(pin.clone())).unwrap();
    let st = p.store();
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
        .board_created_row(BOARD, "dm", "~dm", &pin, now())
        .unwrap();
    st.append(&actor, kinds::BOARD_CREATED, &bc).unwrap();
    st.append(
        &actor,
        "task.created",
        &serde_json::json!({"taskId": "t1", "title": "a row before the add"}),
    )
    .unwrap();
    if mint {
        p.boards
            .board_secrets(BOARD)
            .unwrap()
            .mint(1, &p.ident.device_id())
            .unwrap();
    }
}

fn admit(founder: &Person, joiner_pin: &MemberPin) -> Result<Bundle, String> {
    bundle::admit(
        &*founder.boards,
        &founder.ident,
        BOARD,
        joiner_pin,
        "someone",
    )
}

/// A founded board with `other` added, and its bundle.
fn offered(founder: &Person, other: &Person) -> Bundle {
    found(founder, true);
    admit(founder, &other.ident.board_pin()).unwrap()
}

#[test]
fn a_device_with_no_secret_doesnt_burn_the_add() {
    let (bob, ann) = (person("Bob"), person("Ann"));
    found(&bob, false);
    let before = bob.rows();
    let e = admit(&bob, &ann.ident.board_pin()).unwrap_err();
    assert!(e.contains("no secret"), "{e}");
    assert_eq!(bob.rows(), before, "nothing was written");
    let m = bob.boards.board_verifier(BOARD).unwrap().membership();
    assert!(m.person(&ann.id()).is_none());

    // With the secret in hand the same add goes through.
    bob.boards
        .board_secrets(BOARD)
        .unwrap()
        .mint(1, &bob.ident.device_id())
        .unwrap();
    let b = admit(&bob, &ann.ident.board_pin()).unwrap();
    assert_eq!(bob.rows(), before + 1);
    assert_eq!(b.rows.len(), before + 1, "the bundle holds the add");
}

#[test]
fn rebuild_writes_nothing_and_carries_the_same_offer() {
    let (bob, ann) = (person("Bob"), person("Ann"));
    let first = offered(&bob, &ann);
    let before = bob.rows();
    let again = rebuild(&*bob.boards, &bob.ident, BOARD, &ann.id()).unwrap();
    assert_eq!(bob.rows(), before, "a retry writes no row");
    assert_eq!(again.secret, first.secret);
    assert_eq!(again.maker, first.maker);
    assert_eq!(again.board_epoch, first.board_epoch);
    assert_eq!((again.name.as_str(), again.tag.as_str()), ("dm", "~dm"));
    assert_eq!(again.rows, first.rows);
    assert!(check_offer(&ann.ident, BOARD, &again).is_ok());
}

#[test]
fn rebuild_is_only_for_someone_already_added() {
    let (bob, ann, cal) = (person("Bob"), person("Ann"), person("Cal"));
    offered(&bob, &ann);
    let e = rebuild(&*bob.boards, &bob.ident, BOARD, &cal.id()).unwrap_err();
    assert!(e.contains("isn't in this space"), "{e}");
    // A person who is only a member can't hand the secret on.
    let b = rebuild(&*bob.boards, &bob.ident, BOARD, &ann.id()).unwrap();
    ann.join(&bob, ROLE_DESKTOP, &b).unwrap();
    let e = rebuild(&*ann.boards, &ann.ident, BOARD, &bob.id()).unwrap_err();
    assert!(e.contains("only an owner"), "{e}");
}

#[test]
fn a_good_offer_folds_to_its_creator_and_the_two() {
    let (bob, ann) = (person("Bob"), person("Ann"));
    let b = offered(&bob, &ann);
    let f = check_offer(&ann.ident, BOARD, &b).unwrap();
    assert_eq!(f.creator, bob.ident.board_pin());
    assert!(f.membership.created);
    assert_eq!(f.membership.creator, bob.id());
    assert_eq!(f.membership.members.len(), 2);
    assert_eq!(f.membership.person(&bob.id()).unwrap().role, BOARD_OWNER);
    assert_eq!(f.membership.person(&ann.id()).unwrap().role, BOARD_MEMBER);
    assert!(f.verifier.cert(&bob.ident.device_id()).is_some());
    assert!(ann.holds_nothing(), "the fold touched no disk");
}

#[test]
fn an_offer_with_a_third_person_is_refused() {
    let (bob, ann, cal) = (person("Bob"), person("Ann"), person("Cal"));
    offered(&bob, &ann);
    let b = admit(&bob, &cal.ident.board_pin()).unwrap();
    let e = check_offer(&ann.ident, BOARD, &b).err().unwrap();
    assert!(e.contains("exactly its two"), "{e}");
    assert!(ann.holds_nothing());
}

#[test]
fn a_person_who_left_still_counts_as_one_of_the_people() {
    // Members in any state: a third person gone again is still a third.
    let (bob, ann, cal) = (person("Bob"), person("Ann"), person("Cal"));
    offered(&bob, &ann);
    admit(&bob, &cal.ident.board_pin()).unwrap();
    let row = bob
        .ident
        .board_member_removed_row(1, &cal.id(), "", None, now())
        .unwrap();
    bob.store()
        .append(&person_actor(&bob.id()), kinds::BOARD_MEMBER_REMOVED, &row)
        .unwrap();
    let b = rebuild(&*bob.boards, &bob.ident, BOARD, &ann.id()).unwrap();
    let e = check_offer(&ann.ident, BOARD, &b).err().unwrap();
    assert!(e.contains("exactly its two"), "{e}");
}

#[test]
fn an_offer_for_someone_elses_pin_is_refused() {
    for wrong in [
        |p: &mut MemberPin, _: &MemberPin| p.root_device_id = "another-device".into(),
        |p: &mut MemberPin, other: &MemberPin| p.person_pub = other.person_pub.clone(),
    ] {
        let (bob, ann) = (person("Bob"), person("Ann"));
        found(&bob, true);
        let mut pin = ann.ident.board_pin();
        wrong(&mut pin, &bob.ident.board_pin());
        let b = admit(&bob, &pin).unwrap();
        let e = check_offer(&ann.ident, BOARD, &b).err().unwrap();
        assert!(e.contains("under the key"), "{e}");
        assert!(ann.holds_nothing());
    }
}

#[test]
fn an_offer_under_another_key_epoch_is_refused() {
    let (bob, ann) = (person("Bob"), person("Ann"));
    found(&bob, true);
    let pin = MemberPin {
        person_epoch: ann.ident.board_pin().person_epoch + 1,
        ..ann.ident.board_pin()
    };
    let b = admit(&bob, &pin).unwrap();
    let e = check_offer(&ann.ident, BOARD, &b).err().unwrap();
    assert!(e.contains("under the key"), "{e}");
}

#[test]
fn an_offer_to_its_own_creator_is_refused() {
    let (bob, ann) = (person("Bob"), person("Ann"));
    let b = offered(&bob, &ann);
    let e = check_offer(&bob.ident, BOARD, &b).err().unwrap();
    assert!(e.contains("exactly its two"), "{e}");
}

#[test]
fn a_board_created_that_doesnt_verify_is_refused() {
    let (bob, ann) = (person("Bob"), person("Ann"));
    let mut b = offered(&bob, &ann);
    let row = b
        .rows
        .iter_mut()
        .find(|r| r.k == kinds::BOARD_CREATED)
        .unwrap();
    row.sig[0] ^= 1;
    let e = check_offer(&ann.ident, BOARD, &b).err().unwrap();
    assert!(e.contains("doesn't verify"), "{e}");
    assert!(ann.holds_nothing());

    b.rows.retain(|r| r.k != kinds::BOARD_CREATED);
    let e = check_offer(&ann.ident, BOARD, &b).err().unwrap();
    assert!(e.contains("no board.created"), "{e}");
}

#[test]
fn a_different_board_or_a_damaged_secret_is_refused() {
    let (bob, ann) = (person("Bob"), person("Ann"));
    let b = offered(&bob, &ann);
    let e = check_offer(&ann.ident, "another", &b).err().unwrap();
    assert!(e.contains("different space"), "{e}");

    let mut short = b.clone();
    short.secret = "c2hvcnQ=".into();
    let e = check_offer(&ann.ident, BOARD, &short).err().unwrap();
    assert!(e.contains("damaged"), "{e}");
    let mut no_maker = b.clone();
    no_maker.maker.clear();
    let e = check_offer(&ann.ident, BOARD, &no_maker).err().unwrap();
    assert!(e.contains("damaged"), "{e}");
    assert!(ann.holds_nothing());
}

#[test]
fn a_row_the_store_would_refuse_doesnt_count_in_the_fold() {
    // A third person's add with a broken device signature: the store stops
    // Bob's run there, and so does the fold, so both see two people.
    let (bob, ann, cal) = (person("Bob"), person("Ann"), person("Cal"));
    offered(&bob, &ann);
    let mut b = admit(&bob, &cal.ident.board_pin()).unwrap();
    let row = b
        .rows
        .iter_mut()
        .filter(|r| r.k == kinds::BOARD_MEMBER_ADDED)
        .max_by_key(|r| r.s)
        .unwrap();
    row.sig[0] ^= 1;
    let f = check_offer(&ann.ident, BOARD, &b).unwrap();
    assert_eq!(f.membership.members.len(), 2);
    ann.join(&bob, ROLE_DESKTOP, &b).unwrap();
    let m = ann.boards.board_verifier(BOARD).unwrap().membership();
    assert_eq!(m.members.len(), 2);
}

#[test]
fn a_seq_gap_stops_the_fold_as_it_stops_the_store() {
    let (bob, ann) = (person("Bob"), person("Ann"));
    let mut b = offered(&bob, &ann);
    // The row before the add is gone, so the add sits past a gap.
    b.rows.retain(|r| r.k != "task.created");
    let e = check_offer(&ann.ident, BOARD, &b).err().unwrap();
    assert!(e.contains("under the key"), "{e}");
}

#[test]
fn a_run_with_no_cert_yet_waits_for_the_run_that_certifies_it() {
    // Ann's rows sort wherever her device id falls, and they only verify
    // once Bob's run has pinned her. Either order folds the same.
    let (bob, ann) = (person("Bob"), person("Ann"));
    let b = offered(&bob, &ann);
    let done = ann.join(&bob, ROLE_DESKTOP, &b).unwrap();
    mesh::boards::insert_board_rows(&bob.store(), done).unwrap();
    let mut b = rebuild(&*bob.boards, &bob.ident, BOARD, &ann.id()).unwrap();
    for _ in 0..2 {
        b.rows.reverse();
        let f = check_offer(&ann.ident, BOARD, &b).unwrap();
        assert!(f.verifier.cert(&ann.ident.device_id()).is_some());
    }
}

#[test]
fn join_takes_its_role_from_the_caller() {
    let (bob, ann) = (person("Bob"), person("Ann"));
    let b = offered(&bob, &ann);
    let rows = ann.join(&bob, ROLE_NODE, &b).unwrap();
    assert_eq!(rows.len(), 2, "its cert and its profile");
    let ver = ann.boards.board_verifier(BOARD).unwrap();
    assert_eq!(ver.cert(&ann.ident.device_id()).unwrap().role, ROLE_NODE);

    let (bob, ann) = (person("Bob"), person("Ann"));
    let b = offered(&bob, &ann);
    ann.join(&bob, ROLE_DESKTOP, &b).unwrap();
    let ver = ann.boards.board_verifier(BOARD).unwrap();
    assert_eq!(ver.cert(&ann.ident.device_id()).unwrap().role, ROLE_DESKTOP);
}

#[test]
fn join_keeps_the_secret_before_any_row_goes_in() {
    let (bob, ann) = (person("Bob"), person("Ann"));
    let b = offered(&bob, &ann);
    ann.boards.break_secrets.store(true, Ordering::SeqCst);
    assert!(ann.join(&bob, ROLE_DESKTOP, &b).is_err());
    assert_eq!(ann.rows(), 0, "no row goes in without the secret");
}

#[test]
fn a_refused_bundle_leaves_no_secret_and_no_board() {
    let (bob, ann, cal) = (person("Bob"), person("Ann"), person("Cal"));
    let b = offered(&bob, &ann);
    // Cal isn't the person the board lists; Ann's inviter must be an owner.
    let e = cal.join(&bob, ROLE_DESKTOP, &b).unwrap_err();
    assert!(e.contains("under the key"), "{e}");
    assert!(cal.holds_nothing());
    let e = ann.join(&cal, ROLE_DESKTOP, &b).unwrap_err();
    assert!(e.contains("isn't an owner"), "{e}");
    assert!(ann.holds_nothing());
}

#[test]
fn a_second_join_writes_nothing_and_sends_the_same_rows() {
    let (bob, ann) = (person("Bob"), person("Ann"));
    let b = offered(&bob, &ann);
    let first = ann.join(&bob, ROLE_DESKTOP, &b).unwrap();
    let held = ann.rows();

    // The `done` was lost: Bob holds neither row and offers again.
    let b = rebuild(&*bob.boards, &bob.ident, BOARD, &ann.id()).unwrap();
    let again = ann.join(&bob, ROLE_DESKTOP, &b).unwrap();
    assert_eq!(ann.rows(), held, "a retry writes no row");
    assert_eq!(again, first);
    assert_eq!(again.first().map(|e| e.seq), Some(1));

    // And Bob's log takes them, so the hand-off ends.
    mesh::boards::insert_board_rows(&bob.store(), again).unwrap();
    let ver = bob.boards.board_verifier(BOARD).unwrap();
    assert!(ver.cert(&ann.ident.device_id()).is_some());
}
