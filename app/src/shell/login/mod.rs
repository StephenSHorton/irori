//! Launch at login (Go settings_service.go:12-141 over Wails v3.0.0-alpha.98
//! autostart; surface.md 4, 7.4; architecture.md D9).
//!
//! The OS registration is the only state; nothing is kept beside it. It is
//! off until the person turns it on. The Rust build reads and writes the
//! same registrations Wails made, so one the Go build left is seen and
//! removable:
//! - Windows: the HKCU `Software\Microsoft\Windows\CurrentVersion\Run` value
//!   `Irori` = `<exe> --startup`, each part quoted only when it has
//!   whitespace or `"`. A value under any name whose first token is this exe
//!   counts as registered, as Wails' `find` does.
//! - macOS: `SMAppService.mainAppService` from a bundled app on 13+, else a
//!   LaunchAgent plist `~/Library/LaunchAgents/Irori.plist`. SMAppService
//!   passes no arguments, so a login launch there opens the window (the Go
//!   build has the same gap).
//!
//! Tests never touch the real registration: the Windows key and the
//! LaunchAgents dir are parameters.

pub mod launchagent;
#[cfg(target_os = "macos")]
mod macos;
pub mod winrun;

/// Go `autostartID`: the Run value name, the LaunchAgent label.
pub const ID: &str = "Irori";
/// Go `startupArg`: what a login launch passes, so it starts in the tray.
pub const STARTUP_ARG: &str = "--startup";

/// Go `startedAtLogin`.
pub fn started_at_login(args: &[String]) -> bool {
    args.iter().any(|a| a == STARTUP_ARG)
}

/// Go `resolvedExecutable`: the running exe with symlinks resolved, in the
/// plain form Go prints (no `\\?\` prefix on Windows, which
/// `std::fs::canonicalize` adds and the Wails-written Run value lacks).
pub fn resolved_exe() -> Result<String, String> {
    let exe =
        std::env::current_exe().map_err(|e| format!("autostart: get executable path: {e}"))?;
    let p = std::fs::canonicalize(&exe).unwrap_or(exe);
    Ok(strip_verbatim(&p.to_string_lossy()))
}

/// Drops Windows' verbatim prefix: `\\?\C:\x` → `C:\x`, `\\?\UNC\h\s` →
/// `\\h\s`.
pub fn strip_verbatim(p: &str) -> String {
    if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = p.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    p.to_string()
}

/// The launch-at-login registration for this exe.
pub struct LoginItem {
    #[cfg(windows)]
    run: winrun::RunKey,
    #[cfg(target_os = "macos")]
    mac: macos::Mac,
}

impl LoginItem {
    /// The real registration: the HKCU Run key, or SMAppService and
    /// `~/Library/LaunchAgents`.
    pub fn system() -> LoginItem {
        LoginItem {
            #[cfg(windows)]
            run: winrun::RunKey::new(winrun::RUN_SUBKEY),
            #[cfg(target_os = "macos")]
            mac: macos::Mac::system(),
        }
    }

    /// The registration under a scratch HKCU Run subkey instead of the
    /// real one (the rehearsal's `IRORI_DEV_RUN_KEY`, shell probe only).
    #[cfg(windows)]
    pub fn with_run_subkey(subkey: &str) -> LoginItem {
        LoginItem {
            run: winrun::RunKey::new(subkey),
        }
    }

    /// Whether this exe is registered to start at login (Go
    /// `AutostartManager.IsEnabled`).
    pub fn is_enabled(&self) -> Result<bool, String> {
        #[cfg(windows)]
        {
            let exe = resolved_exe()?;
            Ok(self.run.find(&exe)?.is_some())
        }
        #[cfg(target_os = "macos")]
        {
            self.mac.is_enabled()
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            Err("autostart: not supported on this platform".into())
        }
    }

    /// The cutover check at GUI start (surface.md 7.4,
    /// `::update::login::plan`): brings a Run value for this exe that the Go
    /// build (or an older install) wrote to the exact Rust form, so later
    /// toggles find and replace it. Values for other exes are left alone,
    /// so a dev build changes nothing. Windows only: SMAppService and the
    /// LaunchAgent keep the bundle path, which the `.app` swap keeps.
    /// Returns whether it rewrote the value.
    pub fn cutover(&self) -> Result<bool, String> {
        #[cfg(windows)]
        {
            let exe = resolved_exe()?;
            if let ::update::login::Plan::Rewrite { command, .. } = self.run.cutover(&exe)? {
                irori_core::log(&format!(
                    "irori: launch at login: rewrote the Run value as {command}"
                ));
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Registers (with `--startup`) or removes the login launch.
    /// Re-enabling overwrites the registration.
    pub fn set(&self, enabled: bool) -> Result<(), String> {
        #[cfg(windows)]
        {
            let exe = resolved_exe()?;
            if enabled {
                self.run.enable(ID, &exe, &[STARTUP_ARG])
            } else {
                self.run.disable(&exe)
            }
        }
        #[cfg(target_os = "macos")]
        {
            if enabled {
                self.mac.enable(ID, &[STARTUP_ARG])
            } else {
                self.mac.disable()
            }
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = enabled;
            Err("autostart: not supported on this platform".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_flag() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(started_at_login(&v(&["--startup"])));
        assert!(started_at_login(&v(&["x", "--startup"])));
        assert!(!started_at_login(&v(&["-startup"])));
        assert!(!started_at_login(&v(&[])));
    }

    #[test]
    fn verbatim_prefix() {
        assert_eq!(
            strip_verbatim(r"\\?\C:\Users\a\irori.exe"),
            r"C:\Users\a\irori.exe"
        );
        assert_eq!(
            strip_verbatim(r"\\?\UNC\host\share\i.exe"),
            r"\\host\share\i.exe"
        );
        assert_eq!(strip_verbatim(r"C:\x.exe"), r"C:\x.exe");
        assert_eq!(
            strip_verbatim("/Applications/Irori.app"),
            "/Applications/Irori.app"
        );
    }
}
