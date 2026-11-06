//! Card #452 (spawned-agents.md §4 "Reaching an agent that has gone",
//! decision 6): a message to a spawned agent that has finished lands on
//! the card it was spawned for, marked gone, and wakes that card's holder.
//! It never asks for an agent; an agent that wasn't spawned keeps today's
//! path.

use crate::common;

use common::*;
use coord::{NewTask, SpawnAsk};
use store::kinds;

fn to_finished(text: &str) -> inbox::Message {
    inbox::Message {
        id: "msg_01FINISHEDTEST0000000000000".into(),
        from: inbox::Sender::new(KEY_C, "gamma", "claude"),
        to: inbox::Address::new("agent", KEY_A),
        text: text.into(),
        ..inbox::Message::default()
    }
}

#[test]
fn a_message_to_a_finished_spawned_agent_lands_on_its_card_and_wakes_the_holder() {
    let h = new_harness();
    let co = &h.b.co;
    let t = co
        .create_task(
            "user:dev1",
            NewTask {
                title: "Save format v2".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    let g = co
        .request_spawn(
            "irori",
            SpawnAsk {
                task_id: t.id.clone(),
                device: "dev1".into(),
                folder: "/repo".into(),
                permission_mode: "bypassPermissions".into(),
                name: "sorrel".into(),
                ..SpawnAsk::default()
            },
        )
        .unwrap()
        .unwrap();
    assert!(co.bind_spawn(&t.id, g, KEY_A).unwrap());
    // sorrel finished and left; beta took the card on.
    co.leave(KEY_A, &t.id, KEY_A).unwrap();
    co.claim_task(KEY_B, &t.id).unwrap();
    h.b.roster.remove(KEY_A);
    let spawns = h.b.st_rows(kinds::TASK_SPAWN);

    let m = to_finished("does v2 keep the slot thumbnails?");
    let (tid, n) = tools::notice::record_for_finished(co, &m, KEY_A, "sorrel")
        .unwrap()
        .expect("sorrel was spawned for a card");
    assert_eq!(tid, t.id);

    let cm = co.get_comment(&m.id).expect("the message is on the card");
    assert!(
        cm.gone && cm.task_id == t.id && cm.to == KEY_A && cm.by == KEY_C,
        "{cm:?}"
    );
    assert_eq!(cm.text, "does v2 keep the slot thumbnails?");

    assert!(n.wake, "the holder is woken");
    assert_eq!(n.to, vec![KEY_B.to_string()]);
    assert_eq!(n.reference, format!("task:{}", t.id));
    assert!(
        n.text
            .starts_with("A message for sorrel, who has finished, landed on #"),
        "{}",
        n.text
    );

    // Nothing starts an agent: no new spawn request, nobody new on it.
    assert_eq!(h.b.st_rows(kinds::TASK_SPAWN), spawns);
    assert_eq!(co.get_task(&t.id).unwrap().holders, vec![KEY_B.to_string()]);

    // Again (the sweep retries): one entry, still nothing spawned.
    tools::notice::record_for_finished(co, &m, KEY_A, "sorrel").unwrap();
    assert_eq!(
        co.comments(&t.id)
            .unwrap()
            .iter()
            .filter(|c| c.id == m.id)
            .count(),
        1
    );
    assert_eq!(h.b.st_rows(kinds::TASK_SPAWN), spawns);
}

#[test]
fn a_finished_agents_card_with_nobody_on_it_keeps_the_message_for_the_next_agent() {
    let h = new_harness();
    let co = &h.b.co;
    let t = co
        .create_task(
            "user:dev1",
            NewTask {
                title: "Net messages".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    let g = co
        .request_spawn(
            "irori",
            SpawnAsk {
                task_id: t.id.clone(),
                device: "dev1".into(),
                folder: "/repo".into(),
                permission_mode: "bypassPermissions".into(),
                name: "yarrow".into(),
                ..SpawnAsk::default()
            },
        )
        .unwrap()
        .unwrap();
    co.bind_spawn(&t.id, g, KEY_A).unwrap();
    co.leave(KEY_A, &t.id, KEY_A).unwrap();
    h.b.roster.remove(KEY_A);
    let spawns = h.b.st_rows(kinds::TASK_SPAWN);

    let m = to_finished("ready to merge?");
    let (_, n) = tools::notice::record_for_finished(co, &m, KEY_A, "")
        .unwrap()
        .unwrap();
    assert!(co.get_comment(&m.id).is_some_and(|c| c.gone));
    assert!(n.to.is_empty(), "nobody to wake: {:?}", n.to);
    assert_eq!(h.b.st_rows(kinds::TASK_SPAWN), spawns, "no agent asked for");
}

#[test]
fn an_agent_that_was_not_spawned_keeps_todays_path() {
    let h = new_harness();
    h.b.roster.remove(KEY_A);
    let m = to_finished("anything else?");
    assert!(
        tools::notice::record_for_finished(&h.b.co, &m, KEY_A, "alpha")
            .unwrap()
            .is_none()
    );
    assert!(h.b.co.get_comment(&m.id).is_none());
}
