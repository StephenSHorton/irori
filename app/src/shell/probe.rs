//! A hook for the shell's end-to-end test (app/tests/shell_exe.rs) and the
//! rehearsal's update leg (tools/rehearse).
//!
//! With `IRORI_DEV_SHELL_LOG=<file>` set, the window role appends one line
//! per shell event to that file (`ready`, `second-instance`, `show`,
//! `page-load <scheme>://<host>`, `api-port <port>`, `exiting`), never puts the
//! window on screen, installs no tray icon, skips the real launch-at-login
//! cutover and the auto-update, and quits once `<file>.quit` exists.
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
