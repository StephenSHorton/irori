//! Echo cancelling, noise suppression and gain: sonora, WebRTC's audio
//! processing (AEC3, NS, AGC2) in Rust, at 48 kHz mono in 10 ms chunks.
//!
//! The echo canceller must hear exactly what goes to the speaker, so the
//! mixed playout passes through [`Dsp::render`] before it is queued for the
//! device, and every mic chunk through [`Dsp::capture`].

use sonora::config::{
    EchoCanceller, GainController2, HighPassFilter, MaxProcessingRate, NoiseSuppression,
    NoiseSuppressionLevel, Pipeline,
};
use sonora::{AudioProcessing, Config, StreamConfig};

/// Samples in one 10 ms chunk at 48 kHz.
pub const CHUNK: usize = 480;

#[derive(Debug, Clone, Copy)]
pub struct DspConfig {
    pub echo_cancel: bool,
    pub noise_suppress: bool,
    pub auto_gain: bool,
}

impl Default for DspConfig {
    fn default() -> Self {
        DspConfig {
            echo_cancel: true,
            noise_suppress: true,
            auto_gain: true,
        }
    }
}

pub struct Dsp {
    apm: AudioProcessing,
    scratch: [f32; CHUNK],
}

/// How well the echo canceller is doing, from its own statistics.
#[derive(Debug, Clone, Copy, Default)]
pub struct EchoStats {
    /// Echo return loss enhancement, dB: how much echo it removes.
    pub erle_db: Option<f64>,
    /// The delay it found between speaker and mic, ms.
    pub delay_ms: Option<i32>,
}

impl Dsp {
    pub fn new(c: DspConfig) -> Self {
        let sc = StreamConfig::new(48_000, 1);
        let config = Config {
            pipeline: Pipeline {
                maximum_internal_processing_rate: MaxProcessingRate::Rate48kHz,
                ..Default::default()
            },
            high_pass_filter: Some(HighPassFilter::default()),
            echo_canceller: c.echo_cancel.then(EchoCanceller::default),
            noise_suppression: c.noise_suppress.then(|| NoiseSuppression {
                level: NoiseSuppressionLevel::Moderate,
                ..Default::default()
            }),
            gain_controller2: c.auto_gain.then(|| GainController2 {
                adaptive_digital: Some(Default::default()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let apm = AudioProcessing::builder()
            .config(config)
            .capture_config(sc)
            .render_config(sc)
            .build();
        Dsp {
            apm,
            scratch: [0.0; CHUNK],
        }
    }

    /// 10 ms about to be played.
    pub fn render(&mut self, x: &[f32; CHUNK]) {
        let _ = self
            .apm
            .process_render_f32(&[&x[..]], &mut [&mut self.scratch[..]]);
    }

    /// 10 ms from the mic, cleaned in place. `delay_ms` is the time from a
    /// sample being rendered to it being heard by the mic, as far as the
    /// buffers and devices say.
    pub fn capture(&mut self, x: &mut [f32; CHUNK], delay_ms: i32) {
        let _ = self.apm.set_stream_delay_ms(delay_ms.clamp(0, 500));
        let src = *x;
        let _ = self.apm.process_capture_f32(&[&src[..]], &mut [&mut x[..]]);
    }

    pub fn echo(&self) -> EchoStats {
        let s = self.apm.statistics();
        EchoStats {
            erle_db: s.echo_return_loss_enhancement,
            delay_ms: s.delay_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    /// Speech-like noise: filtered white noise with a slow envelope, from a
    /// fixed seed.
    fn voice(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        let mut lp = 0.0f32;
        (0..n)
            .map(|i| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let w = (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                lp = 0.7 * lp + 0.3 * w;
                let env =
                    0.5 + 0.5 * ((i as f32 / 48_000.0) * 2.0 * std::f32::consts::PI * 3.0).sin();
                lp * env * 0.6
            })
            .collect()
    }

    /// The far end's voice, played and heard back by the mic 30 ms later
    /// and quieter, is mostly removed once the canceller has converged.
    #[test]
    fn cancels_a_delayed_echo() {
        let mut d = Dsp::new(DspConfig {
            echo_cancel: true,
            noise_suppress: false,
            auto_gain: false,
        });
        let far = voice(48_000 * 6, 1);
        let delay = 48 * 30;
        let mut before = 0.0;
        let mut after = 0.0;
        for k in 0..far.len() / CHUNK {
            let mut r = [0.0; CHUNK];
            r.copy_from_slice(&far[k * CHUNK..(k + 1) * CHUNK]);
            d.render(&r);
            let mut mic = [0.0f32; CHUNK];
            for (i, m) in mic.iter_mut().enumerate() {
                let j = (k * CHUNK + i) as isize - delay as isize;
                *m = if j >= 0 { 0.5 * far[j as usize] } else { 0.0 };
            }
            let raw = rms(&mic);
            d.capture(&mut mic, 30);
            // The last two seconds, after convergence.
            if k >= 400 {
                before += raw;
                after += rms(&mic);
            }
        }
        let reduction_db = 20.0 * (before / after.max(1e-9)).log10();
        assert!(
            reduction_db > 15.0,
            "only {reduction_db:.1} dB of echo removed"
        );
    }
}
