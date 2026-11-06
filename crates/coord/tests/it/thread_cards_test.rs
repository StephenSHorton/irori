//! Cards made from a thread (card #155): board_create's `from` links the
//! card to the person's thread both ways, and an agent that answered one
//! open thread gets it by default.

use crate::common;

use common::*;
use coord::roster::AgentCard;
use coord::*;

fn agents(e: &Env) -> Vec<AgentCard> {
    use coord::roster::Agents as _;
    e.ros.agents()
}

fn thread(e: &Env, text: &str) -> Task {
    e.c.post_ask(
        USER,
        NewAsk {
            text: text.into(),
            thread: true,
            ..NewAsk::default()
        },
        &agents(e),
    )
    .unwrap()
    .0
}

fn card(e: &Env, by: &str, title: &str, from: &str) -> Result<Task> {
    e.c.create_task(
        by,
        NewTask {
            title: title.into(),
            from: from.into(),
            ..NewTask::default()
        },
    )
}

fn lines(e: &Env, id: &str) -> Vec<Activity> {
    e.c.activity(id).unwrap_or_default()
}

#[test]
fn from_links_the_card_and_the_thread() {
    let e = Env::new();
    let th = thread(&e, "Can the lobby show who's typing?");
    e.c.add_comment(ALICE, &th.id, "Yes: a card for it is coming.", false)
        .unwrap();
    let t = card(
        &e,
        ALICE,
        "Typing dots in the lobby",
        &format!("#{}", th.short),
    )
    .unwrap();
    assert_eq!(t.from, th.id);
    let th2 = e.c.get_task(&th.id).unwrap();
    assert_eq!(th2.made, vec![t.id.clone()]);
    let made: Vec<_> = lines(&e, &th.id)
        .into_iter()
        .filter(|a| a.kind == MADE_KIND)
        .collect();
    assert!(
        made.len() == 1
            && made[0].by == ALICE
            && made[0].what == format!("made #{} from this thread", t.short),
        "{made:?}"
    );
    assert!(
        lines(&e, &t.id)
            .iter()
            .any(|a| a.what == format!("put the card up from the thread #{}", th.short)),
        "{:?}",
        lines(&e, &t.id)
    );
}

#[test]
fn from_must_name_a_thread() {
    let e = Env::new();
    let other = card(&e, ALICE, "Not a thread", "").unwrap();
    want_err(
        card(&e, ALICE, "From a card", &format!("#{}", other.short)),
        ErrorKind::Invalid,
    );
}

#[test]
fn an_agent_in_one_open_thread_gets_it_by_default() {
    let e = Env::new();
    e.ros.set(ALICE, "busy");
    e.ros.set(CAROL, "busy");
    let th = thread(&e, "Is the relay up?");
    // Handed to bob (the idle one): his card comes from it already, once
    // he has answered there (card #322).
    want_err(card(&e, BOB, "Check the relay", ""), ErrorKind::NotAllowed);
    e.c.add_comment(BOB, &th.id, "I'll check it.", false)
        .unwrap();
    let t = card(&e, BOB, "Check the relay", "").unwrap();
    assert_eq!(t.from, th.id);
    assert_eq!(e.c.get_task(&th.id).unwrap().made, vec![t.id.clone()]);
    // Carol neither holds nor answered it: no default.
    let c0 = card(&e, CAROL, "Unrelated", "").unwrap();
    assert!(c0.from.is_empty(), "{c0:?}");
    // Bob answered: the thread nobody holds now still counts for him.
    e.c.sweep_asks(&agents(&e)).unwrap();
    let t1 = card(&e, BOB, "Check the relay again", "").unwrap();
    assert_eq!(t1.from, th.id);

    // The person never gets a default.
    let p = card(&e, USER, "The person's card", "").unwrap();
    assert!(p.from.is_empty(), "{p:?}");

    // A subtask comes from its parent, not the thread.
    let sub =
        e.c.create_task(
            BOB,
            NewTask {
                title: "Ping it".into(),
                parent: t.id.clone(),
                ..NewTask::default()
            },
        )
        .unwrap();
    assert!(sub.from.is_empty(), "{sub:?}");

    // Two open threads answered: ambiguous, so none.
    let th2 = thread(&e, "And the lobby?");
    e.c.add_comment(BOB, &th2.id, "Looking.", false).unwrap();
    e.c.sweep_asks(&agents(&e)).unwrap();
    let t2 = card(&e, BOB, "Which one?", "").unwrap();
    assert!(t2.from.is_empty(), "{t2:?}");

    // Resolved threads don't count.
    e.c.resolve_thread(USER, &th.id).unwrap();
    let t3 = card(&e, BOB, "Lobby fix", "").unwrap();
    assert_eq!(t3.from, th2.id);
}

#[test]
fn an_edit_from_a_thread_says_so_in_the_thread() {
    let e = Env::new();
    e.ros.set(ALICE, "busy");
    e.ros.set(CAROL, "busy");
    let t = card(&e, BOB, "Relay retries", "").unwrap();
    let th = thread(&e, "Should the relay retry forever?");
    e.c.add_comment(BOB, &th.id, "No, three times.", false)
        .unwrap();
    e.c.edit_task(
        BOB,
        &t.id,
        TaskEdit {
            objective: Some("Retry three times, then say so.".into()),
            ..TaskEdit::default()
        },
    )
    .unwrap();
    assert_eq!(e.c.get_task(&th.id).unwrap().made, vec![t.id.clone()]);
    assert!(
        lines(&e, &th.id)
            .iter()
            .any(|a| a.kind == MADE_KIND
                && a.what == format!("changed #{} from this thread", t.short)),
        "{:?}",
        lines(&e, &th.id)
    );
    assert!(
        lines(&e, &t.id).iter().any(|a| a
            .what
            .ends_with(&format!("(from the thread #{})", th.short))),
        "{:?}",
        lines(&e, &t.id)
    );
}

/// Card #318: the holder edits from a thread someone else took (or nobody
/// did). `from` links the thread; it never holds the edit back.
#[test]
fn a_holder_edits_from_a_thread_it_did_not_take() {
    let e = Env::new();
    e.ros.set(BOB, "busy");
    e.ros.set(CAROL, "busy");
    // Alice is the idle one: the thread is hers.
    let th = thread(&e, "Rename the relay card?");
    assert_eq!(th.ask.as_ref().unwrap().taker, ALICE, "{th:?}");
    let t = card(&e, BOB, "Relay retries", "").unwrap();
    assert!(t.from.is_empty(), "{t:?}");

    // Explicit from: the thread Bob didn't take. He answers there first
    // (card #322); until then the edit is refused, never silently held.
    want_err(
        e.c.edit_task(
            BOB,
            &t.id,
            TaskEdit {
                title: Some("Relay: retry three times".into()),
                from: format!("#{}", th.short),
                ..TaskEdit::default()
            },
        ),
        ErrorKind::NotAllowed,
    );
    e.c.add_comment(BOB, &th.id, "On it.", false).unwrap();
    let got =
        e.c.edit_task(
            BOB,
            &t.id,
            TaskEdit {
                title: Some("Relay: retry three times".into()),
                objective: Some("Retry three times, then say so.".into()),
                done_when: Some("A dropped relay retries 3x.".into()),
                from: format!("#{}", th.short),
            },
        )
        .unwrap();
    let now = e.c.get_task(&t.id).unwrap();
    assert_eq!(now.title, "Relay: retry three times");
    assert_eq!(now.objective, "Retry three times, then say so.");
    assert_eq!(now.done_criteria, "A dropped relay retries 3x.");
    assert_eq!(got.title, now.title);
    assert!(
        lines(&e, &t.id)
            .iter()
            .any(|a| a.what.contains("renamed it from")),
        "{:?}",
        lines(&e, &t.id)
    );
    assert!(
        e.c.get_task(&th.id).unwrap().made.contains(&t.id),
        "the thread still lists the card"
    );

    // No from: Bob answered the thread, so it is the default, and the
    // edit still lands.
    e.c.edit_task(
        BOB,
        &t.id,
        TaskEdit {
            title: Some("Relay: retry, then report".into()),
            ..TaskEdit::default()
        },
    )
    .unwrap();
    assert_eq!(
        e.c.get_task(&t.id).unwrap().title,
        "Relay: retry, then report"
    );

    // Someone who doesn't manage the card still can't, thread or not.
    want_err(
        e.c.edit_task(
            CAROL,
            &t.id,
            TaskEdit {
                title: Some("mine".into()),
                from: format!("#{}", th.short),
                ..TaskEdit::default()
            },
        ),
        ErrorKind::NotAllowed,
    );
}

/// Card #322: an agent that makes a card from the person's thread answers
/// there first, so the person sees its answer and can resolve the thread.
#[test]
fn a_card_from_a_thread_waits_for_the_agents_answer() {
    let e = Env::new();
    let other = card(&e, ALICE, "Board page", "").unwrap();
    let th = thread(&e, "Hide the board settings behind a cog?");
    let r = format!("#{}", th.short);
    // Not answered yet: refused, and the thread lists no card.
    let err = card(&e, ALICE, "Settings behind a cog", &r).unwrap_err();
    assert!(err.is(ErrorKind::NotAllowed), "{err}");
    assert!(
        err.to_string().contains("haven't answered") && err.to_string().contains("board_comment"),
        "{err}"
    );
    assert!(e.c.get_task(&th.id).unwrap().made.is_empty());

    // A board_edit from it is refused the same way.
    want_err(
        e.c.edit_task(
            ALICE,
            &other.id,
            TaskEdit {
                objective: Some("Settings in a dialog.".into()),
                from: r.clone(),
                ..TaskEdit::default()
            },
        ),
        ErrorKind::NotAllowed,
    );

    // Answered: the card is made, and the thread shows the answer and it.
    e.c.add_comment(ALICE, &th.id, "Yes: I'll move them into a dialog.", false)
        .unwrap();
    let t = card(&e, ALICE, "Settings behind a cog", &r).unwrap();
    assert_eq!(t.from, th.id);
    assert_eq!(e.c.get_task(&th.id).unwrap().made, vec![t.id.clone()]);
    let cs = e.c.comments(&th.id).unwrap();
    assert!(
        cs.iter()
            .any(|c| c.by == ALICE && c.text.contains("dialog")),
        "{cs:?}"
    );
    assert!(
        lines(&e, &th.id)
            .iter()
            .any(|a| a.kind == MADE_KIND && a.what == format!("made #{} from this thread", t.short)),
        "{:?}",
        lines(&e, &th.id)
    );

    // The person never needs to answer their own thread.
    let p = card(&e, USER, "The person's own card", &r).unwrap();
    assert_eq!(p.from, th.id);

    // Once resolved, a card may still name it as where it came from.
    let th2 = thread(&e, "And the people panel?");
    e.c.resolve_thread(USER, &th2.id).unwrap();
    let late = card(&e, BOB, "People panel", &format!("#{}", th2.short)).unwrap();
    assert_eq!(late.from, th2.id);
}
