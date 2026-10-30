//! I:internal/fswatch/coalesce.go: merges repeats of a path inside the
//! debounce window and forwards them. Lost events (a full `out`, or a full
//! raw queue the reader reports through `in_lost`) leave a sticky Overflow
//! that goes out as soon as `out` has room, so a reader that falls behind
//! always learns it must rescan.
#![cfg_attr(not(windows), allow(dead_code))]

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
                    flush(&mut pending, &mut lost);
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
    use std::sync::mpsc::{Receiver, sync_channel};

    fn ev(p: &str) -> Event {
        Event {
            path: p.into(),
            overflow: false,
        }
    }

    /// Go `drain`: reads until quiet for 300 ms.
    fn drain(out: &Receiver<Event>) -> (Vec<String>, bool) {
        let mut paths = vec![];
        let mut overflow = false;
        while let Ok(e) = out.recv_timeout(Duration::from_millis(300)) {
            if e.overflow {
                overflow = true;
            } else {
                paths.push(e.path);
            }
        }
        (paths, overflow)
    }

    fn lost_case(run: impl FnOnce(&SyncSender<Event>, &AtomicBool)) {
        let (tx, rx) = sync_channel(8);
        let (otx, orx) = sync_channel(2);
        let lost = Arc::new(AtomicBool::new(false));
        run(&tx, &lost);
        let l2 = lost.clone();
        let t = std::thread::spawn(move || {
            coalesce(rx, otx, Duration::from_millis(1), r"C:\r".into(), l2)
        });
        std::thread::sleep(Duration::from_millis(100)); // let the flush meet a full out
        let (_, overflow) = drain(&orx);
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
        let t = std::thread::spawn(move || {
            coalesce(
                rx,
                otx,
                Duration::from_millis(1),
                "x".into(),
                Arc::new(AtomicBool::new(false)),
            )
        });
        for i in 0..8u8 {
            tx.send(ev(&format!("x/{}", char::from(b'a' + i)))).unwrap();
        }
        std::thread::sleep(Duration::from_millis(50)); // the flush finds out full
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
