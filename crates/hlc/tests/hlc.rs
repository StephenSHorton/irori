//! The Go hlc tests (I:internal/hlc/hlc_test.go, store.md §6 "hlc"), plus a
//! replay of Go-recorded clock scripts (testdata/golden/hlc/hlc.json).

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hlc::{Clock, Timestamp, unix_milli};

fn at_ms(ms: i64) -> SystemTime {
    if ms >= 0 {
        UNIX_EPOCH + Duration::from_millis(ms as u64)
    } else {
        UNIX_EPOCH - Duration::from_millis(ms.unsigned_abs())
    }
}

fn fixed(ms: i64) -> Clock {
    Clock::with_now(Box::new(move || at_ms(ms)))
}

#[test]
fn observe_bounded_caps_the_jump() {
    let wall = 1_700_000_000_000i64;
    let c = fixed(wall);
    let bound = Timestamp::from_unix_ms(wall + 2 * 60 * 1000);
    c.observe_bounded(Timestamp::from_unix_ms(wall + 24 * 3600 * 1000), bound);
    let got = c.now();
    assert!(
        got > bound && got.0 <= bound.0 + 1,
        "after a far-future observe: {got}, bound {bound}"
    );
    c.observe_bounded(Timestamp(5), bound);
    assert_eq!(c.now().0, bound.0 + 2, "clock moved back or jumped");
}

#[test]
fn now_increases_when_wall_stalls() {
    let fixed_ms = 1_700_000_000_000i64;
    let c = fixed(fixed_ms);
    let (a, b) = (c.now(), c.now());
    assert!(b > a, "want b > a, got {b} <= {a}");
    assert_eq!(a.wall(), at_ms(fixed_ms));
    assert_eq!(a.wall_ms(), fixed_ms);
    assert_eq!(b.logical(), 1);
}

#[test]
fn observe_moves_past_remote() {
    let c = fixed(1000);
    let remote = Timestamp(5000 << 16);
    c.observe(remote);
    assert!(c.now() > remote);
}

#[test]
fn json_is_a_bare_integer() {
    let t = Timestamp(1 << 62 | 7);
    assert_eq!(serde_json::to_string(&t).unwrap(), "4611686018427387911");
    assert_eq!(
        serde_json::from_str::<Timestamp>("4611686018427387911").unwrap(),
        t
    );
    assert_eq!(Timestamp::from_i64(t.as_i64()), t);
    assert_eq!(Timestamp(u64::MAX).as_i64(), -1);
}

#[test]
fn unix_milli_floors_before_1970() {
    assert_eq!(unix_milli(UNIX_EPOCH - Duration::from_micros(500)), -1);
    assert_eq!(unix_milli(UNIX_EPOCH - Duration::from_millis(3)), -3);
    assert_eq!(unix_milli(UNIX_EPOCH + Duration::from_micros(1500)), 1);
}

#[test]
fn concurrent_now_is_strictly_increasing_per_clock() {
    let c = Arc::new(fixed(1_700_000_000_000));
    let mut hs = Vec::new();
    for _ in 0..8 {
        let c = c.clone();
        hs.push(std::thread::spawn(move || {
            (0..500).map(|_| c.now().0).collect::<Vec<_>>()
        }));
    }
    let mut all: Vec<u64> = hs.into_iter().flat_map(|h| h.join().unwrap()).collect();
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), 8 * 500, "a timestamp was handed out twice");
}

/// Go-recorded scripts: each "now" must return what Go's clock returned.
#[test]
fn go_golden_scripts() {
    let raw = std::fs::read(manifest_dir().join("../../testdata/golden/hlc/hlc.json")).unwrap();
    let g: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    let scripts = g["scripts"].as_object().unwrap();
    assert!(scripts.len() >= 6);
    for (name, steps) in scripts {
        let wall = Arc::new(AtomicI64::new(0));
        let w = wall.clone();
        let c = Clock::with_now(Box::new(move || at_ms(w.load(Ordering::SeqCst))));
        for (i, s) in steps.as_array().unwrap().iter().enumerate() {
            let a = Timestamp(s["a"].as_u64().unwrap_or(0));
            let b = Timestamp(s["b"].as_u64().unwrap_or(0));
            match s["op"].as_str().unwrap() {
                "wall" => wall.store(s["wallMs"].as_i64().unwrap(), Ordering::SeqCst),
                "now" => {
                    let want = s["out"].as_u64().unwrap_or(0);
                    assert_eq!(c.now().0, want, "{name} step {i}");
                }
                "observe" => c.observe(a),
                "observeBounded" => c.observe_bounded(a, b),
                op => panic!("unknown op {op}"),
            }
        }
    }
    for d in g["decode"].as_array().unwrap() {
        let t = Timestamp(d["t"].as_u64().unwrap());
        assert_eq!(t.wall_ms(), d["wallMs"].as_i64().unwrap(), "{t}");
        assert_eq!(
            u64::from(t.logical()),
            d["logical"].as_u64().unwrap(),
            "{t}"
        );
        assert_eq!(unix_milli(t.wall()), t.wall_ms());
    }
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
