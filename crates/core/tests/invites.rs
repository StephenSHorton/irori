//! Board invites through two Cores (card #44, v0.5-shared-boards §4.1): two
//! made-up people's Cores, each its own first-run person on loopback iroh.
//! The first makes a board and an invite; the second joins from the link
//! and its screen shows five words, which the inviter types. Then
//! the board is on both, and a card written on either reaches the other on
//! `irori/board/1`. No LAN lookup, nothing off 127.0.0.1.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use irori_core::{Core, InviteStatus, devices};
use irori_invite::Ticket;
use irori_net::endpoint::IpMode;
use irori_net::transport::IrohNet;

fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(40),
            "timed out waiting for: {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A Core on loopback iroh with no LAN lookup.
fn core(data: &std::path::Path, claude: &std::path::Path) -> Core {
    let mut o = common::options(data, claude);
    o.pair_net = Some(Arc::new(move |ident, chain, guests| {
        let mut nc = devices::default_net_config(ident, chain, guests);
        nc.ip = IpMode::Loopback;
        nc.lan_config = None;
        nc.gossip = false;
        IrohNet::new(nc)
    }));
    Core::start(o).unwrap()
}

fn new_task(title: &str) -> coord::NewTask {
    coord::NewTask {
        title: title.into(),
        ..Default::default()
    }
}

fn has_task(c: &Core, board: &str, id: &str) -> bool {
    c.board_coord(board)
        .is_ok_and(|co| co.get_task(id).is_some())
}

struct Two {
    a: Core,
    b: Core,
    _dirs: Vec<tempfile::TempDir>,
}

fn two() -> Two {
    let dirs: Vec<tempfile::TempDir> = (0..4).map(|_| tempfile::tempdir().unwrap()).collect();
    let a = core(dirs[0].path(), dirs[1].path());
    let b = core(dirs[2].path(), dirs[3].path());
    Two { a, b, _dirs: dirs }
}

/// Both sides up to the words: returns (inviter's, invitee's) status.
fn to_the_words(t: &Two, board: &str) -> (InviteStatus, InviteStatus) {
    let (ra, rb) = (t.a.pairing_runtime().clone(), t.b.pairing_runtime().clone());
    let st = ra.block_on(t.a.invite(board)).unwrap();
    assert_eq!(st.role, "host");
    assert_eq!(st.status.state, "waiting");
    assert!(!st.expires_at.is_empty());
    let ticket = Ticket::parse(&st.link).unwrap();
    assert_eq!(ticket.board_id, board);
    assert!(!ticket.addrs.is_empty());
    for a in &ticket.addrs {
        let a: std::net::SocketAddr = a.parse().unwrap();
        assert!(a.ip().is_loopback(), "non-loopback address {a} in the link");
    }
    let sb = rb.block_on(t.b.join_invite(&st.link)).unwrap();
    assert_eq!(sb.role, "guest");
    assert_eq!(sb.status.board_id, board);
    wait("both screens show the words", || {
        t.a.invite_status().status.state == "words" && t.b.invite_status().status.state == "words"
    });
    (t.a.invite_status(), t.b.invite_status())
}

#[test]
fn an_invitee_joins_a_board_from_a_link_with_the_words_checked() {
    let t = two();
    let (a, b) = (&t.a, &t.b);
    let board = a.create_board("MLG & co", "mlg").unwrap();
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    {
        let s = seen.clone();
        a.watch_invites(Some(Arc::new(move |st: InviteStatus| {
            s.lock().unwrap().push(st.status.state)
        })));
    }
    let (sa, sb) = to_the_words(&t, &board);
    // The invitee reads the words out; the inviter's screen never shows
    // them, it asks for them.
    assert_eq!(sb.status.words.len(), 5);
    assert!(sa.status.words.is_empty());
    assert_eq!(sa.status.peer_fingerprint, sb.status.my_fingerprint);
    assert_eq!(sb.status.peer_fingerprint, sa.status.my_fingerprint);
    // A second invite waits for this one.
    let ra = a.pairing_runtime().clone();
    assert!(ra.block_on(a.invite(&board)).is_err());

    a.confirm_invite_words(&sb.status.words.join(" ")).unwrap();
    wait("the invite is done on both sides", || {
        a.invite_status().status.state == "done" && b.invite_status().status.state == "done"
    });
    assert!(b.boards().contains(&board));
    assert_eq!(b.board_info(&board).unwrap().name, "MLG & co");
    // The invitee's other devices open it from the private log.
    let joined = b
        .store()
        .list(&store::Filter {
            kinds: vec![store::kinds::BOARD_JOINED.into()],
            ..Default::default()
        })
        .unwrap();
    assert!(
        joined
            .iter()
            .any(|e| String::from_utf8_lossy(&e.payload).contains(&board)),
        "no board.joined for the board in the invitee's private log"
    );
    let states = seen.lock().unwrap().clone();
    for s in ["waiting", "words", "joining", "done"] {
        assert!(
            states.iter().any(|x| x == s),
            "watcher never saw {s}: {states:?}"
        );
    }

    // The board is shared: cards move both ways on its own connection.
    let ta = a
        .board_coord(&board)
        .unwrap()
        .create_task(
            &format!("person:{}", a.identity().current().id),
            new_task("from the inviter"),
        )
        .unwrap();
    a.kick_board(&board);
    wait("the invitee gets the inviter's card", || {
        has_task(b, &board, &ta.id)
    });
    let tb = b
        .board_coord(&board)
        .unwrap()
        .create_task(
            &format!("person:{}", b.identity().current().id),
            new_task("from the invitee"),
        )
        .unwrap();
    b.kick_board(&board);
    wait("the inviter gets the invitee's card", || {
        has_task(a, &board, &tb.id)
    });
    // The private meshes never met.
    assert!(a.net().conns().is_empty() && b.net().conns().is_empty());
    a.close().unwrap();
    b.close().unwrap();
}

#[test]
fn wrong_words_twice_end_the_invite_and_nothing_is_shared() {
    let t = two();
    let (a, b) = (&t.a, &t.b);
    let board = a.create_board("private plans", "pp").unwrap();
    let (_, sb) = to_the_words(&t, &board);
    let mut wrong = sb.status.words.clone();
    wrong.reverse();
    assert!(a.confirm_invite_words(&wrong.join(" ")).is_err());
    assert_eq!(a.invite_status().status.misses, 1);
    assert!(a.confirm_invite_words("not the words at all").is_err());
    wait("the invite fails on both sides", || {
        a.invite_status().status.state == "failed" && b.invite_status().status.state == "failed"
    });
    assert!(!b.boards().contains(&board));
    // A new invite may open now.
    let ra = a.pairing_runtime().clone();
    let again = ra.block_on(a.invite(&board)).unwrap();
    assert_eq!(again.status.state, "waiting");
    a.cancel_invite();
    wait("canceling ends it", || {
        a.invite_status().status.state == "failed"
    });
    a.close().unwrap();
    b.close().unwrap();
}

#[test]
fn a_link_nobody_answers_fails_on_the_invitee() {
    let t = two();
    let (a, b) = (&t.a, &t.b);
    let board = a.create_board("gone", "gone").unwrap();
    let ra = a.pairing_runtime().clone();
    let link = ra.block_on(a.invite(&board)).unwrap().link;
    a.cancel_invite();
    let rb = b.pairing_runtime().clone();
    rb.block_on(b.join_invite(&link)).unwrap();
    wait("the invitee gives up", || {
        b.invite_status().status.state == "failed"
    });
    assert!(!b.invite_status().status.error.is_empty());
    assert!(!b.boards().contains(&board));
    a.close().unwrap();
    b.close().unwrap();
}
