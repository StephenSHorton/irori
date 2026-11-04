//! An agent keeps to its device's sides (card #496): on its own it never
//! claims, joins or assigns a card on a side its device doesn't run, and
//! the refusal says to ask the person. The person's own moves go through,
//! and so does an agent the person asked on the card.

mod common;

use common::*;
use coord::*;

/// An agent on another device, one that runs Work.
const WORKER: &str = "dev-b/worker-session";

fn sides(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// This device (dev-a) runs only Personal; dev-b runs Work. The card is a
/// Work card typed elsewhere.
fn env_with_work_card(shared: bool) -> (Env, Task) {
    let e = Env::new();
    e.c.set_device_sides(&sides(&["Personal"])).unwrap();
    e.remote(
        "dev-b",
        1,
        hlc_at(e.clk.now()),
        "user",
        coord::sides::KIND,
        &coord::sides::SidesRow {
            device_id: "dev-b".into(),
            sides: sides(&["Work"]),
            at_device: "dev-b".into(),
        },
    );
    let t =
        e.c.create_task(
            USER,
            NewTask {
                title: "Ship the quarterly report".into(),
                side: "Work".into(),
                shared,
                ..NewTask::default()
            },
        )
        .unwrap();
    assert_eq!(t.side, "Work");
    (e, t)
}

fn refused<T: std::fmt::Debug>(r: coord::Result<T>) -> String {
    let err = r.expect_err("refused");
    assert_eq!(err.kind(), ErrorKind::NotAllowed, "{err}");
    let msg = err.to_string();
    assert!(msg.contains(ASK_THE_PERSON), "{msg}");
    assert!(msg.contains("Work side"), "{msg}");
    msg
}

fn m(key: &str, name: &str) -> Mention {
    Mention {
        key: key.into(),
        name: name.into(),
        gone: false,
    }
}

#[test]
fn an_agents_own_cross_side_claim_is_refused_with_ask_the_person() {
    let (e, t) = env_with_work_card(false);
    let msg = refused(e.c.claim_task(ALICE, &t.id));
    assert!(msg.contains("runs only Personal"), "{msg}");
    let t = e.c.get_task(&t.id).unwrap();
    assert!(t.holders.is_empty(), "{t:?}");
}

#[test]
fn an_agents_own_cross_side_join_is_refused() {
    let (e, t) = env_with_work_card(true);
    refused(e.c.join(ALICE, &t.id, ALICE));
    assert!(e.c.get_task(&t.id).unwrap().holders.is_empty());
}

#[test]
fn an_agents_own_cross_side_assign_is_refused() {
    let (e, t) = env_with_work_card(false);
    // To itself, and to another agent on its own Personal-only device.
    refused(e.c.assign_task(ALICE, &t.id, ALICE));
    let msg = refused(e.c.assign_task(ALICE, &t.id, BOB));
    assert!(msg.contains(BOB), "{msg}");
    refused(e.c.hand_off(ALICE, &t.id, BOB, "yours"));
    assert!(e.c.get_task(&t.id).unwrap().assigned.is_empty());
}

#[test]
fn an_agent_may_hand_a_card_to_a_device_that_runs_its_side() {
    let (e, t) = env_with_work_card(false);
    let t = e.c.assign_task(ALICE, &t.id, WORKER).unwrap();
    assert_eq!(t.assigned, [WORKER]);
    // And that agent takes it: its device runs Work.
    let t = e.c.claim_task(WORKER, &t.id).unwrap();
    assert!(t.holders.iter().any(|h| h == WORKER));
}

#[test]
fn the_persons_assign_goes_through_and_the_agent_then_claims() {
    let (e, t) = env_with_work_card(false);
    let t = e.c.assign_task(USER, &t.id, ALICE).unwrap();
    assert_eq!(t.assigned, [ALICE]);
    let t = e.c.claim_task(ALICE, &t.id).unwrap();
    assert_eq!(t.holders, [ALICE]);
}

#[test]
fn the_persons_own_claim_and_adding_an_agent_go_through() {
    let (e, t) = env_with_work_card(true);
    // The person adds an agent to a shared card across the line.
    let t = e.c.join(USER, &t.id, ALICE).unwrap();
    assert!(t.holders.iter().any(|h| h == ALICE));
    let (e, t) = env_with_work_card(false);
    e.c.claim_task(USER, &t.id).unwrap();
}

#[test]
fn a_person_on_another_device_is_still_the_person() {
    // is_owner covers user:<dev> on any of the person's devices.
    let e = Env::with(
        "dev-a",
        Some(std::sync::Arc::new(|k: &str| k.starts_with("user:"))),
    );
    e.c.set_device_sides(&sides(&["Personal"])).unwrap();
    let t =
        e.c.create_task(
            USER,
            NewTask {
                title: "Work thing".into(),
                side: "Work".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.assign_task("user:dev-b", &t.id, ALICE).unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
}

#[test]
fn the_persons_mention_lets_the_agent_take_it() {
    let (e, t) = env_with_work_card(false);
    refused(e.c.claim_task(ALICE, &t.id));
    e.c.person_comment(
        USER,
        &t.id,
        "@alice can you take this?",
        false,
        &[m(ALICE, "alice")],
        None,
    )
    .unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
    // Only the one mentioned: Bob still asks.
    let (e, t) = env_with_work_card(true);
    e.c.person_comment(USER, &t.id, "@alice", false, &[m(ALICE, "alice")], None)
        .unwrap();
    refused(e.c.join(BOB, &t.id, BOB));
    e.c.join(ALICE, &t.id, ALICE).unwrap();
}

#[test]
fn an_agents_mention_is_not_the_person_asking() {
    let (e, t) = env_with_work_card(false);
    e.c.comment_on(
        BOB,
        &t.id,
        CommentInput {
            text: "@alice take it".into(),
            mentions: vec![m(ALICE, "alice")],
            ..CommentInput::default()
        },
    )
    .unwrap();
    refused(e.c.claim_task(ALICE, &t.id));
}

#[test]
fn the_persons_message_naming_the_card_lets_the_agent_take_it() {
    let (e, t) = env_with_work_card(false);
    e.c.record_message(MessageEntry {
        task_id: t.id.clone(),
        msg_id: "msg_1".into(),
        from: USER.into(),
        to: ALICE.into(),
        text: "please pick this up".into(),
        ..MessageEntry::default()
    })
    .unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
    // Another agent's message is not the person asking.
    let (e, t) = env_with_work_card(false);
    e.c.record_message(MessageEntry {
        task_id: t.id.clone(),
        msg_id: "msg_2".into(),
        from: BOB.into(),
        to: ALICE.into(),
        text: "you take it".into(),
        ..MessageEntry::default()
    })
    .unwrap();
    refused(e.c.claim_task(ALICE, &t.id));
}

#[test]
fn an_agent_the_person_asked_may_take_it_by_its_own_assign() {
    let (e, t) = env_with_work_card(false);
    e.c.person_comment(USER, &t.id, "@alice", false, &[m(ALICE, "alice")], None)
        .unwrap();
    e.c.assign_task(ALICE, &t.id, ALICE).unwrap();
    // But the person asked Alice, not Bob: handing it on to Bob is refused.
    refused(e.c.assign_task(ALICE, &t.id, BOB));
}

#[test]
fn a_spawned_agent_takes_the_card_it_was_started_for() {
    let (e, t) = env_with_work_card(false);
    e.c.request_spawn(
        "irori",
        SpawnAsk {
            task_id: t.id.clone(),
            device: "dev-a".into(),
            folder: "/repo".into(),
            permission_mode: "bypassPermissions".into(),
            name: "for-1".into(),
            ..SpawnAsk::default()
        },
    )
    .unwrap();
    const SPAWNED: &str = "dev-a/spawned-session";
    assert!(e.c.bind_spawn(&t.id, 1, SPAWNED).unwrap());
    let t = e.c.claim_task(SPAWNED, &t.id).unwrap();
    assert_eq!(t.holders, [SPAWNED]);
}

#[test]
fn irori_itself_is_not_held_to_a_side() {
    let (e, t) = env_with_work_card(false);
    e.c.assign_task("irori", &t.id, ALICE).unwrap();
}

#[test]
fn cards_on_a_side_the_device_runs_and_unsided_cards_are_as_before() {
    let e = Env::new();
    e.c.set_device_sides(&sides(&["Personal"])).unwrap();
    let own = e.c.create_task(USER, NewTask { title: "Mine".into(), ..NewTask::default() }).unwrap();
    assert_eq!(own.side, "Personal");
    e.c.claim_task(ALICE, &own.id).unwrap();
    // With no pin at all, every card is free to take.
    let e = Env::new();
    let t =
        e.c.create_task(
            USER,
            NewTask {
                title: "Work".into(),
                side: "Work".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
}

#[test]
fn a_card_already_held_stays_held_after_a_pin() {
    let e = Env::new();
    let t =
        e.c.create_task(
            USER,
            NewTask {
                title: "Work".into(),
                side: "Work".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
    e.c.set_device_sides(&sides(&["Personal"])).unwrap();
    // Claiming again is a no-op, not a refusal.
    e.c.claim_task(ALICE, &t.id).unwrap();
}
