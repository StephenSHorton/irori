//! Assistants talk, card 6: Let it answer. A tap on any device appends an
//! `assistant.answer.requested` row; the assistant's computer serves it
//! once, past the receive setting, and writes a record naming the request.
//! A quiet `held` line is an `assistant.chat.update` row too.

use crate::assistant_answers::{
    FIXED, WAIT, answered, answers, close, control, fake_turn, question, send, set_receive, world,
};

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::common::wait_for;
use inbox::receive;
use irori_core::assistant_answers::{answer_id, waiting_id};
use irori_core::assistant_chat::{AnswerRequested, ChatEntry, ChatRow};

/// The update lines of `d`'s chat, oldest first.
fn lines(d: &crate::common::mesh::Dev) -> Vec<irori_core::assistant_chat::Update> {
    let mut page = d.core.assistant_chat_list("", 200).unwrap().items;
    page.reverse();
    page.into_iter()
        .filter_map(|i| match i.entry {
            ChatEntry::Update(u) => Some(u),
            _ => None,
        })
        .collect()
}

/// Under Ask me the assistant posts the waiting line and runs no turn.
/// Let it answer, tapped on Bob's other computer, answers once as the
/// assistant's card with the answer id; a second tap answers nothing more;
/// each tap leaves a record.
#[test]
fn let_it_answer_passes_ask_me_and_cannot_answer_twice() {
    let w = world(2, false);
    let (a1, b1, b2) = (&w.alice[0], &w.bob[0], &w.bob[1]);
    let runs = Arc::new(AtomicUsize::new(0));
    for d in &w.bob {
        d.core.set_answerer(Some(fake_turn(&runs)));
    }
    set_receive(b1, &w.space, receive::ASK);
    let q = question(&w, "who reviews PR #12?");
    send(&w, &q, 0);
    let line = answered(&w, b1, &q.post_id);
    assert_eq!(line.waiting.as_deref(), Some("ask"));
    assert_eq!(line.post_id, waiting_id(&w.card, &q.post_id));
    assert_eq!(runs.load(Ordering::SeqCst), 0, "Ask me runs no turn");

    // The tap is on the computer that isn't asked to answer; whichever is
    // the assistant's computer serves it.
    let first = b2
        .core
        .assistant_answer_request(&q.post_id, &w.space, "let")
        .unwrap();
    wait_for(WAIT, "the answer is posted", || {
        answers(a1, &w.conv, &q.post_id)
            .iter()
            .any(|p| p.waiting.is_none())
    });
    let a = answers(a1, &w.conv, &q.post_id)
        .into_iter()
        .find(|p| p.waiting.is_none())
        .unwrap();
    assert_eq!(a.text, FIXED);
    assert_eq!(a.post_id, answer_id(&w.card, &q.post_id));
    assert_eq!(a.by, format!("agent:{}", w.card));
    wait_for(WAIT, "the tap has a record on both computers", || {
        [b1, b2]
            .iter()
            .all(|d| lines(d).iter().any(|u| u.request == first))
    });
    let record = lines(b1).into_iter().find(|u| u.request == first).unwrap();
    assert_eq!(record.key, format!("req:{first}"));
    assert!(record.text.starts_with("Answered"), "{}", record.text);

    // A second tap, here or there, answers nothing more.
    let second = b1
        .core
        .assistant_answer_request(&q.post_id, &w.space, "let")
        .unwrap();
    wait_for(WAIT, "the second tap has a record", || {
        lines(b1).iter().any(|u| u.request == second)
    });
    control(&w);
    std::thread::sleep(Duration::from_secs(3));
    let n = answers(a1, &w.conv, &q.post_id)
        .iter()
        .filter(|p| p.waiting.is_none())
        .count();
    assert_eq!(n, 1, "one answer");
    assert_eq!(runs.load(Ordering::SeqCst), 1, "one tap, one turn");
    close(w);
}

/// Closed posts nothing, but a tap is the owner's choice: it answers.
#[test]
fn let_it_answer_passes_closed() {
    let w = world(1, true);
    let b1 = &w.bob[0];
    set_receive(b1, &w.space, receive::CLOSED);
    let q = question(&w, "anyone there?");
    send(&w, &q, 0);
    std::thread::sleep(Duration::from_secs(5));
    assert!(answers(b1, &w.conv, &q.post_id).is_empty());
    b1.core
        .assistant_answer_request(&q.post_id, &w.space, "let")
        .unwrap();
    let a = answered(&w, b1, &q.post_id);
    assert_eq!(a.text, FIXED);
    close(w);
}

/// A tap found more than an hour old becomes a "tap again" line, not a
/// public post.
#[test]
fn an_old_tap_says_tap_again_and_posts_nothing() {
    let w = world(1, true);
    let b1 = &w.bob[0];
    set_receive(b1, &w.space, receive::ASK);
    let q = question(&w, "an old tap?");
    send(&w, &q, 0);
    answered(&w, b1, &q.post_id);
    let at = b1.core.assistant_chat_now() - 2 * 3600 * 1000;
    b1.core
        .assistant_chat_write(ChatRow::AnswerRequested(AnswerRequested {
            request: "old-tap".into(),
            post: q.post_id.clone(),
            space: w.space.clone(),
            how: "let".into(),
            by: format!("user:{}", b1.core.device_id()),
            at,
        }))
        .unwrap();
    wait_for(WAIT, "the old tap has a record", || {
        lines(b1).iter().any(|u| u.request == "old-tap")
    });
    let r = lines(b1)
        .into_iter()
        .find(|u| u.request == "old-tap")
        .unwrap();
    assert!(r.text.contains("Tap again"), "{}", r.text);
    control(&w);
    assert!(
        answers(&w.alice[0], &w.conv, &q.post_id)
            .iter()
            .all(|p| p.waiting.is_some()),
        "no answer"
    );
    close(w);
}

/// A tap names a post the space doesn't hold, or one that isn't a question
/// to this assistant: nothing is posted and the tap is recorded as refused.
#[test]
fn a_tap_is_checked_against_the_post_itself() {
    let w = world(1, true);
    let b1 = &w.bob[0];
    // A post that mentions nobody isn't a question to the assistant.
    let chat = crate::assistant_answers::question(&w, "just talking");
    let chat = coord::talk::PostRow {
        mentions: vec![],
        ..chat
    };
    send(&w, &chat, 0);
    wait_for(WAIT, "bob has it", || {
        b1.core
            .talk()
            .posts(&w.conv)
            .iter()
            .any(|p| p.post_id == chat.post_id)
    });
    // `space` is only a hint: a wrong one changes nothing.
    let req = b1
        .core
        .assistant_answer_request(&chat.post_id, "some-other-space", "let")
        .unwrap();
    wait_for(WAIT, "the tap is recorded", || {
        lines(b1).iter().any(|u| u.request == req)
    });
    assert!(answers(b1, &w.conv, &chat.post_id).is_empty());
    assert!(
        b1.core
            .assistant_answer_request("", &w.space, "let")
            .is_err()
    );
    assert!(
        b1.core
            .assistant_answer_request("post_x", &w.space, "shout")
            .is_err()
    );
    close(w);
}

/// A held line is a quiet `assistant.chat.update` row keyed by the
/// question's post id, so the person's other devices show it.
#[test]
fn a_held_line_is_a_row_every_device_shows() {
    let w = world(2, true);
    let (b1, b2) = (&w.bob[0], &w.bob[1]);
    let qs: Vec<_> = (0..9).map(|i| question(&w, &format!("q{i}?"))).collect();
    for q in &qs {
        send(&w, q, 0);
    }
    wait_for(WAIT, "the rate limit holds one on both computers", || {
        [b1, b2].iter().all(|d| {
            lines(d)
                .iter()
                .any(|u| u.key.starts_with("held:") && u.text.contains("were not answered"))
        })
    });
    close(w);
}
