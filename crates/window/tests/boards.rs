//! Go boards_service_test.go on a scratch Core: shared boards in the
//! window's grid and on a card's page, Share and Withdraw, and a comment
//! on a shared card reaching its agent. Agent a (alpha) is the Rig's live
//! session; Go's fakes become real rows.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use irori_window::boards::SCOPE_BOARD;
use irori_window::cards::{CardSummary, SCOPE_MINE};
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

/// A Rig on loopback iroh with no LAN lookup (the invite tests).
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
