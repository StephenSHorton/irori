//! Card #1075 (merge-desk §12.12 B8): a card, edit or steps row an agent
//! writes on the private log after it read another person's text carries
//! `others`; that card's spawn waits on the person's "OK to start an agent
//! on this?" and a clean agent's card does not.

use crate::common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::*;
use coord::rows::TaskSpawnRow;
use coord::*;
use store::kinds;

const SWEEPER_KEY: &str = "irori";

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        objective: "why it matters".into(),
        ..NewTask::default()
    }
}

/// A coord whose spawn switch is on and whose BOB has read another
/// person's text.
fn env_with_bob_flagged() -> (Env, Coord) {
    let e = Env::new();
    let on = Arc::new(AtomicBool::new(true));
    let flagged = format!("agent:{BOB}");
    let c = e.reopen_with(Options {
        spawning: Some(Arc::new(move || on.load(Ordering::SeqCst))),
        others_of: Some(Arc::new(move |actor: &str| actor == flagged)),
        ..e.opts()
    });
    (e, c)
}

fn ask(id: &str) -> SpawnAsk {
    SpawnAsk {
        task_id: id.into(),
        device: "dev-a".into(),
        folder: "/repo".into(),
        permission_mode: "acceptEdits".into(),
        name: "for-1".into(),
        ..SpawnAsk::default()
    }
}

fn say(c: &Coord, t: &Task, pick: i64) {
    let q = t
        .blockers
        .iter()
        .find(|b| b.question == GIVEN_QUESTION)
        .expect("asked")
        .clone();
    c.answer(
        USER,
        &t.id,
        &q.id,
        BlockerAnswer {
            pick,
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
}

fn ids(v: Vec<Task>) -> Vec<String> {
    v.into_iter().map(|t| t.id).collect()
}

#[test]
fn rows_written_after_reading_others_text_are_marked() {
    let (e, c) = env_with_bob_flagged();
    let mine = c.create_task(BOB, nt("From a flagged session")).unwrap();
    let clean = c.create_task(CAROL, nt("From a clean session")).unwrap();
    let person = c.create_task(USER, nt("The person's own")).unwrap();
    assert!(c.get_task(&mine.id).unwrap().others);
    assert!(!c.get_task(&clean.id).unwrap().others);
    assert!(!c.get_task(&person.id).unwrap().others);
    let created = e.rows(kinds::TASK_CREATED);
    assert_eq!(created.len(), 3);
    let text = |i: usize| String::from_utf8_lossy(&created[i].payload).to_string();
    assert!(text(0).contains("\"others\":true"), "{}", text(0));
    assert!(!text(1).contains("others"));
    assert!(!text(2).contains("others"));

    // An edit and a steps row from the flagged session mark an unmarked
    // card; the mark is sticky.
    c.edit_task(
        BOB,
        &clean.id,
        TaskEdit {
            objective: Some("blake asked for this".into()),
            ..TaskEdit::default()
        },
    )
    .unwrap();
    assert!(c.get_task(&clean.id).unwrap().others, "an edit marks");
    c.edit_task(
        CAROL,
        &clean.id,
        TaskEdit {
            title: Some("tidied by a clean session".into()),
            ..TaskEdit::default()
        },
    )
    .unwrap();
    assert!(c.get_task(&clean.id).unwrap().others, "sticky");

    c.set_steps(
        BOB,
        &person.id,
        vec![Step {
            id: "s1".into(),
            text: "do it".into(),
            ..Step::default()
        }],
    )
    .unwrap();
    assert!(c.get_task(&person.id).unwrap().others, "steps mark");

    // The marks fold the same from the log on a fresh coord.
    let again = e.reopen_with(e.opts());
    for id in [&mine.id, &clean.id, &person.id] {
        assert!(again.get_task(id).unwrap().others, "{id} after replay");
    }
}

#[test]
fn a_marked_card_waits_on_ok_to_start_and_a_clean_one_does_not() {
    let (_e, c) = env_with_bob_flagged();
    let marked = c.create_task(BOB, nt("Marked")).unwrap();
    let clean = c.create_task(CAROL, nt("Clean")).unwrap();
    assert_eq!(
        ids(c.cards_to_spawn()),
        vec![clean.id.clone()],
        "only the clean card spawns"
    );
    assert_eq!(c.spawn_hold(&marked.id), SpawnHold::NeedsOk);
    assert_eq!(c.spawn_hold(&clean.id), SpawnHold::Clear);

    // It is asked about, once.
    assert_eq!(ids(c.marked_to_ask()), vec![marked.id.clone()]);
    c.ask_marked(USER, &marked.id, "Written after reading another person's text.")
        .unwrap();
    let asked = c.get_task(&marked.id).unwrap();
    assert!(
        asked
            .blockers
            .iter()
            .any(|b| b.open() && b.question == GIVEN_QUESTION)
    );
    assert!(c.marked_to_ask().is_empty(), "asked once");
    assert!(!ids(c.cards_to_spawn()).contains(&marked.id));

    // A yes starts one agent; its request is the card's next generation.
    say(&c, &asked, 1);
    let said = c.get_task(&marked.id).unwrap();
    assert!(said.given_said.as_ref().is_some_and(|y| y.yes), "{said:?}");
    assert!(ids(c.cards_to_spawn()).contains(&marked.id));
    assert_eq!(c.spawn_hold(&marked.id), SpawnHold::Clear);
    assert_eq!(
        c.request_spawn(
            SWEEPER_KEY,
            SpawnAsk {
                others: true,
                ..ask(&marked.id)
            }
        )
        .unwrap(),
        Some(1)
    );
    c.bind_spawn(&marked.id, 1, "dev-a/marked-agent").unwrap();
    assert_eq!(c.spawned_others(), vec!["dev-a/marked-agent".to_string()]);
    // The yes is spent with that agent: it holds the card, and a second
    // agent would need another yes.
    assert_eq!(c.spawn_hold(&marked.id), SpawnHold::NoNeed);
}

#[test]
fn a_not_now_leaves_the_card_without_an_agent_until_the_person_starts_one() {
    let (_e, c) = env_with_bob_flagged();
    let marked = c.create_task(BOB, nt("Marked")).unwrap();
    c.ask_marked(USER, &marked.id, "ctx").unwrap();
    let asked = c.get_task(&marked.id).unwrap();
    say(&c, &asked, 2);
    assert!(c.cards_to_spawn().is_empty());
    assert!(c.marked_to_ask().is_empty(), "a Not now isn't asked again");
    // Their own Start an agent on the card is their yes.
    c.spawn_now(&marked.id);
    assert_eq!(c.spawn_hold(&marked.id), SpawnHold::Clear);
    assert_eq!(ids(c.cards_to_spawn()), vec![marked.id.clone()]);
}

#[test]
fn an_agent_cannot_say_yes_for_the_person() {
    let (_e, c) = env_with_bob_flagged();
    let marked = c.create_task(BOB, nt("Marked")).unwrap();
    c.ask_marked(USER, &marked.id, "ctx").unwrap();
    let asked = c.get_task(&marked.id).unwrap();
    let q = asked
        .blockers
        .iter()
        .find(|b| b.question == GIVEN_QUESTION)
        .unwrap()
        .clone();
    // The agent that made the card resolves the question itself.
    let _ = c.answer(
        BOB,
        &marked.id,
        &q.id,
        BlockerAnswer {
            pick: 1,
            resolve: true,
            ..BlockerAnswer::default()
        },
    );
    assert!(!ids(c.cards_to_spawn()).contains(&marked.id));
    assert!(
        c.get_task(&marked.id)
            .unwrap()
            .given_said
            .is_none_or(|y| !y.yes)
    );
}

#[test]
fn the_spawn_row_says_whether_its_agent_read_others_text() {
    let (e, c) = env_with_bob_flagged();
    let clean = c.create_task(CAROL, nt("Clean")).unwrap();
    c.request_spawn(
        SWEEPER_KEY,
        SpawnAsk {
            others: true,
            ..ask(&clean.id)
        },
    )
    .unwrap();
    let rows: Vec<TaskSpawnRow> = e
        .rows(kinds::TASK_SPAWN)
        .iter()
        .map(payload::<TaskSpawnRow>)
        .collect();
    assert!(rows[0].others);
    c.bind_spawn(&clean.id, 1, "dev-a/spawned").unwrap();
    assert_eq!(c.spawned_others(), vec!["dev-a/spawned".to_string()]);
}
