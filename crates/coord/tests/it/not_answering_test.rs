//! Card #110: an agent whose session still runs but leaves its wakes
//! unread shows as not answering (roster status [`roster::NOT_ANSWERING`]).
//! It gets no new cards or asks, a card handed to it moves on, its held
//! cards say so once, and they can be taken over like a gone agent's. A
//! holder that went before the Core restarted stays gone across it, and
//! unheld cards that never got a hand-out are handed out.

use crate::common;

use common::*;
use coord::roster::{self, AgentCard};
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

fn card(key: &str, status: &str, since_min: u64) -> AgentCard {
    let at = GoTime::unix(fixed_now().secs - 60 * (60 - since_min as i64), 0);
    AgentCard {
        key: key.into(),
        device_id: "dev-a".into(),
        status: status.into(),
        updated_at: at,
        last_activity: at,
        ..AgentCard::default()
    }
}

/// ALICE posts; BOB, CAROL and DAVE are idle, CAROL idle longest.
fn room() -> Vec<AgentCard> {
    vec![
        card(ALICE, "idle", 5),
        card(BOB, "idle", 20),
        card(CAROL, "idle", 10),
        card(DAVE, "idle", 30),
    ]
}

fn mute(mut all: Vec<AgentCard>, key: &str) -> Vec<AgentCard> {
    for a in &mut all {
        if a.key == key {
            a.status = roster::NOT_ANSWERING.into();
        }
    }
    all
}

#[test]
fn a_card_handed_to_it_moves_on_and_it_gets_no_new_one() {
    let e = Env::new();
    let t = e.c.create_task(ALICE, nt("Port the parser")).unwrap();
    let n = e.c.offer_card(ALICE, &t.id, &room()).unwrap();
    assert_eq!(n.to, [CAROL], "CAROL idle longest");

    // CAROL stops answering: the card moves on to BOB, as if it went away.
    let ns = e.c.sweep_asks(&mute(room(), CAROL)).unwrap();
    assert!(
        ns.len() == 1 && ns[0].to == [BOB] && ns[0].text.contains("went away"),
        "{ns:?}"
    );

    // A new card skips CAROL while it isn't answering.
    let t2 = e.c.create_task(ALICE, nt("Port the lexer")).unwrap();
    let n2 = e.c.offer_card(ALICE, &t2.id, &mute(room(), CAROL)).unwrap();
    assert_eq!(n2.to, [DAVE], "BOB has a card; CAROL isn't answering");
}

#[test]
fn its_cards_say_so_once_and_can_be_taken_over() {
    let e = Env::new();
    e.ros.set(DAVE, "idle");
    let t = e.c.create_task(CAROL, nt("Fix the parser")).unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
    e.ros.set(ALICE, roster::NOT_ANSWERING);

    let ns = e.c.sweep_notices().unwrap();
    let said: Vec<Comment> =
        e.c.comments(&t.id)
            .unwrap()
            .into_iter()
            .filter(|c| c.by == "irori")
            .collect();
    assert!(
        said.len() == 1 && said[0].text.contains("isn't answering"),
        "{said:?}"
    );
    let told: Vec<&Notice> = ns.iter().filter(|n| n.task_id == t.id).collect();
    assert!(
        told.len() == 1 && told[0].to == [CAROL] && told[0].text.contains("isn't answering"),
        "the poster, a live agent, hears: {ns:?}"
    );

    // Once: the next sweeps add nothing.
    e.c.sweep_notices().unwrap();
    let n =
        e.c.comments(&t.id)
            .unwrap()
            .iter()
            .filter(|c| c.by == "irori")
            .count();
    assert_eq!(n, 1);

    // Taken over like a gone holder's: not at once, but after the grace.
    want_err(e.c.claim_task(BOB, &t.id), ErrorKind::Held);
    e.clk.add(GONE_GRACE + secs(1));
    let got = e.c.claim_task(BOB, &t.id).unwrap();
    assert!(got.holders.contains(&BOB.to_string()), "{got:?}");
}

#[test]
fn it_answers_again_and_keeps_its_card() {
    let e = Env::new();
    let t = e.c.create_task(CAROL, nt("Fix the parser")).unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
    e.ros.set(ALICE, roster::NOT_ANSWERING);
    e.c.sweep_notices().unwrap();
    e.clk.add(mins(2));
    e.ros.set(ALICE, "idle");
    e.c.sweep_notices().unwrap();
    e.clk.add(HOLDER_GRACE + mins(1));
    e.c.sweep_notices().unwrap();
    let got = e.c.get_task(&t.id).unwrap();
    assert_eq!(got.holders, [ALICE], "answering again stops the clock");
    want_err(e.c.claim_task(BOB, &t.id), ErrorKind::Held);
}

#[test]
fn a_holder_gone_before_a_restart_stays_gone_across_it() {
    let mut e = Env::new();
    let t = e.c.create_task(CAROL, nt("Stop the prompt")).unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
    let u = e.c.create_task(CAROL, nt("Other card")).unwrap();
    e.c.claim_task(DAVE, &u.id).unwrap();

    // ALICE's session ends (presence writes agent.gone); DAVE just drops
    // off the roster with no row. Then the Core restarts.
    #[derive(serde::Serialize)]
    struct Gone<'a> {
        key: &'a str,
        reason: &'a str,
    }
    e.st.append(
        "irori",
        kinds::AGENT_GONE,
        &Gone {
            key: ALICE,
            reason: "gone",
        },
    )
    .unwrap();
    e.ros.set(ALICE, "");
    e.clk.add(GONE_GRACE + secs(1));
    e.c = e.reopen();

    let got = e.c.claim_task(BOB, &t.id).unwrap();
    assert!(
        got.holders.contains(&BOB.to_string()),
        "ALICE went before the restart: {got:?}"
    );
    want_err(e.c.claim_task(CAROL, &u.id), ErrorKind::Held);
}

#[test]
fn a_gone_agent_seen_again_is_not_gone_from_its_old_row() {
    let mut e = Env::new();
    let t = e.c.create_task(CAROL, nt("Stop the prompt")).unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
    #[derive(serde::Serialize)]
    struct Key<'a> {
        key: &'a str,
    }
    e.st.append("irori", kinds::AGENT_GONE, &Key { key: ALICE })
        .unwrap();
    e.st.append("irori", kinds::AGENT_SEEN, &Key { key: ALICE })
        .unwrap();
    e.clk.add(GONE_GRACE + secs(1));
    e.c = e.reopen();
    // It is live again, then drops off: its grace starts now.
    e.ros.set(ALICE, "");
    want_err(e.c.claim_task(BOB, &t.id), ErrorKind::Held);
}

#[test]
fn an_unheld_card_that_was_never_handed_out_goes_out() {
    let e = Env::new();
    // Posted with no hand-out (before 0.7.1, or its offer row lost).
    let t = e.c.create_task(ALICE, nt("Old card")).unwrap();
    assert!(e.c.get_task(&t.id).unwrap().offer.is_none());
    let ns = e.c.sweep_asks(&room()).unwrap();
    assert!(
        ns.len() == 1 && ns[0].to == [CAROL] && ns[0].kind == NOTICE_OFFER,
        "{ns:?}"
    );
    let o = e.c.get_task(&t.id).unwrap().offer.unwrap();
    assert!(o.to == CAROL && o.reason == OFFER_HANDED, "{o:?}");

    // With nobody free it waits once, then goes out when one is.
    let e = Env::new();
    let t = e.c.create_task(ALICE, nt("Old card")).unwrap();
    let busy = vec![card(BOB, "busy", 20)];
    assert!(e.c.sweep_asks(&busy).unwrap().is_empty());
    assert_eq!(
        e.c.get_task(&t.id).unwrap().offer.unwrap().reason,
        OFFER_WAITING
    );
    let rows = e.rows(kinds::TASK_OFFERED).len();
    assert!(e.c.sweep_asks(&busy).unwrap().is_empty());
    assert_eq!(
        e.rows(kinds::TASK_OFFERED).len(),
        rows,
        "waits without new rows"
    );
    let ns = e.c.sweep_asks(&room()).unwrap();
    assert!(ns.len() == 1 && ns[0].to == [CAROL], "{ns:?}");
}
