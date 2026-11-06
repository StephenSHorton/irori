//! Where notes live (card #401, docs/design/notes.md §3, §8): a Mine note
//! stays in the private log even when it names a shared board; a board
//! note goes in that board's log only.

use crate::common;

use common::Rig;
use store::Filter;

fn note_rows(st: &store::Store) -> Vec<String> {
    st.list(&Filter {
        kinds: vec!["note.".into()],
        limit: 1000,
        ..Default::default()
    })
    .unwrap()
    .iter()
    .map(|ev| {
        let v: serde_json::Value = serde_json::from_slice(&ev.payload).unwrap();
        v["noteId"].as_str().unwrap_or_default().to_string()
    })
    .collect()
}

fn nn(text: &str, tags: &[&str]) -> coord::NewNote {
    coord::NewNote {
        kind: "rule".into(),
        text: text.into(),
        tags: tags.iter().map(|t| t.to_string()).collect(),
        ..Default::default()
    }
}

#[test]
fn a_mine_note_never_enters_a_board_log() {
    let r = Rig::new();
    let c = &r.core;
    let b = c.create_board("MLG", "mlg").unwrap();
    let me = c.user_sender().key;

    // A Mine note about the board, written by the person and accepted at once.
    let mine = c
        .propose_note(&me, "mine", nn("Home IPs stay out of MLG.", &["board:mlg"]))
        .unwrap();
    assert_eq!((mine.scope.as_str(), mine.board_id.as_str()), ("mine", ""));
    assert_eq!(mine.note.state, coord::NOTE_ACCEPTED);
    // Alone on the board there is no device to wait on (#411, #392): a
    // board note goes in the board log only.
    let shared = c
        .propose_note(&me, "mlg", nn("Ship models to the repo.", &["everywhere"]))
        .unwrap()
        .note;
    assert_eq!(
        note_rows(&c.board_store(&b).unwrap()),
        vec![shared.id.clone()]
    );
    assert_eq!(note_rows(c.store()), vec![mine.note.id.clone()]);

    let all = c.notes("", false).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(c.notes("mine", false).unwrap().len(), 1);
    assert_eq!(c.notes("mlg", false).unwrap()[0].board_id, b);
    assert_eq!(c.search_notes("repo", true).len(), 1);
    // The person removes it from the board; an unknown scope is refused.
    c.remove_note(&me, &shared.id, "").unwrap();
    assert!(c.notes("mlg", false).unwrap().is_empty());
    assert!(
        c.propose_note(&me, "nope", nn("x", &["everywhere"]))
            .is_err()
    );
}

/// The `wake.issued` rows' payloads, oldest first.
fn wake_rows(st: &store::Store) -> Vec<serde_json::Value> {
    st.list(&Filter {
        kinds: vec!["wake.issued".into()],
        limit: 1000,
        ..Default::default()
    })
    .unwrap()
    .iter()
    .map(|ev| serde_json::from_slice(&ev.payload).unwrap())
    .collect()
}

/// Card #412: an agent proposes and searches notes with the tools; a wake
/// about a matching card carries the accepted notes (and lists them on its
/// wake.issued row), never a proposed one, and not twice to one agent.
#[test]
fn a_wake_on_a_matching_card_carries_its_accepted_notes() {
    let r = Rig::new();
    let c = &r.core;
    let me = c.user_sender().key;
    let caller = |key: &str, name: &str| tools::Caller {
        sender: inbox::Sender::new(key, name, "claude"),
        path: inbox::path::MOD.into(),
    };
    let (a, b) = (caller(&r.key_a, "alpha"), caller(&r.key_b, "beta"));
    let tool = |cl: &tools::Caller, name: &str, args: serde_json::Value| -> String {
        c.tools()
            .call(cl, name, &serde_json::to_vec(&args).unwrap())
            .unwrap()
    };
    const REPO: &str = "remote:github.com/acme/demo";

    // The person's notes are live at once.
    let repo_note = c
        .propose_note(
            &me,
            "mine",
            nn(
                "Run check.sh before landing on main.",
                &[&format!("repo:{REPO}")],
            ),
        )
        .unwrap();
    let path_note = c
        .propose_note(
            &me,
            "mine",
            nn(
                "The wake fence is asserted byte for byte.",
                &[&format!("path:{REPO}:crates/inbox")],
            ),
        )
        .unwrap();
    let elsewhere = c
        .propose_note(
            &me,
            "mine",
            nn("Other repo only.", &["repo:remote:github.com/acme/other"]),
        )
        .unwrap();
    let topic = c
        .propose_note(&me, "mine", nn("Search-only note.", &["topic:release"]))
        .unwrap();

    // An agent's proposal waits for the person, and reaches no agent.
    let out = tool(
        &b,
        "notes_propose",
        serde_json::json!({"kind": "gotcha", "text": "PROPOSED-ONLY never wakes.", "tags": ["everywhere"]}),
    );
    assert!(
        out.starts_with("Proposed note:")
            && out.contains("reaches no agent until the person accepts it"),
        "notes_propose = {out}"
    );
    let proposed = c.search_notes("PROPOSED-ONLY", false);
    assert_eq!(proposed.len(), 1);
    assert_eq!(proposed[0].note.state, coord::NOTE_PROPOSED);
    let pid = proposed[0].note.reference();
    // Its author changes it directly; an edit needs why.
    assert!(
        tool(
            &b,
            "notes_propose",
            serde_json::json!({"note": pid, "text": "PROPOSED-ONLY, edited."})
        )
        .starts_with("Error: ")
    );
    let out = tool(
        &b,
        "notes_propose",
        serde_json::json!({"note": pid, "text": "PROPOSED-ONLY, edited.", "why": "clearer"}),
    );
    assert!(out.starts_with("Changed your proposal"), "edit = {out}");
    // An edit of a live note waits as a pending change.
    let out = tool(
        &b,
        "notes_propose",
        serde_json::json!({"note": repo_note.note.reference(), "remove": true, "why": "gone"}),
    );
    assert!(out.starts_with("Proposed the removal"), "remove = {out}");
    assert!(c.search_notes("check.sh", true)[0].note.live());

    // A scope that names nothing refuses in the tool's own words.
    let e = c
        .propose_note(&me, "xyz", nn("x", &["topic:x"]))
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("no space \"xyz\": give mine, or the tag or id of a space or one of its boards"),
        "{e}"
    );

    // Search reads accepted notes only.
    let out = tool(
        &a,
        "notes_search",
        serde_json::json!({"q": "proposed-only"}),
    );
    assert!(out.starts_with("No accepted note"), "search = {out}");
    let out = tool(
        &a,
        "notes_search",
        serde_json::json!({"q": "topic:release"}),
    );
    assert!(
        out.contains(&topic.note.reference()) && out.contains("untrusted data"),
        "search = {out}"
    );

    // A card in the repo, with a path under the path note, assigned to alpha.
    let out = tool(
        &b,
        "board_create",
        serde_json::json!({"title": "Fence work", "repo": REPO, "links": ["crates/inbox/src/fence.rs"], "assign_to": "alpha"}),
    );
    assert!(out.starts_with("Created #"), "board_create = {out}");
    let task = c
        .coord()
        .list_tasks(&coord::TaskFilter::default())
        .into_iter()
        .find(|t| t.title == "Fence work")
        .unwrap();

    let w = c
        .inbox()
        .next_wake(&r.key_a, inbox::path::MOD)
        .unwrap()
        .expect("alpha has the assignment");
    let p = &w.prompt;
    assert!(
        p.contains("\nNotes from Irori for task:"),
        "no Notes section:\n{p}"
    );
    assert!(
        p.contains(&path_note.note.reference()) && p.contains("byte for byte"),
        "{p}"
    );
    assert!(
        p.contains(&repo_note.note.reference()) && p.contains(&format!("matched repo:{REPO}")),
        "{p}"
    );
    assert!(p.contains("by the person, accepted by the person"), "{p}");
    for absent in ["PROPOSED-ONLY", "Other repo only", "Search-only"] {
        assert!(!p.contains(absent), "{absent} reached the wake:\n{p}");
    }
    // Most specific first: the path note before the repo note.
    assert!(p.find("byte for byte").unwrap() < p.find("Run check.sh").unwrap());
    let _ = elsewhere;

    // The wake.issued row lists them, with the card.
    let rows = wake_rows(c.store());
    let row = rows.iter().rfind(|v| v["to"] == r.key_a.as_str()).unwrap();
    assert_eq!(row["notesTask"], format!("task:{}", task.id).as_str());
    let ids: Vec<&str> = row["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![path_note.note.id.as_str(), repo_note.note.id.as_str()]
    );

    // The next wake about the card doesn't repeat them.
    c.inbox().mark_delivered(&r.key_a, &w.token).unwrap();
    c.inbox()
        .mark_engaged(&r.key_a, &w.token, "turn-1")
        .unwrap();
    c.inbox().turn_ended(&r.key_a, &w.token, "turn-1").unwrap();
    let out = tool(
        &b,
        "send",
        serde_json::json!({"to": "alpha", "text": "one more thing", "task_id": format!("task:{}", task.id)}),
    );
    assert!(out.starts_with("Sent"), "send = {out}");
    let w2 = c
        .inbox()
        .next_wake(&r.key_a, inbox::path::MOD)
        .unwrap()
        .expect("alpha has the message");
    assert!(
        !w2.prompt.contains("Notes from Irori"),
        "notes repeated:\n{}",
        w2.prompt
    );
}

#[test]
fn an_agents_board_note_goes_through_the_gate_and_waits_for_a_person() {
    // The tool path shares Services::propose_note with the window, so the
    // write gate (#411) applies to it too; one device here, so it is open.
    let r = Rig::new();
    let c = &r.core;
    c.create_board("MLG", "mlg").unwrap();
    let a = tools::Caller {
        sender: inbox::Sender::new(&r.key_a, "alpha", "claude"),
        path: inbox::path::MOD.into(),
    };
    let out = c
        .tools()
        .call(
            &a,
            "notes_propose",
            &serde_json::to_vec(&serde_json::json!({"scope": "mlg", "kind": "rule", "text": "BOARD-AGENT note.", "tags": ["everywhere"]})).unwrap(),
        )
        .unwrap();
    assert!(
        out.starts_with("Proposed note:")
            && out.contains("in space mlg")
            && out.contains("until a person in the space accepts it"),
        "{out}"
    );
    let found = c.search_notes("BOARD-AGENT", false);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].scope, "mlg");
    assert_eq!(found[0].note.state, coord::NOTE_PROPOSED);
    assert!(c.search_notes("BOARD-AGENT", true).is_empty());
}

/// Card #555: the person comments on one subtask; the agent on its sibling
/// is woken for something else and hears those words, with the card's ref,
/// once. An agent's comment isn't among them.
#[test]
fn a_wake_on_a_sibling_carries_the_persons_words_once() {
    let r = Rig::new();
    let c = &r.core;
    let co = c.coord();
    let person = c.user_sender().key;
    let b = tools::Caller {
        sender: inbox::Sender::new(&r.key_b, "beta", "claude"),
        path: inbox::path::MOD.into(),
    };
    let tool = |name: &str, args: serde_json::Value| -> String {
        c.tools()
            .call(&b, name, &serde_json::to_vec(&args).unwrap())
            .unwrap()
    };
    let new = |title: &str, parent: &str| {
        co.create_task(
            &person,
            coord::NewTask {
                title: title.into(),
                parent: parent.into(),
                ..coord::NewTask::default()
            },
        )
        .unwrap()
    };
    let p = new("Merge desk", "");
    let one = new("Slice 2", &p.short_ref());
    let two = new("Slice 4", &p.short_ref());
    co.claim_task(&r.key_a, &two.id).unwrap();
    // Whatever taking the card queued goes first.
    while let Some(w) = c.inbox().next_wake(&r.key_a, inbox::path::MOD).unwrap() {
        c.inbox().mark_delivered(&r.key_a, &w.token).unwrap();
        c.inbox().mark_engaged(&r.key_a, &w.token, "t0").unwrap();
        c.inbox().turn_ended(&r.key_a, &w.token, "t0").unwrap();
    }
    co.person_comment(
        &person,
        &one.id,
        "A place to talk about PRs",
        false,
        &[],
        None,
    )
    .unwrap();
    co.add_comment(&r.key_b, &one.id, "AGENT-LINE on the slice", false)
        .unwrap();

    let send = |text: &str| {
        let out = tool(
            "send",
            serde_json::json!({"to": "alpha", "text": text, "task_id": format!("task:{}", two.id)}),
        );
        assert!(out.starts_with("Sent"), "send = {out}");
        c.inbox()
            .next_wake(&r.key_a, inbox::path::MOD)
            .unwrap()
            .expect("alpha has the message")
    };
    let w = send("about your card");
    let prompt = &w.prompt;
    assert!(
        prompt.contains(&format!("around {}", two.short_ref()))
            && prompt.contains(&format!("the person, on {} (comment:", one.short_ref()))
            && prompt.contains("A place to talk about PRs")
            && prompt.contains("whether any of them change it"),
        "{prompt}"
    );
    assert!(!prompt.contains("AGENT-LINE"), "{prompt}");
    c.inbox().mark_delivered(&r.key_a, &w.token).unwrap();
    c.inbox().mark_engaged(&r.key_a, &w.token, "t1").unwrap();
    c.inbox().turn_ended(&r.key_a, &w.token, "t1").unwrap();
    // Recorded on the row, so a re-sent wake reads the same.
    let rows = wake_rows(c.store());
    assert!(
        rows.iter().any(|v| {
            v["to"] == r.key_a.as_str()
                && v["saidFor"] == two.short_ref().as_str()
                && v["said"][0]["card"] == one.short_ref().as_str()
        }),
        "{rows:?}"
    );
    let w2 = send("one more thing");
    assert!(
        !w2.prompt.contains("A place to talk about PRs"),
        "repeated:\n{}",
        w2.prompt
    );
}

/// Card #555: on a shared board the same query reads the board's log, and
/// counts this person's words there.
#[test]
fn the_persons_words_count_on_a_shared_board_too() {
    let r = Rig::new();
    let c = &r.core;
    let b = c.create_board("MLG", "mlg").unwrap();
    let co = c.board_coord(&b).unwrap();
    let person = format!("{}{}", store::kinds::PERSON_ACTOR_PREFIX, c.person().id);
    let new = |title: &str, parent: &str| {
        co.create_task(
            &person,
            coord::NewTask {
                title: title.into(),
                parent: parent.into(),
                ..coord::NewTask::default()
            },
        )
        .unwrap()
    };
    let p = new("Parent", "");
    let one = new("One", &p.short_ref());
    let two = new("Two", &p.short_ref());
    co.person_comment(&person, &one.id, "Mine, on the board", false, &[], None)
        .unwrap();
    let got = co.said_near(&two.id);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].card, one.short_ref());
}

/// Card #555: on a shared board the agent's key there is agent:<cardId>,
/// so its wake finds its card through the board's map.
#[test]
fn a_wake_on_a_shared_board_carries_the_persons_words() {
    let r = Rig::new();
    let c = &r.core;
    let b = c.create_board("MLG", "mlg").unwrap();
    let co = c.board_coord(&b).unwrap();
    let person = format!("{}{}", store::kinds::PERSON_ACTOR_PREFIX, c.person().id);
    let alpha = tools::Caller {
        sender: inbox::Sender::new(&r.key_a, "alpha", "claude"),
        path: inbox::path::MOD.into(),
    };
    let beta = tools::Caller {
        sender: inbox::Sender::new(&r.key_b, "beta", "claude"),
        path: inbox::path::MOD.into(),
    };
    let tool = |cl: &tools::Caller, name: &str, args: serde_json::Value| -> String {
        c.tools()
            .call(cl, name, &serde_json::to_vec(&args).unwrap())
            .unwrap()
    };
    let new = |title: &str, parent: &str| {
        co.create_task(
            &person,
            coord::NewTask {
                title: title.into(),
                parent: parent.into(),
                ..coord::NewTask::default()
            },
        )
        .unwrap()
    };
    let p = new("Parent", "");
    let one = new("One", &p.short_ref());
    let two = new("Two", &p.short_ref());
    let out = tool(
        &alpha,
        "board_join",
        serde_json::json!({"task_id": format!("task:{}", two.id)}),
    );
    assert!(out.contains("You hold"), "board_join = {out}");
    while let Some(w) = c.inbox().next_wake(&r.key_a, inbox::path::MOD).unwrap() {
        c.inbox().mark_delivered(&r.key_a, &w.token).unwrap();
        c.inbox().mark_engaged(&r.key_a, &w.token, "t0").unwrap();
        c.inbox().turn_ended(&r.key_a, &w.token, "t0").unwrap();
    }
    co.person_comment(&person, &one.id, "Said on the board", false, &[], None)
        .unwrap();
    let out = tool(
        &beta,
        "send",
        serde_json::json!({"to": "alpha", "text": "about your card"}),
    );
    assert!(out.starts_with("Sent"), "send = {out}");
    let w = c
        .inbox()
        .next_wake(&r.key_a, inbox::path::MOD)
        .unwrap()
        .expect("alpha has the message");
    assert!(
        w.prompt.contains("Said on the board") && w.said_for == two.short_ref(),
        "{}",
        w.prompt
    );
}

/// A card on a new board with a note for every wake on it, a Mine note for
/// every wake, and a message about the card waiting for alpha. Sent before
/// any test holds a lock: only the wake runs under the inbox lock.
fn a_message_about_a_board_card(r: &Rig) -> (String, String) {
    let c = &r.core;
    let me = c.user_sender().key;
    let b = c.create_board("MLG", "mlg").unwrap();
    let co = c.board_coord(&b).unwrap();
    let person = format!("{}{}", store::kinds::PERSON_ACTOR_PREFIX, c.person().id);
    let task = co
        .create_task(
            &person,
            coord::NewTask {
                title: "Shared work".into(),
                ..coord::NewTask::default()
            },
        )
        .unwrap();
    c.propose_note(&me, "mine", nn("MINE-NOTE every wake.", &["everywhere"]))
        .unwrap();
    c.propose_note(&me, "mlg", nn("BOARD-NOTE every wake.", &["everywhere"]))
        .unwrap();
    send_about(r, &task.id, "about the board card");
    (b, task.id)
}

fn send_about(r: &Rig, task_id: &str, text: &str) {
    let beta = tools::Caller {
        sender: inbox::Sender::new(&r.key_b, "beta", "claude"),
        path: inbox::path::MOD.into(),
    };
    let args =
        serde_json::json!({"to": "alpha", "text": text, "task_id": format!("task:{task_id}")});
    let out = r
        .core
        .tools()
        .call(&beta, "send", &serde_json::to_vec(&args).unwrap())
        .unwrap();
    assert!(out.starts_with("Sent"), "send = {out}");
}

/// Alpha's wake, issued on another thread while this one holds `_held`: a
/// wait on what it holds fails here instead of hanging the run.
fn wake_while_held(r: &Rig, held: impl Sized) -> inbox::Wake {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::scope(|s| {
        // In here, so a failure lets go before the scope joins the thread.
        let _held = held;
        s.spawn(|| {
            let w = r.core.inbox().next_wake(&r.key_a, inbox::path::MOD);
            let _ = tx.send(w);
        });
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .expect("the wake waited on a lock a board write can hold while it waits on the inbox")
            .unwrap()
            .expect("alpha has the message")
    })
}

fn finish(r: &Rig, w: &inbox::Wake, turn: &str) {
    let ib = r.core.inbox();
    ib.mark_delivered(&r.key_a, &w.token).unwrap();
    ib.mark_engaged(&r.key_a, &w.token, turn).unwrap();
    ib.turn_ended(&r.key_a, &w.token, turn).unwrap();
}

/// A wake about a board's card reads that board without waiting (card #561)
/// and still carries its notes, told apart from Mine's.
#[test]
fn a_wake_on_a_board_card_carries_the_boards_notes() {
    let r = Rig::new();
    a_message_about_a_board_card(&r);
    let w = r
        .core
        .inbox()
        .next_wake(&r.key_a, inbox::path::MOD)
        .unwrap()
        .expect("alpha has the message");
    let p = &w.prompt;
    assert!(p.contains("MINE-NOTE") && p.contains("BOARD-NOTE"), "{p}");
    assert!(
        p.contains("by a person in the space, accepted by a person in the space"),
        "{p}"
    );
}

/// Card #561: a board write holds its coord and waits on the inbox for its
/// hop, so a wake under the inbox lock must not wait on that coord. It goes
/// out without the board's notes, which the next wake gives.
#[test]
fn a_wake_never_waits_on_a_busy_board() {
    let r = Rig::new();
    let (b, task) = a_message_about_a_board_card(&r);
    let co = r.core.board_coord(&b).unwrap();
    let w = wake_while_held(&r, co.hold_lock());
    assert!(w.prompt.contains("MINE-NOTE"), "{}", w.prompt);
    assert!(!w.prompt.contains("BOARD-NOTE"), "{}", w.prompt);
    finish(&r, &w, "t1");

    send_about(&r, &task, "again, the board free");
    let w = r
        .core
        .inbox()
        .next_wake(&r.key_a, inbox::path::MOD)
        .unwrap()
        .expect("alpha has the message");
    assert!(w.prompt.contains("BOARD-NOTE"), "{}", w.prompt);
}

/// Card #561: a board's coord holder may wait on the boards list, and a
/// wake waits on it under the inbox lock, so `route` must not keep the list
/// while it waits on a busy board.
#[test]
fn route_never_keeps_the_boards_list_while_a_board_is_busy() {
    let r = Rig::new();
    let c = &r.core;
    let b = c.create_board("MLG", "mlg").unwrap();
    let co = c.board_coord(&b).unwrap();
    let held = co.hold_lock();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::scope(|s| {
        let _held = held;
        // An id in no log: it reads every board's coord, and waits here.
        s.spawn(|| {
            let _ = c.route("no-such-card");
        });
        std::thread::sleep(std::time::Duration::from_millis(200));
        s.spawn(move || {
            drop(c.hold_boards_list());
            let _ = tx.send(());
        });
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .expect("route kept the boards list while it waited on a board coord");
    });
}

/// The check every test runs under: a blocking board coord read in an inbox hook
/// panics in a debug build, whether or not the board happens to be busy.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "under the inbox lock")]
fn waiting_on_a_board_coord_in_an_inbox_hook_is_caught() {
    let r = Rig::new();
    let b = r.core.create_board("MLG", "mlg").unwrap();
    let co = r.core.board_coord(&b).unwrap();
    coord::with_inbox_lock_held(|| co.get_task("nope"));
}
