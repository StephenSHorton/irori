//! coord.md §6 "expiry timer fires without a row" as a behaviour test, on
//! the timer alone (I:internal/coord/subscribe_test.go
//! TestSubscribeExpiryTimer): a coord with no Follow loop, so no 1 s tick
//! can prune for it; nobody calls Sweep, Leases or expire. The lease
//! running out reaches the subscriber by itself, the timer re-arms for the
//! next lease, and it disarms when the last subscriber leaves.

use std::sync::Arc;
use std::time::{Duration, Instant};

use coord::{Change, Coord, LEASE_EXPIRED};
use gocompat::sub::Queue;
use irori_core::timers::{self, Kick};

const ALICE: &str = "dev-a/alice-session";

fn next(q: &Queue<Change>) -> Change {
    q.recv_timeout(Duration::from_secs(2))
        .expect("no change within 2s")
        .expect("subscription closed")
}

#[test]
fn test_expiry_timer_fires_without_a_row() {
    let dir = tempfile::tempdir().unwrap();
    let st = Arc::new(
        store::Store::open(
            dir.path().join("irori.db"),
            "dev-a",
            Arc::new(store::Clock::new()),
        )
        .unwrap(),
    );
    let now: Arc<dyn Fn() -> gocompat::time::GoTime + Send + Sync> = Arc::new(presence::now_local);
    let co = Arc::new(
        Coord::new(
            st.clone(),
            None,
            coord::Options {
                now: Some(now.clone()),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let rearm = Kick::new();
    timers::arm(&co, &rearm);
    let stop = sys::cancel::Cancel::new();
    let h = {
        let (co, rearm, stop, now) = (co.clone(), rearm.clone(), stop.clone(), now.clone());
        std::thread::spawn(move || timers::run_expiry(&co, &rearm, &*now, &stop))
    };

    let q = co.subscribe();
    co.acquire(ALICE, "gpu/long", Duration::from_secs(3600), false)
        .unwrap();
    let t = Instant::now();
    co.acquire(ALICE, "gpu/short", Duration::from_millis(100), false)
        .unwrap();
    let rows = st.vector().unwrap();
    next(&q);
    next(&q);
    assert_eq!(
        next(&q),
        Change {
            kind: LEASE_EXPIRED.into(),
            resource: "gpu/short".into(),
            holder: ALICE.into(),
            ..Change::default()
        },
        "want gpu/short expired"
    );
    assert!(
        t.elapsed() >= Duration::from_millis(90),
        "fired early: {:?}",
        t.elapsed()
    );
    assert_eq!(st.vector().unwrap(), rows, "no row was involved");
    let at = co.next_expiry().expect("re-armed");
    let in_50m = capture::gotime::add(&now(), Duration::from_secs(50 * 60));
    assert!(
        capture::gotime::after(&at, &in_50m),
        "timer not re-armed for gpu/long: {at}"
    );

    // The last subscriber leaving disarms it: a lease running out then
    // goes unnoticed until someone reads (Go stops the timer).
    drop(q);
    assert!(co.next_expiry().is_none());
    co.acquire(ALICE, "gpu/brief", Duration::from_millis(50), false)
        .unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        co.leases().iter().all(|l| l.resource != "gpu/brief"),
        "a read still prunes it"
    );

    stop.cancel();
    rearm.close();
    h.join().unwrap();
}
