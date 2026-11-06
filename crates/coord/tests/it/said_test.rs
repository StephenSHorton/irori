//! What the person said on the cards around a card (card #555): the one
//! answer a brief, a wake and `show` share.

use crate::common;

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

#[test]
fn a_subtask_or_a_line_naming_the_card_isnt_crowded_out() {
    let e = Env::new();
    let g = e.c.create_task(ALICE, nt("Grandparent")).unwrap();
    let p = sub(&e, &g, "Parent");
    // Subtasks are one level, so the crowd is cards that name it.
    let top = e.c.create_task(ALICE, nt("Top")).unwrap();
    let kid = sub(&e, &top, "Kid");
    say(&e, &kid, "Older words on the subtask");
    let naming = NewTask {
        title: "Names it".into(),
        objective: format!("About {}", top.short_ref()),
        ..NewTask::default()
    };
    for i in 0..4 {
        let o = e.c.create_task(ALICE, naming.clone()).unwrap();
        say(&e, &o, &format!("crowd {i}a"));
        say(&e, &o, &format!("crowd {i}b"));
    }
    let got = texts(&e, &top);
    assert_eq!(got.len(), SAID_MAX);
    assert!(
        got.iter().any(|t| t == "Older words on the subtask"),
        "{got:?}"
    );
    let shown =
        e.c.search_show(&format!("task:{}", top.id), &ShowOptions::default())
            .unwrap();
    assert!(
        shown.text.contains("Older words on the subtask"),
        "{}",
        shown.text
    );

    // On a card that only names it, the naming line beats newer ones.
    let x = e.c.create_task(ALICE, nt("Unrelated work")).unwrap();
    say(
        &e,
        &x,
        &format!("{} must stop saying merge run", p.short_ref()),
    );
    say(&e, &x, "x's own work 1");
    say(&e, &x, "x's own work 2");
    assert!(
        texts(&e, &p).iter().any(|t| t.contains("must stop saying")),
        "{:?}",
        texts(&e, &p)
    );
}

#[test]
fn only_words_folded_as_the_persons_count() {
    let e = Env::new();
    let p = e.c.create_task(ALICE, nt("Parent")).unwrap();
    let a = sub(&e, &p, "A");
    let b = sub(&e, &p, "B");
    // An agent's row that claims to be the person's.
    e.write_raw(
        BOB,
        "task.comment",
        &rows::TaskCommentRow {
            task_id: a.id.clone(),
            comment_id: "forged".into(),
            by: USER.into(),
            text: "FORGED words".into(),
            at: e.clk.now(),
            ..rows::TaskCommentRow::default()
        },
    );
    // It folded under the person's name, so only `person` keeps it out.
    let on_a = e.c.comments(&a.id).unwrap();
    assert!(
        on_a.iter()
            .any(|c| c.text == "FORGED words" && c.by == USER),
        "{on_a:?}"
    );
    assert!(
        e.c.said_near(&b.id).is_empty(),
        "{:?}",
        e.c.said_near(&b.id)
    );

    // A picked option is the agent's text, not the person's words.
    e.c.claim_task(ALICE, &a.id).unwrap();
    let (_, raised) =
        e.c.raise(
            ALICE,
            &a.id,
            NewBlocker {
                question: "Which?".into(),
                options: vec![BlockerOption {
                    answer: "AGENT WORDING drop the done check".into(),
                    ..BlockerOption::default()
                }],
                ..NewBlocker::default()
            },
        )
        .unwrap();
    e.c.answer(
        USER,
        &a.id,
        &raised.id,
        BlockerAnswer {
            pick: 1,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    let on_a = e.c.comments(&a.id).unwrap();
    assert!(
        on_a.iter()
            .any(|c| c.text.starts_with("AGENT WORDING") && c.by == USER && c.pick > 0),
        "{on_a:?}"
    );
    assert!(
        e.c.said_near(&b.id).is_empty(),
        "{:?}",
        e.c.said_near(&b.id)
    );
}

/// The read a wake makes of a board: under the inbox lock it must never
/// wait on a coord, whose writes can take the inbox lock under their own.
#[test]
fn the_non_waiting_read_returns_while_the_log_is_locked() {
    use std::sync::{Arc, Barrier};
    let (inside, go) = (Arc::new(Barrier::new(2)), Arc::new(Barrier::new(2)));
    let (i2, g2) = (inside.clone(), go.clone());
    // The owner check runs under the coord's lock: this key parks there.
    let owner: KeyFn = Arc::new(move |k: &str| {
        if k == "park" {
            i2.wait();
            g2.wait();
        }
        k == USER
    });
    let e = Env::with("dev-a", Some(owner));
    std::thread::scope(|s| {
        let c = &e.c;
        let held = s.spawn(move || c.is_person("park"));
        inside.wait();
        assert!(e.c.try_said_for(BOB, &[], true).is_none(), "it waited");
        go.wait();
        held.join().unwrap();
    });
    assert!(e.c.try_said_for(BOB, &[], true).is_some());
}

#[test]
fn a_wake_finds_the_agents_card_by_a_comment_ref_or_its_working_card() {
    let e = Env::new();
    let p = e.c.create_task(ALICE, nt("Parent")).unwrap();
    let a = sub(&e, &p, "A");
    let b = sub(&e, &p, "B");
    e.c.claim_task(BOB, &b.id).unwrap();
    say(&e, &a, "On the sibling");
    let (cm, _) =
        e.c.person_comment(USER, &b.id, "On its own card", false, &[], None)
            .unwrap();
    let on = |refs: &[String], working: bool, key: &str| e.c.said_for(key, refs, working);

    let got = on(&[format!("comment:{}", cm.id)], false, BOB).unwrap();
    assert_eq!(got.0, b.short_ref());
    assert_eq!(got.1[0].text, "On the sibling");
    // Someone else's card says nothing to it; its working card does.
    assert!(on(&[format!("task:{}", a.id)], false, BOB).is_none());
    assert_eq!(
        on(&[format!("task:{}", a.id)], true, BOB).unwrap().0,
        b.short_ref()
    );
    assert!(on(&[], true, CAROL).is_none());
    // The non-waiting read answers the same on an idle log.
    assert_eq!(
        e.c.try_said_for(BOB, &[], true).unwrap().unwrap().0,
        b.short_ref()
    );
}
