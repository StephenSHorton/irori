//! Port of I:internal/coord/board_test.go. One change: Go plants an overlapping lease straight into the fold for
//! TestBoardCardsAndLeases; here it arrives as another device's row.
#![allow(clippy::type_complexity)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use coord::rows::{BlockerClosedRow, LeaseAcquiredRow, TaskStateRow};
use coord::*;
use store::kinds;

const DAVE: &str = "dev-a/dave-session";

fn path_res(glob: &str) -> String {
    format!("path:{REPO_A}:{glob}")
}

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

fn shared(e: &Env, by: &str) -> Task {
    e.c.create_task(
        by,
        NewTask {
            title: "shared".into(),
            shared: true,
            ..NewTask::default()
        },
    )
    .unwrap()
}

fn bb(kind: &str, r: &str) -> Option<BlockedBy> {
    Some(BlockedBy {
        kind: kind.into(),
        reference: r.into(),
    })
}

#[test]
fn test_holders_claim_assign() {
    let cases: Vec<(&str, bool, Vec<(&str, &str, &str, Vec<&str>)>)> = vec![
        (
            "unshared: claim then assign replace",
            false,
            vec![
                (ALICE, "claim", "", vec![ALICE]),
                (ALICE, "assign", BOB, vec![BOB]),
                (USER, "assign", CAROL, vec![CAROL]),
                (CAROL, "claim", "", vec![CAROL]),
            ],
        ),
        (
            "shared: claim and assign append, no duplicates",
            true,
            vec![
                (ALICE, "claim", "", vec![ALICE]),
                (BOB, "claim", "", vec![ALICE, BOB]),
                (ALICE, "assign", CAROL, vec![ALICE, BOB, CAROL]),
                (BOB, "claim", "", vec![ALICE, BOB, CAROL]),
                (ALICE, "assign", BOB, vec![ALICE, BOB, CAROL]),
            ],
        ),
    ];
    for (name, sh, steps) in cases {
        let e = Env::new();
        let task =
            e.c.create_task(
                ALICE,
                NewTask {
                    title: "t".into(),
                    shared: sh,
                    ..NewTask::default()
                },
            )
            .unwrap();
        for (i, (by, op, who, want)) in steps.into_iter().enumerate() {
            let got = if op == "claim" {
                e.c.claim_task(by, &task.id).unwrap()
            } else {
                e.c.assign(by, &task.id, who).unwrap().0
            };
            assert_eq!(got.holders, want, "{name} step {i}");
            assert_eq!(got.holder, want[0], "{name} step {i}");
        }
    }
}

#[test]
fn test_shared_claim_not_held() {
    let e = Env::new();
    let task = shared(&e, ALICE);
    e.c.claim_task(ALICE, &task.id).unwrap();
    let got = e.c.claim_task(BOB, &task.id).unwrap();
    assert_eq!(got.state, WORKING);
    assert_eq!(got.holders.len(), 2);
    let t2 = e.c.create_task(ALICE, nt("u")).unwrap();
    e.c.claim_task(ALICE, &t2.id).unwrap();
    want_err(e.c.claim_task(BOB, &t2.id), ErrorKind::Held);
}

#[test]
fn test_holder_filter_and_permissions_use_holders() {
    let e = Env::new();
    let task = shared(&e, USER);
    e.c.claim_task(ALICE, &task.id).unwrap();
    e.c.claim_task(BOB, &task.id).unwrap();
    let got = e.c.list_tasks(&TaskFilter {
        holder: BOB.into(),
        ..TaskFilter::default()
    });
    assert_eq!(ids(&got), std::slice::from_ref(&task.id));
    e.c.set_task_state(BOB, &task.id, INPUT_REQUIRED, "?")
        .unwrap();
    want_err(
        e.c.set_task_state(CAROL, &task.id, WORKING, ""),
        ErrorKind::NotAllowed,
    );
    e.c.block(BOB, &task.id, "waiting", None).unwrap();
    want_err(e.c.block(CAROL, &task.id, "x", None), ErrorKind::NotAllowed);
}

#[test]
fn test_observe_tracks_every_holder() {
    // Go reads firstAbsent; here the effect: after the grace, a second
    // holder that left the roster is taken off the card.
    let e = Env::new();
    let task = shared(&e, ALICE);
    e.c.claim_task(ALICE, &task.id).unwrap();
    e.c.claim_task(BOB, &task.id).unwrap();
    e.ros.set(BOB, "");
    e.c.sweep().unwrap();
    e.clk.add(HOLDER_GRACE);
    e.c.sweep().unwrap();
    assert_eq!(e.c.get_task(&task.id).unwrap().holders, [ALICE]);
}

#[test]
fn test_create_task_board_fields() {
    let e = Env::new();
    e.c.acquire(CAROL, "port:5173", Duration::from_secs(3600), false)
        .unwrap();
    let other = e.c.create_task(ALICE, nt("other")).unwrap();
    let got =
        e.c.create_task(
            USER,
            NewTask {
                title: "port 5173 is taken".into(),
                shared: true,
                links: vec![
                    "https://linear.app/acme/issue/ABC-123/x".into(),
                    " ".into(),
                    "https://linear.app/acme/issue/ABC-123/x".into(),
                    "docs/a.md".into(),
                ],
                block_reason: " a dead dev server holds it ".into(),
                blocked_by: bb(BLOCKED_ON_LEASE, " port:5173 "),
                ..NewTask::default()
            },
        )
        .unwrap();
    assert!(got.shared && got.state == BLOCKED && got.reason == "a dead dev server holds it");
    assert_eq!(got.blocked_by, bb(BLOCKED_ON_LEASE, "port:5173"));
    assert_eq!(got.links.len(), 2);
    assert_eq!(got.links[0].label, "Linear ABC-123");
    assert_eq!(got.links[1].kind, LINK_PATH);
    let rows = e.rows("task.");
    assert_eq!(rows.len(), 3);
    assert_eq!(
        (rows[1].kind.as_str(), rows[2].kind.as_str()),
        (kinds::TASK_CREATED, kinds::TASK_STATUS)
    );
    let st: TaskStateRow = payload(&rows[2]);
    assert!(st.state == BLOCKED && !st.reason.is_empty() && st.blocked_by.is_some());

    let long = "x".repeat(MAX_REASON + 1);
    let too_many: Vec<String> = (0..=MAX_LINKS)
        .map(|i| format!("docs/{}.md", (b'a' + i as u8) as char))
        .collect();
    let cases: Vec<(&str, NewTask, ErrorKind)> = vec![
        (
            "blockedBy without reason",
            NewTask {
                title: "x".into(),
                blocked_by: bb(BLOCKED_ON_PERSON, ""),
                ..NewTask::default()
            },
            ErrorKind::Invalid,
        ),
        (
            "reason too long",
            NewTask {
                title: "x".into(),
                block_reason: long,
                ..NewTask::default()
            },
            ErrorKind::Invalid,
        ),
        (
            "bad link",
            NewTask {
                title: "x".into(),
                links: vec!["ftp://x/y".into()],
                ..NewTask::default()
            },
            ErrorKind::Invalid,
        ),
        (
            "too many links",
            NewTask {
                title: "x".into(),
                links: too_many,
                ..NewTask::default()
            },
            ErrorKind::Invalid,
        ),
        (
            "blocked on a missing task",
            NewTask {
                title: "x".into(),
                block_reason: "r".into(),
                blocked_by: bb(BLOCKED_ON_TASK, "nope"),
                ..NewTask::default()
            },
            ErrorKind::NotFound,
        ),
        (
            "bad lease ref",
            NewTask {
                title: "x".into(),
                block_reason: "r".into(),
                blocked_by: bb(BLOCKED_ON_LEASE, "Not A Resource"),
                ..NewTask::default()
            },
            ErrorKind::Invalid,
        ),
    ];
    for (name, n, want) in cases {
        let r = e.c.create_task(ALICE, n);
        assert!(r.as_ref().is_err_and(|e| e.is(want)), "{name}: {r:?}");
    }
    let b =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "y".into(),
                block_reason: "after other".into(),
                blocked_by: bb(BLOCKED_ON_TASK, &other.id),
                ..NewTask::default()
            },
        )
        .unwrap();
    assert_eq!(b.state, BLOCKED);
    let c =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "z".into(),
                block_reason: "just blocked".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    assert_eq!(c.state, BLOCKED);
    assert!(c.blocked_by.is_none());
}

#[test]
fn test_assign_permissions_and_notice() {
    let cases: Vec<(&str, &str, &str, &str, Option<ErrorKind>, Vec<&str>, usize)> = vec![
        ("creator", "", ALICE, BOB, None, vec![BOB], 1),
        (
            "stranger, no holder (decision 21)",
            "",
            CAROL,
            BOB,
            None,
            vec![BOB],
            1,
        ),
        ("stranger assigns itself", "", CAROL, CAROL, None, vec![], 1),
        ("holder hands off", BOB, BOB, CAROL, None, vec![CAROL], 1),
        ("person", BOB, USER, CAROL, None, vec![CAROL], 1),
        (
            "stranger while held",
            BOB,
            CAROL,
            DAVE,
            Some(ErrorKind::NotAllowed),
            vec![],
            0,
        ),
        (
            "to a person: no recipients",
            "",
            ALICE,
            USER,
            None,
            vec![],
            1,
        ),
        ("to the holder: no-op", BOB, ALICE, BOB, None, vec![], 0),
    ];
    for (name, holder, by, to, want_e, want_to, want_rows) in cases {
        let e = Env::new();
        let task =
            e.c.create_task(
                ALICE,
                NewTask {
                    title: "fix it".into(),
                    objective: "the objective".into(),
                    done_criteria: "tests pass".into(),
                    ..NewTask::default()
                },
            )
            .unwrap();
        if !holder.is_empty() {
            e.c.claim_task(holder, &task.id).unwrap();
        }
        let r = e.c.assign(by, &task.id, to);
        if let Some(k) = want_e {
            want_err(r, k);
            continue;
        }
        let (got, n) = r.unwrap();
        assert_eq!(e.rows(kinds::TASK_ASSIGNED).len(), want_rows, "{name}");
        assert_eq!(n.to, want_to, "{name}");
        if want_rows == 0 {
            assert!(n.kind.is_empty() && n.to.is_empty(), "{name}: {n:?}");
            continue;
        }
        assert!(
            got.holder == to
                && n.kind == NOTICE_ASSIGNED
                && n.reference == format!("task:{}", task.id)
                && n.by == by
                && !n.wake
                && n.title == "fix it",
            "{name}: {got:?} {n:?}"
        );
        for s in [
            "fix it".to_string(),
            format!("task:{}", task.id),
            "the objective".into(),
            "tests pass".into(),
            format!("show task:{}", task.id),
        ] {
            assert!(n.text.contains(&s), "{name}: {:?} lacks {s:?}", n.text);
        }
    }
}

#[test]
fn test_assign_notice_text() {
    // board_impl.go:139-150 verbatim.
    let e = Env::new();
    let task =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "fix \"it\"".into(),
                objective: "obj".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    let (_, n) = e.c.assign(ALICE, &task.id, BOB).unwrap();
    assert_eq!(
        n.text,
        format!(
            "{ALICE} assigned you a task: \"fix \\\"it\\\"\" (#1, task:{id}).\nObjective: obj\nClaim it to start.\nshow task:{id} for the card, its comments and links.",
            id = task.id
        )
    );
}

#[test]
fn test_assign_notice_cuts_long_fields() {
    let e = Env::new();
    let task =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "t".into(),
                objective: "o".repeat(2000),
                done_criteria: "d".repeat(700),
                ..NewTask::default()
            },
        )
        .unwrap();
    let (_, n) = e.c.assign(ALICE, &task.id, BOB).unwrap();
    assert!(n.text.contains(&format!("{}…", "o".repeat(599))));
    assert!(!n.text.contains(&"o".repeat(600)));
    assert!(n.text.contains(&format!("{}…", "d".repeat(599))));
}

#[test]
fn test_assign_task_keeps_v02_rule() {
    let e = Env::new();
    let task = e.c.create_task(ALICE, nt("t")).unwrap();
    want_err(
        e.c.assign_task(CAROL, &task.id, CAROL),
        ErrorKind::NotAllowed,
    );
    e.c.assign(CAROL, &task.id, CAROL).unwrap();
}

#[test]
fn test_join_leave_share() {
    let e = Env::new();
    let sh = shared(&e, ALICE);
    let un = e.c.create_task(ALICE, nt("u")).unwrap();
    let done =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "d".into(),
                shared: true,
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.set_task_state(ALICE, &done.id, COMPLETED, "").unwrap();
    type Step<'a> = (
        &'a str,
        Box<dyn Fn() -> Result<Task> + 'a>,
        Option<ErrorKind>,
        Vec<&'a str>,
        &'a str,
    );
    let c = &e.c;
    let (s, u, d) = (sh.id.clone(), un.id.clone(), done.id.clone());
    let steps: Vec<Step> = vec![
        (
            "join moves submitted to working",
            Box::new(|| c.join(BOB, &s, BOB)),
            None,
            vec![BOB],
            WORKING,
        ),
        (
            "second joins",
            Box::new(|| c.join(CAROL, &s, CAROL)),
            None,
            vec![BOB, CAROL],
            WORKING,
        ),
        (
            "join again: no-op",
            Box::new(|| c.join(CAROL, &s, CAROL)),
            None,
            vec![BOB, CAROL],
            WORKING,
        ),
        (
            "agent adds another: refused",
            Box::new(|| c.join(BOB, &s, DAVE)),
            Some(ErrorKind::NotAllowed),
            vec![],
            "",
        ),
        (
            "person adds someone",
            Box::new(|| c.join(USER, &s, DAVE)),
            None,
            vec![BOB, CAROL, DAVE],
            WORKING,
        ),
        (
            "join unshared: refused",
            Box::new(|| c.join(BOB, &u, BOB)),
            Some(ErrorKind::NotAllowed),
            vec![],
            "",
        ),
        (
            "join done: refused",
            Box::new(|| c.join(BOB, &d, BOB)),
            Some(ErrorKind::BadState),
            vec![],
            "",
        ),
        (
            "empty who",
            Box::new(|| c.join(BOB, &s, " ")),
            Some(ErrorKind::Invalid),
            vec![],
            "",
        ),
        (
            "unshare with three holders",
            Box::new(|| c.set_shared(ALICE, &s, false)),
            Some(ErrorKind::Invalid),
            vec![],
            "",
        ),
        (
            "leave someone else: refused",
            Box::new(|| c.leave(BOB, &s, CAROL)),
            Some(ErrorKind::NotAllowed),
            vec![],
            "",
        ),
        (
            "leave",
            Box::new(|| c.leave(BOB, &s, BOB)),
            None,
            vec![CAROL, DAVE],
            WORKING,
        ),
        (
            "leave not holding",
            Box::new(|| c.leave(BOB, &s, BOB)),
            Some(ErrorKind::NotFound),
            vec![],
            "",
        ),
        (
            "person removes one",
            Box::new(|| c.leave(USER, &s, DAVE)),
            None,
            vec![CAROL],
            WORKING,
        ),
        (
            "stranger can't share",
            Box::new(|| c.set_shared(DAVE, &s, false)),
            Some(ErrorKind::NotAllowed),
            vec![],
            "",
        ),
        (
            "holder unshares",
            Box::new(|| c.set_shared(CAROL, &s, false)),
            None,
            vec![CAROL],
            WORKING,
        ),
        (
            "last leaves: state kept, no holder",
            Box::new(|| c.leave(CAROL, &s, CAROL)),
            None,
            vec![],
            WORKING,
        ),
        (
            "creator shares again",
            Box::new(|| c.set_shared(ALICE, &s, true)),
            None,
            vec![],
            WORKING,
        ),
        (
            "share again: no-op",
            Box::new(|| c.set_shared(ALICE, &s, true)),
            None,
            vec![],
            WORKING,
        ),
    ];
    for (name, f, want_e, holders, state) in steps {
        e.clk.add(secs(1));
        let r = f();
        if let Some(k) = want_e {
            assert!(r.as_ref().is_err_and(|x| x.is(k)), "{name}: {r:?}");
            continue;
        }
        let got = r.unwrap_or_else(|x| panic!("{name}: {x}"));
        assert_eq!(
            (got.holders.clone(), got.state.as_str()),
            (
                holders.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                state
            ),
            "{name}"
        );
        assert_eq!(got.holder, holders.first().copied().unwrap_or(""), "{name}");
    }
    assert_eq!(e.rows(kinds::TASK_SHARED).len(), 2);
    assert_eq!(e.rows(kinds::TASK_JOINED).len(), 3);
}

#[test]
fn test_block_validation() {
    let e = Env::new();
    let task = e.c.create_task(ALICE, nt("t")).unwrap();
    let other = e.c.create_task(ALICE, nt("o")).unwrap();
    e.c.acquire(DAVE, &path_res("src/**"), Duration::from_secs(3600), false)
        .unwrap();
    let cases: Vec<(
        &str,
        &str,
        String,
        Option<BlockedBy>,
        Option<ErrorKind>,
        Option<BlockedBy>,
    )> = vec![
        (
            "no reason",
            ALICE,
            "  ".into(),
            None,
            Some(ErrorKind::Invalid),
            None,
        ),
        (
            "reason too long",
            ALICE,
            "é".repeat(MAX_REASON + 1),
            None,
            Some(ErrorKind::Invalid),
            None,
        ),
        (
            "reason at the cap",
            ALICE,
            "é".repeat(MAX_REASON),
            None,
            None,
            None,
        ),
        (
            "stranger",
            CAROL,
            "r".into(),
            None,
            Some(ErrorKind::NotAllowed),
            None,
        ),
        (
            "lease normalized",
            ALICE,
            "r".into(),
            bb(BLOCKED_ON_LEASE, &format!("path:{REPO_A}:./src/")),
            None,
            bb(BLOCKED_ON_LEASE, &path_res("src/**")),
        ),
        (
            "lease bad",
            ALICE,
            "r".into(),
            bb(BLOCKED_ON_LEASE, "path:x"),
            Some(ErrorKind::Invalid),
            None,
        ),
        (
            "lease empty",
            ALICE,
            "r".into(),
            bb(BLOCKED_ON_LEASE, ""),
            Some(ErrorKind::Invalid),
            None,
        ),
        (
            "lease nobody holds",
            ALICE,
            "r".into(),
            bb(BLOCKED_ON_LEASE, "port:9"),
            Some(ErrorKind::Invalid),
            None,
        ),
        (
            "task",
            ALICE,
            "r".into(),
            bb(BLOCKED_ON_TASK, &other.id),
            None,
            bb(BLOCKED_ON_TASK, &other.id),
        ),
        (
            "task self",
            ALICE,
            "r".into(),
            bb(BLOCKED_ON_TASK, &task.id),
            Some(ErrorKind::Invalid),
            None,
        ),
        (
            "task missing",
            ALICE,
            "r".into(),
            bb(BLOCKED_ON_TASK, "nope"),
            Some(ErrorKind::NotFound),
            None,
        ),
        (
            "person anyone",
            USER,
            "r".into(),
            bb(BLOCKED_ON_PERSON, ""),
            None,
            bb(BLOCKED_ON_PERSON, ""),
        ),
        (
            "person key",
            ALICE,
            "r".into(),
            bb(BLOCKED_ON_PERSON, USER),
            None,
            bb(BLOCKED_ON_PERSON, USER),
        ),
        (
            "person not a user key",
            ALICE,
            "r".into(),
            bb(BLOCKED_ON_PERSON, BOB),
            Some(ErrorKind::Invalid),
            None,
        ),
        (
            "agent",
            ALICE,
            "r".into(),
            bb(BLOCKED_ON_AGENT, BOB),
            None,
            bb(BLOCKED_ON_AGENT, BOB),
        ),
        (
            "agent empty",
            ALICE,
            "r".into(),
            bb(BLOCKED_ON_AGENT, ""),
            Some(ErrorKind::Invalid),
            None,
        ),
        (
            "external free text",
            ALICE,
            "r".into(),
            bb(BLOCKED_ON_EXTERNAL, "CI on #57"),
            None,
            bb(BLOCKED_ON_EXTERNAL, "CI on #57"),
        ),
        (
            "ref too long",
            ALICE,
            "r".into(),
            bb(BLOCKED_ON_EXTERNAL, &"x".repeat(MAX_REF + 1)),
            Some(ErrorKind::Invalid),
            None,
        ),
        (
            "unknown kind",
            ALICE,
            "r".into(),
            bb("weather", ""),
            Some(ErrorKind::Invalid),
            None,
        ),
    ];
    for (name, by, reason, b, want_e, want_bb) in cases {
        let r = e.c.block(by, &task.id, &reason, b);
        if let Some(k) = want_e {
            assert!(r.as_ref().is_err_and(|x| x.is(k)), "{name}: {r:?}");
            continue;
        }
        let got = r.unwrap_or_else(|x| panic!("{name}: {x}"));
        assert_eq!(
            (got.state.as_str(), got.reason.as_str()),
            (BLOCKED, reason.trim()),
            "{name}"
        );
        assert_eq!(got.blocked_by, want_bb, "{name}");
    }
}

#[test]
fn test_block_replace_noop_and_leave() {
    let e = Env::new();
    let task = e.c.create_task(ALICE, nt("t")).unwrap();
    e.c.set_task_state(ALICE, &task.id, WORKING, "keep me")
        .unwrap();
    want_err(
        e.c.set_task_state(ALICE, &task.id, BLOCKED, ""),
        ErrorKind::BadState,
    );
    let got = e.c.block(ALICE, &task.id, "one", None).unwrap();
    assert_eq!(got.note, "keep me");
    e.c.block(ALICE, &task.id, "one", None).unwrap();
    let got =
        e.c.block(ALICE, &task.id, "two", bb(BLOCKED_ON_PERSON, ""))
            .unwrap();
    assert!(got.reason == "two" && got.blocked_by.is_some());
    assert_eq!(e.rows(kinds::TASK_STATUS).len(), 3);
    let got =
        e.c.set_task_state(ALICE, &task.id, WORKING, "answered")
            .unwrap();
    assert!(got.state == WORKING && got.reason.is_empty() && got.blocked_by.is_none());
    e.c.set_task_state(ALICE, &task.id, COMPLETED, "").unwrap();
    want_err(
        e.c.block(ALICE, &task.id, "late", None),
        ErrorKind::BadState,
    );
}

#[test]
fn test_unblock() {
    let cases: Vec<(
        &str,
        bool,
        Vec<&str>,
        &str,
        String,
        &str,
        Vec<&str>,
        Option<ErrorKind>,
    )> = vec![
        (
            "no holders: submitted, creator hears",
            false,
            vec![],
            USER,
            "freed it".into(),
            SUBMITTED,
            vec![ALICE],
            None,
        ),
        (
            "no holders, creator unblocks: nobody",
            false,
            vec![],
            ALICE,
            "".into(),
            SUBMITTED,
            vec![],
            None,
        ),
        (
            "holder: working, holder hears",
            false,
            vec![BOB],
            CAROL,
            "".into(),
            WORKING,
            vec![BOB],
            None,
        ),
        (
            "anyone, even a stranger",
            false,
            vec![BOB],
            DAVE,
            "".into(),
            WORKING,
            vec![BOB],
            None,
        ),
        (
            "shared: other holders hear",
            true,
            vec![BOB, CAROL, USER],
            BOB,
            "".into(),
            WORKING,
            vec![CAROL],
            None,
        ),
        (
            "note too long",
            false,
            vec![],
            USER,
            "n".repeat(MAX_REASON + 1),
            "",
            vec![],
            Some(ErrorKind::Invalid),
        ),
    ];
    for (name, sh, holders, by, note, want_state, want_to, want_e) in cases {
        let e = Env::new();
        let task =
            e.c.create_task(
                ALICE,
                NewTask {
                    title: "t".into(),
                    shared: sh,
                    ..NewTask::default()
                },
            )
            .unwrap();
        for h in &holders {
            e.c.claim_task(h, &task.id).unwrap();
        }
        e.c.block(
            USER,
            &task.id,
            "waiting on the db",
            bb(BLOCKED_ON_EXTERNAL, "db"),
        )
        .unwrap();
        let r = e.c.unblock(by, &task.id, &note);
        if let Some(k) = want_e {
            want_err(r, k);
            continue;
        }
        let (got, n) = r.unwrap();
        assert!(
            got.state == want_state && got.reason.is_empty() && got.blocked_by.is_none(),
            "{name}: {got:?}"
        );
        assert_eq!(n.to, want_to, "{name}");
        assert!(
            n.kind == NOTICE_UNBLOCKED && n.by == by && n.reference == format!("task:{}", task.id),
            "{name}"
        );
        assert!(
            n.text.contains("waiting on the db") && n.text.contains(want_state),
            "{name}: {}",
            n.text
        );
        let rows = e.rows(kinds::BLOCKER_CLOSED);
        assert_eq!(rows.len(), 1);
        let p: BlockerClosedRow = payload(&rows[0]);
        assert_eq!(
            (p.restored.as_str(), p.by.as_str(), p.note.as_str()),
            (want_state, by, note.as_str()),
            "{name}"
        );
        want_err(e.c.unblock(by, &task.id, ""), ErrorKind::BadState);
    }
}

#[test]
fn test_unblock_notice_text() {
    // board_impl.go:318-328 verbatim.
    let e = Env::new();
    let task = e.c.create_task(ALICE, nt("t")).unwrap();
    e.c.claim_task(BOB, &task.id).unwrap();
    e.c.block(BOB, &task.id, "the db", None).unwrap();
    let (_, n) = e.c.unblock(USER, &task.id, "it is back").unwrap();
    assert_eq!(
        n.text,
        format!(
            "{USER} unblocked \"t\" (#1, task:{id}): it is back. It was blocked on: the db. It is working again.\nshow task:{id} for the card, its comments and links.",
            id = task.id
        )
    );
}

#[test]
fn test_comments() {
    let e = Env::new();
    let task = e.c.create_task(ALICE, nt("t")).unwrap();
    e.c.claim_task(BOB, &task.id).unwrap();
    for (name, text) in [
        ("empty", " \n ".to_string()),
        ("too long", "c".repeat(MAX_COMMENT + 1)),
    ] {
        let r = e.c.add_comment(USER, &task.id, &text, false);
        assert!(
            r.as_ref().is_err_and(|x| x.is(ErrorKind::Invalid)),
            "{name}"
        );
    }
    want_err(
        e.c.add_comment(USER, "nope", "hi", false),
        ErrorKind::NotFound,
    );
    let (c1, n1) =
        e.c.add_comment(USER, &task.id, " use postgres ", false)
            .unwrap();
    assert!(c1.text == "use postgres" && c1.by == USER && c1.task_id == task.id && !c1.wake);
    assert!(
        n1.kind == NOTICE_COMMENT
            && n1.reference == format!("comment:{}", c1.id)
            && n1.to == [BOB]
            && !n1.wake
    );
    assert!(
        n1.text.contains("use postgres") && n1.text.contains(&format!("show task:{}", task.id))
    );
    assert_eq!(
        n1.text,
        format!(
            "{USER} commented on \"t\" (#1, task:{id}):\nuse postgres\nshow task:{id} for the card, its comments and links.",
            id = task.id
        )
    );
    e.clk.add(secs(1));
    let (_, n2) = e.c.add_comment(BOB, &task.id, "ok", true).unwrap();
    assert!(n2.to.is_empty() && n2.wake);
    e.clk.add(secs(1));
    let (_, n3) =
        e.c.add_comment(ALICE, &task.id, &"é".repeat(MAX_COMMENT), true)
            .unwrap();
    assert!(n3.wake && n3.to == [BOB]);
    e.c.set_task_state(BOB, &task.id, COMPLETED, "").unwrap();
    e.clk.add(secs(1));
    let (c4, _) = e.c.add_comment(USER, &task.id, "thanks", false).unwrap();
    let cs = e.c.comments(&task.id).unwrap();
    assert!(cs.len() == 4 && cs[0].id == c1.id && cs[3].id == c4.id);
    let got = e.c.get_task(&task.id).unwrap();
    assert!(got.comments == 4 && got.updated_at.equal(&c4.at));
    assert_eq!(e.c.get_comment(&c1.id), Some(c1));
    assert!(e.c.get_comment("nope").is_none());
    assert!(e.c.comments("nope").is_none());
    let empty = e.c.create_task(ALICE, nt("empty")).unwrap();
    assert_eq!(e.c.comments(&empty.id), Some(vec![]));
}

fn claimed(e: &Env) -> Task {
    let t = e.c.create_task(ALICE, nt("ship it")).unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap()
}

fn msg(task: &str, id: &str, from: &str, to: &str, text: &str) -> MessageEntry {
    MessageEntry {
        task_id: task.into(),
        msg_id: id.into(),
        from: from.into(),
        to: to.into(),
        text: text.into(),
        ..MessageEntry::default()
    }
}

#[test]
fn test_record_message() {
    let e = Env::new();
    let task = claimed(&e);
    let acts = e.c.activity(&task.id).unwrap().len();
    let rows = || e.rows(kinds::TASK_COMMENT).len();
    let mut m = msg(&task.id, "msg_1", ALICE, BOB, " can you look? ");
    m.files = 2;
    let cm = e.c.record_message(m.clone()).unwrap();
    assert!(
        cm.id == "msg_1"
            && cm.msg_id == "msg_1"
            && cm.by == ALICE
            && cm.to == BOB
            && cm.text == "can you look?"
    );
    assert!(
        cm.device == "dev-a"
            && cm.hlc != 0
            && cm.seq != 0
            && cm.files == 2
            && cm.attachments.is_empty()
            && !cm.gone
            && !cm.held
            && !cm.wake
    );
    assert_eq!(
        e.rows(kinds::TASK_COMMENT)[0].actor,
        format!("agent:{ALICE}")
    );
    assert_eq!(e.c.activity(&task.id).unwrap().len(), acts);
    let again = e.c.record_message(m.clone()).unwrap();
    assert_eq!(again, cm);
    assert_eq!(rows(), 1);
    let mut g = m.clone();
    g.gone = true;
    g.writer = "irori".into();
    g.text = "other text".into();
    for _ in 0..2 {
        let got = e.c.record_message(g.clone()).unwrap();
        assert!(
            got.gone && got.text == "can you look?" && got.hlc == cm.hlc && rows() == 2,
            "{got:?}"
        );
    }
    assert_eq!(e.rows(kinds::TASK_COMMENT)[1].actor, "irori");
    let cs = e.c.comments(&task.id).unwrap();
    assert!(cs.len() == 1 && cs[0].gone);
    let long =
        e.c.record_message(msg(
            &task.id,
            "msg_2",
            ALICE,
            BOB,
            &"x".repeat(MAX_COMMENT + 50),
        ))
        .unwrap();
    assert_eq!(long.text.chars().count(), MAX_COMMENT);
    assert!(long.text.ends_with("… (full: msg:msg_2)"));
    let mut h = msg(&task.id, "msg_3", ALICE, BOB, "secret");
    h.held = true;
    let held = e.c.record_message(h).unwrap();
    assert!(held.text.is_empty() && held.held);
    let n = rows();
    let mut f = msg(&task.id, "msg_3", ALICE, BOB, "secret");
    f.fill = true;
    f.writer = "irori".into();
    for _ in 0..2 {
        let got = e.c.record_message(f.clone()).unwrap();
        assert!(
            got.text == "secret" && !got.held && got.hlc == held.hlc && rows() == n + 1,
            "{got:?}"
        );
    }
    assert_eq!(e.rows(kinds::TASK_COMMENT)[n].actor, "irori");
    let mut f1 = msg(&task.id, "msg_1", ALICE, BOB, "x");
    f1.fill = true;
    f1.writer = "irori".into();
    e.c.record_message(f1).unwrap();
    assert_eq!(rows(), n + 1);
    e.c.set_task_state(ALICE, &task.id, COMPLETED, "").unwrap();
    e.c.record_message(msg(&task.id, "msg_4", BOB, ALICE, "late"))
        .unwrap();
    assert_eq!(e.c.get_task(&task.id).unwrap().comments, 4);
    want_err(
        e.c.record_message(msg("nope", "msg_5", ALICE, BOB, "x")),
        ErrorKind::NotFound,
    );
    want_err(
        e.c.record_message(msg(&task.id, "01ABC", ALICE, BOB, "x")),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.record_message(msg(&task.id, "msg_6", "dev-b/zed", ALICE, "x")),
        ErrorKind::NotAllowed,
    );
    let mut g6 = msg(&task.id, "msg_6", "dev-b/zed", ALICE, "x");
    g6.gone = true;
    g6.writer = "irori".into();
    e.c.record_message(g6).unwrap();
    let mut f7 = msg(&task.id, "msg_7", "dev-b/zed", ALICE, "x");
    f7.fill = true;
    f7.writer = "irori".into();
    e.c.record_message(f7).unwrap();
    let mut f8 = msg(&task.id, "msg_8", "dev-b/zed", "dev-c/yan", "x");
    f8.fill = true;
    f8.writer = "irori".into();
    want_err(e.c.record_message(f8), ErrorKind::NotAllowed);
}

#[test]
fn test_comment_no_holders_tells_creator() {
    for (creator, by, want) in [
        (ALICE, USER, vec![ALICE]),
        (ALICE, ALICE, vec![]),
        (USER, BOB, vec![]),
    ] {
        let e = Env::new();
        let task = e.c.create_task(creator, nt("t")).unwrap();
        let (_, n) = e.c.add_comment(by, &task.id, "x", false).unwrap();
        assert_eq!(n.to, want, "creator {creator} by {by}");
    }
}

#[test]
fn test_links() {
    let e = Env::new();
    let task = e.c.create_task(ALICE, nt("t")).unwrap();
    let got =
        e.c.add_link(CAROL, &task.id, "https://github.com/a/b/pull/57")
            .unwrap();
    assert!(got.links.len() == 1 && got.links[0].label == "GitHub #57");
    e.c.add_link(CAROL, &task.id, "https://github.com/a/b/pull/57")
        .unwrap();
    assert_eq!(e.rows(kinds::TASK_LINKED).len(), 1);
    want_err(
        e.c.add_link(CAROL, &task.id, "javascript:alert(1)://x"),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.add_link(CAROL, "nope", "docs/a.md"),
        ErrorKind::NotFound,
    );
    for i in 1..MAX_LINKS {
        e.c.add_link(
            CAROL,
            &task.id,
            &format!("docs/{}.md", (b'a' + i as u8) as char),
        )
        .unwrap();
    }
    want_err(
        e.c.add_link(CAROL, &task.id, "docs/full.md"),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.remove_link(CAROL, &task.id, "docs/b.md"),
        ErrorKind::NotAllowed,
    );
    want_err(
        e.c.remove_link(ALICE, &task.id, "docs/zz.md"),
        ErrorKind::NotFound,
    );
    let got = e.c.remove_link(ALICE, &task.id, r"docs\b.md").unwrap();
    assert_eq!(got.links.len(), MAX_LINKS - 1);
    assert!(!got.links.iter().any(|l| l.reference == "docs/b.md"));
    e.c.set_task_state(ALICE, &task.id, COMPLETED, "").unwrap();
    let got =
        e.c.add_link(USER, &task.id, "0123456789abcdef0123")
            .unwrap();
    assert_eq!(got.links.last().unwrap().label, "commit 0123456");
}

#[test]
fn test_refusals() {
    let e = Env::new();
    let hour = Duration::from_secs(3600);
    e.c.acquire(ALICE, &path_res("src/**"), hour, false)
        .unwrap();
    e.clk.add(secs(1));
    want_err(
        e.c.acquire(BOB, &path_res("src/a.go"), Duration::ZERO, false),
        ErrorKind::Held,
    );
    let got = e.c.refusals();
    let want = Refusal {
        resource: path_res("src/a.go"),
        by: BOB.into(),
        held_by: ALICE.into(),
        held: path_res("src/**"),
        until: plus(fixed_now(), hour),
        at: plus(fixed_now(), secs(1)),
        conflict: false,
    };
    assert_eq!(got.len(), 1);
    assert!(
        got[0].resource == want.resource
            && got[0].by == want.by
            && got[0].held_by == want.held_by
            && got[0].held == want.held
    );
    assert!(got[0].until.equal(&want.until) && got[0].at.equal(&want.at));
    e.c.acquire(ALICE, &path_res("src/**"), 2 * hour, false)
        .unwrap();
    let _ =
        e.c.acquire(BOB, &path_res("src/a.go"), Duration::ZERO, false);
    assert_eq!(e.rows(kinds::LEASE_REFUSED).len(), 1);
    e.clk.add(secs(1));
    let _ =
        e.c.acquire(CAROL, &path_res("src/a.go"), Duration::ZERO, false);
    e.clk.add(secs(1));
    let _ =
        e.c.acquire(BOB, &path_res("src/b.go"), Duration::ZERO, false);
    let got = e.c.refusals();
    assert!(
        got.len() == 3 && got[0].resource == path_res("src/b.go") && got[1].by == CAROL,
        "{got:?}"
    );
    e.c.release(ALICE, &path_res("src/**")).unwrap();
    assert!(e.c.refusals().is_empty());
    e.clk.add(secs(1));
    e.c.acquire(ALICE, &path_res("src/**"), hour, false)
        .unwrap();
    assert!(e.c.refusals().is_empty());
    let _ =
        e.c.acquire(BOB, &path_res("src/a.go"), Duration::ZERO, false);
    assert_eq!(e.rows(kinds::LEASE_REFUSED).len(), 4);
    e.c.acquire(BOB, &path_res("src/a.go"), Duration::ZERO, true)
        .unwrap();
    assert!(e.c.refusals().is_empty());
    e.c.acquire(CAROL, "gpu/qwen", mins(1), false).unwrap();
    let _ = e.c.acquire(DAVE, "gpu/qwen", Duration::ZERO, false);
    assert_eq!(e.c.refusals().len(), 1);
    e.clk.add(mins(2));
    assert!(e.c.refusals().is_empty());
}

#[test]
fn test_refusal_one_row_per_holder() {
    let e = Env::new();
    e.c.acquire(ALICE, &path_res("src/a.go"), Duration::ZERO, false)
        .unwrap();
    e.c.acquire(ALICE, &path_res("src/b.go"), Duration::ZERO, false)
        .unwrap();
    e.c.acquire(CAROL, &path_res("src/c.go"), Duration::ZERO, false)
        .unwrap();
    let err =
        e.c.acquire(BOB, &path_res("src/**"), Duration::ZERO, false)
            .unwrap_err();
    assert!(err.is(ErrorKind::Held));
    assert_eq!(
        err.to_string(),
        format!(
            "coord: held by someone else: {} is leased by {ALICE} until 2026-10-26T12:30:00Z (pass steal to take it)",
            path_res("src/a.go")
        )
    );
    let rows = e.rows(kinds::LEASE_REFUSED);
    assert_eq!(rows.len(), 2);
    let r: Refusal = payload(&rows[0]);
    assert_eq!(
        (r.held_by.as_str(), r.held.as_str()),
        (ALICE, path_res("src/a.go").as_str())
    );
}

#[test]
fn test_board_groups() {
    let owner: KeyFn = Arc::new(|k: &str| k == USER || k == "user:dev-b");
    let e = Env::with("dev-a", Some(owner));
    let mk = |title: &str, mut n: NewTask| {
        e.clk.add(secs(1));
        n.title = title.into();
        e.c.create_task(ALICE, n).unwrap()
    };
    let open = mk("open", NewTask::default());
    let working = mk("working", NewTask::default());
    e.c.claim_task(BOB, &working.id).unwrap();
    let assigned = mk("assigned to bob", NewTask::default());
    e.c.assign_task(ALICE, &assigned.id, BOB).unwrap();
    let mine = mk("assigned to user", NewTask::default());
    e.c.assign_task(ALICE, &mine.id, USER).unwrap();
    let asks = mk("asks", NewTask::default());
    e.c.claim_task(CAROL, &asks.id).unwrap();
    e.c.set_task_state(CAROL, &asks.id, INPUT_REQUIRED, "?")
        .unwrap();
    let on_anyone = mk(
        "on anyone",
        NewTask {
            block_reason: "r".into(),
            blocked_by: bb(BLOCKED_ON_PERSON, ""),
            ..NewTask::default()
        },
    );
    let on_other = mk(
        "on the person at another device",
        NewTask {
            block_reason: "r".into(),
            blocked_by: bb(BLOCKED_ON_PERSON, "user:dev-b"),
            ..NewTask::default()
        },
    );
    want_err(
        e.c.create_task(
            ALICE,
            NewTask {
                title: "made up".into(),
                block_reason: "r".into(),
                blocked_by: bb(BLOCKED_ON_PERSON, "user:dev-z"),
                ..NewTask::default()
            },
        ),
        ErrorKind::Invalid,
    );
    let on_bob = mk(
        "on bob",
        NewTask {
            block_reason: "r".into(),
            blocked_by: bb(BLOCKED_ON_AGENT, BOB),
            ..NewTask::default()
        },
    );
    let ext = mk(
        "external",
        NewTask {
            block_reason: "ci".into(),
            ..NewTask::default()
        },
    );
    let mut dones = Vec::new();
    for s in [COMPLETED, FAILED, CANCELED] {
        let d = mk("done", NewTask::default());
        e.c.set_task_state(ALICE, &d.id, s, "").unwrap();
        dones.push(d);
    }
    let cids = |cs: &[Card]| cs.iter().map(|c| c.task.id.clone()).collect::<Vec<_>>();
    let v = |x: &[&Task]| x.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
    let cases = [
        (
            "the person",
            BoardFilter {
                viewer: USER.into(),
                ..BoardFilter::default()
            },
            v(&[&on_other, &on_anyone, &asks, &mine]),
            v(&[&ext, &on_bob]),
            v(&[&assigned, &working]),
            v(&[&open]),
            v(&[&dones[2], &dones[1], &dones[0]]),
        ),
        (
            "whoever is at the app",
            BoardFilter {
                done_limit: 2,
                ..BoardFilter::default()
            },
            v(&[&on_other, &on_anyone, &asks]),
            v(&[&ext, &on_bob]),
            v(&[&mine, &assigned, &working]),
            v(&[&open]),
            v(&[&dones[2], &dones[1]]),
        ),
        (
            "agent bob",
            BoardFilter {
                viewer: BOB.into(),
                done_limit: 1,
                ..BoardFilter::default()
            },
            v(&[&on_bob, &assigned]),
            v(&[&ext, &on_other, &on_anyone]),
            v(&[&asks, &mine, &working]),
            v(&[&open]),
            v(&[&dones[2]]),
        ),
    ];
    for (name, f, needs, blocked, working_g, open_g, done) in cases {
        let b = e.c.board(&f);
        assert_eq!(cids(&b.needs_you), needs, "{name} needsYou");
        assert_eq!(cids(&b.blocked), blocked, "{name} blocked");
        assert_eq!(cids(&b.working), working_g, "{name} working");
        assert_eq!(cids(&b.open), open_g, "{name} open");
        assert_eq!(cids(&b.done), done, "{name} done");
        assert_eq!(b.done_total, 3, "{name}");
    }
    let b = e.c.board(&BoardFilter {
        repo_key: REPO_A.into(),
        ..BoardFilter::default()
    });
    assert_eq!(
        b.open.len() + b.working.len() + b.done.len() + b.needs_you.len() + b.blocked.len(),
        0
    );
    // Empty groups marshal as [] (Go makes them non-nil).
    assert!(
        String::from_utf8(gocompat::json::marshal(&b).unwrap())
            .unwrap()
            .contains("\"leases\":[]")
    );
}

#[test]
fn test_board_done_cap() {
    let e = Env::new();
    for _ in 0..DEFAULT_DONE_CARD + 5 {
        e.clk.add(secs(1));
        let d = e.c.create_task(ALICE, nt("d")).unwrap();
        e.c.set_task_state(ALICE, &d.id, COMPLETED, "").unwrap();
    }
    for (limit, want) in [
        (0, DEFAULT_DONE_CARD),
        (-1, DEFAULT_DONE_CARD),
        (3, 3),
        (MAX_DONE_CARD + 1, DEFAULT_DONE_CARD + 5),
    ] {
        let v = e.c.board(&BoardFilter {
            done_limit: limit,
            ..BoardFilter::default()
        });
        assert_eq!(
            (v.done.len() as i64, v.done_total),
            (want, DEFAULT_DONE_CARD + 5),
            "limit {limit}"
        );
    }
}

#[test]
fn test_board_cards_and_leases() {
    let e = Env::new();
    let hour = Duration::from_secs(3600);
    let sh =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "sh".into(),
                shared: true,
                repo_key: REPO_A.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(ALICE, &sh.id).unwrap();
    e.c.claim_task(BOB, &sh.id).unwrap();
    e.c.acquire(ALICE, &path_res("src/**"), hour, false)
        .unwrap();
    e.c.acquire(BOB, &path_res("src/a.go"), 2 * hour, true)
        .unwrap();
    e.c.acquire(ALICE, &path_res("docs/**"), hour, false)
        .unwrap();
    // An overlapping lease of another holder, from another device.
    e.c.join(USER, &sh.id, "dev-b/zed").unwrap();
    e.remote(
        "dev-b",
        1,
        hlc_at(e.clk.now()) + 1,
        "agent:dev-b/zed",
        kinds::LEASE_ACQUIRED,
        &LeaseAcquiredRow {
            lease: Lease {
                resource: path_res("docs/x.md"),
                holder: "dev-b/zed".into(),
                until: plus(fixed_now(), hour),
                acquired_at: fixed_now(),
                ..Lease::default()
            },
            stolen: vec![],
        },
    );
    e.clk.add(secs(1));
    let blocked =
        e.c.create_task(
            CAROL,
            NewTask {
                title: "needs src".into(),
                repo_key: REPO_A.into(),
                block_reason: "bob has it".into(),
                blocked_by: bb(BLOCKED_ON_LEASE, &path_res("src/a.go")),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.acquire(DAVE, "gpu/qwen", hour, false).unwrap();
    let gone =
        e.c.create_task(
            CAROL,
            NewTask {
                title: "needs gpu".into(),
                block_reason: "dave has it".into(),
                blocked_by: bb(BLOCKED_ON_LEASE, "gpu/qwen"),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.release(DAVE, "gpu/qwen").unwrap();
    e.c.acquire(DAVE, "port:8080", mins(30), false).unwrap();
    let _ = e.c.acquire(CAROL, "port:8080", Duration::ZERO, false);
    e.clk.add(secs(1));
    e.c.add_comment(USER, &sh.id, "first", false).unwrap();
    e.clk.add(secs(1));
    let (c2, _) = e.c.add_comment(USER, &sh.id, "second", false).unwrap();
    let v = e.c.board(&BoardFilter {
        viewer: USER.into(),
        ..BoardFilter::default()
    });
    let all: Vec<&Card> = v
        .working
        .iter()
        .chain(&v.blocked)
        .chain(&v.needs_you)
        .chain(&v.open)
        .collect();
    let find = |id: &str| *all.iter().find(|c| c.task.id == id).expect("card");
    let (sh_card, blocked_card, gone_card) = (find(&sh.id), find(&blocked.id), find(&gone.id));
    assert_eq!(
        sh_card.last_comment.as_ref().map(|c| c.id.clone()),
        Some(c2.id.clone())
    );
    assert_eq!(
        sh_card.overlaps,
        [format!(
            "{} ~ {}",
            path_res("docs/**"),
            path_res("docs/x.md")
        )]
    );
    assert!(!blocked_card.resolved && gone_card.resolved);
    assert_eq!(v.leases.len(), 2, "{:?}", v.leases);
    assert_eq!(v.leases[0].lease.resource, "port:8080");
    assert_eq!(v.leases[0].refused.len(), 1);
    assert_eq!(v.leases[0].refused[0].by, CAROL);
    assert!(v.leases[0].blocks.is_empty());
    assert_eq!(v.leases[1].lease.resource, path_res("src/a.go"));
    assert_eq!(v.leases[1].blocks, std::slice::from_ref(&blocked.id));
    let v = e.c.board(&BoardFilter {
        repo_key: REPO_B.into(),
        ..BoardFilter::default()
    });
    assert_eq!(v.leases.len(), 1);
    assert_eq!(v.leases[0].lease.resource, "port:8080");
    e.c.release(USER, "port:8080").unwrap();
    assert_eq!(e.c.board(&BoardFilter::default()).leases.len(), 1);
}

#[test]
fn test_sweep_notices_closes_resolved_blockers() {
    let e = Env::new();
    e.c.acquire(BOB, "gpu/qwen", Duration::from_secs(3600), false)
        .unwrap();
    let on_lease =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "needs gpu".into(),
                block_reason: "bob has it".into(),
                blocked_by: bb(BLOCKED_ON_LEASE, "gpu/qwen"),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(ALICE, &on_lease.id).unwrap();
    let first = e.c.create_task(USER, nt("first")).unwrap();
    let on_task =
        e.c.create_task(
            CAROL,
            NewTask {
                title: "after first".into(),
                block_reason: "wait".into(),
                blocked_by: bb(BLOCKED_ON_TASK, &first.id),
                ..NewTask::default()
            },
        )
        .unwrap();
    let other =
        e.c.create_task(
            CAROL,
            NewTask {
                title: "person".into(),
                block_reason: "ask".into(),
                blocked_by: bb(BLOCKED_ON_PERSON, ""),
                ..NewTask::default()
            },
        )
        .unwrap();
    assert!(e.c.sweep_notices().unwrap().is_empty());
    e.c.release(BOB, "gpu/qwen").unwrap();
    e.c.set_task_state(USER, &first.id, COMPLETED, "").unwrap();
    let ns = e.c.sweep_notices().unwrap();
    assert_eq!(ns.len(), 2, "{ns:?}");
    for n in &ns {
        assert!(
            n.kind == NOTICE_UNBLOCKED && n.by == "irori" && !n.wake,
            "{n:?}"
        );
    }
    let by = |id: &str| ns.iter().find(|n| n.task_id == id).unwrap();
    assert!(by(&on_lease.id).to == [ALICE] && by(&on_lease.id).text.contains("lease released"));
    assert!(by(&on_task.id).to == [CAROL] && by(&on_task.id).text.contains("task completed"));
    let got = e.c.get_task(&on_lease.id).unwrap();
    assert_eq!(
        (got.state.as_str(), got.note.as_str()),
        (WORKING, "lease released")
    );
    assert_eq!(e.c.get_task(&on_task.id).unwrap().state, SUBMITTED);
    assert_eq!(e.c.get_task(&other.id).unwrap().state, BLOCKED);
    for ev in e.rows(kinds::BLOCKER_CLOSED) {
        assert_eq!(ev.actor, "irori");
    }
    assert!(e.c.sweep_notices().unwrap().is_empty());
}

#[test]
fn test_sweep_notices_expired_lease() {
    let e = Env::new();
    e.c.acquire(BOB, "port:3000", mins(1), false).unwrap();
    let task =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "t".into(),
                block_reason: "r".into(),
                blocked_by: bb(BLOCKED_ON_LEASE, "port:3000"),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.clk.add(mins(2));
    e.c.sweep().unwrap();
    assert_eq!(e.c.get_task(&task.id).unwrap().state, SUBMITTED);
}

#[test]
fn test_board_replay_equality() {
    let e = Env::new();
    let c = &e.c;
    let step = || e.clk.add(secs(1));
    let sh = c
        .create_task(
            ALICE,
            NewTask {
                title: "sh".into(),
                shared: true,
                links: vec!["docs/a.md".into()],
                ..NewTask::default()
            },
        )
        .unwrap();
    step();
    c.claim_task(ALICE, &sh.id).unwrap();
    c.join(BOB, &sh.id, BOB).unwrap();
    c.join(USER, &sh.id, CAROL).unwrap();
    c.leave(ALICE, &sh.id, ALICE).unwrap();
    step();
    c.block(BOB, &sh.id, "waiting", bb(BLOCKED_ON_PERSON, ""))
        .unwrap();
    c.unblock(USER, &sh.id, "done waiting").unwrap();
    c.add_link(CAROL, &sh.id, "https://github.com/a/b/issues/3")
        .unwrap();
    c.remove_link(BOB, &sh.id, "docs/a.md").unwrap();
    step();
    let _ = c.add_comment(USER, &sh.id, "hello", true);
    c.acquire(ALICE, "port:5173", Duration::from_secs(3600), false)
        .unwrap();
    let un = c
        .create_task(
            BOB,
            NewTask {
                title: "un".into(),
                block_reason: "port".into(),
                blocked_by: bb(BLOCKED_ON_LEASE, "port:5173"),
                ..NewTask::default()
            },
        )
        .unwrap();
    c.assign(CAROL, &un.id, DAVE).unwrap();
    c.set_shared(DAVE, &un.id, true).unwrap();
    let _ = c.acquire(BOB, "port:5173", Duration::ZERO, false);
    c.acquire(ALICE, "gpu/x", Duration::from_secs(3600), false)
        .unwrap();
    let _ = c.acquire(CAROL, "gpu/x", Duration::ZERO, false);
    let _ = c.release(ALICE, "gpu/x");
    step();
    let c2 = e.reopen();
    assert_eq!(
        c.list_tasks(&TaskFilter::default()),
        c2.list_tasks(&TaskFilter::default())
    );
    assert_eq!(c.comments(&sh.id), c2.comments(&sh.id));
    assert_eq!(c.comments(&sh.id).unwrap().len(), 1);
    assert_eq!(c.refusals(), c2.refusals());
    assert_eq!(c.refusals().len(), 1);
    let f = BoardFilter {
        viewer: USER.into(),
        ..BoardFilter::default()
    };
    assert_eq!(c.board(&f), c2.board(&f));
    let got = c2.get_task(&sh.id).unwrap();
    assert_eq!(got.holders, [BOB, CAROL]);
    assert!(
        got.state == WORKING
            && got.note == "done waiting"
            && got.links.len() == 1
            && got.links[0].label == "GitHub #3"
            && got.comments == 1
    );
}

#[test]
fn test_board_replay_tolerates() {
    let e = Env::new();
    let task = e.c.create_task(ALICE, nt("t")).unwrap();
    let id = &task.id;
    let rows: Vec<(&str, String)> = vec![
        (kinds::TASK_JOINED, format!(r#"{{"taskId":"{id}"}}"#)),
        (
            kinds::TASK_JOINED,
            format!(r#"{{"taskId":"missing","who":"{BOB}"}}"#),
        ),
        (
            kinds::TASK_LINKED,
            format!(r#"{{"link":{{}},"taskId":"{id}"}}"#),
        ),
        (
            kinds::TASK_COMMENT,
            r#"{"commentId":"c1","taskId":"missing","text":"x"}"#.into(),
        ),
        (
            kinds::BLOCKER_CLOSED,
            format!(r#"{{"restored":"working","taskId":"{id}"}}"#),
        ),
        (kinds::LEASE_REFUSED, r#"{"resource":"gpu/x"}"#.into()),
        (kinds::TASK_SHARED, r#""nope""#.into()),
    ];
    for (kind, raw) in rows {
        e.st.append("irori", kind, &gocompat::json::RawJson(raw.into_bytes()))
            .unwrap();
    }
    let c2 = e.reopen();
    let got = c2.get_task(id).unwrap();
    assert!(got.state == SUBMITTED && got.holders.is_empty() && got.links.is_empty());
    assert!(c2.comments("missing").is_none());
}

#[test]
fn test_person_comment_hearers() {
    let gone: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    type Setup = Box<dyn Fn(&Env) -> Task>;
    let cases: Vec<(&str, Setup, Vec<&str>, Vec<&str>, Vec<&str>, bool, String)> = vec![
        (
            "held",
            Box::new(|e: &Env| {
                let t = e.c.create_task(ALICE, nt("t")).unwrap();
                e.c.claim_task(BOB, &t.id).unwrap();
                e.c.watch(CAROL, &t.id).unwrap()
            }),
            vec![],
            vec![BOB],
            vec![CAROL],
            false,
            format!("{BOB} answers it"),
        ),
        (
            "assigned",
            Box::new(|e: &Env| {
                let t = e.c.create_task(USER, nt("t")).unwrap();
                e.c.assign(USER, &t.id, BOB).unwrap().0
            }),
            vec![],
            vec![BOB],
            vec![],
            false,
            String::new(),
        ),
        (
            "held and assigned: workers first",
            Box::new(|e: &Env| {
                let t =
                    e.c.create_task(
                        USER,
                        NewTask {
                            title: "t".into(),
                            shared: true,
                            ..NewTask::default()
                        },
                    )
                    .unwrap();
                e.c.assign(USER, &t.id, CAROL).unwrap();
                e.c.join(BOB, &t.id, BOB).unwrap()
            }),
            vec![],
            vec![BOB, CAROL],
            vec![],
            false,
            String::new(),
        ),
        (
            "watched only",
            Box::new(|e: &Env| {
                let t = e.c.create_task(USER, nt("t")).unwrap();
                e.c.watch(CAROL, &t.id).unwrap()
            }),
            vec![],
            vec![],
            vec![CAROL],
            true,
            "Nobody else is on it".into(),
        ),
        (
            "nobody holds: the creator",
            Box::new(|e: &Env| {
                let t = e.c.create_task(ALICE, nt("t")).unwrap();
                e.c.watch(CAROL, &t.id).unwrap()
            }),
            vec![],
            vec![ALICE],
            vec![CAROL],
            false,
            format!("{ALICE} answers it"),
        ),
        (
            "the holder has gone: the creator",
            Box::new(|e: &Env| {
                let t = e.c.create_task(ALICE, nt("t")).unwrap();
                e.c.claim_task(BOB, &t.id).unwrap()
            }),
            vec![BOB],
            vec![ALICE],
            vec![],
            false,
            String::new(),
        ),
        (
            "a creating watcher answers once",
            Box::new(|e: &Env| {
                let t = e.c.create_task(ALICE, nt("t")).unwrap();
                e.c.watch(ALICE, &t.id).unwrap()
            }),
            vec![],
            vec![ALICE],
            vec![],
            false,
            String::new(),
        ),
        (
            "nobody reachable",
            Box::new(|e: &Env| {
                let t = e.c.create_task(ALICE, nt("t")).unwrap();
                e.c.claim_task(BOB, &t.id).unwrap();
                e.c.watch(CAROL, &t.id).unwrap()
            }),
            vec![ALICE, BOB, CAROL],
            vec![],
            vec![],
            false,
            String::new(),
        ),
        (
            "the person's own card",
            Box::new(|e: &Env| e.c.create_task(USER, nt("t")).unwrap()),
            vec![],
            vec![],
            vec![],
            false,
            String::new(),
        ),
    ];
    for (name, setup, gone_now, answer, watch, watch_wake, watch_answers) in cases {
        let e = Env::new();
        let task = setup(&e);
        *gone.lock().unwrap() = gone_now.iter().map(|s| s.to_string()).collect();
        let g = gone.clone();
        let around = move |k: &str| !g.lock().unwrap().iter().any(|x| x == k);
        let (cm, ns) =
            e.c.person_comment(USER, &task.id, "which tag?", true, Some(&around))
                .unwrap();
        let unheard = answer.is_empty() && watch.is_empty();
        assert!(
            cm.unheard == unheard && cm.by == USER && cm.text == "which tag?",
            "{name}: {cm:?}"
        );
        let mut a = None;
        let mut w = None;
        for n in &ns {
            assert!(
                n.kind == NOTICE_COMMENT
                    && n.reference == format!("comment:{}", cm.id)
                    && n.text.contains("which tag?"),
                "{name}: {n:?}"
            );
            if n.watching {
                w = Some(n);
            } else {
                a = Some(n);
            }
        }
        assert_eq!(
            ns.len(),
            a.is_some() as usize + w.is_some() as usize,
            "{name}"
        );
        assert_eq!(answer.is_empty(), a.is_none(), "{name}");
        if let Some(a) = a {
            assert!(
                a.to == answer
                    && a.wake
                    && a.text.contains("Answer it on the card with board_comment"),
                "{name}: {a:?}"
            );
        }
        assert_eq!(watch.is_empty(), w.is_none(), "{name}");
        if let Some(w) = w {
            assert!(
                w.to == watch && w.wake == watch_wake && w.text.contains(&watch_answers),
                "{name}: {w:?}"
            );
        }
        assert_eq!(
            e.reopen().get_comment(&cm.id).unwrap().unheard,
            unheard,
            "{name}"
        );
    }
    let e = Env::new();
    let task = e.c.create_task(ALICE, nt("t")).unwrap();
    want_err(
        e.c.person_comment(BOB, &task.id, "me too", false, None),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.person_comment(USER, &task.id, " ", false, None),
        ErrorKind::Invalid,
    );
}

// ---- search ----

#[test]
fn test_search_docs() {
    use search::{Kind, Source};
    let e = Env::new();
    let t1 =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "one".into(),
                objective: "obj".into(),
                done_criteria: "dc".into(),
                repo_key: REPO_A.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(BOB, &t1.id).unwrap();
    e.clk.add(secs(1));
    let t2 =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "two".into(),
                block_reason: "stuck".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.clk.add(secs(1));
    let (c1, _) =
        e.c.add_comment(USER, &t1.id, "from a person", false)
            .unwrap();
    e.clk.add(secs(1));
    let (c2, _) =
        e.c.add_comment(CAROL, &t2.id, "from an agent", false)
            .unwrap();

    let docs =
        e.c.search_docs(gocompat::time::GoTime::default(), 0)
            .unwrap();
    let refs: Vec<&str> = docs.iter().map(|d| d.r#ref.as_str()).collect();
    // t1's comment bumped its Updated, so t2 comes first.
    assert_eq!(docs.len(), 4, "docs = {refs:?}");
    let by_ref = |r: String| docs.iter().find(|d| d.r#ref == r).unwrap().clone();
    let d = by_ref(format!("task:{}", t1.id));
    assert!(
        d.kind == Kind::Task
            && d.title == "one"
            && d.body == "#1\nobj\ndc"
            && d.agent == BOB
            && d.repo_key == REPO_A
            && d.at.equal(&t1.created_at)
            && d.updated.equal(&c1.at),
        "t1 doc {d:?}"
    );
    let d = by_ref(format!("task:{}", t2.id));
    assert!(d.body == "#2\nstuck" && d.agent.is_empty(), "t2 doc {d:?}");
    let d = by_ref(format!("comment:{}", c1.id));
    assert!(
        d.kind == Kind::Comment
            && d.title == "comment on one"
            && d.body == "from a person"
            && d.agent.is_empty()
            && d.repo_key == REPO_A,
        "c1 doc {d:?}"
    );
    let d = by_ref(format!("comment:{}", c2.id));
    assert_eq!(d.agent, CAROL, "c2 doc {d:?}");
    for w in docs.windows(2) {
        assert!(
            w[1].updated.unix_nano() >= w[0].updated.unix_nano(),
            "not oldest first: {refs:?}"
        );
    }
    // since is inclusive; limit caps.
    assert_eq!(e.c.search_docs(c2.at, 0).unwrap().len(), 2);
    let docs =
        e.c.search_docs(gocompat::time::GoTime::default(), 2)
            .unwrap();
    assert!(
        docs.len() == 2 && docs[0].r#ref == format!("comment:{}", c1.id),
        "limit 2: {docs:?}"
    );
}

#[test]
fn test_search_show() {
    use search::{Kind, ShowOptions, Source};
    let e = Env::new();
    let dep = e.c.create_task(ALICE, nt("dep")).unwrap();
    let task =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "the card".into(),
                objective: "do the thing".into(),
                done_criteria: "it works".into(),
                repo_key: REPO_A.into(),
                shared: true,
                links: vec!["https://linear.app/acme/issue/ABC-9/x".into()],
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(BOB, &task.id).unwrap();
    e.c.block(
        BOB,
        &task.id,
        "after dep",
        Some(BlockedBy {
            kind: BLOCKED_ON_TASK.into(),
            reference: dep.id.clone(),
        }),
    )
    .unwrap();
    let mut cids = vec![];
    for i in 0..13u8 {
        e.clk.add(secs(1));
        let text = if i == 12 {
            "long ".repeat(200)
        } else {
            format!("comment {}", (b'a' + i) as char)
        };
        let (cm, _) = e.c.add_comment(USER, &task.id, &text, false).unwrap();
        cids.push(cm.id);
    }
    let sh =
        e.c.search_show(&format!(" TASK:{}", task.id), &ShowOptions::default())
            .unwrap();
    assert!(
        sh.r#ref == format!("task:{}", task.id)
            && sh.kind == Kind::Task
            && sh.title == "the card"
            && !sh.more,
        "shown {sh:?}"
    );
    for s in [
        "state: blocked, shared".to_string(),
        format!("holders: {BOB}"),
        format!("blocked: after dep (on task {})", dep.id),
        "do the thing".into(),
        "it works".into(),
        "Linear ABC-9: https://linear.app/acme/issue/ABC-9/x".into(),
        "comments (13, newest 10 shown".into(),
        "comment d".into(),
        "comment l".into(),
        format!("show comment:{}", cids[12]),
    ] {
        assert!(sh.text.contains(&s), "show lacks {s:?}:\n{}", sh.text);
    }
    assert!(
        !sh.text.contains("comment c"),
        "an older comment was rendered"
    );
    assert_eq!(
        sh.related,
        vec![
            format!("task:{}", dep.id),
            format!("comment:{}", cids[2]),
            format!("comment:{}", cids[1]),
            format!("comment:{}", cids[0]),
        ]
    );
    // Paging.
    let r = format!("task:{}", task.id);
    let p1 =
        e.c.search_show(
            &r,
            &ShowOptions {
                limit: 100,
                ..ShowOptions::default()
            },
        )
        .unwrap();
    assert!(p1.more && p1.next == p1.text.len() as i64, "page 1 {p1:?}");
    let p2 =
        e.c.search_show(
            &r,
            &ShowOptions {
                offset: p1.next,
                limit: 100,
                ..ShowOptions::default()
            },
        )
        .unwrap();
    assert!(
        sh.text[p1.next as usize..].starts_with(&p2.text),
        "page 2 doesn't follow page 1"
    );

    let cs =
        e.c.search_show(&format!("comment:{}", cids[0]), &ShowOptions::default())
            .unwrap();
    assert!(
        cs.kind == Kind::Comment
            && cs.title == "comment on the card"
            && cs.text.contains("comment a")
            && cs.related == vec![format!("task:{}", task.id)],
        "comment shown {cs:?}"
    );

    for (r, not_found) in [
        ("task:nope", true),
        ("comment:nope", true),
        ("msg:x", true),
        ("nonsense", false),
    ] {
        let err = e.c.search_show(r, &ShowOptions::default()).unwrap_err();
        if not_found {
            assert!(err.is_not_found(), "{r}: {err}");
        } else {
            assert_eq!(err, search::Error::BadRef, "{r}");
        }
    }
    assert_eq!(e.c.search_kinds(), vec![Kind::Task, Kind::Comment]);
}
