//! Card #436: a test that fails on clean main gets one card. On Nov 3 five
//! agents each found attach_exe red on main, each raised its own blocker
//! and spent an hour on it; now the second report joins the first card,
//! and every reporter wakes when that card completes.

use crate::common;

use common::*;
use coord::*;

fn held(e: &Env, who: &str, title: &str) -> Task {
    let t =
        e.c.create_task(
            who,
            NewTask {
                title: title.into(),
                repo_key: REPO_A.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(who, &t.id).unwrap();
    t
}

fn wait_on(e: &Env, who: &str, id: &str, red: &Task) -> Task {
    e.c.block(
        who,
        id,
        "attach_exe fails on clean main",
        Some(BlockedBy {
            kind: BLOCKED_ON_TASK.into(),
            reference: red.id.clone(),
        }),
    )
    .unwrap()
}

#[test]
fn a_second_report_of_the_same_red_test_joins_the_first_card() {
    let e = Env::new();
    let a = held(&e, ALICE, "Talk 3");
    let b = held(&e, BOB, "Merge desk slice 2");

    let (red, made) =
        e.c.main_red(ALICE, &a.id, "attach_exe", "3573cf11")
            .unwrap();
    assert!(made);
    assert_eq!(red.title, "attach_exe fails on clean main");
    assert_eq!(red.scope, ["main-red/attach_exe"]);
    assert_eq!(main_red_test(&red), Some("attach_exe"));
    assert_eq!(red.repo_key, REPO_A);
    assert!(red.holders.is_empty(), "it goes out to a free agent");
    assert!(red.objective.contains("3573cf11"), "{}", red.objective);
    wait_on(&e, ALICE, &a.id, &red);

    // Bob hits the same test: no second card.
    let (again, made) = e.c.main_red(BOB, &b.id, "attach_exe", "").unwrap();
    assert!(!made);
    assert_eq!(again.id, red.id);
    wait_on(&e, BOB, &b.id, &red);
    let reds: Vec<Task> =
        e.c.list_tasks(&TaskFilter::default())
            .into_iter()
            .filter(|t| main_red_test(t).is_some())
            .collect();
    assert_eq!(reds.len(), 1, "{reds:?}");

    // A different test is a different card.
    let (other, made) =
        e.c.main_red(BOB, &b.id, "events_stream_replay_modes", "")
            .unwrap();
    assert!(made);
    assert_ne!(other.id, red.id);

    // The red card itself can't wait on itself.
    let err = e.c.main_red(CAROL, &red.id, "attach_exe", "").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Invalid);

    // Carol fixes it: both reporters wake.
    e.c.claim_task(CAROL, &red.id).unwrap();
    e.c.set_task_state(CAROL, &red.id, COMPLETED, "fixed on main")
        .unwrap();
    let ns = e.c.sweep_notices().unwrap();
    let mut woke: Vec<&str> = ns
        .iter()
        .filter(|n| n.kind == NOTICE_UNBLOCKED)
        .flat_map(|n| n.to.iter().map(String::as_str))
        .collect();
    woke.sort();
    let mut want = vec![ALICE, BOB];
    want.sort();
    assert_eq!(woke, want, "{ns:?}");
    assert_eq!(e.c.get_task(&a.id).unwrap().state, WORKING);
    assert_eq!(e.c.get_task(&b.id).unwrap().state, WORKING);

    // Main goes red again for it later: a new card, the old one is done.
    let (next, made) = e.c.main_red(ALICE, &a.id, "attach_exe", "").unwrap();
    assert!(made);
    assert_ne!(next.id, red.id);
}

#[test]
fn test_names_with_colons_become_a_scope_glob() {
    assert_eq!(
        main_red_scope("tests::attach_exe"),
        "main-red/tests/attach_exe"
    );
    assert_eq!(main_red_scope("a b"), "main-red/a/b");
    assert_eq!(
        parse_main_red("  attach_exe  3573cf11 ").unwrap(),
        ("attach_exe".to_string(), "3573cf11".to_string())
    );
    assert!(parse_main_red("  ").is_err());
}
