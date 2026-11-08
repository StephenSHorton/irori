//! A card's weight (card #963): light, normal or heavy, with one line on
//! why and who set it. A weight the person set is changed only by a person.

use crate::common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::*;
use coord::rows::{TaskCreatedRow, TaskWeightRow};
use coord::*;
use store::{Event, kinds};

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

fn weight(e: &Env, id: &str) -> Option<Weight> {
    e.c.get_task(id).unwrap().weight
}

fn level(e: &Env, id: &str) -> String {
    weight(e, id).map(|w| w.level).unwrap_or_default()
}

/// An agent weighs a card with a reason; the person overrides it, and
/// from then on agents and the assistant are refused until the person
/// clears it. It all survives a reopen of the log.
#[test]
fn the_persons_weight_is_theirs() {
    let e = Env::new();
    let t = e.c.create_task(ALICE, nt("plan the split")).unwrap();
    assert_eq!(weight(&e, &t.id), None, "no weight: normal");

    want_err(
        e.c.set_weight(ALICE, &t.id, "huge", "x"),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.set_weight(ALICE, &t.id, "heavy", ""),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.set_weight(ALICE, &t.id, "heavy", &"x".repeat(MAX_WEIGHT_REASON + 1)),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.set_weight(ALICE, "01NOPE", "heavy", "x"),
        ErrorKind::NotFound,
    );

    let (got, changed) =
        e.c.set_weight(ALICE, &t.id, "heavy", "planning the split")
            .unwrap();
    assert!(changed);
    let w = got.weight.unwrap();
    assert_eq!(
        (
            w.level.as_str(),
            w.reason.as_str(),
            w.set_by.as_str(),
            w.by.as_str()
        ),
        ("heavy", "planning the split", SET_BY_AGENT, ALICE)
    );
    assert!(
        !e.c.set_weight(ALICE, &t.id, "heavy", "planning the split")
            .unwrap()
            .1,
        "the same weight writes nothing"
    );

    // The assistant (it writes as Irori) may change an agent's weight.
    e.c.set_weight("irori", &t.id, "normal", "mostly busy work")
        .unwrap();
    let w = weight(&e, &t.id).unwrap();
    assert_eq!(
        (w.level.as_str(), w.set_by.as_str()),
        ("normal", SET_BY_ASSISTANT)
    );

    // The person's pick, with or without a reason.
    e.c.set_weight(USER, &t.id, "light", "").unwrap();
    let w = weight(&e, &t.id).unwrap();
    assert_eq!(
        (w.level.as_str(), w.set_by.as_str()),
        ("light", SET_BY_PERSON)
    );
    assert!(w.by_person());

    want_err(
        e.c.set_weight(ALICE, &t.id, "heavy", "it is hard"),
        ErrorKind::NotAllowed,
    );
    want_err(e.c.set_weight(ALICE, &t.id, "", ""), ErrorKind::NotAllowed);
    want_err(
        e.c.set_weight("irori", &t.id, "heavy", "it is hard"),
        ErrorKind::NotAllowed,
    );
    assert_eq!(level(&e, &t.id), "light");

    let back = e.reopen();
    let w = back.get_task(&t.id).unwrap().weight.unwrap();
    assert_eq!(
        (w.level.as_str(), w.set_by.as_str()),
        ("light", SET_BY_PERSON)
    );

    // Cleared by the person, it is normal again and agents may weigh it.
    assert!(e.c.set_weight(USER, &t.id, "", "").unwrap().1);
    assert_eq!(weight(&e, &t.id), None);
    e.c.set_weight(ALICE, &t.id, "heavy", "it is hard").unwrap();
    assert_eq!(level(&e, &t.id), "heavy");
    assert_eq!(e.rows(kinds::TASK_WEIGHT).len(), 5);
}

fn event(dev: &str, seq: i64, at: u64, actor: &str, kind: &str, raw: Vec<u8>) -> Event {
    Event {
        device_id: dev.into(),
        seq,
        hlc: hlc::Timestamp(at),
        actor: actor.into(),
        kind: kind.into(),
        payload: raw,
        sig: Vec::new(),
    }
}

fn weight_row(dev: &str, seq: i64, at: u64, actor: &str, by: &str, level: &str) -> Event {
    let raw = gocompat::json::marshal(&TaskWeightRow {
        task_id: "A".into(),
        level: level.into(),
        reason: "why".into(),
        by: by.into(),
        at: fixed_now(),
        ..TaskWeightRow::default()
    })
    .unwrap();
    event(dev, seq, at, actor, kinds::TASK_WEIGHT, raw)
}

/// Rows from other devices fold in any order and give one answer: the
/// person's weight stands over an agent's later row, an agent's row
/// written before the person's clear stays cleared, and a row that claims
/// to be the person's from an agent counts as nobody's.
#[test]
fn weights_fold_the_same_in_any_order() {
    let base = hlc_at(fixed_now());
    // Each writer on its own device: one device's rows come in its order.
    let created = event(
        "dev-f",
        1,
        base + 1,
        "agent:dev-f/s",
        kinds::TASK_CREATED,
        gocompat::json::marshal(&TaskCreatedRow {
            task_id: "A".into(),
            num: 1,
            title: "A".into(),
            by: "dev-f/s".into(),
            at: fixed_now(),
            ..TaskCreatedRow::default()
        })
        .unwrap(),
    );
    let two: KeyFn = Arc::new(|k: &str| k == "user:dev-a" || k == "user:dev-b");
    let run = |rows: &[&Event], order: &[usize]| -> Env {
        let e = Env::with("dev-a", Some(two.clone()));
        for &i in order {
            e.c.handle(rows[i]).unwrap();
        }
        e
    };

    // The person sets heavy; an agent's later row says light.
    let person = weight_row("dev-b", 1, base + 5, "user", "user:dev-b", "heavy");
    let agent = weight_row("dev-c", 1, base + 9, "agent:dev-c/s", "dev-c/s", "light");
    let rows = [&created, &person, &agent];
    for (name, order) in BTreeMap::from([
        ("in order", vec![0, 1, 2]),
        ("agent first", vec![0, 2, 1]),
        ("weight before the card", vec![2, 1, 0]),
    ]) {
        let e = run(&rows, &order);
        let w = e.c.get_task("A").unwrap().weight.unwrap();
        assert_eq!(
            (w.level.as_str(), w.set_by.as_str()),
            ("heavy", SET_BY_PERSON),
            "{name}"
        );
    }

    // An agent's row, then the person clears: cleared in any order. An
    // agent's row after the clear counts.
    let agent_before = weight_row("dev-c", 1, base + 3, "agent:dev-c/s", "dev-c/s", "heavy");
    let clear = weight_row("dev-b", 1, base + 6, "user", "user:dev-b", "");
    let agent_after = weight_row("dev-d", 1, base + 8, "agent:dev-d/s", "dev-d/s", "light");
    let rows = [&created, &agent_before, &clear, &agent_after];
    for order in [vec![0, 1, 2], vec![0, 2, 1], vec![2, 1, 0]] {
        let e = run(&rows, &order);
        assert_eq!(e.c.get_task("A").unwrap().weight, None, "{order:?}");
    }
    for order in [vec![0, 1, 2, 3], vec![3, 2, 1, 0], vec![0, 3, 1, 2]] {
        let e = run(&rows, &order);
        assert_eq!(level(&e, "A"), "light", "{order:?}");
    }

    // An agent's row that claims the person's key, and a junk level: both
    // ignored.
    let forged = weight_row("dev-c", 1, base + 4, "agent:dev-c/s", "user:dev-b", "heavy");
    let junk = weight_row("dev-b", 1, base + 5, "user", "user:dev-b", "huge");
    let e = run(&[&created, &forged, &junk], &[0, 1, 2]);
    assert_eq!(e.c.get_task("A").unwrap().weight, None);
}

/// A card can be weighed as it is made (card #964), in one go with its
/// task.created: an agent gives a reason, a bad weight writes no card, and
/// a split checks every subtask's weight before it makes the first.
#[test]
fn a_card_is_weighed_as_it_is_made() {
    let e = Env::new();
    let weighed = |title: &str, level: &str, why: &str| NewTask {
        weight: level.into(),
        weight_reason: why.into(),
        ..nt(title)
    };
    let before = e.c.list_tasks(&TaskFilter::default()).len();
    want_err(
        e.c.create_task(ALICE, weighed("plan", "heavy", "")),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.create_task(ALICE, weighed("plan", "huge", "x")),
        ErrorKind::Invalid,
    );
    assert_eq!(
        e.c.list_tasks(&TaskFilter::default()).len(),
        before,
        "a refused weight writes no card"
    );

    let t =
        e.c.create_task(ALICE, weighed("plan", " Heavy ", "planning the split"))
            .unwrap();
    let w = t.weight.clone().expect("weighed as it is made");
    assert_eq!(
        (w.level.as_str(), w.reason.as_str(), w.set_by.as_str()),
        ("heavy", "planning the split", SET_BY_AGENT)
    );
    assert_eq!(level(&e, &t.id), "heavy");
    // The person may give one without a reason; none is normal.
    let p = e.c.create_task(USER, weighed("tidy", "light", "")).unwrap();
    assert_eq!(level(&e, &p.id), "light");
    let n = e.c.create_task(ALICE, nt("plain")).unwrap();
    assert_eq!(weight(&e, &n.id), None);

    // A split: a bad second subtask makes neither.
    let sub = |title: &str, level: &str, why: &str| NewSubtask {
        title: title.into(),
        weight: level.into(),
        weight_reason: why.into(),
        ..NewSubtask::default()
    };
    let before = e.c.list_tasks(&TaskFilter::default()).len();
    want_err(
        e.c.split(
            ALICE,
            &t.id,
            vec![
                sub("build", "normal", "follows the plan"),
                sub("docs", "light", ""),
            ],
            vec![],
        ),
        ErrorKind::Invalid,
    );
    assert_eq!(e.c.list_tasks(&TaskFilter::default()).len(), before);
    let (_, made) =
        e.c.split(
            ALICE,
            &t.id,
            vec![
                sub("build", "normal", "follows the plan"),
                sub("docs", "light", "a line per tool"),
                sub("wire", "", ""),
            ],
            vec![],
        )
        .unwrap();
    let got: Vec<String> = made.iter().map(|k| level(&e, &k.id)).collect();
    assert_eq!(got, ["normal", "light", ""]);
    assert_eq!(made[1].weight.as_ref().unwrap().reason, "a line per tool");

    // It survives a reopen of the log.
    let back = e.reopen();
    assert_eq!(back.get_task(&t.id).unwrap().weight.unwrap().level, "heavy");
    assert_eq!(
        back.get_task(&made[0].id).unwrap().weight.unwrap().level,
        "normal"
    );
}

fn stepped(e: &Env, by: &str) -> Task {
    let t = e.c.create_task(USER, nt("Move me")).unwrap();
    e.c.claim_task(by, &t.id).unwrap();
    e.c.set_steps(
        by,
        &t.id,
        vec![Step {
            text: "do it".into(),
            ..Step::default()
        }],
    )
    .unwrap();
    t
}

/// Card #966: a holder moves a card one weight, with why and left_off; it
/// leaves; and the rules that keep moves bounded refuse the rest.
#[test]
fn a_holder_moves_its_card_one_weight() {
    let e = Env::new();
    let t = stepped(&e, ALICE);
    want_err(
        e.c.move_weight(BOB, &t.id, true, "why", "next"),
        ErrorKind::NotAllowed,
    );
    want_err(
        e.c.move_weight(ALICE, &t.id, true, "", "next"),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.move_weight(ALICE, &t.id, true, "why", " "),
        ErrorKind::Invalid,
    );
    let got =
        e.c.move_weight(ALICE, &t.id, true, "harder", "next")
            .unwrap();
    let w = got.weight.unwrap();
    assert_eq!((w.level.as_str(), w.moved.as_str()), ("heavy", "up"));
    assert_eq!(got.left_off.unwrap().text, "next");
    assert!(got.holders.is_empty(), "it left");
    // Heavy is the top; moved up, it doesn't come back down.
    e.c.claim_task(ALICE, &t.id).unwrap();
    want_err(
        e.c.move_weight(ALICE, &t.id, true, "why", "next"),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.move_weight(ALICE, &t.id, false, "why", "next"),
        ErrorKind::NotAllowed,
    );
    // A plain edit over the move doesn't reopen the way back.
    e.c.set_weight(ALICE, &t.id, "normal", "edited").unwrap();
    want_err(
        e.c.move_weight(ALICE, &t.id, false, "why", "next"),
        ErrorKind::NotAllowed,
    );
    e.c.leave(ALICE, &t.id, ALICE).unwrap();
    // Down works the same way, from normal, and the person's weight is theirs.
    let d = stepped(&e, ALICE);
    let got =
        e.c.move_weight(ALICE, &d.id, false, "busy work", "next")
            .unwrap();
    assert_eq!(got.weight.unwrap().level, "light");
    let p = stepped(&e, ALICE);
    e.c.set_weight(USER, &p.id, "normal", "").unwrap();
    want_err(
        e.c.move_weight(ALICE, &p.id, true, "why", "next"),
        ErrorKind::NotAllowed,
    );
}
