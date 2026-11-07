//! Card #563: a move is an edit row with `board` and `num`, a board delete
//! is a rename row with `deleted`, and both are folded by one walk in
//! RowPos order (docs/design/no-server.md §3.3), so arrival order changes
//! nothing.
#![allow(clippy::type_complexity)]

use crate::common;

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
    st: Arc<Store>,
    clk: Clk,
    list: Arc<Mutex<BoardList>>,
    // Last: fields drop in order, and Windows can't delete an open file.
    _dir: tempfile::TempDir,
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
        finished: Vec::new(),
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
            l.tags.iter().find(|(_, b)| b == id).map(|(t, _)| t.clone())
        })),
        board_tags: Some(Arc::new(|| vec!["net".to_string(), "art".to_string()])),
        board_view: Some(Arc::new(move || Some(l2.lock().unwrap().clone()))),
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
    assert!(
        !plain.contains("board") && !plain.contains("num"),
        "{plain}"
    );
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
    assert_eq!(
        c.task_ref("net#1").unwrap(),
        "Y",
        "who holds it comes first"
    );
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
    assert_eq!(
        c.board(&BoardFilter::default()).open.len(),
        3,
        "no card lost"
    );
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
    let rows = [
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

/// Found by review: letters go with the claim, so a lettered old ref means
/// the one card that held it, and a made-up letter means nothing.
#[test]
fn a_lettered_old_ref_finds_only_the_card_that_held_it() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let base = hlc_at(fixed_now());
    land(&s, &c, &created("dev-b", 1, base + 1, "X", 5, NET));
    land(&s, &c, &created("dev-c", 1, base + 2, "Y", 5, NET));
    assert_eq!(c.task_ref("net#5a").unwrap(), "X");
    assert_eq!(c.task_ref("net#5b").unwrap(), "Y");
    land(&s, &c, &moved("dev-c", 2, base + 3, "Y", 1, ART));
    assert_eq!(c.task_ref("net#5").unwrap(), "X");
    assert_eq!(c.task_ref("net#5a").unwrap(), "X");
    assert_eq!(c.task_ref("net#5b").unwrap(), "Y");
    assert!(c.task_ref("net#5z").is_err());
}

#[test]
fn a_deleted_boards_lettered_refs_still_tell_its_cards_apart() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let base = hlc_at(fixed_now());
    land(&s, &c, &created("dev-b", 1, base + 1, "P", 1, NET));
    land(&s, &c, &created("dev-c", 1, base + 2, "Q", 1, NET));
    delete(&s, &c, "dev-d", 1, base + 3, NET);
    assert_eq!(c.task_ref("net#1a").unwrap(), "P");
    assert_eq!(c.task_ref("net#1b").unwrap(), "Q");
    let e = c.task_ref("net#1").unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Ambiguous);
    assert!(e.to_string().contains("net#1 was more than one"), "{e}");
}

/// A card made on a board already deleted keeps the ref its maker saw.
#[test]
fn a_card_made_on_a_deleted_board_keeps_the_ref_its_writer_saw() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let base = hlc_at(fixed_now());
    delete(&s, &c, "dev-b", 1, base + 1, NET);
    land(&s, &c, &created("dev-c", 1, base + 2, "Z", 7, NET));
    assert_eq!(c.get_task("Z").unwrap().short_ref(), "wind#1");
    assert_eq!(c.task_ref("net#7").unwrap(), "Z");
}

/// A family that came apart comes back together with one move, and a card
/// already on the board keeps its number.
#[test]
fn a_move_reunites_a_family_and_leaves_alone_a_card_already_there() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let p = card(&c, NET, "parent");
    let k = card(&c, "", "kid");
    c.split(ME, &p.id, Vec::new(), vec![k.id.clone()]).unwrap();
    assert_eq!(
        place(&c, &k.id),
        (NET.to_string(), "net#2".to_string()),
        "adopting puts it on the parent's board"
    );
    let base = hlc_at(fixed_now()) + (1 << 30);
    land(&s, &c, &moved("dev-b", 1, base, &k.id, 1, ART));
    assert_eq!(place(&c, &k.id).0, ART, "apart");
    let t = c.move_task(ME, &p.id, NET).unwrap();
    assert_eq!(t.short_ref(), "net#1", "the parent didn't move");
    assert_eq!(place(&c, &k.id).0, NET);

    let p2 = card(&c, NET, "parent two");
    let k2 = c
        .create_task(
            ME,
            NewTask {
                title: "kid two".into(),
                parent: p2.id.clone(),
                ..NewTask::default()
            },
        )
        .unwrap();
    land(&s, &c, &moved("dev-b", 2, base + 100_000, &k2.id, 5, ART));
    assert_eq!(c.get_task(&k2.id).unwrap().short_ref(), "art#5");
    let t = c.move_task(ME, &p2.id, ART).unwrap();
    assert_eq!(t.board, ART);
    assert_eq!(c.get_task(&k2.id).unwrap().short_ref(), "art#5", "kept");
}

/// History doesn't change at a restart when the board is deleted later,
/// and a board the log doesn't hold yet isn't guessed at.
#[test]
fn the_move_line_names_the_board_the_row_named() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let w = card(&c, "", "w1");
    c.move_task(ME, &w.id, NET).unwrap();
    s.clk.add(std::time::Duration::from_secs(5));
    delete(&s, &c, "dev-b", 1, hlc_at(s.clk.now()), NET);
    let c = coord_on(&s);
    let d = c.get_task(&w.id).unwrap();
    assert_eq!(d.short_ref(), "wind#2");
    let lines = c.activity(&w.id).unwrap();
    assert!(
        lines.iter().any(|a| a.what.contains("moved it to net")),
        "{lines:?}"
    );
    assert!(!lines.iter().any(|a| a.what.contains("moved it to wind")));
}

/// A card's old ref in text reaches it, as task_ref does.
#[test]
fn a_mention_of_an_old_ref_reaches_the_moved_card() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let w = card(&c, "", "w1");
    let other = card(&c, "", "w2");
    c.move_task(ME, &w.id, NET).unwrap();
    c.add_comment(ME, &other.id, "see wind#1", false).unwrap();
    let lines = c.activity(&w.id).unwrap();
    assert!(
        lines.iter().any(|a| a.what.contains("mentioned this in")),
        "{lines:?}"
    );
}

/// A failed read keeps the list: a blank one would undo every delete.
#[test]
fn a_board_list_that_cant_be_read_changes_nothing() {
    let s = space("dev-a");
    let dark = Arc::new(Mutex::new(false));
    let (d2, l2) = (dark.clone(), s.list.clone());
    let c = Coord::new(
        s.st.clone(),
        None,
        Options {
            board_view: Some(Arc::new(move || {
                if *d2.lock().unwrap() {
                    return None;
                }
                Some(l2.lock().unwrap().clone())
            })),
            ..opts(&s)
        },
    )
    .unwrap();
    let n = card(&c, NET, "n1");
    s.clk.add(std::time::Duration::from_secs(5));
    delete(&s, &c, "dev-b", 1, hlc_at(s.clk.now()), NET);
    assert_eq!(place(&c, &n.id).0, "");
    *dark.lock().unwrap() = true;
    land(
        &s,
        &c,
        &event(
            "dev-c",
            1,
            hlc_at(s.clk.now()) + 1,
            kinds::SPACE_BOARD_RENAMED,
            format!(r#"{{"boardId":"{ART}","name":"Paint","tag":""}}"#).into_bytes(),
        ),
    );
    assert_eq!(place(&c, &n.id).0, "", "still on the first board");
}

/// A one-run ref (`#5a`, card #558) still means the card it meant after
/// that card moves: the letters go by each card's create claim.
#[test]
fn a_one_run_ref_still_finds_its_card_after_a_move() {
    let s = space("dev-a");
    let c = coord_on(&s);
    land(&s, &c, &created("dev-b", 1, 10, "x", 5, ""));
    land(&s, &c, &created("dev-b", 2, 20, "y", 5, NET));
    assert_eq!(c.task_ref("#5a").unwrap(), "x");
    assert_eq!(c.task_ref("#5b").unwrap(), "y");
    land(&s, &c, &moved("dev-b", 3, 30, "x", 6, NET));
    assert_eq!(place(&c, "x"), (NET.to_string(), "net#6".to_string()));
    assert_eq!(c.task_ref("#5a").unwrap(), "x");
    assert_eq!(c.task_ref("#5b").unwrap(), "y");
}

/// A delete that only a catch-up folds (it landed between the replay and
/// the subscription) still sends the board's cards to the first board.
#[test]
fn a_delete_folded_by_a_catch_up_moves_the_cards() {
    let s = space("dev-a");
    let c = coord_on(&s);
    land(&s, &c, &created("dev-b", 1, 10, "n", 1, NET));
    s.list
        .lock()
        .unwrap()
        .deleted
        .push((NET.into(), pos("dev-b", 2, 20)));
    let raw = format!(r#"{{"boardId":"{NET}","name":"","tag":"","deleted":true}}"#);
    s.st.insert(&event(
        "dev-b",
        2,
        20,
        kinds::SPACE_BOARD_RENAMED,
        raw.into_bytes(),
    ))
    .unwrap();
    c.catch_up().unwrap();
    assert_eq!(place(&c, "n"), (String::new(), "wind#1".to_string()));
    assert_eq!(c.task_ref("net#1").unwrap(), "n");
}

/// Finishes `board` (or reopens it) as if its row had just landed.
fn set_finished(s: &Space, c: &Coord, seq: i64, at: u64, board: &str, finished: bool) {
    {
        let mut l = s.list.lock().unwrap();
        l.finished.retain(|b| b != board);
        if finished {
            l.finished.push(board.into());
        }
    }
    let raw = format!(r#"{{"boardId":"{board}","finished":{finished}}}"#);
    land(
        s,
        c,
        &event(
            "dev-b",
            seq,
            at,
            kinds::SPACE_BOARD_FINISHED,
            raw.into_bytes(),
        ),
    );
}

/// Card #690: a finished board is read-only on this device (no new card,
/// no move onto it, none of its cards reopens), its cards stay where they
/// are with their refs, and a card another device put there still folds.
#[test]
fn a_finished_board_takes_no_card_here_and_keeps_its_own() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let n1 = card(&c, NET, "n1");
    let n2 = card(&c, NET, "n2");
    let w = card(&c, "", "w1");
    let names = |ts: Vec<Task>| {
        let mut v = ts.into_iter().map(|t| t.title).collect::<Vec<_>>();
        v.sort();
        v
    };
    assert_eq!(names(c.open_on_board(NET)), ["n1", "n2"]);
    c.set_task_state(ME, &n1.id, "completed", "").unwrap();
    c.set_task_state(ME, &n2.id, "canceled", "").unwrap();
    assert!(c.open_on_board(NET).is_empty());
    assert_eq!(names(c.open_on_board("")), ["w1"]);
    assert_eq!(
        names(c.open_on_board("b")),
        ["w1"],
        "the first board by its id"
    );

    set_finished(&s, &c, 1, 100, NET, true);
    let e = c
        .create_task(
            ME,
            NewTask {
                title: "late".into(),
                board: NET.into(),
                ..NewTask::default()
            },
        )
        .unwrap_err();
    assert!(e.to_string().contains("board \"net\" is finished"), "{e}");
    let e = c.move_task(ME, &w.id, NET).unwrap_err();
    assert!(e.to_string().contains("is finished"), "{e}");
    let e = c.reopen(ME, &n1.id).unwrap_err();
    assert!(e.to_string().contains("is finished"), "{e}");
    // Its cards keep their place and refs, and other boards are as before.
    assert_eq!(place(&c, &n1.id), (NET.to_string(), "net#1".to_string()));
    assert_eq!(card(&c, ART, "a1").short_ref(), "art#1");
    // An older device's card onto it still folds: the check is a writer's.
    land(&s, &c, &created("dev-b", 2, 200, "t-old", 3, NET));
    assert_eq!(place(&c, "t-old"), (NET.to_string(), "net#3".to_string()));
    // Nothing is adopted under a card there either.
    let e = c
        .split(ME, "t-old", Vec::new(), vec![w.id.clone()])
        .unwrap_err();
    assert!(e.to_string().contains("is finished"), "{e}");
    // A restart reads the same, and a reopen opens it up again.
    let c = coord_on(&s);
    assert!(c.move_task(ME, &w.id, NET).is_err());
    set_finished(&s, &c, 3, 300, NET, false);
    c.reopen(ME, &n1.id).unwrap();
    assert_eq!(c.move_task(ME, &w.id, NET).unwrap().board, NET);
}
