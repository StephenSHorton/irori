//! The window assistant's caller (card #783): `X-Irori-Assistant` with
//! this run's secret reads as the person through `tools::ASSISTANT_TOOLS`
//! only, and nothing else.

use crate::common;

use irori_api::HEADER_ASSISTANT;
use irori_api::attach::HEADER_MEMBER;
use serde_json::{Value, json};

fn call(rig: &common::Rig, headers: &[(&str, &str)], name: &str, args: Value) -> (u16, String) {
    let body = serde_json::to_vec(&json!({ "args": args })).unwrap();
    let r = rig
        .rt
        .block_on(
            rig.client
                .raw("POST", &format!("/v1/tools/{name}"), headers, Some(body)),
        )
        .unwrap();
    let v: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
    let text = v["text"]
        .as_str()
        .or(v["error"].as_str())
        .unwrap_or_default()
        .to_string();
    (r.status, text)
}

#[test]
fn assistant_reads_as_the_person_and_only_reads() {
    let rig = common::Rig::new();
    let secret = rig.core.assistant_secret().to_string();
    assert_eq!(secret.len(), 64);
    let me = format!("user:{}", rig.core.device_id());
    let co = rig.core.coord();
    let card = co
        .create_task(
            &me,
            coord::NewTask {
                title: "rename the pineapple settings".into(),
                objective: "plain words".into(),
                ..Default::default()
            },
        )
        .unwrap();
    // Assigned to the person: the person's Board shows it as needing them.
    co.assign(&me, &card.id, &me).unwrap();
    let h = [(HEADER_ASSISTANT, secret.as_str())];

    let (code, text) = call(&rig, &h, "search", json!({ "query": "pineapple" }));
    assert_eq!(code, 200, "{text}");
    assert!(text.contains("rename the pineapple settings"), "{text}");

    let (code, text) = call(&rig, &h, "show", json!({ "ref": card.short_ref() }));
    assert_eq!(code, 200, "{text}");
    assert!(text.contains("plain words"), "{text}");

    let (code, text) = call(&rig, &h, "board", json!({}));
    assert_eq!(code, 200, "{text}");
    assert!(text.contains("rename the pineapple settings"), "{text}");
    assert!(!text.starts_with("Error"), "{text}");

    let (code, text) = call(&rig, &h, "overview", json!({}));
    assert_eq!(code, 200, "{text}");
    assert!(text.starts_with("Overview for "), "{text}");

    // Every allowed tool runs for it (none needs an agent).
    for name in tools::ASSISTANT_TOOLS {
        let args = if name == "why" {
            json!({ "path": "README.md" })
        } else {
            json!({})
        };
        let (code, text) = call(&rig, &h, name, args);
        assert_eq!(code, 200, "{name}: {text}");
        assert!(!text.contains("isn't bound to an agent"), "{name}: {text}");
    }

    // A write is refused before it runs, read tools outside the list too.
    for name in ["board_create", "send", "inbox", "talk_read", "join"] {
        let (code, text) = call(&rig, &h, name, json!({ "title": "nope" }));
        assert_eq!(code, 403, "{name}: {text}");
        assert!(text.contains("the assistant only reads"), "{name}: {text}");
    }
    assert!(
        co.list_tasks(&coord::TaskFilter::default())
            .iter()
            .all(|t| t.title != "nope")
    );

    // A wrong secret, or the header beside another identity, is refused.
    let (code, _) = call(
        &rig,
        &[(HEADER_ASSISTANT, &"0".repeat(64))],
        "search",
        json!({ "query": "pineapple" }),
    );
    assert_eq!(code, 403);
    let (code, _) = call(
        &rig,
        &[(HEADER_ASSISTANT, &secret), (HEADER_MEMBER, "x")],
        "search",
        json!({ "query": "pineapple" }),
    );
    assert_eq!(code, 403);
    let (code, _) = call(&rig, &[], "search", json!({ "query": "pineapple" }));
    assert_eq!(code, 403);
}

#[test]
fn assistant_tool_list_is_exactly_the_read_list() {
    let rig = common::Rig::new();
    let out = rig
        .rt
        .block_on(
            rig.client
                .call("GET", "/v1/tools?client=assistant", &[], None),
        )
        .unwrap();
    let specs: Vec<Value> = serde_json::from_slice(&out).unwrap();
    let mut got: Vec<String> = specs
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_string())
        .collect();
    got.sort();
    let mut want: Vec<String> = tools::ASSISTANT_TOOLS
        .iter()
        .map(|s| s.to_string())
        .collect();
    want.sort();
    assert_eq!(got, want);
    assert!(specs.iter().all(|s| s["readOnly"] == true), "{specs:?}");
}

/// Card #1009: `screen` returns what the window last said it shows, and
/// says so plainly before it has said anything; no agent client lists it.
#[test]
fn screen_returns_the_windows_snapshot_and_only_to_the_assistant() {
    let rig = common::Rig::new();
    let secret = rig.core.assistant_secret().to_string();
    let h = [(HEADER_ASSISTANT, secret.as_str())];

    let (code, text) = call(&rig, &h, "screen", json!({}));
    assert_eq!(code, 200, "{text}");
    assert!(text.contains("has not said what is on screen"), "{text}");

    rig.core
        .set_assistant_screen("Page: Home\nIn view: #12 Rename the pineapple");
    let (code, text) = call(&rig, &h, "screen", json!({}));
    assert_eq!(code, 200, "{text}");
    assert_eq!(text, "Page: Home\nIn view: #12 Rename the pineapple");

    // The mod and `irori mcp` for agents never list it.
    for client in ["claude", "grok", "assistant"] {
        let out = rig
            .rt
            .block_on(
                rig.client
                    .call("GET", &format!("/v1/tools?client={client}"), &[], None),
            )
            .unwrap();
        let specs: Vec<Value> = serde_json::from_slice(&out).unwrap();
        let has = specs.iter().any(|s| s["name"] == "screen");
        assert_eq!(has, client == "assistant", "client {client:?}");
    }
}
