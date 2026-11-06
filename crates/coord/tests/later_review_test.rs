//! Card #570: regression tests for the coord review's findings
//! (docs/design/later-order-findings.md). Each section is one group of
//! findings; the helpers at the top are shared.

mod common;

use std::time::Duration;

use common::*;
use coord::*;

// ---- shared helpers ----

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        objective: "why it matters".into(),
        ..NewTask::default()
    }
}

fn make(e: &Env, title: &str) -> Task {
    e.c.create_task(ALICE, nt(title)).unwrap()
}

fn opt(answer: &str) -> BlockerOption {
    BlockerOption {
        answer: answer.into(),
        ..BlockerOption::default()
    }
}

fn on(kind: &str, r: &str) -> Option<BlockedBy> {
    Some(BlockedBy {
        kind: kind.into(),
        reference: r.into(),
    })
}

fn tomorrow() -> Back {
    Back::Date(plus(fixed_now(), Duration::from_secs(24 * 3600)))
}

fn lane_of(c: &Coord, viewer: &str, id: &str) -> &'static str {
    let v = c.board(&BoardFilter {
        viewer: viewer.into(),
        ..BoardFilter::default()
    });
    let has = |g: &[Card]| g.iter().any(|c| c.task.id == id);
    match () {
        _ if has(&v.needs_you) => "needs_you",
        _ if has(&v.later) => "later",
        _ if has(&v.blocked) => "blocked",
        _ if has(&v.working) => "working",
        _ if has(&v.open) => "open",
        _ if has(&v.done) => "done",
        _ => "",
    }
}

// ---- findings 1, 2, 8, 11, 16: the same-question fold and circle edges ----

/// Finding 1: dropping a task wait's options makes it an edge, so a
/// circle it closes is refused.
#[test]
fn revise_dropping_options_checks_the_circle() {
    let e = Env::new();
    let one = make(&e, "one");
    let two = make(&e, "two");
    let (_, b) =
        e.c.raise(
            ALICE,
            &one.id,
            NewBlocker {
                question: "Wait for two, or go?".into(),
                options: vec![opt("Wait"), opt("Go")],
                blocked_by: on(BLOCKED_ON_TASK, &two.id),
                ..NewBlocker::default()
            },
        )
        .unwrap();
    e.c.add_after(ALICE, &two.id, std::slice::from_ref(&one.id))
        .unwrap();
    let got =
        e.c.revise(ALICE, &one.id, &b.id, BlockerRevision::default());
    let err = got.expect_err("that closes a circle");
    assert!(err.to_string().contains("circle"), "{err}");
    assert!(task_waits(&e.c.get_task(&one.id).unwrap()).is_empty());
}

/// Finding 2: a raise with options onto an optionless item of the same
/// question becomes that item's wait, so it is an edge.
#[test]
fn raise_onto_an_optionless_same_question_checks_the_circle() {
    let e = Env::new();
    let one = make(&e, "one");
    let two = make(&e, "two");
    e.c.raise(
        ALICE,
        &one.id,
        NewBlocker {
            question: "Which way?".into(),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    e.c.add_after(ALICE, &two.id, std::slice::from_ref(&one.id))
        .unwrap();
    let got = e.c.raise(
        ALICE,
        &one.id,
        NewBlocker {
            question: "Which way?".into(),
            options: vec![opt("Wait"), opt("Go")],
            blocked_by: on(BLOCKED_ON_TASK, &two.id),
            ..NewBlocker::default()
        },
    );
    let err = got.expect_err("that closes a circle");
    assert!(err.to_string().contains("circle"), "{err}");
    assert!(task_waits(&e.c.get_task(&one.id).unwrap()).is_empty());
    e.reopen();
    assert!(task_waits(&e.c.get_task(&one.id).unwrap()).is_empty());
}

/// Finding 8, case 1: an after item that reads like the card's own
/// reason is refused, not folded over its wait.
#[test]
fn a_new_cards_after_clashing_with_its_reason_is_refused() {
    let e = Env::new();
    let one = make(&e, "one");
    let two = make(&e, "two");
    let got = e.c.create_task(
        ALICE,
        NewTask {
            block_reason: "After #2: two".into(),
            blocked_by: on(BLOCKED_ON_TASK, &one.id),
            after: vec![two.short_ref()],
            ..nt("three")
        },
    );
    let err = got.expect_err("the after item reads like the reason");
    assert!(err.to_string().contains("same"), "{err}");
    assert_eq!(e.c.list_tasks(&TaskFilter::default()).len(), 2);
}

/// Finding 8, case 2: a new subtask's reason that reads like its held
/// parent's hold is refused, not absorbed into it.
#[test]
fn a_new_subtasks_reason_clashing_with_the_inherited_hold_is_refused() {
    let e = Env::new();
    let p = make(&e, "parent");
    let y = make(&e, "other");
    e.c.park(ALICE, &p.id, "voice calls", Back::Word).unwrap();
    let got = e.c.create_task(
        ALICE,
        NewTask {
            parent: p.id.clone(),
            block_reason: "Not yet: voice calls".into(),
            blocked_by: on(BLOCKED_ON_TASK, &y.id),
            ..nt("kid")
        },
    );
    let err = got.expect_err("the reason reads like the hold");
    assert!(err.to_string().contains("same"), "{err}");
}

/// Finding 11, scenario 1: a block with the hold's own words doesn't
/// rewrite its `on`, so a date hold still comes back.
#[test]
fn a_block_with_the_holds_words_is_refused() {
    let e = Env::new();
    let t = make(&e, "card");
    let (t, _) = e.c.park(ALICE, &t.id, "not now", tomorrow()).unwrap();
    let item = t.parked().unwrap().clone();
    assert_eq!(item.question, "Not yet: not now");
    let got = e.c.block(ALICE, &t.id, "Not yet: not now", None);
    assert!(got.is_err(), "{got:?}");
    let t = e.c.get_task(&t.id).unwrap();
    assert_eq!(t.parked().unwrap().on, item.on);
}

/// Finding 11, scenario 2: a raise on the person with the hold's words
/// doesn't turn the hold into Needs you.
#[test]
fn a_raise_with_the_holds_words_is_refused() {
    let e = Env::new();
    let t = make(&e, "card");
    let (t, _) = e.c.park(ALICE, &t.id, "not now", tomorrow()).unwrap();
    let item = t.parked().unwrap().clone();
    let got = e.c.raise(
        ALICE,
        &t.id,
        NewBlocker {
            question: "Not yet: not now".into(),
            blocked_by: on(BLOCKED_ON_PERSON, ""),
            ..NewBlocker::default()
        },
    );
    assert!(got.is_err(), "{got:?}");
    let t = e.c.get_task(&t.id).unwrap();
    assert_eq!(t.parked().unwrap().on, item.on);
    assert_eq!(lane_of(&e.c, USER, &t.id), "later");
}

/// Finding 16, e2: a hold takes no options.
#[test]
fn revise_refuses_options_on_a_hold() {
    let e = Env::new();
    let t = make(&e, "card");
    let (t, _) = e.c.park(ALICE, &t.id, "not now", Back::Word).unwrap();
    let bid = t.parked().unwrap().id.clone();
    let got = e.c.revise(
        ALICE,
        &t.id,
        &bid,
        BlockerRevision {
            options: vec![opt("go")],
            ..BlockerRevision::default()
        },
    );
    assert!(got.is_err(), "{got:?}");
    assert!(
        e.c.get_task(&t.id)
            .unwrap()
            .parked()
            .unwrap()
            .options
            .is_empty()
    );
}

/// Finding 16, e3: a card hold stays an edge, so the circle back is
/// still refused.
#[test]
fn a_card_hold_stays_an_edge_after_a_revise() {
    let e = Env::new();
    let x = make(&e, "first");
    let t = make(&e, "card");
    let (t, _) =
        e.c.park(ALICE, &t.id, "after first", Back::Card(x.id.clone()))
            .unwrap();
    let bid = t.parked().unwrap().id.clone();
    let _ = e.c.revise(
        ALICE,
        &t.id,
        &bid,
        BlockerRevision {
            options: vec![opt("go")],
            ..BlockerRevision::default()
        },
    );
    assert!(e.c.order_reaches(&t.id, &x.id));
    let got = e.c.add_after(ALICE, &x.id, std::slice::from_ref(&t.id));
    assert!(got.is_err(), "{got:?}");
}
