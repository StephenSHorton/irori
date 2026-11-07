//! Irori's screen share (#657): capture one window or a whole screen and
//! encode it as H.264 on the machine's own hardware encoder.
//!
//! The Mac captures with ScreenCaptureKit and encodes with VideoToolbox; the
//! PC captures with Windows.Graphics.Capture and encodes with Media
//! Foundation. Both come from `moq-video` (docs/research/08-voice-calls/
//! screen-share.md, stack A), and frames stay on the GPU from capture to
//! encoder: an IOSurface-backed `CVPixelBuffer` on the Mac, a Direct3D 11
//! texture on the PC.
//!
//! Only video is captured. ScreenCaptureKit captures audio only when asked
//! (`capturesAudio`), and nothing here asks, so Irori's own sounds (a call's
//! voices, notification chimes) never reach a share. WGC has no audio.
//!
//! This crate knows nothing of iroh or the window: it turns a [`Source`] into
//! [`Chunk`]s, each one access unit in Annex-B with parameter sets in band on
//! every keyframe. Sending them and drawing them is the next layer's job.

mod chunk;
pub mod cpu;
#[cfg(target_os = "macos")]
pub mod grant;
mod synthetic;

pub use chunk::{Chunk, is_keyframe};
pub use synthetic::synthetic_frame;

/// The raw picture type a [`Capture`] yields and an [`Encoder`] takes.
pub use moq_video::Frame;

use std::time::Duration;

/// What can go wrong opening or running a share.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// From capture or the encoder backend.
    #[error(transparent)]
    Video(#[from] moq_video::Error),
    /// A setting moq-video refuses (a zero frame rate, a timestamp past the
    /// end of time).
    #[error("{0}")]
    Settings(String),
    /// The capture stopped on its own: the window closed, the person pressed
    /// Stop in the menu bar, or the grant was taken away.
    #[error("the capture ended")]
    Ended,
}

pub type Result<T> = std::result::Result<T, Error>;

/// How to encode a share. [`Settings::default`] is the card's target: 1080p
/// at 10 frames a second.
#[derive(Clone, Debug)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Target bitrate in kbit/s; `None` lets moq-video pick about 0.07 bits
    /// per pixel per second (1.45 Mbit/s at 1080p 10).
    pub bitrate_kbps: Option<u32>,
    /// A keyframe at least this often even if nobody asks, so a decoder
    /// that missed one recovers. Viewers that join ask for one
    /// ([`Encoder::request_keyframe`]) rather than waiting.
    pub keyframe_every: Duration,
    /// Refuse a software encoder. On by default: a share that silently falls
    /// back to the CPU would load the machine the person is working on.
    pub hardware_only: bool,
    /// Draw the mouse pointer into the picture.
    pub cursor: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 10,
            bitrate_kbps: None,
            keyframe_every: Duration::from_secs(10),
            hardware_only: true,
            cursor: true,
        }
    }
}

impl Settings {
    fn rate(&self) -> Result<moq_video::Rate> {
        moq_video::Rate::new(self.fps, 1).map_err(|e| Error::Settings(e.to_string()))
    }

    fn size(&self) -> moq_video::Size {
        moq_video::Size::new(self.width, self.height)
    }
}

/// What to share.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// One window, by the id [`windows`] lists.
    Window(String),
    /// A whole display: `None` is the main one.
    Display(Option<String>),
}

/// A window that can be shared.
#[derive(Clone, Debug)]
pub struct Window {
    /// Pass to [`Source::Window`].
    pub id: String,
    pub title: String,
    /// The app that owns it.
    pub app: String,
    /// Points on the Mac, pixels on the PC.
    pub width: u32,
    pub height: u32,
}

/// The windows on screen now. On the Mac this needs the Screen Recording
/// grant (see [`grant`]); without it the list comes back empty or fails.
pub async fn windows() -> Result<Vec<Window>> {
    let list = moq_video::capture::windows().await?;
    Ok(list
        .into_iter()
        .map(|w| Window {
            id: w.id,
            title: w.title,
            app: w.app,
            width: w.width,
            height: w.height,
        })
        .collect())
}

/// A display that can be shared.
#[derive(Clone, Debug)]
pub struct Display {
    /// Pass to [`Source::Display`].
    pub id: String,
    pub name: String,
    /// Points on the Mac, pixels on the PC.
    pub width: u32,
    pub height: u32,
}

/// The displays attached now. Needs the grant on the Mac, like [`windows`].
pub async fn displays() -> Result<Vec<Display>> {
    let list = moq_video::capture::displays().await?;
    Ok(list
        .into_iter()
        .map(|d| Display {
            id: d.id,
            name: d.name,
            width: d.width,
            height: d.height,
        })
        .collect())
}

/// A running capture of one [`Source`], scaled to the settings' size.
pub struct Capture {
    stream: moq_video::capture::Stream,
    size: moq_video::Size,
}

impl Capture {
    /// Start capturing. On the Mac, ScreenCaptureKit scales the window into
    /// `width` x `height` itself, keeping its aspect ratio.
    pub async fn open(source: &Source, settings: &Settings) -> Result<Self> {
        let mut config = moq_video::capture::Config::default();
        config.source = match source {
            Source::Window(id) => moq_video::capture::Source::Window(id.clone()),
            Source::Display(id) => moq_video::capture::Source::Display(id.clone()),
        };
        config.width = Some(settings.width);
        config.height = Some(settings.height);
        config.framerate = Some(settings.rate()?);
        config.cursor = settings.cursor;
        let stream = moq_video::capture::open(&config).await?;
        Ok(Self {
            stream,
            size: settings.size(),
        })
    }

    /// The next picture, waiting for one. Pictures the encoder was too slow
    /// for are dropped, not queued. A frame at another size (WGC hands over
    /// the window's own size) is scaled on the GPU first.
    pub async fn next(&mut self) -> Result<Frame> {
        let frame = self.stream.read().await?.ok_or(Error::Ended)?;
        if frame.size() == self.size {
            return Ok(frame);
        }
        Ok(frame.resize(self.size, &moq_video::resize::Config::default())?)
    }

    /// The size the platform is capturing at.
    pub fn captured_size(&self) -> (u32, u32) {
        (self.stream.width(), self.stream.height())
    }
}

/// An H.264 encoder on this machine's hardware. Not `Send`: the Windows
/// backend's COM apartment belongs to the thread that opened it, so keep it
/// on one thread (a current-thread runtime, or `spawn_local`).
pub struct Encoder {
    inner: moq_video::encode::Encoder,
}

impl Encoder {
    pub fn new(settings: &Settings) -> Result<Self> {
        let rate = settings.rate()?;
        let mut config = moq_video::encode::Config::new(settings.width, settings.height, rate);
        config.bitrate = settings
            .bitrate_kbps
            .map(|k| moq_net::bandwidth::Rate::from_kbps(u64::from(k)));
        config.gop = moq_video::encode::Gop::keyframe_every(settings.keyframe_every, rate);
        config.codec = moq_video::encode::Codec::H264;
        config.kind = if settings.hardware_only {
            moq_video::encode::Kind::Hardware
        } else {
            moq_video::encode::Kind::Auto
        };
        Ok(Self {
            inner: moq_video::encode::Encoder::new(&config)?,
        })
    }

    /// The backend in use: "videotoolbox" or "mediafoundation".
    pub fn name(&self) -> &str {
        self.inner.name()
    }

    /// The current target bitrate in kbit/s.
    pub fn bitrate_kbps(&self) -> u32 {
        (self.inner.bitrate().as_bps() / 1000) as u32
    }

    /// Change the target bitrate on the running session, for when the link
    /// says it is carrying less (or more) than we send.
    pub fn set_bitrate_kbps(&mut self, kbps: u32) -> Result<()> {
        self.inner
            .set_bitrate(moq_net::bandwidth::Rate::from_kbps(u64::from(kbps)))?;
        Ok(())
    }

    /// Make the next frame a keyframe: a viewer joined, or lost a frame.
    pub fn request_keyframe(&mut self) -> Result<()> {
        self.inner.cut()?;
        Ok(())
    }

    /// Encode one frame. Usually one chunk comes back; a backend that holds
    /// frames may return none now and two later.
    pub fn encode(&mut self, frame: &Frame) -> Result<Vec<Chunk>> {
        let out = self.inner.encode(frame)?;
        Ok(out.into_iter().map(Chunk::from_encoded).collect())
    }

    /// Hand back anything the backend still holds.
    pub fn flush(&mut self) -> Result<Vec<Chunk>> {
        let out = self.inner.flush()?;
        Ok(out.into_iter().map(Chunk::from_encoded).collect())
    }
}

/// A timestamp for a frame `elapsed` into the share.
pub fn timestamp(elapsed: Duration) -> Result<moq_net::Timestamp> {
    moq_net::Timestamp::from_micros(elapsed.as_micros() as u64)
        .map_err(|e| Error::Settings(e.to_string()))
}
