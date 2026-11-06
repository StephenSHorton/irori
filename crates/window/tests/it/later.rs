//! Not yet and order in the window (card #570): a held card's line, a run
//! of cards each after the last, the person's Not yet and Start now, and
//! Share refusing a held card.

use crate::common;

use common::*;
use irori_window::cards::CardSummary;
use irori_window::later::{LaterBack, OrderState};

fn me(r: &Rig) -> String {
    r.w.core.user_sender().key
}

fn card(r: &Rig, title: &str) -> coord::Task {
    r.w.core
        .coord()
        .create_task(
            &me(r),
            coord::NewTask {
                title: title.into(),
                ..Default::default()
            },
        )
        .unwrap()
}

fn summary(r: &Rig, id: &str) -> CardSummary {
    r.w.cards().into_iter().find(|c| c.id == id).unwrap()
}

#[test]
fn a_held_card_says_what_brings_it_back() {
    let r = Rig::new();
    let w = &r.w;
    let t = card(&r, "Voice calls");
    let other = card(&r, "Talk 8");
    assert!(summary(&r, &t.id).later.is_none());

    w.not_yet(&t.id, "", "").unwrap();
    let l = summary(&r, &t.id).later.expect("held");
    assert_eq!(l.back, LaterBack::Word);
    assert_eq!(l.line, "when you say");
    assert!(l.why.ends_with("held it back"), "{l:?}");
    // Agents read it too: never "you held it back".
    assert!(!l.why.to_lowercase().starts_with("you "), "{l:?}");
    assert!(!l.by.is_empty(), "{l:?}");
    // The page carries it too.
    assert_eq!(w.card(&t.id).unwrap().summary.later, Some(l));

    // A change of what brings it back.
    let r_other = format!("#{}", other.short);
    w.not_yet(&t.id, &r_other, "after the Talk work").unwrap();
    let l = summary(&r, &t.id).later.expect("held");
    assert_eq!(l.back, LaterBack::Card);
    assert_eq!(l.line, format!("when {r_other} is done"));
    assert_eq!(l.why, "after the Talk work");
    assert_eq!(l.card, other.id);
    // A hold until a card is no order.
    assert!(summary(&r, &t.id).order.is_none());

    w.not_yet(&t.id, "2099-12-01", "not before then").unwrap();
    let l = summary(&r, &t.id).later.expect("held");
    assert_eq!(
        (l.back, l.line.as_str()),
        (LaterBack::Date, "on 1 Dec 2099")
    );
    let s = summary(&r, &t.id);
    assert_eq!(s.state, coord::BLOCKED);

    // Start now brings it back, and it goes out as if just posted.
    w.start_now(&t.id).unwrap();
    let s = summary(&r, &t.id);
    assert!(s.later.is_none(), "{:?}", s.later);
    assert_eq!(s.state, coord::SUBMITTED);
}

#[test]
fn not_yet_and_start_now_say_what_went_wrong() {
    let r = Rig::new();
    let w = &r.w;
    let t = card(&r, "Tidy Settings");
    let e = w.not_yet(&t.id, "soon", "").unwrap_err();
    assert!(e.contains("a card (#83)"), "{e}");
    let e = w.not_yet(&t.id, "2001-01-01", "").unwrap_err();
    assert!(e.contains("has passed"), "{e}");
    let e = w.not_yet(&t.id, "#9999", "").unwrap_err();
    assert!(!e.is_empty());
    let e = w.start_now(&t.id).unwrap_err();
    assert!(e.contains("isn't held"), "{e}");
    assert!(summary(&r, &t.id).later.is_none());
}

#[test]
fn a_version_hold_names_this_device() {
    let r = Rig::new();
    let w = &r.w;
    let t = card(&r, "Drop the old reader");
    w.not_yet(&t.id, "v99.0.0", "").unwrap();
    let l = summary(&r, &t.id).later.expect("held");
    assert_eq!(l.back, LaterBack::Version);
    assert!(
        l.line.starts_with("when this ") && l.line.ends_with(" runs v99.0.0"),
        "{l:?}"
    );
}

#[test]
fn a_run_of_three_cards_shows_its_order() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let (a, b, c) = (card(&r, "Talk 4"), card(&r, "Talk 5"), card(&r, "Talk 6"));
    co.add_after(&r.key_a, &b.id, &[format!("#{}", a.short)])
        .unwrap();
    co.add_after(&r.key_a, &c.id, &[format!("#{}", b.short)])
        .unwrap();

    let ids = |s: &CardSummary| -> Vec<String> {
        s.order
            .as_ref()
            .expect("an order")
            .steps
            .iter()
            .map(|x| x.id.clone())
            .collect()
    };
    let want = vec![a.id.clone(), b.id.clone(), c.id.clone()];
    for t in [&a, &b, &c] {
        assert_eq!(ids(&summary(&r, &t.id)), want, "{}", t.title);
    }
    let o = summary(&r, &b.id).order.unwrap();
    assert_eq!(o.previous.unwrap().id, a.id);
    let next = o.next.unwrap();
    assert_eq!(
        (next.id.as_str(), next.short.as_str(), next.title.as_str()),
        (c.id.as_str(), c.short.as_str(), "Talk 6")
    );
    let o = summary(&r, &a.id).order.unwrap();
    assert!(o.previous.is_none());
    assert_eq!(o.next.unwrap().id, b.id);
    let o = summary(&r, &c.id).order.unwrap();
    assert!(o.next.is_none());
    assert_eq!(o.steps[2].state, OrderState::Waiting);
    // The page reads it the same way.
    assert_eq!(
        r.w.card(&b.id).unwrap().summary.order.unwrap().steps.len(),
        3
    );

    // A held step reads as later.
    r.w.not_yet(&c.id, "", "").unwrap();
    let o = summary(&r, &a.id).order.unwrap();
    assert_eq!(o.steps[2].state, OrderState::Later);

    // Once the first is done it stays at the head, done.
    co.set_task_state(&me(&r), &a.id, coord::COMPLETED, "done")
        .unwrap();
    wait("b's wait on a ends", || {
        r.w.core
            .coord()
            .get_task(&b.id)
            .is_some_and(|t| t.after_cards().is_empty())
    });
    let o = summary(&r, &b.id).order.expect("still in order");
    assert_eq!(o.steps[0].id, a.id);
    assert_eq!(o.steps[0].state, OrderState::Done);
    assert_eq!(o.previous.unwrap().id, a.id);
}

#[test]
fn a_loop_shows_as_a_loop_not_an_order() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let (a, b) = (card(&r, "One"), card(&r, "Two"));
    co.add_after(&r.key_a, &b.id, &[format!("#{}", a.short)])
        .unwrap();
    // This device refuses the circle.
    let e = co
        .add_after(&r.key_a, &a.id, &[format!("#{}", b.short)])
        .unwrap_err();
    assert!(e.to_string().contains("circle"), "{e}");
    assert!(summary(&r, &a.id).wait_loop.is_empty());
}

#[test]
fn share_refuses_a_held_card() {
    let r = Rig::new();
    let w = &r.w;
    let mlg = w.boards_create("MLG", "MLG").unwrap();
    let t = card(&r, "Voice calls");
    w.not_yet(&t.id, "", "").unwrap();
    let e = w.boards_share_preview(&t.id, &mlg.board_id).unwrap_err();
    assert!(e.contains("held back as Not yet"), "{e}");
    let e = w.boards_share(&t.id, &mlg.board_id, "x").unwrap_err();
    assert!(e.contains("held back as Not yet"), "{e}");

    // A parent with a held subtask can't go either.
    let co = w.core.coord();
    let p = card(&r, "Talk");
    let (_, kids) = co
        .split(
            &me(&r),
            &p.id,
            vec![coord::NewSubtask {
                title: "Talk 8".into(),
                ..Default::default()
            }],
            vec![],
        )
        .unwrap();
    w.not_yet(&kids[0].id, "", "").unwrap();
    let e = w.boards_share_preview(&p.id, &mlg.board_id).unwrap_err();
    assert!(e.contains("its subtask") && e.contains("Not yet"), "{e}");

    // Brought back, it shares.
    w.start_now(&t.id).unwrap();
    w.boards_share_preview(&t.id, &mlg.board_id).unwrap();
}

#[test]
fn holding_back_takes_the_agent_off_and_only_the_person_starts_it() {
    let r = Rig::new();
    let w = &r.w;
    let co = w.core.coord();
    let t = card(&r, "Voice calls");
    co.claim_task(&r.key_a, &t.id).unwrap();
    assert!(!summary(&r, &t.id).working.is_empty());

    w.not_yet(&t.id, "", "").unwrap();
    let s = summary(&r, &t.id);
    assert!(s.working.is_empty(), "{:?}", s.working);
    assert_eq!(s.later.map(|l| l.back), Some(LaterBack::Word));
    // No agent takes it or brings it back: it waits on the person's word.
    assert!(co.claim_task(&r.key_a, &t.id).is_err());
    let e = co.bring_back(&r.key_a, &t.id, "go").unwrap_err();
    assert!(e.to_string().contains("person"), "{e}");
    assert!(summary(&r, &t.id).later.is_some());

    w.start_now(&t.id).unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap();
}
