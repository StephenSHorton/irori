//! Card #470: an open ask nobody takes starts an agent like a card does.
//! Since #453 an ask goes only to an idle terminal agent with "Offer it
//! cards" on; with none free it is listed to spawn (same switch, backoff
//! and crash limit as a card), nobody else is handed it while its agent
//! starts, and the bound agent takes the ask through its take lock. An
//! opted-in idle agent still takes it first.

mod common;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use common::*;
use coord::roster::AgentCard;
use coord::*;
use gocompat::time::GoTime;

const SPAWNED: &str = "dev-a/spawned-session";
const SWEEPER_KEY: &str = "irori";

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

struct Rig {
    on: Arc<AtomicBool>,
    offered: Arc<RwLock<HashSet<String>>>,
}

/// Spawning read from `on`, "Offer it cards" from `offered`.
fn rig(e: &mut Env, offered: &[&str]) -> Rig {
    let r = Rig {
        on: Arc::new(AtomicBool::new(true)),
        offered: Arc::new(RwLock::new(offered.iter().map(|s| s.to_string()).collect())),
    };
    let (f, o) = (r.on.clone(), r.offered.clone());
    e.c = e.reopen_with(Options {
        spawning: Some(Arc::new(move || f.load(Ordering::SeqCst))),
        offers: Some(Arc::new(move |k: &str| o.read().unwrap().contains(k))),
        ..e.opts()
    });
    r
}

fn sask(id: &str) -> SpawnAsk {
    SpawnAsk {
        task_id: id.into(),
        device: "dev-a".into(),
        folder: "/repo".into(),
        permission_mode: "bypassPermissions".into(),
        name: "thread-1".into(),
        ..SpawnAsk::default()
    }
}

fn post(e: &Env, text: &str, thread: bool, agents: &[AgentCard]) -> Task {
    e.c.post_ask(
        USER,
        NewAsk {
            text: text.into(),
            thread,
            ..NewAsk::default()
        },
        agents,
    )
    .unwrap()
    .0
}

fn ask_of(e: &Env, id: &str) -> Ask {
    e.c.get_task(id).unwrap().ask.unwrap()
}

#[test]
fn a_thread_nobody_takes_starts_an_agent_that_takes_it_and_answers() {
    let mut e = Env::new();
    let r = rig(&mut e, &[]);
    // Terminal agents idle, none with "Offer it cards" on.
    let agents = vec![idle(BOB, 20), idle(CAROL, 10)];
    let tk = post(&e, "What did we ship today?", true, &agents);
    let a = ask_of(&e, &tk.id);
    assert!(a.taker.is_empty() && a.unclaimed, "{a:?}");
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&tk.id));

    let g =
        e.c.request_spawn(SWEEPER_KEY, sask(&tk.id))
            .unwrap()
            .unwrap();
    assert!(e.c.get_task(&tk.id).unwrap().spawn_pending());
    assert_eq!(e.c.request_spawn(SWEEPER_KEY, sask(&tk.id)).unwrap(), None);
    assert!(e.c.cards_to_spawn().is_empty(), "pending: not listed again");
    // Bob turns "Offer it cards" on while it starts: the spawn has it.
    r.offered.write().unwrap().insert(BOB.into());
    let ns = e.c.sweep_asks(&agents).unwrap();
    assert!(
        ns.iter().all(|n| !n.to.contains(&BOB.to_string())),
        "{ns:?}"
    );
    assert!(ask_of(&e, &tk.id).taker.is_empty());

    // Bound: it takes the ask through the take lock, at once.
    assert!(e.c.bind_spawn(&tk.id, g, SPAWNED).unwrap());
    let t = e.c.get_task(&tk.id).unwrap();
    let a = t.ask.clone().unwrap();
    assert!(
        a.taker == SPAWNED && !a.handed && a.reason == TAKE_TAKEN,
        "{a:?}"
    );
    assert_eq!(t.holders, [SPAWNED]);
    assert!(t.assigned.is_empty(), "{t:?}");
    assert_eq!(e.c.spawned_card(SPAWNED), Some(tk.id.clone()));

    // Its first turn ends before it replies: it keeps it.
    let mut all = agents.clone();
    let mut sp = idle(SPAWNED, 59);
    sp.updated_at = GoTime::unix(fixed_now().secs + 30, 0);
    sp.last_activity = sp.updated_at;
    all.push(sp);
    e.clk.add(secs(60));
    e.c.sweep_asks(&all).unwrap();
    assert_eq!(ask_of(&e, &tk.id).taker, SPAWNED);
    assert!(e.c.cards_to_spawn().is_empty());

    // It answers: answered, nobody holds it, nothing more to start.
    e.c.add_comment(SPAWNED, &tk.id, "We shipped #12.", false)
        .unwrap();
    e.c.sweep_asks(&all).unwrap();
    let a = ask_of(&e, &tk.id);
    assert!(a.reason == TAKE_ANSWERED && a.taker.is_empty(), "{a:?}");
    assert!(e.c.cards_to_spawn().is_empty(), "answered: no second agent");
}

#[test]
fn an_opted_in_idle_agent_still_takes_it_first() {
    let mut e = Env::new();
    let _r = rig(&mut e, &[CAROL, BOB]);
    let agents = vec![idle(BOB, 20), idle(CAROL, 10)];
    let tk = post(&e, "Is CI green?", true, &agents);
    let a = ask_of(&e, &tk.id);
    assert_eq!(a.taker, CAROL, "{a:?}");
    assert!(e.c.cards_to_spawn().is_empty(), "taken: no spawn");
    // A plain ask too: Carol has one, so Bob.
    let tk = post(&e, "Tidy the README", false, &agents);
    assert_eq!(ask_of(&e, &tk.id).taker, BOB);
    assert!(e.c.cards_to_spawn().is_empty());
}

#[test]
fn an_agent_offered_cards_after_it_was_listed_still_takes_it_first() {
    let mut e = Env::new();
    let r = rig(&mut e, &[]);
    let agents = vec![idle(BOB, 20)];
    let tk = post(&e, "Is CI green?", true, &agents);
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&tk.id));
    // Bob turns "Offer it cards" on before the spawn pass asks for one.
    r.offered.write().unwrap().insert(BOB.into());
    e.c.sweep_asks(&agents).unwrap();
    assert_eq!(ask_of(&e, &tk.id).taker, BOB);
    assert!(e.c.cards_to_spawn().is_empty());
    assert_eq!(e.c.request_spawn(SWEEPER_KEY, sask(&tk.id)).unwrap(), None);
}

#[test]
fn a_plain_ask_spawned_for_is_closed_by_its_agent() {
    let mut e = Env::new();
    let _r = rig(&mut e, &[]);
    let tk = post(&e, "Should we drop Windows 10?", false, &[idle(BOB, 20)]);
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&tk.id));
    let g =
        e.c.request_spawn(SWEEPER_KEY, sask(&tk.id))
            .unwrap()
            .unwrap();
    assert!(e.c.bind_spawn(&tk.id, g, SPAWNED).unwrap());
    assert_eq!(ask_of(&e, &tk.id).taker, SPAWNED);
    let t =
        e.c.ask_close(
            SPAWNED,
            &tk.id,
            AskOutcome {
                kind: OUTCOME_ANSWERED.into(),
                note: "keep it a year".into(),
                ..AskOutcome::default()
            },
        )
        .unwrap();
    assert!(is_terminal(&t.state), "{t:?}");
    assert!(e.c.cards_to_spawn().is_empty());
}

#[test]
fn the_switch_backoff_and_a_named_ask_keep_it_from_spawning() {
    let mut e = Env::new();
    let r = rig(&mut e, &[]);
    let agents = vec![idle(BOB, 20)];
    let tk = post(&e, "What's left on the parser?", true, &agents);
    r.on.store(false, Ordering::SeqCst);
    assert!(e.c.cards_to_spawn().is_empty(), "switch off");
    r.on.store(true, Ordering::SeqCst);
    let g =
        e.c.request_spawn(SWEEPER_KEY, sask(&tk.id))
            .unwrap()
            .unwrap();
    e.c.end_spawn(SWEEPER_KEY, &tk.id, g, SPAWN_FAILED, "not trusted")
        .unwrap();
    assert!(e.c.cards_to_spawn().is_empty(), "failed: waits");
    e.clk.add(SPAWN_RETRY);
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&tk.id));

    // A plain ask naming an agent goes to it alone: never spawned for.
    let named =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "@bob check the build".into(),
                mentions: vec![Mention {
                    key: BOB.into(),
                    ..Mention::default()
                }],
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap()
        .0;
    assert!(!ids(&e.c.cards_to_spawn()).contains(&named.id));

    // A thread with "Agents answer now" off is never handed out.
    let quiet =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "a note to self".into(),
                thread: true,
                quiet: true,
                ..NewAsk::default()
            },
            &agents,
        )
        .unwrap()
        .0;
    e.clk.add(secs(60));
    e.c.sweep_asks(&agents).unwrap();
    assert!(!ids(&e.c.cards_to_spawn()).contains(&quiet.id));
}

#[test]
fn a_crashing_agent_stops_its_asks_spawns_after_two() {
    let mut e = Env::new();
    let _r = rig(&mut e, &[]);
    let tk = post(&e, "Why is the build slow?", true, &[]);
    for n in 1..=2u32 {
        assert_eq!(
            ids(&e.c.cards_to_spawn()),
            std::slice::from_ref(&tk.id),
            "round {n}"
        );
        let g =
            e.c.request_spawn(SWEEPER_KEY, sask(&tk.id))
                .unwrap()
                .unwrap();
        let key = format!("dev-a/spawned-{n}");
        e.ros.set(&key, "busy");
        assert!(e.c.bind_spawn(&tk.id, g, &key).unwrap());
        e.c.sweep_asks(&[]).unwrap();
        // It goes away holding it.
        e.ros.set(&key, "");
        e.c.sweep_asks(&[]).unwrap();
        e.clk.add(mins(10));
        e.c.sweep_asks(&[]).unwrap();
        let t = e.c.get_task(&tk.id).unwrap();
        assert!(t.ask.as_ref().unwrap().taker.is_empty(), "{t:?}");
        assert_eq!(t.spawn_crashes, n);
        e.c.sweep_asks(&[]).unwrap();
    }
    assert!(e.c.cards_to_spawn().is_empty(), "crashed twice: stops");
}

#[test]
fn an_agent_irori_ended_is_no_crash_and_the_ask_waits_for_the_person() {
    let mut e = Env::new();
    let _r = rig(&mut e, &[]);
    let tk = post(&e, "Should we drop Windows 10?", false, &[]);
    let g =
        e.c.request_spawn(SWEEPER_KEY, sask(&tk.id))
            .unwrap()
            .unwrap();
    let key = "dev-a/spawned-1";
    e.ros.set(key, "idle");
    assert!(e.c.bind_spawn(&tk.id, g, key).unwrap());
    e.c.sweep_asks(&[]).unwrap();
    // Irori ends it (it went quiet), then it is gone from presence.
    e.c.mark_ended(key, fixed_now());
    e.ros.set(key, "");
    e.c.sweep_asks(&[]).unwrap();
    e.clk.add(mins(10));
    e.c.sweep_asks(&[]).unwrap();
    let t = e.c.get_task(&tk.id).unwrap();
    assert!(t.ask.as_ref().unwrap().taker.is_empty(), "{t:?}");
    assert_eq!(t.spawn_crashes, 0, "ended by Irori, not crashed");
    assert!(e.c.cards_to_spawn().is_empty(), "waits for the person");
    // The person writes on it: a fresh agent.
    e.c.add_comment(USER, &tk.id, "Any thoughts?", false)
        .unwrap();
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&tk.id));
}
