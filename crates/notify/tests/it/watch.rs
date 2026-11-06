//! I:internal/notify/watch_test.go (the Board and ask cards, and the inbox
//! half through the real loop), ask_test.go and owner_test.go. The inbox
//! half's own logic tests are crates/inbox/tests/it/notify.rs.

use crate::common;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime};

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
/// Long enough for a paced Board reconcile to have run and posted.
const QUIET: Duration = Duration::from_millis(350);

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

/// The Board, counting the views Watch takes of it.
struct Counted {
    co: Arc<Coord>,
    views: Arc<AtomicUsize>,
}

impl notify::watch::BoardSource for Counted {
    fn subscribe(&self) -> Arc<gocompat::sub::Queue<coord::Change>> {
        self.co.subscribe()
    }
    fn board(&self, f: &coord::BoardFilter) -> coord::BoardView {
        self.views.fetch_add(1, Ordering::SeqCst);
        self.co.board(f)
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
    views: Arc<AtomicUsize>,
}

type Wrap = Arc<dyn Fn(&str, &str) -> String + Send + Sync>;

/// The sha256 of the one screenshot the rig's device holds.
const LOCAL_SHOT: &str = "abababababababababababababababababababababababababababababababab";

fn tiny_png() -> Vec<u8> {
    let mut out = vec![];
    let mut e = png::Encoder::new(&mut out, 4, 2);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    let mut w = e.write_header().unwrap();
    w.write_image_data(&[90u8; 4 * 2 * 3]).unwrap();
    w.finish().unwrap();
    out
}

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
    // This device holds one screenshot (card #149): LOCAL_SHOT, a 4x2 PNG.
    let shot = dir.path().join("shot.png");
    std::fs::write(&shot, tiny_png()).unwrap();
    let file: notify::FileFn = Arc::new(move |sha: &str| (sha == LOCAL_SHOT).then(|| shot.clone()));
    let views = Arc::new(AtomicUsize::new(0));
    let handle = watch(
        &n,
        Sources {
            inbox: Some(ib.clone()),
            board: Some(Arc::new(Counted {
                co: co.clone(),
                views: views.clone(),
            })),
            user: USER.into(),
            name: Some(name),
            room: Some(co.clone()),
            file: Some(file),
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
        views,
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

/// Card #554: a burst of changes shares its Board views, and the view after
/// the last change still posts and clears what the burst left.
#[test]
fn watch_board_a_burst_costs_few_views_and_ends_right() {
    const BURST: usize = 300;
    let r = rig();
    let gone =
        r.co.create_task(KEY_A, new_task("answered in the burst"))
            .unwrap();
    r.co.claim_task(KEY_A, &gone.id).unwrap();
    r.block_on_person(KEY_A, &gone.id, "still there?");
    r.fake.wait_post(&needs_id(&gone.id));
    let busy = r.co.create_task(KEY_A, new_task("a busy card")).unwrap();
    let kept =
        r.co.create_task(KEY_A, new_task("blocked in the burst"))
            .unwrap();
    r.co.claim_task(KEY_A, &kept.id).unwrap();
    std::thread::sleep(notify::watch::BOARD_EVERY * 2); // the setup's views are done

    let before = r.views.load(Ordering::SeqCst);
    let start = Instant::now();
    for i in 0..BURST {
        r.co.add_comment(KEY_B, &busy.id, &format!("change {i}"), false)
            .unwrap();
    }
    r.co.unblock(USER, &gone.id, "yes").unwrap();
    r.block_on_person(KEY_A, &kept.id, "the last change");
    let took = start.elapsed();

    r.fake.wait_post(&needs_id(&kept.id));
    r.fake.wait_gone(&needs_id(&gone.id));
    std::thread::sleep(notify::watch::BOARD_EVERY * 2); // nothing still to come
    let views = r.views.load(Ordering::SeqCst) - before;
    // One at once, one per interval the burst ran, one for the lane card it
    // answered, one after its last row.
    let paced = (took.as_millis() / notify::watch::BOARD_EVERY.as_millis()) as usize + 3;
    assert!(
        views <= paced && views < BURST / 10,
        "{views} views for {BURST} changes in {took:?}"
    );
}

/// Card #554: answered and asked back inside one interval, a card that
/// left the lane and came back is still a new question.
#[test]
fn watch_board_an_ask_back_right_after_the_answer_posts_again() {
    let r = rig();
    let t = r.co.create_task(KEY_A, new_task("quick")).unwrap();
    r.co.claim_task(KEY_A, &t.id).unwrap();
    r.block_on_person(KEY_A, &t.id, "blue or green?");
    let id = needs_id(&t.id);
    r.fake.wait_post(&id);
    let b = r.co.get_task(&t.id).unwrap().open_blockers()[0].id.clone();
    r.co.answer(
        USER,
        &t.id,
        &b,
        coord::BlockerAnswer {
            text: "blue".into(),
            ..Default::default()
        },
    )
    .unwrap();
    r.fake.wait_gone(&id);
    r.co.comment_on(
        KEY_A,
        &t.id,
        coord::CommentInput {
            text: "which blue?".into(),
            blocker_id: b,
            missing: vec!["the shade".into()],
            ..Default::default()
        },
    )
    .unwrap();
    r.fake.wait_post(&id);
}

/// Card #554: a change still waiting for its reconcile at stop gets it. The
/// next start seeds quietly, so it would never get a card.
#[test]
fn watch_board_a_stop_reads_what_was_waiting() {
    let mut r = rig();
    let t = r.co.create_task(KEY_A, new_task("last moment")).unwrap();
    r.co.claim_task(KEY_A, &t.id).unwrap();
    r.block_on_person(KEY_A, &t.id, "still there?");
    r.handle.take().unwrap().stop();
    r.fake.wait_post(&needs_id(&t.id));
}

/// A lone change isn't held back for the pace of a burst.
#[test]
fn watch_board_a_lone_change_reconciles_at_once() {
    let r = rig();
    let t = r.co.create_task(KEY_A, new_task("alone")).unwrap();
    r.co.claim_task(KEY_A, &t.id).unwrap();
    std::thread::sleep(notify::watch::BOARD_EVERY * 2);
    let start = Instant::now();
    r.block_on_person(KEY_A, &t.id, "now?");
    r.fake.wait_post(&needs_id(&t.id));
    assert!(
        start.elapsed() < notify::watch::BOARD_EVERY,
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn watch_board_needs_you() {
    let r = rig();
    let task =
        r.co.create_task(KEY_A, new_task("ship the release"))
            .unwrap();
    r.co.claim_task(KEY_A, &task.id).unwrap();
    r.fake.no_post(QUIET); // working, not on the person
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
    r.fake.no_post(QUIET);

    // A new reason while still in the lane: no repost.
    r.block_on_person(KEY_A, &task.id, "still need the number");
    r.fake.no_post(QUIET);
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

/// Card #509: an agent's external wait on a PR is the person's merge. It
/// used to sit in Blocked, unseen; now it pops up and names the PR, and a
/// wait on CI stays quiet.
#[test]
fn watch_board_a_wait_on_a_pr_asks_the_person_to_merge() {
    let r = rig();
    let ci = r.co.create_task(KEY_A, new_task("wait on CI")).unwrap();
    r.co.claim_task(KEY_A, &ci.id).unwrap();
    r.co.block(
        KEY_A,
        &ci.id,
        "CI on the release",
        Some(coord::BlockedBy {
            kind: coord::BLOCKED_ON_EXTERNAL.into(),
            reference: "CI on the release".into(),
        }),
    )
    .unwrap();
    r.fake.no_post(QUIET);

    let pr = "https://github.com/a/b/pull/41";
    let task = r.co.create_task(KEY_A, new_task("part 10")).unwrap();
    r.co.claim_task(KEY_A, &task.id).unwrap();
    r.co.block(
        KEY_A,
        &task.id,
        "Review and merge PR #41",
        Some(coord::BlockedBy {
            kind: coord::BLOCKED_ON_EXTERNAL.into(),
            reference: pr.into(),
        }),
    )
    .unwrap();
    let id = needs_id(&task.id);
    r.fake.wait_post(&id);
    let c = r.fake.card(&id);
    assert_eq!(c.title, "alpha waits on you to merge PR #41 (#2)");
    assert!(
        c.body.starts_with("Review and merge PR #41"),
        "{:?}",
        c.body
    );
}

/// Card #520: a card nobody holds whose wait's agent is gone pops up as
/// such, never "blocked on you", with Open (nothing to answer on it).
#[test]
fn watch_board_a_wait_on_a_gone_agent_says_so() {
    let r = rig();
    let task = r.co.create_task(KEY_A, new_task("the spawner")).unwrap();
    r.co.claim_task(KEY_A, &task.id).unwrap();
    r.co.block(
        KEY_A,
        &task.id,
        "Waiting on beta's GUI spawn",
        Some(coord::BlockedBy {
            kind: coord::BLOCKED_ON_AGENT.into(),
            reference: KEY_B.into(),
        }),
    )
    .unwrap();
    r.fake.no_post(QUIET);
    r.ros.drop_key(KEY_B);
    r.co.leave(KEY_A, &task.id, KEY_A).unwrap();
    let id = needs_id(&task.id);
    r.fake.wait_post(&id);
    let c = r.fake.card(&id);
    assert!(
        c.title.starts_with("Nobody holds it, and ")
            && c.title.ends_with(", whom it waits on, is gone (#1)"),
        "{:?}",
        c.title
    );
    assert!(
        c.body.starts_with("Waiting on beta's GUI spawn"),
        "{:?}",
        c.body
    );
}

/// Card #520: on the person's own card there is no agent left to name,
/// but a wait no agent will end still pops up.
#[test]
fn watch_board_a_gone_wait_on_the_persons_own_card_pops_up() {
    let r = rig();
    let task = r.co.create_task(USER, new_task("the spawner")).unwrap();
    r.co.claim_task(KEY_A, &task.id).unwrap();
    r.co.block(
        KEY_A,
        &task.id,
        "Waiting on beta's GUI spawn",
        Some(coord::BlockedBy {
            kind: coord::BLOCKED_ON_AGENT.into(),
            reference: KEY_B.into(),
        }),
    )
    .unwrap();
    r.fake.no_post(QUIET);
    r.ros.drop_key(KEY_B);
    r.co.leave(KEY_A, &task.id, KEY_A).unwrap();
    let id = needs_id(&task.id);
    r.fake.wait_post(&id);
    let c = r.fake.card(&id);
    assert!(
        c.title.contains("whom it waits on, is gone"),
        "{:?}",
        c.title
    );
}

/// Card #130: an agent's completed waits on the person's Done (card
/// #112). Its card is good news: green, "finished", Open and Mark Done,
/// and Mark Done closes the card. A blocked card still warns.
#[test]
fn watch_board_finished_is_ready_to_review() {
    let r = rig_with(
        None,
        coord::Options {
            done_check: true,
            ..coord::Options::default()
        },
    );
    let task =
        r.co.create_task(KEY_A, new_task("ship the release"))
            .unwrap();
    r.co.claim_task(KEY_A, &task.id).unwrap();
    r.block_on_person(KEY_A, &task.id, "which version number?");
    let id = needs_id(&task.id);
    r.fake.wait_post(&id);
    let c = r.fake.card(&id);
    assert_eq!(c.title, "alpha is blocked on you (#1)");
    assert_eq!(c.kind, toast::Kind::Warn, "a blocker still warns");
    r.co.unblock(USER, &task.id, "0.4").unwrap();
    r.fake.wait_gone(&id);

    // Card #140: the body is the proof's first line, with its screenshots.
    let shot = coord::FileRef {
        sha256: "ab".repeat(32),
        name: "panel.png".into(),
        size: 12,
        mime: "image/png".into(),
        device: "dev1".into(),
    };
    let got =
        r.co.set_task_state_with(
            KEY_A,
            &task.id,
            coord::COMPLETED,
            "Release notes cut for 0.4.\nChecked: cargo test; opened the page.",
            vec![shot],
            true,
        )
        .unwrap();
    assert_eq!(got.state, coord::BLOCKED, "a done check, not closed");
    let c = r
        .fake
        .wait_title(&id, "alpha finished “ship the release” (#1)");
    assert_eq!(c.body, "Release notes cut for 0.4. · 1 screenshot");
    // Card #149: that screenshot is on this device, so it is the thumbnail.
    let img = c
        .image
        .clone()
        .expect("the proof's screenshot as a thumbnail");
    assert_eq!((img.w, img.h), (4, 2));
    assert_eq!(&img.rgba[..4], &[90, 90, 90, 255]);
    assert_eq!(c.kind, toast::Kind::Success);
    let labels: Vec<&str> = c.actions.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(labels, vec!["Open", "Mark Done"]);
    r.fake.drain();
    // A press takes the card down, as the engine does: Mark Done.
    r.fake.press(&id, "Mark Done");
    wait_for("Mark Done closes the card", || {
        r.co.get_task(&task.id)
            .is_some_and(|t| t.state == coord::COMPLETED && t.open_blockers().is_empty())
    });
    r.fake.no_post(QUIET); // closed: it doesn't come back
}

/// Card #140: new proof while the card is still up re-posts it in place
/// with the new first line, and a question raised after it went up stops
/// Mark Done from closing the card over it.
#[test]
fn watch_board_finished_follows_new_proof() {
    let r = rig_with(
        None,
        coord::Options {
            done_check: true,
            ..coord::Options::default()
        },
    );
    let task = r.co.create_task(KEY_A, new_task("ship it")).unwrap();
    r.co.claim_task(KEY_A, &task.id).unwrap();
    let id = needs_id(&task.id);
    r.co.set_task_state_with(
        KEY_A,
        &task.id,
        coord::COMPLETED,
        "First try.",
        Vec::new(),
        true,
    )
    .unwrap();
    r.fake.wait_title(&id, "alpha finished “ship it” (#1)");
    assert_eq!(r.fake.card(&id).body, "First try.");
    assert!(
        r.fake.card(&id).image.is_none(),
        "no screenshot, no thumbnail"
    );
    // Card #149: a screenshot another device holds gives no thumbnail.
    let elsewhere = coord::FileRef {
        sha256: "cd".repeat(32),
        name: "other.png".into(),
        size: 12,
        mime: "image/png".into(),
        device: "dev2".into(),
    };
    r.co.set_task_state_with(
        KEY_A,
        &task.id,
        coord::COMPLETED,
        "Second try.
more",
        vec![elsewhere],
        true,
    )
    .unwrap();
    wait_for("the card shows the new proof", || {
        r.fake.is_live(&id) && r.fake.card(&id).body == "Second try. · 1 screenshot"
    });
    assert!(
        r.fake.card(&id).image.is_none(),
        "not on this device: no thumbnail"
    );

    // A real question after the card went up: Mark Done refuses.
    r.co.raise(
        KEY_A,
        &task.id,
        coord::NewBlocker {
            question: "Which version?".into(),
            blocked_by: Some(coord::BlockedBy {
                kind: coord::BLOCKED_ON_PERSON.into(),
                ..Default::default()
            }),
            ..coord::NewBlocker::default()
        },
    )
    .unwrap();
    let err = notify::watch::mark_done(&r.co, USER, &task.id).unwrap_err();
    assert!(err.contains("open question"), "{err}");
    assert_ne!(r.co.get_task(&task.id).unwrap().state, coord::COMPLETED);
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
    r.fake.no_post(QUIET);

    // An agent handing the person a card still pops one.
    let handed = r.co.create_task(KEY_A, new_task("review the PR")).unwrap();
    r.co.assign_task(KEY_A, &handed.id, USER).unwrap();
    r.fake.wait_post(&needs_id(&handed.id));
    assert_eq!(
        r.fake.card(&needs_id(&handed.id)).title,
        "alpha handed you a card (#3)"
    );
}

/// Posts an ask that alpha is handed and gives back, so it waits unclaimed
/// with one agent told.
fn ask_told_one(r: &Rig) -> Task {
    let alpha: Vec<CoordCard> = r
        .ros
        .agents()
        .into_iter()
        .filter(|c| c.key == KEY_A)
        .collect();
    let (ask, _) =
        r.co.post_ask(
            USER,
            NewAsk {
                text: "who can look at the release notes?".into(),
                ..NewAsk::default()
            },
            &alpha,
        )
        .unwrap();
    assert_eq!(r.co.get_task(&ask.id).unwrap().ask.unwrap().taker, KEY_A);
    r.co.leave(KEY_A, &ask.id, KEY_A).unwrap();
    r.co.sweep_asks(&[]).unwrap();
    let a = r.co.get_task(&ask.id).unwrap().ask.unwrap();
    assert!(a.unclaimed && a.told == 1 && a.taker.is_empty(), "{a:?}");
    ask
}

/// The take requests written so far (giving an ask back writes one too).
fn requests(r: &Rig) -> usize {
    r.st.list(&store::Filter {
        kinds: vec!["take.requested".into()],
        ..Default::default()
    })
    .unwrap()
    .len()
}

fn labels(c: &toast::Note) -> Vec<&str> {
    c.actions.iter().map(|a| a.label.as_str()).collect()
}

#[test]
fn watch_unclaimed_ask() {
    let r = rig();
    // Nobody free (told == 0): Open and Later, nothing that wakes again or
    // ends the ask, and it says it waits for a free agent.
    let (ask, _) =
        r.co.post_ask(
            USER,
            NewAsk {
                text: "anyone?".into(),
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap();
    let id = unclaimed_id(&ask.id);
    r.fake.wait_post(&id);
    let c = r.fake.card(&id);
    assert!(c.title.contains("Nobody took") && c.title.contains(&format!("#{}", ask.short)));
    assert!(c.body.contains("first agent that is"), "{c:?}");
    assert_eq!(labels(&c), vec!["Open", "Later"]);
    let before = requests(&r);
    r.fake.press(&id, "Later");
    // Later only dismisses: the ask stays open, nothing is requested, and
    // nothing reposts it while it waits.
    r.co.sweep_asks(&[]).unwrap();
    r.fake.no_post(QUIET);
    assert_eq!(requests(&r), before);
    let still = r.co.get_task(&ask.id).unwrap();
    assert!(still.state != coord::CANCELED && still.ask.unwrap().is_open());

    // Told but none took it: Wake them clears heard.
    let ask = ask_told_one(&r);
    let id = unclaimed_id(&ask.id);
    r.fake.wait_post(&id);
    let c = r.fake.card(&id);
    assert!(c.body.contains("1 agents were told"), "{c:?}");
    assert_eq!(labels(&c), vec!["Wake them", "Later"]);
    let before = requests(&r);
    r.fake.press(&id, "Wake them");
    wait_for("Wake them writes a reopen request", || {
        requests(&r) == before + 1
    });
    // Nobody is free now: the owner's sweep opens it again, still
    // unclaimed, and the card comes back without Wake them.
    r.co.sweep_asks(&[]).unwrap();
    r.fake.wait_post(&id);
    let c = r.fake.card(&id);
    assert!(
        c.body.contains("You woke them") && c.body.contains("first agent"),
        "{c:?}"
    );
    assert_eq!(labels(&c), vec!["Open", "Later"]);
    r.fake.press(&id, "Open");
    let t = r.opened();
    assert!(
        t.tab == TAB_BOARD && t.ref_ == format!("task:{}", ask.id),
        "{t:?}"
    );
    r.co.ask_claim(KEY_A, &ask.id, Duration::from_secs(1))
        .unwrap();
    r.fake.wait_gone(&id);
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
    let ask = ask_told_one(&r);
    let id = unclaimed_id(&ask.id);
    r.fake.wait_post(&id);
    let before = requests(&r);
    r.fake.press(&id, "Wake them");
    wait_for("the reopen request", || requests(&r) == before + 1);
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

/// A returned card's Board view for the BoardCards harness (card #483).
fn returned_view(lane: &str, was: &str, spawns: Vec<coord::Spawn>) -> BoardView {
    let card = Card {
        task: Task {
            id: "t1".into(),
            short: "453".into(),
            title: "Spawned agents part 9".into(),
            returned: Some(Returned {
                was: was.into(),
                at: GoTime::from_system(SystemTime::now()),
                person: true,
                row: "dev1:5".into(),
            }),
            spawns,
            ..Task::default()
        },
        ..Card::default()
    };
    if lane == "needs_you" {
        BoardView {
            needs_you: vec![card],
            ..BoardView::default()
        }
    } else {
        BoardView {
            open: vec![card],
            ..BoardView::default()
        }
    }
}

fn posted(f: &Fake, id: &str) -> bool {
    f.lock().posts.iter().any(|n| n.id == id)
}

/// Card #483: no key in a returned card. A name source that knows nothing
/// (""), or one that falls back to the key, gives "Its agent"; the card's
/// spawn record names the agent it started.
#[test]
fn watch_returned_card_never_names_a_key() {
    let id = returned_id("t1");
    let names: [inbox::watch::NameFn; 2] = [
        Arc::new(|_: &str| String::new()),
        Arc::new(|k: &str| k.to_string()),
    ];
    for name in names {
        let f = Fake::new();
        let n = notifier(&f);
        let mut w = BoardCards::new(
            &n,
            &Sources {
                name: Some(name.clone()),
                ..Sources::default()
            },
        );
        w.returned(&returned_view("open", KEY_A, vec![]), true);
        assert_eq!(f.drain(), 1);
        let body = f.card(&id).body;
        assert_eq!(
            body,
            "Its agent went away, so it's open again. Assign it to an agent, or close it."
        );
        assert!(!body.contains(KEY_A) && !body.contains('/'), "{body}");

        // The card's spawn record bound to that key names it.
        let f = Fake::new();
        let n = notifier(&f);
        let mut w = BoardCards::new(
            &n,
            &Sources {
                name: Some(name),
                ..Sources::default()
            },
        );
        let spawns = vec![
            coord::Spawn {
                key: KEY_B.into(),
                name: "Someone else".into(),
                ..coord::Spawn::default()
            },
            coord::Spawn {
                key: KEY_A.into(),
                name: "Bing Bong".into(),
                ..coord::Spawn::default()
            },
        ];
        w.returned(&returned_view("open", KEY_A, spawns), true);
        assert_eq!(f.drain(), 1);
        let body = f.card(&id).body;
        assert!(body.starts_with("Bing Bong went away"), "{body}");
    }

    // No key at all: still "Its agent".
    let f = Fake::new();
    let n = notifier(&f);
    let mut w = BoardCards::new(&n, &Sources::default());
    w.returned(&returned_view("open", "", vec![]), true);
    assert_eq!(f.drain(), 1);
    assert!(f.card(&id).body.starts_with("Its agent went away"));
}

/// Card #483: a returned card waiting on the person (Needs you: their Done
/// or their answer) gets no "assign it" card, and one already up clears;
/// once it stops waiting with nobody on it, the card comes.
#[test]
fn watch_returned_card_not_while_it_waits_on_the_person() {
    let f = Fake::new();
    let n = notifier(&f);
    let mut w = BoardCards::new(
        &n,
        &Sources {
            name: Some(Arc::new(|_: &str| "alpha".to_string())),
            ..Sources::default()
        },
    );
    let id = returned_id("t1");
    w.returned(&returned_view("needs_you", KEY_A, vec![]), true);
    assert_eq!(f.drain(), 0, "it waits on the person: no card");
    assert!(!posted(&f, &id));

    w.returned(&returned_view("open", KEY_A, vec![]), true);
    assert_eq!(f.drain(), 1, "it stopped waiting, nobody on it: the card");
    assert!(f.card(&id).body.starts_with("alpha went away"));

    w.returned(&returned_view("needs_you", KEY_A, vec![]), true);
    f.wait_gone(&id);
}

/// Card #453's state: an agent finished a card that waits on the person's
/// Done, then went away. No "assign it" card while that Done is open.
#[test]
fn watch_returned_card_waiting_on_done() {
    let r = rig_with(
        None,
        coord::Options {
            done_check: true,
            holder_grace: Duration::from_millis(20),
            ..coord::Options::default()
        },
    );
    let tk =
        r.co.create_task(USER, new_task("Spawned agents part 9"))
            .unwrap();
    r.co.claim_task(KEY_A, &tk.id).unwrap();
    let got =
        r.co.set_task_state(KEY_A, &tk.id, coord::COMPLETED, "Shipped it.")
            .unwrap();
    assert_eq!(got.state, coord::BLOCKED, "a done check, not closed");
    r.fake.wait_post(&needs_id(&tk.id));
    r.ros.drop_key(KEY_A);
    wait_for("the gone holder is taken off", || {
        r.co.sweep_notices().unwrap();
        r.co.get_task(&tk.id)
            .unwrap()
            .returned
            .is_some_and(|t| t.person)
    });
    let t = r.co.get_task(&tk.id).unwrap();
    assert!(t.holders.is_empty(), "{t:?}");
    std::thread::sleep(Duration::from_millis(200));
    assert!(!posted(&r.fake, &returned_id(&tk.id)), "no assign-it card");
    assert!(r.fake.is_live(&needs_id(&tk.id)), "its Done is still up");
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
