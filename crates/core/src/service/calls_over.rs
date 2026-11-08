//! When a huddle is over (card #1043, docs/design/call-agents.md §3).
//!
//! Today a huddle is live until an end mark says otherwise, so a call whose
//! last device crashed stays "live" for ever. `Live` (in `calls.rs`) is
//! per-device memory, filled only by `ring`/`join` frames and empty after a
//! restart, so "absent from `Live`" can't mean over: a device that just
//! started or woke reads every running huddle as absent. [`over`] is the one
//! answer every rule that says "after the call" uses, and [`Heard`] is its
//! own record, kept beside `Live` and never pruned by it.
//!
//! The decision is a pure function of its inputs, so each rule is tested
//! with a made-up clock.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

/// A live call nobody has said anything about for this long has gone quiet
/// (`calls.rs`'s `LIVE_FOR`).
pub(super) const QUIET_FOR: Duration = Duration::from_secs(50);
/// A huddle with a transcript line newer than this isn't over, whatever the
/// frames say.
pub(super) const LINE_FRESH: Duration = Duration::from_secs(2 * 60);
/// A huddle this device never heard live, whose newest row is older than
/// this, is over.
pub(super) const UNHEARD_QUIET: Duration = Duration::from_secs(10 * 60);
/// A tick later than this after the last one means the device slept or its
/// process was stopped: it can't vouch for what it didn't hear meanwhile.
const SLEEP_GAP: Duration = Duration::from_secs(30);
/// How often the heard record is looked after.
pub(super) const TICK_EVERY: Duration = Duration::from_secs(5);
/// A huddle not heard for this long is forgotten here.
const FORGET_AFTER: Duration = Duration::from_secs(6 * 60 * 60);

/// What [`over`] needs to know about one huddle.
#[derive(Debug, Clone, Copy)]
pub(super) struct OverInput {
    pub now: SystemTime,
    /// An end mark is in the rows.
    pub ended: bool,
    /// This device is in the call.
    pub in_it: bool,
    /// The last `ring` or `join` this device heard for it.
    pub heard: Option<SystemTime>,
    /// Since when this device's own network has been up without a break.
    pub online_since: SystemTime,
    /// The newest transcript line's time.
    pub last_line: Option<SystemTime>,
    /// The newest of the huddle's start mark, lines and assistant rows.
    pub newest_row: Option<SystemTime>,
}

fn since(now: SystemTime, then: SystemTime) -> Duration {
    now.duration_since(then).unwrap_or_default()
}

/// Whether the huddle is over, as this device can tell:
/// - **over** if an end mark is in the rows;
/// - **over** if this device heard it live, then heard nothing for
///   [`QUIET_FOR`] while its own network stayed up, and no line is newer
///   than [`LINE_FRESH`]. Another member device needn't be connected: in a
///   two-device space the survivor of a crash has nobody else to see;
/// - **over** if this device never heard it live (it started, woke, or
///   lost its network since) and the huddle's newest row is older than
///   [`UNHEARD_QUIET`];
/// - otherwise live. Not knowing counts as live, so the after-call steps
///   stay off.
pub(super) fn over(i: &OverInput) -> bool {
    if i.ended {
        return true;
    }
    if i.in_it {
        return false;
    }
    match i.heard {
        Some(h) if h >= i.online_since => {
            since(i.now, h) > QUIET_FOR && i.last_line.is_none_or(|l| since(i.now, l) > LINE_FRESH)
        }
        // Never heard, or heard before a gap this device can't vouch for.
        _ => i
            .newest_row
            .is_some_and(|t| since(i.now, t) > UNHEARD_QUIET),
    }
}

/// This device's record of the huddles it heard live, and of whether its own
/// network has stayed up.
#[derive(Debug)]
pub(super) struct Heard {
    heard: BTreeMap<String, SystemTime>,
    online_since: SystemTime,
    last_tick: SystemTime,
    /// A task is keeping the record (see `calls.rs`).
    pub ticking: bool,
}

impl Default for Heard {
    fn default() -> Heard {
        Heard::new(SystemTime::now())
    }
}

impl Heard {
    pub(super) fn new(now: SystemTime) -> Heard {
        Heard {
            heard: BTreeMap::new(),
            online_since: now,
            last_tick: now,
            ticking: false,
        }
    }

    /// A task begins keeping the record at `now`: this device's network
    /// counts as up without a break from here on.
    pub(super) fn start(&mut self, now: SystemTime) {
        self.ticking = true;
        self.online_since = now;
        self.last_tick = now;
    }

    /// A `ring` or `join` for `call_id` was heard at `now`.
    pub(super) fn hear(&mut self, call_id: &str, now: SystemTime) {
        self.heard.insert(call_id.to_string(), now);
    }

    pub(super) fn of(&self, call_id: &str) -> Option<SystemTime> {
        self.heard.get(call_id).copied()
    }

    pub(super) fn online_since(&self) -> SystemTime {
        self.online_since
    }

    /// One look at the network: `up` says whether this device's own is up.
    /// A network that was down, or a gap since the last look (the device
    /// slept), restarts "online since". Drops huddles long forgotten and
    /// says whether anything is left to look after.
    pub(super) fn tick(&mut self, now: SystemTime, up: bool) -> bool {
        if !up || since(now, self.last_tick) > SLEEP_GAP {
            self.online_since = now;
        }
        self.last_tick = now;
        self.heard.retain(|_, h| since(now, *h) < FORGET_AFTER);
        !self.heard.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 + s)
    }

    fn base(now: u64) -> OverInput {
        OverInput {
            now: t(now),
            ended: false,
            in_it: false,
            heard: None,
            online_since: t(0),
            last_line: None,
            newest_row: None,
        }
    }

    #[test]
    fn an_end_mark_is_over_whatever_else() {
        let mut i = base(10);
        i.ended = true;
        i.in_it = true;
        i.heard = Some(t(10));
        assert!(over(&i));
    }

    #[test]
    fn a_call_i_am_in_is_never_over() {
        let mut i = base(10_000);
        i.in_it = true;
        i.heard = Some(t(1));
        i.newest_row = Some(t(1));
        assert!(!over(&i));
    }

    #[test]
    fn a_crashed_call_is_over_once_it_went_quiet_with_my_network_up() {
        // Heard at t=100, online since t=0, last line at t=90.
        let mut i = base(100 + 49);
        i.heard = Some(t(100));
        i.last_line = Some(t(90));
        assert!(!over(&i), "still inside the quiet window");
        i.now = t(100 + 51);
        assert!(!over(&i), "a line within 2 minutes keeps it live");
        i.now = t(90 + 121);
        assert!(over(&i));
        // With no line at all, 50 s is enough.
        i.last_line = None;
        i.now = t(151);
        assert!(over(&i));
    }

    #[test]
    fn a_crash_is_seen_by_the_lone_survivor_of_a_two_device_space() {
        // The survivor heard the other's last heartbeat; nobody else is
        // connected, and its own network never dropped.
        let mut i = base(100 + 180);
        i.heard = Some(t(100));
        i.online_since = t(0);
        i.last_line = Some(t(95));
        assert!(over(&i));
    }

    #[test]
    fn a_network_that_dropped_since_the_last_beat_is_not_over() {
        // Heard at t=100, but this device's network came back at t=130: it
        // may have missed every beat since.
        let mut i = base(100 + 180);
        i.heard = Some(t(100));
        i.online_since = t(130);
        i.newest_row = Some(t(95));
        assert!(
            !over(&i),
            "falls to the unheard rule, which needs 10 minutes"
        );
        i.now = t(95 + 601);
        assert!(over(&i));
    }

    #[test]
    fn a_device_that_starts_mid_call_never_calls_it_over() {
        // Never heard it; the call's rows are recent.
        let mut i = base(1_000);
        i.heard = None;
        i.online_since = t(990);
        i.last_line = Some(t(950));
        i.newest_row = Some(t(950));
        assert!(!over(&i));
        // No row at all: not knowing counts as live.
        i.last_line = None;
        i.newest_row = None;
        assert!(!over(&i));
    }

    #[test]
    fn an_unheard_call_with_old_rows_is_over() {
        let mut i = base(10_000);
        i.newest_row = Some(t(10_000 - 601));
        assert!(over(&i));
        i.newest_row = Some(t(10_000 - 599));
        assert!(!over(&i));
    }

    #[test]
    fn a_device_that_wakes_mid_call_hears_it_again_before_it_could_call_it_over() {
        let mut h = Heard::new(t(0));
        h.hear("c", t(100));
        // Sleeps from t=105 to t=400, then ticks.
        assert!(h.tick(t(105), true));
        assert!(h.tick(t(400), true));
        assert_eq!(h.online_since(), t(400), "the gap restarts online-since");
        let mut i = base(400);
        i.heard = h.of("c");
        i.online_since = h.online_since();
        i.newest_row = Some(t(390));
        assert!(!over(&i), "its record can't vouch for the sleep");
        // The next heartbeat refreshes it.
        h.hear("c", t(410));
        i.now = t(420);
        i.heard = h.of("c");
        assert!(!over(&i));
        i.now = t(410 + 51);
        i.newest_row = Some(t(390));
        i.last_line = None;
        assert!(over(&i), "and it goes quiet for real afterwards");
    }

    #[test]
    fn the_network_going_down_restarts_online_since() {
        let mut h = Heard::new(t(0));
        h.hear("c", t(1));
        h.tick(t(5), true);
        assert_eq!(h.online_since(), t(0));
        h.tick(t(10), false);
        assert_eq!(h.online_since(), t(10));
        h.tick(t(15), true);
        assert_eq!(h.online_since(), t(10));
    }

    #[test]
    fn a_huddle_long_forgotten_leaves_the_record() {
        let mut h = Heard::new(t(0));
        h.hear("c", t(1));
        let mut now = 1;
        while now < 6 * 60 * 60 + 10 {
            now += 5;
            if !h.tick(t(now), true) {
                break;
            }
        }
        assert!(h.of("c").is_none());
    }
}
