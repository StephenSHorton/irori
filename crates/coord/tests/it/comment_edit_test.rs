//! The person edits their comment before any agent read it (card #105).

use crate::common;

use common::*;
use coord::*;
use store::kinds;

#[test]
fn test_edit_comment_folds_and_replays() {
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
    let (cm, _) =
        e.c.person_comment(USER, &t.id, "use the red one", false, &[], None)
            .unwrap();
    let ed =
        e.c.edit_comment(USER, &cm.id, "  use the blue one ")
            .unwrap();
    assert_eq!(ed.text, "use the blue one");
    assert!(!ed.edited_at.is_zero());
    // Same text again: no row.
    e.c.edit_comment(USER, &cm.id, "use the blue one").unwrap();
    assert_eq!(e.rows(kinds::TASK_COMMENT_EDITED).len(), 1);
    let cs = e.c.comments(&t.id).unwrap();
    assert_eq!(
        cs.iter().filter(|c| c.id == cm.id).count(),
        1,
        "no new comment"
    );
    assert_eq!(e.c.get_comment(&cm.id).unwrap().text, "use the blue one");

    let c = e.reopen();
    assert_eq!(c.get_comment(&cm.id).unwrap().text, "use the blue one");

    // Only the person, only their own comment, never empty.
    want_err(
        e.c.edit_comment(ALICE, &cm.id, "mine now"),
        ErrorKind::NotAllowed,
    );
    want_err(e.c.edit_comment(USER, &cm.id, "  "), ErrorKind::Invalid);
    want_err(e.c.edit_comment(USER, "01NOPE", "x"), ErrorKind::NotFound);
    let (ac, _) =
        e.c.add_comment_with(ALICE, &t.id, "on it", false, vec![])
            .unwrap();
    want_err(
        e.c.edit_comment(USER, &ac.id, "not yours"),
        ErrorKind::NotAllowed,
    );
}
