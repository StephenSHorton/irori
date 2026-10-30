//! I:internal/core/migrate_test.go TestStartMigratesAV03DataDir: a data
//! dir as v0.3.2 left it (device.json without keys, irori.db at schema v1
//! with unsigned rows) opens, keeps its history and is signed.

mod common;

use std::path::Path;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use irori_core::Core;
use irori_identity::DeviceJson;
use store::{Vector, kinds};

fn write_v03_log(data: &Path) -> DeviceJson {
    let dev = DeviceJson::load_or_create(data).unwrap();
    let db = rusqlite::Connection::open(data.join("irori.db")).unwrap();
    db.pragma_update(None, "journal_mode", "WAL").unwrap();
    db.execute_batch(
        "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL DEFAULT (unixepoch()));
         CREATE TABLE events (
            device_id TEXT NOT NULL, seq INTEGER NOT NULL, hlc INTEGER NOT NULL,
            actor TEXT NOT NULL, kind TEXT NOT NULL, payload BLOB,
            PRIMARY KEY (device_id, seq)
         ) WITHOUT ROWID;
         CREATE INDEX events_hlc ON events (hlc);
         CREATE INDEX events_kind ON events (kind, hlc);
         INSERT INTO schema_migrations (version) VALUES (1);",
    )
    .unwrap();
    let d = dev.id.clone();
    let alpha = format!("{d}/aaaaaaaa-1111-4111-8111-111111111111");
    let beta = format!("{d}/bbbbbbbb-2222-4222-8222-222222222222");
    let at = "2026-10-20T10:00:00Z";
    let rows: Vec<(String, &str, String)> = vec![
        (
            "irori".into(),
            "device.started",
            r#"{"name":"pc","os":"windows","version":"0.3.2","headless":false}"#.into(),
        ),
        (
            "irori".into(),
            "agent.seen",
            format!(
                r#"{{"key":"{alpha}","deviceId":"{d}","agent":"claude","sessionId":"aaaaaaaa-1111-4111-8111-111111111111","name":"alpha","status":"idle"}}"#
            ),
        ),
        (
            "irori".into(),
            "agent.seen",
            format!(
                r#"{{"key":"{beta}","deviceId":"{d}","agent":"claude","sessionId":"bbbbbbbb-2222-4222-8222-222222222222","name":"beta","status":"idle"}}"#
            ),
        ),
        (
            "user".into(),
            "policy.set",
            format!(
                r#"{{"mode":"everyone","block":[],"allow":[],"paused":false,"by":"user:{d}","at":"{at}"}}"#
            ),
        ),
        (
            format!("agent:{alpha}"),
            "message.sent",
            format!(
                r#"{{"id":"msg_1","from":{{"key":"{alpha}","name":"alpha","kind":"claude"}},"to":{{"kind":"agent","value":"{beta}"}},"text":"hello from v0.3","trust":"untrusted","sentAt":"{at}"}}"#
            ),
        ),
        (
            format!("agent:{alpha}"),
            "delivery.state",
            format!(
                r#"{{"deliveryId":"del_1","messageId":"msg_1","to":"{beta}","state":"queued","by":"{alpha}","at":"{at}"}}"#
            ),
        ),
        (
            "irori".into(),
            "member.joined",
            format!(r#"{{"key":"{d}/grok-1","name":"grokky"}}"#),
        ),
        (
            format!("agent:{alpha}"),
            "task.created",
            format!(r#"{{"taskId":"t_v03","title":"carried over","by":"{alpha}","at":"{at}"}}"#),
        ),
        (
            "user".into(),
            "policy.set",
            format!(
                r#"{{"mode":"allowlist","block":["gamma"],"allow":["alpha"],"paused":true,"by":"user:{d}","at":"{at}"}}"#
            ),
        ),
    ];
    // 2026-10-20T10:00:00Z.
    let base: i64 = 1_792_490_400_000;
    for (i, (actor, kind, payload)) in rows.iter().enumerate() {
        db.execute(
            "INSERT INTO events (device_id, seq, hlc, actor, kind, payload) VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![d, i as i64 + 1, (base + i as i64) << 16, actor, kind, payload.as_bytes()],
        )
        .unwrap();
    }
    dev
}

#[test]
fn test_start_migrates_a_v03_data_dir() {
    let (data, claude) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let dev = write_v03_log(data.path());
    let start = || {
        let mut o = common::options(data.path(), claude.path());
        o.version = "0.4.0-test".into();
        Core::start(o).unwrap()
    };
    let c = start();
    assert!(
        c.device().id == dev.id
            && !c.device().device_pub.is_empty()
            && c.device().person_id == c.person().id,
        "device after migration {:?}",
        c.device()
    );
    assert!(
        data.path()
            .join(format!("irori.db{}", store::BACKUP_SUFFIX))
            .exists(),
        "no v0.3 backup"
    );
    // The last v0.3 policy survives the move to local_events.
    let pol = c.inbox().policy();
    assert!(
        pol.mode == inbox::policy_mode::ALLOWLIST && pol.paused && pol.block == ["gamma"],
        "policy after migration {pol:?}"
    );
    // Folds see the old history.
    assert!(c.coord().get_task("t_v03").is_some(), "v0.3 task missing");
    let d = c.inbox().get("del_1").expect("v0.3 delivery");
    assert_eq!(d.message_id, "msg_1");
    // Every own row is signed by the device key, and no local kind is left
    // in the log.
    let pub_bytes: [u8; 32] = hex::decode(&c.device().device_pub)
        .unwrap()
        .try_into()
        .unwrap();
    let vk = VerifyingKey::from_bytes(&pub_bytes).unwrap();
    let evs = c
        .store()
        .missing(&Vector::default(), store::MAX_MISSING_LIMIT)
        .unwrap();
    let mut moved = 0;
    for ev in &evs {
        let sig = Signature::from_slice(&ev.sig).unwrap_or_else(|_| {
            panic!("row {} ({}) has no signature", ev.seq, ev.kind);
        });
        assert!(
            vk.verify(&store::signing_bytes(ev), &sig).is_ok(),
            "row {} ({}) not signed by the device key",
            ev.seq,
            ev.kind
        );
        assert!(
            !kinds::is_local(&ev.kind),
            "local kind {} left in events",
            ev.kind
        );
        if ev.kind == kinds::LOCAL_MOVED {
            moved += 1;
        }
    }
    assert_eq!(moved, 3, "local.moved placeholders");
    // The root certified itself; its owner key is the person, a made-up one
    // isn't.
    assert!(c.is_owner(&format!("user:{}", dev.id)) && !c.is_owner("user:someone-else"));
    let ds = c.certified_devices();
    assert!(
        ds.len() == 1
            && ds[0].cert.device_id == dev.id
            && ds[0].cert.role == irori_identity::objects::ROLE_NODE,
        "certified devices {ds:?}"
    );
    let n = evs.len();
    c.close().unwrap();

    // A second start migrates nothing again and writes no second self-cert.
    let c2 = start();
    let evs2 = c2
        .store()
        .missing(&Vector::default(), store::MAX_MISSING_LIMIT)
        .unwrap();
    let certs = evs2
        .iter()
        .filter(|e| e.kind == kinds::PERSON_DEVICE_ADDED)
        .count();
    // Only the new device.started row is added.
    assert_eq!((evs2.len(), certs), (n + 1, 1), "second start");
    let pol = c2.inbox().policy();
    assert!(
        pol.mode == inbox::policy_mode::ALLOWLIST && pol.paused,
        "policy after restart {pol:?}"
    );
    assert!(
        c2.coord().get_task("t_v03").is_some(),
        "task missing after restart"
    );
    c2.close().unwrap();
}
