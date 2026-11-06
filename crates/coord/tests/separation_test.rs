//! Card #477 (docs/design/work-personal-separation.md §2): what keeps one
//! of a person's linked devices from starting another device's cards. On
//! the private log a card is spawned, and handed out, only by the device
//! its `created_by` names: a card made on the work computer (by the person
//! there or by one of its agents) is never listed to spawn or handed out
//! by a personal device, and the reverse. The test runs both directions.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::*;
use coord::roster::AgentCard;
use coord::rows::TaskCreatedRow;
use coord::*;
use gocompat::time::GoTime;
use store::kinds;

const WORK: &str = "dev-work";
const MAC: &str = "dev-mac";

fn person() -> KeyFn {
    Arc::new(|k: &str| k.starts_with("user:"))
}

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        objective: "why it matters".into(),
        ..NewTask::default()
    }
}

/// A coord over e's store with the private board's switch read from `on`.
fn spawning(e: &Env, on: &Arc<AtomicBool>) -> Coord {
    let f = on.clone();
    e.reopen_with(Options {
        spawning: Some(Arc::new(move || f.load(Ordering::SeqCst))),
        ..e.opts()
    })
}

/// A terminal agent idle on `dev`, outside any repo (the case the repo
/// filter lets through, §2.2).
fn idle_on(dev: &str) -> AgentCard {
    let at = GoTime::unix(fixed_now().secs - 60 * 40, 0);
    AgentCard {
        key: format!("{dev}/terminal"),
        device_id: dev.into(),
        status: "idle".into(),
        updated_at: at,
        last_activity: at,
        ..AgentCard::default()
    }
}

/// Two cards the other device made, synced in: one the person typed
/// there, one an agent there posted.
fn cards_from(e: &Env, other: &str) -> [String; 2] {
    let by = [format!("user:{other}"), format!("{other}/agent-session")];
    let ids = ["01M6KRP9NJC2CTBH23M08BAYJ6", "01M6KRP9NJC2CTBH23M08BAYJ7"];
    for (i, (id, by)) in ids.iter().zip(by.iter()).enumerate() {
        let at = plus(fixed_now(), secs(1 + i as u64));
        e.remote(
            other,
            1 + i as i64,
            hlc_at(at),
            if by.starts_with("user:") { "user" } else { by },
            kinds::TASK_CREATED,
            &TaskCreatedRow {
                task_id: (*id).into(),
                num: 40 + i as i64,
                title: format!("Made on {other}"),
                objective: "work that belongs there".into(),
                by: by.clone(),
                at,
                ..TaskCreatedRow::default()
            },
        );
    }
    ids.map(String::from)
}

#[test]
fn a_card_made_on_one_linked_device_never_spawns_on_another() {
    for (me, other) in [(MAC, WORK), (WORK, MAC)] {
        let mut e = Env::with(me, Some(person()));
        let on = Arc::new(AtomicBool::new(true));
        e.c = spawning(&e, &on);
        let mine = e.c.create_task(&format!("user:{me}"), nt("Mine")).unwrap();
        let theirs = cards_from(&e, other);
        for id in &theirs {
            let t = e.c.get_task(id).expect("synced in");
            assert_eq!(t.state, SUBMITTED, "{me}: it needs an agent");
        }
        // Only the card made here is this device's to spawn.
        assert_eq!(
            ids(&e.c.cards_to_spawn()),
            std::slice::from_ref(&mine.id),
            "{me} spawns only its own card"
        );
        // A replay (a restart) reads it the same way.
        e.c = spawning(&e, &on);
        assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&mine.id));
    }
}

#[test]
fn a_card_made_on_one_linked_device_is_never_handed_out_by_another() {
    // Spawning off, so the plain hand-out runs. The agent idle here is
    // free, outside any repo, and allowed hand-outs (no Offer it cards
    // gate on this coord): it still gets only this device's card, because
    // this device runs no hand-out for the other device's cards.
    for (me, other) in [(MAC, WORK), (WORK, MAC)] {
        let e = Env::with(me, Some(person()));
        let mine = e.c.create_task(&format!("user:{me}"), nt("Mine")).unwrap();
        let theirs = cards_from(&e, other);
        let out = e.c.sweep_asks(&[idle_on(me)]).unwrap();
        assert!(
            out.iter().all(|n| n.task_id == mine.id),
            "{me} hands out only its own card: {out:?}"
        );
        assert!(e.c.get_task(&mine.id).unwrap().live_offer().is_some());
        // Another free agent here, and sweeps again: still nothing.
        let mut two = idle_on(me);
        two.key = format!("{me}/second");
        for _ in 0..3 {
            let out = e.c.sweep_asks(&[two.clone()]).unwrap();
            assert!(out.iter().all(|n| n.task_id == mine.id), "{out:?}");
        }
        for id in &theirs {
            let t = e.c.get_task(id).unwrap();
            assert!(t.offer.is_none(), "{me} wrote no hand-out for {id}");
        }
    }
}

// ---- Card #495: automatic picks stay on the card's side ----
//
// The work computer runs Work, the Mac runs Personal (each pinned on its
// own device). Irori picks only agents on a device that runs the card's
// side: a Work card never reaches the Mac's agent, a Personal card never
// the work computer's. A card on a side its own device doesn't run waits
// and says so; it is never handed to another device instead.

use coord::sides::{KIND as SIDES, SidesRow};

/// The side each test device runs.
fn side_of(dev: &str) -> &'static str {
    if dev == WORK { "Work" } else { "Personal" }
}

/// e's device runs its side, and `other`'s pin (written on that device)
/// syncs in.
fn pin_both(e: &Env, me: &str, other: &str) {
    e.c.set_device_sides(&[side_of(me).to_string()]).unwrap();
    e.remote(
        other,
        1,
        hlc_at(plus(fixed_now(), secs(5))),
        "user",
        SIDES,
        &SidesRow {
            device_id: other.into(),
            sides: vec![side_of(other).into()],
            at_device: other.into(),
        },
    );
    assert!(!e.c.runs_side(me, side_of(other)), "{me} pinned");
    assert!(!e.c.runs_side(other, side_of(me)), "{other}'s pin counts");
}

fn on_side(title: &str, side: &str) -> NewTask {
    NewTask {
        side: side.into(),
        ..nt(title)
    }
}

/// Every (card, agent) these notices handed out.
fn handed_to(out: &[Notice]) -> Vec<(String, String)> {
    out.iter()
        .filter(|n| n.kind == NOTICE_OFFER || n.kind == NOTICE_ASK)
        .flat_map(|n| n.to.iter().map(|k| (n.task_id.clone(), k.clone())))
        .collect()
}

#[test]
fn a_hand_out_reaches_only_an_agent_on_the_cards_side() {
    // Spawning off: the plain hand-out runs.
    for (me, other) in [(WORK, MAC), (MAC, WORK)] {
        let mut e = Env::with(me, Some(person()));
        pin_both(&e, me, other);
        let user = format!("user:{me}");
        // A card typed here takes this device's side.
        let mine = e.c.create_task(&user, nt("Mine")).unwrap();
        assert_eq!(mine.side, side_of(me));
        // One on the other side, made here (asked for by name).
        let theirs =
            e.c.create_task(&user, on_side("Theirs", side_of(other)))
                .unwrap();
        for round in 0..2 {
            // Only the other device's agent is free: neither card goes to
            // it.
            let out = e.c.sweep_asks(&[idle_on(other)]).unwrap();
            assert!(handed_to(&out).is_empty(), "{me} round {round}: {out:?}");
            for id in [&mine.id, &theirs.id] {
                let t = e.c.get_task(id).unwrap();
                assert!(
                    t.offer.as_ref().is_none_or(|o| o.to.is_empty()),
                    "{me}: {} went to {:?}",
                    t.title,
                    t.offer
                );
            }
            // The card on the other side waits and says why; this side's
            // card just waits for a free agent.
            assert_eq!(
                e.c.get_task(&theirs.id).unwrap().side_wait,
                format!("no device here runs {}", side_of(other))
            );
            assert_eq!(e.c.get_task(&mine.id).unwrap().side_wait, "");
            // A replay (a restart) reads it the same way.
            e.c = e.reopen();
        }
        // An agent here comes free: it gets this side's card, never the
        // other side's, however many sweeps run.
        let out = e.c.sweep_asks(&[idle_on(me), idle_on(other)]).unwrap();
        assert_eq!(
            handed_to(&out),
            vec![(mine.id.clone(), format!("{me}/terminal"))],
            "{me}"
        );
        let mut two = idle_on(me);
        two.key = format!("{me}/second");
        for _ in 0..3 {
            let out = e.c.sweep_asks(&[two.clone(), idle_on(other)]).unwrap();
            assert!(
                handed_to(&out).iter().all(|(id, _)| *id != theirs.id),
                "{me}: {out:?}"
            );
        }
        let t = e.c.get_task(&theirs.id).unwrap();
        assert!(
            t.offer.as_ref().is_none_or(|o| o.to.is_empty()),
            "{:?}",
            t.offer
        );
    }
}

#[test]
fn a_card_on_a_side_its_device_doesnt_run_never_spawns_there() {
    for (me, other) in [(WORK, MAC), (MAC, WORK)] {
        let mut e = Env::with(me, Some(person()));
        let on = Arc::new(AtomicBool::new(true));
        e.c = spawning(&e, &on);
        pin_both(&e, me, other);
        let user = format!("user:{me}");
        let mine = e.c.create_task(&user, nt("Mine")).unwrap();
        let theirs =
            e.c.create_task(&user, on_side("Theirs", side_of(other)))
                .unwrap();
        assert_eq!(
            ids(&e.c.cards_to_spawn()),
            std::slice::from_ref(&mine.id),
            "{me} spawns only its side's card"
        );
        assert_eq!(
            e.c.get_task(&theirs.id).unwrap().side_wait,
            format!("no device here runs {}", side_of(other))
        );
        // After a replay, the same.
        e.c = spawning(&e, &on);
        assert_eq!(ids(&e.c.cards_to_spawn()), std::slice::from_ref(&mine.id));
        // The person's own "Start an agent" on it goes through ("unless
        // asked", §7.5).
        e.c.spawn_now(&theirs.id);
        let mut want = vec![mine.id.clone(), theirs.id.clone()];
        want.sort();
        let mut got = ids(&e.c.cards_to_spawn());
        got.sort();
        assert_eq!(got, want);
    }
}

#[test]
fn an_ask_goes_only_to_an_agent_on_its_side() {
    for (me, other) in [(WORK, MAC), (MAC, WORK)] {
        let mut e = Env::with(me, Some(person()));
        pin_both(&e, me, other);
        let user = format!("user:{me}");
        // Only the other device has a free agent: the ask isn't left to
        // it (ask_owner) or handed to its agent (free_agents).
        let (t, n) =
            e.c.post_ask(
                &user,
                NewAsk {
                    text: "who can look at the build?".into(),
                    ..NewAsk::default()
                },
                &[idle_on(other)],
            )
            .unwrap();
        assert_eq!(t.side, side_of(me));
        let a = t.ask.clone().unwrap();
        assert!(a.owner.is_empty() || a.owner == me, "{me} keeps it: {a:?}");
        assert!(a.taker.is_empty() && n.to.is_empty(), "{me}: {a:?} {n:?}");
        for _ in 0..3 {
            let out = e.c.sweep_asks(&[idle_on(other)]).unwrap();
            assert!(handed_to(&out).is_empty(), "{me}: {out:?}");
        }
        // After a replay, the same; then an agent here takes it.
        e.c = e.reopen();
        assert!(handed_to(&e.c.sweep_asks(&[idle_on(other)]).unwrap()).is_empty());
        let out = e.c.sweep_asks(&[idle_on(other), idle_on(me)]).unwrap();
        assert_eq!(
            handed_to(&out),
            vec![(t.id.clone(), format!("{me}/terminal"))],
            "{me}"
        );
        // The inbox's re-check agrees: a pick for the other device's agent
        // is off-side, one for this device's isn't.
        let r = format!("task:{}", t.id);
        assert!(e.c.off_side(NOTICE_ASK, &r, &format!("{other}/terminal")));
        assert!(!e.c.off_side(NOTICE_ASK, &r, &format!("{me}/terminal")));
    }
}

#[test]
fn an_ask_the_person_names_an_agent_in_still_reaches_it() {
    // "Unless asked" (§7.5): the person @-mentioning an agent across the
    // line goes through, and the inbox doesn't hold it.
    for (me, other) in [(WORK, MAC), (MAC, WORK)] {
        let e = Env::with(me, Some(person()));
        pin_both(&e, me, other);
        let there = idle_on(other);
        let (t, n) =
            e.c.post_ask(
                &format!("user:{me}"),
                NewAsk {
                    text: "@there can you look?".into(),
                    mentions: vec![Mention {
                        key: there.key.clone(),
                        name: "there".into(),
                        gone: false,
                    }],
                    ..NewAsk::default()
                },
                std::slice::from_ref(&there),
            )
            .unwrap();
        assert_eq!(n.to, vec![there.key.clone()], "{me}: {n:?}");
        assert!(!e.c.off_side(NOTICE_ASK, &format!("task:{}", t.id), &there.key));
    }
}

#[test]
fn a_quiet_thread_is_handed_only_to_an_agent_on_its_side() {
    for (me, other) in [(WORK, MAC), (MAC, WORK)] {
        let mut e = Env::with(me, Some(person()));
        pin_both(&e, me, other);
        let (t, n) =
            e.c.post_ask(
                &format!("user:{me}"),
                NewAsk {
                    text: "anyone around?".into(),
                    thread: true,
                    ..NewAsk::default()
                },
                &[idle_on(other)],
            )
            .unwrap();
        assert!(n.to.is_empty(), "{me}: {n:?}");
        // Minutes on, still nobody on the other side.
        for _ in 0..3 {
            e.clk.add(secs(60));
            let out = e.c.sweep_asks(&[idle_on(other)]).unwrap();
            assert!(handed_to(&out).is_empty(), "{me}: {out:?}");
        }
        e.c = e.reopen();
        e.clk.add(secs(60));
        assert!(handed_to(&e.c.sweep_asks(&[idle_on(other)]).unwrap()).is_empty());
        // An agent here: it gets it.
        let out = e.c.sweep_asks(&[idle_on(other), idle_on(me)]).unwrap();
        assert_eq!(
            handed_to(&out),
            vec![(t.id.clone(), format!("{me}/terminal"))],
            "{me}"
        );
    }
}

#[test]
fn the_inbox_check_names_a_hand_out_across_the_line() {
    for (me, other) in [(WORK, MAC), (MAC, WORK)] {
        let e = Env::with(me, Some(person()));
        pin_both(&e, me, other);
        let user = format!("user:{me}");
        let mine = e.c.create_task(&user, nt("Mine")).unwrap();
        let r = format!("task:{}", mine.id);
        let there = format!("{other}/terminal");
        assert!(e.c.off_side(NOTICE_OFFER, &r, &there));
        assert!(!e.c.off_side(NOTICE_OFFER, &r, &format!("{me}/terminal")));
        // Only picks: a comment notice isn't one.
        assert!(!e.c.off_side(NOTICE_COMMENT, &r, &there));
        // The same after a replay.
        let again = e.reopen();
        assert!(again.off_side(NOTICE_OFFER, &r, &there));
        // A card with no side is anybody's.
        e.c.set_device_sides(&[]).unwrap();
        let any = e.c.create_task(&user, nt("Any")).unwrap();
        assert_eq!(any.side, "");
        assert!(!e.c.off_side(NOTICE_OFFER, &format!("task:{}", any.id), &there));
    }
}

#[test]
fn with_no_pins_a_card_goes_out_as_before() {
    // No device.sides row anywhere: a card has no side, and the other
    // device's free agent is handed this device's card as today.
    for (me, other) in [(WORK, MAC), (MAC, WORK)] {
        let e = Env::with(me, Some(person()));
        let mine = e.c.create_task(&format!("user:{me}"), nt("Mine")).unwrap();
        assert_eq!(mine.side, "");
        let out = e.c.sweep_asks(&[idle_on(other)]).unwrap();
        assert_eq!(
            handed_to(&out),
            vec![(mine.id.clone(), format!("{other}/terminal"))]
        );
        assert_eq!(e.c.get_task(&mine.id).unwrap().side_wait, "");
    }
}

#[test]
fn a_hand_out_on_a_side_this_device_stopped_running_still_moves_on() {
    // Review fix: a card handed to an agent here, then the person pins
    // this device to the other side. When that agent goes, the hand-out is
    // let go (so the agent isn't counted busy with it), and nothing new is
    // picked.
    for (me, other) in [(WORK, MAC), (MAC, WORK)] {
        let e = Env::with(me, Some(person()));
        let user = format!("user:{me}");
        let card =
            e.c.create_task(&user, on_side("Theirs later", side_of(other)))
                .unwrap();
        let out = e.c.sweep_asks(&[idle_on(me)]).unwrap();
        assert_eq!(
            handed_to(&out),
            vec![(card.id.clone(), format!("{me}/terminal"))]
        );
        pin_both(&e, me, other);
        // Its agent went away: the hand-out moves on, to nobody.
        let mut two = idle_on(me);
        two.key = format!("{me}/second");
        let out = e.c.sweep_asks(&[two]).unwrap();
        assert!(handed_to(&out).is_empty(), "{me}: {out:?}");
        let t = e.c.get_task(&card.id).unwrap();
        let o = t.offer.clone().unwrap();
        assert!(o.to.is_empty(), "{me}: still handed to {:?}", o.to);
        assert_eq!(
            t.side_wait,
            format!("no device here runs {}", side_of(other))
        );
    }
}

#[test]
fn the_persons_start_an_agent_clears_the_side_wait() {
    for (me, other) in [(WORK, MAC), (MAC, WORK)] {
        let mut e = Env::with(me, Some(person()));
        let on = Arc::new(AtomicBool::new(true));
        e.c = spawning(&e, &on);
        pin_both(&e, me, other);
        let t =
            e.c.create_task(&format!("user:{me}"), on_side("Theirs", side_of(other)))
                .unwrap();
        assert_ne!(e.c.get_task(&t.id).unwrap().side_wait, "");
        e.c.spawn_now(&t.id);
        assert_eq!(
            e.c.get_task(&t.id).unwrap().side_wait,
            "",
            "{me}: an agent is starting"
        );
    }
}

/// Card #574: the side is judged on the device that hands the card out,
/// whichever device asks.
#[test]
fn spawn_hold_reads_the_side_on_the_cards_own_device() {
    let on = Arc::new(AtomicBool::new(true));
    let e = Env::on("dev-a");
    let f = on.clone();
    let c = e.reopen_with(Options {
        spawning: Some(Arc::new(move || {
            f.load(std::sync::atomic::Ordering::SeqCst)
        })),
        ..e.opts()
    });
    c.set_device_sides(&["work".to_string()]).unwrap();
    let t = c
        .create_task(
            "user:dev-b",
            NewTask {
                title: "Made on B, a side only here".into(),
                side: "work".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    assert_eq!(c.spawn_hold(&t.id), SpawnHold::Clear, "B has no pin");
    let mine = c
        .create_task(
            "user:dev-a",
            NewTask {
                title: "Made here, off the pin".into(),
                side: "home".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    assert_eq!(
        c.spawn_hold(&mine.id),
        SpawnHold::OffSide("no device here runs home".into())
    );
}
