//! Card #509: an agent's external wait on a GitHub PR is polled, whoever
//! holds the card, so its holders hear the PR merge. Card #970: it stays
//! its agent's, in Blocked: it never counts in the person's Needs you (no
//! row asks them to merge), open, merged or closed.

use crate::common;

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
fn a_wait_on_a_pr_stays_its_agents() {
    let e = Env::new();
    let t = claimed(&e, "part 10");
    let b = external(&e, &t.id, PR, &[]);
    assert_eq!(b.pr_wait().as_deref(), Some(PR));
    assert_eq!(e.c.polled_waits(&t.id), [PR]);
    assert_eq!(lane(&e, USER, &t.id), (false, true), "the person");
    assert_eq!(lane(&e, "", &t.id), (false, true), "no viewer");
    // Not an agent's: neither the raiser's nor another's.
    assert!(!lane(&e, ALICE, &t.id).0);
    assert!(!lane(&e, BOB, &t.id).0);
}

#[test]
fn the_pr_may_be_only_among_the_waits_links() {
    let e = Env::new();
    let t = claimed(&e, "part 10");
    external(&e, &t.id, "PR #41 review", &[PR]);
    assert_eq!(e.c.polled_waits(&t.id), [PR]);
    assert_eq!(lane(&e, USER, &t.id), (false, true));
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
    assert_eq!(b.pr_wait(), None);
    assert!(e.c.polled_waits(&t.id).is_empty());
    assert_eq!(lane(&e, USER, &t.id), (false, true));
    // A PR that isn't GitHub's isn't one Irori polls.
    let u = claimed(&e, "elsewhere");
    external(&e, &u.id, "https://gitlab.com/a/b/-/merge_requests/3", &[]);
    assert!(e.c.polled_waits(&u.id).is_empty());
    assert_eq!(lane(&e, USER, &u.id), (false, true));
}

/// Open, merged or closed, the wait stays Blocked; a merged PR isn't asked
/// about again, a closed one is (GitHub can reopen it, card #548).
#[test]
fn a_pr_wait_is_polled_until_it_merges_and_never_needs_the_person() {
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
        assert_eq!(lane(&e, USER, &t.id), (false, true), "open");
        assert_eq!(e.c.polled_waits(&t.id), [PR]);
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
        assert_eq!(
            e.c.polled_waits(&t.id).is_empty(),
            state == PR_MERGED,
            "{state}"
        );
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
/// undid what was known of the PR. It holds live and on a replay, for
/// each final state.
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
    assert_eq!(lane(&e, USER, &t.id), (false, true), "nothing known yet");

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

/// A merge can't be undone: a device that asked just before it may still
/// write OPEN with a later time, and that row must lose.
#[test]
fn a_stale_open_never_replaces_a_merge() {
    let e = Env::new();
    let t = claimed(&e, "part 10");
    external(&e, &t.id, PR, &[]);
    let merged_at = e.clk.now();
    let later = plus(merged_at, mins(5));
    e.remote(
        "pc",
        1,
        hlc_at(merged_at),
        "irori",
        kinds::PR_STATE,
        &pr_row(&t.id, PR_MERGED, "", merged_at),
    );
    e.remote(
        "mac",
        1,
        hlc_at(later),
        "irori",
        kinds::PR_STATE,
        &pr_row(&t.id, PR_OPEN, "", later),
    );
    assert_eq!(e.c.pr_state(&t.id, PR).unwrap().state, PR_MERGED);
    assert_eq!(lane(&e, USER, &t.id), (false, true));

    // The writer agrees: no row, and no "wasn't taken" error at each poll.
    let rows = e.rows("pr.").len();
    e.c.record_pr_state(
        &t.id,
        PrCheck {
            pr: PR.into(),
            state: PR_OPEN.into(),
            ..PrCheck::default()
        },
    )
    .unwrap();
    assert_eq!(e.rows("pr.").len(), rows);
}

/// A phone hears a merge only as another device's row, so that row has to
/// tell the window: nothing else redraws the card.
#[test]
fn another_devices_merge_tells_subscribers() {
    let e = Env::new();
    let t = claimed(&e, "part 10");
    external(&e, &t.id, PR, &[]);
    let q = e.c.subscribe();
    let at = e.clk.now();
    e.remote(
        "mac",
        1,
        hlc_at(at),
        "irori",
        kinds::PR_STATE,
        &pr_row(&t.id, PR_MERGED, "", at),
    );
    let ch = q.recv_timeout(std::time::Duration::from_secs(2));
    let ch = ch.unwrap().expect("a change");
    assert_eq!(
        (ch.kind.as_str(), ch.task_id.as_str()),
        (TASK_CHANGED, t.id.as_str())
    );
}

#[test]
fn a_closed_wait_no_longer_counts() {
    let e = Env::new();
    let t = claimed(&e, "part 10");
    external(&e, &t.id, PR, &[]);
    e.c.unblock(ALICE, &t.id, "merged by hand").unwrap();
    let t = e.c.get_task(&t.id).unwrap();
    assert!(t.blockers.iter().all(|b| b.pr_wait().is_none()));
    assert!(e.c.polled_waits(&t.id).is_empty());
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
