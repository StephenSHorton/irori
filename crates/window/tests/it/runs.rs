//! Live tasks on cards (card #1249, live-tasks.md §3, §5, §6): a run joins
//! the cards its agent holds on read, on the private board and on a shared
//! one, without writing an `agent.card`; Stop marks the run and its tail
//! Stopped.

use crate::common;

use common::*;
use irori_core::runs::{RunId, RunReport};
use irori_window::cards::{RunKind, SCOPE_ALL};
use store::kinds;

fn start(r: &Rig, key: &str, task: &str, cmd: &str, output: Option<&std::path::Path>) {
    r.w.core
        .record_run(
            key,
            &RunReport {
                event: "start".into(),
                kind: "bash".into(),
                task_id: Some(task.into()),
                command: Some(cmd.into()),
                output_file: output.map(|p| p.to_string_lossy().into_owned()),
                ..RunReport::default()
            },
        )
        .unwrap();
}

fn agent_cards(r: &Rig, board: &str) -> usize {
    r.w.core
        .board_store(board)
        .unwrap()
        .list(&store::Filter {
            limit: store::MAX_LIST_LIMIT,
            ..store::Filter::default()
        })
        .unwrap()
        .iter()
        .filter(|e| e.kind == kinds::AGENT_CARD)
        .count()
}

#[test]
fn a_run_shows_on_private_and_shared_cards_with_no_agent_card_written() {
    let r = Rig::new();
    let w = &r.w;
    // A private card beta holds.
    let co = w.core.coord();
    let mine = co
        .create_task(
            &r.key_b,
            coord::NewTask {
                title: "private work".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_b, &mine.id).unwrap();
    // A shared board's card alpha holds (the setup makes its board key).
    let b = w.boards_create("Horton", "horton").unwrap();
    let bco = w.core.board_coord(&b.board_id).unwrap();
    let me = w
        .core
        .board_key(&b.board_id, &w.core.user_sender().key)
        .unwrap();
    let shared = bco
        .create_task(
            &me,
            coord::NewTask {
                title: "shared work".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let alpha = w.core.board_key(&b.board_id, &r.key_a).unwrap();
    bco.claim_task(&alpha, &shared.id).unwrap();
    let before = agent_cards(&r, &b.board_id);

    start(&r, &r.key_a, "bk1", "cargo test", None);
    start(&r, &r.key_b, "bk2", "sleep 30", None);

    for (id, key, run_id, cmd) in [
        (&mine.id, &r.key_b, "bk2", "sleep 30"),
        (&shared.id, &r.key_a, "bk1", "cargo test"),
    ] {
        let d = w.card(id).unwrap();
        assert_eq!(d.summary.runs.len(), 1, "{id}: {:?}", d.summary.runs);
        let run = &d.summary.runs[0];
        assert_eq!(run.run, run_id);
        assert_eq!(run.cmd, cmd);
        assert_eq!(run.kind, RunKind::Bash);
        assert_eq!(&run.who.key, key);
        assert!(run.ended.is_none() && run.exit.is_none() && !run.stopped);
        // Not sampled yet: no reading.
        assert!(run.cpu_cores.is_none());
    }
    // A light run is on no summary (chips are for heavy runs).
    let cards = w.cards_in(SCOPE_ALL);
    for id in [&mine.id, &shared.id] {
        let s = cards.iter().find(|c| &c.id == id).expect("listed");
        assert!(s.runs.is_empty(), "{:?}", s.runs);
    }
    let ser = serde_json::to_value(cards.iter().find(|c| c.id == mine.id).unwrap()).unwrap();
    assert!(ser.get("runs").is_none(), "no runs, no field");

    // An end shows on the page with its code.
    w.core
        .record_run(
            &r.key_a,
            &RunReport {
                event: "end".into(),
                task_id: Some("bk1".into()),
                status: Some("failed".into()),
                exit_code: Some(101),
                ..RunReport::default()
            },
        )
        .unwrap();
    let d = w.card(&shared.id).unwrap();
    assert_eq!(d.summary.runs[0].exit, Some(101));
    assert!(d.summary.runs[0].ended.is_some());

    assert_eq!(
        agent_cards(&r, &b.board_id),
        before,
        "the join wrote no agent.card"
    );
}

#[test]
fn stop_marks_the_run_and_its_tail_stopped() {
    let r = Rig::new();
    let w = &r.w;
    let co = w.core.coord();
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "build".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let tasks = tmp
        .path()
        .join("claude")
        .join("proj")
        .join("sess")
        .join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    let out = tasks.join("bstop1.output");
    std::fs::write(&out, "compiling\n").unwrap();
    // A file outside claude's tasks folders never gets a tail.
    let stray = tmp.path().join("bstray1.output");
    std::fs::write(&stray, "x\n").unwrap();

    start(&r, &r.key_a, "bstop1", "cargo build", Some(&out));
    start(&r, &r.key_a, "bstray1", "cargo doc", Some(&stray));
    wait("the run's tail opens on its card", || {
        w.term_list(&t.id).iter().any(|i| i.tail.is_some())
    });
    let tails: Vec<_> = w
        .term_list(&t.id)
        .into_iter()
        .filter_map(|i| i.tail)
        .collect();
    assert_eq!(tails.len(), 1, "{tails:?}");
    assert_eq!(tails[0].run, "bstop1");
    assert_eq!(tails[0].label, "cargo build");
    assert!(!tails[0].stopped);
    // A running tail is no shell in use: it never holds an update back.
    assert_eq!(w.core.open_shells(), 0);
    let shell = w
        .term_open(
            term::Scope::Card(t.id.clone()),
            80,
            24,
            Some(tmp.path().to_string_lossy().into_owned()),
        )
        .unwrap();
    assert_eq!(w.core.open_shells(), 1);
    w.core.terms().close(shell.id).unwrap();
    assert_eq!(w.core.open_shells(), 0);

    w.runs_stop(&r.key_a, "bstop1").unwrap();
    let row = w.core.runs_of(&r.key_a);
    let row = row
        .iter()
        .find(|x| x.id() == RunId::new(&r.key_a, "bstop1"))
        .unwrap();
    assert!(row.stopped_by_person);
    let tail = w.term_list(&t.id).into_iter().find_map(|i| i.tail).unwrap();
    assert!(tail.stopped, "the tail reads Stopped");
    let d = w.card(&t.id).unwrap();
    let run = d.summary.runs.iter().find(|x| x.run == "bstop1").unwrap();
    assert!(run.stopped);

    // An unknown run, or one on another device, is refused.
    assert!(w.runs_stop(&r.key_a, "nope").is_err());
    w.core
        .record_run(
            &r.key_a,
            &RunReport {
                event: "end".into(),
                task_id: Some("bstray1".into()),
                exit_code: Some(0),
                ..RunReport::default()
            },
        )
        .unwrap();
    assert!(
        w.runs_stop(&r.key_a, "bstray1").is_err(),
        "an ended run is not stopped"
    );
    for i in w.term_list(&t.id) {
        let _ = w.core.terms().close(i.id);
    }
    // The closed shell leaves its folder (#960) before the TempDir goes.
    assert!(w.core.terms().settle(std::time::Duration::from_secs(20)));
    wait("the shell to leave its folder", || {
        std::fs::remove_dir_all(tmp.path()).is_ok()
    });
}
