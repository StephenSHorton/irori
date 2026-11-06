//! Threads (card #116): the person's conversation with the room. The
//! agents on the cards it names hear it, woken, and nobody waits on them
//! (card #512): it goes out at once to one idle agent, or with none free
//! it is unclaimed (and Irori starts its agent); once answered nobody holds it, follow-ups go to the
//! agents that answered, and only the person resolves it.

use crate::common;

use common::*;
use coord::roster::AgentCard;
use coord::*;

fn agents(e: &Env) -> Vec<AgentCard> {
    use coord::roster::Agents as _;
    e.ros.agents()
}

fn thread(e: &Env, text: &str) -> (Task, Notice) {
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
}

fn ask_of(c: &Coord, id: &str) -> Ask {
    c.get_task(id).and_then(|t| t.ask).unwrap()
}

/// A card alice holds and is busy on.
fn alices_card(e: &Env) -> Task {
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "lobby join".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.ros.set(ALICE, "busy");
    e.c.claim_task(ALICE, &t.id).unwrap()
}

#[test]
fn a_thread_goes_to_the_cards_it_names_first() {
    let e = Env::new();
    let card = alices_card(&e);
    let (tk, n) = thread(&e, &format!("Is #{} still moving?", card.short));
    let a = ask_of(&e.c, &tk.id);
    assert!(
        a.thread && a.taker.is_empty() && a.heard == [ALICE],
        "{a:?}"
    );
    assert!(tk.holders.is_empty(), "nobody holds a thread");
    assert!(
        n.to == [ALICE]
            && n.wake
            && n.text.contains("started a thread")
            && n.text.contains("Nobody waits on you"),
        "{n:?}"
    );

    // Nobody waits on alice (card #512): the first sweep hands it out at
    // once, and with nobody free it is unclaimed, so Irori starts its own
    // agent. Alice still heard it.
    e.ros.set(BOB, "busy");
    e.ros.set(CAROL, "busy");
    assert!(e.c.sweep_asks(&agents(&e)).unwrap().is_empty());
    let a = ask_of(&e.c, &tk.id);
    assert!(
        a.unclaimed && a.taker.is_empty() && a.heard == [ALICE],
        "{a:?}"
    );

    // Alice answers: answered, still nobody holds it, and no hand-out.
    e.c.add_comment(
        ALICE,
        &tk.id,
        "Waiting on your call about the relay.",
        false,
    )
    .unwrap();
    assert!(e.c.sweep_asks(&agents(&e)).unwrap().is_empty());
    let a = ask_of(&e.c, &tk.id);
    assert!(
        a.reason == "answered" && a.taker.is_empty() && !a.unclaimed,
        "{a:?}"
    );
    e.clk.add(secs(60));
    assert!(e.c.sweep_asks(&agents(&e)).unwrap().is_empty());
    assert!(e.c.get_task(&tk.id).unwrap().holders.is_empty());

    // The person's follow-up goes to alice, woken.
    let (_, ns) =
        e.c.person_comment(USER, &tk.id, "Our relay.", true, &[], None)
            .unwrap();
    assert!(
        ns.len() == 1 && ns[0].to == [ALICE] && ns[0].wake && ns[0].text.contains("Our relay."),
        "{ns:?}"
    );

    // Only the person resolves it.
    want_err(
        e.c.ask_close(
            ALICE,
            &tk.id,
            AskOutcome {
                kind: OUTCOME_ANSWERED.into(),
                note: "done".into(),
                ..AskOutcome::default()
            },
        ),
        ErrorKind::NotAllowed,
    );
    let t = e.c.resolve_thread(USER, &tk.id).unwrap();
    assert!(is_terminal(&t.state), "{t:?}");
    assert!(e.c.get_task(&tk.id).unwrap().open_ask().is_none());
}

#[test]
fn an_idle_agent_gets_it_at_once_besides_the_cards_it_names() {
    let e = Env::new();
    let card = alices_card(&e);
    let (tk, _) = thread(&e, &format!("Did #{} finish?", card.short));
    e.ros.set(CAROL, "busy");
    // No quiet spell (card #512): the next sweep, at once.
    let ns = e.c.sweep_asks(&agents(&e)).unwrap();
    assert!(
        ns.len() == 1
            && ns[0].to == [BOB]
            && ns[0].text.contains("handed it to you")
            && ns[0].text.contains(ALICE),
        "{ns:?}"
    );
    let a = ask_of(&e.c, &tk.id);
    assert!(a.taker == BOB && a.handed, "{a:?}");
    // Bob answers: answered, and bob no longer holds it.
    e.c.add_comment(BOB, &tk.id, "It finished an hour ago.", false)
        .unwrap();
    e.c.sweep_asks(&agents(&e)).unwrap();
    let a = ask_of(&e.c, &tk.id);
    assert!(a.reason == "answered" && a.taker.is_empty(), "{a:?}");
    assert!(e.c.get_task(&tk.id).unwrap().holders.is_empty());
}

#[test]
fn a_thread_naming_nothing_goes_to_an_idle_agent_at_once() {
    let e = Env::new();
    e.ros.set(ALICE, "busy");
    e.ros.set(CAROL, "busy");
    let (tk, n) = thread(&e, "What did we ship today?");
    assert!(
        n.to == [BOB] && n.text.contains("handed it to you"),
        "{n:?}"
    );
    assert_eq!(ask_of(&e.c, &tk.id).taker, BOB);
}

/// Card #124: a thread handed out by the sweep (not at post) still brings
/// the files the person posted with it.
#[test]
fn a_thread_handed_later_brings_its_files() {
    let e = Env::new();
    let card = alices_card(&e);
    let file = FileRef {
        sha256: "a".repeat(64),
        name: "shot.png".into(),
        size: 9,
        ..FileRef::default()
    };
    let (_, n) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: format!("What is this on #{}?", card.short),
                files: vec![file.clone()],
                thread: true,
                ..NewAsk::default()
            },
            &agents(&e),
        )
        .unwrap();
    assert_eq!(
        n.attachments,
        vec![file.clone()],
        "alice hears it with the file"
    );
    e.ros.set(CAROL, "busy");
    let ns = e.c.sweep_asks(&agents(&e)).unwrap();
    assert!(ns.len() == 1 && ns[0].to == [BOB], "{ns:?}");
    assert_eq!(ns[0].attachments, vec![file]);
}

/// Card #169: the person reopens a resolved thread. It is live again
/// (open ask, not terminal), their reply goes to the agent that answered,
/// a replay folds the same, and only the person reopens it.
#[test]
fn a_resolved_thread_reopens() {
    let e = Env::new();
    let card = alices_card(&e);
    let (tk, _) = thread(&e, &format!("Is #{} still moving?", card.short));
    e.c.add_comment(ALICE, &tk.id, "Yes, nearly there.", false)
        .unwrap();
    e.c.sweep_asks(&agents(&e)).unwrap();
    e.c.resolve_thread(USER, &tk.id).unwrap();
    want_err(e.c.reopen_thread(ALICE, &tk.id), ErrorKind::NotAllowed);

    let t = e.c.reopen_thread(USER, &tk.id).unwrap();
    assert!(!is_terminal(&t.state), "{t:?}");
    assert!(t.open_ask().is_some(), "the thread's ask is open again");
    let a = ask_of(&e.c, &tk.id);
    assert!(a.reason == "answered" && a.taker.is_empty(), "{a:?}");
    want_err(e.c.reopen_thread(USER, &tk.id), ErrorKind::BadState);

    // No hand-out: it was answered; the reply goes to alice.
    e.clk.add(secs(60));
    assert!(e.c.sweep_asks(&agents(&e)).unwrap().is_empty());
    let (_, ns) =
        e.c.person_comment(USER, &tk.id, "One more thing.", true, &[], None)
            .unwrap();
    assert!(
        ns.len() == 1
            && ns[0].to == [ALICE]
            && ns[0].wake
            && ns[0].text.contains("One more thing."),
        "{ns:?}"
    );

    // A replay folds the same, and it can be resolved again.
    let again = e.reopen().get_task(&tk.id).unwrap();
    assert!(again.open_ask().is_some() && !is_terminal(&again.state));
    let t = e.c.resolve_thread(USER, &tk.id).unwrap();
    assert!(is_terminal(&t.state));
}

/// Card #169: a thread nobody answered goes out again when reopened.
#[test]
fn a_reopened_unanswered_thread_goes_out_again() {
    let e = Env::new();
    e.ros.set(ALICE, "busy");
    e.ros.set(BOB, "busy");
    e.ros.set(CAROL, "busy");
    let (tk, _) = thread(&e, "Anyone around?");
    e.c.resolve_thread(USER, &tk.id).unwrap();
    e.c.reopen_thread(USER, &tk.id).unwrap();
    e.ros.set(BOB, "idle");
    let ns = e.c.sweep_asks(&agents(&e)).unwrap();
    assert!(ns.iter().any(|n| n.to == [BOB]), "{ns:?}");
}

#[test]
fn only_threads_reopen_as_threads() {
    let e = Env::new();
    let card = alices_card(&e);
    want_err(e.c.reopen_thread(USER, &card.id), ErrorKind::Invalid);
}
