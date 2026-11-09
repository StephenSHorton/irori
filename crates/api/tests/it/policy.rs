//! Card #1196 (GitHub #305): the old Pause all left the app with Stop here
//! (card #787). `POST /v1/policy` from the composer refuses to turn it on,
//! so an older mod's `/irori pause` can't strand a device again, and still
//! lifts one an old Wind down left on.

use std::time::Duration;

use irori_api::client::Opened;
use serde_json::Value;

use crate::common;

const PATH: &str = "/v1/policy";

fn attach_secret(rig: &common::Rig) -> String {
    let path = format!("/v1/attach?session={}&mod=0.7.2", common::SESS_A);
    rig.rt.block_on(async {
        let Opened::Stream(mut lines, _) = rig.client.open(&path, &[]).await.unwrap() else {
            panic!("{path}: refused")
        };
        let hello = lines
            .next_json(Duration::from_secs(5))
            .await
            .expect("hello");
        tokio::spawn(async move { while lines.next().await.is_some() {} });
        let hello: Value = serde_json::from_slice(&hello).unwrap();
        hello["attachSecret"]
            .as_str()
            .expect("attach secret")
            .to_string()
    })
}

fn post(rig: &common::Rig, secret: &str, paused: bool) -> (u16, String) {
    let body = format!(r#"{{"mode":"everyone","block":[],"allow":[],"paused":{paused}}}"#);
    let headers = [("X-Irori-Attach", secret), ("X-Irori-Origin", "composer")];
    let r = rig
        .rt
        .block_on(
            rig.client
                .raw("POST", PATH, &headers, Some(body.into_bytes())),
        )
        .unwrap();
    (r.status, String::from_utf8_lossy(&r.body).into_owned())
}

#[test]
fn the_composer_cant_turn_the_old_pause_all_on_but_can_lift_it() {
    let rig = common::Rig::new();
    let secret = attach_secret(&rig);
    let (s, body) = post(&rig, &secret, true);
    assert_eq!(s, 400, "{body}");
    assert!(body.contains("Stop here"), "{body}");
    assert!(!rig.core.inbox().policy().paused);

    // One an old Wind down left on: resume still lifts it.
    let inbox = rig.core.inbox();
    let mut pol = inbox.policy();
    pol.paused = true;
    inbox.set_policy(pol, "user:test").unwrap();
    let (s, body) = post(&rig, &secret, false);
    assert_eq!(s, 200, "{body}");
    assert!(!rig.core.inbox().policy().paused);
}
