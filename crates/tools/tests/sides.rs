//! Card #496 through the agent tools: an agent sees which side its device
//! runs on the board, and its own board_join or board_assign of a card on
//! another side is refused with the ask-the-person reason; once the person
//! assigns it, board_join goes through.

mod common;

use common::*;

fn work_card(h: &Harness) -> String {
    h.b.co.set_device_sides(&["Personal".to_string()]).unwrap();
    let t =
        h.b.co
            .create_task(
                "user:dev1",
                coord::NewTask {
                    title: "Quarterly report".into(),
                    side: "Work".into(),
                    ..coord::NewTask::default()
                },
            )
            .unwrap();
    t.id
}

#[test]
fn the_board_names_this_devices_side_and_each_cards() {
    let h = new_harness();
    work_card(&h);
    let out = h.okj(&call_a(), "board", json!({"repo": "*"}));
    has(
        &out,
        &[
            "This device runs the Personal side. Cards on Work are only read here, never taken",
            "\"Quarterly report\"; Work side",
        ],
    );
}

#[test]
fn an_agents_own_board_join_and_assign_across_sides_are_refused() {
    let h = new_harness();
    let id = work_card(&h);
    let out = h.callj(&call_a(), "board_join", json!({"task_id": id}));
    has(
        &out,
        &[
            "Error",
            "on the Work side",
            "runs only Personal",
            coord::ASK_THE_PERSON,
        ],
    );
    let out = h.callj(
        &call_a(),
        "board_assign",
        json!({"task_id": id, "to": "beta"}),
    );
    has(
        &out,
        &[
            "Error",
            "on the Work side",
            "beta's device",
            coord::ASK_THE_PERSON,
        ],
    );
    has_not(&out, &["lists its holders"]);
    assert!(h.b.co.get_task(&id).unwrap().holders.is_empty());
}

#[test]
fn after_the_person_assigns_it_board_join_goes_through() {
    let h = new_harness();
    let id = work_card(&h);
    h.b.co.assign_task("user:dev1", &id, KEY_A).unwrap();
    let out = h.okj(&call_a(), "board_join", json!({"task_id": id}));
    has(&out, &["You hold"]);
}
