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
    // The person asks again: the count starts over, and an agent starts.
    e.c.ask_again(USER, &tk.id).unwrap();
    e.c.sweep_asks(&[]).unwrap();
    assert_eq!(e.c.get_task(&tk.id).unwrap().spawn_crashes, 0);
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&tk.id));
}

#[test]
fn asking_again_wakes_an_ask_that_rests() {
    let mut e = Env::new();
    let _r = rig(&mut e, &[]);
    let tk = post(&e, "What's left on the parser?", true, &[]);
    let g =
        e.c.request_spawn(SWEEPER_KEY, sask(&tk.id))
            .unwrap()
            .unwrap();
    let key = "dev-a/spawned-1";
    e.ros.set(key, "idle");
    assert!(e.c.bind_spawn(&tk.id, g, key).unwrap());
    e.c.sweep_asks(&[]).unwrap();
    e.c.mark_ended(key, fixed_now());
    e.ros.set(key, "");
    e.c.sweep_asks(&[]).unwrap();
    e.clk.add(mins(10));
    e.c.sweep_asks(&[]).unwrap();
    assert!(e.c.cards_to_spawn().is_empty(), "rests");
    e.c.ask_again(USER, &tk.id).unwrap();
    e.c.sweep_asks(&[]).unwrap();
    e.c.sweep_asks(&[]).unwrap();
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&tk.id));
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

// Card #454 (spawned-agents.md §4, §14 part 10).

fn spawn_bound(e: &Env, id: &str) {
    let g = e.c.request_spawn(SWEEPER_KEY, sask(id)).unwrap().unwrap();
    assert!(e.c.bind_spawn(id, g, SPAWNED).unwrap());
}

#[test]
fn a_thread_keeps_the_channel_it_was_started_from() {
    let mut e = Env::new();
    let _r = rig(&mut e, &[]);
    let ask = |text: &str, thread: bool, channel: &str| {
        e.c.post_ask(
            USER,
            NewAsk {
                text: text.into(),
                thread,
                channel: channel.into(),
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap()
        .0
    };
    let t = ask("Why is main red?", true, " ch_01ABC ");
    assert_eq!(ask_of(&e, &t.id).channel, "ch_01ABC");
    // Not a channel id, or not a thread: nothing kept.
    let t = ask("Why is main red?", true, "#irori");
    assert_eq!(ask_of(&e, &t.id).channel, "");
    let t = ask("Tidy the README", false, "ch_01ABC");
    assert_eq!(ask_of(&e, &t.id).channel, "");
    // It survives a reload (the fold reads it from the row).
    let t = ask("Is CI green?", true, "ch_02DEF");
    e.c = e.reopen_with(e.opts());
    assert_eq!(ask_of(&e, &t.id).channel, "ch_02DEF");
}

#[test]
fn an_ask_its_agent_makes_a_card_keeps_that_agent() {
    let mut e = Env::new();
    let _r = rig(&mut e, &[]);
    let tk = post(&e, "The export button is broken", false, &[idle(BOB, 20)]);
    spawn_bound(&e, &tk.id);
    let t =
        e.c.ask_close(
            SPAWNED,
            &tk.id,
            AskOutcome {
                kind: OUTCOME_CARD.into(),
                title: "Fix the export button".into(),
                note: "a card".into(),
                ..AskOutcome::default()
            },
        )
        .unwrap();
    // The ask is the card, under the same id: the agent carries on as its
    // agent, and nothing else is started for it.
    assert_eq!(t.state, WORKING, "{t:?}");
    assert_eq!(t.holders, [SPAWNED]);
    assert_eq!(e.c.spawned_card(SPAWNED), Some(tk.id.clone()));
    assert!(e.c.cards_to_spawn().is_empty());
}

#[test]
fn a_thread_the_person_makes_a_card_after_its_answer_gets_its_own_agent() {
    let mut e = Env::new();
    let r = rig(&mut e, &[]);
    let tk = post(&e, "Can the board sort by age?", true, &[idle(BOB, 20)]);
    spawn_bound(&e, &tk.id);
    e.c.add_comment(SPAWNED, &tk.id, "Not yet; it's a small change.", false)
        .unwrap();
    e.c.sweep_asks(&[idle(BOB, 20)]).unwrap();
    let a = ask_of(&e, &tk.id);
    assert!(a.taker.is_empty() && a.reason == TAKE_ANSWERED, "{a:?}");
    let t =
        e.c.ask_close(
            USER,
            &tk.id,
            AskOutcome {
                kind: OUTCOME_CARD.into(),
                title: "Sort the board by age".into(),
                ..AskOutcome::default()
            },
        )
        .unwrap();
    // Nobody holds it: it goes out as a new card, not stuck in working.
    assert_eq!(t.state, SUBMITTED, "{t:?}");
    assert!(t.holders.is_empty(), "{t:?}");
    assert!(t.open_ask().is_none());
    // The spawn pass sees it once its last agent's request is settled, as
    // any card needing an agent: switch on, it is listed.
    r.on.store(true, Ordering::SeqCst);
    let listed = ids(&e.c.cards_to_spawn());
    assert!(
        listed.contains(&tk.id) || e.c.get_task(&tk.id).unwrap().spawn_pending(),
        "{listed:?}"
    );
}

#[test]
fn an_ask_the_person_makes_a_card_while_its_agent_holds_it_stays_with_it() {
    let mut e = Env::new();
    let _r = rig(&mut e, &[]);
    let tk = post(&e, "Rename the app", false, &[idle(BOB, 20)]);
    spawn_bound(&e, &tk.id);
    let t =
        e.c.ask_close(
            USER,
            &tk.id,
            AskOutcome {
                kind: OUTCOME_CARD.into(),
                title: "Rename the app".into(),
                ..AskOutcome::default()
            },
        )
        .unwrap();
    assert_eq!(t.state, WORKING, "{t:?}");
    assert_eq!(t.holders, [SPAWNED]);
    assert!(e.c.cards_to_spawn().is_empty());
}

#[test]
fn a_private_thread_about_a_shared_board_spawns_too() {
    let mut e = Env::new();
    let _r = rig(&mut e, &[]);
    let (tk, _) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "What's left on the launch board?".into(),
                thread: true,
                board: "brd_launch".into(),
                ..NewAsk::default()
            },
            &[idle(BOB, 20)],
        )
        .unwrap();
    let a = ask_of(&e, &tk.id);
    assert_eq!(a.board, "brd_launch");
    assert!(a.unclaimed, "{a:?}");
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&tk.id));
}

#[test]
fn a_thread_made_a_card_before_its_agent_answers_goes_out_as_a_new_card() {
    let mut e = Env::new();
    let _r = rig(&mut e, &[]);
    let tk = post(&e, "Can we cache the board?", true, &[idle(BOB, 20)]);
    spawn_bound(&e, &tk.id);
    assert_eq!(ask_of(&e, &tk.id).taker, SPAWNED);
    let t =
        e.c.ask_close(
            USER,
            &tk.id,
            AskOutcome {
                kind: OUTCOME_CARD.into(),
                title: "Cache the board".into(),
                ..AskOutcome::default()
            },
        )
        .unwrap();
    // The thread's agent lets go: the card is a new card, unheld, and its
    // agent (a card's) is started afresh; the thread's goes.
    assert_eq!(t.state, SUBMITTED, "{t:?}");
    assert!(t.holders.is_empty(), "{t:?}");
}

// Card #512: a thread gets its own agent at once. The agents it names are
// woken, and nobody waits on them: no quiet spell before the hand-out.

/// A card alice holds and is busy on, and a thread naming it.
fn thread_naming_alices_card(e: &Env, agents: &[AgentCard]) -> Task {
    let card =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "lobby join".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(ALICE, &card.id).unwrap();
    post(
        e,
        &format!("Is #{} still moving?", card.short),
        true,
        agents,
    )
}

#[test]
fn a_thread_naming_a_busy_agent_spawns_at_once_and_the_agent_can_still_answer() {
    let mut e = Env::new();
    let _r = rig(&mut e, &[]);
    let agents = vec![idle(BOB, 20)];
    let tk = thread_naming_alices_card(&e, &agents);
    let a = ask_of(&e, &tk.id);
    assert!(a.heard == [ALICE] && !a.unclaimed, "alice is woken: {a:?}");
    // The hand-out runs at the next sweep, with no clock advance: nobody
    // with "Offer it cards" on is free, so it is unclaimed and listed.
    assert!(e.c.sweep_asks(&agents).unwrap().is_empty());
    let a = ask_of(&e, &tk.id);
    assert!(a.unclaimed && a.heard == [ALICE], "{a:?}");
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&tk.id));
    spawn_bound(&e, &tk.id);
    assert_eq!(ask_of(&e, &tk.id).taker, SPAWNED);

    // Its agent answers; alice, done with her work, chimes in later.
    e.c.add_comment(SPAWNED, &tk.id, "It merged an hour ago.", false)
        .unwrap();
    e.c.sweep_asks(&agents).unwrap();
    e.clk.add(secs(5));
    e.c.add_comment(ALICE, &tk.id, "And the docs are in too.", false)
        .unwrap();
    e.c.sweep_asks(&agents).unwrap();
    let a = ask_of(&e, &tk.id);
    assert!(a.reason == TAKE_ANSWERED && a.taker.is_empty(), "{a:?}");
    assert!(e.c.cards_to_spawn().is_empty(), "answered: no second agent");
    assert_eq!(e.c.thread_to(&tk.id), [SPAWNED, ALICE]);
}

#[test]
fn a_thread_naming_an_agent_still_goes_to_an_opted_in_idle_agent_first() {
    let mut e = Env::new();
    let _r = rig(&mut e, &[BOB]);
    let agents = vec![idle(BOB, 20)];
    let tk = thread_naming_alices_card(&e, &agents);
    let ns = e.c.sweep_asks(&agents).unwrap();
    assert!(
        ns.len() == 1 && ns[0].to == [BOB] && ns[0].text.contains("handed it to you"),
        "{ns:?}"
    );
    assert_eq!(ask_of(&e, &tk.id).taker, BOB);
    assert!(e.c.cards_to_spawn().is_empty(), "taken: no spawn");
}

/// The named agent answers while the thread's own agent starts: the
/// thread stays answered when that agent comes up, so it isn't reopened
/// for a second answer.
#[test]
fn a_thread_answered_while_its_agent_starts_stays_answered() {
    let mut e = Env::new();
    let _r = rig(&mut e, &[]);
    let agents = vec![idle(BOB, 20)];
    let tk = thread_naming_alices_card(&e, &agents);
    e.c.sweep_asks(&agents).unwrap();
    let g =
        e.c.request_spawn(SWEEPER_KEY, sask(&tk.id))
            .unwrap()
            .unwrap();
    e.c.add_comment(ALICE, &tk.id, "Yes, still moving.", false)
        .unwrap();
    e.c.sweep_asks(&agents).unwrap();
    assert_eq!(ask_of(&e, &tk.id).reason, TAKE_ANSWERED);
    assert!(e.c.bind_spawn(&tk.id, g, SPAWNED).unwrap());
    let a = ask_of(&e, &tk.id);
    assert!(
        a.reason == TAKE_ANSWERED && a.taker.is_empty(),
        "still answered: {a:?}"
    );
    assert!(e.c.cards_to_spawn().is_empty());
}

/// Card #541's words: what a person is told waits for an agent is only what
/// would really start one. An answered thread needs nobody until they
/// reply, and a stale request is no agent starting.
#[test]
fn waits_for_agent_is_only_work_that_would_start() {
    let mut e = Env::on("dev-a");
    let _r = rig(&mut e, &[]);
    let (t, _) =
        e.c.post_ask(
            "user:dev-a",
            NewAsk {
                text: "Why?".into(),
                thread: true,
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap();
    assert!(e.c.waits_for_agent(&e.c.get_task(&t.id).unwrap()));
    e.c.request_spawn(SWEEPER_KEY, sask(&t.id)).unwrap();
    assert!(
        !e.c.waits_for_agent(&e.c.get_task(&t.id).unwrap()),
        "a fresh request is starting"
    );
    e.clk.add(SPAWN_GRACE + secs(1));
    assert!(
        e.c.waits_for_agent(&e.c.get_task(&t.id).unwrap()),
        "past its grace it isn't"
    );
    // The normal end of a thread: answered, its agent wound down, nobody
    // holds it. It waits on the person, not on an agent.
    e.write_raw(
        SWEEPER_KEY,
        store::kinds::TAKE_STATE,
        &serde_json::json!({ "taskId": t.id, "reason": TAKE_ANSWERED }),
    );
    let t2 = e.c.get_task(&t.id).unwrap();
    assert!(t2.open_ask().is_some(), "still open");
    assert!(
        !e.c.waits_for_agent(&t2),
        "an answered thread needs no agent"
    );
}

/// Two computers take one ask at once: every device ends on the same one,
/// whatever order the rows reach it in, and a later take beats both.
#[test]
fn an_ask_taken_twice_settles_the_same_on_every_device() {
    fn moved(dev: &str, moves: i64) -> serde_json::Value {
        serde_json::json!({ "taskId": "", "owner": dev, "moves": moves })
    }
    let owner_after = |rows: &[(&str, i64)]| {
        let mut e = Env::on("phone");
        let _r = rig(&mut e, &[]);
        let (t, _) =
            e.c.post_ask(
                "user:phone",
                NewAsk {
                    text: "Why?".into(),
                    thread: true,
                    ..NewAsk::default()
                },
                &[],
            )
            .unwrap();
        for (dev, moves) in rows {
            let mut row = moved(dev, *moves);
            row["taskId"] = t.id.clone().into();
            e.remote(dev, 1, 1, SWEEPER_KEY, store::kinds::ASK_MOVED, &row);
        }
        e.c.get_task(&t.id).unwrap().ask.unwrap().owner
    };
    assert_eq!(owner_after(&[("mac", 1), ("pc", 1)]), "mac");
    assert_eq!(owner_after(&[("pc", 1), ("mac", 1)]), "mac");
    assert_eq!(owner_after(&[("mac", 1), ("pc", 2)]), "pc");
    assert_eq!(owner_after(&[("pc", 2), ("mac", 1)]), "pc");
}

/// Two computers both ask for one card's agent at the same generation: the
/// lowest device id keeps it on every device, whatever the order.
#[test]
fn one_generation_on_two_computers_settles_the_same_on_every_device() {
    let spawn_of = |rows: &[&str]| {
        let mut e = Env::on("phone");
        let _r = rig(&mut e, &[]);
        let t =
            e.c.create_task(
                "user:phone",
                NewTask {
                    title: "Rename".into(),
                    ..NewTask::default()
                },
            )
            .unwrap();
        for dev in rows {
            let row = serde_json::json!({
                "taskId": t.id, "op": SPAWN_REQUESTED, "gen": 1, "device": dev,
                "by": "irori", "permissionMode": "acceptEdits", "name": "a",
            });
            e.remote(dev, 1, 1, SWEEPER_KEY, store::kinds::TASK_SPAWN, &row);
        }
        e.c.get_task(&t.id)
            .unwrap()
            .spawn_request()
            .unwrap()
            .device
            .clone()
    };
    assert_eq!(spawn_of(&["mac", "pc"]), "mac");
    assert_eq!(spawn_of(&["pc", "mac"]), "mac");
}

/// The phone (dev-a) posts a thread it owns; the Mac (dev-b) hears it, with
/// spawning on and `devs` saying which devices can run agents.
fn phone_thread_on_mac(devs: Arc<RwLock<AgentDevices>>) -> (Env, Task) {
    // One person's two devices: both read the person's rows as theirs.
    let owner: KeyFn = Arc::new(|k: &str| k == USER);
    let phone = Env::with("dev-a", Some(owner.clone()));
    let mut mac = Env::with("dev-b", Some(owner));
    let _r = rig(&mut mac, &[]);
    let d = devs.clone();
    mac.c = mac.reopen_with(Options {
        spawning: Some(Arc::new(|| true)),
        agent_devices: Some(Arc::new(move || d.read().unwrap().clone())),
        ..mac.opts()
    });
    let t = post(&phone, "Why is the sync stuck?", true, &[]);
    assert_eq!(t.ask.as_ref().unwrap().owner, "dev-a");
    let rows = phone.st.missing(&mac.st.vector().unwrap(), 1000).unwrap();
    let (_, r) = mac.st.insert_batch(&rows);
    r.unwrap();
    for ev in &rows {
        mac.c.handle(ev).unwrap();
    }
    assert_eq!(ask_of(&mac, &t.id).owner, "dev-a");
    (mac, t)
}

/// Card #541: an ask moves off a computer that went away only once it has
/// been away a while, counted from when it went, not from when the ask was
/// made: a link that dropped a moment ago moves nothing.
#[test]
fn an_ask_moves_off_an_owner_only_once_it_has_been_away_a_while() {
    let devs = Arc::new(RwLock::new(AgentDevices {
        homes: vec!["dev-b".into(), "dev-a".into()],
        moves: HashSet::from(["dev-a".to_string(), "dev-b".to_string()]),
        ..AgentDevices::default()
    }));
    let (mac, t) = phone_thread_on_mac(devs.clone());
    // The ask is old, and dev-a's link only now drops.
    mac.clk.add(mins(5));
    {
        let mut d = devs.write().unwrap();
        d.away.insert("dev-a".into());
        d.away_since
            .insert("dev-a".into(), mac.clk.now().unix_nano());
    }
    mac.c.pull_asks().unwrap();
    assert_eq!(ask_of(&mac, &t.id).owner, "dev-a", "gone a moment");
    mac.clk.add(secs(31));
    mac.c.pull_asks().unwrap();
    assert_eq!(ask_of(&mac, &t.id).owner, "dev-b", "gone a while");
}

/// Card #541: an ask moves only off a device whose Irori reads
/// `ask.moved`: an older one would ignore the move and keep handing the
/// ask out, so two agents would answer it.
#[test]
fn an_ask_stays_with_an_owner_that_does_not_read_moves() {
    let devs = Arc::new(RwLock::new(AgentDevices {
        homes: vec!["dev-b".into()],
        cant: HashSet::from(["dev-a".to_string()]),
        moves: HashSet::from(["dev-b".to_string()]),
        ..AgentDevices::default()
    }));
    let (mac, t) = phone_thread_on_mac(devs.clone());
    mac.c.pull_asks().unwrap();
    assert_eq!(
        ask_of(&mac, &t.id).owner,
        "dev-a",
        "an older phone keeps it"
    );
    devs.write().unwrap().moves.insert("dev-a".into());
    mac.c.pull_asks().unwrap();
    assert_eq!(
        ask_of(&mac, &t.id).owner,
        "dev-b",
        "a phone that reads moves"
    );
}
