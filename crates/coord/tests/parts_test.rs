//! Several agents on one card (docs/design/spawned-agents.md §3, part 7)
//! on the private board: a helper's part goes to the lead, and a card
//! split in three asks the person once.

mod common;

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

/// The spawn queue's order (card #450, §6): what the person made or asked
/// for, then a lead's helper parts, then agents' cards, each oldest first,
/// whatever their age across the groups.
#[test]
fn the_spawn_queue_puts_the_persons_cards_then_helpers_then_the_rest() {
    let (e, _, made) = limited(0, None, 2);
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
