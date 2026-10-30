//! I:internal/inbox/untaken_test.go: MaxWakeTries and the append fallback
//! (card #23), and appended wakes.

mod common;

use std::collections::HashSet;

use common::*;
use inbox::{MAX_WAKE_TRIES, SendOptions, mode, path, st};

#[test]
fn unconfirmed_wake_falls_back_to_append() {
    let mut h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    let d = h.send_with(
        &person(),
        &to("beta"),
        "done",
        SendOptions {
            mode: mode::WAKE.into(),
            ref_: "task:01M62BP2MKFWJ4FK189419DNRV".into(),
            notice: true,
            reason: "answered".into(),
            ..Default::default()
        },
    );
    let mut tokens = HashSet::new();
    for i in 0..MAX_WAKE_TRIES {
        let w = h.wake(KEY_B);
        assert!(
            w.delivery_ids == vec![d.id.clone()] && !tokens.contains(&w.token),
            "try {i}: {w:?}"
        );
        tokens.insert(w.token);
        h.no_append(KEY_B);
        if i == 1 {
            h.reopen(); // the count is folded from wake.issued rows
            h.ib.set_reachable(KEY_B, path::MOD, true);
        }
        h.reclaim_all();
        h.want_state(&d.id, st::QUEUED);
    }
    h.no_wake(KEY_B);
    assert!(h.ib.outstanding_wake(KEY_B).is_none());
    let a = h.append_for(KEY_B);
    assert!(
        a.token.starts_with("[irori-append:")
            && a.mode == mode::NEXT_TURN
            && a.delivery_ids == vec![d.id.clone()]
    );
    assert!(a.prompt.contains("reason=\"answered\"") && a.prompt.contains("No turn was started"));
    h.ib.mark_delivered(KEY_B, &a.token).unwrap();
    let got = h.want_state(&d.id, st::DELIVERED);
    assert!(got.wake_id == a.id && got.via == path::MOD);
    h.ib.mark_engaged(KEY_B, &a.token, "turn-human").unwrap();
    assert_eq!(h.want_state(&d.id, st::SKIPPED).note, "notice");
}

#[test]
fn appended_wake_engages_without_a_hop() {
    let h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "q");
    let w = h.wake(KEY_B);
    h.ib.mark_delivered(KEY_B, &w.token).unwrap();
    h.ib.mark_appended_engaged(KEY_B, &w.token, "turn-human")
        .unwrap();
    h.want_state(&d.id, st::ENGAGED);
    assert_eq!(h.ib.running_wake(KEY_B), "");
    let (m, _) = h.send(
        &claude(KEY_B, "beta"),
        &to("gamma"),
        "from the person's turn",
    );
    assert!(m.hop == 0 && m.parent_id.is_empty());
    let (_, d2) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "q2");
    let w2 = h.deliver_engage(KEY_B, "t2");
    assert!(w2.delivery_ids.contains(&d2.id));
    let (m, _) = h.send(&claude(KEY_B, "beta"), &to("gamma"), "from the woken turn");
    assert_eq!(m.hop, 1);
}

#[test]
fn appended_wake_does_not_gate_the_next() {
    let mut h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    let (_, d1) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "refused");
    let w1 = h.wake(KEY_B);
    h.ib.mark_appended_delivered(KEY_B, &w1.token).unwrap();
    assert_eq!(h.want_state(&d1.id, st::DELIVERED).wake_id, w1.id);
    assert!(h.ib.outstanding_wake(KEY_B).is_none());
    h.reopen();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    let (_, d2) = h.send(&claude(KEY_C, "gamma"), &to("beta"), "next");
    assert_eq!(h.wake(KEY_B).delivery_ids, vec![d2.id]);

    let h2 = Harness::new();
    h2.abc();
    h2.ib.set_reachable(KEY_B, path::MOD, true);
    h2.send(&claude(KEY_A, "alpha"), &to("beta"), "queued, then dropped");
    let w = h2.wake(KEY_B);
    h2.ib.mark_delivered(KEY_B, &w.token).unwrap();
    let (_, s) = h2
        .ib
        .outstanding_wake(KEY_B)
        .expect("pending submit should gate");
    assert_eq!(s, st::DELIVERED);
    h2.ib.mark_appended_delivered(KEY_B, &w.token).unwrap();
    assert!(h2.ib.outstanding_wake(KEY_B).is_none());
    h2.ib
        .mark_appended_engaged(KEY_B, &w.token, "turn-human")
        .unwrap();
    assert_eq!(h2.want_state(&w.delivery_ids[0], st::ENGAGED).wake_id, w.id);
}

#[test]
fn untaken_item_rides_the_next_wake() {
    let h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    let (_, stuck) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "first");
    for _ in 0..MAX_WAKE_TRIES {
        h.wake(KEY_B);
        h.reclaim_all();
    }
    h.append_for(KEY_B);
    h.reclaim_all();
    h.no_wake(KEY_B);
    let (_, fresh) = h.send(&claude(KEY_C, "gamma"), &to("beta"), "second");
    let w = h.wake(KEY_B);
    assert_eq!(w.delivery_ids, vec![fresh.id.clone(), stuck.id.clone()]);
    h.ib.mark_delivered(KEY_B, &w.token).unwrap();
    h.want_state(&stuck.id, st::DELIVERED);
    h.ib.mark_engaged(KEY_B, &w.token, "t1").unwrap();
    h.want_state(&stuck.id, st::ENGAGED);
    h.want_state(&fresh.id, st::ENGAGED);
}

#[test]
fn wake_tries_restart_when_requeued() {
    let h = Harness::new();
    h.abc();
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "q");
    for _ in 0..MAX_WAKE_TRIES - 1 {
        h.wake(KEY_B);
        h.reclaim_all();
    }
    let w = h.deliver_engage(KEY_B, "t1");
    h.ib.defer(KEY_B, &d.id, 1, "").unwrap();
    h.ib.turn_ended(KEY_B, &w.token, "t1").unwrap();
    h.clk.advance(mins(1) + secs(1));
    h.sweep();
    h.want_state(&d.id, st::QUEUED);
    for _ in 0..MAX_WAKE_TRIES {
        h.wake(KEY_B);
        h.reclaim_all();
    }
    h.no_wake(KEY_B);
    h.append_for(KEY_B);
}

#[test]
fn channel_pushes_keep_repeating() {
    let h = Harness::new();
    h.abc();
    h.ros.put(&[grok_card(KEY_G, "grok", "")]);
    h.ib.set_reachable(KEY_G, path::CHANNEL, true);
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to("grok"), "q");
    for _ in 0..MAX_WAKE_TRIES + 1 {
        let w = h.ib.next_wake(KEY_G, path::CHANNEL).unwrap().expect("push");
        assert_eq!(w.delivery_ids[0], d.id);
        h.reclaim_all();
    }
}

#[test]
fn wake_landing_mid_turn() {
    let h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    h.send(&claude(KEY_A, "alpha"), &to("beta"), "first");
    let w1 = h.deliver_engage(KEY_B, "t1");
    assert_eq!(h.ib.running_wake(KEY_B), w1.id);
    let (_, d) = h.send(&claude(KEY_C, "gamma"), &to("beta"), "while you work");
    let w2 = h.wake(KEY_B);
    assert_eq!(w2.delivery_ids, vec![d.id.clone()]);
    h.ib.mark_delivered(KEY_B, &w2.token).unwrap();
    h.want_state(&d.id, st::DELIVERED);
    h.ib.turn_ended(KEY_B, &w1.token, "t1").unwrap();
    h.ib.mark_engaged(KEY_B, &w2.token, "t2").unwrap();
    h.want_state(&d.id, st::ENGAGED);
    let (_, d2) = h.send(
        &claude(KEY_C, "gamma"),
        &to("beta"),
        "dropped while you work",
    );
    for _ in 0..MAX_WAKE_TRIES {
        h.wake(KEY_B);
        h.reclaim_all();
    }
    h.no_wake(KEY_B);
    let a = h.append_for(KEY_B);
    h.ib.mark_delivered(KEY_B, &a.token).unwrap();
    h.ib.turn_ended(KEY_B, &w2.token, "t2").unwrap();
    h.ib.mark_engaged(KEY_B, &a.token, "t3").unwrap();
    h.want_state(&d2.id, st::ENGAGED);
}
