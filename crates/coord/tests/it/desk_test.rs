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
