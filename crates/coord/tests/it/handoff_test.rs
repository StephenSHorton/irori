//! Hand-offs (card #85): assigning wakes an idle assignee, carries a note,
//! and the card shows it offered until the assignee takes it up.

use crate::common;

use common::*;
use coord::*;

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

fn get(c: &Coord, id: &str) -> Task {
    c.get_task(id).unwrap_or_else(|| panic!("no task {id}"))
}

/// ALICE holds a card and hands it to BOB with a note.
fn handed(e: &Env) -> (Task, Notice) {
    let t = e.c.create_task(ALICE, nt("ship it")).unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
    e.c.hand_off(
        ALICE,
        &t.id,
        BOB,
        "  tests pass on feat/x at abc123; next: land it  ",
    )
    .unwrap()
}

#[test]
fn test_handoff_wakes_an_idle_assignee_with_the_note() {
    let e = Env::new();
    let (t, n) = handed(&e);
    assert!(n.wake, "BOB is idle: {n:?}");
    assert_eq!(n.to, vec![BOB.to_string()]);
    assert!(
        n.text
            .contains("Hand-off note: tests pass on feat/x at abc123; next: land it"),
        "{}",
        n.text
    );
    let t = get(&e.c, &t.id);
    assert_eq!(t.assigned, vec![BOB.to_string()]);
    assert_eq!(
        t.handoffs.get(BOB).map(String::as_str),
        Some("tests pass on feat/x at abc123; next: land it")
    );
    let acts = e.c.activity(&t.id).unwrap();
    assert!(
        acts.iter().any(|a| a.what
            == format!("assigned it to {BOB}: tests pass on feat/x at abc123; next: land it")),
        "{acts:?}"
    );
}

#[test]
fn test_handoff_to_a_busy_or_unknown_agent_waits_for_next_turn() {
    let e = Env::new();
    e.ros.set(BOB, "busy");
    let (_, n) = handed(&e);
    assert!(!n.wake, "busy hears at its next turn: {n:?}");
    let e = Env::new();
    e.ros.set(BOB, "");
    let (_, n) = handed(&e);
    assert!(!n.wake, "not in the roster: {n:?}");
}

#[test]
fn test_plain_assign_has_no_note() {
    let e = Env::new();
    let t = e.c.create_task(ALICE, nt("x")).unwrap();
    let (_, n) = e.c.assign(ALICE, &t.id, BOB).unwrap();
    assert!(!n.text.contains("Hand-off note"), "{}", n.text);
    assert!(get(&e.c, &t.id).handoffs.is_empty());
}

#[test]
fn test_note_too_long_is_refused() {
    let e = Env::new();
    let t = e.c.create_task(ALICE, nt("x")).unwrap();
    let r = e.c.hand_off(ALICE, &t.id, BOB, &"n".repeat(2001));
    assert!(r.is_err());
}

#[test]
fn test_offer_ends_when_the_assignee_takes_it_up() {
    // Each way of taking it up clears the offer and its note.
    type Take = fn(&Env, &str);
    let ways: [(&str, Take); 4] = [
        ("claim", |e, id| {
            e.c.claim_task(BOB, id).unwrap();
        }),
        ("step tick", |e, id| {
            e.c.set_step(
                BOB,
                id,
                "s1",
                StepUpdate {
                    done: Some(true),
                    doing: None,
                },
            )
            .unwrap();
        }),
        ("comment", |e, id| {
            e.c.add_comment(BOB, id, "on it", false).unwrap();
        }),
        ("status", |e, id| {
            e.c.set_task_state(BOB, id, "working", "").unwrap();
        }),
    ];
    for (name, take) in ways {
        let e = Env::new();
        let t = e.c.create_task(ALICE, nt("x")).unwrap();
        e.c.set_steps(
            ALICE,
            &t.id,
            vec![Step {
                text: "one".into(),
                ..Step::default()
            }],
        )
        .unwrap();
        e.c.hand_off(ALICE, &t.id, BOB, "where it is").unwrap();
        assert_eq!(get(&e.c, &t.id).state, "submitted", "{name}");
        take(&e, &t.id);
        let t = get(&e.c, &t.id);
        assert!(t.assigned.is_empty(), "{name}: {:?}", t.assigned);
        assert!(t.handoffs.is_empty(), "{name}: {:?}", t.handoffs);
        assert_eq!(t.holders, vec![BOB.to_string()], "{name}");
        assert_eq!(t.state, "working", "{name}");
    }
}

#[test]
fn test_someone_elses_comment_keeps_the_offer() {
    let e = Env::new();
    let (t, _) = handed(&e);
    e.c.add_comment(CAROL, &t.id, "fyi", false).unwrap();
    e.c.person_comment(USER, &t.id, "thanks", false, &[], None)
        .unwrap();
    let t = get(&e.c, &t.id);
    assert_eq!(t.assigned, vec![BOB.to_string()]);
    assert!(t.handoffs.contains_key(BOB));
}

#[test]
fn test_leaving_drops_the_offer() {
    let e = Env::new();
    let (t, _) = handed(&e);
    e.c.leave(BOB, &t.id, BOB).unwrap();
    let t = get(&e.c, &t.id);
    assert!(t.assigned.is_empty() && t.handoffs.is_empty(), "{t:?}");
}

#[test]
fn test_show_lists_the_note() {
    let e = Env::new();
    let (t, _) = handed(&e);
    let sh = search::Source::search_show(
        &e.c,
        &format!("task:{}", t.id),
        &search::ShowOptions::default(),
    )
    .unwrap()
    .text;
    assert!(
        sh.contains(&format!("hand-off note for {BOB}: tests pass")),
        "{sh}"
    );
}
