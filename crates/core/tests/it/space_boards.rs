//! Card #558: a space is a group of people, a board a list of tasks. The
//! boards inside a space through the Core's public calls, on one device.

use crate::common;

use common::Rig;
use coord::ErrorKind as CK;
use irori_core::{BoardError, Core};

fn new_task(title: &str) -> coord::NewTask {
    coord::NewTask {
        title: title.into(),
        ..Default::default()
    }
}

/// A space `wind` holding one more board, `net`: (space id, net's id).
fn wind_and_net(c: &Core) -> (String, String) {
    let id = c.create_board("Wind Waker Multiplayer", "wind").unwrap();
    let net = c.create_space_board(&id, "Netcode", "net").unwrap();
    (id, net)
}

fn on_board(r: &Rig, space: &str, board: &str, title: &str) -> coord::Task {
    let co = r.core.board_coord(space).unwrap();
    let by = r.core.board_key(space, &r.key_a).unwrap();
    co.create_task(
        &by,
        coord::NewTask {
            board: board.into(),
            ..new_task(title)
        },
    )
    .unwrap()
}

/// A new space can't take a tag a board inside another space holds.
#[test]
fn a_new_space_cant_take_an_inner_boards_tag() {
    let r = Rig::new();
    wind_and_net(&r.core);
    let e = r.core.create_board("Network", "net").unwrap_err();
    assert!(e.to_string().contains("already board"), "{e}");
    assert!(matches!(&e, BoardError::Coord(c) if c.kind() == CK::Invalid));
}

fn names(c: &Core, id: &str) -> Vec<String> {
    c.space_boards(id)
        .unwrap()
        .into_iter()
        .map(|b| format!("{}:{}", b.tag, b.name))
        .collect()
}

fn space_name(c: &Core, id: &str) -> String {
    let s = c.board_list().into_iter().find(|s| s.board_id == id);
    s.unwrap().name
}

/// A board's rename changes only its name: its tag and every ref stay, and
/// it takes no check of the tag, which it's already holding.
#[test]
fn renaming_a_board_changes_only_its_name() {
    let r = Rig::new();
    let (id, net) = wind_and_net(&r.core);
    let t = on_board(&r, &id, &net, "rollback");
    r.core.rename_space_board(&id, &net, " Sockets ").unwrap();
    assert_eq!(
        names(&r.core, &id),
        ["wind:Wind Waker Multiplayer", "net:Sockets"]
    );
    assert_eq!(r.core.task_ref("net#1").unwrap(), t.id);
    assert_eq!(space_name(&r.core, &id), "Wind Waker Multiplayer");
}

#[test]
fn a_rename_needs_a_name_and_a_board_in_the_space() {
    let r = Rig::new();
    let (id, net) = wind_and_net(&r.core);
    let other = r.core.create_board("Other", "other").unwrap();
    let kind = |e: BoardError| match e {
        BoardError::Coord(c) => c.kind(),
        e => panic!("{e}"),
    };
    let e = r.core.rename_space_board(&id, &net, "  ").unwrap_err();
    assert_eq!(kind(e), CK::Invalid);
    let e = r.core.rename_space_board(&id, "nope", "x").unwrap_err();
    assert_eq!(kind(e), CK::NotFound);
    let e = r.core.rename_space_board(&other, &net, "x").unwrap_err();
    assert_eq!(kind(e), CK::NotFound, "net is in another space");
    assert_eq!(names(&r.core, &id)[1], "net:Netcode");
}

/// The first board's name is its own once renamed, and the space's name
/// stays apart from it, each way: until then it follows the space.
#[test]
fn a_first_boards_name_and_the_spaces_are_separate() {
    let r = Rig::new();
    let c = &r.core;
    let (id, _) = wind_and_net(c);
    // No name of its own yet: it follows the space's.
    c.rename_board(&id, "Wind Waker").unwrap();
    assert_eq!(names(c, &id)[0], "wind:Wind Waker");
    // Renaming it leaves the space's name alone.
    c.rename_space_board(&id, &id, "Tasks").unwrap();
    assert_eq!(names(c, &id)[0], "wind:Tasks");
    assert_eq!(space_name(c, &id), "Wind Waker");
    assert_eq!(
        c.board_verifier(&id).unwrap().membership().name,
        "Wind Waker"
    );
    // With a name of its own, a space rename leaves it alone too.
    c.rename_board(&id, "Wind Waker Online").unwrap();
    assert_eq!(names(c, &id)[0], "wind:Tasks");
    assert_eq!(space_name(c, &id), "Wind Waker Online");
}

/// Agents see a first board's own name, in the list and on a card alike.
#[test]
fn the_tools_name_a_first_board_the_same_everywhere() {
    let r = Rig::new();
    let (id, _) = wind_and_net(&r.core);
    r.core.rename_space_board(&id, &id, "Tasks").unwrap();
    let caller = tools::Caller {
        sender: inbox::Sender::new(&r.key_a, "alpha", "claude"),
        path: String::new(),
    };
    let call = |tool: &str, args: serde_json::Value| {
        let args = serde_json::to_vec(&args).unwrap();
        r.core.tools().call(&caller, tool, &args).unwrap()
    };
    let out = call(
        "board_create",
        serde_json::json!({"title": "decomp", "board": "wind"}),
    );
    assert!(out.contains("on Tasks (wind)"), "from boards(): {out}");
    let out = call(
        "board_share",
        serde_json::json!({"task_id": "wind#1", "board": "wind"}),
    );
    assert!(out.contains("on Tasks (wind)"), "from board_of(): {out}");
}

/// A card on an inner board is on that board, not the space's first: its
/// name and tag in what the tools say about the card.
#[test]
fn the_tools_name_a_cards_own_board() {
    let r = Rig::new();
    let (id, _) = wind_and_net(&r.core);
    r.core.rename_space_board(&id, &id, "Tasks").unwrap();
    let caller = tools::Caller {
        sender: inbox::Sender::new(&r.key_a, "alpha", "claude"),
        path: String::new(),
    };
    let call = |tool: &str, args: serde_json::Value| {
        let args = serde_json::to_vec(&args).unwrap();
        r.core.tools().call(&caller, tool, &args).unwrap()
    };
    call(
        "board_create",
        serde_json::json!({"title": "decomp", "board": "net"}),
    );
    let out = call(
        "board_share",
        serde_json::json!({"task_id": "net#1", "board": "wind"}),
    );
    assert!(
        out.contains("already on Netcode (net) in Wind Waker Multiplayer"),
        "{out}"
    );
}

/// The board tool shows a space once, with its people and autonomy, and
/// its boards under it; a card's result names the space an inner board is
/// in (card #558).
#[test]
fn the_board_tool_shows_spaces_with_their_boards() {
    let r = Rig::new();
    let (id, _) = wind_and_net(&r.core);
    r.core.create_board("Lemmings", "lem").unwrap();
    r.core.rename_space_board(&id, &id, "Tasks").unwrap();
    let caller = tools::Caller {
        sender: inbox::Sender::new(&r.key_a, "alpha", "claude"),
        path: String::new(),
    };
    let call = |tool: &str, args: serde_json::Value| {
        let args = serde_json::to_vec(&args).unwrap();
        r.core.tools().call(&caller, tool, &args).unwrap()
    };
    let out = call("board", serde_json::json!({}));
    assert!(out.contains("Spaces you may post on"), "{out}");
    let wind = out
        .lines()
        .position(|l| l.starts_with("- \"Wind Waker Multiplayer\": "))
        .expect(&out);
    assert_eq!(
        out.lines().nth(wind + 1),
        Some("  boards: wind \"Tasks\", net \"Netcode\""),
        "{out}"
    );
    assert_eq!(out.matches("your autonomy there").count(), 2, "{out}");
    assert!(out.contains("  boards: lem \"Lemmings\""), "{out}");

    let out = call(
        "board_create",
        serde_json::json!({"title": "decomp", "board": "net"}),
    );
    assert!(
        out.contains("on Netcode (net) in Wind Waker Multiplayer"),
        "{out}"
    );
}

/// A board nobody has put a card on has its tag, through the real hook.
#[test]
fn a_ref_to_a_board_with_no_cards_is_not_on_another_board() {
    let r = Rig::new();
    let (id, _) = wind_and_net(&r.core);
    let co = r.core.board_coord(&id).unwrap();
    let e = co.task_ref("net#4").unwrap_err();
    assert_eq!(e.kind(), CK::NotFound);
    assert!(!e.to_string().contains("another board"), "{e}");
    let e = co.task_ref("elsewhere#4").unwrap_err();
    assert!(e.to_string().contains("another board"), "{e}");
}

/// A subtask goes on its parent's board, made either way: with `parent`
/// (the board isn't named) or by splitting the parent.
#[test]
fn a_subtask_stays_on_its_parents_board() {
    let r = Rig::new();
    let (id, net) = wind_and_net(&r.core);
    let parent = on_board(&r, &id, &net, "rollback");
    let co = r.core.board_coord(&id).unwrap();
    let by = r.core.board_key(&id, &r.key_a).unwrap();
    let kid = co
        .create_task(
            &by,
            coord::NewTask {
                parent: parent.id.clone(),
                ..new_task("kid")
            },
        )
        .unwrap();
    assert_eq!(kid.board, net);
    assert_eq!(kid.tag, "net");
    let (_, parts) = co
        .split(
            &by,
            &parent.id,
            vec![coord::NewSubtask {
                title: "part".into(),
                ..Default::default()
            }],
            Vec::new(),
        )
        .unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].board, net);
    assert_eq!(parts[0].tag, "net");
}

/// board_create and board_share given an inner board's tag put the card on
/// that board, numbered there, and say so (card #558).
#[test]
fn the_tools_put_a_card_on_the_inner_board_they_name() {
    let r = Rig::new();
    let (id, net) = wind_and_net(&r.core);
    r.core
        .inbox()
        .set_autonomy(
            inbox::AutonomySetting {
                board_id: id.clone(),
                preset: inbox::autonomy::AUTONOMOUS.into(),
            },
            &r.core.user_sender().key,
        )
        .unwrap();
    let caller = tools::Caller {
        sender: inbox::Sender::new(&r.key_a, "alpha", "claude"),
        path: String::new(),
    };
    let call = |tool: &str, args: serde_json::Value| {
        let args = serde_json::to_vec(&args).unwrap();
        r.core.tools().call(&caller, tool, &args).unwrap()
    };
    let co = r.core.board_coord(&id).unwrap();

    let out = call(
        "board_create",
        serde_json::json!({"title": "decomp", "board": "net"}),
    );
    assert!(out.contains("Created net#1"), "{out}");
    let made = co.get_task(&co.task_ref("net#1").unwrap()).unwrap();
    assert_eq!(
        (made.board.as_str(), made.tag.as_str()),
        (net.as_str(), "net")
    );

    let private = r
        .core
        .coord()
        .create_task(&r.core.user_sender().key, new_task("rollback"))
        .unwrap();
    let out = call(
        "board_share",
        serde_json::json!({"task_id": private.id, "board": "net"}),
    );
    assert!(
        out.contains("to Netcode (net) in Wind Waker Multiplayer as net#2"),
        "{out}"
    );
    let shared = co.get_task(&co.task_ref("net#2").unwrap()).unwrap();
    assert_eq!(shared.title, "rollback");
    assert_eq!(
        (shared.board.as_str(), shared.tag.as_str()),
        (net.as_str(), "net")
    );
    // Nothing landed on the space's first board.
    assert_eq!(co.task_ref("wind#1").unwrap_err().kind(), CK::NotFound);

    // By the inner board's id too.
    let private = r
        .core
        .coord()
        .create_task(&r.core.user_sender().key, new_task("lag"))
        .unwrap();
    let out = call(
        "board_share",
        serde_json::json!({"task_id": private.id, "board": net}),
    );
    assert!(out.contains("as net#3"), "{out}");
}

/// A notes scope that names a board inside a space means that space:
/// notes are kept per space (card #558).
#[test]
fn a_notes_scope_by_an_inner_board_means_its_space() {
    let r = Rig::new();
    let (id, net) = wind_and_net(&r.core);
    let me = r.core.user_sender().key;
    for scope in ["net", net.as_str()] {
        let n = coord::NewNote {
            kind: "rule".into(),
            text: format!("rollback by {scope}"),
            tags: vec!["topic:netcode".into()],
            ..Default::default()
        };
        let got = r.core.propose_note(&me, scope, n).unwrap();
        assert_eq!(got.board_id, id, "scope {scope}");
        assert_eq!(got.scope, "wind", "scope {scope}");
    }
}

/// Spaces-boards.md §1, agreed with #538: a hidden DM space, known by its
/// private marker only, lists no boards, takes no board and takes no card.
#[test]
fn a_hidden_dm_space_has_no_boards_and_takes_none() {
    let r = Rig::new();
    let c = &r.core;
    let (id, _) = wind_and_net(c);
    assert_eq!(c.space_boards(&id).unwrap().len(), 2);

    let me = c.identity().current().id;
    let pin = c.board_verifier(&id).unwrap().pinned(&me).unwrap();
    c.write_dm_marker(&id, &pin, "bob").unwrap();
    assert!(c.is_dm_space(&id));

    assert!(c.space_boards(&id).unwrap().is_empty(), "it lists boards");
    let e = c.create_space_board(&id, "Art", "art").unwrap_err();
    assert!(e.to_string().contains("holds no boards"), "{e}");
    let co = c.board_coord(&id).unwrap();
    let by = c.board_key(&id, &r.key_a).unwrap();
    assert!(
        co.create_task(&by, new_task("in a DM space")).is_err(),
        "a card went into a hidden space"
    );
}

/// Card #563: a move through the Core, and a delete. No card is lost, every
/// old ref still works, and the deleted board's tag stays taken.
#[test]
fn a_card_moves_and_a_deleted_boards_cards_land_on_the_first() {
    let r = Rig::new();
    let (id, net) = wind_and_net(&r.core);
    let co = r.core.board_coord(&id).unwrap();
    let by = r.core.board_key(&id, &r.key_a).unwrap();
    let w = on_board(&r, &id, "", "on the first");
    let n = on_board(&r, &id, &net, "on net");
    let t = co.move_task(&by, &w.id, &net).unwrap();
    assert_eq!(t.short_ref(), "net#2");
    assert_eq!(r.core.task_ref("wind#1").unwrap(), w.id);

    r.core.delete_space_board(&id, &net).unwrap();
    assert_eq!(names(&r.core, &id), ["wind:Wind Waker Multiplayer"]);
    let (a, b) = (co.get_task(&n.id).unwrap(), co.get_task(&w.id).unwrap());
    assert_eq!((a.board.as_str(), a.tag.as_str()), ("", "wind"));
    assert_eq!((b.board.as_str(), b.tag.as_str()), ("", "wind"));
    assert_eq!((a.short.as_str(), b.short.as_str()), ("2", "3"));
    assert_eq!(r.core.task_ref("net#1").unwrap(), n.id);
    assert_eq!(r.core.task_ref("net#2").unwrap(), w.id);
    assert_eq!(r.core.task_ref("wind#1").unwrap(), w.id);

    let e = r
        .core
        .create_space_board(&id, "Network", "net")
        .unwrap_err();
    assert!(e.to_string().contains("already board"), "{e}");
    let e = co.move_task(&by, &w.id, &net).unwrap_err();
    assert_eq!(e.kind(), CK::NotFound);
}

#[test]
fn the_first_board_cant_be_deleted_and_a_deleted_one_stays_gone() {
    let r = Rig::new();
    let (id, net) = wind_and_net(&r.core);
    let e = r.core.delete_space_board(&id, &id).unwrap_err();
    assert!(matches!(&e, BoardError::Coord(c) if c.kind() == CK::NotAllowed));
    let e = r.core.delete_space_board(&id, "nope").unwrap_err();
    assert!(matches!(&e, BoardError::Coord(c) if c.kind() == CK::NotFound));
    r.core.delete_space_board(&id, &net).unwrap();
    let e = r.core.delete_space_board(&id, &net).unwrap_err();
    assert!(matches!(&e, BoardError::Coord(c) if c.kind() == CK::NotFound));
    let e = r.core.rename_space_board(&id, &net, "Back").unwrap_err();
    assert!(matches!(&e, BoardError::Coord(c) if c.kind() == CK::NotFound));
}
