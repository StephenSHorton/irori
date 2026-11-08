//! Card #520: on a card nobody holds, a wait on an agent that is gone, or
//! an outside wait its gone agent raised, needs the person (Hand to an
//! agent, or stop the card): it counts in their Needs you until it is
//! handed on, the card is stopped, or the card gets a holder. A question
//! already counts as a decision, so gone_waits leaves those out and
//! nothing shows twice; a wait on a PR is a gone wait like any outside
//! wait (card #970).

use crate::common;

use common::*;
use coord::*;

const PR: &str = "https://github.com/a/b/pull/41";

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

fn wait(e: &Env, id: &str, question: &str, kind: &str, on: &str) -> Blocker {
    e.c.raise(
        ALICE,
        id,
        NewBlocker {
            question: question.into(),
            blocked_by: (!kind.is_empty()).then(|| BlockedBy {
                kind: kind.into(),
                reference: on.into(),
            }),
            ..NewBlocker::default()
        },
    )
    .unwrap()
    .1
}

/// (needs_you, blocked) for `viewer`.
fn lane(e: &Env, viewer: &str, id: &str) -> (bool, bool) {
    let v = e.c.board(&BoardFilter {
        viewer: viewer.into(),
        ..BoardFilter::default()
    });
    let has = |g: &[Card]| g.iter().any(|c| c.task.id == id);
    (has(&v.needs_you), has(&v.blocked))
}

#[test]
fn a_wait_on_a_gone_agent_needs_the_person_once_nobody_holds_the_card() {
    let e = Env::new();
    let t = card(&e, ALICE);
    let b = wait(
        &e,
        &t.id,
        "Waiting on bob's GUI spawn",
        BLOCKED_ON_AGENT,
        BOB,
    );
    e.c.leave(ALICE, &t.id, ALICE).unwrap();
    // Bob is still here: it waits on him, not on the person.
    assert!(e.c.gone_waits(&t.id).is_empty());
    assert_eq!(lane(&e, USER, &t.id), (false, true));
    e.ros.set(BOB, "gone");
    assert_eq!(
        e.c.gone_waits(&t.id),
        [Stranded {
            blocker_id: b.id.clone(),
            agent: BOB.into(),
            waited_on: true,
        }]
    );
    assert_eq!(lane(&e, USER, &t.id), (true, false), "the person");
    assert_eq!(lane(&e, "", &t.id), (true, false), "no viewer");
    // Not an agent's to act on.
    assert!(!lane(&e, CAROL, &t.id).0);
}

#[test]
fn a_held_card_is_its_holders_not_the_persons() {
    let e = Env::new();
    let t = card(&e, ALICE);
    wait(&e, &t.id, "Waiting on bob", BLOCKED_ON_AGENT, BOB);
    e.ros.set(BOB, "gone");
    // Alice holds it: she can drop the wait herself.
    assert!(e.c.gone_waits(&t.id).is_empty());
    assert!(!lane(&e, USER, &t.id).0);
    // Nobody holds it now: the person's. Once Carol takes it, hers again.
    e.c.leave(ALICE, &t.id, ALICE).unwrap();
    assert!(lane(&e, USER, &t.id).0);
    e.c.claim_task(CAROL, &t.id).unwrap();
    assert!(e.c.gone_waits(&t.id).is_empty());
    assert!(!lane(&e, USER, &t.id).0, "it got a holder");
}

#[test]
fn an_outside_wait_whose_gone_agent_raised_it_needs_the_person() {
    let e = Env::new();
    let t = card(&e, ALICE);
    let b = wait(
        &e,
        &t.id,
        "Waiting on CI for the release",
        BLOCKED_ON_EXTERNAL,
        "CI on the release",
    );
    e.c.leave(ALICE, &t.id, ALICE).unwrap();
    assert!(!lane(&e, USER, &t.id).0, "alice is still live");
    e.ros.set(ALICE, "gone");
    assert_eq!(
        e.c.gone_waits(&t.id),
        [Stranded {
            blocker_id: b.id,
            agent: ALICE.into(),
            waited_on: false,
        }]
    );
    assert_eq!(lane(&e, USER, &t.id), (true, false));
}

#[test]
fn a_question_and_a_card_wait_are_left_to_their_own_rows() {
    let e = Env::new();
    let t = card(&e, ALICE);
    let other = card(&e, CAROL);
    wait(&e, &t.id, "Which repo?", "", "");
    wait(
        &e,
        &t.id,
        "Waits on the desk card",
        BLOCKED_ON_TASK,
        &other.id,
    );
    e.c.leave(ALICE, &t.id, ALICE).unwrap();
    e.ros.set(ALICE, "gone");
    // The question is stranded (and a decision already), the card wait
    // isn't: neither is a gone wait.
    assert_eq!(e.c.stranded(&t.id).len(), 1);
    assert!(e.c.gone_waits(&t.id).is_empty());
}

/// Card #970: a gone agent's wait on a PR is a gone wait like any outside
/// wait, open PR or merged (no merge row asks the person to merge it).
#[test]
fn a_gone_agents_wait_on_a_pr_is_a_gone_wait() {
    let e = Env::new();
    let t = card(&e, ALICE);
    let b = wait(&e, &t.id, "Merge PR #41", BLOCKED_ON_EXTERNAL, PR);
    e.c.leave(ALICE, &t.id, ALICE).unwrap();
    e.ros.set(ALICE, "gone");
    assert_eq!(e.c.stranded(&t.id).len(), 1);
    let gone = || {
        e.c.gone_waits(&t.id)
            .iter()
            .map(|s| s.blocker_id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(gone(), std::slice::from_ref(&b.id));
    assert!(lane(&e, USER, &t.id).0);
    // Still polled, so a merge is heard.
    assert_eq!(e.c.polled_waits(&t.id), [PR]);
    e.c.record_pr_state(
        &t.id,
        PrCheck {
            pr: PR.into(),
            state: PR_MERGED.into(),
            ..PrCheck::default()
        },
    )
    .unwrap();
    assert!(e.c.polled_waits(&t.id).is_empty());
    assert_eq!(gone(), [b.id]);
    assert!(lane(&e, USER, &t.id).0);
}

#[test]
fn handing_it_on_or_stopping_the_card_takes_it_out() {
    let e = Env::new();
    let t = card(&e, ALICE);
    wait(&e, &t.id, "Waiting on bob", BLOCKED_ON_AGENT, BOB);
    let u = card(&e, ALICE);
    wait(&e, &u.id, "Waiting on bob too", BLOCKED_ON_AGENT, BOB);
    e.c.leave(ALICE, &t.id, ALICE).unwrap();
    e.c.leave(ALICE, &u.id, ALICE).unwrap();
    e.ros.set(BOB, "gone");
    assert!(lane(&e, USER, &t.id).0);
    assert!(lane(&e, USER, &u.id).0);

    e.c.hand_on(USER, &t.id).unwrap();
    assert!(e.c.gone_waits(&t.id).is_empty());
    assert!(!lane(&e, USER, &t.id).0, "handed on");

    e.c.set_task_state(USER, &u.id, CANCELED, "").unwrap();
    assert!(e.c.gone_waits(&u.id).is_empty());
    assert!(!lane(&e, USER, &u.id).0, "stopped");
}

#[test]
fn a_card_hand_on_would_refuse_says_why_and_still_counts() {
    let e = Env::new();
    let t = card(&e, ALICE);
    wait(&e, &t.id, "Waiting on bob", BLOCKED_ON_AGENT, BOB);
    e.c.leave(ALICE, &t.id, ALICE).unwrap();
    e.ros.set(BOB, "gone");
    assert_eq!(e.c.hand_on_refusal(&t.id), "");
    // A subtask under it: Hand to an agent would refuse, so Home says why.
    e.c.create_task(
        USER,
        NewTask {
            title: "a part".into(),
            parent: t.id.clone(),
            ..NewTask::default()
        },
    )
    .unwrap();
    assert_eq!(
        e.c.hand_on_refusal(&t.id),
        "it has subtasks; hand those on instead"
    );
    assert!(e.c.hand_on(USER, &t.id).is_err());
    // It still needs the person (Open, then Stop waiting or stop the card).
    assert!(lane(&e, USER, &t.id).0);
    // The board view carries the gone waits for the notifier.
    let v = e.c.board(&BoardFilter::default());
    let c = v.needs_you.iter().find(|c| c.task.id == t.id).unwrap();
    assert_eq!(c.gone_waits.len(), 1);
}
