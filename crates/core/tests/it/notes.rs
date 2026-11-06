//! Where notes live (card #401, docs/design/notes.md §3, §8): every note is
//! the person's own, in the private log, even when it names a shared space
//! (card #616).

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

/// Card #616: every note is the person's own. A note about a shared space
/// stays in the private log, and a note row an older device wrote into a
/// board log (0.11.0-0.12.3 still can) is kept there but never read.
#[test]
fn every_note_is_mine_and_board_note_rows_go_unread() {
    let r = Rig::new();
    let c = &r.core;
    let b = c.create_board("MLG", "mlg").unwrap();
    let me = c.user_sender().key;

    // A note about the space, written by the person and accepted at once.
    let mine = c
        .propose_note(&me, nn("Home IPs stay out of MLG.", &["board:mlg"]))
        .unwrap();
    assert_eq!(mine.state, coord::NOTE_ACCEPTED);
    assert_eq!(note_rows(c.store()), vec![mine.id.clone()]);
    assert!(note_rows(&c.board_store(&b).unwrap()).is_empty());

    // As an older device would: a note row straight into the board's log.
    let person = format!("{}{}", store::kinds::PERSON_ACTOR_PREFIX, c.person().id);
    let old = c
        .board_coord(&b)
        .unwrap()
        .propose_note(&person, nn("OLD-BOARD-NOTE ships models.", &["everywhere"]))
        .unwrap();
    assert_eq!(note_rows(&c.board_store(&b).unwrap()), vec![old.id.clone()]);

    let all = c.notes(false);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, mine.id);
    assert!(c.search_notes("OLD-BOARD-NOTE", false).is_empty());
    assert_eq!(c.search_notes("home", true).len(), 1);
    // It can't be changed through the Core either: there is no such note.
    assert!(c.remove_note(&me, &old.id, "").is_err());
    c.remove_note(&me, &format!("note:{}", mine.id), "")
        .unwrap();
    assert!(c.notes(false).is_empty());
    assert_eq!(c.notes(true).len(), 1);
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
            nn(
                "Run check.sh before landing on main.",
                &[&format!("repo:{REPO}")],
            ),
        )
        .unwrap();
    let path_note = c
        .propose_note(
            &me,
            nn(
                "The wake fence is asserted byte for byte.",
                &[&format!("path:{REPO}:crates/inbox")],
            ),
        )
        .unwrap();
    let elsewhere = c
        .propose_note(
            &me,
            nn("Other repo only.", &["repo:remote:github.com/acme/other"]),
        )
        .unwrap();
    let topic = c
        .propose_note(&me, nn("Search-only note.", &["topic:release"]))
        .unwrap();

    // An agent's note is live at once (card #616): no one accepts it.
    let out = tool(
        &b,
        "notes_propose",
        serde_json::json!({"kind": "gotcha", "text": "AGENT-NOTE reaches wakes at once.", "tags": ["everywhere"]}),
    );
    assert!(
        out.starts_with("Wrote note:") && out.contains("it is live"),
        "notes_propose = {out}"
    );
    let agent_note = c.search_notes("AGENT-NOTE", true);
    assert_eq!(agent_note.len(), 1);
    assert!(agent_note[0].live() && agent_note[0].accepted_by.is_empty());
    let aid = agent_note[0].reference();
    // An edit needs why, and applies at once.
    assert!(
        tool(
            &b,
            "notes_propose",
            serde_json::json!({"note": aid, "text": "AGENT-NOTE, edited."})
        )
        .starts_with("Error: ")
    );
    let out = tool(
        &b,
        "notes_propose",
        serde_json::json!({"note": aid, "text": "AGENT-NOTE reaches wakes, edited.", "why": "clearer"}),
    );
    assert_eq!(out, format!("Changed {aid}."));
    assert_eq!(
        c.search_notes("AGENT-NOTE", true)[0].text,
        "AGENT-NOTE reaches wakes, edited."
    );
    // So does a removal, of the person's note too.
    let out = tool(
        &b,
        "notes_propose",
        serde_json::json!({"note": elsewhere.reference(), "remove": true, "why": "gone", "scope": "mine"}),
    );
    assert_eq!(out, format!("Removed {}.", elsewhere.reference()));

    let out = tool(
        &a,
        "notes_search",
        serde_json::json!({"q": "other repo only"}),
    );
    assert!(out.starts_with("No note holds"), "search = {out}");
    let out = tool(
        &a,
        "notes_search",
        serde_json::json!({"q": "topic:release"}),
    );
    assert!(
        out.contains(&topic.reference()) && out.contains("untrusted data"),
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
        p.contains(&path_note.reference()) && p.contains("byte for byte"),
        "{p}"
    );
    assert!(
        p.contains(&repo_note.reference()) && p.contains(&format!("matched repo:{REPO}")),
        "{p}"
    );
    assert!(p.contains("by the person, accepted by the person"), "{p}");
    // An agent's note names its writer and no accepter.
    let line = p
        .lines()
        .skip_while(|l| !l.contains("AGENT-NOTE reaches wakes, edited."))
        .nth(1)
        .unwrap_or_default();
    assert!(
        line.contains("by beta (") && !line.contains("accepted"),
        "{p}"
    );
    for absent in ["Other repo only", "Search-only"] {
        assert!(!p.contains(absent), "{absent} reached the wake:\n{p}");
    }
    // Most specific first: the path note, the repo note, then everywhere.
    assert!(p.find("byte for byte").unwrap() < p.find("Run check.sh").unwrap());
    assert!(p.find("Run check.sh").unwrap() < p.find("AGENT-NOTE").unwrap());

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
        vec![
            path_note.id.as_str(),
            repo_note.id.as_str(),
            agent_note[0].id.as_str()
        ]
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

/// Card #616: notes_propose takes no shared scope, and refuses tags that
/// could never match a wake, each with what to give instead.
#[test]
fn the_tool_refuses_a_board_scope_and_tags_that_cant_match() {
    let r = Rig::new();
    let c = &r.core;
    let space = c.create_board("MLG", "mlg").unwrap();
    let a = tools::Caller {
        sender: inbox::Sender::new(&r.key_a, "alpha", "claude"),
        path: inbox::path::MOD.into(),
    };
    let tool = |args: serde_json::Value| -> String {
        c.tools()
            .call(&a, "notes_propose", &serde_json::to_vec(&args).unwrap())
            .unwrap()
    };
    let note = |tags: serde_json::Value| serde_json::json!({"kind": "gotcha", "text": "A gotcha.", "tags": tags});
    let machine = c.machine_names()[0].clone();

    // A space's tag as scope: refused plainly, pointing at board:.
    let mut args = note(serde_json::json!(["everywhere"]));
    args["scope"] = "mlg".into();
    let out = tool(args);
    assert!(
        out.starts_with("Error: ")
            && out.contains("there are no shared notes")
            && out.contains("board:<tag>"),
        "{out}"
    );
    // mine (an older spec's default) still goes.
    let mut args = note(serde_json::json!(["topic:scope"]));
    args["scope"] = "Mine".into();
    assert!(tool(args).starts_with("Wrote note:"));

    // A * in any tag: path tags are literal prefixes.
    let out = tool(note(serde_json::json!([
        "path:remote:github.com/a/b:src/*.rs"
    ])));
    assert!(
        out.starts_with("Error: ") && out.contains("never expand"),
        "{out}"
    );
    // A machine no device is called; the refusal lists the ones that are.
    let out = tool(note(serde_json::json!(["machine:pc"])));
    assert!(
        out.starts_with("Error: ")
            && out.contains("no device is called \"pc\"")
            && out.contains(&format!("machine:{machine}")),
        "{out}"
    );
    // This device's name, in any case, goes.
    let out = tool(note(serde_json::json!([format!(
        "machine:{}",
        machine.to_uppercase()
    )])));
    assert!(out.starts_with("Wrote note:"), "{out}");
    // An agent's edit that gives tags is checked the same way.
    let id = c.search_notes("A gotcha", false)[0].reference();
    let out = tool(serde_json::json!({"note": id, "tags": ["machine:pc"], "why": "narrower"}));
    assert!(out.contains("no device is called"), "{out}");
    // Nothing refused was written.
    assert_eq!(c.notes(false).len(), 2);

    // A source card on a shared space: kept as the note's source text,
    // since the private log can't check or watch that card.
    let b = space;
    let person = format!("{}{}", store::kinds::PERSON_ACTOR_PREFIX, c.person().id);
    let card = c
        .board_coord(&b)
        .unwrap()
        .create_task(
            &person,
            coord::NewTask {
                title: "On the space".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let mut args = note(serde_json::json!(["board:mlg"]));
    args["source"] = format!("task:{}", card.id).into();
    let out = tool(args);
    assert!(out.starts_with("Wrote note:"), "{out}");
    let n = c
        .notes(false)
        .into_iter()
        .find(|n| n.tags == ["board:mlg"])
        .unwrap();
    assert_eq!(
        (n.source.as_str(), n.task_id.as_str()),
        (format!("task:{}", card.id).as_str(), "")
    );
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

/// A card on a new space, a Mine note for every wake, a Mine note tagged
/// with the space, an old board note row, and a message about the card
/// waiting for alpha. Sent before
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
    c.propose_note(&me, nn("MINE-NOTE every wake.", &["everywhere"]))
        .unwrap();
    // A Mine note about the space (card #616), and a note row an older
    // device wrote into the space's log, which no wake reads.
    c.propose_note(&me, nn("BOARD-TAG on mlg cards.", &["board:mlg"]))
        .unwrap();
    co.propose_note(&person, nn("OLD-BOARD-NOTE every wake.", &["everywhere"]))
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

/// A wake about a space's card reads that space without waiting (card
/// #561) for its tag: Mine notes tagged board:<tag> match it (card #616).
#[test]
fn a_wake_on_a_board_card_carries_mine_notes_about_its_space() {
    let r = Rig::new();
    a_message_about_a_board_card(&r);
    let w = r
        .core
        .inbox()
        .next_wake(&r.key_a, inbox::path::MOD)
        .unwrap()
        .expect("alpha has the message");
    let p = &w.prompt;
    assert!(p.contains("MINE-NOTE") && p.contains("BOARD-TAG"), "{p}");
    assert!(p.contains("matched board:mlg"), "{p}");
    assert!(!p.contains("OLD-BOARD-NOTE"), "{p}");
    assert!(p.contains("by the person, accepted by the person"), "{p}");
}

/// Card #561: a board write holds its coord and waits on the inbox for its
/// hop, so a wake under the inbox lock must not wait on that coord. It goes
/// out without the space's board: matches, which the next wake gives.
#[test]
fn a_wake_never_waits_on_a_busy_board() {
    let r = Rig::new();
    let (b, task) = a_message_about_a_board_card(&r);
    let co = r.core.board_coord(&b).unwrap();
    let w = wake_while_held(&r, co.hold_lock());
    assert!(w.prompt.contains("MINE-NOTE"), "{}", w.prompt);
    // The card isn't found on a busy space, so its tag doesn't match yet.
    assert!(!w.prompt.contains("BOARD-TAG"), "{}", w.prompt);
    finish(&r, &w, "t1");

    send_about(&r, &task, "again, the board free");
    let w = r
        .core
        .inbox()
        .next_wake(&r.key_a, inbox::path::MOD)
        .unwrap()
        .expect("alpha has the message");
    assert!(w.prompt.contains("BOARD-TAG"), "{}", w.prompt);
    assert!(!w.prompt.contains("OLD-BOARD-NOTE"), "{}", w.prompt);
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
