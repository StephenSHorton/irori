//! `GET /v1/boards/settings` (v0.5 §7.2-7.3): the person's receive and
//! autonomy rows, read-only over the loopback API. New in stage 2.

mod common;

use serde_json::{Value, json};

#[test]
fn board_settings_are_read_only() {
    let rig = common::Rig::new();
    let get = || -> Value {
        let b = rig
            .rt
            .block_on(rig.client.call("GET", "/v1/boards/settings", &[], None))
            .unwrap();
        serde_json::from_slice(&b).unwrap()
    };
    assert_eq!(
        get(),
        json!({"receive": [], "autonomy": [], "receiveDefault": "open", "autonomyDefault": "trusted"})
    );
    // The app writes them as the person.
    let ib = rig.core.inbox();
    let user = format!("user:{}", rig.core.device().id);
    ib.set_receive(
        inbox::ReceiveSetting {
            board_id: "b0".into(),
            preset: inbox::receive::ASK.into(),
            ..inbox::ReceiveSetting::default()
        },
        &user,
    )
    .unwrap();
    ib.set_autonomy(
        inbox::AutonomySetting {
            board_id: "b0".into(),
            preset: inbox::autonomy::ASK.into(),
        },
        &user,
    )
    .unwrap();
    assert_eq!(
        get(),
        json!({
            "receive": [{"boardId": "b0", "preset": "ask"}],
            "autonomy": [{"boardId": "b0", "preset": "ask"}],
            "receiveDefault": "open",
            "autonomyDefault": "trusted"
        })
    );
    // Nothing changes them through the API.
    let e = rig
        .rt
        .block_on(rig.client.call(
            "POST",
            "/v1/boards/settings",
            &[],
            Some(br#"{"preset":"open"}"#.to_vec()),
        ))
        .unwrap_err();
    assert!(e.to_string().contains("405"), "{e}");
}
