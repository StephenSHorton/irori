//! One board log held by two made-up people (card #570), p1 on dev-a and p2
//! on dev-b, each with its own store and coord. Rows move between them by
//! vector, as the board sync does, and both folds must agree.

use std::sync::{Arc, Mutex};

use coord::{Coord, Options, RowPos};
use gocompat::json::RawJson;
use store::{OpenOptions, Store, kinds};

use super::Clk;

pub const P1: &str = "person:p1";
pub const P2: &str = "person:p2";
pub const CARD1: &str = "agent:c1";
pub const CARD2: &str = "agent:c2";

/// The people who own the board now.
pub type Owners = Arc<Mutex<Vec<String>>>;

pub struct Side {
    pub clk: Clk,
    person: String,
    owners: Owners,
    pub st: Arc<Store>,
    pub c: Coord,
    // Last: the store holds files open in it.
    pub _dir: tempfile::TempDir,
}

impl Side {
    /// A new coord over the same store, as after a restart.
    pub fn restart(&mut self) {
        self.c = coord_for(&self.st, &self.clk, &self.person, &self.owners);
    }
}

fn key_person(k: &str) -> String {
    match k {
        P1 | CARD1 => "p1".into(),
        P2 | CARD2 => "p2".into(),
        _ => String::new(),
    }
}

fn device_person(d: &str) -> String {
    match d {
        "dev-a" => "p1".into(),
        "dev-b" => "p2".into(),
        _ => String::new(),
    }
}

fn side(dev: &str, person: &str, owners: &Owners) -> Side {
    let clk = Clk(Arc::new(Mutex::new(super::fixed_now())));
    let c2 = clk.clone();
    let dir = tempfile::tempdir().unwrap();
    let hc = hlc::Clock::with_now(Box::new(move || c2.now().to_system()));
    let st = Arc::new(
        Store::open_with(
            dir.path().join("boards").join("b.db"),
            dev,
            Arc::new(hc),
            OpenOptions {
                board: true,
                ..OpenOptions::default()
            },
        )
        .unwrap(),
    );
    let c = coord_for(&st, &clk, person, owners);
    Side {
        clk,
        person: person.into(),
        owners: owners.clone(),
        st,
        c,
        _dir: dir,
    }
}

fn coord_for(st: &Arc<Store>, clk: &Clk, person: &str, owners: &Owners) -> Coord {
    let me = person.to_string();
    let who = owners.clone();
    let o = Options {
        now: Some(clk.func()),
        board: true,
        person: person.into(),
        is_owner: Some(Arc::new(move |k: &str| k == me)),
        key_person: Some(Arc::new(key_person)),
        device_person: Some(Arc::new(device_person)),
        tag: Some(Arc::new(|| "mlg".to_string())),
        board_owner: Some(Arc::new(move |p: &str, _: Option<&RowPos>| {
            who.lock().unwrap().iter().any(|o| o == p)
        })),
        ..Options::default()
    };
    Coord::new(st.clone(), None, o).unwrap()
}

fn raw(s: &str) -> RawJson {
    RawJson(s.as_bytes().to_vec())
}

/// p1 on dev-a and p2 on dev-b, with p1 the board's owner.
pub fn two_people() -> (Side, Side, Owners) {
    let owners: Owners = Arc::new(Mutex::new(vec!["p1".into()]));
    let a = side("dev-a", P1, &owners);
    let b = side("dev-b", P2, &owners);
    a.st.append(
        P1,
        kinds::BOARD_CREATED,
        &raw(r#"{"boardId":"b","creator":{"personId":"p1"},"name":"mlg","tag":"mlg"}"#),
    )
    .unwrap();
    a.st.append(
        P1,
        kinds::BOARD_MEMBER_ADDED,
        &raw(r#"{"personId":"p2","role":"member"}"#),
    )
    .unwrap();
    a.st.append(
        CARD1,
        kinds::AGENT_CARD,
        &raw(r#"{"cardId":"c1","personId":"p1","status":"active"}"#),
    )
    .unwrap();
    pull(&a, &b);
    b.st.append(
        CARD2,
        kinds::AGENT_CARD,
        &raw(r#"{"cardId":"c2","personId":"p2","status":"active"}"#),
    )
    .unwrap();
    sync(&a, &b);
    (a, b, owners)
}

/// Moves what each side lacks from the other, both ways, and folds it.
pub fn sync(a: &Side, b: &Side) {
    pull(a, b);
    pull(b, a);
}

/// Moves what `to` lacks from `from` and folds it on `to`.
pub fn pull(from: &Side, to: &Side) {
    let rows = from.st.missing(&to.st.vector().unwrap(), 1000).unwrap();
    let (_, r) = to.st.insert_batch(&rows);
    r.unwrap();
    to.c.fold_live(&rows);
}
