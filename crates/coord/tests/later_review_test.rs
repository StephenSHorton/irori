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

// ---- findings 12, 26 and section 5 bug 1: the sweep's closes ----

mod sweep_closes {
    use super::*;
    use crate::common::two_people::*;

    /// Finding 12: p2's after wait ends on dev-b while p1 parks the card
    /// on the person's word on dev-a. The word hold stands on both.
    #[test]
    fn a_sweep_that_hasnt_seen_the_hold_leaves_it() {
        let (mut a, mut b, _) = two_people();
        let t = a.c.create_task(P1, nt("card")).unwrap();
        let f = b.c.create_task(P2, nt("first")).unwrap();
        sync(&a, &b);
        b.c.add_after(P2, &t.id, std::slice::from_ref(&f.id))
            .unwrap();
        sync(&a, &b);
        a.c.park(P1, &t.id, "not now", Back::Word).unwrap();
        b.c.set_task_state(P2, &f.id, COMPLETED, "done").unwrap();
        b.c.sweep_notices().unwrap();
        sync(&a, &b);
        for round in ["live", "restarted"] {
            for (name, s) in [("dev-a", &a), ("dev-b", &b)] {
                let got = s.c.get_task(&t.id).unwrap();
                assert_eq!(got.state, BLOCKED, "{name} {round}: {:?}", got.blockers);
                let p = got
                    .parked()
                    .unwrap_or_else(|| panic!("{name} {round}: not held"));
                assert_eq!(p.back(), Some(Back::Word), "{name} {round}");
                assert!(task_waits(&got).is_empty(), "{name} {round}");
                assert_eq!(s.c.ignored_rows(), 0, "{name} {round}");
            }
            a.restart();
            b.restart();
        }
    }

    /// Finding 26 and section 5 bug 1: p2's device may not close p1's
    /// agent's waits, so its sweeps write nothing, whether one item or
    /// every item has ended. p1's device then closes them.
    #[test]
    fn a_non_raisers_sweep_writes_nothing() {
        for both in [false, true] {
            let (a, b, owners) = two_people();
            owners.lock().unwrap().clear();
            let t = a.c.create_task(P1, nt("card")).unwrap();
            let x = a.c.create_task(P1, nt("x")).unwrap();
            let y = a.c.create_task(P1, nt("y")).unwrap();
            a.c.add_after(CARD1, &t.id, &[x.id.clone(), y.id.clone()])
                .unwrap();
            a.c.set_task_state(P1, &x.id, COMPLETED, "done").unwrap();
            if both {
                a.c.set_task_state(P1, &y.id, COMPLETED, "done").unwrap();
            }
            pull(&a, &b);
            let before = b.st.vector().unwrap();
            for _ in 0..4 {
                b.c.sweep_notices().unwrap();
            }
            assert_eq!(b.c.ignored_rows(), 0, "both={both}");
            assert_eq!(b.st.vector().unwrap(), before, "both={both}: dev-b wrote");
            let open =
                b.c.get_task(&t.id)
                    .unwrap()
                    .blockers
                    .iter()
                    .filter(|b| b.open())
                    .count();
            assert_eq!(open, 2, "both={both}");

            a.c.sweep_notices().unwrap();
            sync(&a, &b);
            for s in [&a, &b] {
                let got = s.c.get_task(&t.id).unwrap();
                assert_eq!(
                    task_waits(&got),
                    if both { vec![] } else { vec![y.id.clone()] }
                );
                assert_eq!(got.state == BLOCKED, !both, "both={both}");
                assert_eq!(s.c.ignored_rows(), 0, "both={both}");
            }
        }
    }
}

// ---- findings 13, 23: a held card's pending spawn on a board ----

mod held_spawns {
    use super::*;
    use crate::common::two_people::*;

    fn ask(id: &str) -> SpawnAsk {
        SpawnAsk {
            task_id: id.into(),
            device: "dev-a".into(),
            folder: "/repo".into(),
            permission_mode: "bypassPermissions".into(),
            name: "for-1".into(),
            ..SpawnAsk::default()
        }
    }

    /// p1's card with a spawn pending on dev-a, parked by p2 on dev-b.
    fn parked_by_p2() -> (Side, Side, Task, i64) {
        let (a, b, _) = two_people();
        let t = a.c.create_task(P1, nt("Voice calls")).unwrap();
        let g = a.c.request_spawn("irori", ask(&t.id)).unwrap().unwrap();
        sync(&a, &b);
        b.c.park(P2, &t.id, "not now", Back::Word).unwrap();
        sync(&a, &b);
        (a, b, t, g)
    }

    /// Finding 13 and 23: p2's park writes no dead cancel, and the agent
    /// starting on dev-a isn't bound to the held card: dev-a ends the
    /// request instead.
    #[test]
    fn another_persons_park_never_binds_the_starting_agent() {
        let (mut a, mut b, t, g) = parked_by_p2();
        assert_eq!(b.c.ignored_rows(), 0, "dev-b wrote a dead cancel");
        assert!(
            !a.c.bind_spawn(&t.id, g, CARD1).unwrap(),
            "bound a held card"
        );
        sync(&a, &b);
        for round in ["live", "restarted"] {
            for (name, s) in [("dev-a", &a), ("dev-b", &b)] {
                let got = s.c.get_task(&t.id).unwrap();
                assert!(got.holders.is_empty(), "{name} {round}: {:?}", got.holders);
                assert!(got.assigned.is_empty(), "{name} {round}");
                assert!(!got.spawn_pending(), "{name} {round}: {:?}", got.spawns);
                assert!(got.parked().is_some(), "{name} {round}");
                assert_eq!(got.state, BLOCKED, "{name} {round}");
                assert!(!s.c.is_spawned(CARD1), "{name} {round}");
                assert_eq!(s.c.ignored_rows(), 0, "{name} {round}");
            }
            a.restart();
            b.restart();
        }
    }

    /// Finding 23: with no agent showing yet, dev-a's sweep ends the held
    /// card's request, once.
    #[test]
    fn the_authors_sweep_ends_a_held_cards_spawn() {
        let (a, b, t, _) = parked_by_p2();
        a.c.sweep_notices().unwrap();
        let after = a.st.vector().unwrap();
        a.c.sweep_notices().unwrap();
        assert_eq!(a.st.vector().unwrap(), after, "a second sweep wrote");
        sync(&a, &b);
        for (name, s) in [("dev-a", &a), ("dev-b", &b)] {
            let got = s.c.get_task(&t.id).unwrap();
            assert!(!got.spawn_pending(), "{name}: {:?}", got.spawns);
            assert_eq!(got.spawns[0].op, SPAWN_CANCELED, "{name}");
            assert_eq!(s.c.ignored_rows(), 0, "{name}");
        }
        // dev-b's sweep never writes for p1's card.
        let before = b.st.vector().unwrap();
        b.c.sweep_notices().unwrap();
        assert_eq!(b.st.vector().unwrap(), before);
    }
}

// ---- findings 3, 7, 15, 17, 18, 21, 27: held parents and their subtasks ----

mod held_parents {
    use super::*;
    use coord::roster::AgentCard;
    use coord::rows::{TaskCreatedRow, TaskParentRow};
    use gocompat::time::GoTime;
    use store::kinds;

    fn agents() -> Vec<AgentCard> {
        let at = GoTime::unix(fixed_now().secs - 3600, 0);
        [BOB, CAROL]
            .iter()
            .map(|k| AgentCard {
                key: (*k).into(),
                device_id: "dev-a".into(),
                status: "idle".into(),
                updated_at: at,
                last_activity: at,
                ..AgentCard::default()
            })
            .collect()
    }

    fn family(e: &Env, n: usize) -> (Task, Vec<Task>) {
        let p = make(e, "parent");
        let subs = (0..n)
            .map(|i| NewSubtask {
                title: format!("part {i}"),
                ..NewSubtask::default()
            })
            .collect();
        e.c.split(ALICE, &p.id, subs, Vec::new()).unwrap()
    }

    fn held(c: &Coord, id: &str) -> bool {
        c.get_task(id).unwrap().parked().is_some()
    }

    fn unheard_ask(e: &Env, text: &str) -> Task {
        e.c.post_ask(
            USER,
            NewAsk {
                text: text.into(),
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap()
        .0
    }

    fn as_card(parent: &str) -> AskOutcome {
        AskOutcome {
            kind: OUTCOME_CARD.into(),
            note: "a real card".into(),
            title: "The card it became".into(),
            parent: parent.into(),
            ..AskOutcome::default()
        }
    }

    /// Finding 3: the person closes ask A as a card under P, which is held
    /// until X, and X comes after A. That circle is refused.
    #[test]
    fn an_ask_closed_under_a_held_parent_checks_the_circle() {
        let e = Env::new();
        let a = unheard_ask(&e, "make the thing");
        let x = make(&e, "x");
        let p = make(&e, "parent");
        e.c.add_after(ALICE, &x.id, std::slice::from_ref(&a.id))
            .unwrap();
        e.c.park(USER, &p.id, "after x", Back::Card(x.id.clone()))
            .unwrap();
        let got = e.c.ask_close(USER, &a.id, as_card(&p.id));
        let err = got.expect_err("that closes a circle");
        assert!(err.to_string().contains("circle"), "{err}");
        let at = e.c.get_task(&a.id).unwrap();
        assert!(at.open_ask().is_some(), "nothing written");
        assert!(at.parent.is_empty());
    }

    /// Finding 21: an agent closes the ask it took as a card under a held
    /// parent. It would keep the card, so that is refused.
    #[test]
    fn a_taken_ask_isnt_closed_under_a_held_parent() {
        let e = Env::new();
        let p = make(&e, "parent");
        e.c.park(USER, &p.id, "not now", Back::Word).unwrap();
        let a = unheard_ask(&e, "make the thing");
        e.c.ask_claim(BOB, &a.id, Duration::from_secs(1)).unwrap();
        let got = e.c.ask_close(BOB, &a.id, as_card(&p.id));
        let err = got.expect_err("a held parent can't take a card its taker keeps");
        assert!(err.to_string().contains("Not yet"), "{err}");
        let at = e.c.get_task(&a.id).unwrap();
        assert!(at.open_ask().is_some(), "nothing written");
        assert!(at.parent.is_empty());
        // Without the parent it closes as before.
        let mut o = as_card("");
        o.parent.clear();
        let t = e.c.ask_close(BOB, &a.id, o).unwrap();
        assert_eq!(t.state, WORKING);
    }

    /// Finding 21: the person's close of an ask nobody keeps takes the
    /// held parent's hold.
    #[test]
    fn an_untaken_ask_closed_under_a_held_parent_is_held() {
        let e = Env::new();
        let p = make(&e, "parent");
        e.c.park(USER, &p.id, "not now", Back::Word).unwrap();
        let a = unheard_ask(&e, "make the thing");
        let t = e.c.ask_close(USER, &a.id, as_card(&p.id)).unwrap();
        assert!(t.parked().is_some(), "{t:?}");
        assert!(e.c.sweep_asks(&agents()).unwrap().is_empty());
    }

    /// Finding 7: a parent held until its own subtask, which BOB is on.
    /// The subtask stays free, so the hold is allowed.
    #[test]
    fn a_parent_can_be_held_until_its_own_held_subtask() {
        let e = Env::new();
        let (p, ks) = family(&e, 1);
        e.c.claim_task(BOB, &ks[0].id).unwrap();
        e.c.park(USER, &p.id, "after part 0", Back::Card(ks[0].id.clone()))
            .unwrap();
        assert!(held(&e.c, &p.id));
        let k = e.c.get_task(&ks[0].id).unwrap();
        assert!(k.parked().is_none());
        assert_eq!(k.holders, vec![BOB.to_string()]);
    }

    /// Finding 7, adopting: P is held until K and BOB is on K. P may
    /// still take K as a subtask, and K stays free.
    #[test]
    fn a_parent_held_until_a_card_can_adopt_it_while_held() {
        let e = Env::new();
        let p = make(&e, "parent");
        let k = make(&e, "k");
        e.c.park(USER, &p.id, "after k", Back::Card(k.id.clone()))
            .unwrap();
        e.c.claim_task(BOB, &k.id).unwrap();
        e.c.split(ALICE, &p.id, Vec::new(), vec![k.id.clone()])
            .unwrap();
        let kt = e.c.get_task(&k.id).unwrap();
        assert_eq!(kt.parent, p.id);
        assert!(kt.parked().is_none());
    }

    /// Finding 15: an agent rewords a parent's word hold. Bringing the
    /// parent back still frees its subtask.
    #[test]
    fn rewording_a_parents_hold_keeps_its_subtasks_in_step() {
        let e = Env::new();
        let (p, ks) = family(&e, 1);
        let (pt, _) = e.c.park(USER, &p.id, "not now", Back::Word).unwrap();
        let bid = pt.parked().unwrap().id.clone();
        e.c.revise(
            ALICE,
            &p.id,
            &bid,
            BlockerRevision {
                question: "Not yet: until the spring".into(),
                ..BlockerRevision::default()
            },
        )
        .unwrap();
        let k = e.c.get_task(&ks[0].id).unwrap();
        assert_eq!(k.open_blockers().len(), 1, "{:?}", k.blockers);
        assert_eq!(k.parked().unwrap().question, "Not yet: until the spring");
        e.c.bring_back(USER, &p.id, "go").unwrap();
        let k = e.c.get_task(&ks[0].id).unwrap();
        assert!(k.parked().is_none(), "{:?}", k.blockers);
        assert_eq!(k.state, SUBMITTED);
    }

    /// Finding 15, on the subtask: what it took from its parent is
    /// changed on the parent, not on the subtask.
    #[test]
    fn rewording_a_subtasks_inherited_hold_is_refused() {
        let e = Env::new();
        let (p, ks) = family(&e, 1);
        e.c.park(USER, &p.id, "not now", Back::Word).unwrap();
        let bid =
            e.c.get_task(&ks[0].id)
                .unwrap()
                .parked()
                .unwrap()
                .id
                .clone();
        let got = e.c.revise(
            ALICE,
            &ks[0].id,
            &bid,
            BlockerRevision {
                question: "Not yet: until the spring".into(),
                ..BlockerRevision::default()
            },
        );
        let err = got.expect_err("it's the parent's hold");
        assert!(err.to_string().contains(&p.short_ref()), "{err}");
        e.c.bring_back(USER, &p.id, "go").unwrap();
        assert!(!held(&e.c, &ks[0].id));
    }

    /// Finding 17: a word hold on a parent changed to wait on its own
    /// subtask frees that subtask; the other keeps the new hold.
    #[test]
    fn holding_a_parent_until_its_subtask_frees_that_subtask() {
        let e = Env::new();
        let (p, ks) = family(&e, 2);
        e.c.park(USER, &p.id, "not now", Back::Word).unwrap();
        assert!(held(&e.c, &ks[0].id) && held(&e.c, &ks[1].id));
        e.c.park(USER, &p.id, "after part 0", Back::Card(ks[0].id.clone()))
            .unwrap();
        let k0 = e.c.get_task(&ks[0].id).unwrap();
        assert!(k0.parked().is_none(), "{:?}", k0.blockers);
        assert_eq!(k0.state, SUBMITTED);
        let k1 = e.c.get_task(&ks[1].id).unwrap();
        assert_eq!(k1.open_blockers().len(), 1);
        assert_eq!(k1.parked().unwrap().later, "card");
    }

    /// Finding 18: reopening a subtask that was finished when its parent
    /// was held gives it the parent's hold.
    #[test]
    fn a_reopened_subtask_of_a_held_parent_is_held() {
        let e = Env::new();
        let (p, ks) = family(&e, 2);
        e.c.set_task_state(USER, &ks[1].id, CANCELED, "no").unwrap();
        e.c.park(USER, &p.id, "not now", Back::Word).unwrap();
        e.c.reopen(USER, &ks[1].id).unwrap();
        let k = e.c.get_task(&ks[1].id).unwrap();
        assert!(k.parked().is_some(), "{k:?}");
        assert!(e.c.sweep_asks(&agents()).unwrap().is_empty());
        e.c.bring_back(USER, &p.id, "go").unwrap();
        assert!(!held(&e.c, &ks[1].id));
    }

    fn old_subtask(
        id: &str,
        parent: &str,
        by: &str,
        at: GoTime,
    ) -> (TaskCreatedRow, TaskParentRow) {
        (
            TaskCreatedRow {
                task_id: id.into(),
                title: "part from an old device".into(),
                objective: "why".into(),
                by: by.into(),
                at,
                ..TaskCreatedRow::default()
            },
            TaskParentRow {
                task_id: id.into(),
                parent: parent.into(),
                by: by.into(),
                at,
            },
        )
    }

    /// Finding 27: an old device splits a held parent. This device's
    /// sweep puts the parent's hold on the new subtask, once.
    #[test]
    fn a_sweep_holds_an_old_devices_subtask_of_a_held_parent() {
        let e = Env::new();
        let p = make(&e, "parent");
        e.c.park(USER, &p.id, "not now", Back::Word).unwrap();
        let at = plus(fixed_now(), Duration::from_secs(1));
        let id = "01M6HPNJ2VWDNPD5FPHVFK4Q0D";
        let (c, pr) = old_subtask(id, &p.id, "dev-old/agent", at);
        e.remote(
            "dev-old",
            1,
            hlc_at(at),
            "dev-old/agent",
            kinds::TASK_CREATED,
            &c,
        );
        e.remote(
            "dev-old",
            2,
            hlc_at(at) + 1,
            "dev-old/agent",
            kinds::TASK_PARENT,
            &pr,
        );
        assert!(!held(&e.c, id), "the old device wrote no hold");
        e.c.sweep_notices().unwrap();
        let k = e.c.get_task(id).unwrap();
        assert_eq!(k.state, BLOCKED);
        assert_eq!(k.open_blockers().len(), 1);
        assert_eq!(k.parked().unwrap().back(), Some(Back::Word));
        let n = e.st.vector().unwrap();
        e.c.sweep_notices().unwrap();
        assert_eq!(e.st.vector().unwrap(), n, "a second sweep wrote");
        assert!(e.c.sweep_asks(&agents()).unwrap().is_empty());
        // The person freeing it alone isn't undone by the next sweep.
        e.c.bring_back(USER, id, "this part now").unwrap();
        e.c.sweep_notices().unwrap();
        assert!(!held(&e.c, id));
        assert!(held(&e.c, &p.id));
    }

    /// Finding 27 on a board: p2's old device splits p1's held parent.
    /// p2's subtask may be held by dev-a (an owner) and dev-b (its
    /// author's person); both agree, with one item.
    #[test]
    fn a_board_sweep_holds_an_old_devices_subtask_where_it_counts() {
        use crate::common::two_people::*;
        let (mut a, mut b, owners) = two_people();
        let p = a.c.create_task(P1, nt("parent")).unwrap();
        a.c.park(P1, &p.id, "not now", Back::Word).unwrap();
        sync(&a, &b);
        let at = plus(fixed_now(), Duration::from_secs(1));
        let id = "01M6HPNJ2VWDNPD5FPHVFK4Q0D";
        let (c, pr) = old_subtask(id, &p.id, CARD2, at);
        let e1 = b.st.append(CARD2, kinds::TASK_CREATED, &c).unwrap();
        let e2 = b.st.append(CARD2, kinds::TASK_PARENT, &pr).unwrap();
        b.c.fold_live(&[e1, e2]);
        sync(&a, &b);
        a.c.sweep_notices().unwrap();
        b.c.sweep_notices().unwrap();
        sync(&a, &b);
        for round in ["live", "restarted"] {
            for (name, s) in [("dev-a", &a), ("dev-b", &b)] {
                let k = s.c.get_task(id).unwrap();
                assert_eq!(k.state, BLOCKED, "{name} {round}");
                assert_eq!(k.open_blockers().len(), 1, "{name} {round}");
                assert!(k.parked().is_some(), "{name} {round}");
                assert_eq!(s.c.ignored_rows(), 0, "{name} {round}");
            }
            a.restart();
            b.restart();
        }

        // With no owner, dev-a (not the author's person) writes nothing.
        owners.lock().unwrap().clear();
        let id2 = "01M6HPNJ2VWDNPD5FPHVFK4Q0E";
        let (c, pr) = old_subtask(id2, &p.id, CARD2, at);
        let e1 = b.st.append(CARD2, kinds::TASK_CREATED, &c).unwrap();
        let e2 = b.st.append(CARD2, kinds::TASK_PARENT, &pr).unwrap();
        b.c.fold_live(&[e1, e2]);
        pull(&b, &a);
        let before = a.st.vector().unwrap();
        a.c.sweep_notices().unwrap();
        assert_eq!(a.st.vector().unwrap(), before, "dev-a wrote a dead row");
        assert!(!held(&a.c, id2));
    }
}
