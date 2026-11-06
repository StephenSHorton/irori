//! What the window does with a DM between two people (card #538,
//! docs/design/talk-dms.md §6, §7), through the real Core API on loopback
//! iroh: who can be DMed, what a DM waits on, and leaving one. Made-up
//! people only.

use crate::common::mesh::{Dev, invited, person};
use crate::common::wait_for;

const WAIT: u64 = 45;

fn close(devs: impl IntoIterator<Item = Dev>) {
    for d in devs {
        d.core.close().unwrap();
    }
}

fn linked(a: &Dev, b: &Dev, board: &str) {
    let peer = b.core.device_id().to_string();
    wait_for(WAIT, "the two devices connect on their space", || {
        a.core
            .net()
            .board_conns(board)
            .iter()
            .any(|c| c.peer == peer)
    });
}

fn dm_with(d: &Dev, other: &Dev) -> String {
    d.core.talk_dm(&d.user(), &other.person_actor()).unwrap()
}

fn post(d: &Dev, conv: &str, text: &str) {
    d.core
        .talk_post(&d.core.user_sender(), conv, text, &[], false, None)
        .unwrap();
}

fn holds(d: &Dev, conv: &str, text: &str) -> bool {
    d.core
        .talk_posts(&d.user(), conv, None, 50)
        .is_ok_and(|ps| ps.iter().any(|p| p.text == text))
}

fn row(d: &Dev, conv: &str) -> Option<tools::talk::Conversation> {
    d.core
        .talk_conversations(&d.user())
        .unwrap()
        .into_iter()
        .find(|c| c.conv_id == conv)
}

/// A DM that has reached both sides: Alice founded it, Bob holds it.
fn started(a: &Dev, b: &Dev) -> (String, String) {
    let dm = dm_with(a, b);
    post(a, &dm, "hi bob");
    wait_for(WAIT, "bob reads alice's DM", || holds(b, &dm, "hi bob"));
    let spaces = a.core.dm_spaces_with(&b.core.person().id);
    assert_eq!(spaces.len(), 1, "{spaces:?}");
    wait_for(WAIT, "the space stops being pending", || {
        !a.core.dm_pending(&spaces[0])
    });
    (dm, spaces[0].clone())
}

/// §6: the people you share a space with, once each however many spaces
/// you share, and a hidden DM space never counts as one: a person you
/// share only that with can't be asked over any space.
#[test]
fn the_picker_lists_who_you_share_a_space_with() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    assert!(a1.core.talk_dm_people().is_empty(), "nobody yet");
    let e = a1.core.talk_dm(&a1.user(), &b1.person_actor()).unwrap_err();
    assert!(e.contains("don't share a space"), "{e}");

    let board = invited(a1, b1);
    linked(a1, b1, &board);
    let people = a1.core.talk_dm_people();
    assert_eq!(people.len(), 1, "{people:?}");
    assert_eq!(people[0].key, b1.person_actor());
    assert_eq!(people[0].name, b1.core.person().name);
    assert_eq!(people[0].board_id, board);

    let (_, space) = started(a1, b1);
    let people = a1.core.talk_dm_people();
    assert_eq!(people.len(), 1, "the DM's space adds nobody: {people:?}");
    assert_eq!(people[0].board_id, board);
    assert!(a1.core.is_dm_space(&space));

    // Only a hidden space left: nobody to ask over.
    b1.core.leave_board(&board).unwrap();
    wait_for(WAIT, "alice hears bob left the space", || {
        a1.core.talk_dm_people().is_empty()
    });
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §6: a DM waits only while its first posts haven't reached the other
/// person, and the row carries their name throughout.
#[test]
fn a_dm_row_says_it_waits_until_the_hand_off_is_over() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    linked(a1, b1, &board);
    b1.core.close_board_net(&board);

    let dm = dm_with(a1, b1);
    let name = b1.core.person().name;
    let c = row(a1, &dm).expect("an opened DM is listed");
    assert_eq!(c.name, name);
    assert_eq!(c.waiting, "", "a draft has sent nothing");

    post(a1, &dm, "when you're back");
    let c = row(a1, &dm).unwrap();
    assert_eq!(c.name, name);
    assert_eq!(c.waiting, tools::talk::WAITING_FOR_THEM);

    b1.core.reopen_board_net(&board).unwrap();
    wait_for(WAIT, "the row stops waiting", || {
        row(a1, &dm).is_some_and(|c| c.waiting.is_empty())
    });
    wait_for(WAIT, "bob's row", || row(b1, &dm).is_some());
    let c = row(b1, &dm).unwrap();
    assert_eq!(c.name, a1.core.person().name);
    assert_eq!(c.waiting, "");
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §7: the founder is the space's only owner and can still leave. What was
/// in it goes from her side, Bob stops writing to it, and his next post
/// founds a new space that brings her back.
#[test]
fn the_founder_leaves_a_dm_and_a_later_post_offers_a_new_one() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    linked(a1, b1, &board);
    let (dm, old) = started(a1, b1);
    let alice_id = a1.core.person().id.clone();

    a1.core.talk_dm_leave(&dm).unwrap();
    assert!(row(a1, &dm).is_none(), "her DM is gone");
    assert!(!holds(a1, &dm, "hi bob"), "and what was in it");

    let v = b1.core.board_verifier(&old).unwrap();
    wait_for(WAIT, "bob hears she left", || {
        !v.membership().active(&alice_id)
    });
    assert!(holds(b1, &dm, "hi bob"), "bob keeps what he has");
    assert_eq!(row(b1, &dm).unwrap().waiting, "", "nothing asks her back");

    post(b1, &dm, "still there?");
    wait_for(WAIT, "a new space brings her back", || {
        holds(a1, &dm, "still there?")
    });
    assert!(!holds(a1, &dm, "hi bob"), "the old log stays gone");
    wait_for(
        WAIT,
        "her old space is forgotten once he holds the row",
        || !a1.core.dm_spaces_with(&b1.core.person().id).contains(&old),
    );
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §7: Bob is away when the founder leaves. Her `left` row waits for him,
/// and her room stays up for it, so he still hears.
#[test]
fn the_founder_leaves_a_dm_while_the_other_person_is_away() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    linked(a1, b1, &board);
    let (dm, old) = started(a1, b1);
    let alice_id = a1.core.person().id.clone();
    b1.core.close_board_net(&old);

    a1.core.talk_dm_leave(&dm).unwrap();
    assert!(row(a1, &dm).is_none());

    b1.core.reopen_board_net(&old).unwrap();
    let v = b1.core.board_verifier(&old).unwrap();
    wait_for(WAIT, "bob hears she left", || {
        !v.membership().active(&alice_id)
    });
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §7: the other side leaves. The founder's next post goes in a new space,
/// which is how he hears from her again.
#[test]
fn the_other_person_leaves_a_dm_and_a_later_post_offers_a_new_one() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    linked(a1, b1, &board);
    let (dm, old) = started(a1, b1);
    let bob_id = b1.core.person().id.clone();

    b1.core.talk_dm_leave(&dm).unwrap();
    assert!(row(b1, &dm).is_none());
    let v = a1.core.board_verifier(&old).unwrap();
    wait_for(WAIT, "alice hears he left", || {
        !v.membership().active(&bob_id)
    });

    post(a1, &dm, "one more thing");
    wait_for(WAIT, "a new space brings him back", || {
        holds(b1, &dm, "one more thing")
    });
    assert!(!holds(b1, &dm, "hi bob"), "the old log stays gone");
    close(alice.drain(..).chain(bob.drain(..)));
}

/// §7: a space nobody else has heard of has nobody to tell: no `left` row
/// waits for ever, and no hand-off runs for it later. A draft goes the
/// same way, and a key that isn't a person DM is refused.
#[test]
fn leaving_a_dm_nobody_heard_of_leaves_nothing_behind() {
    let mut alice = person("alice", 1);
    let mut bob = person("bob", 1);
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = invited(a1, b1);
    linked(a1, b1, &board);
    b1.core.close_board_net(&board);

    let dm = dm_with(a1, b1);
    a1.core.talk_dm_leave(&dm).unwrap();
    assert!(row(a1, &dm).is_none(), "a draft goes");

    let dm = dm_with(a1, b1);
    post(a1, &dm, "never mind");
    let bob_id = b1.core.person().id.clone();
    let spaces = a1.core.dm_spaces_with(&bob_id);
    assert_eq!(spaces.len(), 1, "{spaces:?}");
    assert!(a1.core.dm_pending(&spaces[0]));

    a1.core.talk_dm_leave(&dm).unwrap();
    assert!(row(a1, &dm).is_none());
    assert!(a1.core.dm_spaces_with(&bob_id).is_empty());
    assert_eq!(a1.core.boards().len(), 1, "only the space they share");

    b1.core.reopen_board_net(&board).unwrap();
    linked(a1, b1, &board);
    assert!(b1.core.dm_spaces_with(&a1.core.person().id).is_empty());
    assert!(row(b1, &dm).is_none());

    assert!(a1.core.talk_dm_leave("dm:0000000000000000").is_err());
    let agent = a1.core.talk_dm(&a1.user(), &a1.agent).unwrap();
    assert!(a1.core.talk_dm_leave(&agent).is_err());
    close(alice.drain(..).chain(bob.drain(..)));
}
