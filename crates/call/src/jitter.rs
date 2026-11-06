//! One peer's jitter buffer and decoder.
//!
//! Frames are kept by their timestamp and played on our own clock, a fixed
//! delay behind the sender's: `play time = sender time + base + target`.
//! `base` is the smallest one-way delay seen lately (it absorbs the clock
//! offset between the devices), `target` the 95th percentile of the jitter
//! above it plus a little, clamped to [`MIN_TARGET_MS`, `MAX_TARGET_MS`]. A
//! late burst past the cap is lost and concealed rather than letting the
//! delay grow (audio-stack.md §5: cap the target, let PLC and FEC cover the
//! tail).
//!
//! For each 20 ms played:
//! - the frame is here: decode it;
//! - it isn't but the next one is: decode the next one's in-band FEC copy;
//! - the sender said it went quiet (DTX): Opus comfort noise;
//! - otherwise: Opus loss concealment.
//!
//! When the buffer drifts from its target (a jitter change, or the two
//! devices' clocks running at slightly different rates) a frame is dropped
//! or a concealed one inserted, in silence when possible.

use std::collections::{BTreeMap, VecDeque};

use crate::wire::{self, FLAG_DTX, FLAG_MUTED, FLAG_PROBE, FRAME, Header};

/// The least delay added for jitter, ms.
pub const MIN_TARGET_MS: f64 = 30.0;
/// The most, ms: past it a late frame is concealed instead.
pub const MAX_TARGET_MS: f64 = 140.0;
/// How long arrival delays are remembered for the estimate, ms.
const WINDOW_MS: f64 = 4000.0;
/// Room above the 95th percentile, ms.
const MARGIN_MS: f64 = 10.0;
/// No packet for this long while not in DTX: the peer is gone or stuck.
/// Stop playing and buffer afresh when it comes back.
const STALL_MS: f64 = 600.0;
/// Frames more than this far from the play point start over (a restarted
/// sender, or a bogus timestamp).
const RESET_FRAMES: i64 = 500;
const SAMPLES_PER_MS: f64 = 48.0;

/// What one pull played.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Played {
    /// Not started, or stalled: silence.
    Idle,
    Normal,
    /// Recovered from the next frame's FEC.
    Fec,
    /// Lost and concealed.
    Concealed,
    /// The sender is quiet: comfort noise.
    Quiet,
}

#[derive(Debug, Clone, Default)]
pub struct JitterStats {
    /// Datagrams received.
    pub received: u64,
    /// Datagrams that should have arrived by now but never did.
    pub lost: u64,
    /// Arrived after their time to play had passed.
    pub late: u64,
    pub fec: u64,
    pub concealed: u64,
    /// Frames dropped or inserted to follow the target.
    pub dropped: u64,
    pub inserted: u64,
    /// The delay this buffer aims for, ms.
    pub target_ms: f64,
    /// Audio buffered ahead of the play point, ms.
    pub depth_ms: f64,
    /// Jitter: the 95th percentile of the arrival delay above the base, ms.
    pub jitter_ms: f64,
    /// The sender's last header said its mic is muted.
    pub muted: bool,
}

struct Packet {
    flags: u8,
    data: Vec<u8>,
}

pub struct JitterBuffer {
    dec: opus::Decoder,
    /// Frames waiting, by unwrapped timestamp.
    frames: BTreeMap<i64, Packet>,
    /// The last raw timestamp seen and its unwrapped value.
    last_ts: Option<(u32, i64)>,
    /// (arrival ms, arrival ms - sender ms) of recent packets.
    delays: VecDeque<(f64, f64)>,
    base: f64,
    target: f64,
    jitter: f64,
    /// The unwrapped timestamp to play next, once started.
    next: Option<i64>,
    /// The sender went quiet (the last frame played or the newest one seen
    /// had DTX set).
    quiet: bool,
    last_arrival: f64,
    /// Highest sequence number seen (unwrapped) and the first.
    seq_hi: Option<i64>,
    seq_lo: i64,
    last_seq: u16,
    /// Probe frames played, waiting to be collected (call-dev).
    probes: Vec<i64>,
    stats: JitterStats,
}

impl JitterBuffer {
    pub fn new() -> Result<Self, opus::Error> {
        Ok(JitterBuffer {
            dec: opus::Decoder::new(48_000, opus::Channels::Mono)?,
            frames: BTreeMap::new(),
            last_ts: None,
            delays: VecDeque::new(),
            base: 0.0,
            target: MIN_TARGET_MS,
            jitter: 0.0,
            next: None,
            quiet: false,
            last_arrival: 0.0,
            seq_hi: None,
            seq_lo: 0,
            last_seq: 0,
            probes: Vec::new(),
            stats: JitterStats {
                target_ms: MIN_TARGET_MS,
                ..Default::default()
            },
        })
    }

    /// A datagram's header and packet, arriving at `now_ms` on our clock.
    pub fn insert(&mut self, h: Header, data: &[u8], now_ms: f64) {
        self.stats.received += 1;
        self.stats.muted = h.flags & FLAG_MUTED != 0;
        self.last_arrival = now_ms;
        self.count_seq(h.seq);

        let ts = self.unwrap_ts(h.ts);
        if let Some(n) = self.next {
            if (ts - n).abs() > RESET_FRAMES * FRAME as i64 {
                self.restart();
                return self.insert_fresh(h, data, ts, now_ms);
            }
            if ts < n {
                self.stats.late += 1;
                // Still a delay sample: it is how late frames teach the
                // estimate to wait longer.
                self.note_delay(ts, now_ms);
                return;
            }
        }
        self.insert_fresh(h, data, ts, now_ms);
    }

    fn insert_fresh(&mut self, h: Header, data: &[u8], ts: i64, now_ms: f64) {
        self.note_delay(ts, now_ms);
        if h.flags & FLAG_DTX != 0 {
            self.quiet = true;
        }
        self.frames.entry(ts).or_insert(Packet {
            flags: h.flags,
            data: data.to_vec(),
        });
        // A runaway sender can't grow memory: keep about 2 s.
        while self.frames.len() > 100 {
            self.frames.pop_first();
        }
    }

    fn unwrap_ts(&mut self, ts: u32) -> i64 {
        let u = match self.last_ts {
            None => ts as i64,
            Some((raw, un)) => un + wire::ts_diff(ts, raw),
        };
        // Only move the reference forward, so a reordered old packet
        // doesn't drag it back.
        if self.last_ts.is_none_or(|(_, un)| u > un) {
            self.last_ts = Some((ts, u));
        }
        u
    }

    fn count_seq(&mut self, seq: u16) {
        match self.seq_hi {
            None => {
                self.seq_hi = Some(seq as i64);
                self.seq_lo = seq as i64;
                self.last_seq = seq;
            }
            Some(hi) => {
                let d = wire::seq_diff(seq, self.last_seq) as i64;
                if d > 0 {
                    self.seq_hi = Some(hi + d);
                    self.last_seq = seq;
                }
            }
        }
        if let Some(hi) = self.seq_hi {
            let expected = (hi - self.seq_lo + 1) as u64;
            self.stats.lost = expected.saturating_sub(self.stats.received);
        }
    }

    fn note_delay(&mut self, ts: i64, now_ms: f64) {
        let d = now_ms - ts as f64 / SAMPLES_PER_MS;
        self.delays.push_back((now_ms, d));
        while self
            .delays
            .front()
            .is_some_and(|&(t, _)| now_ms - t > WINDOW_MS)
        {
            self.delays.pop_front();
        }
        self.base = self
            .delays
            .iter()
            .map(|&(_, d)| d)
            .fold(f64::INFINITY, f64::min);
        let mut j: Vec<f64> = self.delays.iter().map(|&(_, d)| d - self.base).collect();
        j.sort_by(|a, b| a.total_cmp(b));
        let p95 = j[((j.len() - 1) as f64 * 0.95).round() as usize];
        self.jitter = p95;
        self.target = (p95 + MARGIN_MS).clamp(MIN_TARGET_MS, MAX_TARGET_MS);
    }

    fn restart(&mut self) {
        self.frames.clear();
        self.delays.clear();
        self.next = None;
        self.last_ts = None;
        self.quiet = false;
        self.dec.reset_state().ok();
    }

    /// The timestamp that should be playing at `now_ms`.
    fn ideal(&self, now_ms: f64) -> i64 {
        ((now_ms - self.base - self.target) * SAMPLES_PER_MS) as i64
    }

    /// The next 20 ms (960 samples) to play at `now_ms`.
    pub fn pull(&mut self, out: &mut [f32], now_ms: f64) -> Played {
        debug_assert_eq!(out.len(), FRAME as usize);
        let played = self.pull_inner(out, now_ms);
        self.stats.target_ms = self.target;
        self.stats.jitter_ms = self.jitter;
        self.stats.depth_ms = match (self.next, self.frames.last_key_value()) {
            (Some(n), Some((&last, _))) => {
                ((last + FRAME as i64 - n).max(0)) as f64 / SAMPLES_PER_MS
            }
            _ => 0.0,
        };
        played
    }

    fn pull_inner(&mut self, out: &mut [f32], now_ms: f64) -> Played {
        let ideal = self.ideal(now_ms);
        let Some(mut next) = self.next else {
            // Start once the oldest frame's time has come.
            match self.frames.first_key_value() {
                Some((&first, _)) if first <= ideal => {
                    self.next = Some(first);
                    return self.pull_inner(out, now_ms);
                }
                _ => {
                    out.fill(0.0);
                    return Played::Idle;
                }
            }
        };
        let f = FRAME as i64;

        // Gone quiet without DTX for a long time: stop and buffer afresh.
        if !self.quiet && now_ms - self.last_arrival > STALL_MS && self.frames.is_empty() {
            self.next = None;
            self.dec.reset_state().ok();
            out.fill(0.0);
            return Played::Idle;
        }

        let have = |m: &BTreeMap<i64, Packet>, t: i64| m.contains_key(&t);
        let silent_here = self.quiet || !have(&self.frames, next);

        // Too deep: behind the ideal play point by more than two frames.
        // Skip ahead in silence, or past six frames even in speech.
        let behind = ideal - next;
        if behind > 2 * f && (silent_here || behind > 6 * f) {
            self.frames.remove(&next);
            next += f;
            self.stats.dropped += 1;
            // A frame skipped in DTX needs no decode; in speech the decoder
            // simply continues from the next one.
        }
        // Ahead of the ideal point (too early, or the target just grew):
        // insert a concealed frame when the frame isn't here yet (it may
        // still come in time), more than a frame ahead in silence, or past
        // three frames ahead regardless.
        let ahead = next - ideal;
        let missing = !have(&self.frames, next);
        if ahead > 3 * f || (ahead > f && silent_here) || (ahead > 0 && missing && !self.quiet) {
            self.stats.inserted += 1;
            self.next = Some(next);
            return self.conceal(out);
        }

        // Drop what is now behind the play point.
        while let Some((&t, _)) = self.frames.first_key_value() {
            if t >= next {
                break;
            }
            self.frames.remove(&t);
        }

        let played = if let Some(p) = self.frames.remove(&next) {
            self.quiet = p.flags & FLAG_DTX != 0;
            if p.flags & FLAG_PROBE != 0 {
                self.probes.push(next);
            }
            self.decode(&p.data, out, false);
            if self.quiet {
                Played::Quiet
            } else {
                Played::Normal
            }
        } else if !self.quiet
            && let Some(p) = self.frames.get(&(next + f))
            && p.flags & FLAG_DTX == 0
        {
            let data = p.data.clone();
            self.stats.fec += 1;
            self.decode(&data, out, true);
            Played::Fec
        } else {
            self.conceal(out)
        };
        self.next = Some(next + f);
        played
    }

    fn conceal(&mut self, out: &mut [f32]) -> Played {
        self.decode(&[], out, false);
        if self.quiet {
            Played::Quiet
        } else {
            self.stats.concealed += 1;
            Played::Concealed
        }
    }

    fn decode(&mut self, data: &[u8], out: &mut [f32], fec: bool) {
        match self.dec.decode_float(data, out, fec) {
            Ok(n) if n < out.len() => out[n..].fill(0.0),
            Ok(_) => {}
            Err(_) => out.fill(0.0),
        }
    }

    pub fn stats(&self) -> &JitterStats {
        &self.stats
    }

    /// Timestamps (unwrapped, 48 kHz) of probe frames played since the last
    /// call.
    pub fn take_probes(&mut self) -> Vec<i64> {
        std::mem::take(&mut self.probes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 440 Hz tone, Opus-encoded the way the engine does, 20 ms a frame.
    struct Sender {
        enc: opus::Encoder,
        seq: u16,
        ts: u32,
        phase: f32,
    }

    impl Sender {
        fn new() -> Self {
            let mut enc =
                opus::Encoder::new(48_000, opus::Channels::Mono, opus::Application::Voip).unwrap();
            enc.set_inband_fec(true).unwrap();
            enc.set_packet_loss_perc(10).unwrap();
            Sender {
                enc,
                seq: 7,
                ts: u32::MAX - 5 * FRAME,
                phase: 0.0,
            }
        }
        fn next(&mut self) -> (Header, Vec<u8>) {
            let mut pcm = [0f32; FRAME as usize];
            for s in pcm.iter_mut() {
                *s = 0.3 * self.phase.sin();
                self.phase += 2.0 * std::f32::consts::PI * 440.0 / 48_000.0;
            }
            let mut buf = [0u8; 1500];
            let n = self.enc.encode_float(&pcm, &mut buf).unwrap();
            let h = Header {
                flags: 0,
                seq: self.seq,
                ts: self.ts,
            };
            self.seq = self.seq.wrapping_add(1);
            self.ts = self.ts.wrapping_add(FRAME);
            (h, buf[..n].to_vec())
        }
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    /// Feeds `frames` 20 ms frames, with `arrive(i)` giving each one's
    /// arrival delay in ms (None = lost), and pulls every 20 ms. Returns the
    /// buffer and what each pull played.
    fn run(frames: usize, arrive: impl Fn(usize) -> Option<f64>) -> (JitterBuffer, Vec<Played>) {
        let mut s = Sender::new();
        let mut jb = JitterBuffer::new().unwrap();
        let mut pending: Vec<(f64, Header, Vec<u8>)> = Vec::new();
        let mut played = Vec::new();
        let t0 = 1000.0;
        let mut out = [0f32; FRAME as usize];
        for i in 0..frames + 20 {
            let now = t0 + i as f64 * 20.0;
            if i < frames {
                let (h, p) = s.next();
                if let Some(d) = arrive(i) {
                    pending.push((now + d, h, p));
                }
            }
            // Deliver in arrival order, in 1 ms steps across this 20 ms.
            for ms in 0..20 {
                let t = now + ms as f64;
                pending.sort_by(|a, b| a.0.total_cmp(&b.0));
                while pending.first().is_some_and(|x| x.0 <= t) {
                    let (at, h, p) = pending.remove(0);
                    jb.insert(h, &p, at);
                }
            }
            played.push(jb.pull(&mut out, now + 20.0));
        }
        (jb, played)
    }

    #[test]
    fn steady_stream_plays_every_frame_after_the_start() {
        let (jb, played) = run(200, |_| Some(15.0));
        let first = played.iter().position(|p| *p != Played::Idle).unwrap();
        // Starts within the minimum target plus a frame.
        assert!(first <= 3, "started at {first}");
        assert!(
            played[first..200].iter().all(|p| *p == Played::Normal),
            "{:?}",
            &played[first..200]
        );
        let st = jb.stats();
        assert_eq!(st.lost, 0);
        assert_eq!(st.late, 0);
        assert_eq!(st.target_ms, MIN_TARGET_MS);
    }

    fn count(played: &[Played], k: Played) -> usize {
        played.iter().filter(|p| **p == k).count()
    }

    #[test]
    fn a_lost_frame_is_recovered_from_fec() {
        // Only the pulls while the sender sends; after it stops the tail is
        // concealed until the stall.
        let (jb, played) = run(100, |i| if i == 50 { None } else { Some(10.0) });
        let during = &played[..100];
        assert_eq!(count(during, Played::Fec), 1, "{during:?}");
        assert_eq!(count(during, Played::Concealed), 0, "{during:?}");
        assert_eq!(jb.stats().lost, 1);
    }

    #[test]
    fn two_lost_in_a_row_conceal_one_and_recover_one() {
        let (_, played) = run(100, |i| if i == 50 || i == 51 { None } else { Some(10.0) });
        let during = &played[..100];
        assert_eq!(count(during, Played::Concealed), 1, "{during:?}");
        assert_eq!(count(during, Played::Fec), 1, "{during:?}");
    }

    #[test]
    fn jitter_raises_the_target_and_loses_nothing() {
        // Every fourth frame 60 ms late: the target grows past 60 ms.
        let (jb, played) = run(300, |i| Some(if i % 4 == 0 { 70.0 } else { 10.0 }));
        let st = jb.stats();
        assert!(st.target_ms >= 60.0, "{st:?}");
        let tail = &played[150..300];
        assert!(tail.iter().all(|p| *p == Played::Normal), "{tail:?}");
    }

    #[test]
    fn the_target_is_capped_and_later_frames_are_concealed() {
        // A 400 ms spike on a few frames: past the cap, so they're late,
        // not waited for.
        let (jb, _) = run(300, |i| {
            Some(if (100..104).contains(&i) { 400.0 } else { 10.0 })
        });
        let st = jb.stats();
        assert!(st.target_ms <= MAX_TARGET_MS);
        assert!(st.late >= 1, "{st:?}");
    }

    #[test]
    fn reordered_frames_still_play_in_order() {
        // Odd frames overtake even ones by 15 ms.
        let (jb, played) = run(200, |i| Some(if i % 2 == 0 { 25.0 } else { 10.0 }));
        let first = played.iter().position(|p| *p != Played::Idle).unwrap();
        assert!(played[first + 5..200].iter().all(|p| *p == Played::Normal));
        assert_eq!(jb.stats().late, 0);
    }

    #[test]
    fn decoded_audio_has_the_tone() {
        let mut s = Sender::new();
        let mut jb = JitterBuffer::new().unwrap();
        let mut out = [0f32; FRAME as usize];
        let mut loud = 0;
        for i in 0..50 {
            let now = i as f64 * 20.0;
            let (h, p) = s.next();
            jb.insert(h, &p, now + 5.0);
            if jb.pull(&mut out, now + 20.0) == Played::Normal && rms(&out) > 0.1 {
                loud += 1;
            }
        }
        assert!(loud > 40, "{loud}");
    }

    #[test]
    fn a_stalled_sender_stops_playback_and_restarts_cleanly() {
        let (_, played) = run(200, |i| {
            if (80..140).contains(&i) {
                None
            } else {
                Some(10.0)
            }
        });
        assert!(played[110..130].contains(&Played::Idle));
        assert!(
            played[150..200].iter().all(|p| *p == Played::Normal),
            "{:?}",
            &played[150..200]
        );
    }
}
