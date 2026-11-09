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
