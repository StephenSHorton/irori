//! A huddle's title and "over" (card #1043, docs/design/call-agents.md §3,
//! §8), through the real Core on loopback iroh: a title set by a person in a
//! channel, a person DM and a Personal channel reaches every member device
//! and beats an assistant's; a title naming another conversation, or written
//! under someone else's name, is dropped; a space that can't store `call.`
//! rows refuses the write; `over` is true after an end mark and for a huddle
//! whose lines went quiet, and never for one a device finds mid-call. Made-up
//! people only.

use coord::talk::{CALL_END, CALL_START, CallInfo, CallMark};
use irori_core::Core;
use store::{Filter, kinds};

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

fn info(c: &Core, call: &str) -> Option<CallInfo> {
    c.talk().calls().into_iter().find(|i| i.call_id == call)
}

fn title(c: &Core, call: &str) -> String {
    info(c, call).map(|i| i.title).unwrap_or_default()
}

fn titles(c: &Core, space: Option<&str>) -> Vec<store::Event> {
    let f = Filter {
        kinds: vec![kinds::CALL_TITLE.into()],
        ..Default::default()
    };
    match space {
        Some(s) => c.board_store(s).unwrap().list(&f).unwrap(),
        None => Vec::new(),
    }
}

fn line(d: &Dev, conv: &str, call: &str, text: &str) {
    let t = now_ms();
    d.core
        .talk()
        .write_segment(conv, call, 0, t, t + 900, text, &d.user())
        .unwrap();
}

fn linked(a: &Dev, b: &Dev, board: &str) {
    let peer = b.core.device_id().to_string();
    wait_for(WAIT, "the two devices connect on their space", || {
        a.core
            .net()
            .board_conns(board)
            .iter()
            .any(|c| c.peer == peer)
    });
}

/// A DM that has reached both sides, and the hidden space it lives in.
fn dm_started(a: &Dev, b: &Dev) -> (String, String) {
    let dm = a.core.talk_dm(&a.user(), &b.person_actor()).unwrap();
    a.core
        .talk_post(&a.core.user_sender(), &dm, "hi", &[], false, None, vec![])
        .unwrap();
    wait_for(WAIT, "bob reads alice's DM", || {
        b.core
            .talk_posts(&b.user(), &dm, None, 50)
            .is_ok_and(|ps| ps.iter().any(|p| p.text == "hi"))
    });
    let spaces = a.core.dm_spaces_with(&b.core.person().id);
    assert_eq!(spaces.len(), 1, "{spaces:?}");
    wait_for(WAIT, "the DM's space stops being pending", || {
        !a.core.dm_pending(&spaces[0])
    });
    wait_for(WAIT, "the DM's space takes call rows", || {
        a.core.space_tolerance(&spaces[0]).is_ok_and(|t| t.ready())
    });
    (dm, spaces[0].clone())
}

/// Titles in a space's channel and in a person DM reach the other person's
/// devices, a person's newest beats an assistant's, and a forged row is
/// dropped.
#[test]
fn a_title_reaches_every_member_device_in_a_channel_and_a_dm() {
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

    // The huddle exists once someone has spoken in it.
    line(a1, &ch.id, CH_CALL, "morning");
    for d in [b1, b2] {
        wait_for(WAIT, "bob's device holds the huddle", || {
            info(&d.core, CH_CALL).is_some()
        });
    }

    // An assistant proposes a title; a person's replaces it, and an
    // assistant's later one doesn't.
    a1.core
        .talk()
        .write_title(&ch.id, CH_CALL, "Standup?", true, &a1.user())
        .unwrap();
    assert_eq!(
        a1.core.call_title_set(CH_CALL, " Signing plan ").unwrap(),
        "Signing plan"
    );
    a1.core
        .talk()
        .write_title(&ch.id, CH_CALL, "Wrap-up", true, &a1.user())
        .unwrap();
    for d in [b1, b2] {
        wait_for(WAIT, "bob's device shows the person's title", || {
            title(&d.core, CH_CALL) == "Signing plan"
        });
        let i = info(&d.core, CH_CALL).unwrap();
        assert!(!i.title_by_assistant);
    }
    assert_eq!(title(&a1.core, CH_CALL), "Signing plan");

    // A title written under someone else's name, and one naming another
    // conversation of the space, are dropped on every device.
    let forge = |actor: &str, by: &str, conv: &str, text: &str| {
        // The log may refuse some of these outright; what it keeps, the
        // fold must still drop.
        let _ = a1.core.board_store(&space).unwrap().append(
            actor,
            kinds::CALL_TITLE,
            &serde_json::json!({
                "callId": CH_CALL, "convId": conv, "title": text,
                "assistant": false, "by": by, "at": "2026-11-08T00:00:00Z",
            }),
        );
    };
    let me = a1.person_actor();
    forge(&me, &b1.person_actor(), &ch.id, "Not alice's to say");
    forge(&me, &me, &other.id, "Elsewhere");
    a1.core.kick_board(&space);
    // The next real title shows the forged ones came and went.
    b1.core.call_title_set(CH_CALL, "Signing, v2").unwrap();
    b1.core.kick_board(&space);
    for d in [a1, b1, b2] {
        wait_for(WAIT, "every device shows the newest person title", || {
            title(&d.core, CH_CALL) == "Signing, v2"
        });
    }

    // A person DM: the pair's hidden space carries it.
    let (dm, dm_space) = {
        linked(a1, b1, &space);
        dm_started(a1, b1)
    };
    // Bob's second device may still be joining the DM's space, which the
    // gate waits for: write when it takes call rows.
    wait_for(WAIT, "the DM takes the line", || {
        let t = now_ms();
        a1.core
            .talk()
            .write_segment(&dm, DM_CALL, 0, t, t + 900, "just us", &a1.user())
            .is_ok()
    });
    wait_for(WAIT, "bob holds the DM huddle", || {
        info(&b1.core, DM_CALL).is_some()
    });
    wait_for(WAIT, "the DM takes the title", || {
        a1.core.call_title_set(DM_CALL, "One on one").is_ok()
    });
    wait_for(WAIT, "bob's device shows the DM's title", || {
        title(&b1.core, DM_CALL) == "One on one"
    });
    assert_eq!(
        titles(&a1.core, Some(&dm_space)).len(),
        1,
        "the title rode the DM's own space"
    );
    // A title for a DM huddle naming another conversation folds to nothing.
    a1.core
        .board_store(&dm_space)
        .unwrap()
        .append(
            &me,
            kinds::CALL_TITLE,
            &serde_json::json!({
                "callId": DM_CALL, "convId": ch.id, "title": "Wrong place",
                "assistant": false, "by": me, "at": "2026-11-08T00:00:00Z",
            }),
        )
        .unwrap();
    a1.core.kick_board(&dm_space);
    wait_for(WAIT, "bob's DM takes the title", || {
        b1.core.call_title_set(DM_CALL, "Still one on one").is_ok()
    });
    wait_for(WAIT, "alice shows bob's DM title", || {
        title(&a1.core, DM_CALL) == "Still one on one"
    });

    for d in alice.iter().chain(bob.iter()) {
        d.core.close().unwrap();
    }
}

/// A Personal channel's huddle title is the person's own devices'.
#[test]
fn a_title_reaches_the_persons_other_device_in_a_personal_channel() {
    let bob = person("bob", 2);
    let (b1, b2) = (&bob[0], &bob[1]);
    let ch = b1
        .core
        .talk()
        .create_channel(&b1.user(), "notes", "")
        .unwrap();
    line(b1, &ch.id, SELF_CALL, "to myself");
    b1.core.call_title_set(SELF_CALL, "Notes to self").unwrap();
    assert_eq!(title(&b1.core, SELF_CALL), "Notes to self");
    wait_for(WAIT, "the other device shows the title", || {
        title(&b2.core, SELF_CALL) == "Notes to self"
    });
    for d in &bob {
        d.core.close().unwrap();
    }
}

/// A space where a device can't store `call.` rows yet refuses the title
/// write, and writes nothing; once it can, the title goes in.
#[test]
fn a_space_that_isnt_ready_refuses_the_title() {
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
        .write_title(&ch.id, CH_CALL, "Too early", false, &a.user())
        .unwrap_err();
    assert!(e.to_string().contains("can't store call rows"), "{e}");
    assert!(titles(&a.core, Some(&id)).is_empty(), "nothing was written");
    assert_eq!(title(&a.core, CH_CALL), "");

    wait_for(WAIT, "b opens the space", || {
        b.core.board_coord(&id).is_ok()
    });
    wait_for(WAIT, "a hears b at tolerant 2", || {
        a.core.space_tolerance(&id).unwrap().ready()
    });
    a.core
        .talk()
        .write_title(&ch.id, CH_CALL, "Now it can", false, &a.user())
        .unwrap();
    assert_eq!(titles(&a.core, Some(&id)).len(), 1);
}

/// `over`: true after an end mark, here and on the other person's device;
/// true for a huddle whose lines are old and that this device never heard
/// live; false for one with recent rows, which is all a device that starts
/// mid-call has.
#[test]
fn over_after_an_end_mark_and_never_on_a_device_that_starts_mid_call() {
    let alice = person("alice", 1);
    let bob = person("bob", 1);
    let (a, b) = (&alice[0], &bob[0]);
    let space = invited(a, b);
    wait_for(WAIT, "alice hears bob read spaces", || {
        a.core.space_not_ready(&space).unwrap().is_empty()
    });
    a.core.space_gate(&space).unwrap();
    let ch = a
        .core
        .talk_channel_create_in(&a.user(), "general", "", &space)
        .unwrap();
    wait_for(WAIT, "bob holds the channel", || {
        b.core.talk().channel(&ch.id).is_some()
    });

    // A huddle in progress: a start mark and a fresh line. Bob never heard
    // it live (no frame), and it is not over for him or for Alice.
    let mark = |op: &str| CallMark {
        call_id: CH_CALL.into(),
        op: op.into(),
        ..CallMark::default()
    };
    a.core
        .post_call_mark(&ch.id, "started a huddle", mark(CALL_START))
        .unwrap();
    line(a, &ch.id, CH_CALL, "we are talking");
    wait_for(WAIT, "bob holds the huddle", || {
        info(&b.core, CH_CALL).is_some_and(|i| i.lines == 1 && i.start_at.is_some())
    });
    assert!(!a.core.call_over(CH_CALL));
    assert!(!b.core.call_over(CH_CALL), "a device finding it mid-call");
    // Not knowing counts as live.
    assert!(!b.core.call_over("ffffffffffffffffffffffffffffffff"));

    // A huddle whose last row is 20 minutes old and that nobody here heard
    // live: over (a crash nobody wrote an end mark for).
    let old = now_ms() - 20 * 60 * 1000;
    a.core
        .talk()
        .write_segment(&ch.id, DM_CALL, 0, old, old + 900, "long ago", &a.user())
        .unwrap();
    wait_for(WAIT, "bob holds the old huddle", || {
        info(&b.core, DM_CALL).is_some()
    });
    assert!(a.core.call_over(DM_CALL));
    assert!(b.core.call_over(DM_CALL));

    // The end mark makes the first one over everywhere.
    let mut end = mark(CALL_END);
    end.duration_ms = 5_000;
    a.core
        .post_call_mark(&ch.id, "ended a huddle", end)
        .unwrap();
    a.core.kick_board(&space);
    assert!(a.core.call_over(CH_CALL));
    wait_for(WAIT, "bob sees it over", || b.core.call_over(CH_CALL));
    for d in alice.iter().chain(bob.iter()) {
        d.core.close().unwrap();
    }
}
