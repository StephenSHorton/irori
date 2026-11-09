//! One assistant, card 8: the chat's held session (one-assistant.md §8),
//! through Core's worker and the window's turn, over a fake `claude` that
//! stays running and answers one stream-json line at a time (never the
//! real one). A second question goes to the same process with only its
//! message; grants begin and end per message; New chat, a change of model,
//! a stopped turn and context past 60k each seed a new process from the
//! rows; what the session hasn't seen comes as "Since we last spoke" lines;
//! quitting writes no reply; and ten questions from two devices are
//! answered by one process.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use irori_core::assistant_chat::{
    ChatDone, ChatEvent, ChatJob, ChatRow, Cleared, SINCE_HEAD, Update,
};
use irori_window::Window;

use crate::common::{Rig, unix_ms};

/// The assistant's close tool (`tools::TOOL_CLOSE`).
const CLOSE: &str = "assistant_close";

/// A fake held `claude`: counts its starts, keeps its argv and pid, writes
/// each stdin line to `in.txt`, and answers each with a delta and a
/// `result` whose input tokens are `<dir>/context` (1000 when missing).
/// With `<dir>/block` it waits for `<dir>/go-<n>` before answering its
/// n-th line.
fn fake_held(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join("claude");
    let d = dir.display();
    let script = format!(
        r#"#!/bin/sh
n=$(cat '{d}/starts' 2>/dev/null || echo 0)
echo $((n+1)) > '{d}/starts'
echo "$@" > '{d}/argv.txt'
echo $$ > '{d}/pid'
i=0
while IFS= read -r line; do
  i=$((i+1))
  printf '%s\n' "$line" >> '{d}/in.txt'
  if [ -f '{d}/block' ]; then
    while [ ! -f '{d}/go-'$i ]; do sleep 0.05; done
  fi
  ctx=$(cat '{d}/context' 2>/dev/null || echo 1000)
  printf '{{"type":"stream_event","event":{{"type":"content_block_delta","index":0,"delta":{{"type":"text_delta","text":"ok %s"}}}}}}\n' "$i"
  printf '{{"type":"result","subtype":"success","is_error":false,"result":"ok %s","usage":{{"input_tokens":%s,"output_tokens":0}}}}\n' "$i" "$ctx"
done
"#
    );
    std::fs::write(&p, script).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn starts(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join("starts"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// The text of each message written to the fake's stdin, in order.
fn messages(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("in.txt"))
        .unwrap_or_default()
        .lines()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["message"]["content"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect()
}

/// `w`'s Core answers chat questions through `w`'s turn on `bin`.
fn install(w: &Arc<Window>, bin: &Path) {
    let weak = Arc::downgrade(w);
    let bin = bin.to_path_buf();
    w.core.set_chat_turn(Some(Arc::new(
        move |job: &ChatJob, emit: &dyn Fn(ChatEvent) -> bool| match weak.upgrade() {
            Some(w) => w.chat_turn_with(Ok(bin.clone()), job, emit),
            None => ChatDone {
                later: true,
                ..ChatDone::default()
            },
        },
    )));
}

/// A Rig that is the assistant's computer, answering through the fake.
fn rig(dir: &Path) -> Rig {
    let r = Rig::new();
    r.core.set_assistant_home(r.core.device_id()).unwrap();
    install(&r.w, &fake_held(dir));
    r
}

fn wait_for(what: &str, secs: u64, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Asks `text` and waits for its reply's text.
fn ask(r: &Rig, text: &str) -> String {
    let turn = r.core.assistant_chat_ask(text, "", "").unwrap();
    replied(r, &turn)
}

fn replied(r: &Rig, turn: &str) -> String {
    let mut out = String::new();
    wait_for(&format!("a reply to {turn}"), 20, || {
        match r.core.assistant_chat_turn(turn).and_then(|t| t.said) {
            Some(s) => {
                out = if s.error.is_empty() {
                    s.text
                } else {
                    format!("error: {}", s.error)
                };
                true
            }
            None => false,
        }
    });
    out
}

#[test]
fn a_second_question_goes_to_the_same_process_alone() {
    let f = tempfile::tempdir().unwrap();
    let r = rig(f.path());
    assert_eq!(ask(&r, "what is blocked?"), "ok 1");
    assert_eq!(ask(&r, "and the second one?"), "ok 2");
    assert_eq!(starts(f.path()), 1, "one process for both");
    let m = messages(f.path());
    assert_eq!(m.len(), 2, "{m:?}");
    assert_eq!(m[0], "The person: what is blocked?\n");
    // The held session already has the first turn: the question alone.
    assert_eq!(m[1], "The person: and the second one?\n");
    // Its argv is the chat's: held, with the chat's tools.
    let argv = std::fs::read_to_string(f.path().join("argv.txt")).unwrap();
    assert!(argv.contains("--input-format stream-json"), "{argv}");
    assert!(
        argv.contains("--mcp-config") && argv.contains("--allowedTools mcp__irori"),
        "{argv}"
    );
    assert!(
        argv.contains("--include-partial-messages"),
        "streams: {argv}"
    );
    assert_eq!(r.w.chat_held_starts(), 1);
}

/// No board here: every ref names itself.
fn as_typed(r: &str) -> Option<String> {
    Some(r.to_string())
}

#[test]
fn grants_begin_and_end_with_each_message() {
    let f = tempfile::tempdir().unwrap();
    let r = rig(f.path());
    std::fs::write(f.path().join("block"), "").unwrap();
    let g = r.core.assistant_grants();
    let t1 = r.core.assistant_chat_ask("close #5", "", "").unwrap();
    wait_for("the first message", 20, || messages(f.path()).len() == 1);
    // While the first question runs, its grant is open and nothing else.
    assert!(g.take(CLOSE, "#9", "", &as_typed).is_none());
    assert!(g.take(CLOSE, "#5", "", &as_typed).is_some());
    std::fs::write(f.path().join("go-1"), "").unwrap();
    replied(&r, &t1);
    let t2 = r.core.assistant_chat_ask("close #9", "", "").unwrap();
    wait_for("the second message", 20, || messages(f.path()).len() == 2);
    assert!(
        g.take(CLOSE, "#5", "", &as_typed).is_none(),
        "the first message's grant ended"
    );
    assert!(g.take(CLOSE, "#9", "", &as_typed).is_some());
    // Spent once: a second close of #9 has no grant.
    assert!(g.take(CLOSE, "#9", "", &as_typed).is_none());
    std::fs::write(f.path().join("go-2"), "").unwrap();
    replied(&r, &t2);
    // The second message's grants end with it.
    let t3 = r.core.assistant_chat_ask("thanks", "", "").unwrap();
    wait_for("the third message", 20, || messages(f.path()).len() == 3);
    assert!(g.take(CLOSE, "#9", "", &as_typed).is_none());
    std::fs::write(f.path().join("go-3"), "").unwrap();
    replied(&r, &t3);
    assert_eq!(starts(f.path()), 1, "all three on one process");
}

#[test]
fn new_chat_seeds_a_new_session_with_nothing_from_before() {
    let f = tempfile::tempdir().unwrap();
    let r = rig(f.path());
    ask(&r, "remember the word pelican");
    ask(&r, "and the word otter");
    r.core
        .assistant_chat_write(ChatRow::Cleared(Cleared {
            by: format!("user:{}", r.core.device_id()),
            at: unix_ms(),
        }))
        .unwrap();
    ask(&r, "what words did I say?");
    assert_eq!(starts(f.path()), 2, "New chat starts a new process");
    let m = messages(f.path());
    assert_eq!(m.len(), 3, "{m:?}");
    // The new session's seed holds no turn from before New chat.
    assert_eq!(m[2], "The person: what words did I say?\n");
    assert!(!m[2].contains("pelican") && !m[2].contains("otter"));
}

#[test]
fn what_it_hasnt_seen_comes_as_since_we_last_spoke() {
    let f = tempfile::tempdir().unwrap();
    let r = rig(f.path());
    ask(&r, "post the lobby fix");
    r.core
        .assistant_chat_write(ChatRow::Update(Update {
            key: "t1|ready:#12:b1".into(),
            thread: "t1".into(),
            text: "#12 is ready for your check.".into(),
            at: unix_ms(),
            ..Update::default()
        }))
        .unwrap();
    ask(&r, "how is it going?");
    ask(&r, "anything else?");
    assert_eq!(starts(f.path()), 1);
    let m = messages(f.path());
    assert_eq!(
        m[1],
        format!("{SINCE_HEAD}\n- #12 is ready for your check.\n\nThe person: how is it going?\n")
    );
    // Told once: the next message has no since line.
    assert_eq!(m[2], "The person: anything else?\n");
}

#[test]
fn a_new_session_starts_on_a_change_of_model_past_60k_and_after_a_stop() {
    let f = tempfile::tempdir().unwrap();
    let r = rig(f.path());
    ask(&r, "one");
    // The chat's model changes in Settings: a new process on the new argv.
    let mut s = r.core.spawn_setting("");
    s.assistant_model = "haiku".into();
    r.core.set_spawn_setting(s).unwrap();
    ask(&r, "two");
    assert_eq!(starts(f.path()), 2, "a change of model starts again");
    let argv = std::fs::read_to_string(f.path().join("argv.txt")).unwrap();
    assert!(argv.contains("--model haiku"), "{argv}");
    // The new session is seeded with the turn before.
    let m = messages(f.path());
    assert!(
        m[1].contains("The conversation so far") && m[1].contains("The person: one"),
        "{}",
        m[1]
    );

    // Past 60k: that turn is answered, and the next one seeds a new process.
    std::fs::write(f.path().join("context"), "61000").unwrap();
    ask(&r, "three");
    std::fs::write(f.path().join("context"), "1000").unwrap();
    ask(&r, "four");
    assert_eq!(starts(f.path()), 3, "past 60k starts again");
    let m = messages(f.path());
    assert!(
        m[3].contains("The person: three") && m[3].ends_with("The person: four\n"),
        "{}",
        m[3]
    );

    // A stopped turn ends its process; the next question seeds a new one.
    std::fs::write(f.path().join("block"), "").unwrap();
    let t = r.core.assistant_chat_ask("five", "", "").unwrap();
    wait_for("the fifth message", 20, || messages(f.path()).len() == 5);
    r.core.assistant_chat_stop(&t).unwrap();
    assert_eq!(replied(&r, &t), "error: Stopped");
    std::fs::remove_file(f.path().join("block")).unwrap();
    ask(&r, "six");
    assert_eq!(starts(f.path()), 4, "a stop starts again");
    assert_eq!(r.w.chat_held_starts(), 4);
}

#[test]
fn quitting_ends_the_session_and_writes_no_reply() {
    let f = tempfile::tempdir().unwrap();
    let r = rig(f.path());
    ask(&r, "one");
    std::fs::write(f.path().join("block"), "").unwrap();
    let t = r.core.assistant_chat_ask("two", "", "").unwrap();
    wait_for("the second message", 20, || messages(f.path()).len() == 2);
    // The quit path: the worker takes no new question, then the runs end.
    r.core.set_chat_turn(None);
    r.w.assistant_shutdown();
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        r.core.assistant_chat_turn(&t).unwrap().said.is_none(),
        "the next start answers it"
    );
    let pid = std::fs::read_to_string(f.path().join("pid")).unwrap();
    wait_for("the held process to end", 10, || {
        !std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    });
}

#[test]
fn another_computer_taking_over_ends_the_held_process() {
    let f = tempfile::tempdir().unwrap();
    let r = rig(f.path());
    ask(&r, "one");
    assert!(r.w.chat_held_starts() == 1);
    let pid = std::fs::read_to_string(f.path().join("pid")).unwrap();
    r.w.chat_held_end();
    wait_for("the held process to end", 10, || {
        !std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    });
    ask(&r, "two");
    assert_eq!(starts(f.path()), 2);
    let m = messages(f.path());
    assert!(m[1].contains("The person: one"), "seeded again: {}", m[1]);
}

/// The done-when's two devices: ten questions, asked in turn on two paired
/// Cores, all answered by the assistant's computer from one process.
#[test]
fn ten_questions_from_two_devices_in_one_process() {
    use crate::mesh::{dev, lans, pair, wait};
    let f = tempfile::tempdir().unwrap();
    let l = lans(2);
    let a = dev(l[0].0, l[0].1.clone(), false);
    let b = dev(l[1].0, l[1].1.clone(), false);
    pair(&a, &b);
    a.core().set_assistant_home(a.core().device_id()).unwrap();
    let bin = fake_held(f.path());
    install(&a.w, &bin);
    let home = a.core().device_id().to_string();
    wait("both know the assistant's computer", || {
        b.core().assistant_home_status().device == home
    });
    let mut turns = Vec::new();
    for i in 0..10 {
        let d = if i % 2 == 0 { &a } else { &b };
        let t = d
            .core()
            .assistant_chat_ask(&format!("question {i}"), "", "")
            .unwrap();
        // Each is answered before the next is asked, as the person would.
        wait(&format!("question {i} answered on both"), || {
            [&a, &b].iter().all(|x| {
                x.core()
                    .assistant_chat_turn(&t)
                    .is_some_and(|t| t.said.is_some_and(|s| s.error.is_empty()))
            })
        });
        turns.push(t);
    }
    assert_eq!(starts(f.path()), 1, "one process answered all ten");
    assert_eq!(a.w.chat_held_starts(), 1);
    let m = messages(f.path());
    assert_eq!(m.len(), 10, "{m:?}");
    for (i, msg) in m.iter().enumerate().skip(1) {
        assert_eq!(*msg, format!("The person: question {i}\n"));
    }
}
