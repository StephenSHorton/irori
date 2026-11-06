//! A card's linked PRs as GitHub says they stand (card #208): the poller's
//! checks become pr.state rows, an open PR holds an agent's completed, a
//! merge wakes the holder, changes requested sends a card in review back
//! to working, and a GitHub that can't be asked is said once.

use crate::common;

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
    // Card #614: the closing comment may say where the work landed.
    assert!(
        ns[0].text.contains(&format!(
            "Read the closing comment first (`gh pr view {PR} --comments`)"
        )),
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

/// Card #542: a device that can't ask GitHub says nothing about a PR known
/// merged or closed, so no row is written and the two sides can't keep
/// rewriting each other.
#[test]
fn an_error_over_a_settled_state_writes_nothing() {
    for state in [PR_MERGED, PR_CLOSED] {
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

/// Card #548: gh breaking on a PR known open keeps OPEN and shows why,
/// once. Completed stays refused, as one failed gh mustn't let open work
/// through, and the refusal says what is wrong. A good answer clears it.
#[test]
fn gh_failing_on_an_open_pr_keeps_it_open_and_says_why() {
    let e = Env::new();
    let t = held_with_pr(&e);
    e.c.record_pr_state(&t.id, check(PR_OPEN, "APPROVED"))
        .unwrap();
    let rows = e.rows("pr.").len();

    e.clk.add(secs(60));
    let (_, ns) = e.c.record_pr_state(&t.id, failed()).unwrap();
    assert!(ns.is_empty(), "a failure tells nobody");
    let st = e.c.pr_state(&t.id, PR).unwrap();
    assert_eq!(
        (st.state.as_str(), st.review.as_str(), st.error.as_str()),
        (PR_OPEN, "APPROVED", "gh is not installed")
    );
    assert_eq!(e.rows("pr.").len(), rows + 1);

    // Said once.
    e.clk.add(secs(60));
    e.c.record_pr_state(&t.id, failed()).unwrap();
    assert_eq!(e.rows("pr.").len(), rows + 1);

    let err =
        e.c.set_task_state(ALICE, &t.id, COMPLETED, "done")
            .unwrap_err();
    assert!(
        err.to_string().contains("gh is not installed") && err.to_string().contains(PR),
        "{err}"
    );
    let shown = search::Source::search_show(
        &e.c,
        &format!("task:{}", t.id),
        &search::ShowOptions::default(),
    )
    .unwrap()
    .text;
    assert!(
        shown.contains("GitHub: open, approved; not asked since: gh is not installed"),
        "{shown}"
    );

    // gh works again: the error goes, and a merge is heard as a merge.
    e.clk.add(secs(60));
    e.c.record_pr_state(&t.id, check(PR_OPEN, "APPROVED"))
        .unwrap();
    assert!(e.c.pr_state(&t.id, PR).unwrap().error.is_empty());
    e.clk.add(secs(60));
    e.c.record_pr_state(&t.id, failed()).unwrap();
    e.clk.add(secs(60));
    let (_, ns) = e.c.record_pr_state(&t.id, check(PR_MERGED, "")).unwrap();
    assert_eq!(ns.len(), 1);
    assert!(ns[0].text.contains("was merged"), "{}", ns[0].text);
    assert!(e.c.pr_state(&t.id, PR).unwrap().error.is_empty());
}

/// Card #548: GitHub reopens a closed PR. The holder hears it without a
/// wake, completed is refused again, and the merge is told as ever.
#[test]
fn a_reopened_pr_is_open_again_and_the_holder_hears() {
    let e = Env::new();
    let t = held_with_pr(&e);
    e.c.record_pr_state(&t.id, check(PR_OPEN, "")).unwrap();
    e.clk.add(secs(60));
    let (_, ns) = e.c.record_pr_state(&t.id, check(PR_CLOSED, "")).unwrap();
    assert_eq!(ns.len(), 1);

    // Still closed, a review changing on it says nothing.
    e.clk.add(secs(60));
    let (_, ns) =
        e.c.record_pr_state(&t.id, check(PR_CLOSED, PR_CHANGES_REQUESTED))
            .unwrap();
    assert!(ns.is_empty(), "{ns:?}");

    e.clk.add(secs(60));
    let (_, ns) = e.c.record_pr_state(&t.id, check(PR_OPEN, "")).unwrap();
    assert_eq!(ns.len(), 1);
    assert!(ns[0].text.contains("was reopened"), "{}", ns[0].text);
    assert!(!ns[0].wake);
    assert!(
        e.c.set_task_state(ALICE, &t.id, COMPLETED, "done").is_err(),
        "open again"
    );

    e.clk.add(secs(60));
    let (_, ns) = e.c.record_pr_state(&t.id, check(PR_MERGED, "")).unwrap();
    assert_eq!(ns.len(), 1);
    assert!(ns[0].wake);
}

/// Card #548: a merge wait on a closed PR leaves the person's merges but
/// is still asked about, and comes back when GitHub reopens it.
#[test]
fn a_closed_merge_wait_is_still_polled_and_comes_back_on_a_reopen() {
    let e = Env::new();
    let t = held_with_pr(&e);
    e.c.block(
        ALICE,
        &t.id,
        "Review and merge PR #7",
        Some(BlockedBy {
            kind: BLOCKED_ON_EXTERNAL.into(),
            reference: PR.into(),
        }),
    )
    .unwrap();
    e.c.record_pr_state(&t.id, check(PR_CLOSED, "")).unwrap();
    assert!(e.c.merge_waits(&t.id).is_empty());
    assert_eq!(e.c.polled_waits(&t.id), vec![PR.to_string()]);

    e.clk.add(secs(60));
    e.c.record_pr_state(&t.id, check(PR_OPEN, "")).unwrap();
    assert_eq!(e.c.merge_waits(&t.id), vec![PR.to_string()]);

    e.clk.add(secs(60));
    e.c.record_pr_state(&t.id, check(PR_MERGED, "")).unwrap();
    assert!(e.c.polled_waits(&t.id).is_empty(), "a merge is final");
}

#[test]
fn only_a_linked_pr_is_recorded() {
    let e = Env::new();
    let t = held_with_pr(&e);
    let mut c = check(PR_OPEN, "");
    c.pr = "https://github.com/a/b/pull/8".into();
    want_err(e.c.record_pr_state(&t.id, c), ErrorKind::Invalid);
}
