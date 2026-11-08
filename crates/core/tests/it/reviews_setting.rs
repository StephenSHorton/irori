//! "Agents review other people's PRs without asking" (card #1072, merge-desk
//! §12.8): on when unset, the person's alone to set, and the inbox follows
//! it.

use crate::common;

use std::time::Duration;

use common::*;
use store::kinds;

/// A load of the `spawn.cap` rows, and time for it to run.
fn sweep(r: &Rig) {
    let cap = r.core.spawn_cap();
    r.core.set_spawn_cap(cap).unwrap();
    std::thread::sleep(Duration::from_millis(400));
}

#[test]
fn on_when_there_is_no_row_and_the_inbox_agrees() {
    let r = Rig::new();
    assert!(r.core.reviews_unasked());
    assert!(r.core.inbox().reviews_unasked());
}

#[test]
fn the_inbox_reads_the_setting_after_a_toggle_and_a_reload() {
    let r = Rig::new();
    r.core.set_reviews_unasked(false).unwrap();
    assert!(!r.core.reviews_unasked());
    assert!(!r.core.inbox().reviews_unasked());
    // A load from the store (what another device's row, or a restart,
    // goes through) keeps it.
    sweep(&r);
    assert!(!r.core.reviews_unasked());
    assert!(!r.core.inbox().reviews_unasked());
    r.core.set_reviews_unasked(true).unwrap();
    sweep(&r);
    assert!(r.core.reviews_unasked());
    assert!(r.core.inbox().reviews_unasked());
}

#[test]
fn a_row_from_an_agent_or_irori_is_ignored() {
    let r = Rig::new();
    r.core.set_reviews_unasked(false).unwrap();
    for actor in [r.key_a.as_str(), "irori"] {
        let forged = serde_json::json!({
            "device": "reviews", "cap": 0, "on": true, "by": actor,
        });
        r.core
            .store()
            .append(actor, kinds::SPAWN_CAP, &forged)
            .unwrap();
    }
    sweep(&r);
    assert!(!r.core.reviews_unasked());
    assert!(!r.core.inbox().reviews_unasked());
}

#[test]
fn no_tool_sets_it() {
    let r = Rig::new();
    let specs: serde_json::Value =
        serde_json::from_slice(&r.core.tools().specs_json("claude")).unwrap();
    for s in specs.as_array().unwrap() {
        let name = s["name"].as_str().unwrap();
        assert!(
            !name.contains("review_setting") && !name.contains("reviews"),
            "{name}"
        );
    }
}
