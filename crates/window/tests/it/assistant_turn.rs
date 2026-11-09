//! One assistant, card 3: the window's chat turn, the closure Core's chat
//! worker runs each question through (one-assistant.md §5), over a fake
//! `claude` (never the real one). It posts a whole draft by itself, once,
//! only while no reply or posted thread exists; a Stop ends the child at
//! once, even while it waits on a tool call; a failure names the computer.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use irori_core::assistant_chat::{
    Asked, ChatEvent, ChatJob, ChatKind, ChatRow, Posted, PostedThread, Said,
};
use irori_window::assistant::AssistantEvent;

use crate::common::{Rig, unix_ms};

/// A stream-json line saying `text`.
fn delta(text: &str) -> String {
    serde_json::json!({
        "type": "stream_event",
        "event": {"type": "content_block_delta", "index": 0,
                  "delta": {"type": "text_delta", "text": text}},
    })
    .to_string()
}

const TOOL_SCREEN: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"mcp__irori__irori_screen","input":{}}]}}"#;
const SUCCESS: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"ok"}"#;

/// A fake `claude` that prints `lines`, then runs `then` (shell).
fn fake(dir: &Path, lines: &[String], then: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let out = dir.join("out.txt");
    std::fs::write(&out, lines.join("\n") + "\n").unwrap();
    let p = dir.join("claude");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\ncat >/dev/null &\ncat '{}'\n{then}\n",
            out.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

/// `r`'s Core is the assistant's computer, with question `turn` asked.
fn asked(r: &Rig, turn: &str, text: &str) {
    r.core.set_assistant_home(r.core.device_id()).unwrap();
    r.core
        .assistant_chat_write(ChatRow::Asked(Asked {
            turn: turn.into(),
            text: text.into(),
            device: r.core.device_id().into(),
            at: unix_ms(),
            ..Asked::default()
        }))
        .unwrap();
}

fn job(turn: &str) -> ChatJob {
    ChatJob {
        turn: turn.into(),
        prompt: "The person: post the lobby fix\n".into(),
        screen: "Page: Board\nIn view: #12".into(),
        grants: Vec::new(),
        cancel: Arc::new(AtomicBool::new(false)),
        kind: ChatKind::Chat,
        files: Vec::new(),
    }
}

const REPLY: &str = "Posting it.\n<<thread>>\nFix the lobby.\n<</thread>>";

/// A reply whose draft posts by itself starts exactly one thread from the
/// assistant's computer, and the run returns it to go in the reply row.
#[test]
fn a_whole_draft_posts_one_thread() {
    let r = Rig::new();
    let d = tempfile::tempdir().unwrap();
    let bin = fake(d.path(), &[delta(REPLY), SUCCESS.into()], "");
    asked(&r, "t1", "post the lobby fix");
    let seen: Mutex<Vec<ChatEvent>> = Mutex::default();
    let done = r.w.chat_turn_with(Ok(bin), &job("t1"), &|e| {
        seen.lock().unwrap().push(e);
        true
    });
    assert_eq!(done.error, "", "{done:?}");
    assert_eq!(done.text, REPLY);
    let p = done.posted.expect("the draft was posted");
    assert!(!p.id.is_empty() && !p.short.is_empty(), "{p:?}");
    let kinds: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|e| e.kind.clone())
        .collect();
    assert_eq!(kinds.first().map(String::as_str), Some("delta"));
    assert_eq!(kinds.last().map(String::as_str), Some("done"));
}

/// A run for a turn that already has a reply, or a posted thread, posts
/// nothing: another computer got there first.
#[test]
fn no_thread_once_the_turn_has_a_reply_or_a_post() {
    let r = Rig::new();
    let d = tempfile::tempdir().unwrap();
    let bin = fake(d.path(), &[delta(REPLY), SUCCESS.into()], "");
    asked(&r, "t2", "post the lobby fix");
    r.core
        .assistant_chat_write(ChatRow::Said(Said {
            turn: "t2".into(),
            text: "done elsewhere".into(),
            device: "another".into(),
            ..Said::default()
        }))
        .unwrap();
    let done = r.w.chat_turn_with(Ok(bin.clone()), &job("t2"), &|_| true);
    assert_eq!(done.posted, None);
    asked(&r, "t3", "post the lobby fix");
    r.core
        .assistant_chat_write(ChatRow::Posted(PostedThread {
            turn: "t3".into(),
            posted: Posted {
                id: "task:01X".into(),
                short: "9".into(),
                tag: String::new(),
            },
        }))
        .unwrap();
    let done = r.w.chat_turn_with(Ok(bin), &job("t3"), &|_| true);
    assert_eq!(done.posted, None);
}

/// A computer that isn't the assistant's computer posts nothing.
#[test]
fn no_thread_off_the_assistants_computer() {
    let r = Rig::new();
    let d = tempfile::tempdir().unwrap();
    let bin = fake(d.path(), &[delta(REPLY), SUCCESS.into()], "");
    asked(&r, "t4", "post the lobby fix");
    // The person picks a computer that isn't this one: no other device is
    // known, so a hand-written row names it.
    r.core
        .store()
        .append(
            "user",
            store::kinds::ASSISTANT_HOME,
            &irori_core::assistant_home::HomeRow {
                device: "elsewhere".into(),
                how: "person".into(),
                by: format!("user:{}", r.core.device_id()),
                at: gocompat::time::GoTime::unix(unix_ms() / 1000, 0),
            },
        )
        .unwrap();
    crate::common::wait("the pick is followed", || !r.core.is_assistant_home());
    let done = r.w.chat_turn_with(Ok(bin), &job("t4"), &|_| true);
    assert_eq!(done.posted, None);
}

/// A Stop during a run that waits on a tool call ends the child within a
/// second; the run comes back as stopped, not as a failure or a closing.
#[test]
fn a_stop_ends_the_child_within_a_second() {
    let r = Rig::new();
    let d = tempfile::tempdir().unwrap();
    let bin = fake(
        d.path(),
        &[delta("Let me look."), TOOL_SCREEN.into()],
        "exec sleep 30",
    );
    asked(&r, "t5", "what's on screen?");
    let j = job("t5");
    let cancel = j.cancel.clone();
    // Stop once it is waiting on its tool call.
    let waiting = Arc::new(AtomicBool::new(false));
    let w2 = waiting.clone();
    let tripped: Arc<Mutex<Option<Instant>>> = Arc::default();
    let t2 = tripped.clone();
    std::thread::spawn(move || {
        while !w2.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(200));
        *t2.lock().unwrap() = Some(Instant::now());
        cancel.store(true, Ordering::SeqCst);
    });
    let done = r.w.chat_turn_with(Ok(bin), &j, &|e| {
        if e.kind == "tool" {
            waiting.store(true, Ordering::SeqCst);
        }
        true
    });
    let took = tripped.lock().unwrap().expect("cancel tripped").elapsed();
    assert!(
        took < Duration::from_secs(1),
        "ended {took:?} after the Stop"
    );
    assert_eq!(done.error, "");
    assert!(!done.later);
    assert_eq!(done.text, "Let me look.");
    assert_eq!(done.looked.as_deref(), Some("Board, #12"));
    assert_eq!(done.posted, None);
}

/// A failed run says so in words that name the computer.
#[test]
fn a_failure_names_the_computer() {
    let r = Rig::new();
    let d = tempfile::tempdir().unwrap();
    let bin = fake(d.path(), &[], "echo 'Not logged in' >&2\nexit 1");
    asked(&r, "t6", "anything?");
    let done = r.w.chat_turn_with(Ok(bin), &job("t6"), &|_| true);
    let name = r.core.device_label(r.core.device_id());
    assert_eq!(
        done.error,
        format!(
            "Claude Code on {name} couldn't answer: it isn't signed in. Run `claude` in a terminal on {name} to sign in."
        )
    );
    assert!(!done.later);
}

/// A watch opened before a turn runs gets its events until it ends; one
/// opened while it runs first gets the text so far.
#[test]
fn a_watch_gets_the_turns_events() {
    let r = Rig::new();
    let d = tempfile::tempdir().unwrap();
    // It ends only once the test says so.
    let go = d.path().join("go");
    let bin = fake(
        d.path(),
        &[delta("Two cards "), delta("are blocked.")],
        &format!(
            "while [ ! -f '{}' ]; do sleep 0.05; done\necho '{SUCCESS}'",
            go.display()
        ),
    );
    asked(&r, "t7", "what's blocked?");
    let early: Arc<Mutex<Vec<AssistantEvent>>> = Arc::default();
    let e2 = early.clone();
    r.w.assistant_chat_watch(
        "t7",
        Box::new(move |e| {
            e2.lock().unwrap().push(e);
            true
        }),
    );
    let w = r.w.clone();
    let run = std::thread::spawn(move || w.chat_turn_with(Ok(bin), &job("t7"), &|_| true));
    // Join it while it runs.
    let late: Arc<Mutex<Vec<AssistantEvent>>> = Arc::default();
    crate::common::wait("the run has said its text", || {
        early
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.kind == "delta")
            .count()
            == 2
    });
    let l2 = late.clone();
    r.w.assistant_chat_watch(
        "t7",
        Box::new(move |e| {
            l2.lock().unwrap().push(e);
            true
        }),
    );
    std::fs::write(&go, "").unwrap();
    let done = run.join().unwrap();
    assert_eq!(done.text, "Two cards are blocked.");
    let early = early.lock().unwrap().clone();
    assert_eq!(early.last().unwrap(), &AssistantEvent::done());
    let late = late.lock().unwrap().clone();
    assert_eq!(
        late.first().unwrap(),
        &AssistantEvent::delta("Two cards are blocked.")
    );
    assert_eq!(late.last().unwrap(), &AssistantEvent::done());
}
