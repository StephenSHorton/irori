//! Replays of testdata/golden/presence/presence.json
//! (tools/goldens/presence_golden_test.go).

mod common;

use std::io::Cursor;
use std::sync::Arc;

use common::testdata;
use gocompat::time::GoTime;
use presence::internal::{
    Session, display_name, is_session_file, one_line, parse_lstart, read_session, read_tail,
    scan_mode,
};
use presence::{AgentCard, Change, ChangeKind, Options, SOURCE_IRORI, Watcher, local, slug};
use serde_json::Value;

fn golden() -> Value {
    let p = manifest_dir().join("../../testdata/golden/presence/presence.json");
    serde_json::from_slice(&std::fs::read(p).expect("golden file")).expect("golden JSON")
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap()
}

/// A Go `[]byte` field (base64, null for empty).
fn b64(v: &Value) -> Vec<u8> {
    use base64::Engine;
    match v {
        Value::Null => vec![],
        v => base64::engine::general_purpose::STANDARD
            .decode(v.as_str().unwrap())
            .unwrap(),
    }
}

fn utc(t: &GoTime) -> String {
    if t.is_zero() {
        String::new()
    } else {
        t.utc().format_rfc3339_nano()
    }
}

#[test]
fn read_session_matches_go() {
    for c in golden()["sessions"].as_array().unwrap() {
        let raw = b64(&c["in"]);
        let input = String::from_utf8_lossy(&raw).into_owned();
        let got = read_session(&raw[..]);
        assert_eq!(
            got.is_some(),
            c["ok"].as_bool().unwrap(),
            "readSession({input:?})"
        );
        let Some(g) = got else { continue };
        let feats: Vec<String> = match &c["peerFeatures"] {
            Value::Null => vec![],
            v => serde_json::from_value(v.clone()).unwrap(),
        };
        assert_eq!(g.pid, c["pid"].as_i64().unwrap(), "{input:?}");
        assert_eq!(g.id, s(c, "id"), "{input:?}");
        assert_eq!(g.cwd, s(c, "cwd"), "{input:?}");
        assert_eq!(g.name, s(c, "name"), "{input:?}");
        assert_eq!(g.name_source, s(c, "nameSource"), "{input:?}");
        assert_eq!(g.status, s(c, "status"), "{input:?}");
        assert_eq!(g.kind, s(c, "kind"), "{input:?}");
        assert_eq!(g.entrypoint, s(c, "entrypoint"), "{input:?}");
        assert_eq!(g.version, s(c, "version"), "{input:?}");
        assert_eq!(g.peer_features, feats, "{input:?}");
        assert_eq!(g.proc_start, c["procStart"].as_u64().unwrap(), "{input:?}");
        assert_eq!(utc(&g.started_at), s(c, "startedAt"), "{input:?}");
        assert_eq!(utc(&g.updated_at), s(c, "updatedAt"), "{input:?}");
    }
}

#[test]
fn read_tail_and_scan_mode_match_go() {
    for c in golden()["tails"].as_array().unwrap() {
        let raw = b64(&c["src"]);
        let src = &raw[..];
        let n = c["n"].as_u64().unwrap();
        let size = src.len() as u64;
        let t = read_tail(&mut Cursor::new(src), size, n);
        let what = format!("readTail({:?}, n={n})", String::from_utf8_lossy(src));
        assert_eq!(t.title, s(c, "title"), "{what}");
        assert_eq!(t.custom, s(c, "custom"), "{what}");
        assert_eq!(t.ai, s(c, "ai"), "{what}");
        assert_eq!(t.last_text, s(c, "lastText"), "{what}");
        assert_eq!(utc(&t.last_activity), s(c, "lastActivity"), "{what}");
        assert_eq!(t.permission_mode, s(c, "permissionMode"), "{what}");
        assert_eq!(
            scan_mode(&mut Cursor::new(src), size, n),
            s(c, "mode"),
            "scanMode {what}"
        );
    }
}

#[test]
fn small_functions_match_go() {
    let g = golden();
    for c in g["lstart"].as_array().unwrap() {
        let input = s(c, "in");
        let got = parse_lstart(input).map_or("!".to_string(), |t| t.format_rfc3339_nano());
        assert_eq!(got, s(c, "out"), "parseLstart({input:?})");
    }
    for c in g["slug"].as_array().unwrap() {
        assert_eq!(slug(s(c, "in")), s(c, "out"));
    }
    for c in g["oneLine"].as_array().unwrap() {
        assert_eq!(one_line(s(c, "in"), 240), s(c, "out"), "{:?}", s(c, "in"));
    }
    for c in g["displayName"].as_array().unwrap() {
        let v: Value = serde_json::from_str(s(c, "in")).unwrap();
        let sess = Session {
            id: s(&v, "id").into(),
            cwd: s(&v, "cwd").into(),
            name: s(&v, "name").into(),
            ..Session::default()
        };
        assert_eq!(display_name(&sess, ""), s(c, "out"), "{v}");
    }
    for c in g["isSessionFile"].as_array().unwrap() {
        assert_eq!(
            is_session_file(s(c, "in")).to_string(),
            s(c, "out"),
            "{:?}",
            s(c, "in")
        );
    }
}

#[test]
fn json_matches_go() {
    let g = golden();
    let want = |name: &str| -> String {
        let j = g["json"].as_array().unwrap();
        s(j.iter().find(|j| j["in"] == name).unwrap(), "out").to_string()
    };
    let m = |v: &dyn erased::Ser| v.go_json();
    // A zero Go card has a nil peerFeatures (null); every card Go and Rust
    // build has a non-nil one ([] or more), and Rust's Vec always writes [].
    assert_eq!(
        m(&AgentCard::default()),
        want("cardZero").replace(r#""peerFeatures":null"#, r#""peerFeatures":[]"#)
    );
    // Go's zero Change has kind ""; a Rust Change always has a kind.
    assert_eq!(
        m(&Change::default()),
        want("changeZero")
            .replace(r#""peerFeatures":null"#, r#""peerFeatures":[]"#)
            .replace(r#""kind":"""#, r#""kind":"added""#)
    );
    let at = GoTime::parse_rfc3339("2026-10-29T08:07:06.5Z").unwrap();
    let full = AgentCard {
        key: "d/s".into(),
        device_id: "d".into(),
        agent: "claude".into(),
        session_id: "s".into(),
        pid: 42,
        name: "<n>&".into(),
        title: "t\u{2028}".into(),
        name_source: SOURCE_IRORI.into(),
        ai_title: "ai".into(),
        cwd: r"C:\x".into(),
        status: "busy".into(),
        session_kind: "bg".into(),
        entrypoint: "cli".into(),
        version: "2.1.310".into(),
        peer_features: vec!["a".into(), "b".into()],
        started_at: at,
        updated_at: GoTime::unix(at.secs + 1, i64::from(at.nanos)),
        last_activity: GoTime::unix(at.secs, i64::from(at.nanos) + 1_000_000),
        last_text: "last".into(),
        transcript: r"C:\t.jsonl".into(),
        repo_key: "remote:x/y".into(),
        permission_mode: "bypassPermissions".into(),
        device_state: "recent".into(),
    };
    assert_eq!(m(&full), want("cardFull"));
    // And back, as another device's row is read.
    let back: AgentCard = serde_json::from_str(&want("cardFull")).unwrap();
    assert_eq!(back, full);
    let removed = Change {
        kind: ChangeKind::Removed,
        card: AgentCard {
            key: "d/s".into(),
            ..AgentCard::default()
        },
    };
    assert_eq!(m(&removed), want("changeRemoved"));

    // The fixture cards, scanned with every pid live.
    let w = Watcher::new(Options {
        claude_dir: gocompat::filepath::abs(&testdata().to_string_lossy()).unwrap(),
        device_id: "dev1".into(),
        live: Some(Arc::new(|_, _| true)),
        ..Options::default()
    });
    w.rescan();
    let root = gocompat::filepath::abs(&testdata().to_string_lossy()).unwrap();
    let cards = w.snapshot();
    assert_eq!(cards.len(), 2);
    for (i, mut c) in cards.into_iter().enumerate() {
        c.started_at = c.started_at.utc();
        c.updated_at = c.updated_at.utc();
        c.last_activity = c.last_activity.utc();
        c.transcript = c.transcript.replace(&root, "$TESTDATA");
        if !cfg!(windows) {
            // Go exported the fixtures on Windows, with \ separators.
            c.transcript = c.transcript.replace('/', "\\");
        }
        assert_eq!(m(&c), want(&format!("fixture{i}")), "fixture{i}");
    }
}

/// Go's local offsets, checked when this machine is in the zone the golden
/// was exported in (the reference instant's offset agrees).
#[test]
fn local_offsets_match_go() {
    let g = golden();
    let offs = g["local"].as_array().unwrap();
    let at = |c: &Value| GoTime::unix(c["unix"].as_i64().unwrap(), 0);
    let first = &offs[0];
    if local(at(first)).offset != first["offset"].as_i64().unwrap() as i32 {
        eprintln!("not the export's time zone; skipped");
        return;
    }
    for c in offs {
        assert_eq!(
            local(at(c)).offset,
            c["offset"].as_i64().unwrap() as i32,
            "offset at {}",
            c["unix"]
        );
    }
}

mod erased {
    pub trait Ser {
        fn go_json(&self) -> String;
    }
    impl<T: serde::Serialize> Ser for T {
        fn go_json(&self) -> String {
            String::from_utf8(gocompat::json::marshal(self).unwrap()).unwrap()
        }
    }
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
