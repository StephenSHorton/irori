//! Whole engines on fake devices, linked in-process: what one mic says comes
//! out of the other speakers, and how long it takes.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use call::device::fake::{self as dev, Recording};
use call::dsp::DspConfig;
use call::transcribe::fake as tr;
use call::{Config, Engine, Inlet, Link, Segment, Tap};

/// Delivers to another engine as if over a network, from `from`.
struct Wire {
    to: Inlet,
    from: String,
}

impl Link for Wire {
    fn send(&self, d: &[u8]) -> bool {
        self.to.receive(&self.from, d.to_vec());
        true
    }
}

/// 200 ms of a 600 Hz tone at 1 s, 3 s, 5 s ...; silence between.
fn bursts() -> dev::Gen {
    Box::new(|n| {
        let t = n as f64 / 48_000.0;
        let in_burst = t >= 1.0 && (t - 1.0) % 2.0 < 0.2;
        if in_burst {
            0.3 * (2.0 * std::f64::consts::PI * 600.0 * t).sin() as f32
        } else {
            0.0
        }
    })
}

/// The same bursts, starting at 2 s.
fn later_bursts() -> dev::Gen {
    let mut b = bursts();
    Box::new(move |n| if n < 96_000 { 0.0 } else { b(n - 48_000) })
}

fn silence() -> dev::Gen {
    Box::new(|_| 0.0)
}

fn config() -> Config {
    // Echo cancelling on, as in a call; noise suppression and gain off so a
    // pure tone isn't treated as noise and the levels stay predictable.
    Config {
        dsp: DspConfig {
            echo_cancel: true,
            noise_suppress: false,
            auto_gain: false,
        },
        ..Config::default()
    }
}

struct Node {
    engine: Engine,
    rec: Arc<Mutex<Recording>>,
}

fn node(sound: dev::Gen) -> Node {
    let rec = Arc::new(Mutex::new(Recording::default()));
    let engine = Engine::start(config(), dev::devices(sound, rec.clone())).expect("engine");
    Node { engine, rec }
}

fn connect(nodes: &[(&str, &Node)]) {
    for (a, na) in nodes {
        for (b, nb) in nodes {
            if a != b {
                na.engine.add_peer(
                    b,
                    Arc::new(Wire {
                        to: nb.engine.inlet(),
                        from: a.to_string(),
                    }),
                );
            }
        }
    }
}

/// The first sample index at or after `from` whose 1 ms block is loud.
fn onset(x: &[f32], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 48 <= x.len() {
        let r = (x[i..i + 48].iter().map(|v| v * v).sum::<f32>() / 48.0).sqrt();
        if r > 0.05 {
            return Some(i);
        }
        i += 48;
    }
    None
}

#[test]
fn two_engines_carry_a_voice_both_ways_under_150_ms() {
    let a = node(bursts());
    let b = node(later_bursts());
    connect(&[("a", &a), ("b", &b)]);
    std::thread::sleep(Duration::from_millis(3600));

    let (b_samples, b_start) = {
        let r = b.rec.lock().unwrap();
        (r.samples.clone(), r.started.unwrap())
    };
    let a_start = a.rec.lock().unwrap().started.unwrap();
    // The second burst (3 s on a's mic), so the jitter buffer has settled.
    let said = a_start + Duration::from_millis(3000);
    let from = said.saturating_duration_since(b_start).as_millis() as usize * 48;
    let heard = onset(&b_samples, from.saturating_sub(48 * 50)).expect("b heard a");
    let heard_at = b_start + Duration::from_secs_f64(heard as f64 / 48_000.0);
    let ms = heard_at.saturating_duration_since(said).as_secs_f64() * 1000.0;
    eprintln!("software mouth-to-ear (fake devices): {ms:.1} ms");
    assert!(ms > 20.0 && ms < 150.0, "{ms} ms");

    let sb = b.engine.stats();
    let pa = &sb.peers[0];
    assert_eq!(pa.id, "a");
    assert_eq!(pa.jitter.lost, 0, "{pa:?}");
    assert!(pa.jitter.received > 10, "{pa:?}");

    // And back: b's voice (from 2 s) comes out of a's speaker.
    let a_samples = a.rec.lock().unwrap().samples.clone();
    let from = (b_start + Duration::from_millis(2000)).saturating_duration_since(a_start);
    let back = onset(&a_samples, from.as_millis() as usize * 48);
    assert!(back.is_some(), "a heard b");
}

#[test]
fn a_third_engine_joins_and_hears_and_is_heard() {
    let a = node(bursts());
    let b = node(silence());
    let c = node(silence());
    connect(&[("a", &a), ("b", &b), ("c", &c)]);
    std::thread::sleep(Duration::from_millis(1600));
    for n in [&b, &c] {
        let r = n.rec.lock().unwrap();
        let loud = onset(&r.samples, 0);
        assert!(loud.is_some(), "a third device hears the first");
    }
    let st = c.engine.stats();
    assert_eq!(st.peers.len(), 2);
    assert!(
        st.peers
            .iter()
            .any(|p| p.id == "a" && p.jitter.received > 10),
        "{st:?}"
    );
}

#[test]
fn muting_stops_the_voice_and_says_so() {
    let a = node(bursts());
    let b = node(silence());
    a.engine.set_muted(true);
    connect(&[("a", &a), ("b", &b)]);
    std::thread::sleep(Duration::from_millis(1600));
    let r = b.rec.lock().unwrap();
    assert!(
        onset(&r.samples, 0).is_none(),
        "nothing heard from a muted mic"
    );
    let st = b.engine.stats();
    assert!(st.peers[0].jitter.muted, "{st:?}");
    assert!(a.engine.stats().muted);
}

#[test]
fn removing_a_peer_stops_sending_to_it() {
    let a = node(bursts());
    let b = node(silence());
    connect(&[("a", &a), ("b", &b)]);
    std::thread::sleep(Duration::from_millis(300));
    a.engine.remove_peer("b");
    let before = b.engine.stats().peers[0].jitter.received;
    std::thread::sleep(Duration::from_millis(400));
    let st = b.engine.stats();
    assert!(
        st.peers[0].jitter.received <= before + 1,
        "{before} -> {:?}",
        st.peers[0]
    );
    let _ = Instant::now();
}

/// What a tap heard: each chunk's length and loudest sample.
type Heard = Arc<Mutex<Vec<(usize, f32)>>>;

fn listen() -> (Tap, Heard) {
    let heard: Heard = Arc::default();
    let h = heard.clone();
    let tap: Tap = Box::new(move |x: &[f32]| {
        let peak = x.iter().fold(0f32, |m, v| m.max(v.abs()));
        h.lock().unwrap().push((x.len(), peak));
    });
    (tap, heard)
}

#[test]
fn a_tap_hears_the_mic_in_10_ms_chunks() {
    let a = node(bursts());
    let (tap, heard) = listen();
    a.engine.set_tap(Some(tap));
    std::thread::sleep(Duration::from_millis(1500));
    let h = heard.lock().unwrap().clone();
    assert!(h.iter().all(|&(n, _)| n == 480));
    assert!(
        (130..=165).contains(&h.len()),
        "{} chunks in 1.5 s",
        h.len()
    );
    assert!(h.iter().any(|&(_, p)| p > 0.1), "the burst at 1 s");
}

#[test]
fn a_muted_engine_taps_zeros() {
    let a = node(bursts());
    a.engine.set_muted(true);
    let (tap, heard) = listen();
    a.engine.set_tap(Some(tap));
    std::thread::sleep(Duration::from_millis(1500));
    let h = heard.lock().unwrap().clone();
    assert!(h.len() > 100, "{} chunks", h.len());
    assert!(h.iter().all(|&(_, p)| p == 0.0));
}

#[test]
fn set_tap_none_stops_the_tap() {
    let a = node(silence());
    let (tap, heard) = listen();
    a.engine.set_tap(Some(tap));
    std::thread::sleep(Duration::from_millis(300));
    a.engine.set_tap(None);
    std::thread::sleep(Duration::from_millis(50));
    let before = heard.lock().unwrap().len();
    assert!(before > 10, "{before}");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(heard.lock().unwrap().len(), before);
}

#[test]
fn the_fake_transcriber_writes_one_segment_per_burst() {
    let a = node(bursts());
    let got: Arc<Mutex<Vec<Segment>>> = Arc::default();
    let g = got.clone();
    let mut t = tr::energy("a")(Arc::new(move |s| g.lock().unwrap().push(s))).expect("start");
    a.engine.set_tap(Some(t.tap()));
    // The bursts at 1 s and 3 s; stop in the quiet after the second, before
    // QUIET_MS has passed, so stop() must flush it.
    std::thread::sleep(Duration::from_millis(3400));
    a.engine.set_tap(None);
    t.stop();
    let got = got.lock().unwrap().clone();
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(got[0].text, "a 0");
    assert_eq!(got[1].text, "a 1");
    for (i, s) in got.iter().enumerate() {
        let at = 1000 + 2000 * i as u64;
        assert!(s.start_ms + 150 > at && s.start_ms < at + 100, "{s:?}");
        let len = s.end_ms - s.start_ms;
        assert!((150..=300).contains(&len), "{s:?}");
    }
}
