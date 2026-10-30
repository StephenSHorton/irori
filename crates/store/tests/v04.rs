//! The Go v0.4 store tests (I:internal/store/v04_test.go, store.md §6
//! "store v0.4").

mod common;

use std::collections::BTreeMap;

use common::*;
use sha2::{Digest, Sha256};
use store::kinds::*;
use store::{BACKUP_SUFFIX, Cursor, ErrorKind, Event, Filter, Store, Timestamp};

fn file_hash(p: &std::path::Path) -> [u8; 32] {
    Sha256::digest(std::fs::read(p).unwrap()).into()
}

#[test]
fn append_signs_every_row() {
    let dir = tempdir();
    let sg = KeySigner::random();
    let s = Store::open_with(
        dir.path().join("irori.db"),
        "dev-a",
        clock(),
        opts(Some(sg.clone()), None),
    )
    .unwrap();
    let p: BTreeMap<&str, &str> = [("key", "dev-a/x")].into();
    let ev = must_append(&s, AGENT_SEEN, &p);
    assert!(sg.verifies(&ev));
    let held = all(&s);
    assert_eq!(held.len(), 1);
    assert!(sg.verifies(&held[0]));
    let mut forged = held[0].clone();
    forged.actor = "user".into();
    assert!(!sg.verifies(&forged), "signature survived an actor change");
    let raw = String::from_utf8(gocompat::json::marshal(&held[0]).unwrap()).unwrap();
    assert!(!raw.contains("sig"), "sig in JSON: {raw}");
}

#[test]
fn local_kinds_stay_in_local_events() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    let empty: BTreeMap<&str, i32> = BTreeMap::new();
    assert_eq!(
        s.append("user", POLICY_SET, &empty).unwrap_err().kind(),
        ErrorKind::LocalKind
    );
    assert_eq!(
        s.append_local("irori", AGENT_SEEN, &()).unwrap_err().kind(),
        ErrorKind::NotLocal
    );
    must_append(&s, AGENT_SEEN, &());
    for i in 0..3 {
        let p: BTreeMap<&str, i32> = [("n", i)].into();
        let ev = s.append_local("user", POLICY_SET, &p).unwrap();
        assert_eq!(ev.seq, i as i64 + 1);
    }
    s.append_local("irori", MEMBER_JOINED, &()).unwrap();
    assert_eq!(s.vector().unwrap()["dev-a"], 1, "vector counts local rows");
    let evs = all(&s);
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].kind, AGENT_SEEN);
    let loc = s
        .list(&Filter {
            local: true,
            kinds: vec![POLICY_SET.into()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(loc.len(), 3);
    assert_eq!(loc[2].payload, br#"{"n":2}"#);
    assert_eq!(
        s.insert(&ev("dev-b", 1, 1 << 16, POLICY_SET))
            .unwrap_err()
            .kind(),
        ErrorKind::LocalKind
    );
}

#[test]
fn insert_records_forks() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    let first = Event {
        device_id: "dev-b".into(),
        seq: 1,
        hlc: Timestamp(5 << 16),
        actor: "irori".into(),
        kind: AGENT_SEEN.into(),
        payload: br#"{"a":1}"#.to_vec(),
        sig: b"s1".to_vec(),
    };
    s.insert(&first).unwrap();
    s.insert(&first).unwrap();
    let mut other = first.clone();
    other.payload = br#"{"a":2}"#.to_vec();
    for _ in 0..2 {
        assert_eq!(s.insert(&other).unwrap_err().kind(), ErrorKind::Fork);
    }
    let forks = s.forks().unwrap();
    assert_eq!(forks.len(), 1);
    assert_eq!(forks[0].stored.payload, br#"{"a":1}"#);
    assert_eq!(forks[0].other.payload, br#"{"a":2}"#);
    assert_eq!(forks[0].other_hash, store::row_hash(&other));
    assert_eq!(forks[0].reason, store::FORK_TWO_ROWS);
    let evs = all(&s);
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].payload, br#"{"a":1}"#);
}

#[test]
fn insert_refuses_hlc_that_does_not_advance() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    s.insert(&ev("dev-b", 1, 10 << 16, AGENT_SEEN)).unwrap();
    for h in [10u64 << 16, 9 << 16] {
        assert_eq!(
            s.insert(&ev("dev-b", 2, h, AGENT_SEEN)).unwrap_err().kind(),
            ErrorKind::HlcOrder
        );
    }
    s.insert(&ev("dev-b", 2, (10 << 16) + 1, AGENT_SEEN))
        .unwrap();
}

#[test]
fn insert_batch_commits_up_to_the_first_refusal() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    let evs = [
        ev("dev-b", 1, 1 << 16, AGENT_SEEN),
        ev("dev-b", 2, 2 << 16, AGENT_SEEN),
        ev("dev-b", 1, 1 << 16, AGENT_SEEN), // duplicate: applied
        ev("dev-b", 4, 4 << 16, AGENT_SEEN), // gap
        ev("dev-b", 3, 3 << 16, AGENT_SEEN),
    ];
    let (n, err) = s.insert_batch(&evs);
    assert_eq!(n, 3);
    assert_eq!(err.unwrap_err().kind(), ErrorKind::Gap);
    assert_eq!(s.vector().unwrap()["dev-b"], 2);
}

#[test]
fn cursor_at_page_boundary_with_equal_hlcs() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    let tie = 7u64 << 16;
    for dev in ["dev-b", "dev-c", "dev-d"] {
        s.insert(&ev(dev, 1, tie, AGENT_SEEN)).unwrap();
    }
    let mut got = Vec::new();
    let mut after = Cursor::default();
    loop {
        let evs = s
            .list(&Filter {
                after: after.clone(),
                limit: 2,
                ..Default::default()
            })
            .unwrap();
        for ev in &evs {
            got.push(ev.device_id.clone());
            after = ev.cursor();
        }
        if evs.len() < 2 {
            break;
        }
    }
    assert_eq!(got.join(","), "dev-b,dev-c,dev-d");
    let evs = s
        .list(&Filter {
            after_hlc: Timestamp(tie - 1),
            limit: 2,
            ..Default::default()
        })
        .unwrap();
    let again = s
        .list(&Filter {
            after_hlc: evs[1].hlc,
            limit: 2,
            ..Default::default()
        })
        .unwrap();
    assert!(
        again.is_empty(),
        "AfterHLC skips the tie (the bug the cursor fixes)"
    );
}

#[test]
fn hold_appends_until_own_rows_return() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    must_append(&s, AGENT_SEEN, &());
    s.hold_appends(3);
    assert_eq!(
        s.append("irori", AGENT_SEEN, &()).unwrap_err().kind(),
        ErrorKind::Restoring
    );
    s.hold_appends(0);
    must_append(&s, AGENT_SEEN, &());
}

#[test]
fn one_time_migration_of_a_v03_log() {
    let dir = tempdir();
    let path = dir.path().join("irori.db");
    let orig = v03_db(&path, "dev-a");
    let sg = KeySigner::random();
    let open = || Store::open_with(&path, "dev-a", clock(), opts(Some(sg.clone()), None)).unwrap();
    let s = open();

    let backup = dir.path().join(format!("irori.db{BACKUP_SUFFIX}"));
    {
        let b = rusqlite::Connection::open(&backup).unwrap();
        let ver: i64 = b
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(ver, 1);
        let n: i64 = b
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n as usize, orig.len());
    }
    let sum = file_hash(&backup);

    let evs = all(&s);
    assert_eq!(evs.len(), orig.len());
    for (i, ev) in evs.iter().enumerate() {
        assert!(sg.verifies(ev), "row {} not signed", ev.seq);
        assert_eq!(ev.seq, i as i64 + 1);
        assert_eq!(ev.hlc, orig[i].hlc);
        assert_eq!(ev.actor, orig[i].actor);
        if store::is_local(&orig[i].kind) {
            assert_eq!(ev.kind, LOCAL_MOVED);
            assert_eq!(ev.payload, b"{}");
        } else {
            assert_eq!(ev.kind, orig[i].kind);
            assert_eq!(ev.payload, orig[i].payload);
        }
    }
    let loc = s
        .list(&Filter {
            local: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(loc.len(), 3);
    assert_eq!(loc[0].kind, POLICY_SET);
    assert_eq!(loc[1].kind, MEMBER_JOINED);
    assert_eq!(loc[2].kind, POLICY_SET);
    assert_eq!(loc[2].payload, orig[5].payload);
    assert_eq!(loc[2].hlc, orig[5].hlc);
    let next = must_append(&s, AGENT_STATUS, &());
    assert_eq!(next.seq, orig.len() as i64 + 1);
    assert!(sg.verifies(&next));
    s.close().unwrap();

    let s2 = open();
    assert_eq!(all(&s2).len(), orig.len() + 1);
    assert_eq!(
        s2.list(&Filter {
            local: true,
            ..Default::default()
        })
        .unwrap()
        .len(),
        3
    );
    s2.close().unwrap();
    assert_eq!(file_hash(&backup), sum, "backup rewritten on a later start");
}

#[test]
fn migration_needs_a_signer_and_refuses_unsigned_own_rows() {
    let dir = tempdir();
    let path = dir.path().join("irori.db");
    v03_db(&path, "dev-a");
    let err = Store::open(&path, "dev-a", clock()).unwrap_err();
    assert_eq!(
        err.to_string(),
        "store: migration 3: store: this log needs the v0.4 migration, which signs rows; open it with a Signer"
    );
    {
        // Nothing was migrated: the ALTER TABLE rolled back too.
        let c = rusqlite::Connection::open(&path).unwrap();
        let v: i64 = c
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(v, 1);
    }
    let sg = KeySigner::random();
    let s = Store::open_with(&path, "dev-a", clock(), opts(Some(sg.clone()), None)).unwrap();
    s.close().unwrap();
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute(
            "INSERT INTO events (device_id, seq, hlc, actor, kind, payload) VALUES ('dev-a', 99, 1, 'irori', 'agent.seen', 'null')",
            [],
        )
        .unwrap();
    }
    let err = Store::open_with(&path, "dev-a", clock(), opts(Some(sg), None)).unwrap_err();
    assert_eq!(
        err.to_string(),
        "store: 1 of this device's rows are unsigned; the log was changed outside Irori"
    );
}

#[test]
fn fresh_database_takes_no_backup() {
    let dir = tempdir();
    let s = Store::open_with(
        dir.path().join("irori.db"),
        "dev-a",
        clock(),
        opts(Some(KeySigner::random()), None),
    )
    .unwrap();
    s.close().unwrap();
    assert!(!dir.path().join(format!("irori.db{BACKUP_SUFFIX}")).exists());
}

#[test]
fn backup_retaken_after_a_rollback() {
    let dir = tempdir();
    let path = dir.path().join("irori.db");
    let orig = v03_db(&path, "dev-a");
    let sg = KeySigner::random();
    let s = Store::open_with(&path, "dev-a", clock(), opts(Some(sg.clone()), None)).unwrap();
    s.close().unwrap();
    let backup = dir.path().join(format!("irori.db{BACKUP_SUFFIX}"));
    let raw = std::fs::read(&backup).unwrap();
    for suffix in ["", "-wal", "-shm"] {
        let f = dir.path().join(format!("irori.db{suffix}"));
        match std::fs::remove_file(&f) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("{e}"),
            _ => {}
        }
    }
    std::fs::write(&path, raw).unwrap();
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute(
            "INSERT INTO events (device_id, seq, hlc, actor, kind, payload) VALUES ('dev-a', ?1, ?2, 'irori', 'agent.seen', '{}')",
            rusqlite::params![orig.len() as i64 + 1, orig[orig.len() - 1].hlc.as_i64() + (1 << 16)],
        )
        .unwrap();
    }
    let s = Store::open_with(&path, "dev-a", clock(), opts(Some(sg), None)).unwrap();
    assert_eq!(all(&s).len(), orig.len() + 1);
    s.close().unwrap();
    let b = rusqlite::Connection::open(&backup).unwrap();
    let n: i64 = b
        .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n as usize, orig.len() + 1);
    assert!(
        !dir.path()
            .join(format!("irori.db{BACKUP_SUFFIX}.tmp"))
            .exists()
    );
}

#[test]
fn text_payload_reads_back_as_bytes() {
    // Hand-written TEXT values (the 'null' above) read like BLOBs.
    let dir = tempdir();
    let path = dir.path().join("irori.db");
    let s = open_test(dir.path(), "dev-a");
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute(
            "INSERT INTO events (device_id, seq, hlc, actor, kind, payload, sig) VALUES ('dev-b', 1, 1, 'irori', 'agent.seen', 'null', x'')",
            [],
        )
        .unwrap();
    }
    let evs = all(&s);
    assert_eq!(evs[0].payload, b"null");
    assert!(evs[0].sig.is_empty(), "a zero-length sig reads as unsigned");
}
