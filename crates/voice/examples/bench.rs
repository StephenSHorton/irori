//! Whisper timing on this machine:
//! `cargo run --release -p irori-voice --example bench -- <model.bin> <speech.wav>`.
//! Transcribes the first 1, 4 and 8 s of the WAV with a few settings.
//! On the phone: build it for aarch64-linux-android (docs/android.md) and
//! run it over adb, to see whether base.en keeps up with a call (#656).

#[cfg(any(windows, target_os = "macos", target_os = "android"))]
fn main() {
    use std::time::Instant;
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};
    let mut a = std::env::args().skip(1);
    let model = a.next().expect("model");
    let wav = a.next().expect("wav");
    let d = std::fs::read(&wav).unwrap();
    let pcm: Vec<f32> = d[44..]
        .chunks(2)
        .map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0)
        .collect();
    whisper_rs::install_logging_hooks();
    let mut cp = WhisperContextParameters::default();
    cp.flash_attn(std::env::var_os("FA").is_some());
    let ctx = WhisperContext::new_with_params(&model, cp).unwrap();
    let mut st = ctx.create_state().unwrap();
    for threads in [2, 4] {
        for secs in [1usize, 4, 8] {
            let n = (secs * 16_000).min(pcm.len());
            let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            p.set_language(Some("en"));
            p.set_n_threads(threads);
            p.set_no_context(true);
            p.set_single_segment(true);
            p.set_no_timestamps(true);
            p.set_temperature_inc(0.0);
            if std::env::var_os("AC").is_some() {
                // 50 encoder frames per second of audio, plus a margin.
                p.set_audio_ctx(((n * 50 / 16_000) + 64).min(1500) as i32);
            }
            p.set_print_progress(false);
            let t = Instant::now();
            st.full(p, &pcm[..n]).unwrap();
            let text: String = st
                .as_iter()
                .filter_map(|s| s.to_str_lossy().ok().map(|c| c.to_string()))
                .collect();
            println!(
                "threads {threads:>2}  {secs} s audio  {:>6} ms  {}",
                t.elapsed().as_millis(),
                text.trim()
            );
        }
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "android")))]
fn main() {}
