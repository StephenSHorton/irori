//! Go's core board tests (I:internal/core/boards_test.go,
//! boards_own_test.go): shared boards on one person's devices, through the
//! Core's public calls. Made-up fixture identities, the file keystore, no
//! network on a single device.

mod common;

use std::time::Duration;

use common::{Rig, wait_for};
use coord::ErrorKind as CK;
use irori_core::{BOARDS_DIR, BoardError, Core, valid_board_id};
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

    // After a restart the boards are found on disk and the index comes back.
    let (c2, _r) = restart(r);
    assert_eq!(c2.boards().len(), 2, "boards after restart");
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
    let topics = c.board_secrets(&id).unwrap().topics().unwrap();
    assert!(
        topics.len() == 2
            && topics[0].epoch == 2
            && topics[1].epoch == 1
            && topics[0].topic != topics[1].topic,
        "topics after the rotation"
    );

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

/// One device of the pair: its Core and its one live session.
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
}

fn free_udp() -> std::net::SocketAddr {
    std::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
}

/// A Core with one live session `name`/`sess`, whose mesh runs on loopback
/// iroh with a LAN lookup unicast to `to` (as tests/pair.rs).
fn dev(
    data: tempfile::TempDir,
    name: &str,
    sess: &str,
    lan: std::net::SocketAddr,
    to: std::net::SocketAddr,
) -> Dev {
    let (cl, w) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let sessions = cl.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    common::write_session(&sessions, "900001.json", sess, name, w.path());
    let mut o = common::options(data.path(), cl.path());
    o.pair_net = Some(std::sync::Arc::new(move |ident, chain, guests| {
        let mut nc = irori_core::devices::default_net_config(ident, chain, guests);
        nc.ip = irori_net::endpoint::IpMode::Loopback;
        let lc = nc.lan_config.as_mut().expect("a LAN lookup");
        lc.bind = Some(lan);
        lc.targets = Some(std::sync::Arc::new(move || vec![to]));
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

/// Two Cores of one person, B certified by A as pairing does, waiting
/// until each owns the other's user key.
fn paired() -> (Dev, Dev) {
    let (la, lb) = (free_udp(), free_udp());
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = dev(da, "alpha", "aaaaaaaa-1111-4111-8111-00000000000a", la, lb);
    std::fs::copy(
        a.data.join(irori_identity::PERSON_FILE),
        db.path().join(irori_identity::PERSON_FILE),
    )
    .unwrap();
    let b = dev(db, "beta", "bbbbbbbb-2222-4222-8222-00000000000b", lb, la);
    let add = a
        .core
        .identity()
        .cert_for(
            b.core.device_id(),
            &hex::encode(b.core.identity().device_pub()),
            "mac",
            irori_identity::ROLE_DESKTOP,
            presence::now_local(),
        )
        .unwrap();
    a.core
        .store()
        .append("irori", kinds::PERSON_DEVICE_ADDED, &add)
        .unwrap();
    let person = a
        .core
        .store()
        .list(&Filter {
            kinds: vec!["person.".into()],
            ..Default::default()
        })
        .unwrap();
    for _ in 0..2 {
        for ev in &person {
            let _ = b.core.chain().learn(ev.into());
        }
    }
    b.core.mesh().kick();
    wait_for(45, "a and b pair", || {
        a.core.is_owner(&format!("user:{}", b.core.device_id()))
            && b.core.is_owner(&format!("user:{}", a.core.device_id()))
    });
    (a, b)
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
    // The first secret is the creating device's alone (§6.1).
    assert_eq!(a.core.board_info(&id).unwrap().secret_epoch, 1);
    assert_eq!(
        b.core.board_info(&id).unwrap().secret_epoch,
        0,
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
