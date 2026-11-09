//! The window's pumps (Go inbox_service_test.go, board_service_test.go's
//! TestBoardPumpForwards, cards_service_test.go's
//! TestCardChangesReachThePump, devices:pairing) on a scratch Core, with
//! the emitter recording what reaches the page.

use crate::common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use irori_core::{CoreEvent, NotifChange, TalkChange};
use irori_window::events;
use irori_window::inbox::InboxCounts;
use irori_window::signals::{self, Bucket, Expander, MAX_IDS};
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
    // Every payload is the envelope; the first card's id rides in one.
    let all = seen.of(events::EVENT_BOARD_CHANGED);
    assert!(
        all.iter()
            .all(|v| v["seq"].is_u64() && v["all"].is_boolean() && v["cards"].is_array())
    );
    assert!(
        all.iter().any(|v| v["cards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| *c == t.id.as_str())),
        "no payload named the card {}: {all:?}",
        t.id
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
    // Nobody joined, so cancel shuts the guest window before it returns
    // (card #119): it used to close in a spawned task, after the page had
    // already heard the new state, and this check raced it.
    assert!(!r.w.core.guest_windows().any_open());
    wait("devices:pairing after cancel", || {
        seen.of(events::EVENT_DEVICES_PAIRING)
            .last()
            .is_some_and(|v| v["state"] != irori_pair::STATE_WAITING)
    });
    assert!(!r.w.core.guest_windows().any_open());
    stop.cancel();
}

fn board_ev(kind: &str, task_id: &str, resource: &str) -> CoreEvent {
    CoreEvent::Board(coord::Change {
        kind: kind.into(),
        task_id: task_id.into(),
        resource: resource.into(),
        ..Default::default()
    })
}

fn strs(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .unwrap_or_else(|| panic!("not a list: {v}"))
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

/// The kind map (list-signals.md §4): each kind goes where the table says,
/// and anything else, an empty kind included, is `all`.
#[test]
fn the_kind_map_sends_each_kind_to_its_bucket() {
    let r = Rig::new();
    let mut ex = Expander::default();
    let mut f = signals::fold_board(&[
        board_ev(coord::TASK_CHANGED, "t1", ""),
        board_ev(coord::COMMENT_ADDED, "t2", ""),
        board_ev(coord::TASK_CHANGED, "t1", ""),
        board_ev(coord::NOTE_CHANGED, "", "n1"),
        board_ev(coord::DESK_CHANGED, "", "repo"),
        board_ev(coord::BOARD_PIN_CHANGED, "board:b1", ""),
    ]);
    assert!(!f.all && f.desk && f.pin);
    assert_eq!(
        f.cards,
        ["t1", "t2"],
        "a card once, the pin's id not a card"
    );
    assert_eq!(f.notes, ["n1"]);
    // Expanded: unknown ids are carried as they are, the desk and the note
    // ride along.
    let c = r.w.expand(&mut ex, &mut f).unwrap();
    assert!(!c.all && c.desk && c.notes == ["n1"]);
    assert!(c.cards.contains(&"t1".to_string()) && c.cards.contains(&"t2".to_string()));
    assert!(!c.cards.iter().any(|c| c.starts_with("board:")));

    assert_eq!(signals::bucket_of(coord::TASK_CHANGED), Bucket::Card);
    assert_eq!(signals::bucket_of(coord::BOARD_PIN_CHANGED), Bucket::Pin);
    for kind in [
        coord::RESYNC,
        irori_core::spawn_capable::AGENTS_CHANGED,
        "",
        "a kind nobody mapped",
    ] {
        assert_eq!(signals::bucket_of(kind), Bucket::All, "{kind:?}");
        let f = signals::fold_board(&[board_ev(kind, "t1", "")]);
        assert!(f.all, "{kind:?} should read everything");
        // Only an agents change keeps the grid's cache (card #1147): it
        // moves what every read builds anew.
        let keeps = kind == irori_core::spawn_capable::AGENTS_CHANGED;
        assert_eq!(f.cold, !keeps, "{kind:?}");
    }
    // A lease change names its holder, whose rows the cache knows (card
    // #1147): not all.
    for kind in [
        coord::LEASE_CHANGED,
        coord::LEASE_EXPIRED,
        coord::LEASE_REFUSED,
    ] {
        assert_eq!(signals::bucket_of(kind), Bucket::Lease, "{kind:?}");
        let f = signals::fold_board(&[CoreEvent::Board(coord::Change {
            kind: kind.into(),
            resource: "ww/slot-1".into(),
            holder: "dev/agent".into(),
            ..Default::default()
        })]);
        assert!(!f.all && !f.cold && f.lease, "{kind:?}");
        assert_eq!(f.holders, ["dev/agent"]);
    }
    // A mapped kind with no id to name is all too.
    assert!(signals::fold_board(&[board_ev(coord::TASK_CHANGED, "", "")]).all);
    assert!(signals::fold_board(&[board_ev(coord::NOTE_CHANGED, "", "")]).all);
    // Both Resync forms: the hub's and a board-set change's.
    assert!(signals::fold_board(&[CoreEvent::Resync]).all);
    let mut f = signals::fold_board(&[board_ev(coord::RESYNC, "", "")]);
    assert!(f.all);
    let c = r.w.expand(&mut ex, &mut f).unwrap();
    assert!(c.all && c.cards.is_empty());
}

/// More than 200 ids is `all` (§3.3).
#[test]
fn more_than_200_ids_is_all() {
    let r = Rig::new();
    let mut ex = Expander::default();
    let evs: Vec<CoreEvent> = (0..MAX_IDS)
        .map(|i| board_ev(coord::TASK_CHANGED, &format!("t{i}"), ""))
        .collect();
    let c = r.w.expand(&mut ex, &mut signals::fold_board(&evs)).unwrap();
    assert!(!c.all && c.cards.len() == MAX_IDS);
    let evs: Vec<CoreEvent> = (0..=MAX_IDS)
        .map(|i| board_ev(coord::TASK_CHANGED, &format!("t{i}"), ""))
        .collect();
    let c = r.w.expand(&mut ex, &mut signals::fold_board(&evs)).unwrap();
    assert!(c.all && c.cards.is_empty());
}

/// A card's parent and subtasks ride with it (one hop of family), and a
/// presence change that touches no card's row sends nothing.
#[test]
fn a_card_change_takes_its_family_and_a_quiet_presence_nothing() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let parent = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "p".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let kid = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "k".into(),
                parent: parent.id.clone(),
                ..Default::default()
            },
        )
        .unwrap();
    let mut ex = Expander::default();
    let c =
        r.w.expand(
            &mut ex,
            &mut signals::fold_board(&[board_ev(coord::TASK_CHANGED, &kid.id, "")]),
        )
        .unwrap();
    assert!(!c.all);
    assert!(
        c.cards.contains(&kid.id) && c.cards.contains(&parent.id),
        "{:?}",
        c.cards
    );
    // And the other way: the parent's change names the kid.
    let c =
        r.w.expand(
            &mut ex,
            &mut signals::fold_board(&[board_ev(coord::TASK_CHANGED, &parent.id, "")]),
        )
        .unwrap();
    assert!(c.cards.contains(&kid.id) && c.cards.contains(&parent.id));

    // An agent that holds nothing moved: no row to read.
    let card =
        r.w.core
            .local_agents()
            .into_iter()
            .find(|c| c.key == r.key_b)
            .unwrap();
    let pc = presence::Change {
        kind: presence::ChangeKind::Updated,
        card,
    };
    let mut f = signals::fold_board(&[CoreEvent::Presence(Box::new(pc.clone()))]);
    assert_eq!(f.agents, std::slice::from_ref(&r.key_b));
    assert!(
        r.w.expand(&mut ex, &mut f).is_none(),
        "nothing shows agent b"
    );
    // Once agent b holds the card, its presence change names the card.
    co.claim_task(&r.key_b, &parent.id).unwrap();
    r.w.expand(
        &mut ex,
        &mut signals::fold_board(&[board_ev(coord::TASK_CHANGED, &parent.id, "")]),
    );
    let mut f = signals::fold_board(&[CoreEvent::Presence(Box::new(pc))]);
    let c = r.w.expand(&mut ex, &mut f).unwrap();
    assert!(!c.all && c.cards == [parent.id.clone()], "{:?}", c.cards);
}

/// `seq` counts up per event name from 1, `boards_pin` included, whichever
/// thread sends; and a card change's payload names the card.
#[test]
fn seq_counts_up_per_name_including_boards_pin() {
    let r = Rig::new();
    let (seen, stop) = pumps(&r);
    let id = r.w.core.create_board("Alpha", "alpha").unwrap();
    wait("the board opens", || r.w.core.board_coord(&id).is_ok());
    for i in 0..6 {
        r.w.boards_pin(&id, i % 2 == 0).unwrap();
        if i == 2 {
            r.w.core
                .coord()
                .create_task(
                    &r.key_a,
                    coord::NewTask {
                        title: "t".into(),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
    }
    std::thread::sleep(Duration::from_millis(600));
    for name in [events::EVENT_BOARDS_CHANGED, events::EVENT_BOARD_CHANGED] {
        let seqs: Vec<u64> = seen
            .of(name)
            .iter()
            .map(|v| v["seq"].as_u64().unwrap())
            .collect();
        assert!(seqs.len() >= 2, "{name}: {seqs:?}");
        let want: Vec<u64> = (1..=seqs.len() as u64).collect();
        assert_eq!(seqs, want, "{name} counts 1, 2, 3 with no gap");
    }
    assert!(
        seen.of(events::EVENT_BOARDS_CHANGED).len() >= 6,
        "every pin sent boards:changed"
    );
    stop.cancel();
}

/// A board opening reaches the page as `all` on both signals (a board-set
/// `Resync`, list-signals.md §3.1), and a card change names its id.
#[test]
fn a_board_set_resync_reads_everything_and_a_card_names_itself() {
    let r = Rig::new();
    let (seen, stop) = pumps(&r);
    r.w.core.create_board("Alpha", "alpha").unwrap();
    wait("board:changed all", || {
        seen.of(events::EVENT_BOARD_CHANGED)
            .iter()
            .any(|v| v["all"] == true)
    });
    wait("boards:changed", || {
        !seen.of(events::EVENT_BOARDS_CHANGED).is_empty()
    });
    let all = seen
        .of(events::EVENT_BOARD_CHANGED)
        .into_iter()
        .find(|v| v["all"] == true)
        .unwrap();
    assert!(strs(&all["cards"]).is_empty() && strs(&all["notes"]).is_empty());

    let t =
        r.w.core
            .coord()
            .create_task(
                &r.key_a,
                coord::NewTask {
                    title: "x".into(),
                    ..Default::default()
                },
            )
            .unwrap();
    wait("the card's id", || {
        seen.of(events::EVENT_BOARD_CHANGED)
            .iter()
            .any(|v| v["all"] == false && strs(&v["cards"]).contains(&t.id))
    });
    stop.cancel();
}

/// Inbox: a delivery names itself and its card; a policy write (the
/// `POLICY_SET` shape: `Updated`, no `to`, no delivery), a mesh re-fold,
/// the inbox's own `Resync` and a kick are `all`.
#[test]
fn inbox_names_deliveries_and_cards_and_reads_all_for_the_rest() {
    let r = Rig::new();
    let (seen, stop) = pumps(&r);
    let t =
        r.w.core
            .coord()
            .create_task(
                &r.key_a,
                coord::NewTask {
                    title: "c".into(),
                    ..Default::default()
                },
            )
            .unwrap();
    let to = inbox::Address::new(inbox::addr::AGENT, &r.key_b);
    let (_, d) =
        r.w.core
            .inbox()
            .send_with(
                &agent(&r.key_a),
                &to,
                "see this",
                inbox::SendOptions {
                    ref_: format!("task:{}", t.id),
                    ..Default::default()
                },
            )
            .unwrap();
    wait("inbox:changed names the delivery and its card", || {
        seen.of(events::EVENT_INBOX_CHANGED).iter().any(|v| {
            v["all"] == false
                && strs(&v["deliveries"]).contains(&d.id)
                && strs(&v["cards"]).contains(&t.id)
                && v["unread"].is_i64()
        })
    });
    // A local policy write is POLICY_SET: all.
    let before = seen.of(events::EVENT_INBOX_CHANGED).len();
    r.w.inbox_set_paused(true).unwrap();
    wait("a policy write reads all", || {
        seen.of(events::EVENT_INBOX_CHANGED)
            .iter()
            .skip(before)
            .any(|v| v["all"] == true && strs(&v["deliveries"]).is_empty())
    });
    stop.cancel();

    // The shapes, for the writes a test can't make here (a remote
    // POLICY_SET, the mesh re-fold, a lost event, a kick).
    let ch = |kind, to: &str, did: &str| {
        CoreEvent::Inbox(inbox::Change {
            kind,
            to: to.into(),
            delivery_id: did.into(),
            created: false,
            was_to: None,
        })
    };
    let updated = inbox::ChangeKind::Updated;
    assert_eq!(
        signals::fold_inbox(&[ch(updated, "", "")], false),
        (true, vec![])
    );
    assert_eq!(
        signals::fold_inbox(&[ch(inbox::ChangeKind::Resync, "", "")], false),
        (true, vec![])
    );
    assert_eq!(
        signals::fold_inbox(&[CoreEvent::Resync], false),
        (true, vec![])
    );
    assert_eq!(signals::fold_inbox(&[], true), (true, vec![]));
    // A wake issued names a recipient and no delivery: nothing to read.
    assert_eq!(
        signals::fold_inbox(&[ch(inbox::ChangeKind::WakeReady, "k", "")], false),
        (false, vec![])
    );
    assert_eq!(
        signals::fold_inbox(
            &[
                ch(updated, "k", "d1"),
                ch(inbox::ChangeKind::HeldNew, "k", "d1")
            ],
            false
        ),
        (false, vec!["d1".to_string()])
    );
}

/// Talk: a post names its conversation; a thread's read cursor has
/// `thread:` stripped; the all rules (a store resync, `talk.card`, an
/// empty change, a lost event, a re-fold) read everything.
#[test]
fn talk_names_conversations_and_reads_all_for_the_rest() {
    let r = Rig::new();
    let (seen, stop) = pumps(&r);
    let ch = r.w.talk_channel_create("general", None, None).unwrap();
    r.w.talk_post(&ch, "hello", &[], false, None, &[]).unwrap();
    wait("talk:changed names the channel", || {
        seen.of(events::EVENT_TALK_CHANGED)
            .iter()
            .any(|v| v["all"] == false && strs(&v["convs"]).contains(&ch))
    });
    stop.cancel();

    let t = |c: TalkChange| CoreEvent::Talk(c);
    let (all, convs, calls) = signals::fold_talk(&[
        t(TalkChange::convs(["a", "thread:t9"])),
        t(TalkChange::of(["a", "b"], ["c1"])),
        t(TalkChange::convs(["t9"])),
    ]);
    assert!(!all);
    assert_eq!(convs, ["a", "t9", "b"], "bare thread ids, each once");
    assert_eq!(calls, ["c1"]);
    for ev in [
        t(TalkChange::all()),
        t(TalkChange::convs(Vec::<String>::new())),
        CoreEvent::Resync,
    ] {
        let (all, convs, calls) = signals::fold_talk(&[t(TalkChange::convs(["a"])), ev]);
        assert!(all && convs.is_empty() && calls.is_empty());
    }
}

/// Notifications: keys and ask ids by name; a store resync, an empty ask
/// id (asks lost or refolded), a change that names nothing and a lost
/// event read everything.
#[test]
fn notifications_name_keys_and_asks_and_read_all_for_the_rest() {
    let n = |k: NotifChange| CoreEvent::Notifications(k);
    let (all, keys, asks) = signals::fold_notifications(&[
        n(NotifChange::keys(["k1", "k2"])),
        n(NotifChange::keys(["k1"])),
        CoreEvent::Asks("a1".into()),
        CoreEvent::Asks("a1".into()),
    ]);
    assert!(!all);
    assert_eq!(
        (keys.as_slice(), asks.as_slice()),
        (
            &["k1".to_string(), "k2".to_string()][..],
            &["a1".to_string()][..]
        )
    );
    for ev in [
        n(NotifChange::all()),
        n(NotifChange::keys(Vec::<String>::new())),
        CoreEvent::Asks(String::new()),
        CoreEvent::Resync,
    ] {
        let (all, keys, asks) = signals::fold_notifications(&[
            n(NotifChange::keys(["k1"])),
            CoreEvent::Asks("a1".into()),
            ev,
        ]);
        assert!(all && keys.is_empty() && asks.is_empty());
    }
}

/// The debug switches only count in a debug build, and only a number.
#[test]
fn the_debug_switches_take_a_number() {
    use std::ffi::OsString;
    let v = |s: &str| signals::dev_switch_of(Some(OsString::from(s)));
    if cfg!(debug_assertions) {
        assert_eq!((v("3"), v(" 7 "), v("x"), v("")), (3, 7, 0, 0));
    } else {
        assert_eq!(v("3"), 0, "a release build ignores it");
    }
    assert_eq!(signals::dev_switch_of(None), 0);
}

/// A card a thread made takes its thread with it (the made-card index,
/// §3.3 step 1), and the thread's own change does not name the card.
#[test]
fn a_made_card_takes_the_thread_that_made_it() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let (th, _) = co
        .post_agent_thread(
            &r.key_a,
            coord::NewAsk {
                text: "Should we?".into(),
                thread: true,
                ..coord::NewAsk::default()
            },
        )
        .unwrap();
    let card = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "made".into(),
                from: th.id.clone(),
                ..Default::default()
            },
        )
        .unwrap();
    let mut ex = Expander::default();
    let c =
        r.w.expand(
            &mut ex,
            &mut signals::fold_board(&[board_ev(coord::TASK_CHANGED, &card.id, "")]),
        )
        .unwrap();
    assert!(!c.all);
    assert!(
        c.cards.contains(&card.id) && c.cards.contains(&th.id),
        "the made card names its thread: {:?}",
        c.cards
    );
}

/// Card #1149: a thread leaving the resolved lists (reopened) sends `all`:
/// the page keeps the newest few resolved per bucket, so it can't bring
/// back the one that was pushed out. Resolving one, or a comment on a
/// resolved one, names it instead.
#[test]
fn a_reopened_thread_sends_all() {
    let r = Rig::new();
    let w = &r.w;
    let t = w.start_thread("Just me", &[], &[], "").unwrap().task.id;
    let mut ex = Expander::default();
    let mut send = |id: &str| {
        w.expand(
            &mut ex,
            &mut signals::fold_board(&[board_ev(coord::TASK_CHANGED, id, "")]),
        )
    };
    // The first batch builds the index.
    let c = send(&t).unwrap();
    assert!(!c.all && c.cards.contains(&t));
    w.resolve_thread(&t).unwrap();
    let c = send(&t).unwrap();
    assert!(!c.all, "resolving names the thread: {c:?}");
    assert!(c.cards.contains(&t));
    let c = send(&t).unwrap();
    assert!(!c.all, "still resolved: {c:?}");
    w.reopen_thread(&t).unwrap();
    let c = send(&t).unwrap();
    assert!(c.all, "reopened: {c:?}");
    let c = send(&t).unwrap();
    assert!(!c.all, "only once: {c:?}");
}

/// A talking line ends and a card goes stale with no event: the window
/// keeps the earliest time and `due_cards` hands the ids over then.
#[test]
fn rows_that_change_by_the_clock_come_due() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "w".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap();
    co.add_comment(&r.key_a, &t.id, "hello", false).unwrap();
    let mut ex = Expander::default();
    r.w.expand(
        &mut ex,
        &mut signals::fold_board(&[board_ev(coord::TASK_CHANGED, &t.id, "")]),
    );
    let due = ex.next_due().expect("a working card turns stale later");
    let now = unix_ms();
    assert!(
        due > now && due <= now + 2 * 3600 * 1000 + 5000,
        "due {due} now {now}"
    );
    assert!(r.w.due_cards(&mut ex).is_empty(), "nothing is due yet");
}
