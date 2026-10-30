//! I:internal/inbox/policy_test.go and review_test.go.

mod common;

use common::*;
use inbox::{
    BYPASS_MODE, ErrorKind, GONE_GRACE, MAX_DEFER, MAX_HOP, Policy, SYSTEM_KEY, Sender,
    UNENGAGED_LIMIT, hold, path, policy_mode, st, system_sender,
};

fn pol(mode: &str, block: &[&str], allow: &[&str], paused: bool) -> Policy {
    Policy {
        mode: mode.into(),
        block: block.iter().map(|s| s.to_string()).collect(),
        allow: allow.iter().map(|s| s.to_string()).collect(),
        paused,
    }
}

#[test]
fn policy_holds() {
    const KEY_N: &str = "dev1/nnnnnnnn-5555-4555-8555-555555555555";
    const KEY_U: &str = "dev1/uuuuuuuu-6666-4666-8666-666666666666";
    const KEY_X: &str = "devX/xxxxxxxx-7777-4777-8777-777777777777";
    let none = |p: Policy| p;
    let e = || pol("", &[], &[], false);
    let cases: Vec<(&str, Policy, Sender, &str, &str)> = vec![
        (
            "bypass to bypass flows",
            e(),
            claude(KEY_A, "alpha"),
            KEY_B,
            "",
        ),
        (
            "grok to a bypass receiver is held",
            e(),
            grok(KEY_G, "grok"),
            KEY_B,
            hold::BYPASS_RECEIVER,
        ),
        (
            "own default-mode sender to bypass flows",
            e(),
            claude(KEY_N, "normal"),
            KEY_B,
            "",
        ),
        (
            "own unknown-mode sender to bypass flows",
            e(),
            claude(KEY_U, "unknown"),
            KEY_B,
            "",
        ),
        (
            "unknown device's sender to bypass is held",
            e(),
            claude(KEY_X, "stranger"),
            KEY_B,
            hold::BYPASS_RECEIVER,
        ),
        (
            "grok under a claude key's device is held",
            e(),
            grok("dev1/grok-other", "grok2"),
            KEY_B,
            hold::BYPASS_RECEIVER,
        ),
        (
            "a grok card sending as claude is held",
            e(),
            claude("dev1/grok-other", "grok2"),
            KEY_B,
            hold::BYPASS_RECEIVER,
        ),
        (
            "own sender still blocked",
            pol("", &["normal"], &[], false),
            claude(KEY_N, "normal"),
            KEY_B,
            hold::BLOCKED,
        ),
        (
            "own sender still paused",
            pol("", &[], &[], true),
            claude(KEY_N, "normal"),
            KEY_B,
            hold::PAUSED,
        ),
        (
            "own sender still needs the allowlist",
            pol(policy_mode::ALLOWLIST, &[], &[], false),
            claude(KEY_N, "normal"),
            KEY_B,
            hold::NOT_ALLOWED,
        ),
        (
            "to an unknown-mode receiver flows",
            e(),
            grok(KEY_G, "grok"),
            KEY_U,
            "",
        ),
        ("to grok flows", e(), claude(KEY_N, "normal"), KEY_G, ""),
        (
            "allow-listed grok reaches bypass",
            pol("", &[], &["GROK"], false),
            grok(KEY_G, "grok"),
            KEY_B,
            "",
        ),
        ("the person reaches bypass", e(), person(), KEY_B, ""),
        (
            "blocked by key",
            pol("", &[KEY_A], &[], false),
            claude(KEY_A, "alpha"),
            KEY_B,
            hold::BLOCKED,
        ),
        (
            "blocked by name, any case",
            pol("", &["Alpha"], &[], false),
            claude(KEY_A, "alpha"),
            KEY_B,
            hold::BLOCKED,
        ),
        (
            "block beats allow",
            pol("", &["alpha"], &["alpha"], false),
            claude(KEY_A, "alpha"),
            KEY_B,
            hold::BLOCKED,
        ),
        (
            "the person can't be blocked",
            pol("", &[USER, "stephen"], &[], false),
            person(),
            KEY_B,
            "",
        ),
        (
            "allowlist without the sender",
            pol(policy_mode::ALLOWLIST, &[], &[], false),
            claude(KEY_A, "alpha"),
            KEY_B,
            hold::NOT_ALLOWED,
        ),
        (
            "allowlist with the sender",
            pol(policy_mode::ALLOWLIST, &[], &[KEY_A], false),
            claude(KEY_A, "alpha"),
            KEY_B,
            "",
        ),
        (
            "allowlist and the person",
            pol(policy_mode::ALLOWLIST, &[], &[], false),
            person(),
            KEY_B,
            "",
        ),
        (
            "paused holds agents",
            pol("", &[], &[], true),
            claude(KEY_A, "alpha"),
            KEY_B,
            hold::PAUSED,
        ),
        (
            "paused holds the person too",
            pol("", &[], &[], true),
            person(),
            KEY_B,
            hold::PAUSED,
        ),
        (
            "blocked while paused stays blocked",
            pol("", &["alpha"], &[], true),
            claude(KEY_A, "alpha"),
            KEY_B,
            hold::BLOCKED,
        ),
        ("Irori reaches bypass", e(), system_sender(), KEY_B, ""),
        (
            "Irori passes allowlist mode",
            pol(policy_mode::ALLOWLIST, &[], &[], false),
            system_sender(),
            KEY_B,
            "",
        ),
        (
            "Irori blocked by key",
            pol("", &[SYSTEM_KEY], &[], false),
            system_sender(),
            KEY_B,
            hold::BLOCKED,
        ),
        (
            "Irori blocked by name",
            pol("", &["IRORI"], &[], false),
            system_sender(),
            KEY_B,
            hold::BLOCKED,
        ),
        (
            "paused holds Irori",
            pol("", &[], &[], true),
            system_sender(),
            KEY_B,
            hold::PAUSED,
        ),
        (
            "open: grok to a bypass receiver flows",
            pol(policy_mode::OPEN, &[], &[], false),
            grok(KEY_G, "grok"),
            KEY_B,
            "",
        ),
        (
            "open: unknown device's sender to bypass flows",
            pol(policy_mode::OPEN, &[], &[], false),
            claude(KEY_X, "stranger"),
            KEY_B,
            "",
        ),
        (
            "open: blocked grok still held",
            pol(policy_mode::OPEN, &["grok"], &[], false),
            grok(KEY_G, "grok"),
            KEY_B,
            hold::BLOCKED,
        ),
        (
            "open: paused still holds",
            pol(policy_mode::OPEN, &[], &[], true),
            grok(KEY_G, "grok"),
            KEY_B,
            hold::PAUSED,
        ),
    ];
    for (name, p, from, dst, want) in cases {
        let h = Harness::new();
        h.abc();
        h.ros.put(&[
            grok_card(KEY_G, "grok", ""),
            card(KEY_N, "normal", "", "default"),
            card(KEY_U, "unknown", "", ""),
            card(KEY_X, "stranger", "", "default"),
            grok_card("dev1/grok-other", "grok2", ""),
        ]);
        h.ib.set_policy(none(p), USER).unwrap();
        let (_, d) = h.ib.send(&from, &to(dst), "hi", "").unwrap();
        assert!(
            d.hold == want
                && (want.is_empty()) == (d.state == st::QUEUED)
                && (!want.is_empty()) == (d.state == st::HELD),
            "{name}: state {} hold {:?}, want hold {want:?}",
            d.state,
            d.hold
        );
    }
}

#[test]
fn policy_alias_block_and_validation() {
    let h = Harness::new();
    h.abc();
    h.send(&claude(KEY_A, "Old Name"), &to("beta"), "x");
    h.ib.set_policy(pol("", &[" old name ", "", "old name"], &[], false), USER)
        .unwrap();
    let p = h.ib.policy();
    assert!(
        p.block.len() == 1 && p.mode == policy_mode::EVERYONE,
        "{p:?}"
    );
    let (_, d) = h.send(&claude(KEY_A, "new name"), &to("beta"), "x");
    assert_eq!(d.hold, hold::BLOCKED);
    let e =
        h.ib.set_policy(pol("sometimes", &[], &[], false), USER)
            .unwrap_err();
    assert_eq!(
        e.to_string(),
        "inbox: unknown policy mode \"sometimes\" (everyone, allowlist or open)"
    );
    h.ib.set_policy(pol(policy_mode::OPEN, &["old name"], &[], false), USER)
        .unwrap();
    assert_eq!(h.ib.policy().mode, policy_mode::OPEN);
    let mut p = h.ib.policy();
    p.block[0] = "changed".into();
    assert_eq!(h.ib.policy().block[0], "old name");
}

#[test]
fn release_dismiss_and_unpause() {
    let h = Harness::new();
    h.abc();
    h.ros.put(&[grok_card(KEY_G, "grok", "")]);
    let (_, d1) = h.send(&grok(KEY_G, "grok"), &to("beta"), "held 1");
    let (_, d2) = h.send(&grok(KEY_G, "grok"), &to("beta"), "held 2");
    h.ib.release(&d1.id, USER).unwrap();
    assert_eq!(h.want_state(&d1.id, st::QUEUED).hold, "");
    want_err(h.ib.release(&d1.id, USER), ErrorKind::BadState);
    want_err(h.ib.release("dlv_nope", USER), ErrorKind::NotFound);
    h.ib.dismiss(&d2.id, USER).unwrap();
    h.want_state(&d2.id, st::DISMISSED);
    h.ib.dismiss(&d2.id, USER).unwrap();
    want_err(h.ib.dismiss(&d1.id, USER), ErrorKind::BadState);
    want_err(h.ib.dismiss("dlv_nope", USER), ErrorKind::NotFound);

    h.ib.set_policy(pol("", &[], &[], true), USER).unwrap();
    h.no_wake(KEY_B);
    let (_, p1) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "paused 1");
    let (_, p2) = h.send(&claude(KEY_C, "gamma"), &to("beta"), "paused 2");
    assert!(p1.hold == hold::PAUSED && p2.hold == hold::PAUSED);
    h.ib.set_policy(pol("", &["gamma"], &[], false), USER)
        .unwrap();
    h.want_state(&p1.id, st::QUEUED);
    assert_eq!(h.want_state(&p2.id, st::HELD).hold, hold::BLOCKED);
    let w = h.wake(KEY_B);
    assert_eq!(w.delivery_ids, vec![d1.id.clone(), p1.id.clone()]);
}

#[test]
fn full_auto_keeps_the_hop_limit() {
    let h = Harness::new();
    h.abc();
    h.ib.set_policy(pol(policy_mode::OPEN, &[], &[], false), USER)
        .unwrap();
    let agents = [
        claude(KEY_A, "alpha"),
        claude(KEY_B, "beta"),
        claude(KEY_C, "gamma"),
    ];
    let (_, mut d) = h.send(&agents[0], &to(KEY_B), "hop 0");
    let mut i = 1usize;
    loop {
        let cur = &agents[i % 3];
        if d.state == st::HELD {
            assert!(d.hold == hold::HOP_LIMIT && d.message.hop == MAX_HOP + 1);
            break;
        }
        let w = h.deliver_engage(&cur.key, &format!("t{i}"));
        d = h
            .send(cur, &to(&agents[(i + 1) % 3].key), &format!("hop {i}"))
            .1;
        h.ib.turn_ended(&cur.key, &w.token, &format!("t{i}"))
            .unwrap();
        assert!(i <= MAX_HOP as usize + 2, "never held");
        i += 1;
    }
}

#[test]
fn allow_list_ignores_self_chosen_name() {
    const KEY_N: &str = "devX/nnnnnnnn-5555-4555-8555-555555555555";
    let h = Harness::new();
    h.abc();
    h.ros.put(&[
        grok_card(KEY_G, "grok", ""),
        card(KEY_N, "normal", "", "default"),
    ]);
    h.ib.set_policy(pol("", &[], &["gamma"], false), USER)
        .unwrap();
    for from in [grok(KEY_G, "gamma"), claude(KEY_N, "gamma")] {
        let (_, d) = h.send(&from, &to("alpha"), "spoof");
        assert!(
            d.state == st::HELD && d.hold == hold::BYPASS_RECEIVER,
            "{}: {} {}",
            from.key,
            d.state,
            d.hold
        );
    }
    let (_, d) = h.send(&grok(KEY_G, "gamma"), &to("alpha"), "again");
    assert_eq!(d.hold, hold::BYPASS_RECEIVER);
    h.ib.set_policy(pol(policy_mode::ALLOWLIST, &[], &["GROK"], false), USER)
        .unwrap();
    let (_, d) = h.send(&grok(KEY_G, "someone"), &to("alpha"), "ok");
    assert_eq!(d.state, st::QUEUED);
    let (_, d) = h.send(&claude(KEY_N, "grok"), &to("alpha"), "spoof");
    assert_eq!(d.hold, hold::NOT_ALLOWED);
    h.ib.set_policy(pol("", &["gamma"], &[], false), USER)
        .unwrap();
    let (_, d) = h.send(&grok(KEY_G, "grok"), &to("beta"), "x");
    assert_eq!(d.hold, hold::BLOCKED);
}

#[test]
fn alias_cannot_hijack_card_name() {
    let h = Harness::new();
    h.abc();
    h.ros.put(&[grok_card(KEY_G, "grok", "")]);
    h.send(&grok(KEY_G, "beta"), &to("alpha"), "I'm beta now");
    assert_eq!(
        h.send(&claude(KEY_A, "alpha"), &to("beta"), "live").1.to,
        KEY_B
    );
    h.ros.remove(&[KEY_B]);
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "gone");
    assert!(d.to == KEY_B && d.state == st::QUEUED);
    h.send(&grok(KEY_G, "zeta"), &to("alpha"), "x");
    assert_eq!(
        h.send(&claude(KEY_A, "alpha"), &to("zeta"), "y").1.to,
        KEY_G
    );
}

#[test]
fn held_not_shown_to_recipient() {
    let h = Harness::new();
    h.abc();
    h.ros.put(&[grok_card(KEY_G, "grok", "")]);
    let (_, held) = h.send(&grok(KEY_G, "grok"), &to("alpha"), "SECRET held text");
    let (_, dis) = h.send(&grok(KEY_G, "grok"), &to("alpha"), "SECRET dismissed text");
    h.ib.dismiss(&dis.id, USER).unwrap();
    let (_, ok) = h.send(&claude(KEY_B, "beta"), &to("alpha"), "fine");
    for all in [false, true] {
        let got = h.ib.for_agent(KEY_A, all, 0);
        assert!(got.len() == 1 && got[0].id == ok.id, "all={all}: {got:?}");
    }
    let seen: Vec<String> = h.ib.list(0).into_iter().map(|d| d.id).collect();
    assert!(seen.contains(&held.id) && seen.contains(&dis.id));
    h.ib.release(&held.id, USER).unwrap();
    assert_eq!(h.ib.for_agent(KEY_A, false, 0).len(), 2);
}

#[test]
fn repo_work_never_assigned_to_sender() {
    // assign and release
    {
        let h = Harness::new();
        h.abc();
        h.ib.set_reachable(KEY_A, path::MOD, true);
        let (_, d1) = h.send(&claude(KEY_A, "alpha"), &to_repo("repo1"), "one");
        let (_, d2) = h.send(&claude(KEY_A, "alpha"), &to_repo("repo1"), "two");
        assert_eq!(d1.hold, hold::UNOWNED);
        h.sweep();
        assert_eq!(h.want_state(&d1.id, st::HELD).to, "");
        want_err(h.ib.release(&d2.id, USER), ErrorKind::BadState);
        assert_eq!(h.state(&d2.id).to, "");
        h.ib.set_reachable(KEY_B, path::MOD, true);
        h.ib.release(&d2.id, USER).unwrap();
        assert_eq!(h.want_state(&d2.id, st::QUEUED).to, KEY_B);
        h.sweep();
        assert_eq!(h.want_state(&d1.id, st::QUEUED).to, KEY_B);
    }
    // move when the holder goes
    {
        let h = Harness::new();
        h.abc();
        h.ib.set_reachable(KEY_A, path::MOD, true);
        h.clk.advance(secs(1));
        h.ib.set_reachable(KEY_B, path::MOD, true);
        let (_, d) = h.send(&claude(KEY_A, "alpha"), &to_repo("repo1"), "work");
        assert_eq!(d.to, KEY_B);
        h.ros.remove(&[KEY_B]);
        h.sweep();
        h.clk.advance(GONE_GRACE + secs(1));
        h.sweep();
        assert_eq!(h.want_state(&d.id, st::MOVED).moved_to, "");
        for x in h.ib.list(0) {
            if x.message_id == d.message_id && x.id != d.id {
                assert!(
                    x.to.is_empty() && x.hold == hold::UNOWNED,
                    "new delivery to {} ({} {})",
                    x.to,
                    x.state,
                    x.hold
                );
            }
        }
    }
    // move when the holder doesn't start
    {
        let h = Harness::new();
        h.abc();
        h.ib.set_reachable(KEY_A, path::MOD, true);
        h.clk.advance(secs(1));
        h.ib.set_reachable(KEY_B, path::MOD, true);
        let (_, d) = h.send(&claude(KEY_A, "alpha"), &to_repo("repo1"), "work");
        h.clk.advance(UNENGAGED_LIMIT + secs(1));
        h.sweep();
        assert_eq!(h.want_state(&d.id, st::QUEUED).to, KEY_B);
    }
}

#[test]
fn delivered_never_engaged_is_reminded() {
    let h = Harness::new();
    h.abc();
    h.ib.set_reachable(KEY_B, path::MOD, true);
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "q");
    let w = h.wake(KEY_B);
    h.ib.mark_delivered(KEY_B, &w.token).unwrap();
    h.clk.advance(UNENGAGED_LIMIT);
    h.sweep();
    h.no_wake(KEY_B);
    h.clk.advance(secs(1));
    h.sweep();
    let r = h.wake(KEY_B);
    assert!(r.reminder && r.delivery_ids == vec![d.id.clone()], "{r:?}");
    h.want_state(&d.id, st::DELIVERED);
    for _ in 0..48 {
        h.clk.advance(mins(30));
        h.sweep();
        h.no_wake(KEY_B);
    }
}

#[test]
fn defer_clamps_huge_minutes() {
    let h = Harness::new();
    h.abc();
    let (_, d) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "q");
    for m in [
        i64::MAX,
        153722867280913,
        (MAX_DEFER.as_secs() / 60) as i64 + 1,
    ] {
        h.ib.defer(KEY_B, &d.id, m, "").unwrap();
        assert_eq!(
            h.state(&d.id).until,
            add(h.clk.now(), MAX_DEFER),
            "minutes {m}"
        );
    }
}

#[test]
fn rename_survives_replay() {
    let mut h = Harness::new();
    h.abc();
    h.send(&claude(KEY_B, "beta"), &to("alpha"), "x");
    h.ros.put(&[card(KEY_A, "alpha2", "repo1", BYPASS_MODE)]);
    h.send(&claude(KEY_B, "beta"), &to("alpha2"), "y");
    h.ros.remove(&[KEY_A]);
    let check = |h: &Harness| {
        let (_, d) =
            h.ib.send(&claude(KEY_B, "beta"), &to("alpha2"), "z", "")
                .unwrap();
        assert_eq!(d.to, KEY_A);
        want_err(
            h.ib.send(&claude(KEY_B, "beta"), &to("alpha"), "z", ""),
            ErrorKind::UnknownAgent,
        );
    };
    check(&h);
    h.reopen();
    check(&h);
}
