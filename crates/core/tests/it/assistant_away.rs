//! Card #786 job (c): "while you were away". A focus after the threshold
//! is a return, measured from persisted attention; one return is one
//! summary; nothing happened means no turn; the switch only turns it off.

use crate::common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use coord::{COMPLETED, NewBlocker, NewTask, PR_MERGED, PrCheck, Task};
use gocompat::time::GoTime;
use irori_core::Core;
use irori_core::away::AWAY_AFTER_SECS;
use irori_core::thread_names::NameAsk;

fn is_away_turn(a: &NameAsk) -> bool {
    a.system.contains("come back")
}

#[derive(Default)]
struct Fake {
    asked: Mutex<Vec<NameAsk>>,
    fail: Mutex<bool>,
}

impl Fake {
    fn turns(&self) -> usize {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .filter(|a| is_away_turn(a))
            .count()
    }
    fn last_prompt(&self) -> String {
        let asked = self.asked.lock().unwrap();
        asked
            .iter()
            .rfind(|a| is_away_turn(a))
            .map(|a| a.prompt.clone())
            .unwrap_or_default()
    }
}

fn install(c: &Core) -> Arc<Fake> {
    let f = Arc::new(Fake::default());
    let g = f.clone();
    c.set_namer(Some(Arc::new(move |a: &NameAsk| {
        g.asked.lock().unwrap().push(a.clone());
        if is_away_turn(a) {
            if *g.fail.lock().unwrap() {
                return Err("signed out".to_string());
            }
            return Ok("#1 is done\n1 card is waiting on you".to_string());
        }
        Ok("A title".to_string())
    })));
    f
}

struct Clock(Arc<Mutex<GoTime>>);

impl Clock {
    fn advance(&self, secs: i64) {
        let mut t = self.0.lock().unwrap();
        *t = GoTime::unix_milli(t.unix_milli_value() + secs * 1000);
    }
}

fn clocked() -> (Rig, Clock) {
    let clock = Arc::new(Mutex::new(
        GoTime::from_system(std::time::SystemTime::now()),
    ));
    let c2 = clock.clone();
    let r = Rig::with(move |o| o.now = Some(Arc::new(move || *c2.lock().unwrap())));
    (r, Clock(clock))
}

fn person(r: &Rig) -> String {
    format!("user:{}", r.core.device_id())
}

fn card(r: &Rig, title: &str, shared: bool) -> Task {
    r.core
        .coord()
        .create_task(
            &person(r),
            NewTask {
                title: title.into(),
                shared,
                ..NewTask::default()
            },
        )
        .unwrap()
}

fn complete(r: &Rig, id: &str) {
    r.core
        .coord()
        .set_task_state(&person(r), id, COMPLETED, "")
        .unwrap();
}

/// Lets the assistant loop finish whatever it was given.
fn settle() {
    std::thread::sleep(Duration::from_millis(300));
}

fn gone_and_back(r: &Rig, c: &Clock, secs: i64) {
    r.core.away_blur();
    c.advance(secs);
    r.core.away_focus();
}

/// One completed card while away, then the return.
fn away_with_a_done_card(r: &Rig, c: &Clock) {
    r.core.away_focus();
    r.core.away_blur();
    c.advance(AWAY_AFTER_SECS / 2);
    let t = card(r, "Ship the login fix", false);
    complete(r, &t.id);
    c.advance(AWAY_AFTER_SECS);
    r.core.away_focus();
}

#[test]
fn coming_back_after_the_threshold_gives_one_summary() {
    let (r, c) = clocked();
    assert!(r.core.away_on(), "on unless the person turns it off");
    let f = install(&r.core);
    away_with_a_done_card(&r, &c);
    wait_for(60, "the turn", || f.turns() == 1);
    assert!(f.last_prompt().contains("Ship the login fix"));
    assert!(f.last_prompt().contains("<completed>"));
    let s = (0..200)
        .find_map(|_| {
            std::thread::sleep(Duration::from_millis(20));
            r.core.away_take()
        })
        .expect("the summary");
    assert_eq!(s.lines, vec!["#1 is done", "1 card is waiting on you"]);
    assert!(r.core.away_take().is_none(), "taken once");
}

#[test]
fn a_flip_within_the_threshold_does_nothing() {
    let (r, c) = clocked();
    let f = install(&r.core);
    r.core.away_focus();
    let t = card(&r, "Ship it", false);
    complete(&r, &t.id);
    gone_and_back(&r, &c, AWAY_AFTER_SECS - 1);
    settle();
    assert_eq!(f.turns(), 0);
    assert!(r.core.away_take().is_none());
}

#[test]
fn a_second_focus_of_one_return_adds_nothing() {
    let (r, c) = clocked();
    let f = install(&r.core);
    away_with_a_done_card(&r, &c);
    r.core.away_focus();
    r.core.away_blur();
    r.core.away_focus();
    wait_for(60, "the turn", || f.turns() == 1);
    settle();
    assert_eq!(f.turns(), 1);
}

#[test]
fn nothing_happened_means_no_turn_and_no_summary() {
    let (r, c) = clocked();
    let f = install(&r.core);
    r.core.away_focus();
    gone_and_back(&r, &c, AWAY_AFTER_SECS * 2);
    settle();
    assert_eq!(f.turns(), 0);
    assert!(r.core.away_take().is_none());
}

#[test]
fn a_failed_turn_shows_nothing() {
    let (r, c) = clocked();
    let f = install(&r.core);
    *f.fail.lock().unwrap() = true;
    away_with_a_done_card(&r, &c);
    wait_for(60, "the turn", || f.turns() == 1);
    settle();
    assert!(r.core.away_take().is_none());
}

#[test]
fn switched_off_it_asks_nothing_and_on_again_it_works() {
    let (r, c) = clocked();
    let f = install(&r.core);
    r.core.set_away_on(false).unwrap();
    assert!(!r.core.away_on());
    away_with_a_done_card(&r, &c);
    settle();
    assert_eq!(f.turns(), 0);
    r.core.set_away_on(true).unwrap();
    away_with_a_done_card(&r, &c);
    wait_for(60, "the turn", || f.turns() == 1);
}

#[test]
fn no_namer_on_this_device_means_no_summary() {
    let (r, c) = clocked();
    away_with_a_done_card(&r, &c);
    settle();
    assert!(r.core.away_take().is_none());
}

#[test]
fn a_shared_card_is_not_mentioned() {
    let (r, c) = clocked();
    let f = install(&r.core);
    r.core.away_focus();
    r.core.away_blur();
    c.advance(10);
    let t = card(&r, "Our shared launch", true);
    complete(&r, &t.id);
    c.advance(AWAY_AFTER_SECS);
    r.core.away_focus();
    settle();
    assert_eq!(f.turns(), 0, "only a shared card happened");
    assert!(r.core.away_take().is_none());
}

#[test]
fn a_pr_merged_while_away_is_mentioned() {
    let (r, c) = clocked();
    let f = install(&r.core);
    let t = card(&r, "Ship", false);
    r.core
        .coord()
        .add_link(&person(&r), &t.id, "https://github.com/a/b/pull/7")
        .unwrap();
    r.core.away_focus();
    r.core.away_blur();
    c.advance(AWAY_AFTER_SECS + 1);
    let merged = PrCheck {
        pr: "https://github.com/a/b/pull/7".into(),
        state: PR_MERGED.into(),
        ..PrCheck::default()
    };
    r.core.coord().record_pr_state(&t.id, merged).unwrap();
    c.advance(1);
    r.core.away_focus();
    wait_for(60, "the turn", || f.turns() == 1);
    assert!(f.last_prompt().contains("PR 7"));
}

#[test]
fn waiting_cards_go_in_only_as_a_count() {
    let (r, c) = clocked();
    let f = install(&r.core);
    let t = card(&r, "Decide the name", false);
    r.core.coord().claim_task(&r.key_a, &t.id).unwrap();
    r.core.away_focus();
    r.core.away_blur();
    c.advance(AWAY_AFTER_SECS + 1);
    r.core
        .coord()
        .raise(
            &r.key_a,
            &t.id,
            NewBlocker {
                question: "Pick the secret word".into(),
                ..NewBlocker::default()
            },
        )
        .unwrap();
    c.advance(1);
    r.core.away_focus();
    wait_for(60, "the turn", || f.turns() == 1);
    let p = f.last_prompt();
    assert!(p.contains("<waiting cards=\"1\"/>"), "{p}");
    assert!(!p.contains("Pick the secret word"), "{p}");
}

#[test]
fn a_wake_alone_is_no_return_and_a_short_sleep_says_nothing() {
    let (r, c) = clocked();
    let f = install(&r.core);
    r.core.away_focus();
    let t = card(&r, "Wrap up", false);
    complete(&r, &t.id);
    r.core.away_blur();
    c.advance(10);
    r.core.away_woke();
    settle();
    assert_eq!(f.turns(), 0, "a wake alone says nothing");
    c.advance(10);
    r.core.away_focus();
    settle();
    assert_eq!(f.turns(), 0, "a short sleep is within the threshold");
}

#[test]
fn a_long_sleep_with_the_window_focused_is_a_return_on_the_next_focus() {
    let (r, c) = clocked();
    let f = install(&r.core);
    r.core.away_focus();
    let t = card(&r, "Wrap up", false);
    complete(&r, &t.id);
    c.advance(AWAY_AFTER_SECS + 1);
    r.core.away_woke();
    settle();
    assert_eq!(f.turns(), 0, "a wake alone says nothing");
    r.core.away_focus();
    wait_for(60, "the turn", || f.turns() == 1);
}

#[test]
fn a_restart_counts_and_does_not_repeat() {
    let clock = Arc::new(Mutex::new(
        GoTime::from_system(std::time::SystemTime::now()),
    ));
    let c2 = clock.clone();
    let now = move || *c2.lock().unwrap();
    let now2 = now.clone();
    let r = Rig::with(move |o| o.now = Some(Arc::new(now.clone())));
    let c = Clock(clock);
    let f = install(&r.core);
    away_with_a_done_card(&r, &c);
    wait_for(60, "the first summary's turn", || f.turns() == 1);
    r.core.close().unwrap();

    // Back soon after the restart: not away, the same return not repeated.
    c.advance(60);
    let mut o = options(&r.data, &r.claude);
    let n = now2.clone();
    o.now = Some(Arc::new(n));
    let again = Core::start(o).unwrap();
    let f2 = install(&again);
    again.away_focus();
    settle();
    assert_eq!(f2.turns(), 0);
    again.away_blur();

    // Away across the next restart: attention on disk counts.
    let co = again.coord();
    let me = format!("user:{}", again.device_id());
    let t = co
        .create_task(
            &me,
            NewTask {
                title: "After the restart".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.set_task_state(&me, &t.id, COMPLETED, "").unwrap();
    again.close().unwrap();
    c.advance(AWAY_AFTER_SECS + 1);
    let mut o = options(&r.data, &r.claude);
    o.now = Some(Arc::new(now2));
    let third = Core::start(o).unwrap();
    let f3 = install(&third);
    third.away_focus();
    wait_for(60, "the summary after a restart", || f3.turns() == 1);
    assert!(f3.last_prompt().contains("After the restart"));
    third.close().unwrap();
}
