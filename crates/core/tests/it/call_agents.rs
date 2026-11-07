//! A huddle's end, heard by the agents of another person in its channel
//! (card #656), through the real Core on loopback iroh: Bob's device ends
//! a transcribed huddle, and Alice's agent in the channel is woken once,
//! referring to the channel; a huddle in their DM wakes nobody. Made-up
//! people only.

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

fn close(devs: impl IntoIterator<Item = Dev>) {
    for d in devs {
        d.core.close().unwrap();
    }
}
