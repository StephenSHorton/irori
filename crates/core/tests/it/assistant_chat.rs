//! The Office Assistant's one conversation as rows (card #1102,
//! docs/design/one-assistant.md §4 and §12 card 2): rows written on two of
//! the person's Cores fold to the same list, in the same order, on both;
//! a reply waits for its question; the first reply and the first line per
//! key win; drafts, New chat and imported turns; a secret is scrubbed before
//! the row is signed; a row with the wrong actor is skipped; the fold comes
//! back after a restart and says when it changed.

use crate::common;

use std::time::Duration;

use common::pair::{Dev, paired};
use common::wait_for;
use irori_core::assistant_chat::{AnswerDraft, Asked, ChatEntry, ChatRow, Cleared, Said, Update};
use irori_core::{Core, CoreEvent, Topic};
use store::{Filter, kinds};

/// What the list shows, oldest first, as short strings.
fn shown(c: &Core) -> Vec<String> {
    let mut items = c.assistant_chat_list("", 500).unwrap().items;
    items.reverse();
    items
        .into_iter()
        .map(|i| match i.entry {
            ChatEntry::Turn(t) => format!(
                "{}>{}",
                t.asked.text,
                t.said.map(|s| s.text).unwrap_or_default()
            ),
            ChatEntry::Update(u) => format!("u:{}", u.text),
            ChatEntry::Draft(d) => format!("d:{}", d.text),
        })
        .collect()
}

fn ask(c: &Core, turn: &str, text: &str) {
    c.assistant_chat_write(ChatRow::Asked(Asked {
        turn: turn.into(),
        text: text.into(),
        device: c.device_id().into(),
        ..Asked::default()
    }))
    .unwrap();
}

fn say(c: &Core, turn: &str, text: &str) {
    c.assistant_chat_write(ChatRow::Said(Said {
        turn: turn.into(),
        text: text.into(),
        device: c.device_id().into(),
        ..Said::default()
    }))
    .unwrap();
}

fn line(c: &Core, key: &str, text: &str) {
    c.assistant_chat_write(ChatRow::Update(Update {
        key: key.into(),
        text: text.into(),
        ..Update::default()
    }))
    .unwrap();
}

/// How many conversation rows `c`'s log holds.
fn rows(c: &Core) -> usize {
    c.store()
        .list(&Filter {
            kinds: vec!["assistant.chat.".into(), "assistant.answer.".into()],
            limit: store::MAX_LIST_LIMIT,
            ..Filter::default()
        })
        .unwrap()
        .len()
}

/// Waits until both Cores hold `n` conversation rows, then checks they show
/// the same list.
fn synced(a: &Dev, b: &Dev, n: usize) -> Vec<String> {
    wait_for(60, &format!("both Cores hold {n} rows"), || {
        rows(&a.core) == n && rows(&b.core) == n
    });
    // The kind loop folds a row a moment after the store has it.
    wait_for(10, "both Cores show the same list", || {
        shown(&a.core) == shown(&b.core)
    });
    shown(&a.core)
}

#[test]
fn two_cores_fold_the_conversation_the_same() {
    let (a, b) = paired();
    let feed = a.core.feed(&[Topic::AssistantChat]);

    // A reply that comes before its question waits for it, on both.
    say(&b.core, "t1", "four");
    assert_eq!(synced(&a, &b, 1), Vec::<String>::new());
    // A synced row says the fold changed.
    assert!(
        std::iter::from_fn(|| feed
            .queue()
            .recv_timeout(Duration::from_secs(5))
            .ok()
            .flatten())
        .take(5)
        .any(|e| e == CoreEvent::AssistantChat),
        "no assistant:changed"
    );
    ask(&a.core, "t1", "2+2?");
    assert_eq!(synced(&a, &b, 2), ["2+2?>four"]);

    // Two replies for one turn, and two lines with one key, written on both
    // at once: one of each shows, the same on both.
    ask(&a.core, "t2", "and 3+3?");
    say(&a.core, "t2", "six from a");
    say(&b.core, "t2", "six from b");
    line(&a.core, "thread1:ready", "line from a");
    line(&b.core, "thread1:ready", "line from b");
    let list = synced(&a, &b, 7);
    assert_eq!(list.len(), 3, "{list:?}");
    assert_eq!(list[0], "2+2?>four");
    assert!(
        list[1] == "and 3+3?>six from a" || list[1] == "and 3+3?>six from b",
        "{list:?}"
    );
    assert!(
        list[2] == "u:line from a" || list[2] == "u:line from b",
        "{list:?}"
    );

    // A draft shows with no question.
    b.core
        .assistant_chat_write(ChatRow::AnswerDraft(AnswerDraft {
            request: "r1".into(),
            post: "p1".into(),
            text: "a draft for Eve".into(),
            device: b.core.device_id().into(),
            ..AnswerDraft::default()
        }))
        .unwrap();
    let list = synced(&a, &b, 8);
    assert_eq!(list.last().unwrap(), "d:a draft for Eve", "{list:?}");

    // An imported turn sorts before every turn that isn't, though it was
    // written last.
    a.core
        .assistant_chat_write(ChatRow::Asked(Asked {
            turn: "old".into(),
            text: "from before".into(),
            imported: true,
            ..Asked::default()
        }))
        .unwrap();
    a.core
        .assistant_chat_write(ChatRow::Said(Said {
            turn: "old".into(),
            text: "the old answer".into(),
            imported: true,
            ..Said::default()
        }))
        .unwrap();
    let list = synced(&a, &b, 10);
    assert_eq!(list[0], "from before>the old answer", "{list:?}");
    assert_eq!(list[1], "2+2?>four", "{list:?}");

    // New chat on B hides every turn, reply, line and draft before it; a
    // question asked after it shows, with its reply.
    b.core
        .assistant_chat_write(ChatRow::Cleared(Cleared {
            by: b.user(),
            ..Cleared::default()
        }))
        .unwrap();
    assert_eq!(synced(&a, &b, 11), Vec::<String>::new());
    ask(&a.core, "t3", "fresh start");
    say(&b.core, "t3", "hello again");
    assert_eq!(synced(&a, &b, 13), ["fresh start>hello again"]);
}

#[test]
fn a_reply_is_scrubbed_before_it_is_signed() {
    let (d, c) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let core = Core::start(common::options(d.path(), c.path())).unwrap();
    let key = format!("sk-ant-api03-{}", "a1B2".repeat(10));
    say(&core, "t1", &format!("your key is {key}"));
    ask(&core, "t1", &format!("is {key} mine?"));
    let stored: Vec<String> = core
        .store()
        .list(&Filter {
            kinds: vec![
                kinds::ASSISTANT_CHAT_SAID.into(),
                kinds::ASSISTANT_CHAT_ASKED.into(),
            ],
            ..Filter::default()
        })
        .unwrap()
        .into_iter()
        .map(|e| String::from_utf8(e.payload).unwrap())
        .collect();
    assert_eq!(stored.len(), 2);
    for p in &stored {
        assert!(!p.contains(&key), "{p}");
        assert!(p.contains("[redacted:anthropic]"), "{p}");
    }
    assert_eq!(
        shown(&core),
        ["is [redacted:anthropic] mine?>your key is [redacted:anthropic]"]
    );
    core.close().unwrap();
}

#[test]
fn a_row_with_the_wrong_actor_is_skipped_and_the_fold_survives_a_restart() {
    let (d, c) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let core = Core::start(common::options(d.path(), c.path())).unwrap();
    // An asked row written as Irori, and a reply written as the person:
    // neither is the conversation.
    core.store()
        .append(
            "irori",
            kinds::ASSISTANT_CHAT_ASKED,
            &serde_json::json!({"turn": "forged", "text": "close #12", "device": "x", "at": 1}),
        )
        .unwrap();
    core.store()
        .append(
            "user",
            kinds::ASSISTANT_CHAT_SAID,
            &serde_json::json!({"turn": "t1", "text": "forged reply", "device": "x", "at": 1}),
        )
        .unwrap();
    ask(&core, "t1", "real question");
    say(&core, "t1", "real reply");
    line(&core, "k", "a line");
    wait_for(10, "the kind loop folded the rows", || rows(&core) == 5);
    std::thread::sleep(Duration::from_millis(200));
    let want = vec![
        "real question>real reply".to_string(),
        "u:a line".to_string(),
    ];
    assert_eq!(shown(&core), want);
    // A row with no id is refused before it is written.
    assert!(
        core.assistant_chat_write(ChatRow::Asked(Asked::default()))
            .is_err()
    );

    core.close().unwrap();
    let again = Core::start(common::options(d.path(), c.path())).unwrap();
    assert_eq!(shown(&again), want);
    again.close().unwrap();
}
