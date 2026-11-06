//! Several agents on one card (docs/design/spawned-agents.md §3, part 7)
//! on the private board: a helper's part goes to the lead, and a card
//! split in three asks the person once.

use crate::common;

use common::*;
use coord::*;

const LEAD: &str = "dev-a/lead-session";
const HELPERS: [&str; 3] = ["dev-a/helper-1", "dev-a/helper-2", "dev-a/helper-3"];

fn env() -> Env {
    let mut e = Env::new();
    e.c = e.reopen_with(Options {
        done_check: true,
        ..e.opts()
    });
    e
}

fn sub(title: &str, spawn: bool) -> NewSubtask {
    NewSubtask {
        title: title.into(),
        spawn,
        ..NewSubtask::default()
    }
}

fn done_checks(c: &Coord, id: &str) -> usize {
    c.get_task(id)
        .unwrap()
        .blockers
        .iter()
        .filter(|b| b.done_check)
        .count()
}

/// A card the lead holds, split into `subs`, each claimed by a helper.
fn led(e: &Env, subs: Vec<NewSubtask>) -> (Task, Vec<Task>) {
    let p =
        e.c.create_task(
            USER,
            NewTask {
                title: "big card".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(LEAD, &p.id).unwrap();
    let (p, made) = e.c.split(LEAD, &p.id, subs, vec![]).unwrap();
    for (k, h) in made.iter().zip(HELPERS) {
        e.c.claim_task(h, &k.id).unwrap();
    }
    (p, made)
}

#[test]
fn a_card_split_in_three_raises_one_done_check() {
    let e = env();
    let (p, made) = led(
        &e,
        vec![sub("tests", true), sub("ui", true), sub("docs", true)],
    );
    assert!(made.iter().all(|k| k.spawn), "each part is marked");
    for (k, h) in made.iter().zip(HELPERS) {
        // The helper says done: a question for the lead, not the person.
        let got =
            e.c.set_task_state(h, &k.id, COMPLETED, "merged-ready @abc")
                .unwrap();
        assert_eq!(got.state, BLOCKED);
        let b = got.blockers.iter().find(|b| b.open()).unwrap();
        assert_eq!(b.question, PART_QUESTION);
        assert!(!b.done_check);
        assert_eq!(b.context, "merged-ready @abc");
        let on = b.on.as_ref().unwrap();
        assert_eq!(
            (on.kind.as_str(), on.reference.as_str()),
            (BLOCKED_ON_AGENT, LEAD)
        );
        // Saying it again replaces the question.
        let again = e.c.set_task_state(h, &k.id, COMPLETED, "").unwrap();
        assert_eq!(again.blockers.iter().filter(|b| b.open()).count(), 1);
        // The lead accepts: completed, with no done check.
        let ok =
            e.c.set_task_state(LEAD, &k.id, COMPLETED, "merged")
                .unwrap();
        assert_eq!(ok.state, COMPLETED);
        assert_eq!(done_checks(&e.c, &k.id), 0);
    }
    // Only the parent asks the person.
    let got =
        e.c.set_task_state(LEAD, &p.id, COMPLETED, "all three merged")
            .unwrap();
    assert_eq!(got.state, BLOCKED);
    let all: usize = std::iter::once(&p.id)
        .chain(made.iter().map(|k| &k.id))
        .map(|id| done_checks(&e.c, id))
        .sum();
    assert_eq!(all, 1);
    // A replay agrees.
    let c2 = e.reopen();
    assert_eq!(done_checks(&c2, &p.id), 1);
    assert!(made.iter().all(|k| c2.get_task(&k.id).unwrap().spawn));
}

#[test]
fn an_unmarked_subtask_still_asks_the_person() {
    let e = env();
    let (_, made) = led(&e, vec![sub("plain", false)]);
    assert!(!made[0].spawn);
    let got =
        e.c.set_task_state(HELPERS[0], &made[0].id, COMPLETED, "done")
            .unwrap();
    assert_eq!(done_checks(&e.c, &got.id), 1);
}

#[test]
fn a_part_with_no_lead_asks_the_person() {
    let e = env();
    let (p, made) = led(&e, vec![sub("tests", true)]);
    e.c.leave(LEAD, &p.id, LEAD).unwrap();
    e.c.set_task_state(HELPERS[0], &made[0].id, COMPLETED, "done")
        .unwrap();
    assert_eq!(done_checks(&e.c, &made[0].id), 1);
}

/// An env on a spawning board.
fn spawning() -> Env {
    use std::sync::Arc;
    let mut e = env();
    e.c = e.reopen_with(Options {
        done_check: true,
        spawning: Some(Arc::new(|| true)),
        ..e.opts()
    });
    e
}

/// #498's run: the lead's session drops off while the helpers work.
fn lead_lost() -> (Env, Task, Vec<Task>) {
    let e = spawning();
    let (p, made) = led(&e, vec![sub("a", true), sub("b", true), sub("c", true)]);
    e.c.leave(LEAD, &p.id, LEAD).unwrap();
    (e, p, made)
}

fn spawn_ids(c: &Coord) -> Vec<String> {
    c.cards_to_spawn().into_iter().map(|t| t.id).collect()
}

#[test]
fn a_part_whose_lead_went_waits_for_the_next_lead() {
    let (e, p, made) = lead_lost();
    assert!(
        !spawn_ids(&e.c).contains(&p.id),
        "nothing to merge yet: a fresh lead would only idle out"
    );
    for (k, h) in made.iter().zip(HELPERS) {
        e.c.set_task_state(h, &k.id, COMPLETED, "on my branch")
            .unwrap();
        assert_eq!(done_checks(&e.c, &k.id), 0, "not the person's to answer");
    }
    assert!(spawn_ids(&e.c).contains(&p.id));
    assert!(!is_terminal(&e.c.get_task(&p.id).unwrap().state));
}

#[test]
fn the_next_lead_accepts_the_parts_and_the_card_asks_once() {
    let (e, p, made) = lead_lost();
    for (k, h) in made.iter().zip(HELPERS) {
        e.c.set_task_state(h, &k.id, COMPLETED, "on my branch")
            .unwrap();
    }
    let next = "dev-a/next-lead";
    e.c.claim_task(next, &p.id).unwrap();
    assert!(!spawn_ids(&e.c).contains(&p.id), "it has its lead");
    for k in &made {
        e.c.set_task_state(next, &k.id, COMPLETED, "merged")
            .unwrap();
        assert_eq!(done_checks(&e.c, &k.id), 0);
    }
    e.c.set_task_state(next, &p.id, COMPLETED, "merged and landed")
        .unwrap();
    assert_eq!(done_checks(&e.c, &p.id), 1);
}

#[test]
fn a_part_done_before_the_lead_goes_still_brings_a_new_lead() {
    let e = spawning();
    let (p, made) = led(&e, vec![sub("a", true)]);
    e.c.set_task_state(HELPERS[0], &made[0].id, COMPLETED, "on my branch")
        .unwrap();
    e.c.leave(LEAD, &p.id, LEAD).unwrap();
    assert!(spawn_ids(&e.c).contains(&p.id));
    assert_eq!(done_checks(&e.c, &made[0].id), 0);
}

#[test]
fn a_part_waiting_on_a_gone_lead_is_not_stranded_while_a_lead_is_coming() {
    let e = spawning();
    let (p, made) = led(&e, vec![sub("a", true)]);
    e.c.set_task_state(HELPERS[0], &made[0].id, COMPLETED, "on my branch")
        .unwrap();
    e.c.leave(LEAD, &p.id, LEAD).unwrap();
    e.c.leave(HELPERS[0], &made[0].id, HELPERS[0]).unwrap();
    assert!(e.c.stranded(&made[0].id).is_empty());
}

#[test]
fn a_parent_waiting_on_the_person_gets_its_lead_with_their_answer() {
    let e = spawning();
    let (p, made) = led(&e, vec![sub("a", true)]);
    let (_, b) =
        e.c.raise(
            LEAD,
            &p.id,
            NewBlocker {
                question: "Which way?".into(),
                ..NewBlocker::default()
            },
        )
        .unwrap();
    e.c.leave(LEAD, &p.id, LEAD).unwrap();
    e.c.set_task_state(HELPERS[0], &made[0].id, COMPLETED, "on my branch")
        .unwrap();
    assert!(
        !spawn_ids(&e.c).contains(&p.id),
        "Irori would end a lead on an open question at once"
    );
    e.c.answer(
        USER,
        &p.id,
        &b.id,
        BlockerAnswer {
            text: "left".into(),
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    assert!(spawn_ids(&e.c).contains(&p.id));
}

#[test]
fn a_parents_wait_on_its_own_part_doesnt_keep_the_next_lead_away() {
    let e = spawning();
    let (p, made) = led(&e, vec![sub("a", true)]);
    e.c.raise(
        LEAD,
        &p.id,
        NewBlocker {
            question: "Waiting on part a".into(),
            blocked_by: Some(BlockedBy {
                kind: BLOCKED_ON_TASK.into(),
                reference: made[0].id.clone(),
            }),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    e.c.leave(LEAD, &p.id, LEAD).unwrap();
    e.c.set_task_state(HELPERS[0], &made[0].id, COMPLETED, "on my branch")
        .unwrap();
    assert!(spawn_ids(&e.c).contains(&p.id));
}

/// A lead Irori started on `id` drops off, as an idle session does.
fn lead_drops(e: &Env, id: &str, key: &str) {
    let g =
        e.c.request_spawn(
            "irori",
            SpawnAsk {
                task_id: id.into(),
                device: "dev-a".into(),
                folder: "/repo".into(),
                permission_mode: "bypassPermissions".into(),
                name: "lead".into(),
                ..SpawnAsk::default()
            },
        )
        .unwrap()
        .unwrap();
    e.ros.set(key, "busy");
    assert!(e.c.bind_spawn(id, g, key).unwrap());
    e.c.claim_task(key, id).unwrap();
    e.ros.set(key, "");
    e.c.sweep_notices().unwrap();
    e.clk.add(HOLDER_GRACE + std::time::Duration::from_secs(1));
    e.c.sweep_notices().unwrap();
    assert!(e.c.get_task(id).unwrap().holders.is_empty());
}

#[test]
fn the_persons_answer_to_crashed_twice_brings_the_next_lead() {
    let e = spawning();
    let p =
        e.c.create_task(
            USER,
            NewTask {
                title: "big card".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    let lead = "dev-a/first-lead";
    let g =
        e.c.request_spawn(
            "irori",
            SpawnAsk {
                task_id: p.id.clone(),
                device: "dev-a".into(),
                folder: "/repo".into(),
                permission_mode: "bypassPermissions".into(),
                name: "lead".into(),
                ..SpawnAsk::default()
            },
        )
        .unwrap()
        .unwrap();
    e.ros.set(lead, "busy");
    assert!(e.c.bind_spawn(&p.id, g, lead).unwrap());
    e.c.claim_task(lead, &p.id).unwrap();
    let (_, made) =
        e.c.split(lead, &p.id, vec![sub("a", true), sub("b", true)], vec![])
            .unwrap();
    e.ros.set(HELPERS[0], "busy");
    e.ros.set(HELPERS[1], "busy");
    e.c.claim_task(HELPERS[0], &made[0].id).unwrap();
    e.c.claim_task(HELPERS[1], &made[1].id).unwrap();
    e.ros.set(lead, "");
    e.c.sweep_notices().unwrap();
    e.clk.add(HOLDER_GRACE + std::time::Duration::from_secs(1));
    e.c.sweep_notices().unwrap();
    e.c.set_task_state(HELPERS[0], &made[0].id, COMPLETED, "on my branch")
        .unwrap();
    assert!(spawn_ids(&e.c).contains(&p.id));
    lead_drops(&e, &p.id, "dev-a/second-lead");
    let t = e.c.get_task(&p.id).unwrap();
    assert_eq!(t.spawn_crashes, 2);
    let q = t
        .blockers
        .iter()
        .find(|b| b.open() && b.question == CRASHED_QUESTION)
        .expect("asked once")
        .clone();
    assert!(!spawn_ids(&e.c).contains(&p.id));
    e.c.answer(
        USER,
        &p.id,
        &q.id,
        BlockerAnswer {
            text: CRASHED_START.into(),
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    assert_eq!(e.c.get_task(&p.id).unwrap().spawn_crashes, 0);
    assert!(spawn_ids(&e.c).contains(&p.id));
    e.c.set_task_state(HELPERS[1], &made[1].id, COMPLETED, "on my branch")
        .unwrap();
    assert_eq!(done_checks(&e.c, &made[1].id), 0);
}

#[test]
fn a_part_waiting_on_irori_shows_once_no_lead_can_come() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let on = Arc::new(AtomicBool::new(true));
    let switch = on.clone();
    let mut e = env();
    e.c = e.reopen_with(Options {
        done_check: true,
        spawning: Some(Arc::new(move || switch.load(Ordering::SeqCst))),
        ..e.opts()
    });
    let (p, made) = led(&e, vec![sub("a", true)]);
    e.c.leave(LEAD, &p.id, LEAD).unwrap();
    e.c.set_task_state(HELPERS[0], &made[0].id, COMPLETED, "on my branch")
        .unwrap();
    e.c.leave(HELPERS[0], &made[0].id, HELPERS[0]).unwrap();
    assert!(e.c.stranded(&made[0].id).is_empty(), "a lead is coming");
    on.store(false, Ordering::SeqCst);
    assert_eq!(e.c.stranded(&made[0].id).len(), 1);
}

#[test]
fn a_marked_step_moves_its_mark_to_the_subtask_and_marks_stay() {
    let e = env();
    let t =
        e.c.create_task(
            USER,
            NewTask {
                title: "card".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(LEAD, &t.id).unwrap();
    let st = |text: &str, spawn: bool| Step {
        text: text.into(),
        spawn,
        ..Step::default()
    };
    let got =
        e.c.set_steps(LEAD, &t.id, vec![st("plan", false), st("tests", true)])
            .unwrap();
    assert!(!got.steps[0].spawn && got.steps[1].spawn);
    // Redone without the mark: it stays.
    let got =
        e.c.set_steps(LEAD, &t.id, vec![st("plan", false), st("tests", false)])
            .unwrap();
    assert!(got.steps[1].spawn);
    let (_, made) =
        e.c.split(
            LEAD,
            &t.id,
            vec![
                NewSubtask {
                    title: "plan".into(),
                    steps: vec!["s1".into()],
                    ..NewSubtask::default()
                },
                NewSubtask {
                    title: "tests".into(),
                    steps: vec!["s2".into()],
                    ..NewSubtask::default()
                },
            ],
            vec![],
        )
        .unwrap();
    assert!(!made[0].spawn);
    assert!(made[1].spawn);
}

#[test]
fn an_unmarked_subtask_of_a_led_card_is_not_spawned() {
    use std::sync::Arc;
    let mut e = env();
    e.c = e.reopen_with(Options {
        done_check: true,
        spawning: Some(Arc::new(|| true)),
        ..e.opts()
    });
    let p =
        e.c.create_task(
            USER,
            NewTask {
                title: "big card".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(LEAD, &p.id).unwrap();
    let (_, made) =
        e.c.split(
            LEAD,
            &p.id,
            vec![sub("plain", false), sub("helper", true)],
            vec![],
        )
        .unwrap();
    let listed = ids(&e.c.cards_to_spawn());
    assert!(
        !listed.contains(&made[0].id),
        "unmarked: the lead's to give"
    );
    assert!(listed.contains(&made[1].id), "marked: a helper is spawned");
    // With no lead, an unmarked subtask needs its own agent again.
    e.c.leave(LEAD, &p.id, LEAD).unwrap();
    assert!(ids(&e.c.cards_to_spawn()).contains(&made[0].id));
}

/// A led card split into `n` marked parts on a spawning board, with the
/// card's own Up to N and the board's Agents per card.
fn limited(agents: i64, board: Option<i64>, n: usize) -> (Env, Task, Vec<Task>) {
    use std::sync::Arc;
    let mut e = env();
    e.c = e.reopen_with(Options {
        done_check: true,
        spawning: Some(Arc::new(|| true)),
        agents_per_card: board.map(|b| Arc::new(move || b) as Arc<dyn Fn() -> i64 + Send + Sync>),
        ..e.opts()
    });
    let p =
        e.c.create_task(
            USER,
            NewTask {
                title: "big card".into(),
                agents,
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(LEAD, &p.id).unwrap();
    let subs = (0..n).map(|i| sub(&format!("part {i}"), true)).collect();
    let (p, made) = e.c.split(LEAD, &p.id, subs, vec![]).unwrap();
    (e, p, made)
}

/// How many of `made` the sweep lists (all made at one moment, so which
/// ones is by id).
fn listed(e: &Env, made: &[Task]) -> usize {
    let l = ids(&e.c.cards_to_spawn());
    assert!(l.iter().all(|id| made.iter().any(|k| k.id == *id)));
    l.len()
}

#[test]
fn helpers_stay_within_the_cards_agent_limit() {
    // The default 4: the lead and three helpers.
    let (e, _, made) = limited(0, None, 5);
    assert_eq!(listed(&e, &made), 3);
    // Up to 2 on the card: one helper.
    let (e, _, made) = limited(2, None, 3);
    assert_eq!(listed(&e, &made), 1);
    // 1: the lead works alone.
    let (e, _, _) = limited(1, None, 2);
    assert!(e.c.cards_to_spawn().is_empty());
    // The board's Agents per card, unless the card says.
    let (e, _, made) = limited(0, Some(3), 3);
    assert_eq!(listed(&e, &made), 2);
    let (e, p, made) = limited(3, Some(2), 3);
    assert_eq!(listed(&e, &made), 2);
    // A helper on a part takes a slot; the rest wait.
    e.c.claim_task(HELPERS[0], &made[0].id).unwrap();
    assert_eq!(listed(&e, &made), 1);
    assert!(!ids(&e.c.cards_to_spawn()).contains(&made[0].id));
    // Its part done and accepted, the slot is free again.
    e.c.claim_task(HELPERS[1], &made[1].id).unwrap();
    assert!(e.c.cards_to_spawn().is_empty());
    e.c.set_task_state(LEAD, &made[0].id, COMPLETED, "merged")
        .unwrap();
    assert_eq!(ids(&e.c.cards_to_spawn()), [made[2].id.clone()]);
    assert_eq!(e.c.get_task(&p.id).unwrap().agents, 3);
}

#[test]
fn up_to_n_is_checked() {
    let e = env();
    let r = e.c.create_task(
        USER,
        NewTask {
            title: "x".into(),
            agents: MAX_AGENTS_PER_CARD + 1,
            ..NewTask::default()
        },
    );
    want_err(r, ErrorKind::Invalid);
}
