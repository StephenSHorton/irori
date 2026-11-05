//! Card #558: Personal is a space with its own boards, kept in the private
//! log. Its default board is implicit, so every private card stays on it.

mod common;

use common::{Rig, options};
use coord::ErrorKind as CK;
use irori_core::{BoardError, Core, PERSONAL_BOARD};
use store::{Filter, kinds};

fn card(r: &Rig, board: &str, title: &str) -> coord::Task {
    r.core
        .coord()
        .create_task(
            &r.core.user_sender().key,
            coord::NewTask {
                title: title.into(),
                board: board.into(),
                ..Default::default()
            },
        )
        .unwrap()
}

fn names(c: &Core) -> Vec<String> {
    c.personal_boards()
        .unwrap()
        .into_iter()
        .map(|b| format!("{}:{}", b.tag, b.name))
        .collect()
}

/// Personal has one board before anyone adds one, and its rows are in the
/// private log with no gate to pass.
#[test]
fn boards_go_in_the_private_log() {
    let r = Rig::new();
    assert_eq!(names(&r.core), [":Personal"]);
    r.core.create_personal_board("Home jobs", "home").unwrap();
    assert_eq!(names(&r.core), [":Personal", "home:Home jobs"]);
    let rows = r
        .core
        .store()
        .list(&Filter {
            kinds: vec![kinds::SPACE_BOARD_CREATED.into()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].actor, "user");
}

/// The sentinel renames the default board, whose tag stays empty: its
/// cards' refs don't move. Another board's rename keeps its tag too.
#[test]
fn the_default_board_renames_through_the_sentinel() {
    let r = Rig::new();
    let old = card(&r, "", "already here");
    let home = r.core.create_personal_board("Home jobs", "home").unwrap();
    r.core
        .rename_personal_board(PERSONAL_BOARD, "Inbox")
        .unwrap();
    r.core.rename_personal_board(&home, "House").unwrap();
    assert_eq!(names(&r.core), [":Inbox", "home:House"]);
    assert_eq!(r.core.task_ref(&old.short_ref()).unwrap(), old.id);

    let e = r.core.rename_personal_board("nope", "x").unwrap_err();
    assert!(matches!(&e, BoardError::Coord(c) if c.kind() == CK::NotFound));
    let e = r
        .core
        .rename_personal_board(PERSONAL_BOARD, " ")
        .unwrap_err();
    assert!(matches!(&e, BoardError::Coord(c) if c.kind() == CK::Invalid));
}

/// Each board counts from 1, and the default board carries on from the
/// highest number it has, whatever the others hold.
#[test]
fn each_personal_board_has_its_own_number_run() {
    let r = Rig::new();
    let (a, b) = (card(&r, "", "one"), card(&r, "", "two"));
    assert_eq!(
        (a.short_ref().as_str(), b.short_ref().as_str()),
        ("#1", "#2")
    );
    let home = r.core.create_personal_board("Home jobs", "home").unwrap();
    let yard = r.core.create_personal_board("Yard", "yard").unwrap();
    assert_eq!(card(&r, &home, "fence").short_ref(), "home#1");
    assert_eq!(card(&r, &home, "gate").short_ref(), "home#2");
    assert_eq!(card(&r, &yard, "weeds").short_ref(), "yard#1");
    assert_eq!(card(&r, "", "three").short_ref(), "#3");
}

/// A tagged ref finds its card through the Core, and a tag no board holds
/// says so.
#[test]
fn a_tagged_ref_resolves_through_task_ref() {
    let r = Rig::new();
    let home = r.core.create_personal_board("Home jobs", "home").unwrap();
    let c = card(&r, &home, "fence");
    assert_eq!(r.core.task_ref("home#1").unwrap(), c.id);
    assert_eq!(r.core.task_ref("HOME#1").unwrap(), c.id);
    assert_eq!(r.core.route(&c.id).unwrap().0, "", "it is a private card");
    assert_eq!(r.core.task_ref("home#2").unwrap_err().kind(), CK::NotFound);
    assert_eq!(r.core.task_ref("yard#1").unwrap_err().kind(), CK::NotFound);
}

/// A board with no cards still has its tag: its ref isn't "on another
/// board", as a tag this log never named is.
#[test]
fn an_empty_personal_board_keeps_its_tag() {
    let r = Rig::new();
    r.core.create_personal_board("Home jobs", "home").unwrap();
    let e = r.core.coord().task_ref("home#1").unwrap_err();
    assert!(!e.to_string().contains("another board"), "{e}");
    let e = r.core.coord().task_ref("yard#1").unwrap_err();
    assert!(e.to_string().contains("another board"), "{e}");
}

/// A default-board card's whole ref is `#N`, so another board having the
/// number can't take it away. With none there, a bare number finds the one
/// card that has it, and two boards' are ambiguous.
#[test]
fn a_bare_number_means_the_default_board_first() {
    let r = Rig::new();
    let old = card(&r, "", "already here");
    let home = r.core.create_personal_board("Home jobs", "home").unwrap();
    let yard = r.core.create_personal_board("Yard", "yard").unwrap();
    let fence = card(&r, &home, "fence");
    assert_eq!(r.core.task_ref("#1").unwrap(), old.id);
    assert_eq!(r.core.task_ref("home#1").unwrap(), fence.id);

    card(&r, &home, "gate");
    assert_eq!(
        r.core.task_ref("#2").unwrap(),
        r.core.task_ref("home#2").unwrap()
    );
    card(&r, &yard, "weeds");
    card(&r, &yard, "hedge");
    let e = r.core.task_ref("#2").unwrap_err();
    assert_eq!(e.kind(), CK::Ambiguous);
    assert!(
        e.to_string().contains("home#2") && e.to_string().contains("yard#2"),
        "{e}"
    );
}

/// A subtask stays on its parent's Personal board, with that board's next
/// number.
#[test]
fn a_subtask_stays_on_its_parents_personal_board() {
    let r = Rig::new();
    let home = r.core.create_personal_board("Home jobs", "home").unwrap();
    let parent = card(&r, &home, "fence");
    let kid = r
        .core
        .coord()
        .create_task(
            &r.core.user_sender().key,
            coord::NewTask {
                title: "posts".into(),
                parent: parent.id,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(kid.short_ref(), "home#2");
}

/// A tag is one board's among Personal and the spaces, either way round.
#[test]
fn personal_and_spaces_share_one_set_of_tags() {
    let r = Rig::new();
    r.core.create_personal_board("Home jobs", "home").unwrap();
    let e = r.core.create_board("Household", "home").unwrap_err();
    assert!(e.to_string().contains("already board"), "{e}");

    let id = r.core.create_board("Wind", "wind").unwrap();
    r.core.create_space_board(&id, "Netcode", "net").unwrap();
    for tag in ["wind", "net", "home"] {
        let e = r.core.create_personal_board("Mine", tag).unwrap_err();
        assert!(e.to_string().contains("already board"), "{tag}: {e}");
    }
    let e = r
        .core
        .create_personal_board("Mine", "Not A Tag")
        .unwrap_err();
    assert!(matches!(&e, BoardError::Coord(c) if c.kind() == CK::Invalid));
}

/// An id the log holds no board for is refused on the way in; the sentinel
/// is never a card's board.
#[test]
fn a_card_goes_only_on_a_board_personal_has() {
    let r = Rig::new();
    for id in ["nope", PERSONAL_BOARD] {
        let e = r
            .core
            .coord()
            .create_task(
                &r.core.user_sender().key,
                coord::NewTask {
                    title: "x".into(),
                    board: id.into(),
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert_eq!(e.kind(), CK::Invalid, "{id}");
    }
}

/// Reopening reads every board, ref and number back from the log, with the
/// cards from before any board where they were.
#[test]
fn boards_and_refs_survive_a_reopen() {
    let r = Rig::new();
    let old = card(&r, "", "already here");
    let home = r.core.create_personal_board("Home jobs", "home").unwrap();
    let fence = card(&r, &home, "fence");
    r.core
        .rename_personal_board(PERSONAL_BOARD, "Inbox")
        .unwrap();
    r.core.close().unwrap();
    let c = Core::start(options(&r.data, &r.claude)).unwrap();
    assert_eq!(names(&c), [":Inbox", "home:Home jobs"]);
    assert_eq!(c.task_ref("home#1").unwrap(), fence.id);
    assert_eq!(c.task_ref("#1").unwrap(), old.id);
    let gate = c
        .coord()
        .create_task(
            &c.user_sender().key,
            coord::NewTask {
                title: "gate".into(),
                board: home,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(gate.short_ref(), "home#2");
    c.close().unwrap();
}
