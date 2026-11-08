//! Settings › Agents › Running agents (card #448): each row names its card
//! and the model and effort its agent started with. An agent on a shared
//! space's card shows the same as one on a Personal card (card #1070): its
//! mark in the person's own log names the space, and the card lives there.

use crate::common;

use common::*;
use irori_window::agents::RunningAgent;

fn ask(task_id: &str, device: &str, name: &str, model: &str, effort: &str) -> coord::SpawnAsk {
    coord::SpawnAsk {
        task_id: task_id.into(),
        device: device.into(),
        model: model.into(),
        effort: effort.into(),
        permission_mode: "acceptEdits".into(),
        name: name.into(),
        ..coord::SpawnAsk::default()
    }
}

fn row<'a>(rows: &'a [RunningAgent], card_id: &str) -> &'a RunningAgent {
    rows.iter()
        .find(|r| r.card_id == card_id)
        .unwrap_or_else(|| panic!("no row for card {card_id}: {rows:?}"))
}

#[test]
fn a_shared_space_agent_shows_its_card_and_model_like_a_personal_one() {
    let r = Rig::new();
    let w = &r.w;
    let me = w.core.device_id().to_string();
    let person = w.core.user_sender().key;

    // A Personal card whose agent is beta.
    let co = w.core.coord();
    let own = co
        .create_task(
            &person,
            coord::NewTask {
                title: "Personal card".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let g = co
        .request_spawn(&person, ask(&own.id, &me, "beta", "sonnet", "medium"))
        .unwrap()
        .expect("the Personal request was written");
    assert!(co.bind_spawn(&own.id, g, &r.key_b).unwrap());

    // A shared space's card whose agent is alpha: bound on the space as
    // its agent:<cardId>, marked in the person's own log with the space,
    // as the spawner does (card #455).
    let mlg = w.boards_create("MLG", "mlg").unwrap();
    let bco = w.core.board_coord(&mlg.board_id).unwrap();
    let bme = w.core.board_key(&mlg.board_id, &person).unwrap();
    let shared = bco
        .create_task(
            &bme,
            coord::NewTask {
                title: "Shared card".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let g = bco
        .request_spawn(&bme, ask(&shared.id, &me, "alpha", "opus", "high"))
        .unwrap()
        .expect("the shared request was written");
    co.mark_spawned_elsewhere(&r.key_a, &shared.id, g, &me, &mlg.board_id, false)
        .unwrap();
    let alpha = w.core.board_key(&mlg.board_id, &r.key_a).unwrap();
    assert!(bco.bind_spawn(&shared.id, g, &alpha).unwrap());

    // A second shared card whose agent is still starting.
    let waiting = bco
        .create_task(
            &bme,
            coord::NewTask {
                title: "Shared card starting".into(),
                ..Default::default()
            },
        )
        .unwrap();
    bco.request_spawn(&bme, ask(&waiting.id, &me, "gamma", "haiku", "low"))
        .unwrap()
        .expect("the starting request was written");

    let rows = w.agents_running();
    let p = row(&rows, &own.id);
    assert_eq!(
        (p.key.as_str(), p.card_short.as_str(), p.card_title.as_str()),
        (r.key_b.as_str(), "#1", "Personal card"),
        "{p:?}"
    );
    assert_eq!((p.model.as_str(), p.effort.as_str()), ("sonnet", "medium"));

    let s = row(&rows, &shared.id);
    assert_eq!(
        (s.key.as_str(), s.card_short.as_str(), s.card_title.as_str()),
        (r.key_a.as_str(), "mlg#1", "Shared card"),
        "{s:?}"
    );
    assert_eq!(
        (s.model.as_str(), s.effort.as_str(), s.here),
        ("opus", "high", true),
        "{s:?}"
    );

    let st = row(&rows, &waiting.id);
    assert_eq!(
        (
            st.status.as_str(),
            st.card_short.as_str(),
            st.card_title.as_str()
        ),
        ("starting", "mlg#2", "Shared card starting"),
        "{st:?}"
    );
    assert_eq!((st.model.as_str(), st.effort.as_str()), ("haiku", "low"));
    assert_eq!(rows.len(), 3, "{rows:?}");
}
