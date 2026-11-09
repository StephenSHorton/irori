//! Card #570 through the tools: Not yet (board_later, board_create's
//! later) and order (after on board_create, after and not_after on
//! board_edit), and how the board and show lists say both.

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

fn plain(h: &Harness, title: &str) -> String {
    h.new_card(&call_a(), json!({"title": title}))
}

#[test]
fn a_card_made_after_others_waits_for_them() {
    let h = new_harness();
    let one = plain(&h, "Spaces");
    let two = plain(&h, "Board tags");
    let out = h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Per-board numbers", "after": ["#1", "#2"]}),
    );
    has(
        &out,
        &[
            "It comes after #1, #2: Irori hands it out once they are done.",
            "order: after #1, #2",
        ],
    );
    has_not(&out, &["Irori handed it to", "Irori starts an agent"]);
    let id = created_id(&out);
    let t = h.b.co.get_task(&id).unwrap();
    assert_eq!(t.state, coord::BLOCKED);
    // Each card it comes after shows what comes next.
    let board = h.okj(&call_a(), "board", json!({"repo": "*"}));
    has(&board, &["order: next #3"]);
    let _ = (one, two);
    // A card that doesn't exist is refused before anything is written.
    h.failsj(
        &call_a(),
        "board_create",
        json!({"title": "Nowhere", "after": ["#99"]}),
        &[],
    );
}

#[test]
fn a_card_made_not_yet_says_what_brings_it_back() {
    let h = new_harness();
    plain(&h, "Spaces");
    let cases = [
        (
            json!({"until": "#1", "why": "after the move"}),
            "comes back when #1 is done",
        ),
        (
            json!({"until": "2026-12-01", "why": "not before December"}),
            "comes back on 2026-12-01",
        ),
        (
            json!({"why": "the person will say"}),
            "comes back when the person says",
        ),
        (json!({}), "comes back when the person says"),
    ];
    for (later, says) in cases {
        let out = h.okj(
            &call_a(),
            "board_create",
            json!({"title": "Voice calls", "later": later}),
        );
        has(
            &out,
            &[
                &format!("It is Not yet: it {says}. Nobody starts it until then."),
                "[not yet]",
                "not yet [b",
            ],
        );
        has_not(&out, &["Irori handed it to", "  blocked ["]);
        let t = h.b.co.get_task(&created_id(&out)).unwrap();
        assert!(t.parked().is_some(), "{:?}", t.blockers);
    }
    // The why goes on the card.
    let out = h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Phone app", "later": {"until": "", "why": "after the desktop ships"}}),
    );
    has(&out, &["why: \"after the desktop ships\""]);
    h.failsj(
        &call_a(),
        "board_create",
        json!({"title": "Voice", "later": {"until": "soon", "why": "x"}}),
        &["until takes a card (#12 or tag#12), a date (2026-12-01)"],
    );
    h.failsj(
        &call_a(),
        "board_create",
        json!({"title": "Voice", "later": {"until": "2026-01-01", "why": "x"}}),
        &["has passed"],
    );
    h.failsj(
        &call_a(),
        "board_create",
        json!({"title": "Voice", "later": "soon"}),
        &["argument later should be an object"],
    );
}

/// A card made Not yet with assign_to isn't given to anyone (card #570).
#[test]
fn a_card_made_not_yet_isnt_assigned() {
    let h = new_harness();
    let out = h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Voice calls", "assign_to": KEY_B, "later": {"why": "after the move"}}),
    );
    has(
        &out,
        &[
            "It is Not yet: it comes back when the person says.",
            "It isn't assigned",
        ],
    );
    has_not(&out, &["Assigning it failed", "Assigned to"]);
    let t = h.b.co.get_task(&created_id(&out)).unwrap();
    assert!(t.parked().is_some() && t.holders.is_empty() && t.assigned.is_empty());
    // A subtask takes its parent's hold, and says so too.
    let out = h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Ringing", "parent": t.short_ref(), "assign_to": KEY_B}),
    );
    has(
        &out,
        &[
            "It is Not yet: it comes back when the person says.",
            "It isn't assigned",
        ],
    );
    has_not(&out, &["Assigning it failed", "Assigned to"]);
}

#[test]
fn board_edit_sets_the_order_and_refuses_a_circle() {
    let h = new_harness();
    let one = plain(&h, "Spaces");
    let _two = plain(&h, "Board tags");
    let three = plain(&h, "Per-board numbers");
    let out = h.okj(
        &call_a(),
        "board_edit",
        json!({"task_id": "#3", "after": ["#1", "#2"]}),
    );
    has(
        &out,
        &["Edited #3; it comes after #1, #2.", "order: after #1, #2"],
    );
    // Two cards can't each wait for the other: the circle is named.
    let out = h.failsj(
        &call_a(),
        "board_edit",
        json!({"task_id": "#1", "after": ["#3"]}),
        &["that makes a circle: #1 waits on #3, which waits on #1"],
    );
    has_not(&out, &["show #<n>"]);
    assert!(afters(&h, &one).is_empty());
    let out = h.okj(
        &call_a(),
        "board_edit",
        json!({"task_id": "#3", "not_after": ["#1"]}),
    );
    has(&out, &["Edited #3; it no longer comes after #1."]);
    assert_eq!(afters(&h, &three).len(), 1);
    h.failsj(
        &call_a(),
        "board_edit",
        json!({"task_id": "#3", "not_after": ["#1"]}),
        &["#3 doesn't come after #1"],
    );
    // Words and order in one call.
    let out = h.okj(
        &call_a(),
        "board_edit",
        json!({"task_id": "#3", "title": "Numbers per board", "not_after": ["#2"]}),
    );
    has(
        &out,
        &[
            "its thread says what changed",
            "it no longer comes after #2",
            "[submitted]",
        ],
    );
    h.failsj(
        &call_a(),
        "board_edit",
        json!({"task_id": "#3"}),
        &["after or not_after (its order), or weight"],
    );
}

fn afters(h: &Harness, id: &str) -> Vec<String> {
    let t = h.b.co.get_task(id).unwrap();
    t.open_blockers()
        .into_iter()
        .filter(|b| b.later.is_empty())
        .filter_map(|b| b.on.map(|on| on.reference))
        .collect()
}

#[test]
fn a_chain_shows_each_cards_place() {
    let h = new_harness();
    for t in ["One", "Two", "Three", "Four"] {
        plain(&h, t);
    }
    h.okj(
        &call_a(),
        "board_edit",
        json!({"task_id": "#2", "after": ["#1"]}),
    );
    h.okj(
        &call_a(),
        "board_edit",
        json!({"task_id": "#3", "after": ["#2"]}),
    );
    h.okj(
        &call_a(),
        "board_edit",
        json!({"task_id": "#4", "after": ["#3"]}),
    );
    let board = h.okj(&call_a(), "board", json!({"repo": "*"}));
    has(
        &board,
        &[
            "order: 1 of 4; next #2",
            "order: after #1; 2 of 4; next #3",
            "order: after #2; 3 of 4; next #4",
            "order: after #3; 4 of 4",
        ],
    );
    // One card's own lines find its place too.
    let out = h.okj(
        &call_a(),
        "board_edit",
        json!({"task_id": "#3", "title": "Three, renamed"}),
    );
    has(&out, &["order: after #2; 3 of 4; next #4"]);
}

#[test]
fn a_card_is_held_back_and_brought_back() {
    let h = new_harness();
    let id = plain(&h, "Voice calls");
    h.okj(&call_b(), "board_join", json!({"task_id": "#1"}));
    // Only its holder (or a person) puts a card someone is on aside.
    h.failsj(
        &call_a(),
        "board_later",
        json!({"task_id": "#1", "until": "2026-12-01", "why": "not before December"}),
        &["only its holder or a person may hold it back"],
    );
    let out = h.okj(
        &call_b(),
        "board_later",
        json!({"task_id": "#1", "until": "2026-12-01", "why": "not before December"}),
    );
    has(
        &out,
        &[
            "#1 is Not yet: it comes back on 2026-12-01.",
            "Nobody starts it until then",
            "[not yet]",
            "nobody holds it",
            "why: \"not before December\"",
        ],
    );
    let t = h.b.co.get_task(&id).unwrap();
    assert!(t.holders.is_empty(), "{:?}", t.holders);
    // The board lists it apart, after the open cards, and never as work.
    let board = h.okj(&call_a(), "board", json!({"repo": "*"}));
    has(
        &board,
        &[
            "Not yet (put aside: don't start or join these; each comes back by itself, or when the person brings it back) (1):",
            "not yet [b",
            "comes back on 2026-12-01",
        ],
    );
    has_not(&board, &["\nOpen (", "\nBlocked ("]);
    // An agent can't start it.
    let out = h.failsj(
        &call_c(),
        "board_join",
        json!({"task_id": "#1"}),
        &[
            "is held",
            "comes back by itself, or when the person brings it back",
        ],
    );
    has_not(&out, &["show #<n>"]);
    h.failsj(
        &call_a(),
        "board_later",
        json!({"task_id": "#1", "why": "x", "back": true}),
        &["leave out until and why"],
    );
    let out = h.okj(
        &call_a(),
        "board_later",
        json!({"task_id": "#1", "back": true, "note": "the person moved it up"}),
    );
    has(
        &out,
        &["Brought #1 back: it goes out again as if just posted."],
    );
    has_not(&out, &["[not yet]"]);
    h.failsj(
        &call_a(),
        "board_later",
        json!({"task_id": "#1", "back": true}),
        &["#1 isn't held"],
    );
    h.failsj(
        &call_a(),
        "board_later",
        json!({"task_id": "#1"}),
        &["give why"],
    );
}

fn created_id_of(h: &Harness, r: &str) -> String {
    h.b.co.task_ref(r).unwrap()
}

#[test]
fn changing_a_hold_keeps_the_card_not_yet() {
    let h = new_harness();
    plain(&h, "Voice calls");
    plain(&h, "Phone app");
    h.okj(
        &call_a(),
        "board_later",
        json!({"task_id": "#1", "until": "2026-12-01", "why": "not before December"}),
    );
    let out = h.okj(
        &call_a(),
        "board_later",
        json!({"task_id": "#1", "until": "#2", "why": "after the phone app"}),
    );
    has(
        &out,
        &[
            "#1 stays Not yet: it now comes back when #2 is done.",
            "why: \"after the phone app\"",
        ],
    );
    let t = h.b.co.get_task(&created_id_of(&h, "#1")).unwrap();
    assert_eq!(
        t.open_blockers()
            .iter()
            .filter(|b| !b.later.is_empty())
            .count(),
        1,
        "{:?}",
        t.blockers
    );
}

#[test]
fn only_the_person_brings_back_a_card_that_waits_for_their_word() {
    let h = new_harness();
    plain(&h, "Voice calls");
    let out = h.okj(
        &person(),
        "board_later",
        json!({"task_id": "#1", "why": "I'll say when"}),
    );
    has(
        &out,
        &["#1 is Not yet: it comes back when the person says."],
    );
    let out = h.failsj(
        &call_a(),
        "board_later",
        json!({"task_id": "#1", "back": true}),
        &["#1 waits for the person's word: only a person brings it back."],
    );
    has_not(&out, &["show #<n>"]);
    // Nor may an agent change it.
    h.failsj(
        &call_a(),
        "board_later",
        json!({"task_id": "#1", "until": "2026-12-01", "why": "x"}),
        &["only a person brings it back"],
    );
    let out = h.okj(
        &person(),
        "board_later",
        json!({"task_id": "#1", "back": true}),
    );
    has(&out, &["Brought #1 back"]);
}

#[test]
fn a_version_hold_names_the_device_and_a_card_back_keeps_its_order() {
    let h = new_harness();
    plain(&h, "Spaces");
    plain(&h, "Voice calls");
    // No device named: this one.
    let out = h.okj(
        &call_a(),
        "board_later",
        json!({"task_id": "#2", "until": "version 99.0.0", "why": "needs the new sync"}),
    );
    has(&out, &["#2 is Not yet: it comes back when", "runs 99.0.0"]);
    has_not(&out, &["when the person says"]);
    // Brought back while still after another card: it waits for that.
    h.okj(
        &call_a(),
        "board_edit",
        json!({"task_id": "#2", "after": ["#1"]}),
    );
    let out = h.okj(
        &call_a(),
        "board_later",
        json!({"task_id": "#2", "back": true}),
    );
    has(
        &out,
        &[
            "Brought #2 back: it still waits on its other items",
            "order: after #1",
        ],
    );
    let id = h.b.co.task_ref("#2").unwrap();
    assert_eq!(h.b.co.get_task(&id).unwrap().state, coord::BLOCKED);
}
