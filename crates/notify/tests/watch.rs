//! I:internal/notify/watch_test.go (the Board and ask cards, and the inbox
//! half through the real loop), ask_test.go and owner_test.go. The inbox
//! half's own logic tests are crates/inbox/tests/notify.rs.

mod common;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, SystemTime};

use common::*;
use coord::roster::{AgentCard as CoordCard, Agents};
use coord::{BoardView, Card, Coord, NewAsk, NewTask, Returned, Task};
use gocompat::time::GoTime;
use inbox::{Address, Inbox, Policy, Sender, addr, policy_mode, st};
use notify::watch::BoardCards;
use notify::{
    Notifier, Sources, TAB_BOARD, TAB_HELD, Target, WatchHandle, ask_id, held_id, needs_id,
    returned_id, unclaimed_id, watch,
};
use store::Store;

const KEY_A: &str = "dev1/aaaaaaaa-1111-4111-8111-111111111111";
const KEY_B: &str = "dev1/bbbbbbbb-2222-4222-8222-222222222222";
const USER: &str = "user:dev1";

/// Both services' roster: key -> name.
#[derive(Default)]
struct Roster(Mutex<BTreeMap<String, String>>);

impl Roster {
    fn new() -> Arc<Roster> {
        let r = Roster::default();
        r.0.lock().unwrap().insert(KEY_A.into(), "alpha".into());
        r.0.lock().unwrap().insert(KEY_B.into(), "beta".into());
        Arc::new(r)
    }
    fn drop_key(&self, k: &str) {
        self.0.lock().unwrap().remove(k);
    }
}

fn session(key: &str) -> String {
    key.split_once('/')
        .map(|(_, s)| s.to_string())
        .unwrap_or_default()
}

impl inbox::Roster for Roster {
    fn agents(&self) -> Vec<inbox::AgentCard> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|(k, n)| inbox::AgentCard {
                key: k.clone(),
                device_id: "dev1".into(),
                session_id: session(k),
                agent: "claude".into(),
                name: n.clone(),
                status: "idle".into(),
                ..Default::default()
            })
            .collect()
    }
}

impl Agents for Roster {
    fn agents(&self) -> Vec<CoordCard> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|(k, n)| CoordCard {
                key: k.clone(),
                device_id: "dev1".into(),
                session_id: session(k),
                agent: "claude".into(),
                name: n.clone(),
                status: "idle".into(),
                ..Default::default()
            })
            .collect()
    }
}

fn open_store(dir: &std::path::Path) -> Arc<Store> {
    let clock = hlc::Clock::new();
    Arc::new(Store::open(dir.join("irori.db"), "dev1", Arc::new(clock)).unwrap())
}

fn alpha() -> Sender {
    Sender::new(KEY_A, "alpha", "claude")
}

struct Rig {
    _dir: tempfile::TempDir,
    st: Arc<Store>,
    ib: Arc<Inbox>,
    co: Arc<Coord>,
    ros: Arc<Roster>,
    fake: Arc<Fake>,
    n: Notifier,
    opened: mpsc::Receiver<Target>,
    handle: Option<WatchHandle>,
}

type Wrap = Arc<dyn Fn(&str, &str) -> String + Send + Sync>;

fn rig_with(wrap: Option<Wrap>, opts: coord::Options) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let st = open_store(dir.path());
    let ros = Roster::new();
    let ib = Arc::new(
        Inbox::new(
            st.clone(),
            ros.clone(),
            inbox::Options {
                device_id: "dev1".into(),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let co = Arc::new(Coord::new(st.clone(), Some(ros.clone()), opts).unwrap());
    let fake = Fake::new();
    let n = notifier(&fake);
    let (tx, opened) = mpsc::channel();
    let tx = Mutex::new(tx);
    n.set_opener(move |t| {
        let _ = tx.lock().unwrap().send(t);
    });
    let names: BTreeMap<String, String> = [(KEY_A, "alpha"), (KEY_B, "beta")]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let name: inbox::watch::NameFn = Arc::new(move |k: &str| {
        let base = names.get(k).cloned().unwrap_or_default();
        match &wrap {
            Some(w) => w(k, &base),
            None => base,
        }
    });
    let handle = watch(
        &n,
        Sources {
            inbox: Some(ib.clone()),
            board: Some(co.clone()),
            user: USER.into(),
            name: Some(name),
            room: Some(co.clone()),
            ..Sources::default()
        },
    );
    // Watch subscribes before it reads; let it settle so a test's first
    // change isn't sent before anyone listens.
    std::thread::sleep(Duration::from_millis(50));
    Rig {
        _dir: dir,
        st,
        ib,
        co,
        ros,
        fake,
        n,
        opened,
        handle,
    }
}

fn rig() -> Rig {
    rig_with(None, coord::Options::default())
}

impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            h.stop();
        }
    }
}

impl Rig {
    fn held_message(&self, text: &str) -> inbox::Delivery {
        self.ib
            .set_policy(
                Policy {
                    mode: policy_mode::EVERYONE.into(),
                    block: vec![KEY_A.into()],
                    ..Policy::default()
                },
                USER,
            )
            .unwrap();
        let (_, d) = self
            .ib
            .send(&alpha(), &Address::new(addr::AGENT, KEY_B), text, "")
            .unwrap();
        assert_eq!(d.state, st::HELD);
        d
    }

    fn block_on_person(&self, by: &str, id: &str, reason: &str) {
        self.co
            .block(
                by,
                id,
                reason,
                Some(coord::BlockedBy {
                    kind: coord::BLOCKED_ON_PERSON.into(),
                    ..Default::default()
                }),
            )
            .unwrap();
    }

    fn opened(&self) -> Target {
        self.opened
            .recv_timeout(Duration::from_secs(2))
            .expect("nothing opened")
    }
}

fn new_task(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

// ---- held and message cards through the loop ----

#[test]
fn watch_held_release_acts_as_person() {
    let r = rig();
    let d = r.held_message("please run the migration");
    let id = held_id(KEY_A);
    r.fake.wait_post(&id);
    let c = r.fake.card(&id);
    assert_eq!(c.title, "alpha sent beta a message Irori held");
    assert!(
        c.body
            .starts_with("The sender is on your block list: please run")
    );
    assert!(c.sticky && c.kind == toast::Kind::Warn);
    r.fake.press(&id, "Release");
    wait_for("the release", || {
        r.ib.get(&d.id).is_some_and(|x| x.state != st::HELD)
    });
    let got = r.ib.get(&d.id).unwrap();
    assert_eq!(got.decided_by, USER, "released as the person");
    r.fake.wait_gone(&id);
}

#[test]
fn watch_held_opens_the_held_list() {
    let r = rig();
    r.held_message("x");
    let id = held_id(KEY_A);
    r.fake.wait_post(&id);
    r.fake.click(&id);
    let t = r.opened();
    assert!(t.tab == TAB_HELD && t.ref_ == KEY_A);
}

#[test]
fn watch_message_to_person() {
    let r = rig();
    let person = Sender::new(USER, "you", "user");
    let (m, _) =
        r.ib.send(
            &person,
            &Address::new(addr::AGENT, KEY_A),
            "how is it going?",
            "",
        )
        .unwrap();
    let (_, back) =
        r.ib.send(
            &alpha(),
            &Address::new(addr::REPLY, &m.id),
            "done, tests pass",
            "",
        )
        .unwrap();
    assert_eq!(back.to, USER, "the reply goes to the person");
    let id = notify::message_id(&back.id);
    r.fake.wait_post(&id);
    let c = r.fake.card(&id);
    assert_eq!(c.title, "alpha sent you a message");
    assert_eq!(c.body, "done, tests pass");
    let labels: Vec<&str> = c.actions.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(labels, vec!["Open"]);
    r.fake.click(&id);
    let t = r.opened();
    assert!(t.tab == "inbox" && t.ref_ == back.id);
}

/// The inbox drops changes for a reader whose buffer is full and never
/// says so; the watcher re-reads, so a dismiss it didn't hear still takes
/// the delivery off the card, and a hold it didn't hear still lands on it.
#[test]
fn watch_catches_up_after_dropped_changes() {
    let stall = Arc::new(AtomicBool::new(false));
    let (stalled_tx, stalled) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let stalled_tx = Mutex::new(stalled_tx);
    let s2 = stall.clone();
    let wrap: Wrap = Arc::new(move |_k: &str, name: &str| {
        if s2.swap(false, Ordering::SeqCst) {
            let _ = stalled_tx.lock().unwrap().send(());
            let _ = release_rx.lock().unwrap().recv();
        }
        name.to_string()
    });
    let r = rig_with(Some(wrap), coord::Options::default());
    let d = r.held_message("first");
    let id = held_id(KEY_A);
    r.fake.wait_post(&id);

    stall.store(true, Ordering::SeqCst);
    r.held_message("second"); // the watcher stalls building its card
    stalled
        .recv_timeout(Duration::from_secs(5))
        .expect("the watcher never stalled");
    for _ in 0..80 {
        // more changes than the 64-slot buffer
        r.ib.set_policy(
            Policy {
                mode: policy_mode::EVERYONE.into(),
                block: vec![KEY_A.into()],
                ..Policy::default()
            },
            USER,
        )
        .unwrap();
    }
    r.ib.dismiss(&d.id, USER).unwrap();
    let (_, d3) =
        r.ib.send(&alpha(), &Address::new(addr::AGENT, KEY_B), "third", "")
            .unwrap();
    assert_eq!(d3.state, st::HELD);
    release_tx.send(()).unwrap();

    let c = r
        .fake
        .wait_title(&id, "alpha sent beta 2 messages Irori held");
    assert!(c.body.contains("third"), "{c:?}");
}

// ---- the Board ----

#[test]
fn watch_board_needs_you() {
    let r = rig();
    let task =
        r.co.create_task(KEY_A, new_task("ship the release"))
            .unwrap();
    r.co.claim_task(KEY_A, &task.id).unwrap();
    r.fake.no_post(Duration::from_millis(100)); // working, not on the person
    r.block_on_person(KEY_A, &task.id, "which version number?");
    let id = needs_id(&task.id);
    r.fake.wait_post(&id);
    let c = r.fake.card(&id);
    assert_eq!(c.title, "alpha is blocked on you (#1)");
    assert!(c.body.starts_with("which version number?"), "{:?}", c.body);
    let labels: Vec<&str> = c.actions.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(labels, vec!["Answer", "Later"]);
    r.fake.press(&id, "Answer");
    let t = r.opened();
    assert!(t.tab == TAB_BOARD && t.ref_ == format!("task:{}", task.id));
    // A comment while it is still blocked doesn't pop it up again.
    r.co.add_comment(KEY_B, &task.id, "I think 0.4", false)
        .unwrap();
    r.fake.no_post(Duration::from_millis(100));

    // A new reason while still in the lane: no repost.
    r.block_on_person(KEY_A, &task.id, "still need the number");
    r.fake.no_post(Duration::from_millis(80));
    // Posted again here to see it go: unblocked clears it.
    r.n.post(notify::Note {
        id: id.clone(),
        title: "up".into(),
        ..notify::Note::default()
    });
    r.fake.wait_post(&id);
    r.co.unblock(USER, &task.id, "0.4").unwrap();
    r.fake.wait_gone(&id);

    // Blocked on the person again later: a new card.
    r.block_on_person(KEY_A, &task.id, "and the date?");
    r.fake.wait_post(&id);
}

#[test]
fn watch_seeds_quietly() {
    // A card already waiting when the app starts doesn't pop up.
    let dir = tempfile::tempdir().unwrap();
    let st = open_store(dir.path());
    let ros = Roster::new();
    let co = Arc::new(Coord::new(st.clone(), Some(ros), coord::Options::default()).unwrap());
    let task = co.create_task(KEY_A, new_task("t")).unwrap();
    co.block(
        KEY_A,
        &task.id,
        "r",
        Some(coord::BlockedBy {
            kind: coord::BLOCKED_ON_PERSON.into(),
            ..Default::default()
        }),
    )
    .unwrap();
    let f = Fake::new();
    let n = notifier(&f);
    let h = watch(
        &n,
        Sources {
            board: Some(co.clone()),
            user: USER.into(),
            ..Sources::default()
        },
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    h.stop();
    assert_eq!(
        f.post_count(),
        0,
        "a card already waiting popped up at start"
    );
}

#[test]
fn watch_headless_returns() {
    assert!(watch(&Notifier::headless(), Sources::default()).is_none());
}

#[test]
fn watch_board_ignores_the_persons_own_moves() {
    let r = rig();
    // (a) The person takes a card an agent made.
    let task = r.co.create_task(KEY_A, new_task("agent work")).unwrap();
    r.co.claim_task(USER, &task.id).unwrap();
    // (b) The person's own card, blocked on them by them.
    let own = r.co.create_task(USER, new_task("renew the cert")).unwrap();
    r.block_on_person(USER, &own.id, "waiting on vendor");
    let v = r.co.board(&coord::BoardFilter {
        viewer: USER.into(),
        ..Default::default()
    });
    assert!(v.needs_you.len() >= 2, "both cards in Needs you");
    r.fake.no_post(Duration::from_millis(150));

    // An agent handing the person a card still pops one.
    let handed = r.co.create_task(KEY_A, new_task("review the PR")).unwrap();
    r.co.assign_task(KEY_A, &handed.id, USER).unwrap();
    r.fake.wait_post(&needs_id(&handed.id));
    assert_eq!(
        r.fake.card(&needs_id(&handed.id)).title,
        "alpha handed you a card (#3)"
    );
}

#[test]
fn watch_unclaimed_ask() {
    let r = rig();
    let (ask, _) =
        r.co.post_ask(
            USER,
            NewAsk {
                text: "who can look at the release notes?".into(),
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap();
    let id = unclaimed_id(&ask.id);
    r.fake.wait_post(&id);
    let c = r.fake.card(&id);
    assert!(c.title.contains("Nobody took") && c.title.contains(&format!("#{}", ask.short)));
    let labels: Vec<&str> = c.actions.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(labels, vec!["Wake them", "Close"]);
    r.fake.press(&id, "Wake them");
    wait_for("Wake them writes a reopen request", || {
        r.st.list(&store::Filter {
            kinds: vec!["take.requested".into()],
            ..Default::default()
        })
        .unwrap()
        .len()
            == 1
    });
    // Nobody is free: the owner's sweep opens it again, still unclaimed,
    // and the card comes back to say so.
    r.co.sweep_asks(&[]).unwrap();
    r.fake.wait_post(&id);
    assert!(r.fake.card(&id).body.contains("You woke them"));
    r.co.ask_claim(KEY_A, &ask.id, Duration::from_secs(1))
        .unwrap();
    r.fake.wait_gone(&id);

    let (two, _) =
        r.co.post_ask(
            USER,
            NewAsk {
                text: "anyone?".into(),
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap();
    r.fake.wait_post(&unclaimed_id(&two.id));
    r.fake.press(&unclaimed_id(&two.id), "Close");
    r.fake.wait_gone(&unclaimed_id(&two.id));
    assert_eq!(r.co.get_task(&two.id).unwrap().state, coord::CANCELED);
}

#[test]
fn watch_returned_card() {
    let r = rig_with(
        None,
        coord::Options {
            holder_grace: Duration::from_millis(20),
            ..Default::default()
        },
    );
    let tk = r.co.create_task(USER, new_task("Ship the docs")).unwrap();
    r.co.claim_task(KEY_A, &tk.id).unwrap();
    r.ros.drop_key(KEY_A);
    let id = returned_id(&tk.id);
    wait_for("the gone holder is taken off", || {
        r.co.sweep_notices().unwrap();
        r.co.get_task(&tk.id).unwrap().returned.is_some()
    });
    r.fake.wait_post(&id);
    let c = r.fake.card(&id);
    assert!(c.title.contains("Nobody is on"), "{c:?}");
    assert!(c.body.contains("alpha went away"), "{c:?}");
    r.co.claim_task(KEY_B, &tk.id).unwrap();
    r.fake.wait_gone(&id);
}

/// The clock coord stamps rows with is coarse: a Wake them answered in the
/// tick the ask was posted in opens it again with the same since, and the
/// card must still come back.
#[test]
fn watch_unclaimed_ask_woken_in_one_tick() {
    let tick = GoTime::from_system(SystemTime::now()).utc();
    let r = rig_with(
        None,
        coord::Options {
            now: Some(Arc::new(move || tick)),
            ..Default::default()
        },
    );
    let (ask, _) =
        r.co.post_ask(
            USER,
            NewAsk {
                text: "who can look at the release notes?".into(),
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap();
    let id = unclaimed_id(&ask.id);
    r.fake.wait_post(&id);
    r.fake.press(&id, "Wake them");
    wait_for("the reopen request", || {
        r.st.list(&store::Filter {
            kinds: vec!["take.requested".into()],
            ..Default::default()
        })
        .unwrap()
        .len()
            == 1
    });
    r.co.sweep_asks(&[]).unwrap();
    let got = r.co.get_task(&ask.id).unwrap().ask.unwrap();
    assert!(got.reason == coord::TAKE_REOPENED && got.since.equal(&tick));
    r.fake.wait_post(&id);
    assert!(r.fake.card(&id).body.contains("You woke them"));
}

/// A card returned again within one clock step carries the same time: the
/// card still comes back, because it is keyed on the row (card #25).
#[test]
fn watch_returned_again_in_one_tick() {
    let f = Fake::new();
    let n = notifier(&f);
    let mut w = BoardCards::new(
        &n,
        &Sources {
            name: Some(Arc::new(|k: &str| k.to_string())),
            ..Sources::default()
        },
    );
    let tick = GoTime::from_system(SystemTime::now());
    let view = |row: &str| BoardView {
        open: vec![Card {
            task: Task {
                id: "t1".into(),
                title: "Ship it".into(),
                returned: Some(Returned {
                    was: KEY_A.into(),
                    at: tick,
                    person: true,
                    row: row.into(),
                }),
                ..Task::default()
            },
            ..Card::default()
        }],
        ..BoardView::default()
    };
    w.returned(&view("dev1:5"), true);
    w.returned(&view("dev1:5"), true); // the same return: no second card
    assert_eq!(f.drain(), 1, "one return, one card");
    w.returned(&view("dev1:9"), true); // returned again, same at
    assert_eq!(f.drain(), 1, "a return in the same tick posts again");
}

// ---- owner_test.go ----

/// The person is one person on every device: a card they moved from
/// another device is not news to them; a made-up `user:` key is not them.
#[test]
fn watcher_owner_set() {
    let (other, fake) = ("user:dev2", "user:dev-z");
    let n = Notifier::headless();
    let w = BoardCards::new(
        &n,
        &Sources {
            user: USER.into(),
            is_owner: Some(Arc::new(move |k: &str| k == USER || k == other)),
            ..Sources::default()
        },
    );
    let t = |updated: &str, holders: &[&str], created: &str| Task {
        updated_by: updated.into(),
        holders: holders.iter().map(|s| s.to_string()).collect(),
        created_by: created.into(),
        ..Task::default()
    };
    assert!(
        !w.agent_moved(&t(other, &[other], other)),
        "moved by the person elsewhere"
    );
    assert_eq!(w.needs_key(&t(KEY_A, &[other, KEY_A], KEY_A)), KEY_A);
    assert_eq!(
        w.needs_key(&t(KEY_A, &[fake], USER)),
        fake,
        "a made-up key is someone else"
    );
    let w0 = BoardCards::new(
        &n,
        &Sources {
            user: USER.into(),
            ..Sources::default()
        },
    );
    assert!(
        w0.agent_moved(&t(KEY_A, &[other], KEY_A)),
        "nil is_owner: only user counts"
    );
}

// ---- ask_test.go ----

/// An open ask gets a card with its buttons; pressing one answers it as the
/// person, and the card clears once the ask resolves. One waiting from
/// before Watch starts gets its card too.
#[test]
fn ask_cards_answer_and_clear() {
    let dir = tempfile::tempdir().unwrap();
    let st = open_store(dir.path());
    let book =
        coord::approvals::Book::start(coord::approvals::Options::new(st.clone(), "dev1")).unwrap();
    let (rtx, resolved) = mpsc::channel::<coord::approvals::Ask>();
    let rtx = Mutex::new(rtx);
    book.on_resolved(move |a| {
        let _ = rtx.lock().unwrap().send(a.clone());
    });
    let until = GoTime::from_system(SystemTime::now() + Duration::from_secs(60));
    let early = book
        .post(coord::approvals::Ask {
            by: KEY_A.into(),
            name: "alpha".into(),
            title: "Deploy?".into(),
            text: "Ship it now?".into(),
            actions: vec!["Yes".into(), "No".into()],
            until,
            ..Default::default()
        })
        .unwrap();
    let f = Fake::new();
    let n = notifier(&f);
    let h = watch(
        &n,
        Sources {
            asks: Some(book.clone()),
            user: USER.into(),
            ..Sources::default()
        },
    )
    .unwrap();
    let id = ask_id(&early.id);
    wait_for("the waiting ask's card", || f.is_live(&id));
    let c = f.card(&id);
    assert_eq!(c.title, "alpha: Deploy?");
    assert_eq!(c.actions.len(), 2);
    assert!(
        !c.sticky && c.expire > Duration::from_secs(50),
        "it lives until the ask's end"
    );
    f.press(&id, "No");
    let a = resolved
        .recv_timeout(Duration::from_secs(5))
        .expect("no resolve");
    assert!(a.id == early.id && a.choice == "No" && a.answered, "{a:?}");
    wait_for("the card clears", || !f.is_live(&id));

    // A dismiss is an answer with no choice.
    let late = book
        .post(coord::approvals::Ask {
            by: KEY_B.into(),
            text: "Later?".into(),
            actions: vec!["Ok".into()],
            until,
            ..Default::default()
        })
        .unwrap();
    let lid = ask_id(&late.id);
    wait_for("the new ask's card", || f.is_live(&lid));
    assert_eq!(f.card(&lid).title, KEY_B, "no name: the key");
    f.dismiss(&lid);
    let a = resolved
        .recv_timeout(Duration::from_secs(5))
        .expect("no resolve");
    assert!(
        a.id == late.id && a.choice.is_empty() && a.answered,
        "{a:?}"
    );
    h.stop();
}
