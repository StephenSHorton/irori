//! Card #708: an agent puts a card on one of Personal's boards by its tag
//! or id, its subtasks follow it there, and it moves a private card (with
//! its subtasks) between Personal's boards. The board tool lists them.

use crate::common;

use common::Rig;
use irori_core::PERSONAL_BOARD;

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

/// Personal with the decomp boards, as the person made them.
fn decomp(r: &Rig) -> (String, String) {
    let wheels = r
        .core
        .create_personal_board("Hot Wheels micro racers Decomp", "wheels")
        .unwrap();
    let buzz = r
        .core
        .create_personal_board("Buzzing Cars Decomp", "buzz")
        .unwrap();
    (wheels, buzz)
}

/// board_create's board takes a Personal board's tag (any case) or id, and
/// the card stays private, numbered on that board; personal is the
/// default. An unknown board lists Personal's boards.
#[test]
fn board_create_puts_a_card_on_a_personal_board() {
    let r = Rig::new();
    let (wheels, buzz) = decomp(&r);

    let out = tool(
        &r,
        "board_create",
        serde_json::json!({"title": "map the save format", "board": "wheels"}),
    );
    assert!(
        out.contains("Created wheels#1")
            && out.contains("on Hot Wheels micro racers Decomp (wheels) in Personal"),
        "{out}"
    );
    assert!(!out.contains("Visible to"), "a private card: {out}");
    let t = task(&r, "wheels#1");
    assert_eq!(t.board, wheels);
    assert_eq!(r.core.route(&t.id).unwrap().0, "", "it is a private card");

    let out = tool(
        &r,
        "board_create",
        serde_json::json!({"title": "audio", "board": "#BUZZ"}),
    );
    assert!(out.contains("Created buzz#1"), "{out}");
    let out = tool(
        &r,
        "board_create",
        serde_json::json!({"title": "physics", "board": buzz}),
    );
    assert!(out.contains("Created buzz#2"), "{out}");
    assert_eq!(task(&r, "buzz#2").board, buzz);

    for b in ["personal", PERSONAL_BOARD] {
        let out = tool(
            &r,
            "board_create",
            serde_json::json!({"title": "chores", "board": b}),
        );
        assert!(out.contains("Created #"), "{b}: {out}");
        assert!(
            !out.contains(" on "),
            "{b}: the default says nothing: {out}"
        );
    }

    let out = tool(
        &r,
        "board_create",
        serde_json::json!({"title": "x", "board": "colors"}),
    );
    assert!(
        out.starts_with("Error")
            && out.contains("no board \"colors\"")
            && out.contains("Personal's: personal, wheels, buzz."),
        "{out}"
    );
}

/// A subtask made with parent, or by splitting, goes on its parent's
/// Personal board; a board that isn't the parent's is refused.
#[test]
fn subtasks_follow_their_parent_onto_its_personal_board() {
    let r = Rig::new();
    decomp(&r);
    tool(
        &r,
        "board_create",
        serde_json::json!({"title": "decomp", "board": "buzz",
            "steps": [{"text": "a"}, {"text": "b"}]}),
    );
    let out = tool(
        &r,
        "board_create",
        serde_json::json!({"title": "kid", "parent": "buzz#1"}),
    );
    assert!(out.contains("Created buzz#2"), "{out}");
    let out = tool(
        &r,
        "board_create",
        serde_json::json!({"title": "kid 2", "parent": "buzz#1", "board": "buzz"}),
    );
    assert!(out.contains("Created buzz#3"), "{out}");
    let out = tool(
        &r,
        "board_create",
        serde_json::json!({"title": "kid 3", "parent": "buzz#1", "board": "wheels"}),
    );
    assert!(
        out.starts_with("Error") && out.contains("its parent's board"),
        "{out}"
    );

    let out = tool(
        &r,
        "board_split",
        serde_json::json!({"task_id": "buzz#1", "subtasks": [
            {"title": "part a", "steps": ["s1"]},
            {"title": "part b", "steps": ["s2"]},
        ]}),
    );
    assert!(!out.starts_with("Error"), "{out}");
    assert_eq!(task(&r, "buzz#1").subtasks.len(), 4, "{out}");
    for k in ["buzz#4", "buzz#5"] {
        assert!(!task(&r, k).parent.is_empty(), "{k}");
    }
}

/// board_edit's board moves a private card and its subtasks to another of
/// Personal's boards, and back to the default; a subtask alone, or a move
/// to a shared space's board, is refused.
#[test]
fn board_edit_moves_a_card_between_personal_boards() {
    let r = Rig::new();
    let (wheels, _) = decomp(&r);
    let space = r.core.create_board("Wind", "wind").unwrap();
    // On the default board, as the agents' cards landed before #708.
    tool(&r, "board_create", serde_json::json!({"title": "decomp"}));
    let parent = task(&r, "#1");
    tool(
        &r,
        "board_create",
        serde_json::json!({"title": "kid", "parent": "#1"}),
    );
    tool(
        &r,
        "board_create",
        serde_json::json!({"title": "kid 2", "parent": "#1"}),
    );

    let out = tool(
        &r,
        "board_edit",
        serde_json::json!({"task_id": "#2", "board": "wheels"}),
    );
    assert!(
        out.starts_with("Error") && out.contains("move its parent"),
        "{out}"
    );
    let out = tool(
        &r,
        "board_edit",
        serde_json::json!({"task_id": "#1", "board": "wind"}),
    );
    assert!(
        out.starts_with("Error") && out.contains("board_share"),
        "{out}"
    );

    let out = tool(
        &r,
        "board_edit",
        serde_json::json!({"task_id": "#1", "board": "wheels"}),
    );
    assert!(
        out.contains("moved to Hot Wheels micro racers Decomp (wheels) in Personal as wheels#1, with its 2 subtasks")
            && out.contains("old ref #1 still finds it"),
        "{out}"
    );
    let t = r.core.coord().get_task(&parent.id).unwrap();
    assert_eq!(
        (t.board.as_str(), t.short_ref().as_str()),
        (wheels.as_str(), "wheels#1")
    );
    for k in &t.subtasks {
        assert_eq!(r.core.coord().get_task(k).unwrap().board, wheels);
    }
    assert_eq!(r.core.task_ref("#1").unwrap(), parent.id, "its old ref");

    let out = tool(
        &r,
        "board_edit",
        serde_json::json!({"task_id": "wheels#1", "board": "wheels", "title": "decomp it"}),
    );
    assert!(out.contains("already"), "{out}");

    let out = tool(
        &r,
        "board_edit",
        serde_json::json!({"task_id": "wheels#1", "board": "personal"}),
    );
    assert!(out.contains("moved to"), "{out}");
    assert_eq!(r.core.coord().get_task(&parent.id).unwrap().board, "");

    // A card in a shared space isn't moved by an agent.
    let co = r.core.board_coord(&space).unwrap();
    let by = r.core.board_key(&space, &r.key_a).unwrap();
    co.create_task(
        &by,
        coord::NewTask {
            title: "shared".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let out = tool(
        &r,
        "board_edit",
        serde_json::json!({"task_id": "wind#1", "board": "wheels"}),
    );
    assert!(
        out.starts_with("Error") && out.contains("shared space"),
        "{out}"
    );
}

/// The board tool lists Personal's boards by tag once there is more than
/// its default.
#[test]
fn the_board_tool_lists_personal_boards() {
    let r = Rig::new();
    let out = tool(&r, "board", serde_json::json!({}));
    assert!(!out.contains("Personal's boards"), "{out}");
    decomp(&r);
    let out = tool(&r, "board", serde_json::json!({}));
    assert!(
        out.contains(
            "personal \"Personal\", wheels \"Hot Wheels micro racers Decomp\", buzz \"Buzzing Cars Decomp\""
        ),
        "{out}"
    );
}

/// A card made from a thread on one of Personal's boards goes on that
/// board when board is left out. A thread is there when its space moved
/// into Personal (card #640).
#[test]
fn a_card_from_a_thread_on_a_board_goes_on_that_board() {
    let r = Rig::new();
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
        &r,
        "board_comment",
        serde_json::json!({"task_id": "colors#1", "text": "the menus and the AI"}),
    );
    assert!(!out.starts_with("Error"), "{out}");
    let out = tool(
        &r,
        "board_create",
        serde_json::json!({"title": "the rest", "from": "colors#1"}),
    );
    assert!(
        out.contains("Created colors#2")
            && out.contains("on Colors of War Decomp (colors) in Personal"),
        "{out}"
    );
    assert_eq!(task(&r, "colors#2").from, th.id);
    assert_eq!(task(&r, "colors#2").board, m.board_id);
}
