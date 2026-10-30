//! Shared boards stage 2 (v0.5 §5.3, §5.4, §9, §10): one board log held by
//! two made-up people, p1 on dev-a and p2 on dev-b, each with its own store
//! and coord. Rows move between them by vector, as the board sync does,
//! and both folds must agree.
#![allow(clippy::type_complexity)]

mod common;

use gocompat::time::GoTime;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use coord::rows::TaskStateRow;
use coord::*;
use gocompat::json::RawJson;
use store::{Event, OpenOptions, Store, kinds};

const P1: &str = "person:p1";
const P2: &str = "person:p2";
const CARD1: &str = "agent:c1";
const CARD2: &str = "agent:c2";

/// Who owned the board when: `(person, from hlc, until hlc)`.
type Owners = Arc<Mutex<Vec<(String, u64, u64)>>>;

struct Side {
    _dir: tempfile::TempDir,
    st: Arc<Store>,
    clk: Clk,
    c: Coord,
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
        "dev-a" | "dev-a2" => "p1".into(),
        "dev-b" | "dev-b2" => "p2".into(),
        _ => String::new(),
    }
}

fn side(dev: &str, person: &str, owners: &Owners) -> Side {
    let clk = Clk(Arc::new(Mutex::new(fixed_now())));
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
    let me = person.to_string();
    let mine = key_person(person);
    let o2 = owners.clone();
    let o = Options {
        now: Some(clk.func()),
        board: true,
        person: person.into(),
        is_owner: Some(Arc::new(move |k: &str| k == me || key_person(k) == mine)),
        key_person: Some(Arc::new(key_person)),
        device_person: Some(Arc::new(device_person)),
        tag: Some(Arc::new(|| "mlg".to_string())),
        board_owner: Some(Arc::new(move |p: &str, at: Option<&RowPos>| {
            o2.lock().unwrap().iter().any(|(who, from, until)| {
                who == p
                    && at.map_or(*until == u64::MAX, |at| *from < at.hlc && at.hlc < *until)
            })
        })),
        ..Options::default()
    };
    let c = Coord::new(st.clone(), None, o).unwrap();
    Side {
        _dir: dir,
        st,
        clk,
        c,
    }
}

fn raw(s: &str) -> RawJson {
    RawJson(s.as_bytes().to_vec())
}

/// p1 on dev-a and p2 on dev-b, with p1 the board's owner forever. dev-a
/// writes board.created and both people's agent cards travel to dev-b.
fn two_people() -> (Side, Side, Owners) {
    let owners: Owners = Arc::new(Mutex::new(vec![("p1".into(), 0, u64::MAX)]));
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
fn sync(a: &Side, b: &Side) {
    pull(a, b);
    pull(b, a);
}

/// Moves what `to` lacks from `from` and folds it on `to`.
fn pull(from: &Side, to: &Side) {
    let rows = from.st.missing(&to.st.vector().unwrap(), 1000).unwrap();
    let (_, r) = to.st.insert_batch(&rows);
    r.unwrap();
    to.c.fold_live(&rows);
}

fn nt(title: &str, repo: &str) -> NewTask {
    NewTask {
        title: title.into(),
        repo_key: repo.into(),
        ..NewTask::default()
    }
}

fn hours(h: u64) -> GoTime {
    plus(fixed_now(), Duration::from_secs(3600 * h))
}

fn cancel_row(task: &str, dev: &str, seq: i64, at: GoTime, actor: &str) -> Event {
    let r = TaskStateRow {
        task_id: task.into(),
        state: CANCELED.into(),
        note: "not ours".into(),
        by: actor.into(),
        at,
        ..TaskStateRow::default()
    };
    Event {
        device_id: dev.into(),
        seq,
        hlc: hlc::Timestamp(hlc_at(at)),
        actor: actor.into(),
        kind: kinds::TASK_STATUS.into(),
        payload: gocompat::json::marshal(&r).unwrap(),
        sig: Vec::new(),
    }
}

// ---- the cancel rule at the row's time (§5.3) -------------------------------

#[test]
fn cancel_reads_the_owners_at_the_rows_time() {
    let (a, b, owners) = two_people();
    let one = a.c.create_task(P1, nt("one", "")).unwrap();
    let two = a.c.create_task(P1, nt("two", "")).unwrap();
    sync(&a, &b);
    // p2 owned the board from hour 1 to hour 3, and owns it no longer.
    owners
        .lock()
        .unwrap()
        .push(("p2".into(), hlc_at(hours(1)), hlc_at(hours(3))));
    // Written on p2's other device, so both stores take them by insert.
    let in_window = cancel_row(&one.id, "dev-b2", 1, hours(2), P2);
    let after = cancel_row(&two.id, "dev-b2", 2, hours(4), P2);
    for s in [&a, &b] {
        s.clk.add(Duration::from_secs(3600 * 5));
        let before = s.c.ignored_rows();
        for ev in [&in_window, &after] {
            s.st.insert(ev).unwrap();
            s.c.fold_live(std::slice::from_ref(ev));
        }
        assert_eq!(
            s.c.get_task(&one.id).unwrap().state,
            CANCELED,
            "a cancel by an owner at its time counts after they stop owning"
        );
        assert_ne!(
            s.c.get_task(&two.id).unwrap().state,
            CANCELED,
            "a cancel stamped after its writer stopped owning"
        );
        assert_eq!(s.c.ignored_rows(), before + 1);
    }
    // A withdraw now reads today's owners: p2 is none.
    let three = a.c.create_task(P1, nt("three", "")).unwrap();
    sync(&a, &b);
    let e = b.c.withdraw(P2, &three.id, "").unwrap_err();
    assert!(e.is(ErrorKind::NotAllowed), "{e}");
}
