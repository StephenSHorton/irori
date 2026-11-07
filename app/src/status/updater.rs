//! The updater in the desktop app (main's app/src/shell/update.rs, Go
//! `setupUpdater` in I:tray.go and I:update_card.go), on the Rune shell.
//! GUI role only: `serve --headless` and `mcp` never build one (Go
//! releasing.md).
//!
//! - [`WindowUpdater`] is the window crate's sync `Updater` seam over the
//!   async `update::Updater`, run on the shell's Tokio runtime. The window
//!   calls it from `State::spawn`'s threads, never the UI thread.
//! - [`wire`] hooks it up: `update:status` to the window (through
//!   `Window::emit`, so the status package hears it), quit to the app,
//!   every transition to the tray's "Restart to update" item and the
//!   update card.
//! - The shell spawns `update::Updater::auto_update` at start and calls
//!   `update::Updater::install_on_exit` on the way out.
//! - With Keep this device up to date on (card #578, [`super::keep`]), the
//!   shell's loop checks every few hours and restarts when no agent works,
//!   and the card waits a day for that before it shows Install.

use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};

use ::update as upd;
use irori_window::Window;
use irori_window::update::{EVENT_UPDATE_STATUS, UpdateStatus};
use rune::AppHandle;

use super::keep::Keeper;
use super::tray::Tray;
use crate::ui::State;

/// This build's updater, downloading into `<data dir>/updates`. Its lines
/// go to the Core's log (irori.log): the release exe is a GUI-subsystem
/// program with no stderr.
pub fn new(data: &Path) -> Arc<upd::Updater> {
    let u = upd::Updater::new(upd::REPO, irori_roles::commands::version(), data.join("updates"))
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

fn to_window(st: upd::Status) -> UpdateStatus {
    UpdateStatus {
        phase: st.phase,
        version: st.version,
        notes: st.notes,
        error: st.error,
    }
}

/// The window's updater seam (Go's bound `UpdateService`), with Keep this
/// device up to date: the setting and the loop's wake (turned on, it
/// checks at once), and the runtime the async updater runs on.
pub struct WindowUpdater {
    pub u: Arc<upd::Updater>,
    pub keeper: Arc<Keeper>,
    pub wake: Mutex<mpsc::Sender<()>>,
    pub rt: tokio::runtime::Handle,
}

impl irori_window::seams::Updater for WindowUpdater {
    fn status(&self) -> UpdateStatus {
        to_window(self.u.status())
    }
    fn supported(&self) -> bool {
        self.u.supported()
    }
    fn keep_up_to_date(&self) -> Option<bool> {
        self.u.supported().then(|| self.keeper.on())
    }
    fn set_keep_up_to_date(&self, on: bool) -> Result<Option<bool>, String> {
        if self.keep_up_to_date().is_none() {
            return Err("this device doesn't update itself".into());
        }
        let now = self.keeper.set_on(on)?;
        if now {
            let _ = self.wake.lock().unwrap_or_else(|e| e.into_inner()).send(());
        }
        Ok(Some(now))
    }
    fn check_and_download(&self) -> UpdateStatus {
        let u = self.u.clone();
        match wait(&self.rt, async move { u.check_and_download().await }) {
            Ok(Ok(st)) => to_window(st),
            Ok(Err(e)) => {
                let mut st = to_window(self.u.status());
                if st.error.is_empty() {
                    st.phase = upd::PHASE_ERROR.into();
                    st.error = e;
                }
                st
            }
            Err(_) => to_window(self.u.status()),
        }
    }
    fn restart_to_update(&self) -> Result<(), String> {
        self.u.restart_to_update()
    }
}

/// Runs `f` as its own task on the shell's runtime and waits for it. The
/// wait can last a whole download; on a runtime worker it is
/// block_in_place, so the worker's other tasks move to another thread.
fn wait<T: Send + 'static>(
    rt: &tokio::runtime::Handle,
    f: impl std::future::Future<Output = T> + Send + 'static,
) -> Result<T, mpsc::RecvError> {
    let (tx, rx) = mpsc::channel();
    rt.spawn(async move {
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
/// version replaces it.
struct Card {
    n: notify::Notifier,
    /// Restart to update (Go `install`).
    install: Install,
    open: notify::Callback,
    /// Card #578: wait before posting for this version (Keep this device up
    /// to date restarts by itself within a day; the card goes up after).
    hold: Hold,
    st: Mutex<CardState>,
}

type Install = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;
type Hold = Arc<dyn Fn(&str) -> bool + Send + Sync>;

#[derive(Default)]
struct CardState {
    posted: String,
    announced: String,
}

impl Card {
    fn on_status(self: &Arc<Self>, st: &upd::Status) {
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
        let post = post && !(self.hold)(&st.version);
        if !drop.is_empty() {
            self.n.clear(&card_id(&drop));
        }
        if post {
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
/// makes: hooks to the window and the app, the tray item (when there is a
/// tray), the card.
///
/// Returns the card's re-check, which Keep this device up to date's loop
/// calls so a held card goes up once its day has passed.
pub fn wire(
    app: &AppHandle<State>,
    window: &Arc<Window>,
    u: &Arc<upd::Updater>,
    tray: Option<Arc<Tray>>,
    n: &notify::Notifier,
    keeper: &Arc<Keeper>,
) -> Box<dyn Fn(&upd::Status) + Send> {
    let w = Arc::downgrade(window);
    let hq = app.clone();
    u.set_hooks(
        Arc::new(move |name, st| {
            if let Some(w) = w.upgrade() {
                w.emit(name, &to_window(st.clone()));
            }
        }),
        Arc::new(move || hq.quit()),
    );
    debug_assert_eq!(upd::EVENT_UPDATE_STATUS, EVENT_UPDATE_STATUS);
    let h = app.clone();
    let ui = u.clone();
    let card = Arc::new(Card {
        n: n.clone(),
        install: Arc::new(move || ui.restart_to_update()),
        open: Arc::new(move || h.open()),
        hold: {
            let k = keeper.clone();
            Arc::new(move |v: &str| k.hold_card(v))
        },
        st: Mutex::default(),
    });
    let recheck = {
        let card = card.clone();
        Box::new(move |st: &upd::Status| card.on_status(st))
    };
    let uu = u.clone();
    u.set_on_change(Arc::new(move |st| {
        if let Some(t) = &tray {
            let ready = (st.phase == upd::PHASE_READY).then(|| {
                let u = uu.clone();
                let r: super::tray::Restart = Arc::new(move || u.restart_to_update());
                (st.version.clone(), r)
            });
            t.set_update(ready);
        }
        card.on_status(st);
    }));
    recheck
}

/// Go's update_card_test.go: the card on a fake card system.
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use ::update as upd;

    use super::{Card, Hold, Install};

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
        card_held(headless, install, Arc::new(|_| false))
    }

    fn card_held(
        headless: bool,
        install: Install,
        hold: Hold,
    ) -> (Arc<Card>, Arc<Fake>, notify::Notifier) {
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
            hold,
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

    /// Card #578: held while Keep this device up to date waits for a quiet
    /// moment, then posted once by a later re-check.
    #[test]
    fn held_until_released() {
        let held = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let h = held.clone();
        let (c, f, _) = card_held(
            false,
            no_install(),
            Arc::new(move |_| h.load(Ordering::SeqCst)),
        );
        c.on_status(&ready("0.4.0"));
        c.on_status(&ready("0.4.0"));
        assert_eq!(f.post_count(), 0, "posted while held");
        held.store(false, Ordering::SeqCst);
        c.on_status(&ready("0.4.0"));
        c.on_status(&ready("0.4.0"));
        assert!(f.live("update:0.4.0").is_some(), "not posted once released");
        assert_eq!(f.post_count(), 1, "posted more than once");
    }

    #[test]
    fn headless_never_posts() {
        let (c, f, _) = card_with(true, no_install());
        c.on_status(&ready("0.4.0"));
        assert_eq!(f.post_count(), 0, "a headless Notifier showed the card");
    }
}
