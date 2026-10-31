//! I:internal/inbox/mesh_test.go and held_test.go: other devices' rows,
//! the host's holds, delivery.decided requests, undo, HeldHistory.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use inbox::{
    ErrorKind, GONE_GRACE, Inbox, Message, Policy, SendOptions, UNENGAGED_LIMIT, hold, kinds, mode,
    path, st,
};
use serde_json::{Value, json};
use store::{Event, Filter, Timestamp};

const KEY_R: &str = "dev2/rrrrrrrr-9999-4999-8999-999999999999";

/// Inserts rows written by dev2, as the mesh would.
struct Foreign<'a> {
    h: &'a Harness,
    seq: i64,
}

impl Foreign<'_> {
    fn row(&mut self, actor: &str, kind: &str, payload: Value) {
        self.seq += 1;
        let ms = self.h.clk.now().unix_milli_value();
        let at = Timestamp(((ms as u64) << 16) + self.seq as u64);
        self.h
            .st
            .insert(&Event {
                device_id: "dev2".into(),
                seq: self.seq,
                hlc: at,
                actor: actor.into(),
                kind: kind.into(),
                payload: serde_json::to_vec(&payload).unwrap(),
                sig: vec![],
            })
            .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn message(
        &mut self,
        id: &str,
        from: &str,
        kind: &str,
        to_kind: &str,
        to_v: &str,
        text: &str,
        r: &str,
    ) {
        let mut m = json!({
            "id": id, "from": {"key": from, "name": "remote", "kind": kind},
            "to": {"kind": to_kind, "value": to_v}, "text": text, "hop": 0, "trust": "untrusted",
            "sentAt": self.h.clk.now().format_rfc3339_nano(),
        });
        if !r.is_empty() {
            m["ref"] = json!(r);
        }
        self.row(&format!("agent:{from}"), kinds::MESSAGE_SENT, m);
    }
}

#[track_caller]
fn eventually(what: &str, ok: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for: {what}");
}

/// Runs Follow on its own thread until the returned guard drops.
struct Following {
    sub: Arc<store::Subscription>,
    t: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Following {
    fn drop(&mut self) {
        self.sub.cancel();
        if let Some(t) = self.t.take() {
            let _ = t.join();
        }
    }
}

fn follow(h: &Harness) -> Following {
    let sub = Arc::new(h.st.subscribe());
    let (ib, s): (Arc<Inbox>, _) = (h.ib.clone(), sub.clone());
    let t = std::thread::spawn(move || ib.follow(&s));
    Following { sub, t: Some(t) }
}

fn owners2() -> Hooks {
    Hooks {
        is_owner: Some(Arc::new(|k: &str| k == USER || k == "user:dev2")),
        ..Hooks::default()
    }
}

fn paused() -> Policy {
    Policy {
        paused: true,
        ..Policy::default()
    }
}

#[test]
fn a_remote_recipient_is_the_hosts_to_wake_and_sweep() {
    let h = Harness::new();
    h.abc();
    h.ib.set_policy(
        Policy {
            paused: true,
            block: vec![KEY_A.into()],
            ..Policy::default()
        },
        USER,
    )
    .unwrap();
    h.ros.put(&[card(KEY_R, "remote", "repo1", "")]);
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to(KEY_R), "to the mac");
    assert!(
        d.state == st::QUEUED && d.hold.is_empty(),
        "{} {}",
        d.state,
        d.hold
    );
    assert!(!h.ib.owned(&d.id));
    h.ib.set_policy(Policy::default(), USER).unwrap();
    h.no_wake(KEY_R);
    assert!(h.ib.next_append(KEY_R).unwrap().is_none());
    h.ros.remove(&[KEY_R]);
    for _ in 0..3 {
        h.clk.advance(GONE_GRACE + UNENGAGED_LIMIT);
        h.sweep();
    }
    assert_eq!(h.want_state(&d.id, st::QUEUED).to, KEY_R);
}

#[test]
fn repo_work_is_assigned_only_by_its_writer() {
    let mut h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_A, path::MOD, true);
    {
        let mut f = Foreign { h: &h, seq: 0 };
        f.message(
            "msg_remote",
            KEY_R,
            "claude",
            "repo",
            "repo1",
            "anyone?",
            "",
        );
        f.row(
            &format!("agent:{KEY_R}"),
            kinds::DELIVERY_STATE,
            json!({"deliveryId": "dlv_remote", "messageId": "msg_remote", "to": "", "repoKey": "repo1", "state": "held", "hold": "unowned", "by": KEY_R}),
        );
    }
    h.reopen();
    h.ib.set_reachable(KEY_A, path::MOD, true);
    h.sweep();
    assert_eq!(h.want_state("dlv_remote", st::HELD).to, "");
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to_repo("repo2"), "local work");
    assert_eq!(d.to, "");
    h.ib.set_reachable(KEY_C, path::MOD, true);
    h.sweep();
    assert_ne!(h.state(&d.id).to, "");
}

#[test]
fn the_host_holds_and_applies_the_first_request() {
    let mut h = Harness::with(owners2());
    h.ros.put(&[card(KEY_B, "beta", "repo1", "")]);
    h.ib.set_policy(paused(), USER).unwrap();
    {
        let _f = follow(&h);
        let mut f = Foreign { h: &h, seq: 0 };
        f.message("msg_r1", KEY_R, "claude", "agent", KEY_B, "wake up", "");
        f.row(&format!("agent:{KEY_R}"), kinds::DELIVERY_STATE, json!({"deliveryId": "dlv_r1", "messageId": "msg_r1", "to": KEY_B, "state": "queued", "by": KEY_R}));
        eventually("the host holds it", || {
            h.ib.get("dlv_r1")
                .is_some_and(|d| d.state == st::HELD && d.hold == hold::PAUSED)
        });
        f.row(
            &format!("agent:{KEY_R}"),
            kinds::DELIVERY_DECIDED,
            json!({"deliveryId": "dlv_r1", "choice": "release", "by": KEY_R}),
        );
        f.row("user", kinds::DELIVERY_DECIDED, json!({"deliveryId": "dlv_r1", "choice": "dismiss", "by": "user:dev2", "atDevice": "dev2"}));
        f.row("user", kinds::DELIVERY_DECIDED, json!({"deliveryId": "dlv_r1", "choice": "release", "by": "user:dev2", "atDevice": "dev2"}));
        eventually("the first request applies", || {
            h.ib.get("dlv_r1").is_some_and(|d| d.state == st::DISMISSED)
        });
        std::thread::sleep(Duration::from_millis(50));
        h.want_state("dlv_r1", st::DISMISSED);
        assert_eq!(h.ib.ignored_rows(), 1);
    }
    h.reopen();
    let _f = follow(&h);
    std::thread::sleep(Duration::from_millis(50));
    h.want_state("dlv_r1", st::DISMISSED);
}

#[test]
fn reply_inherits_ref() {
    let mut h = Harness::new();
    h.abc();
    let mut replies = vec![];
    for r in ["task:T1", "comment:C1"] {
        let d = h.send_with(
            &claude(KEY_C, "gamma"),
            &to("alpha"),
            &format!("about {r}"),
            SendOptions {
                mode: mode::WAKE.into(),
                ref_: r.into(),
                ..Default::default()
            },
        );
        let (m, nd) = h.ib.reply(&claude(KEY_A, "alpha"), &d.id, "on it").unwrap();
        assert!(m.ref_ == r && nd.message.ref_ == r && nd.to == KEY_C);
        replies.push(nd.id);
    }
    let evs =
        h.st.list(&Filter {
            kinds: vec![kinds::MESSAGE_SENT.into()],
            ..Default::default()
        })
        .unwrap();
    let mut refs = std::collections::HashMap::new();
    for ev in evs {
        let v: Value = serde_json::from_slice(&ev.payload).unwrap();
        if let Some(rt) = v.get("replyTo").and_then(Value::as_str) {
            refs.insert(
                format!(
                    "{} {}",
                    v["text"].as_str().unwrap(),
                    v["ref"].as_str().unwrap_or("")
                ),
                rt.to_string(),
            );
        }
    }
    assert!(
        refs.len() == 2
            && refs.contains_key("on it task:T1")
            && refs.contains_key("on it comment:C1"),
        "{refs:?}"
    );
    h.reopen();
    for (i, r) in ["task:T1", "comment:C1"].iter().enumerate() {
        assert_eq!(h.state(&replies[i]).message.ref_, *r);
    }
    let d = h.send_with(
        &claude(KEY_C, "gamma"),
        &to("alpha"),
        "plain",
        opts(mode::WAKE),
    );
    let (m, _) = h.ib.reply(&claude(KEY_A, "alpha"), &d.id, "ok").unwrap();
    assert_eq!(m.ref_, "");
}

#[test]
fn the_host_fills_the_card_when_it_lets_a_message_through() {
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let c = calls.clone();
    let h = Harness::with(Hooks {
        on_passed: Some(Arc::new(move |m: &Message, to: &str| {
            c.lock().unwrap().push(format!("{} to {to}", m.id));
            Ok(())
        })),
        ..Hooks::default()
    });
    h.ros.put(&[card(KEY_B, "beta", "repo1", "")]);
    h.ib.set_policy(
        Policy {
            block: vec!["dev2/blocked".into()],
            ..Policy::default()
        },
        USER,
    )
    .unwrap();
    let _fo = follow(&h);
    let mut f = Foreign { h: &h, seq: 0 };
    for (from, id) in [(KEY_R, "msg_rrrr"), ("dev2/blocked", "msg_bloc")] {
        f.message(id, from, "claude", "agent", KEY_B, "about T1", "task:T1");
        f.row(&format!("agent:{from}"), kinds::DELIVERY_STATE, json!({"deliveryId": format!("dlv_{id}"), "messageId": id, "to": KEY_B, "state": "queued", "by": from}));
    }
    eventually("the host holds the blocked one", || {
        h.ib.get("dlv_msg_bloc")
            .is_some_and(|d| d.state == st::HELD)
    });
    h.sweep();
    assert_eq!(*calls.lock().unwrap(), vec![format!("msg_rrrr to {KEY_B}")]);
    h.ib.release("dlv_msg_bloc", USER).unwrap();
    assert_eq!(calls.lock().unwrap().len(), 2);
    assert_eq!(calls.lock().unwrap()[1], format!("msg_bloc to {KEY_B}"));
}

/// No hold for a bypass-mode recipient (card #71): every agent's message
/// reaches it, the person's own, another device's, Grok, a stranger's or a
/// spoofed one; only the person's block list and Pause all hold.
#[test]
fn the_host_holds_only_for_the_persons_own_settings() {
    const KEY_UNK: &str = "devX/eeeeeeee-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    const KEY_RG: &str = "dev2/grok-remote";
    let h = Harness::with(owners2());
    h.abc();
    h.ros.put(&[
        card(KEY_R, "mac", "", ""),
        card(KEY_UNK, "stranger", "", "default"),
        grok_card(KEY_RG, "grok", ""),
    ]);
    let _fo = follow(&h);
    let f = Mutex::new(Foreign { h: &h, seq: 0 });
    let send = |id: &str, key: &str, name: &str, kind: &str| {
        let mut f = f.lock().unwrap();
        let mid = format!("msg_{id}");
        let m = json!({
            "id": mid, "from": {"key": key, "name": name, "kind": kind},
            "to": {"kind": "agent", "value": KEY_A}, "text": format!("hi from {id}"), "hop": 0,
            "trust": "untrusted", "sentAt": h.clk.now().format_rfc3339_nano(),
        });
        f.row(&format!("agent:{key}"), kinds::MESSAGE_SENT, m);
        f.row(&format!("agent:{key}"), kinds::DELIVERY_STATE, json!({"deliveryId": format!("dlv_{id}"), "messageId": mid, "to": KEY_A, "state": "queued", "by": key}));
    };
    send("own", KEY_R, "mac", "claude");
    send("grok", KEY_RG, "grok", "grok");
    send("unknown", KEY_UNK, "stranger", "claude");
    send(
        "spoofed",
        "dev1/ffffffff-cccc-4ccc-8ccc-cccccccccccc",
        "local",
        "claude",
    );
    for id in ["dlv_own", "dlv_grok", "dlv_unknown", "dlv_spoofed"] {
        eventually(&format!("{id} through"), || {
            h.ib.get(id).is_some_and(|d| d.state == st::QUEUED)
        });
        let d = h.ib.get(id).unwrap();
        assert!(d.hold.is_empty(), "{id} held for {}", d.hold);
    }
    h.ib.set_policy(
        Policy {
            block: vec!["mac".into()],
            ..Policy::default()
        },
        USER,
    )
    .unwrap();
    send("blocked", KEY_R, "mac", "claude");
    eventually("the block list holds", || {
        h.ib.get("dlv_blocked")
            .is_some_and(|d| d.state == st::HELD && d.hold == hold::BLOCKED)
    });
    h.ib.set_policy(paused(), USER).unwrap();
    send("paused", KEY_R, "mac", "claude");
    eventually("Pause all holds", || {
        h.ib.get("dlv_paused")
            .is_some_and(|d| d.state == st::HELD && d.hold == hold::PAUSED)
    });
}

/// A delivery an older Irori held for bypass_receiver, a hold that no
/// longer exists (card #71), is decided again when its host catches up:
/// queued, unless the person's own settings hold it.
#[test]
fn a_retired_bypass_hold_is_lifted_at_catch_up() {
    for block in [false, true] {
        let mut h = Harness::new();
        h.abc();
        h.ros.put(&[grok_card(KEY_G, "grok", "")]);
        let (_, d) = h.send(&grok(KEY_G, "grok"), &to("beta"), "held by an old rule");
        h.st.append(
            "irori",
            kinds::DELIVERY_STATE,
            &json!({"deliveryId": d.id, "messageId": d.message_id, "to": KEY_B,
                "state": "held", "hold": "bypass_receiver", "by": "irori"}),
        )
        .unwrap();
        if block {
            h.block_grok();
        }
        h.reopen();
        h.ib.catch_up();
        let got = h.ib.get(&d.id).unwrap();
        if block {
            assert!(
                got.state == st::HELD && got.hold == hold::BLOCKED,
                "{got:?}"
            );
        } else {
            assert!(got.state == st::QUEUED && got.hold.is_empty(), "{got:?}");
        }
    }
}

// ---- held_test.go ----------------------------------------------------------

#[test]
fn release_undoes_a_dismiss() {
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let c = calls.clone();
    let mut h = Harness::with(Hooks {
        on_passed: Some(Arc::new(move |m: &Message, to: &str| {
            c.lock().unwrap().push(format!("{} to {to}", m.id));
            Ok(())
        })),
        ..Hooks::default()
    });
    h.abc();
    h.ros.put(&[grok_card(KEY_G, "grok", "")]);
    h.block_grok();
    let (m, d) =
        h.ib.send_with(
            &grok(KEY_G, "grok"),
            &to("beta"),
            "about T1",
            SendOptions {
                ref_: "task:T1".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(d.state == st::HELD && d.hold == hold::BLOCKED && d.held_for == hold::BLOCKED);
    h.clk.advance(mins(1));
    h.ib.dismiss(&d.id, USER).unwrap();
    let got = h.want_state(&d.id, st::DISMISSED);
    assert!(
        got.decided_by == USER && got.decided_at == h.clk.now() && got.held_for == hold::BLOCKED
    );
    assert!(h.ib.for_agent(KEY_B, true, 0).is_empty());
    h.clk.advance(mins(1));
    h.ib.release(&d.id, USER).unwrap();
    let got = h.want_state(&d.id, st::QUEUED);
    assert!(
        got.hold.is_empty()
            && got.held_for == hold::BLOCKED
            && got.decided_by == USER
            && got.decided_at == h.clk.now()
    );
    assert!(got.message.id == m.id && got.message.text == "about T1");
    assert_eq!(*calls.lock().unwrap(), vec![format!("{} to {KEY_B}", m.id)]);
    assert_eq!(h.wake(KEY_B).delivery_ids, vec![d.id.clone()]);
    want_err(h.ib.release(&d.id, USER), ErrorKind::BadState);
    h.reopen();
    let got = h.state(&d.id);
    assert!(got.held_for == hold::BLOCKED && got.decided_by == USER);
}

#[test]
fn held_history() {
    let h = Harness::new();
    h.abc();
    h.ros.put(&[grok_card(KEY_G, "grok", "")]);
    h.block_grok();
    let (_, never) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "never held");
    let (_, d1) = h.send(&grok(KEY_G, "grok"), &to("beta"), "released");
    let (_, d2) = h.send(&grok(KEY_G, "grok"), &to("beta"), "dismissed");
    let (_, d3) = h.send(&grok(KEY_G, "grok"), &to("beta"), "still held");
    assert_eq!(never.state, st::QUEUED);
    h.clk.advance(mins(1));
    h.ib.dismiss(&d2.id, USER).unwrap();
    h.clk.advance(mins(1));
    h.ib.release(&d1.id, USER).unwrap();
    let hist = h.ib.held_history(0);
    assert!(hist.len() == 2 && hist[0].id == d1.id && hist[1].id == d2.id);
    assert!(
        hist[0].state == st::QUEUED
            && hist[1].state == st::DISMISSED
            && hist[1].message.text == "dismissed"
    );
    let got = h.ib.held_history(1);
    assert!(got.len() == 1 && got[0].id == d1.id);
    h.want_state(&d3.id, st::HELD);
    let ds = h.ib.of_message(&d3.message_id);
    assert!(ds.len() == 1 && ds[0].id == d3.id);
}

#[test]
fn a_remote_release_undoes_a_dismiss() {
    let mut h = Harness::with(owners2());
    h.ros.put(&[card(KEY_B, "beta", "repo1", "")]);
    h.ib.set_policy(paused(), USER).unwrap();
    {
        let _fo = follow(&h);
        let mut f = Foreign { h: &h, seq: 0 };
        f.message("msg_r1", KEY_R, "claude", "agent", KEY_B, "wake up", "");
        f.row(&format!("agent:{KEY_R}"), kinds::DELIVERY_STATE, json!({"deliveryId": "dlv_r1", "messageId": "msg_r1", "to": KEY_B, "state": "queued", "by": KEY_R}));
        eventually("the host holds it", || {
            h.ib.get("dlv_r1").is_some_and(|d| d.state == st::HELD)
        });
        h.clk.advance(mins(1));
        let now = h.clk.now().format_rfc3339_nano();
        f.row("user", kinds::DELIVERY_DECIDED, json!({"deliveryId": "dlv_r1", "choice": "dismiss", "by": "user:dev2", "atDevice": "dev2", "at": now}));
        eventually("the dismiss applies", || {
            h.ib.get("dlv_r1").is_some_and(|d| d.state == st::DISMISSED)
        });
        let behind = add(h.clk.now(), Duration::ZERO);
        let behind = gocompat::time::GoTime {
            secs: behind.secs - 3600,
            ..behind
        }
        .format_rfc3339_nano();
        f.row("user", kinds::DELIVERY_DECIDED, json!({"deliveryId": "dlv_r1", "choice": "release", "by": "user:dev2", "atDevice": "dev2", "at": behind, "undo": true}));
        eventually("the undo applies", || {
            h.ib.get("dlv_r1").is_some_and(|d| d.state == st::QUEUED)
        });
        let d = h.ib.get("dlv_r1").unwrap();
        assert!(
            d.decided_by == "user:dev2"
                && d.held_for == hold::PAUSED
                && d.message.text == "wake up",
            "{d:?}"
        );
    }
    h.reopen();
    let _fo = follow(&h);
    std::thread::sleep(Duration::from_millis(50));
    h.want_state("dlv_r1", st::QUEUED);
}

#[test]
fn a_remote_release_from_before_the_dismiss_loses() {
    let h = Harness::with(owners2());
    h.ros.put(&[card(KEY_B, "beta", "repo1", "")]);
    h.ib.set_policy(paused(), USER).unwrap();
    let _fo = follow(&h);
    let mut f = Foreign { h: &h, seq: 0 };
    f.message("msg_r1", KEY_R, "claude", "agent", KEY_B, "wake up", "");
    f.row(&format!("agent:{KEY_R}"), kinds::DELIVERY_STATE, json!({"deliveryId": "dlv_r1", "messageId": "msg_r1", "to": KEY_B, "state": "queued", "by": KEY_R}));
    eventually("the host holds it", || {
        h.ib.get("dlv_r1").is_some_and(|d| d.state == st::HELD)
    });
    h.ib.dismiss("dlv_r1", USER).unwrap();
    let ahead = gocompat::time::GoTime {
        secs: h.clk.now().secs + 3600,
        ..h.clk.now()
    }
    .format_rfc3339_nano();
    f.row("user", kinds::DELIVERY_DECIDED, json!({"deliveryId": "dlv_r1", "choice": "release", "by": "user:dev2", "atDevice": "dev2", "at": ahead}));
    std::thread::sleep(Duration::from_millis(50));
    h.want_state("dlv_r1", st::DISMISSED);
}

#[test]
fn a_remote_release_is_an_undo_only_when_seen_dismissed() {
    let mut h = Harness::new();
    {
        let mut f = Foreign { h: &h, seq: 0 };
        f.message("msg_r2", KEY_A, "claude", "agent", KEY_R, "over there", "");
        f.row(&format!("agent:{KEY_R}"), kinds::DELIVERY_STATE, json!({"deliveryId": "dlv_h", "messageId": "msg_r2", "to": KEY_R, "state": "held", "hold": "paused", "by": "irori"}));
        f.row(&format!("agent:{KEY_R}"), kinds::DELIVERY_STATE, json!({"deliveryId": "dlv_d", "messageId": "msg_r2", "to": KEY_R, "state": "dismissed", "hold": "paused", "by": "user:dev2"}));
    }
    h.reopen();
    for id in ["dlv_h", "dlv_d"] {
        assert!(!h.ib.owned(id));
        h.ib.release(id, USER).unwrap();
    }
    let evs =
        h.st.list(&Filter {
            kinds: vec![kinds::DELIVERY_DECIDED.into()],
            ..Default::default()
        })
        .unwrap();
    let mut undo = std::collections::HashMap::new();
    for e in evs {
        let v: Value = serde_json::from_slice(&e.payload).unwrap();
        undo.insert(
            v["deliveryId"].as_str().unwrap().to_string(),
            v.get("undo").and_then(Value::as_bool).unwrap_or(false),
        );
    }
    assert!(
        undo.len() == 2 && !undo["dlv_h"] && undo["dlv_d"],
        "{undo:?}"
    );
}

#[test]
fn releasing_dismissed_repo_work_with_no_desk_says_so() {
    let h = Harness::new();
    h.abc();
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to_repo("repo2"), "anyone?");
    assert!(d.state == st::HELD && d.hold == hold::UNOWNED && d.to.is_empty());
    h.ib.dismiss(&d.id, USER).unwrap();
    want_err(h.ib.release(&d.id, USER), ErrorKind::BadState);
    h.want_state(&d.id, st::DISMISSED);
}
