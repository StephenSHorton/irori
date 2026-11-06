//! wait / waits / wait_cancel (card #87).

use crate::common;

use common::*;
use inbox::mode::WAKE;
use tools::Backend;

#[test]
fn test_wait_on_a_card() {
    let h = new_harness();
    let (a, b) = (call_a(), call_b());
    let id = h.new_card(&a, json!({"title": "Ship the lobby"}));
    h.okj(&a, "board_join", json!({"task_id": id}));
    let r = h.short(&id);

    h.failsj(&b, "wait", json!({}), &["task_id", "not both"]);
    h.failsj(
        &b,
        "wait",
        json!({"task_id": id, "until": ["soon"]}),
        &["until"],
    );
    h.failsj(&a, "wait", json!({"task_id": id}), &["your own card"]);
    let out = h.okj(
        &b,
        "wait",
        json!({"task_id": id, "until": ["done"], "note": "I build on its API"}),
    );
    has(
        &out,
        &[&format!("woken once when {r} is done"), "it is working now"],
    );
    has(
        &h.okj(&b, "waits", json!({})),
        &[&format!("{r} until done (I build on its API)")],
    );
    // It shows on the card waited on, and on the waiter's own card.
    let mine = h.new_card(&b, json!({"title": "Use the lobby API"}));
    h.okj(&b, "board_join", json!({"task_id": mine}));
    let board = h.okj(&a, "board", json!({"repo": "*"}));
    has(
        &board,
        &[
            "waited on by beta",
            "(until done)",
            &format!("waiting on {r} until done"),
        ],
    );
    let sh = search::Source::search_show(
        h.b.co.as_ref(),
        &format!("task:{id}"),
        &search::ShowOptions::default(),
    )
    .unwrap();
    has(&sh.text, &[&format!("waited on by: {KEY_B} until done")]);

    h.okj(
        &a,
        "board_status",
        json!({"task_id": id, "state": "completed", "note": "merged"}),
    );
    // The core routes what the sweep returns; route it the same way.
    let ns: Vec<_> =
        h.b.co
            .sweep_notices()
            .unwrap()
            .into_iter()
            .filter(|n| n.kind == coord::NOTICE_WAIT)
            .collect();
    assert_eq!(ns.len(), 1, "{ns:?}");
    tools::notice::route_notice(
        h.b.inbox(),
        &inbox::Sender::new("irori", "Irori", "irori"),
        &ns[0],
    );
    let got: Vec<_> = h
        .notices(KEY_B)
        .into_iter()
        .filter(|d| d.message.reason == "wait")
        .collect();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].mode, WAKE);
    has(
        &got[0].message.text,
        &["Your wait went off", "is now completed", "merged"],
    );
    has(&h.okj(&b, "waits", json!({})), &["no open waits"]);
}

#[test]
fn test_wait_cancel() {
    let h = new_harness();
    let (a, b) = (call_a(), call_b());
    let id = h.new_card(&a, json!({"title": "x"}));
    let out = h.okj(&b, "wait", json!({"task_id": id}));
    let wid = out
        .split_once("Waiting (")
        .and_then(|(_, r)| r.split_once(')'))
        .map(|(w, _)| w.to_string())
        .unwrap();
    h.failsj(&b, "wait_cancel", json!({"id": "nope"}), &["no open wait"]);
    h.failsj(&a, "wait_cancel", json!({"id": wid}), &["someone else's"]);
    has(
        &h.okj(&b, "wait_cancel", json!({"id": wid})),
        &["Canceled your wait"],
    );
    has(&h.okj(&b, "waits", json!({})), &["no open waits"]);
}

#[test]
fn test_wait_on_a_branch() {
    let h = new_harness();
    let b = call_b();
    let d = tempfile::tempdir().unwrap();
    let root = canon(d.path());
    git(&root, &["init", "-q"]);
    write_file(&join(&root, "a.txt"), "one");
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "first"]);

    h.failsj(
        &b,
        "wait",
        json!({"branch": "main", "repo": "remote:nowhere/x"}),
        &["no checkout"],
    );
    let out = h.okj(&b, "wait", json!({"branch": "main", "repo": root}));
    has(
        &out,
        &[
            "woken once when main moves",
            "(main at",
            "origin/main doesn't exist yet",
        ],
    );
    let ws = h.b.co.branch_waits();
    assert_eq!(ws.len(), 1);
    assert_eq!(ws[0].refs.len(), 2);
    has(&h.okj(&b, "waits", json!({})), &["branch main"]);
}

/// Card #574: an agent that asks for an agent hears why none has started,
/// and `board` and `show` say the same.
#[test]
fn why_no_agent_shows_after_asking_and_on_the_card() {
    let h = new_harness();
    let a = call_a();
    let id = h.new_card(&a, json!({"title": "Needs hands"}));
    *h.b.spawn_why.lock().unwrap() = "Mac is at its cap (2/2); it starts when one ends".into();
    const LINE: &str = "no agent yet: Mac is at its cap (2/2)";

    h.b.spawn_asked.lock().unwrap().clear();
    let out = h.okj(
        &a,
        "board_steps",
        json!({"task_id": id, "steps": [{"text": "do it", "spawn": true}]}),
    );
    has(&out, &[LINE, "(spawn)"]);
    assert_eq!(
        *h.b.spawn_asked.lock().unwrap(),
        vec![true],
        "asked is said"
    );

    has(&h.okj(&a, "board", json!({"repo": "*"})), &[LINE]);

    h.b.spawn_asked.lock().unwrap().clear();
    let plain = h.okj(
        &a,
        "board_steps",
        json!({"task_id": id, "steps": [{"text": "no mark"}]}),
    );
    assert!(!plain.contains(LINE), "{plain}");

    let out = h.okj(
        &a,
        "board_split",
        json!({"task_id": id, "subtasks": [{"title": "part", "spawn": true, "steps": ["s2"]}]}),
    );
    has(&out, &["(spawn)", LINE]);
}

#[test]
fn a_card_with_no_reason_shows_no_line() {
    let h = new_harness();
    let a = call_a();
    h.new_card(&a, json!({"title": "Quiet"}));
    let board = h.okj(&a, "board", json!({"repo": "*"}));
    assert!(!board.contains("no agent yet"), "{board}");
}

#[test]
fn an_assigned_part_gets_no_line_and_an_empty_why_adds_no_blank_line() {
    let h = new_harness();
    let (a, b) = (call_a(), call_b());
    let id = h.new_card(&a, json!({"title": "Parts"}));
    h.okj(&a, "board_join", json!({"task_id": id}));
    let out = h.okj(
        &a,
        "board_split",
        json!({"task_id": id, "subtasks": [{"title": "part", "spawn": true}]}),
    );
    assert!(!out.contains("\n\n"), "{out}");

    let id = h.new_card(&b, json!({"title": "More parts"}));
    h.okj(&b, "board_join", json!({"task_id": id}));
    *h.b.spawn_why.lock().unwrap() = "Irori starts one at its next pass".into();
    let out = h.okj(
        &b,
        "board_split",
        json!({"task_id": id, "subtasks": [{"title": "part", "spawn": true, "assign_to": "gamma"}]}),
    );
    let made = out.lines().find(|l| l.starts_with("Created")).unwrap();
    assert!(made.contains("Assigned to"), "{out}");
    assert!(!out.contains("next pass Assigned"), "{out}");
}
