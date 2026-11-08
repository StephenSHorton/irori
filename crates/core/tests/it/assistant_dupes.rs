//! Card #786 job (b) through the Core, with a fake namer for the desktop
//! app's Haiku turn: a new card that looks like the same work as an open
//! one is linked, a card with no candidates costs no turn, an unclear
//! reply links nothing, the switch only turns it off, and a batch or a
//! card already there isn't looked at.

use crate::common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use coord::{JOB_DUPLICATE, NewTask, Task};
use gocompat::time::GoTime;
use irori_core::Core;
use irori_core::dupes::MAX_TURNS;
use irori_core::thread_names::{NEW_FOR_SECS, NameAsk};

fn is_dupe_turn(a: &NameAsk) -> bool {
    a.prompt.starts_with("<new>")
}

#[derive(Default)]
struct Fake {
    asked: Mutex<Vec<NameAsk>>,
    answer: Mutex<String>,
}

impl Fake {
    fn turns(&self) -> usize {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .filter(|a| is_dupe_turn(a))
            .count()
    }
}

fn fake(r: &Rig, answer: &str) -> Arc<Fake> {
    let f = Arc::new(Fake::default());
    *f.answer.lock().unwrap() = answer.to_string();
    let g = f.clone();
    r.core.set_namer(Some(Arc::new(move |a: &NameAsk| {
        g.asked.lock().unwrap().push(a.clone());
        Ok(if is_dupe_turn(a) {
            g.answer.lock().unwrap().clone()
        } else {
            "A thread title".to_string()
        })
    })));
    f
}

fn person(r: &Rig) -> String {
    format!("user:{}", r.core.device_id())
}

fn card(r: &Rig, by: &str, title: &str, objective: &str) -> Task {
    r.core
        .coord()
        .create_task(
            by,
            NewTask {
                title: title.into(),
                objective: objective.into(),
                ..NewTask::default()
            },
        )
        .unwrap()
}

fn links(r: &Rig, id: &str) -> Vec<coord::Did> {
    r.core
        .coord()
        .get_task(id)
        .unwrap()
        .did
        .into_iter()
        .filter(|d| d.job == JOB_DUPLICATE)
        .collect()
}

fn settle() {
    std::thread::sleep(Duration::from_millis(700));
}

const OLD: (&str, &str) = ("Login crash on Safari", "safari login page crashes");
const NEW: (&str, &str) = ("Safari login crashes", "the login page crashes on safari");

#[test]
fn a_clear_verdict_links_the_two_cards() {
    let r = Rig::new();
    assert!(r.core.dupes_on(), "on unless the person turns it off");
    let f = fake(&r, "Both want the Safari crash gone\nDUPLICATE 1");
    let me = person(&r);
    let old = card(&r, &me, OLD.0, OLD.1);
    let new = card(&r, &me, NEW.0, NEW.1);
    wait_for(10, "the link", || links(&r, &new.id).len() == 1);
    assert_eq!(links(&r, &old.id).len(), 1);
    assert_eq!(links(&r, &new.id)[0].key, old.id);
    // Its own records fired more changes; nothing runs again.
    settle();
    assert_eq!(f.turns(), 1);
    assert_eq!(
        r.core.coord().get_task(&new.id).unwrap().state,
        coord::SUBMITTED
    );
    let a = f.asked.lock().unwrap();
    let p = &a.iter().find(|a| is_dupe_turn(a)).unwrap().prompt;
    assert!(p.contains("Login crash on Safari"), "{p}");
    assert!(p.contains("Safari login crashes"), "{p}");
}

#[test]
fn no_candidates_means_no_turn() {
    let r = Rig::new();
    let f = fake(&r, "x\nDUPLICATE 1");
    let me = person(&r);
    card(&r, &me, "Tune the cache eviction", "lower the ttl");
    let new = card(&r, &me, "Write the launch email", "for customers");
    settle();
    assert_eq!(f.turns(), 0);
    assert!(links(&r, &new.id).is_empty());
}

#[test]
fn none_or_an_unclear_reply_links_nothing() {
    for answer in [
        "Different things\nNONE",
        "I think these might be duplicates",
        "",
    ] {
        let r = Rig::new();
        let f = fake(&r, answer);
        let me = person(&r);
        let old = card(&r, &me, OLD.0, OLD.1);
        let new = card(&r, &me, NEW.0, NEW.1);
        wait_for(10, "the turn", || f.turns() >= 1);
        settle();
        assert!(links(&r, &new.id).is_empty(), "{answer}");
        assert!(links(&r, &old.id).is_empty(), "{answer}");
    }
}

#[test]
fn switched_off_it_adds_nothing_and_on_again_it_works() {
    let r = Rig::new();
    let f = fake(&r, "x\nDUPLICATE 1");
    let me = person(&r);
    r.core.set_dupes_on(false).unwrap();
    assert!(!r.core.dupes_on());
    card(&r, &me, OLD.0, OLD.1);
    let new = card(&r, &me, NEW.0, NEW.1);
    settle();
    assert_eq!(f.turns(), 0);
    assert!(links(&r, &new.id).is_empty());
    r.core.set_dupes_on(true).unwrap();
    assert!(r.core.dupes_on(), "the switch holds across a toggle");
    let third = card(
        &r,
        &me,
        "Login crash in Safari",
        "safari login crashes again",
    );
    wait_for(10, "the link", || links(&r, &third.id).len() == 1);
}

#[test]
fn no_namer_on_this_device_acts_on_nothing() {
    let r = Rig::new();
    let me = person(&r);
    card(&r, &me, OLD.0, OLD.1);
    let new = card(&r, &me, NEW.0, NEW.1);
    settle();
    assert!(links(&r, &new.id).is_empty());
}

#[test]
fn undo_stops_it_being_linked_again() {
    let r = Rig::new();
    let f = fake(&r, "x\nDUPLICATE 1");
    let me = person(&r);
    let old = card(&r, &me, OLD.0, OLD.1);
    let new = card(&r, &me, NEW.0, NEW.1);
    wait_for(10, "the link", || links(&r, &new.id).len() == 1);
    let d = links(&r, &new.id)[0].clone();
    r.core.coord().undo_did(&me, &new.id, &d.id).unwrap();
    assert!(links(&r, &old.id)[0].undone && links(&r, &new.id)[0].undone);
    // A change to the card is heard and asks nothing more of the pair.
    r.core.coord().claim_task(&r.key_a, &new.id).unwrap();
    settle();
    assert_eq!(f.turns(), 1);
    assert!(!r.core.coord().link_duplicate(&new.id, &old.id).unwrap());
}

#[test]
fn an_agents_batch_is_not_compared_with_itself() {
    let r = Rig::new();
    let f = fake(&r, "x\nDUPLICATE 1");
    let agent = r.key_a.clone();
    let a = card(&r, &agent, OLD.0, OLD.1);
    let b = card(&r, &agent, NEW.0, NEW.1);
    settle();
    assert_eq!(f.turns(), 0, "one agent's cards, made together");
    assert!(links(&r, &a.id).is_empty() && links(&r, &b.id).is_empty());
}

#[test]
fn a_long_run_of_cards_costs_a_bounded_number_of_turns() {
    let r = Rig::new();
    let f = fake(&r, "x\nNONE");
    let me = person(&r);
    for i in 0..(MAX_TURNS + 4) {
        card(
            &r,
            &me,
            "Login crash on Safari",
            &format!("safari login crashes {i}"),
        );
    }
    wait_for(20, "the turns", || f.turns() >= MAX_TURNS);
    settle();
    assert_eq!(f.turns(), MAX_TURNS);
}

#[test]
fn a_card_there_before_a_restart_is_not_new() {
    let r = Rig::new();
    let me = person(&r);
    let a = card(&r, &me, OLD.0, OLD.1);
    let b = card(&r, &me, NEW.0, NEW.1);
    settle();
    r.core.close().unwrap();
    let again = Core::start(options(&r.data, &r.claude)).unwrap();
    let f = Arc::new(Fake::default());
    *f.answer.lock().unwrap() = "x\nDUPLICATE 1".into();
    let g = f.clone();
    again.set_namer(Some(Arc::new(move |a: &NameAsk| {
        g.asked.lock().unwrap().push(a.clone());
        Ok(g.answer.lock().unwrap().clone())
    })));
    // A later change to a card from before the start doesn't make it new.
    let co = again.coord();
    co.claim_task(&r.key_a, &b.id).ok();
    co.claim_task(&r.key_b, &a.id).ok();
    settle();
    assert_eq!(f.turns(), 0);
    assert!(co.get_task(&b.id).unwrap().did.is_empty());
    again.close().unwrap();
}

#[test]
fn a_card_older_than_new_is_left_alone() {
    let clock = Arc::new(Mutex::new(
        GoTime::from_system(std::time::SystemTime::now()),
    ));
    let c2 = clock.clone();
    let r = Rig::with(move |o| o.now = Some(Arc::new(move || *c2.lock().unwrap())));
    let me = person(&r);
    // No namer: unseen, as a card replicated after a reconnect would be.
    card(&r, &me, OLD.0, OLD.1);
    let b = card(&r, &me, NEW.0, NEW.1);
    settle();
    *clock.lock().unwrap() = {
        let t = *clock.lock().unwrap();
        GoTime::unix(t.unix_nano() / 1_000_000_000 + NEW_FOR_SECS + 1, 0)
    };
    let f = fake(&r, "x\nDUPLICATE 1");
    r.core.coord().claim_task(&r.key_b, &b.id).unwrap();
    settle();
    assert_eq!(f.turns(), 0);
    assert!(links(&r, &b.id).is_empty());
}
