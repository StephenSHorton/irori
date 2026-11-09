//! Live tasks on cards (card #1249): the Core keeps the mod's run reports
//! across a restart, and a report that changes a row is a Runs event.

use std::time::Duration;

use irori_core::runs::{RunKind, RunReport};
use irori_core::{Core, CoreEvent, Topic};

use crate::common;

const KEY: &str = "dev-1/sess-1";

fn rep(json: serde_json::Value) -> RunReport {
    serde_json::from_value(json).unwrap()
}

#[test]
fn runs_outlive_a_restart() {
    let (d, c) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let core = Core::start(common::options(d.path(), c.path())).unwrap();
    assert!(core.runs().is_empty());
    let feed = core.feed(&[Topic::Runs]);
    let start = serde_json::json!({
        "event": "start", "kind": "bash", "taskId": "b1", "command": "cargo test",
    });
    core.record_run(KEY, &rep(start)).unwrap();
    assert_eq!(
        feed.queue().recv_timeout(Duration::from_secs(5)),
        Ok(Some(CoreEvent::Runs {
            keys: vec![KEY.to_string()]
        }))
    );
    let wf = serde_json::json!({
        "event": "start", "kind": "workflow", "taskId": "w1", "workflowName": "build",
    });
    core.record_run(KEY, &rep(wf)).unwrap();
    let end = serde_json::json!({"event": "end", "taskId": "b1", "exitCode": 0});
    core.record_run(KEY, &rep(end)).unwrap();
    // A refused report is no event and changes nothing.
    let before = core.runs_of(KEY);
    let bad = serde_json::json!({"event": "start", "kind": "nope", "taskId": "x"});
    assert!(core.record_run(KEY, &rep(bad)).is_err());
    assert_eq!(core.runs_of(KEY), before);
    while feed.try_recv().is_some() {}
    assert_eq!(
        feed.queue().recv_timeout(Duration::from_millis(200)),
        Err(())
    );
    // The Stop stub refuses until the sampler slice lands.
    assert!(core.stop_run(KEY, "w1").is_err());
    drop(feed);
    core.close().unwrap();

    let core = Core::start(common::options(d.path(), c.path())).unwrap();
    let got = core.runs_of(KEY);
    assert_eq!(got, before);
    let rows: Vec<_> = got
        .iter()
        .map(|r| (r.task_id.as_str(), r.kind, r.running(), r.exit))
        .collect();
    assert_eq!(
        rows,
        [
            ("b1", RunKind::Bash, false, Some(0)),
            ("w1", RunKind::Workflow, true, None)
        ]
    );
    core.close().unwrap();
}

/// Two devices (design §7): a background run on the Mac shows on the PC
/// under the Mac agent's key, named for the Mac, and ends with its code; a
/// foreground call never leaves the Mac; a row the Mac writes for an agent
/// on another device is ignored; each run's start and end are written once.
#[test]
fn a_run_on_another_device_shows_here_and_ends_with_its_code() {
    use crate::common::mesh::person;
    use crate::common::wait_for;
    use store::{Filter, kinds};

    let mut devs = person("stephen", 2);
    let (pc, mac) = (devs.pop().unwrap(), devs.pop().unwrap());
    let key = mac.agent.clone();
    let feed = pc.core.feed(&[Topic::Runs]);
    mac.core
        .record_run(
            &key,
            &rep(serde_json::json!({
                "event": "start", "kind": "bash", "taskId": "b1",
                "command": "cargo test", "outputFile": "/private/tmp/x/tasks/b1.output",
            })),
        )
        .unwrap();
    mac.core
        .record_run(
            &key,
            &rep(serde_json::json!({"event": "fg-start", "toolUseId": "toolu_1", "command": "ls"})),
        )
        .unwrap();
    wait_for(60, "the PC folds the Mac's start", || {
        pc.core
            .runs_of(&key)
            .iter()
            .any(|r| r.task_id == "b1" && r.running())
    });
    let r = pc.core.runs_of(&key);
    assert_eq!(r.len(), 1, "no foreground row crosses: {r:?}");
    assert_eq!(
        (r[0].cmd.as_str(), r[0].kind),
        ("cargo test", RunKind::Bash)
    );
    assert!(r[0].output_path.is_none(), "the path stays on the Mac");
    let mac_name = pc.core.device_label(mac.core.device_id());
    assert!(!mac_name.is_empty());
    assert_eq!(pc.core.run_device(&key), Some(mac_name));
    assert_eq!(pc.core.run_device(&pc.agent), None);
    assert_eq!(mac.core.run_device(&key), None);
    let mut heard = false;
    while let Ok(Some(ev)) = feed.recv_timeout(Duration::from_millis(10)) {
        heard |= ev
            == CoreEvent::Runs {
                keys: vec![key.clone()],
            };
    }
    assert!(heard, "the fold publishes the Mac agent's key");

    // The Mac writes a row for a run on the PC's own device: refused.
    let forged = serde_json::json!({
        "key": pc.agent, "run": "bx", "kind": "bash", "op": "start",
        "cmd": "forged", "startedAt": 1, "at": 1,
    });
    mac.core
        .store()
        .append("irori", kinds::AGENT_RUN, &forged)
        .unwrap();

    mac.core
        .record_run(
            &key,
            &rep(serde_json::json!({"event": "end", "taskId": "b1", "status": "failed", "exitCode": 101})),
        )
        .unwrap();
    wait_for(60, "the PC folds the Mac's end", || {
        pc.core
            .runs_of(&key)
            .iter()
            .any(|r| r.task_id == "b1" && !r.running() && r.exit == Some(101))
    });
    let rows = |c: &Core| {
        c.store()
            .list(&Filter {
                kinds: vec![kinds::AGENT_RUN.into()],
                ..Filter::default()
            })
            .unwrap()
    };
    wait_for(60, "the PC holds the forged row", || {
        rows(&pc.core).len() == 3
    });
    assert!(
        pc.core.runs_of(&pc.agent).is_empty(),
        "the forged row is ignored"
    );
    // Once each: a repeat report writes nothing more.
    mac.core
        .record_run(
            &key,
            &rep(serde_json::json!({"event": "end", "taskId": "b1", "exitCode": 101})),
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let mine: Vec<_> = rows(&mac.core)
        .into_iter()
        .filter(|e| e.device_id == mac.core.device_id())
        .map(|e| serde_json::from_slice::<serde_json::Value>(&e.payload).unwrap()["op"].clone())
        .collect();
    assert_eq!(
        mine,
        ["start", "start", "end"].map(serde_json::Value::from),
        "{mine:?}"
    );
    pc.core.close().unwrap();
    mac.core.close().unwrap();
}

/// A restart reads back what this device wrote: an open run's start is
/// not written again.
#[test]
fn a_restart_writes_no_second_start() {
    use store::{Filter, kinds};
    let (d, c) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let core = Core::start(common::options(d.path(), c.path())).unwrap();
    let key = format!("{}/sess-1", core.device_id());
    core.record_run(
        &key,
        &rep(serde_json::json!({"event": "start", "kind": "bash", "taskId": "b1", "command": "sleep 99"})),
    )
    .unwrap();
    let count = |c: &Core| {
        c.store()
            .list(&Filter {
                kinds: vec![kinds::AGENT_RUN.into()],
                ..Filter::default()
            })
            .unwrap()
            .len()
    };
    common::wait_for(60, "the start row", || count(&core) == 1);
    core.close().unwrap();
    let core = Core::start(common::options(d.path(), c.path())).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(count(&core), 1);
    core.record_run(
        &key,
        &rep(serde_json::json!({"event": "end", "taskId": "b1", "exitCode": 0})),
    )
    .unwrap();
    common::wait_for(60, "the end row", || count(&core) == 2);
    core.close().unwrap();
}
