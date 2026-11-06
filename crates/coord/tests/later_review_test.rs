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

// ---- findings 19, 22, 24: changing a hold, and ids across devices ----

mod hold_ids {
    use super::*;
    use crate::common::two_people::*;

    fn hours(n: u64) -> Duration {
        Duration::from_secs(3600 * n)
    }

    fn open_items(t: &Task) -> usize {
        t.blockers.iter().filter(|b| b.open()).count()
    }

    /// Both sides, live and after a restart, pass `check`.
    fn on_both(a: &mut Side, b: &mut Side, id: &str, check: impl Fn(&str, &Task)) {
        for round in ["live", "restarted"] {
            for (name, s) in [("dev-a", &*a), ("dev-b", &*b)] {
                let t = s.c.get_task(id).unwrap();
                check(&format!("{name} {round}"), &t);
                assert_eq!(s.c.ignored_rows(), 0, "{name} {round}");
            }
            a.restart();
            b.restart();
        }
    }

    fn held_on_word(at: &str, t: &Task) {
        assert_eq!(t.state, BLOCKED, "{at}");
        assert_eq!(open_items(t), 1, "{at}: {:?}", t.blockers);
        let p = t.parked().unwrap_or_else(|| panic!("{at}: not held"));
        assert_eq!(p.back(), Some(Back::Word), "{at}");
    }

    /// Finding 19: a date hold passes on dev-a while p2 on dev-b changes
    /// it to the person's word. The word hold stands on both.
    #[test]
    fn changing_a_hold_while_its_date_passes_elsewhere_keeps_the_new_hold() {
        let (mut a, mut b, _) = two_people();
        let t = a.c.create_task(P1, nt("Voice calls")).unwrap();
        a.c.park(P1, &t.id, "not now", tomorrow()).unwrap();
        sync(&a, &b);
        a.clk.add(hours(24));
        a.c.sweep_notices().unwrap();
        assert_eq!(a.c.get_task(&t.id).unwrap().state, SUBMITTED);
        b.c.park(P2, &t.id, "later still", Back::Word).unwrap();
        sync(&a, &b);
        on_both(&mut a, &mut b, &t.id, held_on_word);
    }

    /// Finding 22: the change stamped after the close, and before it.
    /// Neither leaves the card blocked with nothing open.
    #[test]
    fn changing_a_hold_across_its_close_never_sticks_blocked() {
        for b_later in [true, false] {
            let (mut a, mut b, _) = two_people();
            let t = a.c.create_task(P1, nt("Voice calls")).unwrap();
            a.c.park(P1, &t.id, "not now", tomorrow()).unwrap();
            sync(&a, &b);
            if !b_later {
                b.c.park(P2, &t.id, "later still", Back::Word).unwrap();
            }
            a.clk.add(hours(24));
            a.c.sweep_notices().unwrap();
            if b_later {
                b.clk.add(hours(25));
                b.c.park(P2, &t.id, "later still", Back::Word).unwrap();
            }
            sync(&a, &b);
            on_both(&mut a, &mut b, &t.id, held_on_word);
        }
    }

    /// Finding 22, by revise: rewording a date hold while it closes
    /// elsewhere leaves both devices agreeing, and not stuck.
    #[test]
    fn rewording_a_hold_across_its_close_never_sticks_blocked() {
        let (mut a, mut b, _) = two_people();
        let t = a.c.create_task(P1, nt("Voice calls")).unwrap();
        let later = Back::Date(plus(fixed_now(), hours(48)));
        let (t, _) = a.c.park(P1, &t.id, "not now", later.clone()).unwrap();
        let bid = t.parked().unwrap().id.clone();
        sync(&a, &b);
        a.clk.add(hours(48));
        a.c.sweep_notices().unwrap();
        b.c.revise(
            P2,
            &t.id,
            &bid,
            BlockerRevision {
                question: "Not yet: not before the trip".into(),
                ..BlockerRevision::default()
            },
        )
        .unwrap();
        let got = b.c.get_task(&t.id).unwrap();
        assert_eq!(got.parked().unwrap().back(), Some(later));
        sync(&a, &b);
        on_both(&mut a, &mut b, &t.id, |at, t| {
            assert_eq!(t.state == BLOCKED, open_items(t) > 0, "{at}: {t:?}");
        });
    }

    /// Finding 24, case 1: a hold on dev-a and an after on dev-b, made
    /// apart on one card, both stand.
    #[test]
    fn a_hold_and_an_after_made_apart_both_stand() {
        let (mut a, mut b, _) = two_people();
        let t = a.c.create_task(P1, nt("card")).unwrap();
        let x = a.c.create_task(P1, nt("first")).unwrap();
        sync(&a, &b);
        a.c.park(P1, &t.id, "not now", tomorrow()).unwrap();
        b.c.add_after(P2, &t.id, std::slice::from_ref(&x.id))
            .unwrap();
        sync(&a, &b);
        let xid = x.id.clone();
        on_both(&mut a, &mut b, &t.id, move |at, t| {
            assert_eq!(open_items(t), 2, "{at}: {:?}", t.blockers);
            assert!(t.parked().is_some(), "{at}");
            assert!(task_waits(t).contains(&xid), "{at}");
        });
    }

    /// Finding 24, case 2: two holds made apart on one card. The date
    /// one passing doesn't end the person's word.
    #[test]
    fn a_date_close_doesnt_end_a_word_hold_made_apart() {
        let (mut a, mut b, _) = two_people();
        let t = a.c.create_task(P1, nt("card")).unwrap();
        sync(&a, &b);
        a.c.park(P1, &t.id, "not now", tomorrow()).unwrap();
        b.c.park(P2, &t.id, "for later", Back::Word).unwrap();
        sync(&a, &b);
        a.clk.add(hours(24));
        b.clk.add(hours(24));
        a.c.sweep_notices().unwrap();
        b.c.sweep_notices().unwrap();
        sync(&a, &b);
        on_both(&mut a, &mut b, &t.id, held_on_word);
    }
}
