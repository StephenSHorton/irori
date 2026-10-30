//! Replays of the vectors and scripts the Go store exported
//! (testdata/golden/store/, tools/goldens/store_golden_test.go): signing
//! bytes and RowHash, the kinds registry, the schema, the v3 migration, and
//! scripted private-log and board-log sessions whose every answer (error
//! text, applied count, stored row, signature, HLC, fork, refused row,
//! vector, observed row, board counts) must equal Go's.

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::*;
use gocompat::json::RawJson;
use serde_json::{Value, json};
use store::kinds::{self, REGISTRY};
use store::{
    Board, Clock, Cursor, CutClaim, Event, Filter, OpenOptions, Side, Signer, Store, Timestamp,
    Vector, Verifier, WallFn, row_hash, signing_bytes,
};

const GOLD_WALL_MS: u64 = 1_700_000_000_000;

fn gold_now() -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(GOLD_WALL_MS)
}

fn gold_clock() -> Arc<Clock> {
    Arc::new(Clock::with_now(Box::new(gold_now)))
}

#[test]
fn signing_bytes_and_row_hash_match_go() {
    let g = golden("store/signing.json");
    let vecs = g["vectors"].as_array().unwrap();
    assert!(vecs.len() >= 8);
    for v in vecs {
        let ev = g_event(&v["event"]);
        assert_eq!(
            hex::encode(signing_bytes(&ev)),
            v["signingBytes"].as_str().unwrap(),
            "{ev:?}"
        );
        assert_eq!(row_hash(&ev), v["event"]["rowHash"].as_str().unwrap());
    }
    // The signed ones verify with the golden key.
    let k = KeySigner::golden("dev-a");
    let ev = g_event(&vecs[0]["event"]);
    assert!(k.verifies(&ev));
    assert_eq!(
        k.sign(&signing_bytes(&ev)),
        ev.sig,
        "ed25519 is deterministic"
    );
}

fn side_name(s: Side) -> &'static str {
    match s {
        Side::Replicated => "replicated",
        Side::Local => "local",
        Side::Unclassified => "unclassified",
    }
}

fn board_name(b: Board) -> &'static str {
    match b {
        Board::Accepts => "accepts",
        Board::Refuses => "refuses",
        Board::Unclassified => "unclassified",
    }
}

/// Kinds added after the Go port (shared boards stage 2), which Go's
/// table can't list: board logs accept them.
const SINCE_GO: [&str; 3] = [
    kinds::BOARD_ASK_REQUESTED,
    kinds::BOARD_ASK_STATE,
    kinds::BOARD_ASK_EDITED,
];

#[test]
fn kinds_registry_matches_go() {
    let g = golden("store/kinds.json");
    for k in SINCE_GO {
        assert_eq!(store::kind_side(k), Side::Replicated, "{k}");
        assert!(store::board_accepted(k), "{k}");
    }
    let sides: BTreeMap<String, String> = REGISTRY
        .iter()
        .filter(|e| !SINCE_GO.contains(&e.kind))
        .map(|e| (e.kind.to_string(), side_name(e.side).to_string()))
        .collect();
    let want_sides: BTreeMap<String, String> = serde_json::from_value(g["sides"].clone()).unwrap();
    assert_eq!(sides, want_sides, "Side column");
    let boards: BTreeMap<String, String> = REGISTRY
        .iter()
        .filter(|e| e.board != Board::Unclassified && !SINCE_GO.contains(&e.kind))
        .map(|e| (e.kind.to_string(), board_name(e.board).to_string()))
        .collect();
    let want_boards: BTreeMap<String, String> =
        serde_json::from_value(g["boards"].clone()).unwrap();
    assert_eq!(boards, want_boards, "Board column");
    for p in g["probes"].as_array().unwrap() {
        let k = p["kind"].as_str().unwrap();
        assert_eq!(side_name(store::kind_side(k)), p["side"], "{k:?}");
        assert_eq!(board_name(store::kind_board(k)), p["board"], "{k:?}");
        assert_eq!(store::is_local(k), p["isLocal"], "{k:?}");
        assert_eq!(store::is_person_kind(k), p["isPerson"], "{k:?}");
        assert_eq!(store::board_accepted(k), p["boardAccepted"], "{k:?}");
        assert_eq!(
            store::is_board_identity_kind(k),
            p["isBoardIdentity"],
            "{k:?}"
        );
    }
    for a in g["actors"].as_array().unwrap() {
        let actor = a["actor"].as_str().unwrap();
        assert_eq!(store::board_actor_ok(actor), a["ok"], "{actor:?}");
    }
    assert_eq!(g["schemaVersion"], store::schema_version());
}

fn master(path: &Path) -> Value {
    let c = rusqlite::Connection::open(path).unwrap();
    let mut st = c
        .prepare(
            "SELECT type, name, tbl_name, COALESCE(sql, '') FROM sqlite_master ORDER BY type, name",
        )
        .unwrap();
    let mut out: Vec<Value> = st
        .query_map([], |r| {
            let (ty, name, tbl, q): (String, String, String, String) =
                (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
            Ok(json!({"type": ty, "name": name, "tblName": tbl, "sql": q}))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let mut vs = c
        .prepare("SELECT version FROM schema_migrations ORDER BY version")
        .unwrap();
    let versions: Vec<String> = vs
        .query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(|v| v.unwrap().to_string())
        .collect();
    out.push(json!({"type": "versions", "name": format!("[{}]", versions.join(" "))}));
    Value::Array(out)
}

/// Go's sqlite_master rows omit empty sql only through COALESCE; compare
/// field by field so a missing "sql" key reads as "".
fn norm(v: &Value) -> Vec<BTreeMap<String, String>> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|o| {
            ["type", "name", "tblName", "sql"]
                .iter()
                .map(|k| (k.to_string(), o[*k].as_str().unwrap_or("").to_string()))
                .collect()
        })
        .collect()
}

#[test]
fn schema_matches_go() {
    let g = golden("store/schema.json");
    assert_eq!(V1_SQL, g["v1"].as_str().unwrap());
    let d = tempdir();
    let fresh = d.path().join("fresh.db");
    Store::open(&fresh, "dev-a", gold_clock())
        .unwrap()
        .close()
        .unwrap();
    assert_eq!(norm(&master(&fresh)), norm(&g["fresh"]), "fresh database");
    let migrated = d.path().join("migrated.db");
    v03_db(&migrated, "dev-a");
    Store::open_with(
        &migrated,
        "dev-a",
        gold_clock(),
        opts(Some(KeySigner::golden("dev-a")), None),
    )
    .unwrap()
    .close()
    .unwrap();
    assert_eq!(
        norm(&master(&migrated)),
        norm(&g["migrated"]),
        "migrated v0.3 database"
    );
}

#[test]
fn v3_migration_matches_go() {
    let g = golden("store/migration.json");
    let d = tempdir();
    let path = d.path().join("irori.db");
    let orig = v03_db(&path, "dev-a");
    assert_eq!(to_gs(&orig), g["v1Rows"], "the v0.3 fixture");
    let s = Store::open_with(
        &path,
        "dev-a",
        gold_clock(),
        opts(Some(KeySigner::golden("dev-a")), None),
    )
    .unwrap();
    assert_eq!(
        to_gs(&all(&s)),
        g["events"],
        "migrated rows (signatures included)"
    );
    let loc = s
        .list(&Filter {
            local: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(to_gs(&loc), g["local"], "local_events");
    let p: BTreeMap<&str, &str> = [("key", "dev-a/s1"), ("status", "idle")].into();
    let next = s.append("irori", kinds::AGENT_STATUS, &p).unwrap();
    assert_eq!(to_g(&next), g["next"], "first append after the migration");
}

// ------------------------------------------------------------ script replay

struct Replay {
    dir: tempfile::TempDir,
    rel: &'static str,
    board: bool,
    ver: Arc<FakeVerifier>,
    s: Option<Store>,
}

impl Replay {
    fn new(rel: &'static str, board: bool) -> Replay {
        let pubs: Vec<(&str, _)> = ["dev-a", "dev-b", "dev-c"]
            .iter()
            .map(|d| (*d, KeySigner::golden(d).public()))
            .collect();
        let mut r = Replay {
            dir: tempdir(),
            rel,
            board,
            ver: FakeVerifier::new(&pubs),
            s: None,
        };
        r.open();
        r
    }

    fn open(&mut self) {
        let o = OpenOptions {
            signer: Some(Arc::new(KeySigner::golden("dev-a")) as Arc<dyn Signer>),
            verifier: Some(self.ver.clone() as Arc<dyn Verifier>),
            now: Some(Arc::new(gold_now) as WallFn),
            board: self.board,
            ..OpenOptions::default()
        };
        self.s = Some(
            Store::open_with(self.dir.path().join(self.rel), "dev-a", gold_clock(), o).unwrap(),
        );
    }

    fn s(&self) -> &Store {
        self.s.as_ref().unwrap()
    }
}

fn err_text<T>(r: &store::Result<T>) -> String {
    match r {
        Ok(_) => String::new(),
        Err(e) => e.to_string(),
    }
}

fn rows_of(op: &Value) -> Value {
    op.get("rows").cloned().unwrap_or(json!([]))
}

fn filter_of(f: &Value) -> Filter {
    let strs = |v: &Value| -> Vec<String> {
        v.as_array()
            .map(|a| a.iter().map(|s| s.as_str().unwrap().to_string()).collect())
            .unwrap_or_default()
    };
    Filter {
        kinds: strs(&f["kinds"]),
        device_id: f["deviceId"].as_str().unwrap_or("").into(),
        after_hlc: Timestamp(f["afterHlc"].as_u64().unwrap_or(0)),
        after: f
            .get("after")
            .map(|c| Cursor {
                hlc: Timestamp(c["hlc"].as_u64().unwrap()),
                device_id: c["deviceId"].as_str().unwrap().into(),
                seq: c["seq"].as_i64().unwrap(),
            })
            .unwrap_or_default(),
        after_seq: f["afterSeq"].as_i64().unwrap_or(0),
        limit: f["limit"].as_i64().unwrap_or(0),
        desc: f["desc"].as_bool().unwrap_or(false),
        local: f["local"].as_bool().unwrap_or(false),
    }
}

fn vector_of(v: Option<&Value>) -> Vector {
    v.map(|v| serde_json::from_value(v.clone()).unwrap())
        .unwrap_or_default()
}

fn replay(rel_golden: &str, rel_db: &'static str, board: bool) -> usize {
    let g = golden(rel_golden);
    assert_eq!(g["self"], "dev-a");
    let mut r = Replay::new(rel_db, board);
    let ops = g["ops"].as_array().unwrap();
    for (i, op) in ops.iter().enumerate() {
        let name = op["op"].as_str().unwrap();
        let at = format!("{rel_golden} op {i} ({name})");
        let s = r.s();
        match name {
            "append" | "appendLocal" => {
                let actor = op["actor"].as_str().unwrap_or("");
                let kind = op["kind"].as_str().unwrap_or("");
                let payload = RawJson(op["payload"].as_str().unwrap_or("").as_bytes().to_vec());
                let res = if name == "append" {
                    s.append(actor, kind, &payload)
                } else {
                    s.append_local(actor, kind, &payload)
                };
                assert_eq!(err_text(&res), op["err"].as_str().unwrap(), "{at}");
                if let Ok(ev) = &res {
                    assert_eq!(&to_g(ev), &op["out"], "{at}: appended row");
                }
            }
            "insert" => {
                let evs: Vec<Event> = op["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(g_event)
                    .collect();
                let (n, res) = s.insert_batch(&evs);
                assert_eq!(err_text(&res), op["err"].as_str().unwrap(), "{at}");
                assert_eq!(n as i64, op["applied"].as_i64().unwrap(), "{at}: applied");
            }
            "unresolved" => {
                let ev = g_event(&op["events"][0]);
                assert_eq!(
                    err_text(&s.unresolved(&ev)),
                    op["err"].as_str().unwrap(),
                    "{at}"
                );
            }
            "list" => {
                let res = s.list(&filter_of(&op["filter"]));
                assert_eq!(err_text(&res), op["err"].as_str().unwrap(), "{at}");
                assert_eq!(to_gs(&res.unwrap()), rows_of(op), "{at}: rows");
            }
            "missing" => {
                let v = vector_of(op.get("vector"));
                let evs = s.missing(&v, op["limit"].as_i64().unwrap_or(0)).unwrap();
                assert_eq!(to_gs(&evs), rows_of(op), "{at}: rows");
            }
            "vector" => {
                assert_eq!(s.vector().unwrap(), vector_of(op.get("got")), "{at}");
            }
            "refused" => {
                assert_eq!(to_gs(&s.refused(0).unwrap()), rows_of(op), "{at}");
            }
            "forks" => {
                let got: Vec<Value> = s
                    .forks()
                    .unwrap()
                    .iter()
                    .map(|f| {
                        json!({"stored": to_g(&f.stored), "other": to_g(&f.other),
                               "otherHash": f.other_hash, "reason": f.reason})
                    })
                    .collect();
                let want = op.get("forks").cloned().unwrap_or(json!([]));
                assert_eq!(Value::Array(got), want, "{at}");
            }
            "observed" => {
                let got = r.ver.st.lock().unwrap().observed.clone();
                let want: Vec<String> = op
                    .get("observed")
                    .map(|v| serde_json::from_value(v.clone()).unwrap())
                    .unwrap_or_default();
                assert_eq!(got, want, "{at}");
            }
            "reopen" => {
                s.close().unwrap();
                r.s = None;
                Verifier::reset(&*r.ver);
                r.open();
            }
            "setCut" => {
                let mut st = r.ver.st.lock().unwrap();
                st.cuts.insert(
                    op["dev"].as_str().unwrap().into(),
                    op["seq"].as_i64().unwrap(),
                );
            }
            "claim" => {
                r.ver.st.lock().unwrap().claims.push(CutClaim {
                    device_id: op["dev"].as_str().unwrap().into(),
                    seq: op["seq"].as_i64().unwrap(),
                    hash: op["hash"].as_str().unwrap().into(),
                });
            }
            "hold" => s.hold_appends(op["seq"].as_i64().unwrap_or(0)),
            other => panic!("{at}: unknown op {other}"),
        }
        if let Some(c) = op.get("counts") {
            let got = serde_json::to_value(r.s().board_refused()).unwrap();
            assert_eq!(&got, c, "{at}: board counts");
        }
    }
    if let Some(resets) = g.get("resets") {
        assert_eq!(
            r.ver.st.lock().unwrap().resets as i64,
            resets.as_i64().unwrap(),
            "Verifier resets"
        );
    }
    ops.len()
}

#[test]
fn private_log_script_matches_go() {
    let n = replay("store/private.json", "irori.db", false);
    assert!(n > 80, "only {n} ops");
}

#[test]
fn board_log_script_matches_go() {
    let n = replay(
        "store/board.json",
        "boards/0123456789abcdef0123456789abcdef.db",
        true,
    );
    assert!(n > 150, "only {n} ops");
}
