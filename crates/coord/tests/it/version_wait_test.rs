//! Card #485: "Update the Mac to v0.11.0" as a wait Irori can check. A
//! version wait names the least app version and the devices that must run
//! it, and the sweep closes it once each has started with that version or
//! a later one (a device.started row).

use crate::common;

use common::*;
use coord::rows;
use coord::*;

fn on_version(r: &str) -> Option<BlockedBy> {
    Some(BlockedBy {
        kind: BLOCKED_ON_VERSION.into(),
        reference: r.into(),
    })
}

#[derive(serde::Serialize)]
struct Started<'a> {
    headless: bool,
    name: &'a str,
    os: &'a str,
    version: &'a str,
}

fn started(e: &Env, dev: &str, seq: i64, version: &str) {
    let row = Started {
        headless: false,
        name: dev,
        os: "darwin",
        version,
    };
    if dev == "dev-a" {
        e.st.append("irori", store::kinds::DEVICE_STARTED, &row)
            .unwrap();
        return;
    }
    let at = hlc_at(e.clk.now()) + seq as u64;
    e.insert(
        dev,
        seq,
        at,
        "irori",
        store::kinds::DEVICE_STARTED,
        &Started {
            headless: false,
            name: dev,
            os: "darwin",
            version,
        },
    );
}

fn held_card(e: &Env) -> Task {
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "Live check of the spawner".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
    t
}

#[test]
fn a_version_wait_closes_once_every_named_device_runs_it() {
    let e = Env::new();
    started(&e, "mac", 1, "0.10.2");
    started(&e, "pc", 1, "0.10.2");
    let t = held_card(&e);
    let got =
        e.c.block(
            ALICE,
            &t.id,
            "Update the Mac and the PC to v0.11.0",
            on_version("v0.11.0 mac pc mac"),
        )
        .unwrap();
    assert_eq!(got.state, BLOCKED);
    // The v goes, and the repeat with it.
    assert_eq!(got.blocked_by, on_version("0.11.0 mac pc"));
    assert!(e.c.sweep_notices().unwrap().is_empty());

    // The Mac updates past it; the PC is still behind.
    started(&e, "mac", 2, "0.11.1");
    assert!(e.c.sweep_notices().unwrap().is_empty());
    assert_eq!(e.c.get_task(&t.id).unwrap().state, BLOCKED);
    // A prerelease of the version is not the version.
    started(&e, "pc", 2, "0.11.0-rc.1");
    assert!(e.c.sweep_notices().unwrap().is_empty());

    started(&e, "pc", 3, "0.11.0");
    let ns = e.c.sweep_notices().unwrap();
    assert_eq!(ns.len(), 1, "{ns:?}");
    assert_eq!(ns[0].to, [ALICE]);
    assert_eq!(ns[0].kind, NOTICE_UNBLOCKED);
    assert!(
        ns[0]
            .text
            .contains("0.11.0 or later running on all 2 devices"),
        "{:?}",
        ns[0].text
    );
    let now = e.c.get_task(&t.id).unwrap();
    assert_eq!(now.state, WORKING);
    assert!(now.blockers.iter().all(|b| !b.open()));
    // The sweep closes the wait by name, never with a close of the whole
    // card (cards #570, #635).
    assert!(e.rows(store::kinds::BLOCKER_CLOSED).is_empty());
    let resolved = e.rows(store::kinds::BLOCKER_RESOLVED);
    assert_eq!(resolved.len(), 1);
    let p: rows::BlockerResolvedRow = payload(&resolved[0]);
    assert_eq!(p.blocker_id, now.blockers[0].id);
}

#[test]
fn a_version_wait_with_no_device_waits_on_this_one() {
    let e = Env::new();
    started(&e, "dev-a", 1, "0.10.2");
    let t = held_card(&e);
    let got =
        e.c.block(ALICE, &t.id, "Update to 0.11", on_version("0.11.0"))
            .unwrap();
    assert_eq!(got.blocked_by, on_version("0.11.0 dev-a"));
    started(&e, "dev-a", 2, "0.11.0");
    let ns = e.c.sweep_notices().unwrap();
    assert!(ns.iter().any(|n| n.task_id == t.id), "{ns:?}");
    assert_eq!(e.c.get_task(&t.id).unwrap().state, WORKING);
}

#[test]
fn one_version_wait_among_others_closes_alone() {
    let e = Env::new();
    started(&e, "mac", 1, "0.10.2");
    let t = held_card(&e);
    for (q, on) in [
        ("Update the Mac", on_version("0.11.0 mac")),
        ("Which repo for the test?", None),
    ] {
        e.c.raise(
            ALICE,
            &t.id,
            NewBlocker {
                question: q.into(),
                blocked_by: on,
                ..NewBlocker::default()
            },
        )
        .unwrap();
    }
    started(&e, "mac", 2, "0.11.0");
    let ns = e.c.sweep_notices().unwrap();
    assert!(
        ns.iter()
            .any(|n| n.to == [ALICE] && n.text.contains("0.11.0 or later running")),
        "{ns:?}"
    );
    let now = e.c.get_task(&t.id).unwrap();
    assert_eq!(now.state, BLOCKED, "the question is still open");
    let open: Vec<_> = now.blockers.iter().filter(|b| b.open()).collect();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].question, "Which repo for the test?");
    let closed = now
        .blockers
        .iter()
        .find(|b| b.on.as_ref().is_some_and(|o| o.kind == BLOCKED_ON_VERSION))
        .unwrap();
    assert_eq!(
        closed.resolved.as_ref().unwrap().answer,
        "0.11.0 or later running"
    );
}

#[test]
fn a_version_wait_already_met_or_malformed_is_refused() {
    let e = Env::new();
    started(&e, "mac", 1, "0.11.1");
    let t = held_card(&e);
    want_err(
        e.c.block(ALICE, &t.id, "Update the Mac", on_version("0.11.0 mac")),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.block(ALICE, &t.id, "Update", on_version("soon mac")),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.block(ALICE, &t.id, "Update", on_version("")),
        ErrorKind::Invalid,
    );
    // A device never heard from waits until it starts with the version.
    let got =
        e.c.block(ALICE, &t.id, "Update the phone", on_version("0.11.0 phone"))
            .unwrap();
    assert_eq!(got.state, BLOCKED);
}

#[test]
fn the_device_version_hook_answers_on_a_board() {
    let e = Env::new();
    let v = std::sync::Arc::new(std::sync::Mutex::new("0.10.2".to_string()));
    let v2 = v.clone();
    let c = e.reopen_with(Options {
        device_version: Some(std::sync::Arc::new(move |d: &str| {
            if d == "mac" {
                v2.lock().unwrap().clone()
            } else {
                String::new()
            }
        })),
        ..e.opts()
    });
    let t = c
        .create_task(
            ALICE,
            NewTask {
                title: "x".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    c.claim_task(ALICE, &t.id).unwrap();
    c.block(ALICE, &t.id, "Update the Mac", on_version("0.11.0 mac"))
        .unwrap();
    assert!(c.sweep_notices().unwrap().is_empty());
    *v.lock().unwrap() = "0.11.0".into();
    c.sweep_notices().unwrap();
    assert_eq!(c.get_task(&t.id).unwrap().state, WORKING);
}

#[test]
fn a_dev_build_never_meets_a_version() {
    assert!(!version_meets("dev", "0.1.0"));
    assert!(!version_meets("", "0.1.0"));
    assert!(version_meets("0.11.1", "0.11.0"));
    assert!(version_meets("0.12.0", "0.11.9"));
    assert!(!version_meets("0.11.0-rc.2", "0.11.0"));
    assert!(version_meets("v1.0.0", "0.99.0"));
    assert!(valid_version("v0.11.0"));
    assert!(!valid_version("0.11"));
}

fn done_env() -> Env {
    let mut e = Env::new();
    e.c = e.reopen_with(Options {
        done_check: true,
        ..e.opts()
    });
    e
}

/// Card #635: a version arriving on one device closes only its own wait,
/// never the person's done check on the same card.
#[test]
fn a_version_arriving_keeps_the_done_check() {
    let e = done_env();
    started(&e, "mac", 1, "0.10.2");
    let t = held_card(&e);
    e.c.block(ALICE, &t.id, "Update the Mac", on_version("0.11.0 mac"))
        .unwrap();
    let got =
        e.c.set_task_state(ALICE, &t.id, COMPLETED, "all green")
            .unwrap();
    assert_eq!(got.state, BLOCKED);
    assert_eq!(
        got.blockers.iter().filter(|b| b.open()).count(),
        2,
        "{:?}",
        got.blockers
    );

    started(&e, "mac", 2, "0.11.0");
    e.c.sweep_notices().unwrap();
    let now = e.c.get_task(&t.id).unwrap();
    assert_eq!(now.state, BLOCKED, "the done check is still open");
    let open: Vec<_> = now.blockers.iter().filter(|b| b.open()).collect();
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(open[0].done_check);
    assert_eq!(open[0].question, DONE_QUESTION);
    // The close names the wait it closes.
    let rows = e.rows(store::kinds::BLOCKER_RESOLVED);
    assert_eq!(rows.len(), 1);
}

/// Another device of the person: its agent raised the version wait, and
/// its sweep writes a close for it from the log it has seen so far.
const PC_AGENT: &str = "pc/gelato-session";

const SECOND: std::time::Duration = std::time::Duration::from_secs(1);

/// ALICE's card, waiting on a version the PC's agent asked for (b1).
fn pc_wait(e: &Env) -> Task {
    let t = held_card(e);
    let on = on_version("0.11.0 mac");
    e.clk.add(SECOND);
    e.remote(
        "pc",
        1,
        hlc_at(e.clk.now()),
        &format!("agent:{PC_AGENT}"),
        store::kinds::TASK_STATUS,
        &rows::TaskStateRow {
            task_id: t.id.clone(),
            state: BLOCKED.into(),
            reason: "Update the Mac".into(),
            blocked_by: on.clone(),
            blocker: Some(rows::BlockerRow {
                id: "b1".into(),
                question: "Update the Mac".into(),
                on,
                ..rows::BlockerRow::default()
            }),
            by: PC_AGENT.into(),
            at: e.clk.now(),
            ..rows::TaskStateRow::default()
        },
    );
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.state, BLOCKED, "{got:?}");
    got
}

fn pc_close(e: &Env, id: &str, seq: i64, at: u64) {
    e.remote(
        "pc",
        seq,
        at,
        "irori",
        store::kinds::BLOCKER_CLOSED,
        // As the PC's sweep writes it: the items it saw open.
        &rows::BlockerClosedRow {
            task_id: id.into(),
            by: "irori".into(),
            note: "0.11.0 or later running".into(),
            restored: WORKING.into(),
            at: e.clk.now(),
            blocker_ids: vec!["b1".into()],
        },
    );
}

/// The agent's completed raises the person's done check; the hlc of its
/// row.
fn done_check(e: &Env, id: &str) -> u64 {
    e.clk.add(SECOND);
    let got =
        e.c.set_task_state(ALICE, id, COMPLETED, "all green")
            .unwrap();
    assert_eq!(got.state, BLOCKED);
    assert!(got.blockers.iter().any(|b| b.open() && b.done_check));
    e.rows(store::kinds::TASK_STATUS).last().unwrap().hlc.0
}

fn keeps_the_done_check(t: &Task) {
    assert_eq!(t.state, BLOCKED, "{t:?}");
    let open: Vec<_> = t.blockers.iter().filter(|b| b.open()).collect();
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(open[0].done_check);
    assert_eq!(t.reason, DONE_QUESTION);
}

/// #600 as it went: the Mac closed the wait, the agent set the card
/// completed, and then the PC's close of the same wait, written before it
/// had seen the Mac's, arrived. It closes nothing: what it saw open is
/// closed already.
#[test]
fn a_late_close_of_a_closed_version_wait_keeps_the_done_check() {
    let e = done_env();
    let t = pc_wait(&e);
    e.clk.add(SECOND);
    pc_close(&e, &t.id, 2, hlc_at(e.clk.now()));
    let between = e.c.get_task(&t.id).unwrap();
    assert_eq!(between.state, WORKING, "{between:?}");
    let dc = done_check(&e, &t.id);
    pc_close(&e, &t.id, 3, dc - 1);
    let now = e.c.get_task(&t.id).unwrap();
    keeps_the_done_check(&now);
    assert_eq!(now.updated_by, ALICE, "the late close is a no-op");
    keeps_the_done_check(&e.reopen().get_task(&t.id).unwrap());
}

/// The PC's close of a wait still open here arrives after the done check:
/// it closes the wait alone, and the card waits on the person.
#[test]
fn a_late_close_of_an_open_version_wait_keeps_the_done_check() {
    let e = done_env();
    let t = pc_wait(&e);
    let dc = done_check(&e, &t.id);
    pc_close(&e, &t.id, 2, dc - 1);
    let now = e.c.get_task(&t.id).unwrap();
    keeps_the_done_check(&now);
    // The card still waits, so the close's note isn't the card's.
    assert_ne!(now.note, "0.11.0 or later running");
    let b1 = now.blockers.iter().find(|b| b.id == "b1").unwrap();
    assert!(!b1.open());
    assert_eq!(
        b1.resolved.as_ref().unwrap().answer,
        "0.11.0 or later running"
    );
    keeps_the_done_check(&e.reopen().get_task(&t.id).unwrap());
}
