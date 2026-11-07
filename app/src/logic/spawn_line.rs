//! The line for a card on a spawning board that has no agent yet (the
//! React app's lib/spawn-line.ts, cards #448, #450, #541, #576): an agent
//! starting, its place in a device's queue, why one can't start, or what
//! holds the queue.

use gocompat::time::GoTime;
use irori_window::cards::CardSummary;

pub const NOT_TRUSTED: &str = "not trusted: ";

/// What the line reads off a card.
#[derive(Clone, Debug, Default)]
pub struct SpawnState {
    pub starting: String,
    pub at_cap: String,
    /// Its place in that device's queue, from 1 (0: none).
    pub queue_place: i64,
    /// While a usage limit holds spawns: when they start again.
    pub usage_limit: Option<GoTime>,
    pub cant_start: String,
    pub no_agent: String,
}

impl From<&CardSummary> for SpawnState {
    fn from(c: &CardSummary) -> SpawnState {
        SpawnState {
            starting: c.starting.clone(),
            at_cap: c.at_cap.clone(),
            queue_place: c.queue_place,
            usage_limit: c.usage_limit.filter(|t| !t.is_zero()),
            cant_start: c.cant_start.clone(),
            no_agent: c.no_agent.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnTone {
    Quiet,
    Warn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnLine {
    pub text: String,
    pub tone: SpawnTone,
}

/// 1st, 2nd, 3rd, 4th … 11th, 12th, 13th, 21st.
pub fn ordinal(n: i64) -> String {
    let teen = (11..=13).contains(&(n % 100));
    let end = if teen {
        "th"
    } else {
        match n % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    };
    format!("{n}{end}")
}

/// The card's spawn state in words, or `None` when it has none.
pub fn spawn_line(c: &SpawnState) -> Option<SpawnLine> {
    let line = |t: String, tone| Some(SpawnLine { text: t, tone });
    if !c.starting.is_empty() {
        return line(format!("starting an agent on {}", c.starting), SpawnTone::Quiet);
    }
    // Card #541: no computer can take it. Already a whole sentence.
    if !c.no_agent.is_empty() {
        return line(c.no_agent.clone(), SpawnTone::Warn);
    }
    if !c.cant_start.is_empty() {
        if c.cant_start.starts_with(NOT_TRUSTED) {
            return line("can't start: its repo isn't trusted here".into(), SpawnTone::Warn);
        }
        let why = c.cant_start.strip_prefix("couldn't start: ").unwrap_or(&c.cant_start);
        return line(format!("can't start: {why}"), SpawnTone::Warn);
    }
    if c.usage_limit.is_some() {
        return line("queued: usage limit".into(), SpawnTone::Quiet);
    }
    if !c.at_cap.is_empty() && c.queue_place > 0 {
        return line(
            format!("queued: {} for {}", ordinal(c.queue_place), c.at_cap),
            SpawnTone::Quiet,
        );
    }
    if !c.at_cap.is_empty() {
        return line(format!("waiting: {} is at its cap", c.at_cap), SpawnTone::Quiet);
    }
    None
}

/// What Start an agent does for a card a usage limit keeps waiting, for
/// its button's tooltip; "" when that isn't what it waits on.
pub fn start_past_limit(c: &SpawnState) -> &'static str {
    if !c.starting.is_empty() || !c.cant_start.is_empty() || c.usage_limit.is_none() {
        return "";
    }
    "Tries now instead of waiting for the reset. If the limit still stands, Irori holds again."
}

/// Whether a card waits in the spawn queue (a place or a usage limit).
pub fn queued(c: &SpawnState) -> bool {
    c.starting.is_empty()
        && c.cant_start.is_empty()
        && (!c.at_cap.is_empty() || c.usage_limit.is_some())
}

/// A clock time in this computer's zone: "4:30 PM".
pub fn clock(t: &GoTime) -> String {
    let secs = t.unix_milli_value().div_euclid(1000);
    let (_, _, _, h, mi, _) = t.with_offset(sys::tz::local_offset(secs)).civil();
    let (h12, ap) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("{h12}:{mi:02} {ap}")
}

/// When a usage limit's hold ends: "resets 4:30 PM".
pub fn usage_resets(at: &GoTime) -> String {
    if at.is_zero() {
        return String::new();
    }
    format!("resets {}", clock(at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(f: impl Fn(&mut SpawnState)) -> SpawnState {
        let mut s = SpawnState::default();
        f(&mut s);
        s
    }
    fn limit() -> Option<GoTime> {
        GoTime::parse_rfc3339("2026-11-04T23:30:00Z")
    }

    #[test]
    fn nothing_to_say() {
        assert_eq!(spawn_line(&SpawnState::default()), None);
    }

    #[test]
    fn starting_wins() {
        let s = st(|s| {
            s.starting = "Macbook Pro (this one)".into();
            s.at_cap = "Macbook Pro (this one)".into();
        });
        assert_eq!(spawn_line(&s).unwrap().text, "starting an agent on Macbook Pro (this one)");
    }

    #[test]
    fn cant_start() {
        assert_eq!(
            spawn_line(&st(|s| s.cant_start = "not trusted: /x on pc".into())),
            Some(SpawnLine {
                text: "can't start: its repo isn't trusted here".into(),
                tone: SpawnTone::Warn
            })
        );
        assert_eq!(
            spawn_line(&st(|s| s.cant_start = "couldn't start: claude not found".into())).unwrap().text,
            "can't start: claude not found"
        );
        assert_eq!(
            spawn_line(&st(|s| s.cant_start = "no copy of irori on Desktop Computer".into())).unwrap().text,
            "can't start: no copy of irori on Desktop Computer"
        );
    }

    #[test]
    fn queue_places_and_limits() {
        assert_eq!(
            spawn_line(&st(|s| {
                s.at_cap = "the PC (this one)".into();
                s.queue_place = 1;
            })),
            Some(SpawnLine {
                text: "queued: 1st for the PC (this one)".into(),
                tone: SpawnTone::Quiet
            })
        );
        assert_eq!(
            spawn_line(&st(|s| {
                s.at_cap = "the PC".into();
                s.queue_place = 4;
            }))
            .unwrap()
            .text,
            "queued: 4th for the PC"
        );
        assert_eq!(
            spawn_line(&st(|s| {
                s.at_cap = "the PC".into();
                s.queue_place = 2;
                s.usage_limit = limit();
            }))
            .unwrap()
            .text,
            "queued: usage limit"
        );
        assert!(usage_resets(&limit().unwrap()).starts_with("resets "));
        assert!(usage_resets(&limit().unwrap()).ends_with(":30 AM") || usage_resets(&limit().unwrap()).ends_with(":30 PM"));
        assert_eq!(usage_resets(&GoTime::default()), "");
        assert_eq!(
            spawn_line(&st(|s| s.at_cap = "Macbook Pro (this one)".into())),
            Some(SpawnLine {
                text: "waiting: Macbook Pro (this one) is at its cap".into(),
                tone: SpawnTone::Quiet
            })
        );
    }

    #[test]
    fn ordinals() {
        let got: Vec<String> = [1, 2, 3, 4, 11, 12, 13, 21, 22, 101, 111].iter().map(|n| ordinal(*n)).collect();
        assert_eq!(got, ["1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "101st", "111th"]);
    }

    #[test]
    fn queued_is_a_place_or_a_hold() {
        assert!(queued(&st(|s| {
            s.at_cap = "pc".into();
            s.queue_place = 1;
        })));
        assert!(queued(&st(|s| s.usage_limit = limit())));
        assert!(!queued(&st(|s| {
            s.starting = "pc".into();
            s.at_cap = "pc".into();
        })));
        assert!(!queued(&st(|s| {
            s.cant_start = "x".into();
            s.at_cap = "pc".into();
        })));
        assert!(!queued(&SpawnState::default()));
    }

    #[test]
    fn start_past_limit_only_for_a_limit() {
        assert_eq!(start_past_limit(&SpawnState::default()), "");
        assert_eq!(
            start_past_limit(&st(|s| {
                s.at_cap = "the PC".into();
                s.queue_place = 2;
            })),
            ""
        );
        assert!(start_past_limit(&st(|s| s.usage_limit = limit())).contains("Tries now"));
        assert_eq!(
            start_past_limit(&st(|s| {
                s.usage_limit = limit();
                s.starting = "the PC".into();
            })),
            ""
        );
        assert_eq!(
            start_past_limit(&st(|s| {
                s.usage_limit = limit();
                s.cant_start = "no copy".into();
            })),
            ""
        );
    }
}
