//! Port of I:internal/coord/ask_test.go (room asks). Go's dead context in
//! TestAFailedSweepKeepsTheRetell is a store holding its appends here.
#![allow(clippy::type_complexity)]

use crate::common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use coord::roster::AgentCard;
use coord::rows::{
    AskField, TakeRequestedRow, TakeStateRow, TaskClaimedRow, TaskCreatedRow, TaskEditedRow,
    TaskMemberRow, TaskStateRow,
};
use coord::*;
use store::{Event, Filter, kinds};

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

fn agents(e: &Env) -> Vec<AgentCard> {
    use coord::roster::Agents as _;
    e.ros.agents()
}

fn post_ask(e: &Env, text: &str) -> (Task, Notice) {
    e.c.post_ask(
        USER,
        NewAsk {
            text: text.into(),
            ..NewAsk::default()
        },
        &agents(e),
    )
    .unwrap()
}

/// An ask posted while no agent is idle: unclaimed, for the take lock's
/// own tests (claims, rows from other devices).
fn post_unheard(e: &Env, text: &str) -> (Task, Notice) {
    e.c.post_ask(
        USER,
        NewAsk {
            text: text.into(),
            ..NewAsk::default()
        },
        &[],
    )
    .unwrap()
}

fn ask_of(c: &Coord, id: &str) -> Ask {
    c.get_task(id)
        .and_then(|t| t.ask)
        .unwrap_or_else(|| panic!("no ask {id}"))
}

fn sec() -> Duration {
    Duration::from_secs(1)
}

#[test]
fn test_post_ask_hands_it_to_one_idle_agent() {
    let e = Env::new();
    e.ros.set(CAROL, "busy");
    let (tk, n) = post_ask(
        &e,
        "Why is the build slow on the Mac?\nIt was fine last week.",
    );
    assert!(
        tk.title == "Why is the build slow on the Mac?"
            && tk.objective.contains("last week")
            && !tk.short.is_empty(),
        "{tk:?}"
    );
    let a = ask_of(&e.c, &tk.id);
    assert!(
        a.owner == "dev-a"
            && a.told == 1
            && a.reason == TAKE_HANDED
            && a.taker == ALICE
            && a.handed
            && !a.unclaimed,
        "{a:?}"
    );
    assert!(
        n.kind == NOTICE_ASK
            && n.person
            && n.to == [ALICE]
            && n.text.contains(&format!("board_leave #{}", tk.short))
            && n.wake,
        "{n:?}"
    );
    assert_eq!(
        n.text,
        format!(
            "The person asks the room (#1, task:{id}), and Irori handed it to you:\nWhy is the build slow on the Mac?\nIt was fine last week.\nYou hold it: no ask_claim needed. If it isn't clear what they want, talk it through with them on the card first. If it needs a choice only the person can make, ask_close it as a card first (it stays yours), then raise one board_block per question on that card. Otherwise ask_close it with what it became: a card, a change to another card, or nothing. If you can't take it on now, board_leave #1 and it goes to the next idle agent; ending your turn without touching it does the same.\nshow task:{id} for the card, its comments and links.",
            id = tk.id
        )
    );
    want_err(
        e.c.post_ask(
            ALICE,
            NewAsk {
                text: "x".into(),
                ..NewAsk::default()
            },
            &[],
        ),
        ErrorKind::NotAllowed,
    );
    let got = ask_title(&"word ".repeat(40));
    assert!(got.chars().count() <= 80 && got.ends_with('…'), "{got:?}");
    // The cut at the last space (a byte index > 40) then the ellipsis.
    assert_eq!(
        ask_title(&"word ".repeat(40)),
        format!("{}…", "word ".repeat(15).trim_end())
    );
}

#[test]
fn test_two_local_claims_one_taker() {
    let e = Env::new();
    let (tk, _) = post_unheard(&e, "Fix the flaky test");
    let res: Vec<Result<ClaimResult>> = std::thread::scope(|s| {
        let hs: Vec<_> = [ALICE, BOB]
            .into_iter()
            .map(|who| {
                let c = &e.c;
                let id = tk.id.clone();
                s.spawn(move || c.ask_claim(who, &id, sec()))
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let (mut wins, mut held) = (0, 0);
    for r in &res {
        match r {
            Ok(r) if r.mine => wins += 1,
            Err(x) if x.is(ErrorKind::Held) && x.to_string().contains("took") => held += 1,
            other => panic!("{other:?}"),
        }
    }
    assert_eq!((wins, held), (1, 1));
    let a = ask_of(&e.c, &tk.id);
    let got = e.c.get_task(&tk.id).unwrap();
    assert!(!a.taker.is_empty() && got.holders == [a.taker.clone()] && got.state == SUBMITTED);
    assert_eq!(e.rows(kinds::TAKE_STATE).len(), 2);
    let other = if a.taker == BOB { ALICE } else { BOB };
    want_err(e.c.claim_task(other, &tk.id), ErrorKind::NotAllowed);
    want_err(e.c.assign(other, &tk.id, other), ErrorKind::NotAllowed);
    want_err(
        e.c.set_task_state(&a.taker, &tk.id, COMPLETED, ""),
        ErrorKind::NotAllowed,
    );
    want_err(
        e.c.block(&a.taker, &tk.id, "why", None),
        ErrorKind::NotAllowed,
    );
    let r = format!("task:{}", tk.id);
    assert!(!e.c.stale_notice(&r, other).is_empty() && e.c.stale_notice(&r, &a.taker).is_empty());
    let back = ask_of(&e.reopen(), &tk.id);
    assert!(back.taker == a.taker && back.told == a.told);
}

#[test]
fn test_taker_gone_hands_it_back() {
    let e = Env::new();
    let (tk, _) = post_ask(&e, "Look at the crash report");
    assert!(e.c.ask_claim(ALICE, &tk.id, sec()).unwrap().mine);
    e.ros.set(ALICE, "");
    let ns = e.c.sweep_asks(&agents(&e)).unwrap();
    assert!(
        ask_of(&e.c, &tk.id).taker == ALICE && ns.is_empty(),
        "{:?} {ns:?}",
        ask_of(&e.c, &tk.id)
    );
    e.clk.add(GONE_GRACE + sec());
    let ns = e.c.sweep_asks(&agents(&e)).unwrap();
    let a = ask_of(&e.c, &tk.id);
    assert!(
        a.taker == BOB && a.handed && a.heard == [ALICE, BOB],
        "{a:?}"
    );
    assert!(
        ns.len() == 1
            && ns[0].kind == NOTICE_ASK
            && ns[0].to == [BOB]
            && ns[0].text.contains("went away"),
        "{ns:?}"
    );
    assert!(e.c.ask_claim(BOB, &tk.id, sec()).unwrap().mine);
    assert!(!ask_of(&e.c, &tk.id).handed);
    e.c.leave(BOB, &tk.id, BOB).unwrap();
    let ns = e.c.sweep_asks(&agents(&e)).unwrap();
    let a = ask_of(&e.c, &tk.id);
    assert!(a.taker == CAROL && a.handed, "{a:?}");
    assert!(ns.len() == 1 && ns[0].to == [CAROL], "{ns:?}");
    let c2 = e.reopen();
    c2.sweep_asks(&agents(&e)).unwrap();
    assert_eq!(ask_of(&c2, &tk.id).taker, CAROL);
}

#[test]
fn test_offline_taker_keeps_it() {
    let e = Env::new();
    const REMOTE: &str = "dev-b/far-session";
    let (tk, _) = post_unheard(&e, "Check the Mac's disk");
    let at = hlc_at(plus(fixed_now(), mins(1)));
    e.remote(
        "dev-b",
        1,
        at,
        &format!("agent:{REMOTE}"),
        kinds::TAKE_REQUESTED,
        &TakeRequestedRow {
            task_id: tk.id.clone(),
            by: REMOTE.into(),
            op: TAKE_CLAIM.into(),
            at: fixed_now(),
        },
    );
    assert_eq!(ask_of(&e.c, &tk.id).taker, REMOTE);
    let mut offline = agents(&e);
    offline.push(AgentCard {
        key: REMOTE.into(),
        status: "idle".into(),
        device_id: "dev-b".into(),
        device_state: "offline".into(),
        ..AgentCard::default()
    });
    e.clk.add(Duration::from_secs(3600));
    e.c.sweep_asks(&offline).unwrap();
    assert_eq!(ask_of(&e.c, &tk.id).taker, REMOTE);
    let ns = e.c.sweep_asks(&agents(&e)).unwrap();
    let a = ask_of(&e.c, &tk.id);
    assert!(a.taker == ALICE && a.handed, "{a:?}");
    assert!(ns.len() == 1 && ns[0].text.contains("went away"), "{ns:?}");
}

#[test]
fn test_unclaimed_and_ask_again() {
    let e = Env::new();
    for k in [ALICE, BOB, CAROL] {
        e.ros.set(k, "busy");
    }
    let (tk, n) = post_ask(&e, "Tidy the README");
    let a = ask_of(&e.c, &tk.id);
    assert!(
        a.unclaimed && a.told == 0 && a.taker.is_empty() && n.to.is_empty(),
        "{a:?}"
    );
    let rows = e.rows(kinds::TAKE_STATE).len();
    // Nothing changed, nothing written, however long it waits.
    e.clk.add(Duration::from_secs(3600));
    assert!(e.c.sweep_asks(&agents(&e)).unwrap().is_empty());
    assert_eq!(e.rows(kinds::TAKE_STATE).len(), rows);
    // An agent going idle gets it.
    e.ros.set(BOB, "idle");
    let ns = e.c.sweep_asks(&agents(&e)).unwrap();
    assert!(ns.len() == 1 && ns[0].to == [BOB] && ns[0].wake, "{ns:?}");
    let a = ask_of(&e.c, &tk.id);
    assert!(
        a.taker == BOB && a.handed && !a.unclaimed && a.told == 1,
        "{a:?}"
    );
    // BOB gives it back and nobody else is idle: unclaimed, BOB not told again.
    e.c.leave(BOB, &tk.id, BOB).unwrap();
    assert!(e.c.sweep_asks(&agents(&e)).unwrap().is_empty());
    let a = ask_of(&e.c, &tk.id);
    assert!(
        a.taker.is_empty() && a.unclaimed && a.heard == [BOB],
        "{a:?}"
    );
    // The person's "Wake them": everyone may hear it again.
    e.c.ask_again(USER, &tk.id).unwrap();
    let ns = e.c.sweep_asks(&agents(&e)).unwrap();
    assert!(ns.len() == 1 && ns[0].to == [BOB] && ns[0].wake, "{ns:?}");
    assert!(
        ns[0].text.contains("the person wants it picked up"),
        "{ns:?}"
    );
    let a = ask_of(&e.c, &tk.id);
    assert!(a.taker == BOB && a.handed && a.heard == [BOB], "{a:?}");
}

#[test]
fn test_ask_outcomes() {
    let e = Env::new();
    let other =
        e.c.create_task(
            BOB,
            NewTask {
                title: "Ship docs".into(),
                objective: "old".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    let (card, _) = post_ask(&e, "the login page is slow");
    e.c.ask_claim(ALICE, &card.id, sec()).unwrap();
    want_err(
        e.c.ask_close(
            BOB,
            &card.id,
            AskOutcome {
                kind: OUTCOME_NONE.into(),
                note: "x".into(),
                ..AskOutcome::default()
            },
        ),
        ErrorKind::NotAllowed,
    );
    let got =
        e.c.ask_close(
            ALICE,
            &card.id,
            AskOutcome {
                kind: OUTCOME_CARD.into(),
                note: "a real fix".into(),
                title: "Speed up the login page".into(),
                done_when: "under 1s".into(),
                steps: vec![
                    Step {
                        text: "profile".into(),
                        ..Step::default()
                    },
                    Step {
                        text: "fix".into(),
                        ..Step::default()
                    },
                ],
                ..AskOutcome::default()
            },
        )
        .unwrap();
    assert!(
        got.title == "Speed up the login page"
            && got.done_criteria == "under 1s"
            && got.objective == "the login page is slow"
    );
    let ga = got.ask.clone().unwrap();
    assert!(
        got.state == WORKING
            && got.holder == ALICE
            && got.steps.len() == 2
            && !ga.is_open()
            && ga.outcome.unwrap().kind == OUTCOME_CARD
            && got.short == card.short
    );
    let acts = e.c.activity(&card.id).unwrap();
    assert!(
        acts.iter().any(|a| a.what
            == "renamed it from “the login page is slow” to “Speed up the login page”; set done when to “under 1s”"),
        "{acts:?}"
    );
    e.c.set_task_state(ALICE, &card.id, COMPLETED, "done")
        .unwrap();
    e.ros.set(ALICE, "busy");
    let (edit, _) = post_ask(&e, "docs should mention Windows");
    assert_eq!(ask_of(&e.c, &edit.id).taker, BOB);
    e.c.ask_claim(BOB, &edit.id, sec()).unwrap();
    e.c.ask_close(
        BOB,
        &edit.id,
        AskOutcome {
            kind: OUTCOME_EDIT.into(),
            task: other.id.clone(),
            note: "added Windows".into(),
            objective: "new".into(),
            ..AskOutcome::default()
        },
    )
    .unwrap();
    let ed = e.c.get_task(&edit.id).unwrap();
    let ot = e.c.get_task(&other.id).unwrap();
    assert!(
        ed.state == COMPLETED
            && ed.ask.unwrap().outcome.unwrap().task == other.id
            && ot.objective == "new"
            && ot.title == "Ship docs"
    );
    let acts = e.c.activity(&other.id).unwrap();
    assert!(
        acts[0]
            .what
            .contains(&format!("from the ask #{}", edit.short))
            && acts[0].by == BOB,
        "{:?}",
        acts[0]
    );
    let (none, _) = post_ask(&e, "never mind");
    e.c.set_task_state(USER, &none.id, CANCELED, "").unwrap();
    let n = e.c.get_task(&none.id).unwrap();
    let o = n.ask.unwrap().outcome.unwrap();
    assert!(n.state == CANCELED && o.kind == OUTCOME_NONE && o.note == "closed by the person");
    assert!(
        !e.c.stale_notice(&format!("task:{}", none.id), ALICE)
            .is_empty()
    );
    let c2 = e.reopen();
    for id in [&card.id, &edit.id, &none.id, &other.id] {
        let (a, b) = (e.c.get_task(id).unwrap(), c2.get_task(id).unwrap());
        assert!(
            a.title == b.title
                && a.state == b.state
                && a.objective == b.objective
                && a.ask == b.ask,
            "{id}"
        );
    }
}

#[test]
fn test_take_rows_from_the_wrong_device_and_old_claims() {
    let e = Env::new();
    let (tk, _) = post_unheard(&e, "who wants this");
    let at = hlc_at(plus(fixed_now(), mins(1)));
    e.remote(
        "dev-b",
        1,
        at,
        "irori",
        kinds::TAKE_STATE,
        &TakeStateRow {
            task_id: tk.id.clone(),
            taker: "dev-b/x".into(),
            reason: TAKE_TAKEN.into(),
            at: fixed_now(),
            ..TakeStateRow::default()
        },
    );
    assert!(ask_of(&e.c, &tk.id).taker.is_empty());
    assert_eq!(e.c.ignored_rows(), 1);
    e.remote(
        "dev-b",
        2,
        at + 1,
        "agent:dev-b/x",
        kinds::TASK_CLAIMED,
        &TaskClaimedRow {
            task_id: tk.id.clone(),
            by: "dev-b/x".into(),
            at: fixed_now(),
        },
    );
    e.remote(
        "dev-b",
        3,
        at + 2,
        "agent:dev-b/x",
        kinds::TASK_JOINED,
        &TaskMemberRow {
            task_id: tk.id.clone(),
            who: "dev-b/x".into(),
            by: "dev-b/x".into(),
            at: fixed_now(),
            ..TaskMemberRow::default()
        },
    );
    e.remote(
        "dev-b",
        4,
        at + 3,
        "agent:dev-b/x",
        kinds::TASK_STATUS,
        &TaskStateRow {
            task_id: tk.id.clone(),
            state: COMPLETED.into(),
            by: "dev-b/x".into(),
            at: fixed_now(),
            ..TaskStateRow::default()
        },
    );
    let got = e.c.get_task(&tk.id).unwrap();
    assert!(got.holders.is_empty() && got.state == SUBMITTED && got.ask.unwrap().is_open());
    let c2 = e.reopen();
    assert!(c2.get_task(&tk.id).unwrap().holders.is_empty() && c2.ignored_rows() == 1);
}

#[test]
fn test_take_rows_two_orders_and_the_loser_hears() {
    let owner = Env::new();
    let (tk, _) = post_ask(&owner, "partitioned ask");
    owner.c.ask_claim(ALICE, &tk.id, sec()).unwrap();
    let rows = owner
        .st
        .list(&Filter {
            kinds: FOLLOW_KINDS.iter().map(|k| k.to_string()).collect(),
            limit: store::MAX_LIST_LIMIT,
            ..Filter::default()
        })
        .unwrap();
    const FAR: &str = "dev-b/far-session";
    let persons: KeyFn = Arc::new(|k: &str| k == "user:dev-a" || k == "user:dev-b");
    let fold = |remote_first: bool| -> (Ask, Vec<Notice>) {
        let e = Env::with("dev-b", Some(persons.clone()));
        let feed = |rows: &[Event]| {
            for ev in rows {
                e.st.insert(ev).unwrap();
                e.c.handle(ev).unwrap();
            }
        };
        if remote_first {
            feed(&rows);
            want_err(
                e.c.ask_claim(FAR, &tk.id, Duration::from_millis(10)),
                ErrorKind::Held,
            );
        } else {
            feed(&rows[..1]);
            e.write_raw(
                &format!("agent:{FAR}"),
                kinds::TAKE_REQUESTED,
                &TakeRequestedRow {
                    task_id: tk.id.clone(),
                    by: FAR.into(),
                    op: TAKE_CLAIM.into(),
                    at: fixed_now(),
                },
            );
            assert_eq!(ask_of(&e.c, &tk.id).waiting.len(), 1);
            feed(&rows[1..]);
        }
        let ns = e.c.sweep_asks(&[]).unwrap();
        (ask_of(&e.c, &tk.id), ns)
    };
    let (a1, n1) = fold(true);
    let (a2, n2) = fold(false);
    let want = ask_of(&owner.c, &tk.id);
    for mut a in [a1, a2] {
        a.waiting.clear();
        assert_eq!(a, want);
    }
    assert!(n1.is_empty(), "{n1:?}");
    assert!(
        n2.len() == 1
            && n2[0].kind == NOTICE_TAKEN
            && n2[0].to[0] == FAR
            && n2[0].text.contains(&format!("{ALICE} took")),
        "{n2:?}"
    );
}

#[test]
fn test_person_comment_on_an_open_ask_goes_to_its_holder() {
    let e = Env::new();
    let (tk, _) = post_ask(&e, "check the logs");
    let (_, ns) =
        e.c.person_comment(USER, &tk.id, "also the cron ones", false, &[], None)
            .unwrap();
    assert!(ns.len() == 1 && ns[0].to == [ALICE], "{ns:?}");
    // The person's comment isn't the holder acting on it: still handed.
    assert!(e.c.sweep_asks(&agents(&e)).unwrap().is_empty());
    assert!(ask_of(&e.c, &tk.id).handed);
    // Nobody idle: the room is nobody, and nobody is told.
    for k in [ALICE, BOB, CAROL] {
        e.ros.set(k, "busy");
    }
    let (t2, _) = post_ask(&e, "and the disk");
    let (_, ns) =
        e.c.person_comment(USER, &t2.id, "the big one", false, &[], None)
            .unwrap();
    assert!(ns.iter().all(|n| n.to.is_empty()), "{ns:?}");
}

#[test]
fn test_a_takers_close_before_the_take_on_a_third_device() {
    let persons: KeyFn =
        Arc::new(|k: &str| k == "user:dev-a" || k == "user:dev-b" || k == "user:dev-c");
    let owner = Env::with("dev-a", Some(persons.clone()));
    let (tk, _) = post_unheard(&owner, "the deploy is flaky");
    const FAR: &str = "dev-b/far-session";
    let actor = format!("agent:{FAR}");
    let at = hlc_at(plus(fixed_now(), mins(1)));
    owner.remote(
        "dev-b",
        1,
        at,
        &actor,
        kinds::TAKE_REQUESTED,
        &TakeRequestedRow {
            task_id: tk.id.clone(),
            by: FAR.into(),
            op: TAKE_CLAIM.into(),
            at: fixed_now(),
        },
    );
    assert_eq!(ask_of(&owner.c, &tk.id).taker, FAR);
    let title = "Retry the deploy step";
    let later = hlc_at(plus(fixed_now(), mins(2)));
    owner.remote(
        "dev-b",
        2,
        later,
        &actor,
        kinds::TASK_EDITED,
        &TaskEditedRow {
            task_id: tk.id.clone(),
            title: Some(title.into()),
            by: FAR.into(),
            at: fixed_now(),
            ..TaskEditedRow::default()
        },
    );
    owner.remote(
        "dev-b",
        3,
        later + 1,
        &actor,
        kinds::TASK_STATUS,
        &TaskStateRow {
            task_id: tk.id.clone(),
            state: WORKING.into(),
            note: "a card".into(),
            by: FAR.into(),
            at: fixed_now(),
            outcome: Some(Outcome {
                kind: OUTCOME_CARD.into(),
                note: "a card".into(),
                ..Outcome::default()
            }),
            ..TaskStateRow::default()
        },
    );
    owner.remote(
        "dev-b",
        4,
        later + 2,
        &actor,
        kinds::TASK_CLAIMED,
        &TaskClaimedRow {
            task_id: tk.id.clone(),
            by: FAR.into(),
            at: fixed_now(),
        },
    );
    let want = owner.c.get_task(&tk.id).unwrap();
    assert!(
        !want.ask.as_ref().unwrap().is_open()
            && want.title == title
            && want.holder == FAR
            && want.state == WORKING,
        "{want:?}"
    );
    let rows = owner
        .st
        .list(&Filter {
            kinds: FOLLOW_KINDS.iter().map(|k| k.to_string()).collect(),
            limit: store::MAX_LIST_LIMIT,
            ..Filter::default()
        })
        .unwrap();
    let third = Env::with("dev-c", Some(persons));
    let feed = |keep: &dyn Fn(&Event) -> bool| {
        for ev in rows.iter().filter(|ev| keep(ev)) {
            third.st.insert(ev).unwrap();
            third.c.handle(ev).unwrap();
        }
    };
    feed(&|ev| ev.device_id == "dev-a" && ev.kind == kinds::TASK_CREATED);
    feed(&|ev| ev.device_id == "dev-b");
    let got = third.c.get_task(&tk.id).unwrap();
    assert!(
        got.ask.as_ref().unwrap().is_open() && got.title != title,
        "{got:?}"
    );
    feed(&|ev| ev.device_id == "dev-a" && ev.kind != kinds::TASK_CREATED);
    let got = third.c.get_task(&tk.id).unwrap();
    assert!(
        !got.ask.as_ref().unwrap().is_open()
            && got.title == title
            && got.holder == FAR
            && got.state == WORKING,
        "{got:?}"
    );
    assert_eq!(
        got.ask.as_ref().unwrap().outcome,
        want.ask.as_ref().unwrap().outcome
    );
    let r = third.reopen().get_task(&tk.id).unwrap();
    assert!(
        r.title == got.title
            && r.state == got.state
            && r.holder == got.holder
            && r.ask.unwrap().outcome == got.ask.unwrap().outcome
    );
}

#[test]
fn test_edits_need_the_taker() {
    let e = Env::new();
    let other =
        e.c.create_task(
            BOB,
            NewTask {
                title: "Ship docs".into(),
                objective: "old".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    let (tk, _) = post_ask(&e, "docs should mention Windows");
    e.c.ask_claim(ALICE, &tk.id, sec()).unwrap();
    let bad = "rewritten";
    let actor = format!("agent:{CAROL}");
    for r in [
        TaskEditedRow {
            task_id: other.id.clone(),
            objective: Some(bad.into()),
            by: CAROL.into(),
            from: tk.id.clone(),
            at: fixed_now(),
            ..TaskEditedRow::default()
        },
        TaskEditedRow {
            task_id: tk.id.clone(),
            title: Some(bad.into()),
            by: CAROL.into(),
            at: fixed_now(),
            ..TaskEditedRow::default()
        },
        // Any agent may edit a card that isn't an open ask (card #320).
        TaskEditedRow {
            task_id: other.id.clone(),
            title: Some("Ship the docs".into()),
            by: CAROL.into(),
            at: fixed_now(),
            ..TaskEditedRow::default()
        },
    ] {
        e.write_raw(&actor, kinds::TASK_EDITED, &r);
    }
    for id in [&other.id, &tk.id] {
        let got = e.c.get_task(id).unwrap();
        assert!(got.title != bad && got.objective != bad, "{got:?}");
    }
    assert_eq!(e.c.get_task(&other.id).unwrap().title, "Ship the docs");
    let got = e.reopen().get_task(&other.id).unwrap();
    assert!(got.objective != bad && got.title == "Ship the docs");
    let (open, _) = post_ask(&e, "another ask");
    want_err(
        e.c.ask_close(
            ALICE,
            &tk.id,
            AskOutcome {
                kind: OUTCOME_EDIT.into(),
                task: open.id.clone(),
                note: "x".into(),
                title: bad.into(),
                ..AskOutcome::default()
            },
        ),
        ErrorKind::NotAllowed,
    );
    let done = e.c.create_task(BOB, nt("Old work")).unwrap();
    e.c.set_task_state(USER, &done.id, COMPLETED, "").unwrap();
    want_err(
        e.c.ask_close(
            ALICE,
            &tk.id,
            AskOutcome {
                kind: OUTCOME_EDIT.into(),
                task: done.id.clone(),
                note: "x".into(),
                title: bad.into(),
                ..AskOutcome::default()
            },
        ),
        ErrorKind::BadState,
    );
    e.c.ask_close(
        ALICE,
        &tk.id,
        AskOutcome {
            kind: OUTCOME_EDIT.into(),
            task: other.id.clone(),
            note: "added Windows".into(),
            objective: "new".into(),
            ..AskOutcome::default()
        },
    )
    .unwrap();
    assert_eq!(e.c.get_task(&other.id).unwrap().objective, "new");
}

#[test]
fn test_a_given_back_ask_goes_to_the_next_idle_agent() {
    let e = Env::new();
    e.ros.set(BOB, "busy");
    e.ros.set(CAROL, "busy");
    let (tk, n) = post_ask(&e, "wake for this");
    assert!(n.wake && n.to == [ALICE]);
    // Held by ALICE: an agent going idle hears nothing.
    e.ros.set(BOB, "idle");
    assert!(e.c.sweep_asks(&agents(&e)).unwrap().is_empty());
    e.c.leave(ALICE, &tk.id, ALICE).unwrap();
    let ns = e.c.sweep_asks(&agents(&e)).unwrap();
    assert!(
        ns.len() == 1 && ns[0].wake && ns[0].to == [BOB] && ns[0].text.contains("gave it back"),
        "{ns:?}"
    );
    // The agent that gave it back isn't handed it again.
    e.c.leave(BOB, &tk.id, BOB).unwrap();
    let ns = e.c.sweep_asks(&agents(&e)).unwrap();
    assert!(ns.is_empty(), "{ns:?}");
    e.ros.set(CAROL, "idle");
    let ns = e.c.sweep_asks(&agents(&e)).unwrap();
    assert!(ns.len() == 1 && ns[0].to == [CAROL], "{ns:?}");
}

#[test]
fn test_a_failed_sweep_keeps_the_retell() {
    let e = Env::new();
    let (tk, _) = post_ask(&e, "look at the logs");
    e.c.ask_claim(ALICE, &tk.id, sec()).unwrap();
    e.c.leave(ALICE, &tk.id, ALICE).unwrap();
    e.st.hold_appends(1 << 40);
    assert!(e.c.sweep_asks(&agents(&e)).is_err());
    e.st.hold_appends(0);
    let ns = e.c.sweep_asks(&agents(&e)).unwrap();
    assert!(ns.len() == 1 && ns[0].to == [BOB], "{ns:?}");
}

#[test]
fn test_ask_line() {
    let e = Env::new();
    let (tk, _) = post_ask(&e, "look");
    assert_eq!(
        e.c.get_task(&tk.id).unwrap().ask_line(),
        format!(
            "the person asked the room; Irori handed it to {ALICE}, who holds it (ask_close ends it with what it became; board_leave passes it on)"
        )
    );
    e.c.ask_claim(ALICE, &tk.id, sec()).unwrap();
    assert_eq!(
        e.c.get_task(&tk.id).unwrap().ask_line(),
        format!(
            "the person asked the room; {ALICE} took it (ask_close ends it with what it became)"
        )
    );
    let t =
        e.c.ask_close(
            ALICE,
            &tk.id,
            AskOutcome {
                kind: OUTCOME_ANSWERED.into(),
                note: "v1".into(),
                ..AskOutcome::default()
            },
        )
        .unwrap();
    assert_eq!(t.ask_line(), "closed (answered): v1");
}

/// Card #265: the person's ask may be far longer than a comment cap, and
/// its objective keeps all of it.
#[test]
fn test_long_ask() {
    let e = Env::new();
    let long = format!("look at this log:\n{}", "line of log\n".repeat(2000));
    let (t, _) = post_ask(&e, &long);
    assert_eq!(t.objective, long.trim());
    assert!(t.objective.chars().count() > 20_000);
}

/// Card #408: a device with no agents of its own (the phone) leaves its ask
/// to one of the person's computers. It writes no hand-out itself: its
/// roster may be hours old, and it sleeps right after.
#[test]
fn test_a_phone_ask_belongs_to_a_computer() {
    let e = Env::with("dev-p", Some(person()));
    // The roster: the Mac's three agents, nothing on the phone.
    let (tk, n) = post_ask(&e, "This post is about a new 3D model?");
    let a = ask_of(&e.c, &tk.id);
    assert!(
        a.owner == "dev-a" && a.taker.is_empty() && !a.handed,
        "{a:?}"
    );
    assert!(n.to.is_empty(), "{n:?}");
    assert!(e.rows(kinds::TAKE_STATE).is_empty());
    // The phone's own sweeps leave it to the Mac.
    e.clk.add(GONE_GRACE * 10);
    assert!(e.c.sweep_asks(&agents(&e)).unwrap().is_empty());
    assert!(ask_of(&e.c, &tk.id).taker.is_empty());
    // A device that runs agents keeps its own asks, as before.
    let mac = Env::with("dev-a", Some(person()));
    let (tk, _) = post_ask(&mac, "Why is CI red?");
    let a = ask_of(&mac.c, &tk.id);
    assert!(a.owner == "dev-a" && a.taker == ALICE && a.handed, "{a:?}");
}

/// Card #408 (ask #390b): the phone's ask reaches the Mac, which hands it
/// out with its own roster and, when the agent it went to goes away, on to
/// the next idle one; a thread as much as an ask.
#[test]
fn test_the_computer_hands_a_phone_ask_out_and_on() {
    for thread in [false, true] {
        let e = Env::with("dev-a", Some(person()));
        let at = plus(fixed_now(), sec());
        let id = "01M6HPNJ2VWDNPD5FPHVFK4Q0D";
        e.remote(
            "dev-p",
            1,
            hlc_at(at),
            "user",
            kinds::TASK_CREATED,
            &TaskCreatedRow {
                task_id: id.into(),
                num: 7,
                title: "This post is about a new 3D model?".into(),
                objective: "This post is about a new 3D model?".into(),
                ask: Some(AskField {
                    wake: true,
                    thread,
                    owner: "dev-a".into(),
                    ..AskField::default()
                }),
                by: "user:dev-p".into(),
                at,
                ..TaskCreatedRow::default()
            },
        );
        let a = ask_of(&e.c, id);
        assert!(a.owner == "dev-a" && a.taker.is_empty(), "{a:?}");
        let ns = e.c.sweep_asks(&agents(&e)).unwrap();
        let a = ask_of(&e.c, id);
        assert!(a.taker == ALICE && a.handed, "{a:?}");
        assert!(ns.len() == 1 && ns[0].to == [ALICE], "{ns:?}");
        // ALICE goes away: the next idle agent gets it.
        e.ros.set(ALICE, "");
        let ns = e.c.sweep_asks(&agents(&e)).unwrap();
        let a = ask_of(&e.c, id);
        assert!(
            a.taker == BOB && a.handed && a.heard == [ALICE, BOB],
            "{a:?}"
        );
        assert!(
            ns.len() == 1 && ns[0].to == [BOB] && (thread || ns[0].text.contains("went away")),
            "{ns:?}"
        );
    }
}

/// Card #408: an owner named by another person's device isn't taken on a
/// board; privately every device is the person's.
#[test]
fn test_ask_owner_needs_the_posting_device() {
    let e = Env::with("dev-a", Some(person()));
    let at = plus(fixed_now(), sec());
    let id = "01M6HPNJ2VWDNPD5FPHVFK4Q0E";
    e.remote(
        "dev-p",
        1,
        hlc_at(at),
        "user",
        kinds::TASK_CREATED,
        &TaskCreatedRow {
            task_id: id.into(),
            num: 8,
            title: "x".into(),
            objective: "x".into(),
            ask: Some(AskField {
                wake: true,
                ..AskField::default()
            }),
            by: "user:dev-p".into(),
            at,
            ..TaskCreatedRow::default()
        },
    );
    // No owner named: the posting device's, as before.
    assert_eq!(ask_of(&e.c, id).owner, "dev-p");
}

/// The person on any of their devices (the phone's user key too).
fn person() -> KeyFn {
    Arc::new(|k: &str| k.starts_with("user:"))
}
