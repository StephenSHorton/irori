//! Card #447 (spawned agents §2, "Posted"): the person's answer on a card
//! nobody holds puts it back on the Board. A "Not done" on its done check
//! or an answer to a blocker waits on an agent; with no holder, nobody
//! would act on it, so the card goes back to SUBMITTED with its Left off
//! and the answer kept, and the ordinary hand-out (or a spawn) picks it up.

use crate::common;

use std::time::Duration;

use common::*;
use coord::*;

fn env() -> Env {
    let mut e = Env::new();
    e.c = e.reopen_with(Options {
        done_check: true,
        ..e.opts()
    });
    e
}

/// A card ALICE made, BOB holds, with a step and a Left off.
fn held(e: &Env) -> Task {
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "ship it".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(BOB, &t.id).unwrap();
    e.c.set_steps(
        BOB,
        &t.id,
        vec![Step {
            text: "fix the login page".into(),
            ..Step::default()
        }],
    )
    .unwrap();
    let s = e.c.get_task(&t.id).unwrap().steps[0].id.clone();
    e.c.set_step_left_off(
        BOB,
        &t.id,
        &s,
        StepUpdate {
            doing: Some(true),
            ..StepUpdate::default()
        },
        "login fixed on the branch; next: the 500 on save",
    )
    .unwrap()
}

fn check_of(t: &Task) -> String {
    t.blockers
        .iter()
        .find(|b| b.open() && b.done_check)
        .expect("a done check")
        .id
        .clone()
}

fn say(e: &Env, id: &str, bid: &str, text: &str) -> Task {
    e.c.answer(
        USER,
        id,
        bid,
        BlockerAnswer {
            text: text.into(),
            ..BlockerAnswer::default()
        },
    )
    .unwrap()
    .0
}

fn assert_back(e: &Env, id: &str, words: &str) {
    let t = e.c.get_task(id).unwrap();
    assert_eq!(t.state, SUBMITTED, "{t:?}");
    assert!(t.holders.is_empty(), "{t:?}");
    assert!(t.blockers.iter().all(|b| !b.open()), "{:?}", t.blockers);
    assert!(t.note.starts_with("back on the Board"), "{}", t.note);
    // Left off and the answer are still on the card for the next agent.
    assert_eq!(
        t.left_off.as_ref().map(|l| l.text.as_str()),
        Some("login fixed on the branch; next: the 500 on save")
    );
    let cs = e.c.comments(id).unwrap();
    assert!(cs.iter().any(|c| c.by == USER && c.text == words), "{cs:?}");
}

#[test]
fn not_done_on_a_holderless_card_puts_it_back() {
    let e = env();
    let t = held(&e);
    let t = e.c.set_task_state(BOB, &t.id, COMPLETED, "done").unwrap();
    let check = check_of(&t);
    // The agent goes before the person looks.
    e.c.leave(BOB, &t.id, BOB).unwrap();
    assert_eq!(e.c.get_task(&t.id).unwrap().state, BLOCKED);
    say(&e, &t.id, &check, "the save still 500s");
    assert_back(&e, &t.id, "the save still 500s");
}

#[test]
fn a_blocker_answer_on_a_holderless_card_puts_it_back() {
    let e = env();
    let t = held(&e);
    let t = e.c.block(BOB, &t.id, "which port?", None).unwrap();
    let b = t.blockers.iter().find(|b| b.open()).unwrap().id.clone();
    e.c.leave(BOB, &t.id, BOB).unwrap();
    say(&e, &t.id, &b, "8080");
    assert_back(&e, &t.id, "8080");
}

#[test]
fn an_answer_with_the_holder_on_it_stays_with_the_holder() {
    let e = env();
    let t = held(&e);
    let t = e.c.set_task_state(BOB, &t.id, COMPLETED, "done").unwrap();
    let check = check_of(&t);
    let got = say(&e, &t.id, &check, "the save still 500s");
    assert_eq!(got.state, BLOCKED);
    assert!(got.holders.iter().any(|h| h == BOB));
}

#[test]
fn done_on_a_holderless_card_still_closes_it() {
    let e = env();
    let t = held(&e);
    let t = e.c.set_task_state(BOB, &t.id, COMPLETED, "done").unwrap();
    let check = check_of(&t);
    e.c.leave(BOB, &t.id, BOB).unwrap();
    let (got, _) = e.c.say_done(USER, &t.id, &check, &[]).unwrap();
    assert_eq!(got.state, COMPLETED);
}

#[test]
fn the_agent_going_after_the_answer_puts_it_back() {
    let e = env();
    let t = held(&e);
    let t = e.c.set_task_state(BOB, &t.id, COMPLETED, "done").unwrap();
    let check = check_of(&t);
    say(&e, &t.id, &check, "the save still 500s");
    // BOB never acts on it: it drops off the roster.
    e.ros.set(BOB, "");
    e.c.sweep_notices().unwrap();
    e.clk.add(HOLDER_GRACE + Duration::from_secs(1));
    e.c.sweep_notices().unwrap();
    assert_back(&e, &t.id, "the save still 500s");
}

#[test]
fn the_agent_going_before_the_answer_leaves_it_waiting_on_the_person() {
    let e = env();
    let t = held(&e);
    e.c.set_task_state(BOB, &t.id, COMPLETED, "done").unwrap();
    e.ros.set(BOB, "");
    e.c.sweep_notices().unwrap();
    e.clk.add(HOLDER_GRACE + Duration::from_secs(1));
    e.c.sweep_notices().unwrap();
    let got = e.c.get_task(&t.id).unwrap();
    assert!(got.holders.is_empty(), "{got:?}");
    assert_eq!(
        got.state, BLOCKED,
        "the done check still waits on the person"
    );
}
