//! Card #453 (spawned agents §5): an agent the person started in a terminal
//! is not a free agent. Irori hands it unheld cards, asks and threads only
//! when the person turned on "Offer it cards" for it; otherwise it gets
//! work only when given (assign, a mention, its own board_join). Card #464:
//! a failed spawn isn't asked for again on every sweep.

use crate::common;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use common::*;
use coord::roster::AgentCard;
use coord::*;
use gocompat::time::GoTime;

fn idle(key: &str, since_min: i64) -> AgentCard {
    let at = GoTime::unix(fixed_now().secs - 60 * (60 - since_min), 0);
    AgentCard {
        key: key.into(),
        device_id: "dev-a".into(),
        status: "idle".into(),
        updated_at: at,
        last_activity: at,
        ..AgentCard::default()
    }
}

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        objective: "why".into(),
        ..NewTask::default()
    }
}

/// A coord whose "Offer it cards" is on for the keys in `offered`.
fn with_offers(e: &Env, offered: &Arc<RwLock<HashSet<String>>>) -> Coord {
    let o = offered.clone();
    e.reopen_with(Options {
        offers: Some(Arc::new(move |k: &str| o.read().unwrap().contains(k))),
        ..e.opts()
    })
}

#[test]
fn a_terminal_agent_is_offered_no_card_until_the_person_offers_it_cards() {
    let mut e = Env::new();
    let offered = Arc::new(RwLock::new(HashSet::new()));
    e.c = with_offers(&e, &offered);
    let agents = vec![idle(BOB, 20), idle(CAROL, 10)];
    let t = e.c.create_task(USER, nt("Port the parser")).unwrap();
    let n = e.c.offer_card(USER, &t.id, &agents).unwrap();
    assert!(n.to.is_empty(), "nobody takes offered cards: {n:?}");
    assert!(e.c.sweep_asks(&agents).unwrap().is_empty());
    let r = e.c.idle_report(&agents);
    assert_eq!(r.idle, 0, "idle terminal agents aren't free");
    // He turns it on for Carol: the next sweep hands it to her alone.
    offered.write().unwrap().insert(CAROL.to_string());
    let out = e.c.sweep_asks(&agents).unwrap();
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(out[0].to, vec![CAROL.to_string()]);
    // Bob can still take a card himself.
    let t2 = e.c.create_task(USER, nt("Docs")).unwrap();
    e.c.claim_task(BOB, &t2.id).unwrap();
}

#[test]
fn an_ask_naming_nobody_goes_only_to_an_agent_offered_cards() {
    let mut e = Env::new();
    let offered = Arc::new(RwLock::new(HashSet::new()));
    e.c = with_offers(&e, &offered);
    let agents = vec![idle(BOB, 20), idle(CAROL, 10)];
    let (_, n) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "Why is the build slow?".into(),
                ..NewAsk::default()
            },
            &agents,
        )
        .unwrap();
    assert!(n.to.is_empty(), "{n:?}");
    offered.write().unwrap().insert(BOB.to_string());
    let out = e.c.sweep_asks(&agents).unwrap();
    assert!(out.iter().any(|n| n.to == vec![BOB.to_string()]), "{out:?}");
}

#[test]
fn a_spawning_board_never_hands_a_new_card_to_an_idle_agent() {
    // Irori starts an agent for it; handing it out too put two on it.
    let mut e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    let f = on.clone();
    e.c = e.reopen_with(Options {
        spawning: Some(Arc::new(move || f.load(Ordering::SeqCst))),
        ..e.opts()
    });
    let t = e.c.create_task(USER, nt("Spawned")).unwrap();
    let n = e.c.offer_card(USER, &t.id, &[idle(BOB, 20)]).unwrap();
    assert!(n.to.is_empty(), "{n:?}");
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&t.id));
}

fn spawn_ask(id: &str) -> SpawnAsk {
    SpawnAsk {
        task_id: id.into(),
        device: "dev-a".into(),
        folder: "/repo".into(),
        permission_mode: "acceptEdits".into(),
        name: "pawpaw".into(),
        ..SpawnAsk::default()
    }
}

#[test]
fn a_failed_spawn_waits_before_it_is_asked_for_again() {
    let mut e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    let f = on.clone();
    e.c = e.reopen_with(Options {
        spawning: Some(Arc::new(move || f.load(Ordering::SeqCst))),
        ..e.opts()
    });
    let t = e.c.create_task(USER, nt("Untrusted repo")).unwrap();
    let g =
        e.c.request_spawn("irori", spawn_ask(&t.id))
            .unwrap()
            .unwrap();
    assert!(
        e.c.end_spawn("irori", &t.id, g, SPAWN_FAILED, "not trusted")
            .unwrap()
    );
    assert!(
        e.c.cards_to_spawn().is_empty(),
        "not again on the next sweep"
    );
    e.clk.add(secs(60));
    assert!(e.c.cards_to_spawn().is_empty());
    e.clk.add(SPAWN_RETRY);
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&t.id));
}

/// Card #699: a not-trusted failure is looked at again within a minute,
/// not ten; the Core writes nothing while the folder is still untrusted.
#[test]
fn a_not_trusted_spawn_is_looked_at_again_soon() {
    let mut e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    let f = on.clone();
    e.c = e.reopen_with(Options {
        spawning: Some(Arc::new(move || f.load(Ordering::SeqCst))),
        ..e.opts()
    });
    let t = e.c.create_task(USER, nt("Untrusted repo")).unwrap();
    let g =
        e.c.request_spawn("irori", spawn_ask(&t.id))
            .unwrap()
            .unwrap();
    let reason = format!("{NOT_TRUSTED_PREFIX}/repo on dev-a");
    assert!(
        e.c.end_spawn("irori", &t.id, g, SPAWN_FAILED, &reason)
            .unwrap()
    );
    assert!(e.c.cards_to_spawn().is_empty(), "not on the next sweep");
    e.clk.add(secs(30));
    assert!(e.c.cards_to_spawn().is_empty());
    e.clk.add(SPAWN_RETRY_UNTRUSTED);
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&t.id));
    assert!(SPAWN_RETRY_UNTRUSTED < SPAWN_RETRY);
}

#[test]
fn a_failed_spawn_is_asked_for_again_once_the_card_changes() {
    let mut e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    let f = on.clone();
    e.c = e.reopen_with(Options {
        spawning: Some(Arc::new(move || f.load(Ordering::SeqCst))),
        ..e.opts()
    });
    let t = e.c.create_task(USER, nt("Untrusted repo")).unwrap();
    let g =
        e.c.request_spawn("irori", spawn_ask(&t.id))
            .unwrap()
            .unwrap();
    e.c.end_spawn("irori", &t.id, g, SPAWN_FAILED, "not trusted")
        .unwrap();
    assert!(e.c.cards_to_spawn().is_empty());
    e.clk.add(secs(5));
    e.c.edit_task(
        USER,
        &t.id,
        TaskEdit {
            objective: Some("Trusted now".into()),
            ..TaskEdit::default()
        },
    )
    .unwrap();
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&t.id));
}

#[test]
fn the_persons_start_skips_a_failed_spawns_wait() {
    let mut e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    let f = on.clone();
    e.c = e.reopen_with(Options {
        spawning: Some(Arc::new(move || f.load(Ordering::SeqCst))),
        ..e.opts()
    });
    let t = e.c.create_task(USER, nt("Trust and start")).unwrap();
    let g =
        e.c.request_spawn("irori", spawn_ask(&t.id))
            .unwrap()
            .unwrap();
    e.c.end_spawn("irori", &t.id, g, SPAWN_FAILED, "not trusted")
        .unwrap();
    assert!(e.c.cards_to_spawn().is_empty());
    e.c.spawn_now(&t.id);
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&t.id));
    // Once asked for again, a second failure waits as before.
    let g =
        e.c.request_spawn("irori", spawn_ask(&t.id))
            .unwrap()
            .unwrap();
    e.c.end_spawn("irori", &t.id, g, SPAWN_FAILED, "not trusted")
        .unwrap();
    assert!(e.c.cards_to_spawn().is_empty());
}
