//! The Whisper engine (#240, Stephen's pick on blocker b1): the default
//! microphone through cpal, mixed to mono and resampled to 16 kHz, and
//! whisper.cpp (whisper-rs) on this device. No audio leaves the machine.
//!
//! Live text: the phrase being spoken is kept as samples. About every
//! [`TICK`] the worker looks at it: once it has speech it is transcribed
//! whole and sent as a partial (each partial replaces the last); when a
//! pause of [`PAUSE`] follows speech, or the phrase reaches [`MAX_PHRASE`],
//! it is transcribed once more and sent as the final, and a new phrase
//! starts. Stopping finalizes what was said so far, then `Ended`.
//!
//! Speech vs silence is a simple energy gate over 30 ms frames with an
//! adaptive noise floor (`Gate`), enough to cut phrases; Whisper itself
//! decides the words.
//!
//! A feed session (#656) hears pushed audio instead ([`Fed`]): it checks
//! for a phrase's end after every frame, since a lagging session takes
//! seconds of backlog at once, and times each phrase by the samples fed.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::{ErrorCode, Event, Options, Sink, StartError, Status};

/// Whisper's input rate.
pub(crate) const RATE: usize = 16_000;
/// How often the worker re-transcribes the phrase.
const TICK: Duration = Duration::from_millis(700);
/// Silence after speech that ends a phrase.
const PAUSE: Duration = Duration::from_millis(800);
/// The longest phrase before it is cut (Whisper's window is 30 s).
const MAX_PHRASE: Duration = Duration::from_secs(25);
/// Speech shorter than this is not transcribed (a click, a cough).
const MIN_SPEECH: Duration = Duration::from_millis(300);

pub(crate) fn status() -> Status {
    #[cfg(target_os = "macos")]
    if mac::mic_denied() {
        return Status::Denied(mac::MIC_SETTING.into());
    }
    match cpal::default_host().default_input_device() {
        Some(_) => Status::Ready,
        None => Status::Denied("No microphone was found.".into()),
    }
}

/// macOS's microphone privacy switch (#240). When it is off, Core Audio
/// hands the app silence instead of an error, so the status reads it. A
/// first use (not determined) is Ready: opening the stream asks, with
/// NSMicrophoneUsageDescription from the app's Info.plist.
#[cfg(target_os = "macos")]
pub(crate) mod mac {
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};

    pub(crate) const MIC_SETTING: &str = "Allow Irori to use the microphone in System Settings > Privacy & Security > Microphone, then try again.";
    const MIC_PANE: &str =
        "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone";

    pub(crate) fn mic_denied() -> bool {
        let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
            return false;
        };
        let st = unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) };
        st == AVAuthorizationStatus::Denied || st == AVAuthorizationStatus::Restricted
    }

    /// Opens System Settings at Privacy & Security > Microphone.
    pub(crate) fn open_mic_settings() -> Result<(), String> {
        std::process::Command::new("/usr/bin/open")
            .arg(MIC_PANE)
            .status()
            .map_err(|e| e.to_string())
            .and_then(|s| {
                if s.success() {
                    Ok(())
                } else {
                    Err(format!("open exited with {s}"))
                }
            })
    }
}

pub(crate) struct Session {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    pub(crate) fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Where a session's audio comes from.
pub(crate) enum Source {
    /// The default microphone.
    Mic,
    /// 16 kHz mono samples played in real time (tests and the listen
    /// example: speech the engine can hear without a person talking).
    Samples(Vec<f32>),
    /// Audio pushed through a [`Feed`] (a call's cleaned mic).
    Fed(Arc<Fed>),
}

pub(crate) fn start(model: &Path, sink: Sink) -> Result<Session, StartError> {
    start_from(model, Source::Mic, Options::default(), sink)
}

pub(crate) fn start_feed(
    model: &Path,
    input_rate: u32,
    opts: Options,
    sink: Sink,
) -> Result<(Session, Feed), StartError> {
    if input_rate == 0 {
        return Err(StartError {
            code: ErrorCode::Other,
            message: "the feed's sample rate is 0".into(),
        });
    }
    let fed = Arc::new(Fed::new(input_rate as usize));
    let feed = Feed {
        fed: Arc::downgrade(&fed),
    };
    let session = start_from(model, Source::Fed(fed), opts, sink)?;
    Ok((session, feed))
}

/// The push handle; the session's thread owns the [`Fed`], so pushing
/// after it ends does nothing.
#[derive(Clone)]
pub(crate) struct Feed {
    fed: Weak<Fed>,
}

impl Feed {
    pub(crate) fn push(&self, mono: &[f32]) {
        if let Some(f) = self.fed.upgrade() {
            f.push(mono);
        }
    }
}

/// A feed session's backlog: pushed audio at 16 kHz, waiting for the
/// worker.
pub(crate) struct Fed {
    rate: usize,
    backlog: Mutex<Backlog>,
    /// Input samples dropped because the worker held the lock.
    lost: AtomicU64,
}

struct Backlog {
    rs: Resampler,
    samples: VecDeque<f32>,
    /// 16 kHz samples dropped by the cap since the worker last took them.
    dropped: u64,
}

impl Fed {
    fn new(rate: usize) -> Self {
        Fed {
            rate,
            backlog: Mutex::new(Backlog {
                rs: Resampler::new(rate),
                samples: VecDeque::new(),
                dropped: 0,
            }),
            lost: AtomicU64::new(0),
        }
    }

    /// Never blocks: on contention the chunk is dropped and counted. Keeps
    /// at most [`MAX_PHRASE`] of the newest audio.
    fn push(&self, mono: &[f32]) {
        let Ok(mut b) = self.backlog.try_lock() else {
            self.lost.fetch_add(mono.len() as u64, Ordering::Relaxed);
            return;
        };
        let b = &mut *b;
        b.rs.each(&mut mono.iter().copied(), |s| b.samples.push_back(s));
        let cap = samples(MAX_PHRASE);
        if b.samples.len() > cap {
            let n = b.samples.len() - cap;
            b.samples.drain(..n);
            b.dropped += n as u64;
        }
    }

    /// The backlog, and how many 16 kHz samples were lost before it.
    fn take(&self) -> (Vec<f32>, u64) {
        let lost = self.lost.swap(0, Ordering::Relaxed);
        let (q, dropped) = {
            let mut b = self.backlog.lock().unwrap_or_else(|e| e.into_inner());
            (
                std::mem::take(&mut b.samples),
                std::mem::take(&mut b.dropped),
            )
        };
        let gap = dropped + lost * RATE as u64 / self.rate as u64;
        (Vec::from(q), gap)
    }
}

pub(crate) fn start_from(
    model: &Path,
    source: Source,
    opts: Options,
    sink: Sink,
) -> Result<Session, StartError> {
    if !model.is_file() {
        return Err(StartError {
            code: ErrorCode::NoModel,
            message: "The speech model is not downloaded yet.".into(),
        });
    }
    let model = model.to_path_buf();
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), StartError>>(1);
    let st = stop.clone();
    let name = match source {
        Source::Fed(_) => "irori-voice-call",
        _ => "irori-voice",
    };
    let thread = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || run(model, source, opts, sink, st, ready_tx))
        .map_err(|e| StartError {
            code: ErrorCode::Other,
            message: e.to_string(),
        })?;
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(Session {
            stop,
            thread: Some(thread),
        }),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => {
            let _ = thread.join();
            Err(StartError {
                code: ErrorCode::Other,
                message: "the speech thread ended".into(),
            })
        }
    }
}

/// The model loaded last, and from where.
type Loaded = Option<(PathBuf, Arc<WhisperContext>)>;

/// The model kept between sessions (loading takes a moment).
fn loaded() -> &'static Mutex<Loaded> {
    static CTX: OnceLock<Mutex<Loaded>> = OnceLock::new();
    CTX.get_or_init(|| Mutex::new(None))
}

/// Frees the kept model (see [`crate::release`]). A running session keeps
/// its own reference until it ends.
pub(crate) fn release() {
    let ctx = loaded().lock().unwrap_or_else(|e| e.into_inner()).take();
    drop(ctx);
}

/// The loaded model, kept between sessions (loading takes a moment).
fn context(model: &Path) -> Result<Arc<WhisperContext>, StartError> {
    let mut g = loaded().lock().map_err(|e| StartError {
        code: ErrorCode::Other,
        message: e.to_string(),
    })?;
    if let Some((p, c)) = g.as_ref()
        && p == model
    {
        return Ok(c.clone());
    }
    quiet_logs();
    let path = model.to_str().ok_or_else(|| StartError {
        code: ErrorCode::Other,
        message: "the model path is not UTF-8".into(),
    })?;
    let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default()).map_err(
        |e| StartError {
            code: ErrorCode::NoModel,
            message: format!("The speech model could not be loaded ({e}); download it again."),
        },
    )?;
    let ctx = Arc::new(ctx);
    *g = Some((model.to_path_buf(), ctx.clone()));
    Ok(ctx)
}

/// whisper.cpp prints to stderr by default; route it to `log` (unused).
fn quiet_logs() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(whisper_rs::install_logging_hooks);
}

fn run(
    model: PathBuf,
    source: Source,
    opts: Options,
    sink: Sink,
    stop: Arc<AtomicBool>,
    ready: mpsc::SyncSender<Result<(), StartError>>,
) {
    let ctx = match context(&model) {
        Ok(c) => c,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let mut state = match ctx.create_state() {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(StartError {
                code: ErrorCode::Other,
                message: format!("Whisper could not start ({e})."),
            }));
            return;
        }
    };
    let buf: Arc<Mutex<Vec<f32>>> = Arc::default();
    let failed: Arc<Mutex<Option<String>>> = Arc::default();
    // The stream lives on this thread (cpal streams are not Send on every
    // platform) and stops when it drops at the end.
    let mut fed = None;
    let _audio = match source {
        Source::Mic => match open_mic(buf.clone(), failed.clone()) {
            Ok(s) => Audio::Mic(s),
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        },
        Source::Samples(v) => Audio::Player(play(v, buf.clone(), stop.clone())),
        Source::Fed(f) => {
            fed = Some(f);
            Audio::Fed
        }
    };
    let _ = ready.send(Ok(()));

    let threads = threads(opts.max_threads);
    let timed = fed.is_some();
    let mut phrase = Phrase::default();
    let mut gate = Gate::default();
    let mut last_partial = String::new();
    // 16 kHz samples heard since the start: the feed's clock.
    let mut consumed = 0u64;
    let finish = |state: &mut whisper_rs::WhisperState, phrase: &Phrase, consumed: u64| {
        let text = transcribe(state, &phrase.samples, Pass::Final, threads);
        if !timed {
            sink(Event::Final(text));
        } else if !text.is_empty() {
            let (start_ms, end_ms) = phrase.span(consumed);
            sink(Event::Segment {
                text,
                start_ms,
                end_ms,
            });
        }
    };
    let mut next_tick = Instant::now() + TICK;
    loop {
        let stopping = stop.load(Ordering::SeqCst);
        if let Some(msg) = failed.lock().ok().and_then(|mut f| f.take()) {
            sink(Event::Error {
                code: ErrorCode::NoMic,
                message: msg,
            });
            break;
        }
        // Move what the microphone captured (or was fed) into the phrase.
        let fresh = match &fed {
            Some(f) => {
                let (fresh, gap) = f.take();
                consumed += gap;
                fresh
            }
            None => buf
                .lock()
                .map(|mut b| std::mem::take(&mut *b))
                .unwrap_or_default(),
        };
        for frame in fresh.chunks(FRAME) {
            let speech = gate.speech(frame);
            phrase.push(frame, speech);
            consumed += frame.len() as u64;
            if !timed {
                continue;
            }
            if phrase.ended() {
                finish(&mut state, &phrase, consumed);
                phrase = Phrase::default();
                last_partial.clear();
            } else if !phrase.has_speech() && phrase.samples.len() >= samples(PAUSE) {
                phrase.trim_silence();
            }
        }
        if stopping {
            if phrase.has_speech() {
                finish(&mut state, &phrase, consumed);
            }
            break;
        }
        let now = Instant::now();
        if now < next_tick {
            std::thread::sleep(Duration::from_millis(30));
            continue;
        }
        next_tick = now + TICK;
        if !phrase.has_speech() {
            // Keep only a short lead-in of silence before speech starts.
            phrase.trim_silence();
            continue;
        }
        if phrase.ended() {
            finish(&mut state, &phrase, consumed);
            phrase = Phrase::default();
            last_partial.clear();
        } else if opts.partials {
            let text = transcribe(&mut state, &phrase.samples, Pass::Partial, threads);
            if !text.is_empty() && text != last_partial {
                sink(Event::Partial(text.clone()));
                last_partial = text;
            }
        }
    }
    sink(Event::Ended);
}

/// Keeps a session's audio source alive.
enum Audio {
    Mic(#[allow(dead_code)] cpal::Stream),
    Player(#[allow(dead_code)] JoinHandle<()>),
    Fed,
}

/// Feeds `v` into `buf` at real time, 30 ms at a time, then silence.
fn play(v: Vec<f32>, buf: Arc<Mutex<Vec<f32>>>, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let silence = vec![0.0f32; FRAME];
        let mut chunks = v.chunks(FRAME);
        while !stop.load(Ordering::SeqCst) {
            let c = chunks.next().unwrap_or(&silence);
            if let Ok(mut b) = buf.lock() {
                b.extend_from_slice(c);
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    })
}

/// Samples per 30 ms frame at 16 kHz.
const FRAME: usize = RATE * 30 / 1000;

/// The phrase being spoken.
#[derive(Default)]
pub(crate) struct Phrase {
    pub(crate) samples: Vec<f32>,
    /// Samples of speech in it.
    speech: usize,
    /// Samples of silence since the last speech.
    trailing: usize,
}

impl Phrase {
    pub(crate) fn push(&mut self, frame: &[f32], speech: bool) {
        self.samples.extend_from_slice(frame);
        if speech {
            self.speech += frame.len();
            self.trailing = 0;
        } else {
            self.trailing += frame.len();
        }
    }

    pub(crate) fn has_speech(&self) -> bool {
        self.speech >= samples(MIN_SPEECH)
    }

    /// A pause after speech, or too long.
    pub(crate) fn ended(&self) -> bool {
        self.has_speech()
            && (self.trailing >= samples(PAUSE) || self.samples.len() >= samples(MAX_PHRASE))
    }

    /// Where the phrase lies, in ms on the clock `consumed` (16 kHz samples
    /// heard so far, this phrase's last included): from its first sample
    /// (the lead-in included) to its last speech.
    pub(crate) fn span(&self, consumed: u64) -> (u64, u64) {
        let ms = |n: u64| n * 1000 / RATE as u64;
        let start = consumed.saturating_sub(self.samples.len() as u64);
        let end = consumed.saturating_sub(self.trailing as u64);
        (ms(start), ms(end))
    }

    /// Before any speech: keep the last 300 ms so a word's start isn't cut.
    /// A click too short to be speech is forgotten once a pause follows it.
    pub(crate) fn trim_silence(&mut self) {
        if !self.has_speech() && self.trailing >= samples(PAUSE) {
            self.speech = 0;
        }
        let keep = samples(Duration::from_millis(300));
        if self.speech == 0 && self.samples.len() > keep {
            let cut = self.samples.len() - keep;
            self.samples.drain(..cut);
            self.trailing = self.samples.len();
        }
    }
}

fn samples(d: Duration) -> usize {
    (d.as_millis() as usize) * RATE / 1000
}

/// An energy gate with an adaptive noise floor.
pub(crate) struct Gate {
    floor: f32,
}

impl Default for Gate {
    fn default() -> Self {
        Gate { floor: 0.002 }
    }
}

impl Gate {
    pub(crate) fn speech(&mut self, frame: &[f32]) -> bool {
        if frame.is_empty() {
            return false;
        }
        let rms = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt();
        let speech = rms > (self.floor * 3.0).max(0.008);
        // The floor follows quiet frames quickly and loud ones slowly.
        let rate = if rms < self.floor {
            0.2
        } else if speech {
            0.001
        } else {
            0.05
        };
        self.floor += (rms - self.floor) * rate;
        self.floor = self.floor.clamp(0.0005, 0.05);
        speech
    }
}

/// A partial is quick: the encoder sees only the phrase's length
/// (`audio_ctx`, 5-7x faster) and the tokens are capped, because a short
/// context can loop. A final uses the whole window, which is exact.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pass {
    Partial,
    Final,
}

fn transcribe(
    state: &mut whisper_rs::WhisperState,
    audio: &[f32],
    pass: Pass,
    threads: i32,
) -> String {
    let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    p.set_language(Some("en"));
    p.set_n_threads(threads);
    p.set_no_context(true);
    // No temperature fallback: a retry would multiply the time for a partial.
    p.set_temperature_inc(0.0);
    p.set_single_segment(true);
    if pass == Pass::Partial {
        let secs = audio.len() / RATE + 1;
        // 50 encoder frames per second of audio, plus a margin.
        p.set_audio_ctx(((audio.len() * 50 / RATE) + 64).min(1500) as i32);
        // About four words a second is fast speech; leave room.
        p.set_max_tokens((secs * 8 + 8) as i32);
    }
    p.set_no_timestamps(true);
    p.set_suppress_blank(true);
    p.set_suppress_nst(true);
    p.set_print_progress(false);
    p.set_print_realtime(false);
    p.set_print_timestamps(false);
    p.set_print_special(false);
    // Whisper wants at least 1 s; pad short phrases with silence.
    let mut padded;
    let audio = if audio.len() < RATE + RATE / 10 {
        padded = audio.to_vec();
        padded.resize(RATE + RATE / 10, 0.0);
        &padded[..]
    } else {
        audio
    };
    let t = Instant::now();
    let r = state.full(p, audio);
    if std::env::var_os("IRORI_VOICE_TRACE").is_some() {
        eprintln!(
            "whisper: {} samples in {} ms",
            audio.len(),
            t.elapsed().as_millis()
        );
    }
    if r.is_err() {
        return String::new();
    }
    let mut text = String::new();
    for seg in state.as_iter() {
        if let Ok(s) = seg.to_str_lossy() {
            text.push_str(&s);
        }
    }
    clean(&text)
}

/// Whisper's CPU threads: half the cores, at most 4. ggml's thread pool
/// spin-waits, so more threads than it can keep busy (other apps, hyper-
/// threads, efficiency cores) is ruinous: on the PC a pass took ~1 s with
/// 4 threads and 10-160 s with 8 or 16 (examples/bench.rs). `cap` lowers
/// it further (a call's transcriber takes at most 2).
fn threads(cap: Option<i32>) -> i32 {
    let n = std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(1, 4) as i32)
        .unwrap_or(2);
    cap.map_or(n, |c| n.min(c)).max(1)
}

/// Drops Whisper's non-speech markers ("[BLANK_AUDIO]", "(wind blowing)")
/// and squeezes spaces.
pub(crate) fn clean(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut depth = 0i32;
    for c in raw.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let squeezed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    collapse_repeats(&squeezed)
}

/// Whisper can loop on a sentence ("Send it. Send it. Send it."); keep
/// one of each run of the same sentence.
pub(crate) fn collapse_repeats(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        let end_of_sentence =
            matches!(b, b'.' | b'!' | b'?') && (i + 1 == bytes.len() || bytes[i + 1] == b' ');
        if end_of_sentence {
            let sent = text[start..=i].trim();
            if out.last() != Some(&sent) && !sent.is_empty() {
                out.push(sent);
            }
            start = i + 1;
        }
    }
    let rest = text[start..].trim();
    // An unfinished copy of the last sentence is the loop being cut off.
    if !rest.is_empty() && !out.last().is_some_and(|l| l.starts_with(rest)) {
        out.push(rest);
    }
    out.join(" ")
}

/// Opens the default microphone; its samples go into `buf` as 16 kHz mono.
fn open_mic(
    buf: Arc<Mutex<Vec<f32>>>,
    failed: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, StartError> {
    let host = cpal::default_host();
    let dev = host.default_input_device().ok_or_else(|| StartError {
        code: ErrorCode::NoMic,
        message: "No microphone was found.".into(),
    })?;
    let cfg = dev.default_input_config().map_err(mic_error)?;
    let channels = cfg.channels() as usize;
    let rate = cfg.sample_rate() as usize;
    let fmt = cfg.sample_format();
    let config: cpal::StreamConfig = cfg.into();
    let mut rs = Resampler::new(rate);
    let on_err = move |e: cpal::Error| {
        if e.kind() == cpal::ErrorKind::DeviceChanged {
            return;
        }
        if let Ok(mut f) = failed.lock() {
            *f = Some(format!("The microphone stopped ({e})."));
        }
    };
    let mut push = move |mono: &mut dyn Iterator<Item = f32>| {
        let out = rs.feed(mono);
        if let Ok(mut b) = buf.lock() {
            b.extend_from_slice(&out);
        }
    };
    let stream = match fmt {
        cpal::SampleFormat::F32 => dev.build_input_stream(
            config,
            move |d: &[f32], _: &_| push(&mut mono(d, channels, |s| s)),
            on_err,
            None,
        ),
        cpal::SampleFormat::I16 => dev.build_input_stream(
            config,
            move |d: &[i16], _: &_| push(&mut mono(d, channels, |s| s as f32 / 32768.0)),
            on_err,
            None,
        ),
        cpal::SampleFormat::U16 => dev.build_input_stream(
            config,
            move |d: &[u16], _: &_| {
                push(&mut mono(d, channels, |s| (s as f32 - 32768.0) / 32768.0))
            },
            on_err,
            None,
        ),
        other => {
            return Err(StartError {
                code: ErrorCode::NoMic,
                message: format!("The microphone's sample format ({other}) is not supported."),
            });
        }
    }
    .map_err(mic_error)?;
    stream.play().map_err(mic_error)?;
    Ok(stream)
}

/// Averages the channels of each frame.
fn mono<'a, T: Copy + 'a>(
    data: &'a [T],
    channels: usize,
    to_f32: impl Fn(T) -> f32 + 'a,
) -> impl Iterator<Item = f32> + 'a {
    let ch = channels.max(1);
    data.chunks(ch)
        .map(move |f| f.iter().map(|&s| to_f32(s)).sum::<f32>() / f.len() as f32)
}

fn mic_error(e: cpal::Error) -> StartError {
    match e.kind() {
        cpal::ErrorKind::PermissionDenied => StartError {
            code: ErrorCode::MicDenied,
            message: crate::MIC_SETTING.to_string(),
        },
        cpal::ErrorKind::DeviceNotAvailable => StartError {
            code: ErrorCode::NoMic,
            message: "No microphone was found.".into(),
        },
        cpal::ErrorKind::DeviceBusy => StartError {
            code: ErrorCode::NoMic,
            message: "The microphone is in use by another app.".into(),
        },
        _ => StartError {
            code: ErrorCode::NoMic,
            message: format!("The microphone could not be opened ({e})."),
        },
    }
}

/// A streaming resampler to 16 kHz: a box filter over each output step
/// (enough anti-aliasing for speech) with linear positions.
pub(crate) struct Resampler {
    step: f64,
    pos: f64,
    acc: f32,
    n: u32,
}

impl Resampler {
    pub(crate) fn new(from: usize) -> Self {
        Resampler {
            step: from as f64 / RATE as f64,
            pos: 0.0,
            acc: 0.0,
            n: 0,
        }
    }

    pub(crate) fn feed(&mut self, input: &mut dyn Iterator<Item = f32>) -> Vec<f32> {
        let mut out = Vec::new();
        self.each(input, |s| out.push(s));
        out
    }

    /// Like `feed`, handing each output sample to `out` (no allocation).
    pub(crate) fn each(&mut self, input: &mut dyn Iterator<Item = f32>, mut out: impl FnMut(f32)) {
        for s in input {
            self.acc += s;
            self.n += 1;
            self.pos += 1.0;
            if self.pos >= self.step {
                self.pos -= self.step;
                out(self.acc / self.n as f32);
                self.acc = 0.0;
                self.n = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_drops_markers() {
        assert_eq!(clean(" [BLANK_AUDIO]"), "");
        assert_eq!(clean(" Hello (coughs) world. "), "Hello world.");
        assert_eq!(clean("  card  #240 "), "card #240");
    }

    #[test]
    fn repeats_collapse() {
        assert_eq!(
            collapse_repeats("Send it. Send it. Send it. Send"),
            "Send it."
        );
        assert_eq!(
            collapse_repeats("One. Two. Two? Three"),
            "One. Two. Two? Three"
        );
        assert_eq!(
            collapse_repeats("Check #240. Then ship."),
            "Check #240. Then ship."
        );
        assert_eq!(collapse_repeats("no stop"), "no stop");
    }

    #[test]
    fn resampler_rates() {
        let mut r = Resampler::new(48_000);
        let out = r.feed(&mut std::iter::repeat_n(0.5f32, 48_000));
        assert_eq!(out.len(), 16_000);
        assert!(out.iter().all(|&s| (s - 0.5).abs() < 1e-6));
        let mut r = Resampler::new(44_100);
        let out = r.feed(&mut std::iter::repeat_n(0.0f32, 44_100));
        assert!((15_999..=16_001).contains(&out.len()));
    }

    #[test]
    fn phrase_ends_after_a_pause() {
        let mut p = Phrase::default();
        let loud = vec![0.2f32; FRAME];
        let quiet = vec![0.0f32; FRAME];
        for _ in 0..20 {
            p.push(&loud, true);
        }
        assert!(p.has_speech() && !p.ended());
        for _ in 0..30 {
            p.push(&quiet, false);
        }
        assert!(p.ended());
    }

    #[test]
    fn phrase_span_is_on_the_fed_clock() {
        let mut p = Phrase::default();
        let loud = vec![0.2f32; FRAME];
        let quiet = vec![0.0f32; FRAME];
        // 3 s of other phrases before this one.
        let mut consumed = 3 * RATE as u64;
        for _ in 0..10 {
            p.push(&quiet, false);
            consumed += FRAME as u64;
        }
        for _ in 0..40 {
            p.push(&loud, true);
            consumed += FRAME as u64;
        }
        // Cut mid-speech (MAX_PHRASE or stop): it ends where the audio ends.
        assert_eq!(p.span(consumed), (3000, 3000 + 50 * 30));
        for _ in 0..30 {
            p.push(&quiet, false);
            consumed += FRAME as u64;
        }
        assert!(p.ended());
        // Ended by a pause: the end is where the speech stopped.
        assert_eq!(p.span(consumed), (3000, 3000 + 50 * 30));
    }

    #[test]
    fn a_click_is_forgotten_after_a_pause() {
        let mut p = Phrase::default();
        let quiet = vec![0.0f32; FRAME];
        p.push(&[0.2f32; FRAME], true);
        for _ in 0..30 {
            p.push(&quiet, false);
        }
        p.trim_silence();
        assert_eq!(p.samples.len(), samples(Duration::from_millis(300)));
        assert_eq!(p.speech, 0);
    }

    #[test]
    fn feed_backlog_keeps_the_newest() {
        let f = Fed::new(RATE);
        let cap = samples(MAX_PHRASE);
        let chunk: Vec<f32> = (0..160).map(|i| i as f32).collect();
        let pushes = cap / 160 + 50;
        for k in 0..pushes {
            let c: Vec<f32> = chunk.iter().map(|s| s + (k * 1000) as f32).collect();
            f.push(&c);
        }
        let (got, gap) = f.take();
        assert_eq!(got.len(), cap);
        assert_eq!(gap, (pushes * 160 - cap) as u64);
        assert_eq!(*got.last().unwrap(), ((pushes - 1) * 1000 + 159) as f32);
        // The gap is reported once.
        f.push(&chunk);
        assert_eq!(f.take(), (chunk.clone(), 0));
    }

    #[test]
    fn feed_resamples_and_counts_contention() {
        let f = Fed::new(48_000);
        f.push(&[0.5f32; 480]);
        let (got, gap) = f.take();
        assert_eq!((got.len(), gap), (160, 0));
        {
            let _held = f.backlog.lock().unwrap();
            f.push(&[0.5f32; 480]);
        }
        let (got, gap) = f.take();
        assert_eq!((got.len(), gap), (0, 160));
    }

    #[test]
    fn feed_is_a_no_op_after_the_session() {
        let fed = Arc::new(Fed::new(RATE));
        let feed = Feed {
            fed: Arc::downgrade(&fed),
        };
        feed.push(&[0.1; 16]);
        assert_eq!(fed.take().0.len(), 16);
        drop(fed);
        feed.push(&[0.1; 16]);
    }

    #[test]
    fn feed_is_send_sync() {
        fn check<T: Send + Sync + Clone>() {}
        check::<crate::Feed>();
    }

    #[test]
    fn threads_take_the_cap() {
        assert!(threads(Some(2)) <= 2);
        assert_eq!(threads(Some(0)), 1);
        assert!((1..=4).contains(&threads(None)));
    }

    #[test]
    fn gate_tells_speech_from_hiss() {
        let mut g = Gate::default();
        let hiss: Vec<f32> = (0..FRAME)
            .map(|i| if i % 2 == 0 { 0.001 } else { -0.001 })
            .collect();
        let voice: Vec<f32> = (0..FRAME)
            .map(|i| ((i as f32) * 0.05).sin() * 0.2)
            .collect();
        for _ in 0..50 {
            assert!(!g.speech(&hiss));
        }
        assert!(g.speech(&voice));
    }
}
