//! Go TestPlanSweep: the planner table (I:internal/inbox/sweep_test.go).

use std::collections::HashMap;

use super::*;
use crate::{GONE_GRACE, QUIET_LIMIT, REATTACH_GRACE, SYSTEM_KEY, UNENGAGED_LIMIT};

const M: i128 = 60_000_000_000;
const S: i128 = 1_000_000_000;

fn t0() -> GoTime {
    GoTime::parse_rfc3339("2026-10-26T12:00:00Z").unwrap()
}

/// t0 minus n minutes.
fn min(n: i128) -> GoTime {
    tm::add(&t0(), -n * M)
}

fn at(d: i128) -> GoTime {
    tm::add(&t0(), d)
}

struct World {
    gone: HashMap<&'static str, i128>,
    reach: Vec<&'static str>,
    desks: HashMap<&'static str, Vec<&'static str>>,
    /// a's session out of presence this long (and gone as long).
    ended: Option<i128>,
}

fn env(
    gone: &[(&'static str, i128)],
    reach: &[&'static str],
    desks: &[(&'static str, &[&'static str])],
) -> World {
    World {
        gone: gone.iter().copied().collect(),
        reach: reach.to_vec(),
        desks: desks.iter().map(|(k, v)| (*k, v.to_vec())).collect(),
        ended: None,
    }
}

fn ended(d: i128, desks: &[(&'static str, &[&'static str])]) -> World {
    let mut w = env(&[("a", d)], &[], desks);
    w.ended = Some(d);
    w
}

fn run(it: &SweepItem, w: &World) -> Vec<(Act, String)> {
    let gone_for = |a: &str| w.gone.get(a).copied().unwrap_or(0);
    let ended_for = |a: &str| if a == "a" { w.ended.unwrap_or(0) } else { 0 };
    let reachable = |a: &str| w.reach.contains(&a);
    let desk = |repo: &str, ex: &[&str]| {
        w.desks
            .get(repo)
            .and_then(|v| v.iter().find(|k| !ex.contains(k)))
            .map(|k| k.to_string())
            .unwrap_or_default()
    };
    let e = SweepEnv {
        now: t0(),
        unengaged: tm::d(UNENGAGED_LIMIT),
        gone: tm::d(GONE_GRACE),
        quiet: tm::d(QUIET_LIMIT),
        grace: tm::d(REATTACH_GRACE),
        paused: false,
        gone_for: &gone_for,
        ended_for: &ended_for,
        self_dev: "dev1".into(),
        reachable: &reachable,
        desk: &desk,
    };
    plan_sweep(std::slice::from_ref(it), &e)
        .into_iter()
        .map(|a| {
            assert_eq!(a.delivery_id, it.id);
            (a.kind, a.to)
        })
        .collect()
}

fn item(to: &str, repo: &str, state: &str) -> SweepItem {
    SweepItem {
        id: "1".into(),
        to: to.into(),
        repo: repo.into(),
        state: state.into(),
        ..SweepItem::default()
    }
}

#[test]
fn plan_sweep_table() {
    let gg = tm::d(GONE_GRACE);
    let around: &[&str] = &["a", "b"];
    let two: &[(&str, &[&str])] = &[("r", &["a", "b"])];
    let only_b: &[(&str, &[&str])] = &[("r", &["b"])];
    let only_a: &[(&str, &[&str])] = &[("r", &["a"])];
    let none: Vec<(Act, String)> = vec![];
    let mv = |to: &str| vec![(Act::Move, to.to_string())];
    let remind = |to: &str| vec![(Act::Remind, to.to_string())];

    struct Case {
        name: &'static str,
        it: SweepItem,
        w: World,
        want: Vec<(Act, String)>,
    }
    let mut cases: Vec<Case> = vec![];
    let mut add = |name: &'static str, it: SweepItem, w: World, want: Vec<(Act, String)>| {
        cases.push(Case { name, it, w, want })
    };

    let mut it = item("a", "r", st::ENGAGED);
    it.engaged_once = true;
    it.assigned_at = min(5);
    it.updated_at = min(5);
    add(
        "a holder gone 3 minutes loses repo work",
        it,
        env(&[("a", gg + S)], around, only_b),
        mv("b"),
    );

    let mut it = item("a", "", st::QUEUED);
    it.gone_repo = "r".into();
    it.assigned_at = min(5);
    it.updated_at = min(5);
    add(
        "an owner gone for good loses its PR work",
        it,
        env(&[("a", gg + S)], around, only_b),
        mv("b"),
    );

    let mut it = item("a", "", st::QUEUED);
    it.gone_repo = "r".into();
    it.assigned_at = min(30);
    it.updated_at = min(30);
    add(
        "an owner around keeps its unengaged PR work",
        it,
        env(&[], around, only_b),
        none.clone(),
    );

    let mut it = item("a", "r", st::DEFERRED);
    it.until = at(60 * M);
    it.moved_from = true;
    it.engaged_once = true;
    add(
        "even moved and deferred repo work leaves a gone holder",
        it,
        env(&[("a", gg + S)], around, only_b),
        mv("b"),
    );

    let mut it = item("a", "r", st::QUEUED);
    it.assigned_at = min(1);
    add(
        "gone with no desk left: unowned",
        it,
        env(&[("a", gg + S)], &[], &[]),
        mv(""),
    );

    let mut it = item("a", "r", st::ENGAGED);
    it.engaged_once = true;
    it.updated_at = min(5);
    add(
        "a restart blip doesn't move it",
        it,
        env(&[("a", 2 * M)], around, two),
        none.clone(),
    );

    let mut it = item("a", "", st::QUEUED);
    it.assigned_at = min(60);
    add(
        "agent work stays with a gone agent",
        it,
        env(&[("a", 60 * M)], &[], two),
        none.clone(),
    );

    let mut it = item("a", "r", st::DELIVERED);
    it.assigned_at = min(11);
    it.updated_at = min(11);
    add(
        "unstarted new work moves after 10 minutes",
        it,
        env(&[], around, two),
        mv("b"),
    );

    let mut it = item("a", "r", st::QUEUED);
    it.assigned_at = min(11);
    it.updated_at = min(11);
    add(
        "queued, never even delivered, moves too",
        it,
        env(&[], around, two),
        mv("b"),
    );

    let mut it = item("a", "r", st::DELIVERED);
    it.assigned_at = min(10);
    it.updated_at = min(10);
    add(
        "not before 10 minutes",
        it,
        env(&[], around, two),
        none.clone(),
    );

    let mut it = item("b", "r", st::DELIVERED);
    it.moved_from = true;
    it.assigned_at = min(11);
    it.updated_at = min(11);
    add(
        "moved once isn't moved again; reminded once",
        it,
        env(&[], around, two),
        remind("b"),
    );

    let mut it = item("b", "r", st::DELIVERED);
    it.moved_from = true;
    it.reminded = true;
    it.assigned_at = min(30);
    add("...and only once", it, env(&[], around, two), none.clone());

    let mut it = item("a", "r", st::DELIVERED);
    it.assigned_at = min(11);
    it.updated_at = min(11);
    add(
        "a lone front desk that hasn't started is reminded",
        it,
        env(&[], around, only_a),
        remind("a"),
    );

    let mut it = item("a", "r", st::QUEUED);
    it.assigned_at = min(11);
    add(
        "a lone desk with it still queued: nothing",
        it,
        env(&[], around, only_a),
        none.clone(),
    );

    let mut it = item("a", "r", st::DELIVERED);
    it.engaged_once = true;
    it.assigned_at = min(30);
    it.updated_at = min(30);
    add(
        "a holder that started on it keeps it",
        it,
        env(&[], around, two),
        none.clone(),
    );

    let mut it = item("a", "", st::ENGAGED);
    it.engaged_once = true;
    it.updated_at = min(61);
    add(
        "an hour engaged with no reply: remind",
        it,
        env(&[], around, &[]),
        remind("a"),
    );

    let mut it = item("a", "", st::ENGAGED);
    it.reminded = true;
    it.updated_at = min(200);
    add("once", it, env(&[], around, &[]), none.clone());

    let mut it = item("a", "", st::ENGAGED);
    it.updated_at = min(60);
    add(
        "not before the hour",
        it,
        env(&[], around, &[]),
        none.clone(),
    );

    let mut it = item("c", "", st::ENGAGED);
    it.updated_at = min(61);
    add(
        "only reachable holders are reminded",
        it,
        env(&[], around, &[]),
        none.clone(),
    );

    let mut it = item("a", "", st::DEFERRED);
    it.until = t0();
    it.updated_at = min(30);
    add(
        "a due deferral comes back",
        it,
        env(&[], around, &[]),
        vec![(Act::Requeue, String::new())],
    );

    let mut it = item("a", "r", st::DEFERRED);
    it.engaged_once = true;
    it.until = at(M);
    it.assigned_at = min(600);
    it.updated_at = min(600);
    add(
        "a deferral holds until its time",
        it,
        env(&[], around, two),
        none.clone(),
    );

    let mut it = item("a", "", st::QUEUED);
    it.in_flight = true;
    it.issued_at = at(-tm::d(REATTACH_GRACE) - S);
    add(
        "an unconfirmed wake is reclaimed",
        it,
        env(&[], around, &[]),
        vec![(Act::Reclaim, String::new())],
    );

    let mut it = item("a", "", st::QUEUED);
    it.in_flight = true;
    it.issued_at = at(-tm::d(REATTACH_GRACE));
    add(
        "not within the grace",
        it,
        env(&[], around, &[]),
        none.clone(),
    );

    let mut it = item("", "r", st::HELD);
    it.hold = hold::UNOWNED.into();
    add(
        "unowned repo work goes to a desk",
        it,
        env(&[], around, two),
        vec![(Act::Assign, "a".into())],
    );

    let mut it = item("", "r", st::HELD);
    it.hold = hold::UNOWNED.into();
    add(
        "unowned with no desk waits",
        it,
        env(&[], &[], &[]),
        none.clone(),
    );

    let mut it = item("a", "r", st::HELD);
    it.hold = hold::BLOCKED.into();
    it.assigned_at = min(100);
    add(
        "held by policy is a person's call",
        it,
        env(&[("a", 60 * M)], around, two),
        none.clone(),
    );

    let mut it = item("a", "r", st::ANSWERED);
    it.assigned_at = min(100);
    add(
        "answered is left alone",
        it,
        env(&[("a", 60 * M)], around, two),
        none.clone(),
    );

    // next_turn
    let mut it = item("a", "r", st::QUEUED);
    it.next_turn = true;
    it.assigned_at = min(600);
    it.updated_at = min(600);
    add(
        "next_turn repo work is never moved for being unengaged",
        it,
        env(&[], around, two),
        none.clone(),
    );

    let mut it = item("a", "", st::DELIVERED);
    it.next_turn = true;
    it.assigned_at = min(600);
    it.updated_at = min(600);
    add(
        "next_turn delivered never engaged: no reminder",
        it,
        env(&[], around, &[]),
        none.clone(),
    );

    let mut it = item("a", "r", st::DELIVERED);
    it.next_turn = true;
    it.moved_from = true;
    it.assigned_at = min(600);
    it.updated_at = min(600);
    add(
        "next_turn delivered repo work: no reminder",
        it,
        env(&[], around, two),
        none.clone(),
    );

    let mut it = item("a", "", st::ENGAGED);
    it.next_turn = true;
    it.engaged_once = true;
    it.updated_at = min(600);
    add(
        "next_turn engaged and quiet: no reminder",
        it,
        env(&[], around, &[]),
        none.clone(),
    );

    let mut it = item("a", "r", st::DELIVERED);
    it.next_turn = true;
    it.assigned_at = min(5);
    it.updated_at = min(5);
    add(
        "next_turn repo work still leaves a gone holder",
        it,
        env(&[("a", gg + S)], around, only_b),
        mv("b"),
    );

    let mut it = item("a", "", st::QUEUED);
    it.next_turn = true;
    it.in_flight = true;
    it.issued_at = min(3);
    it.assigned_at = min(3);
    add(
        "an unconfirmed append is reclaimed",
        it,
        env(&[], around, &[]),
        vec![(Act::Reclaim, String::new())],
    );

    let mut it = item("a", "", st::DEFERRED);
    it.next_turn = true;
    it.until = min(1);
    add(
        "a due next_turn deferral is re-queued",
        it,
        env(&[], around, &[]),
        vec![(Act::Requeue, String::new())],
    );

    // Ended recipients.
    let past = gg + S;
    let card = vec![(Act::GoneCard, String::new())];
    let to_person = vec![(Act::GonePerson, "user:dev1".to_string())];
    let ag = |state: &str, from: &str, r: &str| {
        let mut it = item("a", "", state);
        it.from = from.into();
        it.ref_ = r.into();
        it.assigned_at = min(5);
        it
    };
    let mut it = ag(st::ENGAGED, "c", "task:T1");
    it.engaged_once = true;
    it.updated_at = min(5);
    add(
        "ended, about a task: on the card",
        it,
        ended(past, &[]),
        card.clone(),
    );
    add(
        "ended, about a comment",
        ag(st::QUEUED, "c", "comment:C1"),
        ended(past, &[]),
        card.clone(),
    );
    let mut it = ag(st::QUEUED, "user:dev1", "task:T1");
    it.from_person = true;
    add(
        "ended, the person's message about a card",
        it,
        ended(past, &[]),
        card.clone(),
    );
    let mut it = ag(st::DELIVERED, "c", "");
    it.updated_at = min(5);
    add(
        "ended, no card, from an agent: to the person",
        it,
        ended(past, &[]),
        to_person.clone(),
    );
    add(
        "ended, a ref that is no card",
        ag(st::QUEUED, "c", "pr:o/r#1"),
        ended(past, &[]),
        to_person.clone(),
    );
    let mut it = ag(st::QUEUED, "user:dev1", "");
    it.from_person = true;
    add(
        "ended, no card, from the person: stays",
        it,
        ended(past, &[]),
        none.clone(),
    );
    let mut it = ag(st::QUEUED, "c", "task:T1");
    it.assigned_at = min(60);
    add(
        "live but unreachable: stays",
        it,
        env(&[("a", 60 * M)], &[], &[]),
        none.clone(),
    );
    add(
        "ended for less than the grace",
        ag(st::QUEUED, "c", "task:T1"),
        ended(gg, &[]),
        none.clone(),
    );
    let mut it = ag(st::QUEUED, "c", "task:T1");
    it.notice = true;
    it.next_turn = true;
    add("a notice stays", it, ended(past, &[]), none.clone());
    let mut it = ag(st::QUEUED, SYSTEM_KEY, "");
    it.system = true;
    add("Irori's own stays", it, ended(past, &[]), none.clone());
    let mut it = ag(st::HELD, "c", "task:T1");
    it.hold = hold::BLOCKED.into();
    add(
        "held is a person's call (ended)",
        it,
        ended(past, &[]),
        none.clone(),
    );
    let mut it = ag(st::QUEUED, "c", "task:T1");
    it.next_turn = true;
    add(
        "a next_turn delivery still hands off",
        it,
        ended(past, &[]),
        card.clone(),
    );
    let mut it = ag(st::QUEUED, "c", "");
    it.in_flight = true;
    it.issued_at = min(5);
    add(
        "in-flight hands off rather than reclaim",
        it,
        ended(past, &[]),
        to_person.clone(),
    );
    let mut it = item("a", "", st::DEFERRED);
    it.from = "c".into();
    it.ref_ = "task:T1".into();
    it.until = at(60 * M);
    it.engaged_once = true;
    add(
        "a deferred delivery hands off",
        it,
        ended(past, &[]),
        card.clone(),
    );
    let mut it = item("a", "r", st::ENGAGED);
    it.from = "c".into();
    it.ref_ = "task:T1".into();
    it.engaged_once = true;
    add(
        "repo work keeps its desk move",
        it,
        ended(past, only_b),
        mv("b"),
    );
    let mut it = ag(st::QUEUED, "c", "");
    it.gone_repo = "r".into();
    add(
        "PR work keeps its repo move",
        it,
        ended(past, only_b),
        mv("b"),
    );

    for c in cases {
        assert_eq!(run(&c.it, &c.w), c.want, "{}", c.name);
    }
}
