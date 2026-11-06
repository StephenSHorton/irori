//! Irori's call layer (#654): Opus 48 kHz mono in 20 ms frames, sent as QUIC
//! datagrams to every other device in the call (a full mesh of 2 to 6),
//! with echo cancelling, noise suppression and gain from sonora (WebRTC's
//! audio processing in Rust). Library choices: docs/research/08-voice-calls.
//!
//! This crate knows nothing of iroh: a peer is a [`Link`] that sends a
//! datagram, and received datagrams are handed to [`Engine::receive`].

pub mod platform;

pub use platform::{AudioRoute, Platform, PlatformError, PlatformEvent, PlatformSink};
