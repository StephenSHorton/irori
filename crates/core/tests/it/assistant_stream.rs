//! One assistant, card 7: the reply streams to the device that asked
//! (one-assistant.md §5, §12). A fake chat turn stands in for the window's
//! run and emits events the test steps through.

use crate::common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::chat::spawner;
use common::mesh::{Dev, person};
use common::wait_for;
use irori_core::assistant_chat::{ChatDone, ChatEvent, ChatJob, ChatTurnFn};

const WAIT: u64 = 90;

/// A turn that says its `parts` one by one, each after `gate` reaches its
/// index, then ends with `text` as the reply.
struct Script {
    parts: Vec<String>,
    text: String,
    gate: AtomicUsize,
    said: AtomicUsize,
    running: Mutex<Vec<String>>,
}

impl Script {
    fn new(parts: &[&str], text: &str) -> Arc<Script> {
        Arc::new(Script {
            parts: parts.iter().map(|p| p.to_string()).collect(),
            text: text.into(),
            gate: AtomicUsize::new(1),
            said: AtomicUsize::new(0),
            running: Mutex::default(),
        })
    }

    fn turn(self: &Arc<Self>) -> ChatTurnFn {
        let s = self.clone();
        Arc::new(move |job: &ChatJob, emit: &dyn Fn(ChatEvent) -> bool| {
            s.running.lock().unwrap().push(job.turn.clone());
            for (i, p) in s.parts.iter().enumerate() {
                while s.gate.load(Ordering::SeqCst) <= i {
                    std::thread::sleep(Duration::from_millis(10));
                }
                emit(ChatEvent {
                    kind: "delta".into(),
                    text: p.clone(),
                });
                s.said.store(i + 1, Ordering::SeqCst);
            }
            emit(ChatEvent {
                kind: "done".into(),
                text: String::new(),
            });
            ChatDone {
                text: s.text.clone(),
                ..ChatDone::default()
            }
        })
    }

    fn open(&self, n: usize) {
        self.gate.store(n, Ordering::SeqCst);
    }
}

/// Two connected Cores, the first the assistant's computer running `s`.
fn pair(s: &Arc<Script>) -> (Dev, Dev) {
    let mut devs = person("stephen", 2);
    let (b, a) = (devs.pop().unwrap(), devs.pop().unwrap());
    for d in [&a, &b] {
        d.core.set_spawner(Some(spawner(&Arc::new(
            std::sync::atomic::AtomicBool::new(true),
        ))));
    }
    a.core.set_chat_turn(Some(s.turn()));
    a.core.set_assistant_home(a.core.device_id()).unwrap();
    wait_for(WAIT, "B follows the pick", || {
        b.core.assistant_home().as_deref() == Some(a.core.device_id())
    });
    (a, b)
}

fn drain(rx: &std::sync::mpsc::Receiver<ChatEvent>) -> Vec<ChatEvent> {
    let mut out = Vec::new();
    loop {
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(e) => out.push(e),
            Err(RecvTimeoutError::Disconnected) => return out,
            Err(RecvTimeoutError::Timeout) => panic!("the stream never ended: {out:?}"),
        }
    }
}

fn deltas(evs: &[ChatEvent]) -> Vec<String> {
    evs.iter()
        .filter(|e| e.kind == "delta")
        .map(|e| e.text.clone())
        .collect()
}

/// A second Core connected to the assistant's computer gets the deltas of
/// a running turn in order, the text said before it opened first, and the
/// reply row is the final text.
#[test]
fn a_second_core_gets_the_deltas_of_a_running_turn_in_order() {
    let s = Script::new(&["one ", "two ", "three"], "one two three");
    let (a, b) = pair(&s);
    let turn = b.core.assistant_chat_ask("count", "", "").unwrap();
    wait_for(WAIT, "the first part is said", || {
        s.said.load(Ordering::SeqCst) == 1
    });
    let rx = b.core.assistant_stream(a.core.device_id(), &turn).unwrap();
    s.open(2);
    wait_for(WAIT, "the second part is said", || {
        s.said.load(Ordering::SeqCst) == 2
    });
    s.open(3);
    let evs = drain(&rx);
    let joined = deltas(&evs).concat();
    assert_eq!(joined, "one two three", "{evs:?}");
    assert_eq!(evs.last().map(|e| e.kind.as_str()), Some("done"));
    wait_for(WAIT, "the row reaches the asking device", || {
        b.core
            .assistant_chat_turn(&turn)
            .is_some_and(|t| t.said.is_some())
    });
    let t = b.core.assistant_chat_turn(&turn).unwrap();
    assert_eq!(t.said.unwrap().text, "one two three");
    a.core.close().unwrap();
    b.core.close().unwrap();
}

/// A stream that is dropped halfway shows no error: the reply row still
/// arrives. A turn that isn't running gives a stream that just ends.
#[test]
fn a_dropped_stream_falls_back_to_the_row() {
    let s = Script::new(&["half ", "the end"], "half the end");
    let (a, b) = pair(&s);
    let turn = b.core.assistant_chat_ask("go", "", "").unwrap();
    wait_for(WAIT, "the first part is said", || {
        s.said.load(Ordering::SeqCst) == 1
    });
    let rx = b.core.assistant_stream(a.core.device_id(), &turn).unwrap();
    let first = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(first.text, "half ");
    drop(rx);
    s.open(2);
    wait_for(WAIT, "the row lands without the stream", || {
        b.core
            .assistant_chat_turn(&turn)
            .is_some_and(|t| t.said.is_some())
    });
    let t = b.core.assistant_chat_turn(&turn).unwrap();
    assert_eq!(t.said.unwrap().text, "half the end");
    // The turn is over: nothing to follow, and no error.
    let rx = b.core.assistant_stream(a.core.device_id(), &turn).unwrap();
    assert!(drain(&rx).is_empty());
    let rx = b
        .core
        .assistant_stream(a.core.device_id(), "no-such-turn")
        .unwrap();
    assert!(drain(&rx).is_empty());
    a.core.close().unwrap();
    b.core.close().unwrap();
}

/// What the stream carried is a view: a key in it is `[redacted:anthropic]`
/// in the row, and the row's text is final.
#[test]
fn the_row_text_is_final_when_the_stream_carried_a_key() {
    let key = format!("sk-ant-api03-{}", "a1B2".repeat(10));
    let line = format!("your key is {key}");
    let s = Script::new(&["your key is ", &key], &line);
    let (a, b) = pair(&s);
    let turn = b.core.assistant_chat_ask("what is my key", "", "").unwrap();
    wait_for(WAIT, "the first part is said", || {
        s.said.load(Ordering::SeqCst) == 1
    });
    let rx = b.core.assistant_stream(a.core.device_id(), &turn).unwrap();
    s.open(2);
    let evs = drain(&rx);
    let streamed = deltas(&evs).concat();
    wait_for(WAIT, "the row reaches the asking device", || {
        b.core
            .assistant_chat_turn(&turn)
            .is_some_and(|t| t.said.is_some())
    });
    let row = b
        .core
        .assistant_chat_turn(&turn)
        .unwrap()
        .said
        .unwrap()
        .text;
    assert!(row.contains("[redacted:anthropic]"), "{row}");
    assert!(!row.contains(&key), "{row}");
    assert_eq!(streamed, line, "the stream carried the key");
    a.core.close().unwrap();
    b.core.close().unwrap();
}
