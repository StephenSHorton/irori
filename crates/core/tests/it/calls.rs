//! A huddle's start and end posts (#654) through the real Core on loopback
//! iroh: they reach the other person in a space's channel and in a person
//! DM with their mark, its people in each reader's own form. Made-up
//! people only.

use coord::talk::{CALL_END, CALL_START, CallMark};
use store::{Filter, kinds};

use crate::common::mesh::{Dev, invited, person};
use crate::common::wait_for;

const WAIT: u64 = 45;

fn close(devs: impl IntoIterator<Item = Dev>) {
    for d in devs {
        d.core.close().unwrap();
    }
}

fn mark(id: &str, op: &str, people: Vec<String>) -> CallMark {
    CallMark {
        call_id: id.into(),
        op: op.into(),
        duration_ms: if op == CALL_END { 90_000 } else { 0 },
        people,
    }
}

/// The mark on `d`'s post `post_id` in `conv`, once it has it.
fn their_mark(d: &Dev, conv: &str, post_id: &str) -> Option<CallMark> {
    d.core
        .talk_posts(&d.user(), conv, None, 50)
        .ok()?
        .into_iter()
        .find(|p| p.post_id == post_id)?
        .call
}

/// The `call` object of post `post_id` as space `space`'s log holds it.
fn logged(d: &Dev, space: &str, post_id: &str) -> serde_json::Value {
    let st = d.core.board_store(space).unwrap();
    st.list(&Filter {
        kinds: vec![kinds::TALK_POST.into()],
        limit: store::MAX_LIST_LIMIT,
        ..Filter::default()
    })
    .unwrap()
    .iter()
    .map(|ev| serde_json::from_slice::<serde_json::Value>(&ev.payload).unwrap())
    .find(|v| v["postId"] == post_id)
    .expect("the post's row")["call"]
        .clone()
}

#[test]
fn a_huddle_mark_reaches_the_space_channel_with_its_people() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let space = invited(a1, b1);
    wait_for(WAIT, "alice hears bob read spaces", || {
        a1.core.space_not_ready(&space).unwrap().is_empty()
    });
    a1.core.space_gate(&space).unwrap();
    let ch = a1
        .core
        .talk_channel_create_in(&a1.user(), "general", "", &space)
        .unwrap();

    let start = a1
        .core
        .post_call_mark(
            &ch.id,
            "started a huddle",
            mark("call_1", CALL_START, vec![]),
        )
        .unwrap();
    assert_eq!(start.by, a1.user());
    let people = vec![a1.user(), b1.person_actor()];
    let end = a1
        .core
        .post_call_mark(
            &ch.id,
            "ended a huddle",
            mark("call_1", CALL_END, people.clone()),
        )
        .unwrap();
    assert_eq!(end.call.as_ref().unwrap().people, people);

    // The log names everyone by board key.
    assert_eq!(
        logged(a1, &space, &end.post_id),
        serde_json::json!({
            "callId": "call_1", "op": "end", "durationMs": 90000,
            "people": [a1.person_actor(), b1.person_actor()],
        })
    );
    assert_eq!(
        logged(a1, &space, &start.post_id),
        serde_json::json!({"callId": "call_1", "op": "start"})
    );

    a1.core.kick_board(&space);
    wait_for(WAIT, "bob gets the end", || {
        their_mark(b1, &ch.id, &end.post_id).is_some()
    });
    let theirs = their_mark(b1, &ch.id, &end.post_id).unwrap();
    assert_eq!((theirs.op.as_str(), theirs.duration_ms), (CALL_END, 90_000));
    assert_eq!(
        theirs.people,
        vec![a1.person_actor(), b1.user()],
        "bob is himself to bob"
    );
    assert_eq!(
        their_mark(b1, &ch.id, &start.post_id).unwrap().op,
        CALL_START
    );
    assert_eq!(
        b1.core.talk_names(&theirs.people),
        vec![a1.core.person().name, "you".to_string()]
    );
    // Alice's own copy reads back as hers.
    assert_eq!(their_mark(a1, &ch.id, &end.post_id).unwrap().people, people);
    close(alice.drain(..).chain(bob.drain(..)));
}

#[test]
fn a_huddle_mark_in_a_dm_needs_its_space_and_never_makes_one() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);

    // A thread and a DM with an agent have no huddle.
    let e = a1
        .core
        .post_call_mark("t_1", "x", mark("call_1", CALL_START, vec![]))
        .unwrap_err();
    assert!(e.contains("thread"), "{e}");
    let agent_dm = coord::talk::dm_id(&a1.user(), &a1.agent);
    let e = a1
        .core
        .post_call_mark(&agent_dm, "x", mark("call_1", CALL_START, vec![]))
        .unwrap_err();
    assert!(e.contains("agent"), "{e}");

    let board = invited(a1, b1);
    let dm = a1.core.talk_dm(&a1.user(), &b1.person_actor()).unwrap();
    // Opened, never written: no space yet, and the mark makes none.
    let e = a1
        .core
        .post_call_mark(&dm, "started a huddle", mark("call_1", CALL_START, vec![]))
        .unwrap_err();
    assert!(e.contains("hasn't started"), "{e}");
    assert!(a1.core.dm_spaces_with(&b1.core.person().id).is_empty());
    assert!(
        a1.core
            .talk_posts(&a1.user(), &dm, None, 50)
            .unwrap()
            .is_empty()
    );

    a1.core
        .talk_post(
            &a1.core.user_sender(),
            &dm,
            "hi bob",
            &[],
            false,
            None,
            Vec::new(),
        )
        .unwrap();
    let spaces = a1.core.dm_spaces_with(&b1.core.person().id);
    assert_eq!(spaces.len(), 1, "{spaces:?}");
    let end = a1
        .core
        .post_call_mark(
            &dm,
            "ended a huddle",
            mark("call_1", CALL_END, vec![a1.user(), b1.person_actor()]),
        )
        .unwrap();
    assert_eq!(
        a1.core.dm_spaces_with(&b1.core.person().id),
        spaces,
        "the mark went in the DM's space"
    );
    a1.core.kick_board(&board);
    wait_for(WAIT, "bob gets the DM's end", || {
        their_mark(b1, &dm, &end.post_id).is_some()
    });
    let theirs = their_mark(b1, &dm, &end.post_id).unwrap();
    assert_eq!(theirs.people, vec![a1.person_actor(), b1.user()]);
    close(alice.drain(..).chain(bob.drain(..)));
}
