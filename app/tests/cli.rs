//! The command line's dispatch (I:main.go): Go's unknown-command text and
//! exit code, and `irori events` with no Core.

use std::process::{Command, Stdio};

fn run(args: &[&str], home: &std::path::Path) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_irori"))
        .args(args)
        .env("IRORI_HOME", home)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn unknown_command_like_go() {
    let home = tempfile::tempdir().unwrap();
    let (code, out, err) = run(&["bogus"], home.path());
    assert_eq!(code, 1);
    assert!(out.is_empty());
    assert_eq!(
        err.trim_end(),
        "irori: unknown command \"bogus\" (gui, serve, mcp, attach, events, pair, node, version)"
    );
}

#[test]
fn events_without_a_core() {
    let home = tempfile::tempdir().unwrap();
    let (code, out, err) = run(&["events"], home.path());
    assert_eq!((code, out.as_str()), (1, ""));
    assert!(err.starts_with("irori: irori is not running"), "{err}");

    let (code, _, err) = run(&["events", "extra"], home.path());
    assert_eq!(code, 1);
    assert_eq!(
        err.trim_end(),
        "irori: events: unexpected argument \"extra\""
    );

    let (code, _, err) = run(&["events", "--bogus"], home.path());
    assert_eq!(code, 1);
    assert!(
        err.starts_with("flag provided but not defined: -bogus\nUsage of events:\n  -follow\n")
            && err
                .trim_end()
                .ends_with("irori: flag provided but not defined: -bogus"),
        "{err}"
    );
}

/// Go setupDiagnostics: every role but version opens crash.log; the long
/// running ones (gui, serve) also write the start line there and to
/// irori.log. A bad flag ends serve before any Core starts.
#[test]
fn diagnostics_files_per_role() {
    let home = tempfile::tempdir().unwrap();
    let (code, _, _) = run(&["version"], home.path());
    assert_eq!(code, 0);
    assert!(
        !home.path().join("crash.log").exists(),
        "version writes nothing"
    );

    run(&["events", "extra"], home.path());
    assert_eq!(std::fs::read(home.path().join("crash.log")).unwrap(), b"");
    assert!(!home.path().join("irori.log").exists());

    let (code, _, _) = run(&["serve", "--bogus"], home.path());
    assert_eq!(code, 1);
    for f in ["crash.log", "irori.log"] {
        let s = std::fs::read_to_string(home.path().join(f)).unwrap();
        assert!(
            s.contains(" start: role=serve pid=") && s.ends_with('\n'),
            "{f}: {s:?}"
        );
    }
}
