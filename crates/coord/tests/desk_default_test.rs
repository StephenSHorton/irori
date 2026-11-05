//! Card #521 (docs/design/merge-desk.md §5, §7): every GitHub repo an
//! agent works in has a merge desk by itself. A `repo.seen` row for a
//! GitHub repo gives it default settings with no `repo.desk` row: on, held
//! by nobody (so nothing merges), polled from the device that saw the repo
//! last. The person's row turns it off, and a repo with no GitHub remote
//! gets no desk.

mod common;

use common::*;
use coord::*;
use serde_json::json;
use store::kinds;

fn seen(repo: &str) -> serde_json::Value {
    json!({"repoKey": repo, "root": "C:/src/b", "at": "2026-11-05T10:00:00Z"})
}

/// This device's capture writes repo.seen; Follow hands it to coord.
fn seen_here(e: &Env, repo: &str) {
    let ev = e.st.append("irori", kinds::REPO_SEEN, &seen(repo)).unwrap();
    e.c.handle(&ev).unwrap();
}

#[test]
fn rows_seen_at_the_same_moment_pick_the_same_device_in_any_order() {
    let at = hlc_at(plus(fixed_now(), mins(5)));
    let one = Env::new();
    one.remote("dev-b", 1, at, "irori", kinds::REPO_SEEN, &seen(REPO_A));
    one.remote("dev-c", 1, at, "irori", kinds::REPO_SEEN, &seen(REPO_A));
    let two = Env::new();
    two.remote("dev-c", 1, at, "irori", kinds::REPO_SEEN, &seen(REPO_A));
    two.remote("dev-b", 1, at, "irori", kinds::REPO_SEEN, &seen(REPO_A));
    let poll = |c: &Coord| c.desk(REPO_A).unwrap().settings.poll_device;
    assert_eq!(poll(&one.c), poll(&two.c));
    assert_eq!(poll(&one.c), poll(&one.reopen()), "and on replay");
}

#[test]
fn a_revoked_device_never_polls_a_desk() {
    let e = Env::new();
    let at = |m: u64| hlc_at(plus(fixed_now(), mins(m)));
    seen_here(&e, REPO_A);
    e.remote("dev-b", 1, at(60), "irori", kinds::REPO_SEEN, &seen(REPO_A));
    assert_eq!(e.c.desk(REPO_A).unwrap().settings.poll_device, "dev-b");
    let revoked: KeyFn = std::sync::Arc::new(|d: &str| d == "dev-b");
    let c = e.reopen_with(Options {
        revoked: Some(revoked),
        ..e.opts()
    });
    let d = c.desk(REPO_A).unwrap();
    assert_eq!(d.settings.poll_device, "dev-a", "back to the device left");
    assert_eq!(d.seen_on, "dev-a");
}

#[test]
fn a_resync_folds_this_devices_own_repo_seen_it_missed() {
    let e = Env::new();
    // Appended, but the live row never reached coord (a full subscription).
    e.st.append("irori", kinds::REPO_SEEN, &seen(REPO_A))
        .unwrap();
    assert!(e.c.desk(REPO_A).is_none());
    e.c.catch_up().unwrap();
    let d = e.c.desk(REPO_A).expect("the desk after the resync");
    assert!(d.on() && d.is_default());
    assert_eq!(d.settings.poll_device, "dev-a");
}

fn pr(n: i64) -> DeskPr {
    DeskPr {
        number: n,
        url: format!("https://github.com/a/b/pull/{n}"),
        title: format!("PR {n}"),
        ..DeskPr::default()
    }
}

#[test]
fn a_github_repo_an_agent_works_in_has_a_desk_with_no_desk_row() {
    let e = Env::new();
    seen_here(&e, REPO_A);
    assert!(e.rows(kinds::REPO_DESK).is_empty());
    let d = e.c.desk(REPO_A).expect("a default desk");
    assert!(d.on());
    assert!(d.is_default());
    assert_eq!(d.settings.holder, "", "nobody holds it, so nothing merges");
    assert_eq!(d.settings.poll_device, "dev-a");
    assert_eq!(d.settings.merge_method, MERGE_SQUASH);
    assert_eq!(d.seen_on, "dev-a");
    // It takes the poller's snapshot like any desk.
    assert!(
        e.c.record_desk_snapshot(REPO_A, vec![pr(3)], false, "you")
            .unwrap()
    );
    // And comes back the same on a restart (replay).
    let again = e.reopen();
    let d = again
        .desk(REPO_A)
        .expect("the default desk after a restart");
    assert!(d.on() && d.is_default());
    assert_eq!(d.settings.poll_device, "dev-a");
    assert_eq!(d.prs.unwrap().viewer, "you");
}

#[test]
fn the_desk_is_polled_from_the_device_that_saw_the_repo_last() {
    let e = Env::new();
    let at = |m: u64| hlc_at(plus(fixed_now(), mins(m)));
    e.remote("dev-b", 1, at(5), "irori", kinds::REPO_SEEN, &seen(REPO_A));
    assert_eq!(e.c.desk(REPO_A).unwrap().settings.poll_device, "dev-b");
    // A row seen earlier, arriving later, doesn't move it.
    e.remote("dev-c", 1, at(1), "irori", kinds::REPO_SEEN, &seen(REPO_A));
    assert_eq!(e.c.desk(REPO_A).unwrap().settings.poll_device, "dev-b");
    // A later one does.
    e.remote("dev-c", 2, at(9), "irori", kinds::REPO_SEEN, &seen(REPO_A));
    assert_eq!(e.c.desk(REPO_A).unwrap().settings.poll_device, "dev-c");
    // The same on replay, whatever order the rows are listed in.
    assert_eq!(
        e.reopen().desk(REPO_A).unwrap().settings.poll_device,
        "dev-c"
    );
}

#[test]
fn a_repo_with_no_github_remote_gets_no_desk() {
    let e = Env::new();
    seen_here(&e, REPO_B);
    seen_here(&e, "remote:gitlab.com/a/b");
    assert!(e.c.desks().is_empty());
    assert!(e.reopen().desks().is_empty());
    assert!(e.c.record_desk_prs(REPO_B, vec![pr(1)], false).is_err());
}

#[test]
fn turning_a_desk_off_keeps_it_off_and_stops_its_snapshots() {
    let e = Env::new();
    seen_here(&e, REPO_A);
    let mut off = e.c.desk(REPO_A).unwrap().settings;
    off.enabled = false;
    off.poll_device = String::new();
    e.c.set_desk(USER, off).unwrap();
    let d = e.c.desk(REPO_A).unwrap();
    assert!(!d.on());
    assert!(!d.is_default());
    // Seen again: still off.
    seen_here(&e, REPO_A);
    assert!(!e.c.desk(REPO_A).unwrap().on());
    assert!(!e.reopen().desk(REPO_A).unwrap().on());
    // The poller gets nothing to write for it.
    assert!(e.c.record_desk_prs(REPO_A, vec![pr(1)], false).is_err());
}

#[test]
fn a_blank_poll_device_follows_where_the_repo_was_seen_a_picked_one_stays() {
    let e = Env::new();
    let at = |m: u64| hlc_at(plus(fixed_now(), mins(m)));
    seen_here(&e, REPO_A);
    // The person names a holder and leaves the poll device blank.
    let mut s = e.c.desk(REPO_A).unwrap().settings;
    s.holder = ALICE.into();
    s.poll_device = String::new();
    e.c.set_desk(USER, s).unwrap();
    assert_eq!(e.c.desk(REPO_A).unwrap().settings.poll_device, "dev-a");
    e.remote("dev-b", 1, at(60), "irori", kinds::REPO_SEEN, &seen(REPO_A));
    let d = e.c.desk(REPO_A).unwrap();
    assert_eq!(d.settings.poll_device, "dev-b");
    assert_eq!(d.settings.holder, ALICE);
    // A device the person picks is never moved.
    let mut s = d.settings;
    s.poll_device = "dev-a".into();
    e.c.set_desk(USER, s).unwrap();
    e.remote("dev-b", 2, at(90), "irori", kinds::REPO_SEEN, &seen(REPO_A));
    assert_eq!(e.c.desk(REPO_A).unwrap().settings.poll_device, "dev-a");
}

#[test]
fn a_repo_no_device_has_seen_still_needs_a_poll_device() {
    let e = Env::new();
    let mut d = new_desk(REPO_A, "");
    d.poll_device = String::new();
    want_err(e.c.set_desk(USER, d), ErrorKind::Invalid);
}

#[test]
fn the_snapshot_keeps_the_persons_login() {
    let e = Env::new();
    seen_here(&e, REPO_A);
    assert!(
        e.c.record_desk_snapshot(REPO_A, vec![pr(1)], false, "you")
            .unwrap()
    );
    // Same list, same login: nothing new.
    assert!(
        !e.c.record_desk_snapshot(REPO_A, vec![pr(1)], false, "you")
            .unwrap()
    );
    // A poll that couldn't learn the login keeps the one heard before.
    assert!(!e.c.record_desk_prs(REPO_A, vec![pr(1)], false).unwrap());
    assert_eq!(e.c.desk(REPO_A).unwrap().prs.unwrap().viewer, "you");
    // A new login is a change.
    assert!(
        e.c.record_desk_snapshot(REPO_A, vec![pr(1)], false, "someone-else")
            .unwrap()
    );
}
