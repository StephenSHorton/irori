//! Wind down (cards #220, #307, #787): each agent's "wound down" shows in
//! the run's status and never as a message to the person; the switch holds
//! agent starts from the moment it goes on, and Off cancels what hasn't
//! reached an agent and lifts only what On set.

use crate::common;

use common::*;
use irori_window::wind_down::row;

#[test]
fn wound_down_replies_show_only_in_the_dialog() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    let me = r.w.core.user_sender().key;
    let st = r.w.wind_down_all().unwrap();
    let rows: Vec<_> = st
        .rows
        .iter()
        .filter(|x| x.key == r.key_a || x.key == r.key_b)
        .cloned()
        .collect();
    assert_eq!(rows.len(), 2, "{st:?}");
    for (key, name) in [(&r.key_a, "alpha"), (&r.key_b, "beta")] {
        let x = rows.iter().find(|x| &x.key == key).unwrap();
        ib.reply(
            &inbox::Sender::new(key, name, "claude"),
            &x.delivery_id,
            &format!("wound down; {name} left #1"),
        )
        .unwrap();
    }
    let st = r.w.wind_down_status();
    for (key, name) in [(&r.key_a, "alpha"), (&r.key_b, "beta")] {
        let x = st.rows.iter().find(|x| &x.key == key).unwrap();
        assert_eq!(x.state, row::DONE);
        assert_eq!(x.reply, format!("wound down; {name} left #1"));
    }
    assert!(
        ib.for_agent(&me, true, 0).is_empty(),
        "no message to the person"
    );
    assert_eq!(r.w.inbox_counts().unread, 0);
}

#[test]
fn a_straggler_stop_is_followed_and_kept_out_too() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    let me = r.w.core.user_sender().key;
    let st = r.w.wind_down_all().unwrap();
    let first = st
        .rows
        .iter()
        .find(|x| x.key == r.key_b)
        .unwrap()
        .delivery_id
        .clone();
    r.w.wind_down_stop(&r.key_b).unwrap();
    let st = r.w.wind_down_status();
    let x = st.rows.iter().find(|x| x.key == r.key_b).unwrap();
    assert_ne!(x.delivery_id, first, "the row follows the Stop");
    let d = ib.get(&x.delivery_id).unwrap();
    assert_eq!(d.mode, inbox::mode::STOP);
    assert!(inbox::is_wind_down(&d.message));
    ib.reply(
        &inbox::Sender::new(&r.key_b, "beta", "claude"),
        &x.delivery_id,
        "wound down",
    )
    .unwrap();
    let st = r.w.wind_down_status();
    let x = st.rows.iter().find(|x| x.key == r.key_b).unwrap();
    assert_eq!(
        (x.state.as_str(), x.reply.as_str()),
        (row::DONE, "wound down")
    );
    assert!(ib.for_agent(&me, true, 0).is_empty());
}

#[test]
fn on_holds_starts_at_once_and_off_cancels_and_lifts_it() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    assert!(!r.w.wind_down_status().on);
    let st = r.w.wind_down_all().unwrap();
    // From the first moment, before any agent has answered.
    assert!(st.on && st.since > 0, "{st:?}");
    assert_eq!(st.devices.len(), 1, "{st:?}");
    let p = r.w.core.agents_pause();
    assert!(p.on, "Pause agents is on here");
    assert!(irori_core::spawn_pause::by_wind_down(&p.by), "{p:?}");
    assert!(!st.finished);
    let ids: Vec<String> = st
        .rows
        .iter()
        .filter(|x| x.key == r.key_a || x.key == r.key_b)
        .map(|x| x.delivery_id.clone())
        .collect();
    assert_eq!(ids.len(), 2, "{st:?}");

    // Off mid-run: neither fake session took its wind-down yet.
    let st = r.w.wind_down_off().unwrap();
    assert!(!st.on && st.off && st.finished, "{st:?}");
    assert!(st.devices.is_empty());
    assert!(!r.w.core.agents_pause().on, "Off lifts the pause On set");
    for id in &ids {
        let x = st.rows.iter().find(|x| &x.delivery_id == id).unwrap();
        assert_eq!(x.state, row::CANCELED, "{x:?}");
        assert_eq!(ib.get(id).unwrap().state, inbox::st::SKIPPED);
    }
    assert!(ib.open_wind_downs().is_empty());
}

#[test]
fn off_leaves_a_device_that_was_paused_before() {
    let r = Rig::new();
    r.w.core.set_agents_paused(true).unwrap();
    let st = r.w.wind_down_all().unwrap();
    // This device's pause is the person's, not Wind down's.
    assert!(!st.on, "{st:?}");
    r.w.wind_down_off().unwrap();
    let p = r.w.core.agents_pause();
    assert!(p.on, "still paused, as before Wind down");
    assert!(!irori_core::spawn_pause::by_wind_down(&p.by));
}

#[test]
fn a_finished_run_keeps_the_switch_on() {
    let r = Rig::new();
    // Every row answered at once: the run finishes, the switch stays on.
    let st = r.w.wind_down_all().unwrap();
    for x in &st.rows {
        let name = if x.key == r.key_a { "alpha" } else { "beta" };
        r.w.core
            .inbox()
            .reply(&inbox::Sender::new(&x.key, name, "claude"), &x.delivery_id, "wound down")
            .unwrap();
    }
    let st = r.w.wind_down_status();
    assert!(st.finished && st.on, "{st:?}");
    assert!(r.w.core.agents_pause().on);
}

/// An agent whose session ended without answering: its wind-down closes
/// (the sweep, or Irori ending it), the row reads no answer and the run
/// finishes.
#[test]
fn a_row_whose_agent_ended_without_answering_reads_no_answer() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    let st = r.w.wind_down_all().unwrap();
    for x in &st.rows {
        if x.key == r.key_a {
            ib.reply(&inbox::Sender::new(&x.key, "alpha", "claude"), &x.delivery_id, "wound down")
                .unwrap();
        } else {
            // As the sweep and Irori's end close it.
            ib.skip(&x.key, &x.delivery_id, inbox::NOTE_RECIPIENT_GONE)
                .unwrap();
        }
    }
    let st = r.w.wind_down_status();
    let b = st.rows.iter().find(|x| x.key == r.key_b).unwrap();
    assert_eq!(b.state, row::NO_ANSWER);
    assert!(st.finished, "{st:?}");
}
