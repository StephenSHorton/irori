//! Ports of I:internal/coord/review_test.go and board_review_test.go. Go's
//! peeks at `impl.held` are left to Leases().
#![allow(clippy::type_complexity)]

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use coord::roster::{AgentCard, Agents};
use coord::rows::{LeaseAcquiredRow, StolenLease, TaskCreatedRow};
use coord::*;
use store::{Event, kinds};

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

#[test]
fn test_repo_keys_with_colons_and_spaces() {
    let keys = [
        "remote:::1/a/b",
        "remote:fe80::1/a/b",
        "remote:host.com/a:b/c",
        "remote:host.com/my org/repo",
        REPO_A,
        REPO_B,
        // What Go's repo.NormalizeRemote makes of Go's six live remotes
        // (https://[::1]/a/b, ssh://git@[fe80::1]:22/a/b.git,
        // git@host.com:a:b/c.git, https://host.com/a%3Ab/c,
        // https://host.com/my%20org/repo, git@github.com:Foo/Bar.git), as
        // Go at be67a86 prints them, written in: coord sits beside capture
        // and can't call its resolver.
        "remote:::1/a/b",
        "remote:fe80::1/a/b",
        "remote:host.com/a:b/c",
        "remote:host.com/a:b/c",
        "remote:host.com/my org/repo",
        "remote:github.com/foo/bar",
    ];
    let e = Env::new();
    for k in keys {
        e.c.create_task(
            ALICE,
            NewTask {
                title: "t".into(),
                repo_key: k.into(),
                ..NewTask::default()
            },
        )
        .unwrap_or_else(|x| panic!("{k}: {x}"));
        let r = parse_resource(&format!("path:{k}:src/**")).unwrap();
        assert!(
            r.repo_key == k && r.glob == "src/**" && r.name == format!("path:{k}:src/**"),
            "{k}: {r:?}"
        );
    }
    let r = parse_resource("path:remote:host.com/a:b/c:src/**").unwrap();
    assert!(r.repo_key == "remote:host.com/a:b/c" && r.glob == "src/**");
    e.c.acquire(
        ALICE,
        "path:remote:host.com/a:b/c:src/**",
        Duration::ZERO,
        false,
    )
    .unwrap();
    e.c.acquire(BOB, "path:remote:host.com/a:src/**", Duration::ZERO, false)
        .unwrap();
    want_err(
        e.c.acquire(
            BOB,
            "path:remote:host.com/a:b/c:src/x.go",
            Duration::ZERO,
            false,
        ),
        ErrorKind::Held,
    );
}

#[test]
fn test_glob_rejects_control_characters() {
    for g in ["a\nb", "src/\x00x", "a\tb", "x\u{85}y"] {
        let r = normalize_glob(g);
        assert!(
            r.as_ref().is_err_and(|x| x.is(ErrorKind::BadResource)),
            "{g:?}: {r:?}"
        );
    }
}

#[test]
fn test_steal_is_one_acquired_row() {
    let e = Env::new();
    e.c.acquire(
        ALICE,
        &format!("path:{REPO_A}:src/**"),
        Duration::ZERO,
        false,
    )
    .unwrap();
    let before = e.rows("lease.").len();
    e.c.acquire(
        BOB,
        &format!("path:{REPO_A}:src/x.go"),
        Duration::ZERO,
        true,
    )
    .unwrap();
    assert_eq!(e.rows("lease.").len() - before, 1);
    e.st.append(
        &format!("agent:{CAROL}"),
        kinds::LEASE_ACQUIRED,
        &LeaseAcquiredRow {
            lease: Lease {
                resource: format!("path:{REPO_A}:src/x.go/**"),
                holder: CAROL.into(),
                until: plus(fixed_now(), Duration::from_secs(3600)),
                acquired_at: fixed_now(),
                stolen_from: BOB.into(),
                ..Lease::default()
            },
            stolen: vec![StolenLease {
                resource: format!("path:{REPO_A}:src/x.go"),
                holder: BOB.into(),
            }],
        },
    )
    .unwrap();
    let got = e.reopen().leases();
    assert!(got.len() == 1 && got[0].holder == CAROL, "{got:?}");
}

#[derive(Default)]
struct CardRoster(Mutex<Vec<AgentCard>>);

impl Agents for CardRoster {
    fn agents(&self) -> Vec<AgentCard> {
        self.0.lock().unwrap().clone()
    }
}

fn card(key: &str, status: &str, at: Option<gocompat::time::GoTime>) -> AgentCard {
    AgentCard {
        key: key.into(),
        status: status.into(),
        updated_at: at.unwrap_or_default(),
        ..AgentCard::default()
    }
}

#[test]
fn test_gone_card_starts_grace_clock() {
    let e = Env::new();
    let ros = Arc::new(CardRoster::default());
    *ros.0.lock().unwrap() = vec![card(ALICE, "idle", None), card(BOB, "idle", None)];
    let r: Arc<dyn Agents> = ros.clone();
    let c = Coord::new(
        e.st.clone(),
        Some(r),
        Options {
            now: Some(e.clk.func()),
            ..Options::default()
        },
    )
    .unwrap();
    let task = c.create_task(ALICE, nt("t")).unwrap();
    c.claim_task(ALICE, &task.id).unwrap();
    *ros.0.lock().unwrap() = vec![
        card(ALICE, "gone", Some(e.clk.now())),
        card(BOB, "idle", None),
    ];
    e.clk.add(Duration::from_secs(3600));
    assert_eq!(c.claim_task(BOB, &task.id).unwrap().holder, BOB);
    let task2 = c.create_task(BOB, nt("t2")).unwrap();
    c.claim_task(BOB, &task2.id).unwrap();
    *ros.0.lock().unwrap() = vec![
        card(ALICE, "idle", None),
        card(BOB, "gone", Some(e.clk.now())),
    ];
    e.clk.add(secs(1));
    want_err(c.claim_task(ALICE, &task2.id), ErrorKind::Held);
}

#[test]
fn test_expired_leases_are_pruned() {
    let e = Env::new();
    for i in 0..5 {
        e.c.acquire(ALICE, &format!("port:{}", 9000 + i), mins(1), false)
            .unwrap();
    }
    e.clk.add(Duration::from_secs(3600));
    assert!(e.c.leases().is_empty());
    e.c.acquire(BOB, "port:1", Duration::ZERO, false).unwrap();
    assert_eq!(e.c.leases().len(), 1);
    assert_eq!(e.reopen().leases().len(), 1);
}

#[test]
fn test_list_tasks_same_millisecond_keeps_creation_order() {
    let e = Env::new();
    let want: Vec<String> = (0..20).map(|i| i.to_string()).collect();
    for w in &want {
        e.c.create_task(ALICE, nt(w)).unwrap();
    }
    for c in [&e.c, &e.reopen()] {
        let got: Vec<String> = c
            .list_tasks(&TaskFilter::default())
            .into_iter()
            .map(|t| t.title)
            .collect();
        assert_eq!(got, want);
    }
}

#[test]
fn test_assignee_claim_moves_to_working() {
    let e = Env::new();
    let task = e.c.create_task(ALICE, nt("t")).unwrap();
    let task = e.c.assign_task(ALICE, &task.id, BOB).unwrap();
    assert!(task.state == SUBMITTED && task.holder == BOB);
    let task = e.c.claim_task(BOB, &task.id).unwrap();
    assert!(task.state == WORKING && task.holder == BOB);
    assert_eq!(e.rows(kinds::TASK_CLAIMED).len(), 1);
    e.c.claim_task(BOB, &task.id).unwrap();
    assert_eq!(e.rows(kinds::TASK_CLAIMED).len(), 1);
    assert_eq!(e.reopen().get_task(&task.id).unwrap().state, WORKING);
}

#[test]
fn test_replay_hlc_tie_at_page_boundary() {
    let e = Env::new();
    let n = store::MAX_LIST_LIMIT as usize;
    for i in 0..n {
        e.c.create_task(ALICE, nt(&i.to_string())).unwrap();
    }
    let rows = e.rows("task.");
    assert_eq!(rows.len(), n);
    let tie = rows.last().unwrap().hlc;
    for (i, dev) in ["dev-b", "dev-c"].iter().enumerate() {
        let id = format!("B{}", i + 1);
        e.st.insert(&Event {
            device_id: dev.to_string(),
            seq: 1,
            hlc: tie,
            actor: format!("agent:{dev}/s"),
            kind: kinds::TASK_CREATED.into(),
            payload: gocompat::json::marshal(&TaskCreatedRow {
                task_id: id.clone(),
                title: id,
                by: format!("{dev}/s"),
                at: fixed_now(),
                ..TaskCreatedRow::default()
            })
            .unwrap(),
            sig: Vec::new(),
        })
        .unwrap();
    }
    let c2 = e.reopen();
    assert_eq!(c2.list_tasks(&TaskFilter::default()).len(), n + 2);
    let last: TaskCreatedRow = payload(rows.last().unwrap());
    for id in ["B1", "B2", last.task_id.as_str()] {
        assert!(c2.get_task(id).is_some(), "{id} missing");
    }
}

// ---- board_review_test.go ----

fn board_has(cs: &[Card], id: &str) -> bool {
    cs.iter().any(|c| c.task.id == id)
}

fn for_who(who: &str) -> BoardFilter {
    BoardFilter {
        viewer: who.into(),
        ..BoardFilter::default()
    }
}

#[test]
fn test_unblock_keeps_unclaimed_assignee_submitted() {
    type Close = Box<dyn Fn(&Env, &str)>;
    let cases: Vec<(&str, Close, bool, &str)> = vec![
        (
            "unblock, never claimed",
            Box::new(|e: &Env, id: &str| {
                e.c.unblock(CAROL, id, "").unwrap();
            }),
            false,
            SUBMITTED,
        ),
        (
            "sweeper, never claimed",
            Box::new(|e: &Env, _id: &str| {
                e.c.release(CAROL, "port:5173").unwrap();
                e.c.sweep_notices().unwrap();
            }),
            false,
            SUBMITTED,
        ),
        (
            "unblock, claimed",
            Box::new(|e: &Env, id: &str| {
                e.c.unblock(CAROL, id, "").unwrap();
            }),
            true,
            WORKING,
        ),
    ];
    for (name, close, claim, want) in cases {
        let e = Env::new();
        e.c.acquire(CAROL, "port:5173", Duration::from_secs(3600), false)
            .unwrap();
        let task = e.c.create_task(ALICE, nt("t")).unwrap();
        e.c.assign(ALICE, &task.id, BOB).unwrap();
        if claim {
            e.c.claim_task(BOB, &task.id).unwrap();
        }
        e.c.block(
            ALICE,
            &task.id,
            "wait",
            Some(BlockedBy {
                kind: BLOCKED_ON_LEASE.into(),
                reference: "port:5173".into(),
            }),
        )
        .unwrap();
        close(&e, &task.id);
        for c in [&e.c, &e.reopen()] {
            let got = c.get_task(&task.id).unwrap();
            assert!(got.state == want && got.holders == [BOB], "{name}: {got:?}");
            assert_ne!(
                board_has(&c.board(&for_who(BOB)).needs_you, &task.id),
                claim,
                "{name}"
            );
        }
    }
}

#[test]
fn test_shared_assignee_not_engaged_until_it_claims() {
    for op in ["claim", "join"] {
        let e = Env::new();
        let task =
            e.c.create_task(
                ALICE,
                NewTask {
                    title: "shared".into(),
                    shared: true,
                    ..NewTask::default()
                },
            )
            .unwrap();
        e.c.claim_task(ALICE, &task.id).unwrap();
        let (got, n) = e.c.assign(ALICE, &task.id, BOB).unwrap();
        assert!(
            n.to == [BOB] && got.assigned == [BOB] && n.text.contains("Claim it"),
            "{op}"
        );
        let v = e.c.board(&for_who(BOB));
        assert!(
            board_has(&v.needs_you, &task.id) && !board_has(&v.working, &task.id),
            "{op}"
        );
        let (kind, got, want_rows) = if op == "claim" {
            (
                kinds::TASK_CLAIMED,
                e.c.claim_task(BOB, &task.id).unwrap(),
                2,
            )
        } else {
            (kinds::TASK_JOINED, e.c.join(BOB, &task.id, BOB).unwrap(), 1)
        };
        assert_eq!(e.rows(kind).len(), want_rows, "{op}");
        assert!(
            got.assigned.is_empty() && got.holders == [ALICE, BOB],
            "{op}: {got:?}"
        );
        let v = e.c.board(&for_who(BOB));
        assert!(
            !board_has(&v.needs_you, &task.id) && board_has(&v.working, &task.id),
            "{op}"
        );
        e.c.claim_task(BOB, &task.id).unwrap();
        assert_eq!(e.rows(kind).len(), want_rows, "{op}");
        assert_eq!(e.reopen().get_task(&task.id).unwrap(), got, "{op}");
    }
}

#[test]
fn test_assigned_follows_holders() {
    let e = Env::new();
    let un = e.c.create_task(ALICE, nt("u")).unwrap();
    e.c.claim_task(ALICE, &un.id).unwrap();
    let (got, _) = e.c.assign(ALICE, &un.id, BOB).unwrap();
    assert!(got.assigned == [BOB] && got.state == WORKING);
    let (got, _) = e.c.assign(BOB, &un.id, CAROL).unwrap();
    assert_eq!(got.assigned, [CAROL]);
    let got =
        e.c.set_task_state(CAROL, &un.id, INPUT_REQUIRED, "?")
            .unwrap();
    assert!(got.assigned.is_empty());
    let sh =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "shared".into(),
                shared: true,
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.assign(ALICE, &sh.id, BOB).unwrap();
    e.c.assign(ALICE, &sh.id, CAROL).unwrap();
    let got = e.c.leave(BOB, &sh.id, BOB).unwrap();
    assert!(got.assigned == [CAROL] && got.holders == [CAROL]);
}

#[test]
fn test_assign_and_join_check_keys() {
    let e = Env::new();
    let task = e.c.create_task(ALICE, nt("t")).unwrap();
    let sh =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "shared".into(),
                shared: true,
                ..NewTask::default()
            },
        )
        .unwrap();
    for k in [
        "irori".to_string(),
        "a\nb".into(),
        "a\tb".into(),
        "k".repeat(MAX_KEY + 1),
    ] {
        want_err(e.c.assign(ALICE, &task.id, &k), ErrorKind::Invalid);
        want_err(e.c.assign_task(ALICE, &task.id, &k), ErrorKind::Invalid);
        want_err(e.c.join(USER, &sh.id, &k), ErrorKind::Invalid);
    }
    assert_eq!(
        e.rows(kinds::TASK_ASSIGNED).len() + e.rows(kinds::TASK_JOINED).len(),
        0
    );
    e.c.assign(ALICE, &task.id, &"k".repeat(MAX_KEY)).unwrap();
}

#[test]
fn test_block_on_terminal_task_refused() {
    let e = Env::new();
    let a = e.c.create_task(ALICE, nt("a")).unwrap();
    let b = e.c.create_task(ALICE, nt("b")).unwrap();
    e.c.set_task_state(ALICE, &b.id, COMPLETED, "").unwrap();
    let on = Some(BlockedBy {
        kind: BLOCKED_ON_TASK.into(),
        reference: b.id.clone(),
    });
    want_err(
        e.c.block(ALICE, &a.id, "waiting", on.clone()),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.create_task(
            ALICE,
            NewTask {
                title: "c".into(),
                block_reason: "waiting".into(),
                blocked_by: on,
                ..NewTask::default()
            },
        ),
        ErrorKind::Invalid,
    );
    assert!(e.c.sweep_notices().unwrap().is_empty());
}
