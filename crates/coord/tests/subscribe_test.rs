//! Port of I:internal/coord/subscribe_test.go. The expiry timer (Go's
//! armExpiry) is the core crate's (irori_core::timers), and coord.md §6
//! "expiry timer fires without a row" runs as a behaviour test there
//! (crates/core/tests/expiry.rs); here its two halves are checked:
//! `next_expiry` (when to fire) and `expire` (what firing does). Dropping
//! the queue is Go's cancel.
#![allow(clippy::type_complexity)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use coord::*;
use gocompat::sub::{Queue, TryRecv};

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

fn next(q: &Queue<Change>) -> Change {
    match q.try_recv() {
        TryRecv::Item(c) => c,
        other => panic!("no change: {other:?}"),
    }
}

fn quiet(q: &Queue<Change>) {
    if let TryRecv::Item(c) = q.try_recv() {
        panic!("unexpected change {c:?}");
    }
}

fn ch(kind: &str, task: &str, resource: &str, holder: &str, reason: &str) -> Change {
    Change {
        kind: kind.into(),
        task_id: task.into(),
        resource: resource.into(),
        holder: holder.into(),
        reason: reason.into(),
    }
}

#[test]
fn test_subscribe_mutations() {
    let e = Env::new();
    let q = e.c.subscribe();
    let task = e.c.create_task(ALICE, nt("t")).unwrap();
    assert_eq!(next(&q), ch(TASK_CHANGED, &task.id, "", "", ""));
    let id = task.id.clone();
    type Step<'a> = (&'a str, Box<dyn Fn() + 'a>, Vec<Change>);
    let c = &e.c;
    let steps: Vec<Step> = vec![
        (
            "claim",
            Box::new(|| {
                c.claim_task(ALICE, &id).unwrap();
            }),
            vec![ch(TASK_CHANGED, &id, "", ALICE, "")],
        ),
        (
            "assign",
            Box::new(|| {
                c.assign_task(ALICE, &id, BOB).unwrap();
            }),
            vec![ch(TASK_CHANGED, &id, "", BOB, "")],
        ),
        (
            "status",
            Box::new(|| {
                c.set_task_state(BOB, &id, INPUT_REQUIRED, "q").unwrap();
            }),
            vec![ch(TASK_CHANGED, &id, "", BOB, "")],
        ),
        (
            "no-op status",
            Box::new(|| {
                c.set_task_state(BOB, &id, INPUT_REQUIRED, "q").unwrap();
            }),
            vec![],
        ),
        (
            "acquire",
            Box::new(|| {
                c.acquire(ALICE, "gpu/qwen", Duration::ZERO, false).unwrap();
            }),
            vec![ch(LEASE_CHANGED, "", "gpu/qwen", ALICE, "")],
        ),
        (
            "renew",
            Box::new(|| {
                c.acquire(ALICE, "gpu/qwen", Duration::from_secs(3600), false)
                    .unwrap();
            }),
            vec![ch(LEASE_CHANGED, "", "gpu/qwen", ALICE, "")],
        ),
        (
            "refused",
            Box::new(|| {
                let _ = c.acquire(BOB, "gpu/qwen", Duration::ZERO, false);
            }),
            vec![ch(LEASE_REFUSED, "", "gpu/qwen", BOB, "")],
        ),
        (
            "refused again: deduped",
            Box::new(|| {
                let _ = c.acquire(BOB, "gpu/qwen", Duration::ZERO, false);
            }),
            vec![],
        ),
        (
            "steal",
            Box::new(|| {
                c.acquire(BOB, "gpu/qwen", Duration::ZERO, true).unwrap();
            }),
            vec![
                ch(LEASE_CHANGED, "", "gpu/qwen", ALICE, REASON_STOLEN),
                ch(LEASE_CHANGED, "", "gpu/qwen", BOB, ""),
            ],
        ),
        (
            "release",
            Box::new(|| {
                c.release(BOB, "gpu/qwen").unwrap();
            }),
            vec![ch(LEASE_CHANGED, "", "gpu/qwen", BOB, REASON_RELEASED)],
        ),
    ];
    for (name, d, want) in steps {
        d();
        for w in want {
            assert_eq!(next(&q), w, "{name}");
        }
        quiet(&q);
    }
}

#[test]
fn test_subscribe_sweep_expiry_and_gone() {
    let e = Env::new();
    let q = e.c.subscribe();
    e.c.acquire(ALICE, "gpu/a", mins(1), false).unwrap();
    e.c.acquire(BOB, "gpu/b", Duration::from_secs(3600), false)
        .unwrap();
    next(&q);
    next(&q);
    e.clk.add(mins(2));
    e.c.sweep().unwrap();
    assert_eq!(next(&q), ch(LEASE_EXPIRED, "", "gpu/a", ALICE, ""));
    e.ros.set(BOB, "");
    e.c.sweep().unwrap();
    quiet(&q);
    e.clk.add(GONE_GRACE);
    e.c.sweep().unwrap();
    assert_eq!(
        next(&q),
        ch(LEASE_CHANGED, "", "gpu/b", BOB, REASON_HOLDER_GONE)
    );
}

#[test]
fn test_subscribe_expiry_timer() {
    let e = Env::new();
    assert!(e.c.next_expiry().is_none());
    let q = e.c.subscribe();
    assert!(e.c.next_expiry().is_none(), "no lease, no timer");
    e.c.acquire(ALICE, "gpu/long", Duration::from_secs(3600), false)
        .unwrap();
    e.c.acquire(ALICE, "gpu/short", secs(1), false).unwrap();
    next(&q);
    next(&q);
    let at = e.c.next_expiry().unwrap();
    assert!(at.equal(&plus(fixed_now(), secs(1))), "{at}");
    // Another device's lease never lapses here, so it never arms the timer.
    e.remote(
        "dev-b",
        1,
        hlc_at(fixed_now()) + 1,
        "agent:dev-b/x",
        store::kinds::LEASE_ACQUIRED,
        &coord::rows::LeaseAcquiredRow {
            lease: Lease {
                resource: "gpu/remote".into(),
                holder: "dev-b/x".into(),
                until: fixed_now(),
                acquired_at: fixed_now(),
                ..Lease::default()
            },
            stolen: vec![],
        },
    );
    next(&q);
    assert!(e.c.next_expiry().unwrap().equal(&at));
    e.clk.add(secs(2));
    e.c.expire();
    assert_eq!(next(&q), ch(LEASE_EXPIRED, "", "gpu/short", ALICE, ""));
    assert!(
        e.c.next_expiry()
            .unwrap()
            .equal(&plus(fixed_now(), Duration::from_secs(3600)))
    );
}

#[test]
fn test_subscribe_slow_reader_resync() {
    let e = Env::new();
    let q = e.c.subscribe();
    for _ in 0..SUB_BUFFER + 5 {
        e.c.create_task(ALICE, nt("t")).unwrap();
    }
    for i in 0..SUB_BUFFER {
        assert_eq!(next(&q).kind, TASK_CHANGED, "change {i}");
    }
    quiet(&q);
    e.c.create_task(ALICE, nt("after")).unwrap();
    assert_eq!(next(&q).kind, RESYNC);
    let task = e.c.create_task(ALICE, nt("caught up")).unwrap();
    assert_eq!(next(&q).task_id, task.id);
}

#[test]
fn test_subscribe_stops_on_cancel() {
    let e = Env::new();
    let q: Arc<Queue<Change>> = e.c.subscribe();
    e.c.acquire(ALICE, "gpu/qwen", Duration::from_secs(3600), false)
        .unwrap();
    next(&q);
    assert!(e.c.next_expiry().is_some());
    drop(q);
    assert!(
        e.c.next_expiry().is_none(),
        "the timer stays armed after the last subscriber left"
    );
    // Changes after the drop go nowhere and nothing panics.
    e.c.release(ALICE, "gpu/qwen").unwrap();
}
