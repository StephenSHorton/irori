//! Card #437: `POST /v1/slots` and `/v1/slots/release`, check.sh's queue.

mod common;

use serde_json::Value;

fn post(rig: &common::Rig, path: &str) -> (u16, Value) {
    let r = rig
        .rt
        .block_on(rig.client.raw("POST", path, &[], None))
        .unwrap();
    let v = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
    (r.status, v)
}

fn take(rig: &common::Rig, run: &str) -> (u16, Value) {
    post(
        rig,
        &format!("/v1/slots?resource=check/mac&slots=2&ttl=60&run={run}"),
    )
}

#[test]
fn two_runs_hold_the_slots_and_the_third_is_told_who() {
    let rig = common::Rig::new();
    let (s, a) = take(&rig, "check-irori-1");
    assert_eq!(s, 200, "{a}");
    assert_eq!(a["resource"], "check/mac/slot-1");
    let dev = rig.core.device_id();
    assert_eq!(a["holder"], format!("{dev}/run-check-irori-1"));
    let (s, b) = take(&rig, "check-irori-2");
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["resource"], "check/mac/slot-2");

    let (s, c) = take(&rig, "check-irori-3");
    assert_eq!(s, 409, "{c}");
    assert_eq!(
        c["held"],
        serde_json::json!(["check-irori-1", "check-irori-2"])
    );

    // A renew keeps the slot.
    let (s, a2) = take(&rig, "check-irori-1");
    assert_eq!(
        (s, &a2["resource"]),
        (200, &Value::from("check/mac/slot-1"))
    );

    let (s, r) = post(
        &rig,
        "/v1/slots/release?resource=check/mac&run=check-irori-1",
    );
    assert_eq!((s, &r["released"]), (200, &Value::Bool(true)));
    let (s, c) = take(&rig, "check-irori-3");
    assert_eq!((s, &c["resource"]), (200, &Value::from("check/mac/slot-1")));
}

#[test]
fn slots_check_their_arguments_and_method() {
    let rig = common::Rig::new();
    for q in [
        "resource=check/mac&slots=2&ttl=60",
        "resource=check/mac&slots=2&ttl=60&run=Bad%20Run",
        "resource=check/mac&slots=0&ttl=60&run=a",
        "resource=check/mac&slots=2&ttl=1&run=a",
        "resource=port:80&slots=2&ttl=60&run=a",
    ] {
        let (s, v) = post(&rig, &format!("/v1/slots?{q}"));
        assert_eq!(s, 400, "{q}: {v}");
    }
    let r = rig
        .rt
        .block_on(rig.client.raw("GET", "/v1/slots", &[], None))
        .unwrap();
    assert_eq!(r.status, 405);
}
