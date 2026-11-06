//! Card #570: cards have an order (docs/design/later-order.md). "After #N"
//! is a plain wait on #N, one item per card; a wait that closes a circle
//! is refused on write, and the two halves of one written on two devices
//! both fold and still show.

use crate::common;

use common::two_people::*;
use common::*;
use coord::rows::{BlockerRow, TaskStateRow};
use coord::*;
use store::kinds;

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

fn after(e: &Env, id: &str, cards: &[&Task]) -> Result<Task> {
    let ids: Vec<String> = cards.iter().map(|t| t.id.clone()).collect();
    e.c.add_after(ALICE, id, &ids)
}

fn on_task(id: &str) -> Option<BlockedBy> {
    Some(BlockedBy {
        kind: BLOCKED_ON_TASK.into(),
        reference: id.into(),
    })
}

fn status_rows(e: &Env) -> usize {
    e.rows(kinds::TASK_STATUS).len()
}

#[test]
fn after_writes_one_wait_per_card() {
    let e = Env::new();
    let a = make(&e, "Spaces");
    let b = make(&e, "Boards");
    let t = make(&e, "Move");
    let got =
        e.c.add_after(ALICE, &t.id, &["#1".to_string(), "#2".to_string()])
            .unwrap();
    assert_eq!(got.state, BLOCKED);
    assert_eq!(task_waits(&got), vec![a.id.clone(), b.id.clone()]);
    let open = got.open_blockers();
    assert_eq!(open.len(), 2);
    assert_eq!(open[0].question, "After #1: Spaces");
    assert_eq!(open[1].question, "After #2: Boards");
    assert!(got.parked().is_none(), "order isn't Not yet");

    let rows = e.rows(kinds::TASK_STATUS);
    assert_eq!(rows.len(), 2);
    let r: TaskStateRow = payload(&rows[0]);
    assert_eq!(r.reason, "After #1: Spaces");
    assert_eq!(r.blocked_by, on_task(&a.id));
    let br = r.blocker.unwrap();
    assert!(!br.reason_only, "a later plain block would replace it");
    assert!(br.later.is_empty());

    // It stays where it is on the board, showing its chain.
    let v = e.c.board(&BoardFilter::default());
    assert!(v.later.is_empty());
    assert!(v.blocked.iter().any(|c| c.task.id == t.id));
}

#[test]
fn a_plain_block_doesnt_replace_an_after() {
    let e = Env::new();
    let a = make(&e, "first");
    let t = make(&e, "second");
    after(&e, &t.id, &[&a]).unwrap();
    let got = e.c.block(ALICE, &t.id, "and another reason", None).unwrap();
    assert_eq!(got.open_blockers().len(), 2);
    assert_eq!(task_waits(&got), vec![a.id.clone()]);
}

#[test]
fn after_keeps_the_holder_and_gives_the_card_back_to_it() {
    let e = Env::new();
    let a = make(&e, "first");
    let t = make(&e, "second");
    e.c.claim_task(ALICE, &t.id).unwrap();
    let got = after(&e, &t.id, &[&a]).unwrap();
    assert_eq!(got.holders, [ALICE.to_string()]);
    e.c.set_task_state(USER, &a.id, COMPLETED, "done").unwrap();
    e.c.sweep_notices().unwrap();
    assert_eq!(e.c.get_task(&t.id).unwrap().state, WORKING);
}

#[test]
fn a_card_goes_out_when_all_it_comes_after_is_done() {
    let e = Env::new();
    let a = make(&e, "first");
    let b = make(&e, "second");
    let t = make(&e, "third");
    after(&e, &t.id, &[&a, &b]).unwrap();
    e.c.set_task_state(USER, &a.id, COMPLETED, "done").unwrap();
    let ns = e.c.sweep_notices().unwrap();
    assert!(ns.iter().all(|n| !n.text.contains("0 more")), "{ns:?}");
    assert_eq!(e.c.get_task(&t.id).unwrap().state, BLOCKED);
    e.c.set_task_state(USER, &b.id, COMPLETED, "done").unwrap();
    e.c.sweep_notices().unwrap();
    assert_eq!(e.c.get_task(&t.id).unwrap().state, SUBMITTED);
}

#[test]
fn a_card_it_already_comes_after_is_left_as_it_is() {
    let e = Env::new();
    let a = make(&e, "first");
    let t = make(&e, "second");
    after(&e, &t.id, &[&a]).unwrap();
    let n = status_rows(&e);
    let got = after(&e, &t.id, &[&a, &a]).unwrap();
    assert_eq!(got.open_blockers().len(), 1);
    assert_eq!(status_rows(&e), n);
}

#[test]
fn a_question_clash_is_refused_not_overwritten() {
    let e = Env::new();
    let a = make(&e, "Spaces");
    let t = make(&e, "Move");
    e.c.raise(
        ALICE,
        &t.id,
        NewBlocker {
            question: "After #1: Spaces".into(),
            blocked_by: Some(BlockedBy {
                kind: BLOCKED_ON_EXTERNAL.into(),
                reference: "the vendor".into(),
            }),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    let n = status_rows(&e);
    want_err(after(&e, &t.id, &[&a]), ErrorKind::Invalid);
    assert_eq!(status_rows(&e), n);
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(
        got.open_blockers()[0].on.as_ref().unwrap().kind,
        BLOCKED_ON_EXTERNAL,
        "not overwritten"
    );
}

#[test]
fn the_whole_list_is_checked_before_the_first_write() {
    let e = Env::new();
    let good = make(&e, "good");
    let done = make(&e, "done");
    e.c.set_task_state(USER, &done.id, COMPLETED, "done")
        .unwrap();
    let t = make(&e, "waiter");
    let n = status_rows(&e);
    let bad = ["#1".to_string(), "#99".to_string()];
    want_err(e.c.add_after(ALICE, &t.id, &bad), ErrorKind::NotFound);
    want_err(after(&e, &t.id, &[&good, &done]), ErrorKind::Invalid);
    want_err(after(&e, &t.id, &[&good, &t]), ErrorKind::Invalid);
    assert_eq!(status_rows(&e), n);
    assert!(e.c.get_task(&t.id).unwrap().open_blockers().is_empty());
}

#[test]
fn the_cap_of_eight_open_items_counts_the_whole_list() {
    let e = Env::new();
    let cards: Vec<Task> = (0..9).map(|i| make(&e, &format!("c{i}"))).collect();
    let t = make(&e, "waiter");
    let n = status_rows(&e);
    let all: Vec<&Task> = cards.iter().collect();
    want_err(after(&e, &t.id, &all), ErrorKind::Invalid);
    assert_eq!(status_rows(&e), n, "nothing written");
    let got = after(&e, &t.id, &all[..8]).unwrap();
    assert_eq!(got.open_blockers().len(), MAX_OPEN_BLOCKERS);
    want_err(after(&e, &t.id, &[all[8]]), ErrorKind::Invalid);
}

#[test]
fn remove_after_closes_item_by_item_and_the_card_goes_out() {
    let e = Env::new();
    let a = make(&e, "first");
    let b = make(&e, "second");
    let t = make(&e, "third");
    after(&e, &t.id, &[&a, &b]).unwrap();
    let got =
        e.c.remove_after(BOB, &t.id, std::slice::from_ref(&a.id))
            .unwrap();
    assert_eq!(task_waits(&got), vec![b.id.clone()]);
    assert_eq!(got.state, BLOCKED);
    let got =
        e.c.remove_after(BOB, &t.id, std::slice::from_ref(&b.id))
            .unwrap();
    assert_eq!(got.state, SUBMITTED);
    assert!(e.rows(kinds::BLOCKER_CLOSED).is_empty());
    assert_eq!(e.rows(kinds::BLOCKER_RESOLVED).len(), 2);
}

#[test]
fn remove_after_checks_the_whole_list_and_leaves_a_hold_alone() {
    let e = Env::new();
    let a = make(&e, "first");
    let b = make(&e, "second");
    let c = make(&e, "third");
    let t = make(&e, "waiter");
    after(&e, &t.id, &[&a]).unwrap();
    e.c.park(ALICE, &t.id, "until it's done", Back::Card(b.id.clone()))
        .unwrap();
    let n = e.rows(kinds::BLOCKER_RESOLVED).len();
    let list = [a.id.clone(), c.id.clone()];
    want_err(e.c.remove_after(ALICE, &t.id, &list), ErrorKind::NotFound);
    want_err(
        e.c.remove_after(ALICE, &t.id, std::slice::from_ref(&b.id)),
        ErrorKind::NotFound,
    );
    assert_eq!(e.rows(kinds::BLOCKER_RESOLVED).len(), n);
    assert_eq!(e.c.get_task(&t.id).unwrap().open_blockers().len(), 2);
}

#[test]
fn a_new_card_can_come_after_cards() {
    let e = Env::new();
    let a = make(&e, "Spaces");
    let b = make(&e, "Boards");
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                after: vec!["#1".into(), b.id.clone(), "#1".into()],
                ..nt("Move")
            },
        )
        .unwrap();
    assert_eq!(t.state, BLOCKED);
    assert_eq!(task_waits(&t), vec![a.id.clone(), b.id.clone()]);
    assert!(
        t.open_blockers()
            .iter()
            .all(|b| b.question.starts_with("After #"))
    );
}

#[test]
fn a_bad_after_on_a_new_card_writes_nothing() {
    let e = Env::new();
    let a = make(&e, "Spaces");
    e.c.set_task_state(USER, &a.id, COMPLETED, "done").unwrap();
    let n = e.rows(kinds::TASK_CREATED).len();
    for after in [vec!["#99".to_string()], vec![a.id.clone()]] {
        assert!(
            e.c.create_task(
                ALICE,
                NewTask {
                    after,
                    ..nt("Move")
                }
            )
            .is_err()
        );
    }
    let many: Vec<Task> = (0..9).map(|i| make(&e, &format!("c{i}"))).collect();
    let n2 = e.rows(kinds::TASK_CREATED).len();
    assert_eq!(n2, n + 9);
    want_err(
        e.c.create_task(
            ALICE,
            NewTask {
                after: many.iter().map(|t| t.id.clone()).collect(),
                ..nt("Move")
            },
        ),
        ErrorKind::Invalid,
    );
    assert_eq!(e.rows(kinds::TASK_CREATED).len(), n2);
}

#[test]
fn waits_on_still_works_beside_after() {
    let e = Env::new();
    let a = make(&e, "first");
    let b = make(&e, "second");
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                block_reason: "needs the first".into(),
                blocked_by: on_task(&a.id),
                after: vec![a.id.clone(), b.id.clone()],
                ..nt("third")
            },
        )
        .unwrap();
    assert_eq!(task_waits(&t), vec![a.id.clone(), b.id.clone()]);
    let open = t.open_blockers();
    assert_eq!(open[0].question, "needs the first");
    let rows = e.rows(kinds::TASK_STATUS);
    let r: TaskStateRow = payload(&rows[0]);
    assert!(r.blocker.unwrap().reason_only, "as before");
}

// ---- circles ----

#[test]
fn a_direct_circle_is_refused_and_named() {
    let e = Env::new();
    let a = make(&e, "one");
    let b = make(&e, "two");
    after(&e, &a.id, &[&b]).unwrap();
    let err = after(&e, &b.id, &[&a]).unwrap_err();
    assert!(err.is(ErrorKind::Invalid), "{err}");
    assert!(
        err.to_string()
            .contains("that makes a circle: #2 waits on #1, which waits on #2"),
        "{err}"
    );
    assert!(e.c.order_reaches(&a.id, &b.id));
    assert!(!e.c.order_reaches(&b.id, &a.id));
}

#[test]
fn every_wait_comes_through_the_check() {
    let e = Env::new();
    let a = make(&e, "one");
    let b = make(&e, "two");
    after(&e, &a.id, &[&b]).unwrap();
    want_err(
        e.c.block(ALICE, &b.id, "Waiting on one", on_task(&a.id)),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.raise(
            ALICE,
            &b.id,
            NewBlocker {
                question: "Waiting on one".into(),
                blocked_by: on_task(&a.id),
                ..NewBlocker::default()
            },
        ),
        ErrorKind::Invalid,
    );
    assert!(e.c.get_task(&b.id).unwrap().open_blockers().is_empty());
}

#[test]
fn a_chain_past_any_cap_is_still_refused() {
    let e = Env::new();
    let cards: Vec<Task> = (0..100).map(|i| make(&e, &format!("c{i}"))).collect();
    for w in cards.windows(2) {
        after(&e, &w[0].id, &[&w[1]]).unwrap();
    }
    let err = after(&e, &cards[99].id, &[&cards[0]]).unwrap_err();
    assert!(err.is(ErrorKind::Invalid), "{err}");
    let s = err.to_string();
    assert!(
        s.starts_with(
            "coord: invalid argument: that makes a circle: #100 waits on #1, which waits on #2"
        ),
        "{s}"
    );
    assert!(s.contains("more cards"), "{s}");
    assert!(
        s.ends_with("which waits on #99, which waits on #100"),
        "{s}"
    );
    assert!(
        s.len() < 600,
        "fits what an agent is shown: {} chars",
        s.len()
    );
    assert!(e.c.order_reaches(&cards[0].id, &cards[99].id));
}

#[test]
fn a_subtask_cant_wait_on_its_own_parent() {
    let e = Env::new();
    let p = make(&e, "parent");
    let (_, ks) =
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
    let err = after(&e, &ks[0].id, &[&p]).unwrap_err();
    assert!(err.to_string().contains("that makes a circle"), "{err}");
    // A parent may wait on its own subtask: that's just so.
    assert!(after(&e, &p.id, &[&ks[0]]).is_ok());
}

#[test]
fn a_circle_through_a_parent_is_refused() {
    let e = Env::new();
    let p = make(&e, "parent");
    let (_, ks) =
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
    let x = make(&e, "outside");
    // x can't finish before the parent, nor the parent before its part.
    after(&e, &x.id, &[&p]).unwrap();
    want_err(after(&e, &ks[0].id, &[&x]), ErrorKind::Invalid);
    // A finished subtask is no edge.
    e.c.set_task_state(USER, &ks[0].id, COMPLETED, "done")
        .unwrap();
    assert!(!e.c.order_reaches(&p.id, &ks[0].id));
}

#[test]
fn a_new_subtask_that_needs_its_parent_is_refused() {
    let e = Env::new();
    let p = make(&e, "parent");
    let x = make(&e, "outside");
    after(&e, &x.id, &[&p]).unwrap();
    let n = e.rows(kinds::TASK_CREATED).len();
    for t in [
        NewTask {
            parent: p.id.clone(),
            after: vec![x.id.clone()],
            ..nt("part")
        },
        NewTask {
            parent: p.id.clone(),
            after: vec![p.id.clone()],
            ..nt("part")
        },
        NewTask {
            parent: p.id.clone(),
            block_reason: "needs outside".into(),
            blocked_by: on_task(&x.id),
            ..nt("part")
        },
        NewTask {
            parent: p.id.clone(),
            later: Some(Back::Card(x.id.clone())),
            ..nt("part")
        },
    ] {
        let err = e.c.create_task(ALICE, t).unwrap_err();
        assert!(err.to_string().contains("the new subtask"), "{err}");
    }
    assert_eq!(e.rows(kinds::TASK_CREATED).len(), n);
}

#[test]
fn adopting_a_card_that_needs_the_parent_is_refused() {
    let e = Env::new();
    let p = make(&e, "parent");
    let x = make(&e, "outside");
    let k = make(&e, "to adopt");
    after(&e, &k.id, &[&x]).unwrap();
    after(&e, &x.id, &[&p]).unwrap();
    let err =
        e.c.split(ALICE, &p.id, Vec::new(), vec![k.id.clone()])
            .unwrap_err();
    assert!(
        err.to_string()
            .contains("that makes a circle: #1 waits on #3"),
        "{err}"
    );
    assert!(e.c.get_task(&k.id).unwrap().parent.is_empty());
}

#[test]
fn a_hold_until_a_card_is_an_edge() {
    let e = Env::new();
    let a = make(&e, "one");
    let b = make(&e, "two");
    let c = make(&e, "three");
    e.c.park(ALICE, &a.id, "after two", Back::Card(b.id.clone()))
        .unwrap();
    // b waiting on a closes it.
    want_err(after(&e, &b.id, &[&a]), ErrorKind::Invalid);
    // Parking back onto a chain does too.
    after(&e, &b.id, &[&c]).unwrap();
    let err =
        e.c.park(ALICE, &c.id, "after one", Back::Card(a.id.clone()))
            .unwrap_err();
    assert!(err.to_string().contains("that makes a circle"), "{err}");
    assert!(e.c.get_task(&c.id).unwrap().parked().is_none());
}

#[test]
fn a_held_parent_with_a_subtask_the_target_needs_is_refused() {
    let e = Env::new();
    let p = make(&e, "parent");
    let (_, ks) =
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
    let x = make(&e, "outside");
    // Holding the parent until x means x waits on what the part waits on.
    after(&e, &x.id, &[&ks[0]]).unwrap();
    let err =
        e.c.park(ALICE, &p.id, "after x", Back::Card(x.id.clone()))
            .unwrap_err();
    assert!(err.to_string().contains("that makes a circle"), "{err}");
    assert!(e.c.get_task(&p.id).unwrap().parked().is_none());
}

#[test]
fn a_wait_with_options_is_a_question_not_an_edge() {
    let e = Env::new();
    let a = make(&e, "one");
    let b = make(&e, "two");
    e.c.raise(
        ALICE,
        &a.id,
        NewBlocker {
            question: "Wait for two, or go?".into(),
            options: vec![
                BlockerOption {
                    answer: "Wait".into(),
                    ..BlockerOption::default()
                },
                BlockerOption {
                    answer: "Go".into(),
                    ..BlockerOption::default()
                },
            ],
            blocked_by: on_task(&b.id),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    assert!(after(&e, &b.id, &[&a]).is_ok());

    // Nor is it refused for closing one: the person's answer ends it.
    let c = make(&e, "three");
    after(&e, &c.id, &[&a]).unwrap();
    let opts = |a: &str| BlockerOption {
        answer: a.into(),
        ..BlockerOption::default()
    };
    let ask = |options: Vec<BlockerOption>| NewBlocker {
        question: "Wait for three, or go?".into(),
        options,
        blocked_by: on_task(&c.id),
        ..NewBlocker::default()
    };
    want_err(e.c.raise(ALICE, &a.id, ask(Vec::new())), ErrorKind::Invalid);
    assert!(
        e.c.raise(ALICE, &a.id, ask(vec![opts("Wait"), opts("Go")]))
            .is_ok()
    );
}

// ---- two devices ----

#[test]
fn two_devices_each_writing_half_a_circle_converge_and_it_still_shows() {
    let (a, b, _) = two_people();
    let one = a.c.create_task(P1, nt("one")).unwrap();
    let two = a.c.create_task(P1, nt("two")).unwrap();
    sync(&a, &b);

    // Each sees only its own half, so neither can refuse.
    a.c.add_after(CARD1, &one.id, std::slice::from_ref(&two.id))
        .unwrap();
    b.c.add_after(CARD2, &two.id, std::slice::from_ref(&one.id))
        .unwrap();
    sync(&a, &b);

    for s in [&a, &b] {
        assert_eq!(s.c.ignored_rows(), 0);
        let get = |k: &str| s.c.get_task(k);
        assert_eq!(task_waits(&get(&one.id).unwrap()), vec![two.id.clone()]);
        assert_eq!(task_waits(&get(&two.id).unwrap()), vec![one.id.clone()]);
        let ring = wait_chain(&get, &one.id).loop_ids;
        assert_eq!(ring.len(), 2, "the loop still shows");
        assert_eq!(ring, wait_chain(&get, &two.id).loop_ids);
    }
}

#[test]
fn a_loop_through_a_held_card_is_the_one_thing_it_shows_in_needs_you() {
    let (a, b, _) = two_people();
    let one = a.c.create_task(P1, nt("one")).unwrap();
    let two = a.c.create_task(P1, nt("two")).unwrap();
    sync(&a, &b);
    a.c.park(CARD1, &one.id, "after two", Back::Card(two.id.clone()))
        .unwrap();
    b.c.add_after(CARD2, &two.id, std::slice::from_ref(&one.id))
        .unwrap();
    sync(&a, &b);
    for (s, who) in [(&a, P1), (&b, P2)] {
        let v = s.c.board(&BoardFilter {
            viewer: who.into(),
            ..BoardFilter::default()
        });
        assert!(v.needs_you.iter().any(|c| c.task.id == one.id), "{who}");
        assert!(v.later.is_empty(), "a Later section must not hide it");
        // An agent isn't asked to fix it.
        let v = s.c.board(&BoardFilter {
            viewer: CARD1.into(),
            ..BoardFilter::default()
        });
        assert!(v.later.iter().any(|c| c.task.id == one.id));
    }
    // Stopping one wait ends it, and the hold is calm again.
    b.c.remove_after(P2, &two.id, std::slice::from_ref(&one.id))
        .unwrap();
    sync(&a, &b);
    let v = a.c.board(&BoardFilter::default());
    assert!(v.later.iter().any(|c| c.task.id == one.id));
}

#[test]
fn a_row_from_another_device_closing_a_circle_folds() {
    let e = Env::new();
    let a = make(&e, "one");
    let b = make(&e, "two");
    after(&e, &a.id, &[&b]).unwrap();
    let on = on_task(&a.id);
    let row = TaskStateRow {
        task_id: b.id.clone(),
        state: BLOCKED.into(),
        reason: "After #1: one".into(),
        blocked_by: on.clone(),
        blocker: Some(BlockerRow {
            id: "b1".into(),
            question: "After #1: one".into(),
            on,
            ..BlockerRow::default()
        }),
        by: BOB.into(),
        at: e.clk.now(),
        ..TaskStateRow::default()
    };
    let at = hlc_at(e.clk.now()) + 1000;
    e.remote("dev-b", 1, at, BOB, kinds::TASK_STATUS, &row);
    let c = wait_chain(&|k: &str| e.c.get_task(k), &a.id);
    assert_eq!(c.loop_ids.len(), 2);
    // And a replay says the same.
    let again = e.reopen();
    assert_eq!(
        wait_chain(&|k: &str| again.get_task(k), &a.id)
            .loop_ids
            .len(),
        2
    );
}

#[test]
fn a_shared_board_wait_stays_on_its_log_and_takes_a_tagged_ref() {
    let (a, _b, _) = two_people();
    let t = a.c.create_task(P1, nt("one")).unwrap();
    let first = a.c.create_task(P1, nt("two")).unwrap();
    let got = a.c.add_after(CARD1, &t.id, &["mlg#2".to_string()]).unwrap();
    assert_eq!(task_waits(&got), vec![first.id.clone()]);
    assert_eq!(got.open_blockers()[0].question, "After mlg#2: two");
    want_err(
        a.c.add_after(CARD1, &t.id, &["01NOTONTHISLOG".to_string()]),
        ErrorKind::NotFound,
    );
}
