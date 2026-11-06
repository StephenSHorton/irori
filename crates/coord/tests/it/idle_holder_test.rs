//! Card #305: a holder that ends its turn on a working card with no
//! blocker and no change to the card is nudged, once per card change, to
//! name what the card waits on or keep going.

use crate::common;

use common::*;
use coord::*;

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        objective: "why it matters".into(),
        ..NewTask::default()
    }
}

fn idle_notices(ns: &[Notice], id: &str) -> Vec<Notice> {
    ns.iter()
        .filter(|n| n.kind == NOTICE_IDLE && n.task_id == id)
        .cloned()
        .collect()
}

#[test]
fn an_idle_holder_with_no_blocker_is_nudged_once_per_change() {
    let e = Env::new();
    let t = e.c.create_task(CAROL, nt("Port the tabs")).unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();

    // Still working: nothing yet.
    e.ros.set(ALICE, "busy");
    e.clk.add(mins(1));
    assert!(idle_notices(&e.c.sweep_notices().unwrap(), &t.id).is_empty());

    // Its turn ends after the card's last change: one nudge, waking it.
    e.clk.add(mins(1));
    e.ros.set_at(ALICE, "idle", e.clk.now());
    let ns = idle_notices(&e.c.sweep_notices().unwrap(), &t.id);
    assert_eq!(ns.len(), 1, "{ns:?}");
    assert_eq!(ns[0].to, [ALICE]);
    assert!(ns[0].wake);
    assert!(ns[0].text.contains("board_block"), "{}", ns[0].text);

    // It ran another turn and changed nothing: not told again.
    e.clk.add(mins(1));
    e.ros.set_at(ALICE, "idle", e.clk.now());
    assert!(idle_notices(&e.c.sweep_notices().unwrap(), &t.id).is_empty());

    // The card changed (a step), then another idle turn: told again.
    e.clk.add(mins(1));
    e.c.set_steps(
        ALICE,
        &t.id,
        vec![Step {
            text: "Tabs".into(),
            ..Step::default()
        }],
    )
    .unwrap();
    e.clk.add(mins(1));
    e.ros.set_at(ALICE, "idle", e.clk.now());
    assert_eq!(idle_notices(&e.c.sweep_notices().unwrap(), &t.id).len(), 1);
}

#[test]
fn a_named_wait_or_an_idle_holder_from_before_the_change_is_not_nudged() {
    let e = Env::new();
    let t = e.c.create_task(CAROL, nt("Check on the PC")).unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();

    // Idle since before the card changed: no turn ended on it yet.
    e.ros.set_at(ALICE, "idle", e.clk.now());
    e.clk.add(mins(1));
    e.c.set_steps(
        ALICE,
        &t.id,
        vec![Step {
            text: "PC".into(),
            ..Step::default()
        }],
    )
    .unwrap();
    assert!(idle_notices(&e.c.sweep_notices().unwrap(), &t.id).is_empty());

    // It names the wait: a blocker on another agent. Then its turn ends.
    e.clk.add(mins(1));
    e.c.raise(
        ALICE,
        &t.id,
        NewBlocker {
            question: "The Windows check".into(),
            blocked_by: Some(BlockedBy {
                kind: "agent".into(),
                reference: BOB.into(),
            }),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    e.clk.add(mins(1));
    e.ros.set_at(ALICE, "idle", e.clk.now());
    assert!(idle_notices(&e.c.sweep_notices().unwrap(), &t.id).is_empty());
}
