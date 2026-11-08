//! Merge desk §12.3 (card #1078, B3): a claim is a `pr/` lease on the PR
//! card's board log. Two made-up people share one log; p1 is the PR's
//! author and p2's agent (card c2) is the claimer unless a test says
//! otherwise.

use crate::common;

use common::two_people::*;
use coord::rows::PrReview;
use coord::*;
use store::{Filter, kinds};

const PR: &str = "https://github.com/a/b/pull/7";
const RES: &str = "pr/a/b/7";

fn with_card() -> (Side, Side, String) {
    let (a, b, _) = two_people();
    let t = a
        .c
        .create_task(
            P1,
            NewTask {
                title: "PR #7: ship it".into(),
                objective: "Irori made this card for the pull request.".into(),
                repo_key: common::REPO_A.into(),
                links: vec![PR.into()],
                pr_card: Some(PrCard {
                    pr: PR.into(),
                    author: "alice".into(),
                }),
                ..NewTask::default()
            },
        )
        .unwrap();
    sync(&a, &b);
    (a, b, t.id)
}

fn state(pr_state: &str, reviews: Vec<PrReview>) -> PrCheck {
    PrCheck {
        pr: PR.into(),
        state: pr_state.into(),
        reviews,
        ..PrCheck::default()
    }
}

fn count(s: &Side, kind: &str) -> usize {
    s.st.list(&Filter {
        kinds: vec![kind.into()],
        limit: store::MAX_LIST_LIMIT,
        ..Filter::default()
    })
    .unwrap()
    .len()
}

fn review(login: &str, st: &str, at: gocompat::time::GoTime) -> PrReview {
    PrReview {
        login: login.into(),
        state: st.into(),
        sha: "abc".into(),
        at,
    }
}

#[test]
fn the_pr_resource_is_one_lowercase_name_from_the_url() {
    assert_eq!(pr_claim_resource("https://github.com/A/B/pull/7").as_deref(), Some(RES));
    assert_eq!(pr_claim_resource("https://github.com/a/b/issues/7"), None);
    assert_eq!(pr_claim_resource("a/b#7"), None);
}

#[test]
fn two_partitioned_claims_resolve_to_one_and_the_loser_learns_it() {
    let (a, b, _) = with_card();
    a.c.claim_pr(CARD1, PR).unwrap();
    b.c.claim_pr(CARD2, PR).unwrap();
    sync(&a, &b);
    sync(&a, &b);
    let wa = a.c.claim_of(PR).unwrap();
    let wb = b.c.claim_of(PR).unwrap();
    assert_eq!(wa.holder, wb.holder, "{wa:?} {wb:?}");
    assert_eq!(wa.holder, CARD1, "the lower device id stands");
    // The loser's claim is refused now, and says who holds it.
    let e = b.c.claim_pr(CARD2, PR).unwrap_err();
    assert!(e.is(ErrorKind::Held), "{e}");
    assert!(e.to_string().contains(CARD1), "{e}");
}

#[test]
fn a_synced_claim_is_refused_and_writes_no_refusal_row() {
    let (a, b, _) = with_card();
    a.c.claim_pr(CARD1, PR).unwrap();
    sync(&a, &b);
    let before = count(&b, kinds::LEASE_REFUSED);
    let e = b.c.claim_pr(CARD2, PR).unwrap_err();
    assert!(e.is(ErrorKind::Held), "{e}");
    assert_eq!(count(&b, kinds::LEASE_REFUSED), before);
    // The holder renews its own.
    a.c.claim_pr(CARD1, PR).unwrap();
}

#[test]
fn a_claim_needs_a_pr_card() {
    let (a, _b, _) = with_card();
    let e = a
        .c
        .claim_pr(CARD1, "https://github.com/a/b/pull/8")
        .unwrap_err();
    assert!(e.is(ErrorKind::NotFound), "{e}");
}

#[test]
fn a_merged_prs_claim_stops_counting_on_another_persons_fold() {
    let (a, b, id) = with_card();
    b.c.claim_pr(CARD2, PR).unwrap();
    sync(&a, &b);
    assert!(a.c.claim_of(PR).is_some());
    a.clk.add(common::secs(5));
    a.c.record_pr_state(&id, state(PR_MERGED, vec![])).unwrap();
    sync(&a, &b);
    assert!(a.c.claim_of(PR).is_none());
    assert!(b.c.claim_of(PR).is_none());
}

#[test]
fn a_commented_review_does_not_end_a_claim_and_an_approved_one_does() {
    let (a, b, id) = with_card();
    b.c.claim_pr(CARD2, PR).unwrap();
    sync(&a, &b);
    b.clk.add(common::secs(60));
    a.clk.add(common::secs(60));
    let t = b.clk.now();
    let login = |_: &str| "bob".to_string();
    let alive = |_: &str| true;
    a.c.record_pr_state(&id, state(PR_OPEN, vec![review("bob", "COMMENTED", t)]))
        .unwrap();
    sync(&a, &b);
    b.c.claim_duties(&alive, &login);
    assert!(b.c.claim_of(PR).is_some(), "a COMMENTED review keeps it");
    // Someone else's APPROVED doesn't end it either.
    a.clk.add(common::secs(5));
    a.c.record_pr_state(&id, state(PR_OPEN, vec![review("carol", "APPROVED", t)]))
        .unwrap();
    sync(&a, &b);
    b.c.claim_duties(&alive, &login);
    assert!(b.c.claim_of(PR).is_some());
    a.clk.add(common::secs(5));
    a.c.record_pr_state(&id, state(PR_OPEN, vec![review("bob", "APPROVED", t)]))
        .unwrap();
    sync(&a, &b);
    let steps = b.c.claim_duties(&alive, &login);
    assert_eq!(steps.len(), 1, "{steps:?}");
    assert_eq!(steps[0].what, "verdict");
    assert!(b.c.claim_of(PR).is_none());
    sync(&a, &b);
    assert!(a.c.claim_of(PR).is_none());
}

#[test]
fn a_review_from_before_the_claim_does_not_end_it() {
    let (a, b, id) = with_card();
    let old = b.clk.now();
    b.clk.add(common::secs(600));
    a.clk.add(common::secs(600));
    b.c.claim_pr(CARD2, PR).unwrap();
    a.c.record_pr_state(&id, state(PR_OPEN, vec![review("bob", "CHANGES_REQUESTED", old)]))
        .unwrap();
    sync(&a, &b);
    b.c.claim_duties(&|_| true, &|_| "bob".into());
    assert!(b.c.claim_of(PR).is_some());
}

#[test]
fn a_claim_held_for_an_hour_writes_at_most_four_rows_and_one_past_two_hours_lapses() {
    let (a, b, _) = with_card();
    b.c.claim_pr(CARD2, PR).unwrap();
    let start = count(&b, kinds::LEASE_ACQUIRED);
    assert_eq!(start, 1);
    for _ in 0..60 {
        b.clk.add(common::secs(60));
        b.c.claim_duties(&|_| true, &|_| String::new());
    }
    let n = count(&b, kinds::LEASE_ACQUIRED);
    assert!(n <= 4, "{n} rows in an hour");
    assert!(b.c.claim_of(PR).is_some());
    // Past 2 hours from acquired_at it is not renewed and lapses.
    for _ in 0..70 {
        b.clk.add(common::secs(60));
        b.c.claim_duties(&|_| true, &|_| String::new());
    }
    assert!(b.c.claim_of(PR).is_none(), "lapsed after the cap");
    sync(&a, &b);
}

#[test]
fn a_claim_whose_agent_is_gone_is_not_renewed() {
    let (_a, b, _) = with_card();
    b.c.claim_pr(CARD2, PR).unwrap();
    for _ in 0..50 {
        b.clk.add(common::secs(60));
        b.c.claim_duties(&|_| false, &|_| String::new());
    }
    assert!(b.c.claim_of(PR).is_none());
}

#[test]
fn a_decline_is_a_comment_with_the_pr_and_no_to() {
    let (a, b, id) = with_card();
    let cm = b.c.decline_review(CARD2, PR, "not my area").unwrap();
    assert_eq!(cm.declines, PR);
    assert!(cm.to.is_empty());
    assert_eq!(cm.task_id, id);
    sync(&a, &b);
    let seen = a
        .c
        .comments(&id)
        .unwrap()
        .into_iter()
        .find(|c| c.id == cm.id)
        .unwrap();
    assert_eq!(seen.declines, PR);
    // It is not a lease row.
    assert!(a.c.claim_of(PR).is_none());
}

#[test]
fn releasing_lets_another_agent_claim() {
    let (a, b, _) = with_card();
    b.c.claim_pr(CARD2, PR).unwrap();
    sync(&a, &b);
    assert!(b.c.release_claim(CARD2, PR).unwrap());
    sync(&a, &b);
    a.c.claim_pr(CARD1, PR).unwrap();
    assert!(!b.c.release_claim(CARD2, PR).unwrap());
}
