//! #91: an ask goes to one idle agent, which holds it as it wakes, and moves
//! on only by events: it gives it back, its turn ends without touching it,
//! or it goes away. No sweep here advances by time alone.

use crate::common;

use std::time::Duration;

use common::*;
use coord::roster::AgentCard;
use coord::*;
use gocompat::time::GoTime;
use store::kinds;

/// An agent card as the roster gives it: `idle_since` is when its status
/// last changed, `active` its newest transcript row.
fn card(key: &str, status: &str, idle_since: GoTime, active: GoTime) -> AgentCard {
    AgentCard {
        key: key.into(),
        device_id: "dev-a".into(),
        status: status.into(),
        updated_at: idle_since,
        last_activity: active,
        ..AgentCard::default()
    }
}

fn idle(key: &str, since_min: u64) -> AgentCard {
    let at = GoTime::unix(fixed_now().secs - 60 * (60 - since_min as i64), 0);
    card(key, "idle", at, at)
}

/// Three idle agents, ALICE idle longest.
fn three() -> Vec<AgentCard> {
    vec![idle(BOB, 20), idle(CAROL, 30), idle(ALICE, 10)]
}

fn post(e: &Env, text: &str, agents: &[AgentCard]) -> (Task, Notice) {
    e.c.post_ask(
        USER,
        NewAsk {
            text: text.into(),
            ..NewAsk::default()
        },
        agents,
    )
    .unwrap()
}

fn ask_of(c: &Coord, id: &str) -> Ask {
    c.get_task(id).and_then(|t| t.ask).unwrap()
}

/// `key`'s card after a turn that ran and ended after the hand-out.
fn turn_ended(key: &str) -> AgentCard {
    let later = plus(fixed_now(), mins(2));
    card(key, "idle", later, later)
}

fn with(mut all: Vec<AgentCard>, c: AgentCard) -> Vec<AgentCard> {
    all.retain(|a| a.key != c.key);
    all.push(c);
    all
}

#[test]
fn three_idle_agents_one_is_woken_the_one_idle_longest() {
    let e = Env::new();
    let (tk, n) = post(&e, "Why is CI red?", &three());
    assert!(n.to == [ALICE] && n.wake && n.kind == NOTICE_ASK, "{n:?}");
    let a = ask_of(&e.c, &tk.id);
    assert!(
        a.taker == ALICE && a.handed && a.told == 1 && a.heard == [ALICE],
        "{a:?}"
    );
    let t = e.c.get_task(&tk.id).unwrap();
    assert_eq!(t.holders, [ALICE]);
    // It holds it as it wakes: its claim is a no-op answer, the others are held off.
    assert!(e.c.ask_claim(ALICE, &tk.id, Duration::ZERO).unwrap().mine);
    want_err(e.c.ask_claim(BOB, &tk.id, Duration::ZERO), ErrorKind::Held);
    let acts = e.c.activity(&tk.id).unwrap();
    assert!(
        acts.iter()
            .any(|x| x.what == format!("handed it to {ALICE}")),
        "{acts:?}"
    );
}

#[test]
fn nothing_moves_on_time_alone() {
    let e = Env::new();
    let (tk, _) = post(&e, "Why is CI red?", &three());
    let rows = e.rows(kinds::TAKE_STATE).len();
    for _ in 0..5 {
        e.clk.add(Duration::from_secs(24 * 3600));
        assert!(e.c.sweep_asks(&three()).unwrap().is_empty());
    }
    assert_eq!(e.rows(kinds::TAKE_STATE).len(), rows);
    let a = ask_of(&e.c, &tk.id);
    assert!(a.taker == ALICE && a.handed && !a.unclaimed, "{a:?}");
}

#[test]
fn a_turn_ended_untouched_hands_it_to_the_next() {
    let e = Env::new();
    let (tk, _) = post(&e, "Why is CI red?", &three());
    // Woken: busy on it. Nothing moves while the turn runs.
    let busy = with(
        three(),
        card(
            ALICE,
            "busy",
            plus(fixed_now(), mins(1)),
            plus(fixed_now(), mins(1)),
        ),
    );
    assert!(e.c.sweep_asks(&busy).unwrap().is_empty());
    assert_eq!(ask_of(&e.c, &tk.id).taker, ALICE);
    // The turn ends without a claim, a comment or a close: BOB next.
    let ns = e.c.sweep_asks(&with(three(), turn_ended(ALICE))).unwrap();
    assert!(
        ns.len() == 1 && ns[0].to == [BOB] && ns[0].wake && ns[0].text.contains("ended its turn"),
        "{ns:?}"
    );
    let a = ask_of(&e.c, &tk.id);
    assert!(
        a.taker == BOB && a.handed && a.heard == [ALICE, BOB],
        "{a:?}"
    );
    let acts = e.c.activity(&tk.id).unwrap();
    assert!(
        acts.iter()
            .any(|x| x.by == ALICE && x.what.starts_with("passed it on")),
        "{acts:?}"
    );
    // BOB and CAROL pass too: nobody is left, it waits unclaimed.
    let all_ended = vec![turn_ended(ALICE), turn_ended(BOB), turn_ended(CAROL)];
    e.clk.add(mins(3));
    let later = plus(fixed_now(), mins(10));
    let ns =
        e.c.sweep_asks(&with(all_ended.clone(), card(BOB, "idle", later, later)))
            .unwrap();
    assert!(ns.len() == 1 && ns[0].to == [CAROL], "{ns:?}");
    e.clk.add(mins(10));
    let latest = plus(fixed_now(), mins(30));
    let done = vec![
        turn_ended(ALICE),
        turn_ended(BOB),
        card(CAROL, "idle", latest, latest),
    ];
    assert!(e.c.sweep_asks(&done).unwrap().is_empty());
    let a = ask_of(&e.c, &tk.id);
    assert!(
        a.taker.is_empty() && a.unclaimed && a.heard == [ALICE, BOB, CAROL],
        "{a:?}"
    );
    // A new agent going idle gets it.
    const DAN: &str = "dev-a/dan-session";
    let ns = e.c.sweep_asks(&with(done, idle(DAN, 1))).unwrap();
    assert!(ns.len() == 1 && ns[0].to == [DAN], "{ns:?}");
}

#[test]
fn touching_it_keeps_it() {
    // A comment from the holder.
    let e = Env::new();
    let (tk, _) = post(&e, "Why is CI red?", &three());
    e.clk.add(mins(1));
    e.c.add_comment(ALICE, &tk.id, "Looking: which branch?", false)
        .unwrap();
    assert!(
        e.c.sweep_asks(&with(three(), turn_ended(ALICE)))
            .unwrap()
            .is_empty()
    );
    let a = ask_of(&e.c, &tk.id);
    assert!(a.taker == ALICE && !a.handed, "{a:?}");
    // A claim.
    let e = Env::new();
    let (tk, _) = post(&e, "Why is CI red?", &three());
    assert!(e.c.ask_claim(ALICE, &tk.id, Duration::ZERO).unwrap().mine);
    assert!(!ask_of(&e.c, &tk.id).handed);
    assert!(
        e.c.sweep_asks(&with(three(), turn_ended(ALICE)))
            .unwrap()
            .is_empty()
    );
    assert_eq!(ask_of(&e.c, &tk.id).taker, ALICE);
    // A close.
    let (t2, _) = post(&e, "And the docs?", &three());
    assert_eq!(ask_of(&e.c, &t2.id).taker, BOB, "ALICE holds the first");
    e.c.ask_close(
        BOB,
        &t2.id,
        AskOutcome {
            kind: OUTCOME_ANSWERED.into(),
            note: "answered on the card".into(),
            ..AskOutcome::default()
        },
    )
    .unwrap();
    assert!(
        e.c.sweep_asks(&with(three(), turn_ended(BOB)))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn giving_it_back_hands_it_on() {
    let e = Env::new();
    let (tk, _) = post(&e, "Why is CI red?", &three());
    e.c.leave(ALICE, &tk.id, ALICE).unwrap();
    let ns = e.c.sweep_asks(&three()).unwrap();
    assert!(
        ns.len() == 1 && ns[0].to == [BOB] && ns[0].text.contains("gave it back"),
        "{ns:?}"
    );
    assert!(ask_of(&e.c, &tk.id).handed);
}

#[test]
fn going_away_or_offline_hands_it_on() {
    for gone in [
        None,
        Some(card(ALICE, "gone", fixed_now(), fixed_now())),
        Some(AgentCard {
            device_state: "offline".into(),
            ..idle(ALICE, 10)
        }),
    ] {
        let e = Env::new();
        let (tk, _) = post(&e, "Why is CI red?", &three());
        let mut now = vec![idle(BOB, 20), idle(CAROL, 30)];
        now.extend(gone.clone());
        let ns = e.c.sweep_asks(&now).unwrap();
        assert!(
            ns.len() == 1 && ns[0].to == [BOB] && ns[0].text.contains("went away"),
            "{gone:?}: {ns:?}"
        );
        assert_eq!(ask_of(&e.c, &tk.id).taker, BOB);
    }
}

#[test]
fn nobody_idle_then_one_goes_idle() {
    let e = Env::new();
    let busy: Vec<AgentCard> = three()
        .into_iter()
        .map(|c| AgentCard {
            status: "busy".into(),
            ..c
        })
        .collect();
    let (tk, n) = post(&e, "Why is CI red?", &busy);
    assert!(n.to.is_empty() && ask_of(&e.c, &tk.id).unclaimed);
    assert!(e.c.sweep_asks(&busy).unwrap().is_empty());
    let ns = e.c.sweep_asks(&with(busy, idle(CAROL, 59))).unwrap();
    assert!(ns.len() == 1 && ns[0].to == [CAROL] && ns[0].wake, "{ns:?}");
}

#[test]
fn one_ask_per_agent_and_same_repo_only() {
    let e = Env::new();
    let (t1, _) = post(&e, "first", &three());
    let (t2, _) = post(&e, "second", &three());
    assert_eq!(ask_of(&e.c, &t1.id).taker, ALICE);
    assert_eq!(ask_of(&e.c, &t2.id).taker, BOB);
    let mut agents = three();
    agents[1].repo_key = REPO_B.into(); // CAROL
    let (t3, _) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "third".into(),
                repo_key: REPO_B.into(),
                ..NewAsk::default()
            },
            &agents,
        )
        .unwrap();
    assert_eq!(ask_of(&e.c, &t3.id).taker, CAROL);
}

/// Card #423: an ask can't wait on other work, so hand-out never has one
/// blocked on a task to skip: blocking an open ask is refused (it becomes
/// a card first, and a card waiting on a task isn't handed out, #397).
#[test]
fn an_open_ask_cannot_be_blocked_on_a_task() {
    let e = Env::new();
    let first =
        e.c.create_task(
            CAROL,
            NewTask {
                title: "Fix the runner".into(),
                objective: "CI needs it".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    let (tk, _) = post(&e, "Why is CI red?", &three());
    want_err(
        e.c.block(
            ALICE,
            &tk.id,
            "waits on the runner fix",
            Some(BlockedBy {
                kind: BLOCKED_ON_TASK.into(),
                reference: first.id.clone(),
            }),
        ),
        ErrorKind::NotAllowed,
    );
}
