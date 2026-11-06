//! Card positions on the Board's canvas (card #93).

use crate::common;

use common::*;
use coord::*;
use store::kinds;

#[test]
fn test_place_keeps_the_newest_and_replays() {
    let e = Env::new();
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "ship".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    let before = e.c.get_task(&t.id).unwrap();
    e.c.place(USER, &t.id, 120, -40).unwrap();
    e.c.place(USER, &t.id, 300, 80).unwrap();
    e.c.place(USER, &t.id, 300, 80).unwrap(); // same spot: no row
    assert_eq!(e.rows(kinds::TASK_PLACED).len(), 2);
    assert_eq!(e.c.positions().get(&t.id), Some(&(300, 80)));
    // Not activity: the card's updated time and log stay as they were.
    let after = e.c.get_task(&t.id).unwrap();
    assert_eq!(after.updated_at, before.updated_at);
    assert!(e.c.sweep_notices().unwrap().is_empty());

    let c = e.reopen();
    assert_eq!(c.positions().get(&t.id), Some(&(300, 80)));

    want_err(e.c.place(USER, "01NOPE", 0, 0), ErrorKind::NotFound);
    want_err(e.c.place(USER, &t.id, 0, MAX_PLACE + 1), ErrorKind::Invalid);
    e.c.place(ALICE, &t.id, 1, 2).unwrap(); // agents may arrange too

    e.c.delete_task(USER, &t.id).unwrap();
    assert!(e.c.positions().is_empty());
}
