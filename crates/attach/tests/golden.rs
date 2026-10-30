//! testdata/golden/attach/attach.json, exported from the Go build by
//! tools/goldens/attach_golden_test.go: every event line byte for byte,
//! Preview and ModAtLeast, against the API crate's wire types (the ones
//! `GET /v1/attach` writes).

use attach::{Api, Event, mod_at_least, preview};
use serde_json::Value;

fn golden() -> Value {
    let p = manifest_dir().join("../../testdata/golden/attach/attach.json");
    serde_json::from_slice(&std::fs::read(p).expect("attach golden")).unwrap()
}

fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or("").to_string()
}

fn list(v: &Value, k: &str) -> Vec<String> {
    v[k].as_array()
        .map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect())
        .unwrap_or_default()
}

/// The Event a Go line decodes to (every key it has is a field).
fn event(v: &Value) -> Event {
    let known = [
        "type",
        "v",
        "session",
        "agentKey",
        "name",
        "nameSource",
        "aiTitle",
        "api",
        "attachSecret",
        "wakeId",
        "token",
        "reminder",
        "deliveryIds",
        "messageIds",
        "prompt",
        "from",
        "mode",
        "overflow",
        "deliveryId",
        "why",
        "preview",
        "to",
        "unread",
        "held",
        "message",
    ];
    for k in v.as_object().unwrap().keys() {
        assert!(
            known.contains(&k.as_str()),
            "a field the Rust Event lacks: {k}"
        );
    }
    Event {
        kind: s(v, "type"),
        v: v["v"].as_i64().unwrap_or(0),
        session: s(v, "session"),
        agent_key: s(v, "agentKey"),
        name: s(v, "name"),
        name_source: s(v, "nameSource"),
        ai_title: s(v, "aiTitle"),
        api: v.get("api").map(|a| Api {
            url: s(a, "url"),
            token: s(a, "token"),
        }),
        attach_secret: s(v, "attachSecret"),
        wake_id: s(v, "wakeId"),
        token: s(v, "token"),
        reminder: v["reminder"].as_bool().unwrap_or(false),
        delivery_ids: list(v, "deliveryIds"),
        message_ids: list(v, "messageIds"),
        prompt: s(v, "prompt"),
        from: s(v, "from"),
        mode: s(v, "mode"),
        overflow: list(v, "overflow"),
        delivery_id: s(v, "deliveryId"),
        why: s(v, "why"),
        preview: s(v, "preview"),
        to: s(v, "to"),
        unread: v["unread"].as_i64().unwrap_or(0),
        held_n: v["held"].as_i64().unwrap_or(0),
        message: s(v, "message"),
    }
}

#[test]
fn event_lines_match_go() {
    let g = golden();
    let evs = g["events"].as_array().unwrap();
    assert!(evs.len() >= 14);
    for e in evs {
        let line = e["line"].as_str().unwrap();
        let v: Value = serde_json::from_str(line).unwrap();
        let ours = gocompat::json::marshal(&event(&v)).unwrap();
        assert_eq!(String::from_utf8(ours).unwrap(), line);
    }
}

#[test]
fn previews_match_go() {
    for p in golden()["previews"].as_array().unwrap() {
        let (i, o) = (p["in"].as_str().unwrap(), p["out"].as_str().unwrap());
        assert_eq!(preview(i), o, "preview({i:?})");
    }
}

#[test]
fn mod_at_least_matches_go() {
    let g = golden();
    let ms = g["modAtLeast"].as_array().unwrap();
    assert!(ms.len() > 100);
    for m in ms {
        let (v, min) = (m["v"].as_str().unwrap(), m["min"].as_str().unwrap());
        assert_eq!(
            mod_at_least(v, min),
            m["want"].as_bool().unwrap(),
            "mod_at_least({v:?}, {min:?})"
        );
    }
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
