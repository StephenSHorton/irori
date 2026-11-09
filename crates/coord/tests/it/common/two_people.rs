//! One board log held by two made-up people (card #570), p1 on dev-a and p2
//! on dev-b, each with its own store and coord. Rows move between them by
//! vector, as the board sync does, and both folds must agree.

use std::sync::{Arc, Mutex};

use coord::roster::{AgentCard, Agents};
use coord::{Coord, Options, RowPos};
use gocompat::json::RawJson;
use store::{OpenOptions, Store, kinds};

use super::Clk;

pub const P1: &str = "person:p1";
pub const P2: &str = "person:p2";
pub const CARD1: &str = "agent:c1";
pub const CARD2: &str = "agent:c2";
/// p2's second agent, on the board only after [`three_people`].
pub const CARD3: &str = "agent:c3";
/// p3 and its agent, from [`three_people`].
pub const P3: &str = "person:p3";
pub const CARD4: &str = "agent:c4";

/// The people who own the board now.
pub type Owners = Arc<Mutex<Vec<String>>>;

/// Each person's GitHub login as their profile says it, and who has left,
/// shared by every side of one test: p1 alice, p2 bob, p3 carol (card
/// #1155).
pub type Logins = Arc<Mutex<People>>;

#[derive(Default)]
pub struct People {
    logins: Vec<(String, String)>,
    gone: Vec<String>,
}

fn logins() -> Logins {
    Arc::new(Mutex::new(People {
        logins: vec![
            ("p1".into(), "alice".into()),
            ("p2".into(), "bob".into()),
            ("p3".into(), "carol".into()),
        ],
        gone: Vec::new(),
    }))
}

pub struct Side {
    pub clk: Clk,
    person: String,
    owners: Owners,
    logins: Logins,
    pub st: Arc<Store>,
    pub c: Coord,
    // Last: the store holds files open in it.
    pub _dir: tempfile::TempDir,
}

impl Side {
    /// A new coord over the same store, as after a restart.
    pub fn restart(&mut self) {
        self.c = coord_for(
            &self.st,
            &self.clk,
            &self.person,
            &self.owners,
            &self.logins,
            None,
        );
    }

    /// `person`'s profile now says GitHub `login` and nothing else, on
    /// every side of the test ("" for none).
    pub fn set_login(&self, person: &str, login: &str) {
        let mut l = self.logins.lock().unwrap();
        l.logins.retain(|(p, _)| p != person);
        l.logins.push((person.into(), login.into()));
    }

    /// `person` is no longer an active member, on every side of the test.
    pub fn set_gone(&self, person: &str) {
        self.logins.lock().unwrap().gone.push(person.into());
    }

    /// A new coord over the same store that sees `agents` as the board's
    /// roster.
    pub fn with_roster(&mut self, agents: Vec<AgentCard>) {
        let r: Arc<dyn Agents> = Arc::new(FixedRoster(agents));
        self.c = coord_for(
            &self.st,
            &self.clk,
            &self.person,
            &self.owners,
            &self.logins,
            Some(r),
        );
    }
}

struct FixedRoster(Vec<AgentCard>);

impl Agents for FixedRoster {
    fn agents(&self) -> Vec<AgentCard> {
        self.0.clone()
    }
}

fn key_person(k: &str) -> String {
    match k {
        P1 | CARD1 => "p1".into(),
        P2 | CARD2 | CARD3 => "p2".into(),
        P3 | CARD4 => "p3".into(),
        _ => String::new(),
    }
}

/// dev-a2, dev-b2 and dev-c2 write no rows of their own here: tests forge
/// rows from them.
fn device_person(d: &str) -> String {
    match d {
        "dev-a" | "dev-a2" | "dev-a3" => "p1".into(),
        "dev-b" | "dev-b2" => "p2".into(),
        "dev-c" | "dev-c2" => "p3".into(),
        _ => String::new(),
    }
}

/// Another device of `person` holding the same log: it pulls everything
/// `from` has.
pub fn device(dev: &str, person: &str, from: &Side) -> Side {
    let s = side(dev, person, &from.owners, &from.logins);
    pull(from, &s);
    s
}

fn side(dev: &str, person: &str, owners: &Owners, logins: &Logins) -> Side {
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
    let c = coord_for(&st, &clk, person, owners, logins, None);
    Side {
        clk,
        person: person.into(),
        owners: owners.clone(),
        logins: logins.clone(),
        st,
        c,
        _dir: dir,
    }
}

fn coord_for(
    st: &Arc<Store>,
    clk: &Clk,
    person: &str,
    owners: &Owners,
    logins: &Logins,
    roster: Option<Arc<dyn Agents>>,
) -> Coord {
    let me = person.to_string();
    let who = owners.clone();
    let ls = logins.clone();
    let ls2 = logins.clone();
    let o = Options {
        now: Some(clk.func()),
        board: true,
        person: person.into(),
        is_owner: Some(Arc::new(move |k: &str| k == me)),
        key_person: Some(Arc::new(key_person)),
        device_person: Some(Arc::new(device_person)),
        // Everyone whose profile claims the login.
        login_people: Some(Arc::new(move |login: &str| {
            let l = ls.lock().unwrap();
            l.logins
                .iter()
                .filter(|(p, x)| {
                    !login.is_empty() && x.eq_ignore_ascii_case(login) && !l.gone.contains(p)
                })
                .map(|(p, _)| p.clone())
                .collect()
        })),
        member_active: Some(Arc::new(move |p: &str| {
            !p.is_empty() && !ls2.lock().unwrap().gone.iter().any(|g| g == p)
        })),
        tag: Some(Arc::new(|| "mlg".to_string())),
        board_owner: Some(Arc::new(move |p: &str, _: Option<&RowPos>| {
            who.lock().unwrap().iter().any(|o| o == p)
        })),
        ..Options::default()
    };
    Coord::new(st.clone(), roster, o).unwrap()
}

fn raw(s: &str) -> RawJson {
    RawJson(s.as_bytes().to_vec())
}

/// p1 on dev-a and p2 on dev-b, with p1 the board's owner.
pub fn two_people() -> (Side, Side, Owners) {
    let owners: Owners = Arc::new(Mutex::new(vec!["p1".into()]));
    let ls = logins();
    let a = side("dev-a", P1, &owners, &ls);
    let b = side("dev-b", P2, &owners, &ls);
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

/// two_people, with p2's second agent c3 and a third member p3 on dev-c
/// with its agent c4.
pub fn three_people() -> (Side, Side, Side, Owners) {
    let (a, b, owners) = two_people();
    let c = side("dev-c", P3, &owners, &a.logins);
    a.st.append(
        P1,
        kinds::BOARD_MEMBER_ADDED,
        &raw(r#"{"personId":"p3","role":"member"}"#),
    )
    .unwrap();
    b.st.append(
        CARD3,
        kinds::AGENT_CARD,
        &raw(r#"{"cardId":"c3","personId":"p2","status":"active"}"#),
    )
    .unwrap();
    sync(&a, &b);
    pull(&a, &c);
    c.st.append(
        CARD4,
        kinds::AGENT_CARD,
        &raw(r#"{"cardId":"c4","personId":"p3","status":"active"}"#),
    )
    .unwrap();
    sync_all(&[&a, &b, &c]);
    (a, b, c, owners)
}

/// Syncs every pair, twice, so all sides hold everything.
pub fn sync_all(sides: &[&Side]) {
    for _ in 0..2 {
        for (i, x) in sides.iter().enumerate() {
            for y in &sides[i + 1..] {
                sync(x, y);
            }
        }
    }
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

/// Moves at most `n` of what `to` lacks from `from`, oldest first, as a
/// sync cut short would.
pub fn pull_n(from: &Side, to: &Side, n: i64) -> usize {
    let rows = from.st.missing(&to.st.vector().unwrap(), n).unwrap();
    let (_, r) = to.st.insert_batch(&rows);
    r.unwrap();
    to.c.fold_live(&rows);
    rows.len()
}
