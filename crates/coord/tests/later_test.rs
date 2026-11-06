//! Card #570: a card held as Not yet (docs/design/later-order.md). It is
//! BLOCKED with one open item that carries `later`: never handed out,
//! never spawned, never Needs you, and it comes back by a card, a date, a
//! version or the person's word.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::two_people::*;
use common::*;
use coord::roster::AgentCard;
use coord::rows::{BlockerResolvedRow, BlockerRow, TaskStateRow};
use coord::*;
use gocompat::time::GoTime;
use store::kinds;

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        objective: "why it matters".into(),
        ..NewTask::default()
    }
}

fn hours(n: u64) -> Duration {
    Duration::from_secs(3600 * n)
}

/// A date a day after the harness's clock.
fn tomorrow() -> Back {
    Back::Date(plus(fixed_now(), hours(24)))
}

fn make(e: &Env, title: &str) -> Task {
    e.c.create_task(ALICE, nt(title)).unwrap()
}

fn parked(e: &Env, title: &str, back: Back) -> Task {
    let t = make(e, title);
    e.c.park(ALICE, &t.id, "not this month", back).unwrap().0
}

fn agents() -> Vec<AgentCard> {
    let at = GoTime::unix(fixed_now().secs - 3600, 0);
    [BOB, CAROL]
        .iter()
        .map(|k| AgentCard {
            key: (*k).into(),
            device_id: "dev-a".into(),
            status: "idle".into(),
            updated_at: at,
            last_activity: at,
            ..AgentCard::default()
        })
        .collect()
}

fn ask(id: &str) -> SpawnAsk {
    SpawnAsk {
        task_id: id.into(),
        device: "dev-a".into(),
        folder: "/repo".into(),
        permission_mode: "bypassPermissions".into(),
        name: "for-1".into(),
        ..SpawnAsk::default()
    }
}

fn spawning(e: &Env) -> Coord {
    let on = Arc::new(AtomicBool::new(true));
    e.reopen_with(Options {
        spawning: Some(Arc::new(move || on.load(Ordering::SeqCst))),
        ..e.opts()
    })
}

fn lane_of(c: &Coord, viewer: &str, id: &str) -> &'static str {
    let v = c.board(&BoardFilter {
        viewer: viewer.into(),
        ..BoardFilter::default()
    });
    let has = |g: &[Card]| g.iter().any(|c| c.task.id == id);
    match () {
        _ if has(&v.needs_you) => "needs_you",
        _ if has(&v.later) => "later",
        _ if has(&v.blocked) => "blocked",
        _ if has(&v.working) => "working",
        _ if has(&v.open) => "open",
        _ if has(&v.done) => "done",
        _ => "",
    }
}

/// What a sweep row closing a hold must be: per item, with where it goes.
fn closes_per_item(e: &Env, id: &str, bid: &str) {
    assert!(
        e.rows(kinds::BLOCKER_CLOSED).is_empty(),
        "blocker.closed names no item"
    );
    let rows: Vec<BlockerResolvedRow> = e
        .rows(kinds::BLOCKER_RESOLVED)
        .iter()
        .map(payload::<BlockerResolvedRow>)
        .filter(|r| r.task_id == id)
        .collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].blocker_id, bid);
    assert!(rows[0].closed);
    assert_eq!(rows[0].option, -1);
    assert_eq!(rows[0].restored, SUBMITTED);
}

fn on_task(id: &str) -> BlockedBy {
    BlockedBy {
        kind: BLOCKED_ON_TASK.into(),
        reference: id.into(),
    }
}

#[test]
fn back_reads_what_it_wrote_and_an_unknown_value_is_the_persons_word() {
    let at = GoTime::parse_rfc3339("2026-12-01T05:00:00Z").unwrap();
    assert_eq!(Back::Date(at).encode(), "date 2026-12-01T05:00:00Z");
    assert_eq!(
        Back::decode("date 2026-12-01T05:00:00Z", None),
        Back::Date(at)
    );
    let v = "0.13.0 dev-a dev-b";
    assert_eq!(
        Back::Version(v.into()).encode(),
        "version 0.13.0 dev-a dev-b"
    );
    assert_eq!(
        Back::decode("version 0.13.0 dev-a dev-b", None),
        Back::Version(v.into())
    );
    assert_eq!(Back::Word.encode(), "word");
    assert_eq!(Back::decode("word", None), Back::Word);
    assert_eq!(Back::Card("t1".into()).encode(), "card");
    assert_eq!(
        Back::decode("card", Some(&on_task("t1"))),
        Back::Card("t1".into())
    );
    assert_eq!(Back::Card("t1".into()).on(), on_task("t1"));
    assert_eq!(Back::Word.on().kind, BLOCKED_ON_AGENT);
    assert_eq!(Back::Word.on().reference, "irori");
    for odd in [
        "someday",
        "date soon",
        "date",
        "version",
        "version 0.13",
        "version 0.13.0",
        "card x",
        "card",
    ] {
        assert_eq!(Back::decode(odd, None), Back::Word, "{odd}");
    }
}

#[test]
fn a_park_is_one_blocked_row_that_old_devices_read_as_a_wait_on_irori() {
    let e = Env::new();
    let t = parked(&e, "Voice calls", tomorrow());
    assert_eq!(t.state, BLOCKED);
    let b = t.parked().expect("held");
    assert_eq!(b.question, "Not yet: not this month");
    assert_eq!(b.later, "date 2026-10-27T12:00:00Z");
    assert_eq!(b.context, "Comes back on 2026-10-27");
    assert_eq!(b.back(), Some(tomorrow()));
    assert_eq!(b.on.as_ref().unwrap().kind, BLOCKED_ON_AGENT);
    assert_eq!(b.on.as_ref().unwrap().reference, "irori");

    let rows = e.rows(kinds::TASK_STATUS);
    assert_eq!(rows.len(), 1);
    let r: TaskStateRow = payload(&rows[0]);
    let br = r.blocker.unwrap();
    assert_eq!(r.state, BLOCKED);
    assert_eq!(r.blocked_by, br.on);
    assert_eq!(br.later, "date 2026-10-27T12:00:00Z");
    assert!(!br.reason_only && !br.replace);

    // A replay reads the same, and `later` is only on a Not yet.
    assert_eq!(e.reopen().get_task(&t.id).unwrap().parked(), Some(b));
    let plain = serde_json::to_string(&Blocker::default()).unwrap();
    assert!(!plain.contains("later"), "{plain}");
    assert!(
        serde_json::to_string(&t)
            .unwrap()
            .contains("\"later\":\"date")
    );
}

#[test]
fn it_is_never_handed_out() {
    let e = Env::new();
    let t = parked(&e, "Not now", Back::Word);
    assert!(e.c.sweep_asks(&agents()).unwrap().is_empty());
    assert_eq!(e.c.unserved_offers(&agents()), 0);
    let got = e.c.get_task(&t.id).unwrap();
    assert!(got.offer.is_none() && got.live_offer().is_none(), "{got:?}");
    assert!(e.rows(kinds::TASK_OFFERED).is_empty());
}

#[test]
fn it_is_never_spawned() {
    let e = Env::new();
    let t = parked(&e, "Not now", Back::Word);
    let c = spawning(&e);
    assert!(c.cards_to_spawn().is_empty());
    assert_eq!(c.request_spawn("irori", ask(&t.id)).unwrap(), None);
    assert!(e.rows(kinds::TASK_SPAWN).is_empty());
}

#[test]
fn it_is_never_needs_you_and_sits_in_its_own_lane() {
    let mut e = Env::new();
    let t = parked(&e, "Not now", Back::Word);
    for viewer in ["", USER, ALICE, BOB] {
        assert_eq!(lane_of(&e.c, viewer, &t.id), "later", "viewer {viewer:?}");
    }
    // The person's reply on its thread doesn't make it a decision for
    // anyone, nor put it back.
    let bid = t.parked().unwrap().id.clone();
    e.c.answer(
        USER,
        &t.id,
        &bid,
        BlockerAnswer {
            text: "good call".into(),
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.state, BLOCKED);
    assert!(got.parked().is_some());
    for viewer in ["", USER, ALICE] {
        assert_eq!(lane_of(&e.c, viewer, &t.id), "later", "viewer {viewer:?}");
    }
    // A gone parker strands nothing.
    e.ros.set(ALICE, "");
    e.clk.add(hours(1));
    e.c = e.reopen();
    assert!(e.c.stranded(&t.id).is_empty());
    assert_eq!(lane_of(&e.c, USER, &t.id), "later");
}

#[test]
fn a_question_raised_on_a_held_card_is_still_the_persons() {
    let e = Env::new();
    let t = parked(&e, "Not now", Back::Word);
    e.c.raise(
        BOB,
        &t.id,
        NewBlocker {
            question: "Is this still wanted?".into(),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    assert_eq!(lane_of(&e.c, USER, &t.id), "needs_you");
}

#[test]
fn it_comes_back_when_its_card_finishes() {
    let e = Env::new();
    let first = make(&e, "first");
    let t = parked(&e, "second", Back::Card(first.id.clone()));
    let b = t.parked().unwrap().clone();
    assert_eq!(b.later, "card");
    assert_eq!(b.on, Some(on_task(&first.id)));
    assert_eq!(task_waits(&t), vec![first.id.clone()]);
    assert_eq!(b.context, "Comes back when #1 finishes");

    e.c.sweep_notices().unwrap();
    assert_eq!(e.c.get_task(&t.id).unwrap().state, BLOCKED);
    e.c.claim_task(BOB, &first.id).unwrap();
    e.c.set_task_state(BOB, &first.id, COMPLETED, "done")
        .unwrap();
    e.c.sweep_notices().unwrap();

    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.state, SUBMITTED);
    assert!(got.parked().is_none() && got.holders.is_empty());
    closes_per_item(&e, &t.id, &b.id);
    assert_eq!(lane_of(&e.c, USER, &t.id), "open");
}

#[test]
fn it_comes_back_on_its_date_and_goes_out_as_if_just_posted() {
    let mut e = Env::new();
    let t = parked(&e, "Voice calls", tomorrow());
    let bid = t.parked().unwrap().id.clone();
    e.clk.add(hours(23));
    e.c.sweep_notices().unwrap();
    assert_eq!(e.c.get_task(&t.id).unwrap().state, BLOCKED, "not yet");

    e.clk.add(hours(1));
    let ns = e.c.sweep_notices().unwrap();
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.state, SUBMITTED);
    assert_eq!(got.blockers[0].resolved.as_ref().unwrap().by, "irori");
    assert_eq!(
        got.blockers[0].resolved.as_ref().unwrap().answer,
        "2026-10-27 came"
    );
    closes_per_item(&e, &t.id, &bid);
    // The notice says where the card is, not that more are open.
    assert_eq!(ns.len(), 1, "{ns:?}");
    assert!(
        ns[0].text.contains("It is submitted again."),
        "{}",
        ns[0].text
    );
    assert!(!ns[0].text.contains("more blockers"), "{}", ns[0].text);

    e.c = e.reopen();
    assert!(!e.c.sweep_asks(&agents()).unwrap().is_empty(), "goes out");
    assert!(e.c.get_task(&t.id).unwrap().live_offer().is_some());
}

#[derive(serde::Serialize)]
struct Started<'a> {
    headless: bool,
    name: &'a str,
    os: &'a str,
    version: &'a str,
}

#[test]
fn it_comes_back_when_its_version_runs() {
    let e = Env::new();
    let row = |v| Started {
        headless: false,
        name: "dev-a",
        os: "darwin",
        version: v,
    };
    e.st.append("irori", store::kinds::DEVICE_STARTED, &row("0.12.0"))
        .unwrap();
    let t = parked(
        &e,
        "After the update",
        Back::Version("v0.13.0 dev-a".into()),
    );
    let b = t.parked().unwrap().clone();
    assert_eq!(b.later, "version 0.13.0 dev-a");
    assert_eq!(b.on.as_ref().unwrap().kind, BLOCKED_ON_AGENT);
    e.c.sweep_notices().unwrap();
    assert_eq!(e.c.get_task(&t.id).unwrap().state, BLOCKED);

    e.st.append("irori", store::kinds::DEVICE_STARTED, &row("0.13.0"))
        .unwrap();
    e.c.sweep_notices().unwrap();
    assert_eq!(e.c.get_task(&t.id).unwrap().state, SUBMITTED);
    closes_per_item(&e, &t.id, &b.id);

    // Already running it: nothing to wait for, as a wait refuses.
    let u = make(&e, "another");
    want_err(
        e.c.park(ALICE, &u.id, "why", Back::Version("0.13.0 dev-a".into())),
        ErrorKind::Invalid,
    );
    assert!(e.c.get_task(&u.id).unwrap().parked().is_none());
}

#[test]
fn a_date_already_gone_is_refused() {
    let e = Env::new();
    let t = make(&e, "late");
    let err =
        e.c.park(ALICE, &t.id, "why", Back::Date(fixed_now()))
            .unwrap_err();
    assert!(err.is(ErrorKind::Invalid), "{err}");
    assert!(e.rows(kinds::TASK_STATUS).is_empty());
}

#[test]
fn only_the_persons_word_ends_a_word_hold() {
    let mut e = Env::new();
    let t = parked(&e, "Not now", Back::Word);
    let bid = t.parked().unwrap().id.clone();
    e.clk.add(hours(24 * 400));
    e.c.sweep_notices().unwrap();
    assert!(e.c.get_task(&t.id).unwrap().parked().is_some(), "no arm");

    want_err(e.c.bring_back(ALICE, &t.id, "go"), ErrorKind::NotAllowed);
    want_err(
        e.c.answer(
            ALICE,
            &t.id,
            &bid,
            BlockerAnswer {
                text: "done waiting".into(),
                resolve: true,
                ..BlockerAnswer::default()
            },
        ),
        ErrorKind::NotAllowed,
    );
    // An agent can't turn it into something that ends by itself.
    want_err(
        e.c.park(
            ALICE,
            &t.id,
            "soon",
            Back::Date(plus(e.clk.now(), hours(1))),
        ),
        ErrorKind::NotAllowed,
    );
    assert!(e.c.get_task(&t.id).unwrap().parked().is_some());

    let (got, _) = e.c.bring_back(USER, &t.id, "let's do it").unwrap();
    assert_eq!(got.state, SUBMITTED);
    let r = got.blockers[0].resolved.as_ref().unwrap();
    assert_eq!((r.answer.as_str(), r.by.as_str()), ("let's do it", USER));
    closes_per_item(&e, &t.id, &bid);
    e.c = e.reopen();
    assert_eq!(e.c.get_task(&t.id).unwrap().state, SUBMITTED);
}

#[test]
fn an_agent_may_bring_back_early_what_isnt_on_the_persons_word() {
    let e = Env::new();
    let t = parked(&e, "Voice calls", tomorrow());
    let (got, _) = e.c.bring_back(BOB, &t.id, "").unwrap();
    assert_eq!(got.state, SUBMITTED);
    want_err(e.c.bring_back(BOB, &t.id, ""), ErrorKind::BadState);
}

// ---- what ends a park, row by row ----

#[test]
fn board_status_refuses_agents_except_canceled_and_failed() {
    let e = Env::new();
    let t = parked(&e, "Not now", Back::Word);
    for s in [WORKING, SUBMITTED, COMPLETED, IN_REVIEW, INPUT_REQUIRED] {
        want_err(
            e.c.set_task_state(BOB, &t.id, s, "go"),
            ErrorKind::NotAllowed,
        );
    }
    assert!(e.c.get_task(&t.id).unwrap().parked().is_some());
    let u = parked(&e, "two", Back::Word);
    assert_eq!(
        e.c.set_task_state(BOB, &t.id, CANCELED, "no")
            .unwrap()
            .state,
        CANCELED
    );
    assert_eq!(
        e.c.set_task_state(BOB, &u.id, FAILED, "no").unwrap().state,
        FAILED
    );
    // The person's status is their word.
    let v = parked(&e, "three", Back::Word);
    let got = e.c.set_task_state(USER, &v.id, SUBMITTED, "go").unwrap();
    assert!(got.parked().is_none());
}

#[test]
fn unblock_with_no_item_refuses_agents() {
    let e = Env::new();
    let t = parked(&e, "Voice calls", tomorrow());
    want_err(e.c.unblock(ALICE, &t.id, "go"), ErrorKind::NotAllowed);
    assert!(e.c.get_task(&t.id).unwrap().parked().is_some());
    // A person's goes item by item, never by blocker.closed.
    let (got, _) = e.c.unblock(USER, &t.id, "go").unwrap();
    assert_eq!(got.state, SUBMITTED);
    assert!(e.rows(kinds::BLOCKER_CLOSED).is_empty());
    assert_eq!(e.rows(kinds::BLOCKER_RESOLVED).len(), 1);
}

#[test]
fn unblock_with_an_item_is_allowed_except_on_the_persons_word() {
    let e = Env::new();
    let resolve = |text: &str| BlockerAnswer {
        text: text.into(),
        resolve: true,
        ..BlockerAnswer::default()
    };
    let t = parked(&e, "Voice calls", tomorrow());
    let bid = t.parked().unwrap().id.clone();
    let (got, _) = e.c.answer(BOB, &t.id, &bid, resolve("sooner")).unwrap();
    assert_eq!(got.state, SUBMITTED);

    let u = parked(&e, "Not now", Back::Word);
    let bid = u.parked().unwrap().id.clone();
    want_err(
        e.c.answer(BOB, &u.id, &bid, resolve("sooner")),
        ErrorKind::NotAllowed,
    );
    let (got, _) = e.c.answer(USER, &u.id, &bid, resolve("now")).unwrap();
    assert_eq!(got.state, SUBMITTED);
}

#[test]
fn claim_and_assign_refuse_agents_and_a_persons_assign_is_their_word() {
    let e = Env::new();
    let t = parked(&e, "Not now", Back::Word);
    want_err(e.c.claim_task(BOB, &t.id), ErrorKind::NotAllowed);
    want_err(e.c.assign(BOB, &t.id, CAROL), ErrorKind::NotAllowed);
    want_err(e.c.assign_task(BOB, &t.id, CAROL), ErrorKind::NotAllowed);
    let got = e.c.get_task(&t.id).unwrap();
    assert!(got.parked().is_some() && got.holders.is_empty());

    let (got, _) = e.c.assign(USER, &t.id, CAROL).unwrap();
    assert!(got.parked().is_none(), "{got:?}");
    assert_eq!(got.state, SUBMITTED);
    assert_eq!(got.holders, [CAROL.to_string()]);
    assert!(e.rows(kinds::BLOCKER_CLOSED).is_empty());
}

#[test]
fn a_shared_cards_join_is_the_same() {
    let e = Env::new();
    let t = make(&e, "pair on it");
    e.c.set_shared(USER, &t.id, true).unwrap();
    e.c.park(ALICE, &t.id, "not now", Back::Word).unwrap();
    want_err(e.c.join(BOB, &t.id, BOB), ErrorKind::NotAllowed);
    let got = e.c.join(USER, &t.id, BOB).unwrap();
    assert!(got.parked().is_none());
    assert_eq!(got.holders, [BOB.to_string()]);
}

#[test]
fn another_items_answer_cant_unpark_it() {
    let e = Env::new();
    let t = parked(&e, "Not now", tomorrow());
    let (_, b) =
        e.c.raise(
            BOB,
            &t.id,
            NewBlocker {
                question: "Which one?".into(),
                ..NewBlocker::default()
            },
        )
        .unwrap();
    e.c.answer(
        USER,
        &t.id,
        &b.id,
        BlockerAnswer {
            text: "the first".into(),
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.state, BLOCKED);
    assert!(got.parked().is_some());
    assert!(!e.c.back_on_board(USER, &t.id, "go").unwrap());
    // The sweep's own way back is shut too.
    e.c.sweep_notices().unwrap();
    assert!(e.c.get_task(&t.id).unwrap().parked().is_some());
}

#[test]
fn answered_left_skips_a_hold() {
    let mut e = Env::new();
    let t = make(&e, "Not now");
    e.c.claim_task(ALICE, &t.id).unwrap();
    e.c.park(ALICE, &t.id, "not now", tomorrow()).unwrap();
    let got = e.c.get_task(&t.id).unwrap();
    assert!(got.left.contains(&ALICE.to_string()));
    let bid = got.parked().unwrap().id.clone();
    // The person replies on the hold's own thread after its agent left.
    e.c.answer(
        USER,
        &t.id,
        &bid,
        BlockerAnswer {
            text: "ok".into(),
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    assert_eq!(
        e.c.get_task(&t.id).unwrap().blockers[0].status,
        BLOCKER_ANSWERED
    );
    e.c.sweep_notices().unwrap();
    e.c = e.reopen();
    e.c.sweep_notices().unwrap();
    let got = e.c.get_task(&t.id).unwrap();
    assert!(got.parked().is_some(), "{got:?}");
    assert_eq!(got.state, BLOCKED);
}

#[test]
fn hand_on_is_refused() {
    let e = Env::new();
    let t = make(&e, "Not now");
    e.c.raise(
        ALICE,
        &t.id,
        NewBlocker {
            question: "Which one?".into(),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    // A question comes first, so this card's hold came in from elsewhere.
    let row = TaskStateRow {
        task_id: t.id.clone(),
        state: BLOCKED.into(),
        reason: "Not yet: later".into(),
        blocked_by: Some(Back::Word.on()),
        blocker: Some(BlockerRow {
            id: "b2".into(),
            question: "Not yet: later".into(),
            on: Some(Back::Word.on()),
            later: "word".into(),
            ..BlockerRow::default()
        }),
        by: ALICE.into(),
        at: e.clk.now(),
        ..TaskStateRow::default()
    };
    e.write_raw(ALICE, kinds::TASK_STATUS, &row);
    assert!(!e.c.hand_on_refusal(&t.id).is_empty());
    assert!(e.c.hand_on_refusal(&t.id).contains("Not yet"));
    e.ros.set(ALICE, "");
    e.clk.add(hours(1));
    want_err(e.c.hand_on(USER, &t.id), ErrorKind::BadState);
    assert!(e.c.get_task(&t.id).unwrap().parked().is_some());
}

// ---- what refuses a park ----

#[test]
fn a_persons_question_goes_first() {
    let e = Env::new();
    let t = make(&e, "with a question");
    e.c.raise(
        ALICE,
        &t.id,
        NewBlocker {
            question: "Which one?".into(),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    want_err(
        e.c.park(ALICE, &t.id, "why", Back::Word),
        ErrorKind::BadState,
    );
    assert_eq!(e.c.get_task(&t.id).unwrap().open_blockers().len(), 1);
}

#[test]
fn a_done_check_goes_first() {
    let mut e = Env::new();
    e.c = e.reopen_with(Options {
        done_check: true,
        ..e.opts()
    });
    let t = make(&e, "finished");
    e.c.claim_task(ALICE, &t.id).unwrap();
    e.c.set_task_state(ALICE, &t.id, COMPLETED, "it works")
        .unwrap();
    want_err(
        e.c.park(ALICE, &t.id, "why", Back::Word),
        ErrorKind::BadState,
    );
}

#[test]
fn an_ownership_change_goes_first() {
    let e = Env::new();
    let t = make(&e, "moving");
    let row = TaskStateRow {
        task_id: t.id.clone(),
        state: BLOCKED.into(),
        reason: "Take this card?".into(),
        blocked_by: Some(BlockedBy {
            kind: BLOCKED_ON_PERSON.into(),
            reference: String::new(),
        }),
        blocker: Some(BlockerRow {
            id: "b1".into(),
            question: "Take this card?".into(),
            transfer: true,
            on: Some(BlockedBy {
                kind: BLOCKED_ON_EXTERNAL.into(),
                reference: "the other person".into(),
            }),
            ..BlockerRow::default()
        }),
        by: ALICE.into(),
        at: e.clk.now(),
        ..TaskStateRow::default()
    };
    e.write_raw(ALICE, kinds::TASK_STATUS, &row);
    want_err(
        e.c.park(ALICE, &t.id, "why", Back::Word),
        ErrorKind::BadState,
    );
}

#[test]
fn a_merge_wait_goes_first() {
    let e = Env::new();
    let t = make(&e, "waits on a merge");
    e.c.block(
        ALICE,
        &t.id,
        "Waiting for the merge",
        Some(BlockedBy {
            kind: BLOCKED_ON_EXTERNAL.into(),
            reference: "https://github.com/a/b/pull/12".into(),
        }),
    )
    .unwrap();
    want_err(
        e.c.park(ALICE, &t.id, "why", Back::Word),
        ErrorKind::BadState,
    );
}

#[test]
fn input_required_goes_first() {
    let e = Env::new();
    let t = make(&e, "needs input");
    e.c.claim_task(ALICE, &t.id).unwrap();
    e.c.set_task_state(ALICE, &t.id, INPUT_REQUIRED, "which?")
        .unwrap();
    want_err(
        e.c.park(ALICE, &t.id, "why", Back::Word),
        ErrorKind::BadState,
    );
    assert_eq!(e.c.get_task(&t.id).unwrap().state, INPUT_REQUIRED);
}

#[test]
fn an_open_ask_and_a_finished_card_arent_held() {
    let e = Env::new();
    let t = make(&e, "done");
    e.c.set_task_state(USER, &t.id, COMPLETED, "done").unwrap();
    want_err(
        e.c.park(ALICE, &t.id, "why", Back::Word),
        ErrorKind::BadState,
    );
    let u = make(&e, "no reason given");
    want_err(e.c.park(ALICE, &u.id, "  ", Back::Word), ErrorKind::Invalid);
}

#[test]
fn a_wait_on_work_stays_beside_a_park() {
    let e = Env::new();
    let first = make(&e, "first");
    let t = make(&e, "second");
    e.c.add_after(ALICE, &t.id, std::slice::from_ref(&first.id))
        .unwrap();
    let (got, _) = e.c.park(ALICE, &t.id, "not now", tomorrow()).unwrap();
    assert_eq!(got.open_blockers().len(), 2);
    let (got, _) = e.c.bring_back(BOB, &t.id, "").unwrap();
    assert_eq!(got.state, BLOCKED, "still after #1");
    assert_eq!(task_waits(&got), vec![first.id.clone()]);
}

#[test]
fn one_park_per_card_changing_it_closes_the_old_item_and_raises_a_new_one() {
    let mut e = Env::new();
    let first = make(&e, "first");
    let t = parked(&e, "Voice calls", tomorrow());
    let bid = t.parked().unwrap().id.clone();
    let (got, _) =
        e.c.park(
            ALICE,
            &t.id,
            "after the first",
            Back::Card(first.id.clone()),
        )
        .unwrap();
    assert_eq!(got.open_blockers().len(), 1);
    let b = got.parked().unwrap();
    assert_ne!(b.id, bid);
    assert_eq!(b.later, "card");
    assert_eq!(b.on, Some(on_task(&first.id)));
    assert_eq!(b.question, "Not yet: after the first");

    // The old item is closed per item, and the new row is no replace.
    let closed: BlockerResolvedRow = payload(e.rows(kinds::BLOCKER_RESOLVED).last().unwrap());
    assert_eq!(closed.blocker_id, bid);
    let rows = e.rows(kinds::TASK_STATUS);
    let r: TaskStateRow = payload(rows.last().unwrap());
    let br = r.blocker.unwrap();
    assert!(!br.replace && !br.revise);
    assert_eq!(br.id, b.id);

    // The old date no longer brings it back.
    e.clk.add(hours(48));
    e.c.sweep_notices().unwrap();
    assert!(e.c.get_task(&t.id).unwrap().parked().is_some());
    e.c = e.reopen();
    assert_eq!(e.c.get_task(&t.id).unwrap().parked().unwrap().later, "card");
}

#[test]
fn a_pending_spawn_ends_on_park() {
    let mut e = Env::new();
    let t = make(&e, "starting");
    e.c = spawning(&e);
    let g = e.c.request_spawn("irori", ask(&t.id)).unwrap().unwrap();
    assert!(e.c.get_task(&t.id).unwrap().spawn_pending());
    let (got, _) = e.c.park(USER, &t.id, "not now", Back::Word).unwrap();
    assert!(!got.spawn_pending(), "{:?}", got.spawns);
    assert_eq!(got.spawns[0].op, SPAWN_CANCELED);
    assert!(!e.c.bind_spawn(&t.id, g, ALICE).unwrap(), "nothing to bind");
    assert!(e.c.get_task(&t.id).unwrap().holders.is_empty());
}

// ---- a held card's holders ----

#[test]
fn its_holder_parks_it_and_comes_off() {
    let e = Env::new();
    let t = make(&e, "held");
    e.c.claim_task(ALICE, &t.id).unwrap();
    let (got, ns) = e.c.park(ALICE, &t.id, "not now", tomorrow()).unwrap();
    assert_eq!(got.state, BLOCKED, "the blocker went first");
    assert!(got.holders.is_empty() && got.holder.is_empty(), "{got:?}");
    assert!(ns.is_empty(), "it knows: {ns:?}");
    let rows = e.st.list(&store::Filter {
        limit: store::MAX_LIST_LIMIT,
        ..store::Filter::default()
    });
    let order: Vec<String> = rows
        .unwrap()
        .iter()
        .map(|r| r.kind.clone())
        .filter(|k| k == kinds::TASK_STATUS || k == kinds::TASK_LEFT)
        .collect();
    assert_eq!(order, [kinds::TASK_STATUS, kinds::TASK_LEFT]);
    // No "nobody is on it" toast: it didn't go away.
    assert!(got.returned.is_none());
}

#[test]
fn a_held_card_is_the_holders_or_a_persons_to_park() {
    let e = Env::new();
    let t = make(&e, "held");
    e.c.claim_task(ALICE, &t.id).unwrap();
    want_err(
        e.c.park(BOB, &t.id, "not now", Back::Word),
        ErrorKind::NotAllowed,
    );
    assert_eq!(e.c.get_task(&t.id).unwrap().state, WORKING);

    let (got, ns) = e.c.park(USER, &t.id, "not now", tomorrow()).unwrap();
    assert!(got.holders.is_empty());
    assert_eq!(ns.len(), 1, "{ns:?}");
    assert_eq!(ns[0].kind, NOTICE_PARKED);
    assert_eq!(ns[0].to, [ALICE.to_string()]);
    assert!(ns[0].wake);
    assert!(ns[0].text.contains("You are off it"), "{}", ns[0].text);

    // It comes back unheld, not silently WORKING.
    e.clk.add(hours(24));
    e.c.sweep_notices().unwrap();
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.state, SUBMITTED);
    assert!(got.holders.is_empty());
}

#[test]
fn a_step_owner_comes_off_too() {
    let e = Env::new();
    let t = make(&e, "with steps");
    e.c.set_steps(
        USER,
        &t.id,
        vec![Step {
            text: "one".into(),
            ..Step::default()
        }],
    )
    .unwrap();
    let sid = e.c.get_task(&t.id).unwrap().steps[0].id.clone();
    e.c.set_step_owner(USER, &t.id, &sid, BOB).unwrap();
    assert_eq!(e.c.get_task(&t.id).unwrap().co_holders, [BOB.to_string()]);
    want_err(
        e.c.park(ALICE, &t.id, "not now", Back::Word),
        ErrorKind::NotAllowed,
    );
    let (got, ns) = e.c.park(USER, &t.id, "not now", Back::Word).unwrap();
    assert!(got.co_holders.is_empty(), "{got:?}");
    assert_eq!(ns[0].to, [BOB.to_string()]);
}

// ---- subtasks ----

fn family(e: &Env, n: usize) -> (Task, Vec<Task>) {
    let p = make(e, "parent");
    let subs = (0..n)
        .map(|i| NewSubtask {
            title: format!("part {i}"),
            ..NewSubtask::default()
        })
        .collect();
    let (p, ks) = e.c.split(ALICE, &p.id, subs, Vec::new()).unwrap();
    (p, ks)
}

#[test]
fn parking_a_parent_parks_its_open_subtasks() {
    let e = Env::new();
    let (p, ks) = family(&e, 2);
    e.c.set_task_state(USER, &ks[1].id, CANCELED, "no").unwrap();
    e.c.park(ALICE, &p.id, "not now", Back::Word).unwrap();
    let k = e.c.get_task(&ks[0].id).unwrap();
    let item = k.parked().expect("real rows hold it");
    assert_eq!(item.question, "Not yet: not now");
    assert_eq!(item.later, "word");
    assert!(e.c.get_task(&ks[1].id).unwrap().parked().is_none());
    assert!(e.c.sweep_asks(&agents()).unwrap().is_empty());
}

#[test]
fn a_held_subtask_stops_the_parent_being_parked() {
    let e = Env::new();
    let (p, ks) = family(&e, 2);
    e.c.claim_task(BOB, &ks[1].id).unwrap();
    want_err(
        e.c.park(USER, &p.id, "not now", Back::Word),
        ErrorKind::BadState,
    );
    assert!(e.c.get_task(&p.id).unwrap().parked().is_none());
    assert!(
        e.c.get_task(&ks[0].id).unwrap().parked().is_none(),
        "no half"
    );
}

#[test]
fn a_subtask_with_a_question_stops_the_parent_being_parked() {
    let e = Env::new();
    let (p, ks) = family(&e, 1);
    e.c.raise(
        ALICE,
        &ks[0].id,
        NewBlocker {
            question: "Which one?".into(),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    want_err(
        e.c.park(USER, &p.id, "not now", Back::Word),
        ErrorKind::BadState,
    );
    assert!(e.c.get_task(&p.id).unwrap().parked().is_none());
}

#[test]
fn new_and_adopted_subtasks_inherit_it() {
    let e = Env::new();
    let (p, _) = family(&e, 1);
    e.c.park(ALICE, &p.id, "not now", Back::Word).unwrap();

    let k =
        e.c.create_task(
            ALICE,
            NewTask {
                parent: p.id.clone(),
                ..nt("new part")
            },
        )
        .unwrap();
    assert_eq!(k.parked().unwrap().later, "word");

    let (_, made) =
        e.c.split(
            ALICE,
            &p.id,
            vec![NewSubtask {
                title: "split part".into(),
                ..NewSubtask::default()
            }],
            Vec::new(),
        )
        .unwrap();
    assert_eq!(made[0].parked().unwrap().later, "word");

    let loose = make(&e, "loose");
    e.c.split(ALICE, &p.id, Vec::new(), vec![loose.id.clone()])
        .unwrap();
    assert_eq!(
        e.c.get_task(&loose.id).unwrap().parked().unwrap().later,
        "word"
    );

    // A card someone is on can't join a held parent, and a new subtask
    // takes the parent's hold, not one of its own.
    let busy = make(&e, "busy");
    e.c.claim_task(BOB, &busy.id).unwrap();
    want_err(
        e.c.split(ALICE, &p.id, Vec::new(), vec![busy.id.clone()]),
        ErrorKind::BadState,
    );
    want_err(
        e.c.create_task(
            ALICE,
            NewTask {
                parent: p.id.clone(),
                later: Some(tomorrow()),
                ..nt("own hold")
            },
        ),
        ErrorKind::Invalid,
    );
}

#[test]
fn bringing_back_a_word_parent_closes_its_subtasks_items() {
    let e = Env::new();
    let (p, ks) = family(&e, 2);
    // This subtask's own hold isn't the parent's.
    e.c.park(ALICE, &ks[1].id, "its own reason", Back::Word)
        .unwrap();
    e.c.park(ALICE, &p.id, "not now", Back::Word).unwrap();
    assert_eq!(
        e.c.get_task(&ks[1].id).unwrap().parked().unwrap().question,
        "Not yet: its own reason"
    );
    want_err(e.c.bring_back(ALICE, &p.id, ""), ErrorKind::NotAllowed);
    e.c.bring_back(USER, &p.id, "go").unwrap();
    assert!(e.c.get_task(&p.id).unwrap().parked().is_none());
    let k = e.c.get_task(&ks[0].id).unwrap();
    assert!(k.parked().is_none());
    assert_eq!(k.state, SUBMITTED);
    assert!(e.c.get_task(&ks[1].id).unwrap().parked().is_some());
    assert!(e.rows(kinds::BLOCKER_CLOSED).is_empty());
}

#[test]
fn every_way_a_person_ends_a_word_parent_reaches_its_subtasks() {
    let e = Env::new();
    let (p, ks) = family(&e, 1);
    e.c.park(ALICE, &p.id, "not now", Back::Word).unwrap();
    e.c.unblock(USER, &p.id, "go").unwrap();
    assert!(e.c.get_task(&ks[0].id).unwrap().parked().is_none());

    let (p, ks) = family(&e, 1);
    let (got, _) = e.c.park(ALICE, &p.id, "not now", Back::Word).unwrap();
    let bid = got.parked().unwrap().id.clone();
    e.c.answer(
        USER,
        &p.id,
        &bid,
        BlockerAnswer {
            text: "go".into(),
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    assert!(e.c.get_task(&ks[0].id).unwrap().parked().is_none());
}

#[test]
fn changing_a_parents_hold_changes_what_its_subtasks_took() {
    let e = Env::new();
    let first = make(&e, "first");
    let (p, ks) = family(&e, 1);
    e.c.park(ALICE, &p.id, "not now", tomorrow()).unwrap();
    let was =
        e.c.get_task(&ks[0].id)
            .unwrap()
            .parked()
            .unwrap()
            .id
            .clone();
    e.c.park(ALICE, &p.id, "after it", Back::Card(first.id.clone()))
        .unwrap();
    let k = e.c.get_task(&ks[0].id).unwrap();
    assert_eq!(k.open_blockers().len(), 1);
    let b = k.parked().unwrap();
    assert_ne!(b.id, was);
    assert_eq!(b.later, "card");
    assert_eq!(b.on, Some(on_task(&first.id)));
}

#[test]
fn a_parent_held_until_its_own_subtask_leaves_that_subtask_free() {
    let e = Env::new();
    let (p, ks) = family(&e, 2);
    e.c.park(ALICE, &p.id, "after part 0", Back::Card(ks[0].id.clone()))
        .unwrap();
    assert!(e.c.get_task(&ks[0].id).unwrap().parked().is_none());
    assert!(e.c.get_task(&ks[1].id).unwrap().parked().is_some());
}

#[test]
fn a_parked_child_doesnt_set_the_parents_rollup_to_blocked() {
    let e = Env::new();
    let (p, ks) = family(&e, 2);
    e.c.park(ALICE, &ks[0].id, "not now", Back::Word).unwrap();
    let got = e.c.get_task(&p.id).unwrap();
    let r = got.rollup.as_ref().unwrap();
    assert_ne!(r.state, BLOCKED);
    assert!(
        r.blocked.is_empty() && r.blocked_question.is_empty(),
        "{r:?}"
    );
    assert_eq!(lane_of(&e.c, USER, &p.id), "open");

    // A question on a child still is one.
    e.c.raise(
        ALICE,
        &ks[1].id,
        NewBlocker {
            question: "Which one?".into(),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    let got = e.c.get_task(&p.id).unwrap();
    let r = got.rollup.as_ref().unwrap();
    assert_eq!(r.state, BLOCKED);
    assert_eq!(r.blocked, ks[1].id);
}

// ---- rows from elsewhere ----

/// An older device's revise of open item `bid`: the row has no `later`.
fn old_revise(e: &Env, id: &str, bid: &str, seq: i64) {
    let row = TaskStateRow {
        task_id: id.into(),
        state: BLOCKED.into(),
        reason: "Not yet: reworded".into(),
        blocked_by: Some(Back::Word.on()),
        blocker: Some(BlockerRow {
            id: bid.into(),
            question: "Not yet: reworded".into(),
            replace: true,
            revise: true,
            on: Some(Back::Word.on()),
            ..BlockerRow::default()
        }),
        by: ALICE.into(),
        at: e.clk.now(),
        ..TaskStateRow::default()
    };
    let at = hlc_at(e.clk.now()) + 1000 + seq as u64;
    e.remote("dev-old", seq, at, ALICE, kinds::TASK_STATUS, &row);
}

#[test]
fn an_old_row_without_later_revising_a_park_keeps_it() {
    let e = Env::new();
    let t = parked(&e, "Not now", Back::Word);
    let bid = t.parked().unwrap().id.clone();
    old_revise(&e, &t.id, &bid, 1);
    let got = e.c.get_task(&t.id).unwrap();
    let b = got.parked().expect("still held");
    assert_eq!(b.question, "Not yet: reworded");
    assert_eq!(b.later, "word");
    assert_eq!(
        e.reopen().get_task(&t.id).unwrap().parked().unwrap().later,
        "word"
    );
}

#[test]
fn this_builds_own_revise_keeps_it_too() {
    let e = Env::new();
    let t = parked(&e, "Voice calls", tomorrow());
    let bid = t.parked().unwrap().id.clone();
    let (got, _) =
        e.c.revise(
            ALICE,
            &t.id,
            &bid,
            BlockerRevision {
                question: "Not yet: not before December".into(),
                ..BlockerRevision::default()
            },
        )
        .unwrap();
    assert_eq!(got.parked().unwrap().back(), Some(tomorrow()));
}

fn odd_park(e: &Env, id: &str, later: &str, on: BlockedBy) {
    let row = TaskStateRow {
        task_id: id.into(),
        state: BLOCKED.into(),
        reason: "Not yet: from the future".into(),
        blocked_by: Some(on.clone()),
        blocker: Some(BlockerRow {
            id: "b1".into(),
            question: "Not yet: from the future".into(),
            on: Some(on),
            later: later.into(),
            ..BlockerRow::default()
        }),
        by: ALICE.into(),
        at: e.clk.now(),
        ..TaskStateRow::default()
    };
    let at = hlc_at(e.clk.now()) + 1000;
    e.remote("dev-new", 1, at, ALICE, kinds::TASK_STATUS, &row);
}

#[test]
fn an_unknown_later_value_stays_parked_as_the_persons_word() {
    let mut e = Env::new();
    let first = make(&e, "first");
    let t = make(&e, "from a newer build");
    odd_park(&e, &t.id, "moon full", on_task(&first.id));
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.parked().unwrap().back(), Some(Back::Word));
    assert_eq!(lane_of(&e.c, USER, &t.id), "later");

    // Not even the task arm closes it, though its `on` is a card.
    e.c.set_task_state(USER, &first.id, COMPLETED, "done")
        .unwrap();
    e.clk.add(hours(24 * 400));
    e.c.sweep_notices().unwrap();
    e.c = e.reopen();
    e.c.sweep_notices().unwrap();
    assert!(e.c.get_task(&t.id).unwrap().parked().is_some());
    assert!(e.rows(kinds::BLOCKER_RESOLVED).is_empty());

    want_err(e.c.bring_back(ALICE, &t.id, ""), ErrorKind::NotAllowed);
    assert_eq!(e.c.bring_back(USER, &t.id, "").unwrap().0.state, SUBMITTED);
}

#[test]
fn an_old_devices_status_row_still_folds() {
    // Refusals are at write time: the fold takes a row an old agent wrote,
    // as every old device does, so they can't part ways.
    let e = Env::new();
    let t = parked(&e, "Not now", Back::Word);
    let row = TaskStateRow {
        task_id: t.id.clone(),
        state: WORKING.into(),
        by: BOB.into(),
        at: e.clk.now(),
        ..TaskStateRow::default()
    };
    let at = hlc_at(e.clk.now()) + 1000;
    e.remote("dev-old", 1, at, BOB, kinds::TASK_STATUS, &row);
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.state, WORKING);
    assert!(got.parked().is_none());
}

// ---- made held ----

#[test]
fn a_card_can_be_made_held() {
    let e = Env::new();
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                block_reason: "after the rewrite".into(),
                later: Some(tomorrow()),
                ..nt("Voice calls")
            },
        )
        .unwrap();
    assert_eq!(t.state, BLOCKED);
    assert_eq!(t.open_blockers().len(), 1);
    assert_eq!(t.parked().unwrap().question, "Not yet: after the rewrite");
    assert_eq!(e.rows(kinds::TASK_STATUS).len(), 1);

    let u =
        e.c.create_task(
            ALICE,
            NewTask {
                later: Some(Back::Word),
                ..nt("Some idea")
            },
        )
        .unwrap();
    assert_eq!(u.parked().unwrap().question, "Not yet: Some idea");
    assert!(e.c.sweep_asks(&agents()).unwrap().is_empty());
}

#[test]
fn a_bad_later_on_a_new_card_writes_nothing() {
    let e = Env::new();
    let n = e.rows(kinds::TASK_CREATED).len();
    want_err(
        e.c.create_task(
            ALICE,
            NewTask {
                later: Some(Back::Date(fixed_now())),
                ..nt("late")
            },
        ),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.create_task(
            ALICE,
            NewTask {
                block_reason: "both".into(),
                blocked_by: Some(BlockedBy {
                    kind: BLOCKED_ON_EXTERNAL.into(),
                    reference: "the vendor".into(),
                }),
                later: Some(Back::Word),
                ..nt("both")
            },
        ),
        ErrorKind::Invalid,
    );
    assert_eq!(e.rows(kinds::TASK_CREATED).len(), n);
}

// ---- shared boards ----

#[test]
fn a_board_close_counts_only_from_a_device_whose_row_folds() {
    let (a, b, owners) = two_people();
    owners.lock().unwrap().clear();
    // p2's card, parked by p2's agent: nothing of it is p1's.
    let t = b.c.create_task(P2, nt("Voice calls")).unwrap();
    b.c.park(CARD2, &t.id, "not now", tomorrow()).unwrap();
    sync(&a, &b);
    assert!(a.c.get_task(&t.id).unwrap().parked().is_some());
    a.clk.add(hours(24));
    b.clk.add(hours(24));

    // p1's device doesn't write a row that would fold to nothing.
    a.c.sweep_notices().unwrap();
    let rows = |s: &Side| {
        s.st.list(&store::Filter {
            kinds: vec![kinds::BLOCKER_RESOLVED.into()],
            limit: store::MAX_LIST_LIMIT,
            ..store::Filter::default()
        })
        .unwrap()
        .len()
    };
    assert_eq!(rows(&a), 0);
    assert_eq!(a.c.ignored_rows(), 0);
    assert!(a.c.get_task(&t.id).unwrap().parked().is_some());

    b.c.sweep_notices().unwrap();
    assert_eq!(rows(&b), 1);
    sync(&a, &b);
    assert_eq!(a.c.ignored_rows(), 0, "it folded on the other side");
    for s in [&a, &b] {
        let got = s.c.get_task(&t.id).unwrap();
        assert!(got.parked().is_none());
        assert_eq!(got.state, SUBMITTED);
    }
}

#[test]
fn a_board_close_counts_from_the_creators_and_an_owners_device() {
    // p1 raised it, on p2's card: p2's device is the creator's.
    let (a, b, owners) = two_people();
    owners.lock().unwrap().clear();
    let t = b.c.create_task(P2, nt("creator closes")).unwrap();
    sync(&a, &b);
    a.c.park(CARD1, &t.id, "not now", tomorrow()).unwrap();
    sync(&a, &b);
    b.clk.add(hours(24));
    b.c.sweep_notices().unwrap();
    sync(&a, &b);
    assert_eq!(a.c.get_task(&t.id).unwrap().state, SUBMITTED);
    assert_eq!(a.c.ignored_rows(), 0);

    // p2's card, parked by p2's agent: p1 owns the board.
    let (a, b, _) = two_people();
    let t = b.c.create_task(P2, nt("owner closes")).unwrap();
    b.c.park(CARD2, &t.id, "not now", tomorrow()).unwrap();
    sync(&a, &b);
    a.clk.add(hours(24));
    a.c.sweep_notices().unwrap();
    sync(&a, &b);
    assert_eq!(b.c.get_task(&t.id).unwrap().state, SUBMITTED);
    assert_eq!(b.c.ignored_rows(), 0);
}

#[test]
fn word_on_a_board_means_any_member_person() {
    let (a, b, _) = two_people();
    let t = a.c.create_task(P1, nt("Not now")).unwrap();
    a.c.park(CARD1, &t.id, "not now", Back::Word).unwrap();
    sync(&a, &b);
    want_err(b.c.bring_back(CARD2, &t.id, ""), ErrorKind::NotAllowed);
    b.c.bring_back(P2, &t.id, "go").unwrap();
    sync(&a, &b);
    assert_eq!(a.c.get_task(&t.id).unwrap().state, SUBMITTED);
    assert_eq!(a.c.ignored_rows() + b.c.ignored_rows(), 0);
}

#[test]
fn a_version_park_keeps_the_boards_refusal() {
    let (a, _b, _) = two_people();
    let t = a.c.create_task(P1, nt("After the update")).unwrap();
    let err =
        a.c.park(CARD1, &t.id, "why", Back::Version("0.13.0 dev-a".into()))
            .unwrap_err();
    assert!(err.is(ErrorKind::Invalid), "{err}");
    assert!(err.to_string().contains("your own board"), "{err}");
    assert!(a.c.get_task(&t.id).unwrap().parked().is_none());
}

#[test]
fn a_person_takes_another_persons_agent_off_a_board_card() {
    let (a, b, _) = two_people();
    let t = a.c.create_task(P1, nt("Held")).unwrap();
    sync(&a, &b);
    b.c.claim_task(CARD2, &t.id).unwrap();
    sync(&a, &b);
    want_err(
        a.c.park(CARD1, &t.id, "not now", Back::Word),
        ErrorKind::NotAllowed,
    );
    a.c.park(P1, &t.id, "not now", Back::Word).unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let got = s.c.get_task(&t.id).unwrap();
        assert!(got.holders.is_empty(), "{got:?}");
        assert_eq!(got.state, BLOCKED);
        assert!(got.parked().is_some());
    }
}
