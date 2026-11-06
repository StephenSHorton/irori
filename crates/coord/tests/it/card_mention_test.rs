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

fn mentioned(c: &Coord, id: &str) -> Vec<String> {
    c.activity(id)
        .unwrap()
        .into_iter()
        .filter(|l| l.what.starts_with("mentioned this in"))
        .map(|l| l.what)
        .collect()
}

fn on(board: &str, title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        board: board.into(),
        ..NewTask::default()
    }
}

/// Personal's rule (card #558): a bare number means the default board first,
/// as `task_ref` and the window's chip read it.
fn personal() -> Env {
    let mut e = Env::new();
    e.c = Coord::new(
        e.st.clone(),
        None,
        Options {
            board_tag: Some(std::sync::Arc::new(|id: &str| {
                (id == "b-home").then(|| "home".to_string())
            })),
            board_tags: Some(std::sync::Arc::new(|| vec!["home".to_string()])),
            ..e.opts()
        },
    )
    .unwrap();
    e
}

#[test]
fn a_bare_number_means_the_default_boards_card() {
    let e = personal();
    let d = e.c.create_task(BOB, on("", "default")).unwrap();
    let h = e.c.create_task(BOB, on("b-home", "home")).unwrap();
    let from = e.c.create_task(BOB, on("", "commented on")).unwrap();
    e.c.add_comment(ALICE, &from.id, "see #1", false).unwrap();
    assert_eq!(mentioned(&e.c, &d.id).len(), 1);
    assert!(mentioned(&e.c, &h.id).is_empty());
}

/// `#1` on card #1 names that card, so no line, and no other board's #1
/// takes the mention for it.
#[test]
fn a_comment_naming_its_own_card_mentions_nothing() {
    let e = personal();
    let d = e.c.create_task(BOB, on("", "default")).unwrap();
    let h = e.c.create_task(BOB, on("b-home", "home")).unwrap();
    e.c.add_comment(ALICE, &d.id, "this is #1", false).unwrap();
    assert!(mentioned(&e.c, &d.id).is_empty());
    assert!(mentioned(&e.c, &h.id).is_empty());
}

/// A tagged ref is its board's card, from any other card.
#[test]
fn a_tagged_mention_reaches_its_board() {
    let e = personal();
    let d = e.c.create_task(BOB, on("", "default")).unwrap();
    let h = e.c.create_task(BOB, on("b-home", "home")).unwrap();
    e.c.add_comment(ALICE, &d.id, "see home#1", false).unwrap();
    assert_eq!(mentioned(&e.c, &h.id).len(), 1);
}

/// A deleted default card keeps its number here too: no line on `home#1`.
#[test]
fn a_deleted_default_card_keeps_its_mentions_off_another_board() {
    let e = Env::with("dev-a", Some(std::sync::Arc::new(|k: &str| k == USER)));
    let c = Coord::new(
        e.st.clone(),
        None,
        Options {
            board_tag: Some(std::sync::Arc::new(|id: &str| {
                (id == "b-home").then(|| "home".to_string())
            })),
            board_tags: Some(std::sync::Arc::new(|| vec!["home".to_string()])),
            ..e.opts()
        },
    )
    .unwrap();
    let d = c.create_task(BOB, on("", "default")).unwrap();
    let h = c.create_task(BOB, on("b-home", "home")).unwrap();
    let from = c.create_task(BOB, on("", "commented on")).unwrap();
    c.delete_task(USER, &d.id).unwrap();
    c.add_comment(ALICE, &from.id, "see #1", false).unwrap();
    assert!(mentioned(&c, &h.id).is_empty());
}
