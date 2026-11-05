//! Card #509: an agent's external wait on a GitHub PR is the person's
//! merge. It counts in their Needs you (so Home shows it and it notifies)
//! until the PR is known merged or closed, or the wait is closed.

mod common;

use common::*;
use coord::rows::PrStateRow;
use coord::*;
use gocompat::time::GoTime;
use store::kinds;

const PR: &str = "https://github.com/a/b/pull/41";

fn claimed(e: &Env, title: &str) -> Task {
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: title.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap()
}

fn external(e: &Env, id: &str, on: &str, links: &[&str]) -> Blocker {
    e.c.raise(
        ALICE,
        id,
        NewBlocker {
            question: "Review and merge PR #41 so part 10 lands on main".into(),
            blocked_by: Some(BlockedBy {
                kind: BLOCKED_ON_EXTERNAL.into(),
                reference: on.into(),
            }),
            links: links.iter().map(|s| s.to_string()).collect(),
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
fn a_wait_on_a_pr_is_the_persons_merge() {
    let e = Env::new();
    let t = claimed(&e, "part 10");
    let b = external(&e, &t.id, PR, &[]);
    assert_eq!(b.merge_wait().as_deref(), Some(PR));
    assert_eq!(lane(&e, USER, &t.id), (true, false), "the person");
    assert_eq!(lane(&e, "", &t.id), (true, false), "no viewer");
    // Not an agent's: neither the raiser's nor another's.
    assert!(!lane(&e, ALICE, &t.id).0);
    assert!(!lane(&e, BOB, &t.id).0);
}

#[test]
fn the_pr_may_be_only_among_the_waits_links() {
    let e = Env::new();
    let t = claimed(&e, "part 10");
    external(&e, &t.id, "PR #41 review", &[PR]);
    assert_eq!(lane(&e, USER, &t.id), (true, false));
}

#[test]
fn a_wait_on_anything_else_stays_blocked() {
    let e = Env::new();
    let t = claimed(&e, "release");
    let b = external(
        &e,
        &t.id,
        "CI on the release",
        &["https://ci.example.com/run/7"],
    );
    assert_eq!(b.merge_wait(), None);
    assert_eq!(lane(&e, USER, &t.id), (false, true));
    // A PR that isn't GitHub's isn't one the person merges here.
    let u = claimed(&e, "elsewhere");
    external(&e, &u.id, "https://gitlab.com/a/b/-/merge_requests/3", &[]);
    assert_eq!(lane(&e, USER, &u.id), (false, true));
}

#[test]
fn a_merged_or_closed_pr_leaves_needs_you() {
    for state in [PR_MERGED, PR_CLOSED] {
        let e = Env::new();
        let t = claimed(&e, "part 10");
        // The PR is only on the wait, not linked on the card: the poller's
        // answer is still taken for it.
        external(&e, &t.id, PR, &[]);
        e.c.record_pr_state(
            &t.id,
            PrCheck {
                pr: PR.into(),
                state: PR_OPEN.into(),
                ..PrCheck::default()
            },
        )
        .unwrap();
        assert!(lane(&e, USER, &t.id).0, "open");
        e.c.record_pr_state(
            &t.id,
            PrCheck {
                pr: PR.into(),
                state: state.into(),
                ..PrCheck::default()
            },
        )
        .unwrap();
        assert_eq!(lane(&e, USER, &t.id), (false, true), "{state}");
    }
}

/// A `pr.state` row as a device writes it: an error and an empty state when
/// it couldn't ask GitHub, as phones before card #542 still do.
fn pr_row(id: &str, state: &str, error: &str, at: GoTime) -> PrStateRow {
    PrStateRow {
        task_id: id.into(),
        pr: PR.into(),
        state: state.into(),
        error: error.into(),
        by: "irori".into(),
        at,
        ..PrStateRow::default()
    }
}

/// Card #542: the phone's "can't reach GitHub" row came after MERGED and
/// put the merge back in the person's Needs you. It holds live and on a
/// replay, for each final state.
#[test]
fn an_error_row_after_a_final_state_changes_nothing() {
    for state in [PR_MERGED, PR_CLOSED] {
        let e = Env::new();
        let t = claimed(&e, "part 10");
        external(&e, &t.id, PR, &[]);
        e.c.record_pr_state(
            &t.id,
            PrCheck {
                pr: PR.into(),
                state: state.into(),
                ..PrCheck::default()
            },
        )
        .unwrap();
        assert_eq!(lane(&e, USER, &t.id), (false, true), "{state}");

        let later = plus(e.clk.now(), mins(5));
        e.remote(
            "phone",
            1,
            hlc_at(later),
            "irori",
            kinds::PR_STATE,
            &pr_row(&t.id, "", "gh isn't installed here", later),
        );

        let heard = e.c.pr_state(&t.id, PR).unwrap();
        assert_eq!(heard.state, state);
        assert!(heard.error.is_empty(), "{state}");
        assert_eq!(lane(&e, USER, &t.id), (false, true), "{state}");
        assert_eq!(e.reopen().pr_state(&t.id, PR).unwrap().state, state);
    }
}

/// The error row can reach a device first, with the later time: the merge
/// that arrives after it must still count, or devices disagree for good.
#[test]
fn a_known_state_beats_an_error_row_that_arrived_first() {
    let e = Env::new();
    let t = claimed(&e, "part 10");
    external(&e, &t.id, PR, &[]);
    let merged_at = e.clk.now();
    let later = plus(merged_at, mins(5));
    e.remote(
        "phone",
        1,
        hlc_at(later),
        "irori",
        kinds::PR_STATE,
        &pr_row(&t.id, "", "gh isn't installed here", later),
    );
    assert_eq!(e.c.pr_state(&t.id, PR).unwrap().state, "");
    assert_eq!(lane(&e, USER, &t.id), (true, false), "nothing known yet");

    e.remote(
        "mac",
        1,
        hlc_at(merged_at),
        "irori",
        kinds::PR_STATE,
        &pr_row(&t.id, PR_MERGED, "", merged_at),
    );
    assert_eq!(e.c.pr_state(&t.id, PR).unwrap().state, PR_MERGED);
    assert_eq!(lane(&e, USER, &t.id), (false, true));
}

#[test]
fn a_closed_wait_no_longer_counts() {
    let e = Env::new();
    let t = claimed(&e, "part 10");
    external(&e, &t.id, PR, &[]);
    e.c.unblock(ALICE, &t.id, "merged by hand").unwrap();
    let t = e.c.get_task(&t.id).unwrap();
    assert!(t.blockers.iter().all(|b| b.merge_wait().is_none()));
    assert!(!lane(&e, USER, &t.id).0);
}

#[test]
fn a_pr_on_neither_the_card_nor_a_wait_is_refused() {
    let e = Env::new();
    let t = claimed(&e, "part 10");
    external(&e, &t.id, PR, &[]);
    let err =
        e.c.record_pr_state(
            &t.id,
            PrCheck {
                pr: "https://github.com/a/b/pull/42".into(),
                state: PR_OPEN.into(),
                ..PrCheck::default()
            },
        )
        .unwrap_err();
    assert!(err.to_string().contains("links no pull request"), "{err}");
}
