//! Card #786 job (a) through the Core: the switch and the listener. The
//! sweep's part is in coord's tidy_test; here a PR that merges on a card
//! nobody holds gets the question by itself, the switch only turns that
//! off, and the job's own writes don't repeat it.

use crate::common;

use common::*;
use coord::{JOB_MERGED, NewTask, PR_MERGED, PR_OPEN, PrCheck};

const PR: &str = "https://github.com/a/b/pull/7";

fn person(r: &Rig) -> String {
    format!("user:{}", r.core.device_id())
}

/// A card nobody holds, linking `PR` and heard as `state`.
fn card_on_pr(r: &Rig, state: &str) -> String {
    let c = r.core.coord();
    let t = c
        .create_task(
            &person(r),
            NewTask {
                title: "ship it".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    c.add_link(&person(r), &t.id, PR).unwrap();
    c.record_pr_state(
        &t.id,
        PrCheck {
            pr: PR.into(),
            state: state.into(),
            ..PrCheck::default()
        },
    )
    .unwrap();
    t.id
}

fn did(r: &Rig, id: &str) -> usize {
    r.core.coord().get_task(id).unwrap().did.len()
}

#[test]
fn a_pr_merging_on_a_card_nobody_holds_is_asked_about() {
    let r = Rig::new();
    assert!(r.core.tidy_on(), "on unless the person turns it off");
    let id = card_on_pr(&r, PR_MERGED);
    wait_for(10, "the card flagged", || {
        r.core
            .coord()
            .get_task(&id)
            .unwrap()
            .did
            .iter()
            .any(|d| d.job == JOB_MERGED)
    });
    let t = r.core.coord().get_task(&id).unwrap();
    assert_eq!(t.state, coord::BLOCKED);
    assert!(t.blockers.iter().any(|b| b.open() && b.done_check));
    // Its own writes fired more changes; it stays one question.
    std::thread::sleep(std::time::Duration::from_millis(600));
    assert_eq!(did(&r, &id), 1);
    assert_eq!(
        r.core
            .coord()
            .get_task(&id)
            .unwrap()
            .blockers
            .iter()
            .filter(|b| b.open())
            .count(),
        1
    );
}

#[test]
fn switched_off_it_adds_nothing() {
    let r = Rig::new();
    r.core.set_tidy_on(false).unwrap();
    assert!(!r.core.tidy_on());
    let id = card_on_pr(&r, PR_MERGED);
    std::thread::sleep(std::time::Duration::from_millis(600));
    assert_eq!(did(&r, &id), 0);
    assert_eq!(
        r.core.coord().get_task(&id).unwrap().state,
        coord::SUBMITTED
    );
    r.core.set_tidy_on(true).unwrap();
    assert!(r.core.tidy_on(), "the switch holds across a toggle");
}

#[test]
fn an_open_pr_is_left_alone() {
    let r = Rig::new();
    let id = card_on_pr(&r, PR_OPEN);
    std::thread::sleep(std::time::Duration::from_millis(600));
    assert_eq!(did(&r, &id), 0);
}

/// A change dropped from the feed arrives only as a resync, and a merged
/// PR's row never comes again.
#[test]
fn a_resync_finds_a_merged_card_whose_change_was_dropped() {
    let r = Rig::new();
    // Off, so nothing hears the merge: the state a dropped change leaves.
    r.core.set_tidy_on(false).unwrap();
    let id = card_on_pr(&r, PR_MERGED);
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(did(&r, &id), 0);
    r.core.set_tidy_on(true).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(did(&r, &id), 0, "the switch alone doesn't look");
    r.core.coord().refold().unwrap();
    wait_for(10, "the card flagged after the resync", || {
        did(&r, &id) == 1
    });
    r.core.coord().refold().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(600));
    assert_eq!(did(&r, &id), 1, "asked once");
}
