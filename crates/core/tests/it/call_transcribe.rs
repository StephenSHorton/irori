//! Card #656: "Transcribe what I say in calls" is on until the person
//! turns it off, survives a restart and reaches their other devices.

use crate::common;

use std::time::Duration;

use common::pair::paired;
use common::{Rig, options, wait_for};
use irori_core::{Core, CoreEvent, Topic};

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
