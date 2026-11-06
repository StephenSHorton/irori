//! Card mentions (card #114): a comment naming another card by its short
//! ref puts a "mentioned this in #N" line on that card, whoever wrote it.

use crate::common;

use common::*;
use coord::*;

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

#[test]
fn a_comment_naming_a_card_puts_a_line_on_it() {
    let e = Env::new();
    let a = e.c.create_task(BOB, nt("Invite join fails")).unwrap();
    let b = e.c.create_task(BOB, nt("Subtask layout")).unwrap();
    let c = e.c.create_task(BOB, nt("Unrelated")).unwrap();
    let (sa, sb) = (a.short_ref(), b.short_ref());
    e.c.add_comment(
        ALICE,
        &a.id,
        &format!("see {sb}, and ({sb}) again; not x{sb} or {sb}0; own {sa}"),
        false,
    )
    .unwrap();
    let lines: Vec<_> =
        e.c.activity(&b.id)
            .unwrap()
            .into_iter()
            .filter(|l| l.what.starts_with("mentioned this in"))
            .collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].what, format!("mentioned this in {sa}"));
    assert_eq!(lines[0].by, ALICE);
    assert_eq!(lines[0].kind, "task.mentioned");
    // Its own card and an unnamed card get no line.
    for id in [&a.id, &c.id] {
        assert!(
            !e.c.activity(id)
                .unwrap()
                .iter()
                .any(|l| l.what.starts_with("mentioned this in")),
            "{id}"
        );
    }
}
