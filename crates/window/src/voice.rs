//! Voice input's window face (#240): the mic button on the text boxes.
//!
//! `voice_status` says whether to show the mic (and whether the model still
//! has to be downloaded), `voice_download_model` fetches it once with
//! progress as `voice:download`, `voice_start` opens a session and returns
//! its id, `voice_stop` ends it, and the session's words arrive as
//! `voice:event` ([`VoiceEvent`]). One session at a time: starting a new
//! one ends the old (its `ended` still arrives, with its own id, so a box
//! only reacts to the session it started). Speech is transcribed on this
//! device (Whisper); nothing is sent anywhere: the box puts the text in and
//! the person edits and sends it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};

use serde::Serialize;
use specta::Type;

use crate::Window;

/// The voice:event event.
pub const EVENT_VOICE: &str = "voice:event";
/// The voice:download event.
pub const EVENT_VOICE_DOWNLOAD: &str = "voice:download";

/// Whether to show the mic, and why not.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStatus {
    /// ready; needs-model (the one-time model download comes first);
    /// denied (no microphone or access off; pressing still tries); or
    /// unsupported (no engine here: the mic is hidden).
    pub state: String,
    /// What to do about it; empty when ready.
    pub reason: String,
    /// The error code whose Settings page fixes it ("mic-denied"), else
    /// empty.
    pub fix: String,
    /// The model download's size in bytes (for "needs-model").
    pub model_bytes: u64,
}

/// One voice:event.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceEvent {
    /// The session this belongs to (voice_start's answer).
    pub session: u32,
    /// partial (the phrase so far, replacing the last partial), final (a
    /// settled phrase; may be empty when it was not understood), error
    /// (code and text set; ended follows) or ended (always last).
    pub kind: String,
    pub text: String,
    /// The error code: mic-denied, no-mic, no-model, unsupported or other.
    /// Empty unless kind is error.
    pub code: String,
}

/// One voice:download: bytes so far of the model download.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceDownload {
    pub done: u64,
    pub total: u64,
}

/// The window's one session, and whether a model download runs.
#[derive(Default)]
pub(crate) struct VoiceSlot {
    inner: Mutex<SlotInner>,
    /// Held while a session starts, so [`Window::voice_shutdown`] waits for
    /// a model that is loading right now and can free it too.
    opening: Mutex<()>,
}

#[derive(Default)]
struct SlotInner {
    next: u32,
    current: Option<(u32, voice::Session)>,
    downloading: bool,
    /// The app is exiting: nothing starts any more.
    closed: bool,
}

/// The status for this machine, given whether the model is there.
pub(crate) fn status_with(engine: voice::Status, model_ready: bool) -> VoiceStatus {
    let mut s = VoiceStatus {
        state: "ready".into(),
        reason: String::new(),
        fix: String::new(),
        model_bytes: voice::MODEL.size,
    };
    match engine {
        voice::Status::Unsupported(why) => {
            s.state = "unsupported".into();
            s.reason = why;
        }
        voice::Status::Denied(why) => {
            s.state = "denied".into();
            s.reason = why;
            s.fix = voice::ErrorCode::MicDenied.as_str().into();
        }
        voice::Status::Ready if !model_ready => {
            s.state = "needs-model".into();
            s.reason = format!(
                "Voice input transcribes on this device. It needs a one-time download of the speech model ({} MB).",
                voice::MODEL.size / 1_000_000
            );
        }
        voice::Status::Ready => {}
    }
    s
}

fn code_from(s: &str) -> Option<voice::ErrorCode> {
    voice::ErrorCode::ALL.into_iter().find(|c| c.as_str() == s)
}

impl Window {
    fn voice_model(&self) -> PathBuf {
        voice::model_path(self.core.data_dir(), &voice::MODEL)
    }

    /// Whether to show the mic and what it needs.
    pub fn voice_status(&self) -> VoiceStatus {
        status_with(
            voice::status(),
            voice::model_ready(self.core.data_dir(), &voice::MODEL),
        )
    }

    /// Downloads the model once (a second call while one runs fails).
    /// Progress goes out as voice:download.
    pub async fn voice_download_model(self: &Arc<Window>) -> Result<(), String> {
        {
            let mut s = self.voice.inner.lock().map_err(|e| e.to_string())?;
            if s.downloading {
                return Err("the speech model is already downloading".into());
            }
            s.downloading = true;
        }
        let w = Arc::downgrade(self);
        let last = Mutex::new(0u64);
        let progress = move |done: u64, total: u64| {
            // About every 1% (and the end): enough for a bar.
            let step = (total / 100).max(1);
            let mut l = last.lock().unwrap_or_else(|e| e.into_inner());
            if done == total || done >= *l + step || done == 0 {
                *l = done;
                if let Some(w) = w.upgrade() {
                    w.emit(EVENT_VOICE_DOWNLOAD, &VoiceDownload { done, total });
                }
            }
        };
        let ua = format!("irori/{}", self.version());
        let r = voice::download_model(
            self.core.data_dir(),
            &voice::MODEL,
            voice::MODEL.url,
            &ua,
            &progress,
        )
        .await;
        if let Ok(mut s) = self.voice.inner.lock() {
            s.downloading = false;
        }
        r.map(|_| ())
    }

    /// Starts a session (ending any other) and returns its id. Blocking:
    /// the engine loads the model and opens the microphone first.
    pub fn voice_start(self: &Arc<Window>) -> Result<u32, String> {
        let model = self.voice_model();
        self.voice_open(|sink| voice::start(&model, sink))
    }

    /// Like [`Window::voice_start`], but the engine hears `samples` instead
    /// of the microphone: the shell probe's way to load the model with no
    /// microphone permission (app/src/shell/probe.rs).
    #[doc(hidden)]
    #[cfg(target_os = "macos")]
    pub fn voice_start_with_samples(self: &Arc<Window>, samples: Vec<f32>) -> Result<u32, String> {
        let model = self.voice_model();
        self.voice_open(|sink| voice::start_with_samples(&model, samples, sink))
    }

    fn voice_open(
        self: &Arc<Window>,
        open: impl FnOnce(voice::Sink) -> Result<voice::Session, voice::StartError>,
    ) -> Result<u32, String> {
        let _opening = self.voice.opening.lock().map_err(|e| e.to_string())?;
        let old = {
            let mut s = self.voice.inner.lock().map_err(|e| e.to_string())?;
            if s.closed {
                return Err("Irori is closing".into());
            }
            s.current.take()
        };
        // Stopped outside the lock: it waits for the engine's thread.
        drop(old);
        let id = {
            let mut s = self.voice.inner.lock().map_err(|e| e.to_string())?;
            s.next = s.next.wrapping_add(1).max(1);
            s.next
        };
        let w: Weak<Window> = Arc::downgrade(self);
        let sink: voice::Sink = Arc::new(move |e| {
            if let Some(w) = w.upgrade() {
                w.emit(EVENT_VOICE, &event(id, e));
            }
        });
        let session = open(sink).map_err(|e| e.message)?;
        let replaced = {
            let mut s = self.voice.inner.lock().map_err(|e| e.to_string())?;
            s.current.replace((id, session))
        };
        drop(replaced);
        Ok(id)
    }

    /// Ends `session` if it is still the current one, after it finalizes
    /// what was said. Blocking.
    pub fn voice_stop(&self, session: u32) -> Result<(), String> {
        let taken = {
            let mut s = self.voice.inner.lock().map_err(|e| e.to_string())?;
            match &s.current {
                Some((id, _)) if *id == session => s.current.take(),
                _ => None,
            }
        };
        if let Some((_, sess)) = taken {
            sess.stop();
        }
        Ok(())
    }

    /// Ends any session and frees the speech model: the shell calls it as
    /// the app exits (macOS's Metal backend asserts at exit otherwise). A
    /// session still starting is waited for and ended, and none starts after.
    pub fn voice_shutdown(&self) {
        let _opening = self.voice.opening.lock().unwrap_or_else(|e| e.into_inner());
        let current = {
            let mut s = self.voice.inner.lock().unwrap_or_else(|e| e.into_inner());
            s.closed = true;
            s.current.take()
        };
        if let Some((_, session)) = current {
            session.stop();
        }
        voice::release();
    }

    /// Opens the OS settings page for an error code from a voice:event or
    /// the status's `fix`.
    pub fn voice_open_settings(&self, code: &str) -> Result<(), String> {
        let c = code_from(code).ok_or_else(|| format!("unknown voice error code {code:?}"))?;
        voice::open_settings(c)
    }
}

fn event(session: u32, e: voice::Event) -> VoiceEvent {
    let (kind, text, code) = match e {
        voice::Event::Partial(t) => ("partial", t, ""),
        voice::Event::Final(t) => ("final", t, ""),
        voice::Event::Error { code, message } => ("error", message, code.as_str()),
        voice::Event::Ended => ("ended", String::new(), ""),
    };
    VoiceEvent {
        session,
        kind: kind.into(),
        text,
        code: code.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_map_to_the_window_shape() {
        assert_eq!(
            event(3, voice::Event::Partial("hello wor".into())),
            VoiceEvent {
                session: 3,
                kind: "partial".into(),
                text: "hello wor".into(),
                code: "".into()
            }
        );
        assert_eq!(
            event(3, voice::Event::Final("hello world".into())).kind,
            "final"
        );
        let e = event(
            4,
            voice::Event::Error {
                code: voice::ErrorCode::MicDenied,
                message: "allow it".into(),
            },
        );
        assert_eq!(
            (e.kind.as_str(), e.code.as_str(), e.text.as_str()),
            ("error", "mic-denied", "allow it")
        );
        assert_eq!(event(4, voice::Event::Ended).kind, "ended");
    }

    #[test]
    fn codes_round_trip() {
        for c in voice::ErrorCode::ALL {
            assert_eq!(code_from(c.as_str()), Some(c));
        }
        assert_eq!(code_from("nope"), None);
    }

    #[test]
    fn status_states() {
        assert_eq!(status_with(voice::Status::Ready, true).state, "ready");
        let s = status_with(voice::Status::Ready, false);
        assert_eq!(s.state, "needs-model");
        assert!(s.reason.contains("MB"));
        assert_eq!(s.model_bytes, voice::MODEL.size);
        let s = status_with(voice::Status::Denied("no mic".into()), true);
        assert_eq!((s.state.as_str(), s.fix.as_str()), ("denied", "mic-denied"));
        let s = status_with(voice::Status::Unsupported("later".into()), false);
        assert_eq!(s.state, "unsupported");
    }
}
