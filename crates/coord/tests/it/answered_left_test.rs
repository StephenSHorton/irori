//! Card #481: an answered blocker waits on the agent it went to to act on
//! the answer. Once that agent left the card, nobody will, so the sweep
//! closes the item as Irori; the last one closed puts the card back to
//! Working (someone still holds it) or Submitted (nobody does).

use crate::common;

use common::*;
use coord::*;
use store::kinds;

const WHY: &str = "the agent it went to left the card";

fn nt(title: &str, shared: bool) -> NewTask {
    NewTask {
        title: title.into(),
        shared,
        ..NewTask::default()
    }
}

fn ask(e: &Env, by: &str, id: &str, q: &str) -> String {
    let (_, b) =
        e.c.raise(
            by,
            id,
            NewBlocker {
                question: q.into(),
                ..NewBlocker::default()
            },
        )
        .unwrap();
    b.id
}

fn say(e: &Env, id: &str, bid: &str, text: &str) {
    e.c.answer(
        USER,
        id,
        bid,
        BlockerAnswer {
            text: text.into(),
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
}

fn item(e: &Env, id: &str, bid: &str) -> Blocker {
    e.c.get_task(id)
        .unwrap()
        .blockers
        .into_iter()
        .find(|b| b.id == bid)
        .unwrap()
}

/// How many rows the sweep could write here.
fn writes(e: &Env) -> usize {
    [
        kinds::BLOCKER_RESOLVED,
        kinds::BLOCKER_CLOSED,
        kinds::TASK_STATUS,
        kinds::TASK_COMMENT,
        kinds::TASK_LEFT,
    ]
    .iter()
    .map(|k| e.rows(k).len())
    .sum()
}

fn closed_by_irori(e: &Env, id: &str, bid: &str) {
    let b = item(e, id, bid);
    let r = b.resolved.as_ref().expect("closed");
    // The person's answer stands; why Irori closed it is the note.
    assert!(
        r.closed && r.by == "irori" && r.answer == "8080" && r.note == WHY,
        "{r:?}"
    );
}

/// The review of #481: an agent that left and then asks on the card from
/// outside is back on it; its answered item stays open.
#[test]
fn an_agent_asking_after_it_left_keeps_its_item() {
    let e = Env::new();
    let t = e.c.create_task(ALICE, nt("ship it", false)).unwrap();
    e.c.claim_task(CAROL, &t.id).unwrap();
    e.c.leave(CAROL, &t.id, CAROL).unwrap();
    e.c.claim_task(BOB, &t.id).unwrap();
    let bid = ask(&e, CAROL, &t.id, "which port?");
    say(&e, &t.id, &bid, "8080");
    e.c.sweep_notices().unwrap();
    assert!(item(&e, &t.id, &bid).open());
}

/// BOB's card, blocked on a question the person answered.
fn answered(e: &Env) -> (Task, String) {
    let t = e.c.create_task(ALICE, nt("ship it", false)).unwrap();
    e.c.claim_task(BOB, &t.id).unwrap();
    let bid = ask(e, BOB, &t.id, "which port?");
    say(e, &t.id, &bid, "8080");
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.state, BLOCKED);
    assert_eq!(item(e, &t.id, &bid).status, BLOCKER_ANSWERED);
    (got, bid)
}

#[test]
fn an_answered_item_closes_once_its_agent_left() {
    let e = Env::new();
    let (t, bid) = answered(&e);
    e.c.leave(BOB, &t.id, BOB).unwrap();
    assert_eq!(e.c.get_task(&t.id).unwrap().state, BLOCKED);
    e.c.sweep_notices().unwrap();
    closed_by_irori(&e, &t.id, &bid);
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.state, SUBMITTED, "{got:?}");
    let n = writes(&e);
    e.c.sweep_notices().unwrap();
    assert_eq!(writes(&e), n, "a second sweep writes");
    // The same state on a replay.
    let c = e.reopen();
    assert_eq!(c.get_task(&t.id).unwrap().state, SUBMITTED);
}

#[test]
fn another_holder_keeps_it_working() {
    let e = Env::new();
    let t = e.c.create_task(ALICE, nt("shared", true)).unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
    e.c.join(BOB, &t.id, BOB).unwrap();
    let bid = ask(&e, BOB, &t.id, "which port?");
    say(&e, &t.id, &bid, "8080");
    e.c.leave(BOB, &t.id, BOB).unwrap();
    let ns = e.c.sweep_notices().unwrap();
    closed_by_irori(&e, &t.id, &bid);
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.state, WORKING, "{got:?}");
    assert_eq!(got.holders, [ALICE]);
    for n in &ns {
        assert!(
            !n.to.iter().any(|k| k == BOB),
            "told the agent that left: {n:?}"
        );
    }
}

#[test]
fn only_the_answered_item_closes() {
    let e = Env::new();
    let (t, bid) = answered(&e);
    let open = ask(&e, BOB, &t.id, "ship today?");
    e.c.leave(BOB, &t.id, BOB).unwrap();
    e.c.sweep_notices().unwrap();
    closed_by_irori(&e, &t.id, &bid);
    assert!(item(&e, &t.id, &open).open());
    assert_eq!(e.c.get_task(&t.id).unwrap().state, BLOCKED);
    let n = writes(&e);
    e.c.sweep_notices().unwrap();
    assert_eq!(writes(&e), n);
}

#[test]
fn an_item_from_an_agent_that_never_held_it_stays() {
    let e = Env::new();
    // CAROL made the card and asks on it; ALICE works it. CAROL never held
    // it, so it never left: its answered item waits on it still.
    let t = e.c.create_task(CAROL, nt("ship it", false)).unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
    let bid = ask(&e, CAROL, &t.id, "which port?");
    say(&e, &t.id, &bid, "8080");
    assert_eq!(item(&e, &t.id, &bid).status, BLOCKER_ANSWERED);
    e.c.sweep_notices().unwrap();
    assert!(item(&e, &t.id, &bid).open(), "{:?}", item(&e, &t.id, &bid));
    assert_eq!(e.c.get_task(&t.id).unwrap().state, BLOCKED);
}

#[test]
fn an_agent_back_on_the_card_keeps_its_item() {
    let e = Env::new();
    let (t, bid) = answered(&e);
    e.c.leave(BOB, &t.id, BOB).unwrap();
    e.c.claim_task(BOB, &t.id).unwrap();
    e.c.sweep_notices().unwrap();
    assert!(item(&e, &t.id, &bid).open());
    assert_eq!(e.c.get_task(&t.id).unwrap().state, BLOCKED);
    // Back by assignment too.
    e.c.leave(BOB, &t.id, BOB).unwrap();
    e.c.assign_task(USER, &t.id, BOB).unwrap();
    e.c.sweep_notices().unwrap();
    assert!(item(&e, &t.id, &bid).open());
}

#[test]
fn an_agent_that_went_away_closes_its_answered_item() {
    let e = Env::new();
    let (t, bid) = answered(&e);
    let open = ask(&e, BOB, &t.id, "ship today?");
    // BOB drops off the roster: the sweep takes it off the card, and the
    // same sweep closes the item that waited on it.
    e.ros.set(BOB, "");
    e.c.sweep_notices().unwrap();
    e.clk.add(HOLDER_GRACE + std::time::Duration::from_secs(1));
    e.c.sweep_notices().unwrap();
    let got = e.c.get_task(&t.id).unwrap();
    assert!(got.holders.is_empty(), "{got:?}");
    closed_by_irori(&e, &t.id, &bid);
    assert!(item(&e, &t.id, &open).open());
    assert_eq!(got.state, BLOCKED);
}
