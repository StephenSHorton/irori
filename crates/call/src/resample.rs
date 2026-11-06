//! A mono sample-rate converter for a stream: push at one rate, take at
//! another. Most devices already run at 48 kHz, and then it only copies.

use std::collections::VecDeque;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

/// Input frames per resampler pass.
const CHUNK: usize = 480;

pub struct Resample {
    rs: Option<Fft<f32>>,
    inbuf: Vec<f32>,
    scratch: Vec<f32>,
    out: VecDeque<f32>,
}

impl Resample {
    pub fn new(from: u32, to: u32) -> Result<Self, String> {
        let rs = if from == to {
            None
        } else {
            Some(
                Fft::<f32>::new(from as usize, to as usize, CHUNK, 1, FixedSync::Input)
                    .map_err(|e| format!("resampler {from} -> {to} Hz: {e}"))?,
            )
        };
        let scratch = vec![0.0; rs.as_ref().map_or(0, |r| r.output_frames_max())];
        Ok(Resample {
            rs,
            inbuf: Vec::with_capacity(CHUNK * 2),
            scratch,
            out: VecDeque::new(),
        })
    }

    pub fn push(&mut self, x: &[f32]) {
        let Some(rs) = self.rs.as_mut() else {
            self.out.extend(x);
            return;
        };
        self.inbuf.extend_from_slice(x);
        while self.inbuf.len() >= CHUNK {
            let (Ok(inp), Ok(mut outp)) =
                (InterleavedSlice::new(&self.inbuf[..CHUNK], 1, CHUNK), {
                    let n = self.scratch.len();
                    InterleavedSlice::new_mut(&mut self.scratch[..], 1, n)
                })
            else {
                return;
            };
            if let Ok((_, n)) = rs.process_into_buffer(&inp, &mut outp, None) {
                self.out.extend(&self.scratch[..n]);
            }
            self.inbuf.drain(..CHUNK);
        }
    }

    /// Samples ready to take.
    pub fn ready(&self) -> usize {
        self.out.len()
    }

    /// Fills `out` if that many are ready.
    pub fn take(&mut self, out: &mut [f32]) -> bool {
        if self.out.len() < out.len() {
            return false;
        }
        let n = out.len();
        for (o, s) in out.iter_mut().zip(self.out.drain(..n)) {
            *o = s;
        }
        true
    }

    /// Drops what is buffered beyond `keep` samples (the oldest first), so a
    /// backlog can't add delay.
    pub fn trim(&mut self, keep: usize) -> usize {
        let extra = self.out.len().saturating_sub(keep);
        self.out.drain(..extra);
        extra
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_rate_copies() {
        let mut r = Resample::new(48_000, 48_000).unwrap();
        r.push(&[1.0, 2.0, 3.0]);
        let mut o = [0.0; 2];
        assert!(r.take(&mut o));
        assert_eq!(o, [1.0, 2.0]);
        assert_eq!(r.ready(), 1);
    }

    #[test]
    fn converts_44k1_to_48k_at_the_right_rate() {
        let mut r = Resample::new(44_100, 48_000).unwrap();
        let tone: Vec<f32> = (0..44_100).map(|i| (i as f32 * 0.05).sin()).collect();
        r.push(&tone);
        // One second in, about one second out (less the resampler's delay).
        let got = r.ready();
        assert!((46_000..=48_000).contains(&got), "{got}");
    }
}
