//! Card #433: an agent presence listed as gone that becomes active again
//! (its device writes agent.seen and lists it) is live everywhere the
//! Board looks: its card is not taken over as a gone holder's, an
//! @mention of it resolves, and it can join its card.

use crate::common;

use common::*;
use coord::*;
use store::kinds;

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        objective: "why it matters".into(),
        ..NewTask::default()
    }
}

#[derive(serde::Serialize)]
struct Key<'a> {
    key: &'a str,
}

fn mentioned(e: &Env, text: &str) -> mentions::Resolved {
    let agents = roster::Agents::agents(e.ros.as_ref());
    mentions::resolve(&agents, BOB, text, &[])
}

#[test]
fn a_gone_agent_active_again_is_live_to_the_board() {
    let mut e = Env::new();
    e.ros.set_name(ALICE, "gudgeon");
    let t = e.c.create_task(CAROL, nt("Fix the full disk")).unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();

    // Presence lists it gone past the grace: a mention of it misses (and
    // its card could be taken over, not_answering_test).
    e.st.append("irori", kinds::AGENT_GONE, &Key { key: ALICE })
        .unwrap();
    e.ros.set(ALICE, "");
    e.clk.add(GONE_GRACE + secs(1));
    e.c = e.reopen();
    let m = mentioned(&e, "@gudgeon are you there?");
    assert!(
        m.mentions.is_empty() && m.misses.len() == 1,
        "a gone agent was mentioned: {m:?}"
    );

    // Its session is active again: its device writes agent.seen and lists
    // it.
    e.st.append("irori", kinds::AGENT_SEEN, &Key { key: ALICE })
        .unwrap();
    e.ros.set(ALICE, "idle");
    e.c = e.reopen();
    want_err(e.c.claim_task(BOB, &t.id), ErrorKind::Held);
    let m = mentioned(&e, "@gudgeon are you there?");
    assert!(
        m.misses.is_empty() && m.mentions.len() == 1 && m.mentions[0].key == ALICE,
        "{m:?}"
    );
    // It joins its card again (board_join), still its own.
    let got = e.c.claim_task(ALICE, &t.id).unwrap();
    assert_eq!(got.holders, [ALICE.to_string()]);
}
