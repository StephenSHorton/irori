//! What a call needs from the operating system beyond the mic and speaker
//! (#654). On Android: a foreground service of type microphone, so a call
//! survives the screen turning off, plus the voice-call audio mode and the
//! speaker/earpiece route. Desktops need none of it and pass no platform.

use std::sync::Arc;

/// Where call audio plays on a phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioRoute {
    Speaker,
    Earpiece,
}

/// Something the platform tells the call while it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformEvent {
    /// The person pressed Hang up on the call's notification.
    HangUp,
    /// The service stopped on its own; the text says why.
    Failed(String),
}

/// Why the platform could not keep a call running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformError {
    /// The microphone permission isn't granted.
    MicDenied,
    /// Android refuses to start a microphone service while the app is in the
    /// background.
    Background,
    Other(String),
}

impl std::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlatformError::MicDenied => f.write_str("the microphone permission isn't granted"),
            PlatformError::Background => {
                f.write_str("a call can only start while Irori is on screen")
            }
            PlatformError::Other(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for PlatformError {}

pub type PlatformSink = Arc<dyn Fn(PlatformEvent) + Send + Sync>;

/// The OS side of a call. Every method may block briefly (Android answers on
/// its main thread), so never call them from that thread.
pub trait Platform: Send + Sync {
    /// Before the mic opens: start keeping the call alive. `title` is the
    /// notification's text, e.g. "In a huddle · #design".
    fn begin(&self, title: &str, events: PlatformSink) -> Result<(), PlatformError>;
    /// After the mic closes.
    fn end(&self);
    /// Move call audio to the speaker or the earpiece.
    fn route(&self, to: AudioRoute) -> Result<(), String>;
}
