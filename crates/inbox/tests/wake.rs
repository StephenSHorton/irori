//! I:internal/inbox/wake_test.go and nextturn_test.go.

mod common;

use common::*;
use inbox::{
    ChangeKind, DIRECT_LIMIT, ErrorKind, GONE_GRACE, MAX_REF, MAX_TEXT, MAX_WAKE_BYTES,
    QUIET_LIMIT, REATTACH_GRACE, SYSTEM_KEY, SendOptions, UNENGAGED_LIMIT, kinds, mode, path, st,
    system_sender, trust,
};
use store::Filter;

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[test]
fn wake_batching_order_and_size() {
    let h = Harness::new();
    h.abc();
    let mut ids = vec![];
    let big = "x".repeat(6000);
    for (i, from) in [
        claude(KEY_A, "alpha"),
        claude(KEY_C, "gamma"),
        claude(KEY_A, "alpha"),
        person(),
    ]
    .iter()
    .enumerate()
    {
        let (_, d) = h.send(
            from,
            &to("beta"),
            &format!("{big}{}", (b'a' + i as u8) as char),
        );
        ids.push(d.id);
        h.clk.advance(secs(1));
    }
    let w1 = h.deliver_engage(KEY_B, "t1");
    assert!(w1.prompt.len() <= MAX_WAKE_BYTES && w1.delivery_ids == ids && w1.overflow == ids[2..]);
    assert!(w1.prompt.contains("4 messages"));
    assert!(w1.prompt.contains(&format!(
        "2 more waiting: read them with the irori inbox tool: {}, {}\n",
        ids[2], ids[3]
    )));
    assert!(
        !w1.prompt.contains(&format!("delivery=\"{}\"", ids[2]))
            && !w1.prompt.contains(&format!("{big}c"))
    );
    for id in &ids {
        h.want_state(id, st::ENGAGED);
    }
    h.no_wake(KEY_B);
    let huge = "\u{1F600}".repeat(MAX_TEXT);
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to("gamma"), &huge);
    let (_, small) = h.send(&claude(KEY_A, "alpha"), &to("gamma"), "small");
    let w = h.deliver_engage(KEY_C, "t2");
    assert!(w.delivery_ids == vec![d.id.clone()] && w.overflow.is_empty());
    let w2 = h.wake(KEY_C);
    assert!(w2.delivery_ids == vec![small.id.clone()] && w2.prompt.contains("trust=\"untrusted\""));
}

#[test]
fn overflow_capped_and_rebuilt() {
    let mut h = Harness::new();
    h.abc();
    let big = "y".repeat(7000);
    let mut ids = vec![];
    for i in 0..603 {
        let text = if i < 3 { big.as_str() } else { "s" };
        ids.push(h.send(&person(), &to("beta"), text).1.id);
    }
    let w = h.wake(KEY_B);
    assert!(
        w.prompt.len() <= MAX_WAKE_BYTES
            && !w.overflow.is_empty()
            && w.delivery_ids.len() < ids.len()
    );
    let (ow, s0) = h.ib.outstanding_wake(KEY_B).unwrap();
    assert!(s0 == st::QUEUED && ow.prompt == w.prompt && ow.overflow == w.overflow);
    h.reopen();
    assert_eq!(h.ib.outstanding_wake(KEY_B).unwrap().0.prompt, w.prompt);
    h.ib.mark_engaged(KEY_B, &w.token, "t").unwrap();
    let rest = &ids[w.delivery_ids.len()..];
    h.want_state(&rest[0], st::QUEUED);
    assert_eq!(h.wake(KEY_B).delivery_ids[0], rest[0]);
}

#[test]
fn nothing_to_wake() {
    let h = Harness::new();
    h.abc();
    h.no_wake(KEY_B);
    assert!(h.ib.next_wake("", path::MOD).is_err());
    h.ros.put(&[grok_card(KEY_G, "grok", "")]);
    h.send(&grok(KEY_G, "grok"), &to("beta"), "held");
    h.no_wake(KEY_B);
}

#[test]
fn reclaim_unconfirmed_wake() {
    let h = Harness::new();
    h.abc();
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "q");
    let w = h.wake(KEY_B);
    h.clk.advance(REATTACH_GRACE);
    h.sweep();
    h.no_wake(KEY_B);
    h.clk.advance(secs(1));
    h.sweep();
    assert_eq!(h.want_state(&d.id, st::QUEUED).wake_id, "");
    let w2 = h.wake(KEY_B);
    assert!(w2.id != w.id && w2.delivery_ids[0] == d.id);
    h.ib.mark_delivered(KEY_B, &w.token).unwrap();
    assert_eq!(h.want_state(&d.id, st::QUEUED).wake_id, w2.id);
    h.ib.mark_delivered(KEY_B, &w2.token).unwrap();
    h.want_state(&d.id, st::DELIVERED);
}

#[test]
fn late_confirm_after_reclaim_counts() {
    let h = Harness::new();
    h.abc();
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "q");
    let w = h.wake(KEY_B);
    h.clk.advance(REATTACH_GRACE + secs(1));
    h.sweep();
    h.want_state(&d.id, st::QUEUED);
    h.ib.mark_delivered(KEY_B, &w.token).unwrap();
    assert_eq!(h.want_state(&d.id, st::DELIVERED).wake_id, w.id);
    h.no_wake(KEY_B);
    let (_, d2) = h.send(&claude(KEY_C, "gamma"), &to("alpha"), "q2");
    let w2 = h.wake(KEY_A);
    h.ib.mark_engaged(KEY_A, &w2.token, "t").unwrap();
    h.want_state(&d2.id, st::ENGAGED);
}

#[test]
fn stale_delivered_wake_stops_gating() {
    let h = Harness::new();
    h.abc();
    h.send(&claude(KEY_A, "alpha"), &to("beta"), "q");
    let w = h.wake(KEY_B);
    h.ib.mark_delivered(KEY_B, &w.token).unwrap();
    let sub = h.ib.subscribe();
    h.send(&claude(KEY_C, "gamma"), &to("beta"), "q2");
    h.no_wake(KEY_B);
    assert_eq!(count(&sub.drain(), ChangeKind::WakeReady, KEY_B), 0);
    h.clk.advance(UNENGAGED_LIMIT);
    h.sweep();
    assert_eq!(count(&sub.drain(), ChangeKind::WakeReady, KEY_B), 1);
    h.sweep();
    assert_eq!(count(&sub.drain(), ChangeKind::WakeReady, KEY_B), 0);
    assert_eq!(h.wake(KEY_B).delivery_ids.len(), 1);
}

#[test]
fn reminder_wake() {
    let h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "q");
    let w = h.deliver_engage(KEY_B, "t1");
    h.ib.turn_ended(KEY_B, &w.token, "t1").unwrap();
    let sub = h.ib.subscribe();
    h.clk.advance(QUIET_LIMIT);
    h.sweep();
    h.no_wake(KEY_B);
    h.clk.advance(secs(1));
    h.sweep();
    assert_eq!(count(&sub.drain(), ChangeKind::WakeReady, KEY_B), 1);
    let r = h.wake(KEY_B);
    assert!(
        r.reminder
            && r.token.starts_with("[irori-reminder:wake_")
            && r.prompt.matches(&r.token).count() == 1
    );
    assert!(!r.prompt.contains("[irori-wake:") && r.prompt.contains(&d.id));
    assert!(h.state(&d.id).reminded);
    h.ib.mark_delivered(KEY_B, &r.token).unwrap();
    h.ib.mark_engaged(KEY_B, &r.token, "t2").unwrap();
    h.ib.turn_ended(KEY_B, &r.token, "t2").unwrap();
    h.want_state(&d.id, st::ENGAGED);
    assert_eq!(h.ib.running_wake(KEY_B), "");
    h.clk.advance(QUIET_LIMIT * 3);
    h.sweep();
    h.no_wake(KEY_B);
    h.ib.defer(KEY_B, &d.id, 10, "after lunch").unwrap();
    assert!(!h.state(&d.id).reminded);
}

#[test]
fn unreachable_is_not_reminded() {
    let h = Harness::new();
    h.abc();
    h.send(&claude(KEY_A, "alpha"), &to("beta"), "q");
    h.deliver_engage(KEY_B, "t1");
    h.clk.advance(QUIET_LIMIT + mins(1));
    h.sweep();
    h.no_wake(KEY_B);
}

#[test]
fn deferral_due() {
    let h = Harness::new();
    h.abc();
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "q");
    h.deliver_engage(KEY_B, "t1");
    h.ib.defer(KEY_B, &d.id, 30, "later").unwrap();
    let sub = h.ib.subscribe();
    h.clk.advance(mins(29));
    h.sweep();
    h.want_state(&d.id, st::DEFERRED);
    h.clk.advance(mins(1));
    h.sweep();
    h.want_state(&d.id, st::QUEUED);
    assert_eq!(count(&sub.drain(), ChangeKind::WakeReady, KEY_B), 1);
    assert_eq!(h.wake(KEY_B).delivery_ids[0], d.id);
}

#[test]
fn subscribe_edges() {
    let h = Harness::new();
    h.abc();
    h.ros.put(&[grok_card(KEY_G, "grok", "")]);
    let sub = h.ib.subscribe();
    h.send(&claude(KEY_A, "alpha"), &to("beta"), "1");
    h.send(&claude(KEY_C, "gamma"), &to("beta"), "2");
    let cs = sub.drain();
    assert_eq!(count(&cs, ChangeKind::WakeReady, KEY_B), 1);
    assert!(count(&cs, ChangeKind::Updated, KEY_B) >= 2);
    let w = h.wake(KEY_B);
    h.ib.mark_delivered(KEY_B, &w.token).unwrap();
    sub.drain();
    h.send(&claude(KEY_A, "alpha"), &to("beta"), "3");
    assert_eq!(count(&sub.drain(), ChangeKind::WakeReady, KEY_B), 0);
    h.ib.mark_engaged(KEY_B, &w.token, "t").unwrap();
    assert_eq!(count(&sub.drain(), ChangeKind::WakeReady, KEY_B), 1);
    let (_, hd) = h.send(&grok(KEY_G, "grok"), &to("beta"), "held");
    let cs = sub.drain();
    assert!(
        count(&cs, ChangeKind::HeldNew, KEY_B) == 1 && cs.last().unwrap().delivery_id == hd.id,
        "{cs:?}"
    );
    h.wake(KEY_B);
    let (w3, _) = h.ib.outstanding_wake(KEY_B).unwrap();
    h.ib.mark_engaged(KEY_B, &w3.token, "t3").unwrap();
    sub.drain();
    h.ib.release(&hd.id, USER).unwrap();
    assert_eq!(count(&sub.drain(), ChangeKind::WakeReady, KEY_B), 1);
    // Subscribers end when cancelled.
    let s2 = h.ib.subscribe();
    s2.cancel();
    assert!(s2.recv().is_none());
}

#[test]
fn turn_ended_frees_gate_for_queued_work() {
    let h = Harness::new();
    h.abc();
    h.send(&claude(KEY_A, "alpha"), &to("beta"), "1");
    let w = h.deliver_engage(KEY_B, "t1");
    let sub = h.ib.subscribe();
    h.ib.turn_ended(KEY_B, &w.token, "t1").unwrap();
    assert_eq!(count(&sub.drain(), ChangeKind::WakeReady, KEY_B), 0);
    want_err(
        h.ib.turn_ended(KEY_B, "[irori-wake:nope]", "t1"),
        ErrorKind::NotFound,
    );
    want_err(
        h.ib.mark_engaged(KEY_A, &w.token, "t1"),
        ErrorKind::NotYours,
    );
}

#[test]
fn wake_item_rides_append_behind_delivered_wake() {
    let mut h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    h.send(&claude(KEY_A, "alpha"), &to("beta"), "first");
    let w1 = h.wake(KEY_B);
    let (_, dq) = h.send(&claude(KEY_C, "gamma"), &to("beta"), "too early");
    h.no_append(KEY_B);
    h.ib.mark_delivered(KEY_B, &w1.token).unwrap();
    let sub = h.ib.subscribe();
    let d2 = h.send_with(
        &person(),
        &to("beta"),
        "the person, as a wake",
        opts(mode::WAKE),
    );
    h.no_wake(KEY_B);
    assert_eq!(count(&sub.drain(), ChangeKind::AppendReady, KEY_B), 0);
    let a = h.append_for(KEY_B);
    assert!(
        a.delivery_ids == vec![dq.id.clone(), d2.id.clone()] && a.mode == mode::NEXT_TURN,
        "{a:?}"
    );
    h.ib.mark_delivered(KEY_B, &a.token).unwrap();
    let got = h.want_state(&d2.id, st::DELIVERED);
    assert!(got.wake_id == a.id && got.mode == mode::WAKE);
    h.ib.mark_engaged(KEY_B, &w1.token, "t1").unwrap();
    h.ib.mark_engaged(KEY_B, &a.token, "t1").unwrap();
    h.want_state(&d2.id, st::ENGAGED);
    h.ib.turn_ended(KEY_B, &w1.token, "t1").unwrap();
    h.no_wake(KEY_B);
    h.no_append(KEY_B);
    drop(sub);
    h.reopen();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    h.no_wake(KEY_B);
    h.no_append(KEY_B);
}

#[test]
fn wake_item_waits_once_delivered_wake_goes_stale() {
    let h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    h.send(&claude(KEY_A, "alpha"), &to("beta"), "q");
    let w = h.wake(KEY_B);
    h.ib.mark_delivered(KEY_B, &w.token).unwrap();
    h.clk.advance(UNENGAGED_LIMIT);
    let (_, d) = h.send(&claude(KEY_C, "gamma"), &to("beta"), "q2");
    h.no_append(KEY_B);
    assert_eq!(h.wake(KEY_B).delivery_ids, vec![d.id]);
}

// ---- nextturn_test.go ------------------------------------------------------

#[test]
fn send_with_modes() {
    let cases = [
        (
            "a person defaults to next_turn",
            person(),
            "",
            mode::NEXT_TURN,
        ),
        (
            "Irori defaults to next_turn",
            system_sender(),
            "",
            mode::NEXT_TURN,
        ),
        (
            "an agent defaults to wake",
            claude(KEY_A, "alpha"),
            "",
            mode::WAKE,
        ),
        (
            "a grok member defaults to wake",
            grok(KEY_G, "grok"),
            "",
            mode::WAKE,
        ),
        ("a person may choose wake", person(), mode::WAKE, mode::WAKE),
        (
            "an agent may choose next_turn",
            claude(KEY_A, "alpha"),
            mode::NEXT_TURN,
            mode::NEXT_TURN,
        ),
    ];
    for (name, from, m, want) in cases {
        let mut h = Harness::new();
        h.abc();
        h.ros.put(&[grok_card(KEY_G, "grok", "")]);
        let d = h.send_with(&from, &to(KEY_C), "hi", opts(m));
        assert_eq!(d.mode, want, "{name}");
        h.reopen();
        assert_eq!(h.state(&d.id).mode, want, "{name} after replay");
    }
    let h = Harness::new();
    h.abc();
    assert!(
        h.ib.send_with(&person(), &to("beta"), "hi", opts("later"))
            .is_err()
    );
    let (m1, d1) = h.ib.send(&person(), &to("beta"), "a", "why").unwrap();
    let (m2, d2) =
        h.ib.send_with(
            &person(),
            &to("beta"),
            "a",
            SendOptions {
                reason: "why".into(),
                mode: mode::WAKE.into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        d1.mode == mode::WAKE
            && d2.mode == d1.mode
            && m1.reason == m2.reason
            && m1.trust == m2.trust
            && d1.state == d2.state
    );
    let d = h.send_with(
        &claude(KEY_A, "alpha"),
        &to("beta"),
        "card moved",
        SendOptions {
            ref_: " task:01ABC ".into(),
            notice: true,
            mode: mode::WAKE.into(),
            ..Default::default()
        },
    );
    assert!(d.message.ref_ == "task:01ABC" && d.message.notice);
    let w = h.wake(KEY_B);
    for want in [
        "ref=\"task:01ABC\"",
        "notice=\"true\"",
        "needs no reply",
        "show tool",
    ] {
        assert!(w.prompt.contains(want), "lacks {want}");
    }
    let long = h.send_with(
        &claude(KEY_A, "alpha"),
        &to("beta"),
        "x",
        SendOptions {
            ref_: format!("task:{}", "z".repeat(400)),
            ..Default::default()
        },
    );
    assert_eq!(long.message.ref_.chars().count(), MAX_REF);
}

#[test]
fn system_sender_rules() {
    let mut h = Harness::new();
    h.abc();
    let sys = system_sender();
    for i in 0..DIRECT_LIMIT + 3 {
        let d = h.send_with(
            &sys,
            &to("beta"),
            &format!("notice {}", (b'a' + i as u8) as char),
            SendOptions {
                notice: true,
                ..Default::default()
            },
        );
        assert!(
            d.state == st::QUEUED
                && d.message.trust == trust::UNTRUSTED
                && d.mode == mode::NEXT_TURN
        );
    }
    want_err(
        h.ib.send(&claude(KEY_A, "alpha"), &to("irori"), "hi", ""),
        ErrorKind::UnknownAgent,
    );
    want_err(
        h.ib.send(&claude(KEY_A, "alpha"), &to(SYSTEM_KEY), "hi", ""),
        ErrorKind::UnknownAgent,
    );
    assert!(h.ib.aliases(SYSTEM_KEY).is_empty());
    let d = h.ib.for_agent(KEY_B, false, 1)[0].clone();
    want_err(
        h.ib.reply(&claude(KEY_B, "beta"), &d.id, "thanks"),
        ErrorKind::BadState,
    );
    h.reopen();
    want_err(
        h.ib.send(&claude(KEY_A, "alpha"), &to("irori"), "hi", ""),
        ErrorKind::UnknownAgent,
    );
}

#[test]
fn append_flow() {
    let mut h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    let sub = h.ib.subscribe();
    let d1 = h.send_with(&person(), &to("beta"), "read this when you can", opts(""));
    h.no_wake(KEY_B);
    let cs = sub.drain();
    assert!(
        count(&cs, ChangeKind::WakeReady, KEY_B) == 0
            && count(&cs, ChangeKind::AppendReady, KEY_B) == 1,
        "{cs:?}"
    );
    let w = h.append_for(KEY_B);
    assert!(
        w.token.starts_with("[irori-append:")
            && w.mode == mode::NEXT_TURN
            && !w.reminder
            && w.delivery_ids == vec![d1.id.clone()]
            && w.overflow.is_empty()
    );
    assert!(w.prompt.matches(&w.token).count() == 1 && w.prompt.starts_with(&w.token));
    assert!(
        w.prompt.contains("mode=\"next_turn\"")
            && w.prompt.contains("No turn was started")
            && w.prompt.contains("trust=\"user\"")
    );
    let d2 = h.send_with(&person(), &to("beta"), "and this", opts(""));
    h.no_append(KEY_B);
    assert_eq!(count(&sub.drain(), ChangeKind::AppendReady, KEY_B), 0);
    assert!(h.ib.outstanding_wake(KEY_B).is_none());
    let oa = h.ib.outstanding_append(KEY_B).unwrap();
    assert!(oa.token == w.token && oa.prompt == w.prompt && oa.mode == mode::NEXT_TURN);
    drop(sub);
    h.reopen();
    let sub = h.ib.subscribe();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    let oa =
        h.ib.outstanding_append(KEY_B)
            .expect("outstanding append lost in replay");
    assert!(oa.token == w.token && oa.prompt == w.prompt);
    let (_, dw) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "wake up");
    let ww = h.wake(KEY_B);
    assert_eq!(ww.delivery_ids, vec![dw.id.clone(), d2.id.clone()]);
    sub.drain();
    h.ib.mark_delivered(KEY_B, &w.token).unwrap();
    let got = h.want_state(&d1.id, st::DELIVERED);
    assert!(got.via == path::MOD && got.wake_id == w.id);
    assert!(h.ib.outstanding_append(KEY_B).is_none());
    h.ib.mark_engaged(KEY_B, &w.token, "turn-p").unwrap();
    h.want_state(&d1.id, st::ENGAGED);
    assert_eq!(h.ib.running_wake(KEY_B), "");
    let (m, _) =
        h.ib.send(&claude(KEY_B, "beta"), &to("gamma"), "from that turn", "")
            .unwrap();
    assert!(m.hop == 0 && m.parent_id.is_empty());
    h.ib.turn_ended(KEY_B, &w.token, "turn-p").unwrap();
    h.ib.reply(&claude(KEY_B, "beta"), &d1.id, "on it").unwrap();
    h.want_state(&d1.id, st::ANSWERED);
}

#[test]
fn append_after_delivered_and_edges() {
    let h = Harness::new();
    h.abc();
    let sub = h.ib.subscribe();
    h.send_with(&person(), &to("beta"), "one", opts(""));
    let w = h.append_for(KEY_B);
    let d2 = h.send_with(&person(), &to("beta"), "two", opts(""));
    sub.drain();
    h.ib.mark_delivered(KEY_B, &w.token).unwrap();
    assert_eq!(count(&sub.drain(), ChangeKind::AppendReady, KEY_B), 1);
    let w2 = h.append_for(KEY_B);
    assert!(w2.token != w.token && w2.delivery_ids == vec![d2.id.clone()]);
    h.ib.mark_engaged(KEY_B, &w2.token, "t").unwrap();
    h.want_state(&d2.id, st::ENGAGED);
    h.no_append(KEY_B);
    assert!(h.ib.next_append("").is_err());
    want_err(
        h.ib.mark_engaged(KEY_A, &w2.token, "t"),
        ErrorKind::NotYours,
    );
}

#[test]
fn append_paused_and_held() {
    let h = Harness::new();
    h.abc();
    let paused = inbox::Policy {
        paused: true,
        ..Default::default()
    };
    h.ib.set_policy(paused.clone(), USER).unwrap();
    let d = h.send_with(&person(), &to("beta"), "later", opts(""));
    assert!(d.state == st::HELD && d.hold == inbox::hold::PAUSED);
    h.no_append(KEY_B);
    let sub = h.ib.subscribe();
    h.ib.set_policy(inbox::Policy::default(), USER).unwrap();
    assert_eq!(count(&sub.drain(), ChangeKind::AppendReady, KEY_B), 1);
    assert_eq!(h.append_for(KEY_B).delivery_ids, vec![d.id]);
    h.send_with(&person(), &to("gamma"), "x", opts(""));
    h.ib.set_policy(paused, USER).unwrap();
    h.no_append(KEY_C);
}

#[test]
fn next_turn_on_channel() {
    let h = Harness::new();
    h.abc();
    h.ros.put(&[grok_card(KEY_G, "grok", "")]);
    h.ib.set_reachable(KEY_G, path::CHANNEL, true);
    let d = h.send_with(&person(), &to("grok"), "fyi", opts(""));
    let w =
        h.ib.next_wake(KEY_G, path::CHANNEL)
            .unwrap()
            .expect("no push");
    assert!(
        w.mode == mode::NEXT_TURN
            && w.token.starts_with("[irori-wake:")
            && w.delivery_ids == vec![d.id.clone()]
    );
    h.ib.ack(KEY_G, &d.id).unwrap();
    h.want_state(&d.id, st::ENGAGED);
    let n = h.send_with(&person(), &to("grok"), "fyi 2", opts(""));
    let (_, wd) = h.send(&claude(KEY_A, "alpha"), &to("grok"), "now");
    let w2 = h.ib.next_wake(KEY_G, path::CHANNEL).unwrap().unwrap();
    assert!(w2.mode == mode::WAKE && w2.delivery_ids == vec![wd.id.clone(), n.id.clone()]);
    assert!(w2.prompt.contains("came along with this one"));
}

#[test]
fn notice_closes_on_engage() {
    let notice = || SendOptions {
        notice: true,
        ref_: "task:01X".into(),
        ..Default::default()
    };
    {
        let h = Harness::new();
        h.abc();
        let d = h.send_with(&system_sender(), &to("beta"), "blocker closed", notice());
        let plain = h.send_with(&person(), &to("beta"), "also", opts(""));
        let w = h.append_for(KEY_B);
        h.ib.mark_delivered(KEY_B, &w.token).unwrap();
        h.want_state(&d.id, st::DELIVERED);
        h.ib.mark_engaged(KEY_B, &w.token, "t").unwrap();
        assert_eq!(h.want_state(&d.id, st::SKIPPED).note, "notice");
        h.want_state(&plain.id, st::ENGAGED);
    }
    {
        let h = Harness::new();
        h.abc();
        let mut o = notice();
        o.mode = mode::WAKE.into();
        let d = h.send_with(&claude(KEY_A, "alpha"), &to("beta"), "assigned you", o);
        h.deliver_engage(KEY_B, "t");
        assert_eq!(h.want_state(&d.id, st::SKIPPED).note, "notice");
        assert_ne!(h.ib.running_wake(KEY_B), "");
    }
    {
        let h = Harness::new();
        h.abc();
        h.ros.put(&[grok_card(KEY_G, "grok", "")]);
        let d = h.send_with(&system_sender(), &to("grok"), "blocker closed", notice());
        h.ib.ack(KEY_G, &d.id).unwrap();
        assert_eq!(h.want_state(&d.id, st::SKIPPED).note, "notice");
        assert!(h.ib.for_agent(KEY_G, false, 0).is_empty());
    }
}

#[test]
fn next_turn_sweep() {
    let h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_A, path::MOD, true);
    h.ib.set_reachable(KEY_B, path::MOD, true);
    let d = h.send_with(&person(), &to_repo("repo1"), "when you get to it", opts(""));
    assert_eq!(d.to, KEY_A);
    let w = h.append_for(KEY_A);
    h.ib.mark_delivered(KEY_A, &w.token).unwrap();
    for _ in 0..5 {
        h.clk.advance(QUIET_LIMIT);
        h.sweep();
    }
    let got = h.want_state(&d.id, st::DELIVERED);
    assert!(got.to == KEY_A && !got.reminded);
    h.no_wake(KEY_A);
    h.ros.remove(&[KEY_A]);
    h.sweep();
    h.clk.advance(GONE_GRACE + secs(1));
    h.sweep();
    h.want_state(&d.id, st::MOVED);
    let open = h.ib.for_agent(KEY_B, false, 0);
    assert!(
        open.len() == 1 && open[0].mode == mode::NEXT_TURN && open[0].message_id == d.message_id,
        "{open:?}"
    );
}

#[test]
fn append_reclaim() {
    let h = Harness::new();
    h.abc();
    let d = h.send_with(&person(), &to("beta"), "q", opts(""));
    let w = h.append_for(KEY_B);
    h.clk.advance(REATTACH_GRACE + secs(1));
    h.sweep();
    assert_eq!(h.want_state(&d.id, st::QUEUED).wake_id, "");
    assert!(h.ib.outstanding_append(KEY_B).is_none());
    let w2 = h.append_for(KEY_B);
    assert!(w2.token != w.token && w2.delivery_ids[0] == d.id);
    h.ib.mark_delivered(KEY_B, &w.token).unwrap();
    assert_eq!(h.want_state(&d.id, st::QUEUED).wake_id, w2.id);
    h.ib.mark_delivered(KEY_B, &w2.token).unwrap();
    h.want_state(&d.id, st::DELIVERED);
    let d3 = h.send_with(&person(), &to("gamma"), "q3", opts(""));
    let w3 = h.append_for(KEY_C);
    h.clk.advance(REATTACH_GRACE + secs(1));
    h.sweep();
    h.ib.mark_delivered(KEY_C, &w3.token).unwrap();
    h.want_state(&d3.id, st::DELIVERED);
    h.no_append(KEY_C);
}

#[test]
fn append_overflow() {
    let h = Harness::new();
    h.abc();
    let big = "n".repeat(7000);
    let ids: Vec<String> = (0..4)
        .map(|_| h.send_with(&person(), &to("beta"), &big, opts("")).id)
        .collect();
    let w = h.append_for(KEY_B);
    assert!(w.delivery_ids == ids && w.overflow == ids[2..] && w.prompt.len() <= MAX_WAKE_BYTES);
    let oa = h.ib.outstanding_append(KEY_B).unwrap();
    assert!(oa.prompt == w.prompt && oa.overflow == w.overflow);
    h.ib.mark_delivered(KEY_B, &w.token).unwrap();
    h.ib.mark_engaged(KEY_B, &w.token, "t").unwrap();
    for id in &ids {
        h.want_state(id, st::ENGAGED);
    }
}

#[test]
fn wake_issued_row_fields() {
    let h = Harness::new();
    h.abc();
    h.send_with(&person(), &to("beta"), "x", opts(""));
    let w = h.append_for(KEY_B);
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to("gamma"), "y");
    let ww = h.wake(KEY_C);
    let evs =
        h.st.list(&Filter {
            kinds: s(&[kinds::WAKE_ISSUED, kinds::DELIVERY_STATE]),
            ..Default::default()
        })
        .unwrap();
    let mut modes = std::collections::HashMap::new();
    let mut dmode = String::new();
    for ev in evs {
        let v: serde_json::Value = serde_json::from_slice(&ev.payload).unwrap();
        let m = v
            .get("mode")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if ev.kind == kinds::WAKE_ISSUED {
            modes.insert(v["wakeId"].as_str().unwrap().to_string(), m);
        } else if v["deliveryId"] == d.id.as_str() {
            dmode = m;
        }
    }
    assert!(
        modes[&w.id] == "next_turn" && modes[&ww.id].is_empty() && dmode == "wake",
        "{modes:?} {dmode}"
    );
}

#[test]
fn channel_next_turn_push_does_not_gate() {
    let h = Harness::new();
    h.abc();
    h.ros.put(&[grok_card(KEY_G, "grok", "")]);
    h.ib.set_reachable(KEY_G, path::CHANNEL, true);
    let fyi = h.send_with(&person(), &to("grok"), "fyi", opts(""));
    let w = h.ib.next_wake(KEY_G, path::CHANNEL).unwrap().unwrap();
    assert_eq!(w.mode, mode::NEXT_TURN);
    assert!(h.ib.outstanding_wake(KEY_G).is_none());
    let (_, urgent) = h.send(&claude(KEY_A, "alpha"), &to("grok"), "urgent");
    let w2 =
        h.ib.next_wake(KEY_G, path::CHANNEL)
            .unwrap()
            .expect("urgent wake held back");
    assert!(w2.mode == mode::WAKE && w2.delivery_ids == vec![urgent.id.clone()]);
    assert_eq!(h.state(&fyi.id).wake_id, w.id);
    h.send(&claude(KEY_A, "alpha"), &to("grok"), "urgent 2");
    assert!(h.ib.next_wake(KEY_G, path::CHANNEL).unwrap().is_none());
}

#[test]
fn overflow_notice_stays_open() {
    let h = Harness::new();
    h.abc();
    let big = "n".repeat(7000);
    let notice = || SendOptions {
        notice: true,
        ref_: "task:01X".into(),
        ..Default::default()
    };
    let ids: Vec<String> = (0..3)
        .map(|_| {
            h.send_with(&system_sender(), &to("beta"), &big, notice())
                .id
        })
        .collect();
    let w = h.append_for(KEY_B);
    assert_eq!(w.overflow, ids[2..]);
    h.ib.mark_delivered(KEY_B, &w.token).unwrap();
    h.ib.mark_engaged(KEY_B, &w.token, "t").unwrap();
    for id in &ids[..2] {
        h.want_state(id, st::SKIPPED);
    }
    h.want_state(&ids[2], st::ENGAGED);
    let open = h.ib.for_agent(KEY_B, false, 0);
    assert!(open.len() == 1 && open[0].id == ids[2]);
    h.ib.skip(KEY_B, &ids[2], "").unwrap();
}
