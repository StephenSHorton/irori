//! Port of I:internal/coord/boardlog_test.go: a coord over a shared
//! board's log. Go's direct `impl.apply`/`asideGate` calls are
//! `Coord::fold_live` here; its `impl.mayWrite` probes are left to the
//! forged-row checks around them.
#![allow(clippy::type_complexity)]

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use coord::rows::{LeaseReleasedRow, TaskCommentRow, TaskMemberRow, TaskStateRow};
use coord::*;
use gocompat::json::RawJson;
use store::{Event, OpenOptions, Store, kinds};

const BOARD_PERSON: &str = "person:p1";
const BOARD_CARD: &str = "agent:c1";

struct Board {
    _dir: tempfile::TempDir,
    st: Arc<Store>,
    clk: Clk,
}

fn board_store(dev: &str) -> Board {
    let clk = Clk(Arc::new(Mutex::new(fixed_now())));
    let c2 = clk.clone();
    let dir = tempfile::tempdir().unwrap();
    let hc = hlc::Clock::with_now(Box::new(move || c2.now().to_system()));
    let st = Store::open_with(
        dir.path().join("boards").join("b.db"),
        dev,
        Arc::new(hc),
        OpenOptions {
            board: true,
            ..OpenOptions::default()
        },
    )
    .unwrap();
    Board {
        _dir: dir,
        st: Arc::new(st),
        clk,
    }
}

fn raw(s: &str) -> RawJson {
    RawJson(s.as_bytes().to_vec())
}

fn created(b: &Board, tag: &str) {
    b.st.append(
        BOARD_PERSON,
        kinds::BOARD_CREATED,
        &raw(&format!(
            r#"{{"boardId":"b","creator":{{"personId":"p1"}},"name":"b","tag":"{tag}"}}"#
        )),
    )
    .unwrap();
}

fn card_row(b: &Board) {
    b.st.append(
        BOARD_CARD,
        kinds::AGENT_CARD,
        &raw(r#"{"cardId":"c1","personId":"p1","status":"active"}"#),
    )
    .unwrap();
}

fn board_opts(b: &Board) -> Options {
    Options {
        now: Some(b.clk.func()),
        board: true,
        person: BOARD_PERSON.into(),
        is_owner: Some(Arc::new(|k: &str| k == BOARD_PERSON)),
        ..Options::default()
    }
}

fn new_board_coord(dev: &str) -> (Board, Coord) {
    let b = board_store(dev);
    created(&b, "b");
    let c = Coord::new(b.st.clone(), None, board_opts(&b)).unwrap();
    (b, c)
}

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

#[test]
fn test_board_coord_actors() {
    let (b, c) = new_board_coord("dev-a");
    assert!(c.is_board());
    card_row(&b);
    let task = c.create_task(BOARD_PERSON, nt("shared work")).unwrap();
    c.assign(BOARD_PERSON, &task.id, BOARD_CARD).unwrap();
    c.claim_task(BOARD_CARD, &task.id).unwrap();
    c.comment_on(
        BOARD_PERSON,
        &task.id,
        CommentInput {
            text: "looks good".into(),
            ..CommentInput::default()
        },
    )
    .unwrap();
    let rows =
        b.st.list(&store::Filter {
            kinds: vec!["task.".into()],
            limit: store::MAX_LIST_LIMIT,
            ..store::Filter::default()
        })
        .unwrap();
    let want = [BOARD_PERSON, BOARD_PERSON, BOARD_CARD, BOARD_PERSON];
    assert_eq!(rows.len(), want.len());
    for (ev, w) in rows.iter().zip(want) {
        assert_eq!(ev.actor, w, "{}", ev.kind);
    }
    for by in ["user:dev-a", ALICE, &format!("api:{ALICE}")] {
        let err = c.create_task(by, nt("leak")).unwrap_err();
        assert_eq!(
            err.store_kind(),
            Some(store::ErrorKind::BoardActor),
            "{by}: {err}"
        );
    }
    let err = c.assign(BOARD_PERSON, &task.id, ALICE).unwrap_err();
    assert_eq!(
        err.store_kind(),
        Some(store::ErrorKind::OutsideBoard),
        "{err}"
    );
    assert_eq!(c.list_tasks(&TaskFilter::default()).len(), 1);
}

#[test]
fn test_board_coord_sets_aside() {
    let (src, from) = new_board_coord("dev-b");
    src.st
        .append(
            BOARD_CARD,
            kinds::AGENT_CARD,
            &raw(r#"{"cardId":"c1","personId":"p1"}"#),
        )
        .unwrap();
    let task = from.create_task(BOARD_PERSON, nt("from b")).unwrap();
    from.comment_on(
        BOARD_CARD,
        &task.id,
        CommentInput {
            text: "first".into(),
            ..CommentInput::default()
        },
    )
    .unwrap();
    let rows = src.st.missing(&store::Vector::new(), 100).unwrap();
    let (dst, c) = new_board_coord("dev-a");
    let pick = |kind: &str| rows.iter().find(|e| e.kind == kind).cloned().unwrap();
    let (board, card, created_ev, comment) = (
        pick(kinds::BOARD_CREATED),
        pick(kinds::AGENT_CARD),
        pick(kinds::TASK_CREATED),
        pick(kinds::TASK_COMMENT),
    );
    let insert = |evs: &[Event]| {
        let (_, r) = dst.st.insert_batch(evs);
        r.unwrap();
    };
    insert(&[board, card]);
    c.fold_live(std::slice::from_ref(&comment));
    assert!(c.unresolved() == 1 && c.comments(&task.id).is_none_or(|v| v.is_empty()));
    insert(&[created_ev.clone(), comment.clone()]);
    c.fold_live(&[created_ev]);
    assert_eq!(c.unresolved(), 0);
    let got = c.comments(&task.id).unwrap();
    assert!(got.len() == 1 && got[0].text == "first" && got[0].by == BOARD_CARD);
}

fn lease_board(
    devs: HashMap<&'static str, &'static str>,
    keys: Arc<Mutex<HashMap<String, String>>>,
) -> (Board, Coord) {
    let b = board_store("dev-a");
    let k2 = keys.clone();
    let o = Options {
        key_person: Some(Arc::new(move |k: &str| {
            k2.lock().unwrap().get(k).cloned().unwrap_or_default()
        })),
        device_person: Some(Arc::new(move |d: &str| {
            devs.get(d).map(|s| s.to_string()).unwrap_or_default()
        })),
        ..board_opts(&b)
    };
    let c = Coord::new(b.st.clone(), None, o).unwrap();
    (b, c)
}

#[test]
fn test_board_owner_rule_is_the_person() {
    let keys = Arc::new(Mutex::new(HashMap::from([
        (BOARD_PERSON.to_string(), "p1".to_string()),
        (BOARD_CARD.to_string(), "p1".to_string()),
        ("person:p2".to_string(), "p2".to_string()),
    ])));
    let (b, c) = lease_board(HashMap::from([("dev-a", "p1"), ("dev-b", "p2")]), keys);
    card_row(&b);
    c.acquire(BOARD_CARD, "port:5173", Duration::from_secs(3600), false)
        .unwrap();
    let r = LeaseReleasedRow {
        resource: "port:5173".into(),
        holder: BOARD_CARD.into(),
        reason: "done".into(),
        by: "person:p2".into(),
        at: fixed_now(),
    };
    let forged = Event {
        device_id: "dev-b".into(),
        seq: 1,
        hlc: hlc::Timestamp(hlc_at(plus(fixed_now(), mins(1)))),
        actor: "person:p2".into(),
        kind: kinds::LEASE_RELEASED.into(),
        payload: gocompat::json::marshal(&r).unwrap(),
        sig: Vec::new(),
    };
    b.st.insert(&forged).unwrap();
    c.fold_live(&[forged]);
    assert_eq!(c.ignored_rows(), 1);
    let ls = c.leases();
    assert!(ls.len() == 1 && ls[0].holder == BOARD_CARD, "{ls:?}");
}

fn state_row(
    task: &str,
    dev: &str,
    seq: i64,
    at: GoTimeArg,
    actor: &str,
    s: &str,
    note: &str,
) -> Event {
    let r = TaskStateRow {
        task_id: task.into(),
        state: s.into(),
        note: note.into(),
        by: actor.into(),
        at: fixed_now(),
        ..TaskStateRow::default()
    };
    Event {
        device_id: dev.into(),
        seq,
        hlc: hlc::Timestamp(hlc_at(at.0)),
        actor: actor.into(),
        kind: kinds::TASK_STATUS.into(),
        payload: gocompat::json::marshal(&r).unwrap(),
        sig: Vec::new(),
    }
}

struct GoTimeArg(gocompat::time::GoTime);

#[test]
fn test_board_cancel_is_the_creators_or_an_owners() {
    let b = board_store("dev-a");
    created(&b, "mlg");
    b.st.append(
        BOARD_PERSON,
        kinds::BOARD_MEMBER_ADDED,
        &raw(r#"{"personId":"p2","role":"member"}"#),
    )
    .unwrap();
    let devs = Arc::new(Mutex::new(HashMap::from([
        ("dev-a".to_string(), "p1".to_string()),
        ("dev-b".to_string(), "p2".to_string()),
    ])));
    let keys = Arc::new(Mutex::new(HashMap::from([
        (BOARD_PERSON.to_string(), "p1".to_string()),
        ("person:p2".to_string(), "p2".to_string()),
    ])));
    let owners = Arc::new(Mutex::new(vec!["p1".to_string()]));
    let (d2, k2, o2) = (devs.clone(), keys.clone(), owners.clone());
    let o = Options {
        key_person: Some(Arc::new(move |k: &str| {
            k2.lock().unwrap().get(k).cloned().unwrap_or_default()
        })),
        device_person: Some(Arc::new(move |d: &str| {
            d2.lock().unwrap().get(d).cloned().unwrap_or_default()
        })),
        tag: Some(Arc::new(|| "mlg".to_string())),
        board_owner: Some(Arc::new(move |p: &str| {
            o2.lock().unwrap().iter().any(|x| x == p)
        })),
        ..board_opts(&b)
    };
    let c = Coord::new(b.st.clone(), None, o).unwrap();
    let task = c.create_task(BOARD_PERSON, nt("shared work")).unwrap();
    assert_eq!(task.short_ref(), "mlg#1");
    assert_eq!(task.refs(), format!("mlg#1, task:{}", task.id));
    for (r, want) in [
        ("mlg#1", None),
        ("#1", None),
        ("1", None),
        ("other#1", Some(ErrorKind::NotFound)),
        ("mlg#2", Some(ErrorKind::NotFound)),
    ] {
        match (c.task_ref(r), want) {
            (Ok(id), None) => assert_eq!(id, task.id, "{r}"),
            (Err(e), Some(k)) => assert!(e.is(k), "{r}: {e}"),
            (got, _) => panic!("{r}: {got:?}"),
        }
    }
    let fold = |ev: Event| {
        b.st.insert(&ev).unwrap();
        c.fold_live(&[ev]);
    };
    let at = |m: u64| GoTimeArg(plus(fixed_now(), mins(m)));
    fold(state_row(
        &task.id,
        "dev-b",
        1,
        at(1),
        "person:p2",
        CANCELED,
        "mine now",
    ));
    assert!(c.get_task(&task.id).unwrap().state != CANCELED && c.ignored_rows() == 1);
    owners.lock().unwrap().push("p2".into());
    fold(state_row(
        &task.id,
        "dev-b",
        2,
        at(2),
        "person:p2",
        CANCELED,
        "mine now",
    ));
    let got = c.get_task(&task.id).unwrap();
    assert!(got.state == CANCELED && !got.withdrawn, "{got:?}");
    let again = c.create_task(BOARD_PERSON, nt("more work")).unwrap();
    let w = c.withdraw(BOARD_PERSON, &again.id, "").unwrap();
    assert!(w.withdrawn && w.state == CANCELED && w.note == "withdrawn from the board");
    card_row(&b);
    keys.lock().unwrap().insert(BOARD_CARD.into(), "p1".into());
    owners.lock().unwrap().retain(|p| p != "p2");
    let hours = |h: u64| GoTimeArg(plus(fixed_now(), Duration::from_secs(3600 * h)));
    fold(state_row(
        &again.id,
        "dev-c",
        1,
        hours(11),
        BOARD_CARD,
        WORKING,
        "still on it",
    ));
    devs.lock().unwrap().insert("dev-c".into(), "p1".into());
    fold(state_row(
        &again.id,
        "dev-c",
        2,
        hours(12),
        BOARD_CARD,
        WORKING,
        "still on it",
    ));
    fold(state_row(
        &again.id,
        "dev-b",
        3,
        hours(13),
        "person:p2",
        WORKING,
        "still on it",
    ));
    let got = c.get_task(&again.id).unwrap();
    assert!(got.withdrawn && got.state == CANCELED, "{got:?}");
    // The person's reopen brings it back (it runs at the store clock, so
    // move it past the rows above).
    b.clk.add(Duration::from_secs(3600 * 20));
    c.reopen(BOARD_PERSON, &again.id).unwrap();
    let got = c.get_task(&again.id).unwrap();
    assert!(!got.withdrawn && !is_terminal(&got.state), "{got:?}");
    let before = c.ignored_rows();
    let sweep = |seq: i64, kind: &str, payload: Vec<u8>| Event {
        device_id: "dev-b".into(),
        seq,
        hlc: hlc::Timestamp(hlc_at(plus(
            fixed_now(),
            Duration::from_secs(3600 * (40 + seq as u64)),
        ))),
        actor: "irori".into(),
        kind: kind.into(),
        payload,
        sig: Vec::new(),
    };
    fold(sweep(
        4,
        kinds::TASK_STATUS,
        gocompat::json::marshal(&TaskStateRow {
            task_id: again.id.clone(),
            state: SUBMITTED.into(),
            note: "gone".into(),
            by: "irori".into(),
            at: fixed_now(),
            ..TaskStateRow::default()
        })
        .unwrap(),
    ));
    fold(sweep(
        5,
        kinds::TASK_LEFT,
        gocompat::json::marshal(&TaskMemberRow {
            task_id: again.id.clone(),
            who: BOARD_CARD.into(),
            by: "irori".into(),
            at: fixed_now(),
            reason: LEFT_GONE.into(),
            person: false,
        })
        .unwrap(),
    ));
    assert_eq!(c.ignored_rows(), before + 2);
}

#[test]
fn test_parse_ref() {
    let cases: [(&str, Option<(&str, &str)>); 10] = [
        ("mlg#12", Some(("mlg", "12"))),
        ("MLG#12b", Some(("mlg", "12b"))),
        ("#12", Some(("", "12"))),
        ("12", Some(("", "12"))),
        ("a-1#3", Some(("a-1", "3"))),
        ("1mlg#3", None),
        ("mlg#", None),
        ("mlg#x", None),
        ("#", None),
        ("01M6336VP921939GKCWCDJP0EZ", None),
    ];
    for (input, want) in cases {
        assert_eq!(
            parse_ref(input),
            want.map(|(a, b)| (a.to_string(), b.to_string())),
            "{input:?}"
        );
    }
}

#[test]
fn test_board_coord_caps_set_aside() {
    let (_b, c) = new_board_coord("dev-a");
    let rows: Vec<Event> = (0..MAX_ASIDE + 3)
        .map(|i| {
            let r = TaskCommentRow {
                task_id: "nowhere".into(),
                comment_id: format!("m{}", (b'a' + (i % 26) as u8) as char),
                by: BOARD_CARD.into(),
                text: "x".into(),
                at: fixed_now(),
                ..TaskCommentRow::default()
            };
            Event {
                device_id: "dev-b".into(),
                seq: i as i64 + 1,
                actor: BOARD_CARD.into(),
                kind: kinds::TASK_COMMENT.into(),
                payload: gocompat::json::marshal(&r).unwrap(),
                ..Event::default()
            }
        })
        .collect();
    for ev in &rows {
        c.aside_gate(ev);
    }
    assert_eq!((c.unresolved(), c.aside_dropped()), (MAX_ASIDE, 3));
}

#[test]
fn test_board_rows_carry_the_writers_hop() {
    // v0.5 §7.1: an agent's comment or assignment on a board carries its
    // running hop, so the bridge on the other side keeps the count.
    let b = board_store("dev-a");
    created(&b, "b");
    let o = Options {
        hop_of: Some(Arc::new(
            |actor: &str| if actor == BOARD_CARD { 3 } else { 0 },
        )),
        ..board_opts(&b)
    };
    let c = Coord::new(b.st.clone(), None, o).unwrap();
    card_row(&b);
    let task = c.create_task(BOARD_PERSON, nt("shared work")).unwrap();
    c.assign(BOARD_CARD, &task.id, BOARD_PERSON).unwrap();
    let (cm, _) = c
        .comment_on(
            BOARD_CARD,
            &task.id,
            CommentInput {
                text: "on it".into(),
                ..CommentInput::default()
            },
        )
        .unwrap();
    c.comment_on(
        BOARD_PERSON,
        &task.id,
        CommentInput {
            text: "thanks".into(),
            ..CommentInput::default()
        },
    )
    .unwrap();
    let rows =
        b.st.list(&store::Filter {
            kinds: vec!["task.".into()],
            limit: store::MAX_LIST_LIMIT,
            ..store::Filter::default()
        })
        .unwrap();
    let hops: Vec<(String, i64)> = rows
        .iter()
        .map(|ev| (ev.kind.clone(), row_hop(&ev.payload)))
        .collect();
    assert_eq!(
        hops,
        [
            (kinds::TASK_CREATED.to_string(), 0),
            (kinds::TASK_ASSIGNED.to_string(), 3),
            (kinds::TASK_COMMENT.to_string(), 3),
            (kinds::TASK_COMMENT.to_string(), 0),
        ]
    );
    // The stamped row still folds as the comment it is.
    let got = c.comments(&task.id).unwrap();
    assert_eq!(got[0].id, cm.id);
    assert_eq!(got[0].text, "on it");
    assert!(carries_hop("board.ask.posted") && !carries_hop(kinds::TASK_CREATED));
}
