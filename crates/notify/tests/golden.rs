//! testdata/golden/notify/settings.json (tools/goldens/notify_golden_test.go):
//! what Go's load reads from a settings.json and the exact bytes its save
//! writes, replayed on the Rust side.

use notify::{Options, SETTINGS_FILE};
use serde_json::Value;

fn options(v: &Value) -> Options {
    Options {
        enabled: v["enabled"].as_bool().unwrap(),
        sound: v["sound"].as_bool().unwrap(),
        volume: v["volume"].as_f64().unwrap(),
        corner: v["corner"].as_str().unwrap().into(),
    }
}

#[test]
fn settings_match_go() {
    let path = manifest_dir().join("../../testdata/golden/notify/settings.json");
    let cases: Vec<Value> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(cases.len() >= 10);
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(SETTINGS_FILE);
        if let Some(b) = c["before"].as_str() {
            std::fs::write(&file, b).unwrap();
        }
        assert_eq!(
            notify::load(dir.path()),
            options(&c["loaded"]),
            "{name}: load"
        );
        if c["save"].is_object() {
            notify::save(dir.path(), &options(&c["save"])).unwrap();
            let got = std::fs::read_to_string(&file).unwrap();
            assert_eq!(got, c["after"].as_str().unwrap(), "{name}: save bytes");
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
