//! Ports of the I:internal/capture/gitlink tests that run git (real temp
//! repos or a recording fake) and that no golden table covers:
//! TestClassifyTwoRoots, TestClassifyFastForwardFromRemovedWorktree,
//! TestDescribeMaxFiles, TestDescribeFakeErrors, TestReflogPartialLineOnDisk
//! and TestNewCommitsFake. (TestParseReflogZone is the `-0730` chunk of
//! gitlink.json's parseReflog vectors.)

mod common;

use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use capture::gitlink::{self, Commit, EditRef, Exec, FileLink, Git, GitError, ReflogEntry};
use capture::{gitsha, gopath};
use common::*;
use gocompat::time::GoTime;
use sys::cancel::Cancel;

const T: Duration = Duration::from_secs(15);

fn sha(c: char) -> String {
    c.to_string().repeat(40)
}

fn at(s: i64) -> GoTime {
    GoTime::unix(1_700_000_000 + s, 0)
}

/// Go's `linkString`.
fn link_string(ls: &[FileLink]) -> String {
    ls.iter()
        .map(|l| format!("{}={}[{}]", l.path, l.link, l.edits.join(",")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Answers from a table keyed by the joined args; a missing key is exit 128.
#[derive(Default)]
struct RecordGit {
    answers: HashMap<String, String>,
    calls: Mutex<Vec<String>>,
}

impl RecordGit {
    fn with(answers: &[(String, String)]) -> RecordGit {
        RecordGit {
            answers: answers.iter().cloned().collect(),
            ..RecordGit::default()
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Git for RecordGit {
    fn run(&self, _: &Cancel, _: Duration, _: &str, args: &[&str]) -> Result<String, GitError> {
        let k = args.join(" ");
        self.calls.lock().unwrap().push(k.clone());
        self.answers.get(&k).cloned().ok_or(GitError::Exit {
            code: 128,
            msg: "exit status 128".into(),
        })
    }
}

/// A repo at `<tmp>/repo` (so worktrees can sit beside it), on main.
fn repo_in_dir(autocrlf: &str) -> (tempfile::TempDir, String, String) {
    let (d, base) = tempdir();
    let root = gopath::join(&[&base, "repo"]);
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "core.autocrlf", autocrlf]);
    (d, base, root)
}

/// Go's `commitIn`: add everything and commit with both dates at `at`.
fn commit_in(dir: &str, msg: &str, at: i64) -> String {
    let date = format!("{at} +0000");
    let mut cmd = Command::new("git");
    cmd.args([
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        msg,
    ])
    .current_dir(dir);
    for (k, _) in std::env::vars() {
        if k.to_ascii_uppercase().starts_with("GIT_") {
            cmd.env_remove(k);
        }
    }
    cmd.env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date);
    git(dir, &["add", "-A"]);
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "commit: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    head(dir)
}

fn describe(root: &str, sha: &str) -> Commit {
    gitlink::describe(&Exec, &Cancel::new(), T, root, sha).unwrap()
}

fn reflog(root: &str) -> Vec<ReflogEntry> {
    let dir = gitlink::git_dir(root).unwrap();
    let data = std::fs::read(gopath::join(&[&dir, "logs", "HEAD"])).unwrap();
    let (entries, n) = gitlink::parse_reflog(&data);
    assert_eq!(n, data.len());
    entries
}

#[test]
fn classify_two_roots() {
    let (_d, base, root) = repo_in_dir("false");
    write_file(&gopath::join(&[&root, "foo.go"]), "package foo\n");
    commit_in(&root, "base", 1_700_000_000);
    let wt_a = gopath::join(&[&base, "wtA"]);
    let wt_b = gopath::join(&[&base, "wtB"]);
    git(&root, &["worktree", "add", "-q", "-b", "a", &wt_a]);
    git(&root, &["worktree", "add", "-q", "-b", "b", &wt_b]);
    let content_a = "package foo\n\n// A\n";
    let content_b = "package foo\n\n// B\n";
    // A's edit matches its commit; B's last edit was rewritten by a
    // formatter before B committed.
    let cands = vec![
        EditRef {
            id: "a1".into(),
            root: wt_a.clone(),
            path: "foo.go".into(),
            after_blob: gitsha::blob(content_a.as_bytes()),
            at: at(1),
            ..EditRef::default()
        },
        EditRef {
            id: "b1".into(),
            root: wt_b.clone(),
            path: "foo.go".into(),
            after_blob: gitsha::blob(format!("{content_b} ").as_bytes()),
            at: at(2),
            ..EditRef::default()
        },
    ];
    write_file(&gopath::join(&[&wt_a, "foo.go"]), content_a);
    write_file(&gopath::join(&[&wt_b, "foo.go"]), content_b);
    let sha_a = commit_in(&wt_a, "A", 1_700_000_010);
    let sha_b = commit_in(&wt_b, "B", 1_700_000_020);
    let ca = describe(&wt_a, &sha_a);
    let cb = describe(&wt_b, &sha_b);
    assert_eq!(
        link_string(&gitlink::classify(&ca, &wt_a, &cands)),
        "foo.go=exact[a1]"
    );
    assert_eq!(
        link_string(&gitlink::classify(&cb, &wt_b, &cands)),
        "foo.go=partial[b1]"
    );
    // Committed from main (say after merging A), A's blob still links
    // exactly to A.
    assert_eq!(
        link_string(&gitlink::classify(&ca, &root, &cands)),
        "foo.go=exact[a1]"
    );
}

#[test]
fn classify_fast_forward_from_removed_worktree() {
    let (_d, base, root) = repo_in_dir("true");
    write_file(&gopath::join(&[&root, "a.txt"]), "a\n");
    commit_in(&root, "base", 1_700_000_000);
    let wt = gopath::join(&[&base, "worktree-wf_1"]);
    git(&root, &["worktree", "add", "-q", "-b", "wf", &wt]);
    let content = "line one\r\nline two\r\n";
    let (raw, lf) = gitsha::pair(content.as_bytes());
    write_file(&gopath::join(&[&wt, "a.txt"]), content);
    let sha = commit_in(&wt, "agent work", 1_700_000_100);
    git(&root, &["worktree", "remove", &wt]);
    git(&root, &["merge", "-q", "--ff-only", "wf"]);
    let log = reflog(&root);
    let (shas, _) = gitlink::new_commits(
        &Exec,
        &Cancel::new(),
        T,
        &root,
        log.last().unwrap(),
        &|_| false,
    )
    .unwrap();
    assert_eq!(shas, vec![sha.clone()]);
    let c = describe(&root, &sha);
    let cands = vec![EditRef {
        id: "e1".into(),
        root: wt,
        path: "a.txt".into(),
        after_blob: raw,
        after_blob_lf: lf,
        at: at(1),
        ..EditRef::default()
    }];
    assert_eq!(
        link_string(&gitlink::classify(&c, &root, &cands)),
        "a.txt=exact[e1]"
    );
}

#[test]
fn describe_max_files() {
    let (_d, _base, root) = repo_in_dir("false");
    for i in 0..gitlink::MAX_FILES + 5 {
        write_file(
            &gopath::join(&[&root, "f", &format!("{}.txt", 10000 + i)]),
            &format!("{i}\n"),
        );
    }
    let sha = commit_in(&root, "many", 1_700_000_000);
    let got = describe(&root, &sha);
    assert_eq!(got.files.map(|f| f.len()), Some(gitlink::MAX_FILES));
    assert_eq!(got.files_omitted, 5);
}

#[test]
fn describe_fake_errors() {
    let sha = sha('a');
    let show = format!(
        "-c log.showSignature=false show -s --no-color --format=%H%x00%P%x00%ct%x00%an%x00%s {sha} --"
    );
    for (name, out) in [
        ("too few fields", format!("{sha}\x00\x00123\n")),
        ("bad time", format!("{sha}\x00\x00soon\x00A\x00s\n")),
        ("bad sha", "nope\x00\x00123\x00A\x00s\n".to_string()),
    ] {
        let g = RecordGit::with(&[(show.clone(), out)]);
        assert!(
            gitlink::describe(&g, &Cancel::new(), T, "r", &sha).is_err(),
            "{name}: want error"
        );
    }
    // diff-tree failing is an error too.
    let g = RecordGit::with(&[(show.clone(), format!("{sha}\x00\x00123\x00A\x00s\n"))]);
    assert!(gitlink::describe(&g, &Cancel::new(), T, "r", &sha).is_err());
    let calls = g.calls();
    assert!(
        calls.len() == 2 && calls[1].contains("--root"),
        "root commit diff-tree should pass --root: {calls:?}"
    );
}

#[test]
fn reflog_partial_line_on_disk() {
    let (_d, _base, root) = repo_in_dir("false");
    write_file(&gopath::join(&[&root, "a.txt"]), "a\n");
    commit_in(&root, "one", 1_700_000_000);
    let dir = gitlink::git_dir(&root).unwrap();
    let data = std::fs::read(gopath::join(&[&dir, "logs", "HEAD"])).unwrap();
    let mut half = data.clone();
    half.extend_from_slice(format!("{} {} A <a@", sha('a'), sha('b')).as_bytes());
    let (got, n) = gitlink::parse_reflog(&half);
    assert!(
        got.len() == 1 && n == data.len(),
        "got {} entries, consumed {n} of {}",
        got.len(),
        data.len()
    );
}

#[test]
fn new_commits_fake() {
    let (a, b, c) = (sha('a'), sha('b'), sha('c'));
    let zero = "0".repeat(40);
    let maxc = format!("--max-count={}", gitlink::MAX_WALK);
    let entry = |old: &str, new: &str| ReflogEntry {
        old: old.into(),
        new: new.into(),
        name: String::new(),
        email: String::new(),
        at: GoTime::default(),
        message: String::new(),
    };
    let run = |g: &RecordGit, e: &ReflogEntry, seen: &dyn Fn(&str) -> bool| {
        gitlink::new_commits(g, &Cancel::new(), T, "r", e, seen)
    };

    // New already seen: no git at all.
    let g = RecordGit::default();
    let got = run(&g, &entry(&a, &b), &|s| s == b).unwrap();
    assert!(got.0.is_empty() && got.1 == 0 && g.calls().is_empty());
    // A zero New (never written for HEAD, but harmless): nothing.
    let got = run(&g, &entry(&a, &zero), &|_| false).unwrap();
    assert!(got.0.is_empty() && g.calls().is_empty());
    // Malformed ids never reach git.
    for e in [
        entry(&a, "--output=x"),
        entry("HEAD~1", &b),
        entry("", "abc"),
    ] {
        assert!(run(&g, &e, &|_| false).is_err(), "entry {e:?}: want error");
    }
    assert!(g.calls().is_empty(), "{:?}", g.calls());

    // Old object gone: falls back to walking New alone.
    let g = RecordGit::with(&[(
        format!("rev-list --reverse {maxc} {b} --"),
        format!("{a}\n{b}\n"),
    )]);
    let (shas, _) = run(&g, &entry(&c, &b), &|s| s == a).unwrap();
    assert!(
        shas == vec![b.clone()] && g.calls().len() == 2,
        "fallback: {shas:?} calls {:?}",
        g.calls()
    );

    // Both walks fail: the error comes back.
    let g = RecordGit::default();
    assert!(run(&g, &entry(&c, &b), &|_| false).is_err());
}
