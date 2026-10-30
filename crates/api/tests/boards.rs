//! `GET /v1/boards[?detail=1]` on a real Core (v0.5 §2.3, §8): never 503;
//! the person's boards, with members, fingerprints and the fold's
//! set-aside counts only under `detail`.

mod common;

use serde_json::{Value, json};

#[test]
fn the_core_serves_its_boards_with_set_aside_counts_under_detail() {
    let rig = common::Rig::new();
    let get = |q: &str| -> Value {
        let b = rig
            .rt
            .block_on(rig.client.call("GET", &format!("/v1/boards{q}"), &[], None))
            .unwrap_or_else(|e| panic!("/v1/boards{q}: {e}"));
        serde_json::from_slice(&b).unwrap()
    };
    // No boards yet: an empty list, not 503.
    assert_eq!(get(""), json!([]));
    assert_eq!(get("?detail=1"), json!([]));

    let c = &rig.core;
    let id = c.create_board("mlg farming", "mlg").unwrap();
    let me = c.person();
    let name = me.name.clone();
    let plain = json!({
        "boardId": id, "name": "mlg farming", "tag": "mlg", "people": [name],
        "owner": true, "devices": 1, "cards": 0, "ready": true,
    });
    assert_eq!(get(""), json!([plain]));
    assert_eq!(get("?detail=0"), json!([plain]));

    // A row naming a card the board doesn't hold waits, unresolved.
    let co = c.board_coord(&id).unwrap();
    co.fold_live(&[store::Event {
        device_id: "dev-elsewhere".into(),
        seq: 1,
        actor: "agent:dev-elsewhere/s1".into(),
        kind: store::kinds::TASK_CLAIMED.into(),
        payload: br#"{"taskId":"no-such-card"}"#.to_vec(),
        ..Default::default()
    }]);
    for q in ["?detail=1", "?detail=true"] {
        let got = get(q);
        let row = &got[0];
        assert_eq!(
            row["detail"]["setAside"],
            json!({"unresolved": 1, "dropped": 0, "ignored": 0}),
            "{q}: {got}"
        );
        let m = &row["detail"]["members"];
        assert_eq!(m.as_array().map(Vec::len), Some(1), "{q}: {got}");
        assert_eq!(m[0]["personId"], json!(me.id), "{got}");
        assert_eq!(
            (&m[0]["role"], &m[0]["state"], &m[0]["devices"]),
            (&json!("owner"), &json!("active"), &json!(1)),
            "{got}"
        );
        let fp = m[0]["fingerprint"].as_str().unwrap_or_default();
        assert_eq!(
            fp,
            irori_pair::fingerprint(&hex::decode(&me.pub_hex).unwrap()),
            "{got}"
        );
        assert!(!fp.is_empty(), "{got}");
        assert!(row["detail"].get("conflicts").is_none(), "{got}");
        let mut no_detail = row.clone();
        no_detail.as_object_mut().unwrap().remove("detail");
        assert_eq!(no_detail, plain, "{q}");
    }
}
