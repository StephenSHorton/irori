//! No engine on this OS (Windows, macOS and Android have `whisper.rs`).
//! The window hides the mic on [`Status::Unsupported`].

use std::path::Path;

use crate::{ErrorCode, Options, Sink, StartError, Status};

const WHY: &str = "Voice input is not available on this device yet.";

pub(crate) fn status() -> Status {
    Status::Unsupported(WHY.to_string())
}

pub(crate) struct Session;

impl Session {
    pub(crate) fn stop(self) {}
}

pub(crate) fn start(_model: &Path, _sink: Sink) -> Result<Session, StartError> {
    Err(StartError {
        code: ErrorCode::Unsupported,
        message: WHY.to_string(),
    })
}

#[derive(Clone)]
pub(crate) struct Feed;

impl Feed {
    pub(crate) fn push(&self, _mono: &[f32]) {}
}

pub(crate) fn start_feed(
    _model: &Path,
    _input_rate: u32,
    _opts: Options,
    _sink: Sink,
) -> Result<(Session, Feed), StartError> {
    Err(StartError {
        code: ErrorCode::Unsupported,
        message: WHY.to_string(),
    })
}

pub(crate) fn release() {}
