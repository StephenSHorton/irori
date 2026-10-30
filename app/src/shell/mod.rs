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

#[cfg(test)]
mod golden;
pub mod instance;
pub mod login;
mod probe;
pub mod tray;
pub mod tray_text;
mod update;

use std::sync::{Arc, OnceLock};

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
    let mut ctx = tauri::generate_context!();
    // The single-instance plugin takes its id from the identifier
    // (architecture.md 4.2); a test copy's IRORI_HOME makes it distinct.
    ctx.config_mut().identifier = instance::current();
    ctx
}

pub fn run(args: &[String]) -> Result<(), String> {
    let startup = login::started_at_login(args);
    let data = datadir::process_data_dir()?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    // One runtime for the Core, the API and Tauri (architecture.md 2).
    tauri::async_runtime::set(rt.handle().clone());

    // The desktop cards, one per data dir (settings.json there). The dev
    // shell probe (automated tests) never puts a card on screen.
    let notifier = notify::Notifier::new(notify::Config {
        dir: Some(data.clone()),
        headless: probe::Probe::from_env().on(),
        ..notify::Config::default()
    });
    let hosted = match host::start(&rt, data.clone(), false, Some(notifier.clone())) {
        Ok(h) => h,
        Err(host::HostError::Start(e @ irori_core::StartError::AlreadyRunning { .. })) => {
            // Another desktop Irori holds the Core: hand the launch to it
            // (it shows its window) and exit. A headless one is an error.
            if let Ok(lf) = irori_core::lockfile::read(&data)
                && !lf.headless
            {
                return hand_off();
            }
            return Err(format!("start: {e}"));
        }
        Err(e) => return Err(format!("start: {e}")),
    };
    let core = hosted.core.clone();
    let api_port = hosted.port;
    irori_core::log(&format!(
        "irori {} desktop app (device {}, data {})",
        version(),
        core.device_id(),
        data.display()
    ));

    // The window's backend (crates/window) with the shell's login item.
    let handle: Arc<OnceLock<AppHandle>> = Arc::default();
    let mut seams = Window::default_seams(&core);
    seams.login = Arc::new(ShellLogin(handle.clone()));
    seams.notifier = Arc::new(irori_window::settings::NotifierSeam(notifier.clone()));
    // The updater (M8), GUI role only; a dev build never checks.
    let updater = update::new(&data);
    seams.updater = Arc::new(update::WindowUpdater(updater.clone()));
    let window = Arc::new(Window::new(
        core.clone(),
        seams,
        Config {
            claude_dir: host::claude_dir()?,
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
    let pumps_stop = sys::cancel::Cancel::new();
    let builder = irori_window::builder();
    let invoke = builder.invoke_handler();

    let probe = probe::Probe::from_env();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init({
            let probe = probe.clone();
            move |app, _args, _cwd| {
                probe.note("second-instance");
                show(app);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(core.clone())
        .manage(window.clone())
        .manage(probe.clone())
        .invoke_handler(invoke)
        .register_asynchronous_uri_scheme_protocol(attachments::SCHEME, {
            let w = window.clone();
            move |_ctx, req, responder| {
                let w = w.clone();
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
            }
        })
        .on_page_load({
            let probe = probe.clone();
            move |_webview, payload| {
                // The probe: which origin the frontend loaded from (the
                // embedded assets: http://tauri.localhost on Windows,
                // tauri://localhost on macOS). Scheme and host, not
                // origin(): a non-special scheme's origin is "null".
                if probe.on() && payload.event() == tauri::webview::PageLoadEvent::Finished {
                    let url = payload.url();
                    probe.note(&format!(
                        "page-load {}://{}",
                        url.scheme(),
                        url.host_str().unwrap_or("")
                    ));
                }
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
            let core = core.clone();
            let probe = probe.clone();
            let window = window.clone();
            let pumps_stop = pumps_stop.clone();
            let updater = updater.clone();
            move |app| {
                let _ = handle.set(app.handle().clone());
                builder.mount_events(app);
                wire_window(app.handle(), &window);
                wire_opener(app.handle(), &notifier);
                irori_window::events::start(window.clone(), pumps_stop);
                // The probe never puts a window on screen.
                main_window(app.handle(), startup || probe.on())?;
                let mut tray_for_update = None;
                if !probe.on() {
                    let tray = Tray::install(app.handle(), version())?;
                    tray_for_update = Some(tray.clone());
                    wire_tray(&tray, &core, &window);
                    let n = core.notifier().clone();
                    tray.set_held_opener(Arc::new(move || {
                        n.open(notify::Target {
                            tab: notify::TAB_HELD.into(),
                            ref_: String::new(),
                        })
                    }));
                    app.manage(tray);
                }
                update::wire(app.handle(), &updater, tray_for_update, &notifier);
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
                probe.watch_quit(app.handle());
                probe.note(&format!("version {}", version()));
                probe.note(&format!("api-port {api_port}"));
                probe.note("ready");
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
    // Go OnShutdown(InstallOnExit): a ready update installs silently and
    // leaves Irori closed (/NORELAUNCH).
    updater.install_on_exit();
    pumps_stop.cancel();
    core.set_reviver(None);
    let closed = hosted.close();
    res.map(|_| ()).and(closed)
}

/// Go `handOff`: a bare app with the same single-instance id, so the
/// plugin passes the launch to the running copy and exits this process.
/// Reaching the end means no copy answered (a Go GUI holds the Core: the
/// two builds' locks differ, surface.md 7.18), so there is nothing to show.
fn hand_off() -> Result<(), String> {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_, _, _| {}))
        .build(context())
        .map_err(|e| e.to_string())?;
    drop(app);
    irori_core::log("irori: another Irori holds this data dir; nothing to hand the launch to");
    Ok(())
}

/// The window (Go gui.go:136-148): title `Irori`, 1100×720, background
/// RGB(233,238,248) so it doesn't flash, URL `/`; hidden at a login launch.
fn main_window(app: &AppHandle, startup: bool) -> tauri::Result<()> {
    let b = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::default())
        .title("Irori")
        .inner_size(1100.0, 720.0)
        .background_color(tauri::window::Color(233, 238, 248, 255))
        .center()
        .visible(!startup);
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
struct ShellLogin(Arc<OnceLock<AppHandle>>);

impl irori_window::seams::LoginItem for ShellLogin {
    fn enabled(&self) -> Result<bool, String> {
        login::LoginItem::system().is_enabled()
    }
    fn set_enabled(&self, on: bool) -> Result<(), String> {
        let res = login::LoginItem::system().set(on);
        if let Some(app) = self.0.get() {
            rebuild_tray(app);
        }
        res
    }
}
