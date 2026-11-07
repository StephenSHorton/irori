//! A huddle's end, heard by the agents of another person in its channel
//! (card #656), through the real Core on loopback iroh: Bob's device ends
//! a transcribed huddle, and Alice's agent in the channel is woken once,
//! referring to the channel; a huddle in their DM wakes nobody; a huddle
//! with several end posts wakes once. Made-up people only.

use coord::talk::{CALL_END, CallMark};

use crate::common::mesh::{Dev, invited, person};
use crate::common::wait_for;

const WAIT: u64 = 45;
const CALL: &str = "0123456789abcdef0123456789abcdef";
const DM_CALL: &str = "fedcba9876543210fedcba9876543210";

fn end(id: &str) -> CallMark {
    CallMark {
        call_id: id.into(),
        op: CALL_END.into(),
        duration_ms: 60_000,
        transcript: true,
        ..CallMark::default()
    }
}

/// Whether `d` holds the end of huddle `id` in `conv`.
fn has_end(d: &Dev, conv: &str, id: &str) -> bool {
    d.core
        .talk_posts(&d.user(), conv, None, 50)
        .is_ok_and(|ps| {
            ps.iter()
                .any(|p| p.call.as_ref().is_some_and(|c| c.call_id == id))
        })
}

/// What reached `d`'s agent referring to `conv`.
fn told(d: &Dev, conv: &str) -> Vec<inbox::Delivery> {
    d.core
        .inbox()
        .list(0)
        .into_iter()
        .filter(|x| x.to == d.agent && x.message.ref_ == format!("talk:{conv}"))
        .collect()
}

#[test]
fn another_persons_huddle_end_wakes_this_persons_agents_in_the_channel() {
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
    let out = a1.tool("talk_join", serde_json::json!({"conv": ch.id}));
    assert!(!out.starts_with("Error"), "talk_join: {out}");

    // Their DM's huddle first: it reaches Alice and tells nobody.
    let dm = b1.core.talk_dm(&b1.user(), &a1.person_actor()).unwrap();
    b1.core
        .talk_post(
            &b1.core.user_sender(),
            &dm,
            "hi alice",
            &[],
            false,
            None,
            vec![],
        )
        .unwrap();
    b1.core
        .post_call_mark(&dm, "ended a huddle", end(DM_CALL))
        .unwrap();
    b1.core.kick_board(&space);
    wait_for(WAIT, "alice gets the DM's end", || {
        has_end(a1, &dm, DM_CALL)
    });

    // The channel's.
    a1.core.kick_board(&space);
    wait_for(WAIT, "bob holds the channel", || {
        b1.core.talk().channel(&ch.id).is_some()
    });
    b1.core
        .post_call_mark(&ch.id, "ended a huddle", end(CALL))
        .unwrap();
    b1.core.kick_board(&space);
    wait_for(WAIT, "alice's agent is woken", || {
        !told(a1, &ch.id).is_empty()
    });
    let d = &told(a1, &ch.id)[0];
    assert_eq!(d.mode, inbox::mode::WAKE);
    assert_eq!(d.message.board.board_id, space);
    assert_eq!(d.message.board.person_id, b1.core.person().id);
    assert!(
        d.message.text.contains(&format!(
            "The huddle in #general ended. talk_read conv=call:{CALL} has the transcript"
        )),
        "{}",
        d.message.text
    );

    // Once, however often the log is pulled; nothing for the DM, and
    // nothing on Bob's side.
    a1.core.kick_board(&space);
    b1.core.kick_board(&space);
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert_eq!(told(a1, &ch.id).len(), 1, "{:?}", told(a1, &ch.id));
    assert!(told(a1, &dm).is_empty(), "{:?}", told(a1, &dm));
    assert!(told(b1, &ch.id).is_empty());
    close(alice.drain(..).chain(bob.drain(..)));
}

/// How many end posts of huddle `id` `d` holds in `conv`.
fn ends(d: &Dev, conv: &str, id: &str) -> usize {
    d.core
        .talk_posts(&d.user(), conv, None, 50)
        .map(|ps| {
            ps.iter()
                .filter(|p| p.call.as_ref().is_some_and(|c| c.call_id == id))
                .count()
        })
        .unwrap_or(0)
}

/// What reached `d`'s agent about huddle `id`.
fn told_of(d: &Dev, conv: &str, id: &str) -> usize {
    told(d, conv)
        .iter()
        .filter(|x| x.message.text.contains(&format!("conv=call:{id}")))
        .count()
}

#[test]
fn a_huddle_ended_twice_wakes_an_agent_once() {
    const OTHER: &str = "00112233445566778899aabbccddeeff";
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 2);
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
    let out = a1.tool("talk_join", serde_json::json!({"conv": ch.id}));
    assert!(!out.starts_with("Error"), "talk_join: {out}");
    a1.core.kick_board(&space);
    for d in [b1, b2] {
        wait_for(WAIT, "bob's device holds the channel", || {
            d.core.talk().channel(&ch.id).is_some()
        });
    }

    // Both of Bob's devices thought they were the last out.
    for d in [b1, b2] {
        d.core
            .post_call_mark(&ch.id, "ended a huddle", end(CALL))
            .unwrap();
        d.core.kick_board(&space);
    }
    wait_for(WAIT, "alice gets both ends", || ends(a1, &ch.id, CALL) == 2);
    wait_for(WAIT, "alice's agent is woken", || {
        told_of(a1, &ch.id, CALL) > 0
    });

    // Alice's device ended it too (her own agents heard it from
    // tell_call_end), then Bob's end arrives.
    a1.core
        .post_call_mark(&ch.id, "ended a huddle", end(OTHER))
        .unwrap();
    b1.core
        .post_call_mark(&ch.id, "ended a huddle", end(OTHER))
        .unwrap();
    b1.core.kick_board(&space);
    wait_for(WAIT, "alice gets bob's end", || {
        ends(a1, &ch.id, OTHER) == 2
    });

    a1.core.kick_board(&space);
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert_eq!(told_of(a1, &ch.id, CALL), 1, "{:?}", told(a1, &ch.id));
    assert_eq!(told_of(a1, &ch.id, OTHER), 0, "{:?}", told(a1, &ch.id));
    close(alice.drain(..).chain(bob.drain(..)));
}

fn close(devs: impl IntoIterator<Item = Dev>) {
    for d in devs {
        d.core.close().unwrap();
    }
}
