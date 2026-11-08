//! Card #786, job (b): the link between two cards that look like the same
//! work. One line on each card, Undo takes both, and the record stops
//! Irori linking them again. The turn that decides is in core's
//! assistant_dupes.

use crate::common;

use common::*;
use coord::*;
use store::kinds;

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

fn make(e: &Env, title: &str) -> Task {
    e.c.create_task(USER, nt(title)).unwrap()
}

fn got(e: &Env, id: &str) -> Task {
    e.c.get_task(id).unwrap()
}

fn links(t: &Task) -> Vec<Did> {
    t.did
        .iter()
        .filter(|d| d.job == JOB_DUPLICATE)
        .cloned()
        .collect()
}

#[test]
fn a_link_is_a_line_on_each_card_naming_the_other() {
    let e = Env::new();
    let a = make(&e, "login crash on safari");
    let b = make(&e, "safari login crashes");
    assert!(e.c.link_duplicate(&a.id, &b.id).unwrap());
    let (la, lb) = (links(&got(&e, &a.id)), links(&got(&e, &b.id)));
    assert_eq!((la.len(), lb.len()), (1, 1));
    assert_eq!(la[0].key, b.id);
    assert_eq!(lb[0].key, a.id);
    assert!(la[0].text.contains(&b.short_ref()), "{}", la[0].text);
    assert!(lb[0].text.contains(&a.short_ref()), "{}", lb[0].text);
    assert_eq!(la[0].by, "irori");
    assert!(did_undoable(&got(&e, &a.id), &la[0]));
}

#[test]
fn a_link_changes_nothing_else_about_either_card() {
    let e = Env::new();
    let a = make(&e, "one");
    let b = make(&e, "two");
    e.c.claim_task(BOB, &b.id).unwrap();
    let (before_a, before_b) = (got(&e, &a.id), got(&e, &b.id));
    e.c.link_duplicate(&a.id, &b.id).unwrap();
    let (after_a, after_b) = (got(&e, &a.id), got(&e, &b.id));
    assert_eq!(after_a.state, before_a.state);
    assert_eq!(after_b.state, before_b.state);
    assert_eq!(after_b.holders, before_b.holders);
    assert!(after_a.blockers.is_empty() && after_b.blockers.is_empty());
    assert!(after_a.links.is_empty());
    assert_eq!(e.rows("assistant.did").len(), 2, "only the two records");
}

#[test]
fn a_restart_keeps_the_link_and_does_not_repeat_it() {
    let e = Env::new();
    let a = make(&e, "one");
    let b = make(&e, "two");
    e.c.link_duplicate(&a.id, &b.id).unwrap();
    let c2 = e.reopen();
    assert_eq!(links(&c2.get_task(&a.id).unwrap()).len(), 1);
    assert!(!c2.link_duplicate(&a.id, &b.id).unwrap());
    assert!(
        !c2.link_duplicate(&b.id, &a.id).unwrap(),
        "either way round"
    );
    assert_eq!(e.rows("assistant.did").len(), 2);
}

#[test]
fn undo_on_either_card_takes_both_and_stops_it_for_good() {
    let e = Env::new();
    let a = make(&e, "one");
    let b = make(&e, "two");
    e.c.link_duplicate(&a.id, &b.id).unwrap();
    let d = links(&got(&e, &b.id))[0].clone();
    e.c.undo_did(USER, &b.id, &d.id).unwrap();
    assert!(links(&got(&e, &a.id))[0].undone);
    assert!(links(&got(&e, &b.id))[0].undone);
    assert!(!did_undoable(&got(&e, &a.id), &links(&got(&e, &a.id))[0]));
    assert!(!e.c.link_duplicate(&a.id, &b.id).unwrap());
    assert!(!e.c.link_duplicate(&b.id, &a.id).unwrap());
    let c2 = e.reopen();
    assert!(!c2.link_duplicate(&a.id, &b.id).unwrap(), "after a restart");
}

#[test]
fn only_the_person_undoes_a_link() {
    let e = Env::new();
    let a = make(&e, "one");
    let b = make(&e, "two");
    e.c.link_duplicate(&a.id, &b.id).unwrap();
    let d = links(&got(&e, &a.id))[0].clone();
    want_err(e.c.undo_did(ALICE, &a.id, &d.id), ErrorKind::NotAllowed);
}

#[test]
fn undo_when_the_other_card_is_gone_still_works() {
    let e = Env::new();
    let a = make(&e, "one");
    let b = make(&e, "two");
    e.c.link_duplicate(&a.id, &b.id).unwrap();
    e.c.delete_task(USER, &b.id).unwrap();
    let d = links(&got(&e, &a.id))[0].clone();
    e.c.undo_did(USER, &a.id, &d.id).unwrap();
    assert!(links(&got(&e, &a.id))[0].undone);
}

fn refused(a: &Task, b: &Task) {
    assert!(!may_link_duplicate(a, b));
    assert!(!may_link_duplicate(b, a));
}

#[test]
fn a_subtask_its_parent_and_its_siblings_are_not_duplicates() {
    let e = Env::new();
    let p = make(&e, "parent");
    let sub = |t: &str| {
        e.c.create_task(
            USER,
            NewTask {
                parent: p.id.clone(),
                ..nt(t)
            },
        )
        .unwrap()
    };
    let (k1, k2) = (sub("kid one"), sub("kid two"));
    refused(&got(&e, &p.id), &got(&e, &k1.id));
    refused(&got(&e, &k1.id), &got(&e, &k2.id));
    assert!(!e.c.link_duplicate(&k1.id, &k2.id).unwrap());
    assert!(e.rows("assistant.did").is_empty());
}

#[test]
fn a_thread_and_the_card_made_from_it_are_not_duplicates() {
    let e = Env::new();
    let (th, _) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "the login is broken".into(),
                thread: true,
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap();
    let card = |t: &str| {
        e.c.create_task(
            USER,
            NewTask {
                from: th.id.clone(),
                ..nt(t)
            },
        )
        .unwrap()
    };
    let (c1, c2) = (card("login one"), card("login two"));
    refused(&got(&e, &th.id), &got(&e, &c1.id));
    refused(&got(&e, &c1.id), &got(&e, &c2.id));
}

#[test]
fn a_card_that_comes_after_another_is_not_its_duplicate() {
    let e = Env::new();
    let a = make(&e, "first");
    let b =
        e.c.create_task(
            USER,
            NewTask {
                after: vec![a.id.clone()],
                ..nt("second")
            },
        )
        .unwrap();
    refused(&got(&e, &a.id), &got(&e, &b.id));
}

#[test]
fn a_closed_or_withdrawn_card_is_not_linked() {
    let e = Env::new();
    let a = make(&e, "open");
    let closed = make(&e, "failed one");
    e.c.set_task_state(USER, &closed.id, FAILED, "no").ok();
    let t = got(&e, &closed.id);
    assert!(is_terminal(&t.state));
    refused(&got(&e, &a.id), &t);
    let mut w = got(&e, &a.id);
    w.withdrawn = true;
    refused(&w, &make(&e, "other"));
}

#[test]
fn different_repos_are_not_duplicates() {
    let e = Env::new();
    let mk = |r: &str| {
        e.c.create_task(
            USER,
            NewTask {
                repo_key: r.into(),
                ..nt("same words")
            },
        )
        .unwrap()
    };
    let (a, b, c) = (
        mk("remote:github.com/a/x"),
        mk("remote:github.com/a/y"),
        mk(""),
    );
    refused(&a, &b);
    refused(&a, &c);
    assert!(may_link_duplicate(&a, &mk("remote:github.com/a/x")));
}

#[test]
fn a_shared_card_or_a_thread_about_a_board_stays_out() {
    let e = Env::new();
    let a = make(&e, "one");
    let mut s = make(&e, "two");
    s.shared = true;
    refused(&a, &s);
    let mut th = make(&e, "three");
    th.ask = Some(Ask {
        board: "b1".into(),
        ..Ask::default()
    });
    refused(&a, &th);
    let mut agents = make(&e, "four");
    agents.ask = Some(Ask {
        opener: ALICE.into(),
        ..Ask::default()
    });
    refused(&a, &agents);
}

#[test]
fn a_shared_board_writes_no_link() {
    let (a, _b, _owners) = common::two_people::two_people();
    let t1 =
        a.c.create_task(common::two_people::P1, nt("board one"))
            .unwrap();
    let t2 =
        a.c.create_task(common::two_people::P1, nt("board two"))
            .unwrap();
    assert!(!a.c.link_duplicate(&t1.id, &t2.id).unwrap());
    assert!(a.c.get_task(&t1.id).unwrap().did.is_empty());
}

#[test]
fn the_row_is_the_assistant_did_kind() {
    let e = Env::new();
    let a = make(&e, "one");
    let b = make(&e, "two");
    e.c.link_duplicate(&a.id, &b.id).unwrap();
    assert!(
        e.rows(kinds::ASSISTANT_DID)
            .iter()
            .all(|r| r.actor == "irori")
    );
}
