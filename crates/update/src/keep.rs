//! Keep this device up to date (card #578, docs/design/no-server.md §4.3):
//! what the app does on each tick of its timer. The app gathers the facts
//! (the setting, the updater's status, what stops a restart) and does what
//! [`Keep::tick`] says, so the rules here need no app, clock or network.
//!
//! - Every [`Timing::check_every`] it checks and downloads.
//! - With an update ready, it restarts once nothing has stopped it on two
//!   ticks in a row.
//! - The Install card waits [`Timing::card_after`], then shows as it does
//!   without the setting. The restart can still happen after that.
//! - A restart that didn't bring the new version up is never tried again
//!   for that version ([`Attempt`]): the card asks instead, so an installer
//!   that gives up can't become a restart every few minutes.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::{PHASE_ERROR, PHASE_IDLE, PHASE_READY, version};

/// The file that remembers the version a restart went for. Beside
/// `updates/`, whose files a download deletes.
pub const ATTEMPT_FILE: &str = "update-attempt";

/// How often things happen. The app's defaults are [`Timing::default`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// Between ticks.
    pub tick: Duration,
    /// Between checks for a release.
    pub check_every: Duration,
    /// How long a ready update waits before the Install card shows.
    pub card_after: Duration,
}

impl Default for Timing {
    fn default() -> Timing {
        Timing {
            tick: Duration::from_secs(60),
            check_every: Duration::from_secs(6 * 60 * 60),
            card_after: Duration::from_secs(24 * 60 * 60),
        }
    }
}

/// Whether the setting is on: the person's choice, else whether this device
/// is the home server. Never write the result back, or a device that only
/// followed the home switch stops following it.
pub fn resolve(explicit: Option<bool>, home_on: bool) -> bool {
    explicit.unwrap_or(home_on)
}

/// What the timer sees on one tick.
#[derive(Debug, Clone)]
pub struct Facts<'a> {
    /// The setting, resolved.
    pub on: bool,
    /// The updater's phase.
    pub phase: &'a str,
    /// The version it has ready, at [`PHASE_READY`].
    pub version: &'a str,
    /// Whether anything stops a restart now.
    pub blocked: bool,
    pub now: SystemTime,
}

/// What to do on a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Nothing,
    /// Check for a release and download it.
    Check,
    /// Make the last check, then restart hidden.
    Restart,
}

/// The timer's memory, for one run of the app.
#[derive(Debug)]
pub struct Keep {
    timing: Timing,
    last_check: SystemTime,
    /// The version at ready, and since when.
    ready: Option<(String, SystemTime)>,
    /// Ticks in a row with nothing stopping a restart.
    clear: u32,
    /// The version a restart already failed to bring up.
    gave_up: Option<String>,
}

impl Keep {
    /// `started` counts as a check: the app checks at start anyway.
    pub fn new(timing: Timing, started: SystemTime, gave_up: Option<String>) -> Keep {
        Keep {
            timing,
            last_check: started,
            ready: None,
            clear: 0,
            gave_up,
        }
    }

    pub fn tick(&mut self, f: &Facts<'_>) -> Act {
        if f.phase == PHASE_READY {
            match &self.ready {
                Some((v, _)) if v == f.version => {}
                _ => self.ready = Some((f.version.to_string(), f.now)),
            }
        } else {
            self.ready = None;
        }
        self.clear = if f.blocked || f.phase != PHASE_READY {
            0
        } else {
            self.clear + 1
        };
        if !f.on {
            self.clear = 0;
            return Act::Nothing;
        }
        if f.phase == PHASE_READY {
            if self.clear >= 2 && self.gave_up.as_deref() != Some(f.version) {
                return Act::Restart;
            }
            return Act::Nothing;
        }
        // A clock set back must not stop the checks for good.
        let due = match f.now.duration_since(self.last_check) {
            Ok(d) => d >= self.timing.check_every,
            Err(_) => true,
        };
        if due && (f.phase == PHASE_IDLE || f.phase == PHASE_ERROR) {
            self.last_check = f.now;
            return Act::Check;
        }
        Act::Nothing
    }

    /// The last check stopped the restart, or it failed: start over.
    pub fn restart_stopped(&mut self) {
        self.clear = 0;
    }

    /// The restart for `version` couldn't start: only the card is left.
    pub fn give_up(&mut self, version: &str) {
        self.gave_up = Some(version.to_string());
    }

    /// Whether the Install card for `version` waits: the setting is on, the
    /// day isn't up, and a restart may still install it.
    pub fn holds_card(&self, on: bool, version: &str, now: SystemTime) -> bool {
        if !on || self.gave_up.as_deref() == Some(version) {
            return false;
        }
        match &self.ready {
            Some((v, since)) if v == version => match now.duration_since(*since) {
                Ok(d) => d < self.timing.card_after,
                Err(_) => true,
            },
            // Ready before the first tick saw it: the day starts then.
            _ => true,
        }
    }
}

/// The version a restart went for, kept across it.
#[derive(Debug, Clone)]
pub struct Attempt {
    path: PathBuf,
}

impl Attempt {
    /// In the data dir.
    pub fn new(data: &Path) -> Attempt {
        Attempt {
            path: data.join(ATTEMPT_FILE),
        }
    }

    /// Writes it down before a restart. Failing here stops the restart: an
    /// attempt that can't be remembered could repeat for ever.
    pub fn record(&self, version: &str) -> Result<(), String> {
        std::fs::write(&self.path, version).map_err(|e| format!("{}: {e}", self.path.display()))
    }

    /// At start: the version a restart failed to bring up, if `running` is
    /// still older than it. An attempt that worked is forgotten.
    pub fn settle(&self, running: &str) -> Option<String> {
        let tried = std::fs::read_to_string(&self.path).ok()?;
        let tried = tried.trim().to_string();
        if !tried.is_empty() && version::is_newer(&tried, running) {
            return Some(tried);
        }
        let _ = std::fs::remove_file(&self.path);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PHASE_CHECKING, PHASE_DOWNLOADING};

    const HOUR: Duration = Duration::from_secs(3600);

    fn t0() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000)
    }

    fn keep() -> Keep {
        Keep::new(Timing::default(), t0(), None)
    }

    fn facts<'a>(phase: &'a str, version: &'a str, blocked: bool, now: SystemTime) -> Facts<'a> {
        Facts {
            on: true,
            phase,
            version,
            blocked,
            now,
        }
    }

    #[test]
    fn explicit_beats_the_home_switch() {
        assert!(resolve(None, true));
        assert!(!resolve(None, false));
        assert!(resolve(Some(true), false));
        assert!(!resolve(Some(false), true));
    }

    #[test]
    fn checks_every_few_hours_not_before() {
        let mut k = keep();
        assert_eq!(k.tick(&facts(PHASE_IDLE, "", false, t0() + HOUR)), Act::Nothing);
        let due = t0() + 6 * HOUR;
        assert_eq!(k.tick(&facts(PHASE_IDLE, "", false, due)), Act::Check);
        assert_eq!(k.tick(&facts(PHASE_IDLE, "", false, due)), Act::Nothing);
        let next = due + 6 * HOUR;
        assert_eq!(k.tick(&facts(PHASE_ERROR, "", false, next)), Act::Check);
    }

    #[test]
    fn a_run_in_progress_is_left_alone() {
        let mut k = keep();
        let due = t0() + 7 * HOUR;
        for p in [PHASE_CHECKING, PHASE_DOWNLOADING] {
            assert_eq!(k.tick(&facts(p, "", false, due)), Act::Nothing);
        }
    }

    #[test]
    fn off_never_checks_or_restarts() {
        let mut k = keep();
        let mut f = facts(PHASE_IDLE, "", false, t0() + 7 * HOUR);
        f.on = false;
        assert_eq!(k.tick(&f), Act::Nothing);
        let mut f = facts(PHASE_READY, "1.1.0", false, t0());
        f.on = false;
        for _ in 0..3 {
            assert_eq!(k.tick(&f), Act::Nothing);
        }
        // Turning it on starts the count afresh, and the skipped check runs.
        let f = facts(PHASE_READY, "1.1.0", false, t0());
        assert_eq!(k.tick(&f), Act::Nothing);
        assert_eq!(k.tick(&f), Act::Restart);
    }

    #[test]
    fn restarts_after_two_clear_ticks_only() {
        let mut k = keep();
        let ready = |b| facts(PHASE_READY, "1.1.0", b, t0());
        assert_eq!(k.tick(&ready(false)), Act::Nothing);
        assert_eq!(k.tick(&ready(true)), Act::Nothing, "a turn started");
        assert_eq!(k.tick(&ready(false)), Act::Nothing, "the count starts over");
        assert_eq!(k.tick(&ready(false)), Act::Restart);
    }

    #[test]
    fn a_busy_device_never_restarts() {
        let mut k = keep();
        for h in 0..100 {
            let f = facts(PHASE_READY, "1.1.0", true, t0() + h * HOUR);
            assert_eq!(k.tick(&f), Act::Nothing);
        }
    }

    #[test]
    fn a_stopped_restart_starts_the_count_over() {
        let mut k = keep();
        let f = facts(PHASE_READY, "1.1.0", false, t0());
        k.tick(&f);
        assert_eq!(k.tick(&f), Act::Restart);
        k.restart_stopped();
        assert_eq!(k.tick(&f), Act::Nothing);
        assert_eq!(k.tick(&f), Act::Restart);
    }

    #[test]
    fn a_failed_version_is_not_tried_again_but_a_newer_one_is() {
        let mut k = Keep::new(Timing::default(), t0(), Some("1.1.0".into()));
        let f = facts(PHASE_READY, "1.1.0", false, t0());
        for _ in 0..5 {
            assert_eq!(k.tick(&f), Act::Nothing);
        }
        assert!(!k.holds_card(true, "1.1.0", t0()), "the card must ask now");
        let f = facts(PHASE_READY, "1.2.0", false, t0());
        assert_eq!(k.tick(&f), Act::Restart);
        k.give_up("1.2.0");
        assert_eq!(k.tick(&f), Act::Nothing);
    }

    #[test]
    fn the_card_waits_a_day_with_the_setting_on() {
        let mut k = keep();
        assert!(k.holds_card(true, "1.1.0", t0()), "before the first tick");
        k.tick(&facts(PHASE_READY, "1.1.0", true, t0()));
        assert!(k.holds_card(true, "1.1.0", t0() + 23 * HOUR));
        assert!(!k.holds_card(true, "1.1.0", t0() + 24 * HOUR));
        assert!(!k.holds_card(false, "1.1.0", t0()), "off: as it always was");
    }

    #[test]
    fn a_new_version_starts_a_new_day() {
        let mut k = keep();
        k.tick(&facts(PHASE_READY, "1.1.0", true, t0()));
        let later = t0() + 30 * HOUR;
        k.tick(&facts(PHASE_READY, "1.2.0", true, later));
        assert!(k.holds_card(true, "1.2.0", later + HOUR));
    }

    #[test]
    fn the_restart_can_still_happen_after_the_card() {
        let mut k = keep();
        k.tick(&facts(PHASE_READY, "1.1.0", true, t0()));
        let late = t0() + 30 * HOUR;
        assert!(!k.holds_card(true, "1.1.0", late));
        k.tick(&facts(PHASE_READY, "1.1.0", false, late));
        assert_eq!(
            k.tick(&facts(PHASE_READY, "1.1.0", false, late)),
            Act::Restart
        );
    }

    #[test]
    fn a_clock_set_back_does_not_stop_the_checks() {
        let mut k = keep();
        let back = t0() - HOUR;
        assert_eq!(k.tick(&facts(PHASE_IDLE, "", false, back)), Act::Check);
    }

    #[test]
    fn an_attempt_survives_only_while_the_old_version_runs() {
        let d = tempfile::tempdir().unwrap();
        let a = Attempt::new(d.path());
        assert_eq!(a.settle("1.0.0"), None);
        a.record("1.1.0").unwrap();
        assert_eq!(a.settle("1.0.0").as_deref(), Some("1.1.0"), "still old");
        assert_eq!(a.settle("1.0.0").as_deref(), Some("1.1.0"), "and again");
        assert_eq!(a.settle("1.1.0"), None, "it worked");
        assert_eq!(a.settle("1.0.0"), None, "and is forgotten");
    }
}
