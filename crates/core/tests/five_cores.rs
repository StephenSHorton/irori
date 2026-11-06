//! The five-Core harness (v0.5-shared-boards §11, card #48): two made-up
//! people on loopback iroh, through the real Core API. Alice has three
//! devices (a1, a2, a3), Bob two (b1, b2); each person's devices are paired
//! as pairing does, and Bob joins Alice's board from an invite link with
//! the words checked (card #44). One test per flow, so a failure names
//! itself; each builds only the Cores its flow needs. A flow that waits on
//! work not built yet is `#[ignore]`d with what it waits on.
//!
//! No home relay here: the Cores find each other by LAN lookup unicast to
//! 127.0.0.1 and by the invite link's direct addresses. The relay path is
//! the live run's (card #48 s3). Made-up people and fixture sessions only.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::mesh::{Dev, invited, person, restart};
use common::wait_for;
use store::Filter;
use store::kinds;

const WAIT: u64 = 45;

fn new_task(title: &str) -> coord::NewTask {
    coord::NewTask {
        title: title.into(),
        ..Default::default()
    }
}

fn card_on(d: &Dev, board: &str, id: &str) -> Option<coord::Task> {
    d.core.board_coord(board).ok()?.get_task(id)
}

fn close(devs: impl IntoIterator<Item = Dev>) {
    for d in devs {
        d.core.close().unwrap();
    }
}

/// Bob joins from a link; cards move both ways on the board's own
/// connection; neither private mesh ever admits the other person, while
/// Alice's own devices stay connected to each other.
#[test]
fn invite_rows_both_ways_and_the_private_mesh_stays_private() {
    let mut alice = person("alice", 2);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    let aco = a1.core.board_coord(&board).unwrap();
    let ta = aco
        .create_task(&a1.person_actor(), new_task("from alice"))
        .unwrap();
    a1.core.kick_board(&board);
    wait_for(WAIT, "bob gets alice's card", || {
        card_on(b1, &board, &ta.id).is_some()
    });
    let bco = b1.core.board_coord(&board).unwrap();
    let tb = bco
        .create_task(&b1.person_actor(), new_task("from bob"))
        .unwrap();
    b1.core.kick_board(&board);
    wait_for(WAIT, "alice gets bob's card", || {
        card_on(a1, &board, &tb.id).is_some()
    });
    // Board connections only with the other person's device; the private
    // mesh only with one's own.
    let a2 = alice[1].core.device_id().to_string();
    let b1_dev = b1.core.device_id().to_string();
    wait_for(WAIT, "a1 and a2 stay connected", || {
        a1.core.net().conns().iter().any(|c| c.peer == a2)
    });
    assert!(
        a1.core.net().conns().iter().all(|c| c.peer != b1_dev),
        "bob's device reached alice's private mesh"
    );
    assert!(
        b1.core
            .net()
            .conns()
            .iter()
            .all(|c| c.peer != a1.core.device_id()),
        "alice's device reached bob's private mesh"
    );
    assert!(
        a1.core
            .net()
            .board_conns(&board)
            .iter()
            .any(|c| c.peer == b1_dev)
    );
    // No board row in either private log.
    for d in [a1, b1] {
        let evs = d
            .core
            .store()
            .list(&Filter {
                kinds: vec![kinds::TASK_CREATED.into(), "member.".into()],
                limit: 1000,
                ..Default::default()
            })
            .unwrap();
        assert!(
            evs.iter().all(|e| {
                let s = String::from_utf8_lossy(&e.payload);
                !s.contains(&ta.id) && !s.contains(&tb.id) && !e.kind.starts_with("member.")
            }),
            "a board row reached {}'s private log",
            d.core.device_id()
        );
    }
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §7.1-7.2 across people: Bob's agent joins Alice's shared card; Alice's
/// comment reaches it as a bridged message from a board: sender, untrusted
/// and fenced with the board; Bob's Ask me holds Alice's wake request; the
/// agent's reply goes on the card, where Alice's Core reads it.
#[test]
fn the_bridge_carries_alices_comment_to_bobs_agent_and_the_reply_back() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    let aco = a1.core.board_coord(&board).unwrap();
    let t = aco
        .create_task(&a1.person_actor(), new_task("tune the loader"))
        .unwrap();
    a1.core.kick_board(&board);
    wait_for(WAIT, "bob gets the card", || {
        card_on(b1, &board, &t.id).is_some()
    });
    let out = b1.tool("board_join", serde_json::json!({"task_id": "mlg#1"}));
    assert!(!out.starts_with("Error"), "board_join: {out}");
    b1.core.kick_board(&board);
    wait_for(WAIT, "alice sees bob's agent on the card", || {
        card_on(a1, &board, &t.id).is_some_and(|x| !x.holders.is_empty())
    });

    aco.comment_on(
        &a1.person_actor(),
        &t.id,
        coord::CommentInput {
            text: "the decoder is the slow part".into(),
            ..Default::default()
        },
    )
    .unwrap();
    a1.core.kick_board(&board);
    // Every delivery to the agent, held ones too (for_agent leaves those out).
    let bridged = |d: &Dev| {
        d.core
            .inbox()
            .list(0)
            .into_iter()
            .filter(|x| x.to == d.agent && x.message.from.key.starts_with("board:"))
            .collect::<Vec<_>>()
    };
    wait_for(WAIT, "bob's agent hears alice's comment", || {
        !bridged(b1).is_empty()
    });
    let d = bridged(b1).remove(0);
    assert_eq!(d.message.trust, inbox::trust::UNTRUSTED);
    assert_eq!(d.message.board.board_id, board);
    assert_eq!(d.message.board.tag, "mlg");
    assert_eq!(d.message.board.person_id, a1.core.person().id);
    assert!(d.message.text.contains("decoder"), "{:?}", d.message.text);
    assert_eq!(d.message.ref_, format!("task:{}", t.id));

    // Ask me: Alice's request to wake is held for Bob.
    b1.core
        .inbox()
        .set_receive(
            inbox::ReceiveSetting {
                board_id: board.clone(),
                preset: inbox::receive::ASK.into(),
                ..Default::default()
            },
            &b1.user(),
        )
        .unwrap();
    aco.comment_on(
        &a1.person_actor(),
        &t.id,
        coord::CommentInput {
            text: "can you look now?".into(),
            wake: true,
            ..Default::default()
        },
    )
    .unwrap();
    a1.core.kick_board(&board);
    wait_for(WAIT, "the wake request is held for bob", || {
        bridged(b1)
            .iter()
            .any(|x| x.state == inbox::st::HELD && x.hold == inbox::hold::MEMBER_WAKE)
    });

    // The agent's reply goes on the card and reaches Alice.
    let out = b1.tool(
        "reply",
        serde_json::json!({"delivery_id": d.id, "text": "the AV1 one"}),
    );
    assert!(out.contains("went on the card"), "reply: {out}");
    b1.core.kick_board(&board);
    wait_for(WAIT, "alice reads the reply on the card", || {
        aco.comments(&t.id)
            .unwrap_or_default()
            .iter()
            .any(|c| c.text == "the AV1 one")
    });
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §5.5: Alice's agent attaches a file to a comment on a shared card; it is
/// offered in the board's log and Bob's agent fetches it from Alice's
/// device on the board connection. A file Alice keeps private never
/// reaches Bob, even when Bob offers its hash on the board himself and
/// names it in a comment.
#[test]
fn a_file_on_a_shared_card_reaches_bob_and_a_private_one_never_does() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    let aco = a1.core.board_coord(&board).unwrap();
    let t = aco
        .create_task(&a1.person_actor(), new_task("look at the trace"))
        .unwrap();
    let files = tempfile::tempdir().unwrap();
    let shared = files.path().join("trace.txt");
    std::fs::write(&shared, b"frame 12 took 40ms").unwrap();
    let out = a1.tool(
        "board_comment",
        serde_json::json!({
            "task_id": "mlg#1",
            "text": "the trace",
            "attach": [shared.to_string_lossy()],
        }),
    );
    assert!(!out.starts_with("Error"), "board_comment: {out}");
    let sha = aco.comments(&t.id).unwrap()[0].attachments[0]
        .sha256
        .clone();
    let offered = |d: &Dev, sha: &str| {
        mesh::files::holders_in(&d.core.board_store(&board).unwrap(), sha).unwrap()
    };
    assert_eq!(offered(a1, &sha), vec![a1.core.device_id().to_string()]);
    a1.core.kick_board(&board);
    wait_for(WAIT, "bob gets the comment", || {
        b1.core
            .board_coord(&board)
            .ok()
            .and_then(|c| c.comments(&t.id))
            .is_some_and(|cs| !cs.is_empty())
    });
    let (path, from) = b1.core.get_attachment(&b1.agent, &sha, "").unwrap();
    assert_eq!(from, a1.core.device_id());
    assert_eq!(std::fs::read(&path).unwrap(), b"frame 12 took 40ms");
    // Bob's device offers it on the board now, so it serves it on.
    assert!(offered(b1, &sha).contains(&b1.core.device_id().to_string()));
    // A blocker's file is offered on the board the same way.
    let log = files.path().join("build.log");
    std::fs::write(&log, b"link error").unwrap();
    let out = a1.tool("board_join", serde_json::json!({"task_id": "mlg#1"}));
    assert!(!out.starts_with("Error"), "board_join: {out}");
    let out = a1.tool(
        "board_block",
        serde_json::json!({
            "task_id": "mlg#1",
            "question": "which build do we ship?",
            "attach": [log.to_string_lossy()],
        }),
    );
    assert!(!out.starts_with("Error"), "board_block: {out}");
    let bsha = aco.get_task(&t.id).unwrap().blockers[0].attachments[0]
        .sha256
        .clone();
    assert_eq!(offered(a1, &bsha), vec![a1.core.device_id().to_string()]);

    // A file Alice keeps private: in her attachment store, offered only to
    // her own devices.
    let private = files.path().join("keys.txt");
    std::fs::write(&private, b"alice only").unwrap();
    let pr = a1
        .core
        .attach_files(&a1.agent, &[private.to_string_lossy().into_owned()])
        .unwrap()
        .remove(0);
    assert!(offered(a1, &pr.sha256).is_empty());
    // Bob knows its hash: he offers it on the board and names it in a
    // comment (closure lets him, as he offered it).
    let bst = b1.core.board_store(&board).unwrap();
    bst.append(
        "irori",
        kinds::ATTACH_OFFERED,
        &serde_json::json!({"sha256": pr.sha256, "size": pr.size, "name": "keys.txt"}),
    )
    .unwrap();
    let bco = b1.core.board_coord(&board).unwrap();
    bco.comment_on(
        &b1.person_actor(),
        &t.id,
        coord::CommentInput {
            text: "send me this one".into(),
            files: vec![coord::FileRef {
                sha256: pr.sha256.clone(),
                name: "keys.txt".into(),
                size: pr.size,
                mime: String::new(),
                device: a1.core.device_id().to_string(),
            }],
            ..Default::default()
        },
    )
    .unwrap();
    b1.core.kick_board(&board);
    wait_for(WAIT, "alice gets bob's offer", || {
        offered(a1, &pr.sha256).contains(&b1.core.device_id().to_string())
    });
    let got = b1.core.get_attachment(&b1.agent, &pr.sha256, "");
    assert!(got.is_err(), "bob got alice's private file: {got:?}");
    assert!(!b1.core.files().has(&pr.sha256));
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §5.2-5.3: Alice shares a private card; Bob gets the shared copy; Alice
/// withdraws it and Bob's board no longer shows it.
#[test]
fn share_then_withdraw_reaches_bob() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    let p = a1
        .core
        .coord()
        .create_task(&a1.user(), new_task("private, then shared"))
        .unwrap();
    let pv = a1.core.preview_share(&p.id, &board).unwrap();
    assert!(pv.visible_to.len() >= 2, "visible to {:?}", pv.visible_to);
    let shared = a1.core.share_to_board(&p.id, &board, &pv.hash).unwrap();
    a1.core.kick_board(&board);
    wait_for(WAIT, "bob gets the shared copy", || {
        card_on(b1, &board, &shared.id).is_some()
    });
    a1.core.withdraw_card(&shared.id).unwrap();
    a1.core.kick_board(&board);
    wait_for(WAIT, "bob sees it withdrawn", || {
        card_on(b1, &board, &shared.id).is_some_and(|t| t.withdrawn)
    });
    let views = b1.core.board_views(&coord::BoardFilter {
        viewer: b1.user(),
        done_limit: 50,
        ..Default::default()
    });
    let shown = views.iter().any(|v| {
        v.open
            .iter()
            .chain(&v.working)
            .chain(&v.done)
            .any(|c| c.task.id == shared.id)
    });
    assert!(!shown, "bob's board still shows the withdrawn card");
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §10: Alice asks Bob's agent to review a PR on the shared card; the
/// request reaches the agent through the bridge.
#[test]
fn a_review_request_reaches_bobs_agent() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    let aco = a1.core.board_coord(&board).unwrap();
    let t = aco
        .create_task(&a1.person_actor(), new_task("loader rewrite"))
        .unwrap();
    aco.add_link(
        &a1.person_actor(),
        &t.id,
        "https://github.com/example-org/example-app/pull/7",
    )
    .unwrap();
    a1.core.kick_board(&board);
    wait_for(WAIT, "bob gets the card", || {
        card_on(b1, &board, &t.id).is_some()
    });
    let out = b1.tool("board_watch", serde_json::json!({"task_id": "mlg#1"}));
    assert!(!out.starts_with("Error"), "board_watch: {out}");
    let card = b1.core.board_key(&board, &b1.agent).unwrap();
    b1.core.kick_board(&board);
    wait_for(WAIT, "alice knows bob's agent's card", || {
        card_on(a1, &board, &t.id).is_some_and(|x| x.watchers.contains(&card))
    });
    // Bob dedicates his agent as his reviewer in the app (§10).
    b1.core.set_board_reviewer(&board, &b1.agent, "").unwrap();
    let rv = b1.core.board_reviewers(&board);
    assert_eq!(rv.len(), 1, "{rv:?}");
    assert_eq!(
        (rv[0].repo.as_str(), rv[0].agent.as_str()),
        ("", b1.agent.as_str())
    );
    b1.core.kick_board(&board);
    wait_for(WAIT, "alice's board knows bob's reviewer", || {
        aco.reviewer(&b1.core.person().id, "") == card
    });
    // Alice's agent asks without naming anyone: the row names Bob's
    // dedicated reviewer, so the bridge can deliver it.
    let out = a1.tool("board_join", serde_json::json!({"task_id": "mlg#1"}));
    assert!(!out.starts_with("Error"), "board_join: {out}");
    let out = a1.tool(
        "board_review",
        serde_json::json!({"task_id": "mlg#1", "pr": "https://github.com/example-org/example-app/pull/7"}),
    );
    assert!(out.starts_with("Asked"), "board_review: {out}");
    assert!(out.contains("Visible to"), "board_review: {out}");
    assert!(
        aco.reviews(&t.id).iter().all(|r| r.to == card),
        "{:?}",
        aco.reviews(&t.id)
    );
    a1.core.kick_board(&board);
    wait_for(WAIT, "bob's agent gets the review request", || {
        b1.core
            .inbox()
            .for_agent(&b1.agent, true, 0)
            .iter()
            .any(|d| d.message.from.key.starts_with("board:") && d.message.text.contains("review"))
    });
    // Alice's own agent is never asked to approve.
    let a_card = a1.core.board_key(&board, &a1.agent).unwrap();
    let out = a1.tool(
        "board_review",
        serde_json::json!({"task_id": "mlg#1", "pr": "https://github.com/example-org/example-app/pull/7", "to": a_card}),
    );
    assert!(out.starts_with("Error"), "own agent asked: {out}");
    close(alice.drain(..).chain(bob.drain(..)));
}

/// A device that isn't a member's never gets a board connection: Carol
/// opens a board with the same id but her own secret, and Alice's Core
/// refuses her proof.
#[test]
fn a_non_member_is_refused() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let mut carol = person("carol", 1);
    let (a1, b1, c1) = (&alice[0], &bob[0], &carol[0]);
    let board = invited(a1, b1);
    // Carol knows the board id (say, from a forwarded link) but holds no
    // secret for it: she makes her own board with that id's files.
    let dir = c1.data.join(irori_core::SPACES_DIR);
    std::fs::create_dir_all(&dir).unwrap();
    let pin = irori_identity::MemberPin {
        person_id: c1.core.person().id.clone(),
        person_pub: c1.core.identity().current().pub_hex.clone(),
        root_device_id: c1.core.device_id().into(),
        person_epoch: 1,
    };
    let verifier = Arc::new(irori_identity::BoardVerifier::new_for(
        &board,
        std::slice::from_ref(&pin),
    ));
    let log = Arc::new(
        store::Store::open_with(
            dir.join(format!("{board}.db")),
            c1.core.device_id(),
            Arc::new(store::Clock::new()),
            store::OpenOptions {
                signer: Some(c1.core.identity().clone()),
                verifier: Some(verifier.clone()),
                board: true,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let secrets = Arc::new(
        irori_identity::BoardSecrets::open(
            &dir.join(format!("{board}.secrets")),
            &board,
            Box::new(irori_identity::FileKeystore),
        )
        .unwrap(),
    );
    let mut s = [0u8; 32];
    getrandom::fill(&mut s).unwrap();
    secrets.add(1, c1.core.device_id(), &s).unwrap();
    let rc = c1.core.pairing_runtime().clone();
    rc.block_on(c1.core.open_board_net(irori_core::BoardParts {
        board_id: board.clone(),
        person_id: pin.person_id.clone(),
        log,
        verifier,
        secrets,
    }))
    .unwrap();
    let a_addrs = a1.core.net().endpoint().expect("bound").bound_sockets();
    c1.core.board_addr_hint(a1.core.device_id(), a_addrs);
    c1.core.kick_board(&board);
    std::thread::sleep(Duration::from_secs(5));
    let c_dev = c1.core.device_id().to_string();
    assert!(
        a1.core
            .net()
            .board_conns(&board)
            .iter()
            .all(|c| c.peer != c_dev),
        "carol got a board connection to alice"
    );
    assert!(a1.core.net().conns().iter().all(|c| c.peer != c_dev));
    close(alice.drain(..).chain(bob.drain(..)).chain(carol.drain(..)));
}

/// §4.5, v0.4 revoke: Alice removes a2; Bob's copy of the board cuts a2.
#[test]
fn alice_removes_a_device_and_bobs_board_cuts_it() {
    let mut alice = person("alice", 2);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    let a2 = alice[1].core.device_id().to_string();
    let bv = b1.core.board_verifier(&board).unwrap();
    wait_for(WAIT, "bob's board knows a2's cert", || {
        bv.cert(&a2).is_some()
    });
    let ra = a1.core.pairing_runtime().clone();
    ra.block_on(a1.core.remove_device(&a2)).unwrap();
    a1.core.kick_board(&board);
    wait_for(WAIT, "bob's board cuts a2", || bv.cut(&a2).is_some());
    let e1 = a1.core.board_info(&board).unwrap().epoch;
    wait_for(WAIT, "the board secret moves on for bob too", || {
        b1.core.board_info(&board).unwrap().secret_epoch > 1
            && b1.core.board_info(&board).unwrap().epoch == e1
    });
    close(alice.drain(..).chain(bob.drain(..)));
}

/// The full rig: Alice on three devices and Bob on two. Every device of
/// both people ends up with the board, and a card from a3 reaches b2.
#[test]
fn five_cores_every_device_gets_the_board() {
    let mut alice = person("alice", 3);
    let mut bob = person("bob", 2);
    let board = invited(&alice[0], &bob[0]);
    for d in alice.iter().chain(bob.iter()) {
        wait_for(
            WAIT,
            &format!("{} opens the board", d.core.device_id()),
            || d.core.board_coord(&board).is_ok(),
        );
    }
    let a3 = &alice[2];
    let alice_pid = a3.core.person().id.clone();
    wait_for(WAIT, "a3's copy of the board has alice as a member", || {
        a3.core
            .board_verifier(&board)
            .is_some_and(|v| v.membership().active(&alice_pid))
    });
    let t = a3
        .core
        .board_coord(&board)
        .unwrap()
        .create_task(&a3.person_actor(), new_task("from a3"))
        .unwrap();
    a3.core.kick_board(&board);
    wait_for(WAIT, "b2 gets a3's card", || {
        card_on(&bob[1], &board, &t.id).is_some()
    });
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §4.4: Bob leaves. Alice's board shows him gone and cuts his device;
/// Bob's board is gone from his views at once and its file stays only
/// until Alice's device says it holds the `left` row, then Bob forgets it.
#[test]
fn bob_leaves_and_alice_refuses_his_new_rows() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    let av = a1.core.board_verifier(&board).unwrap();
    let (bob_pid, b1_dev) = (b1.core.person().id.clone(), b1.core.device_id().to_string());
    wait_for(WAIT, "alice's board has bob", || {
        av.membership().active(&bob_pid)
    });
    b1.core.leave_board(&board).unwrap();
    assert!(
        !b1.core.board_list().iter().any(|s| s.board_id == board),
        "bob still sees the board he left"
    );
    wait_for(WAIT, "alice's board shows bob gone", || {
        !av.membership().active(&bob_pid)
    });
    assert_eq!(
        av.membership().person(&bob_pid).map(|p| p.state.as_str()),
        Some("left")
    );
    assert!(av.cut(&b1_dev).is_some(), "alice doesn't cut bob's device");
    wait_for(
        WAIT,
        "bob forgets the board once alice holds his leave",
        || !b1.core.boards().contains(&board),
    );
    assert!(a1.core.boards().contains(&board), "alice lost the board");
    close(alice.drain(..).chain(bob.drain(..)));
}

/// Card #304: Bob leaves while Alice is offline. The board is gone from
/// every one of Bob's views on both his devices at once, and its file
/// stays out of sight; when Alice comes back, she hears he left and Bob's
/// devices forget the board.
#[test]
fn bob_leaves_while_alice_is_offline_and_she_hears_it_later() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 2);
    let (a1, b1, b2) = (&alice[0], &bob[0], &bob[1]);
    let board = invited(a1, b1);
    let av = a1.core.board_verifier(&board).unwrap();
    let bob_pid = b1.core.person().id.clone();
    wait_for(WAIT, "alice's board has bob", || {
        av.membership().active(&bob_pid)
    });
    wait_for(WAIT, "b2 opens the board", || {
        b2.core
            .board_list()
            .iter()
            .any(|s| s.board_id == board && s.ready)
    });
    let t = a1
        .core
        .board_coord(&board)
        .unwrap()
        .create_task(&a1.person_actor(), new_task("shared work"))
        .unwrap();
    a1.core.kick_board(&board);
    wait_for(WAIT, "bob gets the card", || {
        card_on(b1, &board, &t.id).is_some()
    });

    // Alice goes offline; Bob leaves.
    a1.core.close_board_net(&board);
    b1.core.leave_board(&board).unwrap();
    let shown = |d: &Dev| d.core.board_list().iter().any(|s| s.board_id == board);
    assert!(!shown(b1), "b1 still lists the board bob left");
    assert!(
        b1.core
            .board_views(&coord::BoardFilter::default())
            .is_empty(),
        "b1's tools still see the board's cards"
    );
    wait_for(WAIT, "b2 hides the board too", || !shown(b2));
    // Kept out of sight while nobody else has the leave.
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        b1.core.boards().contains(&board),
        "b1 forgot before alice heard"
    );
    assert!(
        av.membership().active(&bob_pid),
        "alice heard while offline"
    );

    // Alice comes back: she hears Bob left, and his devices forget it.
    a1.core.reopen_board_net(&board).unwrap();
    wait_for(WAIT, "alice's board shows bob gone", || {
        !av.membership().active(&bob_pid)
    });
    for d in [b1, b2] {
        wait_for(
            WAIT,
            &format!("{} forgets the board", d.core.device_id()),
            || !d.core.boards().contains(&board),
        );
    }
    assert!(!shown(b1) && a1.core.boards().contains(&board));
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §4.4 Forget now: Bob leaves and forgets at once, without waiting.
#[test]
fn bob_leaves_and_forgets_now() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    let av = a1.core.board_verifier(&board).unwrap();
    let bob_pid = b1.core.person().id.clone();
    wait_for(WAIT, "alice's board has bob", || {
        av.membership().active(&bob_pid)
    });
    // The board's only owner can't leave it to the others.
    assert!(a1.core.leave_board(&board).is_err());
    b1.core.leave_board(&board).unwrap();
    b1.core.forget_left_board(&board).unwrap();
    assert!(!b1.core.boards().contains(&board));
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §4.3: Alice removes Bob while his agent holds a card. Alice's board
/// takes the agent off and cuts Bob's device, the board secret moves on,
/// Bob's connection closes, and a card Bob writes after never reaches
/// Alice.
#[test]
fn alice_removes_bob_and_his_agent_comes_off() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    let (bob_pid, b1_dev) = (b1.core.person().id.clone(), b1.core.device_id().to_string());
    let av = a1.core.board_verifier(&board).unwrap();
    wait_for(WAIT, "alice's board has bob", || {
        av.membership().active(&bob_pid)
    });
    let t = a1
        .core
        .board_coord(&board)
        .unwrap()
        .create_task(&a1.person_actor(), new_task("shared work"))
        .unwrap();
    a1.core.kick_board(&board);
    wait_for(WAIT, "bob gets the card", || {
        card_on(b1, &board, &t.id).is_some()
    });
    b1.tool("board_join", serde_json::json!({"task_id": t.id}));
    b1.core.kick_board(&board);
    wait_for(WAIT, "alice sees bob's agent on the card", || {
        card_on(a1, &board, &t.id).is_some_and(|c| !c.holders.is_empty())
    });
    let e0 = a1.core.board_info(&board).unwrap().secret_epoch;

    // Only an owner removes, and nobody removes themselves.
    assert!(
        b1.core
            .remove_member(&board, &a1.core.person().id, "")
            .is_err()
    );
    assert!(
        a1.core
            .remove_member(&board, &a1.core.person().id, "")
            .is_err()
    );
    a1.core.remove_member(&board, &bob_pid, "").unwrap();
    assert_eq!(
        av.membership().person(&bob_pid).map(|p| p.state.as_str()),
        Some("removed")
    );
    assert!(av.cut(&b1_dev).is_some());
    let c = card_on(a1, &board, &t.id).unwrap();
    assert!(
        c.holders.is_empty() && c.state == "submitted",
        "bob's agent is still on the card: {:?} {}",
        c.holders,
        c.state
    );
    wait_for(WAIT, "the board secret moves on", || {
        a1.core.board_info(&board).unwrap().secret_epoch > e0
    });
    wait_for(WAIT, "bob's board connection to alice closes", || {
        a1.core
            .net()
            .board_conns(&board)
            .iter()
            .all(|c| c.peer != b1_dev)
    });
    wait_for(WAIT, "bob's copy shows the removal", || {
        b1.core
            .board_verifier(&board)
            .is_some_and(|v| !v.membership().active(&bob_pid))
    });
    let line = b1
        .core
        .board_list()
        .into_iter()
        .find(|s| s.board_id == board)
        .map(|s| s.warnings)
        .unwrap_or_default();
    assert!(
        line.first()
            .is_some_and(|l| l.starts_with("You're no longer on this board")),
        "bob's board line: {line:?}"
    );
    assert!(
        a1.core
            .board_list()
            .iter()
            .find(|s| s.board_id == board)
            .is_some_and(|s| s.warnings.is_empty()),
        "alice's board warns"
    );
    if let Ok(co) = b1.core.board_coord(&board)
        && let Ok(late) = co.create_task(&b1.person_actor(), new_task("after removal"))
    {
        b1.core.kick_board(&board);
        std::thread::sleep(Duration::from_secs(3));
        assert!(
            card_on(a1, &board, &late.id).is_none(),
            "a card bob wrote after his removal reached alice"
        );
    }
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §4.2: Alice makes Bob an owner, who then renames the board and can
/// change roles; the last owner can't stop being one.
#[test]
fn alice_makes_bob_an_owner() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    let (alice_pid, bob_pid) = (a1.core.person().id.clone(), b1.core.person().id.clone());
    wait_for(WAIT, "alice's board has bob", || {
        a1.core
            .board_verifier(&board)
            .is_some_and(|v| v.membership().active(&bob_pid))
    });
    assert!(
        a1.core
            .set_member_role(&board, &alice_pid, "member")
            .is_err()
    );
    assert!(b1.core.set_member_role(&board, &bob_pid, "owner").is_err());
    assert!(a1.core.set_member_role(&board, &bob_pid, "admin").is_err());
    a1.core.set_member_role(&board, &bob_pid, "owner").unwrap();
    a1.core.kick_board(&board);
    let bv = b1.core.board_verifier(&board).unwrap();
    wait_for(WAIT, "bob is an owner on his copy", || {
        bv.membership().owner(&bob_pid)
    });
    b1.core.rename_board(&board, "MLG & Bob").unwrap();
    b1.core.kick_board(&board);
    wait_for(WAIT, "alice sees bob's rename", || {
        a1.core.board_info(&board).unwrap().name == "MLG & Bob"
    });
    // Two owners: Alice may step down now, and then may not change roles.
    a1.core
        .set_member_role(&board, &alice_pid, "member")
        .unwrap();
    assert!(a1.core.set_member_role(&board, &bob_pid, "member").is_err());
    assert!(a1.core.board_info(&board).unwrap().conflicts.is_empty());
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §5.4, §7.3: Bob's agent's lease in the shared card's repo is mirrored
/// to the board; Alice's agent can't take or steal it and asks for it with
/// a release request, which reaches Bob's agent through the bridge. Under
/// Alice's Autonomous preset her agent's lease in that repo mirrors before
/// it works the card.
#[test]
fn a_mirrored_lease_refuses_alice_and_her_release_request_reaches_bob() {
    const REPO: &str = "remote:github.com/example-org/example-app";
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    let aco = a1.core.board_coord(&board).unwrap();
    let t = aco
        .create_task(
            &a1.person_actor(),
            coord::NewTask {
                title: "loader rewrite".into(),
                repo_key: REPO.into(),
                shared: true,
                ..Default::default()
            },
        )
        .unwrap();
    a1.core.kick_board(&board);
    wait_for(WAIT, "bob gets the card", || {
        card_on(b1, &board, &t.id).is_some()
    });

    // Autonomous: Alice's agent holds no shared card yet, but its lease in
    // the shared card's repo mirrors all the same.
    a1.core
        .inbox()
        .set_autonomy(
            inbox::AutonomySetting {
                board_id: board.clone(),
                preset: inbox::autonomy::AUTONOMOUS.into(),
            },
            &a1.user(),
        )
        .unwrap();
    let docs = format!("path:{REPO}:docs/**");
    let out = a1.tool("lease", serde_json::json!({"resource": docs}));
    assert!(out.contains("Mirrored on mlg"), "autonomous lease: {out}");
    let out = a1.tool("release", serde_json::json!({"resource": docs}));
    assert!(out.starts_with("Released"), "release: {out}");
    assert!(
        !aco.leases().iter().any(|l| l.resource == docs),
        "the mirror outlived the release"
    );

    let out = b1.tool("board_join", serde_json::json!({"task_id": "mlg#1"}));
    assert!(!out.starts_with("Error"), "board_join: {out}");
    let src = format!("path:{REPO}:src/**");
    let out = b1.tool("lease", serde_json::json!({"resource": src}));
    assert!(out.contains("Mirrored on mlg"), "bob's lease: {out}");
    b1.core.kick_board(&board);
    wait_for(WAIT, "alice's board shows bob's agent's lease", || {
        aco.leases().iter().any(|l| l.resource == src)
    });

    let out = a1.tool("board_join", serde_json::json!({"task_id": "mlg#1"}));
    assert!(!out.starts_with("Error"), "board_join: {out}");
    for steal in [false, true] {
        let out = a1.tool(
            "lease",
            serde_json::json!({"resource": format!("path:{REPO}:src/loader/**"), "steal": steal}),
        );
        assert!(
            out.contains("another person's") && out.contains("release request"),
            "steal={steal}: {out}"
        );
    }
    assert!(
        !a1.core
            .coord()
            .leases()
            .iter()
            .any(|l| l.resource.contains("src/loader")),
        "a refused board lease left a private one"
    );

    let out = a1.tool(
        "release",
        serde_json::json!({"resource": src, "reason": "the loader fix is mine"}),
    );
    assert!(out.contains("Asked its holder"), "release request: {out}");
    assert!(out.contains("Visible to"), "release request: {out}");
    a1.core.kick_board(&board);
    wait_for(WAIT, "bob's agent hears the release request", || {
        b1.core.inbox().list(0).iter().any(|d| {
            d.to == b1.agent
                && d.message.from.key.starts_with("board:")
                && d.message.text.contains(coord::RELEASE_REQUEST)
                && d.message.text.contains("the loader fix is mine")
        })
    });
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §2.4: an agent's card on the board follows its session: idle, active
/// while busy, gone once it ends, as the other person's board reads it.
#[test]
fn bobs_agent_card_status_follows_his_session() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    let t = a1
        .core
        .board_coord(&board)
        .unwrap()
        .create_task(&a1.person_actor(), new_task("status"))
        .unwrap();
    a1.core.kick_board(&board);
    wait_for(WAIT, "bob gets the card", || {
        card_on(b1, &board, &t.id).is_some()
    });
    let out = b1.tool("board_watch", serde_json::json!({"task_id": "mlg#1"}));
    assert!(!out.starts_with("Error"), "board_watch: {out}");
    let card = b1.core.board_key(&board, &b1.agent).unwrap();
    let id = card
        .trim_start_matches(kinds::AGENT_ACTOR_PREFIX)
        .to_string();
    let status_on_alice = || {
        let st = a1.core.board_store(&board)?;
        let rows = st
            .list(&Filter {
                kinds: vec![kinds::AGENT_CARD.into()],
                ..Default::default()
            })
            .ok()?;
        rows.iter()
            .rev()
            .filter_map(|ev| serde_json::from_slice::<serde_json::Value>(&ev.payload).ok())
            .find(|v| v["cardId"] == id.as_str())
            .map(|v| v["status"].as_str().unwrap_or_default().to_string())
    };
    let wait_status = |want: &str| {
        b1.core.kick_board(&board);
        wait_for(WAIT, &format!("alice reads bob's agent as {want}"), || {
            b1.core.kick_board(&board);
            status_on_alice().as_deref() == Some(want)
        });
    };
    wait_status("idle");

    let file = b1._dirs[1].path().join("sessions").join("900001.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    v["status"] = "busy".into();
    std::fs::write(&file, serde_json::to_vec(&v).unwrap()).unwrap();
    wait_status("active");

    std::fs::remove_file(&file).unwrap();
    wait_status("gone");
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §9: Alice asks the board's room; Bob's free agent hears it through his
/// device's bridge, woken, and takes it with ask_claim, decided on Alice's
/// device.
#[test]
fn a_board_ask_reaches_bob() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    let aco = a1.core.board_coord(&board).unwrap();
    // Bob's agent gets a card on the board by watching one (§2.4), so
    // Alice's board knows it as a free agent.
    let t = aco
        .create_task(&a1.person_actor(), new_task("shared work"))
        .unwrap();
    a1.core.kick_board(&board);
    wait_for(WAIT, "bob gets the card", || {
        card_on(b1, &board, &t.id).is_some()
    });
    let out = b1.tool("board_watch", serde_json::json!({"task_id": "mlg#1"}));
    assert!(!out.starts_with("Error"), "board_watch: {out}");
    let card = b1.core.board_key(&board, &b1.agent).unwrap();
    b1.core.kick_board(&board);
    wait_for(WAIT, "alice's board has bob's agent idle", || {
        b1.core.kick_board(&board);
        a1.core
            .board_roster(&board)
            .iter()
            .any(|a| a.key == card && a.status == "idle")
    });

    // A board's agents carry no repo: a board ask names none.
    let e = a1
        .core
        .board_ask(
            &board,
            coord::NewAsk {
                text: "Who can look at the flaky sync test?".into(),
                repo_key: "remote:github.com/example-org/example-app".into(),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(e.to_string().contains("repo"), "{e}");
    let (ask, n) = a1
        .core
        .board_ask(
            &board,
            coord::NewAsk {
                text: "Who can look at the flaky sync test?".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(ask.short_ref().starts_with("mlg#"), "{}", ask.short_ref());
    assert!(
        ask.ask.as_ref().unwrap().heard.contains(&card),
        "{:?}",
        ask.ask
    );
    // Bob's agent is told through his own device, not Alice's notice.
    assert!(!n.to.contains(&card), "{:?}", n.to);
    a1.core.kick_board(&board);
    let short = ask.short_ref();
    wait_for(WAIT, "bob's agent hears the ask, woken", || {
        b1.core.inbox().list(0).iter().any(|d| {
            d.to == b1.agent
                && d.mode == inbox::mode::WAKE
                && d.message.from.key.starts_with("board:")
                && d.message.text.contains("asks the room")
                && d.message.text.contains("Irori handed it to you")
                && d.message.text.contains(&format!("board_leave {short}"))
        })
    });
    // A kick doesn't tell it twice.
    a1.core.kick_board(&board);
    b1.core.kick_board(&board);
    std::thread::sleep(Duration::from_millis(500));
    let heard = b1
        .core
        .inbox()
        .list(0)
        .iter()
        .filter(|d| d.to == b1.agent && d.message.text.contains("asks the room"))
        .count();
    assert_eq!(heard, 1);

    // Bob's agent holds it (#91); its claim crosses to Alice's device,
    // which writes the take, and the take comes back.
    let stop = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|sc| {
        sc.spawn(|| {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                a1.core.kick_board(&board);
                b1.core.kick_board(&board);
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        // Stops the kicker even when an assert below fails.
        struct Stop<'a>(&'a std::sync::atomic::AtomicBool);
        impl Drop for Stop<'_> {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let _stop = Stop(&stop);
        let out = b1.tool("ask_claim", serde_json::json!({"task_id": short}));
        let took = || {
            card_on(a1, &board, &ask.id)
                .and_then(|t| t.ask)
                .is_some_and(|a| a.taker == card)
        };
        let ok = out.starts_with("You have") || out.starts_with("Requested");
        if ok {
            wait_for(WAIT, "alice's device gives bob's agent the take", took);
            wait_for(WAIT, "bob's board shows the take", || {
                card_on(b1, &board, &ask.id)
                    .and_then(|t| t.ask)
                    .is_some_and(|a| a.taker == card)
            });
        }
        assert!(ok, "ask_claim: {out}");
    });
    close(alice.drain(..).chain(bob.drain(..)));
}

/// Card #392 (Spaces s5): Alice's space is the board Bob joined. Once
/// Alice has heard Bob's device read space rows, a second board and a card
/// on it reach Bob, and so does a channel made in the space and its posts,
/// both ways, through Talk's read model. Read cursors never leave the
/// private log. Then Bob certifies a device that never runs (an Irori that
/// has never said it reads spaces): Alice's space refuses a new channel and
/// a post in the space's channel, naming that device, while a Personal
/// channel still goes.
#[test]
fn a_second_board_and_a_channel_in_a_space_reach_bob() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let space = invited(a1, b1);
    wait_for(WAIT, "alice hears bob read spaces", || {
        a1.core.space_not_ready(&space).unwrap().is_empty()
    });

    // A second board, and a card on it.
    let net = a1
        .core
        .create_space_board(&space, "Netcode", "net")
        .unwrap();
    wait_for(WAIT, "bob gets the second board", || {
        b1.core
            .space_boards(&space)
            .is_ok_and(|bs| bs.iter().any(|b| b.board_id == net && b.tag == "net"))
    });
    let t = a1
        .core
        .board_coord(&space)
        .unwrap()
        .create_task(
            &a1.person_actor(),
            coord::NewTask {
                title: "rollback".into(),
                board: net.clone(),
                ..Default::default()
            },
        )
        .unwrap();
    a1.core.kick_board(&space);
    wait_for(WAIT, "bob gets the card on net", || {
        card_on(b1, &space, &t.id).is_some_and(|x| x.board == net)
    });

    // A channel in the space, and a post in it.
    let ch = a1
        .core
        .talk_channel_create_in(&a1.user(), "netcode", "rollback talk", &space)
        .unwrap();
    assert_eq!(ch.space_id, space);
    a1.core
        .talk_post(
            &a1.core.user_sender(),
            &ch.id,
            "hello bob",
            &[],
            false,
            None,
        )
        .unwrap();
    a1.core.kick_board(&space);
    wait_for(WAIT, "bob's Talk lists the channel", || {
        b1.core
            .talk_conversations(&b1.user())
            .is_ok_and(|cs| cs.iter().any(|c| c.conv_id == ch.id && c.unread == 1))
    });
    // Its row carries the space, so Bob's switcher files it there (#461).
    let row = b1
        .core
        .talk_conversations(&b1.user())
        .unwrap()
        .into_iter()
        .find(|c| c.conv_id == ch.id)
        .unwrap();
    assert_eq!(row.space_id, space);
    let theirs = b1.core.talk().channel(&ch.id).unwrap();
    assert_eq!(
        (theirs.space_id.as_str(), theirs.name.as_str()),
        (space.as_str(), "netcode")
    );
    assert_eq!(theirs.topic, "rollback talk");
    let ps = b1.core.talk_posts(&b1.user(), &ch.id, None, 50).unwrap();
    assert_eq!(ps.len(), 1);
    assert_eq!(ps[0].text, "hello bob");
    assert_eq!(ps[0].by, a1.person_actor(), "alice is a person to bob");
    let conv = b1
        .core
        .talk_conversations(&b1.user())
        .unwrap()
        .into_iter()
        .find(|c| c.conv_id == ch.id)
        .unwrap();
    let alice_in = conv
        .members
        .iter()
        .find(|m| m.key == a1.person_actor())
        .expect("alice is a member");
    assert_eq!(alice_in.kind, coord::talk::PERSON);
    assert!(!conv.joined, "bob hasn't joined yet");

    // Bob answers: alice sees him as a person, and he sees his own post as
    // his own.
    let reply = b1
        .core
        .talk_post(&b1.core.user_sender(), &ch.id, "hi alice", &[], false, None)
        .unwrap();
    b1.core.kick_board(&space);
    wait_for(WAIT, "alice gets bob's reply", || {
        a1.core
            .talk_posts(&a1.user(), &ch.id, None, 50)
            .is_ok_and(|ps| ps.len() == 2)
    });
    let ps = a1.core.talk_posts(&a1.user(), &ch.id, None, 50).unwrap();
    assert_eq!(ps[1].by, b1.person_actor());
    assert_eq!(ps[0].by, a1.user(), "alice's own post is hers");
    assert!(
        b1.core.talk().is_member(&ch.id, &b1.user()),
        "posting joined bob"
    );
    let mine = b1.core.talk_posts(&b1.user(), &ch.id, None, 50).unwrap();
    assert_eq!(mine[1].by, b1.user());
    assert_eq!(mine[1].post_id, reply.post.post_id);

    // Reading is the reader's own: the cursor goes in the private log.
    a1.core
        .talk_mark_read(&ch.id, &reply.post.post_id, &a1.user())
        .unwrap();
    let talk_kinds = |st: &store::Store| -> Vec<String> {
        st.list(&Filter {
            kinds: vec!["talk.".into()],
            ..Default::default()
        })
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect()
    };
    // Every talk row in the space's log is under a board key, never a
    // private user: key (the space's log refuses those).
    for ev in a1
        .core
        .board_store(&space)
        .unwrap()
        .list(&Filter {
            kinds: vec!["talk.".into()],
            ..Default::default()
        })
        .unwrap()
    {
        let p: serde_json::Value = serde_json::from_slice(&ev.payload).unwrap();
        for k in [&serde_json::json!(ev.actor), &p["by"], &p["member"]["key"]] {
            if let Some(k) = k.as_str() {
                assert!(
                    k.starts_with("person:") || k.starts_with("agent:"),
                    "{} names {k}",
                    ev.kind
                );
            }
        }
    }
    let in_space = talk_kinds(&a1.core.board_store(&space).unwrap());
    assert!(in_space.contains(&kinds::TALK_POST.to_string()));
    assert!(!in_space.contains(&kinds::TALK_READ.to_string()));
    assert!(talk_kinds(a1.core.store()).contains(&kinds::TALK_READ.to_string()));
    assert!(
        !talk_kinds(a1.core.store()).contains(&kinds::TALK_POST.to_string()),
        "the space's posts stay out of the private log"
    );

    // Bob certifies a device that never runs Irori with spaces.
    let dev_pub = hex::encode(
        ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])
            .verifying_key()
            .to_bytes(),
    );
    let add = b1
        .core
        .identity()
        .cert_for(
            "phantom-old",
            &dev_pub,
            "old laptop",
            irori_identity::ROLE_DESKTOP,
            presence::now_local(),
        )
        .unwrap();
    b1.core
        .store()
        .append("irori", kinds::PERSON_DEVICE_ADDED, &add)
        .unwrap();
    wait_for(WAIT, "alice's space waits on bob's old device", || {
        a1.core
            .space_not_ready(&space)
            .unwrap()
            .iter()
            .any(|d| d.ends_with("/phanto"))
    });
    let e = a1
        .core
        .talk_channel_create_in(&a1.user(), "later", "", &space)
        .unwrap_err();
    assert!(e.contains("waiting on") && e.contains("phanto"), "{e}");
    let e = a1
        .core
        .talk_post(&a1.core.user_sender(), &ch.id, "too soon", &[], false, None)
        .unwrap_err();
    assert!(e.contains("waiting on") && e.contains("phanto"), "{e}");
    assert_eq!(a1.core.talk().posts(&ch.id).len(), 2, "nothing written");
    // Personal channels are never gated.
    a1.core
        .talk_channel_create(&a1.user(), "notes", "")
        .unwrap();
    close(alice.drain(..).chain(bob.drain(..)));
}

/// Card #461: Alice's post in her space's channel naming Bob's agent wakes
/// it, once, on Bob's device (the bridge), with "Agents answer now" on
/// only; its reply goes in the channel, and one naming Alice's agent wakes
/// it a hop further down; a row naming a channel outside the space wakes
/// nobody; a restart of Bob's Core doesn't wake it again.
#[test]
fn alices_space_post_mentioning_bobs_agent_wakes_it_once() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let space = invited(a1, b1);
    wait_for(WAIT, "alice hears bob read spaces", || {
        a1.core.space_not_ready(&space).unwrap().is_empty()
    });
    let ch = a1
        .core
        .talk_channel_create_in(&a1.user(), "netcode", "", &space)
        .unwrap();
    a1.core.kick_board(&space);
    wait_for(WAIT, "bob's Talk lists the channel", || {
        talk_lists(b1, &ch.id)
    });
    // Bob's agent joins: that mints its card on the space, and the card
    // and the member row reach Alice.
    let out = b1.tool("talk_join", serde_json::json!({"conv": ch.id}));
    assert!(!out.starts_with("Error"), "talk_join: {out}");
    b1.core.kick_board(&space);
    wait_for(WAIT, "alice sees bob's agent in the channel", || {
        a1.core
            .talk()
            .members(&ch.id)
            .iter()
            .any(|m| m.key.starts_with("agent:"))
    });

    let p = a1
        .core
        .talk_post(
            &a1.core.user_sender(),
            &ch.id,
            "@bob1 is rollback ready?",
            &[],
            true,
            None,
        )
        .unwrap();
    assert!(
        p.mentions
            .iter()
            .any(|m| m.key.starts_with("agent:") && !m.gone),
        "bob's agent resolves by its board key: {:?} {:?}",
        p.mentions,
        p.misses
    );
    assert!(
        p.told.is_empty(),
        "nothing routed on alice's side: {:?}",
        p.told
    );
    a1.core.kick_board(&space);
    let bridged = |d: &Dev| {
        d.core
            .inbox()
            .list(0)
            .into_iter()
            .filter(|x| x.to == d.agent && x.message.ref_ == format!("talk:{}", ch.id))
            .collect::<Vec<_>>()
    };
    wait_for(WAIT, "bob's agent is woken", || !bridged(b1).is_empty());
    let d = bridged(b1).remove(0);
    assert_eq!(d.mode, inbox::mode::WAKE);
    assert_eq!(d.message.board.board_id, space);
    assert_eq!(d.message.board.person_id, a1.core.person().id);
    assert!(
        d.message.text.contains("rollback ready?"),
        "{}",
        d.message.text
    );
    assert!(d.message.text.contains("#netcode"), "{}", d.message.text);
    assert!(
        d.message
            .text
            .contains(&format!("talk_post conv={}", ch.id)),
        "{}",
        d.message.text
    );

    // "Agents answer now" off: the post reaches Bob, nobody is woken.
    a1.core
        .talk_post(
            &a1.core.user_sender(),
            &ch.id,
            "@bob1 fyi only",
            &[],
            false,
            None,
        )
        .unwrap();
    a1.core.kick_board(&space);
    wait_for(WAIT, "bob reads the quiet post", || {
        b1.core
            .talk_posts(&b1.user(), &ch.id, None, 50)
            .is_ok_and(|ps| ps.iter().any(|q| q.text == "@bob1 fyi only"))
    });
    assert_eq!(bridged(b1).len(), 1, "the quiet post woke nobody");

    // Alice's agent is in the channel too, so Bob's agent can name it.
    let out = a1.tool("talk_join", serde_json::json!({"conv": ch.id}));
    assert!(!out.starts_with("Error"), "talk_join: {out}");
    a1.core.kick_board(&space);
    wait_for(WAIT, "bob sees alice's agent in the channel", || {
        b1.core
            .talk()
            .members(&ch.id)
            .iter()
            .filter(|m| m.key.starts_with("agent:"))
            .count()
            == 1
    });

    // Bob's agent takes the wake: its turn runs at the delivery's hop + 1.
    let w = b1
        .core
        .inbox()
        .outstanding_wake(&b1.agent)
        .map(|(w, _)| w)
        .or_else(|| {
            b1.core
                .inbox()
                .next_wake(&b1.agent, inbox::path::MOD)
                .unwrap()
        })
        .expect("bob's agent has the wake");
    b1.core.inbox().mark_delivered(&b1.agent, &w.token).unwrap();
    b1.core
        .inbox()
        .mark_engaged(&b1.agent, &w.token, "turn-1")
        .unwrap();

    // The agent's reply goes in the channel and reaches Alice. It names
    // Alice's agent, which Alice's Core wakes one hop down the chain, so
    // MAX_HOP stops two people's agents naming each other forever.
    let out = b1.tool(
        "reply",
        serde_json::json!({"delivery_id": d.id, "text": "@alice1 rollback is ready"}),
    );
    assert!(out.starts_with("Replied in #netcode"), "reply: {out}");
    b1.core.kick_board(&space);
    wait_for(WAIT, "alice reads the reply in the channel", || {
        a1.core
            .talk_posts(&a1.user(), &ch.id, None, 50)
            .is_ok_and(|ps| ps.iter().any(|q| q.text == "@alice1 rollback is ready"))
    });
    assert_ne!(
        b1.core.inbox().get(&d.id).unwrap().state,
        d.state,
        "the delivery is answered"
    );
    wait_for(WAIT, "alice's agent is woken", || !bridged(a1).is_empty());
    assert_eq!(bridged(a1)[0].message.hop, 1, "one hop down the chain");

    // A row in the space's log naming a channel outside the space (a
    // modified client) wakes nobody: here Bob's own Personal channel.
    let private = b1
        .core
        .talk_channel_create(&b1.user(), "private", "")
        .unwrap();
    let bob_agent = p
        .mentions
        .iter()
        .find(|m| m.key.starts_with("agent:"))
        .unwrap()
        .key
        .clone();
    a1.core
        .board_store(&space)
        .unwrap()
        .append(
            &a1.person_actor(),
            kinds::TALK_POST,
            &serde_json::json!({
                "postId": "p-elsewhere",
                "convId": private.id,
                "text": "@bob1 what's in here?",
                "mentions": [bob_agent],
                "agentsNow": true,
                "by": a1.person_actor(),
            }),
        )
        .unwrap();
    // Then a post in the channel that does wake it: the row before it has
    // been through Bob's bridge by the time this one is.
    a1.core
        .talk_post(
            &a1.core.user_sender(),
            &ch.id,
            "@bob1 and the migration?",
            &[],
            true,
            None,
        )
        .unwrap();
    a1.core.kick_board(&space);
    wait_for(WAIT, "bob's agent is woken again", || {
        bridged(b1).len() == 2
    });
    assert!(
        !b1.core
            .inbox()
            .list(0)
            .iter()
            .any(|x| x.message.ref_ == format!("talk:{}", private.id)),
        "the row naming another channel woke nobody"
    );

    // Bob's Core restarts: the space's log replays from disk, and neither
    // post wakes the agent again.
    let b1 = restart(bob.remove(0));
    wait_for(WAIT, "bob's Talk folds the channel again", || {
        b1.core
            .talk_posts(&b1.user(), &ch.id, None, 50)
            .is_ok_and(|ps| ps.iter().any(|q| q.text == "@bob1 and the migration?"))
    });
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(bridged(&b1).len(), 2, "woken once for each post");
    b1.core.close().unwrap();
    close(alice.drain(..).chain(bob.drain(..)));
}

/// Whether `d`'s Talk lists conversation `conv`.
fn talk_lists(d: &Dev, conv: &str) -> bool {
    d.core
        .talk_conversations(&d.user())
        .is_ok_and(|cs| cs.iter().any(|c| c.conv_id == conv))
}

/// Alice's space with Bob on every device in `bobs` reading spaces, and a
/// channel in it with one post of Alice's that each of Bob's devices
/// lists. Returns the space and the channel's id.
fn space_with_a_channel(a1: &Dev, bobs: &[&Dev]) -> (String, String) {
    let space = invited(a1, bobs[0]);
    for d in &bobs[1..] {
        wait_for(WAIT, "bob's other device opens the space", || {
            d.core
                .board_list()
                .iter()
                .any(|s| s.board_id == space && s.ready)
        });
        // No relay or LAN lookup between people here: the invite link
        // gave only b1's addresses, so Alice and Bob's other device are
        // told each other's.
        let addrs = |x: &Dev| x.core.net().endpoint().expect("bound").bound_sockets();
        d.core.board_addr_hint(a1.core.device_id(), addrs(a1));
        a1.core.board_addr_hint(d.core.device_id(), addrs(d));
        d.core.kick_board(&space);
    }
    wait_for(WAIT, "alice hears bob read spaces", || {
        a1.core.space_not_ready(&space).unwrap().is_empty()
    });
    let ch = a1
        .core
        .talk_channel_create_in(&a1.user(), "netcode", "", &space)
        .unwrap();
    a1.core
        .talk_post(
            &a1.core.user_sender(),
            &ch.id,
            "hello bob",
            &[],
            false,
            None,
        )
        .unwrap();
    a1.core.kick_board(&space);
    for d in bobs {
        wait_for(WAIT, "bob's device lists the channel", || {
            talk_lists(d, &ch.id)
        });
    }
    (space, ch.id)
}

/// Card #461: Bob leaves Alice's space. Its channel goes from Talk on both
/// his devices without a reload (b1 at once, b2 from the synced `left`
/// row), and stays gone once Alice holds the leave and his devices forget
/// the space; his Personal channel stays, and Alice keeps the channel and
/// its post.
#[test]
fn leaving_a_space_drops_its_channels_from_talk_live() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 2);
    let (a1, b1, b2) = (&alice[0], &bob[0], &bob[1]);
    let (space, ch) = space_with_a_channel(a1, &[b1, b2]);
    let notes = b1
        .core
        .talk_channel_create(&b1.user(), "notes", "")
        .unwrap();

    // Alice is offline, so neither device can forget the space yet: b2
    // drops the channel from the synced `left` row alone.
    a1.core.close_board_net(&space);
    b1.core.leave_board(&space).unwrap();
    assert!(!talk_lists(b1, &ch), "b1's Talk still lists the channel");
    assert!(b1.core.talk().channel(&ch).is_none());
    assert!(b1.core.talk_posts(&b1.user(), &ch, None, 50).is_err());
    wait_for(WAIT, "b2 drops the channel too", || !talk_lists(b2, &ch));
    assert!(
        b1.core.talk().channel(&notes.id).is_some(),
        "bob's Personal channel went"
    );
    assert!(b2.core.boards().contains(&space), "b2 forgot, not left");

    // Alice comes back and holds the leave; Bob's devices forget the
    // space, and the channel stays gone.
    a1.core.reopen_board_net(&space).unwrap();
    for d in [b1, b2] {
        wait_for(WAIT, "bob's device forgets the space", || {
            !d.core.boards().contains(&space)
        });
        assert!(!talk_lists(d, &ch), "the channel came back");
        assert!(d.core.talk().channel(&ch).is_none());
    }
    assert!(b1.core.talk().channel(&notes.id).is_some());
    let ps = a1.core.talk_posts(&a1.user(), &ch, None, 50).unwrap();
    assert_eq!(ps.len(), 1, "alice lost the post");
    assert!(talk_lists(a1, &ch), "alice lost the channel");
    close(alice.drain(..).chain(bob.drain(..)));
}

/// Card #461: Bob leaves Alice's space and forgets it at once; its
/// channel is gone from his Talk straight away.
#[test]
fn leaving_and_forgetting_a_space_drops_its_channels_at_once() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let (space, ch) = space_with_a_channel(a1, &[b1]);
    b1.core.leave_board(&space).unwrap();
    b1.core.forget_left_board(&space).unwrap();
    assert!(!b1.core.boards().contains(&space));
    assert!(b1.core.talk().channel(&ch).is_none());
    assert!(!talk_lists(b1, &ch));
    assert!(a1.core.talk().channel(&ch).is_some());
    close(alice.drain(..).chain(bob.drain(..)));
}

/// Card #461: forgetting a space on this device, or leaving one nobody
/// else is on, drops its channels through the forget alone.
#[test]
fn forgetting_a_space_drops_its_channels_at_once() {
    let mut alice = person("alice", 1);
    let a1 = &alice[0];
    for leave in [false, true] {
        let tag = if leave { "left" } else { "forgot" };
        let space = a1.core.create_board(tag, tag).unwrap();
        wait_for(WAIT, "the space is ready", || {
            a1.core
                .board_list()
                .iter()
                .any(|s| s.board_id == space && s.ready)
        });
        let ch = a1
            .core
            .talk_channel_create_in(&a1.user(), "netcode", "", &space)
            .unwrap();
        assert!(talk_lists(a1, &ch.id));
        if leave {
            a1.core.leave_board(&space).unwrap(); // nobody else: forgotten
        } else {
            a1.core.forget_board(&space).unwrap();
        }
        assert!(!a1.core.boards().contains(&space));
        assert!(a1.core.talk().channel(&ch.id).is_none());
        assert!(!talk_lists(a1, &ch.id), "the channel stayed in Talk");
    }
    close(alice.drain(..));
}

// ---- person-to-person DMs (card #538, docs/design/talk-dms.md) ---------------

/// The spaces `d`'s switcher lists.
fn in_switcher(d: &Dev) -> Vec<String> {
    d.core
        .board_list()
        .into_iter()
        .map(|s| s.board_id)
        .collect()
}

/// `d`'s DM with `other`, opened: nothing is written.
fn dm_with(d: &Dev, other: &Dev) -> String {
    d.core
        .talk_open_dm(
            &coord::talk::Member::of(&d.user()),
            &coord::talk::Member::of(&other.person_actor()),
            &d.user(),
        )
        .unwrap()
}

fn dm_post(d: &Dev, conv: &str, text: &str) {
    d.core
        .talk_post(&d.core.user_sender(), conv, text, &[], false, None)
        .unwrap();
}

fn dm_holds(d: &Dev, conv: &str, text: &str) -> bool {
    d.core
        .talk_posts(&d.user(), conv, None, 50)
        .is_ok_and(|ps| ps.iter().any(|p| p.text == text))
}

fn board_linked(a: &Dev, b: &Dev, board: &str) {
    let peer = b.core.device_id().to_string();
    wait_for(WAIT, "the two devices connect on their space", || {
        a.core
            .net()
            .board_conns(board)
            .iter()
            .any(|c| c.peer == peer)
    });
}

/// §8: Bob is connected before the DM exists, so no hello will come; making
/// the space walks the live peers of the space they share.
#[test]
fn a_dm_starts_with_bob_already_connected() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    board_linked(a1, b1, &board);

    let dm = dm_with(a1, b1);
    assert_eq!(dm, dm_with(b1, a1), "both sides name one conversation");
    assert_eq!(a1.core.boards().len(), 1, "opening a DM writes nothing");
    assert!(
        a1.core
            .talk_conversations(&a1.user())
            .unwrap()
            .iter()
            .any(|c| c.conv_id == dm && c.name == b1.core.person().name),
        "an opened DM is listed, by the other person's name"
    );

    dm_post(a1, &dm, "hi bob");
    wait_for(WAIT, "bob reads alice's DM", || dm_holds(b1, &dm, "hi bob"));
    dm_post(b1, &dm, "hi alice");
    wait_for(WAIT, "alice reads bob's reply", || {
        dm_holds(a1, &dm, "hi alice")
    });

    // One space, the two of them, and nothing of it on the space they share.
    let spaces = a1.core.dm_spaces_with(&b1.core.person().id);
    assert_eq!(spaces.len(), 1, "{spaces:?}");
    assert_eq!(b1.core.dm_spaces_with(&a1.core.person().id), spaces);
    assert!(!a1.core.dm_pending(&spaces[0]));
    assert_eq!(a1.core.board_info(&spaces[0]).unwrap().members.len(), 2);
    assert!(a1.core.board_rows(&board, kinds::TALK_POST).is_empty());
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §8: Bob is away when Alice writes. Her post waits in a pending log, and
/// the hand-off runs when he comes back: his hello on the space they share.
#[test]
fn a_dm_to_an_offline_friend_lands_later() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    board_linked(a1, b1, &board);
    b1.core.close_board_net(&board);

    let dm = dm_with(a1, b1);
    dm_post(a1, &dm, "when you're back");
    dm_post(a1, &dm, "no rush");
    let bob_id = b1.core.person().id.clone();
    let spaces = a1.core.dm_spaces_with(&bob_id);
    assert_eq!(spaces.len(), 1, "{spaces:?}");
    assert!(a1.core.dm_pending(&spaces[0]));
    assert!(
        dm_holds(a1, &dm, "no rush"),
        "her own posts read back at once"
    );

    b1.core.reopen_board_net(&board).unwrap();
    wait_for(WAIT, "bob's device gets the waiting posts", || {
        dm_holds(b1, &dm, "when you're back") && dm_holds(b1, &dm, "no rush")
    });
    wait_for(WAIT, "the space stops being pending", || {
        !a1.core.dm_pending(&spaces[0])
    });
    dm_post(b1, &dm, "back");
    wait_for(WAIT, "alice reads him", || dm_holds(a1, &dm, "back"));
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §8: a device of Alice's that is asleep can't hold a DM's first row
/// back: the hidden log counts only devices heard to be too old.
#[test]
fn a_two_device_person_can_start_a_dm_with_a_phone_asleep() {
    let mut alice = person("alice", 2);
    let mut bob = person("bob", 1);
    let board = invited(&alice[0], &bob[0]);
    wait_for(WAIT, "the phone opens the space", || {
        alice[1].core.boards().contains(&board)
    });
    let phone = alice.pop().unwrap();
    phone.core.close().unwrap();

    let (a1, b1) = (&alice[0], &bob[0]);
    board_linked(a1, b1, &board);
    let dm = dm_with(a1, b1);
    dm_post(a1, &dm, "the phone can sleep");
    wait_for(WAIT, "bob reads it", || {
        dm_holds(b1, &dm, "the phone can sleep")
    });
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §8: once one of Bob's devices holds the space, the other follows on the
/// marker and `dm.secret`, is certified, and reads and writes the DM too.
#[test]
fn a_second_device_gets_the_dm() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 2);
    let (a1, b1, b2) = (&alice[0], &bob[0], &bob[1]);
    let board = invited(a1, b1);
    board_linked(a1, b1, &board);
    let dm = dm_with(a1, b1);
    dm_post(a1, &dm, "to both of bob");
    wait_for(WAIT, "b1 reads it", || dm_holds(b1, &dm, "to both of bob"));
    wait_for(WAIT, "b2 reads it", || dm_holds(b2, &dm, "to both of bob"));

    let alice_id = a1.core.person().id.clone();
    let id = b1.core.dm_spaces_with(&alice_id)[0].clone();
    wait_for(WAIT, "b2 has the space's secret", || {
        b2.core.board_secrets(&id).is_some_and(|s| s.epoch() >= 1)
    });
    let b2_dev = b2.core.device_id().to_string();
    wait_for(WAIT, "the space certifies b2", || {
        b2.core
            .board_verifier(&id)
            .is_some_and(|v| v.cert(&b2_dev).is_some())
    });
    assert!(b2.core.is_dm_space(&id), "b2 knows what the space is");
    assert!(!in_switcher(b2).contains(&id), "and doesn't list it");

    dm_post(b2, &dm, "from the other device");
    wait_for(WAIT, "alice reads b2", || {
        dm_holds(a1, &dm, "from the other device")
    });
    wait_for(WAIT, "b1 reads b2", || {
        dm_holds(b1, &dm, "from the other device")
    });
    assert_eq!(b2.core.dm_spaces_with(&alice_id), vec![id]);
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §8: both write before either hears the other. Two logs, one
/// conversation, and from then on both write to the same log.
#[test]
fn both_first_messages_make_one_conversation() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    board_linked(a1, b1, &board);
    a1.core.close_board_net(&board);
    b1.core.close_board_net(&board);

    let dm = dm_with(a1, b1);
    assert_eq!(dm_with(b1, a1), dm);
    dm_post(a1, &dm, "alice first");
    dm_post(b1, &dm, "bob first");
    a1.core.reopen_board_net(&board).unwrap();
    b1.core.reopen_board_net(&board).unwrap();
    for d in [a1, b1] {
        wait_for(WAIT, "each side reads both", || {
            dm_holds(d, &dm, "alice first") && dm_holds(d, &dm, "bob first")
        });
    }
    let (alice_id, bob_id) = (a1.core.person().id.clone(), b1.core.person().id.clone());
    let spaces = a1.core.dm_spaces_with(&bob_id);
    assert_eq!(spaces.len(), 2, "{spaces:?}");
    assert_eq!(b1.core.dm_spaces_with(&alice_id), spaces);
    for d in [a1, b1] {
        let listed = d.core.talk_conversations(&d.user()).unwrap();
        assert_eq!(listed.iter().filter(|c| c.conv_id == dm).count(), 1);
    }
    wait_for(WAIT, "both spaces stop being pending", || {
        spaces
            .iter()
            .all(|id| !a1.core.dm_pending(id) && !b1.core.dm_pending(id))
    });
    let from = |d: &Dev, text: &str| {
        d.core
            .talk_post(&d.core.user_sender(), &dm, text, &[], false, None)
            .unwrap()
            .post
            .space
    };
    assert_eq!(from(a1, "settled"), spaces[0], "the lowest counted log");
    assert_eq!(from(b1, "settled too"), spaces[0]);
    wait_for(WAIT, "each reads the other", || {
        dm_holds(a1, &dm, "settled too") && dm_holds(b1, &dm, "settled")
    });
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §8, the done test: a DM comes back from disk on both sides and carries
/// on, and its space is in neither switcher, before or after.
#[test]
fn a_dm_survives_a_restart_with_nothing_in_either_switcher() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let board = invited(&alice[0], &bob[0]);
    board_linked(&alice[0], &bob[0], &board);
    let dm = dm_with(&alice[0], &bob[0]);
    dm_post(&alice[0], &dm, "before the restart");
    wait_for(WAIT, "bob reads it", || {
        dm_holds(&bob[0], &dm, "before the restart")
    });
    let bob_id = bob[0].core.person().id.clone();
    let id = alice[0].core.dm_spaces_with(&bob_id)[0].clone();
    assert_eq!(in_switcher(&alice[0]), vec![board.clone()]);
    assert_eq!(in_switcher(&bob[0]), vec![board.clone()]);

    let a1 = restart(alice.pop().unwrap());
    let b1 = restart(bob.pop().unwrap());
    // At once: the markers load before any space opens.
    assert!(!in_switcher(&a1).contains(&id) && !in_switcher(&b1).contains(&id));
    wait_for(WAIT, "the DM is back on both sides", || {
        dm_holds(&a1, &dm, "before the restart") && dm_holds(&b1, &dm, "before the restart")
    });
    assert!(a1.core.is_dm_space(&id) && b1.core.is_dm_space(&id));
    // This harness has no relay: only the invite's addresses cross people,
    // and those moved with the restart.
    let direct = |d: &Dev| {
        let rt = d.core.pairing_runtime().clone();
        rt.block_on(d.core.net().invite_addr()).unwrap().direct
    };
    b1.core.board_addr_hint(a1.core.device_id(), direct(&a1));
    dm_post(&b1, &dm, "after the restart");
    wait_for(WAIT, "alice reads it", || {
        dm_holds(&a1, &dm, "after the restart")
    });
    assert_eq!(in_switcher(&a1), vec![board.clone()]);
    assert_eq!(in_switcher(&b1), vec![board]);
    assert_eq!(a1.core.dm_spaces_with(&bob_id), vec![id]);
    close([a1, b1]);
}

/// §5: Bob founds a DM and writes into it all a space can hold: a card, an
/// ask, a note, his agent's card, a board inside. Alice's and his own
/// lists, tools, feeds and refs show none of it, though each log holds it.
#[test]
fn nothing_of_a_dm_space_shows_anywhere() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    board_linked(a1, b1, &board);
    wait_for(WAIT, "alice hears bob read spaces", || {
        a1.core.space_not_ready(&board).unwrap().is_empty()
    });
    let dm = dm_with(b1, a1);
    dm_post(b1, &dm, "hello alice");
    wait_for(WAIT, "alice reads it", || dm_holds(a1, &dm, "hello alice"));
    let alice_id = a1.core.person().id.clone();
    let id = b1.core.dm_spaces_with(&alice_id)[0].clone();
    assert!(a1.core.is_dm_space(&id) && b1.core.is_dm_space(&id));
    let changes = a1.core.subscribe_boards();

    let co = b1.core.board_coord(&id).unwrap();
    let bob_key = b1.person_actor();
    let card = co.create_task(&bob_key, new_task("HIDDEN card")).unwrap();
    let (ask, _) = b1
        .core
        .board_ask(
            &id,
            coord::NewAsk {
                text: "HIDDEN ask".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let note = coord::NewNote {
        kind: "rule".into(),
        text: "HIDDEN note".into(),
        tags: vec!["everywhere".into()],
        ..Default::default()
    };
    co.propose_note(&bob_key, note).unwrap();
    let agent = b1.core.board_key(&id, &b1.agent).unwrap();
    let inside = irori_core::new_board_id().unwrap();
    b1.core
        .board_store(&id)
        .unwrap()
        .append(
            &bob_key,
            kinds::SPACE_BOARD_CREATED,
            &serde_json::json!({"boardId": inside, "name": "HIDDEN", "tag": "inner", "by": bob_key}),
        )
        .unwrap();
    b1.core.kick_board(&id);
    wait_for(WAIT, "alice's log holds all of it", || {
        b1.core.kick_board(&id);
        card_on(a1, &id, &card.id).is_some()
            && card_on(a1, &id, &ask.id).is_some()
            && !a1.core.board_rows(&id, kinds::AGENT_CARD).is_empty()
            && a1.core.space_boards(&id).is_ok_and(|bs| bs.len() == 2)
            && !a1.core.board_coord(&id).unwrap().notes(false).is_empty()
    });

    // A card on the shared space is what the same paths do show.
    let seen = b1
        .core
        .board_coord(&board)
        .unwrap()
        .create_task(&bob_key, new_task("SEEN card"))
        .unwrap();
    b1.core.kick_board(&board);
    let mut heard = Vec::new();
    wait_for(WAIT, "the window hears the shared card", || {
        b1.core.kick_board(&board);
        while let Ok(Some(ch)) = changes.recv_timeout(Duration::from_millis(100)) {
            heard.push(ch.task_id);
        }
        heard.contains(&seen.id)
    });
    a1.core.sweep_coord().unwrap();
    while let Ok(Some(ch)) = changes.recv_timeout(Duration::from_millis(100)) {
        heard.push(ch.task_id);
    }
    assert!(
        !heard.contains(&card.id) && !heard.contains(&ask.id),
        "{heard:?}"
    );
    assert!(
        a1.core
            .board_views(&Default::default())
            .iter()
            .any(|v| format!("{v:?}").contains("SEEN card")),
        "the shared card shows"
    );

    for d in [a1, b1] {
        assert_eq!(in_switcher(d), vec![board.clone()]);
        assert!(d.core.board_of(&card.id).is_none());
        assert!(d.core.route(&card.id).is_err());
        let (co, _) = d.core.task_coord(&card.id, &[]).unwrap();
        assert!(co.get_task(&card.id).is_none());
        assert!(d.core.task_ref("inner#1").is_err());
        assert_ne!(d.core.task_ref("#1").ok(), Some(card.id.clone()));
        assert!(d.core.board_roster(&id).is_empty());
        assert!(d.core.board_agents().iter().all(|a| a.board_id != id));
        assert!(d.core.board_people().iter().all(|p| p.board_id != id));
        assert!(d.core.search_notes("HIDDEN", false).is_empty());
        assert!(d.core.notes(&id, false).is_err());
        for got in [
            format!("{:?}", d.core.board_views(&Default::default())),
            format!("{:?}", d.core.idle_report()),
            format!("{:?}", d.core.notes("", false)),
            d.tool("board", serde_json::json!({})),
            d.tool("overview", serde_json::json!({})),
            d.tool("agents", serde_json::json!({})),
        ] {
            assert!(
                !got.contains("HIDDEN") && !got.contains(&id) && !got.contains(&agent),
                "{got}"
            );
        }
        // Nor can an agent name it.
        for board in [id.as_str(), "inner", "~dm"] {
            let args = serde_json::json!({"title": "in there", "board": board});
            let got = d.core.tools().call(
                &tools::Caller {
                    sender: inbox::Sender::new(&d.agent, "agent", "claude"),
                    path: String::new(),
                },
                "board_create",
                &serde_json::to_vec(&args).unwrap(),
            );
            let text = format!("{got:?}");
            assert!(text.contains("no board") || text.contains("Err"), "{text}");
        }
        assert!(
            d.core
                .board_coord(&id)
                .unwrap()
                .list_tasks(&coord::TaskFilter::default())
                .iter()
                .all(|t| t.title != "in there")
        );
    }

    // Alice's agent hears no ask from it and gets no card on it.
    let cards = a1.core.board_rows(&id, kinds::AGENT_CARD);
    assert!(cards.iter().all(|e| e.device_id == b1.core.device_id()));
    assert!(
        a1.core
            .inbox()
            .list(0)
            .iter()
            .all(|d| !d.message.text.contains("HIDDEN")),
    );
    // A tag used in there is free out here.
    a1.core
        .create_space_board(&board, "Mine", "inner")
        .expect("a hidden space holds no tag");
    close(alice.drain(..).chain(bob.drain(..)));
}

/// The Core's boards for `bundle::admit` in a test.
struct TestBoards<'a>(&'a Dev);

impl irori_invite::InviteBoards for TestBoards<'_> {
    fn open_board(&self, _: &str, _: Option<irori_identity::MemberPin>) -> Result<(), String> {
        Ok(())
    }
    fn board_store(&self, id: &str) -> Option<Arc<store::Store>> {
        self.0.core.board_store(id)
    }
    fn board_verifier(&self, id: &str) -> Option<Arc<irori_identity::BoardVerifier>> {
        self.0.core.board_verifier(id)
    }
    fn board_secrets(&self, id: &str) -> Option<Arc<irori_identity::BoardSecrets>> {
        self.0.core.board_secrets(id)
    }
}

/// `d` as a connection shows it, S saying it is `person`.
fn live(d: &Dev, person: &str) -> mesh::boardmesh::LivePeer {
    mesh::boardmesh::LivePeer {
        device_id: d.core.device_id().to_string(),
        device_pub: hex::encode(d.core.identity().device_pub()),
        person_id: person.to_string(),
        dms: mesh::boardmesh::DMS_PROTOCOL,
    }
}

fn frame(id: &str, step: &str, body: serde_json::Value) -> mesh::boardmesh::DmFrame {
    mesh::boardmesh::DmFrame {
        board_id: id.to_string(),
        step: step.to_string(),
        body,
    }
}

/// The proof of `d`'s current key.
fn proof_of(d: &Dev) -> irori_identity::KeyProof {
    let pin = d.core.identity().board_pin();
    d.core
        .chain()
        .key_proof(pin.person_epoch, &pin.person_pub)
        .unwrap()
}

/// `signer`'s cert for `d`'s device.
fn cert_by(signer: &Dev, d: &Dev) -> irori_identity::MemberDeviceAdded {
    let pin = signer.core.identity().board_pin();
    signer
        .core
        .identity()
        .member_cert_for(
            pin.person_epoch,
            d.core.device_id(),
            &hex::encode(d.core.identity().device_pub()),
            irori_identity::ROLE_DESKTOP,
            presence::now_local(),
        )
        .unwrap()
}

/// §8: M, an owner of some space, lists a device of hers as "Bob" there, so
/// S's word says that device is Bob. Neither her own key nor a replay of
/// Bob's real pin and chain gets her added; and listed as Alice, her offer
/// opens nothing on Bob's side.
#[test]
fn a_hostile_owner_cant_read_or_forge_a_dm() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let mut mallory = person("mallory", 1);
    let (a1, b1, m1) = (&alice[0], &bob[0], &mallory[0]);
    let board = invited(a1, b1);
    board_linked(a1, b1, &board);
    b1.core.close_board_net(&board);
    let (alice_id, bob_id) = (a1.core.person().id.clone(), b1.core.person().id.clone());
    let dm = dm_with(a1, b1);
    dm_post(a1, &dm, "for bob only");
    let id = a1.core.dm_spaces_with(&bob_id)[0].clone();

    let as_bob = live(m1, &bob_id);
    let real_pin = b1.core.identity().board_pin();
    let re_rooted = irori_identity::MemberPin {
        root_device_id: m1.core.device_id().to_string(),
        ..real_pin.clone()
    };
    let m_pin = irori_identity::MemberPin {
        person_id: bob_id.clone(),
        ..m1.core.identity().board_pin()
    };
    let mut m_cert_as_bob = cert_by(m1, m1);
    m_cert_as_bob.cert.person_id = bob_id.clone();
    let mut sig_replayed = cert_by(b1, b1);
    sig_replayed.cert.device_id = as_bob.device_id.clone();
    sig_replayed.cert.device_pub = as_bob.device_pub.clone();
    let tries = [
        // Her own key, under Bob's id.
        (m_pin, proof_of(m1), m_cert_as_bob),
        // Bob's real pin and chain, replayed whole.
        (real_pin, proof_of(b1), cert_by(b1, b1)),
        // Re-rooted at her device: the cert still names Bob's.
        (re_rooted.clone(), proof_of(b1), cert_by(b1, b1)),
        // Bob's signature moved onto a cert for her device.
        (re_rooted, proof_of(b1), sig_replayed),
    ];
    for (pin, proof, cert) in tries {
        a1.core.dm_take_frame(
            &board,
            &as_bob,
            frame(
                &id,
                "pin",
                serde_json::json!({"pin": pin, "proof": proof, "cert": cert}),
            ),
        );
        assert!(
            a1.core
                .board_rows(&id, kinds::BOARD_MEMBER_ADDED)
                .is_empty(),
            "a forged pin was added"
        );
    }
    // Bob's own device, with the same pin, is what those were missing.
    let good = serde_json::json!({
        "pin": b1.core.identity().board_pin(),
        "proof": proof_of(b1),
        "cert": cert_by(b1, b1),
    });
    a1.core
        .dm_take_frame(&board, &live(b1, &bob_id), frame(&id, "pin", good));
    assert_eq!(a1.core.board_rows(&id, kinds::BOARD_MEMBER_ADDED).len(), 1);

    // A space M founded, Bob on it, offered with S saying M's device is
    // Alice: its creator isn't the sender S names.
    let hers = m1.core.create_board("friendly", "dm-ish").unwrap();
    let boards = TestBoards(m1);
    let pin = b1.core.identity().board_pin();
    let offer =
        irori_invite::bundle::admit(&boards, m1.core.identity(), &hers, &pin, "bob").unwrap();
    b1.core.dm_take_frame(
        &board,
        &live(m1, &alice_id),
        frame(
            &hers,
            "offer",
            serde_json::json!({"bundle": offer, "proof": proof_of(m1)}),
        ),
    );
    assert!(!b1.core.boards().contains(&hers), "bob opened a forged DM");
    assert!(!b1.core.is_dm_space(&hers));
    assert!(!dm_holds(b1, &dm, "for bob only"));
    close(
        alice
            .drain(..)
            .chain(bob.drain(..))
            .chain(mallory.drain(..)),
    );
}

/// §8: an offer that fails a check touches nothing: no marker, no log, no
/// secret. The same offer with the proof it needs goes through, so it is
/// the check that stopped it.
#[test]
fn a_refused_offer_leaves_nothing_on_disk() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let mut mallory = person("mallory", 1);
    let (a1, b1, m1) = (&alice[0], &bob[0], &mallory[0]);
    let board = invited(a1, b1);
    board_linked(a1, b1, &board);
    b1.core.close_board_net(&board);
    let (alice_id, bob_id) = (a1.core.person().id.clone(), b1.core.person().id.clone());
    let dm = dm_with(a1, b1);
    dm_post(a1, &dm, "an offer");
    let id = a1.core.dm_spaces_with(&bob_id)[0].clone();
    let boards = TestBoards(a1);
    let pin = b1.core.identity().board_pin();
    let offer = irori_invite::bundle::admit(&boards, a1.core.identity(), &id, &pin, "bob").unwrap();
    let from_alice = live(a1, &alice_id);

    let files = |b: &Dev| {
        std::fs::read_dir(b.data.join("spaces"))
            .map(|d| {
                d.flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with(&id))
                    .count()
            })
            .unwrap_or(0)
    };
    let refused = |body: serde_json::Value| {
        b1.core
            .dm_take_frame(&board, &from_alice, frame(&id, "offer", body));
        assert_eq!(files(b1), 0, "a refused offer left a file");
        assert!(!b1.core.boards().contains(&id));
        assert!(!b1.core.is_dm_space(&id), "a refused offer left a marker");
    };
    // Another person's proof, no proof, and a secret cut short.
    refused(serde_json::json!({"bundle": offer, "proof": proof_of(m1)}));
    refused(serde_json::json!({"bundle": offer}));
    let mut short = serde_json::to_value(&offer).unwrap();
    short["secret"] = "c2hvcnQ=".into();
    refused(serde_json::json!({"bundle": short, "proof": proof_of(a1)}));
    // Sent by a device this space doesn't certify, though S says it's Alice.
    b1.core.dm_take_frame(
        &board,
        &live(m1, &alice_id),
        frame(
            &id,
            "offer",
            serde_json::json!({"bundle": offer, "proof": proof_of(a1)}),
        ),
    );
    assert_eq!(files(b1), 0);

    b1.core.dm_take_frame(
        &board,
        &from_alice,
        frame(
            &id,
            "offer",
            serde_json::json!({"bundle": offer, "proof": proof_of(a1)}),
        ),
    );
    assert!(b1.core.boards().contains(&id), "the good offer didn't open");
    assert!(b1.core.is_dm_space(&id));
    wait_for(WAIT, "bob reads the post the offer carried", || {
        dm_holds(b1, &dm, "an offer")
    });
    close(
        alice
            .drain(..)
            .chain(bob.drain(..))
            .chain(mallory.drain(..)),
    );
}

/// §8: Alice owns the space, so she can add someone to it. That breaks the
/// pair: the space's rows stop folding on both sides, whoever writes them,
/// and the next DM goes in a new space.
#[test]
fn a_third_member_cant_touch_a_dm() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let mut carol = person("carol", 1);
    let (a1, b1, c1) = (&alice[0], &bob[0], &carol[0]);
    let board = invited(a1, b1);
    board_linked(a1, b1, &board);
    let dm = dm_with(a1, b1);
    dm_post(a1, &dm, "just us");
    wait_for(WAIT, "bob reads it", || dm_holds(b1, &dm, "just us"));
    let bob_id = b1.core.person().id.clone();
    let id = a1.core.dm_spaces_with(&bob_id)[0].clone();

    let boards = TestBoards(a1);
    let pin = c1.core.identity().board_pin();
    irori_invite::bundle::admit(&boards, a1.core.identity(), &id, &pin, "carol").unwrap();
    wait_for(WAIT, "alice's side stops folding it", || {
        !dm_holds(a1, &dm, "just us")
    });
    wait_for(WAIT, "bob's side stops folding it", || {
        !dm_holds(b1, &dm, "just us")
    });
    let t = b1.core.talk_post(
        &b1.core.user_sender(),
        &dm,
        "this goes elsewhere",
        &[],
        false,
        None,
    );
    let t = t.unwrap();
    assert_ne!(t.post.space, id, "the broken space took a post");
    close(alice.drain(..).chain(bob.drain(..)).chain(carol.drain(..)));
}

/// §4: what wouldn't fold is refused before the append, so nothing stays
/// in the log: an agent's post, a leave, an agent as a member.
#[test]
fn a_dm_refuses_what_wouldnt_fold_before_it_writes() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    board_linked(a1, b1, &board);
    let dm = dm_with(a1, b1);
    dm_post(a1, &dm, "hello");
    let id = a1.core.dm_spaces_with(&b1.core.person().id)[0].clone();
    let count = || {
        let rows = |k| a1.core.board_rows(&id, k).len();
        (
            rows(kinds::TALK_POST),
            rows(kinds::TALK_MEMBER),
            rows(kinds::AGENT_CARD),
        )
    };
    let before = count();
    let user = a1.user();
    let bob_key = coord::talk::Member::of(&b1.person_actor());
    let agent = coord::talk::Member::of(&a1.agent);
    assert!(a1.core.talk_leave(&dm, &bob_key, &user).is_err(), "a leave");
    assert!(
        a1.core.talk_join(&dm, &agent, &user).is_err(),
        "an agent as a member"
    );
    let as_agent = inbox::Sender::new(&a1.agent, "agent", "claude");
    assert!(
        a1.core
            .talk_post(&as_agent, &dm, "psst", &[], false, None)
            .is_err(),
        "an agent's post"
    );
    let out = a1.tool("talk_post", serde_json::json!({"conv": dm, "text": "psst"}));
    assert!(out.starts_with("Error"), "the tool: {out}");
    assert_eq!(count(), before, "a refused row stayed in the log");

    // A mention is dropped, and mints no card for the agent it names.
    a1.core
        .talk_post(
            &a1.core.user_sender(),
            &dm,
            "see this",
            std::slice::from_ref(&a1.agent),
            true,
            None,
        )
        .unwrap();
    assert_eq!(count().2, before.2, "a mention minted a card");
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §2: an offer carries the whole log, so a pending log stops at what a
/// frame can carry; the hand-off still goes through with what it holds.
#[test]
fn a_pending_dm_stops_at_what_an_offer_can_carry() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    board_linked(a1, b1, &board);
    b1.core.close_board_net(&board);
    let dm = dm_with(a1, b1);
    let long = "x".repeat(coord::MAX_PERSON_TEXT);
    let me = a1.core.user_sender();
    let mut sent = 0;
    let why = loop {
        match a1.core.talk_post(&me, &dm, &long, &[], false, None) {
            Ok(_) => sent += 1,
            Err(e) => break e,
        }
        assert!(sent < 1000, "a pending log took everything");
    };
    assert!(why.contains("can't hold more"), "{why}");
    assert!(sent > 0);
    b1.core.reopen_board_net(&board).unwrap();
    wait_for(WAIT, "the big offer lands", || {
        b1.core
            .talk_posts(&b1.user(), &dm, None, 200)
            .is_ok_and(|ps| ps.len() == sent)
    });
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §2: one person holds at most 4 live DM spaces with another, so the
/// members of a space can't fill a disk. A space it already holds can be
/// offered again; a fifth new one can't.
#[test]
fn a_person_holds_at_most_four_dm_spaces_with_one_other() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    board_linked(a1, b1, &board);
    let (alice_id, bob_id) = (a1.core.person().id.clone(), b1.core.person().id.clone());
    let from_alice = live(a1, &alice_id);
    let boards = TestBoards(a1);
    let pin = b1.core.identity().board_pin();
    let creator = a1.core.identity().board_pin();
    let offer_one = |n: usize| {
        let id = a1
            .core
            .create_board(&format!("dm {n}"), &format!("dm-{n}"))
            .unwrap();
        a1.core.write_dm_marker(&id, &creator, &bob_id).unwrap();
        let offer =
            irori_invite::bundle::admit(&boards, a1.core.identity(), &id, &pin, "bob").unwrap();
        let body = serde_json::json!({"bundle": offer, "proof": proof_of(a1)});
        b1.core
            .dm_take_frame(&board, &from_alice, frame(&id, "offer", body.clone()));
        (id, body)
    };
    let mut first = None;
    for n in 0..4 {
        let (id, body) = offer_one(n);
        assert!(b1.core.boards().contains(&id), "offer {n} was refused");
        first.get_or_insert((id, body));
    }
    let (id, _) = offer_one(4);
    assert!(!b1.core.boards().contains(&id), "a fifth space was taken");
    assert!(!b1.core.is_dm_space(&id));
    assert_eq!(b1.core.dm_spaces_with(&alice_id).len(), 4);

    let (id, body) = first.unwrap();
    b1.core
        .dm_take_frame(&board, &from_alice, frame(&id, "offer", body));
    assert!(b1.core.boards().contains(&id), "a held space was dropped");
    close(alice.drain(..).chain(bob.drain(..)));
}

#[derive(Default)]
struct Cards(std::sync::Mutex<Vec<toast::Note>>);

impl notify::Toaster for Cards {
    fn configure(&self, _: toast::Config) {}
    fn set_options(&self, _: toast::Options) {}
    fn post(&self, n: toast::Note) {
        self.0.lock().unwrap().push(n);
    }
    fn close(&self, _: &str) {}
}

/// §4: a post from the other person rings, once, from the space's own
/// follower; the person's own post doesn't.
#[test]
fn a_dm_from_bob_rings_once_on_alices_device() {
    let cards = Arc::new(Cards::default());
    let notes = notify::Notifier::new(notify::Config {
        toaster: Some(cards.clone()),
        ..notify::Config::default()
    });
    let mut alice = common::mesh::person_with("alice", 1, |_| {
        let notes = notes.clone();
        Some(Arc::new(move |o: &mut irori_core::Options| {
            o.notifier = Some(notes.clone());
        }) as common::mesh::Edit)
    });
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    board_linked(a1, b1, &board);
    let dm = dm_with(a1, b1);
    assert_eq!(dm_with(b1, a1), dm);
    dm_post(a1, &dm, "hi bob");
    wait_for(WAIT, "bob reads it", || dm_holds(b1, &dm, "hi bob"));
    dm_post(b1, &dm, "hi alice");
    wait_for(WAIT, "alice reads it", || dm_holds(a1, &dm, "hi alice"));
    let talk = || -> Vec<String> {
        let cards = cards.0.lock().unwrap();
        cards
            .iter()
            .filter(|n| n.id.starts_with("talk:"))
            .map(|n| n.body.clone())
            .collect()
    };
    wait_for(WAIT, "the DM's card", || !talk().is_empty());
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(talk(), ["hi alice"]);
    close(alice.drain(..).chain(bob.drain(..)));
}
