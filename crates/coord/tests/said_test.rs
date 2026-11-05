//! What the person said on the cards around a card (card #555): the one
//! answer a brief, a wake and `show` share.

mod common;

use common::*;
use coord::*;
use search::{ShowOptions, Source};

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

fn sub(e: &Env, parent: &Task, title: &str) -> Task {
    e.c.create_task(
        ALICE,
        NewTask {
            title: title.into(),
            parent: parent.short_ref(),
            ..NewTask::default()
        },
    )
    .unwrap()
}

fn say(e: &Env, t: &Task, text: &str) {
    e.c.person_comment(USER, &t.id, text, false, &[], None)
        .unwrap();
}

fn texts(e: &Env, t: &Task) -> Vec<String> {
    e.c.said_near(&t.id).into_iter().map(|s| s.text).collect()
}

#[test]
fn a_sibling_sees_the_persons_words_with_the_cards_ref_and_no_agents() {
    let e = Env::new();
    let p = e.c.create_task(ALICE, nt("Merge desk")).unwrap();
    let a = sub(&e, &p, "Slice 2");
    let b = sub(&e, &p, "Slice 4");
    let far = e.c.create_task(ALICE, nt("Unrelated")).unwrap();
    say(&e, &a, "The desk is a place to talk about PRs");
    e.c.add_comment(BOB, &a.id, "an agent's line", false)
        .unwrap();
    say(&e, &far, "Nothing to do with it");

    let got = e.c.said_near(&b.id);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].text, "The desk is a place to talk about PRs");
    assert_eq!(got[0].card, a.short_ref());
    assert_eq!(got[0].task_id, a.id);
    assert!(!got[0].comment_id.is_empty());
    // Its own thread is in its brief already, so isn't repeated here.
    assert!(e.c.said_near(&a.id).is_empty());
}

#[test]
fn the_parent_keeps_a_subtasks_words_and_a_slice_hears_the_parents() {
    let e = Env::new();
    let p = e.c.create_task(ALICE, nt("Merge desk")).unwrap();
    let a = sub(&e, &p, "Slice 2");
    let b = sub(&e, &p, "Slice 4");
    say(&e, &a, "Said on a slice");
    say(&e, &p, "Said on the parent");

    assert_eq!(texts(&e, &p), ["Said on a slice"]);
    let mut got = texts(&e, &b);
    got.sort();
    assert_eq!(got, ["Said on a slice", "Said on the parent"]);

    // And on the parent as any agent reads it, with the card and comment.
    let shown =
        e.c.search_show(&format!("task:{}", p.id), &ShowOptions::default())
            .unwrap();
    assert!(
        shown.text.contains("the person, on its subtasks")
            && shown.text.contains(&format!("{} comment:", a.short_ref()))
            && shown.text.contains("Said on a slice"),
        "{}",
        shown.text
    );
}

#[test]
fn cards_that_name_it_or_that_it_names_count_and_so_do_waits() {
    let e = Env::new();
    let t = e.c.create_task(ALICE, nt("The card")).unwrap();
    let in_text =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "Names it".into(),
                objective: format!("Follows {}", t.short_ref()),
                ..NewTask::default()
            },
        )
        .unwrap();
    let by_comment = e.c.create_task(ALICE, nt("A comment names it")).unwrap();
    let named = e.c.create_task(ALICE, nt("It names this")).unwrap();
    let t2 =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "Second".into(),
                objective: format!("See {}", named.short_ref()),
                ..NewTask::default()
            },
        )
        .unwrap();
    say(&e, &in_text, "On the card that names it");
    e.c.add_comment(
        BOB,
        &by_comment.id,
        &format!("about {}", t.short_ref()),
        false,
    )
    .unwrap();
    say(&e, &by_comment, "On the card with the comment");
    say(&e, &named, "On the card it names");

    let mut got = texts(&e, &t);
    got.sort();
    assert_eq!(
        got,
        ["On the card that names it", "On the card with the comment"]
    );
    assert_eq!(texts(&e, &t2), ["On the card it names"]);
}

#[test]
fn it_stays_capped_and_a_busy_card_cant_fill_it() {
    let e = Env::new();
    let p = e.c.create_task(ALICE, nt("Parent")).unwrap();
    let me = sub(&e, &p, "Mine");
    let busy = sub(&e, &p, "Busy");
    for i in 0..20 {
        say(&e, &busy, &format!("busy {i}"));
    }
    let got = texts(&e, &me);
    assert_eq!(
        got,
        ["busy 19", "busy 18"],
        "newest, {SAID_PER_CARD} a card"
    );

    let long = "é".repeat(SAID_CHARS * 3);
    for i in 0..8 {
        let s = sub(&e, &p, &format!("Sibling {i}"));
        say(&e, &s, &long);
    }
    let got = e.c.said_near(&me.id);
    assert_eq!(got.len(), SAID_MAX);
    for s in &got {
        assert!(s.text.chars().count() <= SAID_CHARS + 1, "{}", s.text);
        assert!(s.text.ends_with('…'));
    }
}

#[test]
fn only_lately_counts() {
    let e = Env::new();
    let p = e.c.create_task(ALICE, nt("Parent")).unwrap();
    let a = sub(&e, &p, "A");
    let b = sub(&e, &p, "B");
    say(&e, &a, "Old words");
    e.clk.add(secs(SAID_DAYS as u64 * 86_400 + 60));
    assert!(e.c.said_near(&b.id).is_empty());
    say(&e, &a, "New words");
    assert_eq!(texts(&e, &b), ["New words"]);
}
