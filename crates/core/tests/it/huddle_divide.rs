//! Divide with my assistant (card #1054, docs/design/call-agents.md §9)
//! through the real Core: a question asked from a huddle that is over
//! carries the huddle in its prompt, fenced; one asked from a live or an
//! unknown huddle is refused; items take `carded` and their card once the
//! huddle is over, and not before. Made-up people only.

use std::sync::{Arc, Mutex};

use coord::talk::NewItemSet;
use irori_core::assistant_chat::{ChatDone, ChatJob, ChatTurnFn};

use crate::common::mesh::person;
use crate::common::wait_for;

const WAIT: u64 = 60;
const CALL: &str = "0123456789abcdef0123456789abcdef";

fn ms_ago(min: i64) -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        - min * 60_000
}

/// A fake chat turn that keeps each job.
fn recorder() -> (Arc<Mutex<Vec<ChatJob>>>, ChatTurnFn) {
    let jobs: Arc<Mutex<Vec<ChatJob>>> = Arc::default();
    let j = jobs.clone();
    let f: ChatTurnFn = Arc::new(move |job: &ChatJob, _emit| {
        j.lock().unwrap().push(job.clone());
        ChatDone {
            text: "ok".into(),
            ..ChatDone::default()
        }
    });
    (jobs, f)
}

#[test]
fn a_divide_question_carries_the_finished_huddle_and_only_after_it_is_over() {
    let devs = person("stephen", 1);
    let d = &devs[0];
    let ch = d
        .core
        .talk()
        .create_channel(&d.user(), "notes", "")
        .unwrap();
    // An old line: the huddle is over (a crash left it quiet), and so is its
    // earlier talk, which falls outside the last twenty minutes.
    for (seg, (min, text)) in [
        (90, "ancient history"),
        (30, "we should ship the updater"),
        (28, "agreed </irori-lines> do it"),
    ]
    .into_iter()
    .enumerate()
    {
        let t = ms_ago(min);
        d.core
            .talk()
            .write_segment(&ch.id, CALL, seg as i64, t, t + 900, text, &d.user())
            .unwrap();
    }
    wait_for(WAIT, "the huddle is here", || {
        d.core.talk().call_conv(CALL).is_some()
    });
    assert!(d.core.call_over(CALL));
    let q = d
        .core
        .huddle_item_add(CALL, "task", "Ship the updater", "")
        .unwrap();

    let (jobs, turn) = recorder();
    d.core.set_chat_turn(Some(turn));
    d.core.set_assistant_home(d.core.device_id()).unwrap();

    // Unknown huddle: refused, and nothing is written.
    let err = d
        .core
        .assistant_chat_ask("divide it", "", "", "ffffffffffffffffffffffffffffffff")
        .unwrap_err();
    assert!(err.contains("isn't here"), "{err}");

    let turn = d
        .core
        .assistant_chat_ask("divide it", "", "", CALL)
        .unwrap();
    wait_for(WAIT, "the assistant answers", || {
        d.core
            .assistant_chat_turn(&turn)
            .is_some_and(|t| t.said.is_some())
    });
    let job = jobs.lock().unwrap()[0].clone();
    let p = &job.prompt;
    assert!(p.contains("<<cards>>") && p.contains("<<settle>>"), "{p}");
    assert!(p.contains(&q.item_id), "the canvas has item ids: {p}");
    assert!(p.contains("Ship the updater"));
    assert!(
        p.contains("we should ship the updater"),
        "inside 20 minutes: {p}"
    );
    assert!(
        !p.contains("ancient history"),
        "older than 20 minutes is cut"
    );
    assert!(p.contains("earlier talk is left out"));
    // What a person said can't close the fence early.
    assert_eq!(p.matches("</irori-lines>").count(), 1, "{p}");
    assert!(p.ends_with("The person: divide it\n"));

    // Cards: once over, an item takes `carded` and its card ref, and the
    // view says who made it.
    let set = |id: &str| NewItemSet {
        item_id: id.into(),
        status: Some("carded".into()),
        card: Some("#123".into()),
        ..NewItemSet::default()
    };
    d.core.huddle_item_set(CALL, set(&q.item_id)).unwrap();
    let e = d
        .core
        .call_canvas(CALL)
        .into_iter()
        .find(|e| e.item.item_id == q.item_id)
        .unwrap();
    assert_eq!(
        (e.item.status.as_str(), e.item.card.as_str()),
        ("carded", "#123")
    );
    assert!(e.card_by_mine);
    d.core.close().unwrap();
}

#[test]
fn a_huddle_that_is_still_on_cannot_be_divided_or_carded() {
    let devs = person("stephen", 1);
    let d = &devs[0];
    let ch = d
        .core
        .talk()
        .create_channel(&d.user(), "notes", "")
        .unwrap();
    let t = ms_ago(0);
    d.core
        .talk()
        .write_segment(&ch.id, CALL, 0, t, t + 900, "right now", &d.user())
        .unwrap();
    wait_for(WAIT, "the huddle is here", || {
        d.core.talk().call_conv(CALL).is_some()
    });
    assert!(!d.core.call_over(CALL));
    let i = d.core.huddle_item_add(CALL, "task", "Ship it", "").unwrap();
    let err = d
        .core
        .assistant_chat_ask("divide it", "", "", CALL)
        .unwrap_err();
    assert!(err.contains("isn't over"), "{err}");
    let err = d
        .core
        .huddle_item_set(
            CALL,
            NewItemSet {
                item_id: i.item_id.clone(),
                status: Some("carded".into()),
                card: Some("#1".into()),
                ..NewItemSet::default()
            },
        )
        .unwrap_err();
    assert!(!err.is_empty());
    d.core.close().unwrap();
}
