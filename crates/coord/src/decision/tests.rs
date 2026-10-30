//! Ports of I:internal/decision/decision_test.go and anchor_test.go.
#![allow(clippy::type_complexity)]

use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use gocompat::time::GoTime;

use super::anchor::{MAX_CHECK_SIZE, SCAN_BUDGET};
use super::lines::hash_lines;
use super::*;

/// Restores a tunable when the test ends, even on a panic.
struct Restore(&'static std::sync::atomic::AtomicI64, i64);
impl Drop for Restore {
    fn drop(&mut self) {
        self.0.store(self.1, Ordering::Relaxed);
    }
}

/// The tunables are process-wide; tests that change them or rely on their
/// defaults take this lock.
static TUNE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn tune() -> std::sync::MutexGuard<'static, ()> {
    TUNE.lock().unwrap_or_else(|e| e.into_inner())
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn write_file(root: &Path, rel: &str, content: &str) -> String {
    let full = root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(&full, content).unwrap();
    s(&full)
}

const TEN_LINES: &str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";

#[test]
fn test_validate() {
    let rep = |c: &str, n: usize| c.repeat(n);
    let cases: Vec<(&str, String, String, Option<Error>, String)> = vec![
        (
            "plain",
            "use sqlite".into(),
            "".into(),
            None,
            "use sqlite".into(),
        ),
        (
            "trims",
            "  use sqlite \r\n".into(),
            "  because \t".into(),
            None,
            "use sqlite".into(),
        ),
        ("empty", "".into(), "".into(), Some(Error::Empty), "".into()),
        (
            "blank",
            " \t\n ".into(),
            "why".into(),
            Some(Error::Empty),
            "".into(),
        ),
        (
            "stmt at max runes",
            rep("é", MAX_STATEMENT),
            "".into(),
            None,
            rep("é", MAX_STATEMENT),
        ),
        (
            "stmt over max",
            rep("a", MAX_STATEMENT + 1),
            "".into(),
            Some(Error::TooLong),
            rep("a", MAX_STATEMENT + 1),
        ),
        (
            "stmt over max multibyte",
            rep("界", MAX_STATEMENT + 1),
            "".into(),
            Some(Error::TooLong),
            rep("界", MAX_STATEMENT + 1),
        ),
        (
            "stmt max after trim",
            format!("  {}  ", rep("a", MAX_STATEMENT)),
            "".into(),
            None,
            rep("a", MAX_STATEMENT),
        ),
        (
            "why at max runes",
            "s".into(),
            rep("界", MAX_WHY),
            None,
            "s".into(),
        ),
        (
            "why over max",
            "s".into(),
            rep("b", MAX_WHY + 1),
            Some(Error::TooLong),
            "s".into(),
        ),
    ];
    for (name, stmt, why, want, want_stmt) in cases {
        let mut d = Decision {
            statement: stmt,
            why: why.clone(),
            ..Decision::default()
        };
        assert_eq!(validate(&mut d).err(), want, "{name}");
        if !want_stmt.is_empty() {
            assert_eq!(d.statement, want_stmt, "{name}");
        }
        assert_eq!(d.why, why.trim(), "{name}");
    }
}

#[test]
fn test_parse_lines() {
    let ok = [
        ("", 0, 0),
        ("  ", 0, 0),
        ("40", 40, 40),
        (" 40 ", 40, 40),
        ("40-52", 40, 52),
        ("40 - 52", 40, 52),
        ("40 -52", 40, 52),
        ("40- 52", 40, 52),
        ("1-1", 1, 1),
        ("007", 7, 7),
    ];
    for (input, a, b) in ok {
        assert_eq!(parse_lines(input), Ok((a, b)), "{input:?}");
    }
    let bad = [
        "0",
        "0-3",
        "40-39",
        "+40",
        "-40",
        "40-",
        "-",
        "a",
        "4a",
        "1-2-3",
        "1.5",
        "1,2",
        "4 0",
        "99999999999999999999999",
    ];
    for input in bad {
        assert_eq!(parse_lines(input), Err(Error::BadLines), "{input:?}");
    }
}

#[test]
fn test_make_anchor_paths() {
    let rootd = tempfile::tempdir().unwrap();
    let root = s(rootd.path());
    write_file(rootd.path(), "src/app/main.go", TEN_LINES);
    write_file(rootd.path(), "top.txt", TEN_LINES);
    let other = tempfile::tempdir().unwrap();
    let outside = write_file(other.path(), "other.go", TEN_LINES);
    let abs = s(&rootd.path().join("src").join("app").join("main.go"));
    let sep = std::path::MAIN_SEPARATOR_STR;
    let mut cases: Vec<(&str, String, &str, Option<Error>)> = vec![
        (
            "relative",
            "src/app/main.go".into(),
            "src/app/main.go",
            None,
        ),
        (
            "backslashes",
            r"src\app\main.go".into(),
            "src/app/main.go",
            None,
        ),
        (
            "dot prefix",
            "./src/app/main.go".into(),
            "src/app/main.go",
            None,
        ),
        (
            "doubled slashes",
            "src//app/./main.go".into(),
            "src/app/main.go",
            None,
        ),
        ("spaces trimmed", "  top.txt ".into(), "top.txt", None),
        ("absolute native", abs.clone(), "src/app/main.go", None),
        (
            "absolute forward",
            abs.replace('\\', "/"),
            "src/app/main.go",
            None,
        ),
        ("dotdot escape", "../x.go".into(), "", Some(Error::BadPath)),
        (
            "dotdot inside",
            "src/../top.txt".into(),
            "",
            Some(Error::BadPath),
        ),
        (
            "dotdot backslash",
            r"src\..\..\x".into(),
            "",
            Some(Error::BadPath),
        ),
        (
            "dotdot absolute",
            format!("{}{sep}..{sep}top.txt", s(&rootd.path().join("src"))),
            "",
            Some(Error::BadPath),
        ),
        ("absolute outside", outside, "", Some(Error::BadPath)),
        ("root itself", root.clone(), "", Some(Error::BadPath)),
        (
            "sibling prefix",
            format!("{root}2{sep}top.txt"),
            "",
            Some(Error::BadPath),
        ),
        ("empty", "".into(), "", Some(Error::BadPath)),
        ("dot", ".".into(), "", Some(Error::BadPath)),
        ("missing", "src/nope.go".into(), "", Some(Error::NoFile)),
        ("directory", "src/app".into(), "", Some(Error::NoFile)),
    ];
    if cfg!(windows) {
        let upper = format!(
            "{}{}",
            gocompat::text::to_upper_str(&abs[..root.len()]),
            &abs[root.len()..]
        );
        cases.push(("windows case-insensitive", upper, "src/app/main.go", None));
        let lower = format!("{}{}", abs[..1].to_lowercase(), &abs[1..]);
        cases.push(("windows lower drive", lower, "src/app/main.go", None));
        cases.push((
            "windows rooted no volume",
            r"\src\app\main.go".into(),
            "",
            Some(Error::BadPath),
        ));
        cases.push((
            "windows volume relative",
            "C:top.txt".into(),
            "",
            Some(Error::BadPath),
        ));
    } else {
        cases.push((
            "rooted outside",
            "/etc/hosts".into(),
            "",
            Some(Error::BadPath),
        ));
    }
    for (name, p, want, err) in cases {
        let got = make_anchor(&root, &p, 0, 0, "rk", "head");
        match err {
            Some(e) => assert_eq!(got.err(), Some(e), "{name}"),
            None => assert_eq!(got.map(|a| a.path).as_deref(), Ok(want), "{name}"),
        }
    }
    assert_eq!(
        make_anchor("", "top.txt", 0, 0, "", "").err(),
        Some(Error::BadPath)
    );
}

#[test]
fn test_make_anchor_whole_file() {
    let rootd = tempfile::tempdir().unwrap();
    let root = s(rootd.path());
    write_file(rootd.path(), "a.txt", "hello\n");
    write_file(rootd.path(), "empty.txt", "");
    let a = make_anchor(&root, "a.txt", 0, 0, "repo1", "abc123").unwrap();
    let want = Anchor {
        repo_key: "repo1".into(),
        root: gocompat::filepath::clean(&root),
        path: "a.txt".into(),
        sha: "abc123".into(),
        hash: "ce013625030ba8dba906f756967f9e9ca394464a".into(),
        ..Anchor::default()
    };
    assert_eq!(a, want);
    let e = make_anchor(&root, "empty.txt", 0, 0, "", "").unwrap();
    assert_eq!(e.hash, "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
    assert_eq!(
        make_anchor(&root, "empty.txt", 1, 1, "", "").err(),
        Some(Error::Range)
    );
}

#[test]
fn test_make_anchor_lines() {
    let rootd = tempfile::tempdir().unwrap();
    let root = s(rootd.path());
    write_file(rootd.path(), "f.txt", TEN_LINES);
    write_file(rootd.path(), "nonl.txt", "a\nb\nc");
    let h = |ls: &[&str]| hash_lines(ls);
    let v = |ls: &[&str]| ls.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let all: Vec<&str> = vec!["l1", "l2", "l3", "l4", "l5", "l6", "l7", "l8", "l9", "l10"];
    let cases: Vec<(&str, &str, i64, i64, String, Vec<String>, Option<Error>)> = vec![
        (
            "middle",
            "f.txt",
            4,
            5,
            h(&["l4", "l5"]),
            v(&["l2", "l3", "l6", "l7"]),
            None,
        ),
        (
            "single",
            "f.txt",
            5,
            5,
            h(&["l5"]),
            v(&["l3", "l4", "l6", "l7"]),
            None,
        ),
        (
            "first line",
            "f.txt",
            1,
            1,
            h(&["l1"]),
            v(&["l2", "l3"]),
            None,
        ),
        (
            "second line",
            "f.txt",
            2,
            3,
            h(&["l2", "l3"]),
            v(&["l1", "l4", "l5"]),
            None,
        ),
        (
            "last line",
            "f.txt",
            10,
            10,
            h(&["l10"]),
            v(&["l8", "l9"]),
            None,
        ),
        ("whole range", "f.txt", 1, 10, h(&all), vec![], None),
        (
            "no final newline",
            "nonl.txt",
            3,
            3,
            h(&["c"]),
            v(&["a", "b"]),
            None,
        ),
        (
            "past end",
            "f.txt",
            10,
            11,
            "".into(),
            vec![],
            Some(Error::Range),
        ),
        (
            "start past end",
            "f.txt",
            11,
            11,
            "".into(),
            vec![],
            Some(Error::Range),
        ),
        (
            "past end no newline",
            "nonl.txt",
            4,
            4,
            "".into(),
            vec![],
            Some(Error::Range),
        ),
        (
            "end before start",
            "f.txt",
            5,
            4,
            "".into(),
            vec![],
            Some(Error::BadLines),
        ),
        (
            "end without start",
            "f.txt",
            0,
            4,
            "".into(),
            vec![],
            Some(Error::BadLines),
        ),
        (
            "negative",
            "f.txt",
            -1,
            2,
            "".into(),
            vec![],
            Some(Error::BadLines),
        ),
    ];
    for (name, file, start, end, hash, ctx, err) in cases {
        let got = make_anchor(&root, file, start, end, "rk", "");
        match err {
            Some(e) => assert_eq!(got.err(), Some(e), "{name}"),
            None => {
                let a = got.unwrap();
                assert_eq!(
                    (a.start, a.end, &a.hash, &a.context),
                    (start, end, &hash, &ctx),
                    "{name}"
                );
            }
        }
    }
}

#[test]
fn test_line_hash_known_value() {
    let rootd = tempfile::tempdir().unwrap();
    write_file(rootd.path(), "x.txt", "a  \r\nb\t\r\n");
    let a = make_anchor(&s(rootd.path()), "x.txt", 1, 2, "", "").unwrap();
    assert_eq!(
        a.hash,
        "7e18f737311b2dc3b2f269dd78396b0351f14fb66efa879f768cb23181883c78"
    );
}

#[test]
fn test_crlf_matches_lf() {
    let _g = tune();
    let rootd = tempfile::tempdir().unwrap();
    let root = s(rootd.path());
    let lf = "package x\n\nfunc A() {   \n\treturn\n}\n\nfunc B() {}\n";
    let crlf = lf.replace('\n', "\r\n");
    write_file(rootd.path(), "lf.go", lf);
    write_file(rootd.path(), "crlf.go", &crlf);
    for (a0, b0) in [(0, 0), (3, 5), (1, 1), (7, 7)] {
        let a = make_anchor(&root, "lf.go", a0, b0, "", "").unwrap();
        let mut b = make_anchor(&root, "crlf.go", a0, b0, "", "").unwrap();
        assert_eq!(
            (&a.hash, &a.context),
            (&b.hash, &b.context),
            "lines {a0}-{b0}"
        );
        if a0 == 0 {
            assert_eq!(a.hash, gitsha::blob(lf.as_bytes()));
        }
        b.path = "lf.go".into();
        assert_eq!(check(&b, &root).state, AnchorState::Ok, "lines {a0}-{b0}");
    }
}

fn st(state: AnchorState, start: i64, end: i64) -> Status {
    Status { state, start, end }
}

#[test]
fn test_check() {
    let _g = tune();
    let orig = "a\nb\nc\nd\ne\nf\n";
    let mk = |content: &str, start: i64, end: i64| {
        let d = tempfile::tempdir().unwrap();
        write_file(d.path(), "f.txt", content);
        let a = make_anchor(&s(d.path()), "f.txt", start, end, "rk", "").unwrap();
        (a, d)
    };
    use AnchorState::*;
    let cases: Vec<(&str, i64, i64, String, Status)> = vec![
        ("unchanged", 3, 4, orig.into(), st(Ok, 3, 4)),
        (
            "whitespace only",
            3,
            4,
            "a\nb\nc   \r\nd\t\ne\nf\n".into(),
            st(Ok, 3, 4),
        ),
        (
            "moved down",
            3,
            4,
            "x\ny\na\nb\nc\nd\ne\nf\n".into(),
            st(Moved, 5, 6),
        ),
        ("moved up", 3, 4, "c\nd\ne\nf\n".into(), st(Moved, 1, 2)),
        (
            "edited",
            3,
            4,
            "a\nb\nC\nd\ne\nf\n".into(),
            st(Changed, 0, 0),
        ),
        ("truncated", 5, 6, "a\nb\n".into(), st(Changed, 0, 0)),
        ("emptied", 1, 1, "".into(), st(Changed, 0, 0)),
        ("whole ok", 0, 0, orig.into(), st(Ok, 0, 0)),
        (
            "whole ok crlf",
            0,
            0,
            orig.replace('\n', "\r\n"),
            st(Ok, 0, 0),
        ),
        (
            "whole changed",
            0,
            0,
            format!("{orig}g\n"),
            st(Changed, 0, 0),
        ),
        (
            "whole trailing space changed",
            0,
            0,
            "a \nb\nc\nd\ne\nf\n".into(),
            st(Changed, 0, 0),
        ),
        (
            "tie earlier",
            3,
            4,
            "c\nd\nx\ny\nc\nd\n".into(),
            st(Moved, 1, 2),
        ),
        (
            "nearest",
            4,
            4,
            "d\nx\nx\nx\nd\nx\n".into(),
            st(Moved, 5, 5),
        ),
        (
            "in place despite copy",
            3,
            4,
            "c\nd\nc\nd\n".into(),
            st(Ok, 3, 4),
        ),
        (
            "only later copy",
            1,
            2,
            "x\nx\nx\nx\nx\nx\na\nb\n".into(),
            st(Moved, 7, 8),
        ),
    ];
    for (name, start, end, after, want) in cases {
        let (a, d) = mk(orig, start, end);
        write_file(d.path(), "f.txt", &after);
        assert_eq!(check(&a, &s(d.path())), want, "{name}");
    }

    // gone
    let (a, d) = mk(orig, 2, 2);
    std::fs::remove_file(d.path().join("f.txt")).unwrap();
    assert_eq!(check(&a, &s(d.path())).state, Gone);
    // a directory is gone
    std::fs::create_dir(d.path().join("f.txt")).unwrap();
    assert_eq!(check(&a, &s(d.path())).state, Gone);
    // other root
    let (a, _d) = mk(orig, 2, 3);
    let other = tempfile::tempdir().unwrap();
    write_file(other.path(), "f.txt", &format!("z\n{orig}"));
    assert_eq!(check(&a, &s(other.path())), st(Moved, 3, 4));
    // too big
    {
        let (a, d) = mk(orig, 0, 0);
        let _r = Restore(&MAX_CHECK_SIZE, MAX_CHECK_SIZE.load(Ordering::Relaxed));
        MAX_CHECK_SIZE.store(orig.len() as i64 - 1, Ordering::Relaxed);
        assert_eq!(check(&a, &s(d.path())).state, Changed);
    }
    // eight MiB limit
    {
        let d = tempfile::tempdir().unwrap();
        let big = write_file(d.path(), "big.bin", "");
        let f = std::fs::OpenOptions::new().write(true).open(&big).unwrap();
        f.set_len((8 << 20) + 1).unwrap();
        drop(f);
        let a = Anchor {
            path: "big.bin".into(),
            hash: gitsha::blob(&vec![0u8; (8 << 20) + 1]),
            ..Anchor::default()
        };
        assert_eq!(check(&a, &s(d.path())).state, Changed);
    }

    let tmp = tempfile::tempdir().unwrap();
    let abs = s(&tmp.path().join("f.txt")).replace('\\', "/");
    let unknown: Vec<(&str, Anchor, &str)> = vec![
        ("no root", anchor("f.txt", "x", 0, 0), ""),
        ("no hash", anchor("f.txt", "", 0, 0), "."),
        ("bad range", anchor("f.txt", "x", 5, 4), "."),
        ("negative", anchor("f.txt", "x", -1, 0), "."),
        ("escape", anchor("../f.txt", "x", 0, 0), "."),
        ("absolute", anchor(&abs, "x", 0, 0), "."),
        ("unclean", anchor("./f.txt", "x", 0, 0), "."),
        ("backslash", anchor(r"a\f.txt", "x", 0, 0), "."),
    ];
    for (name, a, root) in unknown {
        assert_eq!(check(&a, root).state, Unknown, "unknown {name}");
    }
}

fn anchor(path: &str, hash: &str, start: i64, end: i64) -> Anchor {
    Anchor {
        path: path.into(),
        hash: hash.into(),
        start,
        end,
        ..Anchor::default()
    }
}

/// Runs `check` on a thread and fails if it panics or takes longer than `d`.
fn check_within(a: Anchor, root: String, d: Duration) -> Status {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(check(&a, &root));
    });
    rx.recv_timeout(d)
        .expect("check did not return in time (or panicked)")
}

#[test]
fn test_check_hostile_ranges() {
    let _g = tune();
    let d = tempfile::tempdir().unwrap();
    write_file(d.path(), "a.txt", "a\nb\nc\nd\n");
    let cases = [
        ("huge end", 2, i64::MAX),
        ("max start and end", i64::MAX, i64::MAX),
        ("huge start", 1 << 40, 1 << 40),
        ("huge start wide", 1 << 40, (1 << 40) + 2),
        ("end past file", 1, 5),
        ("start past file", 9, 9),
    ];
    for (name, start, end) in cases {
        let a = anchor("a.txt", "x", start, end);
        let got = check_within(a, s(d.path()), Duration::from_secs(2));
        assert_eq!(got.state, AnchorState::Changed, "{name}");
    }
}

#[test]
fn test_check_start_past_end_finds_nearest() {
    let _g = tune();
    let d = tempfile::tempdir().unwrap();
    write_file(d.path(), "a.txt", "x\nb\nc\nx\nb\nc\n");
    let mut a = make_anchor(&s(d.path()), "a.txt", 2, 3, "rk", "").unwrap();
    a.start = 1 << 40;
    a.end = (1 << 40) + 1;
    let got = check_within(a, s(d.path()), Duration::from_secs(2));
    assert_eq!(got, st(AnchorState::Moved, 5, 6));
}

#[test]
fn test_check_scan_budget() {
    let _g = tune();
    let d = tempfile::tempdir().unwrap();
    let mut b = String::new();
    for i in 0..2000 {
        b.push_str(&format!("line {i}\n"));
    }
    write_file(d.path(), "a.txt", &b);
    let a = make_anchor(&s(d.path()), "a.txt", 100, 199, "rk", "").unwrap();
    let moved = format!("pad\n{}{b}", "y\n".repeat(1500));
    write_file(d.path(), "a.txt", &moved);
    assert_eq!(check(&a, &s(d.path())), st(AnchorState::Moved, 1601, 1700));
    let _r = Restore(&SCAN_BUDGET, SCAN_BUDGET.load(Ordering::Relaxed));
    SCAN_BUDGET.store(10 << 10, Ordering::Relaxed);
    assert_eq!(check(&a, &s(d.path())).state, AnchorState::Changed);
}

#[test]
fn test_check_large_changed_is_bounded() {
    let _g = tune();
    let d = tempfile::tempdir().unwrap();
    let mut b = String::new();
    for i in 0..200_000 {
        b.push_str(&format!("l{i}\n"));
    }
    write_file(d.path(), "a.txt", &b);
    let mut a = make_anchor(&s(d.path()), "a.txt", 1000, 3000, "rk", "").unwrap();
    a.hash = "0".repeat(64);
    let start = Instant::now();
    let got = check_within(a, s(d.path()), Duration::from_secs(60));
    assert_eq!(got.state, AnchorState::Changed);
    // Debug builds hash slowly; Go allows 2 s (20 s under -race).
    assert!(start.elapsed() < Duration::from_secs(60));
}

#[test]
fn test_check_missing_root_is_unknown() {
    let d = tempfile::tempdir().unwrap();
    write_file(d.path(), "a.txt", "a\nb\n");
    let a = make_anchor(&s(d.path()), "a.txt", 1, 1, "rk", "").unwrap();
    assert_eq!(
        check(&a, &s(&d.path().join("nope"))).state,
        AnchorState::Unknown
    );
    assert_eq!(
        check(&a, &s(&d.path().join("a.txt"))).state,
        AnchorState::Unknown
    );
}

#[test]
fn test_make_anchor_on_disk_case() {
    if !cfg!(any(windows, target_os = "macos")) {
        return; // case-sensitive file system
    }
    let d = tempfile::tempdir().unwrap();
    let root = s(d.path());
    write_file(d.path(), "Src/a.txt", "a\n");
    let mut cases = vec![s(&d.path().join("SRC").join("A.TXT"))];
    if cfg!(windows) {
        cases.push(format!(
            "{}{}\\src\\A.txt",
            root[..1].to_uppercase(),
            &root[1..]
        ));
    }
    cases.push("src/A.TXT".into());
    cases.push("Src/a.txt".into());
    for p in cases {
        let a = make_anchor(&root, &p, 1, 1, "rk", "").unwrap();
        assert_eq!(a.path, "Src/a.txt", "{p:?}");
    }
}

fn t0() -> GoTime {
    GoTime::parse_rfc3339("2026-10-27T12:00:00Z").unwrap()
}

fn dec(id: &str, key: &str, kind: &str, min: i64, sup: &[&str]) -> Decision {
    Decision {
        id: id.into(),
        by: Party {
            key: key.into(),
            kind: kind.into(),
            ..Party::default()
        },
        statement: format!("stmt {id}"),
        supersedes: sup.iter().map(|s| s.to_string()).collect(),
        trust: "untrusted".into(),
        at: crate::gotime::add(&t0(), Duration::from_secs(60 * min as u64)),
        ..Decision::default()
    }
}

fn ids(ls: &[Listed]) -> Vec<String> {
    ls.iter().map(|l| l.decision.id.clone()).collect()
}

#[test]
fn test_view_add_get() {
    let v = View::new();
    assert!(v.get("a").is_none());
    let mut d = dec("a", "k1", "claude", 0, &[]);
    d.anchor = Some(Anchor {
        path: "x.go".into(),
        context: vec!["c1".into()],
        ..Anchor::default()
    });
    d.source = Some(Source {
        turn_id: "t1".into(),
        ..Source::default()
    });
    v.add(d.clone());
    let mut dup = dec("a", "k2", "claude", 5, &[]);
    dup.statement = "other".into();
    v.add(dup);
    v.add(Decision {
        statement: "no id".into(),
        ..Decision::default()
    });
    let got = v.get("a").unwrap();
    assert_eq!(
        (got.statement.as_str(), got.by.key.as_str()),
        ("stmt a", "k1")
    );
    assert_eq!(
        v.list(&Filter {
            all: true,
            ..Filter::default()
        })
        .len(),
        1
    );
    // Rust values are owned: what went in or came out cannot alias the view.
    let again = v.get("a").unwrap();
    assert_eq!(again.anchor.unwrap().context, vec!["c1".to_string()]);
}

#[test]
fn test_view_supersede() {
    let v = View::new();
    v.add(dec("a", "k1", "claude", 0, &[]));
    v.add(dec("b", "k1", "claude", 1, &["a"]));
    v.add(dec("c", "k2", "grok", 2, &["d", "c", ""]));
    assert_eq!(ids(&v.list(&Filter::default())), ["c", "b"]);
    v.add(dec("d", "k2", "grok", 3, &[]));
    assert_eq!(ids(&v.list(&Filter::default())), ["c", "b"]);
    v.add(dec("e", "u", "user", 4, &["a", "a"]));
    let all = v.list(&Filter {
        all: true,
        ..Filter::default()
    });
    assert_eq!(ids(&all), ["e", "d", "c", "b", "a"]);
    let by = |id: &str| {
        all.iter()
            .find(|l| l.decision.id == id)
            .unwrap()
            .superseded_by
            .clone()
    };
    assert_eq!(by("a"), ["b", "e"]);
    assert_eq!(by("d"), ["c"]);
    assert!(by("c").is_empty() && by("b").is_empty());
}

#[test]
fn test_can_supersede() {
    let v = View::new();
    v.add(dec("a", "k1", "claude", 0, &[]));
    v.add(dec("noKey", "", "claude", 0, &[]));
    let p = |key: &str, kind: &str| Party {
        key: key.into(),
        kind: kind.into(),
        ..Party::default()
    };
    let cases = [
        ("author", p("k1", "claude"), "a", None),
        ("author other kind", p("k1", "grok"), "a", None),
        (
            "other agent",
            p("k2", "claude"),
            "a",
            Some(Error::Supersede),
        ),
        ("user", p("someone", "user"), "a", None),
        ("user no key", p("", "user"), "a", None),
        (
            "empty keys do not match",
            p("", "claude"),
            "noKey",
            Some(Error::Supersede),
        ),
        (
            "unknown target",
            p("k1", ""),
            "zzz",
            Some(Error::NoDecision),
        ),
        (
            "unknown target user",
            p("", "user"),
            "zzz",
            Some(Error::NoDecision),
        ),
    ];
    for (name, by, target, want) in cases {
        assert_eq!(v.can_supersede(&by, target).err(), want, "{name}");
    }
}

#[test]
fn test_list_filters() {
    let v = View::new();
    let add = |id: &str, min: i64, rk: &str, path: &str, stmt: &str, why: &str| {
        let mut d = dec(id, "k", "claude", min, &[]);
        d.statement = stmt.into();
        d.why = why.into();
        if !path.is_empty() {
            d.anchor = Some(Anchor {
                repo_key: rk.into(),
                path: path.into(),
                ..Anchor::default()
            });
        }
        v.add(d);
    };
    add("1", 1, "r1", "src/a.go", "Use SQLite", "");
    add(
        "2",
        2,
        "r1",
        "src/b.go",
        "retry twice",
        "the network is FLAKY",
    );
    add("3", 3, "r2", "src/a.go", "no globals", "");
    add("4", 4, "", "", "unanchored sqlite note", "");
    add("5", 4, "r1", "src/a.go", "same time, added later", "");
    let f = |rk: &str, path: &str, q: &str, limit: i64| Filter {
        repo_key: rk.into(),
        path: path.into(),
        query: q.into(),
        all: false,
        limit,
    };
    let cases: Vec<(&str, Filter, Vec<&str>)> = vec![
        (
            "all newest first",
            f("", "", "", 0),
            vec!["5", "4", "3", "2", "1"],
        ),
        ("repo", f("r1", "", "", 0), vec!["5", "2", "1"]),
        ("path", f("", "src/a.go", "", 0), vec!["5", "3", "1"]),
        (
            "path backslash",
            f("", r"src\a.go", "", 0),
            vec!["5", "3", "1"],
        ),
        ("path exact", f("", "src/a", "", 0), vec![]),
        ("repo and path", f("r2", "src/a.go", "", 0), vec!["3"]),
        ("query statement", f("", "", "sqlite", 0), vec!["4", "1"]),
        ("query why", f("", "", "flaky", 0), vec!["2"]),
        ("query trimmed", f("", "", "  Flaky ", 0), vec!["2"]),
        ("query none", f("", "", "zebra", 0), vec![]),
        ("limit", f("", "", "", 2), vec!["5", "4"]),
    ];
    for (name, filter, want) in cases {
        assert_eq!(ids(&v.list(&filter)), want, "{name}");
    }
}

#[test]
fn test_list_limit() {
    let v = View::new();
    for i in 0..600 {
        v.add(dec(&i.to_string(), "k", "claude", i, &[]));
    }
    for (limit, want) in [
        (0, 50),
        (-3, 50),
        (1, 1),
        (50, 50),
        (499, 499),
        (500, 500),
        (501, 500),
        (10000, 500),
    ] {
        let got = v.list(&Filter {
            limit,
            ..Filter::default()
        });
        assert_eq!(got.len(), want, "limit {limit}");
        assert_eq!(got[0].decision.id, "599", "limit {limit}");
    }
}

#[test]
fn test_view_concurrent() {
    let v = std::sync::Arc::new(View::new());
    let mut hs = Vec::new();
    for i in 0..20 {
        let v = v.clone();
        hs.push(std::thread::spawn(move || {
            for j in 0..50i64 {
                let id = format!("{i}-{j}");
                let prev = format!("{i}-{}", j - 1);
                v.add(dec(&id, "k", "claude", j, &[&prev]));
                v.get(&id);
                v.list(&Filter {
                    query: "stmt".into(),
                    ..Filter::default()
                });
                let _ = v.can_supersede(
                    &Party {
                        key: "k".into(),
                        ..Party::default()
                    },
                    &id,
                );
            }
        }));
    }
    for h in hs {
        h.join().unwrap();
    }
    assert_eq!(
        v.list(&Filter {
            all: true,
            limit: 500,
            ..Filter::default()
        })
        .len(),
        500
    );
    assert_eq!(
        v.list(&Filter {
            limit: 500,
            ..Filter::default()
        })
        .len(),
        20
    );
}

#[test]
fn test_gitsha_vectors() {
    assert_eq!(
        gitsha::blob(b""),
        "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
    );
    assert_eq!(
        gitsha::blob(b"hello\n"),
        "ce013625030ba8dba906f756967f9e9ca394464a"
    );
    let (raw, lf) = gitsha::pair(b"hello\r\n");
    assert_eq!(lf, "ce013625030ba8dba906f756967f9e9ca394464a");
    assert_ne!(raw, lf);
    assert_eq!(gitsha::pair(b"hello\n").1, "");
}

#[test]
fn test_decision_payload_round_trip() {
    let d = dec("a", "k", "claude", 0, &["x"]);
    let raw = gocompat::json::marshal(&d).unwrap();
    assert_eq!(
        String::from_utf8(raw.clone()).unwrap(),
        r#"{"id":"a","by":{"key":"k","kind":"claude"},"statement":"stmt a","supersedes":["x"],"trust":"untrusted","at":"2026-10-27T12:00:00Z"}"#
    );
    assert_eq!(crate::gojson::decode::<Decision>(&raw), Some(d));
}
