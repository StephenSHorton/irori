//! Card #563: a move is an edit row with `board` and `num`, a board delete
//! is a rename row with `deleted`, and both are folded by one walk in
//! RowPos order (docs/design/no-server.md §3.3), so arrival order changes
//! nothing.
#![allow(clippy::type_complexity)]

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use common::*;
use coord::rows::{TaskCreatedRow, TaskEditedRow};
use coord::*;
use gocompat::json::RawJson;
use store::{Event, OpenOptions, Store, kinds};

const ME: &str = "person:p1";
const NET: &str = "b-net";
const ART: &str = "b-art";

struct Space {
    _dir: tempfile::TempDir,
    st: Arc<Store>,
    clk: Clk,
    list: Arc<Mutex<BoardList>>,
}

fn space(dev: &str) -> Space {
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
    st.append(
        ME,
        kinds::BOARD_CREATED,
        &RawJson(
            br#"{"boardId":"b","creator":{"personId":"p1"},"name":"b","tag":"wind"}"#.to_vec(),
        ),
    )
    .unwrap();
    let list = BoardList {
        own: "b".into(),
        deleted: Vec::new(),
        tags: vec![
            ("net".to_string(), NET.to_string()),
            ("art".to_string(), ART.to_string()),
        ],
    };
    Space {
        _dir: dir,
        st: Arc::new(st),
        clk,
        list: Arc::new(Mutex::new(list)),
    }
}

fn opts(s: &Space) -> Options {
    let (l1, l2) = (s.list.clone(), s.list.clone());
    Options {
        now: Some(s.clk.func()),
        board: true,
        person: ME.into(),
        is_owner: Some(Arc::new(|k: &str| k == ME)),
        tag: Some(Arc::new(|| "wind".to_string())),
        board_tag: Some(Arc::new(move |id: &str| {
            let l = l1.lock().unwrap();
            if l.deleted.iter().any(|(b, _)| b == id) {
                return None;
            }
            l.tags
                .iter()
                .find(|(_, b)| b == id)
                .map(|(t, _)| t.clone())
        })),
        board_tags: Some(Arc::new(|| vec!["net".to_string(), "art".to_string()])),
        board_view: Some(Arc::new(move || l2.lock().unwrap().clone())),
        ..Options::default()
    }
}

fn coord_on(s: &Space) -> Coord {
    Coord::new(s.st.clone(), None, opts(s)).unwrap()
}

fn card(c: &Coord, board: &str, title: &str) -> Task {
    c.create_task(
        ME,
        NewTask {
            title: title.into(),
            board: board.into(),
            ..NewTask::default()
        },
    )
    .unwrap()
}

fn event(dev: &str, seq: i64, at: u64, kind: &str, raw: Vec<u8>) -> Event {
    Event {
        device_id: dev.into(),
        seq,
        hlc: hlc::Timestamp(at),
        actor: ME.into(),
        kind: kind.into(),
        payload: raw,
        sig: Vec::new(),
    }
}

fn created(dev: &str, seq: i64, at: u64, id: &str, num: i64, board: &str) -> Event {
    let raw = gocompat::json::marshal(&TaskCreatedRow {
        task_id: id.into(),
        num,
        title: id.into(),
        board: board.into(),
        by: ME.into(),
        at: fixed_now(),
        ..TaskCreatedRow::default()
    })
    .unwrap();
    event(dev, seq, at, kinds::TASK_CREATED, raw)
}

fn moved(dev: &str, seq: i64, at: u64, id: &str, num: i64, board: &str) -> Event {
    let raw = gocompat::json::marshal(&TaskEditedRow {
        task_id: id.into(),
        board: Some(board.into()),
        num,
        by: ME.into(),
        at: fixed_now(),
        ..TaskEditedRow::default()
    })
    .unwrap();
    event(dev, seq, at, kinds::BOARD_ASK_EDITED, raw)
}

/// Lands a row the way a pull does: in the store, then the fold.
fn land(s: &Space, c: &Coord, ev: &Event) {
    s.st.insert(ev).unwrap();
    c.handle(ev).unwrap();
}

fn pos(dev: &str, seq: i64, at: u64) -> RowPos {
    RowPos {
        hlc: at,
        dev: dev.into(),
        seq,
    }
}

/// Deletes `board` as if its row had just landed from `dev`.
fn delete(s: &Space, c: &Coord, dev: &str, seq: i64, at: u64, board: &str) {
    s.list
        .lock()
        .unwrap()
        .deleted
        .push((board.into(), pos(dev, seq, at)));
    let raw = format!(r#"{{"boardId":"{board}","name":"","tag":"","deleted":true}}"#);
    land(
        s,
        c,
        &event(dev, seq, at, kinds::SPACE_BOARD_RENAMED, raw.into_bytes()),
    );
}

fn place(c: &Coord, id: &str) -> (String, String) {
    let t = c.get_task(id).unwrap();
    (t.board.clone(), t.short_ref())
}

#[test]
fn a_moved_card_takes_the_next_number_and_keeps_its_old_ref() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let w = card(&c, "", "w1");
    card(&c, NET, "n1");
    let t = c.move_task(ME, &w.id, NET).unwrap();
    assert_eq!(t.short_ref(), "net#2");
    assert_eq!(t.board, NET);
    assert_eq!(c.task_ref("net#2").unwrap(), w.id);
    assert_eq!(c.task_ref("wind#1").unwrap(), w.id, "the old ref finds it");
    // A number it left isn't given out again.
    assert_eq!(card(&c, "", "w2").short_ref(), "wind#2");
    // A restart folds the same place, and the old ref still works.
    let c = coord_on(&s);
    assert_eq!(place(&c, &w.id), (NET.to_string(), "net#2".to_string()));
    assert_eq!(c.task_ref("wind#1").unwrap(), w.id);
    assert_eq!(card(&c, NET, "n3").short_ref(), "net#3");
}

#[test]
fn a_move_back_takes_a_new_number_and_every_old_ref_still_works() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let w = card(&c, "", "w1");
    c.move_task(ME, &w.id, NET).unwrap();
    let t = c.move_task(ME, &w.id, "").unwrap();
    assert_eq!(t.short_ref(), "wind#2");
    assert_eq!(t.board, "");
    assert_eq!(c.task_ref("wind#1").unwrap(), w.id);
    assert_eq!(c.task_ref("net#1").unwrap(), w.id);
}

/// The first board's own id means the same as `""`.
#[test]
fn a_move_to_the_first_board_by_its_id_is_the_same_board() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let w = card(&c, NET, "n1");
    let t = c.move_task(ME, &w.id, "b").unwrap();
    assert_eq!(t.board, "");
    assert_eq!(t.short_ref(), "wind#1");
}

/// The row carries `"board":""`: omitted, it would be an empty edit.
#[test]
fn a_move_to_the_first_board_writes_the_empty_board() {
    let raw = gocompat::json::marshal(&TaskEditedRow {
        task_id: "T".into(),
        board: Some(String::new()),
        num: 3,
        by: ME.into(),
        ..TaskEditedRow::default()
    })
    .unwrap();
    let text = String::from_utf8(raw).unwrap();
    assert!(text.contains(r#""board":"""#), "{text}");
    assert!(text.contains(r#""num":3"#), "{text}");
    let plain = gocompat::json::marshal(&TaskEditedRow::default()).unwrap();
    let plain = String::from_utf8(plain).unwrap();
    assert!(!plain.contains("board") && !plain.contains("num"), "{plain}");
}

#[test]
fn a_parent_takes_its_subtasks_along_and_a_subtask_cant_go_alone() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let p = card(&c, "", "parent");
    let k = c
        .create_task(
            ME,
            NewTask {
                title: "kid".into(),
                parent: p.id.clone(),
                ..NewTask::default()
            },
        )
        .unwrap();
    let e = c.move_task(ME, &k.id, NET).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::BadState);
    assert!(e.to_string().contains("subtask"), "{e}");
    c.move_task(ME, &p.id, NET).unwrap();
    assert_eq!(place(&c, &p.id), (NET.to_string(), "net#1".to_string()));
    assert_eq!(place(&c, &k.id), (NET.to_string(), "net#2".to_string()));
    assert_eq!(c.get_task(&k.id).unwrap().parent, p.id);
}

#[test]
fn a_move_to_no_such_board_or_where_it_is_writes_nothing() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let w = card(&c, NET, "n1");
    let e = c.move_task(ME, &w.id, "b-nope").unwrap_err();
    assert_eq!(e.kind(), ErrorKind::NotFound);
    let before = s.st.vector().unwrap();
    let t = c.move_task(ME, &w.id, NET).unwrap();
    assert_eq!(t.short_ref(), "net#1");
    assert_eq!(s.st.vector().unwrap(), before);
}

#[test]
fn the_last_move_by_place_wins_whatever_order_they_came_in() {
    let base = hlc_at(fixed_now());
    let rows = || {
        vec![
            created("dev-b", 1, base + 1, "X", 1, ""),
            moved("dev-c", 1, base + 5, "X", 1, NET),
            moved("dev-d", 1, base + 9, "X", 1, ART),
        ]
    };
    for order in [[0, 1, 2], [0, 2, 1], [2, 1, 0], [1, 2, 0]] {
        let s = space("dev-a");
        let c = coord_on(&s);
        let r = rows();
        for i in order {
            land(&s, &c, &r[i]);
        }
        assert_eq!(
            place(&c, "X"),
            (ART.to_string(), "art#1".to_string()),
            "{order:?}"
        );
    }
}

/// Devices apart claim one number: letters, as creates do. The claim that
/// loses to a later move drops out, and both boards settle.
#[test]
fn a_clash_takes_letters_and_a_lost_claim_settles_both_boards() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let base = hlc_at(fixed_now());
    for r in [
        created("dev-b", 1, base + 1, "X", 1, ""),
        created("dev-c", 1, base + 2, "Y", 1, NET),
        moved("dev-b", 2, base + 3, "X", 1, NET),
    ] {
        land(&s, &c, &r);
    }
    assert_eq!(c.get_task("Y").unwrap().short_ref(), "net#1a");
    assert_eq!(c.get_task("X").unwrap().short_ref(), "net#1b");
    land(&s, &c, &moved("dev-b", 3, base + 4, "X", 1, ART));
    assert_eq!(c.get_task("Y").unwrap().short_ref(), "net#1");
    assert_eq!(c.get_task("X").unwrap().short_ref(), "art#1");
    assert_eq!(c.task_ref("net#1").unwrap(), "Y", "who holds it comes first");
}

#[test]
fn a_move_that_comes_before_its_card_counts_once_the_card_lands() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let base = hlc_at(fixed_now());
    land(&s, &c, &moved("dev-c", 1, base + 5, "X", 4, NET));
    assert!(c.get_task("X").is_none());
    land(&s, &c, &created("dev-b", 1, base + 1, "X", 9, ""));
    assert_eq!(place(&c, "X"), (NET.to_string(), "net#4".to_string()));
    assert_eq!(c.task_ref("wind#9").unwrap(), "X");
}

#[test]
fn a_private_move_that_comes_first_is_kept_too() {
    let e = Env::new();
    let nt = |t: &str| NewTask {
        title: t.into(),
        ..NewTask::default()
    };
    let a = e.c.create_task(ALICE, nt("one")).unwrap();
    assert_eq!(a.short, "1");
    let raw = gocompat::json::marshal(&TaskEditedRow {
        task_id: "X".into(),
        board: Some("b-home".into()),
        num: 1,
        by: ALICE.into(),
        at: fixed_now(),
        ..TaskEditedRow::default()
    })
    .unwrap();
    let base = hlc_at(fixed_now());
    let mut row = event("dev-a", 101, base + 9, kinds::TASK_EDITED, raw);
    row.actor = ALICE.into();
    e.c.fold_live(&[row]);
    let made = gocompat::json::marshal(&TaskCreatedRow {
        task_id: "X".into(),
        num: 2,
        title: "x".into(),
        by: ALICE.into(),
        at: fixed_now(),
        ..TaskCreatedRow::default()
    })
    .unwrap();
    let mut row = event("dev-a", 100, base + 1, kinds::TASK_CREATED, made);
    row.actor = ALICE.into();
    e.c.fold_live(&[row]);
    assert_eq!(e.c.get_task("X").unwrap().board, "b-home");
}

#[test]
fn a_deleted_boards_cards_go_to_the_first_board_with_old_refs_working() {
    let s = space("dev-a");
    let c = coord_on(&s);
    card(&c, "", "w1");
    let n1 = card(&c, NET, "n1");
    let n2 = card(&c, NET, "n2");
    s.clk.add(std::time::Duration::from_secs(5));
    let base = hlc_at(s.clk.now());
    delete(&s, &c, "dev-b", 1, base, NET);
    s.clk.add(std::time::Duration::from_secs(5));
    assert_eq!(place(&c, &n1.id), (String::new(), "wind#2".to_string()));
    assert_eq!(place(&c, &n2.id), (String::new(), "wind#3".to_string()));
    assert_eq!(c.task_ref("net#1").unwrap(), n1.id);
    assert_eq!(c.task_ref("net#2").unwrap(), n2.id);
    assert_eq!(c.board(&BoardFilter::default()).open.len(), 3, "no card lost");
    // Numbers carry on past the ones a delete gave out, and a restart
    // gives the same.
    let w = card(&c, "", "w4");
    assert_eq!(w.short_ref(), "wind#4");
    let c = coord_on(&s);
    assert_eq!(place(&c, &n2.id), (String::new(), "wind#3".to_string()));
    assert_eq!(place(&c, &w.id), (String::new(), "wind#4".to_string()));
}

/// An older device, or one a moment behind, can still name a deleted board.
#[test]
fn a_card_or_a_move_onto_a_deleted_board_lands_on_the_first() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let base = hlc_at(fixed_now());
    land(&s, &c, &created("dev-b", 1, base + 1, "W", 1, ""));
    delete(&s, &c, "dev-c", 1, base + 2, NET);
    land(&s, &c, &created("dev-b", 2, base + 3, "N", 1, NET));
    land(&s, &c, &moved("dev-b", 3, base + 4, "W", 2, NET));
    assert_eq!(place(&c, "N"), (String::new(), "wind#2".to_string()));
    assert_eq!(place(&c, "W"), (String::new(), "wind#3".to_string()));
}

#[test]
fn a_card_cant_be_made_or_moved_onto_a_deleted_board_here() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let w = card(&c, "", "w1");
    let base = hlc_at(fixed_now()) + (1 << 30);
    delete(&s, &c, "dev-b", 1, base, NET);
    let e = c.move_task(ME, &w.id, NET).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::NotFound);
    assert!(
        c.create_task(
            ME,
            NewTask {
                title: "x".into(),
                board: NET.into(),
                ..NewTask::default()
            },
        )
        .is_err()
    );
}

fn permutations(n: usize) -> Vec<Vec<usize>> {
    if n == 0 {
        return vec![Vec::new()];
    }
    let mut out = Vec::new();
    for p in permutations(n - 1) {
        for i in 0..=p.len() {
            let mut q = p.clone();
            q.insert(i, n - 1);
            out.push(q);
        }
    }
    out
}

/// The heart of it: creates, moves and a delete from three devices, played
/// live in every order, all land where replay of the sorted rows lands.
#[test]
fn every_arrival_order_settles_on_the_same_places() {
    let base = hlc_at(fixed_now());
    let del = 4;
    let rows = vec![
        created("dev-b", 1, base + 1, "A", 1, ""),
        created("dev-c", 1, base + 2, "B", 1, NET),
        moved("dev-e", 1, base + 5, "A", 3, NET),
        moved("dev-f", 1, base + 6, "B", 1, ART),
        created("dev-g", 1, base + 7, "D", 4, NET),
    ];
    let snapshot = |c: &Coord| -> BTreeMap<String, (String, String)> {
        ["A", "B", "D"]
            .iter()
            .map(|id| (id.to_string(), place(c, id)))
            .collect()
    };
    // Row `rows.len()` stands for the delete.
    let play = |order: &[usize]| {
        let s = space("dev-a");
        let c = coord_on(&s);
        for &i in order {
            if i == rows.len() {
                delete(&s, &c, "dev-h", 1, base + del, NET);
            } else {
                land(&s, &c, &rows[i]);
            }
        }
        snapshot(&c)
    };
    let want = play(&[0, 1, 5, 2, 3, 4]);
    // The delete hands B #2; A's later move onto the deleted board lands it
    // on the first as #3, and D's create takes #4.
    assert_eq!(want["A"], (String::new(), "wind#3".to_string()));
    assert_eq!(want["B"], (ART.to_string(), "art#1".to_string()));
    assert_eq!(want["D"], (String::new(), "wind#4".to_string()));
    for order in permutations(rows.len() + 1) {
        assert_eq!(play(&order), want, "order {order:?}");
    }
}

/// A rename that changes a board's tag reaches every card on it.
#[test]
fn a_cards_tag_follows_the_board_list() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let w = card(&c, "", "w1");
    c.move_task(ME, &w.id, NET).unwrap();
    assert_eq!(c.get_task(&w.id).unwrap().tag, "net");
    s.list.lock().unwrap().tags = vec![("sockets".to_string(), NET.to_string())];
    let base = hlc_at(fixed_now()) + (1 << 30);
    land(
        &s,
        &c,
        &event(
            "dev-b",
            1,
            base,
            kinds::SPACE_BOARD_RENAMED,
            format!(r#"{{"boardId":"{NET}","name":"","tag":"sockets"}}"#).into_bytes(),
        ),
    );
    assert_eq!(c.get_task(&w.id).unwrap().tag, "sockets");
}
