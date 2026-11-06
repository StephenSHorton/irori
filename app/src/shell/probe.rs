//! A hook for the shell's end-to-end test (app/tests/shell_exe.rs) and the
//! rehearsal's update leg (tools/rehearse).
//!
//! With `IRORI_DEV_SHELL_LOG=<file>` set, the window role appends one line
//! per shell event to that file (`ready`, `second-instance`, `show`,
//! `page-load <scheme>://<host>`, `api-port <port>`, `exiting`, `closed`), never puts the
//! window on screen, installs no tray icon, skips the real launch-at-login
//! cutover and the auto-update, and quits once `<file>.quit` exists.
//!
//! On macOS it also drives what the shell's exit depends on (card #535):
//! `<file>.terminate` quits through AppKit, as an Apple Event or the app
//! menu does (`menu` in the file picks the app menu's own Quit item), and
//! `<file>.voice` loads the speech model through a session on silence,
//! left running if the file says `session`, otherwise stopped with the
//! model still loaded; it notes `voice-loaded`.
//!
//! And it lets a test look at the real window (card #570's end-to-end
//! pass), since an agent session can't screencapture but a process may
//! always capture its own windows: `<file>.show` puts the window on screen
//! after all (the one exception to the rule above), `<file>.eval` runs the
//! script it holds in the webview (clicks and typing through the DOM), and
//! `<file>.shot` saves the window as the PNG it names, noting `shot <path>`
//! or `shot error: …`. Each file is removed once handled, so it can be
//! written again.
//!
//! A debug build honours the variable alone. A release build honours it
//! only together with `IRORI_DEV_OFFLINE=1` (a Core with no network), so
//! the rehearsal runs the exact exe `scripts/release.ps1` builds through
//! Go's updater: like the offline switch, the probe only takes things away
//! (the window on screen, the tray, the network, the real Run key) and
//! writes to the file it names.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct Probe(Option<Arc<PathBuf>>);

impl Probe {
    pub fn from_env() -> Probe {
        Probe(
            enabled(
                cfg!(debug_assertions),
                std::env::var(irori_core::offline::ENV).ok().as_deref(),
                std::env::var_os("IRORI_DEV_SHELL_LOG"),
            )
            .map(Arc::new),
        )
    }

    pub fn on(&self) -> bool {
        self.0.is_some()
    }

    pub fn note(&self, what: &str) {
        let Some(p) = &self.0 else { return };
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&**p)
        {
            let _ = writeln!(f, "{what}");
        }
    }

    /// The launch-at-login switch-over the GUI start runs, against the
    /// scratch HKCU subkey `IRORI_DEV_RUN_KEY` names instead of the real
    /// Run key (tools/rehearse: the Go updater installs this build over a
    /// scratch Go install whose Wails value sits under that subkey). Notes
    /// `login-cutover keep|rewrite|error: …`, then `login-enabled
    /// true|false`. Nothing without the probe or the variable; the real key
    /// is never touched here.
    pub fn cutover_on_scratch_key(&self) {
        if !self.on() {
            return;
        }
        let Some(subkey) = std::env::var("IRORI_DEV_RUN_KEY")
            .ok()
            .filter(|k| !k.is_empty())
        else {
            return;
        };
        if !is_scratch_run_key(&subkey) {
            self.note("login-cutover error: IRORI_DEV_RUN_KEY is not a scratch key");
            return;
        }
        #[cfg(windows)]
        {
            let item = super::login::LoginItem::with_run_subkey(&subkey);
            match item.cutover() {
                Ok(true) => self.note("login-cutover rewrite"),
                Ok(false) => self.note("login-cutover keep"),
                Err(e) => self.note(&format!("login-cutover error: {e}")),
            }
            match item.is_enabled() {
                Ok(on) => self.note(&format!("login-enabled {on}")),
                Err(e) => self.note(&format!("login-enabled error: {e}")),
            }
        }
        #[cfg(not(windows))]
        self.note("login-cutover error: Windows only");
    }

    /// Spawned agents part 1 (card #445): with `IRORI_DEV_SPAWN_REPO=<a
    /// trusted repo>`, starts one `claude --bg` agent from the GUI process
    /// (a release build has no console, so this is the "first spawn from the
    /// GUI opens no window" check) in a fresh `-w` worktree, with a brief that
    /// only replies. Notes `spawn <short id> <name>` or `spawn error: …`.
    /// The agent is left for the tester to `claude stop` and `claude rm`.
    pub fn spawn_from_gui(&self) {
        if !self.on() {
            return;
        }
        let Some(repo) = std::env::var_os("IRORI_DEV_SPAWN_REPO").filter(|v| !v.is_empty()) else {
            return;
        };
        let probe = self.clone();
        std::thread::spawn(move || {
            let briefs = std::env::temp_dir().join("irori-spawn-probe");
            let spec = irori_window::spawn::SpawnSpec {
                cwd: PathBuf::from(repo),
                name: "irori-gui-spawn-probe".into(),
                brief:
                    "Reply with exactly: gui-spawn-probe-ok. Do nothing else: no tools, no files."
                        .into(),
                permission_mode: "default".into(),
                model: Some("sonnet".into()),
                effort: Some("low".into()),
                worktree: Some("irori-gui-spawn-probe".into()),
                brief_dir: briefs,
                ..Default::default()
            };
            match irori_window::spawn::spawn(&spec) {
                Ok(s) => probe.note(&format!("spawn {} {}", s.short_id, s.name)),
                Err(e) => probe.note(&format!("spawn error: {}", e.replace('\n', " "))),
            }
        });
    }

    /// Quits the app once `<file>.quit` exists, as the tray's Quit does.
    pub fn watch_quit(&self, app: &tauri::AppHandle) {
        let Some(p) = &self.0 else { return };
        let mut quit = p.as_os_str().to_owned();
        quit.push(".quit");
        let quit = PathBuf::from(quit);
        let app = app.clone();
        std::thread::spawn(move || {
            while !quit.exists() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            app.exit(0);
        });
    }
}

#[cfg(target_os = "macos")]
impl Probe {
    /// A file beside the log, `<file>.<ext>`.
    fn beside(&self, ext: &str) -> Option<PathBuf> {
        let mut p = self.0.as_ref()?.as_os_str().to_owned();
        p.push(".");
        p.push(ext);
        Some(PathBuf::from(p))
    }

    /// Quits through `-[NSApplication terminate:]` once `<file>.terminate`
    /// exists. AppKit then calls exit() itself: `run_return` never returns.
    pub fn watch_terminate(&self, app: &tauri::AppHandle) {
        let Some(f) = self.beside("terminate") else {
            return;
        };
        let app = app.clone();
        std::thread::spawn(move || {
            while !f.exists() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let menu = std::fs::read_to_string(&f).is_ok_and(|s| s.trim() == "menu");
            let _ = app.run_on_main_thread(move || terminate(menu));
        });
    }

    /// Loads the speech model once `<file>.voice` exists (see the module
    /// docs). The model is the data dir's, as for a real session.
    pub fn watch_voice(&self, window: &Arc<irori_window::Window>) {
        let Some(f) = self.beside("voice") else {
            return;
        };
        let probe = self.clone();
        let window = window.clone();
        std::thread::spawn(move || {
            while !f.exists() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let keep = std::fs::read_to_string(&f).is_ok_and(|s| s.trim() == "session");
            match window.voice_start_with_samples(Vec::new()) {
                Ok(id) => {
                    if !keep {
                        let _ = window.voice_stop(id);
                    }
                    probe.note("voice-loaded");
                }
                Err(e) => probe.note(&format!("voice error: {e}")),
            }
        });
    }
}

#[cfg(target_os = "macos")]
impl Probe {
    /// Watches `<file>.show`, `<file>.eval` and `<file>.shot` (see the
    /// module docs), in the order they are listed, every 100 ms.
    pub fn watch_window(&self, app: &tauri::AppHandle) {
        let (Some(show), Some(eval), Some(shot)) = (
            self.beside("show"),
            self.beside("eval"),
            self.beside("shot"),
        ) else {
            return;
        };
        let probe = self.clone();
        let app = app.clone();
        std::thread::spawn(move || {
            use tauri::Manager;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let Some(w) = app.get_webview_window(super::MAIN) else {
                    continue;
                };
                if show.exists() {
                    let _ = std::fs::remove_file(&show);
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                    probe.note("shown");
                }
                if let Ok(js) = std::fs::read_to_string(&eval) {
                    let _ = std::fs::remove_file(&eval);
                    match w.eval(&js) {
                        Ok(()) => probe.note("eval"),
                        Err(e) => probe.note(&format!("eval error: {e}")),
                    }
                }
                if let Ok(to) = std::fs::read_to_string(&shot) {
                    let _ = std::fs::remove_file(&shot);
                    let to = PathBuf::from(to.trim());
                    let (tx, rx) = std::sync::mpsc::channel();
                    let w2 = w.clone();
                    // The window number is AppKit's, read on the main thread.
                    let _ = app.run_on_main_thread(move || {
                        let _ = tx.send(window_number(&w2));
                    });
                    let n = rx.recv().ok().flatten();
                    match n.ok_or_else(|| "no window".to_string()).and_then(|n| {
                        // SAFETY: CoreGraphics and ImageIO calls with
                        // their documented types (shot_window).
                        unsafe { shot_window(n, &to) }
                    }) {
                        Ok(()) => probe.note(&format!("shot {}", to.display())),
                        Err(e) => probe.note(&format!("shot error: {e}")),
                    }
                }
            }
        });
    }
}

#[cfg(target_os = "macos")]
fn window_number(w: &tauri::WebviewWindow) -> Option<u32> {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    let ns = w.ns_window().ok()? as *mut AnyObject;
    if ns.is_null() {
        return None;
    }
    // SAFETY: a live NSWindow, on the main thread.
    let n: isize = unsafe { msg_send![&*ns, windowNumber] };
    u32::try_from(n).ok().filter(|n| *n > 0)
}

/// The window numbered `n`, without its shadow, as a PNG at `path`.
#[cfg(target_os = "macos")]
unsafe fn shot_window(n: u32, path: &std::path::Path) -> Result<(), String> {
    use std::ffi::c_void;
    use std::os::unix::ffi::OsStrExt;

    #[repr(C)]
    struct CGRect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCreateImage(r: CGRect, opt: u32, id: u32, img: u32) -> *mut c_void;
        fn CGImageRelease(i: *mut c_void);
    }
    #[link(name = "ImageIO", kind = "framework")]
    unsafe extern "C" {
        fn CGImageDestinationCreateWithURL(
            url: *const c_void,
            ty: *const c_void,
            count: usize,
            opts: *const c_void,
        ) -> *mut c_void;
        fn CGImageDestinationAddImage(d: *mut c_void, i: *mut c_void, p: *const c_void);
        fn CGImageDestinationFinalize(d: *mut c_void) -> bool;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(
            a: *const c_void,
            buf: *const u8,
            len: isize,
            dir: bool,
        ) -> *const c_void;
        fn CFStringCreateWithCString(a: *const c_void, s: *const i8, enc: u32) -> *const c_void;
        fn CFRelease(p: *const c_void);
    }
    // SAFETY: the caller's; CGRectNull asks for the window's own bounds.
    unsafe {
        let null = CGRect {
            x: f64::INFINITY,
            y: f64::INFINITY,
            w: 0.0,
            h: 0.0,
        };
        // kCGWindowListOptionIncludingWindow, kCGWindowImageBoundsIgnoreFraming.
        let img = CGWindowListCreateImage(null, 8, n, 1);
        if img.is_null() {
            return Err("no image".into());
        }
        let p = path.as_os_str().as_bytes();
        let url = CFURLCreateFromFileSystemRepresentation(
            std::ptr::null(),
            p.as_ptr(),
            p.len() as isize,
            false,
        );
        let ty = CFStringCreateWithCString(std::ptr::null(), c"public.png".as_ptr(), 0x0800_0100);
        let dest = CGImageDestinationCreateWithURL(url, ty, 1, std::ptr::null());
        let ok = !dest.is_null() && {
            CGImageDestinationAddImage(dest, img, std::ptr::null());
            CGImageDestinationFinalize(dest)
        };
        if !dest.is_null() {
            CFRelease(dest);
        }
        CFRelease(ty);
        CFRelease(url);
        CGImageRelease(img);
        if ok {
            Ok(())
        } else {
            Err("can't write the PNG".into())
        }
    }
}

/// Sends `terminate:` to the app from the run loop, or the action of the
/// app menu's Quit item (the one on Command-Q) when `menu`. From the run
/// loop because that is where an Apple Event or a menu click arrives:
/// straight from here it would run inside tao's event callback, and tao
/// would wait on itself to deliver the exit.
#[cfg(target_os = "macos")]
fn terminate(menu: bool) {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Sel};
    use objc2::{msg_send, sel};
    use objc2_foundation::NSString;

    let Some(class) = AnyClass::get(c"NSApplication") else {
        return;
    };
    // SAFETY: AppKit's documented selectors with their documented types,
    // on the main thread (run_on_main_thread).
    unsafe {
        let ns_app: Retained<AnyObject> = msg_send![class, sharedApplication];
        let mut action = sel!(terminate:);
        if menu {
            let bar: Option<Retained<AnyObject>> = msg_send![&*ns_app, mainMenu];
            let Some(bar) = bar else { return };
            let q = NSString::from_str("q");
            let n: isize = msg_send![&*bar, numberOfItems];
            let found = (0..n).find_map(|i| {
                let item: Retained<AnyObject> = msg_send![&*bar, itemAtIndex: i];
                let sub: Option<Retained<AnyObject>> = msg_send![&*item, submenu];
                let sub = sub?;
                let m: isize = msg_send![&*sub, numberOfItems];
                (0..m).find_map(|j| {
                    let it: Retained<AnyObject> = msg_send![&*sub, itemAtIndex: j];
                    let key: Retained<NSString> = msg_send![&*it, keyEquivalent];
                    let a: Option<Sel> = msg_send![&*it, action];
                    (*key == *q).then_some(a).flatten()
                })
            });
            let Some(a) = found else { return };
            action = a;
        }
        let none: Option<&AnyObject> = None;
        let () = msg_send![&*ns_app, performSelector: action, withObject: none, afterDelay: 0.0f64];
    }
}

/// The probe's log file, if the probe is on: `IRORI_DEV_SHELL_LOG` set,
/// and in a release build (`dev` false) `IRORI_DEV_OFFLINE=1` too.
fn enabled(dev: bool, offline: Option<&str>, log: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let log = log.filter(|v| !v.is_empty())?;
    if !dev && !irori_core::offline::requested(offline) {
        return None;
    }
    Some(PathBuf::from(log))
}

/// A rehearsal Run key must be a scratch subkey under
/// `Software\irori-rust-rehearse\`, never the real Run key or anything else.
fn is_scratch_run_key(subkey: &str) -> bool {
    let lower = subkey.to_ascii_lowercase();
    lower.starts_with(r"software\irori-rust-rehearse\") && !lower.contains("..")
}

#[cfg(test)]
mod tests {
    use super::{enabled, is_scratch_run_key};
    use std::path::PathBuf;

    #[test]
    fn release_needs_offline() {
        let log = || Some(std::ffi::OsString::from("x.log"));
        let on = Some(PathBuf::from("x.log"));
        assert_eq!(enabled(true, None, log()), on, "dev: the log alone");
        assert_eq!(enabled(true, Some("1"), log()), on);
        assert_eq!(enabled(false, None, log()), None, "release, online");
        assert_eq!(enabled(false, Some("0"), log()), None);
        assert_eq!(enabled(false, Some("true"), log()), None);
        assert_eq!(enabled(false, Some("1"), log()), on, "release, offline");
        assert_eq!(enabled(true, Some("1"), Some("".into())), None);
        assert_eq!(enabled(false, Some("1"), None), None);
    }

    #[test]
    fn only_rehearsal_subkeys() {
        assert!(is_scratch_run_key(
            r"Software\irori-rust-rehearse\run-1\Run"
        ));
        for bad in [
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            r"Software\irori-rust-rehearse",
            r"Software\irori-rust-rehearse\..\Microsoft\Windows\CurrentVersion\Run",
            "",
        ] {
            assert!(!is_scratch_run_key(bad), "{bad}");
        }
    }
}
