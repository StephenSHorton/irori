//! Card #471: a card in review on an open PR that nobody holds any more
//! isn't left silent (#467 and #468 sat that way): the sweep puts it back
//! on the Board, so an agent asks to merge it and merges it on yes. One that
//! waits on another member (GitHub wants an approval) stays where it is.

mod common;

use std::time::Duration;

use common::*;
use coord::*;

const PR: &str = "https://github.com/a/b/pull/7";

/// A card ALICE made, BOB holds and put in review on PR #7.
fn in_review(e: &Env) -> Task {
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
    e.c.add_link(BOB, &t.id, PR).unwrap();
    e.c.set_task_state(BOB, &t.id, IN_REVIEW, "").unwrap()
}

fn heard(e: &Env, id: &str, state: &str, review: &str) {
    e.c.record_pr_state(
        id,
        PrCheck {
            pr: PR.into(),
            state: state.into(),
            review: review.into(),
            ..PrCheck::default()
        },
    )
    .unwrap();
}

/// BOB drops off the roster and the holder grace runs out.
fn bob_goes(e: &Env) {
    e.ros.set(BOB, "");
    e.c.sweep_notices().unwrap();
    e.clk.add(HOLDER_GRACE + Duration::from_secs(1));
    e.c.sweep_notices().unwrap();
}

fn state(e: &Env, id: &str) -> Task {
    e.c.get_task(id).unwrap()
}

#[test]
fn an_open_pr_whose_agent_went_goes_back_on_the_board() {
    let e = Env::new();
    let t = in_review(&e);
    heard(&e, &t.id, PR_OPEN, "");
    bob_goes(&e);
    let got = state(&e, &t.id);
    assert_eq!(got.state, SUBMITTED, "{got:?}");
    assert!(got.holders.is_empty());
    assert_eq!(
        got.note,
        "back on the Board: its PR #7 is still open and nobody is on it: ask the person “Can I merge…?” and merge it on their yes, or complete the card if it merged"
    );
}

#[test]
fn a_card_already_left_in_review_is_picked_up_at_the_next_sweep() {
    let e = Env::new();
    let t = in_review(&e);
    // Never heard: the poller runs on the holder's device, and it left.
    e.c.leave(BOB, &t.id, BOB).unwrap();
    assert_eq!(state(&e, &t.id).state, IN_REVIEW);
    e.c.sweep_notices().unwrap();
    assert_eq!(state(&e, &t.id).state, SUBMITTED);
}

#[test]
fn a_held_card_or_a_merged_pr_stays_in_review() {
    let e = Env::new();
    let t = in_review(&e);
    heard(&e, &t.id, PR_OPEN, "");
    e.c.sweep_notices().unwrap();
    assert_eq!(state(&e, &t.id).state, IN_REVIEW, "its holder is on it");

    let u = in_review(&e);
    heard(&e, &u.id, PR_MERGED, "");
    e.c.leave(BOB, &u.id, BOB).unwrap();
    e.c.sweep_notices().unwrap();
    assert_eq!(state(&e, &u.id).state, IN_REVIEW, "nothing left to merge");
}

#[test]
fn a_pr_that_needs_an_approval_stays_with_the_other_member() {
    let e = Env::new();
    let t = in_review(&e);
    heard(&e, &t.id, PR_OPEN, PR_REVIEW_REQUIRED);
    bob_goes(&e);
    let got = state(&e, &t.id);
    assert_eq!(got.state, IN_REVIEW, "{got:?}");
    assert!(got.holders.is_empty());
}
