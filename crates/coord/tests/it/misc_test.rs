//! Ports of I:internal/coord/holders_test.go, reopen_test.go,
//! pins_test.go and owner_test.go.
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

use crate::common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use coord::roster::Agents as _;
use coord::rows::{
    LeaseAcquiredRow, LeaseReleasedRow, TaskCommentRow, TaskCreatedRow, TaskPinnedRow,
};
use coord::*;
use store::{Event, kinds};

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

fn sec() -> Duration {
    Duration::from_secs(1)
}

fn task(e: &Env, id: &str) -> Task {
    e.c.get_task(id).unwrap_or_else(|| panic!("no task {id}"))
}

fn sweep_returned(e: &Env) -> Vec<Notice> {
    e.c.sweep_notices()
        .unwrap()
        .into_iter()
        .filter(|n| n.kind == NOTICE_RETURNED)
        .collect()
}

// ---- holders_test.go ----

#[test]
fn test_gone_holder_returns_the_card_to_its_creator() {
    let e = Env::new();
    let tk = e.c.create_task(BOB, nt("Fix the parser")).unwrap();
    e.c.claim_task(ALICE, &tk.id).unwrap();
    e.ros.set(ALICE, "");
    assert!(sweep_returned(&e).is_empty());
    e.clk.add(HOLDER_GRACE - sec());
    sweep_returned(&e);
    let got = task(&e, &tk.id);
    assert!(got.state == WORKING && got.holder == ALICE);
    e.clk.add(2 * sec());
    let got = sweep_returned(&e);
    assert!(got.len() == 1 && got[0].to == [BOB], "{got:?}");
    assert_eq!(
        got[0].text,
        format!(
            "Nobody is on \"Fix the parser\" (#1, task:{id}) any more: {ALICE} went away, and it's open again. Take it with board_join, hand it on with board_assign, or close it with board_status.\nshow task:{id} for the card, its comments and links.",
            id = tk.id
        )
    );
    let after = task(&e, &tk.id);
    let r = after.returned.clone().unwrap();
    assert!(
        after.state == SUBMITTED
            && after.holders.is_empty()
            && r.was == ALICE
            && !r.person
            && !r.row.is_empty(),
        "{after:?}"
    );
    assert_eq!(
        after.note,
        "nobody is on it: the agent working it went away"
    );
    e.clk.add(mins(1));
    assert!(sweep_returned(&e).is_empty());
    e.c.claim_task(CAROL, &tk.id).unwrap();
    assert!(task(&e, &tk.id).returned.is_none());
    // Card #490: once someone holds it, "nobody is on it" no longer holds.
    assert_eq!(task(&e, &tk.id).note, "");
}

// Card #490: live #396 said "nobody is on it: wonton left it" while a
// spawned agent held it. Each way back onto the card clears Irori's own
// note, a later blocker doesn't carry it, and a replay agrees.
#[test]
fn test_nobody_note_follows_the_holders() {
    type Back = fn(&Env, &str);
    let ways: [(&str, Back); 3] = [
        ("claim", |e, id| {
            e.c.claim_task(CAROL, id).unwrap();
        }),
        ("offer", |e, id| {
            e.c.assign_task(USER, id, CAROL).unwrap();
        }),
        ("step owner", |e, id| {
            e.c.set_steps(
                USER,
                id,
                vec![Step {
                    text: "Sync".into(),
                    ..Step::default()
                }],
            )
            .unwrap();
            let s = task(e, id).steps[0].id.clone();
            e.c.set_step_owner(USER, id, &s, CAROL).unwrap();
        }),
    ];
    for (how, back) in ways {
        let e = Env::new();
        let tk = e.c.create_task(USER, nt("Pair my devices")).unwrap();
        e.c.claim_task(ALICE, &tk.id).unwrap();
        e.c.leave(ALICE, &tk.id, ALICE).unwrap();
        let t = task(&e, &tk.id);
        assert!(
            t.holders.is_empty() && t.note.starts_with("nobody is on it"),
            "{how}: {t:?}"
        );
        back(&e, &tk.id);
        assert_eq!(task(&e, &tk.id).note, "", "{how}");
        assert_eq!(
            e.reopen().get_task(&tk.id).unwrap().note,
            "",
            "{how}: replay"
        );
        if how == "claim" {
            e.c.block(CAROL, &tk.id, "which db?", None).unwrap();
            assert_eq!(task(&e, &tk.id).note, "", "a blocker doesn't carry it back");
        }
    }
}

#[test]
fn test_other_notes_stay_when_someone_takes_the_card() {
    let e = Env::new();
    let tk = e.c.create_task(ALICE, nt("Ship docs")).unwrap();
    e.c.set_task_state(ALICE, &tk.id, WORKING, "keep me")
        .unwrap();
    assert_eq!(task(&e, &tk.id).note, "keep me");
    e.c.set_steps(
        ALICE,
        &tk.id,
        vec![Step {
            text: "Sync".into(),
            ..Step::default()
        }],
    )
    .unwrap();
    let s = task(&e, &tk.id).steps[0].id.clone();
    e.c.set_step_owner(ALICE, &tk.id, &s, CAROL).unwrap();
    assert_eq!(task(&e, &tk.id).note, "keep me");
}

#[test]
fn test_gone_holder_returns_the_card_to_the_person() {
    for creator in [USER, BOB] {
        let e = Env::new();
        let tk = e.c.create_task(creator, nt("Ship docs")).unwrap();
        e.c.claim_task(ALICE, &tk.id).unwrap();
        e.ros.set(ALICE, "");
        e.ros.set(BOB, "gone");
        sweep_returned(&e);
        e.clk.add(HOLDER_GRACE + sec());
        assert!(sweep_returned(&e).is_empty(), "creator {creator}");
        assert!(
            task(&e, &tk.id).returned.is_some_and(|r| r.person),
            "creator {creator}"
        );
    }
}

#[test]
fn test_gone_holder_on_blocked_and_shared_cards() {
    let e = Env::new();
    let blocked = e.c.create_task(BOB, nt("Blocked one")).unwrap();
    e.c.claim_task(ALICE, &blocked.id).unwrap();
    e.c.block(ALICE, &blocked.id, "which port?", None).unwrap();
    let shared =
        e.c.create_task(
            BOB,
            NewTask {
                title: "Shared one".into(),
                shared: true,
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.join(ALICE, &shared.id, ALICE).unwrap();
    e.c.join(CAROL, &shared.id, CAROL).unwrap();
    e.ros.set(ALICE, "");
    sweep_returned(&e);
    e.clk.add(HOLDER_GRACE + sec());
    let got = sweep_returned(&e);
    assert!(got.len() == 1 && got[0].task_id == blocked.id, "{got:?}");
    let b = task(&e, &blocked.id);
    assert!(b.state == BLOCKED && b.holders.is_empty());
    let s = task(&e, &shared.id);
    assert!(s.state == WORKING && s.holders == [CAROL] && s.returned.is_none());
}

#[test]
fn test_gone_holder_only_on_its_own_device() {
    let e = Env::new();
    let remote = "dev-z/zed-session";
    let a = e.c.create_task(BOB, nt("Remote")).unwrap();
    e.c.assign(BOB, &a.id, remote).unwrap();
    let p = e.c.create_task(BOB, nt("Person's")).unwrap();
    e.c.claim_task(USER, &p.id).unwrap();
    sweep_returned(&e);
    e.clk.add(HOLDER_GRACE + Duration::from_secs(3600));
    assert!(sweep_returned(&e).is_empty());
    assert_eq!(task(&e, &a.id).holders, [remote]);
    assert_eq!(task(&e, &p.id).holders, [USER]);
}

// ---- reopen_test.go ----

fn post_ask(e: &Env, text: &str) -> Task {
    e.c.post_ask(
        USER,
        NewAsk {
            text: text.into(),
            ..NewAsk::default()
        },
        &e.ros.agents(),
    )
    .unwrap()
    .0
}

#[test]
fn test_answered_ask_completes() {
    let e = Env::new();
    let tk = post_ask(&e, "what port does the server use?");
    e.c.ask_claim(ALICE, &tk.id, sec()).unwrap();
    let got =
        e.c.ask_close(
            ALICE,
            &tk.id,
            AskOutcome {
                kind: OUTCOME_ANSWERED.into(),
                note: "8080, said on the card".into(),
                ..AskOutcome::default()
            },
        )
        .unwrap();
    let a = got.ask.clone().unwrap();
    let o = a.outcome.clone().unwrap();
    assert!(
        got.state == COMPLETED
            && !a.is_open()
            && o.kind == OUTCOME_ANSWERED
            && o.note == "8080, said on the card"
    );
    assert_eq!(got.holders, [ALICE]);
    let b = e.reopen().get_task(&tk.id).unwrap();
    assert!(b.state == COMPLETED && b.ask == got.ask);
    want_err(
        e.c.ask_close(
            ALICE,
            &tk.id,
            AskOutcome {
                kind: "maybe".into(),
                note: "x".into(),
                ..AskOutcome::default()
            },
        ),
        ErrorKind::BadState,
    );
    let other = post_ask(&e, "x");
    e.c.ask_claim(ALICE, &other.id, sec()).unwrap();
    want_err(
        e.c.ask_close(
            ALICE,
            &other.id,
            AskOutcome {
                kind: "maybe".into(),
                note: "x".into(),
                ..AskOutcome::default()
            },
        ),
        ErrorKind::Invalid,
    );
}

#[test]
fn test_reopen_keeps_holders() {
    let e = Env::new();
    let tk = e.c.create_task(ALICE, nt("Fix the build")).unwrap();
    e.c.claim_task(ALICE, &tk.id).unwrap();
    want_err(e.c.reopen(USER, &tk.id), ErrorKind::BadState);
    e.c.set_task_state(ALICE, &tk.id, COMPLETED, "done")
        .unwrap();
    let (got, n) = e.c.reopen(USER, &tk.id).unwrap();
    assert!(got.state == WORKING && got.note == REOPEN_NOTE && got.holders == [ALICE]);
    assert!(n.kind == NOTICE_REOPENED && n.to == [ALICE] && n.text.contains("with you on it"));
    assert_eq!(
        n.text,
        format!(
            "The person reopened \"Fix the build\" (#1, task:{id}), which was completed. It is working again, with you on it: pick it up, or board_status it done again if there is nothing left to do.\nshow task:{id} for the card, its comments and links.",
            id = tk.id
        )
    );
    let b = e.reopen().get_task(&tk.id).unwrap();
    assert!(b.state == got.state && b.note == got.note && b.holders == got.holders);
    let acts = e.c.activity(&tk.id).unwrap();
    assert!(acts[0].what == "reopened it" && acts[0].by == USER);
    let free = e.c.create_task(USER, nt("Unheld")).unwrap();
    e.c.set_task_state(USER, &free.id, CANCELED, "").unwrap();
    assert_eq!(e.c.reopen(USER, &free.id).unwrap().0.state, SUBMITTED);
}

#[test]
fn test_reopen_subtasks_and_parents() {
    let e = Env::new();
    let p = e.c.create_task(USER, nt("Parent")).unwrap();
    let k =
        e.c.create_task(
            USER,
            NewTask {
                title: "Child".into(),
                parent: p.id.clone(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.set_task_state(USER, &k.id, COMPLETED, "").unwrap();
    e.c.set_task_state(USER, &p.id, COMPLETED, "").unwrap();
    want_err(e.c.reopen(USER, &p.id), ErrorKind::Subtask);
    e.c.reopen(USER, &k.id).unwrap();
    let (pt, kt) = (task(&e, &p.id), task(&e, &k.id));
    assert!(
        !is_terminal(&pt.state) && !is_terminal(effective_state(&pt)) && !is_terminal(&kt.state)
    );
    assert_eq!(
        pt.note,
        format!("its subtask {} was reopened", k.short_ref())
    );
}

#[test]
fn test_person_comment_reopens_a_finished_card() {
    let e = Env::new();
    let tk = post_ask(&e, "what port does the server use?");
    e.c.ask_claim(ALICE, &tk.id, sec()).unwrap();
    e.c.ask_close(
        ALICE,
        &tk.id,
        AskOutcome {
            kind: OUTCOME_ANSWERED.into(),
            note: "8080".into(),
            ..AskOutcome::default()
        },
    )
    .unwrap();
    let (_, ns) =
        e.c.person_comment(USER, &tk.id, "and in dev?", false, &[], None)
            .unwrap();
    assert!(
        ns.len() == 1 && ns[0].to == [ALICE] && ns[0].text.contains("open again"),
        "{ns:?}"
    );
    let got = task(&e, &tk.id);
    assert!(got.state == WORKING && got.note == REOPEN_COMMENT_NOTE && got.holders == [ALICE]);
    let b = e.reopen().get_task(&tk.id).unwrap();
    assert!(b.state == got.state && b.note == got.note && b.holders == got.holders);
    let acts = e.c.activity(&tk.id).unwrap();
    assert!(
        acts.len() >= 2 && acts[1].what == "reopened by the person's comment",
        "{acts:?}"
    );
    // ALICE works the reopened ask, so another agent takes this one (#82).
    let gone = e.c.create_task(BOB, nt("Left behind")).unwrap();
    e.c.claim_task(BOB, &gone.id).unwrap();
    e.c.set_task_state(BOB, &gone.id, COMPLETED, "").unwrap();
    let nobody = |_: &str| false;
    e.c.person_comment(USER, &gone.id, "still there?", false, &[], Some(&nobody))
        .unwrap();
    assert_eq!(task(&e, &gone.id).state, COMPLETED);
    let free = e.c.create_task(USER, nt("Unheld")).unwrap();
    e.c.set_task_state(USER, &free.id, COMPLETED, "").unwrap();
    e.c.person_comment(USER, &free.id, "fine", false, &[], None)
        .unwrap();
    assert_eq!(task(&e, &free.id).state, COMPLETED);
}

#[test]
fn test_watch_a_finished_card() {
    let e = Env::new();
    let tk = e.c.create_task(USER, nt("Done thing")).unwrap();
    e.c.set_task_state(USER, &tk.id, COMPLETED, "").unwrap();
    assert_eq!(e.c.watch(BOB, &tk.id).unwrap().watchers, [BOB]);
}

// ---- pins_test.go ----

fn pinned_order(c: &Coord) -> Vec<String> {
    let mut ps: Vec<Task> = c
        .list_tasks(&TaskFilter::default())
        .into_iter()
        .filter(|t| t.pinned)
        .collect();
    ps.sort_by(|a, b| a.pin_rank.total_cmp(&b.pin_rank));
    ps.into_iter().map(|t| t.id).collect()
}

#[test]
fn test_pins() {
    let e = Env::new();
    let a = e.c.create_task(ALICE, nt("a")).unwrap();
    let b = e.c.create_task(ALICE, nt("b")).unwrap();
    let c = e.c.create_task(ALICE, nt("c")).unwrap();
    want_err(e.c.pin(ALICE, &a.id, true), ErrorKind::NotAllowed);
    for x in [&a, &b, &c] {
        e.c.pin(USER, &x.id, true).unwrap();
    }
    assert_eq!(
        pinned_order(&e.c),
        [a.id.clone(), b.id.clone(), c.id.clone()]
    );
    e.c.move_pin(USER, &c.id, &a.id).unwrap();
    assert_eq!(
        pinned_order(&e.c),
        [c.id.clone(), a.id.clone(), b.id.clone()]
    );
    e.c.move_pin(USER, &b.id, &a.id).unwrap();
    assert_eq!(
        pinned_order(&e.c),
        [c.id.clone(), b.id.clone(), a.id.clone()]
    );
    assert!(e.c.move_pin(USER, &a.id, "no-such-card").is_err());
    e.c.pin(USER, &b.id, false).unwrap();
    let x = task(&e, &b.id);
    assert!(!x.pinned && x.pin_rank == 0.0);
    let want = [c.id.clone(), a.id.clone()];
    assert_eq!(pinned_order(&e.c), want);
    assert_eq!(pinned_order(&e.reopen()), want);
}

/// Card #138: the person clears done cards; only closed ones, only the
/// person, and the clear survives a reopen of the log. A board card's id
/// (not in this log) is taken as given.
#[test]
fn test_archive() {
    let e = Env::new();
    let a = e.c.create_task(ALICE, nt("a")).unwrap();
    let b = e.c.create_task(ALICE, nt("b")).unwrap();
    e.c.set_task_state(USER, &a.id, CANCELED, "").unwrap();
    want_err(
        e.c.archive(ALICE, std::slice::from_ref(&a.id), true),
        ErrorKind::NotAllowed,
    );
    want_err(
        e.c.archive(USER, &[a.id.clone(), b.id.clone()], true),
        ErrorKind::Invalid,
    );
    assert!(
        e.c.archived_ids().is_empty(),
        "a refused batch writes nothing"
    );
    let n =
        e.c.archive(USER, &[a.id.clone(), "board-card".into()], true)
            .unwrap();
    assert_eq!(n, 2);
    assert_eq!(
        e.c.archive(USER, std::slice::from_ref(&a.id), true)
            .unwrap(),
        0
    );
    let ids = e.c.archived_ids();
    assert!(ids.contains(&a.id) && ids.contains("board-card") && !ids.contains(&b.id));
    assert!(e.c.get_task(&a.id).is_some(), "show still finds it");
    let back = e.reopen();
    assert_eq!(back.archived_ids(), ids);
    e.c.archive(USER, std::slice::from_ref(&a.id), false)
        .unwrap();
    assert!(!e.c.archived_ids().contains(&a.id));
}

/// The person pauses an open card and resumes it (card #207): only the
/// person, never a closed card, a second pause changes nothing, and the
/// pause survives a reopen of the log.
#[test]
fn test_pause() {
    let e = Env::new();
    let a = e.c.create_task(ALICE, nt("a")).unwrap();
    let done = e.c.create_task(ALICE, nt("done")).unwrap();
    e.c.set_task_state(USER, &done.id, CANCELED, "").unwrap();
    want_err(e.c.pause(ALICE, &a.id, true, ""), ErrorKind::NotAllowed);
    want_err(e.c.pause(USER, &done.id, true, ""), ErrorKind::Invalid);
    want_err(e.c.pause(USER, "nope", true, ""), ErrorKind::NotFound);
    assert!(e.c.paused_ids().is_empty());
    let (t, changed) = e.c.pause(USER, &a.id, true, "wait for me").unwrap();
    assert!(changed && t.id == a.id);
    assert!(
        !e.c.pause(USER, &a.id, true, "").unwrap().1,
        "already paused"
    );
    assert!(e.c.paused_ids().contains(&a.id));
    assert!(task(&e, &a.id).paused, "the card says so");
    let back = e.reopen();
    assert!(back.get_task(&a.id).unwrap().paused, "after a reopen too");
    assert_eq!(back.paused_ids(), e.c.paused_ids());
    assert!(e.c.pause(USER, &a.id, false, "").unwrap().1);
    assert!(e.c.paused_ids().is_empty());
}

fn event(dev: &str, seq: i64, at: u64, actor: &str, kind: &str, raw: Vec<u8>) -> Event {
    Event {
        device_id: dev.into(),
        seq,
        hlc: hlc::Timestamp(at),
        actor: actor.into(),
        kind: kind.into(),
        payload: raw,
        sig: Vec::new(),
    }
}

fn pin_row(
    dev: &str,
    seq: i64,
    at: u64,
    actor: &str,
    by: &str,
    id: &str,
    pinned: bool,
    rank: f64,
) -> Event {
    let raw = gocompat::json::marshal(&TaskPinnedRow {
        task_id: id.into(),
        pinned,
        rank,
        by: by.into(),
        at: fixed_now(),
    })
    .unwrap();
    event(dev, seq, at, actor, kinds::TASK_PINNED, raw)
}

#[test]
fn test_pins_across_devices() {
    let base = hlc_at(fixed_now());
    let created = event(
        "dev-b",
        2,
        base + 6,
        "agent:dev-b/s",
        kinds::TASK_CREATED,
        gocompat::json::marshal(&TaskCreatedRow {
            task_id: "A".into(),
            num: 1,
            title: "A".into(),
            by: "dev-b/s".into(),
            at: fixed_now(),
            ..TaskCreatedRow::default()
        })
        .unwrap(),
    );
    let rows = [
        pin_row("dev-b", 1, base + 5, "user", "user:dev-b", "A", true, 1.0),
        created,
        pin_row(
            "dev-b",
            3,
            base + 7,
            "agent:dev-b/s",
            "dev-b/s",
            "A",
            false,
            0.0,
        ),
        pin_row("dev-b", 4, base + 20, "user", "user:dev-b", "A", false, 0.0),
        pin_row("dev-c", 1, base + 10, "user", "user:dev-c", "A", true, 3.0),
    ];
    let three: KeyFn =
        Arc::new(|k: &str| k == "user:dev-a" || k == "user:dev-b" || k == "user:dev-c");
    for (name, order) in BTreeMap::from([
        ("in order", vec![0, 1, 2, 3, 4]),
        ("dev-c first", vec![4, 0, 1, 2, 3]),
        ("dev-c between", vec![0, 1, 4, 2, 3]),
    ]) {
        let e = Env::with("dev-a", Some(three.clone()));
        for i in order {
            e.c.handle(&rows[i]).unwrap();
        }
        assert!(!task(&e, "A").pinned, "{name}");
    }
    let two: KeyFn = Arc::new(|k: &str| k == "user:dev-a" || k == "user:dev-b");
    let e = Env::with("dev-a", Some(two));
    for ev in &rows[..3] {
        e.c.handle(ev).unwrap();
    }
    let x = task(&e, "A");
    assert!(x.pinned && x.pin_rank == 1.0);
    assert_eq!(e.c.ignored_rows(), 1);
}

/// Card #233: a shared board's sidebar pin ("board:<id>") is no card, but
/// a pin row for it from another of the person's devices still tells
/// subscribers, as a board_pin change, or the sidebar never hears it. A
/// row for an unknown card id says nothing.
#[test]
fn test_board_pin_from_another_device_is_a_change() {
    let base = hlc_at(fixed_now());
    let two: KeyFn = Arc::new(|k: &str| k == "user:dev-a" || k == "user:dev-b");
    let e = Env::with("dev-a", Some(two));
    let q = e.c.subscribe();
    let pins = [
        pin_row(
            "dev-b",
            1,
            base + 5,
            "user",
            "user:dev-b",
            "board:abc",
            true,
            1.0,
        ),
        pin_row(
            "dev-b",
            2,
            base + 6,
            "user",
            "user:dev-b",
            "no-such-card",
            true,
            2.0,
        ),
    ];
    for ev in &pins {
        e.c.handle(ev).unwrap();
    }
    let mut got = Vec::new();
    while let Ok(Some(ch)) = q.recv_timeout(Duration::from_millis(100)) {
        got.push((ch.kind, ch.task_id));
    }
    assert_eq!(
        got,
        vec![(BOARD_PIN_CHANGED.to_string(), "board:abc".to_string())]
    );
    assert_eq!(e.c.pinned_ranks().get("board:abc"), Some(&1.0));
}

// ---- owner_test.go ----

#[test]
fn test_owner_keys_and_made_up_keys() {
    let other = "user:dev-b";
    let fake = "user:dev-z";
    let owner: KeyFn = Arc::new(move |k: &str| k == USER || k == "user:dev-b");
    let e = Env::with("dev-a", Some(owner));
    let tk = e.c.create_task(ALICE, nt("t")).unwrap();
    e.c.claim_task(ALICE, &tk.id).unwrap();
    // A made-up key is an agent, and any agent may change any card (#320).
    e.c.set_task_state(fake, &tk.id, INPUT_REQUIRED, "mine now")
        .unwrap();
    e.c.set_task_state(
        other,
        &tk.id,
        INPUT_REQUIRED,
        "the person on another device",
    )
    .unwrap();
    let res = format!("path:{REPO_A}:src/**");
    e.c.acquire(ALICE, &res, mins(1), false).unwrap();
    want_err(e.c.release(fake, &res), ErrorKind::Held);
    e.c.release(other, &res).unwrap();
    assert!(!e.c.is_person(fake) && e.c.is_person(other) && e.c.is_person(USER));
    // actorFor: a made-up key writes as an agent; another device's owner
    // key as the person.
    let t2 = e.c.create_task(fake, nt("by fake")).unwrap();
    let t3 = e.c.create_task(other, nt("by other")).unwrap();
    let rows = e.rows(kinds::TASK_CREATED);
    let actor = |id: &str| {
        rows.iter()
            .find(|r| String::from_utf8_lossy(&r.payload).contains(id))
            .unwrap()
            .actor
            .clone()
    };
    assert_eq!(actor(&t2.id), format!("agent:{fake}"));
    assert_eq!(actor(&t3.id), "user");
}

#[test]
fn test_lease_rows_from_the_wrong_device_are_ignored() {
    let e = Env::new();
    let res = format!("path:{REPO_A}:src/**");
    e.c.acquire(ALICE, &res, Duration::from_secs(3600), false)
        .unwrap();
    let at = hlc_at(plus(fixed_now(), mins(1)));
    let name = normalize_resource(&res).unwrap();
    let hour = plus(fixed_now(), Duration::from_secs(3600));
    e.insert(
        "dev-b",
        1,
        at + 1,
        "irori",
        kinds::LEASE_RELEASED,
        &LeaseReleasedRow {
            resource: name,
            holder: ALICE.into(),
            reason: REASON_RELEASED.into(),
            by: "dev-b/x".into(),
            at: fixed_now(),
        },
    );
    e.insert(
        "dev-b",
        2,
        at + 2,
        "irori",
        kinds::LEASE_ACQUIRED,
        &LeaseAcquiredRow {
            lease: Lease {
                resource: "named:x".into(),
                holder: BOB.into(),
                until: hour,
                acquired_at: fixed_now(),
                ..Lease::default()
            },
            stolen: vec![],
        },
    );
    let c2 = e.reopen();
    let ls = c2.leases();
    assert!(ls.len() == 1 && ls[0].holder == ALICE, "{ls:?}");
    assert_eq!(c2.ignored_rows(), 2);
    e.insert(
        "dev-b",
        3,
        at + 3,
        "irori",
        kinds::LEASE_ACQUIRED,
        &LeaseAcquiredRow {
            lease: Lease {
                resource: "named:y".into(),
                holder: "dev-b/agent".into(),
                until: hour,
                acquired_at: fixed_now(),
                ..Lease::default()
            },
            stolen: vec![],
        },
    );
    assert_eq!(e.reopen().leases().len(), 2);
}

#[test]
fn test_thread_rows_carry_event_device() {
    let e = Env::new();
    let t = e.c.create_task(ALICE, nt("ship it")).unwrap();
    let task_id = e.c.claim_task(ALICE, &t.id).unwrap().id;
    let at = hlc_at(plus(fixed_now(), mins(1)));
    const RECIPIENT: &str = "dev-b/x";
    let row = |id: &str, msg: &str, to: &str, text: &str, gone: bool| TaskCommentRow {
        task_id: task_id.clone(),
        comment_id: id.into(),
        msg_id: msg.into(),
        by: ALICE.into(),
        to: to.into(),
        text: text.into(),
        gone,
        at: fixed_now(),
        ..TaskCommentRow::default()
    };
    e.remote(
        "dev-b",
        1,
        at + 1,
        "irori",
        kinds::TASK_COMMENT,
        &row("c-b", "", "", "says alice", false),
    );
    e.remote(
        "dev-b",
        2,
        at + 2,
        "irori",
        kinds::TASK_COMMENT,
        &row("msg_spoof", "msg_spoof", BOB, "spoof", false),
    );
    e.remote(
        "dev-b",
        3,
        at + 3,
        "irori",
        kinds::TASK_COMMENT,
        &row("msg_gone", "msg_gone", RECIPIENT, "for x", true),
    );
    let check = |c: &Coord| {
        let cs = c.comments(&task_id).unwrap();
        assert!(
            cs.len() == 2
                && cs[0].id == "c-b"
                && cs[0].device == "dev-b"
                && cs[0].by == ALICE
                && cs[0].seq == 1
                && cs[0].hlc == at + 1,
            "{cs:?}"
        );
        let g = &cs[1];
        assert!(
            g.id == "msg_gone" && g.gone && g.device == "dev-b" && g.to == RECIPIENT,
            "{g:?}"
        );
        assert_eq!(c.ignored_rows(), 1);
        let acts = c.activity(&task_id).unwrap();
        assert!(
            acts[0].what == "commented"
                && acts[0].device == "dev-b"
                && acts[0].kind == kinds::TASK_COMMENT
                && acts[0].by == ALICE,
            "{acts:?}"
        );
    };
    check(&e.c);
    check(&e.reopen());
}
