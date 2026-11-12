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
    let st = r.w.wind_down_all(None).unwrap();
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
    let st = r.w.wind_down_all(None).unwrap();
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
    let st = r.w.wind_down_all(None).unwrap();
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
    let st = r.w.wind_down_all(None).unwrap();
    // Every device was paused already: the switch still reads on.
    assert!(st.on && st.since > 0, "{st:?}");
    let p = r.w.core.agents_pause();
    assert!(
        !irori_core::spawn_pause::by_wind_down(&p.by),
        "kept as it was"
    );
    let st = r.w.wind_down_off().unwrap();
    assert!(!st.on, "{st:?}");
    let p = r.w.core.agents_pause();
    assert!(p.on, "still paused, as before Stop everywhere");
    assert!(!irori_core::spawn_pause::by_wind_down(&p.by));
}

/// Start here while Stop everywhere is on: this device's pause goes, but
/// Stop everywhere still holds it, so its wind-downs stay and nothing
/// starts until Start everywhere.
#[test]
fn start_here_during_stop_everywhere_keeps_it_stopped() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    let me = r.w.core.device_id().to_string();
    let st = r.w.wind_down_all(None).unwrap();
    let ids: Vec<String> = st.rows.iter().map(|x| x.delivery_id.clone()).collect();
    assert!(!ids.is_empty(), "{st:?}");
    r.w.agents_pause_set(false, None).unwrap();
    assert!(!r.w.core.agents_pause().on, "its own pause went");
    assert!(
        r.w.core.agents_paused_on(&me),
        "Stop everywhere still holds it"
    );
    let st = r.w.wind_down_status();
    assert!(st.on && !st.off, "{st:?}");
    for id in &ids {
        assert_ne!(ib.get(id).unwrap().state, inbox::st::SKIPPED, "{id}");
    }
    r.w.wind_down_off().unwrap();
    assert!(!r.w.core.agents_paused_on(&me));
}

/// Start everywhere leaves this machine's Stop here run as it is: it goes
/// on, and only Start here ends it.
#[test]
fn start_everywhere_leaves_a_stop_here_run() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    r.w.agents_pause_set(true, None).unwrap();
    let ids: Vec<String> =
        r.w.wind_down_status()
            .rows
            .iter()
            .map(|x| x.delivery_id.clone())
            .collect();
    assert!(!ids.is_empty());
    let st = r.w.wind_down_off().unwrap();
    assert_eq!(st.scope, irori_window::wind_down::SCOPE_DEVICE);
    assert!(!st.off, "{st:?}");
    assert!(r.w.core.agents_pause().on);
    for id in &ids {
        assert_ne!(ib.get(id).unwrap().state, inbox::st::SKIPPED, "{id}");
    }
}

#[test]
fn a_finished_run_keeps_the_switch_on() {
    let r = Rig::new();
    // Every row answered at once: the run finishes, the switch stays on.
    let st = r.w.wind_down_all(None).unwrap();
    for x in &st.rows {
        let name = if x.key == r.key_a { "alpha" } else { "beta" };
        r.w.core
            .inbox()
            .reply(
                &inbox::Sender::new(&x.key, name, "claude"),
                &x.delivery_id,
                "wound down",
            )
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
    let st = r.w.wind_down_all(None).unwrap();
    for x in &st.rows {
        if x.key == r.key_a {
            ib.reply(
                &inbox::Sender::new(&x.key, "alpha", "claude"),
                &x.delivery_id,
                "wound down",
            )
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

/// Pause on this machine (card #787) is the same graceful stop for one
/// device: it holds starts here and winds this device's agents down; off
/// lifts it and closes what hasn't reached an agent.
#[test]
fn pause_here_winds_this_devices_agents_down_and_off_resumes() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    let v = r.w.agents_pause_set(true, None).unwrap();
    assert!(v.on);
    let st = r.w.wind_down_status();
    assert_eq!(st.scope, irori_window::wind_down::SCOPE_DEVICE, "{st:?}");
    assert!(!st.on, "Pause here isn't Pause all");
    let ids: Vec<String> = st
        .rows
        .iter()
        .filter(|x| x.key == r.key_a || x.key == r.key_b)
        .map(|x| x.delivery_id.clone())
        .collect();
    assert_eq!(ids.len(), 2, "{st:?}");
    for id in &ids {
        assert!(inbox::is_wind_down(&ib.get(id).unwrap().message));
    }
    let v = r.w.agents_pause_set(false, None).unwrap();
    assert!(!v.on);
    let st = r.w.wind_down_status();
    assert!(st.off && st.finished, "{st:?}");
    for id in &ids {
        assert_eq!(ib.get(id).unwrap().state, inbox::st::SKIPPED);
    }
}

/// Off of Pause all leaves a wind-down going to a device still paused by
/// its own Pause (its graceful stop goes on).
#[test]
fn unpause_all_keeps_a_paused_devices_wind_downs() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    r.w.agents_pause_set(true, None).unwrap();
    let ids: Vec<String> =
        r.w.wind_down_status()
            .rows
            .iter()
            .map(|x| x.delivery_id.clone())
            .collect();
    let st = r.w.wind_down_all(None).unwrap();
    let again: Vec<String> = st.rows.iter().map(|x| x.delivery_id.clone()).collect();
    assert_eq!(again, ids, "no second wind-down to an agent winding down");
    r.w.wind_down_off().unwrap();
    assert!(r.w.core.agents_pause().on, "this device's own Pause stays");
    for id in &ids {
        assert_ne!(ib.get(id).unwrap().state, inbox::st::SKIPPED, "{id}");
    }
}

// Stop's who (decision 71): On cards are the agents Irori started (the
// spawned mark), Yours the person's terminal sessions.

use irori_window::wind_down::{SCOPE_ALL, SCOPE_DEVICE, StopWho};

const CARDS: Option<StopWho> = Some(StopWho {
    cards: true,
    yours: false,
});
const YOURS: Option<StopWho> = Some(StopWho {
    cards: false,
    yours: true,
});

/// Marks alpha (key_a) as started by Irori; beta stays the person's.
fn mark_a_spawned(r: &Rig) {
    let me = r.w.core.device_id().to_string();
    r.w.core
        .coord()
        .mark_spawned_elsewhere(&r.key_a, "t_stopwho", 1, &me, "stopwho", false)
        .unwrap();
    assert!(r.w.core.agent_spawned(&r.key_a));
    assert!(!r.w.core.agent_spawned(&r.key_b));
}

/// The run's rows for alpha and beta, by key.
fn keys(r: &Rig, st: &irori_window::wind_down::WindDownStatus) -> Vec<String> {
    st.rows
        .iter()
        .filter(|x| x.key == r.key_a || x.key == r.key_b)
        .map(|x| x.key.clone())
        .collect()
}

#[test]
fn stop_everywhere_on_cards_only_reaches_irori_started_agents() {
    let r = Rig::new();
    mark_a_spawned(&r);
    let st = r.w.wind_down_all(CARDS).unwrap();
    assert_eq!(keys(&r, &st), vec![r.key_a.clone()], "{st:?}");
    assert!(st.rows[0].spawned);
    assert!(st.on && st.who.cards && !st.who.yours, "{st:?}");
    let me = r.w.core.device_id().to_string();
    assert!(r.w.core.agents_paused_on(&me), "starts held everywhere");
    assert!(
        r.w.core
            .inbox()
            .open_wind_downs()
            .iter()
            .all(|d| d.to != r.key_b),
        "beta untouched"
    );
}

#[test]
fn stop_everywhere_yours_only_holds_nothing_and_lights_no_switch() {
    let r = Rig::new();
    mark_a_spawned(&r);
    let st = r.w.wind_down_all(YOURS).unwrap();
    assert_eq!(keys(&r, &st), vec![r.key_b.clone()], "{st:?}");
    assert!(!st.rows[0].spawned);
    assert!(!st.on, "no switch: {st:?}");
    assert_eq!(st.scope, SCOPE_ALL);
    assert!(st.who.yours_only());
    assert!(r.w.core.agents_pauses().is_empty(), "no paused:* rows");
    let me = r.w.core.device_id().to_string();
    assert!(!r.w.core.agents_paused_on(&me));
    let d = r.w.core.inbox().get(&st.rows[0].delivery_id).unwrap();
    assert!(inbox::is_wind_down(&d.message));
}

#[test]
fn stop_everywhere_both_is_todays_stop() {
    let r = Rig::new();
    mark_a_spawned(&r);
    let st = r.w.wind_down_all(None).unwrap();
    let mut k = keys(&r, &st);
    k.sort();
    let mut want = vec![r.key_a.clone(), r.key_b.clone()];
    want.sort();
    assert_eq!(k, want, "{st:?}");
    assert!(st.on && st.who == StopWho::BOTH);
}

#[test]
fn stop_here_takes_each_choice() {
    // On cards: holds starts here, winds alpha down only.
    let r = Rig::new();
    mark_a_spawned(&r);
    let v = r.w.agents_pause_set(true, CARDS).unwrap();
    assert!(v.on);
    let st = r.w.wind_down_status();
    assert_eq!(st.scope, SCOPE_DEVICE);
    assert_eq!(keys(&r, &st), vec![r.key_a.clone()], "{st:?}");
    drop(r);

    // Yours: holds nothing, winds beta down, and runs again when asked.
    let r = Rig::new();
    mark_a_spawned(&r);
    let v = r.w.agents_pause_set(true, YOURS).unwrap();
    assert!(!v.on, "Yours lights no switch");
    let st = r.w.wind_down_status();
    assert_eq!(st.scope, SCOPE_DEVICE);
    assert!(st.who.yours_only());
    assert_eq!(keys(&r, &st), vec![r.key_b.clone()], "{st:?}");
    let first = st.id.clone();
    std::thread::sleep(std::time::Duration::from_millis(5));
    r.w.agents_pause_set(true, YOURS).unwrap();
    assert_ne!(r.w.wind_down_status().id, first, "a fresh run each time");
    drop(r);

    // Both: today's Stop here, both agents.
    let r = Rig::new();
    mark_a_spawned(&r);
    assert!(r.w.agents_pause_set(true, None).unwrap().on);
    assert_eq!(keys(&r, &r.w.wind_down_status()).len(), 2);
}

/// Start here and Start everywhere lift holds; a Yours run held none, so
/// its wind-downs not reached yet stay.
#[test]
fn a_yours_run_survives_start_here_and_start_everywhere() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    mark_a_spawned(&r);
    // A held Stop here first, so Start here has something to lift.
    r.w.agents_pause_set(true, CARDS).unwrap();
    let st = r.w.wind_down_all(YOURS).unwrap();
    let id = st.rows[0].delivery_id.clone();
    assert_eq!(st.rows[0].key, r.key_b);
    r.w.agents_pause_set(false, None).unwrap();
    assert_ne!(ib.get(&id).unwrap().state, inbox::st::SKIPPED, "Start here");
    r.w.wind_down_all(CARDS).unwrap();
    // That replaced the run in memory: Start everywhere has only the
    // delivery's own words to go by.
    r.w.wind_down_off().unwrap();
    assert_ne!(
        ib.get(&id).unwrap().state,
        inbox::st::SKIPPED,
        "Start everywhere"
    );
    let st = r.w.wind_down_status();
    assert!(!st.on, "{st:?}");
}

/// Call off closes only the Yours run's rows not reached yet, and lifts
/// nothing.
#[test]
fn call_off_closes_only_that_runs_unreached_rows() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    mark_a_spawned(&r);
    // alpha's held Stop here run, then beta's Yours run.
    r.w.agents_pause_set(true, CARDS).unwrap();
    let a_id = r.w.wind_down_status().rows[0].delivery_id.clone();
    let st = r.w.wind_down_all(YOURS).unwrap();
    let b_id = st.rows[0].delivery_id.clone();
    let st = r.w.wind_down_call_off().unwrap();
    assert!(st.off && st.finished, "{st:?}");
    assert_eq!(st.rows[0].state, row::CANCELED);
    assert_eq!(ib.get(&b_id).unwrap().state, inbox::st::SKIPPED);
    assert_ne!(ib.get(&a_id).unwrap().state, inbox::st::SKIPPED, "alpha's");
    assert!(r.w.core.agents_pause().on, "Call off lifts nothing");
}

/// Start ends a held run, never Call off; Call off on it does nothing.
#[test]
fn call_off_leaves_a_held_run() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    let st = r.w.wind_down_all(None).unwrap();
    let st2 = r.w.wind_down_call_off().unwrap();
    assert!(st2.on && !st2.off, "{st2:?}");
    for x in &st.rows {
        assert_ne!(ib.get(&x.delivery_id).unwrap().state, inbox::st::SKIPPED);
    }
}

/// A Yours run's straggler Stop keeps the Yours words, so Start leaves it.
#[test]
fn a_yours_straggler_stop_still_survives_start() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    mark_a_spawned(&r);
    r.w.wind_down_all(YOURS).unwrap();
    r.w.wind_down_stop(&r.key_b).unwrap();
    let id = r.w.wind_down_status().rows[0].delivery_id.clone();
    assert!(irori_window::wind_down::is_yours_only_text(
        &ib.get(&id).unwrap().message.text
    ));
    r.w.wind_down_off().unwrap();
    assert_ne!(ib.get(&id).unwrap().state, inbox::st::SKIPPED);
}

/// A held Stop that took over a Yours run's ask (the agent was still to
/// be reached) closes it with its own Start, as before the choice.
#[test]
fn a_held_run_that_took_over_a_yours_ask_closes_it_on_start() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    mark_a_spawned(&r);
    let st = r.w.wind_down_all(YOURS).unwrap();
    let id = st.rows[0].delivery_id.clone();
    let st = r.w.wind_down_all(None).unwrap();
    let b = st.rows.iter().find(|x| x.key == r.key_b).unwrap();
    assert_eq!(b.delivery_id, id, "the Both run follows beta's ask");
    r.w.wind_down_off().unwrap();
    assert_eq!(ib.get(&id).unwrap().state, inbox::st::SKIPPED);
}

/// Call off closes only its own asks: one it took over from a held run
/// stays with that run.
#[test]
fn call_off_leaves_an_ask_it_took_over_from_a_held_run() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    mark_a_spawned(&r);
    let st = r.w.wind_down_all(None).unwrap();
    let id = st
        .rows
        .iter()
        .find(|x| x.key == r.key_b)
        .unwrap()
        .delivery_id
        .clone();
    let st = r.w.wind_down_all(YOURS).unwrap();
    assert_eq!(st.rows[0].delivery_id, id);
    r.w.wind_down_call_off().unwrap();
    assert_ne!(ib.get(&id).unwrap().state, inbox::st::SKIPPED);
    assert!(r.w.wind_down_status().on, "the hold stays");
}

/// A later Stop that replaces a Yours run leaves its asks not reached yet
/// to Call off: the Cards run's Start leaves them (decision 71), and Call
/// off still closes them though the run in memory is the Cards one.
#[test]
fn call_off_reaches_a_yours_ask_a_later_stop_replaced() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    mark_a_spawned(&r);
    let st = r.w.wind_down_all(YOURS).unwrap();
    let id = st.rows[0].delivery_id.clone();
    assert_eq!(st.yours_waiting, 1, "{st:?}");
    let st = r.w.wind_down_all(CARDS).unwrap();
    assert!(st.rows.iter().all(|x| x.delivery_id != id), "{st:?}");
    assert_eq!(st.yours_waiting, 1, "still offered: {st:?}");
    // Start everywhere leaves it, as before.
    r.w.wind_down_off().unwrap();
    assert_ne!(ib.get(&id).unwrap().state, inbox::st::SKIPPED);
    let st = r.w.wind_down_call_off().unwrap();
    assert_eq!(ib.get(&id).unwrap().state, inbox::st::SKIPPED);
    assert_eq!(st.yours_waiting, 0, "{st:?}");
}

/// While the replacing Cards run is still on, Call off closes the Yours
/// ask it left and leaves the Cards run's own rows and its hold.
#[test]
fn call_off_during_a_cards_run_leaves_its_rows_and_hold() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    mark_a_spawned(&r);
    let b_id = r.w.wind_down_all(YOURS).unwrap().rows[0]
        .delivery_id
        .clone();
    let st = r.w.wind_down_all(CARDS).unwrap();
    let a_id = st.rows[0].delivery_id.clone();
    let st = r.w.wind_down_call_off().unwrap();
    assert_eq!(ib.get(&b_id).unwrap().state, inbox::st::SKIPPED);
    assert_ne!(ib.get(&a_id).unwrap().state, inbox::st::SKIPPED);
    assert!(st.on && !st.off, "the Cards run goes on: {st:?}");
}

/// The run is memory and the asks are deliveries: after a restart (a new
/// Window on the same data) Call off is still offered and still closes
/// the Yours asks not reached yet.
#[test]
fn call_off_reaches_yours_asks_after_a_restart() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    mark_a_spawned(&r);
    let id = r.w.wind_down_all(YOURS).unwrap().rows[0]
        .delivery_id
        .clone();
    let w2 = irori_window::Window::new(
        r.core.clone(),
        irori_window::Window::default_seams(&r.core),
        irori_window::Config {
            claude_dir: r._dirs[1].path().to_path_buf(),
            downloads: Box::new(|| Ok(std::env::temp_dir())),
        },
    );
    let st = w2.wind_down_status();
    assert!(st.id.is_empty(), "no run in memory: {st:?}");
    assert_eq!(st.yours_waiting, 1, "{st:?}");
    // Start everywhere after the restart still leaves it.
    w2.wind_down_off().unwrap();
    assert_ne!(ib.get(&id).unwrap().state, inbox::st::SKIPPED);
    let st = w2.wind_down_call_off().unwrap();
    assert_eq!(ib.get(&id).unwrap().state, inbox::st::SKIPPED);
    assert_eq!(st.yours_waiting, 0);
}

/// A Yours ask a held run took over is that run's: Call off leaves it.
#[test]
fn call_off_leaves_a_yours_ask_a_held_run_took_over() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    mark_a_spawned(&r);
    let id = r.w.wind_down_all(YOURS).unwrap().rows[0]
        .delivery_id
        .clone();
    let st = r.w.wind_down_all(None).unwrap();
    assert_eq!(st.yours_waiting, 0, "{st:?}");
    r.w.wind_down_call_off().unwrap();
    assert_ne!(ib.get(&id).unwrap().state, inbox::st::SKIPPED);
}
