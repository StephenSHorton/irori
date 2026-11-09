//! Card #1213 (agent-continuity §5, A.2): the one check the hand-out and a
//! thread's card share. An agent is reusable when its context is at or
//! under the person's budget, it never compacted, and its last call was
//! under 55 minutes ago by the fold's own clock.

use crate::common;

use std::sync::{Arc, RwLock};
use std::time::Duration;

use common::*;
use coord::*;
use gocompat::time::GoTime;

fn with_reuse(e: &Env, r: &Arc<RwLock<Option<Reuse>>>) -> Coord {
    let r = r.clone();
    e.reopen_with(Options {
        reusable: Some(Arc::new(move |_: &str| *r.read().unwrap())),
        ..e.opts()
    })
}

fn at(secs_ago: i64) -> GoTime {
    GoTime::unix(fixed_now().secs - secs_ago, 0)
}

fn reuse(ctx: u64, budget: i64) -> Reuse {
    Reuse {
        ctx,
        ctx_at: at(0),
        compacted: false,
        budget,
    }
}

fn set(r: &Arc<RwLock<Option<Reuse>>>, v: Reuse) {
    *r.write().unwrap() = Some(v);
}

#[test]
fn with_nothing_wired_or_no_figure_an_agent_is_reusable() {
    let e = Env::new();
    assert!(e.c.reusable(BOB));
    let r = Arc::new(RwLock::new(None));
    let c = with_reuse(&e, &r);
    assert!(c.reusable(BOB), "no figure yet: a fresh agent");
}

#[test]
fn at_the_budget_it_stays_and_one_token_over_it_retires() {
    let e = Env::new();
    let r = Arc::new(RwLock::new(None));
    let c = with_reuse(&e, &r);
    set(&r, reuse(120_000, 120_000));
    assert!(c.reusable(BOB), "at the budget");
    set(&r, reuse(120_001, 120_000));
    assert!(!c.reusable(BOB), "over the budget");
}

#[test]
fn a_budget_of_zero_is_the_200k_default() {
    let e = Env::new();
    let r = Arc::new(RwLock::new(None));
    let c = with_reuse(&e, &r);
    set(&r, reuse(200_000, 0));
    assert!(c.reusable(BOB));
    set(&r, reuse(200_001, 0));
    assert!(!c.reusable(BOB));
}

#[test]
fn always_fresh_never_reuses_even_a_new_agent() {
    let e = Env::new();
    let r = Arc::new(RwLock::new(None));
    let c = with_reuse(&e, &r);
    set(&r, reuse(1, -1));
    assert!(!c.reusable(BOB));
}

#[test]
fn a_compacted_agent_is_worn_out_however_small_it_reads() {
    let e = Env::new();
    let r = Arc::new(RwLock::new(None));
    let c = with_reuse(&e, &r);
    set(
        &r,
        Reuse {
            compacted: true,
            ..reuse(5_000, 200_000)
        },
    );
    assert!(!c.reusable(BOB));
}

#[test]
fn it_is_warm_until_55_minutes_by_the_folds_own_clock() {
    let e = Env::new();
    let r = Arc::new(RwLock::new(None));
    let c = with_reuse(&e, &r);
    set(&r, reuse(50_000, 200_000));
    assert!(c.reusable(BOB), "just called");
    e.clk.add(Duration::from_secs(55 * 60 - 1));
    assert!(c.reusable(BOB), "54:59 ago");
    e.clk.add(Duration::from_secs(1));
    assert!(!c.reusable(BOB), "55:00 ago is cold");
    e.clk.add(Duration::from_secs(3600));
    assert!(!c.reusable(BOB));
}

#[test]
fn a_figure_with_no_call_time_counts_as_warm() {
    let e = Env::new();
    let r = Arc::new(RwLock::new(None));
    let c = with_reuse(&e, &r);
    set(
        &r,
        Reuse {
            ctx_at: GoTime::default(),
            ..reuse(50_000, 200_000)
        },
    );
    assert!(c.reusable(BOB));
}
