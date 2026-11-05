//! Where notes live (card #401, docs/design/notes.md §3, §8): a Mine note
//! stays in the private log even when it names a shared board; a board
//! note goes in that board's log only.

mod common;

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
