//! Keep this device up to date (card #578, docs/design/no-server.md §4.3,
//! decision b4; main's app/src/shell/keep.rs, its check now on the
//! shell's Tokio runtime). A per-device setting: with it on, the desktop app checks
//! for a release every few hours and downloads it. Once one is ready it
//! restarts by itself at a moment when no agent turn runs on this device,
//! and the Install card waits a day for that moment before it goes up.
//!
//! - The setting is `keep-up-to-date.json` (`{"on": bool}`). With no file
//!   it follows the home server's switch, read at every tick, so turning
//!   the home server on turns this on too. Only the person's own choice is
//!   written.
//! - `auto-update.json` is the marker for the version that is ready: when
//!   it was first seen ready (the day the card waits), and whether an
//!   automatic restart was tried. It lives beside `updates/`, not in it:
//!   a download deletes every other file there.
//! - The relaunched app reads the marker before its window opens
//!   ([`after_start`]): it starts hidden when the window was hidden before
//!   the restart, and a version it tried and still isn't running is never
//!   tried again (no restart loop when an install fails). The installer and
//!   the swap helper are unchanged.
//! - "No agent working" is [`idle_now`]: every Claude session presence
//!   sees on this device is idle (or not answering, or gone) and has been
//!   for a minute, and no Irori inline terminal is open (a restart closes
//!   them). Grok members are `irori mcp` processes that survive a restart
//!   and aren't seen here.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ::update as upd;
use serde::{Deserialize, Serialize};

/// The setting's file in the data dir.
pub const SETTING_FILE: &str = "keep-up-to-date.json";
/// The ready version's marker in the data dir.
pub const MARKER_FILE: &str = "auto-update.json";

/// How often the device looks for a release.
pub const EVERY: Duration = Duration::from_secs(4 * 60 * 60);
/// How long the Install card waits for a restart with no agent working.
pub const HOLD: Duration = Duration::from_secs(24 * 60 * 60);
/// How often the loop wakes. Short, and measured against the wall clock,
/// so a machine that slept doesn't push the day back.
pub const TICK: Duration = Duration::from_secs(10 * 60);
/// How long every session must have been idle: a turn that just ended
/// may be about to start the next one.
pub const SETTLE: Duration = Duration::from_secs(60);
/// A relaunch starts hidden only this soon after the restart was tried.
const RELAUNCH_WINDOW: i64 = 10 * 60;

/// [`EVERY`], [`HOLD`] and [`TICK`], which a test build (one built with
/// `IRORI_UPDATE_API`, pointed at a fake release feed) may shorten with
/// `IRORI_UPDATE_EVERY`, `IRORI_UPDATE_HOLD` and `IRORI_UPDATE_TICK`, in
/// seconds, so a rehearsal doesn't wait hours. Release builds ignore them.
pub fn timings() -> (Duration, Duration, Duration) {
    let test_build = option_env!("IRORI_UPDATE_API").is_some_and(|s| !s.is_empty());
    let pick = |name: &str, d: Duration| {
        if !test_build {
            return d;
        }
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|s| *s > 0)
            .map_or(d, Duration::from_secs)
    };
    (
        pick("IRORI_UPDATE_EVERY", EVERY),
        pick("IRORI_UPDATE_HOLD", HOLD),
        pick("IRORI_UPDATE_TICK", TICK),
    )
}

#[derive(Serialize, Deserialize)]
struct Saved {
    on: bool,
}

/// The person's own choice, if they made one.
pub fn saved(dir: &Path) -> Option<bool> {
    let raw = std::fs::read(dir.join(SETTING_FILE)).ok()?;
    serde_json::from_slice::<Saved>(&raw).ok().map(|s| s.on)
}

/// Whether the setting is on: the person's choice, else the home server's
/// switch.
pub fn on(dir: &Path) -> bool {
    saved(dir).unwrap_or_else(|| home::settings::load(dir).map(|s| s.on).unwrap_or(false))
}

/// Saves the person's choice.
pub fn save(dir: &Path, on: bool) -> std::io::Result<()> {
    write_json(dir, SETTING_FILE, &Saved { on })
}

fn write_json<T: Serialize>(dir: &Path, name: &str, v: &T) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{name}.tmp"));
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(v).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&tmp, dir.join(name))
}

/// The ready version's marker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Marker {
    pub version: String,
    /// Unix seconds when this version was first seen ready.
    pub ready_since: i64,
    /// Unix seconds when an automatic restart was tried; 0 if never.
    #[serde(default)]
    pub tried_at: i64,
    /// The window was hidden when the restart was tried.
    #[serde(default)]
    pub hidden: bool,
    /// The restart was tried and this version didn't come up: never try
    /// it again by itself.
    #[serde(default)]
    pub failed: bool,
}

pub fn load_marker(dir: &Path) -> Option<Marker> {
    let raw = std::fs::read(dir.join(MARKER_FILE)).ok()?;
    serde_json::from_slice(&raw).ok()
}

fn save_marker(dir: &Path, m: &Marker) {
    if let Err(e) = write_json(dir, MARKER_FILE, m) {
        irori_core::log(&format!("update: keep up to date: save {MARKER_FILE}: {e}"));
    }
}

fn clear_marker(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(MARKER_FILE));
}

/// Unix seconds now.
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// At start, before the window opens: whether this launch is the restart
/// an automatic update made with the window hidden. A marker for a version
/// this build has reached is cleared; one that was tried and isn't running
/// is marked failed, so it is never tried again by itself.
pub fn after_start(dir: &Path, running: &str, now: i64) -> bool {
    let Some(mut m) = load_marker(dir) else {
        return false;
    };
    if !upd::version::is_newer(&m.version, running) {
        clear_marker(dir);
        return m.tried_at > 0 && m.hidden && (0..=RELAUNCH_WINDOW).contains(&(now - m.tried_at));
    }
    if m.tried_at > 0 && !m.failed {
        m.failed = true;
        save_marker(dir, &m);
        irori_core::log(&format!(
            "update: keep up to date: the restart to v{} didn't install it; Install on the card will",
            m.version
        ));
    }
    false
}

/// Whether nothing on this device would lose work to a restart now:
/// every local Claude session is idle, not answering, or gone, and has
/// been for [`SETTLE`]; and no inline terminal is open.
pub fn idle_now(
    cards: &[presence::AgentCard],
    terms_open: usize,
    now: i64,
    settle: Duration,
) -> bool {
    if terms_open > 0 {
        return false;
    }
    cards.iter().filter(|c| c.device_state.is_empty()).all(|c| {
        let s = c.status.as_str();
        if s == "gone" {
            return true;
        }
        if s != "idle" && s != coord::roster::NOT_ANSWERING {
            return false;
        }
        let since = c.status_since.secs;
        since <= 0 || now - since >= settle.as_secs() as i64
    })
}

/// The setting and the marker for one data dir, with the times the loop
/// and the card use.
pub struct Keeper {
    dir: PathBuf,
    hold: Duration,
    now: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl Keeper {
    pub fn new(dir: PathBuf, hold: Duration) -> Keeper {
        Keeper {
            dir,
            hold,
            now: Arc::new(unix_now),
        }
    }

    #[cfg(test)]
    fn with_clock(mut self, now: Arc<dyn Fn() -> i64 + Send + Sync>) -> Keeper {
        self.now = now;
        self
    }

    pub fn on(&self) -> bool {
        on(&self.dir)
    }

    pub fn set_on(&self, v: bool) -> Result<bool, String> {
        save(&self.dir, v).map_err(|e| format!("save {SETTING_FILE}: {e}"))?;
        Ok(self.on())
    }

    /// The marker for ready version `v`, made now if it names another one.
    fn ready(&self, v: &str) -> Marker {
        match load_marker(&self.dir) {
            Some(m) if m.version == v => m,
            _ => {
                let m = Marker {
                    version: v.into(),
                    ready_since: (self.now)(),
                    ..Marker::default()
                };
                save_marker(&self.dir, &m);
                m
            }
        }
    }

    /// The card's question for ready version `v`: wait before showing
    /// Install? Only while the setting is on, the day hasn't passed, and a
    /// restart wasn't tried and failed.
    pub fn hold_card(&self, v: &str) -> bool {
        if !self.on() {
            return false;
        }
        let m = self.ready(v);
        !m.failed && (self.now)() < m.ready_since + self.hold.as_secs() as i64
    }

    /// Whether the loop may restart for `v` by itself.
    pub fn may_restart(&self, v: &str) -> bool {
        let m = self.ready(v);
        m.tried_at == 0 && !m.failed
    }

    /// Notes the try before the restart, so the next start can tell.
    pub fn tried(&self, v: &str, hidden: bool) {
        let mut m = self.ready(v);
        m.tried_at = (self.now)();
        m.hidden = hidden;
        save_marker(&self.dir, &m);
    }

    /// The restart couldn't start: leave it to the card.
    pub fn failed(&self, v: &str) {
        let mut m = self.ready(v);
        m.failed = true;
        save_marker(&self.dir, &m);
    }
}

/// What the loop does on one tick.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Wait,
    Check,
    Restart,
}

/// One tick's decision. A check runs from idle or error once `every` has
/// passed (or at once when the person just turned the setting on); a
/// ready version restarts when allowed and nothing is working. Never
/// anything while a check, download or install is running.
pub fn decide(
    on: bool,
    phase: &str,
    since_check: Duration,
    every: Duration,
    may_restart: bool,
    idle: bool,
) -> Action {
    if !on {
        return Action::Wait;
    }
    match phase {
        upd::PHASE_IDLE | upd::PHASE_ERROR if since_check >= every => Action::Check,
        upd::PHASE_READY if may_restart && idle => Action::Restart,
        _ => Action::Wait,
    }
}

/// What the loop needs from the app.
pub struct Hooks {
    /// No agent working on this device ([`idle_now`] over a fresh scan).
    pub idle: Box<dyn Fn() -> bool + Send>,
    /// The main window is on screen.
    pub visible: Box<dyn Fn() -> bool + Send>,
    /// Shows the held Install card once its day has passed.
    pub recheck_card: Box<dyn Fn(&upd::Status) + Send>,
}

/// Runs the loop on its own thread until `stop` is canceled or every
/// sender of `rx` is gone; a message on `rx` (the setting was just turned
/// on) checks at once. The start check already ran (`auto_update`), so the first check
/// here is `every` later.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    keeper: Arc<Keeper>,
    u: Arc<upd::Updater>,
    rt: tokio::runtime::Handle,
    hooks: Hooks,
    rx: mpsc::Receiver<()>,
    stop: sys::cancel::Cancel,
    every: Duration,
    tick: Duration,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("keep-up-to-date".into())
        .spawn(move || {
            // The last check by both clocks: Instant may stop while the
            // machine sleeps, and the wall clock may step back.
            let mut last_check = std::time::Instant::now();
            let mut last_check_wall = unix_now();
            loop {
                let forced = match rx.recv_timeout(tick) {
                    Ok(()) => true,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    Err(mpsc::RecvTimeoutError::Timeout) => false,
                };
                if stop.is_canceled() {
                    return;
                }
                let wall = Duration::from_secs((unix_now() - last_check_wall).max(0) as u64);
                let since = last_check.elapsed().max(wall);
                tick_once(
                    &keeper,
                    &u,
                    &rt,
                    &hooks,
                    if forced { every } else { since },
                    every,
                    &mut || {
                        last_check = std::time::Instant::now();
                        last_check_wall = unix_now();
                    },
                );
            }
        })
}

fn tick_once(
    keeper: &Keeper,
    u: &upd::Updater,
    rt: &tokio::runtime::Handle,
    hooks: &Hooks,
    since_check: Duration,
    every: Duration,
    checked: &mut dyn FnMut(),
) {
    let on = keeper.on();
    let st = u.status();
    let ready = on && st.phase == upd::PHASE_READY && !st.version.is_empty();
    let may = ready && keeper.may_restart(&st.version);
    // The fresh scan runs only when it could matter.
    let idle = may && (hooks.idle)();
    match decide(on, &st.phase, since_check, every, may, idle) {
        Action::Check => {
            checked();
            if let Err(e) = rt.block_on(u.check_and_download()) {
                irori_core::log(&format!("update: keep up to date: check: {e}"));
            }
        }
        Action::Restart => {
            let hidden = !(hooks.visible)();
            keeper.tried(&st.version, hidden);
            irori_core::log(&format!(
                "update: keep up to date: no agent working; restarting to v{}{}",
                st.version,
                if hidden { " (window hidden)" } else { "" }
            ));
            if let Err(e) = u.restart_to_update_in_background() {
                irori_core::log(&format!("update: keep up to date: restart: {e}"));
                keeper.failed(&st.version);
                (hooks.recheck_card)(&u.status());
            }
        }
        Action::Wait => {
            if st.phase == upd::PHASE_READY {
                (hooks.recheck_card)(&st);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, Ordering};

    use super::*;

    fn card(status: &str, since: i64) -> presence::AgentCard {
        let mut c = presence::AgentCard {
            status: status.into(),
            ..presence::AgentCard::default()
        };
        c.status_since.secs = since;
        c
    }

    #[test]
    fn setting_follows_home_server_until_chosen() {
        let d = tempfile::tempdir().unwrap();
        assert!(!on(d.path()), "off with no home server and no choice");
        let mut h = home::settings::Settings {
            on: true,
            ..home::settings::Settings::default()
        };
        home::settings::save(d.path(), &h).unwrap();
        assert!(on(d.path()), "on where the home server is on");
        assert!(saved(d.path()).is_none(), "the default was written");
        save(d.path(), false).unwrap();
        assert!(!on(d.path()), "the person's off must stick");
        save(d.path(), true).unwrap();
        h.on = false;
        home::settings::save(d.path(), &h).unwrap();
        assert!(on(d.path()), "the person's on must stick");
    }

    #[test]
    fn unreadable_setting_falls_back() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(SETTING_FILE), b"{junk").unwrap();
        assert_eq!(saved(d.path()), None);
        assert!(!on(d.path()));
    }

    #[test]
    fn idle_gate() {
        let now = 10_000;
        let s = SETTLE;
        assert!(idle_now(&[], 0, now, s), "no agents");
        assert!(idle_now(&[card("idle", now - 120)], 0, now, s));
        assert!(
            !idle_now(&[card("idle", now - 5)], 0, now, s),
            "just went idle"
        );
        assert!(idle_now(&[card("idle", 0)], 0, now, s), "no since: old");
        assert!(idle_now(
            &[card(coord::roster::NOT_ANSWERING, now - 120)],
            0,
            now,
            s
        ));
        assert!(idle_now(&[card("gone", now)], 0, now, s));
        for busy in ["busy", "waiting", "shell", "", "something new"] {
            assert!(
                !idle_now(&[card("idle", 0), card(busy, 0)], 0, now, s),
                "{busy:?} let a restart through"
            );
        }
        assert!(!idle_now(&[card("idle", 0)], 1, now, s), "an open terminal");
        let mut far = card("busy", 0);
        far.device_state = "connected".into();
        assert!(
            idle_now(&[far], 0, now, s),
            "another device's agent blocked"
        );
    }

    #[test]
    fn decisions() {
        let e = EVERY;
        let long = e + Duration::from_secs(1);
        let short = Duration::from_secs(5);
        assert_eq!(
            decide(false, upd::PHASE_IDLE, long, e, true, true),
            Action::Wait
        );
        assert_eq!(
            decide(true, upd::PHASE_IDLE, long, e, false, false),
            Action::Check
        );
        assert_eq!(
            decide(true, upd::PHASE_ERROR, long, e, false, false),
            Action::Check
        );
        assert_eq!(
            decide(true, upd::PHASE_IDLE, short, e, false, false),
            Action::Wait
        );
        for p in [
            upd::PHASE_CHECKING,
            upd::PHASE_DOWNLOADING,
            upd::PHASE_INSTALLING,
        ] {
            assert_eq!(decide(true, p, long, e, true, true), Action::Wait, "{p}");
        }
        assert_eq!(
            decide(true, upd::PHASE_READY, long, e, true, true),
            Action::Restart
        );
        assert_eq!(
            decide(true, upd::PHASE_READY, long, e, true, false),
            Action::Wait
        );
        assert_eq!(
            decide(true, upd::PHASE_READY, long, e, false, true),
            Action::Wait
        );
    }

    fn keeper(d: &Path, clock: &Arc<AtomicI64>) -> Keeper {
        let c = clock.clone();
        Keeper::new(d.to_path_buf(), HOLD).with_clock(Arc::new(move || c.load(Ordering::SeqCst)))
    }

    #[test]
    fn card_waits_a_day_while_on() {
        let d = tempfile::tempdir().unwrap();
        let clock = Arc::new(AtomicI64::new(1_000));
        let k = keeper(d.path(), &clock);
        assert!(!k.hold_card("1.0.0"), "held with the setting off");
        save(d.path(), true).unwrap();
        assert!(k.hold_card("1.0.0"), "not held on the first ready");
        clock.store(1_000 + HOLD.as_secs() as i64 - 1, Ordering::SeqCst);
        assert!(k.hold_card("1.0.0"), "the day restarted");
        clock.store(1_000 + HOLD.as_secs() as i64, Ordering::SeqCst);
        assert!(!k.hold_card("1.0.0"), "held past the day");
        // A newer version starts its own day.
        assert!(k.hold_card("1.1.0"));
        k.failed("1.1.0");
        assert!(!k.hold_card("1.1.0"), "held after a failed restart");
        assert!(!k.may_restart("1.1.0"));
    }

    #[test]
    fn relaunch_hidden_once_and_no_loop() {
        let d = tempfile::tempdir().unwrap();
        let clock = Arc::new(AtomicI64::new(5_000));
        let k = keeper(d.path(), &clock);
        assert!(k.may_restart("1.0.0"));
        k.tried("1.0.0", true);
        assert!(!k.may_restart("1.0.0"), "tried twice in one run");
        // It came up on the new version, a minute later: hidden, cleared.
        assert!(after_start(d.path(), "1.0.0", 5_060));
        assert!(load_marker(d.path()).is_none(), "the marker stayed");
        assert!(!after_start(d.path(), "1.0.0", 5_061), "hidden twice");

        // The install failed: the old version came back.
        k.tried("1.0.0", true);
        assert!(
            !after_start(d.path(), "0.9.0", 5_060),
            "a failed install starts hidden"
        );
        let m = load_marker(d.path()).unwrap();
        assert!(m.failed, "a failed install may be tried again");
        assert!(!k.may_restart("1.0.0"));
        assert!(!k.hold_card("1.0.0"), "the card waits after a failure");

        // Shown before the restart: shown after.
        clear_marker(d.path());
        k.tried("1.0.0", false);
        assert!(!after_start(d.path(), "1.0.0", 5_060));
        // A stale try (the person quit and opened it much later).
        k.tried("1.0.0", true);
        assert!(!after_start(d.path(), "1.0.0", 5_000 + RELAUNCH_WINDOW + 1));
    }

    #[test]
    fn pending_marker_survives_a_plain_restart() {
        let d = tempfile::tempdir().unwrap();
        let clock = Arc::new(AtomicI64::new(5_000));
        let k = keeper(d.path(), &clock);
        save(d.path(), true).unwrap();
        assert!(k.hold_card("1.0.0"));
        // Restarted on the old version with nothing tried: the day goes on.
        assert!(!after_start(d.path(), "0.9.0", 6_000));
        assert_eq!(load_marker(d.path()).unwrap().ready_since, 5_000);
    }
}
