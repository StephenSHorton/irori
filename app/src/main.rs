// A release build is a GUI-subsystem exe, so no console flashes (surface.md
// 7.15); a debug build keeps its console so panics are visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
mod serve;
mod shell;

use std::process::ExitCode;

fn main() -> ExitCode {
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
    });
}
