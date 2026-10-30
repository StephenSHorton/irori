//! Port of I:internal/coord/subtasks_test.go.

mod common;

use std::collections::BTreeMap;

use common::*;
use coord::rows::{TaskCreatedRow, TaskParentRow};
use coord::*;
use store::{Event, kinds};

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

fn step(text: &str) -> Step {
    Step {
        text: text.into(),
        ..Step::default()
    }
}

fn get(c: &Coord, id: &str) -> Task {
    c.get_task(id).unwrap_or_else(|| panic!("no task {id}"))
}

fn sub(title: &str, steps: &[&str]) -> NewSubtask {
    NewSubtask {
        title: title.into(),
        steps: steps.iter().map(|s| s.to_string()).collect(),
        ..NewSubtask::default()
    }
}

fn row(dev: &str, seq: i64, at: u64, kind: &str, raw: Vec<u8>) -> Event {
    Event {
        device_id: dev.into(),
        seq,
        hlc: hlc::Timestamp(at),
        actor: format!("agent:{dev}/s"),
        kind: kind.into(),
        payload: raw,
        sig: Vec::new(),
    }
}

fn created_row(dev: &str, seq: i64, at: u64, id: &str, num: i64) -> Event {
    let raw = gocompat::json::marshal(&TaskCreatedRow {
        task_id: id.into(),
        num,
        title: id.into(),
        by: format!("{dev}/s"),
        at: fixed_now(),
        ..TaskCreatedRow::default()
    })
    .unwrap();
    row(dev, seq, at, kinds::TASK_CREATED, raw)
}

fn parent_row(dev: &str, seq: i64, at: u64, child: &str, parent: &str) -> Event {
    let raw = gocompat::json::marshal(&TaskParentRow {
        task_id: child.into(),
        parent: parent.into(),
        by: format!("{dev}/s"),
        at: fixed_now(),
    })
    .unwrap();
    row(dev, seq, at, kinds::TASK_PARENT, raw)
}

fn done() -> StepUpdate {
    StepUpdate {
        done: Some(true),
        doing: None,
    }
}

#[test]
fn test_split_and_rollup() {
    let e = Env::new();
    let big =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "big".into(),
                steps: vec![
                    step("design"),
                    step("backend"),
                    step("tools"),
                    step("window"),
                ],
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(ALICE, &big.id).unwrap();
    e.c.set_step(ALICE, &big.id, "s1", done()).unwrap();
    let err =
        e.c.split(ALICE, &big.id, vec![sub("a", &["s2"])], vec![])
            .unwrap_err();
    assert!(
        err.to_string().contains("s3") && err.to_string().contains("s4"),
        "{err}"
    );
    assert_eq!(
        err.to_string(),
        "coord: invalid argument: every open step must go to a subtask; not placed: s3 \"tools\", s4 \"window\""
    );
    assert!(
        e.c.split(
            ALICE,
            &big.id,
            vec![sub("a", &["s1", "s2", "s3", "s4"])],
            vec![]
        )
        .is_err()
    );
    assert!(
        e.c.split(
            ALICE,
            &big.id,
            vec![sub("a", &["s2", "s3"]), sub("b", &["s3", "s4"])],
            vec![]
        )
        .is_err()
    );
    want_err(
        e.c.split(BOB, &big.id, vec![sub("a", &["s2", "s3", "s4"])], vec![]),
        ErrorKind::NotAllowed,
    );
    let (p, kids) =
        e.c.split(
            ALICE,
            &big.id,
            vec![
                sub("backend", &["s2", "extra test"]),
                sub("tools", &["s3"]),
                sub("window", &["s4"]),
            ],
            vec![],
        )
        .unwrap();
    assert!(kids.len() == 3 && p.subtasks.len() == 3 && p.subtasks[0] == kids[0].id);
    assert!(p.steps.len() == 1 && p.steps[0].done && p.steps[0].text == "design");
    let k = &kids[0];
    assert!(
        k.parent == big.id
            && k.steps.len() == 2
            && k.steps[0].text == "backend"
            && k.steps[1].text == "extra test"
    );
    let r = p.rollup.clone().unwrap();
    assert!(r.state == SUBMITTED && r.done == 0 && r.total == 3);
    e.c.claim_task(BOB, &kids[0].id).unwrap();
    e.c.claim_task(CAROL, &kids[1].id).unwrap();
    assert_eq!(get(&e.c, &big.id).rollup.unwrap().state, WORKING);
    e.c.raise(
        BOB,
        &kids[0].id,
        NewBlocker {
            question: "which db?".into(),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    let r = get(&e.c, &big.id).rollup.unwrap();
    assert!(r.state == BLOCKED && r.blocked == kids[0].id && r.blocked_question == "which db?");
    let v = e.c.board(&BoardFilter::default());
    assert!(
        v.blocked
            .iter()
            .chain(&v.needs_you)
            .any(|c| c.task.id == big.id)
    );
    want_err(
        e.c.set_steps(ALICE, &big.id, vec![step("x")]),
        ErrorKind::Subtask,
    );
    want_err(
        e.c.set_task_state(ALICE, &big.id, INPUT_REQUIRED, ""),
        ErrorKind::Subtask,
    );
    let err =
        e.c.set_task_state(ALICE, &big.id, COMPLETED, "")
            .unwrap_err();
    assert!(
        err.is(ErrorKind::Subtask)
            && err.is(ErrorKind::Invalid)
            && err.to_string().contains(&kids[0].short_ref())
    );
    e.c.set_task_state(BOB, &kids[0].id, COMPLETED, "").unwrap();
    e.c.claim_task(ALICE, &kids[2].id).unwrap();
    e.c.set_task_state(ALICE, &kids[2].id, CANCELED, "not needed")
        .unwrap();
    let r = get(&e.c, &big.id).rollup.unwrap();
    assert!(r.done == 1 && r.total == 2 && r.state == WORKING, "{r:?}");
    e.c.set_task_state(CAROL, &kids[1].id, COMPLETED, "")
        .unwrap();
    let pp = get(&e.c, &big.id);
    assert!(
        pp.rollup.as_ref().unwrap().state == COMPLETED
            && effective_state(&pp) == COMPLETED
            && pp.state != COMPLETED
    );
    assert!(
        e.c.board(&BoardFilter::default())
            .done
            .iter()
            .any(|c| c.task.id == big.id)
    );
    let r = get(&e.reopen(), &big.id).rollup.unwrap();
    assert!(r.state == COMPLETED && r.done == 2 && r.total == 2);
}

#[test]
fn test_subtasks_are_one_level() {
    let e = Env::new();
    let p = e.c.create_task(ALICE, nt("parent")).unwrap();
    let k =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "kid".into(),
                parent: p.short_ref(),
                ..NewTask::default()
            },
        )
        .unwrap();
    assert!(k.parent == p.id && get(&e.c, &p.id).subtasks == [k.id.clone()]);
    let err =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "grandkid".into(),
                parent: k.id.clone(),
                ..NewTask::default()
            },
        )
        .unwrap_err();
    assert!(
        err.is(ErrorKind::Subtask) && err.to_string().contains(&p.short_ref()),
        "{err}"
    );
    let other = e.c.create_task(ALICE, nt("other")).unwrap();
    want_err(
        e.c.split(ALICE, &other.id, vec![], vec![p.id.clone()]),
        ErrorKind::Subtask,
    );
    want_err(
        e.c.split(ALICE, &k.id, vec![sub("x", &[])], vec![]),
        ErrorKind::Subtask,
    );
    e.c.split(ALICE, &p.id, vec![], vec![other.short_ref()])
        .unwrap();
    assert_eq!(get(&e.c, &p.id).subtasks, [k.id.clone(), other.id.clone()]);
}

#[test]
fn test_cancel_parent_cascades() {
    let e = Env::new();
    let p = e.c.create_task(ALICE, nt("parent")).unwrap();
    let a =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "a".into(),
                parent: p.id.clone(),
                ..NewTask::default()
            },
        )
        .unwrap();
    let b =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "b".into(),
                parent: p.id.clone(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(BOB, &a.id).unwrap();
    e.c.set_task_state(BOB, &a.id, COMPLETED, "").unwrap();
    e.c.set_task_state(ALICE, &p.id, CANCELED, "dropped")
        .unwrap();
    assert_eq!(get(&e.c, &a.id).state, COMPLETED);
    let bt = get(&e.c, &b.id);
    assert_eq!(bt.state, CANCELED);
    assert_eq!(
        bt.note,
        format!("its parent {} was canceled", p.short_ref())
    );
    assert_eq!(effective_state(&get(&e.c, &p.id)), CANCELED);
}

#[test]
fn test_two_level_link_settles_in_any_order() {
    let base = hlc_at(fixed_now());
    let mut rows = vec![
        created_row("dev-b", 1, base + 1, "A", 1),
        created_row("dev-b", 2, base + 2, "B", 2),
        created_row("dev-b", 3, base + 3, "C", 3),
        parent_row("dev-b", 4, base + 10, "B", "A"),
        parent_row("dev-c", 1, base + 20, "C", "B"),
    ];
    let check = |name: &str, c: &Coord| {
        let (a, b, cc) = (get(c, "A"), get(c, "B"), get(c, "C"));
        assert!(
            b.parent == "A" && cc.parent.is_empty() && a.subtasks == ["B"] && b.subtasks.is_empty(),
            "{name}"
        );
    };
    let orders = BTreeMap::from([
        ("in order", vec![0, 1, 2, 3, 4]),
        ("c first", vec![4, 0, 1, 2, 3]),
        ("late link", vec![0, 1, 2, 4, 3]),
    ]);
    for (name, order) in orders {
        let e = Env::new();
        for i in order {
            e.c.handle(&rows[i]).unwrap();
        }
        check(name, &e.c);
    }
    let e = Env::new();
    for ev in &rows {
        e.st.insert(ev).unwrap();
    }
    check("replay", &e.reopen());
    rows[3] = parent_row("dev-b", 4, base + 20, "B", "A");
    rows[4] = parent_row("dev-c", 1, base + 10, "C", "B");
    let e = Env::new();
    for ev in &rows {
        e.c.handle(ev).unwrap();
    }
    assert!(get(&e.c, "B").parent.is_empty() && get(&e.c, "C").parent == "B");
}

#[test]
fn test_subtask_changes_reach_the_parents_holder() {
    let e = Env::new();
    let p = e.c.create_task(ALICE, nt("parent")).unwrap();
    e.c.claim_task(ALICE, &p.id).unwrap();
    let k =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "kid".into(),
                parent: p.id.clone(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(BOB, &k.id).unwrap();
    let ns = e.c.sweep_notices().unwrap();
    assert!(
        ns.iter().any(|n| n.to.iter().any(|t| t == ALICE)
            && n.text.contains(&format!("(part of {})", p.short_ref()))),
        "{ns:?}"
    );
}

#[test]
fn test_asks_and_subtasks() {
    let e = Env::new();
    let p = e.c.create_task(ALICE, nt("parent")).unwrap();
    let agents = {
        use coord::roster::Agents as _;
        e.ros.agents()
    };
    let (ask, _) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "can someone look at the login flow?".into(),
                ..NewAsk::default()
            },
            &agents,
        )
        .unwrap();
    want_err(
        e.c.create_task(
            ALICE,
            NewTask {
                title: "kid".into(),
                parent: ask.id.clone(),
                ..NewTask::default()
            },
        ),
        ErrorKind::Subtask,
    );
    want_err(
        e.c.split(ALICE, &p.id, vec![], vec![ask.id.clone()]),
        ErrorKind::Subtask,
    );
    want_err(
        e.c.split(USER, &ask.id, vec![sub("x", &[])], vec![]),
        ErrorKind::Subtask,
    );
    let base = hlc_at(fixed_now());
    e.remote(
        "dev-b",
        1,
        base + 1,
        "agent:dev-b/s",
        kinds::TASK_PARENT,
        &TaskParentRow {
            task_id: ask.id.clone(),
            parent: p.id.clone(),
            by: "dev-b/s".into(),
            at: fixed_now(),
        },
    );
    assert!(get(&e.c, &ask.id).parent.is_empty());
    e.c.ask_claim(BOB, &ask.id, std::time::Duration::ZERO)
        .unwrap();
    assert!(
        e.c.ask_close(
            BOB,
            &ask.id,
            AskOutcome {
                kind: OUTCOME_NONE.into(),
                note: "x".into(),
                parent: p.id.clone(),
                ..AskOutcome::default()
            }
        )
        .is_err()
    );
    e.c.ask_close(
        BOB,
        &ask.id,
        AskOutcome {
            kind: OUTCOME_CARD.into(),
            note: "it's a card".into(),
            title: "Fix login".into(),
            parent: p.short_ref(),
            ..AskOutcome::default()
        },
    )
    .unwrap();
    assert!(get(&e.c, &ask.id).parent == p.id && get(&e.c, &p.id).subtasks.contains(&ask.id));
    assert_eq!(get(&e.reopen(), &ask.id).parent, p.id);
}

#[test]
fn test_rejected_reparent_keeps_the_old_parent() {
    let base = hlc_at(fixed_now());
    let rows = [
        created_row("dev-b", 1, base + 1, "A", 1),
        created_row("dev-b", 2, base + 2, "B", 2),
        created_row("dev-b", 3, base + 3, "C", 3),
        created_row("dev-b", 4, base + 4, "D", 4),
        parent_row("dev-b", 5, base + 10, "B", "A"),
        parent_row("dev-b", 6, base + 11, "C", "D"),
        parent_row("dev-c", 1, base + 20, "B", "C"),
    ];
    let check = |name: &str, c: &Coord| {
        assert_eq!(get(c, "B").parent, "A", "{name}");
        assert_eq!(get(c, "A").subtasks, ["B"], "{name}");
    };
    for (name, order) in [
        ("in order", vec![0, 1, 2, 3, 4, 5, 6]),
        ("reparent first", vec![6, 0, 1, 2, 3, 4, 5]),
    ] {
        let e = Env::new();
        for i in order {
            e.c.handle(&rows[i]).unwrap();
        }
        check(name, &e.c);
    }
    let e = Env::new();
    for ev in &rows {
        e.st.insert(ev).unwrap();
    }
    check("replay", &e.reopen());
}

#[test]
fn test_adoption_reaches_the_parents_holder() {
    let e = Env::new();
    let p =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "parent".into(),
                steps: vec![step("plan"), step("build")],
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(CAROL, &p.id).unwrap();
    e.c.set_step(CAROL, &p.id, "s1", done()).unwrap();
    let other = e.c.create_task(BOB, nt("related")).unwrap();
    e.c.sweep_notices().unwrap();
    e.c.split(
        ALICE,
        &p.id,
        vec![sub("build it", &["s2"])],
        vec![other.id.clone()],
    )
    .unwrap();
    let ns = e.c.sweep_notices().unwrap();
    let (mut adopted, mut made, mut parent_line) = (false, false, false);
    for n in ns.iter().filter(|n| n.to.iter().any(|t| t == CAROL)) {
        adopted |= n
            .text
            .contains(&format!("made it part of {}", p.short_ref()));
        made |= n.text.contains(&format!(
            "put the card up as a subtask of {}",
            p.short_ref()
        ));
        if n.task_id == p.id {
            parent_line = n.text.contains("subtasks 0/2 done") && !n.text.contains("steps 1/1");
        }
    }
    assert!(adopted && made && parent_line, "{ns:?}");
    let sh = show(&e.c, &p.id);
    assert!(
        sh.contains("steps done before the split:") && !sh.contains("steps (1/1"),
        "show parent steps:
{sh}"
    );
}

/// The show text of task id.
fn show(c: &Coord, id: &str) -> String {
    search::Source::search_show(c, &format!("task:{id}"), &search::ShowOptions::default())
        .unwrap()
        .text
}

// show names a subtask's parent and lists a parent's subtasks.
#[test]
fn test_show_subtasks() {
    let e = Env::new();
    let p = e.c.create_task(ALICE, nt("parent")).unwrap();
    let k =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "kid".into(),
                parent: p.id.clone(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(BOB, &k.id).unwrap();
    let sh = show(&e.c, &p.id);
    assert!(
        sh.contains(&format!(
            "subtasks: 
- {} [working] \"kid\": {BOB}",
            k.short_ref()
        )) && sh.contains("from its subtasks: 0/1 done"),
        "show parent:
{sh}"
    );
    let sh = show(&e.c, &k.id);
    assert!(
        sh.contains(&format!("part of: {} \"parent\"", p.short_ref())),
        "show subtask:
{sh}"
    );
}
