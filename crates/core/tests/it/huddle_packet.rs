//! What a huddle's assistant knows the space by (card #1046,
//! docs/design/call-agents.md §7): the packet holds only what everyone in
//! the huddle's place can already see. A private card, a card on a named
//! Personal board and a card on a space the other person isn't in never
//! reach a channel's or a DM's packet; a Personal huddle, which only its
//! person sees, gets their own work too. Made-up people only.

use irori_core::HuddleSeed;

use crate::common::mesh::{Dev, invited, person};
use crate::common::wait_for;

const WAIT: u64 = 45;

fn close(devs: impl IntoIterator<Item = Dev>) {
    for d in devs {
        d.core.close().unwrap();
    }
}

fn packet(d: &Dev, conv: &str) -> String {
    d.core
        .huddle_packet_text(&HuddleSeed {
            call_id: "0123456789abcdef0123456789abcdef".into(),
            conv_id: conv.into(),
            ..Default::default()
        })
        .unwrap()
}

fn private_card(d: &Dev, title: &str, board: &str) {
    d.core
        .coord()
        .create_task(
            &d.core.user_sender().key,
            coord::NewTask {
                title: title.into(),
                board: board.into(),
                ..Default::default()
            },
        )
        .unwrap();
}

fn space_card(d: &Dev, space: &str, title: &str) -> coord::Task {
    d.core
        .board_coord(space)
        .unwrap()
        .create_task(
            &d.person_actor(),
            coord::NewTask {
                title: title.into(),
                ..Default::default()
            },
        )
        .unwrap()
}

const SHARED: &str = "Windows signing for the installer";
const PRIVATE: &str = "Dentist on Tuesday";
const HOME: &str = "Fix the garden fence";
const SOLO: &str = "Alice's solo plan";
const BOB_SOLO: &str = "Bob's solo plan";

fn none_private(text: &str, who: &str) {
    for t in [PRIVATE, HOME, SOLO, BOB_SOLO] {
        assert!(!text.contains(t), "{who}'s packet holds {t:?}:\n{text}");
    }
}

#[test]
fn channel_and_dm_packets_hold_only_what_both_people_see() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let space = invited(a1, b1);
    wait_for(WAIT, "alice hears bob read spaces", || {
        a1.core.space_not_ready(&space).unwrap().is_empty()
    });
    a1.core.space_gate(&space).unwrap();

    // The shared card, held by Bob's agent.
    let shared = space_card(a1, &space, SHARED);
    a1.core.kick_board(&space);
    wait_for(WAIT, "bob gets the shared card", || {
        b1.core
            .board_coord(&space)
            .ok()
            .and_then(|c| c.get_task(&shared.id))
            .is_some()
    });
    let out = b1.tool(
        "board_join",
        serde_json::json!({ "task_id": shared.short_ref() }),
    );
    assert!(!out.to_lowercase().contains("error"), "{out}");
    b1.core.kick_board(&space);
    wait_for(WAIT, "alice sees bob's agent hold it", || {
        a1.core
            .board_coord(&space)
            .ok()
            .and_then(|c| c.get_task(&shared.id))
            .is_some_and(|t| !t.holders.is_empty())
    });

    // Alice's private work: the default Personal board, a named one, and a
    // space only she is in. Bob has a solo space too.
    private_card(a1, PRIVATE, "");
    let home = a1.core.create_personal_board("Home jobs", "home").unwrap();
    private_card(a1, HOME, &home);
    let solo = a1.core.create_board("Solo", "solo").unwrap();
    space_card(a1, &solo, SOLO);
    let bob_solo = b1.core.create_board("Bob's own", "bob").unwrap();
    space_card(b1, &bob_solo, BOB_SOLO);

    // A channel's huddle: that space's cards, from either side.
    let ch = a1
        .core
        .talk_channel_create_in(&a1.user(), "general", "", &space)
        .unwrap()
        .id;
    a1.core.kick_board(&space);
    wait_for(WAIT, "bob holds the channel", || {
        b1.core.talk().channel(&ch).is_some()
    });
    for (d, who) in [(a1, "alice"), (b1, "bob")] {
        let text = packet(d, &ch);
        assert!(text.contains("Open cards on MLG farming"), "{text}");
        let line = text
            .lines()
            .find(|l| l.contains(SHARED))
            .unwrap_or_else(|| panic!("{who}'s packet lacks the shared card:\n{text}"));
        assert!(
            line.starts_with(&format!("- {} ", shared.short_ref())),
            "{line}"
        );
        assert!(
            line.contains("held by ") && line.contains("'s agent)"),
            "{line}"
        );
        assert!(text.contains("Agents holding them:\n- "), "{text}");
        none_private(&text, who);
    }

    // A DM's huddle: the spaces both are in, from either side.
    let dm = a1.core.talk_dm(&a1.user(), &b1.person_actor()).unwrap();
    a1.core
        .talk_post(
            &a1.core.user_sender(),
            &dm,
            "hi bob",
            &[],
            false,
            None,
            vec![],
        )
        .unwrap();
    wait_for(WAIT, "bob holds the DM", || {
        b1.core
            .talk_posts(&b1.user(), &dm, None, 50)
            .is_ok_and(|ps| ps.iter().any(|p| p.text == "hi bob"))
    });
    for (d, who) in [(a1, "alice"), (b1, "bob")] {
        let text = packet(d, &dm);
        assert!(
            text.contains("the spaces you share (MLG farming)"),
            "{text}"
        );
        assert!(text.contains(SHARED), "{who}: {text}");
        none_private(&text, who);
    }

    // A Personal huddle is only Alice's: her own work is in it.
    let mine = a1
        .core
        .talk_channel_create(&a1.user(), "notes", "")
        .unwrap()
        .id;
    let text = packet(a1, &mine);
    for t in [SHARED, PRIVATE, HOME, SOLO] {
        assert!(text.contains(t), "the Personal packet lacks {t:?}:\n{text}");
    }
    assert!(!text.contains(BOB_SOLO), "{text}");

    close(alice.drain(..).chain(bob.drain(..)));
}
