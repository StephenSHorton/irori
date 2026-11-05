//! The desktop app (Go `runGUI`, I:gui.go; architecture.md 4.2): the Core
//! in-process with its API, the window, the tray, single instance and
//! launch at login.
//!
//! Irori lives in the tray: closing the window hides it, and Quit is in
//! the tray menu. A `--startup` launch (launch at login) starts with the
//! window hidden. A second launch shows the first one's window and exits;
//! like Go it forwards nothing else (Go's `OnSecondInstanceLaunch` ignores
//! the second launch's arguments). Go registers no URL scheme, so there
//! are no deep links.
//!
//! The window's commands, pumps and `irori` attachments scheme are
//! crates/window's ([`irori_window::Window`]); the shell fills its
//! launch-at-login seam ([`ShellLogin`]) and feeds the tray its inbox and
//! decide counts. The tray is `State<Arc<Tray>>`.

#[cfg(target_os = "android")]
mod apk;
#[cfg(test)]
mod golden;
pub mod instance;
pub mod login;
#[cfg(target_os = "android")]
mod notify_android;
mod probe;
// No tray on a phone (card #226): the same API with nothing behind it.
#[cfg_attr(mobile, path = "tray_mobile.rs")]
pub mod tray;
pub mod tray_text;
mod update;

use std::sync::{Arc, Mutex};

use irori_window::{Config, Window, attachments};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_opener::OpenerExt;

use crate::commands::version;
use crate::{datadir, host};
use tray::Tray;

/// The main window's label.
pub const MAIN: &str = "main";

/// Go `EventLaunchAtLogin`: the live launch-at-login state after every
/// toggle, from the settings UI or the tray (one constant, the window's).
pub use irori_window::events::EVENT_LAUNCH_AT_LOGIN;

/// The one generated context (the frontend is embedded once).
fn context() -> tauri::Context<tauri::Wry> {
    #[allow(unused_mut)]
    let mut ctx = tauri::generate_context!();
    // The single-instance plugin takes its id from the identifier
    // (architecture.md 4.2); a test copy's IRORI_HOME makes it distinct.
    // A phone's identifier is its package name and never changes (#226).
    #[cfg(desktop)]
    {
        ctx.config_mut().identifier = instance::current();
    }
    ctx
}

pub fn run(args: &[String]) -> Result<(), String> {
    let launched = std::time::Instant::now();
    // Opened from Finder, the Dock or at login, the app has launchd's PATH:
    // take the login shell's before anything starts claude, git or gh.
    // SAFETY: first thing in the window role, before any thread is spawned.
    unsafe { sys::shellpath::fix_gui_path() };
    let startup = login::started_at_login(args);
    let data = datadir::process_data_dir()?;
    let rt = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?,
    );
    // One runtime for the Core, the API and Tauri (architecture.md 2).
    tauri::async_runtime::set(rt.handle().clone());

    // Another Irori holding this data dir: the same test the Core makes
    // (Core::start), made before the window opens. A desktop one gets the
    // launch handed to it (it shows its window); a headless one is an error.
    if let Ok(lf) = irori_core::lockfile::read(&data)
        && lf.pid != irori_core::lockfile::own_pid()
    {
        if !lf.headless {
            return hand_off();
        }
        return Err(format!(
            "start: {}",
            irori_core::StartError::AlreadyRunning { pid: lf.pid }
        ));
    }

    // The desktop cards, one per data dir (settings.json there). The dev
    // shell probe (automated tests) never puts a card on screen.
    // A phone's are Android notifications, for mentions and DMs only
    // (card #395).
    #[cfg(target_os = "android")]
    let phone: notify_android::Slot = Arc::default();
    #[cfg(target_os = "android")]
    let toaster: Option<Arc<dyn notify::Toaster>> =
        Some(Arc::new(notify_android::Phone(phone.clone())));
    #[cfg(not(target_os = "android"))]
    let toaster = None;
    let notifier = notify::Notifier::new(notify::Config {
        dir: Some(data.clone()),
        headless: probe::Probe::from_env().on(),
        toaster,
        ..notify::Config::default()
    });

    // The updater (M8), GUI role only; a dev build never checks.
    let updater = update::new(&data);

    // Card #232: the window opens first, with the opening screen, and the
    // Core starts on its own thread (a fully synced log takes seconds, most
    // of all on a phone). `ready` tells the page when it may mount; the
    // window's state is managed once the Core is up.
    let ready = Arc::new(irori_window::ready::Ready::new());
    let hosted: Arc<Mutex<Option<Running>>> = Arc::default();
    let starting: Arc<Mutex<Option<std::thread::JoinHandle<()>>>> = Arc::default();
    let pumps_stop = sys::cancel::Cancel::new();
    let builder = irori_window::builder();
    let invoke = builder.invoke_handler();

    let probe = probe::Probe::from_env();
    let app = tauri::Builder::default();
    // A phone runs one copy by itself (card #226).
    #[cfg(desktop)]
    let app = app.plugin(tauri_plugin_single_instance::init({
        let probe = probe.clone();
        move |app, _args, _cwd| {
            probe.note("second-instance");
            show(app);
        }
    }));
    // Android's install screen for in-app updates (card #270).
    #[cfg(target_os = "android")]
    let app = app.plugin(apk::init(updater.clone()));
    // A tapped notification opens its conversation once the page listens:
    // when it starts (a tap that launched Irori) and each time it shows
    // again (a tap that brought it back).
    #[cfg(target_os = "android")]
    let app = app.plugin(notify_android::init(phone.clone(), notifier.clone()));
    let app = app
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(ready.clone())
        .manage(probe.clone())
        .invoke_handler(invoke)
        .register_asynchronous_uri_scheme_protocol(attachments::SCHEME, |ctx, req, responder| {
            // The page asks for attachments only once it has mounted, after
            // the Core is up; until then there is nothing to serve.
            let Some(w) = ctx.app_handle().try_state::<Arc<Window>>() else {
                let resp = tauri::http::Response::builder()
                    .status(503)
                    .body(Vec::new());
                if let Ok(r) = resp {
                    responder.respond(r);
                }
                return;
            };
            let w = w.inner().clone();
            let r = attachments::Request {
                method: req.method().as_str().to_string(),
                path: req.uri().path().to_string(),
                range: header(&req, "range"),
                if_none_match: header(&req, "if-none-match"),
            };
            // A fetch from another device can take its time (M5); never
            // on the webview's thread.
            std::thread::spawn(move || {
                let resp = w.serve_attachment(&r);
                let mut b = tauri::http::Response::builder().status(resp.status);
                for (k, v) in &resp.headers {
                    b = b.header(k, v);
                }
                b = b.header("Access-Control-Allow-Origin", attachments::app_origin());
                match b.body(resp.body) {
                    Ok(r) => responder.respond(r),
                    Err(e) => irori_core::log(&format!("irori: attachment response: {e}")),
                }
            });
        })
        .on_page_load({
            let probe = probe.clone();
            move |_webview, payload| {
                if payload.event() != tauri::webview::PageLoadEvent::Finished {
                    return;
                }
                // The probe: which origin the frontend loaded from (the
                // embedded assets: http://tauri.localhost on Windows,
                // tauri://localhost on macOS). Scheme and host, not
                // origin(): a non-special scheme's origin is "null".
                if probe.on() {
                    let url = payload.url();
                    probe.note(&format!(
                        "page-load {}://{}",
                        url.scheme(),
                        url.host_str().unwrap_or("")
                    ));
                }
                irori_core::log(&format!(
                    "start: page loaded {} ms after launch",
                    launched.elapsed().as_millis()
                ));
            }
        })
        .on_window_event(|w, ev| {
            // Go hooks WindowClosing: hide and cancel (gui.go:136-148).
            if w.label() == MAIN
                && let WindowEvent::CloseRequested { api, .. } = ev
            {
                api.prevent_close();
                let _ = w.hide();
            }
        })
        .setup({
            let probe = probe.clone();
            let pumps_stop = pumps_stop.clone();
            let updater = updater.clone();
            let ready = ready.clone();
            let hosted = hosted.clone();
            let starting = starting.clone();
            let rt = rt.clone();
            move |app| {
                builder.mount_events(app);
                // The probe never puts a window on screen.
                main_window(app.handle(), startup || probe.on())?;
                irori_core::log(&format!(
                    "start: window open {} ms after launch",
                    launched.elapsed().as_millis()
                ));
                let app = app.handle().clone();
                let h = std::thread::Builder::new()
                    .name("irori-start".into())
                    .spawn(move || {
                        let started = start_core(
                            &rt,
                            &app,
                            data,
                            Started {
                                notifier,
                                updater,
                                probe,
                                pumps_stop,
                                launched,
                            },
                        );
                        match started {
                            Ok(h) => {
                                *hosted.lock().unwrap_or_else(|e| e.into_inner()) = Some(h);
                                ready.set(Ok(()));
                            }
                            Err(e) => {
                                irori_core::log(&format!("irori: {e}"));
                                ready.set(Err(e));
                            }
                        }
                    })
                    .map_err(|e| e.to_string())?;
                *starting.lock().unwrap_or_else(|e| e.into_inner()) = Some(h);
                Ok(())
            }
        })
        .build(context())
        .map_err(|e| e.to_string());
    let res = app.map(|app| {
        app.run_return(|_app, ev| {
            // The tray keeps Irori running with no window open: only Quit
            // (an explicit exit code) ends it.
            match ev {
                RunEvent::ExitRequested {
                    code: None, api, ..
                } => api.prevent_exit(),
                // A Dock click on the running app (check this against the
                // Go build on the Mac).
                #[cfg(target_os = "macos")]
                RunEvent::Reopen { .. } => show(_app),

                _ => {}
            }
        })
    });
    probe.note("exiting");
    // A Core still starting finishes first, so it closes cleanly.
    if let Some(h) = starting.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = h.join();
    }
    // Go OnShutdown(InstallOnExit): a ready update installs silently and
    // leaves Irori closed (/NORELAUNCH).
    updater.install_on_exit();
    pumps_stop.cancel();
    let closed = match hosted.lock().unwrap_or_else(|e| e.into_inner()).take() {
        Some((h, window)) => {
            // Free the speech model before exit (#240: Metal asserts otherwise).
            window.voice_shutdown();
            h.core.set_reviver(None);
            h.core.set_spawner(None);
            h.close()
        }
        None => Ok(()),
    };
    res.map(|_| ()).and(closed)
}

/// The started Core with its API, and the window's backend on it (card
/// #232: made on the start thread, closed after the app exits).
type Running = (host::Hosted, Arc<Window>);

/// What [`start_core`] wires once the Core is up.
struct Started {
    notifier: notify::Notifier,
    updater: Arc<::update::Updater>,
    probe: probe::Probe,
    pumps_stop: sys::cancel::Cancel,
    launched: std::time::Instant,
}

/// Card #232: starts the Core and its API (off the main thread), builds the
/// window's backend on it, manages both, then wires the tray, the opener
/// and the updater on the main thread.
fn start_core(
    rt: &tokio::runtime::Runtime,
    app: &AppHandle,
    data: std::path::PathBuf,
    s: Started,
) -> Result<Running, String> {
    let hosted = match host::start(rt, data.clone(), false, Some(s.notifier.clone())) {
        Ok(h) => h,
        Err(e) => return Err(format!("start: {e}")),
    };
    let core = hosted.core.clone();
    let api_port = hosted.port;
    irori_core::log(&format!(
        "irori {} desktop app (device {}, data {}), core up {} ms after launch",
        version(),
        core.device_id(),
        data.display(),
        s.launched.elapsed().as_millis()
    ));

    // The window's backend (crates/window) with the shell's login item.
    let mut seams = Window::default_seams(&core);
    seams.login = Arc::new(ShellLogin(app.clone()));
    seams.notifier = Arc::new(irori_window::settings::NotifierSeam(s.notifier.clone()));
    seams.updater = Arc::new(update::WindowUpdater(s.updater.clone()));
    let claude_dir = match host::claude_dir() {
        Ok(d) => d,
        Err(e) => {
            let _ = hosted.close();
            return Err(e);
        }
    };
    let window = Arc::new(Window::new(
        core.clone(),
        seams,
        Config {
            claude_dir,
            downloads: Box::new(attachments::downloads_dir),
        },
    ));
    {
        let w = Arc::downgrade(&window);
        core.set_reviver(Some(Arc::new(move |id: &str| match w.upgrade() {
            Some(w) => w.revive(id),
            None => Err("Irori is closing".into()),
        })));
    }
    core.set_spawner(Some(spawner()));
    // Spawning is on by default for the private board where agents can
    // start (card #448, decision 1 (a)); his own switch wins.
    core.set_spawn_default_on(true);
    app.manage(core.clone());
    app.manage(window.clone());
    wire_window(app, &window);
    wire_opener(app, &s.notifier);
    irori_window::events::start(window.clone(), s.pumps_stop);

    // The tray, the launch-at-login cutover and the updater belong on the
    // main thread.
    let ui = {
        let app = app.clone();
        let window = window.clone();
        let probe = s.probe.clone();
        let updater = s.updater.clone();
        let launched = s.launched;
        move || {
            irori_core::log(&format!(
                "start: main-thread wiring begins {} ms after launch",
                launched.elapsed().as_millis()
            ));
            let mut tray_for_update = None;
            if !probe.on() {
                match Tray::install(&app, version()) {
                    Ok(tray) => {
                        tray_for_update = Some(tray.clone());
                        wire_tray(&tray, &core, &window);
                        let n = core.notifier().clone();
                        tray.set_held_opener(Arc::new(move || {
                            n.open(notify::Target {
                                tab: notify::TAB_HELD.into(),
                                ref_: String::new(),
                            })
                        }));
                        let n = core.notifier().clone();
                        tray.set_wind_down_opener(Arc::new(move || {
                            n.open(notify::Target {
                                tab: TAB_WIND_DOWN.into(),
                                ref_: String::new(),
                            })
                        }));
                        app.manage(tray);
                    }
                    Err(e) => irori_core::log(&format!("irori: tray: {e}")),
                }
            }
            update::wire(&app, &updater, tray_for_update, &s.notifier);
            if !probe.on() {
                // The launch-at-login cutover (surface.md 7.4).
                if let Err(e) = login::LoginItem::system().cutover() {
                    irori_core::log(&format!("irori: launch at login cutover: {e}"));
                }
                // Go: AutoUpdate on ApplicationStarted (dev builds and
                // unsupported platforms return at once).
                let u = updater.clone();
                tauri::async_runtime::spawn(async move { u.auto_update().await });
            }
            // The rehearsal's launch-at-login switch-over against a
            // scratch Run key (tools/rehearse; probe only).
            probe.cutover_on_scratch_key();
            probe.spawn_from_gui();
            probe.watch_quit(&app);
            probe.note(&format!("version {}", version()));
            probe.note(&format!("api-port {api_port}"));
            probe.note("ready");
            irori_core::log(&format!(
                "start: main-thread wiring done {} ms after launch",
                launched.elapsed().as_millis()
            ));
        }
    };
    if let Err(e) = app.run_on_main_thread(ui) {
        irori_core::log(&format!("irori: wiring the tray: {e}"));
    }
    irori_core::log(&format!(
        "start: window backend ready {} ms after launch",
        s.launched.elapsed().as_millis()
    ));
    Ok((hosted, window))
}

/// Go `handOff`: a bare app with the same single-instance id, so the
/// plugin passes the launch to the running copy and exits this process.
/// Reaching the end means no copy answered (a Go GUI holds the Core: the
/// two builds' locks differ, surface.md 7.18), so there is nothing to show.
fn hand_off() -> Result<(), String> {
    let app = tauri::Builder::default();
    #[cfg(desktop)]
    let app = app.plugin(tauri_plugin_single_instance::init(|_, _, _| {}));
    let app = app.build(context()).map_err(|e| e.to_string())?;
    drop(app);
    irori_core::log("irori: another Irori holds this data dir; nothing to hand the launch to");
    Ok(())
}

/// The window (Go gui.go:136-148): title `Irori` (hidden in the macOS
/// title bar), 1100×720, URL `/`; hidden at a login launch. The background
/// is the opening screen's (index.html, card #232), so nothing flashes.
fn main_window(app: &AppHandle, startup: bool) -> tauri::Result<()> {
    let b = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::default())
        .title("Irori")
        .background_color(tauri::window::Color(11, 12, 15, 255));
    // A phone's window is the whole screen (card #226). Desktop only: iOS
    // applies inner_size to its WKWebView (Android ignores it), which drew
    // the desktop layout cut off on an iPhone (#269).
    #[cfg(desktop)]
    let b = b.inner_size(1100.0, 720.0).center().visible(!startup);
    #[cfg(mobile)]
    let _ = startup;
    // macOS draws the title in the title bar, where it is hard to read and
    // repeats the header's Irori (#76). Hide it there; the window keeps the
    // title for the Dock, the Window menu and Mission Control.
    #[cfg(target_os = "macos")]
    let b = b.hidden_title(true);
    // A test copy keeps its WebView2 profile in its own data dir instead
    // of %LOCALAPPDATA%\<instance id>.
    #[cfg(windows)]
    let b = match std::env::var_os("IRORI_HOME").filter(|h| !h.is_empty()) {
        Some(h) => b.data_directory(std::path::PathBuf::from(h).join("webview")),
        None => b,
    };
    b.build()?;
    Ok(())
}

/// Go's `show`: show, unminimise and focus the main window. The tray's
/// Open, its left click, a second launch and a card's Open all use it.
pub fn show(app: &AppHandle) {
    if let Some(p) = app.try_state::<probe::Probe>()
        && p.on()
    {
        // Automated tests never put a window on screen.
        p.note("show");
        return;
    }
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.show();
        #[cfg(desktop)]
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn header(req: &tauri::http::Request<Vec<u8>>, name: &str) -> String {
    req.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

/// The window's hooks into the app: events, the OS opener, and Reset
/// quitting the app.
fn wire_window(app: &AppHandle, window: &Arc<Window>) {
    let h = app.clone();
    window.set_emitter(move |name, payload| {
        let _ = h.emit(name, payload);
    });
    let h = app.clone();
    window.set_open_file(move |p| {
        h.opener()
            .open_path(p.to_string_lossy(), None::<&str>)
            .map_err(|e| e.to_string())
    });
    let h = app.clone();
    window.set_on_reset(move || {
        let h = h.clone();
        // The call returns to the window first.
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            h.exit(0);
        });
    });
}

/// A card's Open (Go `notifier.SetOpener`): show the window, then tell
/// the page which tab and ref to open (`app:open`).
/// app:open's tab for the window's Wind down dialog (card #220).
pub const TAB_WIND_DOWN: &str = "wind-down";

fn wire_opener(app: &AppHandle, n: &notify::Notifier) {
    let h = app.clone();
    n.set_opener(move |t| {
        show(&h);
        let _ = h.emit(
            irori_window::events::EVENT_APP_OPEN,
            irori_window::settings::Target {
                tab: t.tab,
                ref_: t.ref_,
            },
        );
    });
}

/// The roster line, the pause checkbox (Go `roster.trayStatus`,
/// `tray.SetPause(.., inboxSvc.SetPaused)`), and the inbox and decide
/// counts from the window's pumps (Go `inboxSvc.onCounts(tray.SetInbox)`,
/// `board.pump(ctx, app, tray.SetDecide)`), so the tray reads each pump's
/// own scan, never a snapshot the pump hasn't refreshed yet.
fn wire_tray(tray: &Arc<Tray>, core: &Arc<irori_core::Core>, window: &Arc<Window>) {
    {
        let t = tray.clone();
        window.on_inbox_counts(Arc::new(move |c| t.set_inbox(tray_counts(c))));
        let t = tray.clone();
        window.on_decide_count(Arc::new(move |n| t.set_decide(n)));
    }
    {
        let tray = tray.clone();
        let core = core.clone();
        std::thread::spawn(move || {
            let changes = core.changes();
            loop {
                let agents = core.local_agents();
                let busy = agents.iter().filter(|a| a.status == "busy").count();
                tray.set_status(&tray_text::agents_line(agents.len(), busy));
                if changes.recv().is_none() {
                    return;
                }
            }
        });
    }
    // InboxService.SetPaused: drops the window's snapshot and nudges its
    // pump, so the window's banner and the tray both follow.
    let w = window.clone();
    tray.set_pause(
        &core.device().name,
        Arc::new(move |paused| w.inbox_set_paused(paused)),
    );
}

/// The tray's shape of the window's inbox counts.
fn tray_counts(c: irori_window::inbox::InboxCounts) -> tray_text::Counts {
    tray_text::Counts {
        unread: c.unread.max(0) as usize,
        held: c.held.max(0) as usize,
        paused: c.paused,
    }
}

/// Go `setLaunchAtLogin`: changes the registration, then announces the
/// live state either way (so a failed toggle snaps back) and rebuilds the
/// tray.
pub fn set_launch_at_login(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let item = login::LoginItem::system();
    let res = item.set(enabled);
    if let Ok(now) = item.is_enabled() {
        let _ = app.emit(EVENT_LAUNCH_AT_LOGIN, now);
    }
    rebuild_tray(app);
    res
}

fn rebuild_tray(app: &AppHandle) {
    if let Some(t) = app.try_state::<Arc<Tray>>() {
        t.rebuild();
    }
}

/// The window's launch-at-login seam on the Wails registrations. The
/// window's SettingsService commands announce the live state themselves;
/// this rebuilds the tray's checkbox after a change.
struct ShellLogin(AppHandle);

impl irori_window::seams::LoginItem for ShellLogin {
    fn enabled(&self) -> Result<bool, String> {
        login::LoginItem::system().is_enabled()
    }
    fn set_enabled(&self, on: bool) -> Result<(), String> {
        let res = login::LoginItem::system().set(on);
        rebuild_tray(&self.0);
        res
    }
}

/// The Core's spawner (spawned agents part 1 and 2, cards #445 and #446):
/// `claude --bg` from this GUI process, with no console window, its trust
/// check, and `claude stop` then `claude rm` to end one.
fn spawner() -> irori_core::spawn::Spawner {
    use irori_core::spawn::{SpawnReq, SpawnStarted, Spawner};
    use irori_window::spawn;
    let opt = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
    Spawner {
        spawn: Arc::new(move |r: &SpawnReq| {
            let got = spawn::spawn(&spawn::SpawnSpec {
                cwd: std::path::PathBuf::from(&r.folder),
                name: r.name.clone(),
                permission_mode: r.permission_mode.clone(),
                model: opt(&r.model),
                effort: opt(&r.effort),
                worktree: r.worktree.clone(),
                settings: r.settings_file.clone(),
                brief_file: Some(r.brief_path.clone()),
                prompt: Some(r.prompt.clone()),
                ..Default::default()
            })?;
            Ok(SpawnStarted {
                name: got.name,
                prefix: got.short_id,
            })
        }),
        end: Arc::new(|id: &str| spawn::end(id)),
        trusted: Arc::new(|p: &std::path::Path| spawn::trust_ok(p)),
        list: Some(Arc::new(|| {
            spawn::list_agents().map(|v| {
                v.into_iter()
                    .map(|a| irori_core::spawn::ClaudeAgent {
                        session_id: a.session_id,
                        live: a.live,
                    })
                    .collect()
            })
        })),
    }
}
