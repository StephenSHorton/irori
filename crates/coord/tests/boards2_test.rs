//! Shared boards stage 2 (v0.5 §5.3, §5.4, §9, §10): one board log held by
//! two made-up people, p1 on dev-a and p2 on dev-b, each with its own store
//! and coord. Rows move between them by vector, as the board sync does,
//! and both folds must agree.
#![allow(clippy::type_complexity)]

mod common;

use gocompat::time::GoTime;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use coord::roster::{AgentCard, Agents};
use coord::rows::TaskStateRow;
use coord::*;
use gocompat::json::RawJson;
use store::{Event, OpenOptions, Store, kinds};

const P1: &str = "person:p1";
const P2: &str = "person:p2";
const CARD1: &str = "agent:c1";
const CARD2: &str = "agent:c2";
/// p2's second agent.
const CARD3: &str = "agent:c3";

/// Who owned the board when: `(person, from hlc, until hlc)`.
type Owners = Arc<Mutex<Vec<(String, u64, u64)>>>;

struct Side {
    st: Arc<Store>,
    clk: Clk,
    c: Coord,
    // Last: fields drop in order, and Windows can't delete an open file.
    _dir: tempfile::TempDir,
}

fn key_person(k: &str) -> String {
    match k {
        P1 | CARD1 => "p1".into(),
        P2 | CARD2 | CARD3 => "p2".into(),
        _ => String::new(),
    }
}

fn device_person(d: &str) -> String {
    match d {
        "dev-a" | "dev-a2" => "p1".into(),
        "dev-b" | "dev-b2" => "p2".into(),
        _ => String::new(),
    }
}

/// side, with the person's done check (card #112) on or off.
fn side_with(dev: &str, person: &str, owners: &Owners, done_check: bool) -> Side {
    side_full(dev, person, owners, done_check, None)
}

/// side_with, with a roster of the agents live on this device.
fn side_full(
    dev: &str,
    person: &str,
    owners: &Owners,
    done_check: bool,
    ros: Option<Arc<TestRoster>>,
) -> Side {
    let clk = Clk(Arc::new(Mutex::new(fixed_now())));
    let c2 = clk.clone();
    let dir = tempfile::tempdir().unwrap();
    let hc = hlc::Clock::with_now(Box::new(move || c2.now().to_system()));
    let st = Arc::new(
        Store::open_with(
            dir.path().join("boards").join("b.db"),
            dev,
            Arc::new(hc),
            OpenOptions {
                board: true,
                ..OpenOptions::default()
            },
        )
        .unwrap(),
    );
    let me = person.to_string();
    let o2 = owners.clone();
    let o = Options {
        now: Some(clk.func()),
        board: true,
        done_check,
        person: person.into(),
        is_owner: Some(Arc::new(move |k: &str| k == me)),
        key_person: Some(Arc::new(key_person)),
        device_person: Some(Arc::new(device_person)),
        tag: Some(Arc::new(|| "mlg".to_string())),
        board_owner: Some(Arc::new(move |p: &str, at: Option<&RowPos>| {
            o2.lock().unwrap().iter().any(|(who, from, until)| {
                who == p && at.map_or(*until == u64::MAX, |at| *from < at.hlc && at.hlc < *until)
            })
        })),
        ..Options::default()
    };
    let c = Coord::new(st.clone(), ros.map(|r| r as Arc<dyn Agents>), o).unwrap();
    Side {
        _dir: dir,
        st,
        clk,
        c,
    }
}

fn raw(s: &str) -> RawJson {
    RawJson(s.as_bytes().to_vec())
}

/// p1 on dev-a and p2 on dev-b, with p1 the board's owner forever. dev-a
/// writes board.created and both people's agent cards travel to dev-b.
fn two_people() -> (Side, Side, Owners) {
    two_people_with(None, None)
}

/// two_people, with each device's roster of live agents.
fn two_people_with(
    ra: Option<Arc<TestRoster>>,
    rb: Option<Arc<TestRoster>>,
) -> (Side, Side, Owners) {
    let owners: Owners = Arc::new(Mutex::new(vec![("p1".into(), 0, u64::MAX)]));
    let a = side_full("dev-a", P1, &owners, false, ra);
    let b = side_full("dev-b", P2, &owners, false, rb);
    a.st.append(
        P1,
        kinds::BOARD_CREATED,
        &raw(r#"{"boardId":"b","creator":{"personId":"p1"},"name":"mlg","tag":"mlg"}"#),
    )
    .unwrap();
    a.st.append(
        P1,
        kinds::BOARD_MEMBER_ADDED,
        &raw(r#"{"personId":"p2","role":"member"}"#),
    )
    .unwrap();
    a.st.append(
        CARD1,
        kinds::AGENT_CARD,
        &raw(r#"{"cardId":"c1","personId":"p1","status":"active"}"#),
    )
    .unwrap();
    pull(&a, &b);
    for c in ["c2", "c3"] {
        b.st.append(
            &format!("agent:{c}"),
            kinds::AGENT_CARD,
            &raw(&format!(
                r#"{{"cardId":"{c}","personId":"p2","status":"active"}}"#
            )),
        )
        .unwrap();
    }
    sync(&a, &b);
    (a, b, owners)
}

/// Moves what each side lacks from the other, both ways, and folds it.
fn sync(a: &Side, b: &Side) {
    pull(a, b);
    pull(b, a);
}

/// Moves what `to` lacks from `from` and folds it on `to`.
fn pull(from: &Side, to: &Side) {
    let rows = from.st.missing(&to.st.vector().unwrap(), 1000).unwrap();
    let (_, r) = to.st.insert_batch(&rows);
    r.unwrap();
    to.c.fold_live(&rows);
}

fn nt(title: &str, repo: &str) -> NewTask {
    NewTask {
        title: title.into(),
        repo_key: repo.into(),
        ..NewTask::default()
    }
}

fn hours(h: u64) -> GoTime {
    plus(fixed_now(), Duration::from_secs(3600 * h))
}

fn cancel_row(task: &str, dev: &str, seq: i64, at: GoTime, actor: &str) -> Event {
    let r = TaskStateRow {
        task_id: task.into(),
        state: CANCELED.into(),
        note: "not ours".into(),
        by: actor.into(),
        at,
        ..TaskStateRow::default()
    };
    Event {
        device_id: dev.into(),
        seq,
        hlc: hlc::Timestamp(hlc_at(at)),
        actor: actor.into(),
        kind: kinds::TASK_STATUS.into(),
        payload: gocompat::json::marshal(&r).unwrap(),
        sig: Vec::new(),
    }
}

// ---- the cancel rule at the row's time (§5.3) -------------------------------

#[test]
fn cancel_reads_the_owners_at_the_rows_time() {
    let (a, b, owners) = two_people();
    let one = a.c.create_task(P1, nt("one", "")).unwrap();
    let two = a.c.create_task(P1, nt("two", "")).unwrap();
    sync(&a, &b);
    // p2 owned the board from hour 1 to hour 3, and owns it no longer.
    owners
        .lock()
        .unwrap()
        .push(("p2".into(), hlc_at(hours(1)), hlc_at(hours(3))));
    // Written on p2's other device, so both stores take them by insert.
    let in_window = cancel_row(&one.id, "dev-b2", 1, hours(2), P2);
    let after = cancel_row(&two.id, "dev-b2", 2, hours(4), P2);
    for s in [&a, &b] {
        s.clk.add(Duration::from_secs(3600 * 5));
        let before = s.c.ignored_rows();
        for ev in [&in_window, &after] {
            s.st.insert(ev).unwrap();
            s.c.fold_live(std::slice::from_ref(ev));
        }
        assert_eq!(
            s.c.get_task(&one.id).unwrap().state,
            CANCELED,
            "a cancel by an owner at its time counts after they stop owning"
        );
        assert_ne!(
            s.c.get_task(&two.id).unwrap().state,
            CANCELED,
            "a cancel stamped after its writer stopped owning"
        );
        assert_eq!(s.c.ignored_rows(), before + 1);
    }
    // A withdraw now reads today's owners: p2 is none.
    let three = a.c.create_task(P1, nt("three", "")).unwrap();
    sync(&a, &b);
    let e = b.c.withdraw(P2, &three.id, "").unwrap_err();
    assert!(e.is(ErrorKind::NotAllowed), "{e}");
}

// ---- mirrored leases (§5.4) ------------------------------------------------

const SRC: &str = "path:remote:github.com/a/b:src/**";
const SRC_MAIN: &str = "path:remote:github.com/a/b:src/main.rs";

/// A shared card in REPO_A on p1's side that both people's agents work:
/// c1 claims it, c2 joins it.
fn shared_card(a: &Side, b: &Side) -> Task {
    let t =
        a.c.create_task(
            P1,
            NewTask {
                shared: true,
                ..nt("shared work", REPO_A)
            },
        )
        .unwrap();
    a.c.claim_task(CARD1, &t.id).unwrap();
    sync(a, b);
    b.c.join(CARD2, &t.id, CARD2).unwrap();
    sync(a, b);
    t
}

fn lease_on(s: &Side, resource: &str) -> Option<Lease> {
    s.c.leases().into_iter().find(|l| l.resource == resource)
}

#[test]
fn a_lease_mirrors_only_through_a_shared_card_in_its_repo() {
    let (a, b, _) = two_people();
    let t = shared_card(&a, &b);
    let bare = a.c.create_task(P1, nt("no repo", "")).unwrap();
    a.c.claim_task(CARD1, &bare.id).unwrap();
    assert_eq!(a.c.mirror_tasks(CARD1, SRC, false), vec![t.id.clone()]);
    assert_eq!(b.c.mirror_tasks(CARD2, SRC, false), vec![t.id.clone()]);
    assert!(a.c.mirror_tasks(CARD1, "port:5173", false).is_empty());
    assert!(
        a.c.mirror_tasks(CARD1, "path:remote:github.com/x/y:src/**", false)
            .is_empty()
    );
    assert!(a.c.mirror_tasks("agent:c9", SRC, false).is_empty());
    assert_eq!(a.c.mirror_tasks("agent:c9", SRC, true), vec![t.id.clone()]);
    assert_eq!(
        a.c.mirror_acquire(CARD1, "port:5173", Duration::ZERO, false, false)
            .unwrap(),
        None,
        "a port lease isn't in any repo"
    );
    let l =
        a.c.mirror_acquire(CARD1, SRC, Duration::ZERO, false, false)
            .unwrap()
            .unwrap();
    assert_eq!(l.holder, CARD1);
    sync(&a, &b);
    assert_eq!(
        lease_on(&b, SRC).unwrap().holder,
        CARD1,
        "p2 sees p1's lease"
    );
    // It blocks p2's agent, and the refusal shows on both sides.
    let e =
        b.c.mirror_acquire(CARD2, SRC_MAIN, Duration::ZERO, false, false)
            .unwrap_err();
    assert!(e.is(ErrorKind::Held), "{e}");
    assert!(e.to_string().contains("release request"), "{e}");
    sync(&a, &b);
    for s in [&a, &b] {
        assert!(
            s.c.refusals()
                .iter()
                .any(|r| r.by == CARD2 && r.held_by == CARD1),
            "{:?}",
            s.c.refusals()
        );
    }
    // Its release mirrors too.
    assert!(a.c.mirror_release(CARD1, SRC).unwrap());
    assert!(!a.c.mirror_release(CARD1, SRC).unwrap(), "released twice");
    sync(&a, &b);
    assert!(lease_on(&b, SRC).is_none());
    // A card that ended mirrors nothing.
    a.c.set_task_state(CARD1, &t.id, COMPLETED, "done").unwrap();
    assert!(a.c.mirror_tasks(CARD1, SRC, false).is_empty());
}

#[test]
fn partitioned_lease_grabs_keep_the_lower_devices() {
    let (a, b, _) = two_people();
    shared_card(&a, &b);
    // Apart, both take overlapping leases.
    a.c.mirror_acquire(CARD1, SRC, Duration::ZERO, false, false)
        .unwrap()
        .unwrap();
    b.c.mirror_acquire(CARD2, SRC_MAIN, Duration::ZERO, false, false)
        .unwrap()
        .unwrap();
    // Not yet overlapping by name, so meet them on one resource too.
    a.c.mirror_acquire(CARD1, SRC_MAIN, Duration::ZERO, false, false)
        .unwrap()
        .unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let l = lease_on(s, SRC_MAIN).unwrap();
        assert_eq!(l.holder, CARD1, "dev-a < dev-b: p1's lease stands");
    }
    // p2's Core refuses its own losing lease and tells its agent.
    b.c.tick();
    sync(&a, &b);
    for s in [&a, &b] {
        assert!(
            s.c.refusals()
                .iter()
                .any(|r| r.by == CARD2 && r.held_by == CARD1 && r.conflict),
            "{:?}",
            s.c.refusals()
        );
    }
    a.c.tick();
    sync(&a, &b);
    assert!(
        !a.c.refusals().iter().any(|r| r.by == CARD1),
        "p1's Core refuses nothing of its winning lease"
    );
}

#[test]
fn nobody_steals_another_persons_lease() {
    let (a, b, _) = two_people();
    let t = shared_card(&a, &b);
    let l =
        a.c.mirror_acquire(CARD1, SRC, Duration::ZERO, false, false)
            .unwrap()
            .unwrap();
    sync(&a, &b);
    for e in [
        b.c.steal_lease(P2, SRC).unwrap_err(),
        b.c.acquire(CARD2, SRC_MAIN, Duration::ZERO, true)
            .unwrap_err(),
        b.c.release(P2, SRC).unwrap_err(),
    ] {
        assert!(e.is(ErrorKind::NotAllowed), "{e}");
        assert!(e.to_string().contains("release request"), "{e}");
    }
    // Forged rows from p2's devices: a steal and a release of p1's lease.
    let stolen = Event {
        device_id: "dev-b2".into(),
        seq: 1,
        hlc: hlc::Timestamp(hlc_at(plus(fixed_now(), mins(1)))),
        actor: P2.into(),
        kind: kinds::LEASE_STOLEN.into(),
        payload: gocompat::json::marshal(&coord::rows::LeaseStolenRow {
            resource: SRC.into(),
            holder: CARD1.into(),
            acquired_at: l.acquired_at,
            by: P2.into(),
            at: plus(fixed_now(), mins(1)),
        })
        .unwrap(),
        sig: Vec::new(),
    };
    let released = Event {
        device_id: "dev-b2".into(),
        seq: 2,
        hlc: hlc::Timestamp(hlc_at(plus(fixed_now(), mins(2)))),
        actor: CARD2.into(),
        kind: kinds::LEASE_RELEASED.into(),
        payload: gocompat::json::marshal(&coord::rows::LeaseReleasedRow {
            resource: SRC.into(),
            holder: CARD1.into(),
            reason: "released".into(),
            by: CARD2.into(),
            at: plus(fixed_now(), mins(2)),
        })
        .unwrap(),
        sig: Vec::new(),
    };
    for s in [&a, &b] {
        let before = s.c.ignored_rows();
        for ev in [&stolen, &released] {
            s.st.insert(ev).unwrap();
            s.c.fold_live(std::slice::from_ref(ev));
        }
        assert_eq!(
            lease_on(s, SRC).unwrap().holder,
            CARD1,
            "a forged row ended it"
        );
        assert_eq!(s.c.ignored_rows(), before + 2);
    }
    // p2 asks instead: a comment on the shared card, which p1's agent hears.
    let (cm, n) =
        b.c.request_release(P2, SRC, "I need src for the fix")
            .unwrap();
    assert_eq!(cm.task_id, t.id);
    assert!(cm.text.starts_with(RELEASE_REQUEST), "{}", cm.text);
    assert!(
        cm.text.contains(CARD1) && cm.text.contains(SRC),
        "{}",
        cm.text
    );
    assert!(n.to.contains(&CARD1.to_string()), "{n:?}");
    // Addressed to the holder, so the bridge carries it to that agent only.
    assert_eq!(cm.to, CARD1);
    sync(&a, &b);
    assert!(
        a.c.comments(&t.id)
            .unwrap()
            .iter()
            .any(|c| c.id == cm.id && c.to == CARD1),
        "the request reached p1's side"
    );
    let e = a.c.request_release(P1, SRC, "").unwrap_err();
    assert!(e.is(ErrorKind::Held), "one's own lease: {e}");
    // p1 steals its own agent's lease: that counts everywhere.
    a.c.steal_lease(P1, SRC).unwrap();
    sync(&a, &b);
    assert!(lease_on(&b, SRC).is_none());
}

// ---- asks on a board (§9) --------------------------------------------------

#[test]
fn a_board_ask_claimed_by_both_peoples_agents_has_one_winner() {
    let (a, b, _) = two_people();
    let (ask, _) =
        a.c.post_ask(
            P1,
            NewAsk {
                text: "Who can look at the flaky sync test?".into(),
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap();
    assert_eq!(ask.ask.as_ref().unwrap().owner, "dev-a");
    sync(&a, &b);
    // Both people's agents ask to take it while apart; p2's asks first.
    let r2 = b.c.ask_claim(CARD2, &ask.id, Duration::ZERO).unwrap();
    assert!(r2.pending && !r2.mine, "{r2:?}");
    // p2's devices never decide the take, and a take row they forge counts
    // nowhere.
    b.c.tick();
    assert!(b.c.get_task(&ask.id).unwrap().ask.unwrap().taker.is_empty());
    sync(&a, &b);
    let r1 = a.c.ask_claim(CARD1, &ask.id, Duration::ZERO).unwrap();
    assert!(!r1.mine || r1.taker == CARD1, "{r1:?}");
    a.c.tick();
    sync(&a, &b);
    let taker = a.c.get_task(&ask.id).unwrap().ask.unwrap().taker;
    assert_eq!(taker, CARD2, "the first request the asking device folded");
    assert_eq!(b.c.get_task(&ask.id).unwrap().ask.unwrap().taker, taker);
    for s in [&a, &b] {
        let rows =
            s.st.list(&store::Filter {
                kinds: vec![kinds::BOARD_ASK_STATE.into()],
                ..store::Filter::default()
            })
            .unwrap();
        assert!(!rows.is_empty() && rows.iter().all(|r| r.device_id == "dev-a"));
        assert!(
            s.st.list(&store::Filter {
                kinds: vec!["take.".into()],
                ..store::Filter::default()
            })
            .unwrap()
            .is_empty(),
            "a private take row on the board"
        );
    }
    // The loser can't close it; the winner closes it as a card on the board.
    let e =
        a.c.ask_close(
            CARD1,
            &ask.id,
            AskOutcome {
                kind: OUTCOME_NONE.into(),
                note: "mine".into(),
                ..AskOutcome::default()
            },
        )
        .unwrap_err();
    assert!(!e.to_string().is_empty());
    b.c.ask_close(
        CARD2,
        &ask.id,
        AskOutcome {
            kind: OUTCOME_CARD.into(),
            title: "Fix the flaky sync test".into(),
            note: "a card for the fix".into(),
            ..AskOutcome::default()
        },
    )
    .unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let t = s.c.get_task(&ask.id).unwrap();
        assert_eq!(t.title, "Fix the flaky sync test", "{:?}", t.ask);
        assert!(t.ask.as_ref().unwrap().outcome.is_some());
        assert_eq!(
            t.short_ref(),
            ask.short_ref(),
            "the outcome card is the ask, on the board"
        );
    }
}

#[test]
fn a_take_lock_row_from_another_device_counts_nowhere() {
    let (a, b, _) = two_people();
    let (ask, _) =
        a.c.post_ask(
            P1,
            NewAsk {
                text: "Anyone?".into(),
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap();
    sync(&a, &b);
    let forged = Event {
        device_id: "dev-b2".into(),
        seq: 1,
        hlc: hlc::Timestamp(hlc_at(plus(fixed_now(), mins(1)))),
        actor: "irori".into(),
        kind: kinds::BOARD_ASK_STATE.into(),
        payload: gocompat::json::marshal(&coord::rows::TakeStateRow {
            task_id: ask.id.clone(),
            taker: CARD2.into(),
            reason: TAKE_TAKEN.into(),
            at: plus(fixed_now(), mins(1)),
            ..coord::rows::TakeStateRow::default()
        })
        .unwrap(),
        sig: Vec::new(),
    };
    for s in [&a, &b] {
        let before = s.c.ignored_rows();
        s.st.insert(&forged).unwrap();
        s.c.fold_live(std::slice::from_ref(&forged));
        assert!(s.c.get_task(&ask.id).unwrap().ask.unwrap().taker.is_empty());
        assert_eq!(s.c.ignored_rows(), before + 1);
    }
    // A take row naming a card that isn't on the board is refused outright.
    let e =
        b.st.append(
            "irori",
            kinds::BOARD_ASK_STATE,
            &raw(r#"{"taskId":"01NOTONTHISBOARD0000000000","taker":"agent:c2"}"#),
        )
        .unwrap_err();
    assert!(e.to_string().contains("not a card on this board"), "{e}");
}

// ---- peer review requests (§10) --------------------------------------------

const PR: &str = "https://github.com/a/b/pull/7";

#[test]
fn a_review_request_reaches_the_other_persons_reviewer() {
    let (a, b, _) = two_people();
    let t = shared_card(&a, &b);
    a.c.add_link(CARD1, &t.id, PR).unwrap();
    sync(&a, &b);
    // Nobody dedicated yet, and one's own person's agents can't approve.
    let e = a.c.request_review(CARD1, &t.id, PR, "").unwrap_err();
    assert!(e.is(ErrorKind::NotFound), "{e}");
    let e = a.c.request_review(CARD1, &t.id, PR, CARD1).unwrap_err();
    assert!(e.is(ErrorKind::NotAllowed), "{e}");
    // Card #471: an agent no member owns is nobody else's either.
    let e =
        a.c.request_review(CARD1, &t.id, PR, "agent:nobody")
            .unwrap_err();
    assert!(e.is(ErrorKind::NotAllowed), "an unknown agent: {e}");
    let e =
        a.c.request_review(CARD1, &t.id, "https://github.com/a/b/pull/8", CARD2)
            .unwrap_err();
    assert!(e.is(ErrorKind::Invalid), "an unlinked PR: {e}");
    let e = a.c.request_review(P1, &t.id, PR, CARD2).unwrap_err();
    assert!(e.is(ErrorKind::Invalid), "a person asks no review: {e}");
    // p2 dedicates c2, for the card's repo only; only p2 may.
    for (by, who) in [(CARD2, CARD2), (P2, CARD1)] {
        let e = b.c.set_reviewer(by, who, REPO_A).unwrap_err();
        assert!(e.is(ErrorKind::NotAllowed), "{by} {who}: {e}");
    }
    b.c.set_reviewer(P2, CARD2, REPO_A).unwrap();
    sync(&a, &b);
    assert_eq!(a.c.reviewer("p2", REPO_A), CARD2);
    assert_eq!(a.c.reviewer("p2", REPO_B), "", "another repo has none");
    // Without to, the row names the dedicated reviewer (never to:"",
    // which the bridge can't deliver).
    let (r, n) = a.c.request_review(CARD1, &t.id, PR, "").unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!((r[0].from.as_str(), r[0].to.as_str()), (CARD1, CARD2));
    assert_eq!(n.kind, NOTICE_REVIEW);
    assert_eq!(n.to, vec![CARD2.to_string()]);
    assert!(n.text.contains(PR), "{}", n.text);
    sync(&a, &b);
    assert_eq!(b.c.reviews(&t.id), r);
    // Forged: p2's device asking as p1's agent, p1's device dedicating
    // p2's agent.
    let at = plus(fixed_now(), mins(5));
    let forged = [
        Event {
            device_id: "dev-b2".into(),
            seq: 1,
            hlc: hlc::Timestamp(hlc_at(at)),
            actor: CARD1.into(),
            kind: kinds::REVIEW_REQUESTED.into(),
            payload: gocompat::json::marshal(&coord::rows::ReviewRequestedRow {
                task_id: t.id.clone(),
                pr: PR.into(),
                from: CARD1.into(),
                to: CARD2.into(),
                at,
            })
            .unwrap(),
            sig: Vec::new(),
        },
        Event {
            device_id: "dev-a2".into(),
            seq: 1,
            hlc: hlc::Timestamp(hlc_at(at)),
            actor: P1.into(),
            kind: kinds::REVIEW_REVIEWER.into(),
            payload: gocompat::json::marshal(&coord::rows::ReviewReviewerRow {
                reviewer: CARD2.into(),
                repo_key: String::new(),
                by: P1.into(),
                at,
            })
            .unwrap(),
            sig: Vec::new(),
        },
    ];
    for s in [&a, &b] {
        let before = s.c.ignored_rows();
        for ev in &forged {
            s.st.insert(ev).unwrap();
            s.c.fold_live(std::slice::from_ref(ev));
        }
        assert_eq!(s.c.ignored_rows(), before + 2);
        let aside = s.c.set_aside();
        assert_eq!(aside.ignored, s.c.ignored_rows());
        assert_eq!(aside.ignored_by.get("dev-b2"), Some(&1), "{aside:?}");
        assert_eq!(aside.ignored_by.get("dev-a2"), Some(&1), "{aside:?}");
        assert_eq!(s.c.reviews(&t.id).len(), 1);
        assert_eq!(s.c.reviewer("p1", ""), "");
    }
    // Asked by name, the request goes to that agent alone.
    let (_, n) = b.c.request_review(CARD2, &t.id, PR, CARD1).unwrap();
    assert_eq!(n.to, vec![CARD1.to_string()]);
}

#[test]
fn a_steal_on_the_board_takes_only_ones_own_persons_lease() {
    let (a, b, _) = two_people();
    let t = shared_card(&a, &b);
    a.c.mirror_acquire(CARD1, SRC, Duration::ZERO, false, false)
        .unwrap()
        .unwrap();
    sync(&a, &b);
    let e =
        b.c.mirror_acquire(CARD2, SRC_MAIN, Duration::ZERO, false, true)
            .unwrap_err();
    assert!(e.is(ErrorKind::NotAllowed), "another person's: {e}");
    assert_eq!(lease_on(&a, SRC).unwrap().holder, CARD1);
    // p1 releases, p2's c2 takes it, and p2's own c3 steals from it.
    assert!(a.c.mirror_release(CARD1, SRC).unwrap());
    sync(&a, &b);
    b.c.mirror_acquire(CARD2, SRC, Duration::ZERO, false, false)
        .unwrap()
        .unwrap();
    b.c.join(CARD3, &t.id, CARD3).unwrap();
    let l =
        b.c.mirror_acquire(CARD3, SRC_MAIN, Duration::ZERO, false, true)
            .unwrap()
            .unwrap();
    assert_eq!(l.stolen_from, CARD2, "one's own person's lease");
    sync(&a, &b);
    for s in [&a, &b] {
        assert!(lease_on(s, SRC).is_none(), "the stolen lease ended");
        assert_eq!(lease_on(s, SRC_MAIN).unwrap().holder, CARD3);
    }
}

/// #61: "irori" names no person, so a board counts its task.created for
/// nothing. Creating a card as Irori on a board is refused, not a panic,
/// and writes nothing; a person's card still goes through after it.
#[test]
fn irori_creating_a_board_card_is_refused() {
    let (a, b, _) = two_people();
    let before = a.c.list_tasks(&TaskFilter::default()).len();
    let err = a.c.create_task("irori", nt("by irori", "")).unwrap_err();
    assert!(err.to_string().contains("Irori itself"), "{err}");
    sync(&a, &b);
    assert_eq!(a.c.list_tasks(&TaskFilter::default()).len(), before);
    assert_eq!(b.c.list_tasks(&TaskFilter::default()).len(), before);
    a.c.create_task(P1, nt("by a person", "")).unwrap();
}

/// Card #112 on a shared board: an agent's completed waits on its
/// person's Done, and that Done (resolve, then completed) closes the card.
#[test]
fn a_done_check_closes_on_a_shared_board() {
    let owners: Owners = Arc::new(Mutex::new(vec![("p1".into(), 0, u64::MAX)]));
    let a = side_with("dev-a", P1, &owners, true);
    a.st.append(
        P1,
        kinds::BOARD_CREATED,
        &raw(r#"{"boardId":"b","creator":{"personId":"p1"},"name":"mlg","tag":"mlg"}"#),
    )
    .unwrap();
    a.st.append(
        CARD1,
        kinds::AGENT_CARD,
        &raw(r#"{"cardId":"c1","personId":"p1","status":"active"}"#),
    )
    .unwrap();
    let a = side_reopen(a, P1, &owners);
    let t = a.c.create_task(P1, nt("ship", "")).unwrap();
    a.c.claim_task(CARD1, &t.id).unwrap();
    let got =
        a.c.set_task_state(CARD1, &t.id, COMPLETED, "shipped")
            .unwrap();
    assert_eq!(got.state, BLOCKED);
    let b = got
        .blockers
        .iter()
        .find(|b| b.open() && b.done_check)
        .expect("a done check");
    let bid = b.id.clone();
    a.c.answer(
        P1,
        &t.id,
        &bid,
        BlockerAnswer {
            pick: 1,
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    let closed = a.c.set_task_state(P1, &t.id, COMPLETED, "").unwrap();
    assert_eq!(closed.state, COMPLETED);
}

/// A new coord over side s's store with the done check on.
fn side_reopen(s: Side, person: &str, owners: &Owners) -> Side {
    side_reopen_with(s, person, owners, Options::default())
}

/// side_reopen, taking anything else from `more` (its spawning switch).
fn side_reopen_with(s: Side, person: &str, owners: &Owners, more: Options) -> Side {
    let me = person.to_string();
    let o2 = owners.clone();
    let o = Options {
        now: Some(s.clk.func()),
        board: true,
        done_check: true,
        person: person.into(),
        is_owner: Some(Arc::new(move |k: &str| k == me)),
        key_person: Some(Arc::new(key_person)),
        device_person: Some(Arc::new(device_person)),
        tag: Some(Arc::new(|| "mlg".to_string())),
        board_owner: Some(Arc::new(move |p: &str, at: Option<&RowPos>| {
            o2.lock().unwrap().iter().any(|(who, from, until)| {
                who == p && at.map_or(*until == u64::MAX, |at| *from < at.hlc && at.hlc < *until)
            })
        })),
        ..more
    };
    let c = Coord::new(s.st.clone(), None, o).unwrap();
    Side { c, ..s }
}

/// The board's roster as each side sees it: c1 is p1's, c2 and c3 are
/// p2's. `idle` are idle (the earlier in the list idle longest); the rest
/// are busy.
fn roster(idle: &[&str]) -> Vec<coord::roster::AgentCard> {
    [CARD1, CARD2, CARD3]
        .iter()
        .map(|k| coord::roster::AgentCard {
            key: (*k).into(),
            status: if idle.contains(k) { "idle" } else { "busy" }.into(),
            updated_at: hours(idle.iter().position(|i| i == k).unwrap_or(9) as u64),
            ..Default::default()
        })
        .collect()
}

/// p1 on dev-a and p2 on dev-b on one shared board.
fn two_people_shared() -> (Side, Side) {
    let owners: Owners = Arc::new(Mutex::new(vec![("p1".into(), 0, u64::MAX)]));
    let a = side_with("dev-a", P1, &owners, false);
    let b = side_with("dev-b", P2, &owners, false);
    a.st.append(
        P1,
        kinds::BOARD_CREATED,
        &raw(r#"{"boardId":"b","creator":{"personId":"p1"},"name":"mlg","tag":"mlg"}"#),
    )
    .unwrap();
    a.st.append(
        P1,
        kinds::BOARD_MEMBER_ADDED,
        &raw(r#"{"personId":"p2","role":"member"}"#),
    )
    .unwrap();
    a.st.append(
        CARD1,
        kinds::AGENT_CARD,
        &raw(r#"{"cardId":"c1","personId":"p1","status":"active"}"#),
    )
    .unwrap();
    pull(&a, &b);
    for c in ["c2", "c3"] {
        b.st.append(
            &format!("agent:{c}"),
            kinds::AGENT_CARD,
            &raw(&format!(
                r#"{{"cardId":"{c}","personId":"p2","status":"active"}}"#
            )),
        )
        .unwrap();
    }
    sync(&a, &b);
    (a, b)
}

/// Card #174, the author's agents (the default): p1's open card goes only
/// to p1's free agent, and only p1's device hands it out.
#[test]
fn open_cards_go_to_the_authors_agents() {
    let (a, b) = two_people_shared();
    let t = a.c.create_task(P1, nt("Fix the loader", "")).unwrap();
    sync(&a, &b);
    // p2's device runs no hand-out for p1's card.
    let (ns, r) = b.c.sweep_asks_partial(&roster(&[CARD2, CARD3, CARD1]));
    r.unwrap();
    assert!(ns.is_empty(), "{ns:?}");
    // p1's agent is busy: p2's free agents don't get it; it waits.
    let (ns, r) = a.c.sweep_asks_partial(&roster(&[CARD2, CARD3]));
    r.unwrap();
    assert!(ns.is_empty(), "{ns:?}");
    // p1's agent goes idle: it gets it.
    let (ns, r) = a.c.sweep_asks_partial(&roster(&[CARD2, CARD1]));
    r.unwrap();
    assert!(ns.len() == 1 && ns[0].to == [CARD1], "{ns:?}");
    sync(&a, &b);
    for s in [&a, &b] {
        let o = s.c.get_task(&t.id).unwrap().live_offer().cloned().unwrap();
        assert_eq!(o.to, CARD1);
    }
}

/// Card #174: a hand-out row only counts from the device that made the
/// card, so another device can't point it at its own agent.
#[test]
fn a_hand_out_from_another_device_counts_nowhere() {
    let (a, b) = two_people_shared();
    let t = a.c.create_task(P1, nt("Fix the loader", "")).unwrap();
    sync(&a, &b);
    // From Irori and from one of p2's agents, on p2's device.
    for actor in ["irori", CARD3] {
        let ev =
            b.st.append(
                actor,
                kinds::TASK_OFFERED,
                &raw(&format!(
                    r#"{{"taskId":"{}","to":"{CARD3}","reason":"handed","heard":["{CARD3}"]}}"#,
                    t.id
                )),
            )
            .unwrap();
        b.c.fold_live(&[ev]);
    }
    sync(&a, &b);
    for s in [&a, &b] {
        assert!(s.c.get_task(&t.id).unwrap().live_offer().is_none());
    }
}

fn idle(key: &str, dev: &str) -> coord::roster::AgentCard {
    coord::roster::AgentCard {
        key: key.into(),
        device_id: dev.into(),
        status: "idle".into(),
        ..Default::default()
    }
}

/// A thread on a shared board (card #134): the agents on the board's card
/// it names hear it, woken; another member's comment is a person's, not an
/// answer; nobody waits on the named agents (card #512), so at once one
/// idle agent of any member gets it, or it is unclaimed; the
/// person's follow-up goes to the agent that answered, on both sides; only
/// the member who started it resolves it.
#[test]
fn a_board_thread_among_two_people() {
    let (a, b, _) = two_people();
    let card = shared_card(&a, &b);
    let (tk, n) =
        a.c.post_ask(
            P1,
            NewAsk {
                text: format!("Is #{} still moving?", card.short),
                thread: true,
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let ask = s.c.get_task(&tk.id).unwrap().ask.unwrap();
        assert!(ask.thread && ask.taker.is_empty(), "{ask:?}");
        assert_eq!(ask.heard, [CARD1, CARD2], "both holders of the named card");
    }
    assert!(
        n.to == [CARD1, CARD2] && n.text.contains("started a thread"),
        "{n:?}"
    );
    assert_eq!(b.c.thread_to(&tk.id), [CARD1, CARD2]);

    // p2 (a person) comments: not an answer, so it is still unanswered.
    // Nobody waits on the named agents (card #512): the first sweep finds
    // no agent free, so it is unclaimed (p1's device starts its agent when
    // spawning is on).
    b.c.person_comment(P2, &tk.id, "I'd like to know too.", false, &[], None)
        .unwrap();
    sync(&a, &b);
    a.c.sweep_asks(&[]).unwrap();
    let ask = a.c.get_task(&tk.id).unwrap().ask.unwrap();
    assert!(
        ask.reason == "unclaimed" && ask.unclaimed && ask.taker.is_empty(),
        "{ask:?}"
    );

    // One idle agent of either person gets it, at once; here p2's card3.
    let ns = a.c.sweep_asks(&[idle(CARD3, "dev-b")]).unwrap();
    assert!(
        ns.len() == 1 && ns[0].to == [CARD3] && ns[0].text.contains("handed it to you"),
        "{ns:?}"
    );
    sync(&a, &b);
    assert_eq!(b.c.get_task(&tk.id).unwrap().ask.unwrap().taker, CARD3);
    // p2's device never writes the thread's take rows.
    b.c.sweep_asks(&[idle(CARD3, "dev-b")]).unwrap();

    // card3 answers on p2's device: answered on both, nobody holds it.
    b.c.add_comment(CARD3, &tk.id, "Yes, merged an hour ago.", false)
        .unwrap();
    sync(&a, &b);
    a.c.sweep_asks(&[]).unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let t = s.c.get_task(&tk.id).unwrap();
        let ask = t.ask.unwrap();
        assert!(ask.reason == "answered" && ask.taker.is_empty(), "{ask:?}");
        assert!(t.holders.is_empty());
        assert_eq!(s.c.thread_to(&tk.id), [CARD3], "the follow-up's audience");
    }

    // p1's follow-up from dev-a reaches card3 (p2's agent hears it through
    // its own device's bridge, from thread_to).
    let (_, ns) =
        a.c.person_comment(
            P1,
            &tk.id,
            "Thanks, and the docs?",
            true,
            &[],
            Some(&|_| false),
        )
        .unwrap();
    assert!(ns.len() == 1 && ns[0].to == [CARD3] && ns[0].wake, "{ns:?}");

    // Agents and the other member don't resolve it; p1 does.
    let close = |s: &Side, by: &str| {
        s.c.ask_close(
            by,
            &tk.id,
            AskOutcome {
                kind: OUTCOME_ANSWERED.into(),
                note: "done".into(),
                ..AskOutcome::default()
            },
        )
    };
    assert!(close(&b, CARD3).is_err());
    assert!(
        b.c.resolve_thread(P2, &tk.id).is_err(),
        "p2 didn't start it"
    );
    assert!(close(&b, P2).is_err(), "nor ask_close it");
    let t = a.c.resolve_thread(P1, &tk.id).unwrap();
    assert!(is_terminal(&t.state), "{t:?}");
    sync(&a, &b);
    assert!(is_terminal(&b.c.get_task(&tk.id).unwrap().state));

    // Reopening is the starter's too (card #169); it's live on both sides.
    assert!(b.c.reopen_thread(P2, &tk.id).is_err(), "p2 didn't start it");
    a.c.reopen_thread(P1, &tk.id).unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let t = s.c.get_task(&tk.id).unwrap();
        assert!(t.open_ask().is_some_and(|a| a.thread), "{t:?}");
        assert_eq!(s.c.thread_to(&tk.id), [CARD3]);
    }
}

// ---- an open agent of another person reviews (card #191) -------------------

struct FixedRoster(Vec<AgentCard>);

impl Agents for FixedRoster {
    fn agents(&self) -> Vec<AgentCard> {
        self.0.clone()
    }
}

/// Side s reopened with a roster of the board's agents.
fn side_with_roster(s: Side, person: &str, owners: &Owners, agents: Vec<AgentCard>) -> Side {
    let me = person.to_string();
    let o2 = owners.clone();
    let o = Options {
        now: Some(s.clk.func()),
        board: true,
        person: person.into(),
        is_owner: Some(Arc::new(move |k: &str| k == me)),
        key_person: Some(Arc::new(key_person)),
        device_person: Some(Arc::new(device_person)),
        tag: Some(Arc::new(|| "mlg".to_string())),
        board_owner: Some(Arc::new(move |p: &str, at: Option<&RowPos>| {
            o2.lock().unwrap().iter().any(|(who, from, until)| {
                who == p && at.map_or(*until == u64::MAX, |at| *from < at.hlc && at.hlc < *until)
            })
        })),
        ..Options::default()
    };
    let r: Arc<dyn Agents> = Arc::new(FixedRoster(agents));
    let c = Coord::new(s.st.clone(), Some(r), o).unwrap();
    Side { c, ..s }
}

#[test]
fn with_nobody_dedicated_a_free_agent_of_another_person_reviews() {
    let (a, b, owners) = two_people();
    let t = shared_card(&a, &b);
    a.c.add_link(CARD1, &t.id, PR).unwrap();
    sync(&a, &b);
    // c1 is p1's own (idle, never chosen); c2 is p2's and busy; c3 is p2's
    // and idle: the request goes to c3.
    let a = side_with_roster(a, P1, &owners, roster(&[CARD1, CARD3]));
    let (r, n) = a.c.request_review(CARD1, &t.id, PR, "").unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!((r[0].from.as_str(), r[0].to.as_str()), (CARD1, CARD3));
    assert_eq!(n.to, vec![CARD3.to_string()]);
    // The holder's request put the card in review.
    assert_eq!(a.c.get_task(&t.id).unwrap().state, IN_REVIEW);

    // Nobody of p2's free: refused, as before.
    let t2 = shared_card(&a, &b);
    a.c.add_link(CARD1, &t2.id, PR).unwrap();
    let a = side_with_roster(a, P1, &owners, roster(&[CARD1]));
    let e = a.c.request_review(CARD1, &t2.id, PR, "").unwrap_err();
    assert!(e.is(ErrorKind::NotFound), "{e}");
}

// ---- a card's PR state on a board (#208, #210) ------------------------------

/// Card #210: the PR poller's pr.state row (actor irori) folds on a board
/// when a device of the holder's person writes it, so a merge wakes the
/// holder once, not at every poll. Any other device's row counts nowhere.
#[test]
fn a_merged_pr_on_a_board_card_tells_the_holder_once() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let (a, b, _) = two_people();
    let t = a.c.create_task(P1, nt("ship it", "")).unwrap();
    sync(&a, &b);
    b.c.claim_task(CARD2, &t.id).unwrap();
    b.c.add_link(CARD2, &t.id, PR).unwrap();
    sync(&a, &b);
    let merged = || PrCheck {
        pr: PR.into(),
        state: PR_MERGED.into(),
        ..PrCheck::default()
    };

    // p1's device doesn't hold the card: its row counts nowhere, and
    // nobody is told.
    let before = a.c.ignored_rows();
    let err = a.c.record_pr_state(&t.id, merged()).unwrap_err();
    assert!(err.to_string().contains("wasn't taken"), "{err}");
    assert_eq!(a.c.ignored_rows(), before + 1);
    sync(&a, &b);
    assert!(b.c.pr_states(&t.id).is_empty());

    // The holder's device: told once, and both sides know it merged.
    b.clk.add(Duration::from_secs(60));
    let (_, ns) = b.c.record_pr_state(&t.id, merged()).unwrap();
    assert_eq!(ns.len(), 1);
    assert!(ns[0].text.contains("was merged"), "{}", ns[0].text);
    sync(&a, &b);
    for s in [&a, &b] {
        let st = s.c.pr_states(&t.id);
        assert_eq!((st.len(), st[0].state.as_str()), (1, PR_MERGED));
    }

    // Heard again (the card still open): nothing new.
    b.clk.add(Duration::from_secs(180));
    let (_, ns) = b.c.record_pr_state(&t.id, merged()).unwrap();
    assert!(ns.is_empty(), "a merged PR is told once");
}

/// Card #509: on a board, an agent's external wait on a PR is its own
/// person's merge, not every member's. Their device's poller hears the
/// PR even after the agent left the card, and its row folds everywhere.
#[test]
fn a_merge_wait_on_a_board_is_its_raisers_persons() {
    const PR: &str = "https://github.com/a/b/pull/41";
    let (a, b, _) = two_people();
    let t = a.c.create_task(P1, nt("part 10", "")).unwrap();
    sync(&a, &b);
    b.c.claim_task(CARD2, &t.id).unwrap();
    b.c.block(
        CARD2,
        &t.id,
        "Review and merge PR #41",
        Some(BlockedBy {
            kind: BLOCKED_ON_EXTERNAL.into(),
            reference: PR.into(),
        }),
    )
    .unwrap();
    b.c.leave(CARD2, &t.id, CARD2).unwrap();
    sync(&a, &b);
    let needs = |s: &Side, viewer: &str| {
        s.c.board(&BoardFilter {
            viewer: viewer.into(),
            ..BoardFilter::default()
        })
        .needs_you
        .iter()
        .any(|c| c.task.id == t.id)
    };
    assert!(!needs(&a, P1), "p2's agent's wait isn't p1's merge");
    assert!(a.c.merge_waits(&t.id).is_empty());
    assert!(needs(&b, P2), "p2's merge");
    assert_eq!(b.c.merge_waits(&t.id), vec![PR.to_string()]);

    // p2's device hears it merged, though nobody holds the card: it folds
    // on both sides and the wait no longer needs p2.
    let merged = PrCheck {
        pr: PR.into(),
        state: PR_MERGED.into(),
        ..PrCheck::default()
    };
    assert!(a.c.record_pr_state(&t.id, merged.clone()).is_err());
    b.c.record_pr_state(&t.id, merged).unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        assert_eq!(s.c.pr_state(&t.id, PR).unwrap().state, PR_MERGED);
    }
    assert!(!needs(&b, P2));
}

/// #230: a board card in review links a merged PR as context and its own
/// PR, still open. The context PR's merge is told once, without a wake
/// and without asking for completed; heard again it says nothing; the own
/// PR's merge then asks for completed.
#[test]
fn a_merged_context_pr_beside_an_open_own_pr_is_told_once_without_a_wake() {
    const CONTEXT: &str = "https://github.com/a/b/pull/271";
    const OWN: &str = "https://github.com/a/b/pull/273";
    let (a, b, _) = two_people();
    let t = a.c.create_task(P1, nt("aimed shots", "")).unwrap();
    sync(&a, &b);
    b.c.claim_task(CARD2, &t.id).unwrap();
    b.c.add_link(CARD2, &t.id, CONTEXT).unwrap();
    b.c.add_link(CARD2, &t.id, OWN).unwrap();
    sync(&a, &b);
    let heard = |pr: &str, state: &str| PrCheck {
        pr: pr.into(),
        state: state.into(),
        ..PrCheck::default()
    };

    b.clk.add(Duration::from_secs(60));
    let (_, ns) = b.c.record_pr_state(&t.id, heard(OWN, PR_OPEN)).unwrap();
    assert!(ns.is_empty());
    b.c.set_task_state(CARD2, &t.id, IN_REVIEW, "PR open")
        .unwrap();

    b.clk.add(Duration::from_secs(60));
    let (_, ns) =
        b.c.record_pr_state(&t.id, heard(CONTEXT, PR_MERGED))
            .unwrap();
    assert_eq!(ns.len(), 1);
    assert!(!ns[0].wake, "a context PR's merge wakes nobody");
    assert!(ns[0].text.contains("was merged"), "{}", ns[0].text);
    assert!(ns[0].text.contains(OWN), "{}", ns[0].text);
    assert!(!ns[0].text.contains("state=completed"), "{}", ns[0].text);
    sync(&a, &b);

    // Every later poll: the row stuck, so nothing new on either side.
    for _ in 0..5 {
        b.clk.add(Duration::from_secs(180));
        let (_, ns) =
            b.c.record_pr_state(&t.id, heard(CONTEXT, PR_MERGED))
                .unwrap();
        assert!(ns.is_empty(), "told once");
        let (_, ns) = b.c.record_pr_state(&t.id, heard(OWN, PR_OPEN)).unwrap();
        assert!(ns.is_empty());
        sync(&a, &b);
    }
    for s in [&a, &b] {
        let st = s.c.pr_states(&t.id);
        assert!(st.iter().any(|p| p.pr == CONTEXT && p.state == PR_MERGED));
    }

    // The card's own PR merges: now the holder is woken to complete.
    b.clk.add(Duration::from_secs(180));
    let (_, ns) = b.c.record_pr_state(&t.id, heard(OWN, PR_MERGED)).unwrap();
    assert_eq!(ns.len(), 1);
    assert!(ns[0].wake);
    assert!(ns[0].text.contains("state=completed"), "{}", ns[0].text);
}

// ---- a gone holder on a board (card #242) ------------------------------------

/// task.left rows `dev` wrote, as `s` holds them.
fn lefts_by(s: &Side, dev: &str) -> usize {
    s.st.missing(&Default::default(), 100_000)
        .unwrap()
        .iter()
        .filter(|e| e.kind == kinds::TASK_LEFT && e.device_id == dev)
        .count()
}

#[test]
fn a_gone_agent_of_another_person_is_left_once_by_its_own_device() {
    let ra = TestRoster::new(&[CARD1]);
    let rb = TestRoster::new(&[CARD2]);
    let (a, b, _) = two_people_with(Some(ra), Some(rb.clone()));
    let tk = a.c.create_task(P1, nt("p1's card", "")).unwrap();
    sync(&a, &b);
    b.c.claim_task(CARD2, &tk.id).unwrap();
    sync(&a, &b);
    assert_eq!(a.c.get_task(&tk.id).unwrap().holders, [CARD2]);
    // p2's agent goes away. dev-a never had it live; dev-b sees it go.
    rb.set(CARD2, "");
    for s in [&a, &b] {
        s.c.sweep().unwrap();
        s.clk.add(HOLDER_GRACE + Duration::from_secs(1));
    }
    // dev-a may not take p2's agent off: its task.left would fold to
    // nothing, so it writes none, sweep after sweep.
    for _ in 0..5 {
        a.c.sweep().unwrap();
        a.clk.add(Duration::from_secs(5));
    }
    assert_eq!(lefts_by(&a, "dev-a"), 0);
    assert_eq!(a.c.get_task(&tk.id).unwrap().holders, [CARD2]);
    // dev-b, p2's device, takes it off once, and the card is free on both.
    for _ in 0..5 {
        b.c.sweep().unwrap();
        b.clk.add(Duration::from_secs(5));
        sync(&a, &b);
        a.c.sweep().unwrap();
    }
    assert_eq!(lefts_by(&b, "dev-b"), 1);
    assert_eq!(lefts_by(&a, "dev-a"), 0);
    for s in [&a, &b] {
        assert!(s.c.get_task(&tk.id).unwrap().holders.is_empty());
    }
}

// ---- who decides (card #264) ------------------------------------------------

fn ask(s: &Side, by: &str, id: &str, q: &str, on: Option<BlockedBy>) -> Result<(Task, Blocker)> {
    s.c.raise(
        by,
        id,
        NewBlocker {
            question: q.into(),
            blocked_by: on,
            ..NewBlocker::default()
        },
    )
}

fn for_person(key: &str) -> Option<BlockedBy> {
    Some(BlockedBy {
        kind: BLOCKED_ON_PERSON.into(),
        reference: key.into(),
    })
}

fn needs(s: &Side, viewer: &str) -> Vec<String> {
    s.c.board(&BoardFilter {
        viewer: viewer.into(),
        done_limit: 1,
        ..BoardFilter::default()
    })
    .needs_you
    .into_iter()
    .map(|c| c.task.id)
    .collect()
}

#[test]
fn a_question_on_a_board_is_for_the_cards_owner_by_default() {
    let (a, b, _) = two_people();
    // p1 made the card; p2's agent asks without saying who decides: it is
    // for p1, the owner (#429), not p2.
    let t = shared_card(&a, &b);
    let (_, item) = ask(&b, CARD2, &t.id, "Which colour?", None).unwrap();
    assert_eq!(item.on, for_person(P1));
    sync(&a, &b);
    let on_a = a.c.get_task(&t.id).unwrap();
    assert_eq!(
        on_a.open_blockers()[0].on,
        for_person(P1),
        "both folds agree"
    );
    assert_eq!(needs(&a, P1), vec![t.id.clone()], "it needs p1");
    assert!(needs(&b, P2).is_empty(), "and not p2");
    // An explicit "person" with no one named is the owner too.
    let (_, item) = ask(&b, CARD2, &t.id, "Ship it?", for_person("")).unwrap();
    assert_eq!(item.on, for_person(P1));
    // The owner's own agent asks: still the owner.
    let (_, item) = ask(&a, CARD1, &t.id, "Rename it?", None).unwrap();
    assert_eq!(item.on, for_person(P1));
    // for still names someone else.
    let (_, item) = ask(&b, CARD2, &t.id, "Blake's call?", for_person(P2)).unwrap();
    assert_eq!(item.on, for_person(P2));
}

/// #429: p2's agent finishing p1's card asks p1 "Is this done?", not p2.
#[test]
fn a_done_check_on_a_board_goes_to_the_cards_owner() {
    let owners: Owners = Arc::new(Mutex::new(vec![("p1".into(), 0, u64::MAX)]));
    let a = side_full("dev-a", P1, &owners, true, None);
    let b = side_full("dev-b", P2, &owners, true, None);
    a.st.append(
        P1,
        kinds::BOARD_CREATED,
        &raw(r#"{"boardId":"b","creator":{"personId":"p1"},"name":"wind","tag":"wind"}"#),
    )
    .unwrap();
    a.st.append(
        P1,
        kinds::BOARD_MEMBER_ADDED,
        &raw(r#"{"personId":"p2","role":"member"}"#),
    )
    .unwrap();
    pull(&a, &b);
    b.st.append(
        CARD2,
        kinds::AGENT_CARD,
        &raw(r#"{"cardId":"c2","personId":"p2","status":"active"}"#),
    )
    .unwrap();
    sync(&a, &b);
    let t = a.c.create_task(P1, nt("p1's card", "")).unwrap();
    sync(&a, &b);
    b.c.claim_task(CARD2, &t.id).unwrap();
    let got =
        b.c.set_task_state(CARD2, &t.id, COMPLETED, "shipped")
            .unwrap();
    let check = got
        .blockers
        .iter()
        .find(|x| x.open() && x.done_check)
        .expect("a done check");
    assert_eq!(check.on, for_person(P1));
    sync(&a, &b);
    assert_eq!(needs(&a, P1), vec![t.id.clone()], "the owner decides Done");
    assert!(
        needs(&b, P2).is_empty(),
        "not the person whose agent did it"
    );
}

#[test]
fn a_question_can_be_for_another_member() {
    let (a, b, _) = two_people();
    let t = shared_card(&a, &b);
    let (_, item) = ask(&a, CARD1, &t.id, "Blake: OK to rename?", for_person(P2)).unwrap();
    assert_eq!(item.on, for_person(P2));
    sync(&a, &b);
    assert_eq!(needs(&b, P2), vec![t.id.clone()]);
    assert!(needs(&a, P1).is_empty(), "p1's agent asked, but p2 decides");
    // Anyone can still answer: p1 resolves p2's question.
    a.c.answer(
        P1,
        &t.id,
        &item.id,
        BlockerAnswer {
            text: "yes".into(),
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    // Nobody who isn't on the board can be asked.
    let e = ask(&a, CARD1, &t.id, "Who?", for_person("person:p9")).unwrap_err();
    assert!(e.is(ErrorKind::Invalid), "{e}");
}

/// Card #236: wind#16's holder heard "PR merged" four times, the later ones
/// after the card was set completed and waited on the person's Done. Once
/// the merge is told, a completed card awaiting Done stays quiet at every
/// later poll, from either device.
#[test]
fn a_merged_pr_on_a_completed_board_card_awaiting_done_is_not_told_again() {
    const PR: &str = "https://github.com/a/b/pull/9";
    let owners: Owners = Arc::new(Mutex::new(vec![("p1".into(), 0, u64::MAX)]));
    let a = side_full("dev-a", P1, &owners, true, None);
    let b = side_full("dev-b", P2, &owners, true, None);
    a.st.append(
        P1,
        kinds::BOARD_CREATED,
        &raw(r#"{"boardId":"b","creator":{"personId":"p1"},"name":"wind","tag":"wind"}"#),
    )
    .unwrap();
    a.st.append(
        P1,
        kinds::BOARD_MEMBER_ADDED,
        &raw(r#"{"personId":"p2","role":"member"}"#),
    )
    .unwrap();
    pull(&a, &b);
    b.st.append(
        CARD2,
        kinds::AGENT_CARD,
        &raw(r#"{"cardId":"c2","personId":"p2","status":"active"}"#),
    )
    .unwrap();
    sync(&a, &b);
    let t = a.c.create_task(P1, nt("ship it", "")).unwrap();
    sync(&a, &b);
    b.c.claim_task(CARD2, &t.id).unwrap();
    b.c.add_link(CARD2, &t.id, PR).unwrap();
    sync(&a, &b);
    let merged = || PrCheck {
        pr: PR.into(),
        state: PR_MERGED.into(),
        ..PrCheck::default()
    };

    b.clk.add(Duration::from_secs(60));
    let (_, ns) = b.c.record_pr_state(&t.id, merged()).unwrap();
    assert_eq!(ns.len(), 1);

    // The holder sets it completed: it waits on the person's Done.
    let got =
        b.c.set_task_state(CARD2, &t.id, COMPLETED, "shipped")
            .unwrap();
    assert_eq!(got.state, BLOCKED);
    assert!(got.blockers.iter().any(|x| x.open() && x.done_check));
    sync(&a, &b);

    // Four more polls, 3 minutes apart (wind#16's repeats): nothing.
    for _ in 0..4 {
        b.clk.add(Duration::from_secs(180));
        let (_, ns) = b.c.record_pr_state(&t.id, merged()).unwrap();
        assert!(
            ns.is_empty(),
            "told again after completed: {:?}",
            ns[0].text
        );
        sync(&a, &b);
        if let Ok((_, ns)) = a.c.record_pr_state(&t.id, merged()) {
            assert!(ns.is_empty(), "the other device told it again");
        }
        sync(&a, &b);
    }
}

/// Card #337: a subtask on a shared board goes out like a top-level card,
/// to the author's free agent (wind#87-91 got a hand-out row; nobody in
/// the board's pool was free); another member's agents never get it.
#[test]
fn a_board_subtask_is_handed_out_like_a_card() {
    let (a, b) = two_people_shared();
    // p1 holds the parent, so p1's one agent is free for the subtask.
    let p = a.c.create_task(P1, nt("Parent", "")).unwrap();
    a.c.claim_task(P1, &p.id).unwrap();
    let s =
        a.c.create_task(
            P1,
            NewTask {
                title: "Sub".into(),
                parent: p.id.clone(),
                ..NewTask::default()
            },
        )
        .unwrap();
    sync(&a, &b);
    let (ns, r) = a.c.sweep_asks_partial(&roster(&[CARD2, CARD3]));
    r.unwrap();
    assert!(ns.is_empty(), "p2's agents never get p1's subtask: {ns:?}");
    let (ns, r) = a.c.sweep_asks_partial(&roster(&[CARD2, CARD1]));
    r.unwrap();
    assert!(
        ns.len() == 1 && ns[0].to == [CARD1] && ns[0].task_id == s.id,
        "{ns:?}"
    );
    assert_eq!(a.c.get_task(&s.id).unwrap().live_offer().unwrap().to, CARD1);
}

/// side_reopen, with `leases` as the live leases this person's agents hold
/// on their private board (card #339).
fn side_with_leases(
    s: Side,
    person: &str,
    owners: &Owners,
    leases: Arc<Mutex<Vec<String>>>,
) -> Side {
    let me = person.to_string();
    let o2 = owners.clone();
    let o = Options {
        now: Some(s.clk.func()),
        board: true,
        person: person.into(),
        is_owner: Some(Arc::new(move |k: &str| k == me)),
        key_person: Some(Arc::new(key_person)),
        device_person: Some(Arc::new(device_person)),
        tag: Some(Arc::new(|| "mlg".to_string())),
        board_owner: Some(Arc::new(move |p: &str, at: Option<&RowPos>| {
            o2.lock().unwrap().iter().any(|(who, from, until)| {
                who == p && at.map_or(*until == u64::MAX, |at| *from < at.hlc && at.hlc < *until)
            })
        })),
        outside_leases: Some(Arc::new(move || leases.lock().unwrap().clone())),
        ..Options::default()
    };
    let c = Coord::new(s.st.clone(), None, o).unwrap();
    Side { c, ..s }
}

/// Card #339, wind#91's case: a board card's external wait names leases
/// that live on its agent's private board. Its person's device closes the
/// wait when the last goes and tells the holder; the other member's
/// device, which can't see those leases, leaves it alone.
#[test]
fn a_board_card_waits_on_its_persons_private_leases() {
    let (a, b, owners) = two_people();
    let la = Arc::new(Mutex::new(vec![
        "ww/dolphin-slot-1".to_string(),
        "ww/dolphin-slot-2".to_string(),
    ]));
    let a = side_with_leases(a, P1, &owners, la.clone());
    let b = side_with_leases(b, P2, &owners, Arc::new(Mutex::new(Vec::new())));
    let t = a.c.create_task(P1, nt("UDP pose transport", "")).unwrap();
    a.c.claim_task(CARD1, &t.id).unwrap();
    let got =
        a.c.block(
            CARD1,
            &t.id,
            "Waiting for a free Dolphin slot",
            Some(BlockedBy {
                kind: BLOCKED_ON_EXTERNAL.into(),
                reference: "a free ww/dolphin-slot-1..4 lease (all four held)".into(),
            }),
        )
        .unwrap();
    // Named leases never show on a board: the wait stays in the agent's
    // words, and p1's devices judge it against p1's private leases.
    assert_eq!(
        got.blocked_by,
        Some(BlockedBy {
            kind: BLOCKED_ON_EXTERNAL.into(),
            reference: "a free ww/dolphin-slot-1..4 lease (all four held)".into(),
        })
    );
    sync(&a, &b);
    assert!(a.c.sweep_notices().unwrap().is_empty());
    assert!(
        b.c.sweep_notices().unwrap().is_empty(),
        "p2's device can't see p1's leases"
    );
    assert_eq!(b.c.get_task(&t.id).unwrap().state, BLOCKED);
    la.lock().unwrap().retain(|n| n != "ww/dolphin-slot-1");
    assert!(a.c.sweep_notices().unwrap().is_empty(), "slot 2 still held");
    la.lock().unwrap().clear();
    let ns = a.c.sweep_notices().unwrap();
    assert_eq!(ns.len(), 1, "{ns:?}");
    assert_eq!(
        (ns[0].kind.as_str(), ns[0].to.clone()),
        (NOTICE_UNBLOCKED, vec![CARD1.to_string()])
    );
    assert_eq!(a.c.get_task(&t.id).unwrap().state, WORKING);
    sync(&a, &b);
    assert_eq!(b.c.get_task(&t.id).unwrap().state, WORKING);
}

fn board_note(text: &str) -> NewNote {
    NewNote {
        kind: "gotcha".into(),
        text: text.into(),
        tags: vec![format!("repo:{REPO_A}")],
        ..NewNote::default()
    }
}

#[test]
fn a_board_note_any_member_accepts_reaches_every_members_agents() {
    let (a, b, _) = two_people();
    let n =
        a.c.propose_note(CARD1, board_note("MLG models ship to the repo, not zips."))
            .unwrap();
    sync(&a, &b);
    let cx = NoteContext {
        repo_key: REPO_A.into(),
        ..NoteContext::default()
    };
    for s in [&a, &b] {
        assert_eq!(s.c.note_by_id(&n.id).unwrap().state, NOTE_PROPOSED);
        assert!(
            s.c.notes_for(&cx).is_empty(),
            "a proposed note reaches no agent"
        );
    }
    // p2's device can't accept as p1: the forged row is ignored.
    let _ = b.c.accept_note(P1, &n.id);
    sync(&a, &b);
    assert_eq!(a.c.note_by_id(&n.id).unwrap().state, NOTE_PROPOSED);
    // An agent never accepts.
    let e = b.c.accept_note(CARD2, &n.id).unwrap_err();
    assert!(e.is(ErrorKind::NotAllowed), "{e}");
    // The other person accepts it: live for both people's agents.
    b.c.accept_note(P2, &n.id).unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let n2 = s.c.note_by_id(&n.id).unwrap();
        assert_eq!(
            (n2.state.as_str(), n2.accepted_by.as_str()),
            (NOTE_ACCEPTED, P2)
        );
        assert_eq!(s.c.notes_for(&cx).len(), 1);
    }
    // Either member removes it.
    a.c.remove_note(P1, &n.id, "done with zips").unwrap();
    sync(&a, &b);
    assert_eq!(b.c.note_by_id(&n.id).unwrap().state, NOTE_REMOVED);
}

#[test]
fn a_board_note_names_only_cards_on_the_board() {
    let (a, b, _) = two_people();
    let t = shared_card(&a, &b);
    let mut n = board_note("from the card");
    n.task_id = t.id.clone();
    n.source = format!("task:{}", t.id);
    a.c.propose_note(CARD1, n.clone()).unwrap();
    n.task_id = "01NOTONTHISBOARD0000000000".into();
    let e = a.c.propose_note(CARD1, n).unwrap_err();
    assert!(e.is(ErrorKind::NotFound), "{e}");
}

/// "Agents answer now" off on a shared board (card #391, irori#6): "@p2
/// look at #N" names only a person, so it wakes no agent and is never
/// handed out; the holders of the card it names read it at their next
/// turn, and one of them may still answer with a correction.
#[test]
fn a_board_thread_naming_only_a_person_wakes_no_agent() {
    let (a, b, _) = two_people();
    let card = shared_card(&a, &b);
    let mentions = vec![Mention {
        key: P2.into(),
        name: "p2".into(),
        ..Mention::default()
    }];
    assert!(!agents_now_default(&mentions), "only a person is named");
    let (tk, n) =
        a.c.post_ask(
            P1,
            NewAsk {
                text: format!("@p2 look at #{}", card.short),
                mentions,
                thread: true,
                quiet: true,
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap();
    assert!(
        n.to == [CARD1, CARD2] && !n.wake && n.text.contains("optional"),
        "read at their next turn, not woken: {n:?}"
    );
    sync(&a, &b);
    for s in [&a, &b] {
        let ask = s.c.get_task(&tk.id).unwrap().ask.unwrap();
        assert!(ask.thread && !ask.wake && ask.taker.is_empty(), "{ask:?}");
    }

    // Minutes on an idle agent is free, but it isn't handed out.
    a.clk.add(Duration::from_secs(120));
    b.clk.add(Duration::from_secs(120));
    assert!(a.c.sweep_asks(&[idle(CARD3, "dev-b")]).unwrap().is_empty());
    sync(&a, &b);
    assert!(b.c.get_task(&tk.id).unwrap().ask.unwrap().taker.is_empty());

    // card1 reads it at its next turn and corrects it.
    a.c.add_comment(CARD1, &tk.id, "That one merged already; see #1.", false)
        .unwrap();
    sync(&a, &b);
    assert!(a.c.sweep_asks(&[idle(CARD3, "dev-b")]).unwrap().is_empty());
    let t = a.c.get_task(&tk.id).unwrap();
    assert!(
        t.holders.is_empty() && !t.ask.as_ref().unwrap().wake,
        "{t:?}"
    );

    // p1's next post: the thread is still off, so even a waking post
    // wakes nobody, here or through p2's bridge (the row isn't wake).
    let (cm, ns) =
        a.c.person_comment(P1, &tk.id, "Thanks, I missed that.", true, &[], None)
            .unwrap();
    assert!(!cm.wake, "{cm:?}");
    assert!(
        !ns.is_empty()
            && ns
                .iter()
                .all(|n| !n.wake && n.text.contains("Since you last read")),
        "{ns:?}"
    );
}

// ---- card #432: a card's owner changes hands when both agree ----

/// The open ownership item on `t` as `s` folds it.
fn transfer_item(s: &Side, id: &str) -> Blocker {
    s.c.get_task(id)
        .unwrap()
        .blockers
        .into_iter()
        .find(|b| b.open() && b.transfer)
        .expect("an open ownership item")
}

fn pick(s: &Side, by: &str, id: &str, bid: &str, n: i64) -> Result<(Task, Vec<Notice>)> {
    s.c.answer(
        by,
        id,
        bid,
        BlockerAnswer {
            pick: n,
            ..BlockerAnswer::default()
        },
    )
}

#[test]
fn an_offered_card_moves_to_the_member_who_accepts() {
    let (a, b, _) = two_people();
    let t = shared_card(&a, &b);
    let (_, item) = a.c.transfer_owner(P1, &t.id, "p2").unwrap();
    assert!(item.transfer);
    assert_eq!(item.on, for_person(P2), "the other side decides");
    assert_eq!(
        item.options
            .iter()
            .map(|o| o.answer.as_str())
            .collect::<Vec<_>>(),
        vec![OWNER_ACCEPT, OWNER_DECLINE]
    );
    sync(&a, &b);
    assert_eq!(needs(&b, P2), vec![t.id.clone()], "it needs p2");
    // The owner can't accept their own offer, and no agent can.
    assert!(pick(&a, P1, &t.id, &item.id, 1).is_err());
    assert!(
        b.c.answer(
            CARD2,
            &t.id,
            &item.id,
            BlockerAnswer {
                text: "accept".into(),
                resolve: true,
                ..BlockerAnswer::default()
            }
        )
        .is_err()
    );
    pick(&b, P2, &t.id, &item.id, 1).unwrap();
    sync(&a, &b);
    for (s, who) in [(&a, "a"), (&b, "b")] {
        let got = s.c.get_task(&t.id).unwrap();
        assert_eq!(got.owner, "p2", "{who}: p2 owns it");
        assert!(got.owner_transfer.is_none(), "{who}: nothing pending");
        assert!(!got.blockers.iter().any(|x| x.open()), "{who}: item closed");
    }
    // From now on an unnamed question, and the done check, are p2's.
    let (_, q) = ask(&a, CARD1, &t.id, "Which colour?", None).unwrap();
    assert_eq!(q.on, for_person(P2));
}

#[test]
fn a_declined_offer_leaves_the_owner() {
    let (a, b, _) = two_people();
    let t = shared_card(&a, &b);
    let (_, item) = a.c.transfer_owner(P1, &t.id, "p2").unwrap();
    sync(&a, &b);
    pick(&b, P2, &t.id, &item.id, 2).unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let got = s.c.get_task(&t.id).unwrap();
        assert_eq!(got.owner, "", "still the creator's");
        assert!(got.owner_transfer.is_none());
        assert!(!got.blockers.iter().any(|x| x.open()));
    }
    let (_, q) = ask(&b, CARD2, &t.id, "Which colour?", None).unwrap();
    assert_eq!(q.on, for_person(P1));
    // A new offer can go out once that one is over.
    a.c.transfer_owner(P1, &t.id, "p2").unwrap();
}

#[test]
fn a_member_asks_for_a_card_and_the_owner_accepts() {
    let (a, b, _) = two_people();
    let t = shared_card(&a, &b);
    let (_, item) = b.c.transfer_owner(P2, &t.id, "").unwrap();
    assert_eq!(item.on, for_person(P1), "the owner decides");
    sync(&a, &b);
    assert_eq!(transfer_item(&a, &t.id).id, item.id);
    assert!(
        a.c.transfer_owner(P1, &t.id, "p2").is_err(),
        "one transfer at a time"
    );
    // The asker can't accept their own request.
    assert!(pick(&b, P2, &t.id, &item.id, 1).is_err());
    pick(&a, P1, &t.id, &item.id, 1).unwrap();
    sync(&a, &b);
    assert_eq!(a.c.get_task(&t.id).unwrap().owner, "p2");
    assert_eq!(b.c.get_task(&t.id).unwrap().owner, "p2");
    // p2 owns it now, so p1 asking for it back is a request to p2.
    let (_, back) = a.c.transfer_owner(P1, &t.id, "").unwrap();
    assert_eq!(back.on, for_person(P2));
}

#[test]
fn an_owner_row_from_the_wrong_side_is_ignored() {
    let (a, b, _) = two_people();
    let t = shared_card(&a, &b);
    // p2's device writes an accept with no offer, then an offer of a card
    // it doesn't own: neither counts on either fold.
    for op in ["accept", "offer"] {
        b.st.append(
            P2,
            kinds::TASK_OWNER,
            &raw(&format!(
                r#"{{"taskId":"{}","op":"{op}","person":"p2","by":"person:p2","at":"2026-11-03T00:00:00Z"}}"#,
                t.id
            )),
        )
        .unwrap();
    }
    sync(&a, &b);
    for s in [&a, &b] {
        let got = s.c.get_task(&t.id).unwrap();
        assert_eq!(got.owner, "");
        assert!(got.owner_transfer.is_none());
    }
    // And p1's own device can't accept its own offer behind the tool's back.
    let (_, item) = a.c.transfer_owner(P1, &t.id, "p2").unwrap();
    a.st.append(
        P1,
        kinds::TASK_OWNER,
        &raw(&format!(
            r#"{{"taskId":"{}","op":"accept","person":"p2","blockerId":"{}","by":"person:p1","at":"2026-11-03T00:00:00Z"}}"#,
            t.id, item.id
        )),
    )
    .unwrap();
    sync(&a, &b);
    assert_eq!(b.c.get_task(&t.id).unwrap().owner, "");
    assert_eq!(a.c.get_task(&t.id).unwrap().owner, "");
}

fn spawn_step(text: &str) -> Step {
    Step {
        text: text.into(),
        spawn: true,
        ..Step::default()
    }
}

/// Part 7 (spawned agents §3): another person's agent can edit p1's
/// card's steps, but its spawn marks are set aside, on both sides; p1's
/// own agent sets one.
#[test]
fn another_persons_spawn_mark_on_a_card_is_refused() {
    let (a, b, _) = two_people();
    let t = shared_card(&a, &b);
    let got =
        b.c.set_steps(CARD2, &t.id, vec![spawn_step("tests")])
            .unwrap();
    assert_eq!(got.steps[0].text, "tests", "the edit goes through");
    assert!(!got.steps[0].spawn, "the mark doesn't");
    sync(&a, &b);
    assert!(!a.c.get_task(&t.id).unwrap().steps[0].spawn);
    // A row written straight to the log gets no further.
    b.st.append(
        CARD2,
        kinds::TASK_STEPS,
        &coord::rows::TaskStepsRow {
            task_id: t.id.clone(),
            steps: vec![Step {
                id: "s1".into(),
                ..spawn_step("tests")
            }],
            by: CARD2.into(),
            at: fixed_now(),
        },
    )
    .unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        assert!(!s.c.get_task(&t.id).unwrap().steps[0].spawn);
    }
    // The owner's agent marks it, and both sides agree.
    a.c.set_steps(CARD1, &t.id, vec![spawn_step("tests")])
        .unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        assert!(s.c.get_task(&t.id).unwrap().steps[0].spawn);
    }
    // p2's agent redoing the steps can't drop it either.
    b.c.set_steps(
        CARD2,
        &t.id,
        vec![Step {
            text: "tests".into(),
            ..Step::default()
        }],
    )
    .unwrap();
    sync(&a, &b);
    assert!(a.c.get_task(&t.id).unwrap().steps[0].spawn);
}

#[test]
fn another_persons_split_makes_no_helper_parts() {
    let (a, b, _) = two_people();
    let t = shared_card(&a, &b);
    let r = b.c.split(
        CARD2,
        &t.id,
        vec![NewSubtask {
            title: "part".into(),
            spawn: true,
            ..NewSubtask::default()
        }],
        vec![],
    );
    let (_, made) = r.unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        assert!(!s.c.get_task(&made[0].id).unwrap().spawn);
    }
}

/// Card #481: an answered item whose agent left the card closes from a
/// device of the raiser's person only; the other person's sweep writes
/// nothing (a row that folds to nothing there was written every sweep,
/// card #242).
#[test]
fn an_answered_item_whose_agent_left_closes_from_the_raisers_device() {
    let (a, b, _) = two_people();
    let t = shared_card(&a, &b);
    let (_, item) = ask(&a, CARD1, &t.id, "which port?", None).unwrap();
    a.c.answer(
        P1,
        &t.id,
        &item.id,
        BlockerAnswer {
            text: "8080".into(),
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    a.c.leave(CARD1, &t.id, CARD1).unwrap();
    sync(&a, &b);
    let mine = |s: &Side, dev: &str| {
        s.st.list(&store::Filter {
            device_id: dev.into(),
            limit: store::MAX_LIST_LIMIT,
            ..store::Filter::default()
        })
        .unwrap()
        .len()
    };
    let n = mine(&b, "dev-b");
    b.c.sweep_notices().unwrap();
    assert_eq!(mine(&b, "dev-b"), n, "p2's device wrote for p1's agent");
    assert_eq!(b.c.get_task(&t.id).unwrap().state, BLOCKED);
    a.c.sweep_notices().unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let got = s.c.get_task(&t.id).unwrap();
        assert_eq!(got.state, WORKING, "{got:?}");
        let r = got.blockers[0].resolved.as_ref().expect("closed");
        assert_eq!(r.by, "irori");
    }
    let n = mine(&a, "dev-a");
    a.c.sweep_notices().unwrap();
    assert_eq!(mine(&a, "dev-a"), n, "closed once");
}

/// Card #485: a board's log keeps no device.started rows, so a version
/// wait there could never close: it is refused, with where it goes.
#[test]
fn a_version_wait_on_a_board_is_refused() {
    let (a, _b, _) = two_people();
    let t = a.c.create_task(P1, nt("live check", REPO_A)).unwrap();
    a.c.claim_task(CARD1, &t.id).unwrap();
    let err =
        a.c.block(
            CARD1,
            &t.id,
            "Update the Mac",
            Some(BlockedBy {
                kind: BLOCKED_ON_VERSION.into(),
                reference: "0.11.0 dev-a".into(),
            }),
        )
        .unwrap_err();
    assert!(err.is(ErrorKind::Invalid), "{err}");
    assert!(err.to_string().contains("your own board"), "{err}");
}

/// Card #512: a thread in a board's own log gets its own agent at once,
/// on the poster's device only, like a board card on the device that made
/// it (#455). It names a card both people's agents hold: they are woken,
/// and nobody waits on them. One with "Agents answer now" off never spawns.
#[test]
fn a_board_thread_spawns_on_the_posters_device_at_once() {
    let (a, b, owners) = two_people();
    let card = shared_card(&a, &b);
    let on = || Options {
        spawning: Some(Arc::new(|| true)),
        ..Options::default()
    };
    let a = side_reopen_with(a, P1, &owners, on());
    let b = side_reopen_with(b, P2, &owners, on());
    let thread = |text: &str, quiet: bool| {
        a.c.post_ask(
            P1,
            NewAsk {
                text: text.into(),
                thread: true,
                quiet,
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap()
        .0
    };
    let tk = thread(&format!("Is #{} still moving?", card.short), false);
    let off = thread(&format!("A note on #{}", card.short), true);
    sync(&a, &b);
    assert_eq!(
        b.c.get_task(&tk.id).unwrap().ask.unwrap().heard,
        [CARD1, CARD2]
    );
    // The first sweep, with no clock advance: nobody free, so unclaimed,
    // and listed on p1's device only.
    a.c.sweep_asks(&[]).unwrap();
    b.c.sweep_asks(&[]).unwrap();
    sync(&a, &b);
    assert_eq!(ids(&a.c.cards_to_spawn()), std::slice::from_ref(&tk.id));
    assert!(
        b.c.cards_to_spawn().is_empty(),
        "{:?}",
        b.c.cards_to_spawn()
    );
    // p2's device can't ask for it either.
    assert_eq!(
        b.c.request_spawn("irori", spawn_ask(&tk.id, "dev-b")).ok(),
        Some(None)
    );

    // p1's device starts it, and its agent takes the thread on both sides.
    let g =
        a.c.request_spawn("irori", spawn_ask(&tk.id, "dev-a"))
            .unwrap()
            .unwrap();
    assert!(a.c.bind_spawn(&tk.id, g, CARD1).unwrap());
    sync(&a, &b);
    for s in [&a, &b] {
        let ask = s.c.get_task(&tk.id).unwrap().ask.unwrap();
        assert!(ask.taker == CARD1 && ask.reason == TAKE_TAKEN, "{ask:?}");
        assert!(s.c.cards_to_spawn().is_empty());
    }
    // Agents answer now off: never handed out, never spawned for.
    assert!(
        !ids(&a.c.cards_to_spawn()).contains(&off.id),
        "{:?}",
        a.c.get_task(&off.id).unwrap().ask
    );
}

/// Card #477 (docs/design/work-personal-separation.md §3): on one space,
/// p2's card is never p1's to start, and the reverse. Both sides turn
/// spawning on here: each lists only its own person's card. Since #455 a
/// board log takes `task.spawn` rows, but only the card author's devices'
/// count: p1's sweep and p1's agent asking for p2's card change nothing
/// on either side, and no device is asked to start it.
#[test]
fn another_persons_card_is_never_spawned_on_your_side() {
    let (a, b, owners) = two_people();
    let theirs = b.c.create_task(P2, nt("Work card", "")).unwrap();
    let mine = a.c.create_task(P1, nt("Personal card", "")).unwrap();
    sync(&a, &b);
    let on = || Options {
        spawning: Some(Arc::new(|| true)),
        ..Options::default()
    };
    let a = side_reopen_with(a, P1, &owners, on());
    let b = side_reopen_with(b, P2, &owners, on());
    assert_eq!(ids(&a.c.cards_to_spawn()), std::slice::from_ref(&mine.id));
    assert_eq!(ids(&b.c.cards_to_spawn()), std::slice::from_ref(&theirs.id));
    // p1's sweep asks for an agent for p2's card anyway.
    let _ = a.c.request_spawn(
        "irori",
        SpawnAsk {
            task_id: theirs.id.clone(),
            device: "dev-a".into(),
            folder: "/work".into(),
            permission_mode: "default".into(),
            name: "for-theirs".into(),
            ..SpawnAsk::default()
        },
    );
    // One of p1's agents writing it straight to the log.
    let row = coord::rows::TaskSpawnRow {
        task_id: theirs.id.clone(),
        generation: 1,
        op: "requested".into(),
        device: "dev-a".into(),
        permission_mode: "default".into(),
        by: CARD1.into(),
        at: fixed_now(),
        ..Default::default()
    };
    if let Ok(ev) = a.st.append(CARD1, kinds::TASK_SPAWN, &row) {
        a.c.fold_live(&[ev]);
    }
    sync(&a, &b);
    for s in [&a, &b] {
        let t = s.c.get_task(&theirs.id).unwrap();
        assert!(t.spawn_request().is_none(), "{:?}", t.spawn_request());
        assert!(s.c.spawns_here().is_empty(), "{:?}", s.c.spawns_here());
    }
    assert!(!ids(&a.c.cards_to_spawn()).contains(&theirs.id));
    assert_eq!(ids(&b.c.cards_to_spawn()), std::slice::from_ref(&theirs.id));
}

// Card #455 (spawned agents §7, §9, part 11): who may start an agent on a
// shared board. p1 is Stephen on dev-a, p2 is a second person on dev-b.

fn spawn_ask(id: &str, device: &str) -> SpawnAsk {
    SpawnAsk {
        task_id: id.into(),
        device: device.into(),
        permission_mode: "acceptEdits".into(),
        name: "otter".into(),
        ..SpawnAsk::default()
    }
}

fn spawn_row(id: &str, device: &str, by: &str, at: GoTime) -> coord::rows::TaskSpawnRow {
    coord::rows::TaskSpawnRow {
        task_id: id.into(),
        generation: 1,
        op: SPAWN_REQUESTED.into(),
        device: device.into(),
        permission_mode: "acceptEdits".into(),
        name: "otter".into(),
        by: by.into(),
        at,
        ..Default::default()
    }
}

/// The second person's request naming Stephen's device is never one his
/// device acts on: not on their own card (it folds there, as their own
/// card's request, but names a device that isn't theirs), and not on his
/// card (refused by the writer check).
#[test]
fn another_persons_spawn_request_never_starts_an_agent_on_my_device() {
    let (a, b) = two_people_shared();
    let theirs = b.c.create_task(P2, nt("Their card", "")).unwrap();
    let mine = a.c.create_task(P1, nt("My card", "")).unwrap();
    sync(&a, &b);
    // Their request on their own card, naming dev-a, from their device.
    assert!(
        b.c.request_spawn(P2, spawn_ask(&theirs.id, "dev-a"))
            .unwrap()
            .is_some()
    );
    // Their request on my card, naming dev-a: as them, and as Irori on
    // their device.
    for (actor, by) in [(P2, P2), ("irori", "irori")] {
        let ev =
            b.st.append(
                actor,
                kinds::TASK_SPAWN,
                &spawn_row(&mine.id, "dev-a", by, b.clk.now()),
            )
            .unwrap();
        b.c.fold_live(&[ev]);
    }
    sync(&a, &b);
    assert!(
        a.c.spawns_here().is_empty(),
        "nothing for dev-a to start: {:?}",
        a.c.spawns_here()
    );
    assert!(!a.c.get_task(&mine.id).unwrap().spawn_pending());
    // My own request on my card naming my device is mine to start, and
    // their device never starts it.
    assert!(
        a.c.request_spawn(P1, spawn_ask(&mine.id, "dev-a"))
            .unwrap()
            .is_some()
    );
    sync(&a, &b);
    let here: Vec<String> = a.c.spawns_here().into_iter().map(|(t, _)| t.id).collect();
    assert_eq!(here, vec![mine.id.clone()]);
    assert!(b.c.spawns_here().is_empty());
}

/// A request from Stephen's other device (dev-a2, his Irori) naming dev-a
/// is dev-a's to start.
#[test]
fn my_other_devices_request_naming_this_device_is_mine_to_start() {
    let (a, _b) = two_people_shared();
    let t = a.c.create_task(P1, nt("Mac repo", "")).unwrap();
    let ev = Event {
        device_id: "dev-a2".into(),
        seq: 1,
        hlc: hlc::Timestamp(((a.clk.now().unix_milli_value() as u64) << 16) + 1),
        actor: "irori".into(),
        kind: kinds::TASK_SPAWN.into(),
        payload: gocompat::json::marshal(&spawn_row(&t.id, "dev-a", "irori", a.clk.now())).unwrap(),
        sig: Vec::new(),
    };
    let (_, r) = a.st.insert_batch(std::slice::from_ref(&ev));
    r.unwrap();
    a.c.fold_live(&[ev]);
    assert_eq!(a.c.spawns_here().len(), 1);
    assert_eq!(a.c.spawns_here()[0].1.device, "dev-a");
}

/// Stephen's spawned agent bound to his board card carries its mark on
/// the second person's device too, and their hand-out never offers it
/// their card, even when it is idle there.
#[test]
fn my_spawned_agent_is_never_offered_another_persons_card() {
    let (a, b) = two_people_shared();
    let mine = a.c.create_task(P1, nt("My card", "")).unwrap();
    let g =
        a.c.request_spawn("irori", spawn_ask(&mine.id, "dev-a"))
            .unwrap()
            .unwrap();
    assert!(a.c.bind_spawn(&mine.id, g, CARD1).unwrap());
    let theirs = b.c.create_task(P2, nt("Their card", "")).unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        assert_eq!(
            s.c.spawned(CARD1).map(|m| m.task_id),
            Some(mine.id.clone()),
            "the mark folds on {}",
            s.c.device_id()
        );
        assert!(
            s.c.get_task(&mine.id)
                .unwrap()
                .holders
                .iter()
                .any(|h| h == CARD1),
            "bound on {}",
            s.c.device_id()
        );
    }
    let (ns, r) = b.c.sweep_asks_partial(&roster(&[CARD1]));
    r.unwrap();
    assert!(ns.is_empty(), "{ns:?}");
    assert!(
        b.c.get_task(&theirs.id)
            .unwrap()
            .live_offer()
            .is_none_or(|o| o.to != CARD1)
    );
    // Their Irori can't mark my agent as started for their card.
    let ev =
        b.st.append(
            "irori",
            kinds::TASK_SPAWNED,
            &raw(&format!(
                r#"{{"taskId":"{}","key":"{CARD1}","gen":1,"device":"dev-a","by":"irori","at":"2026-10-26T12:00:00Z"}}"#,
                theirs.id
            )),
        )
        .unwrap();
    b.c.fold_live(&[ev]);
    sync(&a, &b);
    assert_eq!(a.c.spawned(CARD1).map(|m| m.task_id), Some(mine.id.clone()));
}
