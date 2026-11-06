//! The call engine: one thread that owns the mic, the speaker, the echo
//! canceller, the Opus encoder and a jitter buffer per peer.
//!
//! Every 10 ms of mic audio is cleaned (echo, noise, gain); two of those
//! make a 20 ms Opus frame, encoded once and sent as the same datagram to
//! every peer's [`Link`]. Playout pulls 20 ms from each peer's jitter
//! buffer, mixes them, gives the mix to the echo canceller as what the
//! speaker plays, and queues it for the device. Datagrams arrive from the
//! network through [`Engine::receive`], which only queues them.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ringbuf::traits::{Consumer, Observer, Producer};

use crate::device::{Devices, Opener};
use crate::dsp::{self, Dsp, DspConfig, EchoStats};
use crate::jitter::{JitterBuffer, JitterStats, Played};
use crate::probe::{Clicker, Onsets};
use crate::resample::Resample;
use crate::wire::{self, FLAG_DTX, FLAG_MUTED, FLAG_PROBE, FRAME, Header};

const F: usize = FRAME as usize;
/// Audio kept queued for the speaker, ms: enough to ride out the engine
/// thread's scheduling, little enough not to add delay.
const SPEAKER_QUEUE_MS: u32 = 30;
/// A quiet sender repeats its silence frame this often (in frames), so the
/// other side knows it is still there.
const DTX_KEEPALIVE: u32 = 20;
/// Mic audio waiting longer than this is dropped (the thread fell behind).
const MIC_BACKLOG_MS: u32 = 60;
/// How often the shared stats are refreshed.
const STATS_EVERY: Duration = Duration::from_millis(100);

/// A way to send a datagram to one peer. Must not block.
pub trait Link: Send + Sync {
    /// False if it was dropped (the link is gone, or its queue is full).
    fn send(&self, datagram: &[u8]) -> bool;
}

#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub dsp: DspConfig,
    /// Opus bitrate, bit/s.
    pub bitrate: i32,
    /// The loss Opus plans its in-band FEC for, percent.
    pub loss_percent: i32,
    /// call-dev's mouth-to-ear probe: play a click every two seconds and
    /// time it at the mic.
    pub probe: bool,
    pub muted: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            dsp: DspConfig::default(),
            bitrate: 28_000,
            loss_percent: 10,
            probe: false,
            muted: false,
        }
    }
}

/// One peer as the engine sees it.
#[derive(Debug, Clone, Default)]
pub struct PeerStats {
    pub id: String,
    pub jitter: JitterStats,
    /// Heard speaking in the last half second.
    pub speaking: bool,
    /// Datagrams the link refused.
    pub send_failed: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub peers: Vec<PeerStats>,
    /// This device's mic is above the speech gate (after cleaning).
    pub speaking: bool,
    pub muted: bool,
    pub echo: EchoStats,
    pub sent: u64,
    /// Speaker callbacks that ran dry.
    pub underruns: u64,
    pub mic_rate: u32,
    pub speaker_rate: u32,
    pub input_latency_ms: f64,
    pub output_latency_ms: f64,
    /// call-dev probe: mouth-to-ear times measured so far, ms.
    pub mouth_to_ear_ms: Vec<f64>,
}

enum Cmd {
    Add(String, Arc<dyn Link>),
    Remove(String),
    Datagram(String, Vec<u8>, Instant),
    Stop,
}

struct Shared {
    muted: AtomicBool,
    stats: Mutex<Stats>,
}

/// A running call engine. Dropping it stops the thread and closes the
/// devices.
pub struct Engine {
    tx: mpsc::Sender<Cmd>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Engine {
    /// Opens the devices (on the engine's own thread) and starts. Fails if
    /// the devices or the codec can't be opened.
    pub fn start(cfg: Config, open: Opener) -> Result<Engine, String> {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            muted: AtomicBool::new(cfg.muted),
            stats: Mutex::new(Stats::default()),
        });
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let sh = shared.clone();
        let thread = std::thread::Builder::new()
            .name("irori-call".into())
            .spawn(move || {
                let devices = match open() {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let mut core = match Core::new(cfg, devices, sh) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                core.run(rx);
            })
            .map_err(|e| format!("start the call thread: {e}"))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Engine {
                tx,
                shared,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err("the call thread ended while starting".into())
            }
        }
    }

    pub fn add_peer(&self, id: &str, link: Arc<dyn Link>) {
        let _ = self.tx.send(Cmd::Add(id.to_string(), link));
    }

    pub fn remove_peer(&self, id: &str) {
        let _ = self.tx.send(Cmd::Remove(id.to_string()));
    }

    /// A datagram from `peer`. Never blocks.
    pub fn receive(&self, peer: &str, datagram: Vec<u8>) {
        let _ = self
            .tx
            .send(Cmd::Datagram(peer.to_string(), datagram, Instant::now()));
    }

    pub fn set_muted(&self, on: bool) {
        self.shared.muted.store(on, Ordering::Relaxed);
    }

    pub fn stats(&self) -> Stats {
        self.shared.stats.lock().expect("stats").clone()
    }

    /// A cheap handle the network side can keep to hand in datagrams.
    pub fn inlet(&self) -> Inlet {
        Inlet {
            tx: self.tx.clone(),
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Hands datagrams to an engine from any thread or task.
#[derive(Clone)]
pub struct Inlet {
    tx: mpsc::Sender<Cmd>,
}

impl Inlet {
    pub fn receive(&self, peer: &str, datagram: Vec<u8>) {
        let _ = self
            .tx
            .send(Cmd::Datagram(peer.to_string(), datagram, Instant::now()));
    }
}

struct Peer {
    link: Arc<dyn Link>,
    jb: JitterBuffer,
    /// The 20 ms frame being mixed, and how much of it is used.
    frame: [f32; F],
    pos: usize,
    last_loud: Option<Instant>,
    send_failed: u64,
}

struct Core {
    shared: Arc<Shared>,
    dev: Devices,
    t0: Instant,
    dsp: Dsp,
    enc: opus::Encoder,
    mic_in: Resample,
    spk_out: Resample,
    peers: BTreeMap<String, Peer>,
    /// Cleaned mic audio waiting to make a 20 ms frame.
    acc: [f32; F],
    acc_len: usize,
    seq: u16,
    ts: u32,
    dtx_run: u32,
    sent: u64,
    loud_until: Option<Instant>,
    clicker: Option<Clicker>,
    onsets: Onsets,
    /// A click starts in the frame being built (probe).
    probe_in_frame: bool,
    m2e: Vec<f64>,
    mic_buf: Vec<f32>,
    last_stats: Instant,
}

impl Core {
    fn new(cfg: Config, dev: Devices, shared: Arc<Shared>) -> Result<Self, String> {
        let mut enc = opus::Encoder::new(48_000, opus::Channels::Mono, opus::Application::Voip)
            .map_err(|e| format!("opus encoder: {e}"))?;
        let set =
            |r: Result<(), opus::Error>, what: &str| r.map_err(|e| format!("opus {what}: {e}"));
        set(enc.set_bitrate(opus::Bitrate::Bits(cfg.bitrate)), "bitrate")?;
        set(enc.set_inband_fec(true), "fec")?;
        set(enc.set_packet_loss_perc(cfg.loss_percent), "loss")?;
        set(enc.set_dtx(true), "dtx")?;
        set(enc.set_signal(opus::Signal::Voice), "signal")?;
        set(enc.set_complexity(8), "complexity")?;
        let mic_in = Resample::new(dev.mic_rate, 48_000)?;
        let spk_out = Resample::new(48_000, dev.speaker_rate)?;
        Ok(Core {
            dsp: Dsp::new(cfg.dsp),
            enc,
            mic_in,
            spk_out,
            peers: BTreeMap::new(),
            acc: [0.0; F],
            acc_len: 0,
            seq: 0,
            ts: 0,
            dtx_run: 0,
            sent: 0,
            loud_until: None,
            clicker: cfg.probe.then(Clicker::new),
            onsets: Onsets::new(),
            probe_in_frame: false,
            m2e: Vec::new(),
            mic_buf: vec![0.0; 4096],
            last_stats: Instant::now(),
            t0: Instant::now(),
            shared,
            dev,
        })
    }

    fn ms(&self, t: Instant) -> f64 {
        t.saturating_duration_since(self.t0).as_secs_f64() * 1000.0
    }

    fn run(&mut self, rx: mpsc::Receiver<Cmd>) {
        loop {
            match rx.recv_timeout(Duration::from_millis(2)) {
                Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                Ok(c) => {
                    self.command(c);
                    while let Ok(c) = rx.try_recv() {
                        if matches!(c, Cmd::Stop) {
                            return;
                        }
                        self.command(c);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.playout();
            self.capture();
            if self.last_stats.elapsed() >= STATS_EVERY {
                self.publish();
                self.last_stats = Instant::now();
            }
        }
    }

    fn command(&mut self, c: Cmd) {
        match c {
            Cmd::Add(id, link) => {
                let Ok(jb) = JitterBuffer::new() else { return };
                self.peers.insert(
                    id,
                    Peer {
                        link,
                        jb,
                        frame: [0.0; F],
                        pos: F,
                        last_loud: None,
                        send_failed: 0,
                    },
                );
            }
            Cmd::Remove(id) => {
                self.peers.remove(&id);
            }
            Cmd::Datagram(id, d, at) => {
                let now = self.ms(at);
                if let Some(p) = self.peers.get_mut(&id)
                    && let Some((h, payload)) = wire::parse(&d)
                {
                    p.jb.insert(h, payload, now);
                }
            }
            Cmd::Stop => {}
        }
    }

    /// Queued for the speaker, ms (in our ring and the device's).
    fn speaker_queued_ms(&self) -> f64 {
        let ring = self.dev.speaker.occupied_len() + self.spk_out.ready();
        ring as f64 * 1000.0 / self.dev.speaker_rate as f64
    }

    fn playout(&mut self) {
        let want = (self.dev.speaker_rate * SPEAKER_QUEUE_MS / 1000) as usize;
        while self.dev.speaker.occupied_len() < want {
            // Mix 10 ms.
            let now = self.ms(Instant::now()) + self.speaker_queued_ms();
            let mut mix = [0f32; dsp::CHUNK];
            for p in self.peers.values_mut() {
                if p.pos >= F {
                    let played = p.jb.pull(&mut p.frame, now);
                    p.pos = 0;
                    if matches!(played, Played::Normal | Played::Fec) && rms(&p.frame) > SPEECH_RMS
                    {
                        p.last_loud = Some(Instant::now());
                    }
                }
                for (m, s) in mix.iter_mut().zip(&p.frame[p.pos..p.pos + dsp::CHUNK]) {
                    *m += s;
                }
                p.pos += dsp::CHUNK;
            }
            for m in mix.iter_mut() {
                *m = soft_clip(*m);
            }
            // The echo canceller hears the mix; a probe click is played
            // outside it, so the canceller can't remove it from the mic and
            // it reaches the far end like a voice would.
            self.dsp.render(&mix);
            if let Some(c) = self.clicker.as_mut()
                && c.add(&mut mix, Instant::now())
            {
                self.onsets.expect_click(Instant::now());
            }
            self.spk_out.push(&mix);
            let mut out = [0f32; 2048];
            let n = self.spk_out.ready().min(out.len());
            self.spk_out.take(&mut out[..n]);
            let pushed = self.dev.speaker.push_slice(&out[..n]);
            if pushed < n {
                break;
            }
        }
    }

    fn capture(&mut self) {
        let n = self.dev.mic.pop_slice(&mut self.mic_buf);
        if n > 0 {
            let chunk = self.mic_buf[..n].to_vec();
            self.mic_in.push(&chunk);
        }
        // Fell behind: keep only the newest audio.
        let backlog = 48 * MIC_BACKLOG_MS as usize;
        self.mic_in.trim(backlog);
        let mut x = [0f32; dsp::CHUNK];
        while self.mic_in.take(&mut x) {
            // The probe times sound at the raw mic, before any cleaning.
            if let Some(ms) = self.onsets.feed(&x, Instant::now(), self.mic_in.ready()) {
                self.m2e.push(ms);
            }
            if self.onsets.click_started_in_last_chunk() {
                self.probe_in_frame = true;
            }
            let delay = self.dev.latency.output_us.load(Ordering::Relaxed) as f64 / 1000.0
                + self.dev.latency.input_us.load(Ordering::Relaxed) as f64 / 1000.0
                + self.speaker_queued_ms()
                + self.mic_in.ready() as f64 / 48.0;
            self.dsp.capture(&mut x, delay as i32);
            if rms(&x) > SPEECH_RMS {
                self.loud_until = Some(Instant::now() + Duration::from_millis(500));
            }
            self.acc[self.acc_len..self.acc_len + dsp::CHUNK].copy_from_slice(&x);
            self.acc_len += dsp::CHUNK;
            if self.acc_len == F {
                self.acc_len = 0;
                self.send_frame();
            }
        }
    }

    fn send_frame(&mut self) {
        let muted = self.shared.muted.load(Ordering::Relaxed);
        if muted {
            self.acc.fill(0.0);
        }
        let mut buf = [0u8; 1275];
        let n = match self.enc.encode_float(&self.acc, &mut buf) {
            Ok(n) => n,
            Err(_) => return,
        };
        let ts = self.ts;
        self.ts = self.ts.wrapping_add(FRAME);
        // Opus's DTX: a packet of two bytes or less needn't be sent. Send
        // the first (it tells the far end we went quiet) and then one in
        // DTX_KEEPALIVE.
        let dtx = n <= 2;
        if dtx {
            self.dtx_run += 1;
            if self.dtx_run != 1 && !self.dtx_run.is_multiple_of(DTX_KEEPALIVE) {
                return;
            }
        } else {
            self.dtx_run = 0;
        }
        let mut flags = 0;
        if dtx {
            flags |= FLAG_DTX;
        }
        if muted {
            flags |= FLAG_MUTED;
        }
        if std::mem::take(&mut self.probe_in_frame) {
            flags |= FLAG_PROBE;
        }
        let d = wire::encode(
            Header {
                flags,
                seq: self.seq,
                ts,
            },
            &buf[..n],
        );
        self.seq = self.seq.wrapping_add(1);
        for p in self.peers.values_mut() {
            if !p.link.send(&d) {
                p.send_failed += 1;
            }
        }
        self.sent += 1;
    }

    fn publish(&mut self) {
        let now = Instant::now();
        let half = Duration::from_millis(500);
        let peers = self
            .peers
            .iter_mut()
            .map(|(id, p)| {
                let _ = p.jb.take_probes();
                PeerStats {
                    id: id.clone(),
                    jitter: p.jb.stats().clone(),
                    speaking: p.last_loud.is_some_and(|t| now.duration_since(t) < half),
                    send_failed: p.send_failed,
                }
            })
            .collect();
        let muted = self.shared.muted.load(Ordering::Relaxed);
        let st = Stats {
            peers,
            speaking: !muted && self.loud_until.is_some_and(|t| now < t),
            muted,
            echo: self.dsp.echo(),
            sent: self.sent,
            underruns: self.dev.latency.underruns.load(Ordering::Relaxed),
            mic_rate: self.dev.mic_rate,
            speaker_rate: self.dev.speaker_rate,
            input_latency_ms: self.dev.latency.input_us.load(Ordering::Relaxed) as f64 / 1000.0,
            output_latency_ms: self.dev.latency.output_us.load(Ordering::Relaxed) as f64 / 1000.0,
            mouth_to_ear_ms: self.m2e.clone(),
        };
        *self.shared.stats.lock().expect("stats") = st;
    }
}

/// Above this RMS a 10 ms chunk counts as speech (about -36 dBFS).
const SPEECH_RMS: f32 = 0.016;

pub(crate) fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Keeps a loud mix of several speakers inside [-1, 1] without hard
/// clipping.
fn soft_clip(x: f32) -> f32 {
    if x.abs() <= 0.8 {
        x
    } else {
        x.signum() * (0.8 + 0.2 * ((x.abs() - 0.8) / 0.2).tanh())
    }
}

#[cfg(test)]
mod tests {
    /// What send_frame relies on: with DTX on, silence becomes packets of
    /// two bytes or less after Opus's hangover (about ten frames).
    #[test]
    fn opus_dtx_makes_silence_tiny() {
        let mut e =
            opus::Encoder::new(48000, opus::Channels::Mono, opus::Application::Voip).unwrap();
        e.set_bitrate(opus::Bitrate::Bits(28000)).unwrap();
        e.set_inband_fec(true).unwrap();
        e.set_packet_loss_perc(10).unwrap();
        e.set_dtx(true).unwrap();
        e.set_signal(opus::Signal::Voice).unwrap();
        let z = [0f32; 960];
        let mut b = [0u8; 1275];
        let sizes: Vec<usize> = (0..60)
            .map(|_| e.encode_float(&z, &mut b).unwrap())
            .collect();
        let tiny = sizes[12..].iter().filter(|&&n| n <= 2).count();
        assert!(tiny * 10 >= (sizes.len() - 12) * 8, "{sizes:?}");
    }
}
