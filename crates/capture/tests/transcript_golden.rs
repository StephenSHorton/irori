//! The transcript parser against what Go's parser returned for every fixture
//! of every Go transcript test (testdata/golden/capture/transcript.json,
//! exported by tools/goldens/export.sh with transcript_fixture_test.go and
//! transcript_golden_test.go).

use capture::transcript::{self, Hunk, Item, Parser, Source};
use serde_json::{Value as J, json};

fn golden(name: &str) -> J {
    let p = format!(
        "{}/../../testdata/golden/capture/{name}",
        manifest_dir().display()
    );
    serde_json::from_slice(&std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}"))).unwrap()
}

fn s(v: &J) -> &str {
    v.as_str().unwrap_or("")
}

/// A value marshaled the Go way, read back as JSON.
fn gj<T: serde::Serialize>(v: &T) -> J {
    serde_json::from_slice(&gocompat::json::marshal(v).unwrap()).unwrap()
}

fn opt_bytes(b: &Option<Vec<u8>>) -> J {
    match b {
        Some(b) => J::String(String::from_utf8(b.clone()).unwrap()),
        None => J::Null,
    }
}

/// One item in the shape transcript_golden_test.go's goldItem writes.
fn item_json(it: &Item) -> J {
    match it {
        Item::Turn(t) => json!({ "turn": gj(t) }),
        Item::Edit(e) => json!({
            "edit": gj(e),
            "before": opt_bytes(&e.before),
            "after": opt_bytes(&e.after),
            "patch": e.patch,
        }),
        Item::Summary(m) => json!({ "summary": gj(m), "full": m.full }),
        Item::PrLink(l) => json!({ "prLink": gj(l) }),
        Item::Bash(b) => json!({ "bash": gj(b) }),
    }
}

fn src_of(v: &J) -> Source {
    Source {
        session_id: s(&v["SessionID"]).into(),
        agent_id: s(&v["AgentID"]).into(),
    }
}

#[test]
fn every_go_fixture_parses_the_same() {
    let g = golden("transcript.json");
    let feeds = g["feeds"].as_array().unwrap();
    assert!(feeds.len() > 50, "{} feeds", feeds.len());
    for f in feeds {
        let test = s(&f["test"]);
        let mut p = Parser::new(src_of(&f["src"]));
        let mut items = Vec::new();
        for l in f["lines"].as_array().unwrap() {
            items.extend(p.line(s(l).as_bytes()));
        }
        let open = p.open().map(|t| gj(&t)).unwrap_or(J::Null);
        assert_eq!(open, f["open"], "{test}: open");
        assert_eq!(
            p.last_activity().format_rfc3339_nano(),
            s(&f["last"]),
            "{test}: last activity"
        );
        // TestFullSession ends with a Close; its items follow the lines'.
        if let Some(c) = f.get("close") {
            let at = gocompat::time::GoTime::parse_rfc3339(s(&c["at"])).unwrap();
            items.extend(p.close(at, s(&c["reason"])));
        }
        let got: Vec<J> = items.iter().map(item_json).collect();
        let want = f["items"].as_array().unwrap();
        assert_eq!(&got, want, "{test}: items");
    }
    for name in ["TestCRLFLine", "TestFullSession"] {
        assert!(
            feeds.iter().any(|f| s(&f["test"]) == name),
            "no {name} feed"
        );
    }
}

#[test]
fn ignored_lines_match_go() {
    let g = golden("transcript.json");
    let v = &g["vectors"]["ignored"];
    let mut p = Parser::new(Source {
        session_id: "11111111-2222-3333-4444-555555555555".into(),
        agent_id: String::new(),
    });
    for (l, want) in v["lines"]
        .as_array()
        .unwrap()
        .iter()
        .zip(v["items"].as_array().unwrap())
    {
        let got: Vec<J> = p.line(s(l).as_bytes()).iter().map(item_json).collect();
        let want = want.as_array().cloned().unwrap_or_default();
        assert_eq!(got, want, "line {l}");
    }
    assert_eq!(p.open().map(|t| gj(&t)).unwrap_or(J::Null), v["open"]);
    assert_eq!(p.last_activity().format_rfc3339_nano(), s(&v["last"]));
}

#[test]
fn source_for_matches_go() {
    let g = golden("transcript.json");
    for c in g["vectors"]["sourceFor"].as_array().unwrap() {
        let got = transcript::source_for(s(&c["Dir"]), s(&c["Path"]));
        let want = c["OK"].as_bool().unwrap().then(|| src_of(&c["Src"]));
        assert_eq!(got, want, "{c}");
    }
}

#[test]
fn skip_line_matches_go() {
    let g = golden("transcript.json");
    for c in g["vectors"]["skipLine"].as_array().unwrap() {
        assert_eq!(
            transcript::skip_line(s(&c["Line"]).as_bytes()),
            c["Skip"].as_bool().unwrap(),
            "{c}"
        );
    }
}

#[test]
fn target_matches_go() {
    let g = golden("transcript.json");
    for c in g["vectors"]["target"].as_array().unwrap() {
        let got = transcript::target(s(&c["Tool"]), Some(s(&c["Input"]).as_bytes()), s(&c["Cwd"]));
        assert_eq!(got, s(&c["Want"]), "{c}");
    }
}

#[test]
fn unified_diff_matches_go() {
    let g = golden("transcript.json");
    for c in g["vectors"]["unifiedDiff"].as_array().unwrap() {
        let hunks: Vec<Hunk> = c["Hunks"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|h| Hunk {
                        old_start: h["oldStart"].as_i64().unwrap(),
                        old_lines: h["oldLines"].as_i64().unwrap(),
                        new_start: h["newStart"].as_i64().unwrap(),
                        new_lines: h["newLines"].as_i64().unwrap(),
                        lines: h["lines"]
                            .as_array()
                            .map(|l| l.iter().map(|x| s(x).to_string()).collect())
                            .unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let (d, a, r) = transcript::unified_diff(s(&c["Path"]), &hunks);
        assert_eq!(d, s(&c["Diff"]), "{c}");
        assert_eq!(a, c["Adds"].as_i64().unwrap());
        assert_eq!(r, c["Dels"].as_i64().unwrap());
    }
}

#[test]
fn clip_matches_go() {
    let g = golden("transcript.json");
    for c in g["vectors"]["clip"].as_array().unwrap() {
        let n = c["N"].as_u64().unwrap() as usize;
        assert_eq!(transcript::clip(s(&c["S"]), n), s(&c["Clip"]), "{c}");
        assert_eq!(transcript::clip_bytes(s(&c["S"]), n), s(&c["Byte"]), "{c}");
    }
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
