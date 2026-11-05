//! Card #372: an agent hears once when a fellow holder's device has been
//! offline past the grace, and once more when it's back; an agent its
//! card waits on counts too, and only agents this device hosts are told.

mod common;

use common::*;
use coord::roster::{AgentCard, CONNECTED, OFFLINE};
use coord::*;
use gocompat::time::GoTime;

/// Alice on this device; Bob on dev-b, whose device reads `bob_dev`.
fn agents(bob_dev: &str, seen: GoTime) -> Vec<AgentCard> {
    vec![
        AgentCard {
            key: ALICE.into(),
            status: "idle".into(),
            ..AgentCard::default()
        },
        AgentCard {
            key: "dev-b/bob".into(),
            name: "bob".into(),
            status: "busy".into(),
            device_id: "dev-b".into(),
            device_state: bob_dev.into(),
            device_seen: seen,
            ..AgentCard::default()
        },
    ]
}

fn here(k: &str) -> bool {
    k.starts_with("dev-a/")
}

fn shared_card(e: &Env) -> Task {
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "Pose transport".into(),
                shared: true,
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.join(ALICE, &t.id, ALICE).unwrap();
    e.c.join("dev-b/bob", &t.id, "dev-b/bob").unwrap();
    t
}

#[test]
fn a_fellow_holder_going_offline_past_the_grace_is_told_once_and_back_once() {
    let e = Env::new();
    let t = shared_card(&e);
    let t0 = e.clk.now();
    let seen = plus(t0, secs(0));
    let off = agents(OFFLINE, seen);

    // Online: nothing.
    assert!(
        e.c.sweep_presence_at(&agents(CONNECTED, seen), &t0, &here)
            .is_empty()
    );
    // Offline, within the grace: nothing yet.
    assert!(e.c.sweep_presence_at(&off, &t0, &here).is_empty());
    let mid = plus(t0, PRESENCE_GRACE / 2);
    assert!(e.c.sweep_presence_at(&off, &mid, &here).is_empty());

    // Past the grace: one notice, to Alice only (Bob is the one away).
    let late = plus(t0, PRESENCE_GRACE);
    let ns = e.c.sweep_presence_at(&off, &late, &here);
    assert_eq!(ns.len(), 1, "{ns:?}");
    let n = &ns[0];
    assert_eq!(n.kind, NOTICE_PRESENCE);
    assert_eq!(n.to, [ALICE]);
    assert_eq!(n.task_id, t.id);
    assert!(n.text.contains("device is offline since"), "{}", n.text);
    assert!(n.text.contains("You hold"), "{}", n.text);
    assert!(!n.wake, "a fellow holder going away doesn't wake");

    // Still offline: nothing more.
    let later = plus(t0, mins(30));
    assert!(e.c.sweep_presence_at(&off, &later, &here).is_empty());

    // Back: one notice.
    let ns =
        e.c.sweep_presence_at(&agents(CONNECTED, later), &later, &here);
    assert_eq!(ns.len(), 1, "{ns:?}");
    assert_eq!(ns[0].to, [ALICE]);
    assert!(ns[0].text.contains("back online"), "{}", ns[0].text);
    // And nothing after.
    assert!(
        e.c.sweep_presence_at(&agents(CONNECTED, later), &later, &here)
            .is_empty()
    );
}

#[test]
fn a_flap_shorter_than_the_grace_tells_nobody() {
    let e = Env::new();
    shared_card(&e);
    let t0 = e.clk.now();
    let off = agents(OFFLINE, t0);
    assert!(e.c.sweep_presence_at(&off, &t0, &here).is_empty());
    let back = plus(t0, secs(30));
    assert!(
        e.c.sweep_presence_at(&agents(CONNECTED, back), &back, &here)
            .is_empty()
    );
    // Offline again: the grace starts over.
    let again = plus(t0, secs(60));
    assert!(e.c.sweep_presence_at(&off, &again, &here).is_empty());
    let almost = plus(again, PRESENCE_GRACE / 2);
    assert!(e.c.sweep_presence_at(&off, &almost, &here).is_empty());
}

#[test]
fn an_agent_a_blocker_waits_on_counts_and_wakes() {
    let e = Env::new();
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "Merge".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
    e.c.block(
        ALICE,
        &t.id,
        "Waiting on bob's branch",
        Some(BlockedBy {
            kind: "agent".into(),
            reference: "bob".into(),
        }),
    )
    .unwrap();
    let t0 = e.clk.now();
    let off = agents(OFFLINE, t0);
    assert!(e.c.sweep_presence_at(&off, &t0, &here).is_empty());
    let ns =
        e.c.sweep_presence_at(&off, &plus(t0, PRESENCE_GRACE), &here);
    assert_eq!(ns.len(), 1, "{ns:?}");
    assert_eq!(ns[0].to, [ALICE]);
    assert!(ns[0].wake, "a wait on it wakes");
    assert!(ns[0].text.contains("waits on it"), "{}", ns[0].text);
}

#[test]
fn only_agents_this_device_hosts_are_told() {
    let e = Env::new();
    shared_card(&e);
    let t0 = e.clk.now();
    let off = agents(OFFLINE, t0);
    let nobody = |_: &str| false;
    assert!(e.c.sweep_presence_at(&off, &t0, &nobody).is_empty());
    assert!(
        e.c.sweep_presence_at(&off, &plus(t0, PRESENCE_GRACE), &nobody)
            .is_empty()
    );
}

#[test]
fn card_show_says_where_each_holders_device_is() {
    let e = Env::new();
    let t = shared_card(&e);
    // The Env's roster has no device states: this device's agents read
    // as online here; dev-b/bob isn't in it, so it shows bare.
    let sh = search::Source::search_show(
        &e.c,
        &format!("task:{}", t.id),
        &search::ShowOptions::default(),
    )
    .unwrap();
    assert!(
        sh.text.contains(&format!(
            "holders: {ALICE} (device online, this device), dev-b/bob\n"
        )),
        "{}",
        sh.text
    );
}

// Card #544: a device back through another one has no link of its own, so
// the notice doesn't say online.
#[test]
fn back_through_another_device_says_so() {
    let e = Env::new();
    shared_card(&e);
    let t0 = e.clk.now();
    let off = agents(OFFLINE, t0);
    assert!(e.c.sweep_presence_at(&off, &t0, &here).is_empty());
    let t1 = plus(t0, mins(5));
    assert_eq!(e.c.sweep_presence_at(&off, &t1, &here).len(), 1);
    let mut back = agents(coord::roster::RECENT, t1);
    for a in &mut back {
        a.device_via = "Phone".into();
    }
    let ns = e.c.sweep_presence_at(&back, &t1, &here);
    assert_eq!(ns.len(), 1, "{ns:?}");
    assert!(
        ns[0].text.contains("reachable again, through Phone"),
        "{}",
        ns[0].text
    );
    assert!(!ns[0].text.contains("online"), "{}", ns[0].text);

    // It goes quiet and comes back, as a relayed idle device does: nobody
    // is told or woken again.
    for i in 1..4u64 {
        let t = plus(t1, mins(15 * i));
        let quiet = agents(OFFLINE, t);
        assert!(e.c.sweep_presence_at(&quiet, &t, &here).is_empty());
        let t = plus(t, mins(5));
        assert!(e.c.sweep_presence_at(&quiet, &t, &here).is_empty());
        let mut again = agents(coord::roster::RECENT, t);
        for a in &mut again {
            a.device_via = "Phone".into();
        }
        assert!(e.c.sweep_presence_at(&again, &t, &here).is_empty());
    }

    // A link of its own at last: said once.
    let t2 = plus(t1, mins(90));
    let ns = e.c.sweep_presence_at(&agents(CONNECTED, t2), &t2, &here);
    assert_eq!(ns.len(), 1, "{ns:?}");
    assert!(ns[0].text.contains("back online"), "{}", ns[0].text);
}
