//! Card #580 (no-server.md §4.3 "Leave nothing stranded"): before Irori
//! quits, the device sleeps or a phone goes to the background, it sends
//! its last rows and waits a little for another device to hold them
//! ([`irori_core::Core::sync_out`]); after a wake or a phone coming back
//! it reconnects at once ([`irori_core::Core::kick_sync`]).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use irori_core::Core;
use tauri::Manager;

/// The longest a quit waits; Core::close then takes up to its own 5 s.
const QUIT: Duration = Duration::from_secs(3);
/// macOS waits up to 30 s for a will-sleep answer; Windows about 2 s.
#[cfg(all(desktop, target_os = "macos"))]
const SLEEP: Duration = Duration::from_secs(5);
#[cfg(all(desktop, not(target_os = "macos")))]
const SLEEP: Duration = Duration::from_millis(1500);
/// A phone going to the background.
#[cfg(mobile)]
const BACKGROUND: Duration = Duration::from_secs(2);

/// Set by Reset: its quit sends nothing.
static SKIP: AtomicBool = AtomicBool::new(false);

/// The next quit is Reset's.
pub fn skip() {
    SKIP.store(true, Ordering::SeqCst);
}

/// On the way out, before the Core closes.
pub fn on_quit(core: &Core) {
    if !SKIP.load(Ordering::SeqCst) {
        core.sync_out(QUIT);
    }
}

fn core_of<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Option<Arc<Core>> {
    app.try_state::<Arc<Core>>().map(|c| c.inner().clone())
}

/// Hears the OS sleep and wake for the rest of the process (desktop).
#[cfg(desktop)]
pub fn watch_power<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let app = app.clone();
    let r = sys::power::watch(Arc::new(move |c| {
        let Some(core) = core_of(&app) else { return };
        match c {
            sys::power::Change::WillSleep => {
                core.away_blur();
                core.sync_out(SLEEP);
            }
            sys::power::Change::Woke => {
                core.away_woke();
                core.kick_sync();
            }
        }
    }));
    if let Err(e) = r {
        irori_core::log(&format!("irori: sleep and wake: {e}"));
    }
}

/// The window was shown: the person came to it (card #786).
#[cfg(desktop)]
pub fn on_show<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(core) = core_of(app) {
        core.away_focus();
    }
}

/// The person coming to or leaving the desktop window (card #786).
#[cfg(desktop)]
pub fn on_attention<R: tauri::Runtime>(app: &tauri::AppHandle<R>, ev: &tauri::WindowEvent) {
    let Some(core) = core_of(app) else { return };
    match ev {
        tauri::WindowEvent::Focused(true) => core.away_focus(),
        tauri::WindowEvent::Focused(false) | tauri::WindowEvent::CloseRequested { .. } => {
            core.away_blur()
        }
        _ => {}
    }
}

/// A phone's window going to the background or coming back. The send
/// runs off the UI thread, so the app never stalls on its way out.
#[cfg(mobile)]
pub fn on_window<R: tauri::Runtime>(w: &tauri::Window<R>, ev: &tauri::WindowEvent) {
    let Some(core) = core_of(w.app_handle()) else {
        return;
    };
    match ev {
        tauri::WindowEvent::Suspended => {
            let _ = std::thread::Builder::new()
                .name("irori-sync-out".into())
                .spawn(move || {
                    core.sync_out(BACKGROUND);
                });
        }
        tauri::WindowEvent::Resumed => core.kick_sync(),
        _ => {}
    }
}
