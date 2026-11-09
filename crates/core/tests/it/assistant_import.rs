//! One assistant, card 6: the assistant's computer brings its old chat
//! history in once (one-assistant.md §10). The host decides: only the
//! assistant's computer, only once. Imported rows are never run.

use crate::common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::pair::paired;
use irori_core::assistant_chat::{
    ALREADY_IMPORTED, Act, ChatDone, ChatEntry, ChatRow, ChatTurnFn, Cleared, ImportEntry,
    NO_REPLY, Posted,
};
use irori_core::{Core, Options};

struct Solo {
    core: Core,
    _dirs: Vec<tempfile::TempDir>,
}

fn solo() -> Solo {
    let (d, c) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let o: Options = common::options(d.path(), c.path());
    Solo {
        core: Core::start(o).unwrap(),
        _dirs: vec![d, c],
    }
}

fn home_here(c: &Core) {
    c.set_assistant_home(c.device_id()).unwrap();
    assert!(c.is_assistant_home());
}

fn me(text: &str) -> ImportEntry {
    ImportEntry {
        role: "person".into(),
        text: text.into(),
        ..ImportEntry::default()
    }
}

fn bot(text: &str) -> ImportEntry {
    ImportEntry {
        role: "assistant".into(),
        text: text.into(),
        ..ImportEntry::default()
    }
}

/// What the list shows, oldest first.
fn shown(c: &Core) -> Vec<String> {
    let mut items = c.assistant_chat_list("", 500).unwrap().items;
    items.reverse();
    items
        .into_iter()
        .map(|i| match i.entry {
            ChatEntry::Turn(t) => {
                let s = t.said.unwrap_or_default();
                format!(
                    "{}>{}{}",
                    t.asked.text,
                    s.text,
                    if s.error.is_empty() {
                        String::new()
                    } else {
                        format!(" !{}", s.error)
                    }
                )
            }
            ChatEntry::Update(u) => format!("u:{}", u.text),
            ChatEntry::Draft(d) => format!("d:{}", d.text),
        })
        .collect()
}

/// A chat turn that counts what it is asked to run.
fn counting() -> (Arc<AtomicUsize>, ChatTurnFn) {
    let n = Arc::new(AtomicUsize::new(0));
    let c = n.clone();
    let f: ChatTurnFn = Arc::new(move |_job, _emit| {
        c.fetch_add(1, Ordering::SeqCst);
        ChatDone {
            text: "ran".into(),
            ..ChatDone::default()
        }
    });
    (n, f)
}

/// The assistant's computer imports its history once, in order, mapped per
/// §10; a second call is refused and writes nothing.
#[test]
fn the_assistants_computer_imports_once_in_order_and_a_second_call_is_refused() {
    let s = solo();
    home_here(&s.core);
    let history = vec![
        // Left when the cap cut the question: dropped.
        bot("a reply whose question is gone"),
        me("hello"),
        ImportEntry {
            posted: Some(Posted {
                id: "task:01X".into(),
                short: "9".into(),
                tag: String::new(),
            }),
            looked: "Page: Board".into(),
            ..bot("hi there")
        },
        ImportEntry {
            note: true,
            changed: true,
            ..bot("Closed #3")
        },
        ImportEntry {
            update: "thread-1".into(),
            act: Some(Act {
                kind: "answer".into(),
                card: "task:01C".into(),
                blocker: "b1".into(),
            }),
            ..bot("#5 needs you")
        },
        // An older build's reply sent on a thread: dropped.
        ImportEntry {
            to: "#5".into(),
            ..me("yes go ahead")
        },
        bot("an answer to the dropped one"),
        me("what failed?"),
        ImportEntry {
            note: true,
            ..bot("The assistant couldn't answer just now.")
        },
        me("close #12"),
    ];
    let done = s.core.assistant_chat_import(&history).unwrap();
    assert_eq!((done.turns, done.updates), (3, 1));
    assert_eq!(
        shown(&s.core),
        [
            "hello>hi there",
            "u:#5 needs you",
            "what failed?> !The assistant couldn't answer just now.",
            &format!("close #12> !{NO_REPLY}"),
        ]
    );
    let hello = s
        .core
        .assistant_chat_list("", 500)
        .unwrap()
        .items
        .into_iter()
        .rev()
        .find_map(|i| match i.entry {
            ChatEntry::Turn(t) if t.asked.text == "hello" => Some(t),
            _ => None,
        })
        .unwrap();
    assert!(hello.asked.imported);
    let said = hello.said.unwrap();
    assert!(said.imported);
    assert_eq!(said.changed, ["Closed #3"]);
    assert_eq!(said.posted.unwrap().short, "9");
    assert_eq!(said.looked, "Page: Board");

    let before = shown(&s.core);
    let err = s.core.assistant_chat_import(&history).unwrap_err();
    assert_eq!(err, ALREADY_IMPORTED);
    assert_eq!(shown(&s.core), before);
    s.core.close().unwrap();
}

/// A device that isn't the assistant's computer imports nothing; a claim that
/// lands after the first try still imports.
#[test]
fn only_the_assistants_computer_imports_and_a_later_claim_still_does() {
    let s = solo();
    let history = vec![me("hello"), bot("hi")];
    // No computer recorded yet.
    assert!(s.core.assistant_chat_import(&history).is_err());
    assert!(shown(&s.core).is_empty());
    home_here(&s.core);
    s.core.assistant_chat_import(&history).unwrap();
    assert_eq!(shown(&s.core), ["hello>hi"]);
    s.core.close().unwrap();

    // Another device is the computer: this one refuses.
    let (a, b) = paired();
    a.core.set_assistant_home(a.core.device_id()).unwrap();
    common::wait_for(60, "b learns the computer", || {
        b.core.assistant_home().as_deref() == Some(a.core.device_id())
    });
    assert!(b.core.assistant_chat_import(&history).is_err());
    assert!(shown(&b.core).is_empty());
}

/// A history ending in an unanswered "close #12" imports with a "(no reply)"
/// row, runs no turn and closes nothing; an imported `to` entry never runs.
#[test]
fn imported_questions_never_run() {
    let s = solo();
    home_here(&s.core);
    let (ran, f) = counting();
    s.core.set_chat_turn(Some(f));
    s.core
        .assistant_chat_import(&[
            me("hello"),
            bot("hi"),
            ImportEntry {
                to: "#5".into(),
                ..me("ok")
            },
            me("close #12"),
        ])
        .unwrap();
    assert_eq!(
        shown(&s.core),
        ["hello>hi", &format!("close #12> !{NO_REPLY}")]
    );
    common::chat::holds_for(6, "no turn ran for an imported question", || {
        ran.load(Ordering::SeqCst) == 0
    });
    // A question asked after it still runs.
    s.core.assistant_chat_ask("and now?", "", "").unwrap();
    common::wait_for(30, "the new question runs", || {
        ran.load(Ordering::SeqCst) == 1
    });
    s.core.close().unwrap();
}

/// New chat removes the imported history, and the import stays done.
#[test]
fn new_chat_removes_the_imported_history_and_the_import_stays_done() {
    let s = solo();
    home_here(&s.core);
    s.core
        .assistant_chat_import(&[me("hello"), bot("hi")])
        .unwrap();
    s.core
        .assistant_chat_write(ChatRow::Cleared(Cleared::default()))
        .unwrap();
    assert!(shown(&s.core).is_empty());
    assert_eq!(
        s.core.assistant_chat_import(&[me("again")]).unwrap_err(),
        ALREADY_IMPORTED
    );
    assert!(shown(&s.core).is_empty());
    s.core.close().unwrap();
}
