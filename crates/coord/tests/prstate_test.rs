//! A card's linked PRs as GitHub says they stand (card #208): the poller's
//! checks become pr.state rows, an open PR holds an agent's completed, a
//! merge wakes the holder, changes requested sends a card in review back
//! to working, and a GitHub that can't be asked is said once.

mod common;

use common::*;
use coord::*;

const PR: &str = "https://github.com/a/b/pull/7";

fn held_with_pr(e: &Env) -> Task {
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "ship it".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
    e.c.add_link(ALICE, &t.id, PR).unwrap()
}

fn check(state: &str, review: &str) -> PrCheck {
    PrCheck {
        pr: PR.into(),
        state: state.into(),
        review: review.into(),
        ..PrCheck::default()
    }
}

#[test]
fn an_open_pr_holds_completed_and_a_merge_wakes_the_holder() {
    let e = Env::new();
    let t = held_with_pr(&e);

    // Nothing heard yet: today's rule (completed goes through).
    assert!(e.c.pr_states(&t.id).is_empty());

    // Open on GitHub: an agent's completed is refused, naming the PR.
    let (_, ns) = e.c.record_pr_state(&t.id, check(PR_OPEN, "")).unwrap();
    assert!(ns.is_empty(), "an open PR tells nobody");
    let st = e.c.pr_states(&t.id);
    assert_eq!((st.len(), st[0].state.as_str()), (1, PR_OPEN));
    let err =
        e.c.set_task_state(ALICE, &t.id, COMPLETED, "done")
            .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Invalid);
    assert!(err.to_string().contains(PR), "{err}");

    // Heard again unchanged: no new row, no notice.
    e.clk.add(secs(60));
    let at = e.c.pr_states(&t.id)[0].at;
    let (_, ns) = e.c.record_pr_state(&t.id, check(PR_OPEN, "")).unwrap();
    assert!(ns.is_empty());
    assert_eq!(e.c.pr_states(&t.id)[0].at, at);

    // Merged: the holder is woken to complete with proof, and may.
    e.clk.add(secs(60));
    let (_, ns) =
        e.c.record_pr_state(&t.id, check(PR_MERGED, "APPROVED"))
            .unwrap();
    assert_eq!(ns.len(), 1);
    assert_eq!(ns[0].kind, NOTICE_PR);
    assert_eq!(ns[0].to, vec![ALICE.to_string()]);
    assert!(ns[0].wake);
    assert!(ns[0].text.contains("was merged"), "{}", ns[0].text);
    e.c.set_task_state(ALICE, &t.id, COMPLETED, "merged")
        .unwrap();
}

#[test]
fn changes_requested_on_github_sends_a_card_in_review_back_to_work() {
    let e = Env::new();
    let t = held_with_pr(&e);
    e.c.record_pr_state(&t.id, check(PR_OPEN, "REVIEW_REQUIRED"))
        .unwrap();
    e.c.set_task_state(ALICE, &t.id, IN_REVIEW, "").unwrap();

    e.clk.add(secs(60));
    let (t2, ns) =
        e.c.record_pr_state(&t.id, check(PR_OPEN, PR_CHANGES_REQUESTED))
            .unwrap();
    assert_eq!(t2.state, WORKING);
    assert_eq!(ns.len(), 1);
    assert!(ns[0].wake);
    assert!(ns[0].text.contains("changes requested"), "{}", ns[0].text);

    // Still changes requested: once is enough.
    e.clk.add(secs(60));
    let (_, ns) =
        e.c.record_pr_state(&t.id, check(PR_OPEN, PR_CHANGES_REQUESTED))
            .unwrap();
    assert!(ns.is_empty());
}

#[test]
fn closed_unmerged_tells_the_holder() {
    let e = Env::new();
    let t = held_with_pr(&e);
    e.c.record_pr_state(&t.id, check(PR_OPEN, "")).unwrap();
    e.clk.add(secs(60));
    let (_, ns) = e.c.record_pr_state(&t.id, check(PR_CLOSED, "")).unwrap();
    assert_eq!(ns.len(), 1);
    assert!(
        ns[0].text.contains("closed without merging"),
        "{}",
        ns[0].text
    );
    // A closed PR doesn't hold completed.
    e.c.set_task_state(ALICE, &t.id, COMPLETED, "dropped")
        .unwrap();
}

#[test]
fn no_gh_is_said_once_and_falls_back() {
    let e = Env::new();
    let t = held_with_pr(&e);
    let fail = PrCheck {
        pr: PR.into(),
        error: "gh is not installed".into(),
        ..PrCheck::default()
    };
    let (_, ns) = e.c.record_pr_state(&t.id, fail.clone()).unwrap();
    assert!(ns.is_empty());
    let st = e.c.pr_states(&t.id);
    assert_eq!(st[0].error, "gh is not installed");
    let at = st[0].at;
    // Again: no second row.
    e.clk.add(secs(60));
    e.c.record_pr_state(&t.id, fail).unwrap();
    assert_eq!(e.c.pr_states(&t.id)[0].at, at);
    // Unknown doesn't hold completed.
    e.c.set_task_state(ALICE, &t.id, COMPLETED, "done").unwrap();
}

fn failed() -> PrCheck {
    PrCheck {
        pr: PR.into(),
        error: "gh is not installed".into(),
        ..PrCheck::default()
    }
}

/// Card #542: a device that can't ask GitHub says nothing about a PR whose
/// state is known, so no row is written and the two sides can't keep
/// rewriting each other.
#[test]
fn an_error_over_a_known_state_writes_nothing() {
    for state in [PR_OPEN, PR_MERGED, PR_CLOSED] {
        let e = Env::new();
        let t = held_with_pr(&e);
        e.c.record_pr_state(&t.id, check(state, "APPROVED"))
            .unwrap();
        let before = e.c.pr_states(&t.id);
        let rows = e.rows("pr.").len();

        e.clk.add(secs(60));
        let (_, ns) = e.c.record_pr_state(&t.id, failed()).unwrap();
        assert!(ns.is_empty(), "{state}");
        assert_eq!(e.c.pr_states(&t.id), before, "{state}");
        assert_eq!(e.rows("pr.").len(), rows, "{state}");

        // The next good answer is no change either.
        e.c.record_pr_state(&t.id, check(state, "APPROVED"))
            .unwrap();
        assert_eq!(e.rows("pr.").len(), rows, "{state}");
    }
}

#[test]
fn only_a_linked_pr_is_recorded() {
    let e = Env::new();
    let t = held_with_pr(&e);
    let mut c = check(PR_OPEN, "");
    c.pr = "https://github.com/a/b/pull/8".into();
    want_err(e.c.record_pr_state(&t.id, c), ErrorKind::Invalid);
}
