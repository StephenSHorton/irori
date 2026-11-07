//! `irori`: the irori binary with a Rune window instead of Tauri and a
//! webview (rune-ui spike, card #674). Every role but the window is
//! crates/roles', as in the Tauri app; the window role is [`shell::run`].
//!
//! Headless, as any Rune app: `irori --snap board.png` renders the
//! Board on the data dir's Core to a PNG, `--tree tree.json` dumps the
//! element tree, `--scene detail` opens the first card that needs you.
//! Run it on a copy (`IRORI_HOME=<copy> IRORI_DEV_OFFLINE=1`), never on
//! the live `~/.irori`.

// A release build is a GUI-subsystem exe, so no console flashes (surface.md
// 7.15); a debug build keeps its console so panics are visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod board;
mod instance;
mod shell;
#[cfg(target_os = "macos")]
mod tray_mac;

fn main() -> std::process::ExitCode {
    irori_roles::run_main(shell::run)
}
