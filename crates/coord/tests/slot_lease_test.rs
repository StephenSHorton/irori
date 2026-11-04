//! Card #437: check slots. A slot lease spreads one name over N leases
//! (`check/mac/slot-1`..`-N`); a third run waits while two hold them, and
//! a run holder (`<device>/run-…`, a script, not an agent) is not swept
//! as gone: its lease ends at its until.

mod common;

use common::*;
use coord::*;

const RUN1: &str = "dev-a/run-check-1";
const RUN2: &str = "dev-a/run-check-2";
const RUN3: &str = "dev-a/run-check-3";

fn holders(e: &Env) -> Vec<(String, String)> {
    e.c.leases()
        .into_iter()
        .map(|l| (l.resource, l.holder))
        .collect()
}

#[test]
fn two_slots_hold_two_runs_and_the_third_waits() {
    let e = Env::new();
    let a = e.c.acquire_slot(RUN1, "check/mac", 2, mins(5)).unwrap();
    let b = e.c.acquire_slot(RUN2, "check/mac", 2, mins(5)).unwrap();
    assert_eq!(a.resource, "check/mac/slot-1");
    assert_eq!(b.resource, "check/mac/slot-2");

    let err = e.c.acquire_slot(RUN3, "check/mac", 2, mins(5)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Held);
    assert!(
        err.to_string()
            .contains("held by dev-a/run-check-1, dev-a/run-check-2"),
        "{err}"
    );
    // Waiting writes no refusal: polling it an hour must not tell anyone.
    assert!(e.rows("lease.refused").is_empty());

    // Renewing keeps a run on its own slot.
    let a2 = e.c.acquire_slot(RUN1, "check/mac", 2, mins(5)).unwrap();
    assert_eq!(a2.resource, "check/mac/slot-1");

    // One run ends; the third gets its slot.
    assert!(e.c.release_slot(RUN1, "check/mac").unwrap());
    assert!(!e.c.release_slot(RUN1, "check/mac").unwrap());
    let c = e.c.acquire_slot(RUN3, "check/mac", 2, mins(5)).unwrap();
    assert_eq!(c.resource, "check/mac/slot-1");
    assert_eq!(
        holders(&e),
        [
            ("check/mac/slot-1".to_string(), RUN3.to_string()),
            ("check/mac/slot-2".to_string(), RUN2.to_string()),
        ]
    );
}

#[test]
fn a_run_that_stops_renewing_frees_its_slot_at_its_until() {
    let e = Env::new();
    e.c.acquire_slot(RUN1, "check/mac", 1, mins(2)).unwrap();
    assert!(e.c.acquire_slot(RUN2, "check/mac", 1, mins(2)).is_err());
    e.clk.add(mins(2));
    let l = e.c.acquire_slot(RUN2, "check/mac", 1, mins(2)).unwrap();
    assert_eq!(l.holder, RUN2);
}

#[test]
fn a_run_holder_is_not_swept_as_gone() {
    let e = Env::new();
    e.c.acquire_slot(RUN1, "check/mac", 2, MAX_TTL).unwrap();
    e.c.acquire(ALICE, "gpu/qwen", MAX_TTL, false).unwrap();
    e.ros.set(ALICE, "gone");
    for _ in 0..3 {
        e.c.sweep().unwrap();
        e.clk.add(GONE_GRACE);
    }
    e.c.sweep().unwrap();
    assert_eq!(
        holders(&e),
        [("check/mac/slot-1".to_string(), RUN1.to_string())]
    );
}

#[test]
fn slot_arguments_are_checked() {
    let e = Env::new();
    assert!(e.c.acquire_slot(RUN1, "check/mac", 0, mins(1)).is_err());
    assert!(
        e.c.acquire_slot(RUN1, "check/mac", MAX_SLOTS + 1, mins(1))
            .is_err()
    );
    assert!(e.c.acquire_slot(RUN1, "port:8080", 2, mins(1)).is_err());
    assert!(e.c.acquire_slot(RUN1, "Check Mac", 2, mins(1)).is_err());
    assert!(e.c.acquire_slot("", "check/mac", 2, mins(1)).is_err());
    assert!(is_run_holder(RUN1));
    assert!(!is_run_holder(ALICE));
    assert!(!is_run_holder("run-x"));
}
