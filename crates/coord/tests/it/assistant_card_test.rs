//! Assistants talk, card 2: the Office Assistant's card (kind `assistant`) is
//! never a free agent. A room ask passes it by, and an ask that @-mentions it
//! isn't handed to it.

use crate::common;

use common::*;
use coord::roster::{ASSISTANT_KIND, AgentCard};
use coord::*;
use gocompat::time::GoTime;

const ASSISTANT: &str = "agent:oa-card-1";

fn idle(key: &str, kind: &str, since_min: u64) -> AgentCard {
    let at = GoTime::unix(fixed_now().secs - 60 * (60 - since_min as i64), 0);
    AgentCard {
        key: key.into(),
        agent: kind.into(),
        device_id: "dev-a".into(),
        status: "idle".into(),
        updated_at: at,
        last_activity: at,
        ..AgentCard::default()
    }
}

/// The assistant card has been idle longest of all (as a fresh one is).
fn room() -> Vec<AgentCard> {
    vec![
        AgentCard {
            updated_at: GoTime::default(),
            ..idle(ASSISTANT, ASSISTANT_KIND, 1)
        },
        idle(BOB, "claude", 20),
        idle(CAROL, "claude", 10),
    ]
}

#[test]
fn a_room_ask_passes_the_assistant_card_by() {
    let e = Env::new();
    let (_, n) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "Who can look at the parser?".into(),
                ..NewAsk::default()
            },
            &room(),
        )
        .unwrap();
    assert_eq!(
        n.to,
        [CAROL],
        "CAROL idle longest of the real agents: {n:?}"
    );
}

#[test]
fn an_ask_that_mentions_the_assistant_is_not_handed_to_it() {
    let e = Env::new();
    let (_, n) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "@assistant what is open?".into(),
                mentions: vec![Mention {
                    key: ASSISTANT.into(),
                    ..Mention::default()
                }],
                ..NewAsk::default()
            },
            &room(),
        )
        .unwrap();
    assert!(!n.to.contains(&ASSISTANT.to_string()), "{n:?}");
}

#[test]
fn a_claude_card_with_the_assistants_name_is_still_a_free_agent() {
    let e = Env::new();
    let mut r = room();
    r[0].agent = "claude".into();
    let (_, n) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "Who can look at the parser?".into(),
                ..NewAsk::default()
            },
            &r,
        )
        .unwrap();
    assert_eq!(n.to, [ASSISTANT], "only the kind keeps a card out: {n:?}");
}
