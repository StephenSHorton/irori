//! What one Board view costs on a real log, and how long a window read
//! waits while views run back to back (card #554).
//!
//! `cargo run --release -p irori-coord --example board_cost -- <copy of
//! irori.db> <device id> [rebuild gap ms]`
//!
//! Point it at a copy: opening a store may migrate it. The gap is the wait
//! between views: 0 is a rebuild per change through a burst, 250 a
//! coalesced one.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use coord::{BoardFilter, Coord, Options};
use store::Store;

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p) as usize]
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (db, dev) = (&a[1], &a[2]);
    let gap = Duration::from_millis(a.get(3).and_then(|s| s.parse().ok()).unwrap_or(0));
    let st = Arc::new(Store::open(db, dev, Arc::new(hlc::Clock::new())).unwrap());
    let t = Instant::now();
    let co = Arc::new(Coord::new(st.clone(), None, Options::default()).unwrap());
    println!("replay: {:.2}s", t.elapsed().as_secs_f64());

    let f = BoardFilter {
        viewer: format!("user:{dev}"),
        done_limit: 1,
        ..BoardFilter::default()
    };
    let v = co.board(&f);
    println!(
        "cards: needs {} blocked {} working {} open {} done {}",
        v.needs_you.len(),
        v.blocked.len(),
        v.working.len(),
        v.open.len(),
        v.done_total
    );
    version_waits(&co, &st, &f);
    let mut ms: Vec<f64> = (0..300)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(co.board(&f));
            t.elapsed().as_secs_f64() * 1e3
        })
        .collect();
    println!(
        "view alone: p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms",
        pct(&mut ms, 0.5),
        pct(&mut ms, 0.95),
        pct(&mut ms, 1.0)
    );

    let stop = Arc::new(AtomicBool::new(false));
    let views = Arc::new(AtomicU64::new(0));
    let th = {
        let (co, f, stop, views) = (co.clone(), f.clone(), stop.clone(), views.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::hint::black_box(co.board(&f));
                views.fetch_add(1, Ordering::Relaxed);
                std::thread::sleep(gap);
            }
        })
    };
    let run = Instant::now();
    let mut waits = Vec::new();
    while run.elapsed() < Duration::from_secs(5) {
        let t = Instant::now();
        std::hint::black_box(co.unresolved()); // takes the lock, reads almost nothing
        waits.push(t.elapsed().as_secs_f64() * 1e3);
        std::thread::sleep(Duration::from_millis(5));
    }
    stop.store(true, Ordering::Relaxed);
    th.join().unwrap();
    println!(
        "gap {} ms: {} views in 5s; lock wait p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms ({} reads)",
        gap.as_millis(),
        views.load(Ordering::Relaxed),
        pct(&mut waits, 0.5),
        pct(&mut waits, 0.95),
        pct(&mut waits, 1.0),
        waits.len()
    );
}

/// The version waits on open cards, and what one lookup costs through the
/// app's own store path.
fn version_waits(co: &Coord, st: &Store, f: &BoardFilter) {
    let v = co.board(f);
    let mut n = 0;
    let mut devs = std::collections::BTreeSet::new();
    for c in v
        .needs_you
        .iter()
        .chain(&v.blocked)
        .chain(&v.working)
        .chain(&v.open)
    {
        for b in &c.task.blockers {
            let Some(on) = b.on.as_ref().filter(|_| b.open()) else {
                continue;
            };
            if on.kind == "version" {
                n += 1;
                println!("  {} wait: {:?}", c.task.short_ref(), on.reference);
                devs.extend(coord::parse_version_wait(&on.reference).1);
            }
        }
    }
    println!("open version waits: {n}");
    for d in devs {
        let t = Instant::now();
        let evs = st
            .list(&store::Filter {
                kinds: vec![store::kinds::DEVICE_STARTED.into()],
                device_id: d.clone(),
                desc: true,
                limit: 1,
                ..store::Filter::default()
            })
            .unwrap_or_default();
        println!(
            "  device {d:?}: {} row, {:.2} ms",
            evs.len(),
            t.elapsed().as_secs_f64() * 1e3
        );
    }
}
