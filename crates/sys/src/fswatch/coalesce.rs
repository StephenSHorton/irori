//! I:internal/fswatch/coalesce.go: merges repeats of a path inside the
//! debounce window and forwards them. Lost events (a full `out`, or a full
//! raw queue the reader reports through `in_lost`) leave a sticky Overflow
//! that goes out as soon as `out` has room, so a reader that falls behind
//! always learns it must rescan.
#![cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use super::Event;

/// How often an owed Overflow is offered again while `out` is full (Go's
/// select waits on the send; std channels can't, so this polls).
const OVERFLOW_RETRY: Duration = Duration::from_millis(5);

/// Go `send`: never blocks the OS reader; an event that doesn't fit is
/// dropped and marks `lost`.
pub(crate) fn send(ch: &SyncSender<Event>, e: Event, lost: &AtomicBool) {
    if let Err(TrySendError::Full(_)) = ch.try_send(e) {
        lost.store(true, Ordering::SeqCst);
    }
}

pub(crate) fn coalesce(
    rx: Receiver<Event>,
    out: SyncSender<Event>,
    debounce: Duration,
    root: String,
    in_lost: Arc<AtomicBool>,
) {
    coalesce_observed(rx, out, debounce, root, in_lost, |_| {});
}

/// `coalesce` that reports each timed flush to `flushed` (true when it lost
/// an event to a full `out`), so a test can wait for the flush it sets up
/// instead of sleeping and hoping the thread got there first.
fn coalesce_observed(
    rx: Receiver<Event>,
    out: SyncSender<Event>,
    debounce: Duration,
    root: String,
    in_lost: Arc<AtomicBool>,
    mut flushed: impl FnMut(bool),
) {
    let overflow = Event {
        path: root,
        overflow: true,
    };
    let mut pending: HashMap<String, Event> = HashMap::new();
    let mut timer: Option<Instant> = None;
    let mut lost = false;
    let flush = |pending: &mut HashMap<String, Event>, lost: &mut bool| {
        for (_, e) in pending.drain() {
            if out.try_send(e).is_err() {
                *lost = true;
            }
        }
    };
    loop {
        if in_lost.swap(false, Ordering::SeqCst) {
            lost = true;
        }
        if lost {
            match out.try_send(overflow.clone()) {
                Ok(()) => lost = false,
                Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
        let now = Instant::now();
        let mut wait: Option<Duration> = timer.map(|t| t.saturating_duration_since(now));
        if lost {
            wait = Some(wait.map_or(OVERFLOW_RETRY, |w| w.min(OVERFLOW_RETRY)));
        }
        let got = match wait {
            Some(w) => rx.recv_timeout(w),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match got {
            Ok(e) => {
                if e.overflow {
                    lost = true;
                    continue;
                }
                pending.insert(e.path.clone(), e);
                if timer.is_none() {
                    timer = Some(Instant::now() + debounce);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if timer.is_some_and(|t| Instant::now() >= t) {
                    let was_lost = lost;
                    lost = false;
                    flush(&mut pending, &mut lost);
                    flushed(lost);
                    lost |= was_lost;
                    timer = None;
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                flush(&mut pending, &mut lost);
                if lost || in_lost.load(Ordering::SeqCst) {
                    let _ = out.try_send(overflow);
                }
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::sync_channel;

    fn ev(p: &str) -> Event {
        Event {
            path: p.into(),
            overflow: false,
        }
    }

    fn lost_case(run: impl FnOnce(&SyncSender<Event>, &AtomicBool)) {
        let (tx, rx) = sync_channel(8);
        let (otx, orx) = sync_channel(2);
        let (ftx, frx) = std::sync::mpsc::channel();
        let lost = Arc::new(AtomicBool::new(false));
        run(&tx, &lost);
        let l2 = lost.clone();
        let t = std::thread::spawn(move || {
            coalesce_observed(
                rx,
                otx,
                Duration::from_millis(1),
                r"C:
"
                .into(),
                l2,
                |l| {
                    let _ = ftx.send(l);
                },
            )
        });
        // Read nothing until the flush has met the full out: a reader that
        // started first (a thread slow to start on a loaded machine) makes
        // room, nothing is lost and there is nothing to report.
        let met_full = frx
            .recv_timeout(Duration::from_secs(10))
            .expect("coalesce never flushed");
        assert!(
            met_full,
            "the flush found room in out; the case is not set up"
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        let overflow = loop {
            match orx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(e) if e.overflow => break true,
                Ok(_) => {}
                Err(_) => break false,
            }
        };
        assert!(overflow, "events were lost without an Overflow");
        drop(tx);
        t.join().unwrap();
    }

    #[test]
    fn lost_events_always_report_overflow_out_full_at_flush() {
        lost_case(|tx, _| {
            for i in 0..5 {
                tx.send(ev(&format!(r"C:\r\p{i}"))).unwrap();
            }
        });
    }

    #[test]
    fn lost_events_always_report_overflow_raw_full_at_send() {
        // The raw queue has room for 8: the ninth send is dropped and must
        // still surface as an Overflow once coalesce drains it.
        lost_case(|tx, lost| {
            for i in 0..9 {
                send(tx, ev(&format!(r"C:\r\q{i}")), lost);
            }
        });
    }

    #[test]
    fn slow_reader_gets_overflow() {
        let (tx, rx) = sync_channel(0);
        let (otx, orx) = sync_channel(4);
        let (ftx, frx) = std::sync::mpsc::channel();
        let t = std::thread::spawn(move || {
            coalesce_observed(
                rx,
                otx,
                Duration::from_millis(1),
                "x".into(),
                Arc::new(AtomicBool::new(false)),
                |l| {
                    let _ = ftx.send(l);
                },
            )
        });
        for i in 0..8u8 {
            tx.send(ev(&format!("x/{}", char::from(b'a' + i)))).unwrap();
        }
        // Read nothing until a flush has found out full: 8 events into room
        // for 4 must lose one while nobody reads.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !frx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("no flush met a full out")
        {}
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut got = 0;
        loop {
            let e = orx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|_| panic!("no Overflow after {got} events"));
            got += 1;
            if e.overflow {
                break;
            }
        }
        drop(tx);
        t.join().unwrap();
    }

    #[test]
    fn repeats_of_a_path_merge() {
        let (tx, rx) = sync_channel(16);
        let (otx, orx) = sync_channel(16);
        let t = std::thread::spawn(move || {
            coalesce(
                rx,
                otx,
                Duration::from_millis(20),
                "R".into(),
                Arc::new(AtomicBool::new(false)),
            )
        });
        for _ in 0..5 {
            tx.send(ev("a")).unwrap();
        }
        assert_eq!(orx.recv_timeout(Duration::from_secs(5)).unwrap(), ev("a"));
        assert!(orx.recv_timeout(Duration::from_millis(80)).is_err());
        drop(tx);
        t.join().unwrap();
    }
}
