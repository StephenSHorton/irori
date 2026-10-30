//! The window's pumps (Go inbox_service_test.go, board_service_test.go's
//! TestBoardPumpForwards, cards_service_test.go's
//! TestCardChangesReachThePump, devices:pairing) on a scratch Core, with
//! the emitter recording what reaches the page.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use irori_window::events;
use irori_window::inbox::InboxCounts;
use sys::cancel::Cancel;

/// What the emitter saw, in order.
#[derive(Default, Clone)]
struct Seen(Arc<Mutex<Vec<(String, serde_json::Value)>>>);

impl Seen {
    fn of(&self, name: &str) -> Vec<serde_json::Value> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
            .collect()
    }
}

/// Starts the pumps on `r` with a recording emitter.
fn pumps(r: &Rig) -> (Seen, Cancel) {
    let seen = Seen::default();
    let s = seen.clone();
    r.w.set_emitter(move |name, v| s.0.lock().unwrap().push((name.to_string(), v)));
    let stop = Cancel::new();
    events::start(r.w.clone(), stop.clone());
    (seen, stop)
}

fn agent(key: &str) -> inbox::Sender {
    inbox::Sender::new(key, "agent", "agent")
}

/// Go TestInboxOneScanPerChange and TestInboxScansBeforeFirstChange: the
/// counts and Held read the pump's one scan per change; before the first
/// change they scan on demand; the person's own action makes the next
/// read live. The tray's counts come from that same scan (Go `onCounts`).
#[test]
fn inbox_one_scan_per_change() {
    let r = Rig::new();
    // Before the pump's first scan: on demand.
    assert_eq!(r.w.inbox_counts(), InboxCounts::default());

    let tray: Arc<Mutex<Vec<InboxCounts>>> = Arc::default();
    let t = tray.clone();
    r.w.on_inbox_counts(Arc::new(move |c| t.lock().unwrap().push(c)));
    assert_eq!(tray.lock().unwrap().as_slice(), &[InboxCounts::default()]);

    let (seen, stop) = pumps(&r);
    let to = inbox::Address::new(inbox::addr::AGENT, &r.key_b);
    r.w.core
        .inbox()
        .send(&agent(&r.key_a), &to, "one", "")
        .unwrap();
    wait("inbox:changed with one unread", || {
        seen.of(events::EVENT_INBOX_CHANGED)
            .last()
            .is_some_and(|v| v["unread"] == 1)
    });
    // The read after the event is the pump's scan, and the tray got it.
    assert_eq!(r.w.inbox_counts().unread, 1);
    wait("the tray's counts", || {
        tray.lock().unwrap().last().map(|c| c.unread) == Some(1)
    });

    // The person's own action drops the snapshot: the next read is live,
    // before any event.
    r.w.inbox_set_paused(true).unwrap();
    assert!(r.w.inbox_counts().paused);
    // A pause re-queues nothing, yet the nudge still reaches the page and
    // the tray (Go `nudge`).
    wait("inbox:changed paused", || {
        seen.of(events::EVENT_INBOX_CHANGED)
            .last()
            .is_some_and(|v| v["paused"] == true)
    });
    wait("the tray's pause", || {
        tray.lock().unwrap().last().is_some_and(|c| c.paused)
    });
    stop.cancel();
}

/// Go TestBoardPumpForwards and TestCardChangesReachThePump: a card change
/// reaches board:changed without a poll, a burst is coalesced, and the
/// tray's decide count follows every change.
#[test]
fn board_pump_forwards_and_coalesces() {
    let r = Rig::new();
    let decide: Arc<Mutex<Vec<usize>>> = Arc::default();
    let d = decide.clone();
    r.w.on_decide_count(Arc::new(move |n| d.lock().unwrap().push(n)));
    assert_eq!(decide.lock().unwrap().as_slice(), &[0]);
    let (seen, stop) = pumps(&r);
    let co = r.w.core.coord();

    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "t".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap();
    co.set_steps(
        &r.key_a,
        &t.id,
        vec![coord::Step {
            text: "a".into(),
            ..Default::default()
        }],
    )
    .unwrap();
    wait("board:changed after a card change", || {
        !seen.of(events::EVENT_BOARD_CHANGED).is_empty()
    });

    let before = seen.of(events::EVENT_BOARD_CHANGED).len();
    for _ in 0..20 {
        co.create_task(
            &r.key_a,
            coord::NewTask {
                title: "t".into(),
                ..Default::default()
            },
        )
        .unwrap();
    }
    co.raise(
        &r.key_a,
        &t.id,
        coord::NewBlocker {
            question: "Ship today?".into(),
            ..Default::default()
        },
    )
    .unwrap();
    wait("the tray's decide count", || {
        decide.lock().unwrap().last() == Some(&1)
    });
    std::thread::sleep(Duration::from_millis(400));
    let n = seen.of(events::EVENT_BOARD_CHANGED).len() - before;
    assert!(
        (1..=10).contains(&n),
        "{n} emits for 21 changes, want them coalesced"
    );
    assert!(
        seen.of(events::EVENT_BOARD_CHANGED)
            .iter()
            .all(|v| *v == serde_json::json!({}))
    );
    stop.cancel();
}

/// The pumps stop on cancel; nothing is emitted after.
#[test]
fn pumps_stop_on_cancel() {
    let r = Rig::new();
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    r.w.set_emitter(move |_, _| {
        c.fetch_add(1, Ordering::SeqCst);
    });
    let stop = Cancel::new();
    let hs = events::start(r.w.clone(), stop.clone());
    stop.cancel();
    for h in hs {
        h.join().unwrap();
    }
    let at = count.load(Ordering::SeqCst);
    r.w.core
        .coord()
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "late".into(),
                ..Default::default()
            },
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(count.load(Ordering::SeqCst), at);
}

/// A Rig whose pairing runs on loopback iroh only (no LAN, no relay).
fn loopback_rig() -> Rig {
    Rig::with(|o| {
        o.pair_net = Some(Arc::new(|ident, chain, guests| {
            let key = iroh::SecretKey::from_bytes(&ident.expose_device_seed());
            let admit = Arc::new(irori_net::admit::ChainAdmit {
                chain: chain.clone(),
                self_id: ident.device_id(),
            });
            let mut nc = irori_net::transport::NetConfig::new(ident.device_id(), key, admit);
            nc.ip = irori_net::endpoint::IpMode::Loopback;
            nc.guests = guests.clone();
            irori_net::transport::IrohNet::new(nc)
        }));
    })
}

/// Go DevicesService.attach: every pairing change reaches the page as
/// devices:pairing, through the Core's Pairer (Add a device, Cancel).
#[test]
fn pairing_changes_reach_the_page() {
    let r = loopback_rig();
    let (seen, stop) = pumps(&r);
    let p = r.w.seams.pairing.clone();
    let st = p.add("desktop").unwrap();
    assert_eq!(st.state, irori_pair::STATE_WAITING);
    assert!(!st.code.is_empty() && !st.fingerprint.is_empty());
    assert_eq!(p.status(), st);
    wait("devices:pairing waiting", || {
        seen.of(events::EVENT_DEVICES_PAIRING)
            .iter()
            .any(|v| v["state"] == irori_pair::STATE_WAITING && v["code"] == st.code.as_str())
    });
    // Nobody joined yet: the Pairer's own text.
    assert_eq!(
        p.confirm_words("not the words").unwrap_err(),
        "pair: no pairing is waiting for words"
    );
    p.cancel();
    wait("devices:pairing after cancel", || {
        seen.of(events::EVENT_DEVICES_PAIRING)
            .last()
            .is_some_and(|v| v["state"] != irori_pair::STATE_WAITING)
    });
    assert!(!r.w.core.guest_windows().any_open());
    stop.cancel();
}
