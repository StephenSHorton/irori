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
