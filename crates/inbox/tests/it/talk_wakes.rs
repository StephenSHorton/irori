//! Talk's replying row (card #645): which agents a Talk post woke are
//! still on it.

use crate::common;

use common::*;
use inbox::{ChangeKind, SendOptions, mode};

fn dm(conv: &str) -> SendOptions {
    SendOptions {
        mode: mode::WAKE.into(),
        ref_: format!("talk:{conv}"),
        ..SendOptions::default()
    }
}

fn mention(conv: &str) -> SendOptions {
    SendOptions {
        notice: true,
        reason: inbox::REASON_MENTIONED.into(),
        ..dm(conv)
    }
}

fn on(h: &Harness, conv: &str) -> Vec<String> {
    let mut v: Vec<String> =
        h.ib.talk_wakes(t0())
            .into_iter()
            .filter(|w| w.conv == conv)
            .map(|w| w.agent)
            .collect();
    v.sort();
    v
}

#[test]
fn a_dm_shows_from_the_wake_to_its_turns_end() {
    let h = Harness::new();
    h.abc();
    let d = h.send_with(&person(), &to("beta"), "hi", dm("dm:1"));
    let ws = h.ib.talk_wakes(t0());
    assert_eq!(ws.len(), 1);
    assert!(ws[0].waiting && ws[0].conv == "dm:1" && ws[0].message_id == d.message_id);
    let w = h.deliver_engage(KEY_B, "t1");
    assert_eq!(on(&h, "dm:1"), vec![KEY_B]);
    assert!(!h.ib.talk_wakes(t0())[0].waiting);
    let sub = h.ib.subscribe();
    h.ib.turn_ended(KEY_B, &w.token, "t1").unwrap();
    assert!(count(&sub.drain(), ChangeKind::Updated, KEY_B) >= 1);
    assert!(on(&h, "dm:1").is_empty());
}

#[test]
fn a_mention_stays_while_its_turn_runs() {
    let h = Harness::new();
    h.abc();
    h.send_with(&person(), &to("alpha"), "@alpha", mention("ch_1"));
    h.send_with(&person(), &to("beta"), "@beta", mention("ch_1"));
    assert_eq!(on(&h, "ch_1"), vec![KEY_A, KEY_B]);
    // A notice is skipped "notice" as its turn starts: still on it.
    let wa = h.deliver_engage(KEY_A, "t1");
    h.deliver_engage(KEY_B, "t2");
    assert_eq!(on(&h, "ch_1"), vec![KEY_A, KEY_B]);
    h.ib.turn_ended(KEY_A, &wa.token, "t1").unwrap();
    assert_eq!(on(&h, "ch_1"), vec![KEY_B]);
}

#[test]
fn readings_and_other_refs_never_show() {
    let h = Harness::new();
    h.abc();
    let reading = SendOptions {
        mode: mode::NEXT_TURN.into(),
        reason: "reading".into(),
        notice: true,
        ..dm("ch_1")
    };
    h.send_with(&person(), &to("alpha"), "read", reading);
    let card = SendOptions {
        ref_: "task:01X".into(),
        ..dm("")
    };
    h.send_with(&person(), &to("beta"), "card", card);
    h.send(&person(), &to("gamma"), "plain");
    assert!(h.ib.talk_wakes(t0()).is_empty());
}

#[test]
fn skipped_deferred_and_old_wakes_drop() {
    let h = Harness::new();
    h.abc();
    let a = h.send_with(&person(), &to("alpha"), "a", dm("dm:a"));
    let b = h.send_with(&person(), &to("beta"), "b", dm("dm:b"));
    h.deliver_engage(KEY_A, "t1");
    h.deliver_engage(KEY_B, "t2");
    h.ib.skip(KEY_A, &a.id, "not mine").unwrap();
    h.ib.defer(KEY_B, &b.id, 10, "later").unwrap();
    assert!(h.ib.talk_wakes(t0()).is_empty());
    h.send_with(&person(), &to("gamma"), "c", dm("dm:c"));
    h.clk.advance(mins(31));
    let since = add(t0(), mins(1));
    assert!(h.ib.talk_wakes(since).is_empty());
}

#[test]
fn an_appended_wake_is_on_while_the_turn_it_joined_runs() {
    let h = Harness::new();
    h.abc();
    h.send(&person(), &to("beta"), "first");
    let w1 = h.deliver_engage(KEY_B, "t1");
    h.send_with(&person(), &to("beta"), "@beta", mention("ch_1"));
    // The mod put the second wake into the running turn.
    let w2 = h.wake(KEY_B);
    h.ib.mark_appended_delivered(KEY_B, &w2.token).unwrap();
    h.ib.mark_appended_engaged(KEY_B, &w2.token, "t1").unwrap();
    assert_eq!(on(&h, "ch_1"), vec![KEY_B]);
    h.ib.turn_ended(KEY_B, &w1.token, "t1").unwrap();
    assert!(on(&h, "ch_1").is_empty());
    // A later turn about something else doesn't bring it back.
    h.clk.advance(secs(1));
    h.send(&person(), &to("beta"), "other work");
    h.deliver_engage(KEY_B, "t2");
    assert!(on(&h, "ch_1").is_empty());
}

#[test]
fn the_newest_wake_per_agent_counts() {
    let h = Harness::new();
    h.abc();
    h.send_with(&person(), &to("beta"), "one", dm("dm:1"));
    let w = h.deliver_engage(KEY_B, "t1");
    h.ib.turn_ended(KEY_B, &w.token, "t1").unwrap();
    h.send_with(&person(), &to("beta"), "two", dm("dm:1"));
    let ws = h.ib.talk_wakes(t0());
    assert_eq!(ws.len(), 1);
    assert!(ws[0].waiting);
}
