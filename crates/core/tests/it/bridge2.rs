//! The bridge across two people (v0.5 §7.1, §9, §10, §5.4; card #59): two
//! made-up people's Cores share one board, its rows copied between their
//! board logs as an invite bundle and a sync would move them (no network,
//! nothing opens a socket). Board asks, review requests and lease release
//! requests from one person reach the other's agents through the bridge,
//! under the receiving person's receive setting, once per row.

use crate::common;

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
    two_people_with(None)
}

/// [`two_people`] with `notes` as Bob's desktop cards.
fn two_people_with(notes: Option<notify::Notifier>) -> Two {
    two_people_opts(Rig::new(), |o| o.notifier = notes)
}

/// [`two_people`] with `ra` as Alice's rig and `edit` applied to Bob's
/// Core options.
fn two_people_opts(ra: Rig, edit: impl FnOnce(&mut irori_core::Options)) -> Two {
    let turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let rb = Rig::with(edit);
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

/// Waits until `c`'s roster for `board` lists each of `keys` idle and not
/// offline: the board's follower has folded the agent.card rows a sync
/// copied in.
fn wait_cards(c: &Core, board: &str, keys: &[&str]) {
    wait_for(10, "the other person's cards", || {
        let r = c.board_roster(board);
        keys.iter().all(|k| {
            r.iter()
                .any(|a| a.key == *k && a.status == "idle" && a.device_state != "offline")
        })
    });
}

fn idle(key: &str) -> AgentCard {
    AgentCard {
        key: key.into(),
        status: "idle".into(),
        ..Default::default()
    }
}

/// Idle agents, the first idle longest: the hand-out picks in this order.
fn idle_in_order(keys: &[&str]) -> Vec<AgentCard> {
    keys.iter()
        .enumerate()
        .map(|(i, k)| AgentCard {
            updated_at: gocompat::time::GoTime::from_system(
                std::time::UNIX_EPOCH + Duration::from_secs(1_000 + i as u64),
            ),
            ..idle(k)
        })
        .collect()
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

/// §9, #91: Alice's ask on the board is handed to one of Bob's idle agents,
/// as a message from Alice, woken; it moves on to the next only when that
/// one lets it go or goes away, each hand-out told once.
#[test]
fn a_board_ask_reaches_the_other_persons_free_agents() {
    let t = two_people();
    let (b1, b2) = (
        t.b().board_key(&t.board, &t.rb.key_a).unwrap(),
        t.b().board_key(&t.board, &t.rb.key_b).unwrap(),
    );
    t.sync();
    // Alice's Core hands asks out on its own too, from the cards its board
    // has folded: until it folds Bob's, his agents look gone to it, and
    // it would move the ask off alpha before the test does.
    wait_cards(t.a(), &t.board, &[&b1, &b2]);
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
        m.text.contains("asks the room")
            && m.text.contains("Irori handed it to you")
            && m.text.contains("board_leave mlg#"),
        "{}",
        m.text
    );
    assert!(bridged(t.b(), &t.rb.key_b).is_empty(), "beta wasn't told");

    // Alpha holds it: beta going idle hears nothing.
    co.sweep_asks(&[idle(&b1), idle(&b2)]).unwrap();
    t.sync();
    settle_bridged(t.b(), &t.rb.key_b, 0);
    // Alpha went away: it moves on to beta, and alpha heard it once.
    co.sweep_asks(&[idle(&b2)]).unwrap();
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

/// Records the cards a Notifier posts.
#[derive(Default)]
struct Cards(std::sync::Mutex<Vec<toast::Note>>);

impl notify::Toaster for Cards {
    fn configure(&self, _: toast::Config) {}
    fn set_options(&self, _: toast::Options) {}
    fn post(&self, n: toast::Note) {
        self.0.lock().unwrap().push(n);
    }
    fn close(&self, _: &str) {}
}

impl Cards {
    fn mentions(&self) -> Vec<(String, String, String)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|n| n.id.starts_with("mention:"))
            .map(|n| (n.id.clone(), n.title.clone(), n.body.clone()))
            .collect()
    }
}

/// Card #177: Alice mentions Bob and one of his agents in a comment on a
/// shared card. Bob's device shows Bob a card that opens the comment; his
/// agent is woken by the bridge, under his receive setting, and hears the
/// mention once, not the plain comment too. His other agent, a watcher,
/// hears the comment as before.
#[test]
fn a_mention_reaches_the_other_person_and_their_agent() {
    let cards = Arc::new(Cards::default());
    let notes = notify::Notifier::new(notify::Config {
        toaster: Some(cards.clone()),
        ..notify::Config::default()
    });
    let opened = Arc::new(std::sync::Mutex::new(Vec::<notify::Target>::new()));
    {
        let opened = opened.clone();
        notes.set_opener(move |t| opened.lock().unwrap().push(t));
    }
    let t = two_people_with(Some(notes));
    let a1 = t.a().board_key(&t.board, &t.ra.key_a).unwrap();
    let b1 = t.b().board_key(&t.board, &t.rb.key_a).unwrap();
    let b2 = t.b().board_key(&t.board, &t.rb.key_b).unwrap();
    let bob = Two::person(t.b());
    // Alice's Core lists Bob as a person she can mention there, not herself.
    let people = t.a().board_people();
    assert!(
        people
            .iter()
            .any(|p| p.key == bob && p.board_id == t.board && !p.name.is_empty()),
        "{people:?}"
    );
    assert!(
        !people.iter().any(|p| p.key == Two::person(t.a())),
        "{people:?}"
    );
    let co_a = t.a().board_coord(&t.board).unwrap();
    let card = co_a
        .create_task(
            &a1,
            coord::NewTask {
                title: "Tune the harvest loop".into(),
                objective: "make it faster".into(),
                shared: true,
                ..coord::NewTask::default()
            },
        )
        .unwrap();
    t.sync();
    let co_b = t.b().board_coord(&t.board).unwrap();
    wait_for(10, "the card on bob's side", || {
        co_b.get_task(&card.id).is_some()
    });
    co_b.watch(&b2, &card.id).unwrap();
    t.sync();
    wait_for(10, "bob's watcher on alice's side", || {
        co_a.get_task(&card.id)
            .is_some_and(|c| c.watchers.contains(&b2))
    });

    let m = |key: &str, name: &str| coord::Mention {
        key: key.into(),
        name: name.into(),
        gone: false,
    };
    let (cm, ns) = co_a
        .person_comment(
            &Two::person(t.a()),
            &card.id,
            "@bob @alpha can you look at the loop?",
            false,
            &[m(&bob, "bob"), m(&b1, "alpha")],
            None,
        )
        .unwrap();
    // On Alice's side, nothing for Bob's keys is routed to her own inbox.
    let routed: Vec<String> = ns
        .iter()
        .map(|n| t.a().private_notice(n.clone()))
        .flat_map(|n| n.to)
        .collect();
    assert!(!routed.iter().any(|k| coord::board_form(k)), "{routed:?}");
    t.sync();

    // Bob's agent: one woken mention, not the comment too.
    let ds = wait_bridged(t.b(), &t.rb.key_a, 1);
    let (d, msg) = (&ds[0], &ds[0].message);
    assert_eq!(d.mode, mode::WAKE, "{d:?}");
    assert_eq!(d.hold, "", "Open wakes it: {d:?}");
    assert_eq!(msg.board.kind, row_kind::COMMENT);
    assert_eq!(msg.ref_, format!("task:{}", card.id));
    assert!(
        msg.text.contains("mentioned you on") && msg.text.contains("can you look at the loop?"),
        "{}",
        msg.text
    );
    settle_bridged(t.b(), &t.rb.key_a, 1);
    // Bob's watcher hears the comment as it would have before.
    let ws = wait_bridged(t.b(), &t.rb.key_b, 1);
    assert!(!ws[0].message.text.contains("mentioned you"), "{ws:?}");

    // Bob: a card that opens the card at the comment.
    wait_for(10, "bob's mention card", || !cards.mentions().is_empty());
    let got = cards.mentions();
    assert_eq!(got.len(), 1, "{got:?}");
    let (id, title, body) = &got[0];
    assert!(
        title.contains("mentioned you on") && title.contains("Tune the harvest loop"),
        "{title}"
    );
    assert!(body.contains("can you look at the loop?"), "{body}");
    let n = cards
        .0
        .lock()
        .unwrap()
        .iter()
        .find(|n| &n.id == id)
        .cloned()
        .unwrap();
    (n.on_click.as_ref().expect("it opens"))();
    let opened = opened.lock().unwrap().clone();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].tab, notify::TAB_BOARD);
    assert_eq!(
        opened[0].ref_,
        format!("task:{}/comment:{}", card.id, cm.id)
    );
    // A copy of the same rows again changes nothing.
    t.sync();
    settle_bridged(t.b(), &t.rb.key_a, 1);
    assert_eq!(cards.mentions().len(), 1);
}

/// Card #177, §7.2: under Ask me a mention of Bob's agent is a request to
/// wake it, held for Bob (member_wake); under Closed it arrives at its
/// next turn.
#[test]
fn a_mention_of_their_agent_follows_their_receive_setting() {
    let t = two_people();
    let a1 = t.a().board_key(&t.board, &t.ra.key_a).unwrap();
    let b1 = t.b().board_key(&t.board, &t.rb.key_a).unwrap();
    let co_a = t.a().board_coord(&t.board).unwrap();
    let card = co_a
        .create_task(
            &a1,
            coord::NewTask {
                title: "Count the crops".into(),
                objective: "count them".into(),
                shared: true,
                ..coord::NewTask::default()
            },
        )
        .unwrap();
    t.sync();
    let mention = |text: &str| {
        co_a.person_comment(
            &Two::person(t.a()),
            &card.id,
            text,
            false,
            &[coord::Mention {
                key: b1.clone(),
                name: "alpha".into(),
                gone: false,
            }],
            None,
        )
        .unwrap();
        t.sync();
    };
    set_receive(t.b(), &t.board, receive::ASK);
    mention("@alpha first");
    let ds = wait_bridged(t.b(), &t.rb.key_a, 1);
    assert_eq!(ds[0].hold, hold::MEMBER_WAKE, "{:?}", ds[0]);

    set_receive(t.b(), &t.board, receive::CLOSED);
    mention("@alpha second");
    let ds = wait_bridged(t.b(), &t.rb.key_a, 2);
    let d = ds
        .iter()
        .find(|d| d.message.text.contains("second"))
        .unwrap();
    assert_eq!(d.mode, mode::NEXT_TURN, "{d:?}");
    assert_eq!(d.hold, "", "{d:?}");
}

/// A shared card both people's agents hold, linking [`PR`]: Alice's alpha
/// and Bob's alpha. Returns Alice's and Bob's coords and the card.
fn held_by_both(t: &Two) -> (Arc<coord::Coord>, Arc<coord::Coord>, coord::Task) {
    let a1 = t.a().board_key(&t.board, &t.ra.key_a).unwrap();
    let b1 = t.b().board_key(&t.board, &t.rb.key_a).unwrap();
    let co_a = t.a().board_coord(&t.board).unwrap();
    let co_b = t.b().board_coord(&t.board).unwrap();
    let card = co_a
        .create_task(
            &a1,
            coord::NewTask {
                title: "Ship the loader".into(),
                objective: "ship it".into(),
                shared: true,
                ..coord::NewTask::default()
            },
        )
        .unwrap();
    co_a.claim_task(&a1, &card.id).unwrap();
    co_a.add_link(&a1, &card.id, PR).unwrap();
    t.sync();
    wait_for(10, "bob sees the card", || {
        co_b.get_task(&card.id).is_some_and(|c| !c.links.is_empty())
    });
    co_b.claim_task(&b1, &card.id).unwrap();
    t.sync();
    wait_for(10, "alice sees bob's agent on it", || {
        co_a.get_task(&card.id)
            .is_some_and(|c| c.holders.contains(&b1) || c.co_holders.contains(&b1))
    });
    (co_a, co_b, card)
}

fn heard(state: &str, review: &str) -> coord::PrCheck {
    coord::PrCheck {
        pr: PR.into(),
        state: state.into(),
        review: review.into(),
        ..coord::PrCheck::default()
    }
}

/// Card #548: Alice's device hears the merge first. Bob's device folds the
/// row and its poller skips a merged PR, so Bob's holding agent hears it
/// from his own device's tick, woken, once, however often the tick runs.
#[test]
fn a_merge_heard_on_another_persons_device_reaches_the_holder_once() {
    let t = two_people();
    let (co_a, co_b, card) = held_by_both(&t);
    let (_, ns) = co_a
        .record_pr_state(&card.id, heard(coord::PR_MERGED, ""))
        .unwrap();
    // Bob's agent is in `to`, but only Alice's own stays on her device.
    t.a().route_notices(&inbox::system_sender(), &ns).unwrap();
    t.sync();
    wait_for(10, "bob folds the merge", || {
        co_b.pr_state(&card.id, PR).is_some()
    });

    t.b().check_prs();
    let ds = wait_bridged(t.b(), &t.rb.key_a, 1);
    let (d, m) = (&ds[0], &ds[0].message);
    assert_eq!(d.mode, mode::WAKE, "{d:?}");
    assert_eq!(m.ref_, format!("task:{}", card.id));
    assert!(
        m.text.contains("was merged") && m.text.contains("state=completed"),
        "{}",
        m.text
    );
    // It names the writer's person, not the reader.
    assert!(m.text.starts_with("Heard on "), "{}", m.text);
    assert!(!m.text.starts_with("Heard on bob"), "{}", m.text);
    assert!(bridged(t.b(), &t.rb.key_b).is_empty(), "beta holds nothing");

    for _ in 0..3 {
        t.b().check_prs();
    }
    t.sync();
    settle_bridged(t.b(), &t.rb.key_a, 1);

    // Alice's device wrote it: her agent heard it from the notice alone.
    t.a().check_prs();
    settle_bridged(t.a(), &t.ra.key_a, 0);
}

/// Card #548: both devices hear the merge before either's row crosses.
/// Each told its own agent as it wrote, so neither tick tells it again.
#[test]
fn a_merge_both_people_heard_is_not_told_again() {
    let t = two_people();
    let (co_a, co_b, card) = held_by_both(&t);
    co_a.record_pr_state(&card.id, heard(coord::PR_MERGED, ""))
        .unwrap();
    std::thread::sleep(Duration::from_millis(5));
    co_b.record_pr_state(&card.id, heard(coord::PR_MERGED, ""))
        .unwrap();
    t.sync();
    wait_for(10, "both fold both rows", || {
        [&co_a, &co_b]
            .iter()
            .all(|c| c.pr_news().first().is_some_and(|n| n.writers.len() == 2))
    });
    t.a().check_prs();
    t.b().check_prs();
    settle_bridged(t.a(), &t.ra.key_a, 0);
    settle_bridged(t.b(), &t.rb.key_a, 0);
}

/// Card #548: it crosses as a comment does: under Ask me the wake is held
/// for Bob, as only Alice's device vouches for it. Changes requested and
/// a closing come the same way.
#[test]
fn pr_news_from_another_person_follows_the_receive_setting() {
    let t = two_people();
    let (co_a, co_b, card) = held_by_both(&t);
    set_receive(t.b(), &t.board, receive::ASK);
    co_a.record_pr_state(&card.id, heard(coord::PR_OPEN, coord::PR_CHANGES_REQUESTED))
        .unwrap();
    t.sync();
    wait_for(10, "bob folds it", || co_b.pr_state(&card.id, PR).is_some());
    t.b().check_prs();
    let ds = wait_bridged(t.b(), &t.rb.key_a, 1);
    assert_eq!(ds[0].hold, hold::MEMBER_WAKE, "{:?}", ds[0]);
    assert!(
        ds[0].message.text.contains("changes requested"),
        "{}",
        ds[0].message.text
    );

    set_receive(t.b(), &t.board, receive::OPEN);
    std::thread::sleep(Duration::from_millis(5));
    co_a.record_pr_state(&card.id, heard(coord::PR_CLOSED, ""))
        .unwrap();
    t.sync();
    wait_for(10, "bob folds the closing", || {
        co_b.pr_state(&card.id, PR)
            .is_some_and(|p| p.state == coord::PR_CLOSED)
    });
    t.b().check_prs();
    let ds = wait_bridged(t.b(), &t.rb.key_a, 2);
    assert!(
        ds.iter()
            .any(|d| d.message.text.contains("closed without merging")),
        "{ds:?}"
    );
}

/// Card #174, since 0.11.1: an open card Alice makes on the board never
/// goes to Bob's agents, however often the sweep runs; there is no
/// setting for it any more (her own background agents serve her).
#[test]
fn an_open_card_never_reaches_the_other_persons_agents() {
    let t = two_people();
    let (b1, b2) = (
        t.b().board_key(&t.board, &t.rb.key_a).unwrap(),
        t.b().board_key(&t.board, &t.rb.key_b).unwrap(),
    );
    t.sync();
    // Alice's own agents sit out (paused): since card #337 her idle agents
    // would take her card on the board, and this checks Bob's side.
    let mut pol = t.a().inbox().policy();
    pol.paused = true;
    t.a().inbox().set_policy(pol, "user:test").unwrap();
    let co = t.a().board_coord(&t.board).unwrap();
    // Bob's idle agents don't get it.
    let first = co
        .create_task(
            &Two::person(t.a()),
            coord::NewTask {
                title: "Alice's own".into(),
                ..coord::NewTask::default()
            },
        )
        .unwrap();
    co.sweep_asks(&[idle(&b1), idle(&b2)]).unwrap();
    t.sync();
    settle_bridged(t.b(), &t.rb.key_a, 0);
    let o = co.get_task(&first.id).unwrap().live_offer().cloned();
    assert!(
        o.as_ref().is_none_or(|o| o.to.is_empty()),
        "it waits: {o:?}"
    );

    // Again, with Bob's agents idle in order: still nobody of his.
    co.sweep_asks(&idle_in_order(&[&b1, &b2])).unwrap();
    t.sync();
    settle_bridged(t.b(), &t.rb.key_a, 0);
    settle_bridged(t.b(), &t.rb.key_b, 0);
}

/// Card #187: a thread on a shared board can mention another person and
/// their agents. Bob's mentioned agent hears it first, woken, on Bob's
/// device; Bob gets a notification that opens the thread; the board log
/// keeps the mentions in board form.
#[test]
fn a_board_thread_mention_reaches_the_other_person_and_their_agent() {
    let cards = Arc::new(Cards::default());
    let notes = notify::Notifier::new(notify::Config {
        toaster: Some(cards.clone()),
        ..notify::Config::default()
    });
    let t = two_people_with(Some(notes));
    let b1 = t.b().board_key(&t.board, &t.rb.key_a).unwrap();
    let bob = Two::person(t.b());
    let m = |key: &str, name: &str| coord::Mention {
        key: key.into(),
        name: name.into(),
        gone: false,
    };
    let (th, _) = t
        .a()
        .board_ask(
            &t.board,
            coord::NewAsk {
                text: "@bob @alpha which build is faster?".into(),
                mentions: vec![
                    m(&bob, "bob"),
                    m(&b1, "alpha"),
                    // A private key never goes in a board log.
                    m("dev1/aaaaaaaa-1111-4111-8111-111111111111", "mine"),
                ],
                thread: true,
                ..coord::NewAsk::default()
            },
        )
        .unwrap();
    let a = th.ask.clone().unwrap();
    let keys: Vec<&str> = a.mentions.iter().map(|m| m.key.as_str()).collect();
    assert_eq!(keys, vec![bob.as_str(), b1.as_str()], "{a:?}");
    assert_eq!(
        a.heard,
        vec![b1.clone()],
        "the agent, not the person: {a:?}"
    );
    t.sync();

    // Bob's agent: woken with the thread.
    let ds = wait_bridged(t.b(), &t.rb.key_a, 1);
    let (d, msg) = (&ds[0], &ds[0].message);
    assert_eq!(d.mode, mode::WAKE, "{d:?}");
    assert_eq!(msg.board.kind, row_kind::ASK);
    assert!(
        msg.text.contains("started a thread") && msg.text.contains("which build is faster?"),
        "{}",
        msg.text
    );
    // Bob's other agent isn't named: it doesn't hear it first.
    assert!(bridged(t.b(), &t.rb.key_b).is_empty());

    // Bob: a notification that opens the thread.
    wait_for(10, "bob's mention card", || !cards.mentions().is_empty());
    let got = cards.mentions();
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].1.contains("mentioned you on"), "{:?}", got[0]);
    assert!(got[0].2.contains("which build is faster?"), "{:?}", got[0]);

    // Bob's copy of the log keeps the same board-form mentions.
    let co_b = t.b().board_coord(&t.board).unwrap();
    let theirs = co_b.get_task(&th.id).and_then(|x| x.ask).unwrap();
    assert_eq!(theirs.mentions.len(), 2, "{theirs:?}");
    t.sync();
    settle_bridged(t.b(), &t.rb.key_a, 1);
    assert_eq!(cards.mentions().len(), 1);
}

/// Card #371: a board's summary says how each member is: Alice, at this
/// device, is online; Bob's device isn't connected (no network here) but
/// its rows just arrived, so he was seen moments ago, with his agents on
/// the board counted.
#[test]
fn a_boards_people_show_who_is_online_and_their_agents() {
    let t = two_people();
    t.b().board_key(&t.board, &t.rb.key_a).unwrap();
    t.b().board_key(&t.board, &t.rb.key_b).unwrap();
    // Bob's device writes each card's status as its roster sees it, so
    // under load a card can first read gone until the next status sweep:
    // wait for both to count.
    let summary = || {
        t.sync();
        t.a()
            .board_list()
            .into_iter()
            .find(|s| s.board_id == t.board)
            .unwrap()
    };
    wait_for(30, "bob's two agents counted", || {
        summary()
            .members
            .iter()
            .any(|m| !m.me && m.working + m.free == 2)
    });
    let s = summary();
    let me = s.members.iter().find(|m| m.me).unwrap();
    assert_eq!(me.presence, "online", "{me:?}");
    assert!(me.last_seen.is_zero(), "{me:?}");
    let bob = s.members.iter().find(|m| !m.me).unwrap();
    assert_eq!(bob.person_id, t.b().person().id);
    assert_eq!(bob.presence, "recent", "{bob:?}");
    let ago = now().unix_milli_value() - bob.last_seen.unix_milli_value();
    assert!((0..60_000).contains(&ago), "seen {ago} ms ago: {bob:?}");
    assert_eq!(bob.working + bob.free, 2, "{bob:?}");
}

/// Card #385: Bob picks his colour on shared boards. His Core writes it on
/// his member.profile, and once the board syncs Alice's Core sees it on
/// Bob; going back to the default clears it again.
#[test]
fn a_persons_colour_pick_reaches_the_other_side() {
    let t = two_people();
    let bob = Two::person(t.b());
    let bobs = |c: &Core| {
        c.board_people()
            .into_iter()
            .find(|p| p.key == bob)
            .map(|p| p.color)
    };
    assert_eq!(bobs(t.a()), Some(String::new()), "no pick yet");
    assert!(t.b().set_person_color("mauve").is_err());
    t.b().set_person_color("pink").unwrap();
    wait_for(30, "bob's Core takes the pick", || {
        t.b().person_color() == "pink"
    });
    wait_for(30, "alice sees bob in pink", || {
        t.sync();
        bobs(t.a()).as_deref() == Some("pink")
    });
    t.b().set_person_color("").unwrap();
    wait_for(30, "alice sees bob's default again", || {
        t.sync();
        bobs(t.a()).as_deref() == Some("")
    });
}

/// Card #407: Bob's agent picks a face. Its card on the shared board
/// carries it, so Alice's Core shows Bob's agent with that face, checked.
#[test]
fn an_agents_face_reaches_a_friend_on_a_shared_board() {
    let t = two_people();
    let b1 = t.b().board_key(&t.board, &t.rb.key_a).unwrap();
    let name = t
        .b()
        .mesh_agents()
        .into_iter()
        .find(|a| a.key == t.rb.key_a)
        .map(|a| a.name)
        .unwrap();
    let v = serde_json::json!({"version":1,"id":"kettle","name":"Kettle","body":{"path":"M20 30 H80 V85 H20 Z"},"eyes":{"x":50,"y":55,"spacing":16,"scale":1}});
    let face = coord::avatar::check(&v).unwrap();
    t.b().set_agent_avatar(&name, Some(&face)).unwrap();
    wait_for(30, "alice sees bob's agent's face", || {
        t.sync();
        t.a()
            .board_agents()
            .into_iter()
            .any(|a| a.key == b1 && a.avatar.contains("\"id\":\"kettle\""))
    });
}

/// Card #393: Bob picks an avatar. His Core writes it on his
/// member.profile, and once the space syncs Alice's Core shows it on Bob in
/// the space's members; taking it off clears it again. Bob's colour rides
/// on the same summary.
#[test]
fn a_persons_avatar_reaches_the_other_side() {
    let t = two_people();
    let bob_id = t.b().person().id;
    let bobs = |c: &Core| {
        c.board_list()
            .into_iter()
            .flat_map(|b| b.members)
            .find(|m| m.person_id == bob_id)
            .map(|m| (m.avatar, m.color))
    };
    assert_eq!(
        bobs(t.a()),
        Some((String::new(), String::new())),
        "none yet"
    );
    assert!(
        t.b()
            .set_person_avatar("https://example.com/b.png")
            .is_err()
    );
    let pic = "data:image/png;base64,iVBORw0KGgo=";
    t.b().set_person_avatar(pic).unwrap();
    t.b().set_person_color("pink").unwrap();
    wait_for(30, "bob's Core takes the avatar", || {
        t.b().person_avatar() == pic
    });
    wait_for(30, "alice sees bob's avatar and colour", || {
        t.sync();
        bobs(t.a()) == Some((pic.to_string(), "pink".to_string()))
    });
    t.b().set_person_avatar("").unwrap();
    wait_for(30, "alice sees bob without one again", || {
        t.sync();
        bobs(t.a()).is_some_and(|(a, _)| a.is_empty())
    });
}

/// Card #1045: Bob names his assistant. Alice sees "Juno · <Bob's name>'s
/// assistant" with the person taken from Bob's own profile; with no name
/// chosen, both read the computed default and nothing was written for it;
/// clearing it goes back to the default.
#[test]
fn a_persons_assistant_name_reaches_the_other_side() {
    let t = two_people();
    let bob_id = t.b().person().id;
    let bob = |c: &Core| {
        c.board_list()
            .into_iter()
            .flat_map(|b| b.members)
            .find(|m| m.person_id == bob_id)
    };
    // The person comes from the member's own profile name, whatever Alice
    // has of it now; the chosen name sits beside it.
    let says = |c: &Core, chosen: &str| {
        bob(c).is_some_and(|m| {
            let d = format!("{}'s assistant", m.name);
            m.assistant
                == if chosen.is_empty() {
                    d
                } else {
                    format!("{chosen} · {d}")
                }
        })
    };
    wait_for(30, "alice sees the default, nothing written", || {
        t.sync();
        says(t.a(), "")
    });
    assert_eq!(t.b().assistant_name(), "");
    assert_eq!(
        t.b().assistant_name_or_default(),
        format!("{}'s assistant", t.b().person().name)
    );
    for bad in ["Juno 2", "Juno · Alice's assistant", &"x".repeat(25)] {
        assert!(t.b().set_assistant_name(bad).is_err(), "{bad}");
    }
    t.b().set_assistant_name("Juno").unwrap();
    wait_for(30, "bob's Core takes the name", || {
        t.b().assistant_name() == "Juno"
    });
    assert_eq!(t.b().assistant_name_or_default(), "Juno");
    wait_for(30, "alice sees bob's assistant by its name", || {
        t.sync();
        says(t.a(), "Juno")
    });
    t.b().set_assistant_name("").unwrap();
    wait_for(30, "alice sees the default again", || {
        t.sync();
        says(t.a(), "")
    });
}

/// Card #472: a new agent's word is never one another member's live agent
/// on a shared board has. Bob runs a "kestrel" and shares it on the board;
/// a seed whose first word is kestrel gives Alice's next agent kestrel
/// before Bob's card reaches her, and another word once it has.
#[test]
fn a_new_agents_word_differs_from_a_friends_agent_on_a_shared_board() {
    const SESS_K: &str = "dddddddd-4444-4444-8444-000000000004";
    let t = two_people();
    let seed = (0..100_000)
        .map(|i| format!("seed-{i}"))
        .find(|s| coord::names::pick(s, None) == "kestrel")
        .unwrap();
    assert_eq!(
        t.a().pick_agent_name(&seed),
        "kestrel",
        "nobody Alice knows of is kestrel yet"
    );
    let work = tempfile::tempdir().unwrap();
    common::write_session(
        &t.rb.claude.join("sessions"),
        "900009.json",
        SESS_K,
        "kestrel",
        work.path(),
    );
    let bobs = format!("{}/{SESS_K}", t.b().device_id());
    wait_for(60, "bob's kestrel is live", || {
        t.b().local_agents().iter().any(|c| c.key == bobs)
    });
    t.b().board_key(&t.board, &bobs).unwrap();
    wait_for(60, "alice sees bob's kestrel on the board", || {
        t.sync();
        t.a()
            .board_agents()
            .iter()
            .any(|a| a.name == "kestrel" && a.status != "gone")
    });
    assert!(
        t.a().mesh_agents().iter().all(|a| a.name != "kestrel"),
        "kestrel is only Bob's"
    );
    let got = t.a().pick_agent_name(&seed);
    assert_ne!(got, "kestrel", "Bob's agent has it");
    assert!(coord::names::WORDS.contains(&got.as_str()), "{got}");
}

/// Card #1075 (§12.12 B8): another person's ask reaches Bob's agent; once
/// the agent has it, every card it makes on Bob's private log is marked,
/// and a card by his other agent, which never heard it, is not.
#[test]
fn a_card_made_after_another_persons_words_is_marked() {
    let t = two_people();
    let (b1, b2) = (
        t.b().board_key(&t.board, &t.rb.key_a).unwrap(),
        t.b().board_key(&t.board, &t.rb.key_b).unwrap(),
    );
    t.sync();
    wait_cards(t.a(), &t.board, &[&b1, &b2]);
    let co = t.a().board_coord(&t.board).unwrap();
    co.post_ask(
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
    wait_bridged(t.b(), &t.rb.key_a, 1);
    let nt = |title: &str| coord::NewTask {
        title: title.into(),
        ..coord::NewTask::default()
    };
    let ib = t.b().inbox();
    assert!(!ib.saw_others(&t.rb.key_a), "not read yet");
    let w = ib
        .next_wake(&t.rb.key_a, inbox::path::MOD)
        .unwrap()
        .expect("the ask wakes alpha");
    ib.mark_delivered(&t.rb.key_a, &w.token).unwrap();
    assert!(ib.saw_others(&t.rb.key_a), "delivered: alpha read it");
    assert!(!ib.saw_others(&t.rb.key_b));
    let after = t
        .b()
        .coord()
        .create_task(&t.rb.key_a, nt("Look at the sync test"))
        .unwrap();
    let other = t
        .b()
        .coord()
        .create_task(&t.rb.key_b, nt("Something else"))
        .unwrap();
    assert!(t.b().coord().get_task(&after.id).unwrap().others);
    assert!(!t.b().coord().get_task(&other.id).unwrap().others);
}

// ---- merge desk §12.3 (card #1078, B3): review_claim --------------------

mod claim {
    use super::*;
    use coord::{NewTask, PrCard};
    use irori_core::deskwatch::PrLister;
    use irori_core::prwatch::PrChecker;
    use std::sync::Mutex;

    const REPO: &str = "remote:github.com/a/b";
    /// The PR's author on GitHub, as Alice's PR card names it.
    const AUTHOR: &str = "alice-gh";

    /// A fake gh: who it says wrote the PR, and who the viewer is.
    struct Gh {
        author: Mutex<String>,
        viewer: String,
    }

    impl PrLister for Gh {
        fn list(&self, _: &str, _: usize) -> Result<Vec<coord::DeskPr>, String> {
            Ok(vec![])
        }
        fn viewer(&self) -> Result<String, String> {
            if self.viewer.is_empty() {
                Err("no gh".into())
            } else {
                Ok(self.viewer.clone())
            }
        }
    }

    impl PrChecker for Gh {
        fn check(&self, url: &str) -> coord::PrCheck {
            coord::PrCheck {
                pr: url.into(),
                state: "OPEN".into(),
                ..coord::PrCheck::default()
            }
        }
        fn author(&self, _: &str) -> Result<String, String> {
            Ok(self.author.lock().unwrap().clone())
        }
    }

    /// Alice and Bob; Bob's gh says `viewer`, and says `author` wrote the PR.
    fn world(viewer: &str, author: &str) -> (Two, Arc<Gh>) {
        let gh = Arc::new(Gh {
            author: Mutex::new(author.into()),
            viewer: viewer.into(),
        });
        let g = gh.clone();
        let t = two_people_opts(Rig::new(), move |o| {
            o.desk_lister = Some(g.clone());
            o.pr_checker = Some(g);
        });
        if !viewer.is_empty() {
            // Bob's gh has said who he is: the desk poll learns it.
            t.b()
                .coord()
                .set_desk(
                    &format!("user:{}", t.b().device_id()),
                    coord::new_desk(REPO, t.b().device_id()),
                )
                .unwrap();
            let _ = t.b().poll_desk(REPO);
        }
        (t, gh)
    }

    /// Alice makes the PR card for PR 7, as herself.
    fn pr_card(t: &Two) -> String {
        let co = t.a().board_coord(&t.board).unwrap();
        let card = co
            .create_task(
                &Two::person(t.a()),
                NewTask {
                    title: "PR #7: ship it".into(),
                    objective: "Irori made this card for the pull request.".into(),
                    repo_key: REPO.into(),
                    links: vec![PR.into()],
                    pr_card: Some(PrCard {
                        pr: PR.into(),
                        author: AUTHOR.into(),
                    }),
                    ..NewTask::default()
                },
            )
            .unwrap();
        t.sync();
        card.id
    }

    fn call(c: &Core, key: &str, name: &str, args: serde_json::Value) -> String {
        let caller = tools::Caller {
            sender: inbox::Sender::new(key, "agent", "claude"),
            path: inbox::path::MOD.into(),
        };
        c.tools()
            .call(&caller, name, &serde_json::to_vec(&args).unwrap())
            .unwrap()
    }

    fn rc(c: &Core, key: &str, action: &str) -> String {
        call(
            c,
            key,
            "review_claim",
            serde_json::json!({"pr": PR, "action": action}),
        )
    }

    #[test]
    fn an_agent_with_no_shared_card_claims_and_the_other_person_sees_it() {
        let (t, _gh) = world("bob-gh", AUTHOR);
        pr_card(&t);
        let out = rc(t.b(), &t.rb.key_a, "claim");
        assert!(out.contains("You hold the review claim"), "{out}");
        assert!(out.contains("gh pr view"), "{out}");
        t.sync();
        let held = t.a().board_coord(&t.board).unwrap().claim_of(PR).unwrap();
        assert!(held.holder.starts_with("agent:"), "{held:?}");
        // Another of Bob's agents is told who holds it.
        let again = rc(t.b(), &t.rb.key_b, "claim");
        assert!(again.contains("holds the review claim"), "{again}");
        assert!(again.contains("comment on the PR instead"), "{again}");
        let refused = t.b().board_coord(&t.board).unwrap().claim_of(PR).unwrap();
        assert_eq!(refused.holder, held.holder);
        // confirm, then release.
        assert!(rc(t.b(), &t.rb.key_a, "confirm").contains("still hold"));
        assert!(rc(t.b(), &t.rb.key_b, "confirm").contains("don't hold"));
        assert!(rc(t.b(), &t.rb.key_a, "release").contains("Released"));
    }

    #[test]
    fn the_go_ahead_needs_work_in_the_repo_in_that_space() {
        let (t, _gh) = world("bob-gh", AUTHOR);
        pr_card(&t);
        let out = rc(t.b(), &t.rb.key_a, "claim");
        assert!(
            out.contains(irori_tools_text::OFF),
            "no card of his here: {out}"
        );
        rc(t.b(), &t.rb.key_a, "release");
        // Bob's own card in the repo.
        t.b()
            .board_coord(&t.board)
            .unwrap()
            .create_task(
                &Two::person(t.b()),
                NewTask {
                    title: "Fix the farm".into(),
                    objective: "work".into(),
                    repo_key: REPO.into(),
                    ..NewTask::default()
                },
            )
            .unwrap();
        let out = rc(t.b(), &t.rb.key_a, "claim");
        assert!(out.contains("already allowed this review"), "{out}");
        // With the setting off it is the Off text again.
        t.b().set_reviews_unasked(false).unwrap();
        let out = rc(t.b(), &t.rb.key_a, "claim");
        assert!(out.contains(irori_tools_text::OFF), "{out}");
    }

    mod irori_tools_text {
        pub const OFF: &str = "Your person signs off this review";
    }

    #[test]
    fn the_authors_own_agent_is_refused() {
        let (t, _gh) = world("bob-gh", AUTHOR);
        pr_card(&t);
        let out = rc(t.a(), &t.ra.key_a, "claim");
        assert!(out.contains("own PR"), "{out}");
    }

    #[test]
    fn a_pr_github_says_someone_else_wrote_is_refused() {
        let (t, gh) = world("bob-gh", AUTHOR);
        pr_card(&t);
        *gh.author.lock().unwrap() = "mallory".into();
        let out = rc(t.b(), &t.rb.key_a, "claim");
        assert!(out.contains("someone else wrote"), "{out}");
        assert!(t.b().board_coord(&t.board).unwrap().claim_of(PR).is_none());
    }

    #[test]
    fn an_unknown_github_login_is_refused() {
        let (t, _) = world("", AUTHOR);
        pr_card(&t);
        let out = rc(t.b(), &t.rb.key_a, "claim");
        assert!(out.contains("doesn't know your GitHub login"), "{out}");
    }

    #[test]
    fn a_card_an_agent_made_is_not_a_pr_card_and_lease_refuses_pr_names() {
        let (t, _gh) = world("bob-gh", AUTHOR);
        // A plain card that links the PR.
        let co = t.a().board_coord(&t.board).unwrap();
        co.create_task(
            &Two::person(t.a()),
            NewTask {
                title: "PR #7: ship it".into(),
                objective: "x".into(),
                repo_key: REPO.into(),
                links: vec![PR.into()],
                ..NewTask::default()
            },
        )
        .unwrap();
        t.sync();
        let out = rc(t.b(), &t.rb.key_a, "claim");
        assert!(out.contains("no card for this PR"), "{out}");
        let out = call(
            t.b(),
            &t.rb.key_a,
            "lease",
            serde_json::json!({"resource": "pr/a/b/7"}),
        );
        assert!(out.contains("review_claim"), "{out}");
    }

    #[test]
    fn a_decline_comments_on_the_card_and_lets_go() {
        let (t, _gh) = world("bob-gh", AUTHOR);
        let id = pr_card(&t);
        rc(t.b(), &t.rb.key_a, "claim");
        let out = call(
            t.b(),
            &t.rb.key_a,
            "review_claim",
            serde_json::json!({"pr": PR, "action": "decline", "reason": "not my area"}),
        );
        assert!(out.contains("not you"), "{out}");
        t.sync();
        let co = t.a().board_coord(&t.board).unwrap();
        assert!(co.claim_of(PR).is_none());
        let cs = co.comments(&id).unwrap();
        assert!(
            cs.iter()
                .any(|c| c.declines == PR && c.text.contains("not my area")),
            "{cs:?}"
        );
    }
}
