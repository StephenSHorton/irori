//! The person deletes a card (card #57): task.deleted takes it off the
//! Board on every device, any card in any state, and the log keeps it.
//! New in Rust: no Go original.

mod common;

use common::*;
use coord::rows::{TaskCreatedRow, TaskDeletedRow};
use coord::*;
use store::kinds;

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

#[test]
fn the_person_deletes_any_card() {
    let e = Env::new();
    let t = e.c.create_task(ALICE, nt("opened by mistake")).unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
    let keep = e.c.create_task(ALICE, nt("real work")).unwrap();
    // Agents can't; the person can, while an agent holds it.
    want_err(e.c.delete_task(ALICE, &t.id), ErrorKind::NotAllowed);
    let gone = e.c.delete_task(USER, &t.id).unwrap();
    assert_eq!(gone.id, t.id);
    assert!(e.c.get_task(&t.id).is_none());
    assert_eq!(
        ids(&e.c.list_tasks(&TaskFilter::default())),
        std::slice::from_ref(&keep.id)
    );
    // Its ref and id are gone for agents too, and nothing more lands on it.
    want_err(e.c.task_ref("#1"), ErrorKind::NotFound);
    want_err(
        e.c.comment_on(
            ALICE,
            &t.id,
            CommentInput {
                text: "still here?".into(),
                ..CommentInput::default()
            },
        ),
        ErrorKind::NotFound,
    );
    want_err(e.c.delete_task(USER, &t.id), ErrorKind::NotFound);
    // The log keeps the card and says who deleted it.
    assert_eq!(e.rows(kinds::TASK_CREATED).len(), 2);
    let rows = e.rows(kinds::TASK_DELETED);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].actor, "user");
    assert_eq!(payload::<TaskDeletedRow>(&rows[0]).task_id, t.id);
    // A replay agrees.
    let c2 = e.reopen();
    assert!(c2.get_task(&t.id).is_none());
    assert!(c2.get_task(&keep.id).is_some());
}

#[test]
fn a_deleted_parent_leaves_its_subtasks() {
    let e = Env::new();
    let p = e.c.create_task(ALICE, nt("parent")).unwrap();
    let k =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "child".into(),
                parent: p.id.clone(),
                ..NewTask::default()
            },
        )
        .unwrap();
    assert_eq!(e.c.get_task(&k.id).unwrap().parent, p.id);
    e.c.delete_task(USER, &p.id).unwrap();
    let k = e.c.get_task(&k.id).unwrap();
    assert!(k.parent.is_empty(), "{k:?}");
    // Deleting a subtask takes it out of its parent.
    let p2 = e.c.create_task(ALICE, nt("parent 2")).unwrap();
    let k2 =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "child 2".into(),
                parent: p2.id.clone(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.delete_task(USER, &k2.id).unwrap();
    assert!(e.c.get_task(&p2.id).unwrap().subtasks.is_empty());
}

#[test]
fn only_the_persons_delete_counts_in_any_order() {
    let e = Env::new();
    let t = e.c.create_task(ALICE, nt("keep me")).unwrap();
    // An agent-actor row, or one whose by isn't the person, is ignored.
    e.write_raw(
        &format!("agent:{ALICE}"),
        kinds::TASK_DELETED,
        &TaskDeletedRow {
            task_id: t.id.clone(),
            by: USER.into(),
            at: fixed_now(),
        },
    );
    e.write_raw(
        "user",
        kinds::TASK_DELETED,
        &TaskDeletedRow {
            task_id: t.id.clone(),
            by: ALICE.into(),
            at: fixed_now(),
        },
    );
    assert!(e.c.get_task(&t.id).is_some());
    // Another device's delete that arrives before the card it deletes:
    // the card never shows.
    let id = "01JTASKFROMDEVB0000000000";
    e.remote(
        "dev-c",
        1,
        hlc_at(plus(fixed_now(), mins(2))),
        "user",
        kinds::TASK_DELETED,
        &TaskDeletedRow {
            task_id: id.into(),
            by: USER.into(),
            at: plus(fixed_now(), mins(2)),
        },
    );
    e.remote(
        "dev-b",
        1,
        hlc_at(plus(fixed_now(), mins(1))),
        &format!("agent:{BOB}"),
        kinds::TASK_CREATED,
        &TaskCreatedRow {
            task_id: id.into(),
            title: "from the other device".into(),
            by: BOB.into(),
            at: plus(fixed_now(), mins(1)),
            ..TaskCreatedRow::default()
        },
    );
    assert!(e.c.get_task(id).is_none());
}
