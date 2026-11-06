//! The in_review state (card #191): a card whose work is done and waits on
//! a review shows In review, not Working. The holder's board_review puts it
//! there, a reviewer's changes send it back to working, and an unblock
//! restores it.

use crate::common;

use common::*;
use coord::*;

const PR: &str = "https://github.com/a/b/pull/9";

fn verdict(c: &Coord, by: &str, id: &str, v: &str, text: &str) -> Result<(Comment, Notice)> {
    c.comment_on(
        by,
        id,
        CommentInput {
            text: text.into(),
            verdict: v.into(),
            ..CommentInput::default()
        },
    )
    .map(|(cm, mut ns)| (cm, ns.remove(0)))
}

fn held_card(e: &Env) -> Task {
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "ship it".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap()
}

fn state(e: &Env, id: &str) -> String {
    e.c.get_task(id).unwrap().state
}

#[test]
fn in_review_is_a_valid_open_state() {
    assert!(valid_state(IN_REVIEW));
    assert!(!is_terminal(IN_REVIEW));
}

#[test]
fn the_holders_review_request_puts_the_card_in_review_and_changes_send_it_back() {
    let e = Env::new();
    let t = held_card(&e);
    assert_eq!(state(&e, &t.id), WORKING);

    // A watcher asking leaves the state alone.
    e.c.request_review(CAROL, &t.id, "", BOB).unwrap();
    assert_eq!(state(&e, &t.id), WORKING);

    // The holder asking puts it in review.
    e.clk.add(secs(5));
    e.c.request_review(ALICE, &t.id, "", BOB).unwrap();
    assert_eq!(state(&e, &t.id), IN_REVIEW);

    // An approval leaves it in review: the holder completes it.
    e.clk.add(secs(5));
    verdict(&e.c, BOB, &t.id, VERDICT_APPROVED, "good").unwrap();
    assert_eq!(state(&e, &t.id), IN_REVIEW);

    // Asked again, then changes: back to working, with the reviewer named.
    e.clk.add(secs(5));
    e.c.request_review(ALICE, &t.id, "", BOB).unwrap();
    e.clk.add(secs(5));
    verdict(&e.c, BOB, &t.id, VERDICT_CHANGES, "rename it").unwrap();
    let now = e.c.get_task(&t.id).unwrap();
    assert_eq!(now.state, WORKING);
    assert!(now.note.contains("asked for changes"), "{}", now.note);

    // Asked once more: in review again, and it survives a replay.
    e.clk.add(secs(5));
    e.c.request_review(ALICE, &t.id, "", BOB).unwrap();
    assert_eq!(state(&e, &t.id), IN_REVIEW);
    let c2 = e.reopen();
    assert_eq!(c2.get_task(&t.id).unwrap().state, IN_REVIEW);
}

#[test]
fn board_status_in_review_needs_something_to_review() {
    let e = Env::new();
    let t = held_card(&e);
    want_err(
        e.c.set_task_state(ALICE, &t.id, IN_REVIEW, ""),
        ErrorKind::Invalid,
    );
    e.c.add_link(ALICE, &t.id, PR).unwrap();
    let got =
        e.c.set_task_state(ALICE, &t.id, IN_REVIEW, "with the merge desk")
            .unwrap();
    assert_eq!(got.state, IN_REVIEW);
    // And back by hand.
    let got = e.c.set_task_state(ALICE, &t.id, WORKING, "").unwrap();
    assert_eq!(got.state, WORKING);
}

#[test]
fn an_unblock_restores_in_review() {
    let e = Env::new();
    let t = held_card(&e);
    e.c.add_link(ALICE, &t.id, PR).unwrap();
    e.c.set_task_state(ALICE, &t.id, IN_REVIEW, "").unwrap();
    e.clk.add(secs(5));
    let b = e.c.block(ALICE, &t.id, "the merge lease", None).unwrap();
    assert_eq!(b.state, BLOCKED);
    e.clk.add(secs(5));
    let (u, _) = e.c.unblock(ALICE, &t.id, "got it").unwrap();
    assert_eq!(u.state, IN_REVIEW);
    assert_eq!(e.reopen().get_task(&t.id).unwrap().state, IN_REVIEW);

    // A card that was working when blocked comes back working.
    let w = held_card(&e);
    e.c.block(ALICE, &w.id, "something", None).unwrap();
    let (u, _) = e.c.unblock(ALICE, &w.id, "").unwrap();
    assert_eq!(u.state, WORKING);
}

#[test]
fn completing_from_in_review_works_and_an_in_review_holder_is_free() {
    let e = Env::new();
    let t = held_card(&e);
    e.c.request_review(ALICE, &t.id, "", BOB).unwrap();
    assert_eq!(state(&e, &t.id), IN_REVIEW);

    // In review is not "busy": the holder may take another card.
    let other =
        e.c.create_task(
            CAROL,
            NewTask {
                title: "next".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(ALICE, &other.id).unwrap();

    // Not while the review it waits on isn't approved (Stephen: agents asked
    // him to mark cards done with their PRs still open).
    want_err(
        e.c.set_task_state(ALICE, &t.id, COMPLETED, "merged"),
        ErrorKind::Invalid,
    );
    e.clk.add(secs(5));
    verdict(&e.c, BOB, &t.id, VERDICT_APPROVED, "good").unwrap();
    // Approved: completing it is allowed from in review.
    let done =
        e.c.set_task_state(ALICE, &t.id, COMPLETED, "merged")
            .unwrap();
    assert_ne!(done.state, IN_REVIEW);
}

/// Card #208: a merge Irori has seen on GitHub stands in for an approval
/// that never came; an open PR still holds it.
#[test]
fn completing_from_in_review_after_github_merges_the_pr() {
    let pr = "https://github.com/a/b/pull/3";
    let e = Env::new();
    let t = held_card(&e);
    e.c.add_link(ALICE, &t.id, pr).unwrap();
    // Card #471: none of the person's own agents reviews their PR; the
    // card waits on the PR itself.
    want_err(
        e.c.request_review(ALICE, &t.id, "", BOB),
        ErrorKind::NotAllowed,
    );
    e.c.set_task_state(ALICE, &t.id, IN_REVIEW, "").unwrap();
    assert_eq!(state(&e, &t.id), IN_REVIEW);
    let heard = |s: &str| PrCheck {
        pr: pr.into(),
        state: s.into(),
        ..PrCheck::default()
    };

    // Not approved and the PR open: refused.
    e.c.record_pr_state(&t.id, heard(PR_OPEN)).unwrap();
    want_err(
        e.c.set_task_state(ALICE, &t.id, COMPLETED, "done"),
        ErrorKind::Invalid,
    );
    // Not approved but the PR merged: allowed.
    e.clk.add(secs(5));
    e.c.record_pr_state(&t.id, heard(PR_MERGED)).unwrap();
    let done =
        e.c.set_task_state(ALICE, &t.id, COMPLETED, "merged")
            .unwrap();
    assert_ne!(done.state, IN_REVIEW);
}

#[test]
fn a_parents_rollup_is_in_review_when_every_open_subtask_is() {
    let e = Env::new();
    let p = held_card(&e);
    let mk = |title: &str| {
        e.c.create_task(
            ALICE,
            NewTask {
                title: title.into(),
                parent: p.id.clone(),
                ..NewTask::default()
            },
        )
        .unwrap()
    };
    let a = mk("one");
    let b = mk("two");
    e.c.claim_task(BOB, &a.id).unwrap();
    e.c.claim_task(CAROL, &b.id).unwrap();
    e.c.add_link(BOB, &a.id, PR).unwrap();
    e.c.set_task_state(BOB, &a.id, IN_REVIEW, "").unwrap();
    let shown = |e: &Env| e.c.get_task(&p.id).unwrap().rollup.unwrap().state;
    assert_ne!(shown(&e), IN_REVIEW, "one subtask still works");
    e.c.add_link(CAROL, &b.id, "https://github.com/a/b/pull/10")
        .unwrap();
    e.c.set_task_state(CAROL, &b.id, IN_REVIEW, "").unwrap();
    assert_eq!(shown(&e), IN_REVIEW);
    // A parent is never put in review by hand.
    e.c.add_link(ALICE, &p.id, PR).unwrap();
    want_err(
        e.c.set_task_state(ALICE, &p.id, IN_REVIEW, ""),
        ErrorKind::Invalid,
    );
}

/// Card #347: a wait on another card ends once that card's work is done:
/// it's in review, or an agent completed it and it waits on the person's
/// Done. What's left is the person's, so the wait no longer hides it.
#[test]
fn a_wait_on_a_card_ends_when_it_goes_in_review_or_waits_on_done() {
    let mut e = Env::new();
    e.c = e.reopen_with(Options {
        done_check: true,
        ..e.opts()
    });
    let a = held_card(&e);
    let b =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "the other".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(BOB, &b.id).unwrap();
    let wait = |on: &Task| {
        e.c.create_task(
            CAROL,
            NewTask {
                title: format!("after {}", on.title),
                block_reason: "after it".into(),
                blocked_by: Some(BlockedBy {
                    kind: BLOCKED_ON_TASK.into(),
                    reference: on.id.clone(),
                }),
                ..NewTask::default()
            },
        )
        .unwrap()
    };
    let (wa, wb) = (wait(&a), wait(&b));
    assert!(e.c.sweep_notices().unwrap().is_empty());

    e.clk.add(secs(5));
    e.c.request_review(ALICE, &a.id, "", BOB).unwrap();
    assert_eq!(state(&e, &a.id), IN_REVIEW);
    let got = e.c.set_task_state(BOB, &b.id, COMPLETED, "done").unwrap();
    assert!(got.blockers.iter().any(|x| x.open() && x.done_check));

    let ns = e.c.sweep_notices().unwrap();
    let text = |id: &str| ns.iter().find(|n| n.task_id == id).unwrap().text.clone();
    assert!(text(&wa.id).contains("task in review"), "{ns:?}");
    assert!(text(&wb.id).contains("waiting on Done"), "{ns:?}");
    for w in [&wa, &wb] {
        assert_ne!(state(&e, &w.id), BLOCKED);
    }

    // A new wait on either is refused: it would close at the next sweep.
    for on in [&a, &b] {
        let r = e.c.create_task(
            CAROL,
            NewTask {
                title: "late".into(),
                block_reason: "after it".into(),
                blocked_by: Some(BlockedBy {
                    kind: BLOCKED_ON_TASK.into(),
                    reference: on.id.clone(),
                }),
                ..NewTask::default()
            },
        );
        assert!(r.as_ref().is_err_and(|e| e.is(ErrorKind::Invalid)), "{r:?}");
    }
}
