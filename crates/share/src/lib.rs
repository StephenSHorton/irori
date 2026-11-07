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
pub use synthetic::{i420_frame, synthetic_frame};

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
    /// The source changed size (WGC on the PC ends a capture when the
    /// window is resized) and the capture must be opened again: call
    /// [`Capture::reopen`] and build a new [`Encoder`] at its new
    /// [`Capture::size`] (on the PC the old encoder is bound to the old
    /// capture's Direct3D device). Not the end of the share: that comes as
    /// [`Error::Video`] with `SourceUnavailable` (the window closed, the
    /// person pressed Stop in the menu bar, the grant was taken away).
    #[error("the source changed size; open the capture again")]
    Reopen,
}

pub type Result<T> = std::result::Result<T, Error>;

/// How to encode a share. [`Settings::default`] is the card's target: 1080p
/// at 10 frames a second.
#[derive(Clone, Debug)]
pub struct Settings {
    /// The box the picture is fitted into, keeping the source's aspect
    /// ratio: a 4:3 window shares at 1440x1080, not stretched to 16:9.
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
    /// Enforced on the PC (Media Foundation is asked for hardware encoders
    /// only). On the Mac moq-video opens VideoToolbox with no encoder
    /// specification, so VideoToolbox prefers its hardware encoder but is not
    /// made to use it.
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
    /// These settings with the box set to `size`: for the [`Encoder`] of a
    /// [`Capture`], `settings.sized(capture.size())`.
    pub fn sized(&self, (width, height): (u32, u32)) -> Self {
        Self {
            width,
            height,
            ..self.clone()
        }
    }

    fn rate(&self) -> Result<moq_video::Rate> {
        moq_video::Rate::new(self.fps, 1).map_err(|e| Error::Settings(e.to_string()))
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

/// How far a source may be scaled up to fill the box. On the Mac, window
/// and display sizes are listed in points and a Retina screen has two pixels
/// a point, so up to 2x is still the source's own detail. On the PC they are
/// pixels: never scale up.
#[cfg(target_os = "macos")]
const MAX_UP: f64 = 2.0;
#[cfg(not(target_os = "macos"))]
const MAX_UP: f64 = 1.0;

/// `w` x `h` scaled to fit inside `box_w` x `box_h`, keeping its aspect
/// ratio, scaled up by at most `max_up`; even (H.264's 4:2:0 needs it) and
/// at least 2 x 2.
pub fn fit(w: u32, h: u32, box_w: u32, box_h: u32, max_up: f64) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (box_w.max(2) & !1, box_h.max(2) & !1);
    }
    let scale = (f64::from(box_w) / f64::from(w))
        .min(f64::from(box_h) / f64::from(h))
        .min(max_up);
    let even = |v: f64| ((v.round() as u32) & !1).max(2);
    (even(f64::from(w) * scale), even(f64::from(h) * scale))
}

/// A running capture of one [`Source`], fitted into the settings' box with
/// its aspect ratio kept (see [`Capture::size`]).
pub struct Capture {
    stream: moq_video::capture::Stream,
    size: moq_video::Size,
    source: Source,
    settings: Settings,
}

impl Capture {
    /// Start capturing. The picture is the source fitted into
    /// `settings.width` x `settings.height`: on the Mac ScreenCaptureKit is
    /// asked for that size; on the PC WGC hands over the window's own size
    /// and [`Capture::next`] scales it on the GPU.
    pub async fn open(source: &Source, settings: &Settings) -> Result<Self> {
        let (bw, bh) = (settings.width, settings.height);
        let request = match source_size(source).await {
            Some((w, h)) => fit(w, h, bw, bh, MAX_UP),
            None => (bw, bh),
        };
        let mut config = moq_video::capture::Config::default();
        config.source = match source {
            Source::Window(id) => moq_video::capture::Source::Window(id.clone()),
            Source::Display(id) => moq_video::capture::Source::Display(id.clone()),
        };
        config.width = Some(request.0);
        config.height = Some(request.1);
        config.framerate = Some(settings.rate()?);
        config.cursor = settings.cursor;
        let stream = moq_video::capture::open(&config).await?;
        // The PC ignores the size asked for; fit what it delivers instead.
        let got = (stream.width(), stream.height());
        let (w, h) = if got == request {
            request
        } else {
            fit(got.0, got.1, bw, bh, 1.0)
        };
        Ok(Self {
            stream,
            size: moq_video::Size::new(w, h),
            source: source.clone(),
            settings: settings.clone(),
        })
    }

    /// The next picture, waiting for one. Pictures the encoder was too slow
    /// for are dropped, not queued. A frame at another size is scaled on the
    /// GPU first. [`Error::Reopen`] means the source changed size: call
    /// [`Capture::reopen`]. Cancel-safe: dropping the future loses no frame.
    pub async fn next(&mut self) -> Result<Frame> {
        let frame = self.stream.read().await?.ok_or(Error::Reopen)?;
        if frame.size() == self.size {
            return Ok(frame);
        }
        Ok(frame.resize(self.size, &moq_video::resize::Config::default())?)
    }

    /// Open the same source again after [`Error::Reopen`]. The size may have
    /// changed: build a new [`Encoder`] at [`Capture::size`].
    pub async fn reopen(&mut self) -> Result<()> {
        *self = Self::open(&self.source, &self.settings).await?;
        Ok(())
    }

    /// The size of the pictures [`Capture::next`] returns: build the
    /// [`Encoder`] at this size (see [`Settings::sized`]).
    pub fn size(&self) -> (u32, u32) {
        (self.size.width, self.size.height)
    }

    /// The size the platform is capturing at.
    pub fn captured_size(&self) -> (u32, u32) {
        (self.stream.width(), self.stream.height())
    }
}

/// The source's size as the platform lists it, if it can be found.
async fn source_size(source: &Source) -> Option<(u32, u32)> {
    match source {
        Source::Window(id) => windows()
            .await
            .ok()?
            .into_iter()
            .find(|w| &w.id == id)
            .map(|w| (w.width, w.height)),
        Source::Display(Some(id)) => displays()
            .await
            .ok()?
            .into_iter()
            .find(|d| &d.id == id)
            .map(|d| (d.width, d.height)),
        Source::Display(None) => None,
    }
}

/// An H.264 encoder on this machine's hardware. Its calls block: on the PC
/// `encode` waits on Media Foundation's event queue for the encoded sample.
/// Not `Send` either, since the Windows backend's COM apartment belongs to
/// the thread that opened it. So give it a thread of its own (with its own
/// current-thread runtime if the capture runs there too), never a shared
/// async runtime's worker.
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

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn fit_keeps_aspect_and_never_stretches() {
        // 16:9 fills the box.
        assert_eq!(fit(1920, 1080, 1920, 1080, 1.0), (1920, 1080));
        // 4:3 is pillarboxed, not stretched.
        assert_eq!(fit(1600, 1200, 1920, 1080, 1.0), (1440, 1080));
        // A tall window (#666's Discord, 1280x1392) stays tall.
        assert_eq!(fit(1280, 1392, 1920, 1080, 1.0), (992, 1080));
        // Small windows aren't scaled up on the PC...
        assert_eq!(fit(320, 412, 1920, 1080, 1.0), (320, 412));
        // ...and up to 2x (points to Retina pixels) on the Mac.
        assert_eq!(fit(320, 412, 1920, 1080, 2.0), (640, 824));
        // Odd sizes come out even.
        assert_eq!(fit(641, 481, 1920, 1080, 1.0), (640, 480));
        assert_eq!(fit(0, 0, 1920, 1080, 1.0), (1920, 1080));
    }
}
