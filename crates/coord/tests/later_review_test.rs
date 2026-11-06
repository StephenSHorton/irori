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
        let ia = a.c.get_task(id).unwrap().parked().unwrap().id.clone();
        let ib = b.c.get_task(id).unwrap().parked().unwrap().id.clone();
        assert_eq!(ia, ib, "both devices name one item");

        // Bringing the parent back frees the subtask on both.
        a.c.bring_back(P1, &p.id, "go").unwrap();
        sync(&a, &b);
        for round in ["live", "restarted"] {
            for (name, s) in [("dev-a", &a), ("dev-b", &b)] {
                let k = s.c.get_task(id).unwrap();
                assert!(k.parked().is_none(), "{name} {round}: {:?}", k.blockers);
                assert_eq!(k.state, SUBMITTED, "{name} {round}");
                assert_eq!(s.c.ignored_rows(), 0, "{name} {round}");
            }
            a.restart();
            b.restart();
        }
        a.c.park(P1, &p.id, "not now", Back::Word).unwrap();
        sync(&a, &b);

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

// ---- finding 10: step owners on a held card ----

mod held_steps {
    use super::*;

    fn step(text: &str, owner: &str) -> Step {
        Step {
            text: text.into(),
            owner: owner.into(),
            ..Step::default()
        }
    }

    /// A one-step card the person holds on their word.
    fn held_card(e: &Env) -> Task {
        let t = make(e, "with a step");
        e.c.set_steps(USER, &t.id, vec![step("one", "")]).unwrap();
        e.c.park(USER, &t.id, "not now", Back::Word).unwrap();
        e.c.get_task(&t.id).unwrap()
    }

    fn still_held(e: &Env, id: &str) {
        let t = e.c.get_task(id).unwrap();
        assert!(t.parked().is_some(), "{:?}", t.blockers);
        assert!(t.co_holders.is_empty(), "{:?}", t.co_holders);
        assert!(t.steps.iter().all(|s| s.owner.is_empty()), "{:?}", t.steps);
    }

    /// Finding 10: claim refuses an agent on a held card, and so does
    /// giving itself a step.
    #[test]
    fn an_agent_cant_take_a_step_on_a_held_card() {
        let e = Env::new();
        let t = held_card(&e);
        want_err(e.c.claim_task(BOB, &t.id), ErrorKind::NotAllowed);
        want_err(
            e.c.set_step_owner(BOB, &t.id, "s1", BOB),
            ErrorKind::NotAllowed,
        );
        still_held(&e, &t.id);
    }

    /// Finding 10: an agent can't hand a step on a held card to another,
    /// so nobody is woken onto it.
    #[test]
    fn an_agent_cant_hand_a_step_on_a_held_card() {
        let e = Env::new();
        let t = held_card(&e);
        want_err(
            e.c.set_step_owner(ALICE, &t.id, "s1", CAROL),
            ErrorKind::NotAllowed,
        );
        want_err(
            e.c.set_steps_notice(ALICE, &t.id, vec![step("one", CAROL)]),
            ErrorKind::NotAllowed,
        );
        want_err(
            e.c.set_steps(ALICE, &t.id, vec![step("one", ""), step("two", BOB)]),
            ErrorKind::NotAllowed,
        );
        still_held(&e, &t.id);
    }

    /// Steps with no new owner are still the agent's to edit.
    #[test]
    fn an_agent_may_still_edit_a_held_cards_steps() {
        let e = Env::new();
        let t = held_card(&e);
        let (got, ns) =
            e.c.set_steps_notice(ALICE, &t.id, vec![step("one", ""), step("two", "")])
                .unwrap();
        assert_eq!(got.steps.len(), 2);
        assert!(ns.is_empty(), "{ns:?}");
        still_held(&e, &t.id);
    }

    /// A person giving a step is their word, as assign is: the card
    /// comes back and the owner hears it.
    #[test]
    fn a_person_giving_a_step_brings_the_card_back() {
        let e = Env::new();
        let t = held_card(&e);
        let (got, ns) = e.c.set_step_owner(USER, &t.id, "s1", BOB).unwrap();
        assert!(got.parked().is_none(), "{:?}", got.blockers);
        assert_eq!(got.co_holders, [BOB.to_string()]);
        assert_eq!(ns.len(), 1, "{ns:?}");
        assert_eq!(ns[0].kind, NOTICE_STEP_OWNER);

        let t = held_card(&e);
        let (got, ns) =
            e.c.set_steps_notice(USER, &t.id, vec![step("one", CAROL)])
                .unwrap();
        assert!(got.parked().is_none(), "{:?}", got.blockers);
        assert_eq!(ns[0].to, [CAROL.to_string()]);
    }
}

// ---- findings 5, 6, 14: loops found at read time, and gone waits on held cards ----

mod read_loops {
    use super::*;
    use coord::rows::{BlockerRow, TaskStateRow};
    use store::kinds;

    /// A wait on `on` that this device would refuse, written raw as
    /// another device that couldn't see the rest of the loop would.
    fn wait_raw(e: &Env, by: &str, id: &str, on: &str) {
        let bb = BlockedBy {
            kind: BLOCKED_ON_TASK.into(),
            reference: on.into(),
        };
        let row = TaskStateRow {
            task_id: id.into(),
            state: BLOCKED.into(),
            reason: "after it".into(),
            blocked_by: Some(bb.clone()),
            blocker: Some(BlockerRow {
                id: "b1".into(),
                question: "after it".into(),
                on: Some(bb),
                ..BlockerRow::default()
            }),
            by: by.into(),
            at: e.clk.now(),
            ..TaskStateRow::default()
        };
        e.write_raw(by, kinds::TASK_STATUS, &row);
    }

    fn sorted(mut v: Vec<String>) -> Vec<String> {
        v.sort();
        v
    }

    /// Finding 5, case 1: K is P's subtask and after X, and X waits on P
    /// from afar. All three are one loop, as the write check sees it.
    #[test]
    fn a_loop_through_a_parent_is_found() {
        let e = Env::new();
        let p = make(&e, "parent");
        let x = make(&e, "x");
        let (_, kids) =
            e.c.split(
                ALICE,
                &p.id,
                vec![NewSubtask {
                    title: "k".into(),
                    objective: "why".into(),
                    ..NewSubtask::default()
                }],
                Vec::new(),
            )
            .unwrap();
        let k = &kids[0];
        e.c.add_after(ALICE, &k.id, std::slice::from_ref(&x.id))
            .unwrap();
        assert!(e.c.order_loops().is_empty());
        wait_raw(&e, ALICE, &x.id, &p.id);
        assert!(e.c.order_reaches(&p.id, &x.id) && e.c.order_reaches(&x.id, &p.id));

        let loops = e.c.order_loops();
        assert_eq!(loops.rings().len(), 1);
        let want = sorted(vec![p.id.clone(), k.id.clone(), x.id.clone()]);
        for id in [&p.id, &k.id, &x.id] {
            assert_eq!(sorted(loops.get(id).unwrap().to_vec()), want, "{id}");
        }
        // A plain ring of three: in wait order from the lowest id.
        assert_eq!(loops.get(&p.id).unwrap()[0], want[0]);
        assert_eq!(loops.get(&p.id), loops.get(&x.id));
    }

    /// Finding 5, case 2: a ring of 70 cards closed from afar is one loop,
    /// from every card of it.
    #[test]
    fn a_loop_longer_than_64_cards_is_found() {
        let e = Env::new();
        let ts: Vec<Task> = (0..70).map(|i| make(&e, &format!("c{i}"))).collect();
        for i in 1..70 {
            e.c.add_after(ALICE, &ts[i].id, std::slice::from_ref(&ts[i - 1].id))
                .unwrap();
        }
        assert!(e.c.order_loops().is_empty());
        wait_raw(&e, ALICE, &ts[0].id, &ts[69].id);
        let loops = e.c.order_loops();
        assert_eq!(loops.rings().len(), 1);
        for t in &ts {
            assert_eq!(
                loops.get(&t.id).map(<[String]>::len),
                Some(70),
                "{}",
                t.title
            );
            assert_eq!(loops.get(&t.id), loops.get(&ts[0].id));
        }
        // A card done breaks it.
        e.c.set_task_state(ALICE, &ts[30].id, CANCELED, "not needed")
            .unwrap();
        assert!(e.c.order_loops().is_empty());
    }

    /// Finding 5: a held card in a long loop is still the person's.
    #[test]
    fn a_held_card_in_a_long_loop_needs_the_person() {
        let e = Env::new();
        let ts: Vec<Task> = (0..70).map(|i| make(&e, &format!("c{i}"))).collect();
        for i in 1..69 {
            e.c.add_after(ALICE, &ts[i].id, std::slice::from_ref(&ts[i - 1].id))
                .unwrap();
        }
        e.c.park(
            ALICE,
            &ts[69].id,
            "after c68",
            Back::Card(ts[68].id.clone()),
        )
        .unwrap();
        assert_eq!(lane_of(&e.c, USER, &ts[69].id), "later");
        wait_raw(&e, ALICE, &ts[0].id, &ts[69].id);
        assert_eq!(lane_of(&e.c, USER, &ts[69].id), "needs_you");
        // Not an agent's.
        assert_ne!(lane_of(&e.c, BOB, &ts[69].id), "needs_you");
    }

    /// Finding 6: a person's board() over a long chain of held cards reads
    /// the graph once, not once per held card (5000 took 7 s in release).
    /// The rows are written straight to the log and folded in one replay,
    /// since park's own checks would make the setup the slow part.
    #[test]
    fn a_long_chain_of_held_cards_reads_fast() {
        const N: usize = 5000;
        let e = Env::new();
        let id = |i: usize| format!("01PERF{i:020}");
        let at = e.clk.now();
        for i in 0..N {
            let row = serde_json::json!({
                "taskId": id(i), "num": i + 1, "title": format!("c{i}"),
                "objective": "x", "by": ALICE, "at": at,
            });
            e.st.append(ALICE, kinds::TASK_CREATED, &row).unwrap();
        }
        // Each held until the next.
        for i in 0..N - 1 {
            let on = serde_json::json!({"kind": "task", "ref": id(i + 1)});
            let row = serde_json::json!({
                "taskId": id(i), "state": "blocked", "reason": "Not yet: after",
                "blockedBy": on,
                "blocker": {"id": format!("b1-p{i}"), "question": "Not yet: after",
                    "on": on, "later": "card"},
                "by": ALICE, "at": at,
            });
            e.st.append(ALICE, kinds::TASK_STATUS, &row).unwrap();
        }
        let c = e.reopen();
        let f = BoardFilter {
            viewer: USER.into(),
            ..BoardFilter::default()
        };
        let start = std::time::Instant::now();
        let v = c.board(&f);
        let took = start.elapsed();
        assert_eq!(v.later.len(), N - 1);
        assert!(v.needs_you.is_empty());
        assert!(took < Duration::from_secs(1), "board() took {took:?}");
    }

    /// Finding 14, c1: an agent raises an outside wait, holds the card,
    /// and goes. The card stays in Later, not Needs you.
    #[test]
    fn a_held_cards_outside_wait_whose_agent_went_stays_later() {
        let e = Env::new();
        let t = make(&e, "vendor");
        e.c.claim_task(BOB, &t.id).unwrap();
        e.c.raise(
            BOB,
            &t.id,
            NewBlocker {
                question: "Vendor reply".into(),
                blocked_by: on(BLOCKED_ON_EXTERNAL, "vendor"),
                ..NewBlocker::default()
            },
        )
        .unwrap();
        // The park lets go of the card.
        e.c.park(BOB, &t.id, "not this week", tomorrow()).unwrap();
        assert_eq!(lane_of(&e.c, USER, &t.id), "later");
        e.ros.set(BOB, "gone");
        assert_eq!(lane_of(&e.c, USER, &t.id), "later");
        assert_eq!(lane_of(&e.c, "", &t.id), "later");
        assert!(e.c.gone_waits(&t.id).is_empty());
    }

    /// Finding 14, c2: a wait on an agent that went, beside a hold.
    #[test]
    fn a_held_cards_wait_on_a_gone_agent_stays_later() {
        let e = Env::new();
        let t = make(&e, "carol's part");
        e.c.raise(
            ALICE,
            &t.id,
            NewBlocker {
                question: "Waiting on carol".into(),
                blocked_by: on(BLOCKED_ON_AGENT, CAROL),
                ..NewBlocker::default()
            },
        )
        .unwrap();
        e.c.park(ALICE, &t.id, "not this week", tomorrow()).unwrap();
        e.ros.set(CAROL, "gone");
        assert_eq!(lane_of(&e.c, USER, &t.id), "later");
        assert!(e.c.gone_waits(&t.id).is_empty());
        // Once it's back, the gone wait is the person's again.
        e.c.bring_back(USER, &t.id, "now").unwrap();
        assert_eq!(lane_of(&e.c, USER, &t.id), "needs_you");
        assert!(!e.c.gone_waits(&t.id).is_empty());
    }
}

// ---- findings 4, 9, 20, 25 and section 5 bugs 2, 3: what counts as order, and how waits end ----

mod waits_end {
    use super::*;

    fn block_on(e: &Env, id: &str, reason: &str, k: &str) {
        e.c.block(ALICE, id, reason, on(BLOCKED_ON_TASK, k))
            .unwrap();
    }

    /// Finding 4, case 1: a hold until K isn't order on K, so add_after
    /// writes one, and bringing the card back leaves it after K.
    #[test]
    fn add_after_beside_a_card_hold_is_real_order() {
        let e = Env::new();
        let k = make(&e, "first");
        let a = make(&e, "second");
        e.c.park(USER, &a.id, "not yet", Back::Card(k.id.clone()))
            .unwrap();
        e.c.add_after(ALICE, &a.id, std::slice::from_ref(&k.id))
            .unwrap();
        let (got, _) = e.c.bring_back(USER, &a.id, "now").unwrap();
        assert_eq!(got.state, BLOCKED, "{:?}", got.blockers);
        assert_eq!(task_waits(&got), vec![k.id.clone()]);
    }

    /// Finding 4, case 2: a reason-only waits_on isn't order: a later
    /// block replaces it, and the after item must stay.
    #[test]
    fn add_after_beside_a_reason_only_wait_is_real_order() {
        let e = Env::new();
        let k = make(&e, "first");
        let a = make(&e, "second");
        block_on(&e, &a.id, "needs the first one", &k.id);
        e.c.add_after(ALICE, &a.id, std::slice::from_ref(&k.id))
            .unwrap();
        let got = e.c.block(ALICE, &a.id, "something else now", None).unwrap();
        assert_eq!(task_waits(&got), vec![k.id.clone()], "{:?}", got.blockers);
    }

    /// Finding 4, case 3: a new card's blocked_by is reason-only too.
    #[test]
    fn a_new_cards_after_beside_its_blocked_by_is_real_order() {
        let e = Env::new();
        let x = make(&e, "first");
        let t =
            e.c.create_task(
                ALICE,
                NewTask {
                    block_reason: "needs the first one".into(),
                    blocked_by: on(BLOCKED_ON_TASK, &x.id),
                    after: vec![x.id.clone()],
                    ..nt("second")
                },
            )
            .unwrap();
        let got = e.c.block(ALICE, &t.id, "something else now", None).unwrap();
        assert_eq!(task_waits(&got), vec![x.id.clone()], "{:?}", got.blockers);
    }

    /// Finding 9: the deleted card's short ref still takes its wait off.
    #[test]
    fn remove_after_by_a_deleted_cards_short_ref() {
        let e = Env::new();
        let k = make(&e, "first");
        let a = make(&e, "second");
        e.c.add_after(ALICE, &a.id, std::slice::from_ref(&k.id))
            .unwrap();
        e.c.delete_task(USER, &k.id).unwrap();
        let got = e.c.remove_after(ALICE, &a.id, &[k.short_ref()]).unwrap();
        assert_eq!(got.state, SUBMITTED, "{:?}", got.blockers);
        assert!(task_waits(&got).is_empty());
    }

    /// Parent P with one subtask, and P's lane done once it's completed.
    fn parent_and_kid(e: &Env) -> (Task, Task) {
        let p = make(e, "parent");
        let s =
            e.c.create_task(
                ALICE,
                NewTask {
                    parent: p.id.clone(),
                    ..nt("only subtask")
                },
            )
            .unwrap();
        (p, s)
    }

    /// Finding 20 h2 and section 5 bug 2: a hold until a parent ends when
    /// the parent finishes by rollup.
    #[test]
    fn a_hold_on_a_parent_finished_by_rollup_returns() {
        let e = Env::new();
        let (p, s) = parent_and_kid(&e);
        let t = make(&e, "after the parent");
        e.c.park(USER, &t.id, "not yet", Back::Card(p.id.clone()))
            .unwrap();
        e.c.set_task_state(USER, &s.id, COMPLETED, "done").unwrap();
        e.c.sweep_notices().unwrap();
        let got = e.c.get_task(&t.id).unwrap();
        assert_eq!(got.state, SUBMITTED, "{:?}", got.blockers);
        assert!(got.parked().is_none());
    }

    /// Section 5 bug 2: a plain after on a parent ends the same way.
    #[test]
    fn an_after_on_a_parent_finished_by_rollup_ends() {
        let e = Env::new();
        let (p, s) = parent_and_kid(&e);
        let t = make(&e, "after the parent");
        e.c.add_after(ALICE, &t.id, std::slice::from_ref(&p.id))
            .unwrap();
        e.c.set_task_state(USER, &s.id, COMPLETED, "done").unwrap();
        e.c.sweep_notices().unwrap();
        let got = e.c.get_task(&t.id).unwrap();
        assert_eq!(got.state, SUBMITTED, "{:?}", got.blockers);
    }

    /// Finding 20 h1 and section 5 bug 3: a hold until a deleted card, and
    /// an after on one, both end in the sweep.
    #[test]
    fn waits_on_a_deleted_card_end() {
        let e = Env::new();
        let k = make(&e, "goes away");
        let held = make(&e, "held until it");
        let after = make(&e, "after it");
        e.c.park(USER, &held.id, "not yet", Back::Card(k.id.clone()))
            .unwrap();
        e.c.add_after(ALICE, &after.id, std::slice::from_ref(&k.id))
            .unwrap();
        e.c.delete_task(USER, &k.id).unwrap();
        e.c.sweep_notices().unwrap();
        for id in [&held.id, &after.id] {
            let got = e.c.get_task(id).unwrap();
            assert_eq!(got.state, SUBMITTED, "{:?}", got.blockers);
            assert!(got.parked().is_none());
        }
    }

    /// Finding 25: the person's reply on a hold is a plain comment, so the
    /// hold is never ANSWERED, which arms v0.12.3's answered_left.
    #[test]
    fn a_reply_on_a_hold_never_marks_it_answered() {
        use crate::common::two_people::*;
        let (a, b, _) = two_people();
        let t = a.c.create_task(P1, nt("card")).unwrap();
        a.c.claim_task(CARD1, &t.id).unwrap();
        let (got, _) = a.c.park(CARD1, &t.id, "not now", Back::Word).unwrap();
        let bid = got.parked().unwrap().id.clone();
        a.c.answer(
            P1,
            &t.id,
            &bid,
            BlockerAnswer {
                text: "fine, later".into(),
                ..BlockerAnswer::default()
            },
        )
        .unwrap();
        sync(&a, &b);
        for (name, s) in [("dev-a", &a), ("dev-b", &b)] {
            let got = s.c.get_task(&t.id).unwrap();
            let p = got.parked().unwrap_or_else(|| panic!("{name}: not held"));
            assert_ne!(p.status, BLOCKER_ANSWERED, "{name}");
            let cs = s.c.comments(&t.id).unwrap_or_default();
            assert!(
                cs.iter()
                    .any(|c| c.text == "fine, later" && c.blocker_id.is_empty()),
                "{name}: {cs:?}"
            );
        }
    }
}

// ---- the checkers' second round ----

mod round_two {
    use super::*;
    use crate::common::two_people::*;

    fn hours(n: u64) -> Duration {
        Duration::from_secs(3600 * n)
    }

    fn open_items(t: &Task) -> usize {
        t.blockers.iter().filter(|b| b.open()).count()
    }

    fn holds(t: &Task) -> usize {
        t.blockers
            .iter()
            .filter(|b| b.open() && !b.later.is_empty())
            .count()
    }

    /// Finding 16 by recommend: a hold takes no options, so a card hold
    /// stays an edge and the circle back is refused.
    #[test]
    fn recommend_on_a_hold_is_refused() {
        let e = Env::new();
        let x = make(&e, "first");
        let t = make(&e, "card");
        let (t, _) =
            e.c.park(ALICE, &t.id, "after first", Back::Card(x.id.clone()))
                .unwrap();
        let bid = t.parked().unwrap().id.clone();
        let got = e.c.recommend(BOB, &t.id, &bid, "go", "");
        assert!(got.is_err(), "{got:?}");
        assert!(
            e.c.get_task(&t.id)
                .unwrap()
                .parked()
                .unwrap()
                .options
                .is_empty()
        );
        assert!(e.c.order_reaches(&t.id, &x.id));
        let got = e.c.add_after(ALICE, &x.id, std::slice::from_ref(&t.id));
        assert!(got.is_err(), "{got:?}");
    }

    /// A card waiting on `y` with the words of a hold on "voice calls".
    fn voice_wait(parent: &str, y: &str) -> NewTask {
        NewTask {
            parent: parent.into(),
            block_reason: "Not yet: voice calls".into(),
            blocked_by: on(BLOCKED_ON_TASK, y),
            ..nt("kid")
        }
    }

    /// Finding 8 by park: holding a parent whose subtask has an item with
    /// the hold's words is refused, not merged into that item.
    #[test]
    fn parking_a_parent_over_a_subtasks_same_words_is_refused() {
        let e = Env::new();
        let p = make(&e, "parent");
        let y = make(&e, "other");
        let k = e.c.create_task(ALICE, voice_wait(&p.id, &y.id)).unwrap();
        assert_eq!(task_waits(&k), vec![y.id.clone()]);
        let got = e.c.park(ALICE, &p.id, "voice calls", Back::Word);
        assert!(got.is_err(), "{got:?}");
        let k = e.c.get_task(&k.id).unwrap();
        assert_eq!(task_waits(&k), vec![y.id.clone()]);
        assert!(k.parked().is_none());
        assert!(e.c.get_task(&p.id).unwrap().parked().is_none());
    }

    /// Finding 8 by adopting: a held parent can't take a card whose own
    /// wait has the hold's words.
    #[test]
    fn adopting_a_card_with_the_holds_words_is_refused() {
        let e = Env::new();
        let y = make(&e, "other");
        let k = e.c.create_task(ALICE, voice_wait("", &y.id)).unwrap();
        let p = make(&e, "parent");
        e.c.park(ALICE, &p.id, "voice calls", Back::Word).unwrap();
        let got = e.c.split(ALICE, &p.id, Vec::new(), vec![k.id.clone()]);
        assert!(got.is_err(), "{got:?}");
        let k = e.c.get_task(&k.id).unwrap();
        assert_eq!(task_waits(&k), vec![y.id.clone()]);
        assert!(k.parked().is_none() && k.parent.is_empty());
    }

    /// Both sides agree on t, live and after a restart, and the restart
    /// changes nothing.
    fn agree(a: &mut Side, b: &mut Side, id: &str) -> (String, usize) {
        let look = |s: &Side| {
            let t = s.c.get_task(id).unwrap();
            assert_eq!(t.state == BLOCKED, open_items(&t) > 0, "{t:?}");
            (t.state.clone(), open_items(&t))
        };
        let live = look(a);
        assert_eq!(look(b), live, "live");
        a.restart();
        b.restart();
        assert_eq!(look(a), live, "dev-a restarted");
        assert_eq!(look(b), live, "dev-b restarted");
        live
    }

    /// Finding 4 of round two: two changes of one hold, made apart with
    /// the same words. A close on one device reaches the other.
    #[test]
    fn two_changes_with_the_same_words_made_apart_agree() {
        let (mut a, mut b, _) = two_people();
        let t = a.c.create_task(P1, nt("card")).unwrap();
        a.c.park(P1, &t.id, "not now", tomorrow()).unwrap();
        sync(&a, &b);
        a.c.park(P1, &t.id, "not now", Back::Word).unwrap();
        let later = Back::Date(plus(fixed_now(), hours(72)));
        b.c.park(P2, &t.id, "not now", later).unwrap();
        sync(&a, &b);
        a.clk.add(hours(73));
        b.clk.add(hours(73));
        a.c.sweep_notices().unwrap();
        b.c.sweep_notices().unwrap();
        sync(&a, &b);
        agree(&mut a, &mut b, &t.id);
    }

    /// Finding 5 of round two: a device that has only part of a change
    /// never sees the card free, so no agent takes it.
    #[test]
    fn a_change_seen_in_part_never_frees_the_card() {
        let (a, b, _) = two_people();
        let t = a.c.create_task(P1, nt("card")).unwrap();
        a.c.park(P1, &t.id, "not now", Back::Word).unwrap();
        sync(&a, &b);
        a.c.park(P1, &t.id, "not now", tomorrow()).unwrap();
        while pull_n(&a, &b, 1) > 0 {
            let got = b.c.get_task(&t.id).unwrap();
            assert_eq!(got.state, BLOCKED, "{:?}", got.blockers);
            assert!(got.parked().is_some(), "{:?}", got.blockers);
        }
        assert!(b.c.claim_task(CARD2, &t.id).is_err());
        for s in [&a, &b] {
            let got = s.c.get_task(&t.id).unwrap();
            assert_eq!(holds(&got), 1, "{:?}", got.blockers);
            assert_eq!(got.parked().unwrap().back(), Some(tomorrow()));
        }
    }

    /// Finding 6 of round two: a change ends every hold the card has, so
    /// it comes back on the new date.
    #[test]
    fn a_change_ends_every_hold() {
        let (mut a, mut b, _) = two_people();
        let t = a.c.create_task(P1, nt("card")).unwrap();
        sync(&a, &b);
        a.c.park(P1, &t.id, "not now", tomorrow()).unwrap();
        b.c.park(P2, &t.id, "for later", Back::Word).unwrap();
        sync(&a, &b);
        let later = Back::Date(plus(fixed_now(), hours(48)));
        let (got, _) = a.c.park(P1, &t.id, "next week", later.clone()).unwrap();
        assert_eq!(holds(&got), 1, "{:?}", got.blockers);
        assert_eq!(got.parked().unwrap().back(), Some(later));
        sync(&a, &b);
        a.clk.add(hours(49));
        b.clk.add(hours(49));
        a.c.sweep_notices().unwrap();
        b.c.sweep_notices().unwrap();
        sync(&a, &b);
        let (state, _) = agree(&mut a, &mut b, &t.id);
        assert_eq!(state, SUBMITTED);
    }

    /// Finding 8 of round two: a reword on dev-a crossing a subtask made
    /// on dev-b. Bringing the parent back frees that subtask too.
    #[test]
    fn a_reword_crossing_a_new_subtask_still_frees_it() {
        let (mut a, mut b, _) = two_people();
        let p = a.c.create_task(P1, nt("parent")).unwrap();
        let (pt, _) = a.c.park(P1, &p.id, "not now", Back::Word).unwrap();
        let bid = pt.parked().unwrap().id.clone();
        sync(&a, &b);
        let (_, kids) =
            b.c.split(
                P2,
                &p.id,
                vec![NewSubtask {
                    title: "part".into(),
                    ..NewSubtask::default()
                }],
                Vec::new(),
            )
            .unwrap();
        let k = kids[0].id.clone();
        assert!(b.c.get_task(&k).unwrap().parked().is_some());
        a.c.revise(
            P1,
            &p.id,
            &bid,
            BlockerRevision {
                question: "Not yet: until the spring".into(),
                ..BlockerRevision::default()
            },
        )
        .unwrap();
        sync(&a, &b);
        a.c.bring_back(P1, &p.id, "go").unwrap();
        sync(&a, &b);
        for round in ["live", "restarted"] {
            for (name, s) in [("dev-a", &a), ("dev-b", &b)] {
                let got = s.c.get_task(&k).unwrap();
                assert!(got.parked().is_none(), "{name} {round}: {:?}", got.blockers);
                assert_eq!(got.state, SUBMITTED, "{name} {round}");
            }
            a.restart();
            b.restart();
        }
    }

    /// Finding 8 of round two, by a change: P's word hold becomes a date
    /// on dev-a while dev-b adds a subtask. When the date comes, the
    /// subtask's stale word hold ends with P's.
    #[test]
    fn a_change_crossing_a_new_subtask_ends_with_the_parents_hold() {
        let (mut a, mut b, _) = two_people();
        let p = a.c.create_task(P1, nt("parent")).unwrap();
        a.c.park(P1, &p.id, "not now", Back::Word).unwrap();
        sync(&a, &b);
        let (_, kids) =
            b.c.split(
                P2,
                &p.id,
                vec![NewSubtask {
                    title: "part".into(),
                    ..NewSubtask::default()
                }],
                Vec::new(),
            )
            .unwrap();
        let k = kids[0].id.clone();
        a.c.park(P1, &p.id, "not now", tomorrow()).unwrap();
        sync(&a, &b);
        a.clk.add(hours(25));
        b.clk.add(hours(25));
        for _ in 0..2 {
            a.c.sweep_notices().unwrap();
            b.c.sweep_notices().unwrap();
            sync(&a, &b);
        }
        for round in ["live", "restarted"] {
            for (name, s) in [("dev-a", &a), ("dev-b", &b)] {
                assert!(s.c.get_task(&p.id).unwrap().parked().is_none());
                let got = s.c.get_task(&k).unwrap();
                assert!(got.parked().is_none(), "{name} {round}: {:?}", got.blockers);
                assert_eq!(got.state, SUBMITTED, "{name} {round}");
                assert_eq!(s.c.ignored_rows(), 0, "{name} {round}");
            }
            a.restart();
            b.restart();
        }
    }

    /// Finding 9 of round two: a subtask the person freed stays free,
    /// even once its closed hold is trimmed away.
    #[test]
    fn a_freed_subtask_stays_free_after_its_hold_is_trimmed() {
        let e = Env::new();
        let p = make(&e, "parent");
        let (_, kids) =
            e.c.split(
                ALICE,
                &p.id,
                vec![NewSubtask {
                    title: "part".into(),
                    ..NewSubtask::default()
                }],
                Vec::new(),
            )
            .unwrap();
        let k = kids[0].id.clone();
        e.c.park(USER, &p.id, "not now", Back::Word).unwrap();
        e.c.bring_back(USER, &k, "this part now").unwrap();
        for i in 0..11 {
            let (_, b) =
                e.c.raise(
                    ALICE,
                    &k,
                    NewBlocker {
                        question: format!("Which way {i}?"),
                        ..NewBlocker::default()
                    },
                )
                .unwrap();
            e.c.answer(
                USER,
                &k,
                &b.id,
                BlockerAnswer {
                    text: "this way".into(),
                    resolve: true,
                    ..BlockerAnswer::default()
                },
            )
            .unwrap();
        }
        let got = e.c.get_task(&k).unwrap();
        assert!(got.blockers.iter().all(|b| b.later.is_empty()), "trimmed");
        e.c.sweep_notices().unwrap();
        assert!(e.c.get_task(&k).unwrap().parked().is_none());
        let c = e.reopen();
        c.sweep_notices().unwrap();
        assert!(c.get_task(&k).unwrap().parked().is_none());
    }

    fn step(text: &str) -> Step {
        Step {
            text: text.into(),
            ..Step::default()
        }
    }

    /// Finding 10 of round two: an agent that took a step, or the card,
    /// on a device that hadn't seen the hold yet is taken off it.
    #[test]
    fn an_agent_on_a_card_held_meanwhile_is_taken_off() {
        for claim in [false, true] {
            let (mut a, mut b, _) = two_people();
            let t = b.c.create_task(P2, nt("card")).unwrap();
            b.c.set_steps(P2, &t.id, vec![step("one")]).unwrap();
            sync(&a, &b);
            a.c.park(P1, &t.id, "not now", tomorrow()).unwrap();
            if claim {
                b.c.claim_task(CARD2, &t.id).unwrap();
            } else {
                b.c.set_step_owner(CARD2, &t.id, "s1", CARD2).unwrap();
            }
            sync(&a, &b);
            a.c.sweep_notices().unwrap();
            b.c.sweep_notices().unwrap();
            sync(&a, &b);
            for round in ["live", "restarted"] {
                for (name, s) in [("dev-a", &a), ("dev-b", &b)] {
                    let got = s.c.get_task(&t.id).unwrap();
                    let at = format!("claim={claim} {name} {round}");
                    assert!(got.parked().is_some(), "{at}");
                    assert!(got.holders.is_empty(), "{at}: {:?}", got.holders);
                    assert!(got.co_holders.is_empty(), "{at}: {:?}", got.co_holders);
                    assert!(got.steps.iter().all(|s| s.owner.is_empty()), "{at}");
                    assert_eq!(s.c.ignored_rows(), 0, "{at}");
                }
                a.restart();
                b.restart();
            }
        }
    }

    /// Finding 11 of round two: a deleted card's short ref still takes
    /// its wait off after a restart.
    #[test]
    fn remove_after_by_a_deleted_cards_short_ref_after_a_restart() {
        let e = Env::new();
        let k = make(&e, "first");
        let a = make(&e, "second");
        e.c.add_after(ALICE, &a.id, std::slice::from_ref(&k.id))
            .unwrap();
        e.c.delete_task(USER, &k.id).unwrap();
        let c = e.reopen();
        let got = c.remove_after(ALICE, &a.id, &[k.short_ref()]).unwrap();
        assert_eq!(got.state, SUBMITTED, "{:?}", got.blockers);
        assert!(task_waits(&got).is_empty());
    }
}
