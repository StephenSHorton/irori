//! Card #418 (merge-desk.md §5): a merge desk's lease `desk/<owner>/<name>`
//! is taken only by the repo's desk holder (the person's `repo.desk`), one
//! PR at a time, never with steal; naming another holder releases the old
//! holder's lease, and so does the holder going. `desk.run` counts only
//! from the holder under the lease, and Merging ends with the lease.

mod common;

use common::*;
use coord::rows::LeaseReleasedRow;
use coord::*;
use store::kinds;

/// REPO_A is remote:github.com/a/b.
const DESK: &str = "desk/a/b";
const SHA: &str = "3f2a9c1e0b7d4a5b";

fn desk_held_by(e: &Env, holder: &str) {
    let d = RepoDeskRow {
        holder: holder.into(),
        ..new_desk(REPO_A, "dev-a")
    };
    e.c.set_desk(USER, d).unwrap();
}

#[test]
fn the_desk_lease_is_the_lowercased_owner_and_name() {
    assert_eq!(
        desk_resource("StephenSHorton/Irori"),
        "desk/stephenshorton/irori"
    );
    assert_eq!(
        normalize_resource(&desk_resource(" You/Game-Repo ")).unwrap(),
        "desk/you/game-repo"
    );
    assert!(is_desk(DESK));
    assert!(!is_desk("ww/merge"));
}

#[test]
fn only_the_desk_holder_takes_it_and_not_without_a_desk_or_holder() {
    let e = Env::new();
    // No desk for the repo yet.
    want_err(
        e.c.acquire(ALICE, DESK, mins(30), false),
        ErrorKind::NotAllowed,
    );
    // Unheld.
    desk_held_by(&e, "");
    want_err(
        e.c.acquire(ALICE, DESK, mins(30), false),
        ErrorKind::NotAllowed,
    );
    desk_held_by(&e, ALICE);
    want_err(
        e.c.acquire(BOB, DESK, mins(30), false),
        ErrorKind::NotAllowed,
    );
    want_err(
        e.c.acquire(USER, DESK, mins(30), false),
        ErrorKind::NotAllowed,
    );
    let l = e.c.acquire(ALICE, DESK, mins(30), false).unwrap();
    assert_eq!((l.resource.as_str(), l.holder.as_str()), (DESK, ALICE));
    // Renewing in the run's steps keeps it.
    e.c.acquire(ALICE, DESK, mins(30), false).unwrap();
}

#[test]
fn a_paused_desk_takes_no_lease() {
    let e = Env::new();
    desk_held_by(&e, ALICE);
    e.c.pause_desk(USER, REPO_A, true).unwrap();
    want_err(
        e.c.acquire(ALICE, DESK, mins(30), false),
        ErrorKind::NotAllowed,
    );
    e.c.pause_desk(USER, REPO_A, false).unwrap();
    e.c.acquire(ALICE, DESK, mins(30), false).unwrap();
}

#[test]
fn a_second_take_is_refused_while_a_run_is_on() {
    let e = Env::new();
    desk_held_by(&e, ALICE);
    e.c.acquire(ALICE, DESK, mins(30), false).unwrap();
    want_err(
        e.c.acquire(BOB, DESK, mins(30), false),
        ErrorKind::NotAllowed,
    );
    let held: Vec<_> =
        e.c.leases()
            .into_iter()
            .filter(|l| l.resource == DESK)
            .collect();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].holder, ALICE);
}

#[test]
fn steal_is_refused_even_for_the_holder() {
    let e = Env::new();
    desk_held_by(&e, ALICE);
    e.c.acquire(ALICE, DESK, mins(30), false).unwrap();
    want_err(
        e.c.acquire(BOB, DESK, mins(30), true),
        ErrorKind::NotAllowed,
    );
    want_err(
        e.c.acquire(ALICE, DESK, mins(30), true),
        ErrorKind::NotAllowed,
    );
    let err =
        e.c.acquire(BOB, DESK, mins(30), true)
            .unwrap_err()
            .to_string();
    assert!(err.contains("can't be stolen"), "{err}");
    assert_eq!(e.c.leases()[0].holder, ALICE);
}

#[test]
fn naming_another_holder_releases_the_old_holders_lease() {
    let e = Env::new();
    desk_held_by(&e, ALICE);
    e.c.acquire(ALICE, DESK, mins(30), false).unwrap();
    desk_held_by(&e, BOB);
    assert!(e.c.leases().is_empty());
    let rel = e.rows(kinds::LEASE_RELEASED);
    assert_eq!(rel.len(), 1);
    let p: LeaseReleasedRow = payload(&rel[0]);
    assert_eq!(
        (p.resource.as_str(), p.holder.as_str(), p.reason.as_str()),
        (DESK, ALICE, REASON_DESK_HOLDER_CHANGED)
    );
    want_err(
        e.c.acquire(ALICE, DESK, mins(30), false),
        ErrorKind::NotAllowed,
    );
    e.c.acquire(BOB, DESK, mins(30), false).unwrap();
    // Saving the same holder again (another setting changed) keeps it.
    desk_held_by(&e, BOB);
    assert_eq!(e.c.leases()[0].holder, BOB);
    // Unheld: the lease goes, and nobody takes it.
    desk_held_by(&e, "");
    assert!(e.c.leases().is_empty());
    want_err(
        e.c.acquire(BOB, DESK, mins(30), false),
        ErrorKind::NotAllowed,
    );
}

#[test]
fn the_holder_going_releases_the_desk_lease() {
    let e = Env::new();
    desk_held_by(&e, ALICE);
    e.c.acquire(ALICE, DESK, MAX_TTL, false).unwrap();
    e.ros.set(ALICE, "gone");
    e.c.sweep().unwrap();
    e.clk.add(GONE_GRACE);
    e.c.sweep().unwrap();
    assert!(e.c.leases().is_empty());
    let rel = e.rows(kinds::LEASE_RELEASED);
    assert_eq!(rel.len(), 1);
    let p: LeaseReleasedRow = payload(&rel[0]);
    assert_eq!(
        (p.holder.as_str(), p.reason.as_str()),
        (ALICE, REASON_HOLDER_GONE)
    );
}

#[test]
fn a_run_is_recorded_under_the_lease_and_ends_with_it() {
    let e = Env::new();
    desk_held_by(&e, ALICE);
    // Not without the lease.
    want_err(
        e.c.record_desk_run(ALICE, REPO_A, 7, SHA, ""),
        ErrorKind::NotFound,
    );
    e.c.acquire(ALICE, DESK, mins(30), false).unwrap();
    e.c.record_desk_run(ALICE, REPO_A, 7, SHA, "").unwrap();
    // Another agent can't record one: it doesn't hold the lease.
    want_err(
        e.c.record_desk_run(BOB, REPO_A, 8, SHA, ""),
        ErrorKind::NotFound,
    );
    let run = e.c.desk(REPO_A).unwrap().run.unwrap();
    assert_eq!(
        (run.pr, run.head_sha.as_str(), run.by.as_str()),
        (7, SHA, ALICE)
    );
    // The base sha comes with step 2's renewal.
    e.c.record_desk_run(ALICE, REPO_A, 7, SHA, "abc123")
        .unwrap();
    assert_eq!(e.c.desk(REPO_A).unwrap().run.unwrap().base_sha, "abc123");
    // Merging ends with the lease, released or lapsed.
    e.c.release(ALICE, DESK).unwrap();
    assert!(e.c.desk(REPO_A).unwrap().run.is_none());
    e.c.acquire(ALICE, DESK, mins(1), false).unwrap();
    e.c.record_desk_run(ALICE, REPO_A, 9, SHA, "").unwrap();
    assert_eq!(e.c.desk(REPO_A).unwrap().run.unwrap().pr, 9);
    e.clk.add(mins(2));
    assert!(e.c.desk(REPO_A).unwrap().run.is_none());
}

#[test]
fn a_run_ends_when_the_person_names_another_holder() {
    let e = Env::new();
    desk_held_by(&e, ALICE);
    e.c.acquire(ALICE, DESK, mins(30), false).unwrap();
    e.c.record_desk_run(ALICE, REPO_A, 7, SHA, "").unwrap();
    desk_held_by(&e, BOB);
    assert!(e.c.desk(REPO_A).unwrap().run.is_none());
    // Bob's lease doesn't back Alice's old run.
    e.c.acquire(BOB, DESK, mins(30), false).unwrap();
    assert!(e.c.desk(REPO_A).unwrap().run.is_none());
    let c = e.reopen();
    assert!(c.desk(REPO_A).unwrap().run.is_none());
}
