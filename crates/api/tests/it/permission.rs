//! Card #682: `POST /v1/permission/wait`, a spawned agent's permission
//! hook. The bearer token's holder posts the prompt; a session Irori
//! didn't start gets "none" at once; a body without an id or session is
//! refused, and so is any method but POST.

use serde_json::Value;

use crate::common;

const PATH: &str = "/v1/permission/wait";

fn call(rig: &common::Rig, method: &str, body: &str) -> (u16, Value) {
    let r = rig
        .rt
        .block_on(
            rig.client
                .raw(method, PATH, &[], Some(body.as_bytes().to_vec())),
        )
        .unwrap();
    let v = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
    (r.status, v)
}

#[test]
fn the_hook_route_answers_none_for_a_session_irori_did_not_start() {
    let rig = common::Rig::new();
    let (s, v) = call(
        &rig,
        "POST",
        r#"{"id":"h1","sessionId":"eeeeeeee-5555-4555-8555-000000000005","toolName":"Bash","toolInput":{"command":"ls"},"permissionMode":"default"}"#,
    );
    assert_eq!((s, &v), (200, &serde_json::json!({"decision": "none"})));
    let (s, _) = call(&rig, "POST", r#"{"toolName":"Bash"}"#);
    assert_eq!(s, 400);
    let (s, _) = call(&rig, "POST", "not json");
    assert_eq!(s, 400);
    let (s, _) = call(&rig, "PUT", "{}");
    assert_eq!(s, 405);
}
