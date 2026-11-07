//! `irori`: the irori binary with a Rune window instead of Tauri and a
//! webview (rune-ui, card #674). Every role but the window is
//! crates/roles', as in the Tauri app; the window role is [`shell::run`],
//! and the window itself is [`ui`] and its packages.
//!
//! Headless, as any Rune app: `irori --snap home.png` renders the window
//! on the data dir's Core to a PNG, `--tree tree.json` dumps the element
//! tree, `--scene card` opens the first card that needs you
//! (`--list-scenes` lists them). Run it on a copy
//! (`IRORI_HOME=<copy> IRORI_DEV_OFFLINE=1`), never on the live `~/.irori`.

// A release build is a GUI-subsystem exe, so no console flashes (surface.md
// 7.15); a debug build keeps its console so panics are visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod instance;
mod shell;
#[cfg(target_os = "macos")]
mod tray_mac;
#[allow(dead_code)] // The packages' contract: not every helper is used yet.
mod ui;

// The shared pieces.
mod logic;
mod text;
mod widgets;

// The pages and panels, one package each (ui/mod.rs has the contract).
mod board;
mod call;
mod canvas;
mod card;
mod desk;
mod home;
mod inbox;
mod notes;
mod palette;
mod profile;
mod settings;
mod spaces;
mod status;
mod talk;
mod term;

fn main() -> std::process::ExitCode {
    irori_roles::run_main(shell::run)
}
