//! Card #1249: `POST /v1/runs`, the mod's reports of an agent's runs. Only
//! an attached mod's secret may post, and the run is that agent's.

use std::time::Duration;

use irori_api::client::Opened;
use irori_core::runs::RunKind;
use serde_json::Value;

use crate::common;

const PATH: &str = "/v1/runs";

fn open_stream(rig: &common::Rig, path: &str) -> Value {
    rig.rt.block_on(async {
        let Opened::Stream(mut lines, _) = rig.client.open(path, &[]).await.unwrap() else {
            panic!("{path}: refused")
        };
        let hello = lines
            .next_json(Duration::from_secs(5))
            .await
            .expect("hello");
        tokio::spawn(async move { while lines.next().await.is_some() {} });
        serde_json::from_slice(&hello).unwrap()
    })
}

fn post(rig: &common::Rig, headers: &[(&str, &str)], body: &str) -> (u16, Value) {
    let r = rig
        .rt
        .block_on(
            rig.client
                .raw("POST", PATH, headers, Some(body.as_bytes().to_vec())),
        )
        .unwrap();
    let v = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
    (r.status, v)
}

#[test]
fn an_attached_mod_posts_a_start_and_an_end() {
    let rig = common::Rig::new();
    let hello = open_stream(
        &rig,
        &format!("/v1/attach?session={}&mod=0.7.0", common::SESS_A),
    );
    let secret = hello["attachSecret"].as_str().expect("attach secret");
    let feed = rig.core.feed(&[irori_core::Topic::Runs]);
    // A body naming another agent changes nothing: the run is the caller's.
    let start = serde_json::json!({
        "event": "start", "kind": "bash", "taskId": "be9ihtlp2",
        "toolUseId": "toolu_1", "command": "cargo test",
        "outputFile": "C:\\t\\tasks\\be9ihtlp2.output", "key": rig.key_b,
    })
    .to_string();
    let (s, v) = post(&rig, &[("X-Irori-Attach", secret)], &start);
    assert_eq!((s, &v), (200, &serde_json::json!({"ok": true})));
    assert_eq!(
        feed.queue().recv_timeout(Duration::from_secs(5)),
        Ok(Some(irori_core::CoreEvent::Runs {
            keys: vec![rig.key_a.clone()]
        }))
    );
    assert!(rig.core.runs_of(&rig.key_b).is_empty());
    let got = rig.core.runs_of(&rig.key_a);
    assert_eq!(got.len(), 1);
    assert_eq!(
        (got[0].task_id.as_str(), got[0].kind, got[0].cmd.as_str()),
        ("be9ihtlp2", RunKind::Bash, "cargo test")
    );
    assert!(got[0].running());

    let (s, v) = post(
        &rig,
        &[("X-Irori-Attach", secret)],
        r#"{"event":"end","taskId":"be9ihtlp2","status":"failed","exitCode":101}"#,
    );
    assert_eq!(s, 200, "{v}");
    let got = rig.core.runs_of(&rig.key_a);
    assert!(!got[0].running());
    assert_eq!(got[0].exit, Some(101));
    assert!(matches!(
        feed.queue().recv_timeout(Duration::from_secs(5)),
        Ok(Some(irori_core::CoreEvent::Runs { .. }))
    ));

    // A repeat is fine and no event; a report that isn't one is 400.
    let (s, _) = post(
        &rig,
        &[("X-Irori-Attach", secret)],
        r#"{"event":"end","taskId":"be9ihtlp2","exitCode":101}"#,
    );
    assert_eq!(s, 200);
    assert_eq!(
        feed.queue().recv_timeout(Duration::from_millis(200)),
        Err(())
    );
    for bad in [
        r#"{"event":"start","kind":"bash""#,
        r#"{"event":"begin","kind":"bash","taskId":"b1"}"#,
        r#"{"event":"start","kind":"bash"}"#,
        r#"{"event":"start","kind":"bash","taskId":"b1","exitCode":"x"}"#,
    ] {
        let (s, v) = post(&rig, &[("X-Irori-Attach", secret)], bad);
        assert_eq!(s, 400, "{bad}: {v}");
    }
    assert_eq!(rig.core.runs_of(&rig.key_a).len(), 1);
}

#[test]
fn only_a_live_attach_may_post() {
    let rig = common::Rig::new();
    let body = r#"{"event":"start","kind":"bash","taskId":"b1","command":"sleep 30"}"#;
    // The token alone, a secret nobody holds, and a Grok member's secret.
    let (s, _) = post(&rig, &[], body);
    assert_eq!(s, 403);
    let (s, v) = post(&rig, &[("X-Irori-Attach", "nope")], body);
    assert_eq!(s, 403, "{v}");
    let joined = rig
        .rt
        .block_on(rig.client.call(
            "POST",
            "/v1/members",
            &[],
            Some(br#"{"name":"gamma","session":"g1"}"#.to_vec()),
        ))
        .unwrap();
    let m: Value = serde_json::from_slice(&joined).unwrap();
    let member = m["memberSecret"].as_str().unwrap();
    open_stream(&rig, &format!("/v1/attach?member={member}"));
    let (s, _) = post(&rig, &[("X-Irori-Member", member)], body);
    assert_eq!(s, 403);
    assert!(rig.core.runs().is_empty());

    // No token at all is 401, and the route is POST only.
    let bare = irori_api::client::Client::at(rig.port, "wrong");
    let r = rig
        .rt
        .block_on(bare.raw("POST", PATH, &[], Some(body.as_bytes().to_vec())))
        .unwrap();
    assert_eq!(r.status, 401);
    let r = rig
        .rt
        .block_on(rig.client.raw("GET", PATH, &[], None))
        .unwrap();
    assert_eq!(r.status, 405);
    assert!(rig.core.runs().is_empty());
}
