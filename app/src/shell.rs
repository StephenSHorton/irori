//! The window role (card #674): the Core in-process with its API, the
//! window backend (crates/window, without its Tauri commands) and a Rune
//! window on it, with what the Tauri shell had around it: the tray, launch
//! at login (app/src/login/), the updater and Keep this device up to date
//! (status/updater.rs, status/keep.rs), Reset quitting the app, and the
//! window's shortcuts (ui/keys.rs).

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::Duration;

use irori_roles::commands::version;
use irori_roles::{datadir, host};
use irori_window::voice::EVENT_VOICE;
use irori_window::{Config, Window, attachments};
use rune::{App, AppHandle, AppSpec};

use crate::login::LoginItem;
use crate::status::keep::{self, Keeper};
use crate::status::tray::Tray;
use crate::status::{tray_text, updater};
use crate::ui::{self, Event, State};

/// The last rows' send when the app quits (the Tauri shell's).
const QUIT: Duration = Duration::from_secs(3);
/// macOS waits up to 30 s for a will-sleep answer; Windows about 2 s.
#[cfg(target_os = "macos")]
const SLEEP: Duration = Duration::from_secs(5);
#[cfg(not(target_os = "macos"))]
const SLEEP: Duration = Duration::from_millis(1500);

/// Everything the app does on its way out, once (see [`on_exit`]).
static SHUTDOWN: OnceLock<Box<dyn Fn() -> Result<(), String> + Send + Sync>> = OnceLock::new();
/// Set by Reset: its quit sends nothing (main's sync_out::skip).
static RESET: AtomicBool = AtomicBool::new(false);
/// The window has focus (App::on_focus), for Keep this device up to date,
/// which never restarts under the person's hands.
static FOCUSED: AtomicBool = AtomicBool::new(false);
/// The tray's controller, once there is one: launch at login rebuilds it.
static TRAY: OnceLock<Arc<Tray>> = OnceLock::new();

/// The tab a notifier Target opens the Wind down dialog on (card #220).
const TAB_WIND_DOWN: &str = "wind-down";

/// macOS: Cmd-Q, the Dock's Quit and a logout go through AppKit's
/// `terminate:`, which calls exit() itself, so `try_run` never returns (the
/// Tauri shell's card #535). The shutdown then runs from the C exit hooks,
/// with every thread still alive. Other platforms take it too, once.
extern "C" fn on_exit() {
    if let Some(f) = SHUTDOWN.get()
        && let Err(e) = f()
    {
        irori_core::log(&format!("irori: closing: {e}"));
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let launched = std::time::Instant::now();
    // `--startup` (launch at login) is Irori's; everything else is Rune's.
    let startup = crate::login::started_at_login(args) || args.iter().any(|a| a == "-startup");
    let rune_args: Vec<String> = args
        .iter()
        .filter(|a| *a != "--startup" && *a != "-startup")
        .cloned()
        .collect();
    // Rune decides what is headless (a render, a script, a listing), and
    // answers `--help` here, before the Core starts.
    let headless = rune::tooling::Args::parse_from(rune_args.clone()).headless();
    if !headless {
        // Opened from Finder, the Dock or at login, the app has launchd's
        // PATH: take the login shell's before anything starts claude.
        // SAFETY: first thing in the window role, before any thread starts.
        unsafe { sys::shellpath::fix_gui_path() };
    }
    let data = datadir::process_data_dir()?;
    // Card #578: the restart Keep this device up to date made while the
    // window was hidden comes back hidden too.
    let restarted_hidden = !headless && keep::after_start(&data, version(), keep::unix_now());

    // Another Irori holding this data dir: a window gets the launch handed
    // to it (its window shows, instance.rs); a headless one is an error.
    if let Ok(lf) = irori_core::lockfile::read(&data)
        && lf.pid != irori_core::lockfile::own_pid()
    {
        if !lf.headless && !headless && crate::instance::hand_off(&data) {
            return Ok(());
        }
        return Err(format!(
            "another Irori (pid {}) holds {}; quit it first",
            lf.pid,
            data.display()
        ));
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let notifier = notify::Notifier::new(notify::Config {
        dir: Some(data.clone()),
        headless,
        ..notify::Config::default()
    });
    let mut opts = host::options(&rt, data.clone(), false, Some(notifier.clone()))
        .map_err(|e| format!("start: {e}"))?;
    // The call's transcript (#656): Whisper on this device.
    opts.call_transcriber = Some(irori_window::voice::call_transcriber(data.clone()));
    let hosted = host::start_with(&rt, opts).map_err(|e| format!("start: {e}"))?;
    let core = hosted.core.clone();
    irori_core::log(&format!(
        "irori {} desktop app, Rune window (device {}, data {}), core up {} ms after launch",
        version(),
        core.device_id(),
        data.display(),
        launched.elapsed().as_millis()
    ));

    // The updater (GUI role only; a dev build never checks) and Keep this
    // device up to date (card #578): the setting, and the wake its switch
    // sends the loop.
    let updater = updater::new(&data);
    let (every, hold, tick) = keep::timings();
    let keeper = Arc::new(Keeper::new(data.clone(), hold));
    let (keep_tx, keep_rx) = mpsc::channel::<()>();

    let mut seams = Window::default_seams(&core);
    seams.notifier = Arc::new(irori_window::settings::NotifierSeam(notifier.clone()));
    seams.updater = Arc::new(updater::WindowUpdater {
        u: updater.clone(),
        keeper: keeper.clone(),
        wake: Mutex::new(keep_tx),
        rt: rt.handle().clone(),
    });
    seams.login = Arc::new(ShellLogin);
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
    // A headless render or an offline copy never starts real agents.
    if !headless && !host::offline() {
        core.set_spawner(Some(spawner()));
    }
    core.set_spawn_default_on(true);
    window.set_open_file(open_path);

    let spec = AppSpec {
        scenes: SCENES,
        min_size: Some((720.0, 480.0)),
        poll: Some(ui::poll),
        on_key: Some(ui::keys::on_key),
        ..AppSpec::new("Irori", (1100.0, 720.0), State::default, ui::view)
    };
    let init_window = window.clone();
    let init_data = data.clone();
    let mut app = App::new(spec)
        .init(move || State::new(init_window, init_data, headless))
        // macOS: Edit (undo, copy, paste) and Window (minimize, close).
        .standard_menus()
        .on_focus(|s: &mut State, on, _| {
            s.focused = on;
            FOCUSED.store(on, Ordering::Relaxed);
            if on {
                // The React app re-read on focus; the pumps are the main
                // feed, this covers a missed one.
                let w = s.w.clone();
                s.data.refresh_all(w.as_deref());
            }
        })
        .args(rune_args);
    if startup || restarted_hidden {
        if restarted_hidden {
            irori_core::log("start: restarted by Keep this device up to date; the window stays hidden");
        }
        app = app.windowless();
    }
    let handle = app.handle();

    // Reset (Settings > Devices) marks the device, then the app quits
    // without sending its last rows.
    {
        let h = handle.clone();
        window.set_on_reset(move || {
            RESET.store(true, Ordering::SeqCst);
            let h = h.clone();
            // The call returns to the window first.
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                h.quit();
            });
        });
    }

    // The state's own handle, for State::spawn.
    {
        let h = handle.clone();
        handle.update(move |s: &mut State, _| s.handle = Some(h));
    }
    // Live: every event the window backend emits goes to the packages
    // (ui::dispatch) on the UI thread. A burst of one event is one
    // dispatch with its newest payload; dictation's words each go.
    {
        let h = handle.clone();
        // IRORI_DESK_TRACE=1 logs each dispatch (the spike's live check).
        let trace = std::env::var_os("IRORI_DESK_TRACE").is_some();
        let pending: Arc<Mutex<HashMap<String, serde_json::Value>>> = Arc::default();
        window.set_emitter(move |name, payload| {
            if name == EVENT_VOICE {
                let ev = Event {
                    name: name.to_string(),
                    payload,
                };
                h.update(move |s: &mut State, _| ui::dispatch(s, &ev));
                return;
            }
            let first = {
                let mut p = pending.lock().unwrap_or_else(|e| e.into_inner());
                p.insert(name.to_string(), payload).is_none()
            };
            if !first {
                return;
            }
            let pending = pending.clone();
            let name = name.to_string();
            h.update(move |s: &mut State, _| {
                let payload = pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&name)
                    .unwrap_or_default();
                let ev = Event { name, payload };
                ui::dispatch(s, &ev);
                if trace {
                    irori_core::log(&format!(
                        "desk: {}: {} cards, open card thread {}",
                        ev.name,
                        s.data.cards.len(),
                        s.card.detail.as_ref().map_or(0, |d| d.thread.len())
                    ));
                }
            });
        });
    }
    // A card's Open (a desktop card's click) shows the window and the card.
    {
        let h = handle.clone();
        notifier.set_opener(move |t| {
            h.open();
            let (tab, r) = (t.tab.clone(), t.ref_.clone());
            h.update(move |s: &mut State, _| s.open_target(&tab, &r));
        });
    }
    let pumps_stop = sys::cancel::Cancel::new();
    irori_window::events::start(window.clone(), pumps_stop.clone());

    // Sleep sends this device's newest rows first; waking syncs again.
    if !headless {
        let c = core.clone();
        let r = sys::power::watch(Arc::new(move |ch| match ch {
            sys::power::Change::WillSleep => {
                c.sync_out(SLEEP);
            }
            sys::power::Change::Woke => c.kick_sync(),
        }));
        if let Err(e) = r {
            irori_core::log(&format!("irori: sleep and wake: {e}"));
        }
    }

    // A second launch on this data dir shows this window.
    let instance = if headless {
        None
    } else {
        let h = handle.clone();
        crate::instance::listen(&data, move || h.open())
            .map_err(|e| irori_core::log(&format!("irori: single instance: {e}")))
            .ok()
    };

    // The tray: closing the window keeps Irori running, as the Tauri app
    // does; Quit is in its menu.
    let tray = if headless {
        None
    } else {
        install_tray(&handle, &core, &window)
    };
    if tray.is_some() {
        app = app.keep_running();
    }

    // The updater: update:status to the window, the tray's Restart item,
    // the Install card; then the start check and Keep this device up to
    // date's loop.
    let recheck_card = updater::wire(&handle, &window, &updater, tray, &notifier, &keeper);
    if !headless {
        // The launch-at-login cutover (surface.md 7.4; Windows only).
        std::thread::spawn(|| {
            if let Err(e) = LoginItem::system().cutover() {
                irori_core::log(&format!("irori: launch at login cutover: {e}"));
            }
        });
        // Go: AutoUpdate on ApplicationStarted (dev builds and unsupported
        // platforms return at once).
        let u = updater.clone();
        rt.spawn(async move { u.auto_update().await });
        if updater.supported() {
            let hooks = keep_hooks(&handle, &core, recheck_card);
            if let Err(e) = keep::spawn(
                keeper.clone(),
                updater.clone(),
                rt.handle().clone(),
                hooks,
                keep_rx,
                pumps_stop.clone(),
                every,
                tick,
            ) {
                irori_core::log(&format!("irori: keep up to date: {e}"));
            }
        }
    }

    irori_core::log(&format!(
        "start: Rune window starting {} ms after launch",
        launched.elapsed().as_millis()
    ));
    // On the way out: the last rows leave, the pumps stop, the Core
    // closes, the port file goes. Once, from whichever way out comes first.
    let left = Mutex::new(Some((hosted, instance)));
    let _ = SHUTDOWN.set(Box::new(move || {
        let Some((hosted, instance)) = left.lock().unwrap_or_else(|e| e.into_inner()).take() else {
            return Ok(());
        };
        // Card #580: the last rows leave before anything stops; Reset's
        // quit sends nothing.
        if !RESET.load(Ordering::SeqCst) {
            core.sync_out(QUIT);
        }
        // Go OnShutdown(InstallOnExit): a ready update installs silently
        // and leaves Irori closed.
        updater.install_on_exit();
        pumps_stop.cancel();
        window.call_shutdown();
        window.voice_shutdown();
        core.set_reviver(None);
        core.set_spawner(None);
        drop(instance);
        hosted.close()
    }));
    // SAFETY: registers a plain extern "C" fn with no arguments.
    unsafe { libc::atexit(on_exit) };
    let ran = app.try_run();
    let closed = SHUTDOWN.get().map_or(Ok(()), |f| f());
    ran.and(closed)
}

/// The headless scenes: one per page (`irori --snap x.png --scene talk`).
const SCENES: &[(&str, fn(&mut State))] = &[
    ("home", crate::home::scene),
    ("board", crate::board::scene),
    ("card", crate::card::scene),
    // The spike's name for the card scene.
    ("detail", crate::card::scene),
    ("canvas", crate::canvas::scene),
    ("talk", crate::talk::scene),
    ("call", crate::call::scene),
    ("spaces", crate::spaces::scene),
    ("notes", crate::notes::scene),
    ("settings", crate::settings::scene),
    ("profile", crate::profile::scene),
    ("notifications", crate::inbox::scene),
    ("palette", crate::palette::scene),
    ("desk", crate::desk::scene),
    ("term", crate::term::scene),
    ("status", crate::status::scene),
    ("text", crate::text::scene),
    ("widgets", crate::widgets::scene),
];

/// The OS opener (the Tauri opener plugin's open_path): opens the file
/// with its default app. Windows: Explorer given a file opens it (with
/// `/select,` it would only show it in a folder).
fn open_path(p: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut c = std::process::Command::new("open");
    #[cfg(windows)]
    let mut c = std::process::Command::new("explorer");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut c = std::process::Command::new("xdg-open");
    c.arg(p).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Makes the tray and wires it: the roster line, the inbox and decide
/// counts, the pause checkbox, the held and Wind down openers, launch at
/// login. Windows and Linux: Rune's `Tray`, made here. macOS: tray-icon's,
/// made on the main thread once the loop runs (tray_mac.rs). Its
/// controller when a tray is (or will be) there.
fn install_tray(
    handle: &AppHandle<State>,
    core: &Arc<irori_core::Core>,
    window: &Arc<Window>,
) -> Option<Arc<Tray>> {
    let tray = Tray::new(handle.clone(), version());
    if !install_native(handle, &tray) {
        return None;
    }
    let _ = TRAY.set(tray.clone());
    // The inbox and decide counts, from the window's pumps, so the tray
    // reads each pump's own scan.
    {
        let t = tray.clone();
        window.on_inbox_counts(Arc::new(move |c| {
            t.set_inbox(tray_text::Counts {
                unread: c.unread.max(0) as usize,
                held: c.held.max(0) as usize,
                paused: c.paused,
            })
        }));
        let t = tray.clone();
        window.on_decide_count(Arc::new(move |n| t.set_decide(n)));
    }
    // The roster line (Go `roster.trayStatus`), on every presence change.
    {
        let t = tray.clone();
        let core = core.clone();
        std::thread::Builder::new()
            .name("irori-tray-status".into())
            .spawn(move || {
                let changes = core.changes();
                loop {
                    let agents = core.local_agents();
                    let busy = agents.iter().filter(|a| a.status == "busy").count();
                    t.set_status(&tray_text::agents_line(agents.len(), busy));
                    if changes.recv().is_none() {
                        return;
                    }
                }
            })
            .ok();
    }
    // Pause wakes: drops the window's snapshot and nudges its pump, so the
    // window and the tray both follow.
    let w = window.clone();
    tray.set_pause(&core.device().name, Arc::new(move |paused| w.inbox_set_paused(paused)));
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
    // Through the window, so settings:launch-at-login tells the page.
    let w = Arc::downgrade(window);
    tray.set_login_setter(Arc::new(move |on| match w.upgrade() {
        Some(w) => w.set_launch_at_login(on),
        None => Err("Irori is closing".into()),
    }));
    Some(tray)
}

/// The native icon. True when it is (or will be) there.
#[cfg(target_os = "macos")]
fn install_native(handle: &AppHandle<State>, tray: &Arc<Tray>) -> bool {
    crate::tray_mac::route(tray.clone());
    let t = tray.clone();
    // The menu's lines read the OS (launch at login): here, not on the
    // main thread.
    std::thread::spawn(move || {
        let lines = t.lines();
        t.app_update(move |s: &mut State| match crate::tray_mac::install(version(), &lines) {
            Ok(icon) => s.tray = Some(Box::new(icon)),
            Err(e) => irori_core::log(&format!("irori: tray: {e}")),
        });
    });
    let _ = handle;
    true
}

/// The native icon. True when it is there.
#[cfg(not(target_os = "macos"))]
fn install_native(handle: &AppHandle<State>, tray: &Arc<Tray>) -> bool {
    use rune::tray::{TrayEvent, TrayIcon, TraySpec};
    // Windows: the 囲 logo on a tight white tile (main's trayicon.png).
    let Some((w, ht, px)) = decode_png(include_bytes!("../icons/trayicon.png")) else {
        return false;
    };
    let h = handle.clone();
    let ctl = Arc::downgrade(tray);
    let native = rune::tray::Tray::new(
        TraySpec::new("irori", "Irori", TrayIcon::rgba(w, ht, px))
            .tooltip(format!("Irori {}", version()))
            .menu(crate::status::tray::items(&tray.lines())),
        move |ev| match ev {
            // A left click opens the window (Go trayClickOpensWindow).
            TrayEvent::Activate { token: Some(t) } => h.activate(t),
            TrayEvent::Activate { token: None } => h.open(),
            TrayEvent::Menu(id) => {
                if let Some(c) = ctl.upgrade() {
                    c.clicked(&id);
                }
            }
            _ => {}
        },
    );
    match native {
        Ok(t) => {
            handle.update(move |s: &mut State, _| s.tray = Some(Box::new(t)));
            true
        }
        Err(e) => {
            irori_core::log(&format!("irori: tray: {e}"));
            false
        }
    }
}

/// What Keep this device up to date's loop asks the app (card #578). "No
/// agent working" reads presence again first, so a turn that just started
/// counts; a window the person is using also waits.
fn keep_hooks(
    handle: &AppHandle<State>,
    core: &Arc<irori_core::Core>,
    recheck_card: Box<dyn Fn(&::update::Status) + Send>,
) -> keep::Hooks {
    let (h, c) = (handle.clone(), core.clone());
    let hv = handle.clone();
    keep::Hooks {
        idle: Box::new(move || {
            if FOCUSED.load(Ordering::Relaxed) && h.is_visible() {
                return false;
            }
            c.presence().rescan();
            let cards = c.presence().snapshot();
            let open = c.terms().list().iter().filter(|t| t.2).count();
            keep::idle_now(&cards, open, keep::unix_now(), keep::SETTLE)
        }),
        visible: Box::new(move || hv.is_visible()),
        recheck_card,
    }
}

/// The window's launch-at-login seam on the OS registration (login/).
/// Window::set_launch_at_login announces the live state itself; this
/// rebuilds the tray's checkbox after a change.
struct ShellLogin;

impl irori_window::seams::LoginItem for ShellLogin {
    fn enabled(&self) -> Result<bool, String> {
        LoginItem::system().is_enabled()
    }
    fn set_enabled(&self, on: bool) -> Result<(), String> {
        let res = LoginItem::system().set(on);
        if let Some(t) = TRAY.get() {
            t.rebuild();
        }
        res
    }
}

/// A PNG as RGBA pixels: (width, height, pixels).
pub(crate) fn decode_png(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    (info.color_type == png::ColorType::Rgba).then_some((info.width, info.height, buf))
}

/// The Core's spawner: `claude` sessions for cards (the Tauri shell's).
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
