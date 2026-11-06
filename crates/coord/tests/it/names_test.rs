//! Card #98: coord's notice texts name agents by name, "name (key)",
//! also once they've left the roster; an unnamed agent stays its key.

use crate::common;

use common::*;
use coord::*;

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

#[test]
fn notices_name_the_actor() {
    let e = Env::new();
    e.ros.set_name(ALICE, "oak");
    let t = e.c.create_task(ALICE, nt("x")).unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
    let (_, n) = e.c.assign(ALICE, &t.id, BOB).unwrap();
    assert!(
        n.text
            .starts_with(&format!("oak ({ALICE}) assigned you a task")),
        "{}",
        n.text
    );
    // Gone from the roster, the name still shows.
    e.ros.set(ALICE, "");
    let (_, n) = e.c.add_comment(ALICE, &t.id, "hi", false).unwrap();
    assert!(
        n.text.starts_with(&format!("oak ({ALICE}) commented on")),
        "{}",
        n.text
    );
}

#[test]
fn an_unnamed_agent_stays_its_key() {
    let e = Env::new();
    let t = e.c.create_task(ALICE, nt("x")).unwrap();
    let (_, n) = e.c.assign(ALICE, &t.id, BOB).unwrap();
    assert!(
        n.text.starts_with(&format!("{ALICE} assigned you a task")),
        "{}",
        n.text
    );
}
