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

/// A led card split into `n` marked parts on a spawning board.
fn split_into(n: usize) -> (Env, Task, Vec<Task>) {
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

/// A card has no limit of its own on its agents (card #576): every part
/// waits for an agent, and the device's cap decides how many start.
#[test]
fn a_cards_helpers_have_no_limit_of_their_own() {
    let (e, _, made) = split_into(20);
    assert_eq!(listed(&e, &made), 20);
    // A part with a helper on it isn't listed; the rest still are.
    e.c.claim_task(HELPERS[0], &made[0].id).unwrap();
    assert_eq!(listed(&e, &made), 19);
    assert!(!ids(&e.c.cards_to_spawn()).contains(&made[0].id));
}

/// The spawn queue's order (card #450, §6): what the person made or asked
/// for, then a lead's helper parts, then agents' cards, each oldest first,
/// whatever their age across the groups.
#[test]
fn the_spawn_queue_puts_the_persons_cards_then_helpers_then_the_rest() {
    let (e, _, made) = split_into(2);
    let at = |i: u64| {
        e.clk.add(std::time::Duration::from_secs(i));
    };
    at(1);
    let agents_card =
        e.c.create_task(
            "dev-a/some-agent",
            NewTask {
                title: "an agent's card".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    at(1);
    let mine =
        e.c.create_task(
            USER,
            NewTask {
                title: "my card".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    at(1);
    let later_agents =
        e.c.create_task(
            "dev-a/some-agent",
            NewTask {
                title: "a later agent card".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    let mut want = vec![mine.id.clone()];
    let mut parts: Vec<String> = made.iter().map(|k| k.id.clone()).collect();
    parts.sort();
    want.extend(parts);
    want.push(agents_card.id.clone());
    want.push(later_agents.id.clone());
    assert_eq!(ids(&e.c.cards_to_spawn()), want);
    // His "Start an agent" on an agent's card moves it to the front group.
    e.c.spawn_now(&later_agents.id);
    let l = ids(&e.c.cards_to_spawn());
    assert_eq!(&l[..2], [mine.id.clone(), later_agents.id.clone()]);
}

fn choice(answer: &str) -> BlockerOption {
    BlockerOption {
        answer: answer.into(),
        ..BlockerOption::default()
    }
}

/// Card #557's run: every part is in, the lead asks "Can I merge?" on the
/// card and is gone before the person answers.
fn merge_asked_lead_gone() -> (Env, Task, Blocker) {
    let e = spawning();
    let (p, made) = led(&e, vec![sub("a", true), sub("b", true)]);
    for (k, h) in made.iter().zip(HELPERS) {
        e.c.set_task_state(h, &k.id, COMPLETED, "on my branch")
            .unwrap();
        e.c.set_task_state(LEAD, &k.id, COMPLETED, "merged")
            .unwrap();
    }
    let (_, b) =
        e.c.raise(
            LEAD,
            &p.id,
            NewBlocker {
                question: "Can I merge PR #7 (covers #1)?".into(),
                options: vec![
                    BlockerOption {
                        recommended: true,
                        ..choice("Merge it")
                    },
                    choice("Not yet"),
                ],
                ..NewBlocker::default()
            },
        )
        .unwrap();
    e.c.leave(LEAD, &p.id, LEAD).unwrap();
    (e, p, b)
}

fn open_ids(c: &Coord) -> Vec<String> {
    c.list_tasks(&TaskFilter {
        open: true,
        ..TaskFilter::default()
    })
    .into_iter()
    .map(|t| t.id)
    .collect()
}

#[test]
fn a_parent_asking_to_merge_reads_blocked_not_done() {
    let (e, p, _) = merge_asked_lead_gone();
    let t = e.c.get_task(&p.id).unwrap();
    assert_eq!(effective_state(&t), BLOCKED, "its own question comes first");
    assert!(
        open_ids(&e.c).contains(&p.id),
        "listed open, where a lead binds"
    );
    assert!(
        !e.c.board(&BoardFilter::default())
            .done
            .iter()
            .any(|c| c.task.id == p.id)
    );
    assert!(
        !spawn_ids(&e.c).contains(&p.id),
        "unanswered: a lead would be ended at once"
    );
}

#[test]
fn merge_it_on_a_parent_whose_lead_went_brings_a_fresh_lead() {
    let (e, p, b) = merge_asked_lead_gone();
    e.c.answer(
        USER,
        &p.id,
        &b.id,
        BlockerAnswer {
            pick: 1,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    assert!(spawn_ids(&e.c).contains(&p.id), "the answer brings a lead");
    // Irori's sweep runs while the lead starts: the answer stays for it,
    // not closed as "the agent it went to left".
    e.c.sweep_notices().unwrap();
    let kept = e.c.get_task(&p.id).unwrap();
    assert!(
        kept.blockers.iter().any(|x| x.id == b.id && x.open()),
        "{kept:?}"
    );
    assert_eq!(effective_state(&kept), BLOCKED);
    assert!(spawn_ids(&e.c).contains(&p.id));
    let next = "dev-a/fresh-lead";
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
        .expect("a lead may be asked for");
    e.ros.set(next, "busy");
    assert!(e.c.bind_spawn(&p.id, g, next).unwrap());
    e.c.claim_task(next, &p.id).unwrap();
    assert!(!spawn_ids(&e.c).contains(&p.id), "it has its lead");
    // The fresh lead acts on it: resolves the answer, merges, lands.
    e.c.answer(
        next,
        &p.id,
        &b.id,
        BlockerAnswer {
            text: "merging PR #7 now".into(),
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    let got =
        e.c.set_task_state(next, &p.id, COMPLETED, "PR #7 merged, main green")
            .unwrap();
    assert_eq!(got.state, BLOCKED, "the person's done check");
    assert_eq!(done_checks(&e.c, &p.id), 1);
    assert!(!spawn_ids(&e.c).contains(&p.id));
}

#[test]
fn a_parents_answer_brings_no_lead_off_a_spawning_board() {
    let e = env();
    let (p, made) = led(&e, vec![sub("a", true)]);
    e.c.set_task_state(LEAD, &made[0].id, COMPLETED, "merged")
        .unwrap();
    let (_, b) =
        e.c.raise(
            LEAD,
            &p.id,
            NewBlocker {
                question: "Can I merge?".into(),
                ..NewBlocker::default()
            },
        )
        .unwrap();
    e.c.leave(LEAD, &p.id, LEAD).unwrap();
    e.c.answer(
        USER,
        &p.id,
        &b.id,
        BlockerAnswer {
            text: "yes".into(),
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    assert!(spawn_ids(&e.c).is_empty());
}

#[test]
fn a_part_waits_on_its_lead_now_and_the_parent_names_no_call_for_it() {
    let e = spawning();
    let (p, made) = led(&e, vec![sub("a", true), sub("b", true)]);
    e.c.set_task_state(HELPERS[0], &made[0].id, COMPLETED, "on my branch")
        .unwrap();
    assert_eq!(e.c.part_lead(&made[0].id), Some(vec![LEAD.to_string()]));
    let r = e.c.get_task(&p.id).unwrap().rollup.unwrap();
    assert!(
        r.blocked.is_empty(),
        "waiting on the lead isn't a call for the person: {r:?}"
    );
    e.c.leave(LEAD, &p.id, LEAD).unwrap();
    assert_eq!(e.c.part_lead(&made[0].id), Some(vec![]), "one is coming");
    assert_eq!(e.c.part_lead(&p.id), None, "the parent isn't a part");
    // A part's own question for the person is still the parent's call.
    e.c.raise(
        HELPERS[1],
        &made[1].id,
        NewBlocker {
            question: "Which db?".into(),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    let r = e.c.get_task(&p.id).unwrap().rollup.unwrap();
    assert_eq!(
        (r.blocked.as_str(), r.blocked_question.as_str()),
        (made[1].id.as_str(), "Which db?")
    );
}

/// The lead was ended but its grace hasn't taken it off the card yet when
/// the person answers: the answer still waits for the next lead.
#[test]
fn an_answer_in_the_ended_leads_grace_still_brings_the_next_lead() {
    let e = spawning();
    let (p, made) = led(&e, vec![sub("a", true)]);
    e.c.set_task_state(HELPERS[0], &made[0].id, COMPLETED, "on my branch")
        .unwrap();
    e.c.set_task_state(LEAD, &made[0].id, COMPLETED, "merged")
        .unwrap();
    let (_, b) =
        e.c.raise(
            LEAD,
            &p.id,
            NewBlocker {
                question: "Can I merge?".into(),
                options: vec![choice("Merge it"), choice("Not yet")],
                ..NewBlocker::default()
            },
        )
        .unwrap();
    e.ros.set(LEAD, "busy");
    assert!(!e.c.answer_brings_lead(&p.id), "its lead is on it");
    e.c.mark_ended(LEAD, e.clk.now());
    assert!(
        e.c.get_task(&p.id)
            .unwrap()
            .holders
            .contains(&LEAD.to_string())
    );
    assert!(e.c.answer_brings_lead(&p.id), "a gone lead counts as gone");
    assert!(!spawn_ids(&e.c).contains(&p.id), "not while it is listed");
    e.c.answer(
        USER,
        &p.id,
        &b.id,
        BlockerAnswer {
            pick: 1,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    // Its session goes off the roster; the sweep takes it off the card.
    e.ros.set(LEAD, "");
    e.c.sweep_notices().unwrap();
    e.clk.add(HOLDER_GRACE + std::time::Duration::from_secs(1));
    e.c.sweep_notices().unwrap();
    let t = e.c.get_task(&p.id).unwrap();
    assert!(t.holders.is_empty(), "{t:?}");
    assert!(t.blockers.iter().any(|x| x.id == b.id && x.open()), "{t:?}");
    assert!(spawn_ids(&e.c).contains(&p.id));
}
