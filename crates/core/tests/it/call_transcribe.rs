//! Card #656: "Transcribe what I say in calls" is on until the person
//! turns it off, survives a restart and reaches their other devices.

use crate::common;

use std::time::Duration;

use common::pair::paired;
use common::{Rig, options, wait_for};
use irori_core::{Core, CoreEvent, Topic};
use store::kinds;

#[test]
fn transcribe_is_on_until_turned_off_and_survives_a_restart() {
    let r = Rig::new();
    assert!(r.core.call_transcribe(), "on with no row (decision 52)");

    let feed = r.core.feed(&[Topic::Call]);
    r.core.set_call_transcribe(false).unwrap();
    assert!(!r.core.call_transcribe());
    assert!(
        matches!(
            feed.recv_timeout(Duration::from_secs(5)),
            Ok(Some(CoreEvent::Call))
        ),
        "a call hears the switch"
    );

    r.core.close().unwrap();
    let again = Core::start(options(&r.data, &r.claude)).unwrap();
    assert!(!again.call_transcribe(), "off after a restart");
    again.set_call_transcribe(true).unwrap();
    assert!(again.call_transcribe());
    again.close().unwrap();
}

#[test]
fn transcribe_reaches_the_persons_other_device() {
    let (a, b) = paired();
    assert!(b.core.call_transcribe());
    let feed = b.core.feed(&[Topic::Call]);
    a.core.set_call_transcribe(false).unwrap();
    wait_for(45, "B hears A's switch", || !b.core.call_transcribe());
    assert!(
        std::iter::from_fn(|| feed.try_recv()).any(|e| matches!(e, CoreEvent::Call)),
        "B's call hears it too"
    );
    b.core.set_call_transcribe(true).unwrap();
    wait_for(45, "A hears B's switch", || a.core.call_transcribe());
    a.core.close().unwrap();
    b.core.close().unwrap();
}

#[test]
fn a_row_not_written_by_the_person_is_ignored() {
    let r = Rig::new();
    r.core
        .store()
        .append(
            "agent:x/y",
            kinds::TRANSCRIBE_SET,
            &serde_json::json!({"on": false, "by": "agent:x/y", "at": "2026-01-01T00:00:00Z"}),
        )
        .unwrap();
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        r.core.call_transcribe(),
        "an agent's row doesn't turn it off"
    );
    r.core.close().unwrap();
    let again = Core::start(options(&r.data, &r.claude)).unwrap();
    assert!(again.call_transcribe(), "nor after a restart");
    again.close().unwrap();
}

#[test]
fn devices_that_switched_while_apart_settle_on_the_later_switch() {
    let (a, b) = paired();
    // A turned it off at 12:00; B turned it on at 11:00 while apart, and
    // that row only reaches A now, after A's.
    let set = |d: &common::pair::Dev, on: bool, at: &str| {
        d.core
            .store()
            .append(
                "user",
                kinds::TRANSCRIBE_SET,
                &serde_json::json!({"on": on, "by": format!("user:{}", d.core.device_id()), "at": at}),
            )
            .unwrap();
    };
    set(&a, false, "2030-01-01T12:00:00Z");
    wait_for(45, "B hears A's switch", || !b.core.call_transcribe());
    set(&b, true, "2030-01-01T11:00:00Z");
    wait_for(45, "A holds B's row", || rows(&a) == 2);
    // Each device's log has the earlier switch last: log order would split them.
    for _ in 0..20 {
        assert!(!a.core.call_transcribe(), "A keeps the later switch");
        assert!(!b.core.call_transcribe(), "B keeps the later switch");
        std::thread::sleep(Duration::from_millis(100));
    }
    a.core.close().unwrap();
    b.core.close().unwrap();
}

fn rows(d: &common::pair::Dev) -> usize {
    d.core
        .store()
        .list(&store::Filter {
            kinds: vec![kinds::TRANSCRIBE_SET.into()],
            ..store::Filter::default()
        })
        .unwrap()
        .len()
}
