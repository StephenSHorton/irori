//! I:internal/core/search_test.go: the Core registers coord and the inbox
//! as capture's search sources, and keeps messages to their agents.

mod common;

use common::*;
use coord::NewTask;
use inbox::{Address, Sender, addr};
use search::{Query, ShowOptions};

fn refs(r: &Rig, viewer: &str) -> Vec<String> {
    r.core
        .capture()
        .search(&Query {
            text: "pangolin".into(),
            viewer: viewer.into(),
            ..Query::default()
        })
        .unwrap()
        .into_iter()
        .map(|h| h.r#ref)
        .collect()
}

/// A Board card and a message are found and shown through capture.
#[test]
fn test_search_reaches_board_and_inbox() {
    let r = Rig::new();
    let task = r
        .core
        .coord()
        .create_task(
            &r.key_a,
            NewTask {
                title: "Feed the pangolin".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    let (m, _) = r
        .core
        .inbox()
        .send(
            &Sender::new(&r.key_a, "alpha", "claude"),
            &Address::new(addr::AGENT, "beta"),
            "the pangolin is hungry",
            "",
        )
        .unwrap();
    let got = refs(&r, "");
    for want in [format!("task:{}", task.id), format!("msg:{}", m.id)] {
        assert!(got.contains(&want), "hits {got:?} lack {want}");
        r.core
            .capture()
            .show(&want, &ShowOptions::default())
            .unwrap_or_else(|e| panic!("show {want}: {e}"));
    }
}

/// A message between two agents is found and shown only by them and the
/// person; a third agent's search and show never reach it.
#[test]
fn test_search_keeps_messages_to_their_agents() {
    let r = Rig::new();
    let (m, _) = r
        .core
        .inbox()
        .send(
            &Sender::new(&r.key_a, "alpha", "claude"),
            &Address::new(addr::AGENT, "beta"),
            "secret pangolin plan",
            "",
        )
        .unwrap();
    let rf = format!("msg:{}", m.id);
    let outsider = format!(
        "{}/gggggggg-7777-4777-8777-777777777777",
        r.core.device_id()
    );
    for (viewer, found) in [
        ("", true),
        (r.key_a.as_str(), true),
        (r.key_b.as_str(), true),
        (outsider.as_str(), false),
    ] {
        let got = refs(&r, viewer);
        assert_eq!(got.contains(&rf), found, "viewer {viewer:?}: hits {got:?}");
        let shown = r.core.capture().show(
            &rf,
            &ShowOptions {
                viewer: viewer.into(),
                ..ShowOptions::default()
            },
        );
        match shown {
            Ok(_) => assert!(found, "viewer {viewer:?} was shown it"),
            Err(e) => assert!(!found && e.is_not_found(), "viewer {viewer:?} show: {e}"),
        }
    }
}
