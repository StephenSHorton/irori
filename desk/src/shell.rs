//! The window role (card #674): the Core in-process with its API, the
//! window backend (crates/window, without its Tauri commands) and a Rune
//! window on it. What the Tauri shell (app/src/shell) does that this one
//! doesn't yet is listed in the card's report: launch at login, the
//! updater and Keep this device up to date, handing a second launch to the
//! running copy, the macOS tray, calls and voice on a phone.

use std::path::Path;
use std::sync::Arc;

use irori_roles::commands::version;
use irori_roles::{datadir, host};
use irori_window::events::{EVENT_BOARD_CHANGED, EVENT_ROSTER_CHANGED};
use irori_window::{Config, Window, attachments};
use rune::tray::{Tray, TrayEvent, TrayIcon, TrayItem, TraySpec};
use rune::{App, AppSpec};

use crate::board::{self, State};

/// Rune's own flags: a run with any of them renders headless (no window,
/// no tray, no agents started), the Core still reading the data dir.
const HEADLESS_FLAGS: &[&str] = &[
    "--snap",
    "--tree",
    "--script",
    "--trace",
    "--video",
    "--out",
    "--list-scenes",
    "--strip",
    "--desktop",
];

pub fn run(args: &[String]) -> Result<(), String> {
    let launched = std::time::Instant::now();
    let headless = args.iter().any(|a| HEADLESS_FLAGS.contains(&a.as_str()));
    // `--startup` (launch at login) is Irori's; everything else is Rune's.
    let startup = args.iter().any(|a| a == "--startup" || a == "-startup");
    let rune_args: Vec<String> = args
        .iter()
        .filter(|a| *a != "--startup" && *a != "-startup")
        .cloned()
        .collect();
    if !headless {
        // Opened from Finder, the Dock or at login, the app has launchd's
        // PATH: take the login shell's before anything starts claude.
        // SAFETY: first thing in the window role, before any thread starts.
        unsafe { sys::shellpath::fix_gui_path() };
    }
    let data = datadir::process_data_dir()?;

    // Another Irori holding this data dir. The Tauri shell hands the launch
    // to it (its window shows); this one says so and stops.
    if let Ok(lf) = irori_core::lockfile::read(&data)
        && lf.pid != irori_core::lockfile::own_pid()
    {
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

    let mut seams = Window::default_seams(&core);
    seams.notifier = Arc::new(irori_window::settings::NotifierSeam(notifier.clone()));
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
    window.set_open_file(|p| open_path(p));

    let spec = AppSpec {
        scenes: &[("detail", |s: &mut State| s.select_first())],
        min_size: Some((720.0, 480.0)),
        ..AppSpec::new("Irori", (1100.0, 720.0), State::default, board::view)
    };
    let init_window = window.clone();
    let mut app = App::new(spec)
        .init(move || State::new(init_window))
        .args(rune_args);
    if startup {
        app = app.windowless();
    }
    let handle = app.handle();

    // Live: the window backend's pumps say when the Board or the roster
    // changed; the state re-reads on the UI thread.
    {
        let h = handle.clone();
        window.set_emitter(move |name, _payload| {
            if name == EVENT_BOARD_CHANGED || name == EVENT_ROSTER_CHANGED {
                h.update(|s: &mut State, _| s.refresh());
            }
        });
    }
    // A card's Open (a desktop card's click) shows the window and the card.
    {
        let h = handle.clone();
        notifier.set_opener(move |t| {
            h.open();
            if !t.ref_.is_empty() {
                let id = t.ref_.trim_start_matches("task:").to_string();
                h.update(move |s: &mut State, _| s.select(&id));
            }
        });
    }
    let pumps_stop = sys::cancel::Cancel::new();
    irori_window::events::start(window.clone(), pumps_stop.clone());

    // The tray: Rune has one on Windows and Linux. With it, closing the
    // window keeps Irori running, as the Tauri app does.
    if !headless {
        let h = handle.clone();
        let tray = Tray::new(
            TraySpec::new("irori", "Irori", tray_icon())
                .tooltip(format!("Irori {}", version()))
                .menu(vec![
                    TrayItem::action("open", "Open Irori"),
                    TrayItem::Separator,
                    TrayItem::action("quit", "Quit"),
                ]),
            move |ev| match ev {
                TrayEvent::Activate { token: Some(t) } => h.activate(t),
                TrayEvent::Activate { token: None } => h.open(),
                TrayEvent::Menu(id) if id == "open" => h.open(),
                TrayEvent::Menu(id) if id == "quit" => h.quit(),
                _ => {}
            },
        );
        match tray {
            Ok(t) => {
                app = app.keep_running();
                handle.update(move |s: &mut State, _| s.tray = Some(t));
            }
            Err(e) => irori_core::log(&format!("irori: tray: {e}")),
        }
    }

    irori_core::log(&format!(
        "start: Rune window starting {} ms after launch",
        launched.elapsed().as_millis()
    ));
    let ran = app.try_run();

    // On the way out: the last rows leave, the pumps stop, the Core closes.
    pumps_stop.cancel();
    core.sync_out(std::time::Duration::from_secs(3));
    window.call_shutdown();
    window.voice_shutdown();
    core.set_reviver(None);
    core.set_spawner(None);
    let closed = hosted.close();
    ran.and(closed)
}

/// The OS opener (the Tauri opener plugin's open_path).
fn open_path(p: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut c = std::process::Command::new("open");
    #[cfg(windows)]
    let mut c = {
        let mut c = std::process::Command::new("explorer");
        c.arg("/select,");
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut c = std::process::Command::new("xdg-open");
    c.arg(p).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// The tray's icon: the app icon's 32 px PNG, decoded.
fn tray_icon() -> TrayIcon {
    const PNG: &[u8] = include_bytes!("../../app/icons/32x32.png");
    let decoded = (|| {
        let mut dec = png::Decoder::new(std::io::Cursor::new(PNG));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
        let mut r = dec.read_info().ok()?;
        let mut buf = vec![0; r.output_buffer_size()?];
        let info = r.next_frame(&mut buf).ok()?;
        buf.truncate(info.buffer_size());
        (info.color_type == png::ColorType::Rgba).then_some((info.width, info.height, buf))
    })();
    match decoded {
        Some((w, h, px)) => TrayIcon::rgba(w, h, px),
        None => TrayIcon::rgba(1, 1, vec![0, 0, 0, 0]),
    }
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
