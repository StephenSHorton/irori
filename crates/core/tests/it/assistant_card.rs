//! Assistants talk, card 2: the assistant's computer mints one `agent.card`
//! of kind `assistant` per space, points the person's `member.profile` at
//! it, and a device that can't run the answering turn is never that
//! computer. The card is written as itself, never as the person.

use crate::common;

use std::sync::Arc;
use std::time::Duration;

use common::mesh::person;
use common::wait_for;
use irori_core::Core;
use irori_core::spawn::{SpawnStarted, Spawner};
use irori_identity::kinds;
use store::Filter;

fn ready_spawner() -> Spawner {
    Spawner {
        ready: Some(Arc::new(|| true)),
        spawn: Arc::new(|r| {
            Ok(SpawnStarted {
                name: r.name.clone(),
                prefix: String::new(),
            })
        }),
        end: Arc::new(|_| Ok(String::new())),
        trusted: Arc::new(|_| true),
        list: None,
        stop: None,
    }
}

/// The `agent.card` rows of kind `assistant` on space `id`: (actor, cardId).
fn assistant_rows(c: &Core, id: &str) -> Vec<(String, String)> {
    let st = c.board_store(id).unwrap();
    st.list(&Filter {
        kinds: vec![kinds::AGENT_CARD.into()],
        ..Default::default()
    })
    .unwrap()
    .into_iter()
    .filter_map(|ev| {
        let v: serde_json::Value = serde_json::from_slice(&ev.payload).ok()?;
        if v["kind"] != "assistant" {
            return None;
        }
        Some((ev.actor.clone(), v["cardId"].as_str()?.to_string()))
    })
    .collect()
}

fn pointer(c: &Core, id: &str) -> String {
    let v = c.board_verifier(id).unwrap();
    v.profile_assistant_card(&c.identity().current().id)
}

#[test]
fn the_home_device_mints_one_card_per_space_and_points_at_it() {
    let mut devs = person("stephen", 1);
    let d = devs.pop().unwrap();
    let (a, b) = (
        d.core.create_board("Wind Waker", "wind").unwrap(),
        d.core.create_board("Other", "oth").unwrap(),
    );
    d.core.set_chat_turn(Some(common::chat::idle_turn()));
    d.core.set_spawner(Some(ready_spawner()));
    for s in [&a, &b] {
        wait_for(90, "the assistant card is minted and pointed at", || {
            !pointer(&d.core, s).is_empty()
        });
    }
    for s in [&a, &b] {
        let rows = assistant_rows(&d.core, s);
        assert_eq!(rows.len(), 1, "one card per space: {rows:?}");
        let (actor, card) = &rows[0];
        assert_eq!(actor, &format!("agent:{card}"), "written as itself");
        assert_eq!(&pointer(&d.core, s), card);
        // No assistant row is the person's.
        let st = d.core.board_store(s).unwrap();
        let all = st.list(&Filter::default()).unwrap();
        assert!(
            all.iter()
                .filter(|e| e.kind == kinds::AGENT_CARD)
                .all(|e| !e.actor.starts_with("person:")),
            "no person: actor on a card row"
        );
    }
    assert_ne!(
        assistant_rows(&d.core, &a)[0].1,
        assistant_rows(&d.core, &b)[0].1
    );
    d.core.close().unwrap();
}

#[test]
fn a_restart_reuses_the_card_rather_than_minting_a_second() {
    let mut devs = person("stephen", 1);
    let d = devs.pop().unwrap();
    let a = d.core.create_board("Wind Waker", "wind").unwrap();
    d.core.set_chat_turn(Some(common::chat::idle_turn()));
    d.core.set_spawner(Some(ready_spawner()));
    wait_for(90, "minted", || !pointer(&d.core, &a).is_empty());
    let first = pointer(&d.core, &a);
    let d = common::mesh::restart(d);
    d.core.set_chat_turn(Some(common::chat::idle_turn()));
    d.core.set_spawner(Some(ready_spawner()));
    std::thread::sleep(Duration::from_secs(8));
    assert_eq!(pointer(&d.core, &a), first);
    assert_eq!(assistant_rows(&d.core, &a).len(), 1);
    d.core.close().unwrap();
}

/// A rename and a colour pick rewrite the profile through `keep_current`;
/// the pointer rides along instead of being wiped.
#[test]
fn a_rename_or_a_colour_keeps_the_pointer() {
    let mut devs = person("stephen", 1);
    let d = devs.pop().unwrap();
    let a = d.core.create_board("Wind Waker", "wind").unwrap();
    d.core.set_chat_turn(Some(common::chat::idle_turn()));
    d.core.set_spawner(Some(ready_spawner()));
    wait_for(90, "minted", || !pointer(&d.core, &a).is_empty());
    let first = pointer(&d.core, &a);
    d.core.set_person_name("Stevie").unwrap();
    d.core.set_person_color("indigo").unwrap();
    let me = d.core.identity().current().id;
    wait_for(90, "the profile is rewritten", || {
        let v = d.core.board_verifier(&a).unwrap();
        v.profile(&me).as_deref() == Some("Stevie") && v.profile_color(&me) == "indigo"
    });
    assert_eq!(pointer(&d.core, &a), first);
    d.core.close().unwrap();
}

#[test]
fn a_device_that_cant_run_the_turn_never_mints() {
    let mut devs = person("stephen", 1);
    let d = devs.pop().unwrap();
    let a = d.core.create_board("Wind Waker", "wind").unwrap();
    // No Claude Code here: `assistantTurn` stays off, so it is never home.
    let mut sp = ready_spawner();
    sp.ready = Some(Arc::new(|| false));
    d.core.set_spawner(Some(sp));
    wait_for(90, "the host row is published", || {
        d.core
            .spawn_hosts()
            .get(d.core.device_id())
            .is_some_and(|h| h.no_agents)
    });
    std::thread::sleep(Duration::from_secs(8));
    assert!(pointer(&d.core, &a).is_empty());
    assert!(assistant_rows(&d.core, &a).is_empty());
    d.core.close().unwrap();
}

/// One assistant, card 1: the card is minted on the recorded computer, not
/// on whichever computer can run the turn. The person picks the PC (which
/// has no Claude Code): the Mac, the only one that could, mints nothing; it
/// does again when the person picks it back.
#[test]
fn only_the_recorded_computer_mints() {
    let mut devs = person("stephen", 2);
    let (pc, mac) = (devs.pop().unwrap(), devs.pop().unwrap());
    let (mac_id, pc_id) = (
        mac.core.device_id().to_string(),
        pc.core.device_id().to_string(),
    );
    mac.core.set_chat_turn(Some(common::chat::idle_turn()));
    mac.core.set_spawner(Some(ready_spawner()));
    wait_for(90, "the Mac is the assistant's computer on both", || {
        mac.core.assistant_home().as_deref() == Some(&*mac_id)
            && pc.core.assistant_home().as_deref() == Some(&*mac_id)
    });
    mac.core.set_assistant_home(&pc_id).unwrap();
    wait_for(90, "the person's pick of the PC is followed", || {
        mac.core.assistant_home().as_deref() == Some(&*pc_id)
    });
    assert!(!mac.core.is_assistant_home());
    let a = mac.core.create_board("Wind Waker", "wind").unwrap();
    std::thread::sleep(Duration::from_secs(10));
    assert!(
        assistant_rows(&mac.core, &a).is_empty() && pointer(&mac.core, &a).is_empty(),
        "a computer that isn't recorded minted a card"
    );
    mac.core.set_assistant_home(&mac_id).unwrap();
    wait_for(90, "the Mac mints once it is the recorded computer", || {
        !pointer(&mac.core, &a).is_empty()
    });
    assert_eq!(assistant_rows(&mac.core, &a).len(), 1);
    mac.core.close().unwrap();
    pc.core.close().unwrap();
}
