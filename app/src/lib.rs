//! The irori window's packages (rune-ui, card #674), as a library: the
//! desktop binary (main.rs) runs [`shell::run`]; the Android prototype
//! (card #680) is a cdylib of this library whose `android_main` runs
//! [`shell::run_android`] under Rune's Kotlin host.

mod instance;
mod login;
pub mod shell;
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
mod phone;
mod profile;
mod settings;
mod spaces;
mod status;
mod talk;
mod term;

/// NativeActivity's entry point on Android (card #680's prototype): Rune's
/// `RuneActivity` loads `libirori_app.so` and calls this on its own thread.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: rune::android::AndroidApp) {
    shell::run_android(app);
}
