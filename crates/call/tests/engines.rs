//! Whole engines on fake devices, linked in-process: what one mic says comes
//! out of the other speakers, and how long it takes.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use call::device::fake::{self, Recording};
use call::dsp::DspConfig;
use call::{Config, Engine, Inlet, Link};

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
fn bursts() -> fake::Gen {
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
fn later_bursts() -> fake::Gen {
    let mut b = bursts();
    Box::new(move |n| if n < 96_000 { 0.0 } else { b(n - 48_000) })
}

fn silence() -> fake::Gen {
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

fn node(sound: fake::Gen) -> Node {
    let rec = Arc::new(Mutex::new(Recording::default()));
    let engine = Engine::start(config(), fake::devices(sound, rec.clone())).expect("engine");
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
fn deafening_silences_the_speaker_and_the_mic() {
    let a = node(bursts());
    let b = node(bursts());
    b.engine.set_deafened(true);
    connect(&[("a", &a), ("b", &b)]);
    std::thread::sleep(Duration::from_millis(1600));
    {
        let r = b.rec.lock().unwrap();
        assert!(onset(&r.samples, 0).is_none(), "a deafened speaker plays nothing");
        let r = a.rec.lock().unwrap();
        assert!(onset(&r.samples, 0).is_none(), "a deafened mic sends nothing");
    }
    let st = b.engine.stats();
    assert!(st.deafened && !st.muted, "{st:?}");
    assert!(st.peers[0].jitter.received > 10, "still receiving: {st:?}");
    assert!(a.engine.stats().peers[0].jitter.muted, "the others see it muted");

    // Undeafen: a's voice plays again, from now.
    let at = b.rec.lock().unwrap().samples.len();
    b.engine.set_deafened(false);
    // The next burst starts at 3 s.
    std::thread::sleep(Duration::from_millis(2000));
    let r = b.rec.lock().unwrap();
    assert!(onset(&r.samples, at).is_some(), "heard again after Undeafen");
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
