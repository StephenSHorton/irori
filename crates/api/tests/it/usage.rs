//! Card #599: `POST /v1/usage/limits`, the mod's Claude plan rate-limit
//! windows. Only an attached mod's secret may post.

use std::time::Duration;

use irori_api::client::Opened;
use serde_json::Value;

use crate::common;

const PATH: &str = "/v1/usage/limits";

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

fn in_a_day() -> String {
    let t = std::time::SystemTime::now() + Duration::from_secs(86_400);
    gocompat::time::GoTime::from_system(t)
        .utc()
        .format_rfc3339()
}

#[test]
fn an_attached_mod_posts_the_windows() {
    let rig = common::Rig::new();
    let hello = open_stream(
        &rig,
        &format!("/v1/attach?session={}&mod=0.7.0", common::SESS_A),
    );
    let secret = hello["attachSecret"].as_str().expect("attach secret");
    let feed = rig.core.feed(&[irori_core::Topic::Usage]);
    let reset = in_a_day();
    let body = format!(
        r#"{{"rateLimits":[{{"kind":"five_hour","percentUsed":23.5,"resetsAt":"{reset}"}},{{"kind":"seven_day","percentUsed":41.2}}]}}"#
    );
    let (s, v) = post(&rig, &[("X-Irori-Attach", secret)], &body);
    assert_eq!((s, &v), (200, &serde_json::json!({"ok": true})));
    let got = rig.core.usage_limits();
    assert!(got.observed_at_ms > 0);
    let kinds: Vec<_> = got
        .windows
        .iter()
        .map(|w| (w.kind.as_str(), w.percent_used))
        .collect();
    assert_eq!(kinds, [("five_hour", 23.5), ("seven_day", 41.2)]);
    assert!(got.windows[0].resets_at_ms.is_some());
    assert_eq!(
        feed.queue().recv_timeout(Duration::from_secs(5)),
        Ok(Some(irori_core::CoreEvent::Usage))
    );

    // A body that isn't a reading is refused; one with no window the Core
    // can take is fine but moves nothing, not even the observed time.
    let many = format!(
        r#"{{"rateLimits":[{}]}}"#,
        [r#"{"kind":"k","percentUsed":1}"#; irori_core::usage::MAX_WINDOWS + 1].join(",")
    );
    for bad in [
        r#"{"rateLimits":[{"kind":"five_hour","percentUsed":"lots"}]}"#,
        r#"{"rateLimits":"#,
        many.as_str(),
    ] {
        let (s, v) = post(&rig, &[("X-Irori-Attach", secret)], bad);
        assert_eq!(s, 400, "{bad}: {v}");
    }
    for empty in [
        r#"{}"#,
        r#"{"rateLimits":[]}"#,
        r#"{"rateLimits":[{"kind":"five hour","percentUsed":1}]}"#,
        r#"{"rateLimits":[{"kind":"five_hour","percentUsed":1,"resetsAt":"soon"}]}"#,
        r#"{"rateLimits":[{"kind":"five_hour","percentUsed":1,"resetsAt":"2001-01-01T00:00:00Z"}]}"#,
    ] {
        let (s, v) = post(&rig, &[("X-Irori-Attach", secret)], empty);
        assert_eq!((s, &v), (200, &serde_json::json!({"ok": true})), "{empty}");
    }
    assert_eq!(rig.core.usage_limits(), got);
    assert_eq!(
        feed.queue().recv_timeout(Duration::from_millis(200)),
        Err(())
    );

    // One bad window leaves out only itself; a dotted kind is a kind.
    let body = format!(
        r#"{{"rateLimits":[{{"kind":"five hour","percentUsed":99}},{{"kind":"seven_day.opus","percentUsed":7,"resetsAt":"{reset}"}}]}}"#
    );
    let (s, v) = post(&rig, &[("X-Irori-Attach", secret)], &body);
    assert_eq!(s, 200, "{v}");
    let kinds: Vec<_> = rig
        .core
        .usage_limits()
        .windows
        .iter()
        .map(|w| (w.kind.clone(), w.percent_used))
        .collect();
    assert_eq!(
        kinds,
        [
            ("five_hour".to_string(), 23.5),
            ("seven_day".to_string(), 41.2),
            ("seven_day.opus".to_string(), 7.0)
        ]
    );
}

#[test]
fn only_a_live_attach_may_post() {
    let rig = common::Rig::new();
    let body = r#"{"rateLimits":[{"kind":"five_hour","percentUsed":10}]}"#;
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
    assert_eq!(rig.core.usage_limits().observed_at_ms, 0);

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
}

#[test]
fn a_new_account_in_the_body_replaces_the_windows() {
    let rig = common::Rig::new();
    let hello = open_stream(
        &rig,
        &format!("/v1/attach?session={}&mod=0.7.1", common::SESS_A),
    );
    let secret = hello["attachSecret"].as_str().expect("attach secret");
    let reset = in_a_day();
    let reading = |five: f64, account: &str| {
        format!(
            r#"{{"rateLimits":[{{"kind":"five_hour","percentUsed":{five},"resetsAt":"{reset}"}},{{"kind":"seven_day","percentUsed":50,"resetsAt":"{reset}"}}],"account":"{account}"}}"#
        )
    };
    let (s, v) = post(&rig, &[("X-Irori-Attach", secret)], &reading(80.0, "a/o"));
    assert_eq!(s, 200, "{v}");
    // The same account keeps the higher figure, another takes the lower.
    let (s, _) = post(&rig, &[("X-Irori-Attach", secret)], &reading(10.0, "a/o"));
    assert_eq!(s, 200);
    assert_eq!(rig.core.usage_limits().windows[0].percent_used, 80.0);
    let only_five = format!(
        r#"{{"rateLimits":[{{"kind":"five_hour","percentUsed":4,"resetsAt":"{reset}"}}],"account":"b"}}"#
    );
    let (s, _) = post(&rig, &[("X-Irori-Attach", secret)], &only_five);
    assert_eq!(s, 200);
    let got = rig.core.usage_limits();
    assert_eq!(got.windows.len(), 1);
    assert_eq!(got.windows[0].percent_used, 4.0);
    assert_eq!(got.account.as_deref(), Some("b"));
}
