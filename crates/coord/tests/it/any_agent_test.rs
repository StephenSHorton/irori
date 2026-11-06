//! Card #320: any agent may change any card, and its holders hear who
//! changed what.

use crate::common;

use common::*;
use coord::rows::{TaskEditedRow, TaskStateRow, TaskStepsRow};
use coord::*;
use store::kinds;

fn held(e: &Env) -> Task {
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "ship it".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap()
}

fn step(text: &str) -> Step {
    Step {
        text: text.into(),
        ..Step::default()
    }
}

fn watched(e: &Env) -> Vec<Notice> {
    e.c.sweep_notices()
        .unwrap()
        .into_iter()
        .filter(|n| n.kind == NOTICE_WATCHED)
        .collect()
}

/// The one digest to ALICE, the holder.
fn to_holder(ns: &[Notice]) -> &Notice {
    let mine: Vec<&Notice> = ns
        .iter()
        .filter(|n| n.to.contains(&ALICE.to_string()))
        .collect();
    assert_eq!(mine.len(), 1, "{ns:?}");
    mine[0]
}

#[test]
fn test_non_holder_changes_any_card() {
    let e = Env::new();
    let t = held(&e);
    let (_, b) =
        e.c.raise(
            BOB,
            &t.id,
            NewBlocker {
                question: "Which db?".into(),
                ..NewBlocker::default()
            },
        )
        .unwrap();
    assert_eq!(b.raised_by, BOB);
    let other =
        e.c.create_task(
            CAROL,
            NewTask {
                title: "the schema".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.block(
        BOB,
        &t.id,
        "waits on the schema",
        Some(BlockedBy {
            kind: BLOCKED_ON_TASK.into(),
            reference: other.id.clone(),
        }),
    )
    .unwrap();
    e.c.set_steps(BOB, &t.id, vec![step("a"), step("b")])
        .unwrap();
    e.c.set_step(
        BOB,
        &t.id,
        "s1",
        StepUpdate {
            done: Some(true),
            doing: None,
        },
    )
    .unwrap();
    e.c.edit_task(
        BOB,
        &t.id,
        TaskEdit {
            title: Some("ship it today".into()),
            objective: Some("the release".into()),
            done_when: Some("tagged".into()),
            ..TaskEdit::default()
        },
    )
    .unwrap();
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.holders, vec![ALICE.to_string()], "holders don't change");
    assert_eq!(got.title, "ship it today");
    assert_eq!(
        (got.objective.as_str(), got.done_criteria.as_str()),
        ("the release", "tagged")
    );
    assert_eq!(got.steps.len(), 2);
    assert!(got.steps[0].done);
    assert_eq!(got.open_blockers().len(), 2);
    // The thread says who did it.
    let acts = e.c.activity(&t.id).unwrap();
    for w in ["raised a blocker", "set the steps", "ticked", "renamed it"] {
        assert!(
            acts.iter().any(|a| a.by == BOB && a.what.contains(w)),
            "{w}: {acts:?}"
        );
    }
    // Every fold of the log agrees.
    let again = e.reopen().get_task(&t.id).unwrap();
    assert_eq!(
        (
            again.title.clone(),
            again.steps.len(),
            again.open_blockers().len()
        ),
        (got.title, 2, 2)
    );
}

#[test]
fn test_holder_hears_who_changed_what() {
    let e = Env::new();
    let t = held(&e);
    watched(&e);
    // A blocker from another agent wakes the holder.
    e.c.raise(
        BOB,
        &t.id,
        NewBlocker {
            question: "Which db?".into(),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    let ns = watched(&e);
    let n = to_holder(&ns);
    assert!(n.wake, "{n:?}");
    assert!(
        n.text
            .contains(&format!("{BOB} raised a blocker: Which db?")),
        "{}",
        n.text
    );
    assert!(n.text.contains("a card you hold"), "{}", n.text);
    // Steps and an edit: next turn, no wake.
    e.c.set_steps(BOB, &t.id, vec![step("a")]).unwrap();
    e.c.edit_task(
        BOB,
        &t.id,
        TaskEdit {
            title: Some("ship it today".into()),
            ..TaskEdit::default()
        },
    )
    .unwrap();
    let ns = watched(&e);
    let n = to_holder(&ns);
    assert!(!n.wake, "{n:?}");
    assert!(
        n.text.contains(&format!("{BOB} set the steps")),
        "{}",
        n.text
    );
    assert!(n.text.contains(&format!("{BOB} renamed it")), "{}", n.text);
    // A state change wakes.
    e.c.set_task_state(BOB, &t.id, WORKING, "").unwrap();
    let ns = watched(&e);
    assert!(to_holder(&ns).wake);
    // The holder's own changes, and the person's, tell nobody new.
    e.c.set_steps(ALICE, &t.id, vec![step("b")]).unwrap();
    e.c.edit_task(
        USER,
        &t.id,
        TaskEdit {
            title: Some("by the person".into()),
            ..TaskEdit::default()
        },
    )
    .unwrap();
    assert!(watched(&e).is_empty());
}

#[test]
fn test_another_devices_rows_fold_and_tell_the_holder() {
    let e = Env::new();
    let t = held(&e);
    watched(&e);
    let zed = "dev-b/zed";
    let actor = format!("agent:{zed}");
    let at = |m: u64| hlc_at(plus(fixed_now(), mins(m)));
    e.remote(
        "dev-b",
        1,
        at(1),
        &actor,
        kinds::TASK_EDITED,
        &TaskEditedRow {
            task_id: t.id.clone(),
            title: Some("renamed on b".into()),
            by: zed.into(),
            at: fixed_now(),
            ..TaskEditedRow::default()
        },
    );
    e.remote(
        "dev-b",
        2,
        at(2),
        &actor,
        kinds::TASK_STEPS,
        &TaskStepsRow {
            task_id: t.id.clone(),
            steps: vec![Step {
                id: "s1".into(),
                text: "from b".into(),
                ..Step::default()
            }],
            by: zed.into(),
            at: fixed_now(),
        },
    );
    e.remote(
        "dev-b",
        3,
        at(3),
        &actor,
        kinds::TASK_STATUS,
        &TaskStateRow {
            task_id: t.id.clone(),
            state: FAILED.into(),
            note: "broke on b".into(),
            by: zed.into(),
            at: fixed_now(),
            ..TaskStateRow::default()
        },
    );
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(
        (got.title.as_str(), got.steps.len(), got.state.as_str()),
        ("renamed on b", 1, FAILED)
    );
    let ns = watched(&e);
    let n = to_holder(&ns);
    assert!(n.wake, "the state change wakes: {n:?}");
    for w in ["renamed it", "set the steps", "set it to failed"] {
        assert!(n.text.contains(w), "{w}: {}", n.text);
    }
    // Replay folds the same rows the same way, and tells nobody again.
    let c2 = e.reopen();
    let again = c2.get_task(&t.id).unwrap();
    assert_eq!(
        (
            again.title.as_str(),
            again.steps.len(),
            again.state.as_str()
        ),
        ("renamed on b", 1, FAILED)
    );
    assert!(
        c2.sweep_notices()
            .unwrap()
            .iter()
            .all(|n| n.kind != NOTICE_WATCHED)
    );
}

#[test]
fn test_non_holder_reopens_and_the_holder_hears_once() {
    let e = Env::new();
    let t = held(&e);
    e.c.set_task_state(USER, &t.id, COMPLETED, "done").unwrap();
    watched(&e);
    let (got, n) = e.c.reopen(BOB, &t.id).unwrap();
    assert!(got.state == WORKING && got.holders == [ALICE]);
    assert!(n.kind == NOTICE_REOPENED && n.to == [ALICE], "{n:?}");
    // The reopened notice says it; no digest repeats it.
    assert!(
        watched(&e)
            .iter()
            .all(|w| !w.to.contains(&ALICE.to_string()))
    );
}
