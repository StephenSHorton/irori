//! Card #446 (spawned agents §2, §6, §7): a card on a spawning board gets
//! a `task.spawn` request keyed by card with a gen; while it is pending the
//! card is handed to nobody and repeated kicks write no second request; the
//! agent bound to it carries the spawned_for mark and is never offered
//! another card; rows that no one of this person's could write are ignored.

use crate::common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use common::*;
use coord::roster::AgentCard;
use coord::rows::TaskSpawnRow;
use coord::*;
use gocompat::time::GoTime;
use store::kinds;

const SPAWNED: &str = "dev-a/spawned-session";
const SWEEPER_KEY: &str = "irori";

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        objective: "why it matters".into(),
        ..NewTask::default()
    }
}

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

/// A coord over e's store with the board's switch read from `on`.
fn spawning(e: &Env, on: &Arc<AtomicBool>) -> Coord {
    let f = on.clone();
    e.reopen_with(Options {
        spawning: Some(Arc::new(move || f.load(Ordering::SeqCst))),
        ..e.opts()
    })
}

fn ask(id: &str) -> SpawnAsk {
    SpawnAsk {
        task_id: id.into(),
        device: "dev-a".into(),
        folder: "/repo".into(),
        permission_mode: "bypassPermissions".into(),
        name: "for-1".into(),
        ..SpawnAsk::default()
    }
}

fn spawn_rows(e: &Env) -> Vec<TaskSpawnRow> {
    e.rows(kinds::TASK_SPAWN)
        .iter()
        .map(payload::<TaskSpawnRow>)
        .collect()
}

#[test]
fn a_spawning_board_hands_nothing_out_and_lists_the_card_to_spawn() {
    let mut e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    e.c = spawning(&e, &on);
    let t = e.c.create_task(USER, nt("Port the parser")).unwrap();
    let agents = vec![idle(BOB, 20), idle(CAROL, 10)];
    let out = e.c.sweep_asks(&agents).unwrap();
    assert!(out.is_empty(), "no hand-out on a spawning board: {out:?}");
    assert!(e.c.get_task(&t.id).unwrap().offer.is_none());
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&t.id));
    // Off: the card goes to a free agent as before, and nothing spawns.
    on.store(false, Ordering::SeqCst);
    assert!(e.c.cards_to_spawn().is_empty());
    let out = e.c.sweep_asks(&agents).unwrap();
    assert_eq!(out.len(), 1, "{out:?}");
}

#[test]
fn repeated_kicks_never_spawn_twice() {
    let mut e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    e.c = spawning(&e, &on);
    let t = e.c.create_task(USER, nt("One agent")).unwrap();
    assert_eq!(e.c.request_spawn(SWEEPER_KEY, ask(&t.id)).unwrap(), Some(1));
    for _ in 0..5 {
        assert_eq!(e.c.request_spawn(SWEEPER_KEY, ask(&t.id)).unwrap(), None);
        e.c.sweep_asks(&[idle(BOB, 20)]).unwrap();
    }
    assert_eq!(spawn_rows(&e).len(), 1);
    assert!(e.c.cards_to_spawn().is_empty(), "pending: not listed again");
    // Past the grace a spawn that never showed is asked for again, gen 2.
    e.clk.add(SPAWN_GRACE + secs(1));
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&t.id));
    assert_eq!(e.c.request_spawn(SWEEPER_KEY, ask(&t.id)).unwrap(), Some(2));
}

#[test]
fn a_pending_request_keeps_the_card_from_any_free_agent() {
    // Spawning off on this coord: the plain hand-out still runs, and the
    // pending request alone must keep the card out of it (a terminal agent
    // with Offer it cards, or another device's sweep).
    let e = Env::new();
    let t = e.c.create_task(USER, nt("Starting")).unwrap();
    e.c.request_spawn(SWEEPER_KEY, ask(&t.id)).unwrap();
    let out = e.c.sweep_asks(&[idle(BOB, 20), idle(CAROL, 10)]).unwrap();
    assert!(out.is_empty(), "{out:?}");
    let t = e.c.get_task(&t.id).unwrap();
    assert!(t.spawn_pending());
    assert!(t.live_offer().is_none());
    want_err(
        e.c.offer_card(USER, &t.id, &[idle(BOB, 20)]),
        ErrorKind::BadState,
    );
}

#[test]
fn every_spawn_names_a_permission_mode() {
    let e = Env::new();
    let t = e.c.create_task(USER, nt("Mode")).unwrap();
    let mut a = ask(&t.id);
    a.permission_mode = String::new();
    want_err(
        e.c.request_spawn(SWEEPER_KEY, a.clone()),
        ErrorKind::BadState,
    );
    a.permission_mode = "inherit".into();
    want_err(e.c.request_spawn(SWEEPER_KEY, a), ErrorKind::BadState);
    assert!(spawn_rows(&e).is_empty());
}

#[test]
fn binding_marks_the_agent_and_assigns_it_the_card() {
    let e = Env::new();
    let t = e.c.create_task(USER, nt("Bind")).unwrap();
    let g = e.c.request_spawn(SWEEPER_KEY, ask(&t.id)).unwrap().unwrap();
    assert!(e.c.bind_spawn(&t.id, g, SPAWNED).unwrap());
    let t = e.c.get_task(&t.id).unwrap();
    assert_eq!(t.holders, [SPAWNED]);
    assert_eq!(t.assigned, [SPAWNED]);
    assert!(!t.spawn_pending());
    assert_eq!(e.c.spawned_card(SPAWNED), Some(t.id.clone()));
    // A replay keeps the mark, and so does the agent going.
    let c = e.reopen();
    assert_eq!(c.spawned_card(SPAWNED), Some(t.id.clone()));
    assert_eq!(c.get_task(&t.id).unwrap().holders, [SPAWNED]);
}

#[test]
fn a_spawned_agent_is_never_offered_a_second_card() {
    let e = Env::new();
    let first = e.c.create_task(USER, nt("Its card")).unwrap();
    let g =
        e.c.request_spawn(SWEEPER_KEY, ask(&first.id))
            .unwrap()
            .unwrap();
    e.c.bind_spawn(&first.id, g, SPAWNED).unwrap();
    // Its card is waiting on Stephen's done check: BLOCKED, so busy_with
    // no longer counts it, and only the mark keeps it out.
    e.c.set_task_state(SPAWNED, &first.id, "completed", "done: proof")
        .unwrap();
    let other = e.c.create_task(ALICE, nt("Another card")).unwrap();
    let spawned_idle = idle(SPAWNED, 59);
    let n =
        e.c.offer_card(ALICE, &other.id, std::slice::from_ref(&spawned_idle))
            .unwrap();
    assert!(n.to.is_empty(), "offered a spawned agent: {n:?}");
    let out = e.c.sweep_asks(std::slice::from_ref(&spawned_idle)).unwrap();
    assert!(out.is_empty(), "{out:?}");
    let r = e.c.idle_report(std::slice::from_ref(&spawned_idle));
    assert_eq!(r.idle, 0, "a spawned agent is not a free one");
    // An ask goes to no spawned agent either.
    let (a, n) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "a question".into(),
                ..NewAsk::default()
            },
            &[spawned_idle],
        )
        .unwrap();
    assert!(!n.to.contains(&SPAWNED.to_string()), "{n:?} for {}", a.id);
    // And it can't take one itself.
    want_err(e.c.claim_task(SPAWNED, &other.id), ErrorKind::NotAllowed);
}

/// Card #509: a spawned agent whose work is ready starts a merge card for
/// it and the other ready work, waits on it from its own card, and takes
/// it; it still can't take a card someone else made.
#[test]
fn a_spawned_agent_may_take_the_merge_card_it_made() {
    let e = Env::new();
    let t = e.c.create_task(USER, nt("Its card")).unwrap();
    let g = e.c.request_spawn(SWEEPER_KEY, ask(&t.id)).unwrap().unwrap();
    e.c.bind_spawn(&t.id, g, SPAWNED).unwrap();
    e.c.claim_task(SPAWNED, &t.id).unwrap();
    let merge = e.c.create_task(SPAWNED, nt("Merge: #1 and #2")).unwrap();
    e.c.block(
        SPAWNED,
        &t.id,
        "lands with the merge card",
        Some(BlockedBy {
            kind: BLOCKED_ON_TASK.into(),
            reference: merge.id.clone(),
        }),
    )
    .unwrap();
    e.c.claim_task(SPAWNED, &merge.id).unwrap();
    let other = e.c.create_task(ALICE, nt("Another card")).unwrap();
    want_err(e.c.claim_task(SPAWNED, &other.id), ErrorKind::NotAllowed);
}

#[test]
fn a_spawned_agent_may_take_its_own_cards_subtasks() {
    let e = Env::new();
    let t = e.c.create_task(USER, nt("Lead")).unwrap();
    let g = e.c.request_spawn(SWEEPER_KEY, ask(&t.id)).unwrap().unwrap();
    e.c.bind_spawn(&t.id, g, SPAWNED).unwrap();
    let sub =
        e.c.create_task(
            SPAWNED,
            NewTask {
                title: "part".into(),
                objective: "a part".into(),
                parent: t.id.clone(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(SPAWNED, &sub.id).unwrap();
}

#[test]
fn rows_no_one_of_this_person_could_write_are_ignored() {
    let e = Env::new();
    let t = e.c.create_task(USER, nt("Guarded")).unwrap();
    // dev-b writing as dev-a's agent: not its key to write for.
    let row = TaskSpawnRow {
        task_id: t.id.clone(),
        generation: 1,
        op: SPAWN_REQUESTED.into(),
        device: "dev-a".into(),
        permission_mode: "bypassPermissions".into(),
        by: ALICE.into(),
        at: fixed_now(),
        ..TaskSpawnRow::default()
    };
    e.remote(
        "dev-b",
        1,
        hlc_at(fixed_now()),
        ALICE,
        kinds::TASK_SPAWN,
        &row,
    );
    assert!(!e.c.get_task(&t.id).unwrap().spawn_pending());
    // The person's key written by an agent's actor: not the person.
    let row = TaskSpawnRow {
        by: USER.into(),
        ..row
    };
    e.remote(
        "dev-b",
        2,
        hlc_at(fixed_now()) + 1,
        "dev-b/x",
        kinds::TASK_SPAWN,
        &row,
    );
    assert!(!e.c.get_task(&t.id).unwrap().spawn_pending());
    // A mark for someone's agent from the wrong device does nothing.
    let mark = coord::rows::TaskSpawnedRow {
        task_id: t.id.clone(),
        key: BOB.into(),
        generation: 1,
        device: "dev-a".into(),
        by: CAROL.into(),
        at: fixed_now(),
        ..Default::default()
    };
    e.remote(
        "dev-b",
        3,
        hlc_at(fixed_now()) + 2,
        CAROL,
        kinds::TASK_SPAWNED,
        &mark,
    );
    assert_eq!(e.c.spawned_card(BOB), None);
}

#[test]
fn a_comment_never_spawns() {
    let mut e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    e.c = spawning(&e, &on);
    let t = e.c.create_task(USER, nt("Talked about")).unwrap();
    e.c.claim_task(BOB, &t.id).unwrap();
    e.c.add_comment(ALICE, &t.id, "@bob run this", false)
        .unwrap();
    assert!(
        e.c.cards_to_spawn().is_empty(),
        "a held card needs no agent"
    );
    assert!(spawn_rows(&e).is_empty());
}

// ---- card #447: a card whose spawned agents crash stops spawning ----

/// Spawns agent `key` for card `id` and lets it crash: it drops off the
/// roster and return_gone takes it off the card.
fn spawn_and_crash(e: &Env, id: &str, key: &str) {
    let g = e.c.request_spawn(SWEEPER_KEY, ask(id)).unwrap().unwrap();
    assert!(e.c.bind_spawn(id, g, key).unwrap());
    e.c.claim_task(key, id).unwrap();
    e.ros.set(key, "");
    e.c.sweep_notices().unwrap();
    e.clk.add(HOLDER_GRACE + std::time::Duration::from_secs(1));
    e.c.sweep_notices().unwrap();
    assert!(e.c.get_task(id).unwrap().holders.is_empty());
}

fn crash_question(t: &Task) -> Option<&Blocker> {
    t.blockers
        .iter()
        .find(|b| b.open() && b.question == CRASHED_QUESTION)
}

#[test]
fn one_crash_spawns_again_from_left_off() {
    let mut e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    e.c = spawning(&e, &on);
    let t = e.c.create_task(USER, nt("Crashy")).unwrap();
    spawn_and_crash(&e, &t.id, "dev-a/first");
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.state, SUBMITTED, "{got:?}");
    assert!(crash_question(&got).is_none());
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&t.id));
}

#[test]
fn two_crashes_in_a_row_stop_spawning_and_ask_the_person() {
    let mut e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    e.c = spawning(&e, &on);
    let t = e.c.create_task(USER, nt("Kills its agent")).unwrap();
    spawn_and_crash(&e, &t.id, "dev-a/first");
    spawn_and_crash(&e, &t.id, "dev-a/second");
    let got = e.c.get_task(&t.id).unwrap();
    let q = crash_question(&got).expect("the person is asked").clone();
    assert_eq!(got.state, BLOCKED);
    assert!(e.c.cards_to_spawn().is_empty(), "no third spawn on its own");
    // A replay agrees, and a later sweep doesn't ask twice.
    e.c.sweep_notices().unwrap();
    let c = e.reopen_with(Options {
        spawning: Some(Arc::new(|| true)),
        ..e.opts()
    });
    assert!(c.cards_to_spawn().is_empty());
    let t2 = c.get_task(&t.id).unwrap();
    assert_eq!(
        t2.blockers
            .iter()
            .filter(|b| b.open() && b.question == CRASHED_QUESTION)
            .count(),
        1
    );
    // The person says start another: it is back on the Board and spawns.
    e.c.answer(
        USER,
        &t.id,
        &q.id,
        BlockerAnswer {
            pick: 1,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.state, SUBMITTED, "{got:?}");
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&t.id));
}

/// Card #540: the window sends its answer to the crash question as a
/// resolve. It still goes on the thread, where every brief reads it, and
/// puts the card back on the Board.
#[test]
fn resolving_the_crash_question_clears_the_run() {
    for (pick, text, said) in [
        (1, "", CRASHED_START),
        (0, "try the smaller fixture", "try the smaller fixture"),
    ] {
        let mut e = Env::new();
        let on = Arc::new(AtomicBool::new(true));
        e.c = spawning(&e, &on);
        let t = e.c.create_task(USER, nt("Kills its agent")).unwrap();
        spawn_and_crash(&e, &t.id, "dev-a/first");
        spawn_and_crash(&e, &t.id, "dev-a/second");
        let q = crash_question(&e.c.get_task(&t.id).unwrap())
            .expect("the person is asked")
            .clone();
        e.c.answer(
            USER,
            &t.id,
            &q.id,
            BlockerAnswer {
                pick,
                text: text.into(),
                resolve: true,
                ..BlockerAnswer::default()
            },
        )
        .unwrap();
        let got = e.c.get_task(&t.id).unwrap();
        assert_eq!(got.state, SUBMITTED, "{got:?}");
        assert_eq!(got.spawn_crashes, 0);
        let cs = e.c.comments(&t.id).unwrap();
        assert!(cs.iter().any(|c| c.blocker_id == q.id && c.text == said));
        assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&t.id));
        // A replay agrees.
        let c = e.reopen_with(Options {
            spawning: Some(Arc::new(|| true)),
            ..e.opts()
        });
        assert_eq!(c.get_task(&t.id).unwrap().spawn_crashes, 0);
        assert_eq!(ids(&c.cards_to_spawn()), std::slice::from_ref(&t.id));
    }
}

/// A card someone took while it waited stays theirs, and the resolve stands:
/// the run still clears, so the card can spawn once they let it go.
#[test]
fn resolving_the_crash_question_on_a_held_card_clears_the_run() {
    let mut e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    e.c = spawning(&e, &on);
    let t = e.c.create_task(USER, nt("Kills its agent")).unwrap();
    spawn_and_crash(&e, &t.id, "dev-a/first");
    spawn_and_crash(&e, &t.id, "dev-a/second");
    let q = crash_question(&e.c.get_task(&t.id).unwrap())
        .expect("the person is asked")
        .clone();
    e.ros.set("dev-a/third", "idle");
    e.c.claim_task("dev-a/third", &t.id).unwrap();
    e.c.answer(
        USER,
        &t.id,
        &q.id,
        BlockerAnswer {
            pick: 1,
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.spawn_crashes, 0);
    assert!(got.holders.contains(&"dev-a/third".to_string()), "{got:?}");
    let b = got.blockers.iter().find(|b| b.id == q.id).unwrap();
    assert_eq!(b.resolved.as_ref().unwrap().answer, CRASHED_START);
}

/// Only the person's answer does: an agent resolving the question isn't
/// their say that another may start.
#[test]
fn an_agent_resolving_the_crash_question_keeps_the_run() {
    let mut e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    e.c = spawning(&e, &on);
    let t = e.c.create_task(USER, nt("Kills its agent")).unwrap();
    spawn_and_crash(&e, &t.id, "dev-a/first");
    spawn_and_crash(&e, &t.id, "dev-a/second");
    let q = crash_question(&e.c.get_task(&t.id).unwrap())
        .expect("the person is asked")
        .clone();
    e.c.answer(
        "dev-a/other",
        &t.id,
        &q.id,
        BlockerAnswer {
            text: "looks fine".into(),
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    assert_eq!(
        e.c.get_task(&t.id).unwrap().spawn_crashes,
        MAX_SPAWN_CRASHES
    );
    assert!(e.c.cards_to_spawn().is_empty());
}

#[test]
fn a_finished_agent_between_crashes_breaks_the_run() {
    let mut e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    e.c = spawning(&e, &on);
    let t = e.c.create_task(USER, nt("Mostly fine")).unwrap();
    spawn_and_crash(&e, &t.id, "dev-a/first");
    // The next one leaves on its own (a hand-off), not a crash.
    let g = e.c.request_spawn(SWEEPER_KEY, ask(&t.id)).unwrap().unwrap();
    e.c.bind_spawn(&t.id, g, "dev-a/second").unwrap();
    e.c.claim_task("dev-a/second", &t.id).unwrap();
    e.c.leave("dev-a/second", &t.id, "dev-a/second").unwrap();
    spawn_and_crash(&e, &t.id, "dev-a/third");
    let got = e.c.get_task(&t.id).unwrap();
    assert!(crash_question(&got).is_none(), "{:?}", got.blockers);
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&t.id));
}

#[test]
fn a_spawned_holder_ended_at_its_done_check_finishes_and_nobody_is_told() {
    let mut e = Env::new();
    e.c = e.reopen_with(Options {
        done_check: true,
        ..e.opts()
    });
    // Made by an agent that is still live: a plain holder going would send
    // it "Nobody is on X any more".
    let t = e.c.create_task(BOB, nt("Spawned and done")).unwrap();
    e.ros.set(SPAWNED, "idle");
    let g = e.c.request_spawn(SWEEPER_KEY, ask(&t.id)).unwrap().unwrap();
    e.c.bind_spawn(&t.id, g, SPAWNED).unwrap();
    e.c.set_task_state(SPAWNED, &t.id, "completed", "done: tests pass")
        .unwrap();
    assert_eq!(e.c.get_task(&t.id).unwrap().state, BLOCKED, "a done check");
    // End: Irori writes agent.gone before stop and rm.
    e.st.append(
        "irori",
        kinds::AGENT_GONE,
        &serde_json::json!({ "key": SPAWNED, "reason": "finished" }),
    )
    .unwrap();
    e.c.mark_ended(SPAWNED, e.clk.now());
    e.ros.set(SPAWNED, "");
    let mut told = e.c.sweep_notices().unwrap();
    e.clk.add(ENDED_GRACE + secs(1));
    told.extend(e.c.sweep_notices().unwrap());
    assert!(
        told.iter().all(|n| n.kind != NOTICE_RETURNED),
        "nobody is told the card is open again: {told:?}"
    );
    let after = e.c.get_task(&t.id).unwrap();
    assert!(
        after.holders.is_empty(),
        "off the card within ENDED_GRACE: {after:?}"
    );
    assert!(after.returned.is_none());
    assert_eq!(after.state, BLOCKED, "still waiting on the person's Done");
    let left: Vec<coord::rows::TaskMemberRow> =
        e.rows(kinds::TASK_LEFT).iter().map(payload).collect();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].reason, LEFT_FINISHED);
    // Its mark stays after it went, and a replay reads the same.
    assert_eq!(e.c.spawned_card(SPAWNED), Some(t.id.clone()));
    assert_eq!(e.reopen().spawned_card(SPAWNED), Some(t.id.clone()));
}

// Card #490: a spawned agent starts in its folder and moves once into its
// own worktree; a command it runs in another worktree doesn't move Where.
#[test]
fn a_spawned_agents_where_keeps_to_its_own_worktree() {
    let e = Env::new();
    let t = e.c.create_task(USER, nt("Where")).unwrap();
    let g = e.c.request_spawn(SWEEPER_KEY, ask(&t.id)).unwrap().unwrap();
    assert!(e.c.bind_spawn(&t.id, g, SPAWNED).unwrap());
    let w = |wt: &str, branch: &str| Where {
        who: SPAWNED.into(),
        cwd: wt.into(),
        worktree: wt.into(),
        branch: branch.into(),
        head: "abc1234".into(),
        ..Where::default()
    };
    assert!(e.c.set_where(&t.id, w("/repo", "dev")).unwrap());
    assert!(
        e.c.set_where(&t.id, w("/repo/.claude/worktrees/mine", "worktree-mine"))
            .unwrap()
    );
    assert!(
        !e.c.set_where(&t.id, w("/repo/.claude/worktrees/audit", "worktree-audit"))
            .unwrap()
    );
    assert!(!e.c.set_where(&t.id, w("/repo", "dev")).unwrap());
    let got = e.c.get_task(&t.id).unwrap().where_.unwrap();
    assert_eq!(got.worktree, "/repo/.claude/worktrees/mine");
}

// Card #490 (review): a helper's folder is its own worktree; its Where
// keeps to it, even when it reads its lead's card-named worktree.
#[test]
fn a_helpers_where_keeps_to_its_own_worktree() {
    let e = Env::new();
    let p = e.c.create_task(USER, nt("Lead")).unwrap();
    e.c.claim_task("dev-a/lead-session", &p.id).unwrap();
    let (_, made) =
        e.c.split(
            "dev-a/lead-session",
            &p.id,
            vec![NewSubtask {
                title: "tests".into(),
                spawn: true,
                ..NewSubtask::default()
            }],
            vec![],
        )
        .unwrap();
    let part = &made[0];
    let lead_wt = format!("/repo/.claude/worktrees/card-{}-lead", p.short);
    let own_wt = "/repo/.claude/worktrees/feat-x-tests";
    let g =
        e.c.request_spawn(
            SWEEPER_KEY,
            SpawnAsk {
                folder: own_wt.into(),
                ..ask(&part.id)
            },
        )
        .unwrap()
        .unwrap();
    assert!(e.c.bind_spawn(&part.id, g, SPAWNED).unwrap());
    let w = |wt: &str, branch: &str| Where {
        who: SPAWNED.into(),
        cwd: wt.into(),
        worktree: wt.into(),
        branch: branch.into(),
        head: "abc1234".into(),
        ..Where::default()
    };
    assert!(e.c.set_where(&part.id, w(own_wt, "feat/x-tests")).unwrap());
    assert!(
        !e.c.set_where(
            &part.id,
            w(&lead_wt, &format!("worktree-card-{}-lead", p.short))
        )
        .unwrap()
    );
    assert!(
        !e.c.set_where(
            &part.id,
            w("/repo/.claude/worktrees/audit", "worktree-audit")
        )
        .unwrap()
    );
    let got = e.c.get_task(&part.id).unwrap().where_.unwrap();
    assert_eq!(got.worktree, own_wt);
}

// Card #455 (§9): a request another of the person's devices wrote naming
// this one is this device's to start, while it is the card's newest and
// fresh; a device back online with an older request starts nothing.

fn remote_request(e: &Env, id: &str, seq: i64, generation: i64, device: &str) {
    let row = TaskSpawnRow {
        task_id: id.into(),
        generation,
        op: SPAWN_REQUESTED.into(),
        device: device.into(),
        permission_mode: "acceptEdits".into(),
        name: format!("for-{generation}"),
        by: SWEEPER_KEY.into(),
        at: e.clk.now(),
        ..TaskSpawnRow::default()
    };
    e.remote(
        "dev-b",
        seq,
        hlc_at(e.clk.now()) + seq as u64,
        SWEEPER_KEY,
        kinds::TASK_SPAWN,
        &row,
    );
}

#[test]
fn a_request_from_another_device_naming_this_one_is_this_ones_to_start() {
    let e = Env::new();
    let t = e.c.create_task("user:dev-b", nt("Mac repo")).unwrap();
    remote_request(&e, &t.id, 1, 1, "dev-a");
    let here = e.c.spawns_here();
    assert_eq!(here.len(), 1);
    assert_eq!(here[0].0.id, t.id);
    assert_eq!(here[0].1.generation, 1);
    // It isn't this device's card: its own sweep never asks for it.
    let on = Arc::new(AtomicBool::new(true));
    let c = spawning(&e, &on);
    assert!(c.cards_to_spawn().is_empty());
}

#[test]
fn a_request_naming_another_device_or_superseded_or_stale_starts_nothing_here() {
    let e = Env::new();
    let t = e.c.create_task("user:dev-b", nt("Mac repo")).unwrap();
    remote_request(&e, &t.id, 1, 1, "dev-c");
    assert!(e.c.spawns_here().is_empty(), "names dev-c");
    // gen 2 names this device, then gen 3 moves on to dev-c.
    remote_request(&e, &t.id, 2, 2, "dev-a");
    assert_eq!(e.c.spawns_here().len(), 1);
    remote_request(&e, &t.id, 3, 3, "dev-c");
    assert!(e.c.spawns_here().is_empty(), "superseded by gen 3");
    // An older generation arriving late changes nothing.
    remote_request(&e, &t.id, 4, 2, "dev-a");
    assert!(e.c.spawns_here().is_empty(), "gen 2 after gen 3");
    // Staleness is the card's device's call (its clock, not this one's):
    // past the grace here the request is still listed until the next
    // generation arrives. The Core never starts one it held at its start.
    let u = e.c.create_task("user:dev-b", nt("Other")).unwrap();
    remote_request(&e, &u.id, 5, 1, "dev-a");
    e.clk.add(SPAWN_GRACE);
    assert_eq!(e.c.spawns_here().len(), 1);
    remote_request(&e, &u.id, 6, 2, "dev-c");
    assert!(e.c.spawns_here().is_empty(), "moved on to dev-c");
}

/// Card #574: what a card says about getting an agent, reason by reason.
#[test]
fn spawn_hold_names_what_the_card_itself_is_waiting_on() {
    let e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    let c = spawning(&e, &on);

    let free = c.create_task(USER, nt("Free")).unwrap();
    assert_eq!(c.spawn_hold(&free.id), SpawnHold::Clear);
    assert_eq!(c.spawn_hold("nope"), SpawnHold::NoNeed, "no such card");

    let held = c.create_task(ALICE, nt("Held")).unwrap();
    c.claim_task(ALICE, &held.id).unwrap();
    assert_eq!(c.spawn_hold(&held.id), SpawnHold::NoNeed);

    let join = c
        .create_task(
            USER,
            NewTask {
                shared: true,
                ..nt("Join it")
            },
        )
        .unwrap();
    assert_eq!(c.spawn_hold(&join.id), SpawnHold::Joinable);

    let first = c.create_task(BOB, nt("First")).unwrap();
    c.claim_task(BOB, &first.id).unwrap();
    let after = c.create_task(USER, nt("After")).unwrap();
    c.raise(
        USER,
        &after.id,
        NewBlocker {
            question: "waits".into(),
            blocked_by: Some(BlockedBy {
                kind: BLOCKED_ON_TASK.into(),
                reference: first.id.clone(),
            }),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    assert_eq!(c.spawn_hold(&after.id), SpawnHold::Blocked);

    let done = c.create_task(CAROL, nt("Done")).unwrap();
    c.claim_task(CAROL, &done.id).unwrap();
    c.set_task_state(CAROL, &done.id, CANCELED, "").unwrap();
    assert_eq!(c.spawn_hold(&done.id), SpawnHold::NoNeed);
    assert_eq!(
        c.card_origin(&free.id),
        "dev-a",
        "a person's card, from its key"
    );
    assert_eq!(c.card_origin(&held.id), "dev-a");
}

/// Card #574: a board wait in this log fires as a card's would, on what the
/// core reads from the board; a lost card ends it and says so.
#[test]
fn a_board_wait_fires_on_the_state_the_core_reads() {
    let e = Env::new();
    let c = &e.c;
    let spec = |until: &[&str], state: &str| WaitSpec {
        task_id: "01BOARDCARD".into(),
        until: until.iter().map(|u| u.to_string()).collect(),
        on_board: Some(BoardCard {
            board: "b1".into(),
            card: "mlg#7, task:01BOARDCARD".into(),
            title: "Theirs".into(),
            state: state.into(),
            own: false,
        }),
        ..WaitSpec::default()
    };
    let w = c
        .create_wait(ALICE, spec(&["done", "unblocked"], BLOCKED))
        .unwrap();
    assert_eq!(w.from_state, BLOCKED);
    assert_eq!(c.board_waits().len(), 1);
    assert_eq!(c.board_wait_moved(&w.id, BLOCKED), None, "still blocked");
    assert_eq!(
        c.board_wait_moved(&w.id, WORKING).as_deref(),
        Some(BLOCKED),
        "unblocked"
    );
    c.fire_wait(&w.id, "mlg#7 is now working");
    let (ns, r) = c.sweep_notices_partial();
    r.unwrap();
    let n = ns
        .iter()
        .find(|n| n.to.contains(&ALICE.to_string()))
        .expect("a wake");
    assert!(n.wake);
    assert_eq!(n.title, "Theirs");
    assert!(
        n.text
            .contains("mlg#7, task:01BOARDCARD until done or unblocked"),
        "{}",
        n.text
    );
    assert!(c.board_waits().is_empty(), "it fires once");

    // The sweep leaves a board wait alone: its card is in no log here.
    let w = c.create_wait(ALICE, spec(&["done"], WORKING)).unwrap();
    c.sweep_notices_partial().1.unwrap();
    assert_eq!(c.board_waits().len(), 1, "not ended as card_gone");
    c.board_wait_lost(&w.id);
    let (ns, r) = c.sweep_notices_partial();
    r.unwrap();
    assert!(c.board_waits().is_empty());
    let n = ns.iter().find(|n| n.kind == NOTICE_WAIT).expect("a notice");
    assert!(!n.wake, "nothing happened: said at its next turn");
    assert!(n.text.contains("ended without firing"), "{}", n.text);

    let own = BoardCard {
        own: true,
        ..spec(&[], WORKING).on_board.unwrap()
    };
    let e2 = c
        .create_wait(
            ALICE,
            WaitSpec {
                on_board: Some(own),
                ..spec(&[], WORKING)
            },
        )
        .unwrap_err();
    assert!(e2.to_string().contains("your own card"), "{e2}");
}

/// A wait's waiter on another device is that device's: this one reads
/// nothing for it.
#[test]
fn a_board_wait_is_read_on_its_waiters_device() {
    let e = Env::new();
    let c = &e.c;
    c.create_wait(
        "dev-b/agent",
        WaitSpec {
            task_id: "01BOARDCARD".into(),
            on_board: Some(BoardCard {
                board: "b1".into(),
                state: WORKING.into(),
                ..BoardCard::default()
            }),
            ..WaitSpec::default()
        },
    )
    .unwrap();
    assert_eq!(c.waits("").len(), 1);
    assert!(c.board_waits().is_empty());
}

/// Card #574: the crash question blocks the card, and the line still says
/// it crashed, not just that it's blocked.
#[test]
fn a_card_that_crashed_twice_says_so() {
    let mut e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    e.c = spawning(&e, &on);
    let t = e.c.create_task(USER, nt("Kills its agent")).unwrap();
    spawn_and_crash(&e, &t.id, "dev-a/first");
    spawn_and_crash(&e, &t.id, "dev-a/second");
    assert_eq!(e.c.spawn_hold(&t.id), SpawnHold::Crashed);
}

/// The crash limit is the person's (card #576): read live, with the
/// question saying the number; at the default it's the words it always was.
fn limited_crashes(e: &Env, on: &Arc<AtomicBool>, limit: &Arc<AtomicI64>) -> Coord {
    let (f, n) = (on.clone(), limit.clone());
    e.reopen_with(Options {
        spawning: Some(Arc::new(move || f.load(Ordering::SeqCst))),
        crash_limit: Some(Arc::new(move || n.load(Ordering::SeqCst))),
        ..e.opts()
    })
}

#[test]
fn a_higher_crash_limit_keeps_starting_and_says_its_number() {
    let mut e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    let limit = Arc::new(AtomicI64::new(3));
    e.c = limited_crashes(&e, &on, &limit);
    let t = e.c.create_task(USER, nt("Kills its agent")).unwrap();
    spawn_and_crash(&e, &t.id, "dev-a/first");
    spawn_and_crash(&e, &t.id, "dev-a/second");
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&t.id));
    spawn_and_crash(&e, &t.id, "dev-a/third");
    assert!(e.c.cards_to_spawn().is_empty());
    let got = e.c.get_task(&t.id).unwrap();
    let b = got.blockers.iter().find(|b| b.open()).unwrap().clone();
    assert_eq!(b.question, coord::crash_question(3));
    assert!(b.context.starts_with("The last 3 agents"), "{b:?}");
    assert!(is_crash_question(&b));

    // The person's answer clears the run at any limit.
    e.c.answer(
        USER,
        &t.id,
        &b.id,
        BlockerAnswer {
            pick: 1,
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.spawn_crashes, 0, "{got:?}");
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&t.id));
}

#[test]
fn a_crash_limit_lowered_under_a_card_asks_the_person() {
    let mut e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    let limit = Arc::new(AtomicI64::new(0));
    e.c = limited_crashes(&e, &on, &limit);
    let t = e.c.create_task(USER, nt("Crashed once")).unwrap();
    spawn_and_crash(&e, &t.id, "dev-a/first");
    assert_eq!(e.c.crash_limit(), MAX_SPAWN_CRASHES, "0 is the default");
    assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&t.id));
    limit.store(1, Ordering::SeqCst);
    assert!(e.c.cards_to_spawn().is_empty(), "no agent, so it must ask");
    e.c.stop_crashed(&t.id).unwrap();
    e.c.stop_crashed(&t.id).unwrap();
    let got = e.c.get_task(&t.id).unwrap();
    let open: Vec<_> = got.blockers.iter().filter(|b| b.open()).collect();
    assert_eq!(open.len(), 1, "asked once: {open:?}");
    assert_eq!(open[0].question, coord::crash_question(1));
}

#[test]
fn the_crash_question_at_the_default_is_unchanged() {
    assert_eq!(coord::crash_question(MAX_SPAWN_CRASHES), CRASHED_QUESTION);
}

/// Card #575: a start an agent asked for counts toward the person's limit
/// and never goes first, even on their own card, until they say so.
#[test]
fn an_agents_counted_start_queues_behind_the_persons_cards() {
    let mut e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    e.c = spawning(&e, &on);
    let older =
        e.c.create_task(USER, nt("An agent asked for this"))
            .unwrap();
    e.clk.add(std::time::Duration::from_secs(1));
    let newer = e.c.create_task(USER, nt("Mine")).unwrap();
    let g =
        e.c.request_spawn(
            SWEEPER_KEY,
            SpawnAsk {
                counted: true,
                ..ask(&older.id)
            },
        )
        .unwrap()
        .unwrap();
    assert!(e.c.bind_spawn(&older.id, g, "dev-a/first").unwrap());
    e.c.claim_task("dev-a/first", &older.id).unwrap();
    e.ros.set("dev-a/first", "");
    e.c.sweep_notices().unwrap();
    e.clk.add(HOLDER_GRACE + std::time::Duration::from_secs(1));
    e.c.sweep_notices().unwrap();
    assert_eq!(
        ids(&e.c.cards_to_spawn()),
        [newer.id.clone(), older.id.clone()]
    );
    // The person's Start puts it first again.
    e.c.spawn_now(&older.id);
    assert_eq!(
        ids(&e.c.cards_to_spawn()),
        [older.id.clone(), newer.id.clone()]
    );
}
