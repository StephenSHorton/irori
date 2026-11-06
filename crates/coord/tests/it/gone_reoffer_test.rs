//! Card #338: a card whose holder has gone goes back up and out to the
//! next free agent, as if the holder had board_left it. Today's cases
//! (2026-11-03): holders wound down and their working cards stayed held,
//! or sat unheld and were never offered again, until a lead reassigned
//! them by hand.

use crate::common;

use common::*;
use coord::roster::AgentCard;
use coord::*;
use gocompat::time::GoTime;

const DAVE: &str = "dev-a/dave-session";

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        objective: "why it matters".into(),
        ..NewTask::default()
    }
}

fn idle_at(key: &str, at: GoTime) -> AgentCard {
    AgentCard {
        key: key.into(),
        device_id: "dev-a".into(),
        status: "idle".into(),
        updated_at: at,
        last_activity: at,
        ..AgentCard::default()
    }
}

/// The roster the hand-out sees: ALICE (the poster) and DAVE idle, plus
/// `more`.
fn free(e: &Env, more: &[&str]) -> Vec<AgentCard> {
    let t = e.clk.now();
    let mut v = vec![idle_at(ALICE, t), idle_at(DAVE, t)];
    for k in more {
        v.push(idle_at(k, t));
    }
    v
}

/// Ends BOB's session: the roster says gone, as on a wind-down.
fn end_bob(e: &Env, agents: &mut Vec<AgentCard>) {
    e.ros.set_at(BOB, "gone", e.clk.now());
    agents.retain(|a| a.key != BOB);
}

/// One Core tick: the coord sweep, then the hand-out.
fn tick(e: &Env, agents: &[AgentCard]) -> Vec<Notice> {
    let mut ns = e.c.sweep_notices().unwrap();
    ns.extend(e.c.sweep_asks(agents).unwrap());
    ns
}

fn offers_to(ns: &[Notice], key: &str) -> Vec<Notice> {
    ns.iter()
        .filter(|n| n.kind == NOTICE_OFFER && n.to == [key])
        .cloned()
        .collect()
}

/// BOB took a card Irori handed him, and works it with a branch and a
/// left_off; then his session ends.
fn bob_works_a_handed_card(e: &Env) -> Task {
    let t = e.c.create_task(ALICE, nt("Port the parser")).unwrap();
    let n =
        e.c.offer_card(ALICE, &t.id, &[idle_at(BOB, e.clk.now())])
            .unwrap();
    assert_eq!(n.to, [BOB], "{n:?}");
    e.c.claim_task(BOB, &t.id).unwrap();
    e.c.set_steps(
        BOB,
        &t.id,
        vec![Step {
            text: "lexer".into(),
            ..Step::default()
        }],
    )
    .unwrap();
    e.c.get_task(&t.id).unwrap()
}

#[test]
fn an_ended_holder_s_working_card_is_offered_within_a_minute() {
    let e = Env::new();
    let t = bob_works_a_handed_card(&e);
    let mut agents = free(&e, &[BOB]);
    assert!(offers_to(&tick(&e, &agents), DAVE).is_empty());

    e.clk.add(secs(5));
    end_bob(&e, &mut agents);
    let mut got = Vec::new();
    for _ in 0..12 {
        e.clk.add(secs(5));
        got.extend(offers_to(&tick(&e, &agents), DAVE));
        if !got.is_empty() {
            break;
        }
    }
    assert_eq!(
        got.len(),
        1,
        "no offer to DAVE within a minute of BOB ending"
    );
    let after = e.c.get_task(&t.id).unwrap();
    assert!(after.holders.is_empty(), "{:?}", after.holders);
    assert_eq!(after.state, SUBMITTED);
    assert!(got[0].text.contains("went away"), "{}", got[0].text);

    // Handed once: later sweeps leave it with DAVE, no fresh notices.
    for _ in 0..4 {
        e.clk.add(secs(5));
        let ns = tick(&e, &agents);
        assert!(
            ns.iter().all(|n| n.kind != NOTICE_OFFER),
            "offered again: {ns:?}"
        );
    }
    // DAVE's turn ends without taking it: it moves on to CAROL, never
    // back to DAVE or to BOB.
    e.clk.add(secs(5));
    let later = e.clk.now();
    agents.retain(|a| a.key != DAVE);
    agents.push(idle_at(DAVE, later));
    agents.push(idle_at(CAROL, e.clk.now()));
    let ns = tick(&e, &agents);
    let to: Vec<_> = ns
        .iter()
        .filter(|n| n.kind == NOTICE_OFFER)
        .flat_map(|n| n.to.clone())
        .collect();
    assert_eq!(to, [CAROL], "{ns:?}");
}

#[test]
fn an_assigned_card_whose_old_hand_out_went_elsewhere_is_offered() {
    // The card was handed to CAROL first, then a lead assigned it to BOB
    // by hand. CAROL is still around and busy, so her old hand-out must
    // not hold it once BOB has gone.
    let e = Env::new();
    let t = e.c.create_task(ALICE, nt("Port the parser")).unwrap();
    e.c.offer_card(ALICE, &t.id, &[idle_at(CAROL, e.clk.now())])
        .unwrap();
    e.c.assign_task(ALICE, &t.id, BOB).unwrap();
    e.c.claim_task(BOB, &t.id).unwrap();
    let busy = AgentCard {
        status: "busy".into(),
        ..idle_at(CAROL, e.clk.now())
    };
    let mut agents = free(&e, &[BOB]);
    agents.push(busy);
    tick(&e, &agents);

    e.clk.add(secs(5));
    end_bob(&e, &mut agents);
    let mut got = Vec::new();
    for _ in 0..12 {
        e.clk.add(secs(5));
        got.extend(offers_to(&tick(&e, &agents), DAVE));
        if !got.is_empty() {
            break;
        }
    }
    assert_eq!(got.len(), 1, "never offered after BOB went");
    assert_eq!(e.c.get_task(&t.id).unwrap().state, SUBMITTED);
}

#[test]
fn a_working_card_its_holder_leaves_is_offered() {
    let e = Env::new();
    let t = bob_works_a_handed_card(&e);
    let agents = free(&e, &[]);
    e.c.leave(BOB, &t.id, BOB).unwrap();
    e.clk.add(secs(5));
    let got = offers_to(&tick(&e, &agents), DAVE);
    assert_eq!(got.len(), 1, "a left card wasn't offered");
    assert_eq!(e.c.get_task(&t.id).unwrap().state, SUBMITTED);
}

#[test]
fn a_holder_merely_missing_keeps_its_card_for_the_grace() {
    // No session end, just absent from the roster (a restart): the card
    // stays with BOB for the holder grace, as before.
    let e = Env::new();
    let t = bob_works_a_handed_card(&e);
    let mut agents = free(&e, &[BOB]);
    tick(&e, &agents);
    e.ros.set(BOB, "");
    agents.retain(|a| a.key != BOB);
    for _ in 0..12 {
        e.clk.add(secs(5));
        assert!(offers_to(&tick(&e, &agents), DAVE).is_empty());
    }
    assert_eq!(e.c.get_task(&t.id).unwrap().holders, [BOB]);
}
