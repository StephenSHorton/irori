//! Go boards_service_test.go on a scratch Core: shared boards in the
//! window's grid and on a card's page, Share and Withdraw, and a comment
//! on a shared card reaching its agent. Agent a (alpha) is the Rig's live
//! session; Go's fakes become real rows.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use irori_window::boards::SCOPE_BOARD;
use irori_window::cards::{CardSummary, SCOPE_MINE, SCOPE_OWN};
use irori_window::invites::{INVITE_DONE, INVITE_WORDS, InviteStatus};
use store::kinds::{self, AGENT_ACTOR_PREFIX};

fn create(r: &Rig, title: &str) -> coord::Task {
    r.w.core
        .coord()
        .create_task(
            &r.w.core.user_sender().key,
            coord::NewTask {
                title: title.into(),
                ..Default::default()
            },
        )
        .unwrap()
}

/// TestBoardsInTheWindow: the grid holds every board's live cards beside
/// the private ones, each with its board's name, id and tag (v0.5 §8); a
/// shared card's page says who sees it and offers Withdraw; a private card
/// shared to a board says where it went; a withdrawn card leaves the grid.
#[test]
fn boards_in_the_window() {
    let r = Rig::new();
    let w = &r.w;
    create(&r, "stays private");
    let priv_t = create(&r, "goes to the board");
    let mlg = w.boards_create("MLG", "MLG").unwrap();
    assert!(
        mlg.tag == "mlg" && mlg.name == "MLG" && mlg.owner && mlg.people.len() == 1,
        "board {mlg:?}"
    );
    let p = w.boards_share_preview(&priv_t.id, &mlg.board_id).unwrap();
    assert!(
        p.title == "goes to the board" && p.visible_to.len() == 1,
        "preview {p:?}"
    );
    let shared_id = w.boards_share(&priv_t.id, &mlg.board_id, &p.hash).unwrap();
    let cards = w.cards();
    let shared: CardSummary = cards
        .iter()
        .find(|s| s.id == shared_id)
        .cloned()
        .expect("the shared card is in the grid");
    let privates = cards.iter().filter(|s| s.scope == SCOPE_MINE).count();
    assert!(
        shared.board == mlg.board_id
            && shared.tag == "mlg"
            && shared.short == "1"
            && shared.scope == SCOPE_BOARD
            && shared.scope_label == "MLG",
        "shared card in the grid {shared:?}"
    );
    assert_eq!(privates, 1, "the shared one's private card is closed");
    let d = w.card(&shared_id).unwrap();
    let ob = d.on_board.expect("a board line");
    assert!(
        ob.name == "MLG" && ob.can_withdraw && ob.visible_to.len() == 1,
        "board line {ob:?}"
    );
    let pd = w.card(&priv_t.id).unwrap();
    assert_eq!(pd.shared_to.expect("shared to").ref_, "mlg#1");
    // A write from the window to the private id is refused, naming mlg#1.
    let e = w.comment(&priv_t.id, "more", "none", &[], &[]).unwrap_err();
    assert!(
        e.contains("mlg#1"),
        "a comment on the shared private card: {e}"
    );
    // The person comments on the shared card from the window.
    w.comment(&shared_id, "on the board", "none", &[], &[])
        .unwrap();
    let co = w.core.board_coord(&mlg.board_id).unwrap();
    let cs = co.comments(&shared_id).unwrap_or_default();
    assert!(
        cs.len() == 1 && cs[0].by.starts_with("person:"),
        "board comments {cs:?}"
    );
    // A shared card is pinned in the person's own log (card #161), not
    // on the board.
    w.pin(&shared_id, true).unwrap();
    assert!(
        w.cards().iter().any(|c| c.id == shared_id && c.pinned),
        "the shared card shows pinned"
    );
    let bco = w.core.board_coord(&mlg.board_id).unwrap();
    assert!(
        !bco.get_task(&shared_id).unwrap().pinned,
        "no pin on the board"
    );
    w.pin(&shared_id, false).unwrap();
    w.boards_withdraw(&shared_id).unwrap();
    assert!(
        !w.cards().iter().any(|s| s.id == shared_id),
        "a withdrawn card is still in the grid"
    );
    let t = w.core.coord().get_task(&priv_t.id).unwrap();
    assert!(
        !coord::is_terminal(&t.state),
        "the private card stayed closed: {}",
        t.state
    );
    w.boards_leave(&mlg.board_id).unwrap();
    assert!(w.boards_list().is_empty(), "boards after leave");
}

/// TestCommentOnASharedCardTellsItsAgent: the person's comment from the
/// window on a shared card reaches the agent assigned to it. The board
/// knows the agent as agent:<cardId>; the roster knows its session, so who
/// is around is asked of the session.
#[test]
fn comment_on_a_shared_card_tells_its_agent() {
    let r = Rig::new();
    let w = &r.w;
    let mlg = w.boards_create("MLG", "mlg").unwrap();
    let card_id = "0123456789abcdef0123456789abcdef";
    let agent = format!("{AGENT_ACTOR_PREFIX}{card_id}");
    w.core
        .store()
        .append(
            "irori",
            kinds::BOARD_CARD_OF,
            &serde_json::json!({"boardId": mlg.board_id, "cardId": card_id, "agentKey": r.key_a}),
        )
        .unwrap();
    w.core
        .board_store(&mlg.board_id)
        .unwrap()
        .append(
            &agent,
            kinds::AGENT_CARD,
            &serde_json::json!({"cardId": card_id, "name": "alpha", "kind": "claude",
                "personId": w.core.person().id, "status": "active"}),
        )
        .unwrap();
    wait("the card maps to the session", || {
        w.core.private_key(&agent) == r.key_a
    });
    let co = w.core.board_coord(&mlg.board_id).unwrap();
    let me = w
        .core
        .board_key(&mlg.board_id, &w.core.user_sender().key)
        .unwrap();
    let card = co
        .create_task(
            &me,
            coord::NewTask {
                title: "build it".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.assign(&me, &card.id, &agent).unwrap();
    let res = w
        .comment(&card.id, "which branch?", "wake", &[], &[])
        .unwrap();
    assert!(
        res.told.to == [r.key_a.clone()] && res.answers.len() == 1 && res.answers[0].key == r.key_a,
        "the comment told {:?}, answers {:?}; want the assigned agent's session",
        res.told,
        res.answers
    );
}

/// Card #126: New card on a board's page goes on that board, unheld, or to
/// the agent the person picked; the Boards list counts it and says when the
/// board last had activity.
#[test]
fn new_card_on_a_board() {
    use irori_window::new_card::NewCardInput;
    let r = Rig::new();
    let w = &r.w;
    let mlg = w.boards_create("MLG", "mlg").unwrap();
    let before = w.boards_list();
    assert!(
        before[0].cards == 0 && before[0].last_at.is_zero(),
        "empty board {:?}",
        before[0]
    );
    let res = w
        .create_card(NewCardInput {
            title: "Lay out the farm".into(),
            board: mlg.board_id.clone(),
            ..Default::default()
        })
        .unwrap();
    assert!(res.told.to.is_empty(), "unheld: nobody told {:?}", res.told);
    assert!(res.task.holders.is_empty() && res.task.assigned.is_empty());
    let co = w.core.board_coord(&mlg.board_id).unwrap();
    assert!(co.get_task(&res.task.id).is_some(), "on the board's log");
    assert!(
        w.core.coord().get_task(&res.task.id).is_none(),
        "not on the private board"
    );
    let res = w
        .create_card(NewCardInput {
            title: "Plant the crops".into(),
            board: mlg.board_id.clone(),
            assign_to: r.key_a.clone(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(res.told.to, vec![r.key_a.clone()]);
    assert_eq!(res.told.mode, "wake");
    let me = w
        .core
        .board_key(&mlg.board_id, &w.core.user_sender().key)
        .unwrap();
    co.set_task_state(&me, &res.task.id, coord::COMPLETED, "")
        .unwrap();
    let b = &w.boards_list()[0];
    assert!(
        b.cards == 1 && b.done_week == 1 && !b.last_at.is_zero(),
        "after two cards, one done: {b:?}"
    );
}

/// Card #558: a card made on a board inside a space shows that board's tag
/// and id everywhere the window names it, and `board` stays the space's id.
/// A first board's card carries the space's id, asked for either way.
#[test]
fn a_card_shows_the_board_it_is_on_inside_its_space() {
    use irori_window::new_card::NewCardInput;
    let r = Rig::new();
    let w = &r.w;
    let wind = w.boards_create("Wind", "wind").unwrap();
    let net = w
        .boards_add_to_space(&wind.board_id, "Netcode", "net")
        .unwrap()
        .boards[1]
        .board_id
        .clone();
    let make = |title: &str, space_board: &str| {
        w.create_card(NewCardInput {
            title: title.into(),
            board: wind.board_id.clone(),
            space_board: space_board.into(),
            ..Default::default()
        })
        .unwrap()
        .task
    };
    let on_net = make("rollback", &net);
    let on_first = make("sails", "");
    let on_first_by_id = make("oars", &wind.board_id);
    assert_eq!(on_net.short_ref(), "net#1");
    assert_eq!(on_first.short_ref(), "wind#1");
    assert_eq!(on_first_by_id.short_ref(), "wind#2");
    assert_eq!(on_first_by_id.board, "");

    let grid = w.cards();
    let card = |id: &str| grid.iter().find(|c| c.id == id).unwrap();
    let c = card(&on_net.id);
    assert_eq!(
        (c.board.as_str(), c.tag.as_str(), c.space_board.as_str()),
        (wind.board_id.as_str(), "net", net.as_str())
    );
    let c = card(&on_first.id);
    assert_eq!(
        (c.board.as_str(), c.tag.as_str(), c.space_board.as_str()),
        (wind.board_id.as_str(), "wind", wind.board_id.as_str())
    );

    let d = w.card(&on_net.id).unwrap();
    assert_eq!(d.summary.board, wind.board_id);
    assert_eq!(d.summary.tag, "net");
    assert_eq!(d.summary.space_board, net);
    let on = d.on_board.unwrap();
    assert_eq!(
        (on.id.as_str(), on.tag.as_str(), on.space_board.as_str()),
        (wind.board_id.as_str(), "net", net.as_str())
    );
    let on = w.card(&on_first.id).unwrap().on_board.unwrap();
    assert_eq!(
        (on.tag.as_str(), on.space_board.as_str()),
        ("wind", wind.board_id.as_str())
    );

    // A board the space doesn't hold is refused, not put on the first.
    let e = w
        .create_card(NewCardInput {
            title: "x".into(),
            board: wind.board_id.clone(),
            space_board: "nope".into(),
            ..Default::default()
        })
        .unwrap_err();
    assert!(e.contains("no board"), "{e}");
}

/// Card #558: Rename in a space names one board, and leaves the space's
/// name alone even when that board is the first.
#[test]
fn renaming_a_board_in_a_space_from_the_window() {
    let r = Rig::new();
    let w = &r.w;
    let wind = w.boards_create("Wind", "wind").unwrap();
    let id = wind.board_id;
    let s = w.boards_add_to_space(&id, "Netcode", "net").unwrap();
    let net = s.boards[1].board_id.clone();
    let s = w.boards_rename_in_space(&id, &net, "Sockets").unwrap();
    assert_eq!(s.boards[1].name, "Sockets");
    assert_eq!(s.boards[1].tag, "net");
    let s = w.boards_rename_in_space(&id, &id, "Tasks").unwrap();
    assert_eq!(s.boards[0].name, "Tasks");
    assert_eq!(s.name, "Wind");
    let e = w.boards_rename_in_space(&id, "nope", "x").unwrap_err();
    assert!(e.contains("no board"), "{e}");
}

/// Card #563: Move and Delete from the window. The grid shows each card on
/// its new board, and deleting a board leaves every card on the first.
#[test]
fn moving_a_card_and_deleting_a_board_from_the_window() {
    use irori_window::new_card::NewCardInput;
    let r = Rig::new();
    let w = &r.w;
    let wind = w.boards_create("Wind", "wind").unwrap();
    let id = wind.board_id;
    let s = w.boards_add_to_space(&id, "Netcode", "net").unwrap();
    let net = s.boards[1].board_id.clone();
    let make = |title: &str, space_board: &str| {
        w.create_card(NewCardInput {
            title: title.into(),
            board: id.clone(),
            space_board: space_board.into(),
            ..Default::default()
        })
        .unwrap()
        .task
    };
    let a = make("sails", "");
    let b = make("rollback", &net);
    let t = w.move_card(&a.id, &net).unwrap();
    assert_eq!(t.short_ref(), "net#2");
    let grid = w.cards();
    let c = grid.iter().find(|c| c.id == a.id).unwrap();
    assert_eq!(
        (c.tag.as_str(), c.space_board.as_str()),
        ("net", net.as_str())
    );

    let t = w.move_card(&a.id, &id).unwrap();
    assert_eq!(t.short_ref(), "wind#2");
    w.move_card(&a.id, &net).unwrap();

    let e = w.boards_delete_in_space(&id, &id).unwrap_err();
    assert!(e.contains("first board"), "{e}");
    let s = w.boards_delete_in_space(&id, &net).unwrap();
    assert_eq!(s.boards.len(), 1);
    let grid = w.cards();
    for card in [&a, &b] {
        let c = grid.iter().find(|c| c.id == card.id).unwrap();
        assert_eq!(c.tag, "wind", "no card lost: {c:?}");
        assert_eq!(c.space_board, id);
    }
}

/// A Rig on loopback iroh with no LAN lookup (the invite tests).
/// Personal from the window (card #558): its boards listed, added and
/// renamed; a card made on one shows its tag and board, numbered from 1,
/// while the cards already there stay on the default board as `#N`.
#[test]
fn personal_boards_from_the_window() {
    let r = Rig::new();
    let w = &r.w;
    let old = create(&r, "already here");
    assert_eq!(old.short_ref(), "#1");
    let bs = w.boards_personal().unwrap();
    assert_eq!(bs.len(), 1, "{bs:?}");
    assert_eq!(bs[0].board_id, irori_core::PERSONAL_BOARD);
    assert_eq!((bs[0].name.as_str(), bs[0].tag.as_str()), ("Personal", ""));

    let bs = w.boards_add_to_personal("Home jobs", "HOME").unwrap();
    assert_eq!(bs.len(), 2, "{bs:?}");
    assert_eq!(bs[1].tag, "home");
    let home = bs[1].board_id.clone();

    let on = |board: &str, title: &str| {
        w.create_card(irori_window::new_card::NewCardInput {
            title: title.into(),
            space_board: board.into(),
            ..Default::default()
        })
        .unwrap()
        .task
    };
    let fence = on(&home, "Fix the fence");
    assert_eq!(fence.short_ref(), "home#1");
    let next = on(irori_core::PERSONAL_BOARD, "On the default board");
    assert_eq!(next.short_ref(), "#2");

    let cards = w.cards();
    let card = |id: &str| cards.iter().find(|c| c.id == id).unwrap();
    let c = card(&fence.id);
    assert_eq!((c.tag.as_str(), c.space_board.as_str()), ("home", &*home));
    assert_eq!(c.board, "", "Home keeps it: it is in no shared space");
    assert_eq!(c.scope, SCOPE_MINE);
    let c = card(&old.id);
    assert_eq!(c.tag, "");
    assert_eq!(c.space_board, irori_core::PERSONAL_BOARD);
    assert_eq!(w.card(&fence.id).unwrap().summary.space_board, home);

    let bs = w
        .boards_rename_in_personal(irori_core::PERSONAL_BOARD, "Inbox")
        .unwrap();
    assert_eq!(bs[0].name, "Inbox");
    let bs = w.boards_rename_in_personal(&home, "House").unwrap();
    assert_eq!((bs[1].name.as_str(), bs[1].tag.as_str()), ("House", "home"));
    let e = w.boards_rename_in_personal("nope", "x").unwrap_err();
    assert!(e.contains("no board"), "{e}");
    let e = w.boards_add_to_personal("Again", "home").unwrap_err();
    assert!(e.contains("already board"), "{e}");
}

fn on_loopback() -> Rig {
    Rig::with(|o| {
        o.no_home = true;
        o.pair_net = Some(Arc::new(move |ident, chain, guests| {
            let mut nc = irori_core::devices::default_net_config(ident, chain, guests);
            nc.ip = irori_net::endpoint::IpMode::Loopback;
            nc.lan_config = None;
            nc.gossip = false;
            irori_net::transport::IrohNet::new(nc)
        }));
    })
}

/// Boards → Invite and Join with an invite (§4.1, §8) through the window's
/// seam: two made-up people on loopback iroh. The inviter's status carries
/// the link; the invitee's shows the five words; the inviter types them
/// and both end done, with the board on the invitee's Boards dialog and
/// boards:invite told along the way. Cancel closes an open invite.
#[test]
fn invite_and_join_from_the_window() {
    let (a, b) = (on_loopback(), on_loopback());
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    {
        let s = seen.clone();
        a.w.seams
            .invites
            .watch(Some(Arc::new(move |st: InviteStatus| {
                s.lock().unwrap().push(st.state)
            })));
    }
    let mlg = a.w.boards_create("MLG", "mlg").unwrap();
    let (ia, ib) = (&a.w.seams.invites, &b.w.seams.invites);
    let st = ia.invite(&mlg.board_id).unwrap();
    assert!(
        st.role == "host" && st.open() && st.link.starts_with("irori-invite:"),
        "{st:?}"
    );
    assert!(!st.expires_at.is_empty());
    assert!(
        ia.invite(&mlg.board_id).is_err(),
        "a second invite while one is open"
    );
    let sb = ib.join(&format!("  {}\n", st.link)).unwrap();
    assert_eq!(
        (sb.role.as_str(), sb.board_id.as_str()),
        ("guest", mlg.board_id.as_str())
    );
    wait("both screens show the words", || {
        ia.status().state == INVITE_WORDS && ib.status().state == INVITE_WORDS
    });
    let words = ib.status().words;
    assert_eq!(words.len(), 5);
    assert!(ia.confirm_words("not the words at all here").is_err());
    assert_eq!(ia.status().misses, 1);
    ia.confirm_words(&words.join(" ")).unwrap();
    wait("both done", || {
        ia.status().state == INVITE_DONE && ib.status().state == INVITE_DONE
    });
    wait("the board on the invitee's dialog", || {
        b.w.boards_list().iter().any(|x| x.board_id == mlg.board_id)
    });
    assert!(seen.lock().unwrap().iter().any(|s| s == INVITE_DONE));

    // Cancel: a second invite, closed by the person.
    let st = ia.invite(&mlg.board_id).unwrap();
    assert!(st.open());
    ia.cancel();
    wait("canceled", || !ia.status().open());
    assert!(ib.join("irori-invite:nonsense").is_err());
}

/// A board opened in the Boards dialog (§8): its one member is the person,
/// as owner; the receive and autonomy presets start at their defaults
/// (Open, Trusted), the person sets each for the board, and "" goes back.
#[test]
fn board_settings_from_the_window() {
    let r = Rig::new();
    let w = &r.w;
    let mlg = w.boards_create("MLG", "mlg").unwrap();
    let s = w.boards_settings(&mlg.board_id).unwrap();
    assert!(
        s.members.len() == 1 && s.members[0].me && s.members[0].owner && s.members[0].devices == 1,
        "members {:?}",
        s.members
    );
    assert!(
        s.receive == "open" && !s.receive_own && s.autonomy == "trusted" && !s.autonomy_own,
        "{s:?}"
    );
    let s = w
        .boards_set_receive(&mlg.board_id, "members", vec!["person-b".into()])
        .unwrap();
    assert!(
        s.receive == "members" && s.people == ["person-b"] && s.receive_own,
        "{s:?}"
    );
    let s = w.boards_set_autonomy(&mlg.board_id, "ask").unwrap();
    assert!(s.autonomy == "ask" && s.autonomy_own, "{s:?}");
    assert!(w.boards_set_receive(&mlg.board_id, "loud", vec![]).is_err());
    assert!(
        w.boards_settings("0123456789abcdef0123456789abcdef")
            .is_err()
    );
    let s = w.boards_set_receive(&mlg.board_id, "", vec![]).unwrap();
    assert!(s.receive == "open" && !s.receive_own, "{s:?}");
    let s = w.boards_set_autonomy(&mlg.board_id, "").unwrap();
    assert!(s.autonomy == "trusted" && !s.autonomy_own, "{s:?}");
}

/// The Boards dialog's member actions on a board of one (§4.2-4.4): the
/// owner sees their fingerprint, can't remove themselves or stop being the
/// last owner, and a membership row the board ignored shows as one
/// conflict line. Leave alone forgets at once (nobody to wait for).
#[test]
fn member_actions_and_conflicts_in_the_window() {
    let r = Rig::new();
    let w = &r.w;
    let mlg = w.boards_create("MLG", "mlg").unwrap();
    let me = w.core.person().id;
    let s = w.boards_settings(&mlg.board_id).unwrap();
    assert!(!s.members[0].fingerprint.is_empty(), "{s:?}");
    assert!(s.conflicts.is_empty() && !s.leaving, "{s:?}");
    assert!(w.boards_remove_member(&mlg.board_id, &me).is_err());
    assert!(w.boards_set_role(&mlg.board_id, &me, "member").is_err());
    assert!(w.boards_set_role(&mlg.board_id, "nobody", "owner").is_err());

    // A role row for someone who was never on the board: the fold ignores
    // it, and the owner sees why.
    let epoch = w.core.identity().current().epoch;
    let row = w
        .core
        .identity()
        .board_member_role_row(
            epoch,
            "nobody",
            "owner",
            gocompat::time::GoTime::from_system(std::time::SystemTime::now()),
        )
        .unwrap();
    w.core
        .board_store(&mlg.board_id)
        .unwrap()
        .append(
            &format!("{}{me}", store::kinds::PERSON_ACTOR_PREFIX),
            store::kinds::BOARD_MEMBER_ROLE,
            &row,
        )
        .unwrap();
    let s = w.boards_settings(&mlg.board_id).unwrap();
    assert_eq!(
        s.conflicts,
        ["Your role change for someone didn't take: they are not on the board"],
        "{s:?}"
    );

    w.boards_leave(&mlg.board_id).unwrap();
    assert!(w.boards_list().iter().all(|b| b.board_id != mlg.board_id));
}

/// Card #115: Closed lists finished shared-board cards beside the private
/// ones, each with its board chip, and one cursor pages across both; a
/// withdrawn board card stays out, as it does in the grid.
#[test]
fn closed_holds_finished_board_cards() {
    let r = Rig::new();
    let w = &r.w;
    let co = w.core.coord();
    let me = w.core.user_sender().key;
    for i in 0..2 {
        let t = create(&r, &format!("private {i}"));
        co.set_task_state(&me, &t.id, coord::COMPLETED, "").unwrap();
    }
    let mlg = w.boards_create("MLG", "mlg").unwrap();
    let bco = w.core.board_coord(&mlg.board_id).unwrap();
    let bme = w.core.board_key(&mlg.board_id, &me).unwrap();
    let mut board_ids = Vec::new();
    for (i, state) in [coord::COMPLETED, coord::CANCELED, coord::FAILED]
        .into_iter()
        .enumerate()
    {
        let t = bco
            .create_task(
                &bme,
                coord::NewTask {
                    title: format!("board {i}"),
                    ..Default::default()
                },
            )
            .unwrap();
        bco.set_task_state(&bme, &t.id, state, "").unwrap();
        board_ids.push(t.id);
    }
    // Still open: never under Closed.
    bco.create_task(
        &bme,
        coord::NewTask {
            title: "open on the board".into(),
            ..Default::default()
        },
    )
    .unwrap();

    let all = w.done_cards(0, "").unwrap();
    assert_eq!(all.cards.len(), 5, "closed {:?}", all.cards);
    for id in &board_ids {
        let s = all
            .cards
            .iter()
            .find(|s| &s.id == id)
            .expect("a finished board card under Closed");
        assert!(
            s.scope == SCOPE_BOARD
                && s.scope_label == "MLG"
                && s.board == mlg.board_id
                && s.tag == "mlg",
            "board chip {s:?}"
        );
    }
    assert_eq!(
        all.cards.iter().filter(|s| s.scope == SCOPE_MINE).count(),
        2
    );

    // Pages of 2 walk all five once, newest first.
    let mut seen = Vec::new();
    let mut next = String::new();
    loop {
        let p = w.done_cards(2, &next).unwrap();
        assert!(p.cards.len() <= 2);
        seen.extend(p.cards.iter().map(|s| s.id.clone()));
        if p.next.is_empty() {
            break;
        }
        next = p.next;
    }
    let want: Vec<String> = all.cards.iter().map(|s| s.id.clone()).collect();
    assert_eq!(seen, want, "paging across private and board cards");
}

/// Card #161: a board's family shows as one on the grid (the subtask
/// names its parent and sits right after it; the parent says which
/// subtask waits on a decision), the board page's scope holds only that
/// board, and the person's pins on board cards order with their own.
#[test]
fn board_cards_carry_families_scopes_and_pins() {
    let r = Rig::new();
    let w = &r.w;
    let mine = create(&r, "a private card");
    let mlg = w.boards_create("MLG", "mlg").unwrap();
    let bco = w.core.board_coord(&mlg.board_id).unwrap();
    let me = w
        .core
        .board_key(&mlg.board_id, &w.core.user_sender().key)
        .unwrap();
    let parent = bco
        .create_task(
            &me,
            coord::NewTask {
                title: "the parent".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let kid = bco
        .create_task(
            &me,
            coord::NewTask {
                title: "a subtask".into(),
                parent: parent.id.clone(),
                ..Default::default()
            },
        )
        .unwrap();
    // Something newer between them, so grouping is what keeps them together.
    let other = bco
        .create_task(
            &me,
            coord::NewTask {
                title: "unrelated".into(),
                ..Default::default()
            },
        )
        .unwrap();
    bco.block(
        &me,
        &kid.id,
        "which way?",
        Some(coord::BlockedBy {
            kind: coord::BLOCKED_ON_PERSON.into(),
            ..Default::default()
        }),
    )
    .unwrap();

    let all = w.cards();
    let at = |id: &str| all.iter().position(|c| c.id == id).unwrap();
    let k = &all[at(&kid.id)];
    assert_eq!(
        k.parent.as_ref().map(|p| p.id.as_str()),
        Some(parent.id.as_str())
    );
    assert_eq!(
        at(&kid.id),
        at(&parent.id) + 1,
        "the subtask follows its parent"
    );
    assert!(
        all[at(&parent.id)].subtask_blocked.contains("which way?"),
        "parent says its subtask waits: {:?}",
        all[at(&parent.id)].subtask_blocked
    );
    assert!(at(&other.id) < usize::MAX);
    let d = w.card(&kid.id).unwrap();
    assert_eq!(
        d.summary.parent.map(|p| p.id),
        Some(parent.id.clone()),
        "a board subtask's page names its parent"
    );

    // Scopes.
    let own: Vec<String> = w.cards_in(SCOPE_OWN).into_iter().map(|c| c.id).collect();
    assert_eq!(own, vec![mine.id.clone()]);
    let board: Vec<String> = w
        .cards_in(&mlg.board_id)
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(board.len(), 3);
    assert!(!board.contains(&mine.id));

    // Pins: a board card and a private card in one order, none on the board.
    w.pin(&mine.id, true).unwrap();
    w.pin(&other.id, true).unwrap();
    w.move_pin(&other.id, &mine.id).unwrap();
    let all = w.cards();
    assert_eq!(
        all[0].id, other.id,
        "the board card moved before the private one"
    );
    assert_eq!(all[1].id, mine.id);
    assert!(!bco.get_task(&other.id).unwrap().pinned);
    // Pinning the board parent brings its subtask into Pinned with it.
    w.pin(&parent.id, true).unwrap();
    let all = w.cards();
    let ip = all.iter().position(|c| c.id == parent.id).unwrap();
    assert_eq!(all[ip + 1].id, kid.id);
    assert!(ip + 1 < 4, "the subtask sits in Pinned with its parent");
}

/// Card #161: a board's Closed pages on its own, so newer closed cards
/// elsewhere don't crowd it out.
#[test]
fn closed_pages_per_scope() {
    let r = Rig::new();
    let w = &r.w;
    let mlg = w.boards_create("MLG", "mlg").unwrap();
    let bco = w.core.board_coord(&mlg.board_id).unwrap();
    let me = w
        .core
        .board_key(&mlg.board_id, &w.core.user_sender().key)
        .unwrap();
    let mut on_board = Vec::new();
    for i in 0..2 {
        let t = bco
            .create_task(
                &me,
                coord::NewTask {
                    title: format!("board {i}"),
                    ..Default::default()
                },
            )
            .unwrap();
        bco.set_task_state(&me, &t.id, coord::COMPLETED, "")
            .unwrap();
        on_board.push(t.id);
    }
    let co = w.core.coord();
    let user = w.core.user_sender().key;
    for i in 0..3 {
        let t = create(&r, &format!("private {i}"));
        co.set_task_state(&user, &t.id, coord::COMPLETED, "")
            .unwrap();
    }
    let page = w.done_cards_in(2, "", &mlg.board_id).unwrap();
    let mut got: Vec<String> = page.cards.iter().map(|c| c.id.clone()).collect();
    got.sort();
    on_board.sort();
    assert_eq!(got, on_board, "both board cards on its first page");
    assert!(page.next.is_empty());
    let own = w.done_cards_in(10, "", SCOPE_OWN).unwrap();
    assert_eq!(own.cards.len(), 3);
    assert!(own.cards.iter().all(|c| c.scope == SCOPE_MINE));

    // Card #138: a clear on a board's Closed takes that board's cards
    // alone; Home's clear takes yours alone.
    assert_eq!(w.clear_done_count(0, &mlg.board_id), 2);
    assert_eq!(w.clear_done_count(0, SCOPE_OWN), 3);
    assert_eq!(w.clear_done_count(0, ""), 5);
    assert_eq!(w.clear_done(0, &[], &mlg.board_id).unwrap(), 2);
    assert!(
        w.done_cards_in(10, "", &mlg.board_id)
            .unwrap()
            .cards
            .is_empty()
    );
    assert_eq!(w.done_cards_in(10, "", SCOPE_OWN).unwrap().cards.len(), 3);
}

/// Card #146: a card another member's agent holds shows that agent on it,
/// named and live from the board's agent card, never "gone" ("Nobody on
/// it") just because this device's roster doesn't know the session.
#[test]
fn another_members_agent_shows_on_its_card() {
    let r = Rig::new();
    let w = &r.w;
    let mlg = w.boards_create("Horton", "horton").unwrap();
    let card_id = "abcdef0123456789abcdef0123456789";
    let agent = format!("{AGENT_ACTOR_PREFIX}{card_id}");
    // No board.cardof: the agent is the friend's, on their device.
    w.core
        .board_store(&mlg.board_id)
        .unwrap()
        .append(
            &agent,
            kinds::AGENT_CARD,
            &serde_json::json!({"cardId": card_id, "name": "otter", "kind": "claude",
                "personId": "friend-person", "status": "busy"}),
        )
        .unwrap();
    wait("the friend's agent card folds", || {
        w.core.board_agents().iter().any(|a| a.key == agent)
    });
    let co = w.core.board_coord(&mlg.board_id).unwrap();
    let me = w
        .core
        .board_key(&mlg.board_id, &w.core.user_sender().key)
        .unwrap();
    let card = co
        .create_task(
            &me,
            coord::NewTask {
                title: "their work".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.claim_task(&agent, &card.id).unwrap();
    let s = w
        .cards()
        .into_iter()
        .find(|s| s.id == card.id)
        .expect("the board card is in the grid");
    let who = &s.holder.who;
    assert!(
        who.key == agent && who.name == "otter" && who.live && who.status == "busy",
        "the friend's agent on the card: {who:?}"
    );
    assert!(
        s.holder.state != irori_window::cards::Holding::Gone && s.returned.is_none(),
        "the card reads as nobody on it: {:?}",
        s.holder.state
    );
    let d = w.card(&card.id).unwrap();
    assert_eq!(
        d.summary.holder.who.name, "otter",
        "the card's page names it"
    );

    // Card #268: whose each card is, by person id: who made it, never who
    // holds it (the friend's agent holds the person's card).
    let mine = w.core.person().id;
    assert_eq!(s.owner, mine, "the person made it");
    assert_eq!(d.summary.owner, mine, "the card's page says so too");
    let theirs = co
        .create_task(
            &agent,
            coord::NewTask {
                title: "made by their agent".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let s = w
        .cards()
        .into_iter()
        .find(|s| s.id == theirs.id)
        .expect("their card is in the grid");
    assert_eq!(s.owner, "friend-person", "their agent's card is theirs");
    assert_eq!(w.card(&theirs.id).unwrap().summary.owner, "friend-person");
    let summary = w
        .boards_list()
        .into_iter()
        .find(|b| b.board_id == mlg.board_id)
        .unwrap();
    assert!(
        summary.members.iter().any(|m| m.me && m.person_id == mine),
        "the board's summary lists the person by id: {:?}",
        summary.members
    );
}

/// Card #177: another member's agent is a mention candidate tagged with its
/// board, and a comment on that board's card that mentions it stores its
/// board key (their devices deliver it); nothing is routed here for it.
#[test]
fn another_members_agent_can_be_mentioned_on_its_board() {
    let r = Rig::new();
    let w = &r.w;
    let mlg = w.boards_create("Horton", "horton").unwrap();
    let card_id = "0123456789abcdef0123456789abcdef";
    let agent = format!("{AGENT_ACTOR_PREFIX}{card_id}");
    w.core
        .board_store(&mlg.board_id)
        .unwrap()
        .append(
            &agent,
            kinds::AGENT_CARD,
            &serde_json::json!({"cardId": card_id, "name": "otter", "kind": "claude",
                "personId": "friend-person", "status": "idle"}),
        )
        .unwrap();
    wait("the friend's agent card folds", || {
        w.core.board_agents().iter().any(|a| a.key == agent)
    });
    let c = w
        .mention_candidates()
        .into_iter()
        .find(|c| c.who.key == agent)
        .expect("the friend's agent is a candidate");
    assert_eq!(c.board, mlg.board_id);
    assert_eq!(c.who.name, "otter");
    assert!(
        w.mention_candidates()
            .iter()
            .filter(|c| c.board.is_empty())
            .all(|c| !c.who.key.starts_with(AGENT_ACTOR_PREFIX)),
        "your own agents carry no board"
    );

    let co = w.core.board_coord(&mlg.board_id).unwrap();
    let me = w
        .core
        .board_key(&mlg.board_id, &w.core.user_sender().key)
        .unwrap();
    let card = co
        .create_task(
            &me,
            coord::NewTask {
                title: "shared work".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let res = w
        .comment(
            &card.id,
            "@otter can you look?",
            "wake",
            &[irori_window::ts::Mention {
                key: agent.clone(),
                name: "otter".into(),
                gone: false,
            }],
            &[],
        )
        .unwrap();
    let keys: Vec<&str> = res
        .comment
        .mentions
        .iter()
        .map(|m| m.key.as_str())
        .collect();
    assert_eq!(keys, [agent.as_str()]);
    assert!(
        res.mentioned.iter().all(|o| o.key != agent),
        "this device has no outcome for it: {:?}",
        res.mentioned
    );
    assert!(
        !res.told.to.contains(&agent),
        "nothing is routed here for it: {:?}",
        res.told
    );
    let d = w.card(&card.id).unwrap();
    let item = d
        .thread
        .iter()
        .find(|i| i.text.contains("@otter"))
        .expect("the comment is on the thread");
    assert!(
        item.mentions
            .iter()
            .any(|m| m.who.key == agent && m.who.name == "otter" && !m.gone),
        "{:?}",
        item.mentions
    );
}

/// Card #185: with "My threads" private on a shared board, a thread lives
/// in the person's own log, tagged with the board: only their own agents
/// hear it, its text never enters the board's log (so no other member's
/// device gets it), and naming another member's agent is refused. Shared
/// threads still go in the board's log (#134).
#[test]
fn private_board_threads_stay_in_your_own_log() {
    let r = Rig::new();
    let w = &r.w;
    let b = w.boards_create("Horton", "horton").unwrap();
    let card_id = "fedcba9876543210fedcba9876543210";
    let friend = format!("{AGENT_ACTOR_PREFIX}{card_id}");
    w.core
        .board_store(&b.board_id)
        .unwrap()
        .append(
            &friend,
            kinds::AGENT_CARD,
            &serde_json::json!({"cardId": card_id, "name": "otter", "kind": "claude",
                "personId": "friend-person", "status": "idle"}),
        )
        .unwrap();
    wait("the friend's agent card folds", || {
        w.core.board_agents().iter().any(|a| a.key == friend)
    });
    let co = w.core.board_coord(&b.board_id).unwrap();
    let me = w
        .core
        .board_key(&b.board_id, &w.core.user_sender().key)
        .unwrap();
    let new = |title: &str| {
        co.create_task(
            &me,
            coord::NewTask {
                title: title.into(),
                ..Default::default()
            },
        )
        .unwrap()
    };
    // One card held by my agent alpha, one by the friend's otter.
    let mine = new("my work");
    let alpha = w.core.board_key(&b.board_id, &r.key_a).unwrap();
    co.claim_task(&alpha, &mine.id).unwrap();
    let theirs = new("their work");
    co.claim_task(&friend, &theirs.id).unwrap();

    assert_eq!(w.boards_settings(&b.board_id).unwrap().threads, "shared");
    let s = w.boards_set_threads(&b.board_id, "private").unwrap();
    assert_eq!(s.threads, "private");

    let text = format!(
        "Secret plan: are horton#{} and horton#{} still moving?",
        mine.short, theirs.short
    );
    let res = w.start_thread(&text, &[], &[], &b.board_id).unwrap();
    let id = res.task.id.clone();
    assert_eq!(res.told.to, vec![r.key_a.clone()], "only my agent hears it");
    // It is in my own log, tagged with the board, and not on the board.
    let t = w.core.coord().get_task(&id).expect("in my own log");
    assert_eq!(t.ask.unwrap().board, b.board_id);
    assert!(co.get_task(&id).is_none(), "not in the board's log");
    let board_rows = w
        .core
        .board_store(&b.board_id)
        .unwrap()
        .list(&store::Filter {
            limit: store::MAX_LIST_LIMIT,
            ..store::Filter::default()
        })
        .unwrap();
    assert!(
        board_rows
            .iter()
            .all(|e| !String::from_utf8_lossy(&e.payload).contains("Secret plan")),
        "the thread's text never enters the board's log"
    );
    // The board's page lists it, marked private; your own board's doesn't.
    let on_board = w.threads(&b.board_id);
    let mine_t = on_board
        .iter()
        .find(|x| x.id == id)
        .expect("on the board page");
    assert!(mine_t.private);
    assert!(w.threads("").iter().all(|x| x.id != id));

    // Naming the friend's agent is refused: it would reach their device.
    let err = w
        .start_thread(
            "otter, a word?",
            &[irori_window::ts::Mention {
                key: friend.clone(),
                name: "otter".into(),
                gone: false,
            }],
            &[],
            &b.board_id,
        )
        .unwrap_err();
    assert!(err.contains("another member's"), "{err}");

    // Back to shared: a new thread goes in the board's log, as #134 does.
    w.boards_set_threads(&b.board_id, "shared").unwrap();
    let res = w
        .start_thread("Shared question for everyone", &[], &[], &b.board_id)
        .unwrap();
    assert!(
        co.get_task(&res.task.id).is_some(),
        "a shared thread is on the board"
    );
    let listed = w.threads(&b.board_id);
    assert!(listed.iter().any(|x| x.id == res.task.id && !x.private));
    assert!(
        listed.iter().any(|x| x.id == id && x.private),
        "the private one stays"
    );
}

/// Card #135: a board pins to the sidebar in the person's own log, and
/// unpins; a board that isn't here is refused; card pins are untouched.
#[test]
fn boards_pin_to_the_sidebar() {
    let r = Rig::new();
    let w = &r.w;
    let mlg = w.boards_create("MLG", "mlg").unwrap();
    let site = w.boards_create("Site", "site").unwrap();
    let card = create(&r, "a private card");
    w.pin(&card.id, true).unwrap();
    // #238: each pin says boards:changed, so the sidebar's list reads it
    // without a restart.
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let s = seen.clone();
    w.set_emitter(move |name, _| {
        if name == "boards:changed" {
            s.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    });
    w.boards_pin(&site.board_id, true).unwrap();
    w.boards_pin(&mlg.board_id, true).unwrap();
    assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 2);
    let list = w.boards_list();
    let pin = |id: &str| {
        let b = list.iter().find(|b| b.board_id == id).unwrap();
        (b.pinned, b.pin_rank)
    };
    let (sp, sr) = pin(&site.board_id);
    let (mp, mr) = pin(&mlg.board_id);
    assert!(sp && mp && sr < mr, "pinned in the order pinned: {sr} {mr}");
    w.boards_pin(&site.board_id, false).unwrap();
    assert!(
        !w.boards_list()
            .iter()
            .find(|b| b.board_id == site.board_id)
            .unwrap()
            .pinned
    );
    assert!(w.boards_pin("no-such-board", true).is_err());
    // The board pins don't show as cards, and the card stays pinned.
    let cards = w.cards();
    assert_eq!(cards.len(), 1);
    assert!(cards[0].pinned);
}
