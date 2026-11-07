//! Watching a screen share (#658): the React app's SharePlayer
//! (lib/share.ts, WebCodecs) and the old share_attach / share_detach /
//! share_keyframe commands (main:crates/window/src/commands.rs).
//!
//! [`Viewer::attach`] hands the Core a `ShareSink` that only queues each
//! window message ([`super::logic::parse_share_frame`]'s framing) for a
//! decode thread. That thread owns a moq-video H.264 decoder (VideoToolbox
//! on the Mac, Media Foundation on Windows), turns each picture into RGBA
//! and puts it in a `rune::Pixels`, which wakes the window by itself: no
//! frame goes through the UI thread. Latency over smoothness, as the
//! React player: a backed-up queue drops deltas until the next keyframe,
//! and a decode error opens a fresh decoder that waits for a keyframe,
//! which is asked for (at most once per 2 s).

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use irori_window::Window;
use rune::{AppHandle, Pixels};

use super::logic::{ShareFrame, parse_share_frame};
use crate::ui::State;

/// What the stage shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// Watching; no picture yet.
    Waiting,
    Playing,
    /// This device has no decoder for it.
    Unsupported,
}

impl Look {
    fn from_u8(v: u8) -> Look {
        match v {
            1 => Look::Playing,
            2 => Look::Unsupported,
            _ => Look::Waiting,
        }
    }
}

/// One share being watched: its picture and how it looks.
pub struct Viewer {
    pub share_id: String,
    pub pixels: Pixels,
    look: Arc<AtomicU8>,
    /// Cleared when the viewer goes: the sink then answers false and the
    /// decode thread ends.
    on: Arc<AtomicBool>,
}

impl Viewer {
    pub fn look(&self) -> Look {
        Look::from_u8(self.look.load(Ordering::Relaxed))
    }

    /// Starts watching the call's share `share_id`: the sharer starts
    /// sending here. Fast (the Core only swaps its sink and tells the
    /// sharer), so it runs on the UI thread, in order with
    /// [`Viewer::detach`].
    pub fn attach(w: &Arc<Window>, h: Option<AppHandle<State>>, share_id: &str) -> Result<Viewer, String> {
        let pixels = Pixels::new();
        let look = Arc::new(AtomicU8::new(0));
        let on = Arc::new(AtomicBool::new(true));
        let (tx, rx) = channel::<Vec<u8>>();
        let sink_on = on.clone();
        let sink: irori_core::ShareSink = Arc::new(move |b: Vec<u8>| {
            sink_on.load(Ordering::Relaxed) && tx.send(b).is_ok()
        });
        w.core.share_attach(sink)?;
        let job = Job {
            w: Arc::downgrade(w),
            h,
            pixels: pixels.clone(),
            look: look.clone(),
            on: on.clone(),
        };
        std::thread::Builder::new()
            .name("irori-share-view".into())
            .spawn(move || job.run(rx))
            .map_err(|e| e.to_string())?;
        Ok(Viewer {
            share_id: share_id.to_string(),
            pixels,
            look,
            on,
        })
    }

    /// Stops watching: the sharer stops sending here.
    pub fn detach(self, w: &Window) {
        self.on.store(false, Ordering::Relaxed);
        w.core.share_detach();
    }
}

struct Job {
    w: Weak<Window>,
    h: Option<AppHandle<State>>,
    pixels: Pixels,
    look: Arc<AtomicU8>,
    on: Arc<AtomicBool>,
}

/// How many access units may wait before deltas are dropped.
const BACKLOG: usize = 3;

impl Job {
    fn set_look(&self, l: Look) {
        let v = match l {
            Look::Waiting => 0,
            Look::Playing => 1,
            Look::Unsupported => 2,
        };
        if self.look.swap(v, Ordering::Relaxed) != v
            && let Some(h) = &self.h
        {
            // The stage's words change (the picture itself wakes the
            // window through Pixels).
            h.update(|_: &mut State, _| {});
        }
    }

    fn run(self, rx: Receiver<Vec<u8>>) {
        let mut p = Player::new();
        while let Ok(first) = rx.recv() {
            if !self.on.load(Ordering::Relaxed) {
                break;
            }
            let mut batch = vec![first];
            while let Ok(b) = rx.try_recv() {
                batch.push(b);
            }
            p.feed(&self, batch);
        }
    }

    fn ask_keyframe(&self, p: &mut Player) {
        let now = Instant::now();
        if p.last_ask.is_some_and(|t| now - t < Duration::from_secs(2)) {
            return;
        }
        p.last_ask = Some(now);
        if let Some(w) = self.w.upgrade() {
            w.core.share_keyframe();
        }
    }
}

struct Player {
    dec: Option<Decoder>,
    /// The last config, to open a fresh decoder after an error.
    config: Option<(String, Vec<u8>)>,
    need_key: bool,
    last_ask: Option<Instant>,
}

impl Player {
    fn new() -> Player {
        Player {
            dec: None,
            config: None,
            need_key: true,
            last_ask: None,
        }
    }

    fn feed(&mut self, job: &Job, batch: Vec<Vec<u8>>) {
        // A backed-up queue: keep only what follows its last config, and of
        // the chunks only from the last keyframe on (or none, asking for
        // one), so the picture catches up at once.
        let frames: Vec<ShareFrame<'_>> = batch.iter().filter_map(|b| parse_share_frame(b)).collect();
        let chunks = frames.iter().filter(|f| matches!(f, ShareFrame::Chunk { .. })).count();
        let mut from = 0;
        if chunks > BACKLOG {
            let last_key = frames
                .iter()
                .rposition(|f| matches!(f, ShareFrame::Config { .. } | ShareFrame::Chunk { key: true, .. }));
            match last_key {
                Some(i) => from = i,
                None => {
                    self.need_key = true;
                    job.ask_keyframe(self);
                    return;
                }
            }
            // A keyframe after a config in the same batch still needs that
            // config first.
            if matches!(frames[from], ShareFrame::Chunk { .. })
                && let Some(c) = frames[..from].iter().rposition(|f| matches!(f, ShareFrame::Config { .. }))
            {
                self.configure(job, &frames[c]);
            }
        }
        for f in &frames[from..] {
            match f {
                ShareFrame::Config { .. } => self.configure(job, f),
                ShareFrame::Chunk { key, timestamp_us, data } => self.chunk(job, *key, *timestamp_us, data),
            }
        }
    }

    fn configure(&mut self, job: &Job, f: &ShareFrame<'_>) {
        let ShareFrame::Config { codec, description } = f else { return };
        self.config = Some((codec.to_string(), description.to_vec()));
        self.open(job);
    }

    fn open(&mut self, job: &Job) {
        self.dec = None;
        self.need_key = true;
        let Some((codec, desc)) = &self.config else { return };
        match Decoder::new(codec, desc) {
            Ok(d) => self.dec = Some(d),
            Err(e) => {
                crate::call::log(&format!("share: no decoder for {codec}: {e}"));
                job.set_look(Look::Unsupported);
            }
        }
    }

    fn chunk(&mut self, job: &Job, key: bool, ts: u64, data: &[u8]) {
        if self.dec.is_none() {
            return;
        }
        if !key && self.need_key {
            job.ask_keyframe(self);
            return;
        }
        let Some(d) = self.dec.as_mut() else { return };
        match d.decode(data, ts, key) {
            Ok(Some((w, h, rgba))) => {
                if key {
                    self.need_key = false;
                }
                job.pixels.set(w, h, rgba);
                job.set_look(Look::Playing);
            }
            Ok(None) => {
                if key {
                    self.need_key = false;
                }
            }
            Err(e) => {
                crate::call::log(&format!("share: decode: {e}"));
                // A fresh decoder for the same config waits for a keyframe.
                self.open(job);
                job.ask_keyframe(self);
            }
        }
    }
}

/// The platform decoder, H.264 to tightly packed RGBA.
struct Decoder {
    inner: moq_video::decode::Decoder,
}

impl Decoder {
    fn new(codec: &str, description: &[u8]) -> Result<Decoder, String> {
        let codec: hang::catalog::VideoCodec = codec.parse().map_err(|e| format!("{e}"))?;
        let mut cat = hang::catalog::VideoConfig::new(codec);
        if !description.is_empty() {
            cat.description = Some(bytes::Bytes::copy_from_slice(description));
        }
        cat.optimize_for_latency = Some(true);
        let mut cfg = moq_video::decode::Config::new();
        cfg.output = moq_video::Output::Cpu;
        let inner = moq_video::decode::Decoder::new(&cat, &cfg).map_err(|e| e.to_string())?;
        Ok(Decoder { inner })
    }

    /// The newest picture this access unit gave, if any.
    fn decode(&mut self, data: &[u8], ts_us: u64, key: bool) -> Result<Option<(u32, u32, Vec<u8>)>, String> {
        let ts = moq_net::Timestamp::from_micros(ts_us).map_err(|e| format!("{e:?}"))?;
        let payload = bytes::Bytes::copy_from_slice(data);
        let frames = self.inner.decode(&payload, ts, key).map_err(|e| e.to_string())?;
        let Some(f) = frames.into_iter().last() else { return Ok(None) };
        let rgba = f
            .surface
            .to_rgba(&moq_video::convert::Config::new())
            .map_err(|e| e.to_string())?;
        let (w, h, stride) = (rgba.width(), rgba.height(), rgba.stride());
        let row = w as usize * 4;
        let data = if stride == row {
            rgba.into_data()
        } else {
            let src = rgba.data();
            let mut out = Vec::with_capacity(row * h as usize);
            for y in 0..h as usize {
                out.extend_from_slice(&src[y * stride..y * stride + row]);
            }
            out
        };
        if data.len() != row * h as usize {
            return Err("decoded picture has the wrong size".into());
        }
        Ok(Some((w, h, data)))
    }
}

/// Whether this build has a platform H.264 decoder at all (the stage's
/// "no video decoder" line otherwise).
pub fn can_decode() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}
