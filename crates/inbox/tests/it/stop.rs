//! The person's stop (card #207): a third delivery mode that jumps the wake
//! gate, gets through Pause all, and only the person may send.

use crate::common;

use common::*;
use inbox::{Policy, SendOptions, hold, mode, st};

fn stop_opts() -> SendOptions {
    SendOptions {
        mode: mode::STOP.into(),
        ..SendOptions::default()
    }
}

#[test]
fn stop_jumps_an_outstanding_wake() {
    let h = Harness::new();
    h.abc();
    // A wake delivered but not yet engaged gates the next one.
    let (_, first) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "first");
    let w1 = h.wake(KEY_B);
    h.ib.mark_delivered(KEY_B, &w1.token).unwrap();
    let (_, second) = h.send(&claude(KEY_A, "alpha"), &to("beta"), "second");
    h.no_wake(KEY_B);
    // The person's stop goes out anyway, alone, as a stop wake.
    let (m, d) =
        h.ib.send_with(&person(), &to("beta"), "stop and tell me why", stop_opts())
            .unwrap();
    assert!(m.stop && d.mode == mode::STOP);
    let w = h.wake(KEY_B);
    assert_eq!(w.mode, mode::STOP);
    assert_eq!(w.delivery_ids, vec![d.id.clone()]);
    assert!(
        w.prompt.starts_with("STOP from ") && w.prompt.contains("stop and tell me why"),
        "{}",
        w.prompt
    );
    assert!(
        w.prompt.contains(&w.token),
        "the token must be in the prompt"
    );
    // The gated message still waits for its own wake.
    h.want_state(&first.id, st::DELIVERED);
    h.want_state(&second.id, st::QUEUED);
}

#[test]
fn only_the_person_stops() {
    let h = Harness::new();
    h.abc();
    let err =
        h.ib.send_with(&claude(KEY_A, "alpha"), &to("beta"), "stop", stop_opts())
            .unwrap_err();
    assert!(err.to_string().contains("only the person"), "{err}");
}

#[test]
fn stop_gets_through_pause_all() {
    let h = Harness::new();
    h.abc();
    h.ib.set_policy(
        Policy {
            mode: String::new(),
            block: vec![],
            allow: vec![],
            paused: true,
        },
        "user",
    )
    .unwrap();
    let (_, held) = h.send(&person(), &to("beta"), "an ordinary message");
    assert_eq!(h.state(&held.id).hold, hold::PAUSED);
    let (_, d) =
        h.ib.send_with(&person(), &to("beta"), "stop", stop_opts())
            .unwrap();
    h.want_state(&d.id, st::QUEUED);
    let w = h.wake(KEY_B);
    assert_eq!(w.mode, mode::STOP);
    assert_eq!(w.delivery_ids, vec![d.id]);
    // The paused ordinary message stays held.
    h.no_wake(KEY_B);
}

#[test]
fn stop_wake_mode_survives_a_reopen() {
    let mut h = Harness::new();
    h.abc();
    let (_, d) =
        h.ib.send_with(&person(), &to("beta"), "stop", stop_opts())
            .unwrap();
    let w = h.wake(KEY_B);
    h.reopen();
    let (ow, _) = h.ib.outstanding_wake(KEY_B).unwrap();
    assert_eq!(ow.id, w.id);
    assert_eq!(ow.mode, mode::STOP);
    assert_eq!(h.state(&d.id).mode, mode::STOP);
}
