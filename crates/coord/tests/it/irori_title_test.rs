//! Irori's assistant names the person's new thread (card #782): a title
//! row written as Irori, which lands on an open thread, never over a name
//! the person gave, and keeps what it was for Undo.

use crate::common;

use common::two_people::*;
use common::*;
use coord::rows::TaskEditedRow;
use coord::*;
use store::kinds;

fn thread(e: &Env, text: &str) -> Task {
    e.c.post_ask(
        USER,
        NewAsk {
            text: text.into(),
            thread: true,
            ..NewAsk::default()
        },
        &[],
    )
    .unwrap()
    .0
}

#[test]
fn irori_names_an_open_thread_and_it_replays() {
    let e = Env::new();
    let th = thread(
        &e,
        "hey so the login page is broken again on safari i think",
    );
    assert!(e.c.names_here(&th.id), "the device that posted it names it");
    let got =
        e.c.rename_as_irori(&th.id, "Login broken on Safari", &th.title)
            .unwrap();
    assert_eq!(got.title, "Login broken on Safari");
    assert!(!got.title_by_person, "Irori's name isn't the person's");
    assert_eq!(got.irori_titled, 1);
    assert_eq!(got.title_was, th.title, "kept for Undo");
    assert!(got.open_ask().is_some_and(|a| a.thread), "still open");
    let lines = e.c.activity(&th.id).unwrap();
    let line = lines
        .iter()
        .find(|a| a.what.starts_with("renamed it from"))
        .expect("the thread's log says so");
    assert_eq!(line.by, "irori");
    let edited = e.rows(kinds::TASK_EDITED);
    assert_eq!(edited.len(), 1);
    assert_eq!(edited[0].actor, "irori");

    let back = e.reopen().get_task(&th.id).unwrap();
    assert_eq!(back.title, "Login broken on Safari");
    assert_eq!(back.irori_titled, 1);
    assert_eq!(back.title_was, th.title);
    assert!(!back.title_by_person);
}

#[test]
fn irori_retitles_and_keeps_the_newest_was() {
    let e = Env::new();
    let th = thread(&e, "can we look at the build");
    let one =
        e.c.rename_as_irori(&th.id, "Build check", &th.title)
            .unwrap();
    let two =
        e.c.rename_as_irori(&th.id, "Build cache misses on Windows", &one.title)
            .unwrap();
    assert_eq!(two.irori_titled, 2);
    assert_eq!(two.title_was, "Build check");
    // The same title again writes nothing.
    let same =
        e.c.rename_as_irori(&th.id, "Build cache misses on Windows", &two.title)
            .unwrap();
    assert_eq!(same.irori_titled, 2);
    assert_eq!(e.rows(kinds::TASK_EDITED).len(), 2);
}

#[test]
fn irori_never_renames_what_the_person_named() {
    let e = Env::new();
    let th = thread(&e, "the thing with the lobby");
    e.c.rename_task(USER, &th.id, "Lobby").unwrap();
    want_err(
        e.c.rename_as_irori(&th.id, "Lobby join flow", "Lobby"),
        ErrorKind::NotAllowed,
    );
    // Not even a row written behind its back.
    e.write_raw(
        "irori",
        kinds::TASK_EDITED,
        &TaskEditedRow {
            task_id: th.id.clone(),
            title: Some("sneaky".into()),
            by: "irori".into(),
            at: fixed_now(),
            ..TaskEditedRow::default()
        },
    );
    assert_eq!(e.c.get_task(&th.id).unwrap().title, "Lobby");
    assert_eq!(e.reopen().get_task(&th.id).unwrap().title, "Lobby");
}

#[test]
fn a_rename_meanwhile_wins() {
    let e = Env::new();
    let th = thread(&e, "the thing with the lobby");
    // Irori read the title, the person renamed it, then the name came back.
    let read = th.title.clone();
    e.c.rename_task(USER, &th.id, "Lobby").unwrap();
    assert!(e.c.rename_as_irori(&th.id, "Lobby join", &read).is_err());
    assert_eq!(e.c.get_task(&th.id).unwrap().title, "Lobby");
}

#[test]
fn undo_is_the_persons_rename_and_stays() {
    let e = Env::new();
    let th = thread(&e, "the thing with the lobby");
    let named =
        e.c.rename_as_irori(&th.id, "Lobby join", &th.title)
            .unwrap();
    // Undo puts back what it was, as the person.
    let back = e.c.rename_task(USER, &th.id, &named.title_was).unwrap();
    assert_eq!(back.title, th.title);
    assert!(back.title_by_person, "from now on it's theirs");
    want_err(
        e.c.rename_as_irori(&th.id, "Lobby join", &back.title),
        ErrorKind::NotAllowed,
    );
}

#[test]
fn irori_names_only_the_persons_threads() {
    let e = Env::new();
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "lobby join".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    want_err(
        e.c.rename_as_irori(&t.id, "Lobby", "lobby join"),
        ErrorKind::NotAllowed,
    );
    let (ask, _) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "not a thread".into(),
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap();
    want_err(
        e.c.rename_as_irori(&ask.id, "x", &ask.title),
        ErrorKind::NotAllowed,
    );
}

#[test]
fn an_irori_row_carries_a_title_and_nothing_else() {
    let e = Env::new();
    let th = thread(&e, "the thing with the lobby");
    // An Irori row that also changes the words is an edit, held like any
    // other on an open thread: it doesn't land.
    e.write_raw(
        "irori",
        kinds::TASK_EDITED,
        &TaskEditedRow {
            task_id: th.id.clone(),
            title: Some("Lobby".into()),
            objective: Some("rewritten".into()),
            by: "irori".into(),
            at: fixed_now(),
            ..TaskEditedRow::default()
        },
    );
    let got = e.c.get_task(&th.id).unwrap();
    assert_eq!(got.title, th.title);
    assert_eq!(got.objective, th.objective);
    assert_eq!(got.irori_titled, 0);
}

#[test]
fn a_shared_board_takes_no_name_from_irori() {
    let (a, b, _) = two_people();
    let t =
        a.c.create_task(
            CARD1,
            NewTask {
                title: "lobby join".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    want_err(
        a.c.rename_as_irori(&t.id, "Lobby", "lobby join"),
        ErrorKind::NotAllowed,
    );
    sync(&a, &b);
    assert_eq!(b.c.get_task(&t.id).unwrap().title, "lobby join");
}

#[test]
fn someone_elses_rename_leaves_nothing_to_undo() {
    let e = Env::new();
    let th = thread(&e, "the thing with the lobby");
    e.c.rename_as_irori(&th.id, "Lobby join", &th.title)
        .unwrap();
    // The agent that took the thread renames it: the name isn't Irori's
    // now, so there is nothing to undo, and Irori doesn't name it again.
    assert!(e.c.ask_claim(ALICE, &th.id, secs(1)).unwrap().mine);
    e.write_raw(
        &format!("agent:{ALICE}"),
        kinds::TASK_EDITED,
        &TaskEditedRow {
            task_id: th.id.clone(),
            title: Some("Lobby rework".into()),
            by: ALICE.into(),
            at: fixed_now(),
            ..TaskEditedRow::default()
        },
    );
    let got = e.c.get_task(&th.id).unwrap();
    assert_eq!(got.title, "Lobby rework");
    assert!(got.title_was.is_empty(), "no Undo over someone else's name");
    assert_eq!(got.irori_titled, 1);
    assert!(e.reopen().get_task(&th.id).unwrap().title_was.is_empty());
}

#[test]
fn an_irori_name_that_didnt_take_leaves_no_line() {
    let e = Env::new();
    let th = thread(&e, "the thing with the lobby");
    e.c.rename_task(USER, &th.id, "Lobby").unwrap();
    let before = e.c.activity(&th.id).unwrap().len();
    // Irori's row written before it heard of the person's name.
    e.write_raw(
        "irori",
        kinds::TASK_EDITED,
        &TaskEditedRow {
            task_id: th.id.clone(),
            title: Some("Lobby join".into()),
            by: "irori".into(),
            at: fixed_now(),
            ..TaskEditedRow::default()
        },
    );
    let got = e.c.get_task(&th.id).unwrap();
    assert_eq!(got.title, "Lobby");
    assert_ne!(got.updated_by, "irori");
    assert_eq!(e.c.activity(&th.id).unwrap().len(), before, "no Irori line");
}
