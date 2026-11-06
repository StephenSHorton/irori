//! Waits (card #87): one-shot wakes on a card's state or a branch.

use crate::common;

use common::*;
use coord::*;
use store::kinds;

fn claimed(e: &Env) -> Task {
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "ship it".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap()
}

fn on_task(id: &str, until: &[&str]) -> WaitSpec {
    WaitSpec {
        task_id: id.into(),
        until: until.iter().map(|s| s.to_string()).collect(),
        ..WaitSpec::default()
    }
}

fn waits(ns: &[Notice]) -> Vec<&Notice> {
    ns.iter().filter(|n| n.kind == NOTICE_WAIT).collect()
}

#[test]
fn test_wait_fires_once_on_done() {
    let e = Env::new();
    let t = claimed(&e);
    let w = e.c.create_wait(BOB, on_task(&t.id, &["done"])).unwrap();
    assert_eq!(w.until, vec!["done"]);
    assert_eq!(w.from_state, WORKING);
    assert!(
        waits(&e.c.sweep_notices().unwrap()).is_empty(),
        "nothing yet"
    );

    e.c.raise(
        ALICE,
        &t.id,
        NewBlocker {
            question: "key?".into(),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    assert!(
        waits(&e.c.sweep_notices().unwrap()).is_empty(),
        "blocked isn't done"
    );

    e.c.set_task_state(ALICE, &t.id, COMPLETED, "shipped")
        .unwrap();
    let ns = e.c.sweep_notices().unwrap();
    let got = waits(&ns);
    assert_eq!(got.len(), 1, "{ns:?}");
    assert_eq!(got[0].to, vec![BOB]);
    assert!(got[0].wake);
    assert_eq!(got[0].reference, format!("task:{}", t.id));
    assert!(got[0].text.contains("is now completed"), "{}", got[0].text);
    assert!(got[0].text.contains("shipped"), "{}", got[0].text);
    assert_eq!(e.rows(kinds::WAIT_FIRED).len(), 1);
    assert!(e.c.waits(BOB).is_empty());

    // Once only: the next sweeps wake nobody.
    assert!(waits(&e.c.sweep_notices().unwrap()).is_empty());
    assert!(waits(&e.reopen().sweep_notices().unwrap()).is_empty());
}

#[test]
fn test_wait_already_holds_fires_at_next_sweep() {
    let e = Env::new();
    let t = claimed(&e);
    e.c.set_task_state(ALICE, &t.id, COMPLETED, "").unwrap();
    e.c.sweep_notices().unwrap();
    e.c.create_wait(BOB, on_task(&t.id, &["done"])).unwrap();
    let ns = e.c.sweep_notices().unwrap();
    assert_eq!(waits(&ns).len(), 1, "{ns:?}");
}

#[test]
fn test_wait_changed_is_the_default_and_needs_a_move() {
    let e = Env::new();
    let t = claimed(&e);
    let w = e.c.create_wait(BOB, on_task(&t.id, &[])).unwrap();
    assert_eq!(w.until, vec!["changed"]);
    assert!(waits(&e.c.sweep_notices().unwrap()).is_empty());
    e.c.set_task_state(ALICE, &t.id, FAILED, "broke").unwrap();
    let ns = e.c.sweep_notices().unwrap();
    let got = waits(&ns);
    assert_eq!(got.len(), 1, "{ns:?}");
    assert!(got[0].text.contains("(was working)"), "{}", got[0].text);
}

#[test]
fn test_wait_catches_a_move_made_while_off() {
    // The state moved by a path the fold hook didn't see (here: a replay
    // after reopening): the sweep compares with what the wait last saw.
    let e = Env::new();
    let t = claimed(&e);
    e.c.create_wait(BOB, on_task(&t.id, &["changed"])).unwrap();
    e.c.sweep_notices().unwrap();
    e.c.set_task_state(ALICE, &t.id, FAILED, "").unwrap();
    let c = e.reopen(); // nothing swept before the "restart"
    let ns = c.sweep_notices().unwrap();
    assert_eq!(waits(&ns).len(), 1, "{ns:?}");
    assert!(e.reopen().waits(BOB).is_empty());
}

#[test]
fn test_wait_cancel_and_limits() {
    let e = Env::new();
    let t = claimed(&e);
    // Not on your own card, not as a person, not a bad until.
    want_err(
        e.c.create_wait(ALICE, on_task(&t.id, &["done"])),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.create_wait(USER, on_task(&t.id, &["done"])),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.create_wait(BOB, on_task(&t.id, &["soon"])),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.create_wait(BOB, WaitSpec::default()),
        ErrorKind::Invalid,
    );

    let w = e.c.create_wait(BOB, on_task(&t.id, &["done"])).unwrap();
    want_err(e.c.cancel_wait(CAROL, &w.id), ErrorKind::NotAllowed);
    e.c.cancel_wait(BOB, &w.id).unwrap();
    assert!(e.c.waits("").is_empty());
    e.c.set_task_state(ALICE, &t.id, COMPLETED, "").unwrap();
    assert!(waits(&e.c.sweep_notices().unwrap()).is_empty());

    for _ in 0..MAX_WAITS {
        e.c.create_wait(BOB, on_task(&t.id, &["failed"])).unwrap();
    }
    want_err(
        e.c.create_wait(BOB, on_task(&t.id, &["failed"])),
        ErrorKind::NotAllowed,
    );
}

#[test]
fn test_wait_lapses_when_the_waiter_is_gone_or_a_day_passes() {
    let e = Env::new();
    let t = claimed(&e);
    e.c.create_wait(BOB, on_task(&t.id, &["done"])).unwrap();
    e.c.create_wait(CAROL, on_task(&t.id, &["done"])).unwrap();
    e.ros.set(BOB, "gone");
    e.c.sweep_notices().unwrap(); // first seen absent
    e.clk.add(mins(11));
    let ns = e.c.sweep_notices().unwrap();
    assert!(waits(&ns).is_empty(), "a gone waiter isn't woken: {ns:?}");
    assert!(e.c.waits(BOB).is_empty());
    assert_eq!(e.c.waits(CAROL).len(), 1);

    e.clk.add(WAIT_TTL);
    let ns = e.c.sweep_notices().unwrap();
    let got = waits(&ns);
    assert_eq!(got.len(), 1, "{ns:?}");
    assert!(got[0].text.contains("a day old"), "{}", got[0].text);
    assert!(!got[0].wake, "a lapse needn't wake anyone");
    assert!(e.c.waits("").is_empty());
}

#[test]
fn test_branch_wait_fires_when_the_core_says_so() {
    let e = Env::new();
    want_err(
        e.c.create_wait(
            BOB,
            WaitSpec {
                branch: "main".into(),
                ..WaitSpec::default()
            },
        ),
        ErrorKind::Invalid,
    );
    let w =
        e.c.create_wait(
            BOB,
            WaitSpec {
                branch: "main".into(),
                repo: REPO_A.into(),
                root: "/src/app".into(),
                refs: vec!["refs/heads/main=abc".into()],
                ..WaitSpec::default()
            },
        )
        .unwrap();
    assert_eq!(e.c.branch_waits().len(), 1);
    assert!(waits(&e.c.sweep_notices().unwrap()).is_empty());
    e.c.fire_wait(&w.id, "main moved to def1234 (fix the lobby)");
    e.c.fire_wait(&w.id, "twice");
    let ns = e.c.sweep_notices().unwrap();
    let got = waits(&ns);
    assert_eq!(got.len(), 1, "{ns:?}");
    assert!(got[0].text.contains("def1234"), "{}", got[0].text);
    assert!(got[0].text.contains("branch main of"), "{}", got[0].text);
    assert!(e.c.branch_waits().is_empty());
}
