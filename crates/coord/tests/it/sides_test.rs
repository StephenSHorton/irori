//! Work and personal sides (card #494): a card takes its device's side,
//! subtasks and splits take their parent's, and a device's pin counts
//! only from that device.

use crate::common;

use common::*;
use coord::rows::TaskParentRow;
use coord::sides::{KIND, SidesRow};
use coord::*;

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

fn sub(title: &str) -> NewSubtask {
    NewSubtask {
        title: title.into(),
        ..NewSubtask::default()
    }
}

fn sides(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn pin_row(dev: &str, list: &[&str]) -> SidesRow {
    SidesRow {
        device_id: dev.into(),
        sides: sides(list),
        at_device: dev.into(),
    }
}

#[test]
fn a_new_card_takes_its_devices_first_side() {
    let e = Env::new();
    e.c.set_device_sides(&sides(&["Work", "Personal"])).unwrap();
    let t = e.c.create_task(ALICE, nt("Fix the build")).unwrap();
    assert_eq!(t.side, "Work");
    assert_eq!(e.c.home_side(), "Work");
    // The row is in the log: a replay folds the same side.
    let again = e.reopen();
    assert_eq!(again.get_task(&t.id).unwrap().side, "Work");
    assert_eq!(again.sides_of("dev-a"), Some(sides(&["Work", "Personal"])));
}

#[test]
fn a_side_asked_for_wins_over_the_devices() {
    let e = Env::new();
    e.c.set_device_sides(&sides(&["Work"])).unwrap();
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                side: "  Personal ".into(),
                ..nt("Taxes")
            },
        )
        .unwrap();
    assert_eq!(t.side, "Personal");
}

#[test]
fn with_no_row_a_card_has_no_side_and_every_side_runs() {
    let e = Env::new();
    let t = e.c.create_task(ALICE, nt("Anything")).unwrap();
    assert_eq!(t.side, "");
    assert_eq!(e.c.sides_of("dev-a"), None);
    assert!(e.c.runs_side("dev-a", "Work"));
    assert!(e.c.runs_side("dev-a", "Personal"));
    assert!(e.c.runs_side("dev-b", "Work"));
    assert!(e.c.device_sides().is_empty());
}

#[test]
fn a_pinned_device_runs_only_its_sides_and_unsided_cards() {
    let e = Env::new();
    e.c.set_device_sides(&sides(&["Work"])).unwrap();
    assert!(e.c.runs_side("dev-a", "Work"));
    assert!(e.c.runs_side("dev-a", "work"), "case doesn't count");
    assert!(!e.c.runs_side("dev-a", "Personal"));
    assert!(
        e.c.runs_side("dev-a", ""),
        "a card with no side runs anywhere"
    );
}

#[test]
fn an_empty_pin_runs_every_side_again() {
    let e = Env::new();
    e.c.set_device_sides(&sides(&["Work"])).unwrap();
    e.c.set_device_sides(&[]).unwrap();
    assert_eq!(e.c.sides_of("dev-a"), None);
    assert!(e.c.runs_side("dev-a", "Personal"));
    assert_eq!(e.c.create_task(ALICE, nt("x")).unwrap().side, "");
}

#[test]
fn subtasks_and_splits_take_their_parents_side() {
    let e = Env::new();
    e.c.set_device_sides(&sides(&["Work"])).unwrap();
    let p = e.c.create_task(ALICE, nt("Parent")).unwrap();
    assert_eq!(p.side, "Work");
    // This device moves to Personal: the parent's parts stay on Work.
    e.c.set_device_sides(&sides(&["Personal"])).unwrap();
    let child =
        e.c.create_task(
            ALICE,
            NewTask {
                parent: p.id.clone(),
                ..nt("Child")
            },
        )
        .unwrap();
    assert_eq!(child.side, "Work");
    let (_, parts) =
        e.c.split(ALICE, &p.id, vec![sub("a"), sub("b")], vec![])
            .unwrap();
    assert_eq!(parts.len(), 2);
    for t in &parts {
        assert_eq!(t.side, "Work", "{}", t.title);
    }
    // A fresh card follows the device's new pin.
    assert_eq!(e.c.create_task(ALICE, nt("New")).unwrap().side, "Personal");
}

#[test]
fn a_main_red_card_takes_the_reporting_cards_side() {
    let e = Env::new();
    e.c.set_device_sides(&sides(&["Work"])).unwrap();
    let a = e.c.create_task(ALICE, nt("Talk 3")).unwrap();
    e.c.set_device_sides(&sides(&["Personal"])).unwrap();
    let (red, made) = e.c.main_red(ALICE, &a.id, "attach_exe", "").unwrap();
    assert!(made);
    assert_eq!(red.side, "Work");
}

#[test]
fn an_ask_takes_its_devices_side() {
    let e = Env::new();
    e.c.set_device_sides(&sides(&["Personal"])).unwrap();
    let (t, _) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "what's for dinner".into(),
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap();
    assert_eq!(t.side, "Personal");
}

#[test]
fn another_devices_side_row_is_ignored() {
    let e = Env::new();
    let base = hlc_at(fixed_now());
    // dev-b signs a row pinning dev-a: it doesn't count, live or replayed.
    e.remote(
        "dev-b",
        1,
        base + 1,
        "user",
        KIND,
        &pin_row("dev-a", &["Work"]),
    );
    assert_eq!(e.c.sides_of("dev-a"), None);
    assert_eq!(e.c.create_task(ALICE, nt("x")).unwrap().side, "");
    assert_eq!(e.reopen().sides_of("dev-a"), None);
    // dev-b pinning itself counts, for dev-b only.
    e.remote(
        "dev-b",
        2,
        base + 2,
        "user",
        KIND,
        &pin_row("dev-b", &["Personal"]),
    );
    assert_eq!(e.c.sides_of("dev-b"), Some(sides(&["Personal"])));
    assert!(!e.c.runs_side("dev-b", "Work"));
    assert!(e.c.runs_side("dev-a", "Work"));
    assert_eq!(e.c.create_task(ALICE, nt("y")).unwrap().side, "");
    assert_eq!(e.reopen().sides_of("dev-b"), Some(sides(&["Personal"])));
}

#[test]
fn only_the_person_pins_sides() {
    let e = Env::new();
    let base = hlc_at(fixed_now());
    e.remote(
        "dev-b",
        1,
        base + 1,
        "agent:dev-b/s",
        KIND,
        &pin_row("dev-b", &["Work"]),
    );
    assert_eq!(e.c.sides_of("dev-b"), None);
}

#[test]
fn a_devices_newest_pin_wins() {
    let e = Env::new();
    let base = hlc_at(fixed_now());
    e.remote(
        "dev-b",
        1,
        base + 1,
        "user",
        KIND,
        &pin_row("dev-b", &["Work"]),
    );
    e.remote(
        "dev-b",
        2,
        base + 2,
        "user",
        KIND,
        &pin_row("dev-b", &["Personal"]),
    );
    assert_eq!(e.c.sides_of("dev-b"), Some(sides(&["Personal"])));
    assert_eq!(e.reopen().sides_of("dev-b"), Some(sides(&["Personal"])));
}

#[test]
fn side_names_gather_every_devices_pins_and_cards() {
    let e = Env::new();
    e.c.set_device_sides(&sides(&["Work", "work", " ", "Home lab"]))
        .unwrap();
    assert_eq!(e.c.sides_of("dev-a"), Some(sides(&["Work", "Home lab"])));
    let base = hlc_at(fixed_now());
    e.remote(
        "dev-b",
        1,
        base + 1,
        "user",
        KIND,
        &pin_row("dev-b", &["Personal", "WORK"]),
    );
    e.c.create_task(
        ALICE,
        NewTask {
            side: "Garden".into(),
            ..nt("Plant")
        },
    )
    .unwrap();
    assert_eq!(
        e.c.side_names(),
        sides(&["Work", "Home lab", "Personal", "Garden"])
    );
}

#[test]
fn a_shared_board_carries_no_sides() {
    let e = Env::new();
    let b = e.reopen_with(Options {
        board: true,
        ..e.opts()
    });
    assert!(b.set_device_sides(&sides(&["Work"])).is_err());
}

#[test]
fn the_show_text_names_the_side() {
    let e = Env::new();
    e.c.set_device_sides(&sides(&["Work"])).unwrap();
    let t = e.c.create_task(ALICE, nt("Fix the build")).unwrap();
    let text = search::Source::search_show(
        &e.c,
        &format!("task:{}", t.id),
        &search::ShowOptions::default(),
    )
    .unwrap()
    .text;
    assert!(text.contains("\nside: Work\n"), "{text}");
}

#[test]
fn an_adopted_card_runs_on_its_parents_side_until_it_leaves() {
    let e = Env::new();
    e.c.set_device_sides(&sides(&["Work"])).unwrap();
    let w = e.c.create_task(ALICE, nt("Work parent")).unwrap();
    e.c.set_device_sides(&sides(&["Personal"])).unwrap();
    let p = e.c.create_task(ALICE, nt("Personal card")).unwrap();
    assert_eq!(p.side, "Personal");
    e.c.split(ALICE, &w.id, vec![sub("a")], vec![p.id.clone()])
        .unwrap();
    assert_eq!(e.c.get_task(&p.id).unwrap().side, "Work");
    assert_eq!(e.reopen().get_task(&p.id).unwrap().side, "Work");
    // Out of its parent it has its own side again.
    e.write_raw(
        "agent:dev-a/alice-session",
        store::kinds::TASK_PARENT,
        &TaskParentRow {
            task_id: p.id.clone(),
            parent: String::new(),
            by: ALICE.into(),
            at: fixed_now(),
        },
    );
    let t = e.c.get_task(&p.id).unwrap();
    assert_eq!((t.parent.as_str(), t.side.as_str()), ("", "Personal"));
}
