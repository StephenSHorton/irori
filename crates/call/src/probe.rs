//! Measuring mouth-to-ear with sound (call-dev's probe).
//!
//! Put two devices side by side in a call, the far one muted. This device
//! plays a short click on its own speaker, outside the echo canceller, so
//! the click goes out to the far device like a voice. Its mic hears the
//! click twice: once straight from its own speaker (T0, the "mouth"), and
//! once from the far device's speaker after the trip through the call (T1,
//! the "ear"). Both are times in this mic's own sample stream, so device
//! clocks and buffers on this side cancel out: T1 - T0 is mouth-to-ear,
//! mic capture, encode, network, jitter buffer, decode and the far
//! speaker included.

use std::time::{Duration, Instant};

use crate::dsp::CHUNK;

/// Time between clicks.
const EVERY: Duration = Duration::from_millis(2500);
/// Click: 20 ms of 1 kHz, faded in and out over 2 ms.
const CLICK: usize = 48 * 20;
const FADE: usize = 48 * 2;
const LEVEL: f32 = 0.5;

pub struct Clicker {
    next: Instant,
    /// Samples of the current click still to play.
    left: usize,
}

impl Clicker {
    pub fn new() -> Self {
        Clicker {
            next: Instant::now() + Duration::from_secs(2),
            left: 0,
        }
    }

    /// Adds the click to a 10 ms chunk about to be played; true when a
    /// click starts in it.
    pub fn add(&mut self, x: &mut [f32; CHUNK], now: Instant) -> bool {
        let mut started = false;
        if self.left == 0 && now >= self.next {
            self.left = CLICK;
            self.next = now + EVERY;
            started = true;
        }
        for s in x.iter_mut() {
            if self.left == 0 {
                break;
            }
            let i = CLICK - self.left;
            let env = (i.min(CLICK - 1 - i) as f32 / FADE as f32).min(1.0);
            *s += LEVEL * env * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48_000.0).sin();
            self.left -= 1;
        }
        started
    }
}

/// A stand-in voice for the echo test (call-dev --say): noise shaped like
/// speech, syllables at about 4 Hz, talking for 1.5 s then quiet for 1.5 s.
/// Added to what this device sends, after its own cleaning.
pub struct Voice {
    n: u64,
    seed: u32,
    lp: f32,
}

impl Voice {
    pub fn new() -> Self {
        Voice {
            n: 0,
            seed: 654,
            lp: 0.0,
        }
    }

    pub fn add(&mut self, x: &mut [f32; CHUNK]) {
        for s in x.iter_mut() {
            let t = self.n as f32 / 48_000.0;
            self.n += 1;
            self.seed = self
                .seed
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            let w = (self.seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
            self.lp = 0.6 * self.lp + 0.4 * w;
            let talking = t % 3.0 < 1.5;
            let syllable = (0.5 - 0.5 * (2.0 * std::f32::consts::PI * 4.0 * t).cos()).powi(2);
            if talking {
                *s += 0.8 * self.lp * syllable;
            }
        }
    }
}

impl Default for Voice {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for Clicker {
    fn default() -> Self {
        Self::new()
    }
}

/// How much mic audio to look at after a click, ms.
const LISTEN_MS: usize = 900;
/// T0 must come within this of the click being queued, ms (this device's
/// own speaker-to-mic path).
const MOUTH_WITHIN_MS: usize = 350;
/// The far copy starts this long after T0 at the least: past the click
/// itself and the room's echo of it, ms.
const MIN_GAP_MS: usize = 60;
/// The tone level a click must reach at the mic (1 kHz magnitude).
const MIN_LEVEL: f32 = 0.002;

/// Finds the click in the raw mic stream twice and turns the gap into a
/// mouth-to-ear time. It listens for the click's own 1 kHz tone (a
/// Goertzel filter over 5 ms, every 1 ms), so broadband room noise and
/// speech hardly register.
pub struct Onsets {
    /// Raw mic since the click was queued, while listening.
    heard: Option<Vec<f32>>,
    started_last: bool,
    /// IRORI_CALL_PROBE_DUMP=<file>: the raw mic as f32 LE at 48 kHz, and
    /// each click's tone envelope on stderr, to see what the probe heard.
    dump: Option<std::fs::File>,
}

impl Onsets {
    pub fn new() -> Self {
        Onsets {
            heard: None,
            started_last: false,
            dump: std::env::var_os("IRORI_CALL_PROBE_DUMP")
                .and_then(|p| std::fs::File::create(p).ok()),
        }
    }

    /// A click was just queued for the speaker.
    pub fn expect_click(&mut self, _now: Instant) {
        self.heard = Some(Vec::with_capacity(48 * LISTEN_MS));
        self.started_last = true;
    }

    /// A click was queued since the last chunk fed (the engine marks that
    /// frame).
    pub fn click_started_in_last_chunk(&self) -> bool {
        self.started_last
    }

    /// Feeds 10 ms of raw mic; returns a mouth-to-ear time when one is
    /// measured.
    pub fn feed(&mut self, x: &[f32; CHUNK], _now: Instant, _backlog: usize) -> Option<f64> {
        if let Some(f) = self.dump.as_mut() {
            use std::io::Write;
            let bytes: Vec<u8> = x.iter().flat_map(|v| v.to_le_bytes()).collect();
            let _ = f.write_all(&bytes);
        }
        let heard = self.heard.as_mut()?;
        self.started_last = false;
        heard.extend_from_slice(x);
        if heard.len() < 48 * LISTEN_MS {
            return None;
        }
        let heard = self.heard.take()?;
        let env = envelope(&heard);
        let got = measure(&env);
        if self.dump.is_some() {
            let row: Vec<String> = env
                .iter()
                .step_by(10)
                .map(|v| format!("{}", (v * 1000.0) as i32))
                .collect();
            eprintln!("probe envelope (10 ms steps, x1000): {}", row.join(" "));
            eprintln!("probe measured {got:?}");
        }
        got
    }
}

impl Default for Onsets {
    fn default() -> Self {
        Self::new()
    }
}

/// The 1 kHz magnitude over 5 ms windows, one value per ms.
fn envelope(x: &[f32]) -> Vec<f32> {
    const N: usize = 240;
    let w = 2.0 * std::f32::consts::PI * 1000.0 / 48_000.0;
    let c = 2.0 * w.cos();
    (0..x.len().saturating_sub(N) / 48)
        .map(|ms| {
            let (mut s1, mut s2) = (0f32, 0f32);
            for &v in &x[ms * 48..ms * 48 + N] {
                let s0 = v + c * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            (s1 * s1 + s2 * s2 - c * s1 * s2).max(0.0).sqrt() / (N as f32 / 2.0)
        })
        .collect()
}

/// The earliest index in `env[from..to]` reaching half the peak there, and
/// the peak.
fn rise(env: &[f32], from: usize, to: usize) -> Option<(usize, f32)> {
    let to = to.min(env.len());
    if from >= to {
        return None;
    }
    let peak = env[from..to].iter().cloned().fold(0.0, f32::max);
    let at = (from..to).find(|&i| env[i] >= peak / 2.0)?;
    Some((at, peak))
}

/// T1 - T0 from a click's envelope (index = ms since the click was
/// queued), or None if either copy can't be found. The earliest copy at
/// half the loudest wins, so a weaker second loop later doesn't count.
fn measure(env: &[f32]) -> Option<f64> {
    let (t0, p0) = rise(env, 0, MOUTH_WITHIN_MS)?;
    if p0 < MIN_LEVEL {
        return None;
    }
    let mut sorted = env.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let median = sorted[sorted.len() / 2];
    let (t1, p1) = rise(env, t0 + MIN_GAP_MS, env.len())?;
    if p1 < MIN_LEVEL.max(4.0 * median) {
        return None;
    }
    Some((t1 - t0) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click() -> Vec<f32> {
        let mut c = Clicker::new();
        let start = Instant::now();
        c.next = start;
        let mut out = Vec::new();
        for _ in 0..2 {
            let mut x = [0.0; CHUNK];
            c.add(&mut x, start);
            out.extend_from_slice(&x);
        }
        out
    }

    fn noise(i: usize) -> f32 {
        ((i * 7919 % 101) as f32 / 101.0 - 0.5) * 0.01
    }

    fn mic_with(copies: &[(usize, f32)]) -> Vec<f32> {
        let c = click();
        let mut mic: Vec<f32> = (0..48 * 1000).map(noise).collect();
        for &(at_ms, gain) in copies {
            for (i, s) in c.iter().enumerate() {
                mic[48 * at_ms + i] += gain * s;
            }
        }
        mic
    }

    /// The click heard at 100 ms and its quieter far copy at 290 ms
    /// measure 190 ms, through room noise and a weaker second loop.
    #[test]
    fn pairs_the_click_with_its_far_copy() {
        let mic = mic_with(&[(100, 0.07), (290, 0.03), (480, 0.012)]);
        let mut o = Onsets::new();
        o.expect_click(Instant::now());
        let mut got = None;
        for ch in mic.chunks(CHUNK) {
            let mut x = [0.0; CHUNK];
            x.copy_from_slice(ch);
            got = got.or(o.feed(&x, Instant::now(), 0));
        }
        let ms = got.expect("measured");
        assert!((ms - 190.0).abs() <= 3.0, "{ms}");
    }

    #[test]
    fn no_far_copy_measures_nothing() {
        let mic = mic_with(&[(100, 0.07)]);
        assert!(measure(&envelope(&mic)).is_none());
    }

    #[test]
    fn nothing_without_a_click() {
        let mut o = Onsets::new();
        let mut x = [0.0; CHUNK];
        x[100..200].fill(0.3);
        assert!(o.feed(&x, Instant::now(), 0).is_none());
    }
}
