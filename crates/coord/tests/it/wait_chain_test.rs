//! Card #481: #392 waited on #396, #396 on #394 and #394 on #392, and
//! nothing said so. A loop of waits on cards, and a chain that ends at a
//! card nobody holds, are found from the waits as they stand.

use crate::common;

use common::*;
use coord::*;

fn held(e: &Env, who: &str, title: &str) -> Task {
    let t =
        e.c.create_task(
            who,
            NewTask {
                title: title.into(),
                repo_key: REPO_A.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(who, &t.id).unwrap();
    t
}

fn wait_on(e: &Env, who: &str, id: &str, on: &Task) {
    e.c.block(
        who,
        id,
        &format!("Waits on {}", on.title),
        Some(BlockedBy {
            kind: BLOCKED_ON_TASK.into(),
            reference: on.id.clone(),
        }),
    )
    .unwrap();
}

fn chain(e: &Env, id: &str) -> WaitChain {
    wait_chain(&|k: &str| e.c.get_task(k), id)
}

#[test]
fn three_cards_that_wait_on_each_other_are_one_loop() {
    let e = Env::new();
    let talk4 = held(&e, ALICE, "Talk 4");
    let talk6 = held(&e, BOB, "Talk 6");
    let talk8 = held(&e, CAROL, "Talk 8");
    wait_on(&e, ALICE, &talk4.id, &talk8);
    wait_on(&e, CAROL, &talk8.id, &talk6);
    assert_eq!(chain(&e, &talk6.id), WaitChain::default(), "no loop yet");
    wait_on(&e, BOB, &talk6.id, &talk4);

    // Every card of the loop names the same cards, in wait order from
    // the lowest id.
    let ring = chain(&e, &talk4.id).loop_ids;
    assert_eq!(ring.len(), 3, "{ring:?}");
    let mut ids = [talk4.id.clone(), talk6.id.clone(), talk8.id.clone()];
    ids.sort();
    assert_eq!(ring[0], ids[0]);
    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        assert_eq!(task_waits(&e.c.get_task(a).unwrap()), vec![b.clone()]);
    }
    for t in [&talk4, &talk6, &talk8] {
        let c = chain(&e, &t.id);
        assert_eq!(c.loop_ids, ring, "from {}", t.title);
        assert_eq!(c.stuck, "");
    }

    // A card that waits on the loop isn't in it.
    let after = held(&e, USER, "After Talk");
    wait_on(&e, USER, &after.id, &talk8);
    assert_eq!(chain(&e, &after.id), WaitChain::default());

    // One wait closing breaks it.
    e.c.unblock(CAROL, &talk8.id, "not waiting after all")
        .unwrap();
    for t in [&talk4, &talk6, &talk8] {
        assert!(chain(&e, &t.id).loop_ids.is_empty(), "from {}", t.title);
    }
}

#[test]
fn a_loop_breaks_when_one_of_its_cards_is_done() {
    let e = Env::new();
    let a = held(&e, ALICE, "a");
    let b = held(&e, BOB, "b");
    wait_on(&e, ALICE, &a.id, &b);
    wait_on(&e, BOB, &b.id, &a);
    assert_eq!(chain(&e, &a.id).loop_ids.len(), 2);
    e.c.set_task_state(BOB, &b.id, COMPLETED, "done").unwrap();
    assert!(chain(&e, &a.id).loop_ids.is_empty());
}

#[test]
fn a_question_with_options_on_a_card_is_no_wait() {
    // Card #347: options make it the person's decision, so the loop
    // ends at them, not in a ring.
    let e = Env::new();
    let a = held(&e, ALICE, "a");
    let b = held(&e, BOB, "b");
    wait_on(&e, ALICE, &a.id, &b);
    e.c.raise(
        BOB,
        &b.id,
        NewBlocker {
            question: "Wait for a, or go without it?".into(),
            options: vec![
                BlockerOption {
                    answer: "Wait".into(),
                    ..BlockerOption::default()
                },
                BlockerOption {
                    answer: "Go".into(),
                    ..BlockerOption::default()
                },
            ],
            blocked_by: Some(BlockedBy {
                kind: BLOCKED_ON_TASK.into(),
                reference: a.id.clone(),
            }),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    assert_eq!(chain(&e, &a.id), WaitChain::default());
}

#[test]
fn a_chain_that_ends_at_a_card_nobody_holds_says_which() {
    let e = Env::new();
    let a = held(&e, ALICE, "a");
    let b = held(&e, BOB, "b");
    wait_on(&e, ALICE, &a.id, &b);
    // b is held: nothing stuck.
    assert_eq!(chain(&e, &a.id), WaitChain::default());

    // b's agent asks the person, they answer, and it leaves before it
    // acts on the answer: nobody is on b and nothing hands it out.
    let (_, q) =
        e.c.raise(
            BOB,
            &b.id,
            NewBlocker {
                question: "Which adapter?".into(),
                ..NewBlocker::default()
            },
        )
        .unwrap();
    e.c.answer(
        USER,
        &b.id,
        &q.id,
        BlockerAnswer {
            text: "Wintun".into(),
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    assert_eq!(chain(&e, &a.id).stuck, "", "BOB still holds it");
    e.c.leave(BOB, &b.id, BOB).unwrap();
    let k = e.c.get_task(&b.id).unwrap();
    assert!(k.holders.is_empty() && k.state == BLOCKED, "{k:?}");
    assert_eq!(
        chain(&e, &a.id),
        WaitChain {
            loop_ids: vec![],
            stuck: b.id.clone(),
        }
    );

    // An open card a free agent can take isn't stuck.
    let c =
        e.c.create_task(
            USER,
            NewTask {
                title: "c".into(),
                repo_key: REPO_A.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    let d = held(&e, CAROL, "d");
    wait_on(&e, CAROL, &d.id, &c);
    assert_eq!(chain(&e, &d.id).stuck, "");
}
