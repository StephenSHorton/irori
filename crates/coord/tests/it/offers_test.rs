//! Card #82: an agent posts a card unheld and Irori hands it to one free
//! agent at a time, the way it hands out asks (#91): the one idle longest
//! gets it and takes it with board_join; it moves on only on an event (it
//! gives it back, its turn ends without taking it, or it goes away), never
//! on time. An agent busy on a working card is told no rather than taking
//! a second one.

use crate::common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use coord::roster::AgentCard;
use coord::*;
use gocompat::time::GoTime;
use store::kinds;

const DAVE: &str = "dev-a/dave-session";

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        objective: "why it matters".into(),
        ..NewTask::default()
    }
}

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

/// ALICE posts; BOB, CAROL and DAVE are idle, CAROL idle longest.
fn room() -> Vec<AgentCard> {
    vec![
        idle(ALICE, 5),
        idle(BOB, 20),
        idle(CAROL, 10),
        idle(DAVE, 30),
    ]
}

fn turn_ended(key: &str) -> AgentCard {
    let later = plus(fixed_now(), mins(2));
    card(key, "idle", later, later)
}

fn with(mut all: Vec<AgentCard>, c: AgentCard) -> Vec<AgentCard> {
    all.retain(|a| a.key != c.key);
    all.push(c);
    all
}

fn post(e: &Env, title: &str, agents: &[AgentCard]) -> (Task, Notice) {
    let t = e.c.create_task(ALICE, nt(title)).unwrap();
    let n = e.c.offer_card(ALICE, &t.id, agents).unwrap();
    (e.c.get_task(&t.id).unwrap(), n)
}

fn offer_of(c: &Coord, id: &str) -> Offer {
    c.get_task(id).and_then(|t| t.offer).unwrap()
}

#[test]
fn three_free_agents_one_is_woken_the_one_idle_longest() {
    let e = Env::new();
    let (t, n) = post(&e, "Port the parser", &room());
    assert!(
        n.to == [CAROL] && n.wake && n.kind == NOTICE_OFFER,
        "the poster is left out, CAROL idle longest: {n:?}"
    );
    assert!(n.text.contains("board_join #1"), "{}", n.text);
    assert!(n.text.contains("board_leave #1"), "{}", n.text);
    let o = offer_of(&e.c, &t.id);
    assert!(
        o.to == CAROL && o.reason == OFFER_HANDED && o.heard == [CAROL],
        "{o:?}"
    );
    assert!(t.holders.is_empty(), "handed, not held: {t:?}");
    let acts = e.c.activity(&t.id).unwrap();
    assert!(
        acts.iter()
            .any(|x| x.what == format!("handed it to {CAROL}")),
        "{acts:?}"
    );
    // CAROL takes it: the hand-out is over, nothing more to sweep.
    e.c.claim_task(CAROL, &t.id).unwrap();
    assert!(e.c.get_task(&t.id).unwrap().live_offer().is_none());
    assert!(
        e.c.sweep_asks(&with(room(), turn_ended(CAROL)))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn nothing_moves_on_time_alone() {
    let e = Env::new();
    let (t, _) = post(&e, "Port the parser", &room());
    let rows = e.rows(kinds::TASK_OFFERED).len();
    for _ in 0..5 {
        e.clk.add(Duration::from_secs(24 * 3600));
        assert!(e.c.sweep_asks(&room()).unwrap().is_empty());
    }
    assert_eq!(e.rows(kinds::TASK_OFFERED).len(), rows);
    assert_eq!(offer_of(&e.c, &t.id).to, CAROL);
}

#[test]
fn a_turn_ended_without_taking_it_hands_it_to_the_next() {
    let e = Env::new();
    let (t, _) = post(&e, "Port the parser", &room());
    // Woken and busy on its turn: nothing moves while it runs.
    let later = plus(fixed_now(), mins(1));
    let busy = with(room(), card(CAROL, "busy", later, later));
    assert!(e.c.sweep_asks(&busy).unwrap().is_empty());
    // Its turn ends with no board_join: BOB next (idle longest of the rest).
    let ns = e.c.sweep_asks(&with(room(), turn_ended(CAROL))).unwrap();
    assert!(
        ns.len() == 1 && ns[0].to == [BOB] && ns[0].text.contains("ended its turn"),
        "{ns:?}"
    );
    let o = offer_of(&e.c, &t.id);
    assert!(o.to == BOB && o.heard == [CAROL, BOB], "{o:?}");
    let acts = e.c.activity(&t.id).unwrap();
    assert!(
        acts.iter()
            .any(|x| x.by == CAROL && x.what == "passed it on"),
        "{acts:?}"
    );
    // BOB and DAVE pass too: nobody left (the poster is never handed its
    // own card), so it waits.
    let ended: Vec<AgentCard> = vec![
        idle(ALICE, 5),
        turn_ended(BOB),
        turn_ended(CAROL),
        idle(DAVE, 30),
    ];
    let ns = e.c.sweep_asks(&ended).unwrap();
    assert!(ns.len() == 1 && ns[0].to == [DAVE], "{ns:?}");
    e.clk.add(mins(3));
    let all = vec![
        idle(ALICE, 5),
        turn_ended(BOB),
        turn_ended(CAROL),
        turn_ended(DAVE),
    ];
    let all: Vec<AgentCard> = all
        .into_iter()
        .map(|mut a| {
            if a.key == DAVE {
                a.updated_at = plus(fixed_now(), mins(10));
                a.last_activity = a.updated_at;
            }
            a
        })
        .collect();
    assert!(e.c.sweep_asks(&all).unwrap().is_empty());
    let o = offer_of(&e.c, &t.id);
    assert!(o.to.is_empty() && o.reason == OFFER_WAITING, "{o:?}");
    assert!(o.heard.contains(&DAVE.to_string()), "{o:?}");
    // Waiting writes nothing more on later sweeps.
    let rows = e.rows(kinds::TASK_OFFERED).len();
    assert!(e.c.sweep_asks(&all).unwrap().is_empty());
    assert_eq!(e.rows(kinds::TASK_OFFERED).len(), rows);
    // A new agent goes idle: it goes to it.
    const ERIN: &str = "dev-a/erin-session";
    let ns = e.c.sweep_asks(&with(all, idle(ERIN, 1))).unwrap();
    assert!(ns.len() == 1 && ns[0].to == [ERIN], "{ns:?}");
}

#[test]
fn giving_it_back_hands_it_on() {
    let e = Env::new();
    let (t, _) = post(&e, "Port the parser", &room());
    // CAROL gives it back with board_leave though it never held it.
    e.c.leave(CAROL, &t.id, CAROL).unwrap();
    let o = offer_of(&e.c, &t.id);
    assert!(o.to.is_empty() && o.reason == OFFER_PASSED, "{o:?}");
    let ns = e.c.sweep_asks(&room()).unwrap();
    assert!(
        ns.len() == 1 && ns[0].to == [BOB] && ns[0].text.contains("gave it back"),
        "{ns:?}"
    );
    // Someone it wasn't handed to can't pass it on.
    want_err(e.c.leave(DAVE, &t.id, DAVE), ErrorKind::NotFound);
}

#[test]
fn going_away_or_offline_hands_it_on() {
    let e = Env::new();
    let (t, _) = post(&e, "Port the parser", &room());
    let mut gone = room();
    gone.retain(|a| a.key != CAROL);
    let ns = e.c.sweep_asks(&gone).unwrap();
    assert!(
        ns.len() == 1 && ns[0].to == [BOB] && ns[0].text.contains("went away"),
        "{ns:?}"
    );
    let mut off = idle(BOB, 20);
    off.device_state = "offline".into();
    let ns = e.c.sweep_asks(&with(gone, off)).unwrap();
    assert!(ns.len() == 1 && ns[0].to == [DAVE], "{ns:?}");
    assert_eq!(offer_of(&e.c, &t.id).heard, [CAROL, BOB, DAVE]);
}

#[test]
fn nobody_free_then_one_goes_idle() {
    let e = Env::new();
    let later = plus(fixed_now(), mins(1));
    let busy: Vec<AgentCard> = vec![idle(ALICE, 5), card(BOB, "busy", later, later)];
    let (t, n) = post(&e, "Port the parser", &busy);
    assert!(n.to.is_empty(), "{n:?}");
    let o = offer_of(&e.c, &t.id);
    assert!(o.to.is_empty() && o.reason == OFFER_WAITING, "{o:?}");
    assert!(e.c.sweep_asks(&busy).unwrap().is_empty());
    let ns = e.c.sweep_asks(&[idle(ALICE, 5), idle(BOB, 50)]).unwrap();
    assert!(ns.len() == 1 && ns[0].to == [BOB], "{ns:?}");
}

#[test]
fn a_busy_agent_is_not_handed_a_card_and_cannot_take_a_second() {
    let e = Env::new();
    // CAROL works a card; DAVE owns an open step on one.
    let w = e.c.create_task(CAROL, nt("CAROL's card")).unwrap();
    e.c.claim_task(CAROL, &w.id).unwrap();
    let s = e.c.create_task(BOB, nt("BOB's card")).unwrap();
    e.c.claim_task(BOB, &s.id).unwrap();
    e.c.set_steps(
        BOB,
        &s.id,
        vec![Step {
            text: "a step for dave".into(),
            owner: DAVE.into(),
            ..Step::default()
        }],
    )
    .unwrap();
    e.c.leave(BOB, &s.id, BOB).unwrap();
    e.c.claim_task(ALICE, &s.id).unwrap();
    let (t, n) = post(
        &e,
        "Port the parser",
        &[
            idle(ALICE, 5),
            idle(BOB, 20),
            idle(CAROL, 10),
            idle(DAVE, 30),
        ],
    );
    assert_eq!(n.to, [BOB], "CAROL works a card, DAVE owns a step: {n:?}");
    // CAROL tries to take it anyway: told no.
    let err = e.c.claim_task(CAROL, &t.id).unwrap_err();
    assert!(err.to_string().contains("one card at a time"), "{err}");
    // An agent with a card handed to it isn't handed a second card.
    let (_, n2) = post(&e, "And the lexer", &[idle(ALICE, 5), idle(BOB, 20)]);
    assert!(n2.to.is_empty(), "{n2:?}");
}

#[test]
fn same_repo_only_and_not_on_a_held_or_assigned_card() {
    let e = Env::new();
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                repo_key: REPO_A.into(),
                ..nt("Port the parser")
            },
        )
        .unwrap();
    let mut other = idle(DAVE, 30);
    other.repo_key = REPO_B.into();
    let mut same = idle(BOB, 20);
    same.repo_key = REPO_A.into();
    let n = e.c.offer_card(ALICE, &t.id, &[other, same]).unwrap();
    assert_eq!(n.to, [BOB]);
    // A held card isn't handed out.
    let h = e.c.create_task(ALICE, nt("held")).unwrap();
    e.c.claim_task(ALICE, &h.id).unwrap();
    assert!(e.c.offer_card(ALICE, &h.id, &room()).is_err());
}

#[test]
fn a_stale_hand_out_notice_closes_before_it_wakes() {
    let e = Env::new();
    let (t, n) = post(&e, "Port the parser", &room());
    let r = format!("task:{}", t.id);
    assert_eq!(n.to, [CAROL]);
    assert_eq!(e.c.stale_offer(&r, CAROL), "");
    // CAROL gives it back and it goes to BOB: CAROL's notice is stale.
    e.c.leave(CAROL, &t.id, CAROL).unwrap();
    e.c.sweep_asks(&room()).unwrap();
    assert_eq!(
        e.c.stale_offer(&r, CAROL),
        "Irori handed it to another agent"
    );
    assert_eq!(e.c.stale_offer(&r, BOB), "");
    // The poster takes it after all: BOB's notice is stale too.
    e.c.claim_task(ALICE, &t.id).unwrap();
    assert!(
        e.c.stale_offer(&r, BOB).starts_with("taken by"),
        "{}",
        e.c.stale_offer(&r, BOB)
    );
}

// ---- card #340: the right machine first ----------------------------------

const MAC_FREE: &str = "dev-mac/mac-free-session";
const MAC_REPO: &str = "dev-mac/mac-repo-session";
const PC_FREE: &str = "dev-pc/pc-free-session";
const ONLY_ON_MAC: &str = "remote:github.com/example/mac-only";

fn on(key: &str, dev: &str, cwd: &str, status: &str, since_min: u64) -> AgentCard {
    let at = GoTime::unix(fixed_now().secs - 60 * (60 - since_min as i64), 0);
    AgentCard {
        device_id: dev.into(),
        cwd: cwd.into(),
        ..card(key, status, at, at)
    }
}

/// Everyone starts in the home folder (no repo key), as on the real Mac and
/// PC; the PC agent has been idle longest. One busy Mac agent works in the
/// Mac-only repo.
fn two_machines() -> Vec<AgentCard> {
    let mut repo_agent = on(
        MAC_REPO,
        "dev-mac",
        "/Users/s/projects/mac-only",
        "busy",
        50,
    );
    repo_agent.repo_key = ONLY_ON_MAC.into();
    vec![
        idle(ALICE, 5),
        on(MAC_FREE, "dev-mac", "/Users/s", "idle", 20),
        on(PC_FREE, "dev-pc", "C:\\Users\\s", "idle", 1),
        repo_agent,
    ]
}

fn post_new(e: &Env, t: NewTask, agents: &[AgentCard]) -> Notice {
    let t = e.c.create_task(ALICE, t).unwrap();
    e.c.offer_card(ALICE, &t.id, agents).unwrap()
}

#[test]
fn a_card_whose_repo_is_only_on_the_mac_goes_to_a_mac_agent_first() {
    let e = Env::new();
    let n = post_new(
        &e,
        NewTask {
            repo_key: ONLY_ON_MAC.into(),
            ..nt("Mac build of the bundle")
        },
        &two_machines(),
    );
    assert_eq!(n.to, [MAC_FREE], "the PC agent idle longer must not get it");
}

#[test]
fn needs_pc_goes_to_the_pc_and_needs_mac_to_the_mac() {
    let e = Env::new();
    let mut room = two_machines();
    room.push(on(BOB, "dev-mac", "/Users/s", "idle", 15));
    let n = post_new(
        &e,
        NewTask {
            objective: "Check the installer. needs:pc".into(),
            ..nt("Windows installer check")
        },
        &room,
    );
    assert_eq!(n.to, [PC_FREE]);
    let n = post_new(
        &e,
        NewTask {
            objective: "Notarize. Needs:Mac".into(),
            ..nt("Mac notarize check")
        },
        &room,
    );
    // BOB: the Mac agent idle longest.
    assert_eq!(n.to, [BOB]);
}

#[test]
fn a_card_with_no_machine_signal_still_goes_to_the_one_idle_longest() {
    let e = Env::new();
    let n = post_new(&e, nt("Anywhere work"), &two_machines());
    assert_eq!(n.to, [PC_FREE]);
}

/// Card #341: idle agents and no open work reads as a shortage, not a
/// dispatch gap.
#[test]
fn idle_report_idle_agents_and_no_open_work() {
    let e = Env::new();
    let r = e.c.idle_report(&room());
    assert_eq!(r.idle, 4, "{r:?}");
    assert!(r.unoffered.is_empty(), "{r:?}");
    // A held card is work in hand, not open work.
    let h = e.c.create_task(ALICE, nt("held")).unwrap();
    e.c.claim_task(ALICE, &h.id).unwrap();
    let r = e.c.idle_report(&room());
    assert!(r.idle == 3 && r.unoffered.is_empty(), "{r:?}");
}

/// Card #341, today's case: cards sat unheld while agents were around. A
/// card posted while nobody is free shows as not handed out, with why;
/// once an agent is idle the periodic sweep hands it out and it leaves the
/// report.
#[test]
fn idle_report_lists_unoffered_until_the_sweep_hands_it_out() {
    let e = Env::new();
    let later = plus(fixed_now(), mins(1));
    let busy: Vec<AgentCard> = vec![card(BOB, "busy", later, later)];
    let (t, n) = post(&e, "Port the parser", &busy);
    assert!(n.to.is_empty(), "{n:?}");
    let r = e.c.idle_report(&busy);
    assert_eq!(r.idle, 0);
    assert!(
        r.unoffered.len() == 1
            && r.unoffered[0].task_id == t.id
            && r.unoffered[0].why == "no agent is free",
        "{r:?}"
    );
    // BOB goes idle: the report says the next sweep hands it out, and the
    // sweep does.
    let free = vec![idle(BOB, 50)];
    let r = e.c.idle_report(&free);
    assert!(
        r.idle == 1 && r.unoffered[0].why.contains("next sweep"),
        "{r:?}"
    );
    let ns = e.c.sweep_asks(&free).unwrap();
    assert!(ns.len() == 1 && ns[0].to == [BOB] && ns[0].wake, "{ns:?}");
    let r = e.c.idle_report(&free);
    assert!(r.unoffered.is_empty(), "handed to BOB: {r:?}");
}

/// Card #341: idle agents but none can take it says why.
#[test]
fn idle_report_says_no_free_agent_works_in_the_repo() {
    let e = Env::new();
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                repo_key: REPO_A.into(),
                ..nt("Port the parser")
            },
        )
        .unwrap();
    let mut other = idle(DAVE, 30);
    other.repo_key = REPO_B.into();
    let n =
        e.c.offer_card(ALICE, &t.id, std::slice::from_ref(&other))
            .unwrap();
    assert!(n.to.is_empty(), "{n:?}");
    let r = e.c.idle_report(&[other]);
    assert!(
        r.idle == 1 && r.unoffered.len() == 1 && r.unoffered[0].why == "no free agent works in b",
        "{r:?}"
    );
}

/// Card #397: an unheld card blocked on another task isn't handed out
/// while that task is open. Blocked after a hand-out, the hand-out is
/// stale; once the task is done (the blocker closes by itself), the card
/// goes out once, as if just posted, to the one idle longest.
#[test]
fn a_card_blocked_on_another_task_waits_for_it_then_goes_out_once() {
    let e = Env::new();
    let first = e.c.create_task(BOB, nt("Talk 4")).unwrap();
    e.c.claim_task(BOB, &first.id).unwrap();
    e.c.set_task_state(BOB, &first.id, WORKING, "").unwrap();
    let room = vec![idle(ALICE, 5), idle(CAROL, 10), idle(DAVE, 30)];
    let (t, n) = post(&e, "Talk 6", &room);
    assert_eq!(n.to, [CAROL], "handed out before it was blocked");
    e.c.block(
        ALICE,
        &t.id,
        "waits on Talk 4",
        Some(BlockedBy {
            kind: BLOCKED_ON_TASK.into(),
            reference: first.id.clone(),
        }),
    )
    .unwrap();
    // The hand-out to CAROL no longer applies: its notice closes unread.
    assert!(
        !e.c.stale_offer(&format!("task:{}", t.id), CAROL).is_empty(),
        "CAROL's hand-out should go stale once the card is blocked"
    );
    // CAROL's turn ends: nobody is handed it while Talk 4 runs.
    let ended = vec![idle(ALICE, 5), turn_ended(CAROL), idle(DAVE, 30)];
    for _ in 0..3 {
        let ns = e.c.sweep_asks(&ended).unwrap();
        assert!(ns.iter().all(|n| n.kind != NOTICE_OFFER), "{ns:?}");
        e.c.sweep_notices().unwrap();
    }
    assert!(e.c.idle_report(&ended).unoffered.is_empty());
    // Talk 4 is done: the blocker closes, and the card goes out once.
    e.clk.add(mins(5));
    e.c.set_task_state(BOB, &first.id, COMPLETED, "done")
        .unwrap();
    e.c.sweep_notices().unwrap();
    let got = e.c.get_task(&t.id).unwrap();
    assert!(got.open_blockers().is_empty(), "{:?}", got.blockers);
    let ns = e.c.sweep_asks(&ended).unwrap();
    let offers: Vec<_> = ns.iter().filter(|n| n.kind == NOTICE_OFFER).collect();
    assert_eq!(offers.len(), 1, "{ns:?}");
    // A fresh hand-out, as if just posted: CAROL reached it while it
    // couldn't be started, which is no pass, so she isn't skipped for it;
    // her turn ended later, so DAVE is now idle longest.
    let o = offer_of(&e.c, &t.id);
    assert_eq!(offers[0].to, [DAVE], "{ns:?}");
    assert!(o.to == DAVE && o.heard == [DAVE], "a fresh hand-out: {o:?}");
    // And only once: the next sweep hands nothing.
    assert!(
        e.c.sweep_asks(&ended)
            .unwrap()
            .iter()
            .all(|n| n.kind != NOTICE_OFFER)
    );
}

/// Card #422: a working card in another of the Core's logs (a shared
/// board's, say) makes the agent busy here too: the hand-out skips it for
/// the next free agent, and board_join tells it no, naming that card.
#[test]
fn a_working_card_in_another_log_counts_as_busy() {
    let e = Env::new();
    let elsewhere: BusyFn =
        Arc::new(|k: &str| (k == CAROL).then(|| "wind#4 \"Merge desk\"".to_string()));
    let c = e.reopen_with(Options {
        busy_elsewhere: Some(elsewhere),
        ..e.opts()
    });
    let t = c.create_task(ALICE, nt("Port the parser")).unwrap();
    let n = c.offer_card(ALICE, &t.id, &room()).unwrap();
    assert_eq!(n.to, [BOB], "CAROL works wind#4 elsewhere: BOB next: {n:?}");
    let err = c.claim_task(CAROL, &t.id).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("one card at a time") && msg.contains("wind#4 \"Merge desk\""),
        "{msg}"
    );
    // BOB, free everywhere, takes it.
    c.claim_task(BOB, &t.id).unwrap();
    // A card someone assigned to CAROL still goes through.
    let a = c.create_task(ALICE, nt("Assigned to CAROL")).unwrap();
    c.assign_task(ALICE, &a.id, CAROL).unwrap();
    c.claim_task(CAROL, &a.id).unwrap();
}

/// Card #422: with no hook only this log counts, as before.
#[test]
fn no_busy_elsewhere_hook_changes_nothing() {
    let e = Env::new();
    let (_, n) = post(&e, "Port the parser", &room());
    assert_eq!(n.to, [CAROL], "{n:?}");
}

/// Card #423: a card waiting on something outside or on another agent is
/// held back the way one waiting on a task is (#397): a hand-out from
/// before the wait is stale, nobody is handed it while it waits, and once
/// the wait is resolved it goes out once, fresh, as if just posted.
#[test]
fn a_card_waiting_on_an_agent_or_outside_goes_out_fresh_once_resolved() {
    for (kind, reference) in [
        (BLOCKED_ON_EXTERNAL, "upstream release"),
        (BLOCKED_ON_AGENT, BOB),
    ] {
        let e = Env::new();
        let room = vec![idle(ALICE, 5), idle(CAROL, 10), idle(DAVE, 30)];
        let (t, n) = post(&e, &format!("waits on {kind}"), &room);
        assert_eq!(n.to, [CAROL], "{kind}: handed out before it was blocked");
        e.c.block(
            ALICE,
            &t.id,
            &format!("waits on {reference}"),
            Some(BlockedBy {
                kind: kind.into(),
                reference: reference.into(),
            }),
        )
        .unwrap();
        assert!(
            !e.c.stale_offer(&format!("task:{}", t.id), CAROL).is_empty(),
            "{kind}: CAROL's hand-out should go stale once the card waits"
        );
        let ended = vec![idle(ALICE, 5), turn_ended(CAROL), idle(DAVE, 30)];
        for _ in 0..3 {
            let ns = e.c.sweep_asks(&ended).unwrap();
            assert!(ns.iter().all(|n| n.kind != NOTICE_OFFER), "{kind}: {ns:?}");
            e.c.sweep_notices().unwrap();
        }
        e.clk.add(mins(5));
        e.c.unblock(ALICE, &t.id, "it's in").unwrap();
        let ns = e.c.sweep_asks(&ended).unwrap();
        let offers: Vec<_> = ns.iter().filter(|n| n.kind == NOTICE_OFFER).collect();
        assert!(
            offers.len() == 1 && offers[0].to == [DAVE],
            "{kind}: {ns:?}"
        );
        let o = offer_of(&e.c, &t.id);
        assert!(
            o.to == DAVE && o.heard == [DAVE],
            "{kind}: a fresh hand-out: {o:?}"
        );
        assert!(
            e.c.sweep_asks(&ended)
                .unwrap()
                .iter()
                .all(|n| n.kind != NOTICE_OFFER),
            "{kind}"
        );
    }
}
