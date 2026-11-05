//! Card #208: the Core asks GitHub (here a fake gh) about the PRs its own
//! agents' cards link, records what it hears, and routes what changed.

use crate::common;

use std::sync::{Arc, Mutex};

use common::*;
use coord::{NewTask, PR_MERGED, PR_OPEN, PrCheck};
use irori_core::prwatch::PrChecker;

const PR: &str = "https://github.com/a/b/pull/12";

/// A gh that answers with whatever state the test set, counting calls.
#[derive(Default)]
struct FakeGh {
    state: Mutex<String>,
    calls: Mutex<Vec<String>>,
}

impl PrChecker for FakeGh {
    fn check(&self, url: &str) -> PrCheck {
        self.calls.lock().unwrap().push(url.to_string());
        PrCheck {
            pr: url.into(),
            state: self.state.lock().unwrap().clone(),
            ..PrCheck::default()
        }
    }
}

#[test]
fn the_holders_device_asks_github_and_a_merge_reaches_the_holder() {
    let gh = Arc::new(FakeGh::default());
    *gh.state.lock().unwrap() = PR_OPEN.into();
    let g2 = gh.clone();
    let r = Rig::with(move |o| o.pr_checker = Some(g2));
    let co = r.core.coord();

    let t = co
        .create_task(
            &r.key_a,
            NewTask {
                title: "land the fix".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap();
    co.add_link(&r.key_a, &t.id, PR).unwrap();
    // A card held by an agent on another device: not this device's to ask.
    let other = co
        .create_task(
            "otherdevice/sess",
            NewTask {
                title: "theirs".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.claim_task("otherdevice/sess", &other.id).unwrap();
    co.add_link(
        "otherdevice/sess",
        &other.id,
        "https://github.com/a/b/pull/13",
    )
    .unwrap();

    r.core.check_prs();
    assert_eq!(*gh.calls.lock().unwrap(), vec![PR.to_string()]);
    assert_eq!(co.pr_states(&t.id)[0].state, PR_OPEN);
    // Asked again at once: too soon.
    r.core.check_prs();
    assert_eq!(gh.calls.lock().unwrap().len(), 1);

    // An agent's completed is refused while it is open.
    assert!(
        co.set_task_state(&r.key_a, &t.id, coord::COMPLETED, "done")
            .is_err()
    );

    // Merged (heard on the next due check): the holder hears it, woken.
    *gh.state.lock().unwrap() = PR_MERGED.into();
    let before = r.notices_to(&r.key_a).len();
    // The throttle is minutes; feed the merge through coord as the poller
    // would, then route it the same way.
    let (_, ns) = co.record_pr_state(&t.id, gh.check(PR)).unwrap();
    assert_eq!(ns.len(), 1);
    r.core.route_notices(&inbox::system_sender(), &ns).unwrap();
    let after = r.notices_to(&r.key_a);
    assert_eq!(after.len(), before + 1);
    let last = after.last().unwrap();
    assert!(
        last.message.text.contains("was merged"),
        "{}",
        last.message.text
    );
}

/// A device with no gh.
#[derive(Default)]
struct NoGh {
    calls: Mutex<u32>,
}

impl PrChecker for NoGh {
    fn check(&self, url: &str) -> PrCheck {
        *self.calls.lock().unwrap() += 1;
        PrCheck {
            pr: url.into(),
            error: "gh isn't installed here".into(),
            ..PrCheck::default()
        }
    }

    fn ready(&self) -> bool {
        false
    }
}

/// Card #542: a device that can't ask GitHub asks about nothing and writes
/// no row, even for a merge wait, which any of the person's devices polls.
#[test]
fn a_device_with_no_gh_asks_nothing_and_writes_nothing() {
    let gh = Arc::new(NoGh::default());
    let g2 = gh.clone();
    let r = Rig::with(move |o| o.pr_checker = Some(g2));
    let co = r.core.coord();
    let t = co
        .create_task(
            "otherdevice/sess",
            NewTask {
                title: "part 10".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.claim_task("otherdevice/sess", &t.id).unwrap();
    co.block(
        "otherdevice/sess",
        &t.id,
        "Review and merge PR #12",
        Some(coord::BlockedBy {
            kind: coord::BLOCKED_ON_EXTERNAL.into(),
            reference: PR.into(),
        }),
    )
    .unwrap();

    r.core.check_prs();
    assert_eq!(*gh.calls.lock().unwrap(), 0);
    assert!(co.pr_state(&t.id, PR).is_none());
}

/// Card #509: a wait on a PR's merge is asked about even when nobody here
/// holds its card (its agent may be gone), so the person's Needs you
/// drops it once the PR merges.
#[test]
fn a_merge_wait_is_asked_about_whoever_holds_the_card() {
    let gh = Arc::new(FakeGh::default());
    *gh.state.lock().unwrap() = PR_OPEN.into();
    let g2 = gh.clone();
    let r = Rig::with(move |o| o.pr_checker = Some(g2));
    let co = r.core.coord();
    let t = co
        .create_task(
            "otherdevice/sess",
            NewTask {
                title: "part 10".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.claim_task("otherdevice/sess", &t.id).unwrap();
    co.block(
        "otherdevice/sess",
        &t.id,
        "Review and merge PR #12",
        Some(coord::BlockedBy {
            kind: coord::BLOCKED_ON_EXTERNAL.into(),
            reference: PR.into(),
        }),
    )
    .unwrap();
    let needs = || {
        co.board(&coord::BoardFilter::default())
            .needs_you
            .iter()
            .any(|c| c.task.id == t.id)
    };
    assert!(needs());

    r.core.check_prs();
    assert_eq!(*gh.calls.lock().unwrap(), vec![PR.to_string()]);
    assert_eq!(co.pr_state(&t.id, PR).unwrap().state, PR_OPEN);
    assert!(needs(), "still open");

    *gh.state.lock().unwrap() = PR_MERGED.into();
    co.record_pr_state(&t.id, gh.check(PR)).unwrap();
    assert!(!needs(), "merged: no longer the person's");
}
