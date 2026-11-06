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

impl Default for Clicker {
    fn default() -> Self {
        Self::new()
    }
}

/// 1 ms blocks.
const BLOCK: usize = 48;
/// An onset needs this much quiet before it, in blocks (ms).
const QUIET_BEFORE: u32 = 40;
/// The far end's copy must come this long after T0 at the least, and
/// within the most, ms.
const MIN_GAP_MS: f64 = 25.0;
const MAX_GAP_MS: f64 = 1500.0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Wait {
    Idle,
    /// A click was played: the next onset is T0.
    Mouth {
        since: Instant,
    },
    /// T0 at this mic time (ms): the next onset is T1.
    Ear {
        t0: f64,
    },
}

/// Finds sound onsets in the raw mic stream and pairs them into
/// mouth-to-ear times.
pub struct Onsets {
    /// Mic samples seen (48 kHz).
    samples: u64,
    floor: f32,
    quiet_run: u32,
    wait: Wait,
    started_last: bool,
}

impl Onsets {
    pub fn new() -> Self {
        Onsets {
            samples: 0,
            floor: 0.001,
            quiet_run: QUIET_BEFORE,
            wait: Wait::Idle,
            started_last: false,
        }
    }

    /// A click was just queued for the speaker.
    pub fn expect_click(&mut self, now: Instant) {
        self.wait = Wait::Mouth { since: now };
    }

    /// T0 was found in the chunk last fed.
    pub fn click_started_in_last_chunk(&self) -> bool {
        self.started_last
    }

    /// Feeds 10 ms of raw mic; returns a mouth-to-ear time when one
    /// completes.
    pub fn feed(&mut self, x: &[f32; CHUNK], now: Instant, _backlog: usize) -> Option<f64> {
        self.started_last = false;
        let mut done = None;
        for b in x.chunks(BLOCK) {
            let r = crate::engine::rms(b);
            let t_ms = self.samples as f64 / 48.0;
            self.samples += b.len() as u64;
            let threshold = (self.floor * 8.0).max(0.01);
            if r > threshold {
                if self.quiet_run >= QUIET_BEFORE {
                    done = done.or(self.onset(t_ms, now));
                }
                self.quiet_run = 0;
            } else {
                self.quiet_run = self.quiet_run.saturating_add(1);
                // Follow the room's noise, slowly.
                self.floor = 0.995 * self.floor + 0.005 * r;
            }
        }
        if let Wait::Mouth { since } = self.wait
            && now.duration_since(since) > Duration::from_secs(1)
        {
            self.wait = Wait::Idle;
        }
        done
    }

    fn onset(&mut self, t_ms: f64, _now: Instant) -> Option<f64> {
        match self.wait {
            Wait::Idle => None,
            Wait::Mouth { .. } => {
                self.wait = Wait::Ear { t0: t_ms };
                self.started_last = true;
                None
            }
            Wait::Ear { t0 } => {
                let gap = t_ms - t0;
                if gap < MIN_GAP_MS {
                    None
                } else {
                    self.wait = Wait::Idle;
                    (gap <= MAX_GAP_MS).then_some(gap)
                }
            }
        }
    }
}

impl Default for Onsets {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A click at 100 ms and its copy at 260 ms measure 160 ms.
    #[test]
    fn pairs_the_click_with_its_far_copy() {
        let mut c = Clicker::new();
        let mut click = Vec::new();
        let start = Instant::now();
        c.next = start;
        for _ in 0..2 {
            let mut x = [0.0; CHUNK];
            c.add(&mut x, start);
            click.extend_from_slice(&x);
        }
        let mut mic = vec![0.0f32; 48 * 600];
        let noise = |i: usize| ((i * 7919 % 101) as f32 / 101.0 - 0.5) * 0.002;
        for (i, m) in mic.iter_mut().enumerate() {
            *m = noise(i);
        }
        for (i, s) in click.iter().enumerate() {
            mic[48 * 100 + i] += s;
            mic[48 * 260 + i] += 0.4 * s;
        }
        let mut o = Onsets::new();
        o.expect_click(start);
        let mut got = None;
        for ch in mic.chunks(CHUNK) {
            let mut x = [0.0; CHUNK];
            x.copy_from_slice(ch);
            got = got.or(o.feed(&x, start, 0));
        }
        let ms = got.expect("measured");
        assert!((ms - 160.0).abs() <= 2.0, "{ms}");
    }

    #[test]
    fn nothing_without_a_click() {
        let mut o = Onsets::new();
        let mut x = [0.0; CHUNK];
        x[100..200].fill(0.3);
        assert!(o.feed(&x, Instant::now(), 0).is_none());
    }
}
