//! I:internal/inbox/replay_test.go, owner_test.go, owner_rule_test.go,
//! stale_test.go and search_test.go (search paging and ref parsing belong
//! to the search crate and are left to it).

mod common;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use common::*;
use gocompat::time::GoTime;
use inbox::{
    Address, DIRECT_LIMIT, DIRECT_WINDOW, Delivery, ErrorKind, Inbox, Policy, SendOptions, Sender,
    UNENGAGED_LIMIT, Wake, addr, hold, kinds, mode, path, policy_mode, st, system_sender, trust,
};
use serde_json::json;
use store::{Event, Filter, Timestamp};

#[derive(Debug, PartialEq)]
struct Snapshot {
    list: Vec<Delivery>,
    for_: BTreeMap<String, Vec<Delivery>>,
    for_all: BTreeMap<String, Vec<Delivery>>,
    policy: Policy,
    outstanding: BTreeMap<String, Option<(Wake, &'static str)>>,
    running: BTreeMap<String, String>,
    aliases: BTreeMap<String, Vec<String>>,
}

fn snap(ib: &Inbox, agents: &[&str]) -> Snapshot {
    let mut s = Snapshot {
        list: ib.list(10000),
        for_: BTreeMap::new(),
        for_all: BTreeMap::new(),
        policy: ib.policy(),
        outstanding: BTreeMap::new(),
        running: BTreeMap::new(),
        aliases: BTreeMap::new(),
    };
    for a in agents {
        let a = a.to_string();
        s.for_.insert(a.clone(), ib.for_agent(&a, false, 10000));
        s.for_all.insert(a.clone(), ib.for_agent(&a, true, 10000));
        s.outstanding.insert(a.clone(), ib.outstanding_wake(&a));
        s.running.insert(a.clone(), ib.running_wake(&a));
        s.aliases.insert(a.clone(), ib.aliases(&a));
    }
    s
}

#[test]
fn replay_reproduces_state() {
    let mut h = Harness::new();
    h.abc();
    h.ros.put(&[grok_card(KEY_G, "grok", "repo1")]);
    h.ib.set_reachable(KEY_A, path::MOD, true);
    h.clk.advance(secs(1));
    h.ib.set_reachable(KEY_B, path::MOD, true);
    let (_, d1) = h.send(&claude(KEY_A, "Front Desk"), &to("beta"), "one");
    h.send(&claude(KEY_C, "gamma"), &to("beta"), "two");
    let w = h.deliver_engage(KEY_B, "t1");
    h.send(&claude(KEY_B, "beta"), &to("gamma"), "hop one");
    h.ib.reply(&claude(KEY_B, "beta"), &d1.id, "done").unwrap();
    let (_, held) = h.send(&grok(KEY_G, "grok"), &to("alpha"), "bypass hold");
    let (_, dismissed) = h.send(&grok(KEY_G, "grok"), &to("beta"), "dismiss me");
    h.ib.dismiss(&dismissed.id, USER).unwrap();
    let (_, repo) = h.send(&claude(KEY_C, "gamma"), &to_repo("repo1"), "repo work");
    h.wake(KEY_A);
    let (_, unowned) = h.send(&claude(KEY_A, "alpha"), &to_repo("repo9"), "nobody here");
    let (_, deferred) = h.send(&person(), &to("gamma"), "from the GUI");
    h.ib.ack(KEY_C, &deferred.id).unwrap();
    h.ib.defer(KEY_C, &deferred.id, 45, "after the build")
        .unwrap();
    h.ib.set_policy(
        Policy {
            mode: policy_mode::EVERYONE.into(),
            block: vec!["spam".into()],
            allow: vec!["grok".into()],
            paused: false,
        },
        USER,
    )
    .unwrap();
    h.clk.advance(UNENGAGED_LIMIT + secs(1));
    h.sweep();
    h.clk.advance(mins(1));
    h.sweep();
    let wc = h.wake(KEY_C);
    h.ib.mark_delivered(KEY_C, &wc.token).unwrap();
    h.wake(KEY_B);

    let agents = [KEY_A, KEY_B, KEY_C, KEY_G, USER];
    let before = snap(&h.ib, &agents);
    let states: std::collections::HashSet<String> =
        before.list.iter().map(|d| d.state.clone()).collect();
    for s in [
        st::QUEUED,
        st::HELD,
        st::DELIVERED,
        st::ENGAGED,
        st::ANSWERED,
        st::DEFERRED,
        st::DISMISSED,
        st::MOVED,
    ] {
        assert!(
            states.contains(s),
            "scenario has no {s} delivery: {states:?}"
        );
    }
    assert_eq!(
        before.outstanding[KEY_B].as_ref().map(|x| x.1),
        Some(st::QUEUED)
    );
    assert_eq!(
        before.outstanding[KEY_C].as_ref().map(|x| x.1),
        Some(st::DELIVERED)
    );
    assert_ne!(before.running[KEY_B], "");
    h.reopen();
    let after = snap(&h.ib, &agents);
    for (i, (b, a)) in before.list.iter().zip(&after.list).enumerate() {
        assert_eq!(b, a, "delivery {i}");
    }
    assert_eq!(before, after);
    for id in [&held.id, &unowned.id, &repo.id] {
        assert!(h.ib.get(id).is_some(), "lost {id}");
    }
    let (m, _) = h.send(&claude(KEY_B, "beta"), &to("gamma"), "hop again");
    assert_eq!(m.hop, 1);
    h.ib.mark_engaged(KEY_B, &w.token, "t1").unwrap();
    h.ib.turn_ended(KEY_B, &w.token, "t1").unwrap();
    assert_eq!(h.ib.running_wake(KEY_B), "");
}

#[test]
fn replay_pages_past_list_cap() {
    let mut h = Harness::new();
    h.abc();
    let n = 600;
    for i in 0..n {
        let from = if i % 2 == 1 {
            person()
        } else {
            claude(KEY_A, "alpha")
        };
        h.send(&from, &to("beta"), &format!("m{i}"));
        if i % 7 == 0 {
            h.clk.advance(DIRECT_WINDOW);
        }
    }
    let before = h.ib.for_agent(KEY_B, true, 10000);
    h.reopen();
    let after = h.ib.for_agent(KEY_B, true, 10000);
    assert!(
        after.len() == n && before == after,
        "replayed {} of {n}",
        after.len()
    );
}

// ---- owner_test.go ---------------------------------------------------------

fn owners2() -> Hooks {
    Hooks {
        is_owner: Some(Arc::new(|k: &str| k == USER || k == "user:dev2")),
        ..Hooks::default()
    }
}

fn actor_of_last(h: &Harness, kind: &str) -> String {
    let evs =
        h.st.list(&Filter {
            kinds: vec![kind.into()],
            desc: true,
            limit: 1,
            ..Default::default()
        })
        .unwrap();
    evs[0].actor.clone()
}

#[test]
fn owner_keys_and_made_up_keys() {
    let h = Harness::with(owners2());
    h.abc();
    h.ib.set_policy(
        Policy {
            mode: policy_mode::ALLOWLIST.into(),
            ..Policy::default()
        },
        USER,
    )
    .unwrap();
    let other = Sender::new("user:dev2", "you", "user");
    let fake = Sender::new("user:anything", "you", "user");
    let (m, d) = h.send(&other, &to("alpha"), "from the Mac");
    assert!(d.state != st::HELD && m.trust == trust::USER);
    assert_eq!(actor_of_last(&h, kinds::MESSAGE_SENT), "user");
    let (m, d) = h.send(&fake, &to("alpha"), "trust me");
    assert!(d.state == st::HELD && d.hold == hold::NOT_ALLOWED && m.trust == trust::UNTRUSTED);
    assert_eq!(
        actor_of_last(&h, kinds::MESSAGE_SENT),
        "agent:user:anything"
    );
    h.ib.set_policy(Policy::default(), USER).unwrap();
    for _ in 0..DIRECT_LIMIT + 2 {
        h.send(&other, &to("beta"), "again");
    }
    for i in 0..DIRECT_LIMIT {
        h.ib.send(&fake, &to("beta"), "again", "")
            .unwrap_or_else(|e| panic!("send {i}: {e}"));
    }
    want_err(
        h.ib.send(&fake, &to("beta"), "one more", ""),
        ErrorKind::RateLimited,
    );
    let api = Sender::new(&format!("api:composer@{KEY_A}"), "composer", "claude");
    h.send(&api, &to("gamma"), "via the API");
    assert_eq!(
        actor_of_last(&h, kinds::MESSAGE_SENT),
        format!("api:composer@{KEY_A}")
    );
}

#[test]
fn policy_is_device_local() {
    let mut h = Harness::new();
    h.ib.set_policy(
        Policy {
            mode: policy_mode::ALLOWLIST.into(),
            allow: vec!["alpha".into()],
            paused: true,
            ..Policy::default()
        },
        USER,
    )
    .unwrap();
    let f = |local| Filter {
        kinds: vec![kinds::POLICY_SET.into()],
        local,
        ..Default::default()
    };
    assert!(h.st.list(&f(false)).unwrap().is_empty());
    assert_eq!(h.st.list(&f(true)).unwrap().len(), 1);
    h.reopen();
    let p = h.ib.policy();
    assert!(p.mode == policy_mode::ALLOWLIST && p.paused && p.allow.len() == 1);
}

#[test]
fn owner_only_rows_from_the_wrong_device() {
    let mut h = Harness::new();
    h.abc();
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "hi");
    let at = (add(h.clk.now(), mins(60)).unix_milli_value() as u64) << 16;
    let foreign = |seq: i64, kind: &str, p: serde_json::Value| {
        h.st.insert(&Event {
            device_id: "dev2".into(),
            seq,
            hlc: Timestamp(at + seq as u64),
            actor: "irori".into(),
            kind: kind.into(),
            payload: serde_json::to_vec(&p).unwrap(),
            sig: vec![],
        })
        .unwrap();
    };
    foreign(
        1,
        kinds::DELIVERY_STATE,
        json!({"deliveryId": d.id, "messageId": d.message_id, "to": KEY_B, "state": "dismissed", "by": "dev2/x"}),
    );
    foreign(
        2,
        kinds::WAKE_ISSUED,
        json!({"wakeId": "w_forged", "to": KEY_B, "token": "[irori-wake:forged]", "reminder": false, "deliveryIds": [d.id]}),
    );
    foreign(
        3,
        kinds::TURN_ENGAGED,
        json!({"agent": KEY_B, "wakeId": "w_forged", "turnId": "t"}),
    );
    foreign(
        4,
        kinds::DELIVERY_STATE,
        json!({"deliveryId": "del_from_dev2", "messageId": d.message_id, "to": KEY_B, "state": "queued", "by": "dev2/x"}),
    );
    foreign(
        5,
        kinds::DELIVERY_STATE,
        json!({"deliveryId": "del_prewoken", "messageId": d.message_id, "to": KEY_B, "state": "queued", "wakeId": "w_x", "by": "dev2/x"}),
    );
    foreign(
        6,
        kinds::DELIVERY_STATE,
        json!({"deliveryId": "del_delivered", "messageId": d.message_id, "to": KEY_B, "state": "delivered", "by": "dev2/x"}),
    );
    h.reopen();
    let got = h.ib.get(&d.id).unwrap();
    assert!(got.state == st::QUEUED && got.wake_id != "w_forged");
    assert_eq!(h.ib.ignored_rows(), 5);
    assert_eq!(h.ib.ignored_by()["dev2"], 5);
    for id in ["del_prewoken", "del_delivered"] {
        assert!(h.ib.get(id).is_none(), "{id} landed");
    }
    assert!(h.ib.get("del_from_dev2").is_some());
}

#[test]
fn made_up_user_key_is_an_agent_in_search() {
    let h = Harness::with(owners2());
    h.ros.put(&[card(KEY_A, "alpha", "repo1", "default")]);
    let fake = Sender::new("user:anything", "you", "user");
    let other = Sender::new("user:dev2", "you", "user");
    let (m, _) = h.send(&fake, &to("alpha"), "trust me");
    let d = docs(&h).remove(&format!("msg:{}", m.id)).unwrap();
    assert!(
        d.agent == fake.key
            && d.readers.contains(&fake.key)
            && d.readers.contains(&KEY_A.to_string()),
        "{d:?}"
    );
    // A known sender is addressable by key.
    let (_, back) = h.send(&claude(KEY_A, "alpha"), &to("user:anything"), "hi");
    assert_eq!(back.to, "user:anything");
    let (m, _) = h.send(&other, &to("alpha"), "from the Mac");
    let d = docs(&h).remove(&format!("msg:{}", m.id)).unwrap();
    assert!(d.agent == KEY_A && !d.readers.contains(&other.key), "{d:?}");
    want_err(
        h.ib.send(&claude(KEY_A, "alpha"), &to("user:dev2"), "hi", ""),
        ErrorKind::UnknownAgent,
    );
}

// ---- stale_test.go ---------------------------------------------------------

#[test]
fn stale_notices_are_skipped() {
    let taken: Arc<Mutex<HashMap<String, String>>> = Arc::default();
    let tk = taken.clone();
    let mut h = Harness::with(Hooks {
        stale: Some(Arc::new(move |d: &Delivery| {
            let t = tk.lock().unwrap();
            match t.get(&d.message.ref_) {
                Some(k) if *k != d.to => format!("taken by {k}"),
                _ => String::new(),
            }
        })),
        ..Hooks::default()
    });
    h.abc();
    let ask = || SendOptions {
        notice: true,
        ref_: "task:01ASK".into(),
        mode: mode::NEXT_TURN.into(),
        ..Default::default()
    };
    let to_a = h.send_with(&person(), &to("alpha"), "the person asks the room", ask());
    let to_b = h.send_with(&person(), &to("beta"), "the person asks the room", ask());
    let to_c = h.send_with(
        &person(),
        &to("gamma"),
        "the person asks the room",
        SendOptions {
            mode: mode::WAKE.into(),
            ..ask()
        },
    );
    let plain = h.send_with(
        &person(),
        &to("beta"),
        "a plain message",
        SendOptions {
            ref_: "task:01ASK".into(),
            ..Default::default()
        },
    );
    taken
        .lock()
        .unwrap()
        .insert("task:01ASK".into(), KEY_A.into());
    assert_eq!(h.append_for(KEY_B).delivery_ids, vec![plain.id.clone()]);
    assert_eq!(
        h.want_state(&to_b.id, st::SKIPPED).note,
        format!("taken by {KEY_A}")
    );
    h.no_wake(KEY_C);
    h.want_state(&to_c.id, st::SKIPPED);
    assert_eq!(h.append_for(KEY_A).delivery_ids, vec![to_a.id.clone()]);
    let to_b2 = h.send_with(&person(), &to("beta"), "again", ask());
    h.sweep();
    h.want_state(&to_b2.id, st::SKIPPED);
    h.reopen();
    h.want_state(&to_b.id, st::SKIPPED);
}

// ---- search_test.go --------------------------------------------------------

fn docs(h: &Harness) -> HashMap<String, inbox::search::MsgDoc> {
    docs_since(h, &GoTime::ZERO)
}

fn docs_since(h: &Harness, since: &GoTime) -> HashMap<String, inbox::search::MsgDoc> {
    let ds = h.ib.search_docs(since, 0);
    for w in ds.windows(2) {
        assert!(
            w[0].updated.unix_nano() <= w[1].updated.unix_nano(),
            "docs not oldest first"
        );
    }
    ds.into_iter().map(|d| (d.ref_.clone(), d)).collect()
}

#[test]
fn search_visibility() {
    let mut h = Harness::new();
    h.abc();
    h.ros.put(&[grok_card(KEY_G, "grok", "")]);
    let t0 = h.clk.now();
    let (plain, _) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "parser plan");
    h.clk.advance(mins(1));
    let (held, hd) = h.send(&grok(KEY_G, "grok"), &to("beta"), "held words");
    let (dropped, dd) = h.send(&grok(KEY_G, "grok"), &to("gamma"), "dismissed words");
    assert!(hd.state == st::HELD && dd.state == st::HELD);
    h.ib.dismiss(&dd.id, USER).unwrap();
    let r = |id: &str| format!("msg:{id}");
    let got = docs(&h);
    for (m, want) in [(&plain, true), (&held, false), (&dropped, false)] {
        assert_eq!(got.contains_key(&r(&m.id)), want, "{}", m.text);
        assert_eq!(h.ib.search_show(&m.id, "").is_some(), want, "{}", m.text);
    }
    let d = &got[&r(&plain.id)];
    assert!(
        d.updated == plain.sent_at
            && d.at == t0
            && d.agent == KEY_A
            && d.body == "parser plan"
            && d.title == "alpha -> beta"
    );
    h.clk.advance(mins(60));
    let rel = h.clk.now();
    h.ib.release(&hd.id, USER).unwrap();
    let later = docs_since(&h, &rel);
    let d = &later[&r(&held.id)];
    assert!(d.updated == rel && d.at == held.sent_at && later.len() == 1);
    h.reopen();
    let again = docs_since(&h, &rel);
    assert!(again.len() == 1 && again[&r(&held.id)].updated == rel);
    assert!(!docs(&h).contains_key(&r(&dropped.id)));
}

#[test]
fn search_doc_fields() {
    let h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_A, path::MOD, true);
    const KEY_D: &str = "dev1/dddddddd-4444-4444-8444-444444444444";
    h.ros.put(&[card(
        KEY_D,
        "delta",
        "remote:github.com/acme/app",
        inbox::BYPASS_MODE,
    )]);
    h.ib.set_reachable(KEY_D, path::MOD, true);
    let cases = [
        (
            "agent to agent",
            claude(KEY_B, "beta"),
            to("gamma"),
            "",
            "beta -> gamma",
            KEY_B,
            "",
        ),
        (
            "with a reason",
            claude(KEY_B, "beta"),
            to("gamma"),
            "review",
            "beta -> gamma: review",
            KEY_B,
            "",
        ),
        (
            "to a repo",
            claude(KEY_C, "gamma"),
            to_repo("repo1"),
            "",
            "gamma -> repo repo1",
            KEY_C,
            "repo1",
        ),
        (
            "a person: the recipient's agent",
            person(),
            to("gamma"),
            "",
            "Stephen -> gamma",
            KEY_C,
            "",
        ),
        (
            "Irori: the recipient's agent",
            system_sender(),
            to("gamma"),
            "",
            "Irori -> gamma",
            KEY_C,
            "",
        ),
        (
            "to a PR",
            claude(KEY_B, "beta"),
            Address::new(addr::PR, "acme/app#7"),
            "",
            "beta -> pr:acme/app#7",
            KEY_B,
            "remote:github.com/acme/app",
        ),
    ];
    for (name, from, a, reason, title, agent, repo) in cases {
        let (m, _) =
            h.ib.send_with(
                &from,
                &a,
                &format!("body {name}"),
                SendOptions {
                    reason: reason.into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let d = docs(&h)
            .remove(&format!("msg:{}", m.id))
            .unwrap_or_else(|| panic!("{name}: not found"));
        assert!(
            d.title == title
                && d.agent == agent
                && d.repo_key == repo
                && d.body == format!("body {name}"),
            "{name}: {d:?}"
        );
    }
    assert_eq!(h.ib.search_docs(&GoTime::ZERO, 2).len(), 2);
}

#[test]
fn search_show() {
    let h = Harness::new();
    h.abc();
    h.ros.put(&[grok_card(KEY_G, "grok", "")]);
    let orig = h.send_with(
        &claude(KEY_A, "alpha"),
        &to("beta"),
        "can you take the lexer?",
        SendOptions {
            ref_: "task:01LEX".into(),
            reason: "hand-off".into(),
            ..Default::default()
        },
    );
    let (_, r1) = h.ib.reply(&claude(KEY_B, "beta"), &orig.id, "yes").unwrap();
    h.wake(KEY_A);
    h.ib.send(
        &grok(KEY_G, "grok"),
        &Address::new(addr::REPLY, &orig.message_id),
        "me too",
        "",
    )
    .unwrap();
    let s = h.ib.search_show(&orig.message_id, "").unwrap();
    assert!(s.ref_ == format!("msg:{}", orig.message_id) && s.title == "alpha -> beta: hand-off");
    for want in [
        format!("from: alpha ({KEY_A}) trust=untrusted"),
        "about: task:01LEX".to_string(),
        format!("{} to beta ({KEY_B}): answered", orig.id),
        "can you take the lexer?".to_string(),
    ] {
        assert!(s.text.contains(&want), "text lacks {want:?}:\n{}", s.text);
    }
    assert!(s.text.ends_with("can you take the lexer?\n"));
    assert!(
        s.text.contains("sent: 2026-10-26T09:00:00Z\n"),
        "{}",
        s.text
    );
    assert_eq!(
        s.related,
        vec!["task:01LEX".to_string(), format!("msg:{}", r1.message_id)]
    );
    let back = h.ib.search_show(&r1.message_id, "").unwrap();
    assert_eq!(
        back.related,
        vec![format!("msg:{}", orig.message_id), "task:01LEX".into()]
    );
    assert!(h.ib.search_show("msg_nope", "").is_none());
    // Through search::Source: the same rendering, paged, with Go's errors.
    use search::{Kind, ShowOptions, Source};
    let r = format!(" MSG:{}", orig.message_id);
    let sh = Source::search_show(&*h.ib, &r, &ShowOptions::default()).unwrap();
    assert!(sh.kind == Kind::Msg && sh.text == s.text && !sh.more && sh.related == s.related);
    let p1 = Source::search_show(
        &*h.ib,
        &r,
        &ShowOptions {
            limit: 20,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(p1.more && p1.next == 20 && s.text.starts_with(&p1.text));
    for bad in ["msg:msg_nope", "task:x"] {
        let e = Source::search_show(&*h.ib, bad, &ShowOptions::default()).unwrap_err();
        assert!(e.is_not_found(), "{bad}: {e}");
    }
    // Readers: a stranger agent finds nothing.
    let e = Source::search_show(
        &*h.ib,
        &r,
        &ShowOptions {
            viewer: "dev-x/stranger".into(),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(e.is_not_found());
    assert_eq!(Source::search_kinds(&*h.ib), vec![Kind::Msg]);
    let docs = Source::search_docs(&*h.ib, GoTime::ZERO, 0).unwrap();
    assert_eq!(docs.len(), h.ib.search_docs(&GoTime::ZERO, 0).len());
}

#[test]
fn search_readers() {
    let h = Harness::new();
    h.abc();
    let (m, _) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "private to beta");
    let d = docs(&h).remove(&format!("msg:{}", m.id)).unwrap();
    assert!(
        d.readers.contains(&KEY_A.to_string())
            && d.readers.contains(&KEY_B.to_string())
            && !d.readers.contains(&KEY_C.to_string())
    );
    for (viewer, ok) in [(KEY_A, true), (KEY_B, true), (KEY_C, false), ("", true)] {
        assert_eq!(
            h.ib.search_show(&m.id, viewer).is_some(),
            ok,
            "viewer {viewer:?}"
        );
    }
}
