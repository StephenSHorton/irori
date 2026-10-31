//! The bridge across two people (v0.5 §7.1, §9, §10, §5.4; card #59): two
//! made-up people's Cores share one board, its rows copied between their
//! board logs as an invite bundle and a sync would move them (no network,
//! nothing opens a socket). Board asks, review requests and lease release
//! requests from one person reach the other's agents through the bridge,
//! under the receiving person's receive setting, once per row.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{Rig, wait_for};
use coord::RELEASE_REQUEST;
use coord::roster::AgentCard;
use inbox::{Delivery, ReceiveSetting, hold, mode, receive, row_kind};
use irori_core::Core;
use irori_identity::membership::BOARD_MEMBER;
use irori_identity::objects::ROLE_DESKTOP;
use store::kinds::{self, PERSON_ACTOR_PREFIX};
use store::{Filter, Store};

const PR: &str = "https://github.com/a/b/pull/7";

/// Two Cores per test: the tests here take turns, so their presence
/// watchers start one pair at a time.
static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn now() -> gocompat::time::GoTime {
    gocompat::time::GoTime::from_system(std::time::SystemTime::now())
}

/// Copies every row of `from` that `to` lacks, as a sync would.
fn copy(from: &Store, to: &Store, to_dev: &str) {
    let evs: Vec<store::Event> = from
        .list(&Filter {
            limit: 100_000,
            ..Default::default()
        })
        .unwrap()
        .into_iter()
        .filter(|e| e.device_id != to_dev)
        .collect();
    let (_, r) = to.insert_batch(&evs);
    r.unwrap();
}

struct Two {
    ra: Rig,
    rb: Rig,
    board: String,
    // Last, so both Cores are dropped before the next test's turn.
    _turn: std::sync::MutexGuard<'static, ()>,
}

impl Two {
    fn a(&self) -> &Core {
        &self.ra.core
    }
    fn b(&self) -> &Core {
        &self.rb.core
    }
    fn sa(&self) -> Arc<Store> {
        self.a().board_store(&self.board).unwrap()
    }
    fn sb(&self) -> Arc<Store> {
        self.b().board_store(&self.board).unwrap()
    }
    /// Both ways, twice, so rows that answer rows settle.
    fn sync(&self) {
        for _ in 0..2 {
            copy(&self.sa(), &self.sb(), self.b().device_id());
            copy(&self.sb(), &self.sa(), self.a().device_id());
        }
    }
    fn person(c: &Core) -> String {
        format!("{PERSON_ACTOR_PREFIX}{}", c.person().id)
    }
}

/// Alice (Core a) makes board "mlg" and adds Bob (Core b); Bob's Core
/// opens it from Alice's pin and certifies its own device.
fn two_people() -> Two {
    let turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let (ra, rb) = (Rig::new(), Rig::new());
    let (a, b) = (&ra.core, &rb.core);
    assert_ne!(a.person().id, b.person().id, "two people");
    let board = a.create_board("MLG farming", "mlg").unwrap();
    let alice = a.identity().board_pin();
    let bob = b.identity().board_pin();
    let added = a
        .identity()
        .board_member_added_row(alice.person_epoch, &bob, "bob", BOARD_MEMBER, now())
        .unwrap();
    a.board_store(&board)
        .unwrap()
        .append(&Two::person(a), kinds::BOARD_MEMBER_ADDED, &added)
        .unwrap();
    b.open_board(&board, Some(&alice)).unwrap();
    let t = Two {
        ra,
        rb,
        board,
        _turn: turn,
    };
    copy(&t.sa(), &t.sb(), t.b().device_id());
    let cert = t
        .b()
        .identity()
        .member_cert_for(
            bob.person_epoch,
            t.b().device_id(),
            &hex::encode(t.b().identity().device_pub()),
            ROLE_DESKTOP,
            now(),
        )
        .unwrap();
    t.sb()
        .append("irori", kinds::MEMBER_DEVICE_ADDED, &cert)
        .unwrap();
    t.sync();
    let v = t.a().board_verifier(&t.board).unwrap();
    assert_eq!(
        v.person_of(t.b().device_id()).as_deref(),
        Some(t.b().person().id.as_str()),
        "alice's Core knows bob's device"
    );
    t
}

/// The deliveries to `agent` that came over the bridge, held ones too.
fn bridged(c: &Core, agent: &str) -> Vec<Delivery> {
    c.inbox()
        .list(10_000)
        .into_iter()
        .filter(|d| d.to == agent && d.message.from.key.starts_with("board:"))
        .collect()
}

fn wait_bridged(c: &Core, agent: &str, n: usize) -> Vec<Delivery> {
    wait_for(10, "bridged deliveries", || bridged(c, agent).len() >= n);
    let ds = bridged(c, agent);
    assert_eq!(ds.len(), n, "{ds:?}");
    ds
}

/// Lets the followers run, then checks nothing more crossed.
fn settle_bridged(c: &Core, agent: &str, n: usize) {
    std::thread::sleep(Duration::from_millis(400));
    let ds = bridged(c, agent);
    assert_eq!(ds.len(), n, "{ds:?}");
}

fn idle(key: &str) -> AgentCard {
    AgentCard {
        key: key.into(),
        status: "idle".into(),
        ..Default::default()
    }
}

fn set_receive(c: &Core, board: &str, preset: &str) {
    c.inbox()
        .set_receive(
            ReceiveSetting {
                board_id: board.into(),
                preset: preset.into(),
                ..ReceiveSetting::default()
            },
            &format!("user:{}", c.device_id()),
        )
        .unwrap();
}

/// §9: Alice's ask on the board reaches Bob's free agents as a message from
/// Alice, woken when she asked for a wake, once per round however often
/// her sweep widens it.
#[test]
fn a_board_ask_reaches_the_other_persons_free_agents() {
    let t = two_people();
    let (b1, b2) = (
        t.b().board_key(&t.board, &t.rb.key_a).unwrap(),
        t.b().board_key(&t.board, &t.rb.key_b).unwrap(),
    );
    t.sync();
    let co = t.a().board_coord(&t.board).unwrap();
    let (ask, _) = co
        .post_ask(
            &Two::person(t.a()),
            coord::NewAsk {
                text: "Who can look at the flaky sync test?".into(),
                wake: true,
                ..coord::NewAsk::default()
            },
            &[idle(&b1)],
        )
        .unwrap();
    t.sync();
    let ds = wait_bridged(t.b(), &t.rb.key_a, 1);
    let d = &ds[0];
    let m = &d.message;
    assert_eq!(m.board.kind, row_kind::ASK);
    assert!(m.board.card_id.is_empty(), "from the person: {m:?}");
    assert_eq!(m.board.person_id, t.a().person().id);
    assert_eq!(m.ref_, format!("task:{}", ask.id));
    assert_eq!(d.mode, mode::WAKE, "she asked for a wake: {d:?}");
    assert!(
        m.text.contains("asks the room") && m.text.contains("ask_claim mlg#"),
        "{}",
        m.text
    );
    assert!(bridged(t.b(), &t.rb.key_b).is_empty(), "beta wasn't told");

    // The sweep tells beta too, in the same round: alpha hears it once.
    co.sweep_asks(&[idle(&b1), idle(&b2)]).unwrap();
    t.sync();
    wait_bridged(t.b(), &t.rb.key_b, 1);
    settle_bridged(t.b(), &t.rb.key_a, 1);
    // A copy of the same rows again changes nothing.
    t.sync();
    settle_bridged(t.b(), &t.rb.key_b, 1);
    // Alice's own agents never hear her ask over the bridge.
    assert!(bridged(t.a(), &t.ra.key_a).is_empty());
}

/// §10: Bob's agent asks for a review of a PR Alice's card links; without
/// `to` it reaches Alice's dedicated reviewer. §7.2: under Ask me it is
/// held for Alice (cross_person), as their agents' assignments are.
#[test]
fn a_review_request_reaches_the_dedicated_reviewer() {
    let t = two_people();
    let a1 = t.a().board_key(&t.board, &t.ra.key_a).unwrap();
    let a2 = t.a().board_key(&t.board, &t.ra.key_b).unwrap();
    let b1 = t.b().board_key(&t.board, &t.rb.key_a).unwrap();
    let co_a = t.a().board_coord(&t.board).unwrap();
    let card = co_a
        .create_task(
            &a1,
            coord::NewTask {
                title: "Fix the loader".into(),
                objective: "make it load".into(),
                shared: true,
                ..coord::NewTask::default()
            },
        )
        .unwrap();
    co_a.add_link(&a1, &card.id, PR).unwrap();
    co_a.set_reviewer(&Two::person(t.a()), &a2, "").unwrap();
    t.sync();
    let co_b = t.b().board_coord(&t.board).unwrap();
    wait_for(10, "alice's reviewer on bob's side", || {
        co_b.reviewer(&t.a().person().id, "") == a2
    });
    co_b.request_review(&b1, &card.id, PR, "").unwrap();
    t.sync();
    let ds = wait_bridged(t.a(), &t.ra.key_b, 1);
    let (d, m) = (&ds[0], &ds[0].message);
    assert_eq!(m.board.kind, row_kind::ASSIGN);
    assert!(m.board.card_id.len() > 1, "from bob's agent: {m:?}");
    assert!(
        m.from.name.starts_with("bob's agent") && m.text.contains(PR),
        "{m:?}"
    );
    assert_eq!(d.hold, "", "Open lets it through: {d:?}");
    assert!(
        bridged(t.a(), &t.ra.key_a).is_empty(),
        "only the reviewer hears it"
    );

    // Under Ask me a request naming alpha is held for Alice.
    set_receive(t.a(), &t.board, receive::ASK);
    co_b.request_review(&b1, &card.id, PR, &a1).unwrap();
    t.sync();
    let ds = wait_bridged(t.a(), &t.ra.key_a, 1);
    assert_eq!(ds[0].hold, hold::CROSS_PERSON, "{:?}", ds[0]);
}

/// §5.4: Bob's agent asks Alice's agent to release a lease, on the shared
/// card the lease mirrors through. It reaches the holder alone, held
/// (member_lease) under Ask me and let through under Open.
#[test]
fn a_release_request_reaches_the_lease_holder() {
    let t = two_people();
    let a1 = t.a().board_key(&t.board, &t.ra.key_a).unwrap();
    let a2 = t.a().board_key(&t.board, &t.ra.key_b).unwrap();
    let b1 = t.b().board_key(&t.board, &t.rb.key_a).unwrap();
    let co_a = t.a().board_coord(&t.board).unwrap();
    let card = co_a
        .create_task(
            &a1,
            coord::NewTask {
                title: "Port the loader".into(),
                objective: "port it".into(),
                shared: true,
                ..coord::NewTask::default()
            },
        )
        .unwrap();
    co_a.watch(&a2, &card.id).unwrap();
    t.sync();
    let co_b = t.b().board_coord(&t.board).unwrap();
    wait_for(10, "the card on bob's side", || {
        co_b.get_task(&card.id).is_some()
    });
    // What coord's request_release writes: a comment addressed to the
    // holder (its lease and mirror are coord's own tests).
    let row = |n: u32| {
        serde_json::json!({
            "taskId": card.id,
            "commentId": format!("01RELEASE{n:017}"),
            "by": b1,
            "text": format!("{RELEASE_REQUEST}: {b1} asks {a1} to release src/loader.rs. Release it with the release tool when you can."),
            "wake": false,
            "at": now(),
            "to": a1,
        })
    };
    set_receive(t.a(), &t.board, receive::ASK);
    t.sb().append(&b1, kinds::TASK_COMMENT, &row(1)).unwrap();
    t.sync();
    let ds = wait_bridged(t.a(), &t.ra.key_a, 1);
    assert_eq!(ds[0].message.board.kind, row_kind::LEASE);
    assert_eq!(ds[0].hold, hold::MEMBER_LEASE, "{:?}", ds[0]);
    settle_bridged(t.a(), &t.ra.key_b, 0); // the watcher isn't asked

    set_receive(t.a(), &t.board, receive::OPEN);
    t.sb().append(&b1, kinds::TASK_COMMENT, &row(2)).unwrap();
    t.sync();
    let ds = wait_bridged(t.a(), &t.ra.key_a, 2);
    assert!(ds.iter().any(|d| d.hold.is_empty()), "{ds:?}");
    settle_bridged(t.a(), &t.ra.key_b, 0);
}
