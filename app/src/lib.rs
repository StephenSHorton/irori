#![cfg_attr(mobile, allow(dead_code))]
//! The irori app as a library: the desktop binary (main.rs) and the phone
//! build (card #226) share it.

mod attach;
mod cli;
mod commands;
mod console;
mod datadir;
mod events;
mod host;
mod mcp_cmd;
mod node_cmd;
mod pair_cmd;
mod permission_hook;
mod serve;
mod shell;

use std::process::ExitCode;

/// The desktop entry: every role (`irori`, `irori serve`, ...). main.rs
/// calls it; the phone build starts at [`mobile_main`] instead.
pub fn run_main() -> ExitCode {
    // Go main.go:39-72: the first argument is the role unless it is empty
    // or a flag, so `irori --startup` is the window.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, rest) = cli::split(&args);
    console::use_parent_console(cmd);
    setup_diagnostics(cmd);
    let res = match cmd {
        "" | "gui" => shell::run(rest),
        "serve" => serve::run(rest),
        "mcp" => mcp_cmd::run(rest),
        "attach" => attach::main(rest),
        "events" => events::main(rest),
        "pair" => pair_cmd::run(rest),
        "node" => node_cmd::run(rest),
        "permission-hook" => permission_hook::main(rest),
        "version" => {
            println!("irori {}", commands::version());
            Ok(())
        }
        c => Err(cli::unknown(c)),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        // Go: every role's error prints `irori: <err>`, exit 1.
        Err(e) => {
            eprintln!("irori: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Go `setupDiagnostics`: a panic in any role goes to `<data>/crash.log`.
/// The long-running roles, the app and `serve --headless`, also write a
/// start line there and route the Core's log to the rotated
/// `<data>/irori.log` (and stderr): the app has no console and a headless
/// serve may run from Task Scheduler, so otherwise a crash leaves nothing.
fn setup_diagnostics(cmd: &str) {
    let role = match cmd {
        "" | "gui" => "gui",
        "version" => return,
        r => r,
    };
    let Ok(dir) = datadir::process_data_dir() else {
        return;
    };
    let crash = match irori_core::diag::install_crash_output(&dir) {
        Ok(f) => Some(f),
        Err(e) => {
            irori_core::log(&format!("crash output: {e}"));
            None
        }
    };
    if role != "gui" && role != "serve" {
        return;
    }
    let start = irori_core::diag::start_line(role, commands::version());
    if let Some(f) = &crash
        && let Ok(mut f) = f.lock()
    {
        use std::io::Write;
        let _ = f.write_all(start.as_bytes());
    }
    let log = match irori_core::diag::open_log(&dir) {
        Ok(l) => std::sync::Arc::new(l),
        Err(e) => {
            irori_core::log(&format!("app log: {e}"));
            return;
        }
    };
    let _ = log.write(start.as_bytes());
    // The file first: a GUI launch's stderr may lead nowhere.
    irori_core::set_logger(move |line| {
        let l = format!(
            "{}{line}
",
            irori_core::diag::log_stamp()
        );
        let _ = log.write(l.as_bytes());
        eprint!("{l}");
    });    // Slow board-log lock waits and holds (card #1234).
    gocompat::lockwatch::set_logger(irori_core::log);
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
