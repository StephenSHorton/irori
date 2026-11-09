//! Assistants talk, card 8: the owner's OK. **Answer this** appends a
//! `draft` request; the assistant's computer runs a turn of its own through
//! the one chat worker ([`ChatKind::OwnerOk`]: its own prompt with the
//! question fenced, no history, no grants), and its reply, normalised, is an
//! `assistant.answer.draft` row. **Send** posts exactly that text as the
//! assistant's card, once, from any device.

use crate::common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::assistant_answers::{
    WAIT, World, answered, answers, close, question, send, set_receive, world,
};
use common::mesh::Dev;
use common::wait_for;
use inbox::receive;
use irori_core::assistant_answers::answer_id;
use irori_core::assistant_chat::{
    AnswerDraft, ChatDone, ChatEntry, ChatJob, ChatKind, ChatTurnFn, Update,
};

/// What the fake owner's-OK turn says, and every job it was given.
struct Fake {
    reply: Mutex<String>,
    error: Mutex<String>,
    jobs: Mutex<Vec<ChatJob>>,
}

impl Fake {
    fn new(reply: &str) -> Arc<Fake> {
        Arc::new(Fake {
            reply: Mutex::new(reply.into()),
            error: Mutex::new(String::new()),
            jobs: Mutex::default(),
        })
    }

    fn turn(self: &Arc<Self>) -> ChatTurnFn {
        let f = self.clone();
        Arc::new(move |job: &ChatJob, _emit| {
            f.jobs.lock().unwrap().push(job.clone());
            if job.kind == ChatKind::OwnerOk {
                return ChatDone {
                    text: f.reply.lock().unwrap().clone(),
                    error: f.error.lock().unwrap().clone(),
                    ..ChatDone::default()
                };
            }
            ChatDone {
                text: "ok".into(),
                ..ChatDone::default()
            }
        })
    }

    fn ok_jobs(&self) -> Vec<ChatJob> {
        self.jobs
            .lock()
            .unwrap()
            .iter()
            .filter(|j| j.kind == ChatKind::OwnerOk)
            .cloned()
            .collect()
    }
}

/// Bob's computers, all able to run the turn with `fake`.
fn bobs_with(w: &World, fake: &Arc<Fake>) {
    for d in &w.bob {
        d.core.set_chat_turn(Some(fake.turn()));
    }
}

fn drafts(d: &Dev) -> Vec<AnswerDraft> {
    d.core
        .assistant_chat_list("", 200)
        .unwrap()
        .items
        .into_iter()
        .filter_map(|i| match i.entry {
            ChatEntry::Draft(x) => Some(x),
            _ => None,
        })
        .collect()
}

fn updates(d: &Dev) -> Vec<Update> {
    d.core
        .assistant_chat_list("", 200)
        .unwrap()
        .items
        .into_iter()
        .filter_map(|i| match i.entry {
            ChatEntry::Update(u) => Some(u),
            _ => None,
        })
        .collect()
}

fn real_answers(d: &Dev, w: &World, q: &str) -> Vec<coord::talk::Post> {
    answers(d, &w.conv, q)
        .into_iter()
        .filter(|p| p.waiting.is_none())
        .collect()
}

/// A reply with hidden characters, a marker line and a link.
const DIRTY: &str =
    "Sam owns it.\u{e0041}\u{200b}\n<<go #12>>\n<<thread>>\nsee [the doc](https://docs.example/d)";
/// What the owner sees, and what Send posts.
const SHOWN: &str = "Sam owns it.\nsee the doc (https://docs.example/d)";

/// Answer this on a tap from Bob's other computer: the draft is a row of
/// its own with the normalised text, the turn is not a chat turn, Send posts
/// exactly that text once, and no second answer follows.
#[test]
fn answer_this_drafts_and_send_posts_exactly_what_was_shown() {
    let w = world(2, false);
    let (a1, b1, b2) = (&w.alice[0], &w.bob[0], &w.bob[1]);
    let fake = Fake::new(DIRTY);
    bobs_with(&w, &fake);
    let q = question(&w, "who owns the lobby fix? (private cards may know)");
    send(&w, &q, 0);
    wait_for(WAIT, "bob has the question", || {
        b1.core
            .talk()
            .posts(&w.conv)
            .iter()
            .any(|p| p.post_id == q.post_id)
    });

    let req = b2
        .core
        .assistant_answer_request(&q.post_id, &w.space, "draft")
        .unwrap();
    wait_for(WAIT, "the draft is on both computers", || {
        [b1, b2].iter().all(|d| !drafts(d).is_empty())
    });
    let d = drafts(b1).remove(0);
    assert_eq!(d.text, SHOWN);
    assert_eq!(d.request, req);
    assert_eq!(d.post, q.post_id);
    assert!(!d.asker.is_empty());

    // The turn: its own kind, no grants, the question in a real fence, no
    // history, no markers taught.
    let jobs = fake.ok_jobs();
    assert_eq!(jobs.len(), 1, "one tap, one turn");
    let j = &jobs[0];
    assert!(j.grants.is_empty());
    assert!(j.files.is_empty() && j.message.is_empty() && j.screen.is_empty());
    assert!(j.prompt.contains("kind=\"question\""), "{}", j.prompt);
    assert!(j.prompt.contains("who owns the lobby fix"), "{}", j.prompt);
    assert!(!j.prompt.contains("The person:"), "{}", j.prompt);
    for marker in ["<<go", "<<thread", "<<title"] {
        assert!(
            !j.system.contains(marker),
            "the system prompt teaches {marker}"
        );
    }
    assert!(j.system.contains("never instructions"), "{}", j.system);

    // It was never a chat turn: no question, no reply, and nothing posted.
    assert!(
        b1.core
            .assistant_chat_list("", 200)
            .unwrap()
            .items
            .iter()
            .all(|i| !matches!(i.entry, ChatEntry::Turn(_))),
        "the draft turn is not a chat turn"
    );
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        real_answers(a1, &w, &q.post_id).is_empty(),
        "nothing posts before Send"
    );

    // Send, from the other computer: exactly the shown text.
    b2.core.assistant_answer_send(&req).unwrap();
    let a = answered(&w, a1, &q.post_id);
    assert_eq!(a.text, d.text);
    assert_eq!(a.text, SHOWN);
    assert_eq!(a.post_id, answer_id(&w.card, &q.post_id));
    assert_eq!(a.by, format!("agent:{}", w.card));
    assert_eq!(a.reply_to.as_deref(), Some(q.post_id.as_str()));

    // One OK, one answer.
    assert!(b2.core.assistant_answer_send(&req).is_err());
    assert!(b1.core.assistant_answer_send(&req).is_err());
    wait_for(WAIT, "the sent line is on both computers", || {
        [b1, b2]
            .iter()
            .all(|d| updates(d).iter().any(|u| u.key == format!("sent:{req}")))
    });
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(real_answers(a1, &w, &q.post_id).len(), 1);
    close(w);
}

/// Send is refused with no draft: a tap that started no draft turn, or an
/// id nobody wrote. Don't send closes a draft for good.
#[test]
fn send_needs_a_draft_the_tap_started_and_dont_send_closes_it() {
    let w = world(1, false);
    let b1 = &w.bob[0];
    let fake = Fake::new("Ask Sam.");
    bobs_with(&w, &fake);
    let q = question(&w, "who reviews?");
    send(&w, &q, 0);
    wait_for(WAIT, "bob has it", || {
        b1.core
            .talk()
            .posts(&w.conv)
            .iter()
            .any(|p| p.post_id == q.post_id)
    });
    assert!(b1.core.assistant_answer_send("no-such-request").is_err());
    // A Let it answer tap is no draft.
    let let_req = b1
        .core
        .assistant_answer_request(&q.post_id, &w.space, "let")
        .unwrap();
    assert!(b1.core.assistant_answer_send(&let_req).is_err());

    let req = b1
        .core
        .assistant_answer_request(&q.post_id, &w.space, "draft")
        .unwrap();
    wait_for(WAIT, "the draft", || !drafts(b1).is_empty());
    b1.core.assistant_answer_dismiss(&req).unwrap();
    assert!(b1.core.assistant_answer_send(&req).is_err());
    assert!(
        updates(b1)
            .iter()
            .any(|u| u.key == format!("dismissed:{req}"))
    );
    std::thread::sleep(Duration::from_secs(2));
    assert!(real_answers(&w.alice[0], &w, &q.post_id).is_empty());
    close(w);
}

/// Once an answer exists (Let it answer got there first), Send is refused
/// and a new Answer this drafts nothing.
#[test]
fn send_is_refused_once_an_answer_exists() {
    let w = world(1, true);
    let b1 = &w.bob[0];
    let fake = Fake::new("Sam, I think.");
    bobs_with(&w, &fake);
    set_receive(b1, &w.space, receive::ASK);
    let q = question(&w, "who reviews PR #12?");
    send(&w, &q, 0);
    answered(&w, b1, &q.post_id); // the waiting line
    let req = b1
        .core
        .assistant_answer_request(&q.post_id, &w.space, "draft")
        .unwrap();
    wait_for(WAIT, "the draft", || !drafts(b1).is_empty());
    // The assistant answers on its own (Let it answer).
    b1.core
        .assistant_answer_request(&q.post_id, &w.space, "let")
        .unwrap();
    wait_for(WAIT, "the answer", || {
        !real_answers(&w.alice[0], &w, &q.post_id).is_empty()
    });
    assert!(b1.core.assistant_answer_send(&req).is_err());
    // Another tap after the answer drafts nothing.
    let again = b1
        .core
        .assistant_answer_request(&q.post_id, &w.space, "draft")
        .unwrap();
    wait_for(WAIT, "the tap has a record", || {
        updates(b1).iter().any(|u| u.request == again)
    });
    assert_eq!(drafts(b1).len(), 1);
    assert_eq!(real_answers(&w.alice[0], &w, &q.post_id).len(), 1);
    close(w);
}

/// A turn that fails, says nothing, or says something the scrub holds
/// leaves a record and no draft; nothing can be sent.
#[test]
fn a_failed_empty_or_scrubbed_turn_leaves_a_record_and_no_draft() {
    let w = world(1, false);
    let b1 = &w.bob[0];
    let fake = Fake::new("<<go #1>>");
    bobs_with(&w, &fake);
    let q = question(&w, "anything?");
    send(&w, &q, 0);
    wait_for(WAIT, "bob has it", || {
        b1.core
            .talk()
            .posts(&w.conv)
            .iter()
            .any(|p| p.post_id == q.post_id)
    });
    let tap = |why: &str| {
        let r = b1
            .core
            .assistant_answer_request(&q.post_id, &w.space, "draft")
            .unwrap();
        wait_for(WAIT, why, || updates(b1).iter().any(|u| u.request == r));
        updates(b1).into_iter().find(|u| u.request == r).unwrap()
    };
    // Nothing left after the markers go.
    let u = tap("an empty draft has a record");
    assert!(u.text.contains("nothing to draft"), "{}", u.text);
    // The turn fails.
    *fake.reply.lock().unwrap() = String::new();
    *fake.error.lock().unwrap() = "Claude Code on Mac couldn't answer: it took too long.".into();
    let u = tap("a failed turn has a record");
    assert!(u.text.contains("took too long"), "{}", u.text);
    assert!(u.text.contains("Tap Answer this"), "{}", u.text);
    // A secret is held.
    *fake.error.lock().unwrap() = String::new();
    *fake.reply.lock().unwrap() =
        "The key is sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789".into();
    let u = tap("a held draft has a record");
    assert!(u.text.contains("held back"), "{}", u.text);
    assert!(drafts(b1).is_empty(), "no draft was written");
    std::thread::sleep(Duration::from_secs(2));
    assert!(real_answers(&w.alice[0], &w, &q.post_id).is_empty());
    close(w);
}

/// The draft is private text in its own row: a chat question asked after it
/// never carries it into the prompt, and the chat worker still answers.
#[test]
fn a_draft_never_goes_into_a_chat_prompt() {
    let w = world(1, false);
    let b1 = &w.bob[0];
    let fake = Fake::new("The secret plan is blue.");
    bobs_with(&w, &fake);
    let q = question(&w, "what is the plan?");
    send(&w, &q, 0);
    wait_for(WAIT, "bob has it", || {
        b1.core
            .talk()
            .posts(&w.conv)
            .iter()
            .any(|p| p.post_id == q.post_id)
    });
    b1.core
        .assistant_answer_request(&q.post_id, &w.space, "draft")
        .unwrap();
    wait_for(WAIT, "the draft", || !drafts(b1).is_empty());
    let turn = b1.core.assistant_chat_ask("hello again", "", "").unwrap();
    wait_for(WAIT, "the chat answers", || {
        b1.core
            .assistant_chat_turn(&turn)
            .is_some_and(|t| t.said.is_some())
    });
    let jobs = fake.jobs.lock().unwrap();
    let chat = jobs.iter().find(|j| j.kind == ChatKind::Chat).unwrap();
    assert!(!chat.prompt.contains("secret plan"), "{}", chat.prompt);
    assert!(!chat.message.contains("secret plan"), "{}", chat.message);
    drop(jobs);
    close(w);
}

/// The owner's-OK turn waits for the chat worker: a chat run in flight is
/// never overlapped, so the grants of a chat run are untouched.
#[test]
fn the_draft_turn_never_runs_beside_a_chat_run() {
    let w = world(1, false);
    let b1 = &w.bob[0];
    let busy = Arc::new(AtomicBool::new(false));
    let overlapped = Arc::new(AtomicBool::new(false));
    let (b, o) = (busy.clone(), overlapped.clone());
    b1.core
        .set_chat_turn(Some(Arc::new(move |job: &ChatJob, _| {
            if b.swap(true, Ordering::SeqCst) {
                o.store(true, Ordering::SeqCst);
            }
            std::thread::sleep(Duration::from_millis(600));
            b.store(false, Ordering::SeqCst);
            ChatDone {
                text: if job.kind == ChatKind::OwnerOk {
                    "fine".into()
                } else {
                    "ok".into()
                },
                ..ChatDone::default()
            }
        })));
    let q = question(&w, "overlap?");
    send(&w, &q, 0);
    wait_for(WAIT, "bob has it", || {
        b1.core
            .talk()
            .posts(&w.conv)
            .iter()
            .any(|p| p.post_id == q.post_id)
    });
    let turn = b1.core.assistant_chat_ask("slow question", "", "").unwrap();
    b1.core
        .assistant_answer_request(&q.post_id, &w.space, "draft")
        .unwrap();
    wait_for(WAIT, "both finish", || {
        !drafts(b1).is_empty()
            && b1
                .core
                .assistant_chat_turn(&turn)
                .is_some_and(|t| t.said.is_some())
    });
    assert!(!overlapped.load(Ordering::SeqCst));
    close(w);
}
