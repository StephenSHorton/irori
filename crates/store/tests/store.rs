//! The Go store tests (I:internal/store/store_test.go, store.md §6 "store").

mod common;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::*;
use gocompat::sub::TryRecv;
use store::kinds::*;
use store::{
    Clock, DEFAULT_LIST_LIMIT, ErrorKind, Event, Filter, MAX_LIST_LIMIT, MAX_REPLICATED_HLC,
    SUB_BUFFER, Store, Timestamp, Vector,
};

fn y3000() -> Timestamp {
    // time.Date(3000, 1, 1, 0, 0, 0, 0, time.UTC).UnixMilli()
    Timestamp::from_unix_ms(32_503_680_000_000)
}

fn now_ts() -> Timestamp {
    Timestamp::from_unix_ms(hlc::unix_milli(SystemTime::now()))
}

#[test]
fn concurrent_appends_are_gapless() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    const WORKERS: usize = 8;
    const EACH: usize = 200;
    std::thread::scope(|sc| {
        for w in 0..WORKERS {
            let s = &s;
            sc.spawn(move || {
                for i in 0..EACH {
                    let p: std::collections::BTreeMap<&str, usize> =
                        [("w", w), ("i", i)].into_iter().collect();
                    s.append("irori", AGENT_STATUS, &p).unwrap();
                }
            });
        }
    });
    let evs = s
        .missing(&Vector::new(), (WORKERS * EACH + 1) as i64)
        .unwrap();
    assert_eq!(evs.len(), WORKERS * EACH);
    for (i, ev) in evs.iter().enumerate() {
        assert_eq!(ev.seq, i as i64 + 1, "row {i}");
        if i > 0 {
            assert!(ev.hlc > evs[i - 1].hlc, "seq {} hlc not after", ev.seq);
        }
    }
}

#[test]
fn reopen_continues_seq() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    let one: std::collections::BTreeMap<&str, &str> = [("name", "one")].into();
    must_append(&s, DEVICE_STARTED, &one);
    must_append(
        &s,
        DEVICE_STARTED,
        &[("name", "two")]
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>(),
    );
    s.close().unwrap();
    let s = open_test(dir.path(), "dev-a");
    let ev = must_append(
        &s,
        DEVICE_STARTED,
        &[("name", "three")]
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>(),
    );
    assert_eq!(ev.seq, 3);
    let evs = all(&s);
    assert_eq!(evs.len(), 3);
    assert_eq!(evs[0].payload, br#"{"name":"one"}"#);
}

#[test]
fn insert_is_idempotent_and_refuses_own_id() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    let remote = Event {
        device_id: "dev-b".into(),
        seq: 1,
        hlc: Timestamp(42 << 16),
        actor: "irori".into(),
        kind: AGENT_SEEN.into(),
        payload: br#"{"x":1}"#.to_vec(),
        sig: vec![],
    };
    for _ in 0..3 {
        s.insert(&remote).unwrap();
    }
    assert_eq!(all(&s), vec![remote.clone()]);
    let mut own = remote.clone();
    own.device_id = "dev-a".into();
    assert_eq!(s.insert(&own).unwrap_err().kind(), ErrorKind::OwnDevice);
    let mut gap = remote.clone();
    gap.seq = 3;
    assert_eq!(s.insert(&gap).unwrap_err().kind(), ErrorKind::Gap);
}

#[test]
fn insert_observes_clock() {
    let dir = tempdir();
    let clock = Arc::new(Clock::with_now(Box::new(|| {
        UNIX_EPOCH + Duration::from_millis(1000)
    })));
    let s = Store::open(dir.path().join("irori.db"), "dev-a", clock).unwrap();
    let future = Timestamp(9_000_000 << 16);
    s.insert(&ev("dev-b", 1, future.0, AGENT_GONE)).unwrap();
    let got = must_append(&s, AGENT_GONE, &());
    assert!(got.hlc > future);
}

#[test]
fn hlc_round_trips_as_int64() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    let stamps = [Timestamp(1), now_ts(), y3000(), MAX_REPLICATED_HLC];
    for (i, ts) in stamps.iter().enumerate() {
        s.insert(&ev("dev-b", i as i64 + 1, ts.0, AGENT_SEEN))
            .unwrap();
    }
    let evs = s.list(&Filter::default()).unwrap();
    for (i, ev) in evs.iter().enumerate() {
        assert_eq!(ev.hlc, stamps[i], "row {i}");
    }
    // The clock observed the ceiling and still has room for own rows.
    let ev = s.append("irori", AGENT_SEEN, &()).unwrap();
    assert!(ev.hlc > MAX_REPLICATED_HLC);
}

#[test]
fn insert_refuses_hlc_past_ceiling() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    for ts in [
        MAX_REPLICATED_HLC.0 + 1,
        i64::MAX as u64,
        i64::MAX as u64 + 1,
        u64::MAX,
    ] {
        assert!(s.insert(&ev("dev-b", 1, ts, AGENT_SEEN)).is_err(), "{ts}");
    }
    let ev = s.append("irori", AGENT_SEEN, &()).unwrap();
    assert!(ev.hlc < MAX_REPLICATED_HLC, "refused rows moved the clock");
}

#[test]
fn refused_insert_does_not_move_clock() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    let future = y3000();
    assert_eq!(
        s.insert(&ev("dev-b", 5, future.0, AGENT_SEEN))
            .unwrap_err()
            .kind(),
        ErrorKind::Gap
    );
    s.insert(&ev("dev-b", 1, 1 << 16, AGENT_SEEN)).unwrap();
    assert_eq!(
        s.insert(&ev("dev-b", 1, future.0, AGENT_SEEN))
            .unwrap_err()
            .kind(),
        ErrorKind::Fork
    );
    let ev = s.append("irori", AGENT_SEEN, &()).unwrap();
    assert!(ev.hlc < future);
}

#[test]
fn open_seeds_clock_from_held_rows() {
    let dir = tempdir();
    let future = y3000();
    let s = open_test(dir.path(), "dev-a");
    s.insert(&ev("dev-b", 1, future.0, AGENT_SEEN)).unwrap();
    s.close().unwrap();
    let s2 = open_test(dir.path(), "dev-a");
    let ev = s2.append("irori", AGENT_SEEN, &()).unwrap();
    assert!(ev.hlc > future, "reopened clock not seeded");
}

#[test]
fn insert_refuses_invalid_json_payload() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    let mut bad = ev("dev-b", 1, 1 << 16, AGENT_SEEN);
    bad.payload = b"{not json".to_vec();
    assert!(s.insert(&bad).is_err());
    let mut good = bad.clone();
    good.payload = br#"{"ok":true}"#.to_vec();
    s.insert(&good).unwrap();
    let evs = s.list(&Filter::default()).unwrap();
    let raw = gocompat::json::marshal(&evs).unwrap();
    assert_eq!(
        String::from_utf8(raw).unwrap(),
        format!(
            r#"[{{"deviceId":"dev-b","seq":1,"hlc":{},"actor":"","kind":"agent.seen","payload":{{"ok":true}}}}]"#,
            1u64 << 16
        )
    );
}

#[test]
fn subscribe_ends_on_close() {
    // Go: no goroutine outlives Close. Here: every subscription ends.
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    let subs: Vec<_> = (0..100).map(|_| s.subscribe()).collect();
    s.close().unwrap();
    for sub in &subs {
        assert_eq!(sub.recv_timeout(Duration::from_secs(2)), Ok(None));
    }
}

/// Go `syncOnce`: copy what dst lacks from src, a page of 7 at a time.
fn sync_once(src: &Store, dst: &Store) {
    loop {
        let v = dst.vector().unwrap();
        let evs = src.missing(&v, 7).unwrap();
        if evs.is_empty() {
            return;
        }
        for ev in &evs {
            dst.insert(ev).unwrap();
        }
    }
}

#[test]
fn replication_both_ways() {
    let (da, db, dc) = (tempdir(), tempdir(), tempdir());
    let a = open_test(da.path(), "dev-a");
    let b = open_test(db.path(), "dev-b");
    let c = open_test(dc.path(), "dev-c");
    let m = |k: &'static str, i: i32| -> std::collections::BTreeMap<&'static str, i32> {
        [(k, i)].into()
    };
    for i in 0..20 {
        must_append(&a, AGENT_STATUS, &m("a", i));
    }
    for i in 0..13 {
        must_append(&b, AGENT_SEEN, &m("b", i));
    }
    for i in 0..5 {
        must_append(&c, AGENT_GONE, &m("c", i));
    }
    sync_once(&c, &b);
    sync_once(&a, &b);
    sync_once(&b, &a);
    sync_once(&a, &b);
    let (ra, rb) = (all(&a), all(&b));
    assert_eq!(ra.len(), 38);
    assert_eq!(ra, rb);
    let want: Vector = [("dev-a", 20), ("dev-b", 13), ("dev-c", 5)]
        .into_iter()
        .map(|(d, s)| (d.to_string(), s))
        .collect();
    assert_eq!(a.vector().unwrap(), want);
    assert_eq!(b.vector().unwrap(), want);
    assert!(a.missing(&b.vector().unwrap(), 0).unwrap().is_empty());
}

#[test]
fn missing_respects_vector_and_limit() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    for _ in 0..5 {
        must_append(&s, AGENT_STATUS, &());
    }
    let v: Vector = [("dev-a".to_string(), 2)].into();
    let evs = s.missing(&v, 2).unwrap();
    assert_eq!(evs.iter().map(|e| e.seq).collect::<Vec<_>>(), [3, 4]);
}

#[test]
fn list_filters() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    let started = must_append(&s, DEVICE_STARTED, &());
    let seen = must_append(&s, AGENT_SEEN, &());
    let status = must_append(&s, AGENT_STATUS, &());
    s.insert(&ev("dev-b", 1, status.hlc.0 + 1, AGENT_GONE))
        .unwrap();
    s.insert(&ev("dev-b", 2, status.hlc.0 + 2, "agentx.thing"))
        .unwrap();
    let kinds =
        |f: Filter| -> Vec<String> { s.list(&f).unwrap().into_iter().map(|e| e.kind).collect() };
    let k = |v: &[&str]| -> Vec<String> { v.iter().map(|s| s.to_string()).collect() };
    let ks = |v: &[&str]| -> Vec<String> { v.iter().map(|s| s.to_string()).collect() };
    let cases: Vec<(&str, Filter, Vec<String>)> = vec![
        (
            "all",
            Filter::default(),
            k(&[
                DEVICE_STARTED,
                AGENT_SEEN,
                AGENT_STATUS,
                AGENT_GONE,
                "agentx.thing",
            ]),
        ),
        (
            "prefix",
            Filter {
                kinds: ks(&["agent."]),
                ..Default::default()
            },
            k(&[AGENT_SEEN, AGENT_STATUS, AGENT_GONE]),
        ),
        (
            "exact",
            Filter {
                kinds: ks(&[AGENT_SEEN]),
                ..Default::default()
            },
            k(&[AGENT_SEEN]),
        ),
        (
            "no dot is exact",
            Filter {
                kinds: ks(&["agent"]),
                ..Default::default()
            },
            vec![],
        ),
        (
            "mixed",
            Filter {
                kinds: ks(&[DEVICE_STARTED, "agentx."]),
                ..Default::default()
            },
            k(&[DEVICE_STARTED, "agentx.thing"]),
        ),
        (
            "device",
            Filter {
                device_id: "dev-b".into(),
                ..Default::default()
            },
            k(&[AGENT_GONE, "agentx.thing"]),
        ),
        (
            "after",
            Filter {
                after_hlc: seen.hlc,
                ..Default::default()
            },
            k(&[AGENT_STATUS, AGENT_GONE, "agentx.thing"]),
        ),
        (
            "limit",
            Filter {
                limit: 2,
                ..Default::default()
            },
            k(&[DEVICE_STARTED, AGENT_SEEN]),
        ),
        (
            "desc",
            Filter {
                desc: true,
                limit: 2,
                ..Default::default()
            },
            k(&["agentx.thing", AGENT_GONE]),
        ),
        (
            "combined",
            Filter {
                kinds: ks(&["agent."]),
                device_id: "dev-a".into(),
                after_hlc: started.hlc,
                desc: true,
                ..Default::default()
            },
            k(&[AGENT_STATUS, AGENT_SEEN]),
        ),
    ];
    for (name, f, want) in cases {
        assert_eq!(kinds(f), want, "{name}");
    }
}

#[test]
fn list_limit_bounds() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    let rows: Vec<Event> = (0..MAX_LIST_LIMIT + 5)
        .map(|i| ev("dev-b", i + 1, i as u64 + 1, AGENT_STATUS))
        .collect();
    let (n, r) = s.insert_batch(&rows);
    r.unwrap();
    assert_eq!(n as i64, MAX_LIST_LIMIT + 5);
    for (limit, want) in [
        (0, DEFAULT_LIST_LIMIT),
        (-1, DEFAULT_LIST_LIMIT),
        (5000, MAX_LIST_LIMIT),
    ] {
        let evs = s
            .list(&Filter {
                limit,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(evs.len() as i64, want, "limit {limit}");
    }
}

#[test]
fn subscribe_order() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    let before = must_append(&s, DEVICE_STARTED, &());
    let sub = s.subscribe();
    const N: usize = 50;
    std::thread::scope(|sc| {
        for w in 0..5 {
            let s = &s;
            sc.spawn(move || {
                for i in 0..N / 5 {
                    s.append("irori", AGENT_STATUS, &[w, i]).unwrap();
                }
            });
        }
        s.insert(&ev("dev-b", 1, 1, AGENT_SEEN)).unwrap();
    });
    let mut got = Vec::new();
    while got.len() < N + 1 {
        let ev = sub.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        assert!(!ev.is_resync(), "unexpected resync");
        got.push(ev);
    }
    let mut last = before.seq;
    let mut saw_insert = false;
    for ev in &got {
        if ev.device_id == "dev-b" {
            saw_insert = true;
            continue;
        }
        assert_eq!(ev.seq, last + 1);
        last = ev.seq;
    }
    assert!(saw_insert, "inserted row not delivered");
    sub.cancel();
    while sub.recv().is_some() {}
}

#[test]
fn subscribe_resync_on_slow_reader() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    let sub = s.subscribe();
    let total = SUB_BUFFER + 40;
    for _ in 0..total {
        must_append(&s, AGENT_STATUS, &()); // must not block on the unread queue
    }
    let mut seqs = Vec::new();
    let mut resyncs = 0;
    while !sub.is_empty() {
        let ev = sub.recv().unwrap();
        if ev.is_resync() {
            resyncs += 1;
            continue;
        }
        seqs.push(ev.seq);
    }
    assert_eq!(resyncs, 1);
    assert_eq!(seqs.len(), SUB_BUFFER - 1);
    assert_eq!(*seqs.last().unwrap(), SUB_BUFFER as i64 - 1);
    let evs = s
        .list(&Filter {
            limit: MAX_LIST_LIMIT,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(evs.len(), total);
    let next = must_append(&s, AGENT_STATUS, &());
    assert_eq!(sub.recv().unwrap().seq, next.seq);
}

#[test]
fn subscribe_closes_on_cancel_and_close() {
    let dir = tempdir();
    let s = open_test(dir.path(), "dev-a");
    let sub = s.subscribe();
    sub.cancel();
    assert_eq!(sub.recv(), None, "open after cancel");
    let sub2 = s.subscribe();
    s.close().unwrap();
    assert_eq!(sub2.recv(), None, "open after close");
    assert_eq!(s.subscribe().try_recv(), TryRecv::Closed);
    assert_eq!(
        s.append("irori", AGENT_SEEN, &()).unwrap_err().kind(),
        ErrorKind::Closed
    );
    assert_eq!(
        s.insert(&ev("dev-b", 1, 0, AGENT_SEEN)).unwrap_err().kind(),
        ErrorKind::Closed
    );
}

#[test]
fn open_creates_parent_dir() {
    let dir = tempdir();
    let path = dir.path().join("nested").join("deeper").join("irori.db");
    let s = Store::open(&path, "dev-a", clock()).unwrap();
    s.close().unwrap();
    assert!(path.exists());
}

#[test]
fn migrations_recorded() {
    let dir = tempdir();
    let path = dir.path().join("irori.db");
    let s = open_test(dir.path(), "dev-a");
    s.close().unwrap();
    let c = rusqlite::Connection::open(&path).unwrap();
    let v: i64 = c
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(v, store::schema_version());
    c.execute(
        "INSERT INTO schema_migrations (version) VALUES (?1)",
        [store::schema_version() + 1],
    )
    .unwrap();
    drop(c);
    let err = Store::open(&path, "dev-a", clock()).unwrap_err();
    assert_eq!(
        err.to_string(),
        "store: database schema v4 is newer than this build (v3); update Irori"
    );
    let d2 = tempdir();
    let p2 = d2.path().join("irori.db");
    let s2 = Store::open(&p2, "dev-a", clock()).unwrap();
    let c2 = rusqlite::Connection::open(&p2).unwrap();
    let mode: String = c2
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    drop(s2);
}

#[test]
fn event_json_has_no_sig() {
    let e = Event {
        device_id: "d".into(),
        seq: 1,
        hlc: Timestamp(1 << 62 | 3),
        actor: "a".into(),
        kind: "k".into(),
        payload: br#"{"x":"<"}"#.to_vec(),
        sig: b"sig".to_vec(),
    };
    let raw = String::from_utf8(gocompat::json::marshal(&e).unwrap()).unwrap();
    assert_eq!(
        raw,
        // Go's HTML escaping applies inside the raw payload too.
        format!(
            r#"{{"deviceId":"d","seq":1,"hlc":4611686018427387907,"actor":"a","kind":"k","payload":{{"x":"{}u003c"}}}}"#,
            '\\'
        )
    );
    let zero = String::from_utf8(gocompat::json::marshal(&Event::default()).unwrap()).unwrap();
    assert_eq!(
        zero,
        r#"{"deviceId":"","seq":0,"hlc":0,"actor":"","kind":"","payload":null}"#
    );
}
