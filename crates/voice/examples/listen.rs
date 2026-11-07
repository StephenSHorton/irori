//! Headless check of the engine:
//! `cargo run -p irori-voice --example listen -- <model.bin> [seconds] [speech.wav]`.
//! Prints the status, then every event of one session (default 8 s). With
//! a WAV (16-bit PCM, any rate, mono or stereo) the engine hears that file
//! in real time instead of the microphone.
//!
//! `listen -- --feed <model.bin> <speech.wav>` drives a call transcriber
//! instead (`start_feed` with the call's options): the WAV is pushed in
//! 10 ms chunks at its own rate, in real time, then a second of silence,
//! and each segment prints with its times.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let feed = args.iter().any(|a| a == "--feed");
    args.retain(|a| a != "--feed");
    let mut args = args.into_iter();
    let model = PathBuf::from(args.next().unwrap_or_else(|| voice::MODEL.file.to_string()));
    if feed {
        let wav = args.next().expect("--feed needs a WAV");
        run_feed(&model, &wav);
        voice::release();
        return;
    }
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(8);
    let wav = args.next();
    println!("status: {:?}", voice::status());
    let t0 = Instant::now();
    let sink: voice::Sink =
        Arc::new(move |e| println!("{:>6} ms  {e:?}", t0.elapsed().as_millis()));
    let started = match &wav {
        Some(p) => {
            let (pcm, rate) = read_wav(p);
            voice::start_with_samples(&model, to_16k(&pcm, rate), sink)
        }
        None => voice::start(&model, sink),
    };
    match started {
        Ok(session) => {
            println!("{:>6} ms  listening for {secs} s", t0.elapsed().as_millis());
            std::thread::sleep(Duration::from_secs(secs));
            session.stop();
            println!("{:>6} ms  stopped", t0.elapsed().as_millis());
        }
        Err(e) => println!("start failed: {:?}: {}", e.code, e.message),
    }
    voice::release();
}

/// Pushes the WAV like a call's tap would and prints the segments.
fn run_feed(model: &std::path::Path, wav: &str) {
    let (pcm, rate) = read_wav(wav);
    let t0 = Instant::now();
    let sink: voice::Sink = Arc::new(move |e| {
        let at = t0.elapsed().as_millis();
        match e {
            voice::Event::Segment {
                text,
                start_ms,
                end_ms,
            } => println!("{at:>6} ms  [{start_ms:>6} - {end_ms:>6}] {text}"),
            e => println!("{at:>6} ms  {e:?}"),
        }
    });
    let (session, feed) = match voice::start_feed(model, rate as u32, voice::Options::call(), sink)
    {
        Ok(s) => s,
        Err(e) => {
            println!("start failed: {:?}: {}", e.code, e.message);
            return;
        }
    };
    println!(
        "{:>6} ms  feeding {:.1} s at {rate} Hz",
        t0.elapsed().as_millis(),
        pcm.len() as f64 / rate as f64
    );
    let chunk = rate / 100;
    let silence = vec![0.0f32; chunk];
    let start = Instant::now();
    let chunks = pcm
        .chunks(chunk)
        .chain(std::iter::repeat_n(&silence[..], 100));
    for (i, c) in chunks.enumerate() {
        feed.push(c);
        // Keep to real time without drifting.
        let due = start + Duration::from_millis(10 * (i as u64 + 1));
        if let Some(d) = due.checked_duration_since(Instant::now()) {
            std::thread::sleep(d);
        }
    }
    session.stop();
    println!("{:>6} ms  stopped", t0.elapsed().as_millis());
}

/// A 16-bit PCM WAV as mono samples, and its rate.
fn read_wav(path: &str) -> (Vec<f32>, usize) {
    let d = std::fs::read(path).expect("read wav");
    let u16at = |i: usize| u16::from_le_bytes([d[i], d[i + 1]]);
    let u32at = |i: usize| u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
    let (mut ch, mut rate, mut i) = (1usize, 16_000usize, 12usize);
    while i + 8 <= d.len() {
        let id = &d[i..i + 4];
        let len = u32at(i + 4) as usize;
        if id == b"fmt " {
            ch = u16at(i + 10) as usize;
            rate = u32at(i + 12) as usize;
        } else if id == b"data" {
            let pcm = d[i + 8..(i + 8 + len).min(d.len())]
                .chunks(2 * ch)
                .map(|f| {
                    f.chunks(2)
                        .map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0)
                        .sum::<f32>()
                        / ch as f32
                })
                .collect();
            return (pcm, rate);
        }
        i += 8 + len + (len & 1);
    }
    panic!("no data chunk in {path}");
}

/// Nearest-sample resampling to 16 kHz (enough here).
fn to_16k(pcm: &[f32], rate: usize) -> Vec<f32> {
    let n = pcm.len() * 16_000 / rate;
    (0..n).map(|k| pcm[k * rate / 16_000]).collect()
}
