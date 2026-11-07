//! Card #640: a space nobody else is in moves into Personal as a board,
//! every card with it, and leaves every device of the person.

use crate::common;

use common::pair::paired;
use common::{Rig, wait_for};
use coord::ErrorKind as CK;
use irori_core::{BoardError, Core, MOVED_FIELD, MoveOpts};
use store::{Filter, kinds};

/// What the move must keep of a card.
#[derive(Debug, PartialEq)]
struct Facts {
    short: String,
    title: String,
    state: String,
    comments: i64,
    steps: Vec<(String, bool)>,
    blockers: usize,
    open_blockers: usize,
    subtasks: usize,
    parent: String,
}

fn facts(t: &coord::Task) -> Facts {
    Facts {
        short: t.short_ref(),
        title: t.title.clone(),
        state: t.state.clone(),
        comments: t.comments,
        steps: t.steps.iter().map(|s| (s.text.clone(), s.done)).collect(),
        blockers: t.blockers.len(),
        open_blockers: t.blockers.iter().filter(|b| b.open()).count(),
        subtasks: t.subtasks.len(),
        parent: t.parent.clone(),
    }
}

fn step(text: &str) -> coord::Step {
    coord::Step {
        text: text.into(),
        ..Default::default()
    }
}

/// A space `wind` with three cards: a parent worked by an agent (steps,
/// one ticked, comments by the agent and the person, a blocker the person
/// answered, then done), its subtask, and one left open. Returns the
/// space id and the cards' ids.
fn wind(c: &Core, person: &str, agent: &str) -> (String, Vec<String>) {
    let id = c.create_board("Wind Waker", "wind").unwrap();
    let co = c.board_coord(&id).unwrap();
    let me = c.board_key(&id, person).unwrap();
    let ag = c.board_key(&id, agent).unwrap();
    let parent = co
        .create_task(
            &me,
            coord::NewTask {
                title: "Sea chart".into(),
                objective: "Show where players are".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let sub = co
        .create_task(
            &me,
            coord::NewTask {
                title: "Draw the islands".into(),
                parent: parent.id.clone(),
                ..Default::default()
            },
        )
        .unwrap();
    let open = co
        .create_task(
            &ag,
            coord::NewTask {
                title: "Later idea".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.claim_task(&ag, &sub.id).unwrap();
    co.set_steps(&ag, &sub.id, vec![step("map"), step("dots")])
        .unwrap();
    co.set_step(
        &ag,
        &sub.id,
        "s1",
        coord::StepUpdate {
            done: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    for (by, text) in [(&ag, "started"), (&me, "looks good")] {
        co.comment_on(
            by,
            &sub.id,
            coord::CommentInput {
                text: text.into(),
                ..Default::default()
            },
        )
        .unwrap();
    }
    let (_, b) = co
        .raise(
            &ag,
            &sub.id,
            coord::NewBlocker {
                question: "Which colour?".into(),
                options: vec![
                    coord::BlockerOption {
                        answer: "Blue".into(),
                        recommended: true,
                        ..Default::default()
                    },
                    coord::BlockerOption {
                        answer: "Red".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        )
        .unwrap();
    co.answer(
        &me,
        &sub.id,
        &b.id,
        coord::BlockerAnswer {
            pick: 1,
            resolve: true,
            ..Default::default()
        },
    )
    .unwrap();
    co.set_task_state(&ag, &sub.id, "completed", "drawn")
        .unwrap();
    (id, vec![parent.id, sub.id, open.id])
}

fn space_facts(c: &Core, space: &str, ids: &[String]) -> Vec<Facts> {
    let co = c.board_coord(space).unwrap();
    ids.iter()
        .map(|id| facts(&co.get_task(id).unwrap()))
        .collect()
}

fn personal_facts(c: &Core, ids: &[String]) -> Vec<Facts> {
    ids.iter()
        .map(|id| facts(&c.coord().get_task(id).unwrap()))
        .collect()
}

fn personal_tags(c: &Core) -> Vec<String> {
    c.personal_boards()
        .unwrap()
        .into_iter()
        .map(|b| format!("{}:{}", b.tag, b.name))
        .collect()
}

#[test]
fn a_space_moves_into_personal_with_every_card() {
    let r = Rig::new();
    let me = r.core.user_sender().key;
    let (id, ids) = wind(&r.core, &me, &r.key_a);
    let before = space_facts(&r.core, &id, &ids);
    assert_eq!(before[1].comments, 2, "{before:?}");
    assert_eq!(
        before[1].steps,
        [("map".into(), true), ("dots".into(), false)]
    );

    let m = r
        .core
        .move_space_to_personal(&id, &MoveOpts::default())
        .unwrap();
    assert!(m.forgotten);
    assert_eq!(m.cards, 3);
    assert_eq!((m.tag.as_str(), m.name.as_str()), ("wind", "Wind Waker"));
    assert!(m.rows_copied > 0 && m.rows_already == 0, "{m:?}");

    // The space is gone, and Personal holds its board and every card,
    // with the same refs.
    assert!(!r.core.boards().contains(&id));
    assert!(r.core.board_list().iter().all(|b| b.board_id != id));
    assert!(!r.data.join("spaces").join(format!("{id}.db")).exists());
    assert_eq!(personal_tags(&r.core), [":Personal", "wind:Wind Waker"]);
    assert_eq!(personal_facts(&r.core, &ids), before);
    for (n, want) in ids.iter().enumerate() {
        assert_eq!(&r.core.task_ref(&format!("wind#{}", n + 1)).unwrap(), want);
    }
    let card = r.core.coord().get_task(&ids[0]).unwrap();
    assert_eq!(card.board, m.board_id);
    assert_eq!(card.tag, "wind");
    // The person's rows are the person's here: their comment and answer.
    let comments = r.core.coord().comments(&ids[1]).unwrap();
    assert!(
        comments
            .iter()
            .any(|c| c.by == me && c.text == "looks good")
    );
    // Hand-out state isn't copied, and every copied row says where from.
    let rows = r
        .core
        .store()
        .list(&Filter {
            kinds: vec!["task.".into()],
            limit: store::MAX_LIST_LIMIT,
            ..Default::default()
        })
        .unwrap();
    assert!(rows.iter().all(|e| e.kind != kinds::TASK_OFFERED));
    assert!(
        rows.iter()
            .all(|e| String::from_utf8_lossy(&e.payload).contains(MOVED_FIELD))
    );

    // A restart folds the same.
    r.core.close().unwrap();
    let core = Core::start(common::options(&r.data, &r.claude)).unwrap();
    assert_eq!(personal_facts(&core, &ids), before);
    assert!(!core.boards().contains(&id));
}

/// Kept for a dry run, a second move writes nothing, and the tag stays the
/// space's until the move forgets it.
#[test]
fn moving_again_goes_on_from_where_it_stopped() {
    let r = Rig::new();
    let me = r.core.user_sender().key;
    let (id, ids) = wind(&r.core, &me, &r.key_a);
    let keep = MoveOpts {
        keep_space: true,
        ..Default::default()
    };
    let first = r.core.move_space_to_personal(&id, &keep).unwrap();
    assert!(!first.forgotten);
    assert!(r.core.boards().contains(&id));
    let again = r.core.move_space_to_personal(&id, &keep).unwrap();
    assert_eq!(again.rows_copied, 0, "{again:?}");
    assert_eq!(again.rows_already, first.rows_copied);
    assert_eq!(again.board_id, first.board_id);
    assert_eq!(personal_tags(&r.core).len(), 2);
    // Both hold wind#1 now: the ref is ambiguous until the space goes.
    let e = r.core.task_ref("wind#1").unwrap_err();
    assert_eq!(e.kind(), CK::Ambiguous, "{e}");
    let last = r
        .core
        .move_space_to_personal(&id, &MoveOpts::default())
        .unwrap();
    assert!(last.forgotten && last.rows_copied == 0);
    assert_eq!(r.core.task_ref("wind#1").unwrap(), ids[0]);
}

/// Another space's tag, or one of Personal's, isn't taken over.
#[test]
fn a_tag_held_elsewhere_stops_the_move() {
    let r = Rig::new();
    let me = r.core.user_sender().key;
    let (id, _) = wind(&r.core, &me, &r.key_a);
    // A Personal board can't take the tag while the space holds it…
    let e = r.core.create_personal_board("Wind", "wind").unwrap_err();
    assert!(e.to_string().contains("already board"), "{e}");
    // …and a space with two boards stays where it is.
    r.core.create_space_board(&id, "Netcode", "net").unwrap();
    let e = r
        .core
        .move_space_to_personal(&id, &MoveOpts::default())
        .unwrap_err();
    assert!(
        matches!(&e, BoardError::Coord(c) if c.kind() == CK::NotAllowed),
        "{e}"
    );
    assert!(r.core.boards().contains(&id));
}

/// The other device forgets the space and gets the board and its cards.
#[test]
fn the_other_device_follows_the_move() {
    let (a, b) = paired();
    let (id, ids) = wind(&a.core, &a.user(), &a.agent);
    wait_for(45, "b opens the space", || b.core.board_coord(&id).is_ok());
    let before = space_facts(&a.core, &id, &ids);
    let m = a
        .core
        .move_space_to_personal(&id, &MoveOpts::default())
        .unwrap();
    wait_for(45, "b forgets the space", || !b.core.boards().contains(&id));
    wait_for(45, "b gets the board and its cards", || {
        personal_tags(&b.core).len() == 2
            && ids.iter().all(|t| {
                b.core
                    .coord()
                    .get_task(t)
                    .is_some_and(|t| t.board == m.board_id)
            })
    });
    wait_for(45, "b folds every row", || {
        personal_facts(&b.core, &ids) == before
    });
    assert_eq!(b.core.task_ref("wind#1").unwrap(), ids[0]);
    assert!(b.core.board_list().iter().all(|s| s.board_id != id));
}
