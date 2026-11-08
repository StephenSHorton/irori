//! Card #963 through the tools: an agent weighs a card with board_edit,
//! the board and show say the weight, and a weight the person set is
//! refused to agents.

use crate::common;

use common::*;
use inbox::Sender;
use tools::Caller;

/// The person at the app.
fn person() -> Caller {
    Caller {
        sender: Sender::new("user:dev1", "you", "user"),
        path: inbox::path::MOD.into(),
    }
}

#[test]
fn an_agent_weighs_a_card_and_the_persons_weight_is_theirs() {
    let h = new_harness();
    h.new_card(&call_a(), json!({"title": "Plan the split"}));
    // A weight needs one line on why, and a level the board knows.
    h.failsj(
        &call_a(),
        "board_edit",
        json!({"task_id": "#1", "weight": {"level": "heavy"}}),
        &["give a weight with one line on why"],
    );
    h.failsj(
        &call_a(),
        "board_edit",
        json!({"task_id": "#1", "weight": {"level": "huge", "why": "x"}}),
        &["unknown weight"],
    );
    let out = h.okj(
        &call_a(),
        "board_edit",
        json!({"task_id": "#1", "weight": {"level": "heavy", "why": "planning the split"}}),
    );
    has(&out, &["it is heavy"]);
    let out = h.okj(&call_a(), "board", json!({}));
    has(&out, &["heavy, set by", ": planning the split"]);
    // Cleared, it runs as normal.
    let out = h.okj(
        &call_a(),
        "board_edit",
        json!({"task_id": "#1", "weight": {"level": ""}}),
    );
    has(&out, &["its weight is cleared: it runs as normal"]);
    let out = h.okj(&call_a(), "board", json!({}));
    has_not(&out, &["heavy, set by"]);

    // The person's weight is theirs: an agent can't change or clear it.
    h.okj(
        &person(),
        "board_edit",
        json!({"task_id": "#1", "weight": {"level": "light"}}),
    );
    let out = h.okj(&call_a(), "board", json!({}));
    has(&out, &["light, set by the person"]);
    for level in ["heavy", ""] {
        h.failsj(
            &call_a(),
            "board_edit",
            json!({"task_id": "#1", "weight": {"level": level, "why": "x"}}),
            &["was set by the person: only they change it"],
        );
    }
}
