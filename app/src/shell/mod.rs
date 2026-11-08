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
// The parsing in it is tested on the desktop too.
#[cfg(any(target_os = "android", test))]
mod call_android;
#[cfg(test)]
mod golden;
pub mod instance;
mod keep;
pub mod login;
#[cfg(target_os = "android")]
mod notify_android;
mod probe;
#[cfg(target_os = "macos")]
mod quit_mac;
mod shot;
mod sync_out;
// No tray on a phone (card #226): the same API with nothing behind it.
#[cfg_attr(mobile, path = "tray_mobile.rs")]
pub mod tray;
pub mod tray_text;
mod update;

use std::sync::{Arc, Mutex, OnceLock};

use irori_window::{Config, Window, attachments};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewWindowBuilder, WindowEvent};
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
    // Card #578: the restart Keep this device up to date made while the
    // window was hidden comes back hidden too.
    let restarted_hidden = keep::after_start(&data, version(), keep::unix_now());
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
    // Card #654: a call's microphone service and audio mode on the phone.
    #[cfg(target_os = "android")]
    let call_slot: call_android::Slot = Arc::default();
    // Handed to the Core's call service (#654); desktops need none.
    #[cfg(target_os = "android")]
    let call_platform: Option<Arc<dyn call::Platform>> =
        Some(Arc::new(call_android::Phone::new(call_slot.clone())));
    #[cfg(not(target_os = "android"))]
    let call_platform: Option<Arc<dyn call::Platform>> = None;
    let notifier = notify::Notifier::new(notify::Config {
        dir: Some(data.clone()),
        headless: probe::Probe::from_env().on(),
        toaster,
        ..notify::Config::default()
    });

    // The updater (M8), GUI role only; a dev build never checks.
    let updater = update::new(&data);
    // Keep this device up to date (card #578): the setting, and the wake
    // its switch sends the loop.
    let (every, hold, tick) = keep::timings();
    let keeper = Arc::new(keep::Keeper::new(data.clone(), hold));
    let (keep_tx, keep_rx) = std::sync::mpsc::channel::<()>();

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
    // Calls (#654): the Kotlin CallPlugin behind the phone's call::Platform.
    #[cfg(target_os = "android")]
    let app = app.plugin(call_android::init(call_slot));
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
            #[cfg(mobile)]
            sync_out::on_window(w, ev);
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
            let keeper = keeper.clone();
            let ready = ready.clone();
            let hosted = hosted.clone();
            let starting = starting.clone();
            let rt = rt.clone();
            move |app| {
                builder.mount_events(app);
                #[cfg(desktop)]
                sync_out::watch_power(app.handle());
                // The probe never puts a window on screen.
                main_window(app.handle(), startup || restarted_hidden || probe.on())?;
                if restarted_hidden {
                    irori_core::log(
                        "start: restarted by Keep this device up to date; the window stays hidden",
                    );
                }
                shot::start(app.handle());
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
                                call_platform,
                                updater,
                                keep: Keep {
                                    keeper,
                                    tx: keep_tx,
                                    rx: keep_rx,
                                    every,
                                    tick,
                                },
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
    // Everything the app does on its way out, once. After run_return is
    // too late on macOS (card #535): an Apple Event or the app menu's Quit
    // goes through -[NSApplication terminate:], which calls exit() itself
    // and never returns from the loop, so there it runs on RunEvent::Exit.
    // There the main thread no longer serves its run loop, which the
    // page's IPC answers wait on (card #588), so on macOS every quit runs
    // it first on its own thread with the loop still going (quit_mac) and
    // RunEvent::Exit finds it done.
    // On Windows (card #580) it runs on RunEvent::Exit too: a logoff or
    // shutdown (WM_ENDSESSION) makes tao call exit(0) right after it.
    let shut_down = {
        let done: Arc<OnceLock<Result<(), String>>> = Arc::default();
        let probe = probe.clone();
        move || {
            done.get_or_init(|| {
                probe.note("exiting");
                // A Core still starting finishes first, so it closes cleanly.
                if let Some(h) = starting.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    let _ = h.join();
                }
                // Card #580: the last rows leave before anything stops.
                let core = hosted
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .map(|(h, _)| h.core.clone());
                if let Some(core) = core {
                    sync_out::on_quit(&core);
                }
                // Go OnShutdown(InstallOnExit): a ready update installs
                // silently and leaves Irori closed (/NORELAUNCH).
                updater.install_on_exit();
                pumps_stop.cancel();
                let closed = match hosted.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    Some((h, window)) => {
                        // Leave any huddle and close its devices (#654),
                        // first: its transcriber (#656) holds the speech
                        // model.
                        window.call_shutdown();
                        // Free the speech model before exit (#240: Metal
                        // asserts otherwise).
                        window.voice_shutdown();
                        // Card #783: the corner chat's headless runs end.
                        window.assistant_shutdown();
                        h.core.set_reviver(None);
                        h.core.set_spawner(None);
                        h.core.set_huddle_runner(None);
                        h.close()
                    }
                    None => Ok(()),
                };
                if let Err(e) = &closed {
                    irori_core::log(&format!("irori: closing: {e}"));
                }
                probe.note("closed");
                closed
            })
            .clone()
        }
    };
    #[cfg(any(target_os = "macos", windows))]
    let on_exit = shut_down.clone();
    // The tao app delegate exists once the app is built.
    #[cfg(target_os = "macos")]
    if app.is_ok() {
        let s = shut_down.clone();
        if let Err(e) = quit_mac::install(move || {
            let _ = s();
        }) {
            irori_core::log(&format!("irori: {e}"));
        }
    }
    let res = app.map(|app| {
        app.run_return(move |_app, ev| {
            // The tray keeps Irori running with no window open: only Quit
            // (an explicit exit code) ends it.
            match ev {
                RunEvent::ExitRequested {
                    code: None, api, ..
                } => api.prevent_exit(),
                // Quit (an explicit exit code) shuts down with the loop
                // still running, then asks again (quit_mac).
                #[cfg(target_os = "macos")]
                RunEvent::ExitRequested {
                    code: Some(code),
                    api,
                    ..
                } => {
                    let app = _app.clone();
                    match quit_mac::begin(move || app.exit(code)) {
                        quit_mac::Begin::Started | quit_mac::Begin::Running => api.prevent_exit(),
                        quit_mac::Begin::Done => {}
                    }
                }
                // A Dock click on the running app (check this against the
                // Go build on the Mac).
                #[cfg(target_os = "macos")]
                RunEvent::Reopen { .. } => show(_app),
                #[cfg(any(target_os = "macos", windows))]
                RunEvent::Exit => {
                    let _ = on_exit();
                }

                _ => {}
            }
        })
    });
    let closed = shut_down();
    res.map(|_| ()).and(closed)
}

/// The started Core with its API, and the window's backend on it (card
/// #232: made on the start thread, closed after the app exits).
type Running = (host::Hosted, Arc<Window>);

/// What [`start_core`] wires once the Core is up.
struct Started {
    notifier: notify::Notifier,
    /// The phone's microphone service for calls (#654).
    call_platform: Option<Arc<dyn call::Platform>>,
    updater: Arc<::update::Updater>,
    keep: Keep,
    probe: probe::Probe,
    pumps_stop: sys::cancel::Cancel,
    launched: std::time::Instant,
}

/// Keep this device up to date (card #578): what its loop and switch need.
struct Keep {
    keeper: Arc<keep::Keeper>,
    tx: std::sync::mpsc::Sender<()>,
    rx: std::sync::mpsc::Receiver<()>,
    every: std::time::Duration,
    tick: std::time::Duration,
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
    let hosted =
        match host::options(rt, data.clone(), false, Some(s.notifier.clone())).and_then(|mut o| {
            o.call_platform = s.call_platform.clone();
            // The call's transcript (#656): Whisper on this device.
            o.call_transcriber = Some(irori_window::voice::call_transcriber(data.clone()));
            host::start_with(rt, o)
        }) {
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
    seams.updater = Arc::new(update::WindowUpdater(
        s.updater.clone(),
        s.keep.keeper.clone(),
        Mutex::new(s.keep.tx.clone()),
    ));
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
    // A phone runs no agents (card #524): with no spawner its sweep never
    // lists `claude` agents, so it doesn't log "spawn cleanup: can't find
    // the claude command" every second. Its switch's default is unchanged:
    // the coord's offers and its "starts an agent" lines read it.
    // An offline copy (IRORI_DEV_OFFLINE, a rehearsal or a window check on
    // a copy of a real data dir, card #587) never starts real agents.
    #[cfg(desktop)]
    if !crate::host::offline() {
        // Each agent's permission prompts reach its card through this
        // binary's `permission-hook` role (card #682).
        core.set_permission_hook(std::env::current_exe().ok());
        core.set_spawner(Some(spawner()));
    }
    #[cfg(not(desktop))]
    core.set_runs_no_agents();
    // Irori's assistant names the person's new threads (card #782) with a
    // Haiku turn on this computer's Claude Code. The phone has none.
    #[cfg(desktop)]
    {
        let w = Arc::downgrade(&window);
        core.set_namer(Some(Arc::new(
            move |a: &irori_core::thread_names::NameAsk| match w.upgrade() {
                Some(w) => w.name_thread(a),
                None => Err("Irori is closing".into()),
            },
        )));
        // And weighs each new card (card #965), the same way.
        let w = Arc::downgrade(&window);
        core.set_weigher(Some(Arc::new(
            move |a: &irori_core::thread_names::NameAsk| match w.upgrade() {
                Some(w) => w.weigh_card(a),
                None => Err("Irori is closing".into()),
            },
        )));
        // And holds a huddle's assistant session (card #1046): one
        // `claude -p` per huddle, kept up between looks.
        core.set_huddle_runner(Some(window.huddle_runner()));
    }
    // Spawning is on by default for the private board where agents can
    // start (card #448, decision 1 (a)); his own switch wins.
    core.set_spawn_default_on(true);
    app.manage(core.clone());
    app.manage(window.clone());
    wire_window(app, &window);
    wire_opener(app, &s.notifier);
    let keep_stop = s.pumps_stop.clone();
    irori_window::events::start(window.clone(), s.pumps_stop);

    // The tray, the launch-at-login cutover and the updater belong on the
    // main thread.
    let ui = {
        let app = app.clone();
        let window = window.clone();
        let probe = s.probe.clone();
        let updater = s.updater.clone();
        let keep = s.keep;
        let pumps_stop = keep_stop;
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
            let recheck_card =
                update::wire(&app, &updater, tray_for_update, &s.notifier, &keep.keeper);
            if !probe.on() {
                // The launch-at-login cutover (surface.md 7.4).
                if let Err(e) = login::LoginItem::system().cutover() {
                    irori_core::log(&format!("irori: launch at login cutover: {e}"));
                }
                // Go: AutoUpdate on ApplicationStarted (dev builds and
                // unsupported platforms return at once).
                let u = updater.clone();
                tauri::async_runtime::spawn(async move { u.auto_update().await });
                // Card #578: Keep this device up to date's loop (desktops;
                // a phone's update waits for the person's tap).
                if updater.supported() && !cfg!(target_os = "android") {
                    let hooks = keep_hooks(&app, &core, recheck_card);
                    if let Err(e) = keep::spawn(
                        keep.keeper.clone(),
                        updater.clone(),
                        hooks,
                        keep.rx,
                        pumps_stop.clone(),
                        keep.every,
                        keep.tick,
                    ) {
                        irori_core::log(&format!("irori: keep up to date: {e}"));
                    }
                }
            }
            // The rehearsal's launch-at-login switch-over against a
            // scratch Run key (tools/rehearse; probe only).
            probe.cutover_on_scratch_key();
            probe.spawn_from_gui();
            probe.watch_quit(&app);
            #[cfg(target_os = "macos")]
            {
                probe.watch_terminate(&app);
                probe.watch_voice(&window);
                probe.watch_window(&app);
            }
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

/// What Keep this device up to date's loop asks the app (card #578).
/// "No agent working" reads presence again first, so a turn that just
/// started counts; a window the person is using also waits.
fn keep_hooks(
    app: &AppHandle,
    core: &Arc<irori_core::Core>,
    recheck_card: Box<dyn Fn(&::update::Status) + Send>,
) -> keep::Hooks {
    let (a, c) = (app.clone(), core.clone());
    let av = app.clone();
    keep::Hooks {
        idle: Box::new(move || {
            let focused = a.get_webview_window(MAIN).is_some_and(|w| {
                w.is_focused().unwrap_or(false) && w.is_visible().unwrap_or(false)
            });
            if focused {
                return false;
            }
            c.presence().rescan();
            let cards = c.presence().snapshot();
            let open = c.terms().list().iter().filter(|t| t.2).count();
            keep::idle_now(&cards, open, keep::unix_now(), keep::SETTLE)
        }),
        visible: Box::new(move || {
            av.get_webview_window(MAIN)
                .is_some_and(|w| w.is_visible().unwrap_or(false))
        }),
        recheck_card,
    }
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
    // A window check on a Mac (card #587, shot.rs) may open a debug build
    // on a preview switch.
    let url = shot::window_url().unwrap_or_default();
    let b = WebviewWindowBuilder::new(app, MAIN, url)
        .title("Irori")
        .background_color(tauri::window::Color(11, 12, 15, 255));
    // A phone's window is the whole screen (card #226). Desktop only: iOS
    // applies inner_size to its WKWebView (Android ignores it), which drew
    // the desktop layout cut off on an iPhone (#269).
    #[cfg(desktop)]
    let b = {
        let (w, h) = shot::window_size().unwrap_or((1100.0, 720.0));
        // Tauri's handler takes a file dropped from the OS for its own
        // events and the page's drop never fires; the boxes that take
        // files read the page's (#999).
        b.inner_size(w, h)
            .center()
            .visible(!startup)
            .disable_drag_drop_handler()
    };
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
        sync_out::skip();
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

/// The roster line (Go `roster.trayStatus`), Stop everywhere (card #787;
/// the old pause checkbox is gone), and the inbox and decide
/// counts from the window's pumps (Go `inboxSvc.onCounts(tray.SetInbox)`,
/// `board.pump(ctx, app, tray.SetDecide)`), so the tray reads each pump's
/// own scan, never a snapshot the pump hasn't refreshed yet.
fn wire_tray(tray: &Arc<Tray>, core: &Arc<irori_core::Core>, window: &Arc<Window>) {
    {
        let t = tray.clone();
        window.on_inbox_counts(Arc::new(move |c| t.set_inbox(tray_counts(c))));
        let t = tray.clone();
        window.on_decide_count(Arc::new(move |n| t.set_decide(n)));
        // Stop everywhere's switch (card #787; Wind down in the code): the
        // tray's line follows it, and while on, a click starts everywhere.
        let t = tray.clone();
        window.on_wind_down(Arc::new(move |on| t.set_wind_down(on)));
        let w = window.clone();
        tray.set_wind_down_off(Arc::new(move || w.wind_down_off().map(|_| ())));
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
}

/// The tray's shape of the window's inbox counts.
fn tray_counts(c: irori_window::inbox::InboxCounts) -> tray_text::Counts {
    tray_text::Counts {
        unread: c.unread.max(0) as usize,
        held: c.held.max(0) as usize,
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
#[cfg(desktop)]
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
                settings: r.settings_file.clone(),
                brief_file: Some(r.brief_path.clone()),
                prompt: Some(r.prompt.clone()),
                env: r.env.clone(),
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
                        cwd: a.cwd,
                    })
                    .collect()
            })
        })),
        ready: Some(Arc::new(spawn::claude_installed)),
    }
}
