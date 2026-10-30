//! `irori-toast`: Irori's cards, a native port of go-toast v0.2.0
//! (github.com/StephenSHorton/go-toast, inbox.md §3.13, §4, decision D7).
//!
//! Cards stack in a corner of the active screen, over every other window,
//! never take focus, and carry up to two buttons and the s&box editor
//! sounds. They are custom-drawn windows, not OS notification-center
//! toasts: on Windows layered, topmost, `WS_EX_NOACTIVATE` tool windows on
//! one dedicated UI thread with its own message loop ([`win32`]); on macOS
//! non-activating `NSPanel`s on the AppKit main thread Tauri already runs
//! (`darwin.rs`, compiled and checked on the Mac). Elsewhere every call
//! is a no-op.
//!
//! Set the look once with [`configure`] (start from [`Config::default`]),
//! apply the person's settings with [`set_options`], then [`post`].
//!
//! The model ([`note`], [`layout`], [`config`], [`sound`]) and the window
//! sync ([`engine`]) are OS-agnostic, so tests drive the engine headless
//! with a recording [`engine::Surface`].
//!
//! Not ported: `IgnoreInput`, `HoldClickThrough` and `Main` (Toru's auto
//! clicker and plain-Go programs); Irori never used them.
//!
//! Wiring (M6 shell): on Windows nothing is needed; the first post starts
//! the card thread. On macOS the AppKit side queues all its work on the
//! main dispatch queue, which runs because Tauri runs NSApp on the main
//! thread; never start a second NSApp loop (inbox.md §7 trap 16).

pub mod config;
pub mod engine;
pub mod layout;
pub mod note;
pub mod sound;

#[cfg(windows)]
pub mod win32;

#[cfg(target_os = "macos")]
mod darwin;

use std::sync::LazyLock;
use std::time::Instant;

pub use config::{Accents, Color, Config, DARK, Kind, LIGHT, Palette, hex, rgb};
pub use engine::{Engine, Look, Mouse, Surface};
pub use note::{
    Action, CORNER_BOTTOM_LEFT, CORNER_BOTTOM_RIGHT, CORNER_TOP_LEFT, CORNER_TOP_RIGHT, Callback,
    Note, Options,
};

/// The process's cards.
pub(crate) static ENGINE: LazyLock<Engine> = LazyLock::new(Engine::new);

#[cfg(windows)]
use win32 as platform;

#[cfg(target_os = "macos")]
use darwin as platform;

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    pub fn wake() {}
    pub fn wake_if_started() {}
    pub fn play(_: std::sync::Arc<[u8]>) {}
    pub const DRAWS: bool = false;
}

/// Sets the app-level setup. Call it once at startup, before the first
/// post: the window classes and title are fixed when the first card opens.
pub fn configure(c: Config) {
    ENGINE.configure(c);
}

/// The setup in effect, with defaults filled in.
pub fn current_config() -> Config {
    ENGINE.config()
}

/// Applies the person's settings. Turning cards off clears them.
pub fn set_options(o: Options) {
    if ENGINE.set_options(o) && platform::DRAWS {
        platform::wake_if_started();
    }
}

/// The settings in effect.
pub fn current_options() -> Options {
    ENGINE.options()
}

/// Shows n (or replaces the card with its id) and plays its sound. A no-op
/// when cards are off, and on a platform without cards. Never blocks.
pub fn post(n: Note) {
    if !platform::DRAWS {
        return;
    }
    let p = ENGINE.post(n, Instant::now());
    if let Some((s, vol)) = p.sound
        && let Some(b) = sound::sound_bytes(s, vol)
    {
        platform::play(b);
    }
    if p.shown {
        platform::wake();
    }
}

/// Removes the card with id, if it is up, without running its callbacks.
pub fn close(id: &str) {
    if platform::DRAWS && ENGINE.close(id) {
        platform::wake();
    }
}

/// Posts a plain card: ok picks Success or Error.
pub fn show(title: &str, body: &str, ok: bool) {
    post(Note {
        title: title.into(),
        body: body.into(),
        kind: if ok { Kind::Success } else { Kind::Error },
        ..Note::default()
    });
}

/// Plays kind's sound without a card. It honours the volume but not
/// `enabled` or `sound`.
pub fn play_sound(kind: Kind) {
    if let Some(b) = sound::sound_bytes(sound::sound_for(kind, false), current_options().volume) {
        platform::play(b);
    }
}

/// Whether any card is on screen.
pub fn visible() -> bool {
    ENGINE.visible()
}

/// Keeps a test program alive for d with the cards working: on macOS it
/// runs NSApp on the main thread meanwhile (a program without Tauri has no
/// AppKit loop, and cards need one); elsewhere it sleeps. The app never
/// calls it.
pub fn run_for(d: std::time::Duration) {
    #[cfg(target_os = "macos")]
    darwin::run_for(d);
    #[cfg(not(target_os = "macos"))]
    std::thread::sleep(d);
}

/// The cards on screen, oldest first (smoke tests).
pub fn card_keys() -> Vec<u64> {
    ENGINE.keys()
}
