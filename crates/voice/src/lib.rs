//! Voice input (#240): speech to text on this device with Whisper
//! (whisper.cpp), Stephen's pick over the OS recognizers (blocker b1):
//! one engine on every platform, and no audio leaves the machine.
//!
//! The contract every engine keeps:
//! - [`status`] says whether this build and machine can listen at all.
//!   It is cheap and never prompts. (Whether the model file is there is
//!   the caller's business: it owns the data dir; see [`MODEL`].)
//! - [`start`] opens one listening session and calls the sink from the
//!   engine's own thread: any number of [`Event::Partial`] (the words so far
//!   for the phrase being spoken, each replacing the last), an
//!   [`Event::Final`] when a phrase is settled, then more phrases, and
//!   exactly one [`Event::Ended`] last. A failure arrives as
//!   [`Event::Error`] just before `Ended`.
//! - [`Session::stop`] (or dropping the session) ends it, after
//!   finalizing what was said so far; the sink still gets `Ended`.
//!   Stopping twice is harmless.
//! - [`start_feed`] (#656, call transcripts) is a session that hears audio
//!   pushed through a [`Feed`] instead of the microphone, and reports
//!   [`Event::Segment`] (a phrase with its times) instead of `Final`.
//!
//! Nothing here sends text anywhere: the window puts it in the text box and
//! the person edits and sends it.
//!
//! Engines: `whisper.rs` (cpal microphone + whisper-rs) on Windows, macOS
//! and Android (CPU only). Other OSes get `other.rs`, which reports
//! [`Status::Unsupported`], and the window hides the mic.

use std::path::Path;
use std::sync::Arc;

/// The model this build expects: file name, size and SHA-256. Hosted on
/// Irori's own GitHub release (blocker b2), downloaded once into the data
/// dir by the window.
pub struct Model {
    pub file: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// Whisper base.en (ggml, from whisper.cpp's conversion of OpenAI's MIT
/// weights).
pub const MODEL: Model = Model {
    file: "ggml-base.en.bin",
    url: "https://github.com/StephenSHorton/irori/releases/download/models-whisper/ggml-base.en.bin",
    size: 147_964_211,
    sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
};

/// The window's text for a denied microphone, per platform.
#[cfg(windows)]
pub const MIC_SETTING: &str = "Allow microphone access for desktop apps in Windows Settings > Privacy & security > Microphone, then try again.";
#[cfg(target_os = "macos")]
pub const MIC_SETTING: &str = "Allow Irori to use the microphone in System Settings > Privacy & Security > Microphone, then try again.";
#[cfg(target_os = "android")]
pub const MIC_SETTING: &str =
    "Allow Irori to use the microphone in Settings > Apps > Irori > Permissions, then try again.";
#[cfg(not(any(windows, target_os = "macos", target_os = "android")))]
pub const MIC_SETTING: &str =
    "Allow Irori to use the microphone in the system settings, then try again.";

/// Whether this machine can listen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Ready to start (a permission prompt may still come on first use).
    Ready,
    /// This build or OS has no engine; the window hides the mic.
    Unsupported(String),
    /// The engine exists but can't listen now (no microphone, or access
    /// turned off); the string says what to do.
    Denied(String),
}

/// Why a session failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// Microphone access is off for this app.
    MicDenied,
    /// No microphone, or it is in use.
    NoMic,
    /// The model file is missing or broken.
    NoModel,
    /// No engine in this build.
    Unsupported,
    /// Anything else.
    Other,
}

impl ErrorCode {
    /// The stable name the window gets.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::MicDenied => "mic-denied",
            ErrorCode::NoMic => "no-mic",
            ErrorCode::NoModel => "no-model",
            ErrorCode::Unsupported => "unsupported",
            ErrorCode::Other => "other",
        }
    }

    /// Every code, for parsing.
    pub const ALL: [ErrorCode; 5] = [
        ErrorCode::MicDenied,
        ErrorCode::NoMic,
        ErrorCode::NoModel,
        ErrorCode::Unsupported,
        ErrorCode::Other,
    ];
}

/// One thing a session reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The phrase so far; replaces the previous partial.
    Partial(String),
    /// A settled phrase (may be empty when nothing was understood); the
    /// next partial starts a new one.
    Final(String),
    /// A settled phrase from a feed session, never empty. The times are ms
    /// from the first sample pushed, counted in the samples fed (so they
    /// follow the caller's clock, not the wall clock); `start_ms` includes a
    /// short lead-in of silence and `end_ms` is where the speech stopped.
    Segment {
        text: String,
        start_ms: u64,
        end_ms: u64,
    },
    /// The session failed; `Ended` follows.
    Error { code: ErrorCode, message: String },
    /// The session is over. Always the last event.
    Ended,
}

/// How a session runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Send [`Event::Partial`]s (and spend the passes they take).
    pub partials: bool,
    /// At most this many CPU threads for Whisper (the default rule still
    /// applies under it).
    pub max_threads: Option<i32>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            partials: true,
            max_threads: None,
        }
    }
}

impl Options {
    /// For a call's transcript: finals only, at most 2 threads (4 on
    /// Android, whose cores are slower: base.en's pass takes about 6 s on
    /// 2 of a Pixel's, 4 s on 4), so the call keeps its CPU.
    pub fn call() -> Self {
        Options {
            partials: false,
            max_threads: Some(if cfg!(target_os = "android") { 4 } else { 2 }),
        }
    }
}

/// Where a session's events go. Called from the engine's thread.
pub type Sink = Arc<dyn Fn(Event) + Send + Sync + 'static>;

/// A failure to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartError {
    pub code: ErrorCode,
    pub message: String,
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for StartError {}

#[cfg(any(windows, target_os = "macos", target_os = "android"))]
mod whisper;
#[cfg(any(windows, target_os = "macos", target_os = "android"))]
use whisper as engine;

#[cfg(not(any(windows, target_os = "macos", target_os = "android")))]
mod other;
#[cfg(not(any(windows, target_os = "macos", target_os = "android")))]
use other as engine;

#[cfg(windows)]
mod win;

mod model;
pub use model::{download_model, model_path, model_ready};

/// Whether this machine can listen. Cheap; never prompts.
pub fn status() -> Status {
    engine::status()
}

/// Opens the OS settings page that fixes `code` (the microphone), where
/// the platform has one.
pub fn open_settings(code: ErrorCode) -> Result<(), String> {
    #[cfg(windows)]
    {
        win::open_settings(code)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = code;
        whisper::mac::open_mic_settings()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = code;
        Err("open the system settings to allow the microphone".into())
    }
}

/// Starts a listening session with the model at `model`. Events go to
/// `sink` until [`Event::Ended`].
pub fn start(model: &Path, sink: Sink) -> Result<Session, StartError> {
    engine::start(model, sink).map(|inner| Session { inner: Some(inner) })
}

/// Like [`start`], but the engine hears `samples` (16 kHz mono, played in
/// real time, then silence) instead of the microphone. For tests and the
/// listen example.
#[doc(hidden)]
#[cfg(any(windows, target_os = "macos", target_os = "android"))]
pub fn start_with_samples(
    model: &Path,
    samples: Vec<f32>,
    sink: Sink,
) -> Result<Session, StartError> {
    whisper::start_from(
        model,
        whisper::Source::Samples(samples),
        Options::default(),
        sink,
    )
    .map(|inner| Session { inner: Some(inner) })
}

/// Starts a session that hears what is pushed into the returned [`Feed`]
/// (mono at `input_rate`, at least 16 kHz) instead of the microphone, so it
/// needs no microphone or permission, only the model. Phrases arrive as
/// [`Event::Segment`], timed from the first sample pushed, not from this
/// call. [`Session::stop`] finishes the phrase in progress (its `Segment`
/// reaches the sink before `stop` returns), then `Ended`.
///
/// Blocks while the model loads (seconds the first time): don't call it on
/// the audio thread or an async executor.
pub fn start_feed(
    model: &Path,
    input_rate: u32,
    opts: Options,
    sink: Sink,
) -> Result<(Session, Feed), StartError> {
    engine::start_feed(model, input_rate, opts, sink)
        .map(|(inner, feed)| (Session { inner: Some(inner) }, Feed(feed)))
}

/// Pushes audio into a [`start_feed`] session. Cheap to clone.
#[derive(Clone)]
pub struct Feed(engine::Feed);

impl Feed {
    /// Hands over the next mono samples. Never blocks: a chunk that meets
    /// the engine taking the backlog is dropped (still counted in the
    /// times), and the backlog keeps only the newest 25 s when the engine
    /// lags. Does nothing once the session has ended. Doesn't allocate in
    /// steady state, so it can run on the audio thread.
    pub fn push(&self, mono: &[f32]) {
        self.0.push(mono)
    }
}

/// Frees the speech model kept loaded between sessions. Call it before the
/// process exits: on macOS ggml's Metal backend asserts at exit when a
/// model still holds GPU memory, and the app would quit with a crash report.
/// Starting again later loads the model again.
pub fn release() {
    engine::release()
}

/// One listening session. Dropping it stops it.
pub struct Session {
    inner: Option<engine::Session>,
}

impl Session {
    /// Ends the session; the sink still gets `Ended`.
    pub fn stop(mut self) {
        if let Some(s) = self.inner.take() {
            s.stop();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(s) = self.inner.take() {
            s.stop();
        }
    }
}
