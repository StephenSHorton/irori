//! How long a window read waits while a sync catch-up runs on a real log
//! (card #567).
//!
//! `cargo run --release -p irori-coord --example catchup_cost -- <copy of
//! irori.db> <this device> <other device> <followed rows behind> [views|notify]`
//!
//! With `views` a second reader runs whole Board views through the catch-up;
//! with `notify` one runs a view per Change, as notify-watch does. Each view
//! holds the lock itself, so leave them out to see the catch-up alone.
//!
//! Point it at a fresh copy each run: it moves the other device's newest
//! rows aside, replays, puts them back and catches up, and the fold's duties
//! may write rows. The clock stands still at the log's newest row, so two
//! runs on the same copy end in the same state; the digests compare them.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use coord::{BoardFilter, Coord, Options, TaskFilter};
use gocompat::sub::TryRecv;
use gocompat::time::GoTime;
use store::Store;

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p) as usize]
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Moves `dev`'s newest `behind` followed rows, and every row of it after
/// them, to a side table. The oldest seq moved.
fn stash(db: &str, dev: &str, behind: i64) -> i64 {
    let c = rusqlite::Connection::open(db).unwrap();
    let like: Vec<String> = coord::FOLLOW_KINDS
        .iter()
        .map(|k| match k.strip_suffix('.') {
            Some(_) => format!("kind LIKE '{k}%'"),
            None => format!("kind = '{k}'"),
        })
        .collect();
    let from: i64 = c
        .query_row(
            &format!(
                "SELECT seq FROM events WHERE device_id = ?1 AND ({}) ORDER BY seq DESC LIMIT 1 OFFSET ?2",
                like.join(" OR ")
            ),
            rusqlite::params![dev, behind - 1],
            |r| r.get(0),
        )
        .unwrap();
    c.execute_batch("CREATE TABLE catchup_stash AS SELECT * FROM events WHERE 0")
        .unwrap();
    let n = c
        .execute(
            "INSERT INTO catchup_stash SELECT * FROM events WHERE device_id = ?1 AND seq >= ?2",
            rusqlite::params![dev, from],
        )
        .unwrap();
    c.execute(
        "DELETE FROM events WHERE device_id = ?1 AND seq >= ?2",
        rusqlite::params![dev, from],
    )
    .unwrap();
    println!("moved aside: {n} rows of {dev} from seq {from}");
    from
}

fn unstash(db: &str) {
    let c = rusqlite::Connection::open(db).unwrap();
    c.execute_batch("INSERT INTO events SELECT * FROM catchup_stash; DROP TABLE catchup_stash;")
        .unwrap();
}

fn newest_wall(db: &str) -> i64 {
    let c = rusqlite::Connection::open(db).unwrap();
    c.query_row("SELECT max(hlc) FROM events", [], |r| r.get::<_, i64>(0))
        .map(|h| hlc::Timestamp(h as u64).wall_ms())
        .unwrap()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (db, me, other) = (&a[1], &a[2], &a[3]);
    let behind: i64 = a[4].parse().unwrap();
    let with_views = a.get(5).is_some_and(|s| s == "views");
    let notify = a.get(5).is_some_and(|s| s == "notify");
    let wall = newest_wall(db);
    stash(db, other, behind);
    let st = Arc::new(Store::open(db, me, Arc::new(hlc::Clock::new())).unwrap());
    let at = GoTime::from_system(std::time::UNIX_EPOCH + Duration::from_millis(wall as u64));
    let t = Instant::now();
    let co = Arc::new(
        Coord::new(
            st.clone(),
            None,
            Options {
                now: Some(Arc::new(move || at)),
                ..Options::default()
            },
        )
        .unwrap(),
    );
    println!("replay: {:.2}s", t.elapsed().as_secs_f64());
    let f = BoardFilter {
        viewer: format!("user:{me}"),
        done_limit: 1,
        ..BoardFilter::default()
    };
    let cards = co.list_tasks(&TaskFilter::default()).len();
    let t = Instant::now();
    std::hint::black_box(co.board(&f));
    println!(
        "cards: {cards}; one view alone: {:.2} ms",
        t.elapsed().as_secs_f64() * 1e3
    );
    unstash(db);

    let changes = co.subscribe();
    let stop = Arc::new(AtomicBool::new(false));
    // A cheap read, and a whole view: what a window asks for.
    let reader = |view: bool, every: u64| {
        let (co, f, stop) = (co.clone(), f.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut w = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                let t = Instant::now();
                if view {
                    std::hint::black_box(co.board(&f));
                } else {
                    std::hint::black_box(co.unresolved());
                }
                w.push(t.elapsed().as_secs_f64() * 1e3);
                std::thread::sleep(Duration::from_millis(every));
            }
            w
        })
    };
    let drain = {
        let (q, stop, co, f) = (changes.clone(), stop.clone(), co.clone(), f.clone());
        std::thread::spawn(move || {
            let mut seen: BTreeMap<String, u64> = BTreeMap::new();
            loop {
                match q.try_recv() {
                    TryRecv::Item(c) => {
                        *seen.entry(c.kind).or_default() += 1;
                        if notify {
                            std::hint::black_box(co.board(&f));
                        }
                    }
                    _ if stop.load(Ordering::Relaxed) => return seen,
                    _ => std::thread::sleep(Duration::from_micros(200)),
                }
            }
        })
    };
    let reads = reader(false, 1);
    let views = with_views.then(|| reader(true, 20));
    std::thread::sleep(Duration::from_millis(100));
    let t = Instant::now();
    co.catch_up().unwrap();
    let took = t.elapsed().as_secs_f64() * 1e3;
    std::thread::sleep(Duration::from_millis(100));
    stop.store(true, Ordering::Relaxed);
    let mut r = reads.join().unwrap();
    let mut v = views.map(|h| h.join().unwrap()).unwrap_or_default();
    let seen = drain.join().unwrap();
    println!("catch-up: {took:.1} ms");
    println!(
        "read wait: p50 {:.3} ms, p95 {:.3} ms, p99 {:.3} ms, max {:.1} ms ({} reads)",
        pct(&mut r, 0.5),
        pct(&mut r, 0.95),
        pct(&mut r, 0.99),
        pct(&mut r, 1.0),
        r.len()
    );
    println!(
        "view: p50 {:.2} ms, p95 {:.2} ms, max {:.1} ms ({} views)",
        pct(&mut v, 0.5),
        pct(&mut v, 0.95),
        pct(&mut v, 1.0),
        v.len()
    );
    println!("changes: {seen:?}");

    let ts = co.list_tasks(&TaskFilter::default());
    let links: String = ts
        .iter()
        .map(|t| {
            format!(
                "{} {} {:?} {:?} {}\n",
                t.id, t.parent, t.subtasks, t.rollup, t.side
            )
        })
        .collect();
    let all = serde_json::to_string(&ts).unwrap();
    println!(
        "digest: {} cards, links {:016x}, all {:016x}",
        ts.len(),
        fnv(&links),
        fnv(&all)
    );
}
