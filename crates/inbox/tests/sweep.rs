//! I:internal/inbox/sweep_test.go (all but the pure TestPlanSweep table,
//! which is a unit test in src/sweep/tests.rs).

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use inbox::{
    BYPASS_MODE, ChangeKind, Delivery, Error, ErrorKind, GONE_GRACE, Message, NO_CARD_GRACE,
    NOTE_RECIPIENT_GONE, Policy, REATTACH_GRACE, SendOptions, UNENGAGED_LIMIT, hold, mode, path,
    st,
};

fn wake_ref(r: &str) -> SendOptions {
    SendOptions {
        mode: mode::WAKE.into(),
        ref_: r.into(),
        ..Default::default()
    }
}

/// A recording OnRecipientGone / OnPassed hook that checks it runs with
/// the inbox lock released.
#[derive(Default)]
struct Rec {
    calls: Mutex<Vec<(Message, String)>>,
    err: Mutex<Option<Error>>,
}

impl Rec {
    fn calls(&self) -> Vec<(Message, String)> {
        self.calls.lock().unwrap().clone()
    }
    fn set_err(&self, e: Option<Error>) {
        *self.err.lock().unwrap() = e;
    }
}

fn gone_harness() -> (Harness, Arc<Rec>) {
    let rec = Arc::new(Rec::default());
    let r = rec.clone();
    let h = Harness::with_slot(move |slot| Hooks {
        on_gone: Some(Arc::new(move |m: &Message, to: &str| {
            let ib = slot_inbox(&slot);
            assert!(ib.lock_is_free(), "hook called with the inbox lock held");
            ib.list(0);
            r.calls.lock().unwrap().push((m.clone(), to.to_string()));
            match r.err.lock().unwrap().clone() {
                Some(e) => Err(e),
                None => Ok("T1".to_string()),
            }
        })),
        ..Hooks::default()
    });
    h.abc();
    (h, rec)
}

fn pass_harness() -> (Harness, Arc<Rec>) {
    let rec = Arc::new(Rec::default());
    let r = rec.clone();
    let h = Harness::with_slot(move |slot| Hooks {
        on_passed: Some(Arc::new(move |m: &Message, to: &str| {
            assert!(
                slot_inbox(&slot).lock_is_free(),
                "hook called with the inbox lock held"
            );
            r.calls.lock().unwrap().push((m.clone(), to.to_string()));
            match r.err.lock().unwrap().clone() {
                Some(e) => Err(e),
                None => Ok(()),
            }
        })),
        ..Hooks::default()
    });
    h.abc();
    (h, rec)
}

#[test]
fn repo_front_desk() {
    let h = Harness::new();
    h.abc();
    assert_eq!(h.ib.front_desk("repo1"), "");
    let (_, u) = h.send(&claude(KEY_C, "gamma"), &to_repo("repo1"), "anyone?");
    assert!(
        u.to.is_empty() && u.state == st::HELD && u.hold == hold::UNOWNED && u.repo_key == "repo1"
    );
    want_err(h.ib.release(&u.id, USER), ErrorKind::BadState);
    h.ib.set_reachable(KEY_B, path::MOD, true);
    h.clk.advance(secs(1));
    h.ib.set_reachable(KEY_A, path::MOD, true);
    assert_eq!(h.ib.front_desk("repo1"), KEY_B);
    h.sweep();
    assert_eq!(h.want_state(&u.id, st::QUEUED).to, KEY_B);
    let (_, d) = h.send(&claude(KEY_C, "gamma"), &to_repo("repo1"), "review please");
    assert!(d.to == KEY_B && d.repo_key == "repo1");
    assert_eq!(
        h.send(&claude(KEY_B, "beta"), &to_repo("repo1"), "help")
            .1
            .to,
        KEY_A
    );
    h.ib.set_reachable(KEY_B, path::MOD, false);
    h.ib.set_reachable(KEY_B, path::MOD, true);
    assert_eq!(h.ib.front_desk("repo1"), KEY_B);
    h.ib.set_reachable(KEY_B, path::MOD, false);
    h.clk.advance(REATTACH_GRACE - secs(1));
    assert_eq!(h.ib.front_desk("repo1"), KEY_B);
    h.clk.advance(secs(1));
    assert_eq!(h.ib.front_desk("repo1"), KEY_A);
    h.ib.set_reachable(KEY_B, path::MOD, true);
    assert_eq!(h.ib.front_desk("repo1"), KEY_A);
    h.ros.put(&[grok_card(KEY_G, "grok", "repo2")]);
    h.ib.set_reachable(KEY_G, path::CHANNEL, true);
    assert_eq!(h.ib.front_desk("repo2"), KEY_G);
    assert!(
        h.ib.send(&claude(KEY_A, "alpha"), &to_repo(" "), "x", "")
            .is_err()
    );
}

#[test]
fn sweep_moves_unengaged_repo_work_once() {
    let h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_A, path::MOD, true);
    h.clk.advance(secs(1));
    h.ib.set_reachable(KEY_B, path::MOD, true);
    let (_, d) = h.send(&claude(KEY_C, "gamma"), &to_repo("repo1"), "review");
    assert_eq!(d.to, KEY_A);
    let w = h.wake(KEY_A);
    h.ib.mark_delivered(KEY_A, &w.token).unwrap();
    let sub = h.ib.subscribe();
    h.clk.advance(UNENGAGED_LIMIT + secs(1));
    h.sweep();
    assert_eq!(h.want_state(&d.id, st::MOVED).moved_to, KEY_B);
    let cs = sub.drain();
    assert!(
        count(&cs, ChangeKind::MovedAway, KEY_A) == 1
            && count(&cs, ChangeKind::WakeReady, KEY_B) == 1,
        "{cs:?}"
    );
    let open = h.ib.for_agent(KEY_B, false, 0);
    assert!(
        open.len() == 1
            && open[0].message_id == d.message_id
            && open[0].state == st::QUEUED
            && open[0].repo_key == "repo1"
    );
    let nd = open[0].clone();
    want_err(
        h.ib.reply(&claude(KEY_A, "alpha"), &d.id, "mine?"),
        ErrorKind::BadState,
    );
    h.no_wake(KEY_A);
    let w2 = h.wake(KEY_B);
    h.ib.mark_delivered(KEY_B, &w2.token).unwrap();
    h.clk.advance(UNENGAGED_LIMIT + secs(1));
    h.sweep();
    h.want_state(&nd.id, st::DELIVERED);
    let r = h.wake(KEY_B);
    assert!(r.reminder && r.delivery_ids[0] == nd.id);
    h.clk.advance(UNENGAGED_LIMIT);
    h.sweep();
    h.no_wake(KEY_B);
    h.want_state(&nd.id, st::DELIVERED);
}

#[test]
fn sweep_gone_holder() {
    let on_card = Arc::new(Mutex::new(Vec::<String>::new()));
    let oc = on_card.clone();
    let h = Harness::with(Hooks {
        on_gone: Some(Arc::new(move |m: &Message, _to: &str| {
            oc.lock().unwrap().push(m.id.clone());
            Ok("x".to_string())
        })),
        ..Hooks::default()
    });
    h.abc();
    h.ib.set_reachable(KEY_A, path::MOD, true);
    h.clk.advance(secs(1));
    h.ib.set_reachable(KEY_B, path::MOD, true);
    let (_, repo_work) = h.send(&claude(KEY_C, "gamma"), &to_repo("repo1"), "repo work");
    let (_, agent_work) = h.send(&claude(KEY_C, "gamma"), &to("alpha"), "agent work");
    let card_work = h.send_with(
        &claude(KEY_C, "gamma"),
        &to("alpha"),
        "card work",
        wake_ref("task:x"),
    );
    h.deliver_engage(KEY_A, "t1");
    h.want_state(&repo_work.id, st::ENGAGED);
    h.ros.remove(&[KEY_A]);
    h.ib.set_reachable(KEY_A, path::MOD, false);
    h.sweep();
    h.clk.advance(GONE_GRACE);
    h.sweep();
    h.want_state(&repo_work.id, st::ENGAGED);
    h.clk.advance(secs(1));
    h.sweep();
    assert_eq!(h.want_state(&repo_work.id, st::MOVED).moved_to, KEY_B);
    let got = h.want_state(&agent_work.id, st::MOVED);
    assert!(got.moved_to == USER && got.note == NOTE_RECIPIENT_GONE);
    let mine = h.ib.for_agent(USER, false, 0);
    assert!(
        mine.len() == 1
            && mine[0].message_id == agent_work.message_id
            && mine[0].for_.starts_with("recipient gone: alpha")
    );
    let got = h.want_state(&card_work.id, st::SKIPPED);
    assert_eq!(got.note, "recipient gone; on task:x");
    assert_eq!(*on_card.lock().unwrap(), vec![card_work.message_id.clone()]);
    let open = h.ib.for_agent(KEY_B, false, 0);
    assert!(open.len() == 1 && open[0].message_id == repo_work.message_id);
    h.ros.remove(&[KEY_B]);
    h.sweep();
    h.clk.advance(GONE_GRACE + secs(1));
    h.sweep();
    let unowned: Delivery =
        h.ib.list(0)
            .into_iter()
            .rfind(|d| d.message_id == repo_work.message_id && st::open(&d.state))
            .unwrap();
    assert!(
        unowned.to.is_empty() && unowned.hold == hold::UNOWNED,
        "{unowned:?}"
    );
    const KEY_E: &str = "dev1/eeeeeeee-7777-4777-8777-777777777777";
    h.ros.put(&[card(KEY_E, "epsilon", "repo1", BYPASS_MODE)]);
    h.ib.set_reachable(KEY_E, path::MOD, true);
    h.sweep();
    assert_eq!(h.want_state(&unowned.id, st::QUEUED).to, KEY_E);
}

#[test]
fn sweep_gone_to_card() {
    let (h, g) = gone_harness();
    let d = h.send_with(
        &claude(KEY_C, "gamma"),
        &to("alpha"),
        "about T1",
        wake_ref("task:T1"),
    );
    h.sweep();
    assert!(g.calls().is_empty());
    h.end_a();
    let c = g.calls();
    assert!(
        c.len() == 1 && c[0].0.id == d.message_id && c[0].0.ref_ == "task:T1" && c[0].1 == KEY_A
    );
    assert_eq!(
        h.want_state(&d.id, st::SKIPPED).note,
        "recipient gone; on task:T1"
    );
    assert_eq!(h.ib.list(0).len(), 1);
    h.clk.advance(mins(1));
    h.sweep();
    assert!(g.calls().len() == 1 && h.ib.list(0).len() == 1);
    assert!(h.ib.ended_for(KEY_A).is_zero());
}

#[test]
fn sweep_gone_to_person() {
    // no card
    {
        let (h, g) = gone_harness();
        let d = h.send_with(
            &claude(KEY_C, "gamma"),
            &to("alpha"),
            "no card",
            opts(mode::WAKE),
        );
        let mine = h.send_with(&person(), &to("alpha"), "the person's own", opts(""));
        h.end_a();
        assert!(g.calls().is_empty());
        let old = h.want_state(&d.id, st::MOVED);
        assert!(old.moved_to == USER && old.note == NOTE_RECIPIENT_GONE);
        let got = h.ib.for_agent(USER, false, 0);
        assert!(got.len() == 1 && got[0].message_id == d.message_id && got[0].state == st::QUEUED);
        assert_eq!(got[0].for_, format!("recipient gone: alpha ({KEY_A})"));
        h.want_state(&mine.id, st::QUEUED);
    }
    // the card isn't here
    {
        let (h, g) = gone_harness();
        g.set_err(Some(Error::sentinel(ErrorKind::NoCard)));
        let d = h.send_with(
            &claude(KEY_C, "gamma"),
            &to("alpha"),
            "about a card",
            wake_ref("comment:C1"),
        );
        let mine = h.send_with(
            &person(),
            &to("alpha"),
            "the person's own",
            SendOptions {
                ref_: "task:T9".into(),
                ..Default::default()
            },
        );
        h.end_a();
        assert_eq!(g.calls().len(), 2);
        h.want_state(&d.id, st::QUEUED);
        h.want_state(&mine.id, st::QUEUED);
        h.clk.advance(NO_CARD_GRACE);
        h.sweep();
        h.want_state(&d.id, st::QUEUED);
        h.clk.advance(mins(1));
        h.sweep();
        let old = h.want_state(&d.id, st::MOVED);
        assert!(old.moved_to == USER && old.note == NOTE_RECIPIENT_GONE);
        let got = h.ib.for_agent(USER, false, 0);
        assert!(got.len() == 1 && got[0].for_.starts_with("recipient gone: "));
        h.want_state(&mine.id, st::QUEUED);
    }
    // the card arrives after a miss
    {
        let (h, g) = gone_harness();
        g.set_err(Some(Error::sentinel(ErrorKind::NoCard)));
        let d = h.send_with(
            &claude(KEY_C, "gamma"),
            &to("alpha"),
            "about T1",
            wake_ref("task:T1"),
        );
        h.end_a();
        h.want_state(&d.id, st::QUEUED);
        g.set_err(None);
        h.clk.advance(mins(1));
        h.sweep();
        assert_eq!(
            h.want_state(&d.id, st::SKIPPED).note,
            "recipient gone; on task:T1"
        );
        assert!(h.ib.for_agent(USER, false, 0).is_empty());
    }
    // another error leaves it for the next sweep
    {
        let (h, g) = gone_harness();
        g.set_err(Some(Error::other("restoring")));
        let d = h.send_with(
            &claude(KEY_C, "gamma"),
            &to("alpha"),
            "about T1",
            wake_ref("task:T1"),
        );
        h.end_a();
        h.want_state(&d.id, st::QUEUED);
        assert!(h.ib.for_agent(USER, false, 0).is_empty());
        g.set_err(None);
        h.sweep();
        assert_eq!(g.calls().len(), 2);
        h.want_state(&d.id, st::SKIPPED);
    }
    // no hook: card work goes to the person
    {
        let h = Harness::new();
        h.abc();
        let d = h.send_with(
            &claude(KEY_C, "gamma"),
            &to("alpha"),
            "about T1",
            wake_ref("task:T1"),
        );
        let mine = h.send_with(
            &person(),
            &to("alpha"),
            "the person's own",
            SendOptions {
                ref_: "task:T1".into(),
                ..Default::default()
            },
        );
        h.end_a();
        assert_eq!(h.want_state(&d.id, st::MOVED).moved_to, USER);
        h.want_state(&mine.id, st::QUEUED);
    }
}

fn block(keys: &[&str]) -> Policy {
    Policy {
        block: keys.iter().map(|s| s.to_string()).collect(),
        ..Policy::default()
    }
}

#[test]
fn passed_fills_the_card() {
    // released
    {
        let (h, p) = pass_harness();
        h.ib.set_policy(block(&[KEY_C]), USER).unwrap();
        let d = h.send_with(
            &claude(KEY_C, "gamma"),
            &to("alpha"),
            "about T1",
            wake_ref("task:T1"),
        );
        let plain = h.send_with(
            &claude(KEY_C, "gamma"),
            &to("alpha"),
            "no card",
            opts(mode::WAKE),
        );
        let shown = h.send_with(
            &claude(KEY_A, "alpha"),
            &to("beta"),
            "never held",
            wake_ref("task:T1"),
        );
        h.want_state(&d.id, st::HELD);
        h.sweep();
        assert!(p.calls().is_empty());
        for id in [&d.id, &plain.id] {
            h.ib.release(id, USER).unwrap();
        }
        let c = p.calls();
        assert!(
            c.len() == 1 && c[0].0.id == d.message_id && c[0].1 == KEY_A,
            "{c:?}"
        );
        h.sweep();
        let c = p.calls();
        assert!(c.len() == 1 && c[0].0.id != shown.message_id);
    }
    // unpaused
    {
        let (h, p) = pass_harness();
        h.ib.set_policy(
            Policy {
                paused: true,
                ..Policy::default()
            },
            USER,
        )
        .unwrap();
        let d = h.send_with(
            &claude(KEY_C, "gamma"),
            &to("alpha"),
            "about T1",
            wake_ref("comment:C1"),
        );
        h.want_state(&d.id, st::HELD);
        h.ib.set_policy(Policy::default(), USER).unwrap();
        let c = p.calls();
        assert!(c.len() == 1 && c[0].0.id == d.message_id);
    }
    // a failure is tried again for the grace
    {
        let (h, p) = pass_harness();
        p.set_err(Some(Error::sentinel(ErrorKind::NoCard)));
        h.ib.set_policy(block(&[KEY_C]), USER).unwrap();
        let d = h.send_with(
            &claude(KEY_C, "gamma"),
            &to("alpha"),
            "about T1",
            wake_ref("task:T1"),
        );
        h.ib.release(&d.id, USER).unwrap();
        h.clk.advance(NO_CARD_GRACE);
        h.sweep();
        assert_eq!(p.calls().len(), 2);
        h.clk.advance(mins(1));
        h.sweep();
        h.sweep();
        assert_eq!(p.calls().len(), 3);
    }
}

#[test]
fn ended_ignores_reachability() {
    let (h, g) = gone_harness();
    let d = h.send_with(
        &claude(KEY_C, "gamma"),
        &to("alpha"),
        "about T1",
        wake_ref("task:T1"),
    );
    let n = h.send_with(
        &claude(KEY_C, "gamma"),
        &to("alpha"),
        "no card",
        opts(mode::WAKE),
    );
    for _ in 0..10 {
        h.sweep();
        h.clk.advance(mins(1));
    }
    h.sweep();
    assert!(g.calls().is_empty() && h.ib.ended_for(KEY_A).is_zero());
    h.want_state(&d.id, st::QUEUED);
    h.want_state(&n.id, st::QUEUED);
    h.ros.remove(&[KEY_A]);
    h.sweep();
    h.clk.advance(mins(2));
    h.sweep();
    h.abc();
    h.sweep();
    assert!(h.ib.ended_for(KEY_A).is_zero());
    h.ros.remove(&[KEY_A]);
    h.sweep();
    h.clk.advance(mins(2));
    h.sweep();
    assert!(g.calls().is_empty());
    h.want_state(&d.id, st::QUEUED);
    h.want_state(&n.id, st::QUEUED);
}

#[test]
fn sweep_attach_lost_counts_as_gone() {
    let h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_A, path::MOD, true);
    h.clk.advance(secs(1));
    h.ib.set_reachable(KEY_B, path::MOD, true);
    let (_, d) = h.send(&claude(KEY_C, "gamma"), &to_repo("repo1"), "x");
    h.deliver_engage(KEY_A, "t");
    h.ib.set_reachable(KEY_A, path::MOD, false);
    h.clk.advance(REATTACH_GRACE);
    h.sweep();
    h.clk.advance(GONE_GRACE + secs(1));
    h.sweep();
    assert_eq!(h.want_state(&d.id, st::MOVED).moved_to, KEY_B);
}

#[test]
fn sweep_assigns_held_policy_on_arrival() {
    let h = Harness::new();
    h.abc();
    h.ros.put(&[grok_card(KEY_G, "grok", "")]);
    h.block_grok();
    let (_, d) = h.send(&grok(KEY_G, "grok"), &to_repo("repo1"), "from grok");
    assert_eq!(d.hold, hold::UNOWNED);
    h.ib.set_reachable(KEY_A, path::MOD, true);
    h.sweep();
    let got = h.want_state(&d.id, st::HELD);
    assert!(got.hold == hold::BLOCKED && got.to == KEY_A);
}
