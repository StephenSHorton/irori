//! share-dev: list what can be shared, and time capture + hardware H.264 at
//! the card's target (1080p, 10 fps) for #657.
//!
//! ```sh
//! cargo run --release -p irori-share --example share-dev -- list
//! cargo run --release -p irori-share --example share-dev -- synth --secs 30
//! cargo run --release -p irori-share --example share-dev -- window <id or title> --secs 30
//! cargo run --release -p irori-share --example share-dev -- display --secs 30
//! cargo run --release -p irori-share --example share-dev -- grant [--request]
//! ```
//!
//! Options for synth, window and display:
//! `--secs N` (default 20), `--fps N` (10), `--size WxH` (1920x1080),
//! `--kbps N` (moq-video's default), `--key-every S` (ask for a keyframe
//! every S seconds, default 5; 0 never), `--gop S` (a keyframe at least every
//! S seconds even unasked, default 60 so only our asks make them),
//! `--kbps-at T:N` (change the bitrate to N kbit/s at second T),
//! `--software` (allow a software encoder), `--no-cursor`.
//!
//! CPU is this whole process (capture, scaling, encoder, this loop), as a
//! percent of one core: run it in release, nothing else in the process.
//! synth hands the encoder CPU I420 frames, which it uploads into a
//! CVPixelBuffer (Mac) or NV12 sample (PC) first; real capture is zero-copy
//! from the capture surface, so synth's CPU is an upper bound. synth draws
//! its frames before the meter starts and only copies one per tick.

use share::{Capture, Chunk, Encoder, Settings, Source, cpu, synthetic_frame, timestamp};
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let result = rt.block_on(run(args));
    if let Err(e) = result {
        eprintln!("share-dev: {e}");
        std::process::exit(1);
    }
}

async fn run(args: Vec<String>) -> Result<(), String> {
    let Some(cmd) = args.first().map(String::as_str) else {
        return Err(usage());
    };
    let rest = &args[1..];
    match cmd {
        "list" => list().await,
        "grant" => grant(rest),
        "synth" => bench(None, &Opts::parse(rest)?).await,
        "window" => {
            let Some(want) = rest.first().filter(|a| !a.starts_with("--")) else {
                return Err("window needs an id or part of a title (see list)".into());
            };
            let id = find_window(want).await?;
            bench(Some(Source::Window(id)), &Opts::parse(&rest[1..])?).await
        }
        "display" => {
            let (id, opts) = match rest.first().filter(|a| !a.starts_with("--")) {
                Some(id) => (Some(id.clone()), &rest[1..]),
                None => (None, rest),
            };
            bench(Some(Source::Display(id)), &Opts::parse(opts)?).await
        }
        _ => Err(usage()),
    }
}

fn usage() -> String {
    "usage: share-dev list | grant [--request] | synth [opts] | window <id|title> [opts] | display [id] [opts]".into()
}

async fn list() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    println!("screen recording grant: {}", share::grant::granted());
    match share::displays().await {
        Ok(ds) => {
            println!("displays:");
            for d in ds {
                println!("  {:<12} {}x{}  {}", d.id, d.width, d.height, d.name);
            }
        }
        Err(e) => println!("displays: {e}"),
    }
    let ws = share::windows().await.map_err(|e| e.to_string())?;
    println!("windows ({}):", ws.len());
    for w in ws {
        println!(
            "  {:<12} {:>5}x{:<5} {} | {}",
            w.id, w.width, w.height, w.app, w.title
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn grant(rest: &[String]) -> Result<(), String> {
    println!("granted before: {}", share::grant::granted());
    if rest.iter().any(|a| a == "--request") {
        println!("request returned: {}", share::grant::request());
        println!("granted after: {}", share::grant::granted());
    }
    println!("settings: open \"{}\"", share::grant::SETTINGS_URL);
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn grant(_: &[String]) -> Result<(), String> {
    println!("no grant to check here: Windows.Graphics.Capture asks nothing");
    Ok(())
}

/// A window by its exact id, or the only one whose title or app contains
/// `want` (ignoring case).
async fn find_window(want: &str) -> Result<String, String> {
    let ws = share::windows().await.map_err(|e| e.to_string())?;
    if ws.iter().any(|w| w.id == want) {
        return Ok(want.to_string());
    }
    let low = want.to_lowercase();
    let hits: Vec<_> = ws
        .iter()
        .filter(|w| w.title.to_lowercase().contains(&low) || w.app.to_lowercase().contains(&low))
        .collect();
    match hits.as_slice() {
        [w] => {
            println!("window {} ({} | {})", w.id, w.app, w.title);
            Ok(w.id.clone())
        }
        [] => Err(format!("no window matches {want:?} ({} listed)", ws.len())),
        many => Err(format!(
            "{} windows match {want:?}: {}",
            many.len(),
            many.iter()
                .map(|w| format!("{} ({} | {})", w.id, w.app, w.title))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

struct Opts {
    secs: u64,
    settings: Settings,
    key_every: Option<Duration>,
    kbps_at: Option<(u64, u32)>,
}

impl Opts {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut o = Opts {
            secs: 20,
            settings: Settings {
                keyframe_every: Duration::from_secs(60),
                ..Settings::default()
            },
            key_every: Some(Duration::from_secs(5)),
            kbps_at: None,
        };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut val = || it.next().cloned().ok_or(format!("{a} needs a value"));
            let num = |s: String| s.parse::<u64>().map_err(|_| format!("bad number {s:?}"));
            match a.as_str() {
                "--secs" => o.secs = num(val()?)?,
                "--fps" => o.settings.fps = num(val()?)? as u32,
                "--kbps" => o.settings.bitrate_kbps = Some(num(val()?)? as u32),
                "--gop" => o.settings.keyframe_every = Duration::from_secs(num(val()?)?),
                "--key-every" => {
                    let s = num(val()?)?;
                    o.key_every = (s > 0).then(|| Duration::from_secs(s));
                }
                "--kbps-at" => {
                    let v = val()?;
                    let (t, k) = v.split_once(':').ok_or("--kbps-at wants T:KBPS")?;
                    o.kbps_at = Some((num(t.into())?, num(k.into())? as u32));
                }
                "--size" => {
                    let v = val()?;
                    let (w, h) = v.split_once('x').ok_or("--size wants WxH")?;
                    o.settings.width = num(w.into())? as u32;
                    o.settings.height = num(h.into())? as u32;
                }
                "--software" => o.settings.hardware_only = false,
                "--no-cursor" => o.settings.cursor = false,
                _ => return Err(format!("unknown option {a}")),
            }
        }
        Ok(o)
    }
}

/// Where the frames come from.
enum Feed {
    Capture(Capture),
    /// Pre-drawn frames, played round and round.
    Synth(Vec<moq_video::I420>),
}

#[derive(Default)]
struct Tally {
    frames: u64,
    bytes: u64,
    keys: u64,
    key_bytes: u64,
    encode_ms: Vec<f64>,
}

impl Tally {
    fn add(&mut self, chunks: &[Chunk]) {
        for c in chunks {
            self.frames += 1;
            self.bytes += c.data.len() as u64;
            if c.keyframe {
                self.keys += 1;
                self.key_bytes += c.data.len() as u64;
            }
        }
    }
}

async fn bench(source: Option<Source>, o: &Opts) -> Result<(), String> {
    let s = &o.settings;
    let e = |e: share::Error| e.to_string();
    let mut feed = match &source {
        Some(src) => {
            let cap = Capture::open(src, s).await.map_err(e)?;
            let (w, h) = cap.captured_size();
            println!(
                "capturing {src:?} at {w}x{h}, encoding at {}x{}",
                s.width, s.height
            );
            Feed::Capture(cap)
        }
        None => {
            println!(
                "synthetic {}x{} (drawing 30 frames first)",
                s.width, s.height
            );
            let mut ring = Vec::new();
            for n in 0..30 {
                let f = synthetic_frame(s.width, s.height, n, Duration::ZERO).map_err(e)?;
                match f.surface {
                    moq_video::Surface::I420(i) => ring.push(i),
                    _ => return Err("synthetic frame is not I420".into()),
                }
            }
            Feed::Synth(ring)
        }
    };
    let mut enc = Encoder::new(s).map_err(e)?;
    println!(
        "encoder {} at {} kbit/s, {} fps, {} cores; {} s",
        enc.name(),
        enc.bitrate_kbps(),
        s.fps,
        cpu::cores(),
        o.secs
    );
    println!(" sec  frames  kbit/s  keys  key KB  enc ms(avg)  cpu %1core");

    let tick = Duration::from_secs_f64(1.0 / f64::from(s.fps));
    let meter = cpu::Meter::start();
    let start = Instant::now();
    let mut total = Tally::default();
    let mut sec = Tally::default();
    let mut sec_meter = cpu::Meter::start();
    let mut next_key = o.key_every.map(|d| start + d);
    let mut asked: Option<Instant> = None;
    let mut key_latency_ms: Vec<f64> = Vec::new();
    let mut unanswered = 0u32;
    let mut kbps_done = false;
    let mut next_report = start + Duration::from_secs(1);
    let mut n = 0u64;

    while start.elapsed() < Duration::from_secs(o.secs) {
        // One frame: the next capture, or (synth) the next tick.
        let frame = match &mut feed {
            Feed::Capture(cap) => cap.next().await.map_err(e)?,
            Feed::Synth(ring) => {
                let due = start + tick * n as u32;
                tokio::time::sleep_until(due.into()).await;
                let pic = ring[(n % ring.len() as u64) as usize].clone();
                share::Frame::new(
                    moq_video::Surface::I420(pic),
                    timestamp(start.elapsed()).map_err(e)?,
                )
            }
        };
        n += 1;

        let now = Instant::now();
        if next_key.is_some_and(|t| now >= t) {
            if asked.is_some() {
                unanswered += 1;
            }
            enc.request_keyframe().map_err(e)?;
            asked = Some(now);
            next_key = o.key_every.map(|d| now + d);
        }
        if let Some((t, k)) = o.kbps_at
            && !kbps_done
            && now >= start + Duration::from_secs(t)
        {
            enc.set_bitrate_kbps(k).map_err(e)?;
            println!("      bitrate now {} kbit/s", enc.bitrate_kbps());
            kbps_done = true;
        }

        let t0 = Instant::now();
        let chunks = enc.encode(&frame).map_err(e)?;
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        if let Some(at) = asked
            && chunks.iter().any(|c| c.keyframe)
        {
            key_latency_ms.push(at.elapsed().as_secs_f64() * 1000.0);
            asked = None;
        }
        total.encode_ms.push(ms);
        sec.encode_ms.push(ms);
        total.add(&chunks);
        sec.add(&chunks);

        if Instant::now() >= next_report {
            let i = start.elapsed().as_secs();
            println!(
                "{:>4}  {:>6}  {:>6.0}  {:>4}  {:>6.1}  {:>11.2}  {:>10.1}",
                i,
                sec.frames,
                sec.bytes as f64 * 8.0 / sec_meter.wall().as_secs_f64() / 1000.0,
                sec.keys,
                sec.key_bytes as f64 / 1000.0,
                avg(&sec.encode_ms),
                sec_meter.percent_of_one_core()
            );
            sec = Tally::default();
            sec_meter = cpu::Meter::start();
            next_report += Duration::from_secs(1);
        }
    }
    let tail = enc.flush().map_err(e)?;
    total.add(&tail);

    let wall = meter.wall().as_secs_f64();
    let cpu_pct = meter.percent_of_one_core();
    let p_frames = total.frames - total.keys;
    let mut sorted = total.encode_ms.clone();
    sorted.sort_by(f64::total_cmp);
    println!("---");
    println!("backend        {}", enc.name());
    println!(
        "frames         {} in {:.1} s ({:.2} fps)",
        total.frames,
        wall,
        total.frames as f64 / wall
    );
    println!(
        "bitrate        {:.0} kbit/s (target {})",
        total.bytes as f64 * 8.0 / wall / 1000.0,
        enc.bitrate_kbps()
    );
    println!(
        "keyframes      {} (avg {:.1} KB); other frames avg {:.1} KB",
        total.keys,
        total.key_bytes as f64 / total.keys.max(1) as f64 / 1000.0,
        (total.bytes - total.key_bytes) as f64 / p_frames.max(1) as f64 / 1000.0
    );
    println!(
        "keyframe asks  {} answered, avg {:.1} ms to the keyframe chunk; {} unanswered",
        key_latency_ms.len(),
        avg(&key_latency_ms),
        unanswered + u32::from(asked.is_some())
    );
    println!(
        "encode ms      avg {:.2}, p50 {:.2}, p95 {:.2}, max {:.2}",
        avg(&sorted),
        pct(&sorted, 0.50),
        pct(&sorted, 0.95),
        sorted.last().copied().unwrap_or(0.0)
    );
    println!(
        "cpu            {:.1}% of one core ({:.2}% of all {} cores), {:.2} s cpu",
        cpu_pct,
        cpu_pct / cpu::cores() as f64,
        cpu::cores(),
        meter.cpu().as_secs_f64()
    );
    Ok(())
}

fn avg(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}
