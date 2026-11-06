//! The updater in the desktop app (Go `setupUpdater` in I:tray.go and
//! I:update_card.go). GUI role only: `serve --headless` and `mcp` never
//! build one (Go releasing.md).
//!
//! - [`WindowUpdater`] is the window crate's sync `Updater` seam over the
//!   async `update::Updater`.
//! - [`wire`] hooks it up: `update:status` to the window, quit to the app,
//!   every transition to the tray's "Restart to update" item and the
//!   update card.
//! - The shell spawns `update::Updater::auto_update` at start and
//!   calls `update::Updater::install_on_exit` on the way out.
//! - [`Keeper`] is Keep this device up to date (card #578): on a desktop,
//!   [`Keeper::run`] is the timer, on rules from `update::keep`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use ::update as upd;
use irori_core::Core;
use irori_window::update::{KeepUpToDate, UpdateStatus};
use tauri::{AppHandle, Emitter as _, Manager as _};
use upd::keep::{Act, Attempt, Facts, Keep, Timing};

use super::tray::Tray;

/// This build's updater, downloading into `<data dir>/updates`. Its lines
/// go to the Core's log (irori.log), as Go's `log.Printf` did: the release
/// exe is a GUI-subsystem program with no stderr.
pub fn new(data: &Path) -> Arc<upd::Updater> {
    let u = upd::Updater::new(upd::REPO, crate::commands::version(), data.join("updates"))
        .with_logger(Arc::new(irori_core::log));
    // Test builds only (card #270's emulator run): IRORI_UPDATE_API at
    // build time points the release lookup at a local fake of the GitHub
    // API. Release builds never set it.
    let u = match option_env!("IRORI_UPDATE_API") {
        Some(base) if !base.is_empty() => u.with_api_base(base),
        _ => u,
    };
    Arc::new(u)
}

/// Seconds from a build-time knob (test builds only, as `IRORI_UPDATE_API`:
/// a release build can't be sped up from outside).
fn knob(v: Option<&str>, default: Duration) -> Duration {
    v.and_then(|s| s.parse().ok())
        .map_or(default, Duration::from_secs)
}

fn timing() -> Timing {
    let d = Timing::default();
    Timing {
        tick: knob(option_env!("IRORI_UPDATE_TICK"), d.tick),
        check_every: knob(option_env!("IRORI_UPDATE_EVERY"), d.check_every),
        card_after: knob(option_env!("IRORI_UPDATE_CARD_AFTER"), d.card_after),
    }
}

/// Whether this is a test build pointed at a fake release feed: the probe
/// then still updates, for the rehearsal (tools/rehearse/keep.ps1).
pub fn test_feed() -> bool {
    option_env!("IRORI_UPDATE_API").is_some_and(|b| !b.is_empty())
}

/// The window counts as in use for this long after it last had the focus: a
/// restart closes it, with whatever is typed in it.
const WINDOW_IN_USE: Duration = Duration::from_secs(15 * 60);

/// How long wakes stay held before the last check, so one already on its way
/// shows up in it.
const SETTLE: Duration = Duration::from_secs(3);

/// Keep this device up to date: the setting and its timer.
pub struct Keeper {
    data: PathBuf,
    core: Arc<Core>,
    u: Arc<upd::Updater>,
    keep: Mutex<Keep>,
    attempt: Attempt,
    last_focus: Mutex<Option<SystemTime>>,
    probe: super::probe::Probe,
}

impl Keeper {
    pub fn new(
        data: &Path,
        core: Arc<Core>,
        u: Arc<upd::Updater>,
        probe: super::probe::Probe,
    ) -> Arc<Keeper> {
        let attempt = Attempt::new(data);
        let gave_up = attempt.settle(u.version());
        if let Some(v) = &gave_up {
            irori_core::log(&format!(
                "irori: the restart to v{v} didn't bring it up; the Install card asks instead"
            ));
        }
        Arc::new(Keeper {
            data: data.to_path_buf(),
            core,
            u,
            keep: Mutex::new(Keep::new(timing(), SystemTime::now(), gave_up)),
            attempt,
            last_focus: Mutex::default(),
            probe,
        })
    }

    /// Whether this device can do it at all.
    fn available(&self) -> bool {
        cfg!(desktop) && self.u.supported()
    }

    fn explicit(&self) -> Option<bool> {
        notify::load_keep_up_to_date(&self.data)
    }

    fn on(&self) -> bool {
        self.available()
            && upd::keep::resolve(self.explicit(), self.core.home_server().on)
    }

    fn view(&self) -> KeepUpToDate {
        let explicit = self.explicit();
        KeepUpToDate {
            available: self.available(),
            on: self.on(),
            follows_home: explicit.is_none() && self.core.home_server().on,
        }
    }

    fn holds_card(&self, version: &str) -> bool {
        let on = self.on();
        lock(&self.keep).holds_card(on, version, SystemTime::now())
    }

    /// What stops a restart now: the Core's list, and the window.
    fn blockers(&self, app: &AppHandle) -> Vec<String> {
        let mut why = self.core.restart_blockers();
        let now = SystemTime::now();
        let mut last = lock(&self.last_focus);
        if app
            .get_webview_window(super::MAIN)
            .is_some_and(|w| w.is_focused().unwrap_or(false))
        {
            *last = Some(now);
        }
        let in_use = last.is_some_and(|t| {
            now.duration_since(t)
                .map_or(true, |d| d < WINDOW_IN_USE)
        });
        if in_use {
            why.push("the window is in use".into());
        }
        why
    }

    /// The timer, until `stop`. One tick at a time, on its own thread: a
    /// check can take a whole download.
    pub fn run(
        self: Arc<Self>,
        app: AppHandle,
        card: Arc<Card>,
        stop: sys::cancel::Cancel,
    ) {
        let tick = timing().tick;
        let mut said = Vec::new();
        while !stop.sleep(tick) {
            let st = self.u.status();
            // Only a ready update needs the list; it rereads every session.
            let why = if st.phase == upd::PHASE_READY {
                self.blockers(&app)
            } else {
                Vec::new()
            };
            let on = self.on();
            if on && st.phase == upd::PHASE_READY && why != said {
                irori_core::log(&format!(
                    "irori: update v{} waits: {}",
                    st.version,
                    if why.is_empty() {
                        "nothing now".to_string()
                    } else {
                        why.join("; ")
                    }
                ));
            }
            said.clone_from(&why);
            let act = lock(&self.keep).tick(&Facts {
                on,
                phase: &st.phase,
                version: &st.version,
                blocked: !why.is_empty(),
                now: SystemTime::now(),
            });
            match act {
                Act::Nothing => {}
                Act::Check => {
                    self.probe.note("update-check");
                    let u = self.u.clone();
                    match wait(async move { u.check_and_download().await }) {
                        Ok(Err(e)) => {
                            irori_core::log(&format!("irori: timed update check failed: {e}"))
                        }
                        Err(_) => irori_core::log("irori: timed update check stopped"),
                        Ok(Ok(_)) => {}
                    }
                }
                Act::Restart => {
                    if self.restart(&app, &st.version, &stop) {
                        return;
                    }
                }
            }
            // The day may just be up, or the setting just turned off.
            card.on_status(&self.u.status());
        }
    }

    /// Holds wakes back, checks one last time, and restarts. True when
    /// Irori is on its way out.
    fn restart(&self, app: &AppHandle, version: &str, stop: &sys::cancel::Cancel) -> bool {
        self.core.hold_wakes(true);
        let stopped = stop.sleep(SETTLE);
        let why = self.blockers(app);
        if stopped || !why.is_empty() {
            self.core.hold_wakes(false);
            lock(&self.keep).restart_stopped();
            return stopped;
        }
        if let Err(e) = self.attempt.record(version) {
            irori_core::log(&format!("irori: can't restart to update: {e}"));
            self.core.hold_wakes(false);
            lock(&self.keep).give_up(version);
            return false;
        }
        irori_core::log(&format!(
            "irori: no agent is working: restarting to v{version}"
        ));
        self.probe.note("update-restart");
        match self.u.restart_hidden() {
            Ok(()) => true,
            Err(e) => {
                irori_core::log(&format!("irori: restart to update: {e}"));
                self.core.hold_wakes(false);
                lock(&self.keep).give_up(version);
                false
            }
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn to_window(st: upd::Status) -> UpdateStatus {
    UpdateStatus {
        phase: st.phase,
        version: st.version,
        notes: st.notes,
        error: st.error,
    }
}

/// The window's updater seam (Go's bound `UpdateService`).
pub struct WindowUpdater(pub Arc<upd::Updater>, pub Arc<Keeper>);

impl irori_window::seams::Updater for WindowUpdater {
    fn status(&self) -> UpdateStatus {
        to_window(self.0.status())
    }
    fn supported(&self) -> bool {
        self.0.supported()
    }
    fn check_and_download(&self) -> UpdateStatus {
        let u = self.0.clone();
        let got = wait(async move { u.check_and_download().await });
        match got {
            Ok(Ok(st)) => to_window(st),
            Ok(Err(e)) => {
                let mut st = to_window(self.0.status());
                if st.error.is_empty() {
                    st.phase = upd::PHASE_ERROR.into();
                    st.error = e;
                }
                st
            }
            Err(_) => to_window(self.0.status()),
        }
    }
    fn restart_to_update(&self) -> Result<(), String> {
        if cfg!(target_os = "android") {
            // Card #270: Install downloads, checks and opens Android's
            // install screen; it may take the whole download.
            let u = self.0.clone();
            return match wait(async move { u.install_apk().await }) {
                Ok(r) => r.map(|_| ()),
                Err(_) => Err("the update stopped".into()),
            };
        }
        self.0.restart_to_update()
    }
    fn keep_up_to_date(&self) -> KeepUpToDate {
        self.1.view()
    }
    fn set_keep_up_to_date(&self, on: bool) -> Result<(), String> {
        notify::save_keep_up_to_date(&self.1.data, on)
    }
}

/// Runs `f` as its own task on the shared runtime and waits for it. The
/// window's commands are async on that runtime, where a nested block_on
/// would panic on a worker. The wait can last a whole download, so on a
/// runtime worker it is block_in_place: the worker's other tasks move to
/// another thread instead of stalling behind it. The runtime is
/// multi-thread (shell::run), which block_in_place needs.
fn wait<T: Send + 'static>(
    f: impl std::future::Future<Output = T> + Send + 'static,
) -> Result<T, std::sync::mpsc::RecvError> {
    let (tx, rx) = std::sync::mpsc::channel();
    tauri::async_runtime::spawn(async move {
        let _ = tx.send(f.await);
    });
    let on_worker = tokio::runtime::Handle::try_current()
        .is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread);
    if on_worker {
        tokio::task::block_in_place(|| rx.recv())
    } else {
        rx.recv()
    }
}

/// Go `updateCardID`.
fn card_id(v: &str) -> String {
    format!("update:{v}")
}

/// Go `updateCard`: the sticky "Irori v<v> is ready" card with Install
/// (restart to update now) and Later (close; it installs on quit). Once
/// per version per run; it comes down when the install starts or another
/// version replaces it. With Keep this device up to date on, it waits a day
/// for Irori to restart by itself (`holds`, card #578).
pub struct Card {
    n: notify::Notifier,
    /// Restart to update (Go `install`).
    install: Install,
    open: notify::Callback,
    holds: Holds,
    st: Mutex<CardState>,
}

type Install = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;
type Holds = Arc<dyn Fn(&str) -> bool + Send + Sync>;

#[derive(Default)]
struct CardState {
    posted: String,
    announced: String,
}

impl Card {
    pub fn on_status(self: &Arc<Self>, st: &upd::Status) {
        let (drop, post) = {
            let mut g = self.st.lock().unwrap_or_else(|e| e.into_inner());
            let mut drop = String::new();
            if !g.posted.is_empty() && (st.phase != upd::PHASE_READY || st.version != g.posted) {
                drop = std::mem::take(&mut g.posted);
            }
            let post =
                st.phase == upd::PHASE_READY && !st.version.is_empty() && st.version != g.announced;
            (drop, post)
        };
        if !drop.is_empty() {
            self.n.clear(&card_id(&drop));
        }
        if post && !(self.holds)(&st.version) {
            self.post(&st.version, "", false);
        }
    }

    fn post(self: &Arc<Self>, v: &str, failure: &str, silent: bool) {
        let (body, level) = if failure.is_empty() {
            (
                "Install restarts Irori. Or keep working: it installs when you quit.".to_string(),
                notify::Level::Info,
            )
        } else {
            (
                format!(
                    "Couldn't start the installer: {failure}. Try again, or quit Irori to install."
                ),
                notify::Level::Warn,
            )
        };
        let me = self.clone();
        let ver = v.to_string();
        let note = notify::Note {
            id: card_id(v),
            title: format!("Irori v{v} is ready"),
            body,
            level,
            sticky: true,
            silent,
            actions: vec![
                notify::Action {
                    label: "Install".into(),
                    run: Some(Arc::new(move || me.install_now(&ver))),
                },
                // Only closes the card; it still installs on quit.
                notify::Action {
                    label: "Later".into(),
                    run: None,
                },
            ],
            on_click: Some(self.open.clone()),
            ..notify::Note::default()
        };
        if !self.n.post(note) {
            return;
        }
        let mut g = self.st.lock().unwrap_or_else(|e| e.into_inner());
        g.posted = v.to_string();
        g.announced = v.to_string();
    }

    /// Install: restart to update. On failure the updater is back at
    /// ready and nothing else would say so, so the card comes back with
    /// the reason.
    fn install_now(self: &Arc<Self>, v: &str) {
        if let Err(e) = (self.install)() {
            self.post(v, &e, true);
        }
    }
}

/// Go `setupUpdater` minus the start and shutdown calls, which the shell
/// makes: hooks to the app, the tray item (when there is a tray), the card.
pub fn wire(
    app: &AppHandle,
    u: &Arc<upd::Updater>,
    keeper: &Arc<Keeper>,
    tray: Option<Arc<Tray>>,
    n: &notify::Notifier,
) -> Arc<Card> {
    let h = app.clone();
    let hq = app.clone();
    u.set_hooks(
        Arc::new(move |name, st| {
            let _ = h.emit(name, to_window(st.clone()));
        }),
        Arc::new(move || hq.exit(0)),
    );
    let h = app.clone();
    let ui = u.clone();
    let k = keeper.clone();
    let card = Arc::new(Card {
        n: n.clone(),
        install: Arc::new(move || ui.restart_to_update()),
        open: Arc::new(move || super::show(&h)),
        holds: Arc::new(move |v| k.holds_card(v)),
        st: Mutex::default(),
    });
    let uu = u.clone();
    let on_change = card.clone();
    u.set_on_change(Arc::new(move |st| {
        if let Some(t) = &tray {
            let ready = (st.phase == upd::PHASE_READY).then(|| {
                let u = uu.clone();
                let r: super::tray::Restart = Arc::new(move || u.restart_to_update());
                (st.version.clone(), r)
            });
            t.set_update(ready);
        }
        // A phone shows the window's banner only: the desktop card's
        // "Install restarts Irori" isn't how Android updates (card #270).
        if !cfg!(target_os = "android") {
            on_change.on_status(st);
        }
    }));
    card
}

/// Go's update_card_test.go: the card on a fake card system.
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use ::update as upd;

    use super::{Card, Install};

    /// Go `cardToaster`: keeps the live cards and presses a button the
    /// way the engine does (the card closes, then its run).
    #[derive(Default)]
    struct Fake {
        live: Mutex<HashMap<String, toast::Note>>,
        posts: Mutex<Vec<toast::Note>>,
    }

    impl notify::Toaster for Fake {
        fn configure(&self, _c: toast::Config) {}
        fn set_options(&self, _o: toast::Options) {}
        fn post(&self, n: toast::Note) {
            self.live.lock().unwrap().insert(n.id.clone(), n.clone());
            self.posts.lock().unwrap().push(n);
        }
        fn close(&self, id: &str) {
            self.live.lock().unwrap().remove(id);
        }
    }

    impl Fake {
        fn live(&self, id: &str) -> Option<toast::Note> {
            self.live.lock().unwrap().get(id).cloned()
        }
        fn live_count(&self) -> usize {
            self.live.lock().unwrap().len()
        }
        fn post_count(&self) -> usize {
            self.posts.lock().unwrap().len()
        }
        fn press(&self, id: &str, label: &str) {
            let n = self
                .live
                .lock()
                .unwrap()
                .remove(id)
                .unwrap_or_else(|| panic!("no live card {id:?}"));
            let a = n
                .actions
                .iter()
                .find(|a| a.label == label)
                .unwrap_or_else(|| panic!("card {id:?} has no button {label:?}"));
            if let Some(f) = &a.run {
                f();
            }
        }
    }

    fn card_with(headless: bool, install: Install) -> (Arc<Card>, Arc<Fake>, notify::Notifier) {
        let fake = Arc::new(Fake::default());
        let n = notify::Notifier::new(notify::Config {
            headless,
            toaster: Some(fake.clone()),
            ..notify::Config::default()
        });
        let c = Arc::new(Card {
            n: n.clone(),
            install,
            open: Arc::new(|| {}),
            holds: Arc::new(|_| false),
            st: Mutex::default(),
        });
        (c, fake, n)
    }

    fn card(install: Install) -> (Arc<Card>, Arc<Fake>, notify::Notifier) {
        card_with(false, install)
    }

    fn no_install() -> Install {
        Arc::new(|| panic!("nothing should install"))
    }

    fn counting(calls: &Arc<AtomicUsize>, res: Result<(), String>) -> Install {
        let calls = calls.clone();
        Arc::new(move || {
            calls.fetch_add(1, Ordering::SeqCst);
            res.clone()
        })
    }

    fn st(phase: &str, v: &str) -> upd::Status {
        upd::Status {
            phase: phase.into(),
            version: v.into(),
            notes: String::new(),
            installer_path: std::path::PathBuf::new(),
            error: String::new(),
        }
    }

    fn ready(v: &str) -> upd::Status {
        st(upd::PHASE_READY, v)
    }

    #[test]
    fn posts_on_ready() {
        let (c, f, _) = card(no_install());
        c.on_status(&st(upd::PHASE_CHECKING, ""));
        c.on_status(&st(upd::PHASE_DOWNLOADING, "0.4.0"));
        assert_eq!(f.live_count(), 0, "a card went up before ready");
        c.on_status(&ready("0.4.0"));
        let n = f.live("update:0.4.0").expect("no update:0.4.0 card");
        assert!(n.sticky, "the update card must be sticky");
        assert_eq!(n.title, "Irori v0.4.0 is ready");
        let labels: Vec<&str> = n.actions.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, ["Install", "Later"]);
        // Every later transition at ready must not post it again.
        c.on_status(&ready("0.4.0"));
        assert_eq!(f.post_count(), 1, "posted more than once per version");
    }

    #[test]
    fn install_restarts() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (c, f, _) = card(counting(&calls, Ok(())));
        c.on_status(&ready("0.4.0"));
        f.press("update:0.4.0", "Install");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        c.on_status(&st(upd::PHASE_INSTALLING, "0.4.0"));
        assert_eq!(f.live_count(), 0, "cards still up while installing");
    }

    #[test]
    fn later_just_closes() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (c, f, _) = card(counting(&calls, Ok(())));
        c.on_status(&ready("0.4.0"));
        f.press("update:0.4.0", "Later");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "Later must not install now"
        );
        c.on_status(&ready("0.4.0"));
        assert_eq!(f.live_count(), 0, "Later's card came back");
    }

    #[test]
    fn cleared_when_installing_elsewhere() {
        let (c, f, _) = card(no_install());
        c.on_status(&ready("0.4.0"));
        c.on_status(&st(upd::PHASE_INSTALLING, "0.4.0"));
        assert!(f.live("update:0.4.0").is_none(), "the card stayed up");
    }

    #[test]
    fn superseded() {
        let (c, f, _) = card(no_install());
        c.on_status(&ready("0.4.0"));
        c.on_status(&ready("0.5.0"));
        assert!(f.live("update:0.4.0").is_none(), "0.4.0 stayed up");
        assert!(f.live("update:0.5.0").is_some(), "no 0.5.0 card");
    }

    #[test]
    fn install_failure_reposts_with_reason() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (c, f, _) = card(counting(&calls, Err("access denied".into())));
        c.on_status(&ready("0.4.0"));
        f.press("update:0.4.0", "Install");
        c.on_status(&ready("0.4.0"));
        let n = f.live("update:0.4.0").expect("no card to retry from");
        assert!(n.silent && n.kind == toast::Kind::Warn, "retry card {n:?}");
        assert!(n.body.contains("access denied"), "body {:?}", n.body);
        assert_eq!(f.post_count(), 2, "want the first and the one retry");
    }

    #[test]
    fn respects_notifications_off() {
        let (c, f, n) = card(no_install());
        let mut o = n.options();
        o.enabled = false;
        n.set_options(o.clone()).1.unwrap();
        c.on_status(&ready("0.4.0"));
        assert_eq!(f.post_count(), 0, "a card went up with notifications off");
        o.enabled = true;
        n.set_options(o).1.unwrap();
        c.on_status(&ready("0.4.0"));
        assert!(f.live("update:0.4.0").is_some(), "no card after back on");
    }

    #[test]
    fn holds_until_let_go() {
        use std::sync::atomic::AtomicBool;
        let fake = Arc::new(Fake::default());
        let n = notify::Notifier::new(notify::Config {
            toaster: Some(fake.clone()),
            ..notify::Config::default()
        });
        let hold = Arc::new(AtomicBool::new(true));
        let h = hold.clone();
        let c = Arc::new(Card {
            n,
            install: no_install(),
            open: Arc::new(|| {}),
            holds: Arc::new(move |_| h.load(Ordering::SeqCst)),
            st: Mutex::default(),
        });
        c.on_status(&ready("0.4.0"));
        assert_eq!(fake.post_count(), 0, "the card went up inside the day");
        hold.store(false, Ordering::SeqCst);
        c.on_status(&ready("0.4.0"));
        assert!(fake.live("update:0.4.0").is_some(), "no card after the day");
        c.on_status(&ready("0.4.0"));
        assert_eq!(fake.post_count(), 1, "posted more than once");
    }

    #[test]
    fn headless_never_posts() {
        let (c, f, _) = card_with(true, no_install());
        c.on_status(&ready("0.4.0"));
        assert_eq!(f.post_count(), 0, "a headless Notifier showed the card");
    }
}
