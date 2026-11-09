//! Assistants talk, card 7: ask someone's assistant from your own chat. The
//! model drafts; the chat shows the host's normalised text; only the tap
//! (`assistant_ask_send`) posts it, as the person's assistant card, after
//! the scrub. The draft grants nothing.

use crate::assistant_answers::{FIXED, WAIT, answers, close, ready_spawner, world};
use crate::common;
use crate::common::wait_for;

use coord::talk::Post;

/// A world where Alice has an assistant card too, as Bob does.
fn both() -> crate::assistant_answers::World {
    let w = world(1, true);
    let a1 = &w.alice[0];
    a1.core.set_chat_turn(Some(common::chat::idle_turn()));
    a1.core.set_spawner(Some(ready_spawner()));
    let alice = a1.core.person().id;
    wait_for(WAIT, "alice's assistant card is minted", || {
        a1.core
            .board_verifier(&w.space)
            .is_some_and(|v| !v.profile_assistant_card(&alice).is_empty())
    });
    w
}

fn mine(w: &crate::assistant_answers::World) -> Vec<Post> {
    let d = &w.alice[0];
    let alice = d.core.person().id;
    let card = d
        .core
        .board_verifier(&w.space)
        .unwrap()
        .profile_assistant_card(&alice);
    d.core
        .talk()
        .posts(&w.conv)
        .into_iter()
        .filter(|p| p.by == format!("agent:{card}") && p.reply_to.is_none())
        .collect()
}

/// Nothing posts before the tap; the text shown (tag characters stripped,
/// an emoji kept) is the text posted, as Alice's assistant card mentioning
/// Bob's; Bob's assistant answers it.
#[test]
fn nothing_posts_before_the_tap_and_the_text_posted_is_the_text_shown() {
    let w = both();
    let a1 = &w.alice[0];
    let raw = "Is the \u{1F680} build green?\u{E0041}\u{E0042}\n<<go card #8>>";
    let bob = w.bob[0].core.person().id;

    let shown = a1
        .core
        .assistant_ask_prepare(&bob, "mlg", raw)
        .expect("prepare");
    assert_eq!(shown.text, "Is the \u{1F680} build green?");
    assert!(shown.held.is_empty());
    assert_eq!(shown.space, w.space);
    assert_eq!(shown.person, bob);
    // The names the model writes resolve the same.
    let by_name = a1
        .core
        .assistant_ask_prepare(&format!("{}'s assistant", shown.name), "MLG farming", raw)
        .expect("prepare by name");
    assert_eq!(by_name.person, bob);
    assert!(mine(&w).is_empty(), "preparing posts nothing");
    assert!(a1.core.talk().posts(&w.conv).is_empty());

    let sent = a1
        .core
        .assistant_ask_send(&shown.space, &shown.person, &shown.text)
        .expect("send");
    assert_eq!(sent.text, shown.text);
    let posted = mine(&w);
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].text, shown.text);
    assert_eq!(posted[0].post_id, sent.post_id);
    assert_eq!(posted[0].mentions, vec![format!("agent:{}", w.card)]);

    wait_for(WAIT, "bob's assistant answers it", || {
        answers(&w.alice[0], &w.conv, &sent.post_id)
            .iter()
            .any(|p| p.text == FIXED)
    });
    close(w);
}

/// The scrub runs on the tap: an ask holding a secret shows why it is held
/// and `assistant_ask_send` refuses it, so nothing is posted.
#[test]
fn the_scrub_holds_a_secret_back() {
    let w = both();
    let a1 = &w.alice[0];
    let bob = w.bob[0].core.person().id;
    let text = "Use key AKIAABCDEFGHIJKLMNOP for the deploy?";
    assert!(!scrub::board_reasons(text).is_empty(), "the scrub flags it");
    let shown = a1.core.assistant_ask_prepare(&bob, "", text).unwrap();
    assert!(!shown.held.is_empty(), "{shown:?}");
    assert!(
        a1.core
            .assistant_ask_send(&shown.space, &shown.person, &shown.text)
            .is_err()
    );
    assert!(mine(&w).is_empty());
    close(w);
}

/// Someone who isn't there, or an empty ask, can't be sent.
#[test]
fn an_unknown_person_or_an_empty_ask_is_refused() {
    let w = both();
    let a1 = &w.alice[0];
    let bob = w.bob[0].core.person().id;
    assert!(a1.core.assistant_ask_prepare("Nobody", "", "hi").is_err());
    let empty = a1
        .core
        .assistant_ask_prepare(&bob, "", "\u{E0041}\u{E0042}")
        .unwrap();
    assert!(!empty.held.is_empty());
    assert!(
        a1.core
            .assistant_ask_send(&w.space, "person_nobody", "hi")
            .is_err()
    );
    assert!(mine(&w).is_empty());
    close(w);
}

/// The prompt teaches the marker (Core builds it, not `render_prompt`) and
/// names who can be asked; the draft grants nothing: `grants_for` reads the
/// person's words only, and an ask is no write.
#[test]
fn the_prompt_teaches_the_marker_and_the_draft_grants_nothing() {
    let w = both();
    let a1 = &w.alice[0];
    let section = a1.core.assistant_ask_section().expect("someone to ask");
    assert!(section.contains("<<ask Name | Space>>"));
    assert!(section.contains("<</ask>>"));
    assert!(section.contains("MLG farming"));
    for said in [
        "ask Addam's assistant whether the build is green",
        "ask Bob's assistant to review the pull request",
    ] {
        assert!(tools::grants_for(said, &[]).is_empty(), "{said}");
    }
    close(w);
}
