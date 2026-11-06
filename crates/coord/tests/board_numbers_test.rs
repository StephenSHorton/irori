//! Card #558: each board in a space has its own number run. Numbers are
//! claimed in `task.created.num`, so no existing row changes; the fold keys
//! its claims on (board, number).
#![allow(clippy::type_complexity)]

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use coord::rows::TaskCreatedRow;
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
    Space {
        _dir: dir,
        st: Arc::new(st),
        clk,
    }
}

fn opts(s: &Space) -> Options {
    Options {
        now: Some(s.clk.func()),
        board: true,
        person: ME.into(),
        is_owner: Some(Arc::new(|k: &str| k == ME)),
        tag: Some(Arc::new(|| "wind".to_string())),
        board_tag: Some(Arc::new(|id: &str| match id {
            NET => Some("net".to_string()),
            ART => Some("art".to_string()),
            _ => None,
        })),
        board_tags: Some(Arc::new(|| vec!["net".to_string(), "art".to_string()])),
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

fn claimed(dev: &str, seq: i64, at: u64, id: &str, num: i64, board: &str) -> Event {
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
    Event {
        device_id: dev.into(),
        seq,
        hlc: hlc::Timestamp(at),
        actor: ME.into(),
        kind: kinds::TASK_CREATED.into(),
        payload: raw,
        sig: Vec::new(),
    }
}

fn short_ref(c: &Coord, id: &str) -> String {
    c.get_task(id).unwrap().short_ref()
}

#[test]
fn each_board_counts_from_one_and_carries_on_from_its_highest() {
    let s = space("dev-a");
    let c = coord_on(&s);
    assert_eq!(card(&c, "", "w1").short_ref(), "wind#1");
    assert_eq!(card(&c, "", "w2").short_ref(), "wind#2");
    assert_eq!(card(&c, NET, "n1").short_ref(), "net#1");
    assert_eq!(card(&c, "", "w3").short_ref(), "wind#3");
    assert_eq!(card(&c, NET, "n2").short_ref(), "net#2");
    // A restart folds the same runs and carries on from them.
    let c = coord_on(&s);
    assert_eq!(card(&c, NET, "n3").short_ref(), "net#3");
    assert_eq!(card(&c, ART, "a1").short_ref(), "art#1");
    assert_eq!(card(&c, "", "w4").short_ref(), "wind#4");
}

/// A row that names a board carries on from that board's highest claim,
/// not one above every board's.
#[test]
fn a_board_with_claims_carries_on_from_its_own_highest() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let base = hlc_at(fixed_now());
    c.fold_live(&[
        claimed("dev-b", 1, base, "N7", 7, NET),
        claimed("dev-b", 2, base + 1, "W2", 2, ""),
    ]);
    assert_eq!(card(&c, NET, "n").short_ref(), "net#8");
    assert_eq!(card(&c, "", "w").short_ref(), "wind#3");
    assert_eq!(card(&c, ART, "a").short_ref(), "art#1");
}

#[test]
fn a_tagged_ref_finds_its_board_and_a_bare_one_must_be_unique() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let w1 = card(&c, "", "the wind one");
    let w2 = card(&c, "", "only on wind");
    let n1 = card(&c, NET, "the net one");
    assert_eq!(c.task_ref("wind#1").unwrap(), w1.id);
    assert_eq!(c.task_ref("net#1").unwrap(), n1.id);
    assert_eq!(c.task_ref("#2").unwrap(), w2.id, "only wind has a #2");
    assert_eq!(c.task_ref("2").unwrap(), w2.id);
    assert!(c.task_ref("net#2").is_err(), "net has no #2");
    let e = c.task_ref("#1").unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Ambiguous);
    let text = e.to_string();
    for want in ["wind#1", "net#1", "the wind one", "the net one", &n1.id] {
        assert!(text.contains(want), "{want} in {text}");
    }
}

/// Search's show reads a bare number the same way.
#[test]
fn show_finds_a_unique_number_and_names_a_shared_one() {
    use search::{ShowOptions, Source};
    let s = space("dev-a");
    let c = coord_on(&s);
    card(&c, "", "on wind");
    card(&c, "", "second");
    card(&c, NET, "on net");
    let o = ShowOptions {
        limit: 100,
        ..ShowOptions::default()
    };
    assert!(
        c.search_show("task:#2", &o)
            .unwrap()
            .text
            .contains("second")
    );
    let e = c.search_show("task:#1", &o).unwrap_err().to_string();
    assert!(e.contains("wind#1") && e.contains("net#1"), "{e}");
}

/// Devices apart that claim one number on one board still get letters, and
/// that board's letters don't touch another board's number.
#[test]
fn a_clash_on_one_board_keeps_its_letters() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let base = hlc_at(fixed_now());
    c.fold_live(&[
        claimed("dev-b", 1, base + 1, "B", 5, NET),
        claimed("dev-c", 1, base + 2, "C", 5, NET),
        claimed("dev-d", 1, base + 3, "D", 5, ""),
    ]);
    assert_eq!(short_ref(&c, "B"), "net#5a");
    assert_eq!(short_ref(&c, "C"), "net#5b");
    assert_eq!(short_ref(&c, "D"), "wind#5");
    assert_eq!(c.task_ref("wind#5").unwrap(), "D");
    assert_eq!(c.task_ref("net#5b").unwrap(), "C");
    assert_eq!(c.task_ref("#5b").unwrap(), "C");
    let e = c.task_ref("net#5").unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Ambiguous);
    assert!(e.to_string().contains("devices that were apart"), "{e}");
    assert!(e.to_string().contains("claimed net#5"), "{e}");
    // A bare #5 could be any of the three, though wind's alone is unlettered.
    let e = c.task_ref("#5").unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Ambiguous);
    for want in ["net#5a", "net#5b", "wind#5"] {
        assert!(e.to_string().contains(want), "{want} in {e}");
    }
}

/// A board with no cards has its tag still: the ref isn't "on another
/// board", it's a card the board doesn't have.
#[test]
fn a_board_with_no_cards_keeps_its_tag() {
    let s = space("dev-a");
    let c = coord_on(&s);
    card(&c, "", "w");
    let e = c.task_ref("net#4").unwrap_err();
    assert_eq!(e.kind(), ErrorKind::NotFound);
    assert!(!e.to_string().contains("another board"), "{e}");
    let e = c.task_ref("nope#4").unwrap_err();
    assert!(e.to_string().contains("another board"), "{e}");
}

/// A pre-#558 log: the space numbered its boards in one run, and two
/// devices apart gave two boards one number. Folding it now leaves every
/// card's ref as it was, but for the letter a shared number no longer needs.
#[test]
fn a_log_from_before_keeps_every_ref() {
    let s = space("dev-a");
    let write = |id: &str, num: i64, board: &str| {
        s.st.append(
            ME,
            kinds::TASK_CREATED,
            &TaskCreatedRow {
                task_id: id.into(),
                num,
                title: id.into(),
                board: board.into(),
                by: ME.into(),
                at: fixed_now(),
                ..TaskCreatedRow::default()
            },
        )
        .unwrap();
    };
    write("A", 1, "");
    write("B", 2, NET);
    write("C", 3, ART);
    write("D", 4, "");
    write("E", 5, NET);
    // Claimed apart: both boards' next was #6.
    write("F", 6, "");
    write("G", 6, NET);
    let c = coord_on(&s);
    for (id, want) in [
        ("A", "wind#1"),
        ("B", "net#2"),
        ("C", "art#3"),
        ("D", "wind#4"),
        ("E", "net#5"),
        // These were wind#6a and net#6b.
        ("F", "wind#6"),
        ("G", "net#6"),
    ] {
        assert_eq!(short_ref(&c, id), want, "{id}");
        assert_eq!(c.task_ref(want).unwrap(), id, "{want}");
        assert_eq!(c.get_task(id).unwrap().short_was, "", "{id}");
    }
    // Their old lettered refs, in comments and agents' memory, still find
    // them; a device on an older Irori shows them yet.
    assert_eq!(c.task_ref("#6a").unwrap(), "F");
    assert_eq!(c.task_ref("#6b").unwrap(), "G");
    assert_eq!(c.task_ref("wind#6a").unwrap(), "F");
    assert_eq!(c.task_ref("net#6b").unwrap(), "G");
    assert_eq!(
        c.task_ref("net#6a").unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert_eq!(c.task_ref("#6c").unwrap_err().kind(), ErrorKind::NotFound);
    // Each board carries on from its own highest.
    assert_eq!(card(&c, ART, "a").short_ref(), "art#4");
    assert_eq!(card(&c, NET, "n").short_ref(), "net#7");
    assert_eq!(card(&c, "", "w").short_ref(), "wind#7");
    // The rows themselves are as they were written.
    let f = store::Filter {
        kinds: vec![kinds::TASK_CREATED.into()],
        limit: 100,
        ..Default::default()
    };
    let nums: Vec<i64> =
        s.st.list(&f)
            .unwrap()
            .iter()
            .map(|ev| {
                serde_json::from_slice::<serde_json::Value>(&ev.payload).unwrap()["num"]
                    .as_i64()
                    .unwrap()
            })
            .collect();
    assert_eq!(nums, [1, 2, 3, 4, 5, 6, 6, 4, 7, 7]);
}

/// A board's run after a cut: every map folds again from the rows that are
/// left, and the number a cut card held on the board isn't given out again.
#[test]
fn a_refold_rebuilds_each_boards_refs() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let w = card(&c, "", "w");
    let n = card(&c, NET, "n");
    let base = hlc_at(fixed_now());
    c.fold_live(&[
        claimed("dev-z", 1, base - (1 << 16), "Z1", 1, NET),
        claimed("dev-z", 2, base + (1 << 16), "Z2", 2, NET),
    ]);
    assert_eq!(short_ref(&c, &n.id), "net#1b");
    let vec = s.st.vector().unwrap();
    c.handle(&claimed("dev-y", 1, base, "Y", 1, ART)).unwrap();
    assert_eq!(short_ref(&c, "Y"), "art#1");
    c.refold_if_cut(&vec).unwrap();
    assert!(c.get_task("Z1").is_none());
    assert!(c.get_task("Y").is_none());
    assert_eq!(short_ref(&c, &n.id), "net#1");
    assert_eq!(c.task_ref("net#1").unwrap(), n.id);
    assert_eq!(c.task_ref("#1").unwrap_err().kind(), ErrorKind::Ambiguous);
    assert!(c.task_ref("art#1").is_err(), "cut away");
    assert_eq!(short_ref(&c, &w.id), "wind#1");
    assert_eq!(card(&c, NET, "next").short_ref(), "net#3");
}

/// A row that names a board this device hasn't heard of yet shows the
/// space's tag for now, but it claims on its own board: the number is the
/// same on every device, whichever row came first.
#[test]
fn a_row_ahead_of_its_board_numbers_on_that_board() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let base = hlc_at(fixed_now());
    c.fold_live(&[
        claimed("dev-b", 1, base, "W", 1, ""),
        claimed("dev-b", 2, base + 1, "U", 1, "b-unheard"),
    ]);
    assert_eq!(c.get_task("W").unwrap().short, "1");
    assert_eq!(c.get_task("U").unwrap().short, "1");
    assert_eq!(
        c.task_ref("wind#1").unwrap_err().kind(),
        ErrorKind::Ambiguous
    );
}

fn board_row(dev: &str, seq: i64, at: u64, kind: &str, board: &str, tag: &str) -> Event {
    let raw = format!(r#"{{"boardId":"{board}","name":"n","tag":"{tag}","by":"{ME}","at":""}}"#);
    Event {
        device_id: dev.into(),
        seq,
        hlc: hlc::Timestamp(at),
        actor: ME.into(),
        kind: kind.into(),
        payload: raw.into_bytes(),
        sig: Vec::new(),
    }
}

type Tags = Arc<Mutex<Vec<(String, String)>>>;

/// `opts` with boards that arrive during the test, as the log's rows do.
fn late_boards(s: &Space, tags: &Tags) -> Options {
    let (a, b) = (tags.clone(), tags.clone());
    Options {
        board_tag: Some(Arc::new(move |id: &str| {
            let tags = a.lock().unwrap();
            tags.iter().find(|(b, _)| b == id).map(|(_, t)| t.clone())
        })),
        board_tags: Some(Arc::new(move || {
            b.lock().unwrap().iter().map(|(_, t)| t.clone()).collect()
        })),
        ..opts(s)
    }
}

/// B's rows reach this device before A's `space.board.created`: the card
/// shows the space's tag until the board's row lands, then the board's, the
/// same as a device that heard them in order (and with no restart).
#[test]
fn a_cards_tag_follows_a_board_row_that_lands_after_it() {
    let s = space("dev-c");
    let tags: Tags = Arc::default();
    let c = Coord::new(s.st.clone(), None, late_boards(&s, &tags)).unwrap();
    let base = hlc_at(fixed_now());
    c.fold_live(&[
        claimed("dev-b", 1, base, "W", 3, ""),
        claimed("dev-b", 2, base + 1, "L", 3, "b-late"),
    ]);
    assert_eq!(short_ref(&c, "L"), "wind#3");
    let changes = c.subscribe();
    tags.lock()
        .unwrap()
        .push(("b-late".to_string(), "late".to_string()));
    c.handle(&board_row(
        "dev-a",
        1,
        base + 2,
        kinds::SPACE_BOARD_CREATED,
        "b-late",
        "late",
    ))
    .unwrap();
    assert_eq!(short_ref(&c, "L"), "late#3");
    assert_eq!(short_ref(&c, "W"), "wind#3", "the space's own card stays");
    assert_eq!(c.task_ref("late#3").unwrap(), "L");
    assert_eq!(c.task_ref("wind#3").unwrap(), "W");
    let got = changes.recv_timeout(std::time::Duration::from_secs(5));
    assert_eq!(got.unwrap().unwrap().task_id, "L", "the window hears it");
}

/// A row with no such board, or the space's first board, moves nothing.
#[test]
fn a_board_row_leaves_other_boards_cards_alone() {
    let s = space("dev-a");
    let c = coord_on(&s);
    let n = card(&c, NET, "n");
    let w = card(&c, "", "w");
    let base = hlc_at(fixed_now());
    c.handle(&board_row(
        "dev-b",
        1,
        base,
        kinds::SPACE_BOARD_RENAMED,
        "b",
        "",
    ))
    .unwrap();
    c.handle(&board_row(
        "dev-b",
        2,
        base + 1,
        kinds::SPACE_BOARD_CREATED,
        ART,
        "art",
    ))
    .unwrap();
    assert_eq!(short_ref(&c, &n.id), "net#1");
    assert_eq!(short_ref(&c, &w.id), "wind#1");
}

/// A resync's catch-up reads `space.board.*` rows too: nothing else tells
/// the fold they arrived. The same goes for Personal's log.
#[test]
fn a_catch_up_restamps_too() {
    let mut e = Env::new();
    let tags: Tags = Arc::default();
    let (a, b) = (tags.clone(), tags.clone());
    e.c = Coord::new(
        e.st.clone(),
        None,
        Options {
            board_tag: Some(Arc::new(move |id: &str| {
                let tags = a.lock().unwrap();
                tags.iter().find(|(b, _)| b == id).map(|(_, t)| t.clone())
            })),
            board_tags: Some(Arc::new(move || {
                b.lock().unwrap().iter().map(|(_, t)| t.clone()).collect()
            })),
            ..e.opts()
        },
    )
    .unwrap();
    let base = hlc_at(fixed_now());
    let by = "user:dev-b";
    let row = TaskCreatedRow {
        task_id: "L".into(),
        num: 3,
        title: "L".into(),
        board: "b-late".into(),
        by: by.into(),
        at: fixed_now(),
        ..TaskCreatedRow::default()
    };
    e.insert("dev-b", 1, base, by, kinds::TASK_CREATED, &row);
    e.c.catch_up().unwrap();
    assert_eq!(short_ref(&e.c, "L"), "#3");
    let board =
        serde_json::json!({"boardId": "b-late", "name": "n", "tag": "late", "by": by, "at": ""});
    e.insert("dev-b", 2, base + 1, by, kinds::SPACE_BOARD_CREATED, &board);
    tags.lock()
        .unwrap()
        .push(("b-late".to_string(), "late".to_string()));
    e.c.catch_up().unwrap();
    assert_eq!(short_ref(&e.c, "L"), "late#3");
}

/// Personal: a log with a `home` board beside its default board.
fn personal() -> Env {
    let mut e = Env::with("dev-a", Some(Arc::new(|k: &str| k == USER)));
    e.c = home_coord(&e);
    e
}

fn home_coord(e: &Env) -> Coord {
    Coord::new(
        e.st.clone(),
        None,
        Options {
            board_tag: Some(Arc::new(|id: &str| {
                (id == "b-home").then(|| "home".to_string())
            })),
            board_tags: Some(Arc::new(|| vec!["home".to_string()])),
            ..e.opts()
        },
    )
    .unwrap()
}

fn private(c: &Coord, board: &str, title: &str) -> Task {
    c.create_task(
        USER,
        NewTask {
            title: title.into(),
            board: board.into(),
            ..NewTask::default()
        },
    )
    .unwrap()
}

/// Default-first holds for a deleted card too: a stale `#N` is refused, not
/// passed to `home#N`, and that survives a restart.
#[test]
fn a_deleted_default_card_keeps_its_bare_number() {
    let mut e = personal();
    let gone = private(&e.c, "", "deleted");
    let home = private(&e.c, "b-home", "live");
    assert_eq!(
        (gone.short_ref().as_str(), home.short_ref().as_str()),
        ("#1", "home#1")
    );
    e.c.delete_task(USER, &gone.id).unwrap();
    for _ in 0..2 {
        let err = e.c.task_ref("#1").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(err.to_string().contains("#1 was deleted"), "{err}");
        assert_eq!(e.c.task_ref("home#1").unwrap(), home.id);
        e.c = home_coord(&e);
    }
}

/// A deleted card names only its own board's ref.
#[test]
fn a_tagged_ref_isnt_another_boards_deleted_card() {
    let e = personal();
    let gone = private(&e.c, "", "deleted");
    e.c.delete_task(USER, &gone.id).unwrap();
    let err = e.c.task_ref("home#1").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(!err.to_string().contains("deleted"), "{err}");
    // Deleted on `home`, the tagged ref says so; a bare one means the board
    // that has it, since the default board never had #1 here.
    let e = personal();
    let gone = private(&e.c, "b-home", "deleted");
    e.c.delete_task(USER, &gone.id).unwrap();
    assert!(
        e.c.task_ref("home#1")
            .unwrap_err()
            .to_string()
            .contains("was deleted")
    );
    assert!(
        e.c.task_ref("#1")
            .unwrap_err()
            .to_string()
            .contains("was deleted")
    );
}

/// A new thread reaches the agents on the card it means: `#1` is the default
/// board's, whose number `home#1` shares.
#[test]
fn a_thread_saying_a_bare_number_reaches_the_default_boards_holders() {
    let e = personal();
    let d = private(&e.c, "", "default");
    let h = private(&e.c, "b-home", "home");
    e.c.claim_task(ALICE, &d.id).unwrap();
    e.c.claim_task(BOB, &h.id).unwrap();
    assert_eq!(e.c.thread_hearers("about #1", &[]), [ALICE]);
    assert_eq!(e.c.thread_hearers("about home#1", &[]), [BOB]);
}

/// The default board's live card still comes first over `home`'s.
#[test]
fn a_live_default_card_still_means_a_bare_number() {
    let e = personal();
    let d = private(&e.c, "", "default");
    private(&e.c, "b-home", "home");
    assert_eq!(e.c.task_ref("#1").unwrap(), d.id);
}

/// A hidden space has no boards, so a card has nowhere to go.
#[test]
fn a_hidden_space_takes_no_card() {
    let s = space("dev-a");
    let c = Coord::new(
        s.st.clone(),
        None,
        Options {
            no_cards: Some(Arc::new(|| true)),
            ..opts(&s)
        },
    )
    .unwrap();
    let e = c
        .create_task(
            ME,
            NewTask {
                title: "x".into(),
                ..NewTask::default()
            },
        )
        .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::NotAllowed);
    assert!(e.to_string().contains("no boards"), "{e}");
}

/// A private log has no boards in it: one run, as it always was.
#[test]
fn a_private_log_is_one_run() {
    let e = Env::new();
    let nt = |t: &str| NewTask {
        title: t.into(),
        ..NewTask::default()
    };
    let a = e.c.create_task(ALICE, nt("one")).unwrap();
    let b = e.c.create_task(ALICE, nt("two")).unwrap();
    assert_eq!((a.short.as_str(), b.short.as_str()), ("1", "2"));
    assert_eq!(e.c.task_ref("#2").unwrap(), b.id);
}
