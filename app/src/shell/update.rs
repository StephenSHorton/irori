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

use std::path::Path;
use std::sync::{Arc, Mutex};

use ::update as upd;
use irori_window::update::UpdateStatus;
use tauri::{AppHandle, Emitter as _};

use super::tray::Tray;

/// This build's updater, downloading into `<data dir>/updates`. Its lines
/// go to the Core's log (irori.log), as Go's `log.Printf` did: the release
/// exe is a GUI-subsystem program with no stderr.
pub fn new(data: &Path) -> Arc<upd::Updater> {
    Arc::new(
        upd::Updater::new(upd::REPO, crate::commands::version(), data.join("updates"))
            .with_logger(Arc::new(irori_core::log)),
    )
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
pub struct WindowUpdater(pub Arc<upd::Updater>);

impl irori_window::seams::Updater for WindowUpdater {
    fn status(&self) -> UpdateStatus {
        to_window(self.0.status())
    }
    fn supported(&self) -> bool {
        self.0.supported()
    }
    fn check_and_download(&self) -> UpdateStatus {
        // The window's command is async on the shared runtime: run the
        // check as its own task and wait on a channel (a nested block_on
        // would panic on a worker). The wait can last the whole download,
        // so on a runtime worker it is block_in_place: the worker's other
        // tasks move to another thread instead of stalling behind it. The
        // runtime is multi-thread (shell::run), which block_in_place needs.
        let u = self.0.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        tauri::async_runtime::spawn(async move {
            let _ = tx.send(u.check_and_download().await);
        });
        let on_worker = tokio::runtime::Handle::try_current()
            .is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread);
        let got = if on_worker {
            tokio::task::block_in_place(|| rx.recv())
        } else {
            rx.recv()
        };
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
        self.0.restart_to_update()
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
    st: Mutex<CardState>,
}

type Install = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

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
/// makes: hooks to the app, the tray item (when there is a tray), the card.
pub fn wire(app: &AppHandle, u: &Arc<upd::Updater>, tray: Option<Arc<Tray>>, n: &notify::Notifier) {
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
    let card = Arc::new(Card {
        n: n.clone(),
        install: Arc::new(move || ui.restart_to_update()),
        open: Arc::new(move || super::show(&h)),
        st: Mutex::default(),
    });
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
    fn headless_never_posts() {
        let (c, f, _) = card_with(true, no_install());
        c.on_status(&ready("0.4.0"));
        assert_eq!(f.post_count(), 0, "a headless Notifier showed the card");
    }
}
