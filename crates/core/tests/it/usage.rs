//! Claude usage left (card #599): the Core keeps the mod's reading across a
//! restart, and a reading is a Usage event.

use std::time::Duration;

use irori_core::usage::RateLimit;
use irori_core::{Core, CoreEvent, Topic};

use crate::common;

fn in_hours(h: u64) -> String {
    let t = std::time::SystemTime::now() + Duration::from_secs(h * 3600);
    gocompat::time::GoTime::from_system(t)
        .utc()
        .format_rfc3339()
}

fn rl(kind: &str, pct: f64, resets_at: Option<String>) -> RateLimit {
    RateLimit {
        kind: kind.into(),
        percent_used: pct,
        resets_at,
    }
}

#[test]
fn a_reading_outlives_a_restart() {
    let (d, c) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let core = Core::start(common::options(d.path(), c.path())).unwrap();
    assert_eq!(core.usage_limits(), Default::default());
    let feed = core.feed(&[Topic::Usage]);
    core.record_usage_limits(
        &[
            rl("five_hour", 23.5, Some(in_hours(3))),
            rl("seven_day", 41.2, Some(in_hours(100))),
        ],
        None,
    )
    .unwrap();
    assert_eq!(
        feed.queue().recv_timeout(Duration::from_secs(5)),
        Ok(Some(CoreEvent::Usage))
    );
    // A reading with no window left after the checks, and one too long to
    // be a reading, are no event and change nothing.
    let before = core.usage_limits();
    core.record_usage_limits(&[rl("bad kind", 1.0, None)], None)
        .unwrap();
    core.record_usage_limits(&[], None).unwrap();
    let many: Vec<_> = (0..=irori_core::usage::MAX_WINDOWS)
        .map(|i| rl(&format!("k{i}"), 1.0, None))
        .collect();
    assert!(core.record_usage_limits(&many, None).is_err());
    assert_eq!(core.usage_limits(), before);
    assert_eq!(
        feed.queue().recv_timeout(Duration::from_millis(200)),
        Err(())
    );
    drop(feed);
    core.close().unwrap();

    let core = Core::start(common::options(d.path(), c.path())).unwrap();
    let got = core.usage_limits();
    assert_eq!(got, before);
    let kinds: Vec<_> = got
        .windows
        .iter()
        .map(|w| (w.kind.as_str(), w.percent_used))
        .collect();
    assert_eq!(kinds, [("five_hour", 23.5), ("seven_day", 41.2)]);
    core.close().unwrap();
}

#[test]
fn switching_accounts_replaces_the_windows_and_survives_a_restart() {
    let (d, c) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let core = Core::start(common::options(d.path(), c.path())).unwrap();
    let both = |five, seven| {
        vec![
            rl("five_hour", five, Some(in_hours(3))),
            rl("seven_day", seven, Some(in_hours(100))),
        ]
    };
    core.record_usage_limits(&both(80.0, 60.0), Some("acct-a"))
        .unwrap();
    // The new account's lower figures show on its first reading.
    core.record_usage_limits(&[rl("five_hour", 4.0, Some(in_hours(2)))], Some("acct-b"))
        .unwrap();
    let got = core.usage_limits();
    assert_eq!(got.windows.len(), 1);
    assert_eq!(got.windows[0].percent_used, 4.0);
    assert_eq!(got.account.as_deref(), Some("acct-b"));
    core.close().unwrap();

    let core = Core::start(common::options(d.path(), c.path())).unwrap();
    assert_eq!(core.usage_limits(), got);
    core.close().unwrap();
}
