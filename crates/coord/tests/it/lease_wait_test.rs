//! Card #339: an external blocker that names leases held now waits on
//! those leases, and closes by itself, telling its holder, once the last
//! of them is released (wind#91 sat blocked an hour on "a free
//! ww/dolphin-slot-1..4 lease" after every slot had gone).

use crate::common;

use std::time::Duration;

use common::*;
use coord::*;

fn bb(kind: &str, r: &str) -> Option<BlockedBy> {
    Some(BlockedBy {
        kind: kind.into(),
        reference: r.into(),
    })
}

const SLOTS: [&str; 4] = [
    "ww/dolphin-slot-1",
    "ww/dolphin-slot-2",
    "ww/dolphin-slot-3",
    "ww/dolphin-slot-4",
];

fn held_card(e: &Env) -> Task {
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "UDP pose transport".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
    t
}

#[test]
fn an_external_wait_naming_a_lease_family_closes_when_the_last_goes() {
    let e = Env::new();
    for s in SLOTS {
        e.c.acquire(BOB, s, Duration::from_secs(3600), false)
            .unwrap();
    }
    let t = held_card(&e);
    // wind#91's b1, word for word.
    let got =
        e.c.block(
            ALICE,
            &t.id,
            "Waiting for a free Dolphin slot for the two-Dolphin netem check",
            bb(
                BLOCKED_ON_EXTERNAL,
                "a free ww/dolphin-slot-1..4 lease (all four held; caribou's slot 2 ends 17:32Z)",
            ),
        )
        .unwrap();
    assert_eq!(got.state, BLOCKED);
    assert_eq!(got.blocked_by, bb(BLOCKED_ON_LEASE, &SLOTS.join(" ")));
    for s in &SLOTS[..3] {
        e.c.release(BOB, s).unwrap();
        assert!(
            e.c.sweep_notices().unwrap().is_empty(),
            "{s} freed, one still held"
        );
        assert_eq!(e.c.get_task(&t.id).unwrap().state, BLOCKED);
    }
    e.c.release(BOB, SLOTS[3]).unwrap();
    let ns = e.c.sweep_notices().unwrap();
    assert_eq!(ns.len(), 1, "{ns:?}");
    assert_eq!(ns[0].to, [ALICE]);
    assert_eq!(ns[0].kind, NOTICE_UNBLOCKED);
    assert!(ns[0].text.contains("leases released"), "{:?}", ns[0].text);
    assert_eq!(e.c.get_task(&t.id).unwrap().state, WORKING);
}

#[test]
fn an_expired_lease_frees_the_wait_too() {
    let e = Env::new();
    e.c.acquire(BOB, "gpu/qwen", mins(1), false).unwrap();
    let t = held_card(&e);
    let got =
        e.c.block(
            ALICE,
            &t.id,
            "need the GPU",
            bb(BLOCKED_ON_EXTERNAL, "waiting for gpu/qwen."),
        )
        .unwrap();
    assert_eq!(got.blocked_by, bb(BLOCKED_ON_LEASE, "gpu/qwen"));
    e.clk.add(mins(2));
    let ns = e.c.sweep_notices().unwrap();
    assert!(
        ns.iter().any(|n| n.task_id == t.id && n.to == [ALICE]),
        "{ns:?}"
    );
    assert_eq!(e.c.get_task(&t.id).unwrap().state, WORKING);
}

#[test]
fn an_external_wait_naming_no_held_lease_stays_external() {
    let e = Env::new();
    for s in SLOTS {
        e.c.acquire(BOB, s, Duration::from_secs(3600), false)
            .unwrap();
    }
    e.c.acquire(BOB, "ci", Duration::from_secs(3600), false)
        .unwrap();
    for (text, want) in [
        ("CI on #57", None),
        ("the circuit", None),
        ("ww/dolphin-slot-9", None),
        ("ww/dolphin-slot-5..8", None),
        ("ww/dolphin-slot-2 only", Some("ww/dolphin-slot-2")),
        (
            "one ww/dolphin-slot-N",
            Some("ww/dolphin-slot-1 ww/dolphin-slot-2 ww/dolphin-slot-3 ww/dolphin-slot-4"),
        ),
        (
            "ww/dolphin-slot-2..3",
            Some("ww/dolphin-slot-2 ww/dolphin-slot-3"),
        ),
    ] {
        let t = held_card(&e);
        let got =
            e.c.block(ALICE, &t.id, "wait", bb(BLOCKED_ON_EXTERNAL, text))
                .unwrap();
        let expect = match want {
            None => bb(BLOCKED_ON_EXTERNAL, text),
            Some(names) => bb(BLOCKED_ON_LEASE, names),
        };
        assert_eq!(got.blocked_by, expect, "{text}");
    }
}

#[test]
fn a_lease_wait_may_name_several_leases() {
    let e = Env::new();
    e.c.acquire(BOB, "gpu/a", Duration::from_secs(3600), false)
        .unwrap();
    e.c.acquire(BOB, "gpu/b", Duration::from_secs(3600), false)
        .unwrap();
    let t = held_card(&e);
    let got =
        e.c.block(ALICE, &t.id, "gpus", bb(BLOCKED_ON_LEASE, " gpu/a  gpu/b "))
            .unwrap();
    assert_eq!(got.blocked_by, bb(BLOCKED_ON_LEASE, "gpu/a gpu/b"));
    let t2 = held_card(&e);
    assert!(
        e.c.block(ALICE, &t2.id, "x", bb(BLOCKED_ON_LEASE, "gpu/a gpu/c"))
            .is_err()
    );
    e.c.release(BOB, "gpu/a").unwrap();
    assert!(e.c.sweep_notices().unwrap().is_empty());
    e.c.release(BOB, "gpu/b").unwrap();
    assert_eq!(e.c.sweep_notices().unwrap().len(), 1);
}
