//! Assistants talk, card 4: the answering path, with a fake turn that
//! returns fixed text. Alice asks in Bob's space's #office-assistants
//! channel; Bob's computer (the first desktop that can run the turn) checks
//! the question against §5's table, and posts the answer as the assistant's
//! card with a deterministic answer id.

use crate::common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use common::mesh::{Dev, invited, person};
use common::wait_for;
use coord::talk::{Post, PostRow, assistants_channel_id};
use gocompat::time::GoTime;
use inbox::{ReceiveSetting, receive};
use irori_core::assistant_answers::{answer_id, waiting_id};
use irori_core::spawn::{SpawnStarted, Spawner};

const WAIT: u64 = 90;
const FIXED: &str = "Sam can review it; ask them.";

fn ready_spawner() -> Spawner {
    Spawner {
        ready: Some(Arc::new(|| true)),
        spawn: Arc::new(|r| {
            Ok(SpawnStarted {
                name: r.name.clone(),
                prefix: String::new(),
            })
        }),
        end: Arc::new(|_| Ok(String::new())),
        trusted: Arc::new(|_| true),
        list: None,
    }
}

/// A fake turn that counts its runs and returns [`FIXED`].
fn fake_turn(runs: &Arc<AtomicUsize>) -> irori_core::assistant_answers::Answerer {
    let runs = runs.clone();
    Arc::new(move |_| {
        runs.fetch_add(1, Ordering::SeqCst);
        Ok(FIXED.to_string())
    })
}

struct World {
    alice: Vec<Dev>,
    bob: Vec<Dev>,
    space: String,
    conv: String,
    card: String,
}

fn pointer(d: &Dev, space: &str, person: &str) -> String {
    d.core
        .board_verifier(space)
        .map(|v| v.profile_assistant_card(person))
        .unwrap_or_default()
}

/// Alice in Bob's space, Bob's `bobs` devices ready to run the turn (their
/// assistant card minted, the channel there on both sides). `turns` are
/// installed on the first `turns.len()` of Bob's devices.
fn world(bobs: usize, installed: bool) -> World {
    let alice = person("alice", 1);
    let bob = person("bob", bobs);
    let (a1, b1) = (&alice[0], &bob[0]);
    let space = invited(a1, b1);
    for d in &bob[1..] {
        wait_for(WAIT, "bob's other device opens the space", || {
            d.core
                .board_list()
                .iter()
                .any(|s| s.board_id == space && s.ready)
        });
        let addrs = |x: &Dev| x.core.net().endpoint().expect("bound").bound_sockets();
        d.core.board_addr_hint(a1.core.device_id(), addrs(a1));
        a1.core.board_addr_hint(d.core.device_id(), addrs(d));
        d.core.kick_board(&space);
    }
    for d in &bob {
        d.core.set_spawner(Some(ready_spawner()));
    }
    let bob_id = b1.core.person().id;
    let conv = assistants_channel_id(&space);
    wait_for(WAIT, "bob's assistant card is minted and alice sees it", || {
        !pointer(b1, &space, &bob_id).is_empty()
            && !pointer(a1, &space, &bob_id).is_empty()
            && a1.core.talk().channel(&conv).is_some()
            && b1.core.talk().channel(&conv).is_some()
    });
    let card = pointer(b1, &space, &bob_id);
    let w = World {
        alice,
        bob,
        space,
        conv,
        card,
    };
    if installed {
        let runs = Arc::new(AtomicUsize::new(0));
        for d in &w.bob {
            d.core.set_answerer(Some(fake_turn(&runs)));
        }
    }
    w
}

static N: AtomicUsize = AtomicUsize::new(0);

/// What a question looks like on the wire, before the test edits it.
fn question(w: &World, text: &str) -> PostRow {
    PostRow {
        post_id: format!("post_q{:04}", N.fetch_add(1, Ordering::SeqCst)),
        conv_id: w.conv.clone(),
        text: text.into(),
        mentions: vec![format!("agent:{}", w.card)],
        agents_now: true,
        by: w.alice[0].person_actor(),
        at: GoTime::from_system(SystemTime::now()),
        ..PostRow::default()
    }
}

/// Alice's device writes `r` into the space, with a hop when it has one.
fn send(w: &World, r: &PostRow, hop: i64) {
    let a1 = &w.alice[0];
    let mut v = serde_json::to_value(r).unwrap();
    if hop > 0 {
        v["hop"] = hop.into();
    }
    a1.core
        .space_talk_post(&w.space, &a1.person_actor(), v)
        .unwrap();
    a1.core.kick_board(&w.space);
}

/// The posts answering `q`, as `d` has them.
fn answers(d: &Dev, conv: &str, q: &str) -> Vec<Post> {
    d.core
        .talk()
        .posts(conv)
        .into_iter()
        .filter(|p| p.reply_to.as_deref() == Some(q))
        .collect()
}

/// Waits for the answer to `q` on Bob's device `d` and returns it.
fn answered(w: &World, d: &Dev, q: &str) -> Post {
    wait_for(WAIT, "the assistant answers", || {
        !answers(d, &w.conv, q).is_empty()
    });
    answers(d, &w.conv, q).remove(0)
}

/// After `q` is answered, every question sent before it has been looked at
/// too (one device writes in order, and the bridge reads in order).
fn control(w: &World) {
    let c = question(w, "control?");
    send(w, &c, 0);
    answered(w, &w.bob[0], &c.post_id);
}

fn set_receive(d: &Dev, space: &str, preset: &str) {
    d.core
        .inbox()
        .set_receive(
            ReceiveSetting {
                board_id: space.into(),
                preset: preset.into(),
                ..ReceiveSetting::default()
            },
            &format!("user:{}", d.core.device_id()),
        )
        .unwrap();
}

fn close(w: World) {
    for d in w.alice.into_iter().chain(w.bob) {
        d.core.close().unwrap();
    }
}

/// Open (the default) answers once, as the assistant's card, with the
/// answer id and `replyTo`; a reply starts no turn; a question at the hop
/// limit, one already answered and one in a different channel run nothing;
/// and "Agents answer now" off still reaches the assistant.
#[test]
fn open_answers_once_and_the_checks_stop_the_rest() {
    let w = world(1, false);
    let runs = Arc::new(AtomicUsize::new(0));
    let b1 = &w.bob[0];
    // Asked before the turn is here: nothing yet.
    let early = question(&w, "who reviews PR #12?");
    send(&w, &early, 0);
    std::thread::sleep(Duration::from_secs(3));
    assert!(answers(b1, &w.conv, &early.post_id).is_empty());
    b1.core.set_answerer(Some(fake_turn(&runs)));

    // The scan answers it now (and a question typed with Agents answer now
    // off still reaches the assistant).
    let a = answered(&w, b1, &early.post_id);
    assert_eq!(a.by, format!("agent:{}", w.card));
    assert_eq!(a.post_id, answer_id(&w.card, &early.post_id));
    assert_eq!(a.text, FIXED);
    assert!(a.agents_now);
    assert_eq!(a.mentions, vec![w.alice[0].person_actor()]);

    let quiet = PostRow {
        agents_now: false,
        ..question(&w, "fyi, who reviews?")
    };
    send(&w, &quiet, 0);
    answered(&w, b1, &quiet.post_id);
    // Alice reads the same one post, once.
    wait_for(WAIT, "alice reads the answer", || {
        !answers(&w.alice[0], &w.conv, &early.post_id).is_empty()
    });
    assert_eq!(answers(&w.alice[0], &w.conv, &early.post_id).len(), 1);

    // A reply starts no turn, however it is mentioned.
    let before = runs.load(Ordering::SeqCst);
    let reply = PostRow {
        reply_to: Some(early.post_id.clone()),
        ..question(&w, "thanks!")
    };
    send(&w, &reply, 0);
    // The hop limit.
    let hopped = question(&w, "again?");
    send(&w, &hopped, inbox::MAX_HOP);
    control(&w);
    assert!(answers(b1, &w.conv, &reply.post_id).is_empty());
    assert!(answers(b1, &w.conv, &hopped.post_id).is_empty());
    assert_eq!(
        runs.load(Ordering::SeqCst) - before,
        1,
        "only the control ran"
    );

    // A mention in another channel runs nothing.
    let other = w
        .alice[0]
        .core
        .talk_channel_create_in(&w.alice[0].user(), "general", "", &w.space)
        .unwrap();
    wait_for(WAIT, "bob lists it", || {
        b1.core.talk().channel(&other.id).is_some()
    });
    let runs_now = runs.load(Ordering::SeqCst);
    let elsewhere = PostRow {
        conv_id: other.id.clone(),
        ..question(&w, "who reviews?")
    };
    send(&w, &elsewhere, 0);
    control(&w);
    assert_eq!(
        runs.load(Ordering::SeqCst) - runs_now,
        1,
        "only the control ran"
    );
    assert!(answers(b1, &other.id, &elsewhere.post_id).is_empty());
    close(w);
}

/// Questions too old by the asker's clock are held, not answered.
#[test]
fn an_old_question_is_held_and_not_answered() {
    let w = world(1, true);
    let b1 = &w.bob[0];
    let old = PostRow {
        at: GoTime::from_system(SystemTime::now() - Duration::from_secs(2 * 3600)),
        ..question(&w, "who reviews?")
    };
    send(&w, &old, 0);
    control(&w);
    assert!(answers(b1, &w.conv, &old.post_id).is_empty());
    assert!(
        b1.core
            .assistant_held()
            .iter()
            .any(|h| h.post_id == old.post_id && h.text.contains("in time")),
        "{:?}",
        b1.core.assistant_held()
    );
    close(w);
}

/// Closed answers nothing and posts nothing; Pause all and Ask me post the
/// waiting line (`waiting: "ask"`) under the waiting id, and no turn runs.
#[test]
fn closed_posts_nothing_and_ask_me_and_pause_post_the_waiting_line() {
    let w = world(1, false);
    let b1 = &w.bob[0];
    let runs = Arc::new(AtomicUsize::new(0));
    b1.core.set_answerer(Some(fake_turn(&runs)));

    set_receive(b1, &w.space, receive::CLOSED);
    let closed = question(&w, "closed?");
    send(&w, &closed, 0);
    // Closed leaves no trace to wait for: give the bridge time to read it
    // before the setting changes.
    std::thread::sleep(Duration::from_secs(5));
    set_receive(b1, &w.space, receive::ASK);
    let ask = question(&w, "ask me?");
    send(&w, &ask, 0);
    let line = answered(&w, b1, &ask.post_id);
    assert_eq!(line.waiting.as_deref(), Some("ask"));
    assert_eq!(line.post_id, waiting_id(&w.card, &ask.post_id));
    assert!(line.text.starts_with("I've asked"), "{}", line.text);
    assert_eq!(line.by, format!("agent:{}", w.card));
    assert!(answers(b1, &w.conv, &closed.post_id).is_empty());
    assert_eq!(runs.load(Ordering::SeqCst), 0, "Ask me runs no turn");

    // Pause all is treated as Ask me.
    set_receive(b1, &w.space, receive::OPEN);
    let mut pol = b1.core.inbox().policy();
    pol.paused = true;
    b1.core
        .inbox()
        .set_policy(pol.clone(), &format!("user:{}", b1.core.device_id()))
        .unwrap();
    let paused = question(&w, "paused?");
    send(&w, &paused, 0);
    let line = answered(&w, b1, &paused.post_id);
    assert_eq!(line.waiting.as_deref(), Some("ask"));
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    pol.paused = false;
    b1.core
        .inbox()
        .set_policy(pol, &format!("user:{}", b1.core.device_id()))
        .unwrap();
    control(&w);
    assert!(answers(b1, &w.conv, &closed.post_id).is_empty());
    close(w);
}

/// The ninth question in ten minutes from one person isn't answered, and a
/// held line says so.
#[test]
fn the_rate_limit_holds_the_ninth_question() {
    let w = world(1, true);
    let b1 = &w.bob[0];
    let qs: Vec<PostRow> = (0..9).map(|i| question(&w, &format!("q{i}?"))).collect();
    for q in &qs {
        send(&w, q, 0);
    }
    answered(&w, b1, &qs[7].post_id);
    wait_for(WAIT, "the ninth is held", || {
        b1.core.assistant_held().iter().any(|h| h.text.contains("were not answered"))
    });
    assert!(answers(b1, &w.conv, &qs[8].post_id).is_empty());
    close(w);
}

/// Two of Bob's desktops both home answer; Alice's Core shows one post,
/// and Alice's agent, who asked, is woken once.
#[test]
fn two_homes_show_one_post_and_wake_the_asking_agent_once() {
    let w = world(2, true);
    let (a1, b1, b2) = (&w.alice[0], &w.bob[0], &w.bob[1]);
    // Alice's agent asks: its card on the space is minted by its key.
    let agent_key = a1.core.board_key(&w.space, &a1.agent).unwrap();
    let q = PostRow {
        by: agent_key.clone(),
        ..question(&w, "who reviews PR #12?")
    };
    let mut v = serde_json::to_value(&q).unwrap();
    v["by"] = agent_key.clone().into();
    a1.core.space_talk_post(&w.space, &agent_key, v).unwrap();
    a1.core.kick_board(&w.space);
    answered(&w, b1, &q.post_id);
    answered(&w, b2, &q.post_id);
    wait_for(WAIT, "alice reads the answer", || {
        !answers(a1, &w.conv, &q.post_id).is_empty()
    });
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(answers(a1, &w.conv, &q.post_id).len(), 1, "one post");
    let woken = a1
        .core
        .inbox()
        .list(0)
        .into_iter()
        .filter(|x| x.to == a1.agent && x.message.text.contains(FIXED))
        .count();
    assert_eq!(woken, 1, "the asking agent heard it once");
    close(w);
}
