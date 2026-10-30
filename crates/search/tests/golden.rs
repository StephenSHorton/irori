//! Replays of testdata/golden/search/search.json
//! (tools/goldens/search_golden_test.go).

use gocompat::time::GoTime;
use search::{Hit, Kind, ShowOptions, Shown, page, parse_ref};
use serde_json::Value;

fn golden() -> Value {
    let p = manifest_dir().join("../../testdata/golden/search/search.json");
    serde_json::from_slice(&std::fs::read(p).expect("golden file")).expect("golden JSON")
}

#[test]
fn parse_ref_matches_go() {
    let g = golden();
    for c in g["refs"].as_array().unwrap() {
        let r = c["ref"].as_str().unwrap();
        let got = parse_ref(r);
        match c["err"].as_str().unwrap() {
            "" => {
                let (k, id) = got.unwrap_or_else(|e| panic!("ParseRef({r:?}) = {e}"));
                assert_eq!(k.as_str(), c["kind"].as_str().unwrap(), "{r:?}");
                assert_eq!(id, c["id"].as_str().unwrap(), "{r:?}");
            }
            e => assert_eq!(got.unwrap_err().to_string(), e, "{r:?}"),
        }
    }
}

#[test]
fn page_matches_go() {
    let g = golden();
    for c in g["pages"].as_array().unwrap() {
        let o = ShowOptions {
            offset: c["offset"].as_i64().unwrap(),
            limit: c["limit"].as_i64().unwrap(),
            viewer: String::new(),
        };
        let full = c["full"].as_str().unwrap();
        let (text, more, next) = page(full, &o);
        assert_eq!(text, c["text"].as_str().unwrap(), "{o:?}");
        assert_eq!(more, c["more"].as_bool().unwrap(), "{o:?}");
        assert_eq!(next, c["next"].as_i64().unwrap(), "{o:?}");
    }
}

#[test]
fn json_matches_go() {
    let g = golden();
    let want = |name: &str| -> String {
        g["json"]
            .as_array()
            .unwrap()
            .iter()
            .find(|j| j["name"] == name)
            .unwrap()["json"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let at = GoTime::parse_rfc3339("2026-10-29T08:07:06.5-06:00").unwrap();
    let hit = Hit {
        r#ref: "pr:o/n#1".into(),
        kind: Kind::Pr,
        at,
        title: "PR <o/n>#1 & co".into(),
        snippet: "[x] \u{2028}".into(),
        agent: "d/s".into(),
        repo_key: "remote:github.com/o/n".into(),
    };
    let shown = Shown {
        r#ref: "task:1".into(),
        kind: Kind::Task,
        title: "t".into(),
        text: "<b>".into(),
        more: true,
        next: 4000,
        related: vec!["comment:1".into(), "turn:t-2".into()],
    };
    let empty = Shown {
        r#ref: "msg:1".into(),
        kind: Kind::Msg,
        title: String::new(),
        text: String::new(),
        more: false,
        next: 0,
        related: vec![],
    };
    for (name, got) in [
        ("hitFull", gocompat::json::marshal(&hit).unwrap()),
        ("shownFull", gocompat::json::marshal(&shown).unwrap()),
        (
            "shownEmptyRelated",
            gocompat::json::marshal(&empty).unwrap(),
        ),
    ] {
        assert_eq!(String::from_utf8(got).unwrap(), want(name), "{name}");
    }
    // And back.
    let back: Hit = serde_json::from_str(&want("hitFull")).unwrap();
    assert_eq!(back, hit);
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
