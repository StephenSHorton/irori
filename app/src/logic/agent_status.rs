//! An agent's roster status in words and a tone (the React app's
//! lib/agent-status.ts, card #489): the hover card, Settings, the sidebar,
//! Talk, faces and the card holder line all read it, so a `shell` agent
//! reads "running a job" everywhere.

use gocompat::time::GoTime;
use irori_window::cards::AgentRef;
use irori_window::roster::RosterAgent;

use super::plain::plain_line;

/// How the status text is coloured: working and job the live green, warn
/// amber, idle and muted grey.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Working,
    Job,
    Warn,
    Idle,
    Muted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusText {
    pub text: String,
    pub tone: Tone,
}

/// What [`agent_status`] reads off an agent: a card's [`AgentRef`] or a
/// roster row.
#[derive(Clone, Debug, Default)]
pub struct Who {
    pub status: String,
    /// `None` when the source doesn't say (a roster row).
    pub live: Option<bool>,
    pub offline: bool,
    pub device_state: String,
}

impl Who {
    pub fn status(s: &str) -> Who {
        Who {
            status: s.into(),
            ..Who::default()
        }
    }
}

impl From<&AgentRef> for Who {
    fn from(r: &AgentRef) -> Who {
        Who {
            status: r.status.clone(),
            live: Some(r.live),
            offline: r.offline,
            device_state: String::new(),
        }
    }
}

impl From<&RosterAgent> for Who {
    fn from(a: &RosterAgent) -> Who {
        Who {
            status: a.card.status.clone(),
            live: None,
            offline: false,
            device_state: a.card.device_state.clone(),
        }
    }
}

/// A Go time as ms since the epoch, `None` for the zero time.
pub fn ms(t: &GoTime) -> Option<i64> {
    (!t.is_zero()).then(|| t.unix_milli_value())
}

/// The clock now, in ms since the epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// How long ago an agent was asked for: "just now", "6 min", "2 h"
/// (lib/agents.ts runningSince).
pub fn running_since(at: i64, now: i64) -> String {
    let m = (now - at).div_euclid(60_000);
    if m < 1 {
        return "just now".into();
    }
    if m < 60 {
        return format!("{m} min");
    }
    format!("{} h", m / 60)
}

/// How an agent stands. `since` is when its status began (ms): a job older
/// than a minute reads "running a job · 12 min".
pub fn agent_status(a: &Who, since: Option<i64>, now: i64) -> StatusText {
    let say = |t: &str, tone| StatusText {
        text: t.to_string(),
        tone,
    };
    let s = a.status.as_str();
    // Another member's agent whose state this device doesn't know (#478).
    if s == "unknown" {
        return say("", Tone::Muted);
    }
    if s == "gone" {
        return say("gone", Tone::Muted);
    }
    if a.offline || a.device_state == "offline" {
        return say("device offline", Tone::Muted);
    }
    if a.live == Some(false) {
        return say("not running", Tone::Muted);
    }
    match s {
        "starting" => say("starting", Tone::Muted),
        "person" | "irori" => say("", Tone::Muted),
        "busy" | "working" => say("working", Tone::Working),
        "shell" => StatusText {
            text: format!("running a job{}", job_for(since, now)),
            tone: Tone::Job,
        },
        "unresponsive" => say("not answering", Tone::Warn),
        "idle" => say("idle", Tone::Idle),
        "waiting" => say("waiting on a permission prompt", Tone::Warn),
        _ => say("working", Tone::Working),
    }
}

/// " · 12 min" for a job begun at least a minute ago, else "".
fn job_for(since: Option<i64>, now: i64) -> String {
    match since {
        Some(at) if at > 0 && now - at >= 60_000 => format!(" · {}", running_since(at, now)),
        _ => String::new(),
    }
}

/// A device with no link of its own, only rows through another (#544).
pub fn through_line(via: &str) -> String {
    format!("through {}", plain_line(via))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_793_793_600_000; // 2026-11-04T12:00:00Z

    fn say(status: &str) -> StatusText {
        agent_status(&Who::status(status), None, NOW)
    }

    #[test]
    fn each_status() {
        let t = |s: &str| (say(s).text, say(s).tone);
        assert_eq!(t("busy"), ("working".into(), Tone::Working));
        assert_eq!(t("working"), ("working".into(), Tone::Working));
        assert_eq!(t("shell"), ("running a job".into(), Tone::Job));
        assert_eq!(t("unresponsive"), ("not answering".into(), Tone::Warn));
        assert_eq!(t("idle"), ("idle".into(), Tone::Idle));
        assert_eq!(
            t("waiting"),
            ("waiting on a permission prompt".into(), Tone::Warn)
        );
        assert_eq!(t("gone"), ("gone".into(), Tone::Muted));
        assert_eq!(t("starting"), ("starting".into(), Tone::Muted));
        assert_eq!(t("person"), ("".into(), Tone::Muted));
        assert_eq!(t("irori"), ("".into(), Tone::Muted));
        assert_eq!(t("compacting"), ("working".into(), Tone::Working));
    }

    #[test]
    fn offline_and_not_running_come_before_the_status() {
        let w = |status: &str, f: &dyn Fn(&mut Who)| {
            let mut x = Who::status(status);
            f(&mut x);
            agent_status(&x, None, NOW).text
        };
        assert_eq!(w("busy", &|x| x.offline = true), "device offline");
        assert_eq!(
            w("idle", &|x| x.device_state = "offline".into()),
            "device offline"
        );
        assert_eq!(w("busy", &|x| x.live = Some(false)), "not running");
        assert_eq!(w("gone", &|x| x.offline = true), "gone");
        assert_eq!(
            w("idle", &|x| {
                x.live = Some(true);
                x.device_state = "online".into()
            }),
            "idle"
        );
        // Card #544: a relayed device doesn't read offline.
        assert_eq!(w("busy", &|x| x.device_state = "recent".into()), "working");
    }

    #[test]
    fn a_job_says_how_long_from_a_minute_on() {
        let job = |since: Option<i64>| agent_status(&Who::status("shell"), since, NOW).text;
        assert_eq!(job(Some(NOW - 12 * 60_000)), "running a job · 12 min");
        assert_eq!(job(Some(NOW - 2 * 3_600_000)), "running a job · 2 h");
        assert_eq!(job(Some(NOW - 30_000)), "running a job");
        assert_eq!(job(None), "running a job");
        assert_eq!(job(Some(0)), "running a job");
        assert_eq!(
            agent_status(&Who::status("busy"), Some(NOW - 12 * 60_000), NOW).text,
            "working"
        );
        assert_eq!(ms(&GoTime::default()), None);
    }

    #[test]
    fn through_line_is_one_plain_line() {
        assert_eq!(through_line("Phone"), "through Phone");
        assert!(!through_line("Pho\nne").contains('\n'));
    }
}
