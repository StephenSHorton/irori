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
    _dir: tempfile::TempDir,
    st: Arc<Store>,
    clk: Clk,
    c: Coord,
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

fn side(dev: &str, person: &str, owners: &Owners) -> Side {
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
    let c = Coord::new(st.clone(), None, o).unwrap();
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
    let owners: Owners = Arc::new(Mutex::new(vec![("p1".into(), 0, u64::MAX)]));
    let a = side("dev-a", P1, &owners);
    let b = side("dev-b", P2, &owners);
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
    sync(&a, &b);
    assert!(
        a.c.comments(&t.id).unwrap().iter().any(|c| c.id == cm.id),
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
