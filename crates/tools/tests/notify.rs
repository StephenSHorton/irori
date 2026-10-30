//! The notify tool (I:internal/tools/notify_test.go) over a fake card
//! engine: answers come back at the next turn from Irori, once; info cards
//! send none; the 5-per-10-minutes window slides, parallel calls can't
//! beat it and a card that couldn't be shown frees its slot; refusals.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use tools::{Note, Notifier};

#[derive(Default)]
struct Cards {
    all: Mutex<Vec<Note>>,
    off: Mutex<bool>,
    /// Post fails (the card couldn't be shown).
    fail: Mutex<bool>,
}

impl Notifier for Cards {
    fn headless(&self) -> bool {
        false
    }
    fn enabled(&self) -> bool {
        !*self.off.lock().unwrap()
    }
    fn post(&self, note: Note) -> bool {
        if *self.fail.lock().unwrap() {
            return false;
        }
        self.all.lock().unwrap().push(note);
        true
    }
}

/// The answer reaches a caller whose card left the roster meanwhile.
#[test]
fn answer_after_caller_left() {
    let (h, c) = with_cards();
    h.ok(
        &claude(KEY_G, "grokker"),
        "notify",
        r#"{"text":"Ship?","actions":["Yes","No"]}"#,
    );
    h.b.roster.remove(KEY_G);
    (c.last().actions[0].run)();
    let a = answers(&h, KEY_G);
    assert_eq!(a.len(), 1, "{a:?}");
    assert_eq!(
        a[0].message.text,
        "You asked the person: Ship?. They chose: Yes"
    );
}

/// Parallel calls (Claude Code issues tool calls at once) can't beat the
/// limit: the slot is taken when the count is checked.
#[test]
fn rate_limit_parallel() {
    let (h, c) = with_cards();
    let a = claude(KEY_A, "alpha");
    let ok = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..20 {
            s.spawn(|| {
                let out = h.call(&a, "notify", r#"{"text":"card","actions":["Yes","No"]}"#);
                if !out.starts_with("Error: ") {
                    ok.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            });
        }
    });
    let posted = c.all.lock().unwrap().len();
    let ok = ok.into_inner();
    assert!(
        ok == tools::notify::NOTIFY_LIMIT && posted == tools::notify::NOTIFY_LIMIT,
        "{ok} calls accepted and {posted} cards posted; the limit is {}",
        tools::notify::NOTIFY_LIMIT
    );
}

/// A card that couldn't be shown doesn't use up a slot. Go checks the
/// reserve/unreserve bookkeeping directly; here a failing post does it.
#[test]
fn failed_post_frees_slot() {
    let (h, c) = with_cards();
    let a = claude(KEY_A, "alpha");
    for _ in 0..tools::notify::NOTIFY_LIMIT - 1 {
        h.ok(&a, "notify", r#"{"text":"card"}"#);
    }
    *c.fail.lock().unwrap() = true;
    for _ in 0..3 {
        h.fails(
            &a,
            "notify",
            r#"{"text":"not shown"}"#,
            &["couldn't be shown"],
        );
    }
    *c.fail.lock().unwrap() = false;
    h.ok(&a, "notify", r#"{"text":"fits again"}"#);
    h.fails(&a, "notify", r#"{"text":"over"}"#, &["limit"]);
}

impl Cards {
    fn last(&self) -> Note {
        self.all
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("no card was posted")
    }
}

fn answers(h: &Harness, agent: &str) -> Vec<inbox::Delivery> {
    h.b.ib
        .for_agent(agent, true, 50)
        .into_iter()
        .filter(|d| d.message.from.key == inbox::SYSTEM_KEY && d.message.reason == "notify answer")
        .collect()
}

fn with_cards() -> (Harness, Arc<Cards>) {
    let c = Arc::new(Cards::default());
    (new_harness_with(Some(c.clone())), c)
}

#[test]
fn answer_goes_back_next_turn() {
    let (h, c) = with_cards();
    let out = h.ok(
        &call_a(),
        "notify",
        r#"{"text":"The migration drops the users table. Go ahead?","title":"Drop users?","level":"warn","actions":["Go ahead","Stop"]}"#,
    );
    has(&out, &["next turn", r#""Go ahead""#, r#""Stop""#]);
    let card = c.last();
    assert!(card.id.starts_with("ask:"), "{}", card.id);
    assert_eq!(card.title, "alpha: Drop users?");
    assert_eq!(card.level, "warn");
    assert_eq!(card.expire, tools::notify::NOTIFY_ANSWER);
    assert_eq!(card.actions.len(), 2);
    (card.actions[1].run)(); // Stop
    (card.actions[0].run)(); // a second press is ignored
    (card.on_dismiss.unwrap())();
    let a = answers(&h, KEY_A);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].mode, inbox::mode::NEXT_TURN);
    assert!(a[0].message.notice);
    assert_eq!(
        a[0].message.text,
        "You asked the person: Drop users?. They chose: Stop"
    );
}

#[test]
fn dismissed_expired_and_info() {
    let (h, c) = with_cards();
    h.ok(
        &call_a(),
        "notify",
        r#"{"text":"Pick one","actions":["A","B"]}"#,
    );
    (c.last().on_dismiss.unwrap())();
    assert_eq!(
        answers(&h, KEY_A)[0].message.text,
        "You asked the person: Pick one. They dismissed it without choosing."
    );
    h.ok(
        &call_b(),
        "notify",
        r#"{"text":"Merge now?","title":"Merge","actions":["Yes"]}"#,
    );
    (c.last().on_expire.unwrap())();
    assert_eq!(
        answers(&h, KEY_B)[0].message.text,
        "You asked the person: Merge. No answer: the card went away after 5 minutes without a choice."
    );
    let out = h.ok(
        &call_c(),
        "notify",
        r#"{"text":"Deploy finished","level":"success"}"#,
    );
    has(&out, &["No answer comes back"]);
    let card = c.last();
    assert_eq!(
        (card.title.as_str(), card.body.as_str()),
        ("gamma", "Deploy finished")
    );
    assert_eq!(card.expire, Duration::from_secs(12));
    assert!(card.on_dismiss.is_none() && card.actions.is_empty());
    assert!(answers(&h, KEY_C).is_empty());
}

#[test]
fn rate_limit_slides() {
    let (h, _c) = with_cards();
    for _ in 0..tools::notify::NOTIFY_LIMIT {
        h.ok(&call_a(), "notify", r#"{"text":"card"}"#);
        h.clk.advance(Duration::from_secs(60));
    }
    h.fails(
        &call_a(),
        "notify",
        r#"{"text":"one too many"}"#,
        &["5 notification cards", "10 minutes"],
    );
    h.ok(&call_b(), "notify", r#"{"text":"mine"}"#);
    h.clk.advance(Duration::from_secs(5 * 60));
    h.ok(&call_a(), "notify", r#"{"text":"again"}"#);
    h.fails(&call_a(), "notify", r#"{"text":"and again"}"#, &["limit"]);
}

#[test]
fn refusals() {
    let h = new_harness();
    h.fails(&call_a(), "notify", r#"{"text":"hi"}"#, &["headless"]);
    let (h, c) = with_cards();
    *c.off.lock().unwrap() = true;
    h.fails(
        &call_a(),
        "notify",
        r#"{"text":"hi"}"#,
        &["notifications turned off"],
    );
    *c.off.lock().unwrap() = false;
    h.fails(
        &call_a(),
        "notify",
        &format!(r#"{{"text":"{}"}}"#, "x".repeat(501)),
        &["501 characters"],
    );
    h.fails(
        &call_a(),
        "notify",
        &format!(r#"{{"text":"x","title":"{}"}}"#, "t".repeat(81)),
        &["title is at most 80"],
    );
}
