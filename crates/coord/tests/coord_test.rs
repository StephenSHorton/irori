//! Port of I:internal/coord/coord_test.go.
#![allow(clippy::type_complexity)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use coord::rows::{LeaseAcquiredRow, LeaseReleasedRow, TaskCommentRow};
use coord::*;
use store::kinds;

#[test]
fn test_create_task() {
    let e = Env::new();
    let task =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "  Fix login ".into(),
                objective: "make it work".into(),
                repo_key: REPO_A.into(),
                scope: vec!["./src/auth/".into(), " ".into(), "docs/*.md".into()],
                ..NewTask::default()
            },
        )
        .unwrap();
    assert!(!task.id.is_empty());
    assert_eq!(task.title, "Fix login");
    assert_eq!(task.state, SUBMITTED);
    assert_eq!(task.created_by, ALICE);
    assert!(task.holder.is_empty());
    assert!(task.created_at.equal(&fixed_now()));
    assert_eq!(task.scope, ["src/auth/**", "docs/*.md"]);
    assert_eq!(e.c.get_task(&task.id), Some(task.clone()));
    let rows = e.rows(kinds::TASK_CREATED);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].actor, format!("agent:{ALICE}"));

    want_err(
        e.c.create_task(
            ALICE,
            NewTask {
                title: "  ".into(),
                ..NewTask::default()
            },
        ),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.create_task(
            "",
            NewTask {
                title: "x".into(),
                ..NewTask::default()
            },
        ),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.create_task(
            ALICE,
            NewTask {
                title: "x".into(),
                repo_key: "github.com/a/b".into(),
                ..NewTask::default()
            },
        ),
        ErrorKind::BadResource,
    );
    want_err(
        e.c.create_task(
            ALICE,
            NewTask {
                title: "x".into(),
                scope: vec![r"src\a".into()],
                ..NewTask::default()
            },
        ),
        ErrorKind::BadResource,
    );
    assert!(e.c.get_task("nope").is_none());
}

fn new_task(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

#[test]
fn test_claim_single_writer() {
    let e = Env::new();
    let task = e.c.create_task(ALICE, new_task("t")).unwrap();
    let got = e.c.claim_task(BOB, &task.id).unwrap();
    assert_eq!((got.holder.as_str(), got.state.as_str()), (BOB, WORKING));
    e.c.claim_task(BOB, &task.id).unwrap();
    assert_eq!(e.rows(kinds::TASK_CLAIMED).len(), 1);
    want_err(e.c.claim_task(CAROL, &task.id), ErrorKind::Held);
    want_err(e.c.claim_task(ALICE, &task.id), ErrorKind::Held);
    want_err(e.c.claim_task(BOB, "missing"), ErrorKind::NotFound);
}

#[test]
fn test_claim_from_gone_holder() {
    let e = Env::new();
    let task = e.c.create_task(ALICE, new_task("t")).unwrap();
    e.c.claim_task(BOB, &task.id).unwrap();
    e.ros.set(BOB, "");
    want_err(e.c.claim_task(CAROL, &task.id), ErrorKind::Held);
    e.clk.add(GONE_GRACE - secs(1));
    want_err(e.c.claim_task(CAROL, &task.id), ErrorKind::Held);
    e.ros.set(BOB, "busy");
    want_err(e.c.claim_task(CAROL, &task.id), ErrorKind::Held);
    e.ros.set(BOB, "gone");
    e.clk.add(secs(2));
    want_err(e.c.claim_task(CAROL, &task.id), ErrorKind::Held);
    e.clk.add(GONE_GRACE);
    let got = e.c.claim_task(CAROL, &task.id).unwrap();
    assert_eq!(got.holder, CAROL);
}

#[test]
fn test_person_holder_never_gone() {
    let e = Env::new();
    let task = e.c.create_task(USER, new_task("t")).unwrap();
    e.c.claim_task(USER, &task.id).unwrap();
    e.c.claim_task(USER, &task.id).unwrap();
    want_err(e.c.claim_task(BOB, &task.id), ErrorKind::Held);
    e.clk.add(Duration::from_secs(3600));
    want_err(e.c.claim_task(BOB, &task.id), ErrorKind::Held);
}

#[test]
fn test_concurrent_claims_one_wins() {
    let e = Env::new();
    let task = e.c.create_task(ALICE, new_task("race")).unwrap();
    let n = 16;
    let (mut wins, mut held) = (0, 0);
    std::thread::scope(|s| {
        let hs: Vec<_> = (0..n)
            .map(|i| {
                let c = &e.c;
                let id = task.id.clone();
                s.spawn(move || {
                    let who = format!(
                        "{}{}",
                        [ALICE, BOB, CAROL, "dev-a/x"][i % 4],
                        (b'a' + i as u8) as char
                    );
                    c.claim_task(&who, &id)
                })
            })
            .collect();
        for h in hs {
            match h.join().unwrap() {
                Ok(_) => wins += 1,
                Err(e) if e.is(ErrorKind::Held) => held += 1,
                Err(e) => panic!("{e}"),
            }
        }
    });
    assert_eq!((wins, held), (1, n - 1));
    assert_eq!(e.rows(kinds::TASK_CLAIMED).len(), 1);
}

#[test]
fn test_assign() {
    let e = Env::new();
    let task = e.c.create_task(ALICE, new_task("t")).unwrap();
    want_err(e.c.assign_task(BOB, &task.id, BOB), ErrorKind::NotAllowed);
    let got = e.c.assign_task(ALICE, &task.id, BOB).unwrap();
    assert_eq!((got.holder.as_str(), got.state.as_str()), (BOB, SUBMITTED));
    let got = e.c.assign_task(BOB, &task.id, CAROL).unwrap();
    assert_eq!(got.holder, CAROL);
    want_err(e.c.assign_task(BOB, &task.id, BOB), ErrorKind::NotAllowed);
    let got = e.c.assign_task(USER, &task.id, BOB).unwrap();
    assert_eq!(got.holder, BOB);
    e.c.assign_task(USER, &task.id, BOB).unwrap();
    assert_eq!(e.rows(kinds::TASK_ASSIGNED).len(), 3);
    want_err(e.c.assign_task(USER, &task.id, " "), ErrorKind::Invalid);
    want_err(e.c.assign_task(USER, "missing", BOB), ErrorKind::NotFound);
}

#[test]
fn test_set_task_state() {
    let e = Env::new();
    let task = e.c.create_task(ALICE, new_task("t")).unwrap();
    e.c.claim_task(BOB, &task.id).unwrap();
    want_err(
        e.c.set_task_state(CAROL, &task.id, INPUT_REQUIRED, ""),
        ErrorKind::NotAllowed,
    );
    want_err(
        e.c.set_task_state(BOB, &task.id, "done", ""),
        ErrorKind::BadState,
    );
    let got =
        e.c.set_task_state(BOB, &task.id, INPUT_REQUIRED, "which db?")
            .unwrap();
    assert_eq!(
        (got.state.as_str(), got.note.as_str()),
        (INPUT_REQUIRED, "which db?")
    );
    e.c.set_task_state(BOB, &task.id, INPUT_REQUIRED, "which db?")
        .unwrap();
    e.c.set_task_state(ALICE, &task.id, WORKING, "postgres")
        .unwrap();
    assert_eq!(e.rows(kinds::TASK_STATUS).len(), 2);
    e.clk.add(mins(1));
    let got =
        e.c.set_task_state(BOB, &task.id, COMPLETED, "shipped")
            .unwrap();
    assert_eq!((got.state.as_str(), got.holder.as_str()), (COMPLETED, BOB));
    assert!(got.updated_at.equal(&plus(fixed_now(), mins(1))));
    want_err(
        e.c.set_task_state(USER, &task.id, WORKING, ""),
        ErrorKind::BadState,
    );
    want_err(e.c.claim_task(CAROL, &task.id), ErrorKind::BadState);
    want_err(e.c.assign_task(USER, &task.id, CAROL), ErrorKind::BadState);
    let t2 = e.c.create_task(ALICE, new_task("t2")).unwrap();
    let got = e.c.set_task_state(USER, &t2.id, CANCELED, "").unwrap();
    assert_eq!(got.state, CANCELED);
}

#[test]
fn test_list_tasks() {
    let e = Env::new();
    let t1 =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "1".into(),
                repo_key: REPO_A.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.clk.add(secs(1));
    let t2 =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "2".into(),
                repo_key: REPO_B.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.clk.add(secs(1));
    let t3 =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "3".into(),
                repo_key: REPO_A.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(BOB, &t2.id).unwrap();
    e.c.claim_task(BOB, &t3.id).unwrap();
    e.c.set_task_state(BOB, &t3.id, FAILED, "").unwrap();
    let f = |repo: &str, holder: &str, open: bool| TaskFilter {
        repo_key: repo.into(),
        holder: holder.into(),
        open,
    };
    let cases: Vec<(TaskFilter, Vec<String>)> = vec![
        (
            f("", "", false),
            vec![t1.id.clone(), t2.id.clone(), t3.id.clone()],
        ),
        (f(REPO_A, "", false), vec![t1.id.clone(), t3.id.clone()]),
        (f("", BOB, false), vec![t2.id.clone(), t3.id.clone()]),
        (f("", "", true), vec![t1.id.clone(), t2.id.clone()]),
        (f("", BOB, true), vec![t2.id.clone()]),
        (f("root:none", "", false), vec![]),
    ];
    for (filter, want) in cases {
        assert_eq!(ids(&e.c.list_tasks(&filter)), want, "{filter:?}");
    }
    let mut t4 =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "4".into(),
                scope: vec!["a".into()],
                ..NewTask::default()
            },
        )
        .unwrap();
    t4.scope[0] = "mutated".into();
    assert_eq!(e.c.get_task(&t4.id).unwrap().scope[0], "a");
}

#[test]
fn test_lease_acquire_renew_expire() {
    let e = Env::new();
    let r = format!("path:{REPO_A}:src/**");
    let l = e.c.acquire(ALICE, &r, Duration::ZERO, false).unwrap();
    assert_eq!(
        (l.resource.as_str(), l.holder.as_str()),
        (r.as_str(), ALICE)
    );
    assert!(l.until.equal(&plus(fixed_now(), DEFAULT_TTL)) && l.acquired_at.equal(&fixed_now()));
    e.clk.add(mins(1));
    let l = e.c.acquire(ALICE, &r, mins(5), false).unwrap();
    assert!(l.until.equal(&plus(fixed_now(), mins(6))) && l.acquired_at.equal(&fixed_now()));
    let l =
        e.c.acquire(ALICE, &r, Duration::from_secs(100 * 3600), false)
            .unwrap();
    assert!(l.until.equal(&plus(fixed_now(), mins(1) + MAX_TTL)));
    assert_eq!(e.c.leases(), vec![l.clone()]);
    e.clk.add(MAX_TTL);
    assert!(e.c.leases().is_empty());
    assert!(e.rows("lease.released").is_empty());
    let l = e.c.acquire(BOB, &r, mins(1), false).unwrap();
    assert_eq!(l.holder, BOB);
    assert!(l.stolen_from.is_empty());
    want_err(
        e.c.acquire(BOB, "nope:x", Duration::ZERO, false),
        ErrorKind::BadResource,
    );
    want_err(
        e.c.acquire("", &r, Duration::ZERO, false),
        ErrorKind::Invalid,
    );
}

#[test]
fn test_lease_conflicts() {
    let e = Env::new();
    e.c.acquire(
        ALICE,
        &format!("path:{REPO_A}:src/**"),
        Duration::ZERO,
        false,
    )
    .unwrap();
    e.c.acquire(ALICE, "gpu/qwen", Duration::ZERO, false)
        .unwrap();
    e.c.acquire(ALICE, "port:8799", Duration::ZERO, false)
        .unwrap();
    let cases = [
        (format!("path:{REPO_A}:src/auth/login.go"), true),
        (format!("path:{REPO_A}:./src/"), true),
        (format!("path:{REPO_A}:**/*.go"), true),
        (format!("path:{REPO_A}:docs/**"), false),
        (format!("path:{REPO_B}:src/**"), false),
        ("gpu/qwen".into(), true),
        ("gpu/llama".into(), false),
        ("gpu".into(), false),
        ("port:8799".into(), true),
        ("port:8800".into(), false),
    ];
    for (res, is_held) in cases {
        let r = e.c.acquire(BOB, &res, Duration::ZERO, false);
        if is_held {
            assert!(
                r.as_ref().is_err_and(|e| e.is(ErrorKind::Held)),
                "{res}: {r:?}"
            );
        } else {
            assert!(r.is_ok(), "{res}: {r:?}");
        }
    }
    e.c.acquire(
        ALICE,
        &format!("path:{REPO_A}:src/a.go"),
        Duration::ZERO,
        false,
    )
    .unwrap();
}

#[test]
fn test_lease_steal() {
    let e = Env::new();
    e.c.acquire(
        BOB,
        &format!("path:{REPO_A}:src/a.go"),
        Duration::ZERO,
        false,
    )
    .unwrap();
    e.c.acquire(
        CAROL,
        &format!("path:{REPO_A}:src/b/**"),
        Duration::ZERO,
        false,
    )
    .unwrap();
    e.c.acquire(
        CAROL,
        &format!("path:{REPO_A}:docs/x.md"),
        Duration::ZERO,
        false,
    )
    .unwrap();
    let l =
        e.c.acquire(
            ALICE,
            &format!("path:{REPO_A}:src/**"),
            Duration::ZERO,
            true,
        )
        .unwrap();
    assert_eq!(l.stolen_from, format!("{BOB},{CAROL}"));
    let got = e.c.leases();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].resource, format!("path:{REPO_A}:docs/x.md"));
    assert_eq!(got[1].holder, ALICE);
    assert!(e.rows(kinds::LEASE_RELEASED).is_empty());
    let acq = e.rows(kinds::LEASE_ACQUIRED);
    let p: LeaseAcquiredRow = payload(acq.last().unwrap());
    assert_eq!(p.stolen.len(), 2);
    assert_eq!(
        (p.stolen[0].holder.as_str(), p.stolen[1].holder.as_str()),
        (BOB, CAROL)
    );
    let l =
        e.c.acquire(BOB, &format!("path:{REPO_A}:src/**"), Duration::ZERO, true)
            .unwrap();
    assert_eq!(l.stolen_from, ALICE);
    assert!(l.acquired_at.equal(&fixed_now()));
    let l =
        e.c.acquire(BOB, &format!("path:{REPO_A}:src/**"), Duration::ZERO, false)
            .unwrap();
    assert_eq!(l.stolen_from, ALICE);
}

#[test]
fn test_release() {
    let e = Env::new();
    let r = format!("path:{REPO_B}:a/**");
    e.c.acquire(ALICE, &r, Duration::ZERO, false).unwrap();
    want_err(e.c.release(BOB, &r), ErrorKind::Held);
    want_err(e.c.release(ALICE, "gpu/none"), ErrorKind::NotFound);
    want_err(e.c.release(ALICE, "bad resource"), ErrorKind::BadResource);
    e.c.release(ALICE, &format!("path:{REPO_B}:./a/")).unwrap();
    assert!(e.c.leases().is_empty());
    want_err(e.c.release(ALICE, &r), ErrorKind::NotFound);
    e.c.acquire(BOB, "gpu/qwen", Duration::ZERO, false).unwrap();
    e.c.release(USER, "gpu/qwen").unwrap();
    let rel = e.rows(kinds::LEASE_RELEASED);
    let last = rel.last().unwrap();
    let p: LeaseReleasedRow = payload(last);
    assert_eq!(
        (
            p.holder.as_str(),
            p.by.as_str(),
            p.reason.as_str(),
            last.actor.as_str()
        ),
        (BOB, USER, REASON_RELEASED, "user")
    );
    e.c.acquire(BOB, "gpu/qwen", mins(1), false).unwrap();
    e.clk.add(mins(1));
    want_err(e.c.release(BOB, "gpu/qwen"), ErrorKind::NotFound);
}

#[test]
fn test_sweep_releases_gone_holders() {
    let e = Env::new();
    e.c.acquire(ALICE, "gpu/qwen", MAX_TTL, false).unwrap();
    e.c.acquire(BOB, "port:8799", MAX_TTL, false).unwrap();
    e.c.acquire(USER, "port:9000", MAX_TTL, false).unwrap();
    let sweep = || e.c.sweep().unwrap();
    let holders = || {
        e.c.leases()
            .into_iter()
            .map(|l| l.holder)
            .collect::<Vec<_>>()
    };
    e.ros.set(ALICE, "");
    e.ros.set(BOB, "gone");
    sweep();
    e.clk.add(GONE_GRACE / 2);
    e.ros.set(BOB, "idle");
    sweep();
    e.ros.set(BOB, "");
    sweep();
    e.clk.add(GONE_GRACE / 2);
    sweep();
    assert_eq!(holders(), [BOB, USER]);
    e.clk.add(GONE_GRACE / 2);
    sweep();
    assert_eq!(holders(), [USER]);
    let rel = e.rows(kinds::LEASE_RELEASED);
    assert_eq!(rel.len(), 2);
    for ev in &rel {
        let p: LeaseReleasedRow = payload(ev);
        assert_eq!(
            (p.reason.as_str(), p.by.as_str(), ev.actor.as_str()),
            (REASON_HOLDER_GONE, "irori", "irori")
        );
    }
    e.clk.add(Duration::from_secs(3600));
    sweep();
    assert_eq!(holders(), [USER]);
}

#[test]
fn test_nil_roster_nobody_gone() {
    let e = Env::new();
    let c = Coord::new(
        e.st.clone(),
        None,
        Options {
            now: Some(e.clk.func()),
            gone_grace: secs(1),
            ..Options::default()
        },
    )
    .unwrap();
    c.acquire(ALICE, "gpu/qwen", Duration::ZERO, false).unwrap();
    let _ = c.sweep();
    e.clk.add(mins(1));
    let _ = c.sweep();
    assert_eq!(c.leases().len(), 1);
}

#[test]
fn test_gone_grace_override() {
    let e = Env::new();
    let c = e.reopen_with(Options {
        now: Some(e.clk.func()),
        gone_grace: secs(10),
        ..Options::default()
    });
    c.acquire(ALICE, "gpu/qwen", Duration::ZERO, false).unwrap();
    e.ros.set(ALICE, "");
    let _ = c.sweep();
    e.clk.add(secs(10));
    let _ = c.sweep();
    assert!(c.leases().is_empty());
}

#[test]
fn test_replay_equality() {
    let e = Env::new();
    let c = &e.c;
    let t1 = c
        .create_task(
            ALICE,
            NewTask {
                title: "a".into(),
                objective: "o".into(),
                done_criteria: "d".into(),
                repo_key: REPO_A.into(),
                scope: vec!["src/".into()],
                ..NewTask::default()
            },
        )
        .unwrap();
    e.clk.add(secs(1));
    let t2 = c.create_task(BOB, new_task("b")).unwrap();
    c.claim_task(BOB, &t1.id).unwrap();
    c.assign_task(BOB, &t2.id, CAROL).unwrap();
    c.set_task_state(CAROL, &t2.id, INPUT_REQUIRED, "?")
        .unwrap();
    c.set_task_state(BOB, &t1.id, COMPLETED, "done").unwrap();
    c.acquire(
        ALICE,
        &format!("path:{REPO_A}:src/**"),
        Duration::ZERO,
        false,
    )
    .unwrap();
    c.acquire(
        BOB,
        &format!("path:{REPO_A}:src/x.go"),
        Duration::ZERO,
        true,
    )
    .unwrap();
    c.acquire(CAROL, "gpu/qwen", Duration::from_secs(3600), false)
        .unwrap();
    c.acquire(CAROL, "port:8799", mins(1), false).unwrap();
    c.release(USER, "gpu/qwen").unwrap();
    e.ros.set(CAROL, "");
    let _ = c.sweep();
    e.clk.add(GONE_GRACE);
    let _ = c.sweep();
    e.clk.add(mins(1));
    c.acquire(ALICE, "port:1", Duration::ZERO, false).unwrap();
    let t3 = c.create_task(ALICE, new_task("thread")).unwrap();
    c.claim_task(ALICE, &t3.id).unwrap();
    c.set_steps(
        ALICE,
        &t3.id,
        vec![
            Step {
                text: "a".into(),
                ..Step::default()
            },
            Step {
                text: "b".into(),
                ..Step::default()
            },
        ],
    )
    .unwrap();
    c.set_step(
        ALICE,
        &t3.id,
        "s1",
        StepUpdate {
            done: Some(true),
            doing: None,
        },
    )
    .unwrap();
    for q in ["Ship?", "Which day?"] {
        c.raise(
            ALICE,
            &t3.id,
            NewBlocker {
                question: q.into(),
                ..NewBlocker::default()
            },
        )
        .unwrap();
    }
    c.add_comment(USER, &t3.id, "plain", false).unwrap();
    for mut m in [
        MessageEntry {
            msg_id: "msg_r1".into(),
            from: ALICE.into(),
            to: "dev-b/x".into(),
            text: "hi x".into(),
            ..MessageEntry::default()
        },
        MessageEntry {
            msg_id: "msg_r2".into(),
            from: BOB.into(),
            to: ALICE.into(),
            text: "held".into(),
            held: true,
            files: 3,
            ..MessageEntry::default()
        },
    ] {
        m.task_id = t3.id.clone();
        c.record_message(m).unwrap();
    }
    let at = hlc_at(plus(e.clk.now(), secs(1)));
    e.remote(
        "dev-b",
        1,
        at,
        "irori",
        kinds::TASK_COMMENT,
        &TaskCommentRow {
            task_id: t3.id.clone(),
            comment_id: "msg_r1".into(),
            msg_id: "msg_r1".into(),
            by: ALICE.into(),
            to: "dev-b/x".into(),
            text: "hi x".into(),
            gone: true,
            at: e.clk.now(),
            ..TaskCommentRow::default()
        },
    );
    e.remote(
        "dev-b",
        2,
        at + 1,
        "agent:dev-b/x",
        kinds::TASK_COMMENT,
        &TaskCommentRow {
            task_id: t3.id.clone(),
            comment_id: "c-b".into(),
            by: "dev-b/x".into(),
            text: "from b".into(),
            at: e.clk.now(),
            ..TaskCommentRow::default()
        },
    );
    e.clk.add(secs(1));
    c.answer(
        USER,
        &t3.id,
        "b1",
        BlockerAnswer {
            text: "yes".into(),
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    c.answer(
        USER,
        &t3.id,
        "b2",
        BlockerAnswer {
            text: "soon".into(),
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    e.clk.add(secs(1));
    e.remote(
        "dev-b",
        3,
        hlc_at(e.clk.now()),
        "agent:dev-b/x",
        kinds::TASK_COMMENT,
        &TaskCommentRow {
            task_id: t3.id.clone(),
            comment_id: "c-b2".into(),
            by: "dev-b/x".into(),
            text: "which day?".into(),
            blocker_id: "b2".into(),
            missing: vec!["the day".into()],
            at: e.clk.now(),
            ..TaskCommentRow::default()
        },
    );
    e.clk.add(secs(1));
    c.answer(
        ALICE,
        &t3.id,
        "b1",
        BlockerAnswer {
            text: "shipping now".into(),
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();

    let c2 = e.reopen();
    assert_eq!(
        c.list_tasks(&TaskFilter::default()),
        c2.list_tasks(&TaskFilter::default())
    );
    for tk in c.list_tasks(&TaskFilter::default()) {
        assert_eq!(c.comments(&tk.id), c2.comments(&tk.id));
        assert_eq!(c.activity(&tk.id), c2.activity(&tk.id));
    }
    assert_eq!(c.leases(), c2.leases());
    assert_eq!(c.leases().len(), 2, "{:?}", c.leases());
    let cs = c.comments(&t3.id).unwrap();
    assert_eq!(cs.len(), 7, "{cs:?}");
    assert!(
        cs[1].gone
            && cs[1].device == "dev-a"
            && cs[2].held
            && cs[2].text.is_empty()
            && cs[3].device == "dev-b"
    );
    assert!(cs[4].blocker_id == "b1" && cs[6].blocker_id == "b2" && cs[6].missing.len() == 1);
    let got = c.get_task(&t3.id).unwrap();
    let (b1, b2) = (&got.blockers[0], &got.blockers[1]);
    let r = b1.resolved.as_ref().unwrap();
    assert_eq!(
        (r.answer.as_str(), r.note.as_str()),
        ("yes", "shipping now")
    );
    assert_eq!(b2.status, BLOCKER_REPLIED);
    assert_eq!(b2.missing, ["the day"]);
    assert_eq!(got.state, BLOCKED);
}

#[test]
fn test_replay_pages_past_list_limit() {
    let e = Env::new();
    let n = store::MAX_LIST_LIMIT as usize * 2 + 7;
    for i in 0..n {
        e.c.create_task(ALICE, new_task("t")).unwrap();
        if i % 3 == 0 {
            e.st.append(
                "irori",
                kinds::AGENT_STATUS,
                &std::collections::BTreeMap::from([("i", i)]),
            )
            .unwrap();
        }
    }
    let c2 = e.reopen();
    assert_eq!(c2.list_tasks(&TaskFilter::default()).len(), n);
    assert_eq!(
        e.c.list_tasks(&TaskFilter::default()),
        c2.list_tasks(&TaskFilter::default())
    );
}

#[test]
fn test_replay_tolerates() {
    let e = Env::new();
    let rows: Vec<(&str, Vec<u8>)> = vec![
        (
            kinds::TASK_CREATED,
            format!(r#"{{"by":"{ALICE}","extra":1,"taskId":"x1","title":"old"}}"#).into_bytes(),
        ),
        (
            kinds::TASK_CLAIMED,
            format!(r#"{{"by":"{BOB}","taskId":"missing"}}"#).into_bytes(),
        ),
        (
            kinds::TASK_STATUS,
            br#"{"state":"bogus","taskId":"x1"}"#.to_vec(),
        ),
        (
            kinds::LEASE_RELEASED,
            format!(r#"{{"holder":"{BOB}","resource":"gpu/none"}}"#).into_bytes(),
        ),
        ("task.future", br#"{"a":1}"#.to_vec()),
        (kinds::TASK_CREATED, b"[1,2]".to_vec()),
    ];
    for (kind, raw) in rows {
        e.st.append("irori", kind, &gocompat::json::RawJson(raw))
            .unwrap();
    }
    let c2 = e.reopen();
    let got = c2.get_task("x1").unwrap();
    assert_eq!(got.state, SUBMITTED);
    assert!(!got.created_at.is_zero());
    let _ = Arc::new(());
}
