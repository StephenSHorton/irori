//! The mic and speaker. Device callbacks only copy samples into and out of
//! lock-free rings; all the work happens on the engine's thread. A device
//! opener runs on that thread too, since a cpal stream may not move between
//! threads on every platform.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use ringbuf::HeapRb;
use ringbuf::traits::{Producer, Split};

pub use ringbuf::{HeapCons, HeapProd};

/// How long the devices themselves hold audio, as their callbacks report.
#[derive(Debug, Default)]
pub struct DeviceLatency {
    /// Mic: from the sound reaching it to the callback, µs.
    pub input_us: AtomicU32,
    /// Speaker: from the callback to the sound leaving it, µs.
    pub output_us: AtomicU32,
    /// Speaker callbacks that found too little to play.
    pub underruns: AtomicU64,
    /// Errors the devices reported (unplugged, taken away): the call can't
    /// hear or play until it is joined again.
    pub errors: AtomicU64,
}

/// Open devices: a ring of mono mic samples to read, and one of mono
/// speaker samples to fill.
pub struct Devices {
    pub mic: HeapCons<f32>,
    pub mic_rate: u32,
    pub speaker: HeapProd<f32>,
    pub speaker_rate: u32,
    pub latency: Arc<DeviceLatency>,
    /// Whatever must live as long as the devices play (the streams).
    pub keep: Box<dyn std::any::Any>,
}

/// Opens devices on the engine's thread.
pub type Opener = Box<dyn FnOnce() -> Result<Devices, String> + Send>;

/// Makes an [`Opener`] for each call: the default devices in the app, fake
/// ones in tests.
pub type OpenerFn = Arc<dyn Fn() -> Opener + Send + Sync>;

/// Ring sizes, in seconds of audio: room for a stalled engine thread.
const RING_SECS: f32 = 0.5;

/// A ring pair for a device of `rate`: the producer the device fills (or
/// the consumer it drains) and the other end for the engine.
pub fn ring(rate: u32) -> (HeapProd<f32>, HeapCons<f32>) {
    HeapRb::<f32>::new((rate as f32 * RING_SECS) as usize).split()
}

/// The name of the mic a call would open ("MacBook Pro Microphone"), or ""
/// when there is none or it can't say. It only asks: nothing is opened.
pub fn default_mic_name() -> String {
    use cpal::traits::HostTrait;

    cpal::default_host()
        .default_input_device()
        .map(|d| d.to_string())
        .unwrap_or_default()
}

/// The default mic and speaker, through cpal.
pub fn default_devices() -> Opener {
    Box::new(open_default)
}

fn open_default() -> Result<Devices, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    let mic = host.default_input_device().ok_or("no microphone")?;
    let spk = host.default_output_device().ok_or("no speaker")?;
    let in_cfg = pick(&mic, true)?;
    let out_cfg = pick(&spk, false)?;
    let latency = Arc::new(DeviceLatency::default());

    let (mut mic_prod, mic_cons) = ring(in_cfg.sample_rate);
    let (spk_prod, mut spk_cons) = ring(out_cfg.sample_rate);

    let (in_rate, out_rate) = (in_cfg.sample_rate, out_cfg.sample_rate);
    let in_ch = in_cfg.channels as usize;
    let lat = latency.clone();
    let mut mono = Vec::<f32>::new();
    let input = mic
        .build_input_stream::<f32, _, _>(
            in_cfg,
            move |data: &[f32], info: &cpal::InputCallbackInfo| {
                let ts = info.timestamp();
                {
                    let d = ts.callback.duration_since(ts.capture);
                    lat.input_us.store(d.as_micros() as u32, Ordering::Relaxed);
                }
                mono.clear();
                mono.extend(
                    data.chunks(in_ch)
                        .map(|f| f.iter().sum::<f32>() / in_ch as f32),
                );
                // A full ring means the engine thread stalled: drop the
                // newest rather than block the device.
                let _ = mic_prod.push_slice(&mono);
            },
            {
                let lat = latency.clone();
                move |e| {
                    eprintln!("call: mic: {e}");
                    lat.errors.fetch_add(1, Ordering::Relaxed);
                }
            },
            None,
        )
        .map_err(|e| format!("open the microphone: {e}"))?;

    let out_ch = out_cfg.channels as usize;
    let lat = latency.clone();
    let mut mono = Vec::<f32>::new();
    let output = spk
        .build_output_stream::<f32, _, _>(
            out_cfg,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                use ringbuf::traits::Consumer;
                let ts = info.timestamp();
                {
                    let d = ts.playback.duration_since(ts.callback);
                    lat.output_us.store(d.as_micros() as u32, Ordering::Relaxed);
                }
                let frames = data.len() / out_ch;
                mono.resize(frames, 0.0);
                let got = spk_cons.pop_slice(&mut mono);
                if got < frames {
                    mono[got..].fill(0.0);
                    lat.underruns.fetch_add(1, Ordering::Relaxed);
                }
                for (f, s) in data.chunks_mut(out_ch).zip(&mono) {
                    f.fill(*s);
                }
            },
            {
                let lat = latency.clone();
                move |e| {
                    eprintln!("call: speaker: {e}");
                    lat.errors.fetch_add(1, Ordering::Relaxed);
                }
            },
            None,
        )
        .map_err(|e| format!("open the speaker: {e}"))?;

    input
        .play()
        .map_err(|e| format!("start the microphone: {e}"))?;
    output
        .play()
        .map_err(|e| format!("start the speaker: {e}"))?;
    Ok(Devices {
        mic: mic_cons,
        mic_rate: in_rate,
        speaker: spk_prod,
        speaker_rate: out_rate,
        latency,
        keep: Box::new((input, output)),
    })
}

/// An f32 config at 48 kHz if the device has one, else its default rate in
/// f32 (and the engine resamples).
fn pick(dev: &cpal::Device, input: bool) -> Result<cpal::StreamConfig, String> {
    use cpal::traits::DeviceTrait;
    let f32_48k = if input {
        dev.supported_input_configs().ok().and_then(|it| {
            it.filter(|r| r.sample_format() == cpal::SampleFormat::F32)
                .find_map(|r| r.try_with_sample_rate(48_000))
        })
    } else {
        dev.supported_output_configs().ok().and_then(|it| {
            it.filter(|r| r.sample_format() == cpal::SampleFormat::F32)
                .find_map(|r| r.try_with_sample_rate(48_000))
        })
    };
    let chosen = match f32_48k {
        Some(c) => c,
        None => {
            let d = if input {
                dev.default_input_config()
            } else {
                dev.default_output_config()
            };
            d.map_err(|e| {
                format!(
                    "no usable {} config: {e}",
                    if input { "mic" } else { "speaker" }
                )
            })?
        }
    };
    if chosen.sample_format() != cpal::SampleFormat::F32 {
        return Err(format!(
            "the {} only offers {:?} samples",
            if input { "microphone" } else { "speaker" },
            chosen.sample_format()
        ));
    }
    Ok(chosen.config())
}

/// Devices with no hardware behind them: a generator for the mic and a
/// recorder for the speaker, both clocked in real time in 10 ms steps.
/// For tests and call-dev's tone mode.
pub mod fake {
    use super::*;
    use ringbuf::traits::{Consumer, Producer};
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};

    /// The mic's sound: sample index (48 kHz) to value.
    pub type Gen = Box<dyn FnMut(u64) -> f32 + Send>;

    /// What the speaker played, 48 kHz from when the devices opened.
    #[derive(Default)]
    pub struct Recording {
        pub samples: Vec<f32>,
        /// When sample 0 of both the mic and the speaker happened.
        pub started: Option<Instant>,
    }

    struct Clock {
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for Clock {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    /// Opens a fake mic playing `gen` and a fake speaker recording into
    /// `rec`, both at 48 kHz. Mic sample n and speaker sample n happen at
    /// the same moment.
    pub fn devices(mut sound: Gen, rec: Arc<Mutex<Recording>>) -> Opener {
        Box::new(move || {
            let (mut mic_prod, mic_cons) = ring(48_000);
            let (spk_prod, mut spk_cons) = ring(48_000);
            let stop = Arc::new(AtomicBool::new(false));
            let st = stop.clone();
            let thread = std::thread::spawn(move || {
                let t0 = Instant::now();
                rec.lock().expect("rec").started = Some(t0);
                let mut n: u64 = 0;
                let mut buf = [0f32; 480];
                for tick in 1u64.. {
                    if st.load(Ordering::Relaxed) {
                        return;
                    }
                    for s in buf.iter_mut() {
                        *s = sound(n);
                        n += 1;
                    }
                    let _ = mic_prod.push_slice(&buf);
                    let got = spk_cons.pop_slice(&mut buf);
                    buf[got..].fill(0.0);
                    rec.lock().expect("rec").samples.extend_from_slice(&buf);
                    let due = t0 + Duration::from_millis(10 * tick);
                    if let Some(w) = due.checked_duration_since(Instant::now()) {
                        std::thread::sleep(w);
                    }
                }
            });
            Ok(Devices {
                mic: mic_cons,
                mic_rate: 48_000,
                speaker: spk_prod,
                speaker_rate: 48_000,
                latency: Arc::new(DeviceLatency::default()),
                keep: Box::new(Clock {
                    stop,
                    thread: Some(thread),
                }),
            })
        })
    }
}
