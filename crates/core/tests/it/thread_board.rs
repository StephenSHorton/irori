//! Card #708: a card made from a thread on one of Personal's boards goes
//! on that board when no board is named; a named board, even the default,
//! wins. A thread is on such a board when its space moved into Personal
//! (card #640).

use crate::common;

use common::Rig;

/// An agent's tool call on the Rig's Core, as agent `key_a`.
fn tool(r: &Rig, name: &str, args: serde_json::Value) -> String {
    let caller = tools::Caller {
        sender: inbox::Sender::new(&r.key_a, "alpha", "claude"),
        path: String::new(),
    };
    r.core
        .tools()
        .call(&caller, name, &serde_json::to_vec(&args).unwrap())
        .unwrap()
}

fn task(r: &Rig, rf: &str) -> coord::Task {
    let id = r.core.task_ref(rf).unwrap();
    r.core.coord().get_task(&id).unwrap()
}

/// A space "colors" with an open thread, moved into Personal: the thread
/// is colors#1 there. Returns the board's id and the thread's.
fn thread_on_colors(r: &Rig) -> (String, String) {
    let space = r
        .core
        .create_board("Colors of War Decomp", "colors")
        .unwrap();
    let (th, _) = r
        .core
        .board_ask(
            &space,
            coord::NewAsk {
                text: "what's left on the decomp?".into(),
                thread: true,
                quiet: true,
                ..Default::default()
            },
        )
        .unwrap();
    let m = r
        .core
        .move_space_to_personal(&space, &irori_core::MoveOpts::default())
        .unwrap();
    let t = r.core.coord().get_task(&th.id).unwrap();
    assert_eq!(
        (t.board.as_str(), t.short_ref().as_str()),
        (m.board_id.as_str(), "colors#1")
    );
    let out = tool(
        r,
        "board_comment",
        serde_json::json!({"task_id": "colors#1", "text": "the menus and the AI"}),
    );
    assert!(!out.starts_with("Error"), "{out}");
    (m.board_id, th.id)
}

#[test]
fn a_card_from_a_thread_on_a_board_goes_on_that_board() {
    let r = Rig::new();
    let (board, th) = thread_on_colors(&r);
    // Named, and the one open thread the agent answered.
    for (args, want) in [
        (
            serde_json::json!({"title": "the rest", "from": "colors#1"}),
            "colors#2",
        ),
        (serde_json::json!({"title": "the menus"}), "colors#3"),
    ] {
        let out = tool(&r, "board_create", args);
        assert!(
            out.contains(&format!("Created {want}"))
                && out.contains("\"Colors of War Decomp\" (colors), one of Personal's boards"),
            "{out}"
        );
        let t = task(&r, want);
        assert_eq!(
            (t.from.as_str(), t.board.as_str()),
            (th.as_str(), board.as_str())
        );
    }
    // A subtask still goes on its parent's board, made from no thread.
    let out = tool(
        &r,
        "board_create",
        serde_json::json!({"title": "home", "board": "personal"}),
    );
    assert!(out.contains("Created #1"), "{out}");
    let out = tool(
        &r,
        "board_create",
        serde_json::json!({"title": "kid", "parent": "#1"}),
    );
    assert!(out.contains("Created #2"), "{out}");
}

#[test]
fn a_named_board_wins_over_the_threads() {
    let r = Rig::new();
    let (_, th) = thread_on_colors(&r);
    r.core
        .create_personal_board("Buzzing Cars Decomp", "buzz")
        .unwrap();
    let out = tool(
        &r,
        "board_create",
        serde_json::json!({"title": "chores", "board": "personal"}),
    );
    assert!(out.contains("Created #1"), "{out}");
    assert_eq!(task(&r, "#1").from, th, "still made from it");
    let out = tool(
        &r,
        "board_create",
        serde_json::json!({"title": "audio", "board": "buzz", "from": "colors#1"}),
    );
    assert!(out.contains("Created buzz#1"), "{out}");
}
