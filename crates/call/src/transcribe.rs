//! The seam between a call and whatever writes down what its person says.
//! Core asks for a [`Transcriber`] through a [`TranscriberFn`], hands its
//! [`Tap`] to the engine, and gets each finished phrase as a [`Segment`];
//! it never links a speech model itself.

use std::sync::Arc;

/// Called on the engine thread with each 10 ms chunk (480 samples, 48 kHz
/// mono) of the cleaned mic, after mute; zeros stand in for audio the
/// engine dropped, so counting samples keeps the call's time. Must not
/// block, and neither must dropping it (the engine drops it on its thread).
pub type Tap = Box<dyn FnMut(&[f32]) + Send>;

/// One finished phrase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    /// Ms of audio since the first tapped sample.
    pub start_ms: u64,
    /// Ms of audio since the first tapped sample.
    pub end_ms: u64,
}

/// Where a transcriber sends its segments, from any thread.
pub type SegmentSink = Arc<dyn Fn(Segment) + Send + Sync>;

pub trait Transcriber: Send {
    /// The tap to give the engine.
    fn tap(&mut self) -> Tap;
    /// Finishes the phrase in progress (its Segment goes to the sink before
    /// this returns) and stops.
    fn stop(self: Box<Self>);
}

/// Starts a transcriber sending to the sink.
pub type TranscriberFn =
    Arc<dyn Fn(SegmentSink) -> Result<Box<dyn Transcriber>, String> + Send + Sync>;

/// For tests: a transcriber that hears loudness, not words.
pub mod fake {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::thread::JoinHandle;
    use std::time::Duration;

    use ringbuf::HeapRb;
    use ringbuf::traits::{Consumer, Observer, Producer, Split};

    use super::{Segment, SegmentSink, Tap, Transcriber, TranscriberFn};
    use crate::device::{HeapCons, HeapProd};
    use crate::dsp::CHUNK;
    use crate::engine::rms;

    /// A 10 ms chunk above this RMS is speech.
    pub const LOUD_RMS: f32 = 0.05;
    /// This much quiet ends a phrase, ms.
    pub const QUIET_MS: u64 = 500;
    /// Audio the ring holds before it drops, s.
    const RING_SECS: usize = 2;

    /// For tests: a [`TranscriberFn`] whose transcribers call each loud run
    /// (ended by [`QUIET_MS`] of quiet) "{label} {k}", k from 0, timed by
    /// the samples heard (audio dropped when its ring is full still counts).
    pub fn energy(label: &str) -> TranscriberFn {
        let label = label.to_string();
        Arc::new(move |sink| Ok(Box::new(Energy::start(&label, sink)) as Box<dyn Transcriber>))
    }

    /// For tests: see [`energy`].
    pub struct Energy {
        prod: Option<HeapProd<f32>>,
        /// Samples the tap dropped because the ring was full.
        dropped: Arc<AtomicU64>,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl Energy {
        pub fn start(label: &str, sink: SegmentSink) -> Energy {
            let (prod, cons) = HeapRb::<f32>::new(48_000 * RING_SECS).split();
            let stop = Arc::new(AtomicBool::new(false));
            let st = stop.clone();
            let dropped = Arc::new(AtomicU64::new(0));
            let dr = dropped.clone();
            let label = label.to_string();
            let thread = std::thread::Builder::new()
                .name("irori-fake-transcriber".into())
                .spawn(move || run(cons, dr, st, label, sink))
                .expect("spawn the fake transcriber");
            Energy {
                prod: Some(prod),
                dropped,
                stop,
                thread: Some(thread),
            }
        }
    }

    impl Transcriber for Energy {
        /// The first call gets the ring; later ones get a tap that drops.
        /// A chunk goes in whole or is counted as dropped.
        fn tap(&mut self) -> Tap {
            match self.prod.take() {
                Some(mut p) => {
                    let dropped = self.dropped.clone();
                    Box::new(move |x: &[f32]| {
                        if p.vacant_len() >= x.len() {
                            p.push_slice(x);
                        } else {
                            dropped.fetch_add(x.len() as u64, Ordering::Relaxed);
                        }
                    })
                }
                None => Box::new(|_: &[f32]| {}),
            }
        }

        /// Dropping does the work.
        fn stop(self: Box<Self>) {
            drop(self);
        }
    }

    impl Drop for Energy {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    struct Phrases {
        label: String,
        sink: SegmentSink,
        k: u64,
        /// Samples heard.
        n: u64,
        /// The phrase in progress: its start and its last loud sample.
        open: Option<(u64, u64)>,
    }

    impl Phrases {
        fn chunk(&mut self, x: &[f32]) {
            let end = self.n + x.len() as u64;
            if rms(x) > LOUD_RMS {
                let start = self.open.map_or(self.n, |(s, _)| s);
                self.open = Some((start, end));
            } else if let Some((_, last)) = self.open
                && end - last >= QUIET_MS * 48
            {
                self.flush();
            }
            self.n = end;
        }

        fn flush(&mut self) {
            if let Some((s, e)) = self.open.take() {
                (self.sink)(Segment {
                    text: format!("{} {}", self.label, self.k),
                    start_ms: s / 48,
                    end_ms: e / 48,
                });
                self.k += 1;
            }
        }
    }

    fn run(
        mut cons: HeapCons<f32>,
        dropped: Arc<AtomicU64>,
        stop: Arc<AtomicBool>,
        label: String,
        sink: SegmentSink,
    ) {
        let mut p = Phrases {
            label,
            sink,
            k: 0,
            n: 0,
            open: None,
        };
        let mut x = [0f32; CHUNK];
        loop {
            // Read the flag first, so audio tapped before stop is still heard.
            let stopping = stop.load(Ordering::Acquire);
            while cons.occupied_len() >= CHUNK {
                cons.pop_slice(&mut x);
                p.chunk(&x);
            }
            // Dropped when the ring was full, so after what it held.
            p.n += dropped.swap(0, Ordering::Relaxed);
            if stopping {
                p.flush();
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
