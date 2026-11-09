//! Card #1215 (agent-continuity §6, §7's fallback, A.4): a spawned agent
//! that rests lets go of its cards, takes and leases itself, with a leave
//! that counts no crash and tells nobody; it is never gone, and until it
//! can be woken nothing is sent to it: a thread it answered opens again
//! for a fresh agent, and an ask it let go waits for the person.

use crate::common;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use common::*;
use coord::*;
use gocompat::time::GoTime;
use store::kinds;

const SPAWNED: &str = "dev-a/spawned-session";
const SWEEPER_KEY: &str = "irori";

type Resting = Arc<RwLock<HashMap<String, GoTime>>>;

/// Spawning on, and the resting agents read from `rs`, as core's map.
fn rig(e: &mut Env) -> Resting {
    let rs: Resting = Arc::default();
    let on = Arc::new(AtomicBool::new(true));
    let (f, r) = (on.clone(), rs.clone());
    e.c = e.reopen_with(Options {
        done_check: true,
        spawning: Some(Arc::new(move || f.load(Ordering::SeqCst))),
        resting: Some(Arc::new(move |k: &str| r.read().unwrap().get(k).copied())),
        ..e.opts()
    });
    rs
}

fn sask(id: &str) -> SpawnAsk {
    SpawnAsk {
        task_id: id.into(),
        device: "dev-a".into(),
        folder: "/repo".into(),
        permission_mode: "bypassPermissions".into(),
        name: "rest-1".into(),
        ..SpawnAsk::default()
    }
}

fn bound(e: &Env, id: &str) {
    let g = e.c.request_spawn(SWEEPER_KEY, sask(id)).unwrap().unwrap();
    assert!(e.c.bind_spawn(id, g, SPAWNED).unwrap());
}

/// What core does as the agent rests: the map first, then the let-go.
fn rest(e: &Env, rs: &Resting) -> Vec<String> {
    rs.write().unwrap().insert(SPAWNED.into(), e.clk.now());
    e.ros.set(SPAWNED, roster::RESTING);
    e.c.rest_leave(SPAWNED).unwrap()
}

#[test]
fn a_resting_agent_lets_its_card_and_leases_go_and_nobody_is_told() {
    let mut e = Env::new();
    let rs = rig(&mut e);
    // Made by an agent that is still live: a plain holder going would send
    // it "Nobody is on X any more".
    let t = e
        .c
        .create_task(
            BOB,
            NewTask {
                title: "Fix the save".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.ros.set(BOB, "idle");
    e.ros.set(SPAWNED, "busy");
    bound(&e, &t.id);
    e.c.claim_task(SPAWNED, &t.id).unwrap();
    assert_eq!(e.c.get_task(&t.id).unwrap().state, WORKING);
    e.c.acquire(SPAWNED, "port:8799", Duration::from_secs(600), false)
        .unwrap();

    assert_eq!(rest(&e, &rs), vec![t.id.clone()]);
    let after = e.c.get_task(&t.id).unwrap();
    assert!(after.holders.is_empty(), "{after:?}");
    assert_eq!(after.state, SUBMITTED, "it goes up again");
    assert_eq!(after.spawn_crashes, 0);
    assert!(after.returned.is_none());
    assert_eq!(after.rested.as_ref().map(|r| r.was.as_str()), Some(SPAWNED));
    let left: Vec<rows::TaskMemberRow> = e.rows(kinds::TASK_LEFT).iter().map(payload).collect();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].reason, LEFT_RESTED);
    assert!(
        e.c.leases().iter().all(|l| l.holder != SPAWNED),
        "its lease is released"
    );
    // Asleep, not gone: no take-back, no notice, and it keeps its count.
    let mut told = e.c.sweep_notices().unwrap();
    e.clk.add(HOLDER_GRACE + secs(1));
    told.extend(e.c.sweep_notices().unwrap());
    assert!(
        told.iter().all(|n| n.kind != NOTICE_RETURNED),
        "{told:?}"
    );
    assert!(!e.c.key_gone(SPAWNED));
    assert!(e.c.spawned_live(SPAWNED));
    // Even before the roster lists it again (a restart).
    e.ros.set(SPAWNED, "");
    assert!(!e.c.key_gone(SPAWNED));
    // A second let-go has nothing to do.
    assert!(e.c.rest_leave(SPAWNED).unwrap().is_empty());
}

#[test]
fn a_resting_agent_at_its_done_check_leaves_the_check_with_the_person() {
    let mut e = Env::new();
    let rs = rig(&mut e);
    let t = e
        .c
        .create_task(
            USER,
            NewTask {
                title: "Ship it".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.ros.set(SPAWNED, "idle");
    bound(&e, &t.id);
    e.c.claim_task(SPAWNED, &t.id).unwrap();
    e.c.set_task_state(SPAWNED, &t.id, "completed", "done: shipped")
        .unwrap();
    rest(&e, &rs);
    let after = e.c.get_task(&t.id).unwrap();
    assert_eq!(after.state, BLOCKED, "still waiting on the person's Done");
    assert!(after.holders.is_empty());
}

#[test]
fn a_thread_whose_replier_rests_opens_again_for_a_fresh_agent() {
    let mut e = Env::new();
    let rs = rig(&mut e);
    let (tk, _) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "What did we ship today?".into(),
                thread: true,
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap();
    e.ros.set(SPAWNED, "idle");
    bound(&e, &tk.id);
    e.c.add_comment(SPAWNED, &tk.id, "The save fix and the new roster.", false)
        .unwrap();
    rest(&e, &rs);
    e.c.sweep_asks(&[]).unwrap();
    e.clk.add(mins(10));
    e.c.sweep_asks(&[]).unwrap();
    assert!(e.c.cards_to_spawn().is_empty(), "quiet: it waits for the person");
    // The person follows up: nothing is sent to the resting replier, and
    // the thread goes out again so a fresh agent answers.
    e.clk.add(mins(1));
    let (_, ns) =
        e.c.person_comment(USER, &tk.id, "And yesterday?", true, &[], None)
            .unwrap();
    assert!(
        ns.iter().all(|n| !n.to.iter().any(|k| k == SPAWNED)),
        "{ns:?}"
    );
    e.c.sweep_asks(&[]).unwrap();
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&tk.id));
}

#[test]
fn an_ask_whose_agent_rests_waits_for_the_person() {
    let mut e = Env::new();
    let rs = rig(&mut e);
    let (tk, _) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "Should we drop Windows 10?".into(),
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap();
    e.ros.set(SPAWNED, "idle");
    bound(&e, &tk.id);
    e.c.sweep_asks(&[]).unwrap();
    // The person wrote while it worked: that was its to answer.
    e.c.add_comment(USER, &tk.id, "Look at the install numbers.", false)
        .unwrap();
    e.clk.add(mins(1));
    rest(&e, &rs);
    e.c.sweep_asks(&[]).unwrap();
    e.clk.add(mins(10));
    e.c.sweep_asks(&[]).unwrap();
    let t = e.c.get_task(&tk.id).unwrap();
    assert!(t.ask.as_ref().unwrap().taker.is_empty(), "{t:?}");
    assert_eq!(t.spawn_crashes, 0);
    assert!(e.c.cards_to_spawn().is_empty(), "waits for the person");
    // The person writes again: a fresh agent.
    e.clk.add(mins(1));
    e.c.add_comment(USER, &tk.id, "Any thoughts?", false)
        .unwrap();
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&tk.id));
}

#[test]
fn a_resting_creator_isnt_the_answerer_of_its_card() {
    let mut e = Env::new();
    let rs = rig(&mut e);
    let t = e
        .c
        .create_task(
            SPAWNED,
            NewTask {
                title: "A card the agent made".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.ros.set(SPAWNED, "idle");
    rs.write().unwrap().insert(SPAWNED.into(), e.clk.now());
    e.ros.set(SPAWNED, roster::RESTING);
    let (_, ns) =
        e.c.person_comment(USER, &t.id, "Is this still needed?", true, &[], None)
            .unwrap();
    assert!(
        ns.iter().all(|n| !n.to.iter().any(|k| k == SPAWNED)),
        "{ns:?}"
    );
}
