//! The Go-exported shell golden (testdata/golden/shell/shell.json, written
//! by tools/goldens/shell_golden_test.go in Go's root package).

use serde_json::Value;

use super::{instance, login, tray_text};

fn golden() -> Value {
    let p = manifest_dir().join("../testdata/golden/shell/shell.json");
    serde_json::from_str(&std::fs::read_to_string(p).expect("read shell.json")).unwrap()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

#[test]
fn instance_ids_match_go() {
    let g = golden();
    let rows = g["instanceIDs"].as_array().unwrap();
    assert!(rows.len() >= 8);
    for r in rows {
        let input = s(&r["in"]);
        // The golden is Go's answers on Windows; the unset row holds
        // everywhere, the paths only under Windows' Clean.
        if !cfg!(windows) && !input.is_empty() {
            continue;
        }
        assert_eq!(
            instance::instance_id(Some(input)),
            s(&r["out"]),
            "{input:?}"
        );
    }
}

#[test]
fn tray_texts_match_go() {
    let g = golden();
    for r in g["statusLines"].as_array().unwrap() {
        let c = &r["counts"];
        let counts = tray_text::Counts {
            unread: c["unread"].as_u64().unwrap() as usize,
            held: c["held"].as_u64().unwrap() as usize,
            paused: c["paused"].as_bool().unwrap(),
        };
        let got = tray_text::status_line(
            s(&r["status"]),
            counts,
            r["decide"].as_u64().unwrap() as usize,
        );
        assert_eq!(got, s(&r["out"]));
    }
    for r in g["heldLines"].as_array().unwrap() {
        let n = r["in"].as_u64().unwrap() as usize;
        assert_eq!(tray_text::held_line(n), s(&r["out"]));
    }
    for r in g["pauseLabels"].as_array().unwrap() {
        assert_eq!(tray_text::pause_label(s(&r["in"])), s(&r["out"]));
    }
    for r in g["versionLines"].as_array().unwrap() {
        assert_eq!(tray_text::version_line(s(&r["in"])), s(&r["out"]));
    }
}

#[test]
fn role_tables_match_go() {
    let g = golden();
    for r in g["startedAtLogin"].as_array().unwrap() {
        let args: Vec<String> = r["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| s(a).to_string())
            .collect();
        assert_eq!(
            login::started_at_login(&args),
            r["out"].as_bool().unwrap(),
            "{args:?}"
        );
    }
    for r in g["wantsConsole"].as_array().unwrap() {
        let cmd = s(&r["cmd"]);
        assert_eq!(
            crate::cli::wants_console(cmd),
            r["out"].as_bool().unwrap(),
            "{cmd:?}"
        );
    }
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
