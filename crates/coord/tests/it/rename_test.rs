//! The person renames a card or thread by hand (card #779): the same
//! task.edited row an agent's board_edit writes, and from then on an
//! agent's rename doesn't land, here or from another device.

use crate::common;

use common::two_people::*;
use common::*;
use coord::rows::TaskEditedRow;
use coord::*;
use store::kinds;

fn card(e: &Env, title: &str) -> Task {
    e.c.create_task(
        ALICE,
        NewTask {
            title: title.into(),
            objective: "do it".into(),
            ..NewTask::default()
        },
    )
    .unwrap()
}

fn title_edit(title: &str) -> TaskEdit {
    TaskEdit {
        title: Some(title.into()),
        ..TaskEdit::default()
    }
}

fn title_of(c: &Coord, id: &str) -> String {
    c.get_task(id).unwrap().title
}

#[test]
fn the_person_renames_a_card_and_it_stays_after_a_restart() {
    let e = Env::new();
    let t = card(&e, "lobby join");
    let got = e.c.rename_task(USER, &t.id, "  Lobby join flow  ").unwrap();
    assert_eq!(got.title, "Lobby join flow", "trimmed");
    assert!(got.title_by_person);
    // The card's log says so, as for an agent's rename.
    let lines = e.c.activity(&t.id).unwrap();
    assert!(
        lines.iter().any(|a| a
            .what
            .contains("renamed it from “lobby join” to “Lobby join flow”")),
        "{lines:?}"
    );
    // It is the agents' row kind.
    let edited = e.rows(kinds::TASK_EDITED);
    assert_eq!(edited.len(), 1);
    assert_eq!(edited[0].actor, "user");

    let again = e.reopen();
    let back = again.get_task(&t.id).unwrap();
    assert_eq!(back.title, "Lobby join flow");
    assert!(back.title_by_person, "the flag replays");
}

#[test]
fn only_the_person_renames_through_it() {
    let e = Env::new();
    let t = card(&e, "lobby join");
    want_err(e.c.rename_task(ALICE, &t.id, "mine"), ErrorKind::NotAllowed);
    want_err(e.c.rename_task(USER, &t.id, "   "), ErrorKind::Invalid);
    want_err(
        e.c.rename_task(USER, &t.id, &"x".repeat(201)),
        ErrorKind::Invalid,
    );
    want_err(e.c.rename_task(USER, "nope", "x"), ErrorKind::NotFound);
    assert_eq!(title_of(&e.c, &t.id), "lobby join");
    // The same name writes nothing.
    e.c.rename_task(USER, &t.id, "lobby join").unwrap();
    assert!(e.rows(kinds::TASK_EDITED).is_empty());
}

#[test]
fn the_person_renames_an_open_thread_and_a_done_card() {
    let e = Env::new();
    let (th, _) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "can I rename threads by hand?".into(),
                thread: true,
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap();
    // board_edit refuses an open thread; the person's rename doesn't.
    let got = e.c.rename_task(USER, &th.id, "Renaming threads").unwrap();
    assert_eq!(got.title, "Renaming threads");
    assert!(
        got.open_ask().is_some_and(|a| a.thread),
        "still an open thread"
    );

    let t = card(&e, "lobby join");
    e.c.set_task_state(USER, &t.id, CANCELED, "not now")
        .unwrap();
    let got =
        e.c.rename_task(USER, &t.id, "Lobby join (dropped)")
            .unwrap();
    assert_eq!(got.title, "Lobby join (dropped)");
    assert_eq!(got.state, CANCELED, "renaming doesn't reopen it");
}

#[test]
fn an_agent_doesnt_rename_what_the_person_named() {
    let e = Env::new();
    let t = card(&e, "lobby join");
    // Before the person names it, an agent renames it as ever.
    e.c.edit_task(ALICE, &t.id, title_edit("lobby join v2"))
        .unwrap();
    e.c.rename_task(USER, &t.id, "Join the lobby").unwrap();

    // A rename alone is refused, and says why.
    let err =
        e.c.edit_task(BOB, &t.id, title_edit("lobby join v3"))
            .unwrap_err();
    assert!(err.is(ErrorKind::NotAllowed), "{err}");
    assert!(err.to_string().contains("the person named"), "{err}");
    assert_eq!(title_of(&e.c, &t.id), "Join the lobby");

    // With other changes too it is refused, so nothing lands unsaid; the
    // same changes without the title land.
    want_err(
        e.c.edit_task(
            ALICE,
            &t.id,
            TaskEdit {
                title: Some("lobby join v3".into()),
                objective: Some("join from the lobby".into()),
                ..TaskEdit::default()
            },
        ),
        ErrorKind::NotAllowed,
    );
    let got =
        e.c.edit_task(
            ALICE,
            &t.id,
            TaskEdit {
                objective: Some("join from the lobby".into()),
                ..TaskEdit::default()
            },
        )
        .unwrap();
    assert_eq!(got.title, "Join the lobby");
    assert_eq!(got.objective, "join from the lobby");

    // Giving the name it already has is no change.
    e.c.edit_task(ALICE, &t.id, title_edit("Join the lobby"))
        .unwrap();

    // The person renames it again whenever they like.
    e.c.rename_task(USER, &t.id, "Lobby").unwrap();
    assert_eq!(title_of(&e.c, &t.id), "Lobby");
    assert_eq!(title_of(&e.reopen(), &t.id), "Lobby");
}

#[test]
fn an_agents_rename_from_another_device_doesnt_land_either() {
    let e = Env::new();
    let t = card(&e, "lobby join");
    e.c.rename_task(USER, &t.id, "Join the lobby").unwrap();
    let far = "dev-b/far-session";
    e.remote(
        "dev-b",
        1,
        hlc_at(plus(fixed_now(), mins(1))),
        &format!("agent:{far}"),
        kinds::TASK_EDITED,
        &TaskEditedRow {
            task_id: t.id.clone(),
            title: Some("far rename".into()),
            objective: Some("from afar".into()),
            by: far.into(),
            at: fixed_now(),
            ..TaskEditedRow::default()
        },
    );
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.title, "Join the lobby", "the name stays");
    let replay = e.reopen().get_task(&t.id).unwrap();
    assert_eq!(replay.title, "Join the lobby");
    assert_eq!(replay.objective, got.objective, "both folds agree");
}

#[test]
fn on_a_shared_board_both_people_see_the_name_and_agents_leave_it() {
    let (mut a, b, _) = two_people();
    let t =
        a.c.create_task(
            CARD1,
            NewTask {
                title: "lobby join".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    sync(&a, &b);
    // An agent isn't a person here either.
    want_err(a.c.rename_task(CARD1, &t.id, "x"), ErrorKind::NotAllowed);
    a.c.rename_task(P1, &t.id, "Join the lobby").unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let got = s.c.get_task(&t.id).unwrap();
        assert_eq!(got.title, "Join the lobby");
        assert!(got.title_by_person);
    }
    // p2's agent can't rename it, from either side.
    want_err(
        b.c.edit_task(CARD2, &t.id, title_edit("agent rename")),
        ErrorKind::NotAllowed,
    );
    // The other person can.
    b.c.rename_task(P2, &t.id, "Lobby").unwrap();
    sync(&a, &b);
    a.restart();
    for s in [&a, &b] {
        assert_eq!(title_of(&s.c, &t.id), "Lobby");
        assert_eq!(s.c.ignored_rows(), 0);
    }
}

#[test]
fn closing_an_ask_doesnt_rename_what_the_person_named() {
    let e = Env::new();
    let t = card(&e, "lobby join");
    e.c.rename_task(USER, &t.id, "Join the lobby").unwrap();
    let (ask, _) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "rename the lobby card".into(),
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap();
    assert!(e.c.ask_claim(ALICE, &ask.id, secs(1)).unwrap().mine);
    // An edit outcome that renames it is refused, saying why.
    let err =
        e.c.ask_close(
            ALICE,
            &ask.id,
            AskOutcome {
                kind: "edit".into(),
                task: t.id.clone(),
                title: "lobby v2".into(),
                note: "renamed".into(),
                ..AskOutcome::default()
            },
        )
        .unwrap_err();
    assert!(err.is(ErrorKind::NotAllowed), "{err}");
    assert_eq!(title_of(&e.c, &t.id), "Join the lobby");
    assert!(
        e.c.get_task(&ask.id).unwrap().open_ask().is_some(),
        "still open"
    );

    // The person renames an open ask on its page; its taker can't close it
    // as a card under another name.
    e.c.rename_task(USER, &ask.id, "Lobby rename").unwrap();
    want_err(
        e.c.ask_close(
            ALICE,
            &ask.id,
            AskOutcome {
                kind: "card".into(),
                title: "Something else".into(),
                note: "on it".into(),
                ..AskOutcome::default()
            },
        ),
        ErrorKind::NotAllowed,
    );
    e.c.ask_close(
        ALICE,
        &ask.id,
        AskOutcome {
            kind: "card".into(),
            title: "Lobby rename".into(),
            note: "on it".into(),
            ..AskOutcome::default()
        },
    )
    .unwrap();
    assert_eq!(title_of(&e.c, &ask.id), "Lobby rename");
}

#[test]
fn a_card_made_with_the_persons_name_keeps_it_only_from_the_person() {
    let e = Env::new();
    let named = NewTask {
        title: "Join the lobby".into(),
        title_by_person: true,
        ..NewTask::default()
    };
    // A share or a move makes the card again as the person, keeping it.
    let t = e.c.create_task(USER, named.clone()).unwrap();
    assert!(t.title_by_person);
    assert!(e.reopen().get_task(&t.id).unwrap().title_by_person);
    want_err(
        e.c.edit_task(ALICE, &t.id, title_edit("x")),
        ErrorKind::NotAllowed,
    );
    // An agent can't claim it.
    let u = e.c.create_task(ALICE, named).unwrap();
    assert!(!u.title_by_person);
}
