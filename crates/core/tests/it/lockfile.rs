//! Port of I:internal/discovery/discovery_test.go and rename_windows_test.go,
//! plus the exact bytes Go's MarshalIndent writes.

use std::path::Path;
use std::time::Duration;

use gocompat::time::GoTime;
use irori_core::lockfile::{self, Error, FILE_NAME, Lockfile};

fn sample(pid: i64) -> Lockfile {
    Lockfile {
        pid,
        port: 4321,
        token: "abc".into(),
        version: "0.1.0".into(),
        device_id: "dev1".into(),
        headless: false,
        started_at: GoTime::parse_rfc3339("2026-01-02T03:04:05Z").unwrap(),
    }
}

fn me() -> i64 {
    lockfile::own_pid()
}

#[test]
fn test_new_token() {
    let a = lockfile::new_token().unwrap();
    let b = lockfile::new_token().unwrap();
    let raw = hex::decode(&a).unwrap();
    assert_eq!(raw.len(), 32, "token {a:?} is not 32 hex bytes");
    assert_eq!(a, a.to_lowercase());
    assert_ne!(a, b, "two tokens are equal");
}

#[test]
fn test_write_read_round_trip() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("nested").join("irori");
    let want = sample(me());
    lockfile::write(&dir, &want).unwrap();
    let got = lockfile::read(&dir).unwrap();
    assert_eq!(got, want);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join(FILE_NAME))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

/// Go `json.MarshalIndent(lf, "", "  ")`: two spaces, no trailing newline.
#[test]
fn test_write_bytes_are_gos() {
    let t = tempfile::tempdir().unwrap();
    lockfile::write(t.path(), &sample(1234)).unwrap();
    let got = std::fs::read_to_string(t.path().join(FILE_NAME)).unwrap();
    let want = "{\n  \"pid\": 1234,\n  \"port\": 4321,\n  \"token\": \"abc\",\n  \"version\": \"0.1.0\",\n  \"deviceId\": \"dev1\",\n  \"headless\": false,\n  \"startedAt\": \"2026-01-02T03:04:05Z\"\n}";
    assert_eq!(got, want);
}

#[test]
fn test_write_replaces_and_leaves_no_temp() {
    let t = tempfile::tempdir().unwrap();
    let first = sample(me());
    lockfile::write(t.path(), &first).unwrap();
    let mut second = first.clone();
    second.port = 9999;
    lockfile::write(t.path(), &second).unwrap();
    assert_eq!(lockfile::read(t.path()).unwrap().port, 9999);
    let names: Vec<String> = std::fs::read_dir(t.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec![FILE_NAME.to_string()]);
}

#[test]
fn test_read_missing() {
    let t = tempfile::tempdir().unwrap();
    assert!(matches!(lockfile::read(t.path()), Err(Error::NotRunning)));
}

/// A pid that no longer runs: a child started and reaped.
fn dead_pid() -> i64 {
    let mut c = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--list")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let pid = i64::from(c.id());
    c.wait().unwrap();
    pid
}

#[test]
fn test_read_dead_pid() {
    let t = tempfile::tempdir().unwrap();
    lockfile::write(t.path(), &sample(dead_pid())).unwrap();
    assert!(matches!(lockfile::read(t.path()), Err(Error::NotRunning)));
}

#[test]
fn test_read_corrupt() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join(FILE_NAME), b"{not json").unwrap();
    match lockfile::read(t.path()) {
        Err(Error::Unreadable(s)) => assert!(s.starts_with("discovery: "), "{s}"),
        other => panic!("want a parse error, got {other:?}"),
    }
}

/// Go reads the file with `json.Unmarshal`: keys match case-insensitively.
#[test]
fn test_read_is_go_lenient() {
    let t = tempfile::tempdir().unwrap();
    let body = format!(
        r#"{{"PID":{},"Port":7,"token":"x","STARTEDAT":"2026-01-02T03:04:05.5+02:00"}}"#,
        me()
    );
    std::fs::write(t.path().join(FILE_NAME), body).unwrap();
    let lf = lockfile::read(t.path()).unwrap();
    assert_eq!((lf.port, lf.token.as_str()), (7, "x"));
    assert_eq!(
        lf.started_at.format_rfc3339_nano(),
        "2026-01-02T03:04:05.5+02:00"
    );
}

#[test]
fn test_remove_only_if_ours() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join(FILE_NAME);
    lockfile::write(t.path(), &sample(me())).unwrap();
    // An older instance exiting must leave a newer one's lockfile alone.
    lockfile::remove(t.path(), me() + 1).unwrap();
    assert!(path.exists(), "lockfile removed by another pid");
    lockfile::remove(t.path(), me()).unwrap();
    assert!(!path.exists(), "lockfile still there");
    // Removing again is fine.
    lockfile::remove(t.path(), me()).unwrap();
}

/// A reader holding irori.json open must not make Write fail on Windows.
#[test]
fn test_write_survives_brief_reader() {
    let t = tempfile::tempdir().unwrap();
    let dir: &Path = t.path();
    let mut lf = Lockfile {
        pid: me(),
        port: 1,
        ..Lockfile::default()
    };
    lockfile::write(dir, &lf).unwrap();
    let f = std::fs::File::open(dir.join(FILE_NAME)).unwrap();
    let h = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        drop(f);
    });
    lf.port = 2;
    lockfile::write(dir, &lf).expect("write while a reader had the file open");
    h.join().unwrap();
    assert_eq!(lockfile::read(dir).unwrap().port, 2);
}

/// The token never shows in Debug output.
#[test]
fn test_debug_hides_the_token() {
    let mut lf = sample(1);
    lf.token = "secret-token-value".into();
    assert!(!format!("{lf:?}").contains("secret-token-value"));
}
