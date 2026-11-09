//! The device_free tool (device-free plan §5 D), over the fake Backend.

use crate::common;

use common::*;
use serde_json::json;
use std::time::Duration;

const GB: u64 = 1_000_000_000;

fn full() -> tools::DeviceFree {
    tools::DeviceFree {
        dir: "/work/proj".into(),
        disk: Some((1900 * GB, 3600 * GB)),
        mem: Some((92 * GB, 128 * GB)),
        cpu: Some((71.4, 18)),
    }
}

#[test]
fn test_device_free_prints_every_number_after_margins() {
    let h = new_harness();
    *h.b.free.lock().unwrap() = Some(full());
    let out = h.okj(&call_a(), "device_free", json!({}));
    has(
        &out,
        &[
            "Disk (/work/proj): 1.9 TB free of 3.6 TB. Up to 1.9 TB can be used (keeping 50 GB).",
            "Memory: 92 GB available of 128 GB.",
            "Up to 79 GB can be used (keeping 13 GB).",
            "CPU: 71% busy over 1 s, 18 cores.",
            "Heavy slots (2): slot-1 free; slot-2 free.",
            "lease resource=heavy/3dc30c94 ttl_minutes=",
            "Script slots: none held.",
            "untrusted",
        ],
    );
}

#[test]
fn test_device_free_margins_clamp_and_saturate() {
    let h = new_harness();
    // A small disk keeps the 10 GB floor; a tiny memory keeps 2 GB and
    // saturates at 0.
    *h.b.free.lock().unwrap() = Some(tools::DeviceFree {
        dir: "/d".into(),
        disk: Some((30 * GB, 100 * GB)),
        mem: Some((GB, 4 * GB)),
        cpu: None,
    });
    let out = h.okj(&call_a(), "device_free", json!({}));
    has(
        &out,
        &[
            "Up to 20 GB can be used (keeping 10 GB)",
            "Up to 0 MB can be used (keeping 2.0 GB)",
        ],
    );
}

#[test]
fn test_device_free_says_not_available_here() {
    let h = new_harness();
    *h.b.free.lock().unwrap() = Some(tools::DeviceFree {
        dir: "C:\\p".into(),
        disk: Some((200 * GB, 500 * GB)),
        mem: None,
        cpu: None,
    });
    let out = h.okj(&call_a(), "device_free", json!({}));
    has(
        &out,
        &[
            "Memory: not available here.",
            "CPU: not available here.",
            "Disk (C:\\p): 200 GB free",
        ],
    );
    *h.b.free.lock().unwrap() = None;
    h.failsj(&call_a(), "device_free", json!({}), &["can't be read here"]);
}

#[test]
fn test_device_free_names_each_kind_of_holder() {
    let h = new_harness();
    *h.b.free.lock().unwrap() = Some(full());
    // An agent on a card and an agent with none hold the two heavy slots.
    let card = h.new_card(&call_a(), json!({"title": "build it"}));
    h.okj(&call_a(), "board_join", json!({"task_id": card}));
    h.okj(&call_a(), "lease", json!({"resource": "heavy/3dc30c94"}));
    h.okj(&call_b(), "lease", json!({"resource": "heavy/3dc30c94"}));
    let out = h.okj(&call_c(), "device_free", json!({}));
    has(
        &out,
        &[
            "slot-1 held by alpha",
            " on #",
            "slot-2 held by beta",
            "beta (dev1/bbbbbbbb-2222-4222-8222-222222222222) (no card)",
            "All taken; heavy/3dc30c94/slot-",
            "board_block on=\"lease heavy/3dc30c94/slot-",
        ],
    );
    // A script run (no agent, no card) holds a pool slot; the store's
    // device is dev1, so this device is too for its holders.
    *h.b.device.lock().unwrap() = "dev1".into();
    h.b.co
        .acquire_slot(
            "dev1/run-check-card-9-4411",
            "check/mac",
            2,
            Duration::from_secs(600),
        )
        .unwrap();
    // Another device's run is not this device's.
    let out = h.okj(&call_c(), "device_free", json!({}));
    has(
        &out,
        &["Script slots: check/mac/slot-1 held by script check-card-9-4411 (no card)"],
    );
    *h.b.device.lock().unwrap() = "other".into();
    let out = h.okj(&call_c(), "device_free", json!({}));
    has(&out, &["Script slots: none held."]);
}

#[test]
fn test_device_free_path_default_and_relative() {
    let h = new_harness();
    *h.b.free.lock().unwrap() = Some(full());
    // No path: the caller's own folder.
    h.okj(&call_a(), "device_free", json!({}));
    // An absolute path goes through as given.
    h.okj(&call_a(), "device_free", json!({"path": "/abs/dir"}));
    // A relative path joins to the caller's folder.
    h.okj(&call_a(), "device_free", json!({"path": "rel"}));
    let asked = h.b.free_asked.lock().unwrap().clone();
    assert_eq!(asked[0], "C:\\src\\alpha");
    assert_eq!(asked[1], "/abs/dir");
    assert!(
        asked[2].starts_with("C:\\src\\alpha") && asked[2].ends_with("rel"),
        "{asked:?}"
    );
    // A caller with no known folder: no path means the data dir (""), and a
    // relative path is refused.
    let lost = claude("dev1/zzzzzzzz-9999-4999-8999-999999999999", "lost");
    h.okj(&lost, "device_free", json!({}));
    assert_eq!(h.b.free_asked.lock().unwrap().last().unwrap(), "");
    h.failsj(&lost, "device_free", json!({"path": "rel"}), &["absolute"]);
}
