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

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use common::wait_for;
use irori_core::Core;
use store::Filter;
use store::kinds::{self, PERSON_ACTOR_PREFIX};

const WAIT: u64 = 45;

fn free_udp() -> SocketAddr {
    std::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
}

fn new_task(title: &str) -> coord::NewTask {
    coord::NewTask {
        title: title.into(),
        ..Default::default()
    }
}

/// One device: a Core with one live Claude session.
struct Dev {
    core: Core,
    agent: String,
    data: std::path::PathBuf,
    _dirs: Vec<tempfile::TempDir>,
}

impl Dev {
    fn user(&self) -> String {
        self.core.user_sender().key
    }

    fn person_actor(&self) -> String {
        format!("{PERSON_ACTOR_PREFIX}{}", self.core.person().id)
    }

    fn tool(&self, name: &str, args: serde_json::Value) -> String {
        let caller = tools::Caller {
            sender: inbox::Sender::new(&self.agent, "agent", "claude"),
            path: String::new(),
        };
        self.core
            .tools()
            .call(&caller, name, &serde_json::to_vec(&args).unwrap())
            .unwrap()
    }
}

/// A Core whose mesh runs on loopback iroh with a LAN lookup unicast to
/// `to` (as tests/pair.rs), and one live session `sess`.
fn dev(
    data: tempfile::TempDir,
    name: &str,
    sess: &str,
    lan: SocketAddr,
    to: Vec<SocketAddr>,
) -> Dev {
    let (cl, w) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let sessions = cl.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    common::write_session(&sessions, "900001.json", sess, name, w.path());
    let mut o = common::options(data.path(), cl.path());
    o.pair_net = Some(Arc::new(move |ident, chain, guests| {
        let mut nc = irori_core::devices::default_net_config(ident, chain, guests);
        nc.ip = irori_net::endpoint::IpMode::Loopback;
        nc.gossip = false;
        let lc = nc.lan_config.as_mut().expect("a LAN lookup");
        lc.bind = Some(lan);
        let to = to.clone();
        lc.targets = Some(Arc::new(move || to.clone()));
        lc.every = Duration::from_millis(200);
        irori_net::transport::IrohNet::new(nc)
    }));
    let core = Core::start(o).unwrap();
    let agent = format!("{}/{sess}", core.device_id());
    wait_for(10, "presence lists the session", || {
        core.local_agents().iter().any(|a| a.key == agent)
    });
    Dev {
        core,
        agent,
        data: data.path().to_path_buf(),
        _dirs: vec![data, cl, w],
    }
}

/// One person on `n` devices: the first certifies the others as pairing
/// does, and the call returns once every device owns every other's key.
fn person(who: &str, n: usize) -> Vec<Dev> {
    let lans: Vec<SocketAddr> = (0..n).map(|_| free_udp()).collect();
    let others = |i: usize| -> Vec<SocketAddr> {
        lans.iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, a)| *a)
            .collect()
    };
    let sess = |i: usize| format!("{:08x}-1111-4111-8111-{:012x}", i + 1, i + 1);
    let first = dev(
        tempfile::tempdir().unwrap(),
        &format!("{who}1"),
        &sess(0),
        lans[0],
        others(0),
    );
    let mut devs = vec![first];
    for (i, lan) in lans.iter().enumerate().skip(1) {
        let d = tempfile::tempdir().unwrap();
        std::fs::copy(
            devs[0].data.join(irori_identity::PERSON_FILE),
            d.path().join(irori_identity::PERSON_FILE),
        )
        .unwrap();
        let next = dev(d, &format!("{who}{}", i + 1), &sess(i), *lan, others(i));
        let add = devs[0]
            .core
            .identity()
            .cert_for(
                next.core.device_id(),
                &hex::encode(next.core.identity().device_pub()),
                &format!("{who}{}", i + 1),
                irori_identity::ROLE_DESKTOP,
                presence::now_local(),
            )
            .unwrap();
        devs[0]
            .core
            .store()
            .append("irori", kinds::PERSON_DEVICE_ADDED, &add)
            .unwrap();
        devs.push(next);
    }
    let rows = devs[0]
        .core
        .store()
        .list(&Filter {
            kinds: vec!["person.".into()],
            ..Default::default()
        })
        .unwrap();
    for d in &devs[1..] {
        for _ in 0..2 {
            for ev in &rows {
                let _ = d.core.chain().learn(ev.into());
            }
        }
        d.core.mesh().kick();
    }
    wait_for(WAIT, &format!("{who}'s devices pair"), || {
        devs.iter().all(|x| {
            devs.iter()
                .all(|y| x.core.is_owner(&format!("user:{}", y.core.device_id())))
        })
    });
    devs
}

/// Alice's board, with Bob invited from a link and the words checked.
fn invited(a: &Dev, b: &Dev) -> String {
    let id = a.core.create_board("MLG farming", "mlg").unwrap();
    let ra = a.core.pairing_runtime().clone();
    let link = ra.block_on(a.core.invite(&id)).unwrap().link;
    let rb = b.core.pairing_runtime().clone();
    rb.block_on(b.core.join_invite(&link)).unwrap();
    wait_for(WAIT, "both screens show the words", || {
        a.core.invite_status().status.state == "words"
            && b.core.invite_status().status.state == "words"
    });
    let words = b.core.invite_status().status.words.join(" ");
    a.core.confirm_invite_words(&words).unwrap();
    wait_for(WAIT, "the invite is done on both sides", || {
        a.core.invite_status().status.state == "done"
            && b.core.invite_status().status.state == "done"
    });
    assert!(b.core.boards().contains(&id));
    id
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
    let dir = c1.data.join(irori_core::BOARDS_DIR);
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
/// Bob's board shows "Leaving…" until Alice's device says it holds the
/// `left` row, then Bob forgets it.
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
        b1.core
            .board_list()
            .iter()
            .any(|s| s.board_id == board && s.leaving),
        "bob's board isn't Leaving…"
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
                && d.message.text.contains(&format!("ask_claim {short}"))
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

    // Bob's agent takes it: its claim crosses to Alice's device, which
    // writes the take, and the take comes back.
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
