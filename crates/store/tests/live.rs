//! The differential check on real data (architecture.md §6.3, "Rows"):
//! the Rust store opens a scratch copy of a real `irori.db` and every row's
//! RowHash must equal the one Go computed from another copy of the same
//! file (tools/goldens/store_golden_test.go `TestDiffLiveRowHashes`).
//!
//! Ignored by default: it needs a copy of private data, made as
//! tools/goldens/diff-live.sh does. It prints counts and the
//! `(table, device, seq)` of any mismatch, never a row's contents.
//!
//!   IRORI_DIFF_DB=<copy of irori.db, not the one Go read>
//!   IRORI_DIFF_GO=<Go's hash list>
//!   cargo test -p irori-store --test live -- --ignored --nocapture

mod common;

use std::collections::BTreeMap;

use store::{Cursor, Event, Filter, MAX_LIST_LIMIT, Store, row_hash};

/// Every row of `events` or `local_events` through the store's own paging.
fn page_all(s: &Store, local: bool) -> Vec<Event> {
    let mut out = Vec::new();
    let mut after = Cursor::default();
    loop {
        let evs = s
            .list(&Filter {
                after: after.clone(),
                limit: MAX_LIST_LIMIT,
                local,
                ..Default::default()
            })
            .unwrap();
        out.extend(evs.iter().cloned());
        if (evs.len() as i64) < MAX_LIST_LIMIT {
            return out;
        }
        after = evs[evs.len() - 1].cursor();
    }
}

#[test]
#[ignore = "needs a scratch copy of a real irori.db (IRORI_DIFF_DB, IRORI_DIFF_GO)"]
fn live_row_hashes_match_go() {
    let (Ok(db), Ok(go)) = (
        std::env::var("IRORI_DIFF_DB"),
        std::env::var("IRORI_DIFF_GO"),
    ) else {
        panic!("set IRORI_DIFF_DB and IRORI_DIFF_GO");
    };
    let mut want: BTreeMap<String, String> = BTreeMap::new();
    for line in std::fs::read_to_string(&go).unwrap().lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let (key, hash) = match f.as_slice() {
            [t, d, s, h] => (format!("{t}\t{d}\t{s}"), *h),
            [t, d, s, h, other] => (format!("{t}\t{d}\t{s}\t{other}"), *h),
            _ => panic!("bad line in the Go hash list"),
        };
        want.insert(key, hash.to_string());
    }

    // Any device id reads the log; without a Signer or Verifier the open
    // checks and moves nothing, and the schema is already v3.
    let s = Store::open(&db, "irori-rust-diff", common::clock()).unwrap();
    let mut got: BTreeMap<String, String> = BTreeMap::new();
    for (table, evs) in [
        ("events", page_all(&s, false)),
        ("local_events", page_all(&s, true)),
    ] {
        for ev in evs {
            got.insert(
                format!("{table}\t{}\t{}", ev.device_id, ev.seq),
                row_hash(&ev),
            );
        }
    }
    s.close().unwrap();
    // refused_events and forks: the API caps refused() at 1000 and forks()
    // hides nothing, but read both raw to cover every row.
    let c = rusqlite::Connection::open(&db).unwrap();
    let mut st = c
        .prepare("SELECT device_id, seq, hlc, actor, kind, payload, sig FROM refused_events")
        .unwrap();
    let ev_of = |r: &rusqlite::Row<'_>| -> rusqlite::Result<Event> {
        let b = |i: usize| -> rusqlite::Result<Vec<u8>> {
            Ok(r.get_ref(i)?
                .as_bytes_or_null()?
                .unwrap_or_default()
                .to_vec())
        };
        Ok(Event {
            device_id: r.get(0)?,
            seq: r.get(1)?,
            hlc: store::Timestamp::from_i64(r.get(2)?),
            actor: r.get(3)?,
            kind: r.get(4)?,
            payload: b(5)?,
            sig: b(6)?,
        })
    };
    for ev in st.query_map([], ev_of).unwrap() {
        let ev = ev.unwrap();
        got.insert(
            format!("refused_events\t{}\t{}", ev.device_id, ev.seq),
            row_hash(&ev),
        );
    }
    let mut st = c
        .prepare("SELECT device_id, seq, hlc, actor, kind, payload, sig, other_hash FROM forks")
        .unwrap();
    let rows = st
        .query_map([], |r| Ok((ev_of(r)?, r.get::<_, String>(7)?)))
        .unwrap();
    for row in rows {
        let (ev, other) = row.unwrap();
        got.insert(
            format!("forks\t{}\t{}\t{other}", ev.device_id, ev.seq),
            row_hash(&ev),
        );
    }

    let mut per_table: BTreeMap<&str, usize> = BTreeMap::new();
    for k in got.keys() {
        *per_table.entry(k.split('\t').next().unwrap()).or_default() += 1;
    }
    let mut mismatched = Vec::new();
    for (k, h) in &want {
        match got.get(k) {
            Some(g) if g == h => {}
            Some(_) => mismatched.push(format!("hash differs: {}", k.replace('\t', " "))),
            None => mismatched.push(format!("missing in Rust: {}", k.replace('\t', " "))),
        }
    }
    for k in got.keys() {
        if !want.contains_key(k) {
            mismatched.push(format!("missing in Go: {}", k.replace('\t', " ")));
        }
    }
    println!(
        "rows: Go {}, Rust {} {per_table:?}; mismatches: {}",
        want.len(),
        got.len(),
        mismatched.len()
    );
    for m in mismatched.iter().take(20) {
        println!("  {m}");
    }
    assert!(!want.is_empty(), "the Go hash list is empty");
    assert!(mismatched.is_empty(), "{} rows differ", mismatched.len());
}

/// The Rust half of "Go still opens a copy Rust wrote" (architecture.md §8
/// M2): append and insert on the Rust copy, then list every events row's
/// RowHash for Go's `TestDiffOpenRustDB`, which opens the same copy.
///
///   IRORI_DIFF_DB=<the Rust copy> IRORI_DIFF_MANIFEST=<out>
#[test]
#[ignore = "needs a scratch copy of a real irori.db (IRORI_DIFF_DB, IRORI_DIFF_MANIFEST)"]
fn live_rust_writes_for_go() {
    let (Ok(db), Ok(manifest)) = (
        std::env::var("IRORI_DIFF_DB"),
        std::env::var("IRORI_DIFF_MANIFEST"),
    ) else {
        panic!("set IRORI_DIFF_DB and IRORI_DIFF_MANIFEST");
    };
    let s = Store::open(&db, "irori-rust-diff", common::clock()).unwrap();
    let before = page_all(&s, false).len();
    let p: BTreeMap<&str, &str> = [("by", "rust")].into();
    s.append("irori", store::kinds::AGENT_SEEN, &p).unwrap();
    // A peer row with bytes Rust's own encoder would never write.
    let mut payload = br#"{"a":"<"#.to_vec();
    payload.extend_from_slice("\u{2028}".as_bytes());
    payload.extend_from_slice(br#">", "b" : 1.50}"#);
    let peer = Event {
        device_id: "irori-rust-peer".into(),
        seq: 1,
        hlc: common::clock().now(),
        actor: "irori".into(),
        kind: "future.kind".into(),
        payload,
        sig: b"not-a-real-sig".to_vec(),
    };
    s.insert(&peer).unwrap();
    let rows = page_all(&s, false);
    assert_eq!(rows.len(), before + 2);
    let mut out = String::new();
    for ev in &rows {
        out.push_str(&format!(
            "events\t{}\t{}\t{}\n",
            ev.device_id,
            ev.seq,
            row_hash(ev)
        ));
    }
    std::fs::write(&manifest, out).unwrap();
    s.close().unwrap();
    println!("rust wrote 2 rows; {} rows listed for Go", rows.len());
}
