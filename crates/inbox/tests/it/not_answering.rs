//! Card #110: how long a wake item has waited on an agent unread, so the
//! Core can show an idle session that stopped reading as not answering.

use crate::common;

use common::*;
use inbox::{NOT_ANSWERING_AFTER, path, st};

#[test]
fn a_wake_left_unread_counts_from_when_it_began_to_wait() {
    let h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    assert!(h.ib.unanswered_since(KEY_B, NOT_ANSWERING_AFTER).is_none());
    let start = h.clk.now();
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "q");

    // Woken, delivered, never engaged: re-sends and deliveries don't move
    // the clock.
    let w = h.wake(KEY_B);
    h.clk.advance(mins(3));
    h.ib.mark_delivered(KEY_B, &w.token).unwrap();
    h.want_state(&d.id, st::DELIVERED);
    assert!(
        h.ib.unanswered_since(KEY_B, NOT_ANSWERING_AFTER).is_none(),
        "not yet {NOT_ANSWERING_AFTER:?}"
    );
    h.clk.advance(mins(3));
    assert_eq!(
        h.ib.unanswered_since(KEY_B, NOT_ANSWERING_AFTER),
        Some(start),
        "waiting since it was sent"
    );
    assert!(
        h.ib.unanswered_since(KEY_C, NOT_ANSWERING_AFTER).is_none(),
        "nothing waits on gamma"
    );

    // It takes the wake up: nothing waits any more.
    h.ib.mark_engaged(KEY_B, &w.token, "turn-1").unwrap();
    assert!(h.ib.unanswered_since(KEY_B, NOT_ANSWERING_AFTER).is_none());
}

#[test]
fn a_wake_sent_again_and_again_keeps_its_clock() {
    // A session that never confirms a wake: each is taken back and sent
    // again, and the clock keeps running from the first.
    let h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    let start = h.clk.now();
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "q");
    for _ in 0..3 {
        h.wake(KEY_B);
        h.reclaim_all();
        h.want_state(&d.id, st::QUEUED);
    }
    assert!(h.clk.now().secs - start.secs > NOT_ANSWERING_AFTER.as_secs() as i64);
    assert_eq!(
        h.ib.unanswered_since(KEY_B, NOT_ANSWERING_AFTER),
        Some(start)
    );
}
