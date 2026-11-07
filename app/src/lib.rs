#![cfg_attr(mobile, allow(dead_code))]
//! The irori app as a library: the desktop binary (main.rs) and the phone
//! build (card #226) share it.

// The roles without a window live in crates/roles (the Rune shell in
// desk/ shares them); these names keep `crate::host` and friends working.
pub(crate) use irori_roles::{cli, commands, datadir, host};
use irori_roles::setup_diagnostics;
mod shell;

use std::process::ExitCode;

/// The desktop entry: every role (`irori`, `irori serve`, ...). main.rs
/// calls it; the phone build starts at [`mobile_main`] instead.
pub fn run_main() -> ExitCode {
    irori_roles::run_main(shell::run)
}

/// The phone entry (card #226, iOS #269): Tauri's Android/iOS shell calls
/// it. The app's private storage holds Irori's data, as IRORI_HOME (on
/// Android it also stands in for HOME); a phone runs no Claude sessions, so
/// the Claude dir is an empty one there.
#[cfg(mobile)]
#[tauri::mobile_entry_point]
pub fn mobile_main() {
    // Android: the app's private files dir. iOS (#269): the app container's
    // Library/Application Support, under the HOME iOS gives every app.
    #[cfg(target_os = "android")]
    let (home, files) = {
        const FILES: &str = "/data/data/com.stephenshorton.irori/files";
        (Some(FILES.to_string()), FILES.to_string())
    };
    #[cfg(target_os = "ios")]
    let (home, files) = (
        None::<String>,
        format!(
            "{}/Library/Application Support",
            std::env::var("HOME").unwrap_or_default()
        ),
    );
    let mut vars = vec![
        ("IRORI_HOME", format!("{files}/irori")),
        ("CLAUDE_CONFIG_DIR", format!("{files}/claude")),
    ];
    if let Some(h) = home {
        vars.insert(0, ("HOME", h));
    }
    for (k, v) in vars {
        if std::env::var_os(k).is_none_or(|v| v.is_empty()) {
            let _ = std::fs::create_dir_all(&v);
            // SAFETY: first thing in the process, before any thread starts.
            unsafe { std::env::set_var(k, v) };
        }
    }
    setup_diagnostics("gui");
    if let Err(e) = shell::run(&[]) {
        irori_core::log(&format!("irori: {e}"));
    }
}
