//! Personal's boards in the tools (card #698): board_create and board_share
//! take one by its tag or id, and the board tool lists them. The private
//! coord's board tags come from the harness's `personal` list, as the Core
//! reads them from the private log.

use crate::common;

use common::*;
use coord::TaskFilter;

/// Two boards past Personal's default, as the person made them.
fn with_boards() -> Harness {
    let h = new_harness();
    h.b.personal.lock().unwrap().extend([
        tools::PersonalBoard {
            board_id: "a1b2c3".into(),
            name: "Hot Wheels micro racers Decomp".into(),
            tag: "hw".into(),
        },
        tools::PersonalBoard {
            board_id: "d4e5f6".into(),
            name: "Buzzing Cars".into(),
            tag: "bc".into(),
        },
    ]);
    h
}

fn private_card(h: &Harness, title: &str) -> coord::Task {
    h.b.co
        .list_tasks(&TaskFilter::default())
        .into_iter()
        .find(|t| t.title == title)
        .unwrap_or_else(|| panic!("no card {title:?}"))
}

#[test]
fn board_create_puts_a_card_on_a_personal_board_by_tag() {
    let h = with_boards();
    let out = h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Decomp the loader", "board": "hw"}),
    );
    has(
        &out,
        &[
            "Created hw#1 (task:",
            "on \"Hot Wheels micro racers Decomp\" (hw), one of Personal's boards.",
        ],
    );
    has_not(&out, &["Visible to"]);
    let t = private_card(&h, "Decomp the loader");
    assert_eq!(t.board, "a1b2c3");
    assert_eq!(t.short_ref(), "hw#1");
    // By id, by #tag and in any case, each on its own board.
    h.okj(
        &call_a(),
        "board_create",
        json!({"title": "By id", "board": "d4e5f6"}),
    );
    assert_eq!(private_card(&h, "By id").board, "d4e5f6");
    h.okj(
        &call_a(),
        "board_create",
        json!({"title": "By hash tag", "board": "#BC"}),
    );
    assert_eq!(private_card(&h, "By hash tag").short_ref(), "bc#2");
    // `personal` is the default board: no board on the card's row.
    let out = h.okj(
        &call_a(),
        "board_create",
        json!({"title": "On the default", "board": "personal"}),
    );
    has(&out, &["on \"Personal\", Personal's default board."]);
    assert_eq!(private_card(&h, "On the default").board, "");
}

#[test]
fn board_create_refuses_an_unknown_board_and_a_subtask_with_one() {
    let h = with_boards();
    // No shared spaces here: the refusal names Personal's boards.
    h.failsj(
        &call_a(),
        "board_create",
        json!({"title": "x", "board": "nope"}),
        &["no board \"nope\"", "Personal's boards: hw, bc."],
    );
    let out = h.okj(&call_a(), "board_create", json!({"title": "Parent"}));
    assert!(out.starts_with("Created #1"), "{out}");
    h.failsj(
        &call_a(),
        "board_create",
        json!({"title": "Kid", "parent": "#1", "board": "hw"}),
        &["a subtask goes on its parent's board"],
    );
    assert_eq!(h.b.co.list_tasks(&TaskFilter::default()).len(), 1);
}

#[test]
fn the_board_tool_lists_personal_boards() {
    let h = with_boards();
    let out = h.okj(&call_a(), "board", json!({}));
    has(
        &out,
        &[
            "Personal's boards (board_create board=<board tag>",
            "private: only your person sees them): hw \"Hot Wheels micro racers Decomp\", bc \"Buzzing Cars\"; leave board out for its default, \"Personal\".",
        ],
    );
    has_not(&out, &["Spaces you may post on"]);
    // With only its default, Personal says nothing.
    let h = new_harness();
    let out = h.okj(&call_a(), "board", json!({}));
    has_not(&out, &["Personal's boards"]);
}

#[test]
fn board_share_moves_a_private_card_between_personal_boards() {
    let h = with_boards();
    h.okj(&call_a(), "board_create", json!({"title": "Wheels"}));
    let t = private_card(&h, "Wheels");
    assert_eq!(t.short_ref(), "#1");
    let out = h.okj(
        &call_a(),
        "board_share",
        json!({"task_id": "#1", "board": "hw"}),
    );
    has(
        &out,
        &[
            "Moved #1 to \"Hot Wheels micro racers Decomp\" (hw), one of Personal's boards as hw#1",
            "It stays private",
        ],
    );
    let t = h.b.co.get_task(&t.id).unwrap();
    assert_eq!((t.board.as_str(), t.short_ref()), ("a1b2c3", "hw#1".into()));
    // Already there: nothing moves.
    let out = h.okj(
        &call_a(),
        "board_share",
        json!({"task_id": "hw#1", "board": "HW"}),
    );
    has(&out, &["hw#1 is already on"]);
    // And back to the default.
    let out = h.okj(
        &call_a(),
        "board_share",
        json!({"task_id": "hw#1", "board": "personal"}),
    );
    has(
        &out,
        &["Moved hw#1 to \"Personal\", Personal's default board"],
    );
    assert_eq!(h.b.co.get_task(&t.id).unwrap().board, "");
}

#[test]
fn a_board_tagged_personal_wins_over_the_default() {
    let h = new_harness();
    h.b.personal.lock().unwrap().push(tools::PersonalBoard {
        board_id: "a1b2c3".into(),
        name: "Personal stuff".into(),
        tag: "personal".into(),
    });
    let out = h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Stuff", "board": "personal"}),
    );
    has(
        &out,
        &["on \"Personal stuff\" (personal), one of Personal's boards."],
    );
    assert_eq!(private_card(&h, "Stuff").board, "a1b2c3");
}
