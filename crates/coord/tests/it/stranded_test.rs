//! Card #485: on a card nobody holds, an item whose agent is gone (the one
//! it waits on, or the one that raised it) is marked, the person can hand
//! the card to a new agent, and their answer to such an item goes out with
//! the card instead of to nobody (#445 waited on kestrel; #453's done
//! check was raised by piranha).

use crate::common;

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

fn card(e: &Env, by: &str) -> Task {
    let t =
        e.c.create_task(
            USER,
            NewTask {
                title: "Spawned agents part 1".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(by, &t.id).unwrap()
}

fn on_agent(k: &str) -> Option<BlockedBy> {
    Some(BlockedBy {
        kind: BLOCKED_ON_AGENT.into(),
        reference: k.into(),
    })
}

fn view_of(e: &Env, id: &str) -> Card {
    let v = e.c.board(&BoardFilter::default());
    v.needs_you
        .into_iter()
        .chain(v.blocked)
        .chain(v.working)
        .chain(v.open)
        .find(|c| c.task.id == id)
        .expect("card on the board")
}

#[test]
fn a_wait_on_a_gone_agent_is_stranded_once_nobody_holds_the_card() {
    let e = env();
    let t = card(&e, ALICE);
    e.c.raise(
        ALICE,
        &t.id,
        NewBlocker {
            question: "Waiting on bob's no-daemon GUI spawn on the PC".into(),
            blocked_by: on_agent(BOB),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    // Bob is still here.
    assert!(e.c.stranded(&t.id).is_empty());
    e.ros.set(BOB, "gone");
    // Alice still holds it: she can act.
    assert!(e.c.stranded(&t.id).is_empty());
    e.c.leave(ALICE, &t.id, ALICE).unwrap();
    let st = e.c.stranded(&t.id);
    assert_eq!(
        st,
        [Stranded {
            blocker_id: "b1".into(),
            agent: BOB.into(),
            waited_on: true,
        }]
    );
    assert_eq!(view_of(&e, &t.id).stranded, st);
    assert!(e.c.key_gone(BOB));
    assert!(!e.c.key_gone(ALICE));
    assert!(!e.c.key_gone(USER));
    // Not on the roster at all counts as gone too.
    assert!(e.c.key_gone("dev-a/never-seen"));
}

#[test]
fn a_question_whose_raiser_is_gone_is_stranded_but_a_card_wait_is_not() {
    let e = env();
    let t = card(&e, ALICE);
    let other = card(&e, CAROL);
    e.c.raise(
        ALICE,
        &t.id,
        NewBlocker {
            question: "Which repo do I open the test PRs on?".into(),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    e.c.raise(
        ALICE,
        &t.id,
        NewBlocker {
            question: "Waits on the desk card".into(),
            blocked_by: Some(BlockedBy {
                kind: BLOCKED_ON_TASK.into(),
                reference: other.id.clone(),
            }),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    e.c.leave(ALICE, &t.id, ALICE).unwrap();
    assert!(e.c.stranded(&t.id).is_empty(), "alice is still live");
    e.ros.set(ALICE, "gone");
    let st = e.c.stranded(&t.id);
    assert_eq!(st.len(), 1, "{st:?}");
    assert_eq!(st[0].agent, ALICE);
    assert!(!st[0].waited_on);
    assert_eq!(
        e.c.get_task(&t.id)
            .unwrap()
            .blockers
            .iter()
            .find(|b| b.id == st[0].blocker_id)
            .unwrap()
            .question,
        "Which repo do I open the test PRs on?"
    );
}

#[test]
fn hand_on_puts_the_card_back_and_takes_the_gone_agent_off_it() {
    let e = env();
    let t = card(&e, ALICE);
    e.c.raise(
        ALICE,
        &t.id,
        NewBlocker {
            question: "Waiting on bob".into(),
            blocked_by: on_agent(BOB),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    e.c.leave(ALICE, &t.id, ALICE).unwrap();
    // Nothing is stranded yet: refused.
    want_err(e.c.hand_on(USER, &t.id), ErrorKind::BadState);
    e.ros.set(BOB, "gone");
    // Only the person hands a card on.
    want_err(e.c.hand_on(CAROL, &t.id), ErrorKind::NotAllowed);
    let (was, st) = e.c.hand_on(USER, &t.id).unwrap();
    assert_eq!(was.state, BLOCKED);
    assert_eq!(st.len(), 1);
    let now = e.c.get_task(&t.id).unwrap();
    assert_eq!(now.state, SUBMITTED);
    assert!(now.blockers.iter().all(|b| !b.open()));
    assert!(now.assigned.is_empty());
    assert!(e.c.stranded(&t.id).is_empty());
    // The closed item is said on the card, for the new agent's brief.
    let cs = e.c.comments(&t.id).unwrap_or_default();
    let said = cs
        .iter()
        .find(|c| {
            c.text
                .starts_with("The person handed this card to a new agent")
        })
        .expect("a hand-on comment");
    assert!(
        said.text.contains("\"Waiting on bob\" (waited on"),
        "{}",
        said.text
    );
    // Handed on, it is no longer stranded: a second press is refused.
    want_err(e.c.hand_on(USER, &t.id), ErrorKind::BadState);
}

#[test]
fn a_not_done_on_a_gone_agents_done_check_goes_out_with_the_card() {
    let e = env();
    let t = card(&e, ALICE);
    let got =
        e.c.set_task_state(ALICE, &t.id, COMPLETED, "on main at 8845c11a")
            .unwrap();
    assert_eq!(got.state, BLOCKED);
    let check = got
        .blockers
        .iter()
        .find(|b| b.done_check)
        .unwrap()
        .id
        .clone();
    // Piranha went, and Irori took it off the card.
    e.c.leave(ALICE, &t.id, ALICE).unwrap();
    e.ros.set(ALICE, "gone");
    let st = e.c.stranded(&t.id);
    assert_eq!(st.len(), 1);
    assert_eq!(st[0].blocker_id, check);

    let (now, ns) =
        e.c.answer(
            USER,
            &t.id,
            &check,
            BlockerAnswer {
                text: "Not done: the others haven't been told the next steps".into(),
                ..BlockerAnswer::default()
            },
        )
        .unwrap();
    // Nobody gone is told; the card is back on the Board for a new agent.
    assert!(
        ns.iter().all(|n| !n.to.iter().any(|k| k == ALICE)),
        "{ns:?}"
    );
    assert_eq!(now.state, SUBMITTED);
    // The person's words are on the check's thread, where the new agent's
    // brief reads them.
    let v = view_of(&e, &t.id);
    assert!(v.stranded.is_empty());
    let cs = e.c.comments(&t.id).unwrap_or_default();
    assert!(
        cs.iter()
            .any(|c| c.blocker_id == check && c.by == USER && c.text.contains("next steps")),
        "{cs:?}"
    );
}

#[test]
fn a_gone_holder_is_stranded_once_the_sweep_takes_it_off() {
    let e = env();
    let t =
        e.c.create_task(
            USER,
            NewTask {
                title: "Merge desk slice 4".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.assign(USER, &t.id, BOB).unwrap();
    e.c.raise(
        BOB,
        &t.id,
        NewBlocker {
            question: "Which repo for the live test?".into(),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    // While bob is here the card is his.
    assert!(e.c.stranded(&t.id).is_empty());
    e.ros.set(BOB, "gone");
    // He still holds it until the sweep takes him off (the holder grace).
    assert!(e.c.stranded(&t.id).is_empty());
    e.c.sweep_notices().unwrap();
    e.clk.add(mins(11));
    e.c.sweep_notices().unwrap();
    let gone = e.c.get_task(&t.id).unwrap();
    assert!(gone.holders.is_empty(), "{:?}", gone.holders);
    assert_eq!(gone.state, BLOCKED);
    assert_eq!(e.c.stranded(&t.id).len(), 1);
    e.c.hand_on(USER, &t.id).unwrap();
    let now = e.c.get_task(&t.id).unwrap();
    assert_eq!(now.state, SUBMITTED);
    assert!(now.assigned.is_empty(), "{:?}", now.assigned);
}
