//! Card #415: the merge desk's rows fold the same on replay and from
//! another device, and only the person's (or Irori's) count.

use crate::common;

use common::*;
use coord::*;
use store::kinds;

fn pr(n: i64) -> DeskPr {
    DeskPr {
        number: n,
        url: format!("https://github.com/a/b/pull/{n}"),
        title: format!("PR {n}"),
        ..DeskPr::default()
    }
}

#[test]
fn a_desk_and_its_snapshot_survive_a_restart() {
    let e = Env::new();
    e.c.set_desk(USER, new_desk(REPO_A, "dev-a")).unwrap();
    assert!(
        e.c.record_desk_prs(REPO_A, vec![pr(2), pr(1)], false)
            .unwrap()
    );
    let again = e.reopen();
    let d = again.desk(REPO_A).expect("the desk after a restart");
    assert!(d.on());
    assert_eq!(d.settings.poll_device, "dev-a");
    let s = d.prs.expect("the snapshot after a restart");
    assert_eq!(s.prs.iter().map(|p| p.number).collect::<Vec<_>>(), [1, 2]);
    // Unchanged after the restart: still nothing new to write.
    assert!(
        !again
            .record_desk_prs(REPO_A, vec![pr(1), pr(2)], false)
            .unwrap()
    );
}

#[test]
fn another_devices_desk_rows_fold_and_an_agents_are_ignored() {
    // The person's own devices: user:dev-a and user:dev-b.
    let owner: KeyFn = std::sync::Arc::new(|k: &str| k.starts_with("user:"));
    let e = Env::with("dev-a", Some(owner));
    let at = |m: u64| hlc_at(plus(fixed_now(), mins(m)));
    let mut d = new_desk(REPO_A, "dev-b");
    d.by = "user:dev-b".into();
    d.at = fixed_now();
    e.remote("dev-b", 1, at(1), "user", kinds::REPO_DESK, &d);
    assert_eq!(e.c.desk(REPO_A).unwrap().settings.poll_device, "dev-b");
    // The snapshot dev-b's poller wrote.
    let snap = DeskPrsRow {
        repo_key: REPO_A.into(),
        prs: vec![pr(7)],
        by: "irori".into(),
        at: fixed_now(),
        ..DeskPrsRow::default()
    };
    e.remote("dev-b", 2, at(2), "irori", kinds::DESK_PRS, &snap);
    assert_eq!(e.c.desk(REPO_A).unwrap().prs.unwrap().prs, vec![pr(7)]);
    // An agent's repo.desk (naming its own device to poll) changes nothing.
    let mut bad = new_desk(REPO_A, "dev-evil");
    bad.by = "dev-b/zed".into();
    bad.at = fixed_now();
    e.remote("dev-b", 3, at(3), "agent:dev-b/zed", kinds::REPO_DESK, &bad);
    assert_eq!(e.c.desk(REPO_A).unwrap().settings.poll_device, "dev-b");
    // Nor does a snapshot that isn't Irori's.
    let mut fake = snap.clone();
    fake.prs = Vec::new();
    fake.by = "dev-b/zed".into();
    e.remote("dev-b", 4, at(4), "agent:dev-b/zed", kinds::DESK_PRS, &fake);
    assert_eq!(e.c.desk(REPO_A).unwrap().prs.unwrap().prs.len(), 1);
}

#[test]
fn an_agent_cant_set_up_a_desk() {
    let e = Env::new();
    want_err(
        e.c.set_desk(ALICE, new_desk(REPO_A, "dev-a")),
        ErrorKind::NotAllowed,
    );
    assert!(e.c.desk(REPO_A).is_none());
    // No snapshot for a repo without a desk.
    assert!(e.c.record_desk_prs(REPO_A, vec![pr(1)], false).is_err());
}

fn desk_changed(repo: &str) -> Change {
    Change {
        kind: DESK_CHANGED.into(),
        resource: repo.into(),
        ..Change::default()
    }
}

/// Card #1144 (list-signals.md §3.2): the person's desk row and Irori's PR
/// snapshot each note a `desk` change naming the canonical repo key and no
/// card; a snapshot that writes nothing (the same list) notes nothing, and
/// neither does a `desk.ok` row (nothing shows it).
#[test]
fn desk_rows_note_a_desk_change() {
    let e = Env::new();
    let k = canonical_repo_key(REPO_A);
    let q = e.c.subscribe();
    e.c.set_desk(USER, new_desk(REPO_A, "dev-a")).unwrap();
    assert_eq!(drain_changes(&q), vec![desk_changed(&k)]);
    assert!(e.c.record_desk_prs(REPO_A, vec![pr(1)], false).unwrap());
    assert_eq!(drain_changes(&q), vec![desk_changed(&k)]);
    assert!(!e.c.record_desk_prs(REPO_A, vec![pr(1)], false).unwrap());
    assert!(drain_changes(&q).is_empty(), "nothing written");
    e.write_raw(
        "user",
        kinds::DESK_OK,
        &DeskOkRow {
            repo_key: REPO_A.into(),
            pr: 1,
            head_sha: "abc".into(),
            by: USER.into(),
            at: fixed_now(),
        },
    );
    assert!(drain_changes(&q).is_empty(), "desk.ok shows nowhere");
}

/// Card #1144 (list-signals.md §3.2): another device's `repo.seen` notes
/// the desk when it changes what the desk shows (a new desk, or the device
/// it was seen on last), and not when it only moves that device's time.
#[test]
fn repo_seen_notes_the_desk_only_when_it_shows_a_change() {
    let e = Env::new();
    let k = canonical_repo_key(REPO_A);
    let at = |m: u64| hlc_at(plus(fixed_now(), mins(m)));
    let seen =
        serde_json::json!({"repoKey": REPO_A, "root": "C:/src/b", "at": "2026-11-05T10:00:00Z"});
    let q = e.c.subscribe();
    e.remote("dev-b", 1, at(1), "irori", kinds::REPO_SEEN, &seen);
    assert_eq!(drain_changes(&q), vec![desk_changed(&k)], "a new desk");
    e.remote("dev-b", 2, at(2), "irori", kinds::REPO_SEEN, &seen);
    assert!(drain_changes(&q).is_empty(), "same device, later");
    e.remote("dev-c", 1, at(3), "irori", kinds::REPO_SEEN, &seen);
    assert_eq!(
        drain_changes(&q),
        vec![desk_changed(&k)],
        "seen on dev-c now"
    );
    assert_eq!(e.c.desk(REPO_A).unwrap().seen_on, "dev-c");
}

/// Card #1144 (list-signals.md §3.2): the Core saying which repos are the
/// person's notes a `desk` change for every desk (empty repo key) when
/// what it judged moved, and nothing when it said the same again.
#[test]
fn judging_the_desk_repos_notes_every_desk() {
    let e = Env::new();
    e.c.set_desk(USER, new_desk(REPO_A, "dev-a")).unwrap();
    let q = e.c.subscribe();
    assert!(e.c.set_desk_repos(Vec::<String>::new()));
    assert_eq!(drain_changes(&q), vec![desk_changed("")]);
    assert!(e.c.desk(REPO_A).unwrap().stray);
    assert!(!e.c.set_desk_repos(Vec::<String>::new()));
    assert!(drain_changes(&q).is_empty(), "no change");
    assert!(e.c.set_desk_repos([REPO_A.to_string()]));
    assert_eq!(drain_changes(&q), vec![desk_changed("")]);
    assert!(!e.c.desk(REPO_A).unwrap().stray);
}
