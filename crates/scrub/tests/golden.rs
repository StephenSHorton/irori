//! Replays of the vectors the Go scrub package exported
//! (testdata/golden/scrub/, tools/goldens/scrub_golden_test.go).

use scrub::{Scrubber, go_patterns, skip_file, strict};
use serde_json::Value;

fn golden(rel: &str) -> Value {
    let p = manifest_dir().join("../../testdata/golden/scrub").join(rel);
    serde_json::from_slice(&std::fs::read(p).expect("golden file")).expect("golden JSON")
}

/// Every pattern text here is Go's, character for character.
#[test]
fn patterns_are_gos() {
    let g = golden("patterns.json");
    let ours = go_patterns();
    let gos: Vec<(&str, &str)> = g
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["name"].as_str().unwrap(), p["pattern"].as_str().unwrap()))
        .collect();
    assert_eq!(ours.len(), gos.len(), "pattern count");
    for (name, pat) in &gos {
        let mine = ours
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("no Rust pattern for {name}"));
        assert_eq!(&mine.1, pat, "{name}");
    }
}

#[test]
fn cases_match_go() {
    let g = golden("cases.json");
    let cases = g.as_array().unwrap();
    assert!(cases.len() > 100);
    for c in cases {
        let exact: Vec<&str> = c["exact"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let input = c["in"].as_str().unwrap();
        let s = Scrubber::new(&exact);
        assert_eq!(
            s.string(input),
            c["out"].as_str().unwrap(),
            "String({input:?}) with {} exact values",
            exact.len()
        );
    }
}

#[test]
fn skip_file_matches_go() {
    for c in golden("skipfile.json").as_array().unwrap() {
        let p = c["in"].as_str().unwrap();
        assert_eq!(
            skip_file(p),
            c["want"].as_bool().unwrap(),
            "SkipFile({p:?})"
        );
    }
}

#[test]
fn strict_matches_go() {
    for c in golden("strict.json").as_array().unwrap() {
        let p = c["in"].as_str().unwrap();
        assert_eq!(strict(p), c["out"].as_str().unwrap(), "Strict({p:?})");
    }
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
