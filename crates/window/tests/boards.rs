//! Go boards_service_test.go on a scratch Core: shared boards in the
//! window's grid and on a card's page, Share and Withdraw, and a comment
//! on a shared card reaching its agent. Agent a (alpha) is the Rig's live
//! session; Go's fakes become real rows.

mod common;

use common::*;
use irori_window::boards::SCOPE_BOARD;
use irori_window::cards::{CardSummary, SCOPE_MINE};
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
    let e = w.comment(&priv_t.id, "more", "none").unwrap_err();
    assert!(
        e.contains("mlg#1"),
        "a comment on the shared private card: {e}"
    );
    // The person comments on the shared card from the window.
    w.comment(&shared_id, "on the board", "none").unwrap();
    let co = w.core.board_coord(&mlg.board_id).unwrap();
    let cs = co.comments(&shared_id).unwrap_or_default();
    assert!(
        cs.len() == 1 && cs[0].by.starts_with("person:"),
        "board comments {cs:?}"
    );
    // A shared card isn't pinned.
    assert!(w.pin(&shared_id, true).is_err());
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
    let res = w.comment(&card.id, "which branch?", "wake").unwrap();
    assert!(
        res.told.to == [r.key_a.clone()] && res.answers.len() == 1 && res.answers[0].key == r.key_a,
        "the comment told {:?}, answers {:?}; want the assigned agent's session",
        res.told,
        res.answers
    );
}

/// Without #44's flow in this build, Invite and Join say so, and no
/// invite is open.
#[test]
fn invites_say_they_arent_built_yet() {
    let r = Rig::new();
    let w = &r.w;
    let mlg = w.boards_create("MLG", "mlg").unwrap();
    let inv = &w.seams.invites;
    assert_eq!(
        inv.invite(&mlg.board_id).unwrap_err(),
        irori_window::invites::NO_INVITES
    );
    assert_eq!(
        inv.join("irori-invite:X").unwrap_err(),
        irori_window::invites::NO_INVITES
    );
    assert!(!inv.status().open());
}
