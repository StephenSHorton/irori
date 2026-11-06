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

/// Card #421: a desk "PR #n" card whose merge an older device heard first
/// (no sha, and no rule completing the card) completes when the sha comes.
#[test]
fn a_desk_pr_card_merged_without_its_sha_completes_when_the_sha_comes() {
    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
    let e = Env::new();
    let t =
        e.c.create_task(
            "irori",
            NewTask {
                title: "PR #7: fix it".into(),
                objective: format!("PR #7 on a/b {DESK_PR_CARD_MARK}; its thread."),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.add_link("irori", &t.id, PR).unwrap();
    let t = e.c.get_task(&t.id).unwrap();
    assert!(desk_pr_card(&t), "{t:?}");
    // The older device's row: MERGED, no sha; its fold left the card open.
    e.clk.add(secs(60));
    e.remote(
        "dev-old",
        1,
        1,
        "irori",
        store::kinds::PR_STATE,
        &serde_json::json!({
            "taskId": t.id, "pr": PR, "state": PR_MERGED,
            "mergedAt": "2026-10-26T12:00:30Z", "by": "irori",
            "at": "2026-10-26T12:01:00Z",
        }),
    );
    assert_eq!(e.c.pr_states(&t.id)[0].state, PR_MERGED);
    assert_ne!(e.c.get_task(&t.id).unwrap().state, COMPLETED);

    e.clk.add(secs(60));
    e.c.record_pr_state(
        &t.id,
        PrCheck {
            pr: PR.into(),
            state: PR_MERGED.into(),
            merge_sha: SHA.into(),
            ..PrCheck::default()
        },
    )
    .unwrap();
    assert_eq!(e.c.get_task(&t.id).unwrap().state, COMPLETED);
}

/// Card #421: the holder's poller and the desk's poller, on two devices,
/// both hear the merge with its sha before either's rows reach the other.
/// Each says the sha on the card; once both logs meet, the card shows it
/// once, on both devices.
#[test]
fn two_devices_hearing_one_merge_say_its_sha_once() {
    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
    let a = Env::on("dev-a");
    let t = held_with_pr(&a);
    let rows = |e: &Env, dev: &str| -> Vec<store::Event> {
        e.st.missing(&store::Vector::new(), 10_000)
            .unwrap()
            .into_iter()
            .filter(|ev| ev.device_id == dev)
            .collect()
    };
    let mut b = Env::on("dev-b");
    let (_, r) = b.st.insert_batch(&rows(&a, "dev-a"));
    r.unwrap();
    b.c = b.reopen();
    assert!(b.c.get_task(&t.id).is_some());

    let merged_at = plus(fixed_now(), secs(30));
    let heard = || PrCheck {
        pr: PR.into(),
        state: PR_MERGED.into(),
        merge_sha: SHA.into(),
        merged_at,
        ..PrCheck::default()
    };
    a.clk.add(secs(60));
    b.clk.add(secs(61));
    let before = rows(&a, "dev-a").len();
    a.c.record_pr_state(&t.id, heard()).unwrap();
    b.c.record_pr_state(&t.id, heard()).unwrap();

    // The logs meet: each device takes the other's new rows.
    let from_a: Vec<_> = rows(&a, "dev-a").into_iter().skip(before).collect();
    let from_b = rows(&b, "dev-b");
    for (e, evs) in [(&a, from_b), (&b, from_a)] {
        let (_, r) = e.st.insert_batch(&evs);
        r.unwrap();
        e.c.fold_live(&evs);
    }
    let said = |c: &Coord| {
        c.comments(&t.id)
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c.text.contains(SHA))
            .count()
    };
    for e in [&a, &b] {
        assert_eq!(said(&e.c), 1);
        assert_eq!(e.c.pr_states(&t.id)[0].merge_sha, SHA);
        // Replayed from the log, still once.
        assert_eq!(said(&e.reopen()), 1);
    }
}
