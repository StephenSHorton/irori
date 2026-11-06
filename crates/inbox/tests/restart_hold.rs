//! Keep this device up to date (card #578): before the app restarts itself
//! it asks whether a wake is coming, and holds wakes back for its last look.

mod common;

use common::*;
use inbox::{ChangeKind, Policy, path};

#[test]
fn a_queued_wake_is_coming_until_it_is_engaged() {
    let h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    assert!(!h.ib.wake_coming(KEY_B), "nothing sent");
    h.send(&person(), &to("beta"), "hello");
    assert!(h.ib.wake_coming(KEY_B), "queued");
    let w = h.wake(KEY_B);
    assert!(h.ib.wake_coming(KEY_B), "out, not yet delivered");
    h.ib.mark_delivered(KEY_B, &w.token).unwrap();
    assert!(h.ib.wake_coming(KEY_B), "delivered, not yet engaged");
    h.ib.mark_engaged(KEY_B, &w.token, "t1").unwrap();
    assert!(!h.ib.wake_coming(KEY_B), "engaged: presence shows the turn");
}

#[test]
fn a_queued_wake_stops_counting_after_a_while() {
    let h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    h.send(&person(), &to("beta"), "hello");
    h.clk.advance(inbox::NOT_ANSWERING_AFTER + secs(1));
    assert!(
        !h.ib.wake_coming(KEY_B),
        "a session that stopped listening must not hold a restart for good"
    );
}

#[test]
fn a_session_nobody_can_reach_is_not_about_to_be_woken() {
    let h = Harness::new();
    h.abc();
    h.send(&person(), &to("beta"), "hello");
    assert!(!h.ib.wake_coming(KEY_B));
}

#[test]
fn pause_all_means_no_wake_is_coming() {
    let h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    h.ib.set_policy(
        Policy {
            mode: String::new(),
            block: vec![],
            allow: vec![],
            paused: true,
        },
        "user",
    )
    .unwrap();
    h.send(&person(), &to("beta"), "hello");
    assert!(!h.ib.wake_coming(KEY_B));
}

#[test]
fn held_wakes_wait_and_go_when_let_go() {
    let h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    let sub = h.ib.subscribe();
    h.ib.hold_wakes(true);
    assert!(h.ib.wakes_held());
    h.send(&person(), &to("beta"), "hello");
    h.no_wake(KEY_B);
    sub.drain();
    h.ib.hold_wakes(false);
    let told = sub.drain();
    assert_eq!(
        count(&told, ChangeKind::WakeReady, KEY_B),
        1,
        "the agent's stream must hear again, or the wake sits until the next change"
    );
    h.wake(KEY_B);
}

#[test]
fn a_hold_is_not_saved() {
    let mut h = Harness::new();
    h.abc();
    h.ib.hold_wakes(true);
    h.reopen();
    assert!(!h.ib.wakes_held(), "a crash must not leave wakes held");
}
