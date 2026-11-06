//! Go's core board tests (I:internal/core/boards_test.go,
//! boards_own_test.go): shared boards on one person's devices, through the
//! Core's public calls. Made-up fixture identities, the file keystore, no
//! network on a single device.

mod common;

use std::time::Duration;

use common::pair::{dev, free_udp, paired};
use common::{Rig, wait_for};
use coord::ErrorKind as CK;
use irori_core::{BoardError, Core, SPACES_DIR as BOARDS_DIR, valid_board_id};
use store::Filter;
use store::kinds::{self, AGENT_ACTOR_PREFIX, PERSON_ACTOR_PREFIX};

fn created_ids(st: &store::Store) -> Vec<String> {
    st.list(&Filter {
        kinds: vec![kinds::TASK_CREATED.into()],
        limit: 1000,
        ..Default::default()
    })
    .unwrap()
    .iter()
    .map(|ev| {
        let v: serde_json::Value = serde_json::from_slice(&ev.payload).unwrap();
        v["taskId"].as_str().unwrap_or_default().to_string()
    })
    .collect()
}

fn coord_kind(e: &BoardError) -> Option<CK> {
    match e {
        BoardError::Coord(c) => Some(c.kind()),
        _ => None,
    }
}

fn restart(r: Rig) -> (Core, Rig) {
    r.core.close().unwrap();
    let c = Core::start(common::options(&r.data, &r.claude)).unwrap();
    (c, r)
}

fn new_task(title: &str) -> coord::NewTask {
    coord::NewTask {
        title: title.into(),
        ..Default::default()
    }
}

/// TestBoardRouting: one person, the private board and two shared boards
/// on one device (v0.5 §2.1). Every taskId routes to the one log that holds
/// it, an id in no log routes nowhere, and that still holds after a
/// restart, when the index is rebuilt from the logs.
#[test]
fn board_routing() {
    let r = Rig::new();
    let c = &r.core;
    let private = c
        .coord()
        .create_task(&r.key_a, new_task("private work"))
        .unwrap();
    let b1 = c.create_board("one", "one").unwrap();
    let b2 = c.create_board("two", "two").unwrap();
    assert!(b1 != b2 && valid_board_id(&b1), "board ids {b1} {b2}");
    let on_board = |board: &str, title: &str| {
        let co = c.board_coord(board).unwrap();
        let by = c.board_key(board, &r.key_a).unwrap();
        co.create_task(&by, new_task(title)).unwrap()
    };
    let (t1, t2) = (on_board(&b1, "board one"), on_board(&b2, "board two"));

    let key_a = r.key_a.clone();
    let check = |c: &Core| {
        let cases: [(&str, &str, &str, bool); 6] = [
            ("a private card", &private.id, "", true),
            ("a card on board one", &t1.id, &b1, true),
            ("a card on board two", &t2.id, &b2, true),
            ("an id in no log", "no-such-task", "", false),
            ("an empty id", "", "", false),
            ("a board id is not a task", &b1, "", false),
        ];
        for (name, task, board, ok) in cases {
            match c.route(task) {
                Err(e) => {
                    assert!(!ok, "{name}: Route: {e}");
                    assert_eq!(e.kind(), CK::NotFound, "{name}");
                }
                Ok((got, co)) => {
                    assert!(ok, "{name}: routed to {got}");
                    assert_eq!(got, board, "{name}");
                    assert!(
                        co.get_task(task).is_some(),
                        "{name}: the coord doesn't hold it"
                    );
                    assert_eq!(
                        got.is_empty(),
                        std::sync::Arc::ptr_eq(&co, c.coord()),
                        "{name}: only private cards route to the private coord"
                    );
                    let (co2, _) = c.task_coord(task, &[]).unwrap();
                    assert!(
                        std::sync::Arc::ptr_eq(&co, &co2),
                        "{name}: TaskCoord differs"
                    );
                }
            }
        }
        // An id in no log goes to the private coord, which says not found.
        let (co, keys) = c
            .task_coord("no-such-task", std::slice::from_ref(&key_a))
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(&co, c.coord()) && keys[0] == key_a);
        let e = co
            .set_task_state(&keys[0], "no-such-task", coord::COMPLETED, "")
            .unwrap_err();
        assert_eq!(e.kind(), CK::NotFound);
    };
    check(c);

    // Each log holds only its own cards.
    for (st, want) in [
        (c.store().clone(), &private.id),
        (c.board_store(&b1).unwrap(), &t1.id),
        (c.board_store(&b2).unwrap(), &t2.id),
    ] {
        assert_eq!(created_ids(&st), vec![want.clone()]);
    }

    // After a restart the boards are found on disk and the index comes back
    // (they open just after the start, card #232).
    let (c2, _r) = restart(r);
    wait_for(10, "boards after restart", || c2.boards().len() == 2);
    check(&c2);
    c2.close().unwrap();
}

/// TestBoardKeys: callers become board keys only through board_key (§2.4).
#[test]
fn board_keys() {
    let r = Rig::new();
    let c = &r.core;
    let b1 = c.create_board("one", "one").unwrap();
    let b2 = c.create_board("two", "two").unwrap();
    let me = format!("{PERSON_ACTOR_PREFIX}{}", c.person().id);
    assert_eq!(c.board_key(&b1, &c.user_sender().key).unwrap(), me);
    let card_a = c.board_key(&b1, &r.key_a).unwrap();
    assert!(
        card_a.starts_with(AGENT_ACTOR_PREFIX) && !card_a.contains(c.device_id()),
        "{card_a}"
    );
    assert_eq!(
        c.board_key(&b1, &format!("api:{}", r.key_a)).unwrap(),
        card_a
    );
    let other = c.board_key(&b2, &r.key_a).unwrap();
    assert!(other != card_a && !other.is_empty());
    for k in [
        "user:some-other-device".to_string(),
        "dev-elsewhere/session-1".to_string(),
        format!("{}/no-such-session", c.device_id()),
    ] {
        assert!(c.board_key(&b1, &k).is_err(), "board_key({k}) took it");
    }
    assert!(matches!(
        c.board_key("0123456789abcdef0123456789abcdef", &r.key_a),
        Err(BoardError::NoBoard(_))
    ));
    // The card row is the reduced card, and only on its own board.
    let st = c.board_store(&b1).unwrap();
    let cards = st
        .list(&Filter {
            kinds: vec![kinds::AGENT_CARD.into()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(cards.len(), 1);
    let row: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&cards[0].payload).unwrap();
    for k in row.keys() {
        assert!(
            ["cardId", "name", "kind", "personId", "status"].contains(&k.as_str()),
            "agent.card carries {k}"
        );
    }
    assert_eq!(cards[0].actor, card_a);
    assert_eq!(row["personId"], c.person().id.as_str());
    assert_eq!(row["name"], "alpha");
    // The board_cards mapping maps back.
    assert_eq!(c.private_key(&card_a), r.key_a);
    assert_eq!(c.private_key(&me), c.user_sender().key);
    assert_eq!(c.private_key("agent:someone-elses"), "agent:someone-elses");
    // Writes through task_coord carry board keys.
    let co = c.board_coord(&b1).unwrap();
    let task = co.create_task(&me, new_task("assign me")).unwrap();
    let (rco, keys) = c
        .task_coord(&task.id, &[c.user_sender().key, r.key_a.clone()])
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&rco, &co));
    assert_eq!(keys, vec![me.clone(), card_a.clone()]);
    rco.assign(&keys[0], &task.id, &keys[1]).unwrap();
    assert_eq!(co.get_task(&task.id).unwrap().holders, vec![card_a.clone()]);
    // The card is live while its session is.
    assert!(
        c.board_roster(&b1)
            .iter()
            .any(|a| a.key == card_a && a.status != "gone"),
        "the card of a live session isn't live on the board"
    );
    // The board log refuses private kinds; the private log has no card.
    for kind in [
        "message.sent",
        kinds::AGENT_SEEN,
        "capture.turn",
        kinds::PERSON_PROFILE,
        "task.shared",
    ] {
        let e = st
            .append("irori", kind, &serde_json::json!({}))
            .unwrap_err();
        assert_eq!(
            e.kind(),
            store::ErrorKind::NotBoardKind,
            "board log took {kind}"
        );
    }
    let rows = c
        .store()
        .list(&Filter {
            kinds: vec![kinds::AGENT_CARD.into()],
            ..Default::default()
        })
        .unwrap();
    assert!(rows.is_empty(), "the private log holds agent.card rows");
    r.core.close().unwrap();
}

/// TestBoardToolsRoute: an agent's board tools act on a shared board's card
/// in that board's log, as its card, and a private card stays private.
#[test]
fn board_tools_route() {
    let r = Rig::new();
    let c = &r.core;
    let b1 = c.create_board("one", "one").unwrap();
    let co = c.board_coord(&b1).unwrap();
    let me = format!("{PERSON_ACTOR_PREFIX}{}", c.person().id);
    let task = co.create_task(&me, new_task("on the board")).unwrap();
    let caller = tools::Caller {
        sender: inbox::Sender::new(&r.key_a, "alpha", "claude"),
        path: String::new(),
    };
    let call = |name: &str, args: serde_json::Value| {
        c.tools()
            .call(&caller, name, &serde_json::to_vec(&args).unwrap())
            .unwrap()
    };
    let ok = |out: String, what: &str| {
        assert!(!out.starts_with("Error"), "{what}: {out}");
        out
    };
    ok(
        call(
            "board_join",
            serde_json::json!({"task_id": format!("task:{}", task.id)}),
        ),
        "board_join",
    );
    ok(
        call(
            "board_comment",
            serde_json::json!({"task_id": task.id, "text": "on it"}),
        ),
        "board_comment",
    );
    let card = c.board_key(&b1, &r.key_a).unwrap();
    assert_eq!(co.get_task(&task.id).unwrap().holders, vec![card.clone()]);
    let cms = co.comments(&task.id).unwrap();
    assert!(cms.len() == 1 && cms[0].by == card, "comments {cms:?}");
    let task_rows = |c: &Core| {
        c.store()
            .list(&Filter {
                kinds: vec!["task.".into()],
                ..Default::default()
            })
            .unwrap()
            .len()
    };
    assert_eq!(
        task_rows(c),
        0,
        "the private log got task rows from a board card"
    );
    let out = call(
        "board_status",
        serde_json::json!({"task_id": "no-such-card", "state": "completed"}),
    );
    assert!(out.starts_with("Error"), "a card in no log: {out}");

    // The agent sees the shared card it holds in board and overview, with
    // its tag, and can send about it; a subtask of it lands on the board.
    let out = call("board", serde_json::json!({"repo": "all"}));
    assert!(
        out.contains("one#1") && out.contains("on the board"),
        "board:\n{out}"
    );
    let out = call("overview", serde_json::json!({}));
    assert!(out.contains("one#1"), "overview:\n{out}");
    ok(
        call(
            "send",
            serde_json::json!({"to": r.key_b, "text": "look at this", "task_id": "one#1"}),
        ),
        "send",
    );
    let out = ok(
        call(
            "board_create",
            serde_json::json!({"title": "a piece of it", "parent": "one#1", "assign_to": r.key_b}),
        ),
        "board_create",
    );
    assert!(
        out.contains("one#2"),
        "board_create under a shared card: {out}"
    );
    assert_eq!(
        task_rows(c),
        0,
        "a subtask of a shared card went in the private log"
    );
    r.core.close().unwrap();
}

/// TestBoardLogsSkipJunk: files in boards/ that aren't board logs are left
/// alone at start.
#[test]
fn board_logs_skip_junk() {
    let (d, cl) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let dir = d.path().join(BOARDS_DIR);
    std::fs::create_dir_all(&dir).unwrap();
    for f in ["notes.txt", "..db", "ABCDEF0123456789ABCDEF0123456789.db"] {
        std::fs::write(dir.join(f), "x").unwrap();
    }
    let c = Core::start(common::options(d.path(), cl.path())).unwrap();
    assert!(c.boards().is_empty(), "boards {:?}", c.boards());
    c.close().unwrap();
}

/// Card #392: a v0.10 data dir keeps its board logs in boards/. The next
/// start moves each one to spaces/ and opens it as the same space: same id,
/// every card and member, nothing rewritten. A move cut short by a crash
/// (the sidecars moved, the .db not yet) finishes at the start after.
#[test]
fn board_logs_move_to_spaces() {
    let r = Rig::new();
    let id = r
        .core
        .create_board("Wind Waker Multiplayer", "wind")
        .unwrap();
    let co = r.core.board_coord(&id).unwrap();
    let by = r.core.board_key(&id, &r.key_a).unwrap();
    let t = co.create_task(&by, new_task("sync the boats")).unwrap();
    co.comment_on(
        &by,
        &t.id,
        coord::CommentInput {
            text: "first pass works".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let before = created_ids(&r.core.board_store(&id).unwrap());
    let vector = r.core.board_store(&id).unwrap().vector().unwrap();
    r.core.close().unwrap();

    // Lay the files out as v0.10 did, then cut the move short: the
    // sidecars are in spaces/, the .db is still in boards/.
    let (spaces, boards) = (
        r.data.join(irori_core::SPACES_DIR),
        r.data.join(irori_core::BOARDS_DIR),
    );
    std::fs::rename(&spaces, &boards).unwrap();
    std::fs::create_dir_all(&spaces).unwrap();
    std::fs::rename(
        boards.join(format!("{id}.secrets")),
        spaces.join(format!("{id}.secrets")),
    )
    .unwrap();

    let c = Core::start(common::options(&r.data, &r.claude)).unwrap();
    assert!(spaces.join(format!("{id}.db")).exists(), "the .db moved");
    assert!(!boards.exists(), "the emptied boards/ is gone");
    // board_store waits for the boards still opening after the start.
    let st = c.board_store(&id).unwrap();
    assert_eq!(c.boards().len(), 1, "boards {:?}", c.boards());
    assert_eq!(created_ids(&st), before, "the same cards");
    assert_eq!(st.vector().unwrap(), vector, "the same rows");
    let co = c.board_coord(&id).unwrap();
    assert!(co.get_task(&t.id).is_some(), "the card folds");
    // A start with nothing left to move moves nothing.
    assert_eq!(irori_core::migrate_board_files(&r.data).unwrap(), 0);
    c.close().unwrap();
}

/// Card #392: a space holds more boards. A lone device adds one at once;
/// with two devices the add waits until the other has said, in its board
/// hello, that it reads space rows (an older Irori would stall on them),
/// and then the new board reaches it.
#[test]
fn a_second_board_in_a_space_reaches_the_other_device() {
    let (a, b) = paired();
    let id = a
        .core
        .create_board("Wind Waker Multiplayer", "wind")
        .unwrap();
    // b is a member device (a wrote its cert) but hasn't even opened the
    // space yet, so it can't have said it reads space rows: refused.
    let e = a
        .core
        .create_space_board(&id, "Too soon", "soon")
        .unwrap_err();
    assert!(e.to_string().contains("waiting on"), "{e}");
    assert_eq!(a.core.space_boards(&id).unwrap().len(), 1);
    wait_for(45, "b opens the space", || b.core.board_coord(&id).is_ok());
    let bv = b.core.board_verifier(&id).unwrap();
    wait_for(45, "b's cert arrives", || {
        bv.membership().created && bv.cert(b.core.device_id()).is_some()
    });
    // Until b's hello lands a refuses, naming what it waits on.
    wait_for(45, "a hears b read spaces", || {
        a.core.space_not_ready(&id).unwrap().is_empty()
    });
    let sub = a.core.create_space_board(&id, "Netcode", "net").unwrap();
    assert_ne!(sub, id);
    let names = |c: &Core| -> Vec<String> {
        c.space_boards(&id)
            .unwrap()
            .into_iter()
            .map(|x| format!("{}:{}", x.tag, x.name))
            .collect()
    };
    assert_eq!(
        names(&a.core),
        ["wind:Wind Waker Multiplayer", "net:Netcode"]
    );
    wait_for(45, "b gets the second board", || names(&b.core).len() == 2);
    assert_eq!(names(&b.core), names(&a.core));
    // A card on the new board shows its tag; the space numbers its boards'
    // cards in one run, and a ref finds it by either device.
    let co = a.core.board_coord(&id).unwrap();
    let by = a.core.board_key(&id, &a.user()).unwrap();
    let on_wind = co.create_task(&by, new_task("boats")).unwrap();
    let on_net = co
        .create_task(
            &by,
            coord::NewTask {
                title: "rollback".into(),
                board: sub.clone(),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(on_wind.short_ref(), "wind#1");
    assert_eq!(on_net.short_ref(), "net#2");
    assert_eq!(on_net.board, sub);
    assert_eq!(a.core.task_ref("net#2").unwrap(), on_net.id);
    assert!(a.core.task_ref("wind#2").is_err(), "#2 is on net");
    let bco = b.core.board_coord(&id).unwrap();
    wait_for(45, "b gets the card on net", || {
        bco.get_task(&on_net.id)
            .is_some_and(|t| t.short_ref() == "net#2")
    });
    assert_eq!(b.core.task_ref("net#2").unwrap(), on_net.id);
    // A board the space doesn't hold is refused.
    let e = co
        .create_task(
            &by,
            coord::NewTask {
                title: "x".into(),
                board: "nope".into(),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(e.to_string().contains("no board"), "{e}");
    // A tag in use, here or in a space, is refused.
    for tag in ["wind", "net"] {
        assert!(a.core.create_space_board(&id, "x", tag).is_err(), "{tag}");
    }
}

/// Card #411: a board note waits, like a space row, until every member
/// device has said it reads them (an older Irori would stall on note.*),
/// naming what it waits on; a Mine note never waits. Then the board note
/// reaches the other device.
#[test]
fn a_board_note_waits_for_every_device_then_reaches_it() {
    let (a, b) = paired();
    let id = a.core.create_board("MLG", "mlg").unwrap();
    let me = a.core.user_sender().key;
    let nn = |text: &str| coord::NewNote {
        kind: "rule".into(),
        text: text.into(),
        tags: vec!["everywhere".into()],
        ..Default::default()
    };
    let e = a
        .core
        .propose_note(&me, "mlg", nn("Ship models to the repo."))
        .unwrap_err();
    assert!(e.is(coord::ErrorKind::NotAllowed), "{e}");
    assert!(e.to_string().contains("waiting on"), "{e}");
    assert!(a.core.notes("mlg", false).unwrap().is_empty());
    a.core
        .propose_note(&me, "mine", nn("Home IPs stay out of MLG."))
        .unwrap();
    wait_for(45, "b opens the board", || b.core.board_coord(&id).is_ok());
    wait_for(45, "a hears b read spaces", || {
        a.core.space_not_ready(&id).unwrap().is_empty()
    });
    let n = a
        .core
        .propose_note(&me, "mlg", nn("Ship models to the repo."))
        .unwrap();
    assert_eq!(n.note.state, coord::NOTE_ACCEPTED);
    wait_for(45, "b gets the board note", || {
        b.core
            .notes(&id, false)
            .is_ok_and(|ns| ns.iter().any(|x| x.note.id == n.note.id))
    });
}

/// Card #392: a member device not yet heard to read space rows (an older
/// Irori, or one not seen since this start) holds back every space-only
/// row, and the refusal names it.
#[test]
fn a_space_waits_for_devices_that_dont_read_spaces() {
    let r = Rig::new();
    let id = r.core.create_board("Lemmings Decomp", "lemmings").unwrap();
    // Alone in its space: nothing to wait on.
    assert!(r.core.space_not_ready(&id).unwrap().is_empty());
    let levels = r.core.create_space_board(&id, "Levels", "levels").unwrap();
    assert_eq!(r.core.space_boards(&id).unwrap().len(), 2);
    // An agent names the board inside the space by its tag; the card
    // lands on it, in the space's log.
    let caller = tools::Caller {
        sender: inbox::Sender::new(&r.key_a, "alpha", "claude"),
        path: String::new(),
    };
    let out = r
        .core
        .tools()
        .call(
            &caller,
            "board_create",
            &serde_json::to_vec(&serde_json::json!({"title": "level 3 crash", "board": "levels"}))
                .unwrap(),
        )
        .unwrap();
    assert!(out.contains("levels#1"), "board_create on levels: {out}");
    let t = r.core.task_ref("levels#1").unwrap();
    let co = r.core.board_coord(&id).unwrap();
    assert_eq!(co.get_task(&t).unwrap().board, levels);
    // The space's own board still takes cards by its own tag.
    let out = r
        .core
        .tools()
        .call(
            &caller,
            "board_create",
            &serde_json::to_vec(&serde_json::json!({"title": "decomp", "board": "lemmings"}))
                .unwrap(),
        )
        .unwrap();
    assert!(
        out.contains("lemmings#2"),
        "board_create on lemmings: {out}"
    );
    r.core.close().unwrap();
}

/// TestBoardIdentityKeptCurrent: a board this device creates pins this
/// person with this device as the board root and carries the person's
/// member.* rows (§3), and they follow the private log: a device certified
/// there is certified on the board, a revoke cuts it with a cut over the
/// board's own stream, and the rotation that follows goes on the board with
/// its prevSig. Every row verifies against the board's own chain, and all
/// of it survives a restart.
#[test]
fn board_identity_kept_current() {
    let r = Rig::new();
    let c = &r.core;
    let id = c.create_board("mlg", "mlg").unwrap();
    let me = c.identity().current();
    let v = c.board_verifier(&id).unwrap();
    let pin = v.pinned(&me.id).expect("pinned");
    assert!(
        pin.root_device_id == c.device_id() && pin.person_epoch == me.epoch,
        "pin {pin:?}"
    );
    let m = c.board_member(&id, &me.id).unwrap();
    assert!(
        m.devices.len() == 1 && m.devices[0].cert.device_id == c.device_id() && m.name == me.name,
        "member {m:?}"
    );

    // An agent's row on the board is this person's.
    let by = c.board_key(&id, &r.key_a).unwrap();
    assert_eq!(v.key_person(&by), me.id);
    let co = c.board_coord(&id).unwrap();
    co.acquire(&by, "port:5173", Duration::from_secs(3600), false)
        .unwrap();
    co.release(&by, "port:5173").unwrap();
    assert!(
        co.ignored_rows() == 0 && co.leases().is_empty(),
        "own lease rows ignored {}",
        co.ignored_rows()
    );

    // A device paired in the private log.
    let now = presence::now_local;
    let dev_pub = hex::encode(
        ed25519_dalek::SigningKey::from_bytes(&[7u8; 32])
            .verifying_key()
            .to_bytes(),
    );
    let add = c
        .identity()
        .cert_for(
            "dev-x",
            &dev_pub,
            "laptop",
            irori_identity::ROLE_DESKTOP,
            now(),
        )
        .unwrap();
    c.store()
        .append("irori", kinds::PERSON_DEVICE_ADDED, &add)
        .unwrap();
    wait_for(10, "the paired device is certified on the board", || {
        v.cert("dev-x").is_some()
    });

    // Revoked, then the key rotates (Remove device's two rows).
    let rev = c
        .identity()
        .revoke_of("dev-x", 0, "", "lost", now())
        .unwrap();
    c.store()
        .append("irori", kinds::PERSON_DEVICE_REVOKED, &rev)
        .unwrap();
    let rot = c.identity().rotate(now()).unwrap();
    c.store()
        .append("irori", kinds::PERSON_KEY_ROTATED, &rot)
        .unwrap();
    wait_for(10, "the revoke and rotation reach the board", || {
        v.cut("dev-x").is_some() && v.epoch(&me.id) == rot.epoch
    });
    assert_eq!(v.cut("dev-x"), Some(0), "dev-x wrote nothing on the board");
    // The member device revoke rotates the board secret (§6.1).
    wait_for(10, "the board secret rotates", || {
        c.board_info(&id)
            .is_ok_and(|i| i.epoch == 2 && i.secret_epoch == 2)
    });
    // No other member device is left to move, so the board settles on
    // epoch 2 and epoch 1's topic closes (card #67 item 11).
    wait_for(10, "the board settles on the new epoch", || {
        let topics = c.board_secrets(&id).unwrap().topics().unwrap();
        topics.len() == 1 && topics[0].epoch == 2
    });
    let secrets = c.board_secrets(&id).unwrap();
    assert_eq!(secrets.settled(), 2);
    assert!(secrets.secret(1).is_none(), "epoch 1's secret is gone");

    let check = |c: &Core| {
        let st = c.board_store(&id).unwrap();
        let v = c.board_verifier(&id).unwrap();
        let evs = st
            .list(&Filter {
                limit: 1000,
                ..Default::default()
            })
            .unwrap();
        let mut n: std::collections::HashMap<String, usize> = Default::default();
        for ev in &evs {
            *n.entry(ev.kind.clone()).or_default() += 1;
            store::Verifier::verify_row(&*v, ev)
                .unwrap_or_else(|e| panic!("{}/{} {}: {e}", ev.device_id, ev.seq, ev.kind));
        }
        let k = |s: &str| n.get(s).copied().unwrap_or(0);
        assert!(
            k(kinds::MEMBER_DEVICE_ADDED) == 2
                && k(kinds::MEMBER_DEVICE_REVOKED) == 1
                && k(kinds::MEMBER_KEY_ROTATED) == 1
                && k(kinds::MEMBER_PROFILE) == 1
                && k(kinds::BOARD_CREATED) == 1,
            "member rows {n:?}"
        );
        let info = c.board_info(&id).unwrap();
        assert!(
            info.epoch == 2 && info.secret_epoch == 2 && info.tag == "mlg",
            "info {info:?}"
        );
        let m = c.board_member(&id, &me.id).unwrap();
        assert!(m.epoch == rot.epoch && m.devices.len() == 2, "member {m:?}");
    };
    check(c);
    let (c2, _r) = restart(r);
    check(&c2);
    c2.close().unwrap();
}

/// TestCreateBoardRows: create writes the self-cert first and then
/// board.created, which makes this person the board's one owner; tags are
/// checked and unique on the device; an owner renames; the board's secret
/// is in a sealed file beside the log and in no row (§2.1, §4.2, §6.1).
#[test]
fn create_board_rows() {
    let r = Rig::new();
    let c = &r.core;
    for (name, tag) in [
        ("", "ok"),
        ("ok", ""),
        ("ok", "Mlg"),
        ("ok", "12"),
        ("ok", "a#b"),
    ] {
        let e = c.create_board(name, tag).unwrap_err();
        assert_eq!(
            coord_kind(&e),
            Some(CK::Invalid),
            "create_board({name:?}, {tag:?}): {e}"
        );
    }
    let id = c.create_board("  MLG farming ", "mlg").unwrap();
    let e = c.create_board("another", "mlg").unwrap_err();
    assert_eq!(
        coord_kind(&e),
        Some(CK::Invalid),
        "a second board tagged mlg: {e}"
    );
    let st = c.board_store(&id).unwrap();
    let evs = st
        .list(&Filter {
            device_id: c.device_id().into(),
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    let ks: Vec<&str> = evs.iter().map(|e| e.kind.as_str()).collect();
    assert!(
        ks.len() >= 2
            && ks[0] == kinds::MEMBER_DEVICE_ADDED
            && ks[ks.len() - 1] == kinds::BOARD_CREATED,
        "this device's stream {ks:?}"
    );
    let me = c.person().id;
    let info = c.board_info(&id).unwrap();
    assert!(
        info.name == "MLG farming"
            && info.tag == "mlg"
            && info.creator == me
            && info.members.len() == 1
            && info.members[0].role == irori_identity::membership::BOARD_OWNER
            && info.members[0].state == irori_identity::membership::MEMBER_ACTIVE
            && info.conflicts.is_empty(),
        "info {info:?}"
    );
    assert!(
        info.epoch == 1 && info.secret_epoch == 1,
        "epochs {}/{}",
        info.epoch,
        info.secret_epoch
    );
    c.rename_board(&id, "mlg & co").unwrap();
    let e = c.rename_board(&id, "").unwrap_err();
    assert_eq!(coord_kind(&e), Some(CK::Invalid), "an empty name: {e}");
    assert_eq!(c.board_info(&id).unwrap().name, "mlg & co");
    // The secret: in its sealed file, not in any row.
    let (secret, _) = c
        .board_secrets(&id)
        .unwrap()
        .secret(1)
        .expect("an epoch 1 secret");
    assert!(
        r.data
            .join(BOARDS_DIR)
            .join(format!("{id}.secrets"))
            .exists()
    );
    let needle = hex::encode(&*secret);
    for st in [st.clone(), c.store().clone()] {
        for ev in st
            .list(&Filter {
                limit: 1000,
                ..Default::default()
            })
            .unwrap()
        {
            assert!(
                !String::from_utf8_lossy(&ev.payload).contains(&needle),
                "the secret is in a {} row",
                ev.kind
            );
        }
    }
    drop(st);
    let (c2, _r) = restart(r);
    let info = c2.board_info(&id).unwrap();
    assert!(
        info.name == "mlg & co"
            && c2.board_verifier(&id).unwrap().membership().owner(&me)
            && info.secret_epoch == 1,
        "after restart {info:?}"
    );
    c2.close().unwrap();
}

/// TestForgetBoardStaysForgotten: Forget drops a board on this device only
/// (§4.6), and the board.joined it was opened from doesn't bring it back.
#[test]
fn forget_board_stays_forgotten() {
    let r = Rig::new();
    let c = &r.core;
    let id = c.create_board("notes", "notes").unwrap();
    c.forget_board(&id).unwrap();
    assert!(c.board_coord(&id).is_err(), "the board is still open");
    c.reconcile_boards();
    assert!(
        c.board_coord(&id).is_err(),
        "board.joined brought a forgotten board back"
    );
    for name in [format!("{id}.db"), format!("{id}.secrets")] {
        assert!(
            !r.data.join(BOARDS_DIR).join(&name).exists(),
            "{name} is still there"
        );
    }
    assert!(c.board_list().is_empty());
    let (c2, _r) = restart(r);
    assert!(
        c2.boards().is_empty(),
        "a restart brought a forgotten board back"
    );
    c2.close().unwrap();
}

/// A made-up DM space's id (card #538).
const DM: &str = "0123456789abcdef0123456789abcdef";

/// This person's pin, from a board made for it (card #538).
fn my_pin(c: &Core) -> irori_identity::MemberPin {
    let id = c.create_board("pin", "pin").unwrap();
    let me = c.identity().current().id;
    c.board_verifier(&id).unwrap().pinned(&me).unwrap()
}

/// Card #538: a marker opens its space as board.joined would and fills the
/// cache at once. Forget has to cover the marker, or the next reconcile
/// brings the space back. A newer marker (a later DM offers the space
/// again) does bring it back.
#[test]
fn a_dm_marker_opens_its_space_and_forget_covers_it() {
    let r = Rig::new();
    let c = &r.core;
    let pin = my_pin(c);
    let plain = c.create_board("plain", "plain").unwrap();
    c.write_dm_marker(DM, &pin, "bob").unwrap();
    assert_eq!(c.dm_space(DM), Some((pin.clone(), "bob".to_string())));
    assert!(!c.is_dm_space(&plain), "a plain board has no marker");
    c.reconcile_boards();
    assert!(
        c.board_coord(DM).is_ok(),
        "the marker didn't open the space"
    );
    assert!(
        c.board_list().iter().all(|s| s.board_id != DM),
        "a marker's space shows"
    );

    c.forget_board(DM).unwrap();
    assert!(c.board_coord(DM).is_err(), "the space is still open");
    c.reconcile_boards();
    assert!(
        c.board_coord(DM).is_err(),
        "the marker brought a forgotten space back"
    );
    for name in [format!("{DM}.db"), format!("{DM}.secrets")] {
        assert!(
            !r.data.join(BOARDS_DIR).join(&name).exists(),
            "{name} is still there"
        );
    }
    assert!(c.is_dm_space(DM), "a forgotten space lost its marker");

    c.write_dm_marker(DM, &pin, "bob").unwrap();
    c.reconcile_boards();
    assert!(c.board_coord(DM).is_ok(), "a newer marker didn't reopen it");
}

/// Card #538: a board.forgot newer than the marker forgets the space on
/// this device, and the marker before it doesn't reopen it, then or after
/// a restart. The cache is full as soon as the Core starts, whether or not
/// the space is live.
#[test]
fn board_forgot_beats_an_older_dm_marker_and_a_restart_keeps_the_cache() {
    let r = Rig::new();
    let c = &r.core;
    let pin = my_pin(c);
    c.write_dm_marker(DM, &pin, "bob").unwrap();
    c.reconcile_boards();
    assert!(
        c.board_coord(DM).is_ok(),
        "the marker didn't open the space"
    );
    c.store()
        .append(
            "irori",
            kinds::BOARD_FORGOT,
            &serde_json::json!({ "boardId": DM }),
        )
        .unwrap();
    c.reconcile_boards();
    assert!(c.board_coord(DM).is_err(), "board.forgot didn't forget it");
    assert!(!r.data.join(BOARDS_DIR).join(format!("{DM}.db")).exists());
    c.reconcile_boards();
    assert!(c.board_coord(DM).is_err(), "an older marker reopened it");

    let (c2, _r) = restart(r);
    assert_eq!(c2.dm_space(DM), Some((pin, "bob".to_string())));
    c2.reconcile_boards();
    assert!(c2.board_coord(DM).is_err(), "a restart brought it back");
    c2.close().unwrap();
}

/// Card #538: trust rests on the marker, so one that doesn't name a space,
/// a sound creator and the other person opens nothing and marks nothing.
#[test]
fn a_bad_dm_marker_opens_nothing() {
    let r = Rig::new();
    let c = &r.core;
    let pin = serde_json::to_value(my_pin(c)).unwrap();
    let bare = serde_json::json!({ "personId": "someone" });
    for row in [
        serde_json::json!({ "boardId": DM, "with": "bob" }),
        serde_json::json!({ "boardId": DM, "creator": pin }),
        serde_json::json!({ "boardId": DM, "creator": pin, "with": "" }),
        serde_json::json!({ "boardId": DM, "creator": bare, "with": "bob" }),
        serde_json::json!({ "boardId": "nope", "creator": pin, "with": "bob" }),
    ] {
        c.store()
            .append("irori", kinds::DM_SPACE_JOINED, &row)
            .unwrap();
        c.reconcile_boards();
        assert!(!c.is_dm_space(DM), "{row} marked the space");
        assert!(c.board_coord(DM).is_err(), "{row} opened the space");
        assert!(!c.is_dm_space("nope"), "{row} marked a bad id");
    }
}

/// Card #538: another device of the person opens the space from the
/// marker when it arrives, with nobody calling reconcile.
#[test]
fn a_dm_marker_reaches_the_persons_other_device() {
    let (a, b) = paired();
    let pin = my_pin(&a.core);
    a.core.write_dm_marker(DM, &pin, "bob").unwrap();
    wait_for(45, "b opens the space from the marker", || {
        b.core.board_coord(DM).is_ok()
    });
    assert_eq!(b.core.dm_space(DM), Some((pin, "bob".to_string())));
    a.core.close().unwrap();
    b.core.close().unwrap();
}

/// TestShareAndWithdraw: Share to board (§5.2) writes a fresh card with the
/// scrubbed fields and steps, closes the private card with a note naming
/// the shared copy, and refuses writes to the private id with that name;
/// Withdraw (§5.3) cancels the shared card and brings the private one back.
#[test]
fn share_and_withdraw() {
    let r = Rig::new();
    let c = &r.core;
    let id = c.create_board("MLG", "mlg").unwrap();
    let step = |t: &str| coord::Step {
        text: t.into(),
        ..Default::default()
    };
    let priv_t = c
        .coord()
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: r"fix C:\Users\stephen\mlg\x.go".into(),
                objective: "the build in ~/projects/mlg breaks".into(),
                done_criteria: "it builds".into(),
                links: vec![
                    "https://github.com/StephenSHorton/irori/pull/1".into(),
                    r"C:\Users\stephen\notes.txt".into(),
                ],
                steps: vec![step("read /home/me/logs/a.txt"), step("fix it")],
                ..Default::default()
            },
        )
        .unwrap();
    c.coord()
        .add_comment(&r.key_a, &priv_t.id, "private thoughts", false)
        .unwrap();
    let p = c.preview_share(&priv_t.id, &id).unwrap();
    assert!(
        p.title == "fix [path]"
            && p.goal == "the build in [path] breaks"
            && p.steps == ["read [path]", "fix it"],
        "preview {p:?}"
    );
    assert!(
        p.links.len() == 1
            && p.links[0].starts_with("https://")
            && !p.dropped.is_empty()
            && p.visible_to.len() == 1,
        "preview {p:?}"
    );
    // An edit after the preview is refused until it is previewed again.
    c.coord()
        .set_steps(
            &r.key_a,
            &priv_t.id,
            vec![
                step("read /home/me/logs/a.txt"),
                step("fix it"),
                step("post the plan"),
            ],
        )
        .unwrap();
    assert!(matches!(
        c.share_to_board(&priv_t.id, &id, &p.hash),
        Err(BoardError::PreviewStale(_))
    ));
    let p = c.preview_share(&priv_t.id, &id).unwrap();
    assert_eq!(p.steps.len(), 3);
    let shared = c.share_to_board(&priv_t.id, &id, &p.hash).unwrap();
    assert!(
        shared.title == p.title && shared.short_ref() == "mlg#1" && shared.steps.len() == 3,
        "{shared:?}"
    );
    let co = c.board_coord(&id).unwrap();
    assert!(
        co.comments(&shared.id).unwrap_or_default().is_empty(),
        "the private comments crossed"
    );
    let t2 = c.coord().get_task(&priv_t.id).unwrap();
    assert!(
        t2.state == coord::CANCELED && t2.note.contains("mlg#1"),
        "private card after share {t2:?}"
    );
    let e = match c.task_coord(&priv_t.id, std::slice::from_ref(&r.key_a)) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("a write to the private id was let through"),
    };
    assert!(
        e.contains("shared to a board") && e.contains("mlg#1") && e.contains("visible to"),
        "a write to the private id: {e}"
    );
    assert!(
        c.share_to_board(&priv_t.id, &id, &p.hash).is_err(),
        "a card was shared twice"
    );
    let sc = c.shared_copy_of(&priv_t.id).unwrap();
    assert!(sc.reference == "mlg#1" && sc.board_name == "MLG", "{sc:?}");

    c.withdraw_card(&shared.id).unwrap();
    let w = co.get_task(&shared.id).unwrap();
    assert!(
        w.withdrawn && w.state == coord::CANCELED,
        "withdrawn card {w:?}"
    );
    let back = c.coord().get_task(&priv_t.id).unwrap();
    assert!(
        !coord::is_terminal(&back.state),
        "the private card stayed closed: {back:?}"
    );
    c.task_coord(&priv_t.id, std::slice::from_ref(&r.key_a))
        .unwrap();
    assert!(
        c.withdraw_card(&priv_t.id).is_err(),
        "a private card was withdrawn"
    );
    r.core.close().unwrap();
}

/// TestTaskRefAcrossLogs: B3's rules (§2.5). A tagged ref names a card on
/// that board only; a bare number works when one log has it and is
/// refused, naming each card, when several do.
#[test]
fn task_ref_across_logs() {
    let r = Rig::new();
    let c = &r.core;
    let p1 = c
        .coord()
        .create_task(&r.key_a, new_task("private one"))
        .unwrap();
    let p2 = c
        .coord()
        .create_task(&r.key_a, new_task("private two"))
        .unwrap();
    let id = c.create_board("MLG", "mlg").unwrap();
    let co = c.board_coord(&id).unwrap();
    let by = c.board_key(&id, &c.user_sender().key).unwrap();
    let s1 = co.create_task(&by, new_task("shared one")).unwrap();
    for (rf, want) in [
        ("mlg#1", &s1.id),
        ("MLG#1", &s1.id),
        ("task:mlg#1", &s1.id),
        ("#2", &p2.id),
        ("2", &p2.id),
        (p1.id.as_str(), &p1.id),
    ] {
        assert_eq!(
            &c.task_ref(rf)
                .unwrap_or_else(|e| panic!("task_ref({rf}): {e}")),
            want,
            "task_ref({rf})"
        );
    }
    let e = c.task_ref("#1").unwrap_err();
    let msg = e.to_string();
    assert!(
        e.kind() == CK::Ambiguous && msg.contains("mlg#1") && msg.contains("#1 "),
        "bare #1: {msg}"
    );
    assert_eq!(c.task_ref("mlg#9").unwrap_err().kind(), CK::NotFound);
    assert_eq!(c.task_ref("xyz#1").unwrap_err().kind(), CK::NotFound);
    r.core.close().unwrap();
}

// ---- two devices of one person (Go pairedCores, newHCore) ----------------------

/// Boards stage 2 (card #44 step 5, boards2-transport.md §4): a board's
/// secret reaches the person's other devices on the private mesh as
/// `board.secret`, sealed under the mesh key. A board A makes opens on B
/// from the private board.joined row, and B ends up holding A's epoch 1
/// rather than minting its own.
#[test]
fn board_secret_reaches_own_devices() {
    let (a, b) = paired();
    let id = a.core.create_board("MLG", "mlg").unwrap();
    wait_for(45, "b opens the board", || b.core.board_coord(&id).is_ok());
    let sa = a.core.board_secrets(&id).unwrap();
    assert_eq!(sa.epoch(), 1);
    let sb = b.core.board_secrets(&id).unwrap();
    wait_for(45, "b holds the board's secret", || sb.secret(1).is_some());
    let (ka, ma) = sa.secret(1).unwrap();
    let (kb, mb) = sb.secret(1).unwrap();
    assert_eq!(ma, a.core.device_id());
    assert_eq!(mb, ma, "b holds a's secret, not one of its own");
    assert_eq!(*ka, *kb);
    a.core.close().unwrap();
    b.core.close().unwrap();
}

/// Boards stage 2 (card #60): an invitee with two devices. The invite
/// runs on one; the other opens the board from the private board.joined
/// row, gets its secret on board.secret and the board's rows from its own
/// device on `irori/board/1`, and then cards move between it and the
/// inviter with nothing copied by hand. Three made-up Cores, all on
/// 127.0.0.1.
#[test]
fn an_invitees_other_device_joins_the_board() {
    let (b1, b2) = paired(); // the invitee's two devices
    let (lc, nowhere) = (free_udp(), free_udp());
    let a = dev(
        tempfile::tempdir().unwrap(),
        "gamma",
        "cccccccc-3333-4333-8333-00000000000c",
        lc,
        nowhere,
    );
    let id = a.core.create_board("MLG & co", "mlg").unwrap();
    let ra = a.core.pairing_runtime().clone();
    let link = ra.block_on(a.core.invite(&id)).unwrap().link;
    let rb = b1.core.pairing_runtime().clone();
    rb.block_on(b1.core.join_invite(&link)).unwrap();
    wait_for(45, "the invitee's screen shows the words", || {
        b1.core.invite_status().status.state == "words"
            && a.core.invite_status().status.state == "words"
    });
    let words = b1.core.invite_status().status.words.join(" ");
    a.core.confirm_invite_words(&words).unwrap();
    wait_for(45, "the invite is done", || {
        a.core.invite_status().status.state == "done"
            && b1.core.invite_status().status.state == "done"
    });

    // The invitee's other device.
    wait_for(45, "b2 opens the board", || {
        b2.core.board_coord(&id).is_ok()
    });
    let v2 = b2.core.board_verifier(&id).unwrap();
    wait_for(45, "b2's cert and the board's rows arrive", || {
        v2.membership().created && v2.cert(b2.core.device_id()).is_some()
    });
    let s2 = b2.core.board_secrets(&id).unwrap();
    wait_for(45, "b2 holds the board's secret", || s2.secret(1).is_some());

    let me_a = format!("person:{}", a.core.identity().current().id);
    let from_a = a
        .core
        .board_coord(&id)
        .unwrap()
        .create_task(&me_a, new_task("from the inviter"))
        .unwrap();
    let co2 = b2.core.board_coord(&id).unwrap();
    wait_for(45, "b2 gets the inviter's card", || {
        co2.get_task(&from_a.id).is_some()
    });
    let by2 = b2.core.board_key(&id, &b2.user()).unwrap();
    let from_b2 = co2
        .create_task(&by2, new_task("from the invitee's other device"))
        .unwrap();
    let coa = a.core.board_coord(&id).unwrap();
    wait_for(45, "the inviter gets b2's card", || {
        coa.get_task(&from_b2.id).is_some()
    });
    // The inviter never joined the invitee's private mesh.
    for c in [&b1.core, &b2.core] {
        assert!(
            !c.net().conns().iter().any(|x| x.peer == a.core.device_id()),
            "the inviter is on the invitee's private mesh"
        );
    }
    for d in [a, b1, b2] {
        d.core.close().unwrap();
    }
}

/// TestBoardOnOwnDevices: step 4's one person, two devices. A board A
/// makes opens on B from the private board.joined row; its rows reach B on
/// the board stream, B's cert with them, so B writes as the person; what B
/// writes reaches A. The board's rows never enter either private log, and
/// Leave on B forgets the board on both.
#[test]
fn board_on_own_devices() {
    let (a, b) = paired();
    let id = a.core.create_board("MLG", "mlg").unwrap();
    wait_for(45, "b opens the board", || b.core.board_coord(&id).is_ok());
    assert!(b.data.join(BOARDS_DIR).join(format!("{id}.db")).exists());
    let bv = b.core.board_verifier(&id).unwrap();
    wait_for(45, "b's board.created and cert arrive", || {
        bv.membership().created && bv.cert(b.core.device_id()).is_some()
    });
    let got = b.core.board_list();
    assert!(
        got.len() == 1
            && got[0].name == "MLG"
            && got[0].tag == "mlg"
            && got[0].owner
            && got[0].ready,
        "b's boards {got:?}"
    );
    // The first secret is the creating device's alone (§6.1); b gets it on
    // board.secret (stage 2) rather than making its own.
    assert_eq!(a.core.board_info(&id).unwrap().secret_epoch, 1);
    let bs = b.core.board_secrets(&id).unwrap();
    wait_for(45, "b holds a's secret", || bs.secret(1).is_some());
    assert_eq!(
        bs.secret(1).unwrap().1,
        a.core.device_id(),
        "b made its own first secret"
    );

    // B writes a card on the board as the person; it reaches A.
    let by = b.core.board_key(&id, &b.user()).unwrap();
    let card = b
        .core
        .board_coord(&id)
        .unwrap()
        .create_task(&by, new_task("from the mac"))
        .unwrap();
    assert_eq!(card.short_ref(), "mlg#1");
    let aco = a.core.board_coord(&id).unwrap();
    wait_for(45, "a gets b's card", || aco.get_task(&card.id).is_some());
    assert_eq!(a.core.task_ref("mlg#1").unwrap(), card.id);

    // A's card reaches B the same way.
    let aby = a.core.board_key(&id, &a.user()).unwrap();
    let other = aco.create_task(&aby, new_task("from the pc")).unwrap();
    let bco = b.core.board_coord(&id).unwrap();
    wait_for(45, "b gets a's card", || bco.get_task(&other.id).is_some());

    // Nothing of the board is in either private log.
    for c in [&a.core, &b.core] {
        let evs = c
            .store()
            .list(&Filter {
                kinds: vec![
                    kinds::TASK_CREATED.into(),
                    kinds::BOARD_CREATED.into(),
                    "member.".into(),
                ],
                limit: 1000,
                ..Default::default()
            })
            .unwrap();
        for ev in evs {
            let s = String::from_utf8_lossy(&ev.payload);
            assert!(
                !s.contains(&card.id)
                    && !s.contains(&other.id)
                    && ev.kind != kinds::BOARD_CREATED
                    && !ev.kind.starts_with("member."),
                "{}'s private log holds a board row {}",
                c.device_id(),
                ev.kind
            );
        }
    }

    // Leave on B forgets the board on both devices.
    b.core.leave_board(&id).unwrap();
    assert!(
        b.core.board_coord(&id).is_err(),
        "b still holds the board it left"
    );
    wait_for(45, "a forgets the board b left", || {
        a.core.board_coord(&id).is_err()
    });
    assert!(!b.data.join(BOARDS_DIR).join(format!("{id}.db")).exists());
    a.core.close().unwrap();
    b.core.close().unwrap();
}

/// Card #233: a board that reaches another of the person's devices opens
/// there from board.joined before its rows arrive, nameless and not ready.
/// The window reads the boards list on each zero Change, so one has to
/// come after board.created lands, not only when the board opened:
/// otherwise Boards and the sidebar stay empty until a restart.
#[test]
fn a_board_from_another_device_tells_the_window_when_its_rows_land() {
    let (a, b) = paired();
    let q = b.core.subscribe_boards();
    let id = a.core.create_board("MLG", "mlg").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        assert!(
            !left.is_zero(),
            "no boards change after the board was ready on b; lists seen at each change: {seen:?}"
        );
        let Ok(Some(ch)) = q.recv_timeout(left.min(std::time::Duration::from_millis(200))) else {
            continue;
        };
        if !(ch.kind.is_empty() && ch.task_id.is_empty()) {
            continue;
        }
        // What the window would read now.
        let got = b.core.board_list();
        let ok = got.len() == 1 && got[0].board_id == id && got[0].ready && got[0].name == "MLG";
        seen.push(
            got.iter()
                .map(|s| format!("{}:{}:{}", s.name, s.tag, s.ready))
                .collect::<Vec<_>>(),
        );
        if ok {
            break;
        }
    }
    q.close();
    a.core.close().unwrap();
    b.core.close().unwrap();
}

/// TestBoardCardsAcrossOwnDevices: a shared card works with agents on any
/// of the person's devices (§2.4, §5.2). The PC assigns a shared card to
/// the Mac's agent, which hears it; both devices translate the agent's
/// card; a card shared from the PC is refused for writes on the Mac too;
/// and Withdraw carries the Mac agent's hold over to the private card.
#[test]
fn board_cards_across_own_devices() {
    let (pc, mac) = paired();
    wait_for(45, "the pc sees the mac's session", || {
        pc.core.mesh_agents().iter().any(|a| a.key == mac.agent)
    });
    let id = pc.core.create_board("MLG", "mlg").unwrap();
    wait_for(45, "the mac opens the board", || {
        mac.core
            .board_verifier(&id)
            .is_some_and(|v| v.membership().created)
    });
    let pco = pc.core.board_coord(&id).unwrap();

    // The PC puts the Mac's agent on a shared card.
    let me = pc.core.board_key(&id, &pc.user()).unwrap();
    let card = pco.create_task(&me, new_task("the mac builds it")).unwrap();
    let (co, keys) = pc
        .core
        .task_coord(&card.id, &[pc.user(), mac.agent.clone()])
        .unwrap();
    assert!(
        keys[1] != mac.agent && keys[1].starts_with(AGENT_ACTOR_PREFIX),
        "{}",
        keys[1]
    );
    let (_, n) = co.assign(&keys[0], &card.id, &keys[1]).unwrap();
    assert_eq!(
        pc.core.private_notice(n.clone()).to,
        vec![mac.agent.clone()]
    );
    pc.core.route_notices(&pc.core.user_sender(), &[n]).unwrap();
    wait_for(45, "the mac's agent hears the assignment", || {
        mac.core
            .inbox()
            .for_agent(&mac.agent, true, 50)
            .iter()
            .any(|d| d.message.ref_ == format!("task:{}", card.id) || d.message.ref_ == card.id)
    });
    wait_for(45, "the mac translates the card the pc made", || {
        mac.core.private_key(&keys[1]) == mac.agent
    });
    assert_eq!(mac.core.board_key(&id, &mac.agent).unwrap(), keys[1]);

    // A private card shared from the PC is closed to writes on the Mac.
    let priv_t = pc
        .core
        .coord()
        .create_task(&pc.agent, new_task("go public"))
        .unwrap();
    let p = pc.core.preview_share(&priv_t.id, &id).unwrap();
    let shared = pc.core.share_to_board(&priv_t.id, &id, &p.hash).unwrap();
    wait_for(45, "the mac learns the shared copy", || {
        mac.core
            .shared_copy_of(&priv_t.id)
            .is_some_and(|sc| sc.task_id == shared.id)
    });
    wait_for(45, "the mac has the private card", || {
        mac.core.coord().get_task(&priv_t.id).is_some()
    });
    match mac
        .core
        .task_coord(&priv_t.id, std::slice::from_ref(&mac.agent))
    {
        Err(e) => assert!(e.to_string().contains("shared to a board"), "{e}"),
        Ok(_) => panic!("a write to the shared private card on the mac went through"),
    }

    // The Mac's agent holds the shared card; Withdraw from the PC hands its
    // hold to the private card it came from.
    wait_for(45, "the mac has the shared card", || {
        mac.core.route(&shared.id).is_ok_and(|(b, _)| !b.is_empty())
    });
    let (mco, mkeys) = mac
        .core
        .task_coord(&shared.id, std::slice::from_ref(&mac.agent))
        .unwrap();
    mco.claim_task(&mkeys[0], &shared.id).unwrap();
    wait_for(45, "the pc sees the mac's claim", || {
        pco.get_task(&shared.id)
            .is_some_and(|x| x.holders.len() == 1)
    });
    let back = pc
        .core
        .withdraw_card(&shared.id)
        .unwrap()
        .expect("a private card");
    assert_eq!(back.id, priv_t.id);
    let got = pc.core.coord().get_task(&priv_t.id).unwrap();
    assert!(
        got.assigned
            .iter()
            .chain(&got.holders)
            .any(|k| *k == mac.agent),
        "the mac's agent lost its work on withdraw: {got:?}"
    );
    pc.core.close().unwrap();
    mac.core.close().unwrap();
}

/// TestBoardRosterOnlyTheSessionsDeviceSaysGone: a board key names no
/// device, so the board roster on one device never reads another device's
/// session as gone (v0.4 §6.0); this device's own session that went is
/// missing, so its holds go.
#[test]
fn board_roster_only_the_sessions_device_says_gone() {
    let r = Rig::new();
    let c = &r.core;
    let id = c.create_board("MLG", "mlg").unwrap();
    let st = c.board_store(&id).unwrap();
    let (far, near) = (
        "0123456789abcdef0123456789abcdef",
        "fedcba9876543210fedcba9876543210",
    );
    for (card, agent, name) in [
        (
            far,
            "laptop-dev/aaaaaaaa-1111-4111-8111-00000000000a".to_string(),
            "far",
        ),
        (near, format!("{}/no-such-session", c.device_id()), "near"),
    ] {
        c.store()
            .append(
                "irori",
                kinds::BOARD_CARD_OF,
                &serde_json::json!({"boardId": id, "cardId": card, "agentKey": agent}),
            )
            .unwrap();
        st.append(
            &format!("{AGENT_ACTOR_PREFIX}{card}"),
            kinds::AGENT_CARD,
            &serde_json::json!({"cardId": card, "name": name, "kind": "claude",
                "personId": c.person().id, "status": "active"}),
        )
        .unwrap();
    }
    wait_for(10, "both cards fold", || {
        c.private_key(&format!("{AGENT_ACTOR_PREFIX}{far}")) != format!("{AGENT_ACTOR_PREFIX}{far}")
            && c.private_key(&format!("{AGENT_ACTOR_PREFIX}{near}"))
                != format!("{AGENT_ACTOR_PREFIX}{near}")
            && c.board_roster(&id).iter().any(|a| a.key.ends_with(far))
    });
    let got: std::collections::HashMap<String, String> = c
        .board_roster(&id)
        .into_iter()
        .map(|a| (a.key, a.status))
        .collect();
    let fs = got.get(&format!("{AGENT_ACTOR_PREFIX}{far}"));
    assert!(
        fs.is_some_and(|s| s != "gone"),
        "another device's session read as gone here: {fs:?}"
    );
    assert!(
        !got.contains_key(&format!("{AGENT_ACTOR_PREFIX}{near}")),
        "this device's session that went is still listed"
    );
    drop(st);
    r.core.close().unwrap();
}

/// v0.5 §7.1, §7.3 on the real Core: an agent puts a card on a shared
/// board by tag, the result says who sees it, the board tool lists the
/// board, and the person's own rows never come back through the bridge.
#[test]
fn board_create_and_the_bridge_on_the_core() {
    let r = Rig::new();
    let c = &r.core;
    let b1 = c.create_board("MLG farming", "mlg").unwrap();
    let caller = |key: &str| tools::Caller {
        sender: inbox::Sender::new(key, "alpha", "claude"),
        path: String::new(),
    };
    let call = |key: &str, name: &str, args: serde_json::Value| {
        c.tools()
            .call(&caller(key), name, &serde_json::to_vec(&args).unwrap())
            .unwrap()
    };
    let out = call(
        &r.key_a,
        "board_create",
        serde_json::json!({"title": "Tune the loader", "board": "mlg", "assign_to": r.key_b}),
    );
    assert!(
        out.contains("Created mlg#1") && out.contains("Visible to: "),
        "{out}"
    );
    let co = c.board_coord(&b1).unwrap();
    assert_eq!(co.list_tasks(&coord::TaskFilter::default()).len(), 1);
    let out = call(&r.key_a, "board", serde_json::json!({}));
    assert!(
        out.contains("Shared boards you may post on") && out.contains("- mlg "),
        "{out}"
    );
    // beta, assigned, hears alpha's comment as a notice, never as a bridged
    // message from a board: sender.
    let out = call(
        &r.key_a,
        "board_comment",
        serde_json::json!({"task_id": "mlg#1", "text": "profiled it"}),
    );
    assert!(
        out.ends_with(&format!(
            "Visible to: {}",
            c.board_list()[0].people.join(", ")
        )),
        "{out}"
    );
    std::thread::sleep(std::time::Duration::from_millis(300));
    let ds = c.inbox().for_agent(&r.key_b, true, 0);
    assert!(!ds.is_empty(), "beta heard nothing");
    assert!(
        ds.iter().all(|d| !d.message.from.key.starts_with("board:")),
        "own rows came back through the bridge: {ds:?}"
    );
    r.core.close().unwrap();
}

/// v0.5 §2.3: a row naming a card the board doesn't hold yet waits in the
/// fold's set-aside, and the Core's board coord counts it for
/// `GET /v1/boards?detail=1`; the board still lists with its summary.
#[test]
fn a_row_naming_no_card_on_the_board_is_counted_as_unresolved() {
    let r = Rig::new();
    let c = &r.core;
    let id = c.create_board("mlg", "mlg").unwrap();
    let co = c.board_coord(&id).unwrap();
    assert_eq!(co.set_aside(), coord::SetAside::default());
    let ev = store::Event {
        device_id: "dev-elsewhere".into(),
        seq: 1,
        actor: format!("{AGENT_ACTOR_PREFIX}dev-elsewhere/s1"),
        kind: kinds::TASK_CLAIMED.into(),
        payload: br#"{"taskId":"no-such-card"}"#.to_vec(),
        ..Default::default()
    };
    co.fold_live(std::slice::from_ref(&ev));
    let aside = co.set_aside();
    assert_eq!((aside.unresolved, aside.dropped), (1, 0), "{aside:?}");
    let rows = c.board_list();
    assert!(
        rows.len() == 1 && rows[0].board_id == id && rows[0].ready && rows[0].owner,
        "{rows:?}"
    );
    r.core.close().unwrap();
}

/// Card #183: the person renames themself once, on one device. Their
/// other device takes the name from the person.profile row, and every
/// board they're on shows it (a fresh member.profile), on both devices.
#[test]
fn a_rename_reaches_every_device_and_board() {
    let (a, b) = paired();
    let id = a.core.create_board("MLG", "mlg").unwrap();
    wait_for(45, "b opens the board", || b.core.board_coord(&id).is_ok());
    let p = a.core.set_person_name("Stephen H").unwrap();
    assert_eq!(p.name, "Stephen H");
    assert_eq!(a.core.person().name, "Stephen H");
    let people = |c: &Core| {
        c.board_list()
            .into_iter()
            .find(|s| s.board_id == id)
            .map(|s| s.people)
            .unwrap_or_default()
    };
    wait_for(45, "a's board shows the new name", || {
        people(&a.core) == ["Stephen H"]
    });
    wait_for(45, "b takes the new name", || {
        b.core.person().name == "Stephen H"
    });
    wait_for(45, "b's copy of the board shows it", || {
        people(&b.core) == ["Stephen H"]
    });
    // The same name again writes nothing new; an empty one is refused.
    let before = a.core.store().list(&Filter::default()).unwrap().len();
    a.core.set_person_name("Stephen H").unwrap();
    assert_eq!(
        a.core.store().list(&Filter::default()).unwrap().len(),
        before
    );
    assert!(a.core.set_person_name("   ").is_err());
    a.core.close().unwrap();
    b.core.close().unwrap();
}

/// Card #180: a parent shares as a family. Its open subtasks go under the
/// board parent, an ended one stays private and is listed, a subtask alone
/// is refused naming its parent, and Withdraw brings the family back.
#[test]
fn share_and_withdraw_a_family() {
    let r = Rig::new();
    let c = &r.core;
    let id = c.create_board("MLG", "mlg").unwrap();
    let co = c.coord();
    let parent = co
        .create_task(&r.key_a, new_task("ship the release"))
        .unwrap();
    let sub = |t: &str| coord::NewSubtask {
        title: t.into(),
        ..Default::default()
    };
    let (_, kids) = co
        .split(
            &r.key_a,
            &parent.id,
            vec![
                sub("release notes"),
                sub("announce it in ~/notes/a.md"),
                sub("old idea"),
            ],
            vec![],
        )
        .unwrap();
    let user = c.user_sender().key;
    co.set_task_state(&user, &kids[2].id, coord::COMPLETED, "")
        .unwrap();

    // A subtask alone is refused, and says which card to share instead.
    let e = c.preview_share(&kids[0].id, &id).unwrap_err().to_string();
    assert!(
        e.contains("is a subtask of") && e.contains(&parent.short_ref()),
        "{e}"
    );

    let p = c.preview_share(&parent.id, &id).unwrap();
    assert_eq!(p.subtasks.len(), 2, "{p:?}");
    assert_eq!(p.subtasks[1].title, "announce it in [path]");
    assert!(
        p.dropped.iter().any(|d| d.contains(&kids[1].short_ref())),
        "a subtask's scrub is named: {:?}",
        p.dropped
    );
    assert!(
        p.kept_private.len() == 1 && p.kept_private[0].contains("(completed)"),
        "{:?}",
        p.kept_private
    );
    // The hash covers the subtasks: an edit to one wants a new preview.
    co.edit_task(
        &r.key_a,
        &kids[0].id,
        coord::TaskEdit {
            title: Some("release notes, v2".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(matches!(
        c.share_to_board(&parent.id, &id, &p.hash),
        Err(BoardError::PreviewStale(_))
    ));
    let p = c.preview_share(&parent.id, &id).unwrap();
    let shared = c.share_to_board(&parent.id, &id, &p.hash).unwrap();

    let bco = c.board_coord(&id).unwrap();
    let on_board = bco.get_task(&shared.id).unwrap();
    assert_eq!(on_board.subtasks.len(), 2, "{on_board:?}");
    let titles: Vec<String> = on_board
        .subtasks
        .iter()
        .map(|k| bco.get_task(k).unwrap().title)
        .collect();
    assert_eq!(titles, ["release notes, v2", "announce it in [path]"]);
    for k in &kids[..2] {
        let t = co.get_task(&k.id).unwrap();
        assert!(
            t.state == coord::CANCELED && t.note.contains("a subtask of mlg#1"),
            "private subtask after share: {t:?}"
        );
        assert!(c.shared_copy_of(&k.id).is_some());
    }
    let pt = co.get_task(&parent.id).unwrap();
    assert!(
        pt.state == coord::CANCELED && pt.note.starts_with("Shared to MLG as mlg#1"),
        "private parent after share: {pt:?}"
    );
    assert_eq!(
        co.get_task(&kids[2].id).unwrap().state,
        coord::COMPLETED,
        "the ended subtask stayed as it was"
    );

    // A board subtask alone isn't withdrawn; the parent takes them all back.
    let e = c
        .withdraw_card(&on_board.subtasks[0])
        .unwrap_err()
        .to_string();
    assert!(e.contains("withdraw mlg#1"), "{e}");
    c.withdraw_card(&shared.id).unwrap();
    for k in bco
        .get_task(&shared.id)
        .unwrap()
        .subtasks
        .iter()
        .chain([&shared.id])
    {
        let t = bco.get_task(k).unwrap();
        assert!(t.withdrawn && t.state == coord::CANCELED, "{t:?}");
    }
    for k in &kids[..2] {
        let t = co.get_task(&k.id).unwrap();
        assert!(!coord::is_terminal(&t.state), "subtask back: {t:?}");
    }
    let pt = co.get_task(&parent.id).unwrap();
    assert!(!coord::is_terminal(&pt.state), "parent back: {pt:?}");
    assert_eq!(pt.subtasks.len(), 3);
    r.core.close().unwrap();
}

/// Card #180: one subtask that can't go (an ask, which is never shared)
/// stops the whole family before anything is written.
#[test]
fn a_subtask_that_cant_go_stops_the_family() {
    let r = Rig::new();
    let c = &r.core;
    let id = c.create_board("MLG", "mlg").unwrap();
    let co = c.coord();
    let user = c.user_sender().key;
    let parent = co.create_task(&r.key_a, new_task("tidy the docs")).unwrap();
    co.split(
        &r.key_a,
        &parent.id,
        vec![coord::NewSubtask {
            title: "the readme".into(),
            ..Default::default()
        }],
        vec![],
    )
    .unwrap();
    let (ask, _) = co
        .post_ask(
            &user,
            coord::NewAsk {
                text: "can someone fix the install page?".into(),
                ..Default::default()
            },
            &[],
        )
        .unwrap();
    co.ask_close(
        &user,
        &ask.id,
        coord::AskOutcome {
            kind: coord::OUTCOME_CARD.into(),
            note: "a docs fix".into(),
            title: "the install page".into(),
            parent: parent.id.clone(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(co.get_task(&parent.id).unwrap().subtasks.len(), 2);
    let e = c.preview_share(&parent.id, &id).unwrap_err().to_string();
    assert!(e.contains("is an ask"), "{e}");
    assert!(
        c.board_coord(&id)
            .unwrap()
            .list_tasks(&coord::TaskFilter::default())
            .is_empty(),
        "nothing went to the board"
    );
    assert!(!coord::is_terminal(&co.get_task(&parent.id).unwrap().state));
    r.core.close().unwrap();
}

/// Card #337 (wind#87-91, 2026-11-03): a shared board's open card goes to
/// one of the person's idle agents that isn't on that board yet, which is
/// put on the board as it is handed the card (Stephen: "a board's pool
/// includes the person's own idle agents when the person is on that
/// board").
#[test]
fn a_board_card_skips_an_idle_agent_not_offered_cards() {
    // Card #453: a terminal agent the person didn't offer cards to isn't
    // put on a board to be handed one.
    let r = Rig::new();
    let c = &r.core;
    let b1 = c.create_board("Wind", "wind").unwrap();
    let caller = tools::Caller {
        sender: inbox::Sender::new(&r.key_a, "alpha", "claude"),
        path: String::new(),
    };
    let out = c
        .tools()
        .call(
            &caller,
            "board_create",
            &serde_json::to_vec(&serde_json::json!({"title": "Three Dolphins", "board": "wind"}))
                .unwrap(),
        )
        .unwrap();
    assert!(out.contains("Created wind#1"), "{out}");
    for _ in 0..5 {
        c.sweep_coord().ok();
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        !c.board_roster(&b1)
            .iter()
            .any(|a| c.private_key(&a.key) == r.key_b),
        "beta was put on the board"
    );
    assert!(
        !r.notices_to(&r.key_b)
            .iter()
            .any(|d| d.message.text.contains("handed it to you")),
        "beta was handed it"
    );
    assert!(!c.agent_offered(&r.key_b));
}

#[test]
fn a_board_card_reaches_an_idle_agent_not_on_the_board() {
    let r = Rig::new();
    let c = &r.core;
    let b1 = c.create_board("Wind", "wind").unwrap();
    let caller = |key: &str| tools::Caller {
        sender: inbox::Sender::new(key, "alpha", "claude"),
        path: String::new(),
    };
    let on_board = |k: &str| {
        c.board_roster(&b1)
            .iter()
            .any(|a| c.private_key(&a.key) == k)
    };
    // alpha posts it, so alpha is on the board and never gets its own card;
    // beta has never touched the board. Beta takes offered cards (#453).
    c.set_agent_offer(&r.key_b, true).unwrap();
    // Card #504: check beta is off the board before the card exists, not
    // after. The hand-out is the Core's own and runs on its threads: the
    // asks kick set_agent_offer just sent, a roster change, or the coord
    // sweep's tick can put beta on and hand it wind#1 any moment after
    // board_create, so a check there raced them under load. With no card
    // on the board nothing puts beta on it.
    assert!(
        !on_board(&r.key_b),
        "beta is on the board before any hand-out"
    );
    let out = c
        .tools()
        .call(
            &caller(&r.key_a),
            "board_create",
            &serde_json::to_vec(&serde_json::json!({"title": "Three Dolphins", "board": "wind"}))
                .unwrap(),
        )
        .unwrap();
    assert!(out.contains("Created wind#1"), "{out}");
    wait_for(10, "beta is handed wind#1", || {
        c.sweep_coord().ok();
        r.notices_to(&r.key_b).iter().any(|d| {
            d.message.text.contains("Three Dolphins") && d.message.text.contains("handed it to you")
        })
    });
    assert!(
        on_board(&r.key_b),
        "beta was handed it but isn't on the board"
    );
    let co = c.board_coord(&b1).unwrap();
    let t = co.list_tasks(&coord::TaskFilter::default()).remove(0);
    let o = t.live_offer().cloned().unwrap();
    assert_eq!(c.private_key(&o.to), r.key_b);
    r.core.close().unwrap();
}

/// Card #422: one card at a time holds across logs. alpha works wind#1 on
/// a shared board, so a private card beta posts unheld goes to gamma, who
/// is free everywhere, and alpha is told no when it tries to take it,
/// naming wind#1. Then gamma, working that private card, is told no on a
/// second board card, naming its private one.
#[test]
fn a_working_card_on_a_shared_board_keeps_the_agent_off_private_hand_outs() {
    let r = Rig::new();
    let c = &r.core;
    let b1 = c.create_board("Wind", "wind").unwrap();
    let co = c.board_coord(&b1).unwrap();
    let me = format!("{PERSON_ACTOR_PREFIX}{}", c.person().id);
    let caller = |key: &str, name: &str| tools::Caller {
        sender: inbox::Sender::new(key, name, "claude"),
        path: String::new(),
    };
    let call = |key: &str, name: &str, tool: &str, args: serde_json::Value| -> String {
        match c.tools().call(
            &caller(key, name),
            tool,
            &serde_json::to_vec(&args).unwrap(),
        ) {
            Ok(s) => s,
            Err(e) => format!("Error: {e}"),
        }
    };
    // alpha takes wind#1 up: working, not shared, no subtasks.
    let w = co.create_task(&me, new_task("Merge desk")).unwrap();
    let out = call(
        &r.key_a,
        "alpha",
        "board_join",
        serde_json::json!({"task_id": "wind#1"}),
    );
    assert!(!out.starts_with("Error"), "alpha joins wind#1: {out}");
    let card_a = c.board_key(&b1, &r.key_a).unwrap();
    let w = co.get_task(&w.id).unwrap();
    assert!(
        w.holders == [card_a.clone()] && w.state == coord::WORKING,
        "{w:?}"
    );

    // gamma comes up, free everywhere.
    let sess_c = "cccccccc-3333-4333-8333-000000000003";
    let key_c = format!("{}/{sess_c}", c.device_id());
    common::write_session(
        &r.claude.join("sessions"),
        "900003.json",
        sess_c,
        "gamma",
        &r.data,
    );
    wait_for(10, "presence lists gamma", || {
        c.local_agents().iter().any(|a| a.key == key_c)
    });
    // Both take offered cards (#453): alpha is left out only for wind#1.
    c.set_agent_offer(&r.key_a, true).unwrap();
    c.set_agent_offer(&key_c, true).unwrap();

    // beta posts a private card unheld: alpha is busy on wind#1, so gamma
    // is handed it.
    let out = call(
        &r.key_b,
        "beta",
        "board_create",
        serde_json::json!({"title": "Port the parser"}),
    );
    assert!(out.contains("Created"), "{out}");
    let p = c
        .coord()
        .list_tasks(&coord::TaskFilter::default())
        .into_iter()
        .find(|t| t.title == "Port the parser")
        .unwrap();
    wait_for(10, "the private card is handed to gamma", || {
        c.sweep_coord().ok();
        c.coord()
            .get_task(&p.id)
            .and_then(|t| t.live_offer().cloned())
            .is_some_and(|o| o.to == key_c)
    });
    let o = c.coord().get_task(&p.id).unwrap().offer.unwrap();
    assert!(!o.heard.contains(&r.key_a), "alpha was handed it: {o:?}");

    // alpha tries to take it anyway: told no, naming wind#1.
    let out = call(
        &r.key_a,
        "alpha",
        "board_join",
        serde_json::json!({"task_id": p.id}),
    );
    assert!(
        out.contains("one card at a time") && out.contains("wind#1") && out.contains("Merge desk"),
        "alpha took a second card: {out}"
    );
    assert!(c.coord().get_task(&p.id).unwrap().holders.is_empty());

    // gamma takes it, then is told no on a second board card, naming the
    // private one.
    let out = call(
        &key_c,
        "gamma",
        "board_join",
        serde_json::json!({"task_id": p.id}),
    );
    assert!(!out.starts_with("Error"), "gamma joins: {out}");
    let w2 = co.create_task(&me, new_task("Second desk")).unwrap();
    let out = call(
        &key_c,
        "gamma",
        "board_join",
        serde_json::json!({"task_id": w2.short_ref()}),
    );
    assert!(
        out.contains("one card at a time") && out.contains("Port the parser"),
        "gamma took a board card while working a private one: {out}"
    );
    assert!(co.get_task(&w2.id).unwrap().holders.is_empty());
    r.core.close().unwrap();
}
