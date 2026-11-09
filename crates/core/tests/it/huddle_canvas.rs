//! A huddle's canvas rows (card #1051, docs/design/call-agents.md §8) through
//! the real Core on loopback iroh: items written by a person, by their
//! assistant (the person's row, flagged) and their changes reach every member
//! device in a channel, a person DM and a Personal channel; an assistant's
//! status, card and position are ignored; a row naming another conversation,
//! or written under someone else's name or by an agent in a DM, is dropped;
//! a space that can't store `call.` rows refuses the write. Made-up people
//! only.

use coord::talk::{CanvasItem, NewItem, NewItemSet};
use store::kinds;

use crate::common::mesh::{Dev, invited, person};
use crate::common::pair::paired;
use crate::common::wait_for;

const WAIT: u64 = 60;
const CH_CALL: &str = "0123456789abcdef0123456789abcdef";
const DM_CALL: &str = "fedcba9876543210fedcba9876543210";
const SELF_CALL: &str = "00112233445566778899aabbccddeeff";

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn line(d: &Dev, conv: &str, call: &str, text: &str) {
    let t = now_ms();
    d.core
        .talk()
        .write_segment(conv, call, 0, t, t + 900, text, &d.user())
        .unwrap();
}

fn items(d: &Dev, call: &str) -> Vec<CanvasItem> {
    d.core.talk().call_items(call)
}

fn item(d: &Dev, call: &str, id: &str) -> Option<CanvasItem> {
    items(d, call).into_iter().find(|i| i.item_id == id)
}

fn new(kind: &str, text: &str) -> NewItem {
    NewItem {
        kind: kind.into(),
        text: text.into(),
        ..NewItem::default()
    }
}

/// Alice writes a person's item and her assistant's, then settles and moves
/// them; every device of the place sees the same canvas.
fn exercise(writer: &Dev, readers: &[&Dev], conv: &str, call: &str) {
    // A person's own item keeps the status they give it; the assistant's
    // reads open whatever it says, and its set can't settle or move it.
    let mine = writer
        .core
        .huddle_item_add(call, "decision", "Keep the old updater.", "settled")
        .unwrap();
    assert_eq!(mine.status, "settled");
    let asst = writer
        .core
        .talk()
        .write_item(
            conv,
            call,
            NewItem {
                status: "settled".into(),
                assistant: true,
                asker: Some(writer.user()),
                why: Some("name".into()),
                ..new("answer", "Addam has #412.")
            },
            &writer.user(),
        )
        .unwrap();
    assert_eq!(asst.status, "open");
    writer
        .core
        .talk()
        .write_item_set(
            conv,
            call,
            NewItemSet {
                item_id: asst.item_id.clone(),
                status: Some("dropped".into()),
                x: Some(9.0),
                y: Some(9.0),
                card: Some("mlg#1".into()),
                text: Some("Addam has #412 on signing.".into()),
                assistant: true,
            },
            &writer.user(),
        )
        .unwrap();
    // A person's set settles and moves; `carded` is honoured by the fold
    // whenever it arrives (the Core's write path is what checks liveness).
    writer
        .core
        .huddle_item_set(
            call,
            NewItemSet {
                item_id: asst.item_id.clone(),
                status: Some("settled".into()),
                x: Some(120.0),
                y: Some(-40.0),
                ..NewItemSet::default()
            },
        )
        .unwrap();
    writer
        .core
        .talk()
        .write_item_set(
            conv,
            call,
            NewItemSet {
                item_id: mine.item_id.clone(),
                status: Some("carded".into()),
                card: Some("mlg#7".into()),
                ..NewItemSet::default()
            },
            &writer.user(),
        )
        .unwrap();
    for d in readers.iter().copied().chain([writer]) {
        wait_for(WAIT, "the canvas arrives", || {
            item(d, call, &asst.item_id).is_some_and(|i| i.x == Some(120.0))
                && item(d, call, &mine.item_id).is_some_and(|i| i.status == "carded")
        });
        let a = item(d, call, &asst.item_id).unwrap();
        assert!(a.assistant, "an assistant's item is never the person's");
        assert_eq!(a.status, "settled", "the person's set, not the assistant's");
        assert_eq!((a.x, a.y), (Some(120.0), Some(-40.0)));
        assert_eq!(a.card, "", "an assistant's card is ignored");
        assert_eq!(a.text, "Addam has #412 on signing.", "its own item's text");
        assert_eq!(a.why.as_deref(), Some("name"));
        assert!(a.asker.is_some(), "the asker rides along");
        let m = item(d, call, &mine.item_id).unwrap();
        assert!(!m.assistant);
        assert_eq!((m.status.as_str(), m.card.as_str()), ("carded", "mlg#7"));
    }
}

#[test]
fn canvas_rows_reach_every_member_in_a_channel_and_a_dm() {
    let alice = person("alice", 1);
    let bob = person("bob", 2);
    let (a1, b1, b2) = (&alice[0], &bob[0], &bob[1]);
    let space = invited(a1, b1);
    wait_for(WAIT, "b2 opens the space", || {
        b2.core
            .board_list()
            .iter()
            .any(|s| s.board_id == space && s.ready)
    });
    wait_for(WAIT, "alice hears bob read spaces", || {
        a1.core.space_not_ready(&space).unwrap().is_empty()
    });
    a1.core.space_gate(&space).unwrap();
    let ch = a1
        .core
        .talk_channel_create_in(&a1.user(), "general", "", &space)
        .unwrap();
    let other = a1
        .core
        .talk_channel_create_in(&a1.user(), "other", "", &space)
        .unwrap();
    for d in [b1, b2] {
        wait_for(WAIT, "bob's device holds the channels", || {
            d.core.talk().channel(&ch.id).is_some() && d.core.talk().channel(&other.id).is_some()
        });
    }
    line(a1, &ch.id, CH_CALL, "morning");
    for d in [b1, b2] {
        wait_for(WAIT, "bob's device holds the huddle", || {
            d.core.talk().call_conv(CH_CALL).is_some()
        });
    }
    exercise(a1, &[b1, b2], &ch.id, CH_CALL);

    // Bob's item is signed by him: Alice sees his key, and the asker mapped
    // to the same person on both sides.
    let q = b1
        .core
        .talk()
        .write_item(
            &ch.id,
            CH_CALL,
            NewItem {
                asker: Some(b1.user()),
                ..new("question", "Who has signing?")
            },
            &b1.user(),
        )
        .unwrap();
    wait_for(WAIT, "alice has bob's question", || {
        item(a1, CH_CALL, &q.item_id).is_some()
    });
    let on_alice = item(a1, CH_CALL, &q.item_id).unwrap();
    assert_eq!(on_alice.by, b1.person_actor());
    assert_eq!(on_alice.asker.as_deref(), Some(b1.person_actor().as_str()));
    let shown = a1.core.call_canvas(CH_CALL);
    let e = shown.iter().find(|e| e.item.item_id == q.item_id).unwrap();
    assert!(!e.mine && !e.asker_mine && e.asker_name == e.by_name);

    // Forged rows are dropped on every device: someone else's name; a
    // conversation that isn't the huddle's; a flag-less row carrying an
    // assistant's claim doesn't matter (status is the fold's).
    let me = a1.person_actor();
    let forge = |actor: &str, by: &str, conv: &str, id: &str| {
        let _ = a1.core.board_store(&space).unwrap().append(
            actor,
            kinds::CALL_ITEM,
            &serde_json::json!({
                "itemId": id, "callId": CH_CALL, "convId": conv, "kind": "note",
                "text": "forged", "status": "", "assistant": false, "by": by,
                "at": "2026-11-08T00:00:00Z",
            }),
        );
    };
    forge(&me, &b1.person_actor(), &ch.id, "forged1");
    forge(&me, &me, &other.id, "forged2");
    a1.core.kick_board(&space);
    // A real item after them shows the forged ones came and went.
    let after = b1
        .core
        .huddle_item_add(CH_CALL, "note", "after the forgeries", "")
        .unwrap();
    for d in [a1, b1, b2] {
        wait_for(WAIT, "every device shows the real item", || {
            item(d, CH_CALL, &after.item_id).is_some()
        });
        assert!(item(d, CH_CALL, "forged1").is_none());
        assert!(item(d, CH_CALL, "forged2").is_none());
    }

    // A person DM: the pair's hidden space carries it, and only the pair.
    let dm = a1.core.talk_dm(&a1.user(), &b1.person_actor()).unwrap();
    a1.core
        .talk_post(&a1.core.user_sender(), &dm, "hi", &[], false, None, vec![])
        .unwrap();
    wait_for(WAIT, "bob reads alice's DM", || {
        b1.core
            .talk_posts(&b1.user(), &dm, None, 50)
            .is_ok_and(|ps| ps.iter().any(|p| p.text == "hi"))
    });
    let spaces = a1.core.dm_spaces_with(&b1.core.person().id);
    assert_eq!(spaces.len(), 1, "{spaces:?}");
    wait_for(WAIT, "the DM's space stops being pending", || {
        !a1.core.dm_pending(&spaces[0])
    });
    wait_for(WAIT, "the DM's space takes call rows", || {
        a1.core.space_tolerance(&spaces[0]).is_ok_and(|t| t.ready())
    });
    wait_for(WAIT, "the DM takes the line", || {
        let t = now_ms();
        a1.core
            .talk()
            .write_segment(&dm, DM_CALL, 0, t, t + 900, "just us", &a1.user())
            .is_ok()
    });
    wait_for(WAIT, "bob holds the DM huddle", || {
        b1.core.talk().call_conv(DM_CALL).is_some()
    });
    exercise(a1, &[b1], &dm, DM_CALL);

    // An agent's item, and one naming another conversation, fold into no
    // DM; the DM's space refuses to write them at all.
    let dm_space = &spaces[0];
    let agent = a1.core.board_store(dm_space).unwrap().append(
        "agent:card1",
        kinds::CALL_ITEM,
        &serde_json::json!({
            "itemId": "agentitem", "callId": DM_CALL, "convId": dm, "kind": "note",
            "text": "agent", "status": "", "assistant": false, "by": "agent:card1",
            "at": "2026-11-08T00:00:00Z",
        }),
    );
    let _ = agent;
    let wrong = a1.core.board_store(dm_space).unwrap().append(
        &me,
        kinds::CALL_ITEM,
        &serde_json::json!({
            "itemId": "wrongconv", "callId": DM_CALL, "convId": ch.id, "kind": "note",
            "text": "wrong place", "status": "", "assistant": false, "by": me,
            "at": "2026-11-08T00:00:00Z",
        }),
    );
    let _ = wrong;
    a1.core.kick_board(dm_space);
    let real = b1
        .core
        .huddle_item_add(DM_CALL, "note", "real DM item", "")
        .unwrap();
    wait_for(WAIT, "alice shows the real DM item", || {
        item(a1, DM_CALL, &real.item_id).is_some()
    });
    for d in [a1, b1] {
        assert!(item(d, DM_CALL, "agentitem").is_none());
        assert!(item(d, DM_CALL, "wrongconv").is_none());
    }

    for d in alice.iter().chain(bob.iter()) {
        d.core.close().unwrap();
    }
}

/// A Personal channel's canvas is the person's own devices'.
#[test]
fn canvas_rows_reach_the_persons_other_device_in_a_personal_channel() {
    let bob = person("bob", 2);
    let (b1, b2) = (&bob[0], &bob[1]);
    let ch = b1
        .core
        .talk()
        .create_channel(&b1.user(), "notes", "")
        .unwrap();
    line(b1, &ch.id, SELF_CALL, "to myself");
    wait_for(WAIT, "the other device holds the huddle", || {
        b2.core.talk().call_conv(SELF_CALL).is_some()
    });
    exercise(b1, &[b2], &ch.id, SELF_CALL);
    // The other device can write too.
    let x = b2
        .core
        .huddle_item_add(SELF_CALL, "task", "From the other device.", "")
        .unwrap();
    wait_for(WAIT, "the first device has it", || {
        item(b1, SELF_CALL, &x.item_id).is_some()
    });
    for d in &bob {
        d.core.close().unwrap();
    }
}

/// A space where a device can't store `call.` rows yet refuses the write,
/// and writes nothing.
#[test]
fn a_space_that_isnt_ready_refuses_canvas_writes() {
    let (a, b) = paired();
    let id = a.core.create_board("Huddles", "huddle").unwrap();
    let ch = a
        .core
        .talk()
        .create_channel_in(&a.user(), "general", "", &id)
        .unwrap();
    a.core
        .board_store(&id)
        .unwrap()
        .append_local(
            "irori",
            mesh::boardmesh::KIND_BOARD_HELLO,
            &mesh::boardmesh::SavedHello {
                device_id: b.core.device_id().to_string(),
                core_version: "0.12.4".into(),
                spaces: 1,
                spawns: 1,
                tolerant: 1,
                seq: i64::MAX,
                ..Default::default()
            },
        )
        .unwrap();
    let e = a
        .core
        .talk()
        .write_item(&ch.id, CH_CALL, new("note", "too early"), &a.user())
        .unwrap_err();
    assert!(e.to_string().contains("can't store call rows"), "{e}");
    assert!(a.core.talk().call_items(CH_CALL).is_empty());
}
