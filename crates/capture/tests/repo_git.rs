//! Ports of I:internal/repo/resolver_test.go (fake git and real temp repos;
//! the two that change the environment or cwd are in repo_git_env.rs),
//! I:internal/capture/gitlink's real-git tests (Exec, GitDir, Describe,
//! ReflogMovements, NewCommitsMaxWalk), and the gitsha-vs-git check of
//! ClassifyRealBlobs.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use capture::gitexec::{ExecError, git_env};
use capture::gitlink::{self, Exec, Git, GitError};
use capture::repo::{CACHE_TTL, Resolver};
use capture::{gitsha, gopath};
use common::*;
use gocompat::time::GoTime;
use sys::cancel::Cancel;

/// Answers git calls from a table keyed by the joined args.
#[derive(Default)]
struct FakeGit {
    answers: Mutex<HashMap<String, String>>,
    calls: Mutex<Vec<String>>,
    block: Mutex<bool>,
}

impl FakeGit {
    fn with(answers: &[(String, &str)]) -> Arc<FakeGit> {
        let f = FakeGit::default();
        *f.answers.lock().unwrap() = answers
            .iter()
            .map(|(k, v)| (k.clone(), v.to_string()))
            .collect();
        Arc::new(f)
    }
    fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

fn resolver(f: Arc<FakeGit>, now: Arc<Mutex<GoTime>>) -> Resolver {
    let run = move |timeout: Duration, args: &[String]| -> Result<String, ExecError> {
        let k = args.join(" ");
        f.calls.lock().unwrap().push(k.clone());
        if *f.block.lock().unwrap() {
            std::thread::sleep(timeout);
            return Err(ExecError::Timeout);
        }
        f.answers
            .lock()
            .unwrap()
            .get(&k)
            .cloned()
            .ok_or(ExecError::Exit {
                code: 128,
                stderr: String::new(),
            })
    };
    Resolver::with(Arc::new(run), Arc::new(move || *now.lock().unwrap()))
}

fn abs_dir(name: &str) -> String {
    gocompat::filepath::abs(name).unwrap()
}

#[test]
fn key_caches_per_dir_and_toplevel() {
    let root = abs_dir("r");
    let sub = gopath::join(&[&root, "sub"]);
    let f = FakeGit::with(&[
        (format!("-C {root} rev-parse --show-toplevel"), &root),
        (format!("-C {sub} rev-parse --show-toplevel"), &root),
        (
            format!("-C {root} remote get-url origin"),
            "git@github.com:Foo/Bar.git",
        ),
        (format!("-C {root} rev-list --max-parents=0 HEAD"), "abc"),
    ]);
    let now = Arc::new(Mutex::new(GoTime::unix(1000, 0)));
    let r = resolver(f.clone(), now.clone());
    assert_eq!(r.key(&root), "remote:github.com/foo/bar");
    assert_eq!(f.count(), 2);
    r.key(&root);
    assert_eq!(f.count(), 2, "cached lookup ran git");
    assert_eq!(r.key(&sub), "remote:github.com/foo/bar");
    assert_eq!(f.count(), 3, "a new dir costs only the toplevel lookup");
    let later = capture::gotime::add(&now.lock().unwrap(), CACHE_TTL);
    *now.lock().unwrap() = later;
    r.key(&sub);
    assert_eq!(f.count(), 5, "after the TTL everything is asked again");
}

#[test]
fn key_falls_back_to_root_commit() {
    let x = abs_dir("x");
    let now = Arc::new(Mutex::new(GoTime::unix(0, 0)));
    let f = FakeGit::with(&[
        (format!("-C {x} rev-parse --show-toplevel"), &x),
        (format!("-C {x} remote get-url origin"), "../local/clone"),
        (
            format!("-C {x} rev-list --max-parents=0 HEAD"),
            "ffff\naaaa\n",
        ),
    ]);
    assert_eq!(resolver(f, now.clone()).key("x"), "root:aaaa");
    let f = FakeGit::with(&[(format!("-C {x} rev-parse --show-toplevel"), &x)]);
    assert_eq!(resolver(f, now).key("x"), "");
}

#[test]
fn key_caches_negative() {
    let f = FakeGit::with(&[]);
    let now = Arc::new(Mutex::new(GoTime::unix(0, 0)));
    let r = resolver(f.clone(), now.clone());
    assert_eq!(r.key("nowhere"), "");
    r.key("nowhere");
    assert_eq!(f.count(), 1, "negative not cached");
    let later = capture::gotime::add(&now.lock().unwrap(), CACHE_TTL + Duration::from_secs(1));
    *now.lock().unwrap() = later;
    r.key("nowhere");
    assert_eq!(f.count(), 2, "negative didn't expire");
    assert_eq!(r.key(""), "");
    assert_eq!(f.count(), 2);
}

#[test]
fn key_timeout_not_cached() {
    let f = FakeGit::with(&[]);
    *f.block.lock().unwrap() = true;
    let now = Arc::new(Mutex::new(GoTime::unix(0, 0)));
    let mut r = resolver(f.clone(), now);
    r.set_timeout(Duration::from_millis(50));
    let start = std::time::Instant::now();
    assert_eq!(r.key("slow"), "");
    assert!(start.elapsed() < Duration::from_secs(1));
    let slow = abs_dir("slow");
    *f.block.lock().unwrap() = false;
    *f.answers.lock().unwrap() = [
        (format!("-C {slow} rev-parse --show-toplevel"), slow.clone()),
        (
            format!("-C {slow} rev-list --max-parents=0 HEAD"),
            "abc".to_string(),
        ),
    ]
    .into_iter()
    .collect();
    assert_eq!(r.key("slow"), "root:abc", "timed-out answer was cached");
}

#[test]
fn key_concurrent() {
    let a = abs_dir("a");
    let f = FakeGit::with(&[
        (format!("-C {a} rev-parse --show-toplevel"), &a),
        (format!("-C {a} rev-list --max-parents=0 HEAD"), "abc"),
    ]);
    let r = Arc::new(resolver(f, Arc::new(Mutex::new(GoTime::unix(0, 0)))));
    let hs: Vec<_> = (0..32)
        .map(|_| {
            let r = r.clone();
            std::thread::spawn(move || assert_eq!(r.key("a"), "root:abc"))
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
}

fn init_repo() -> (tempfile::TempDir, String) {
    let (d, dir) = tempdir();
    git(&dir, &["init", "-q"]);
    git(&dir, &["commit", "-q", "--allow-empty", "-m", "first"]);
    (d, dir)
}

#[test]
fn integration_root_commit_remote_and_clones() {
    let (_a, a) = init_repo();
    let want = format!("root:{}", git(&a, &["rev-list", "--max-parents=0", "HEAD"]));
    assert_eq!(real_resolver().key(&a), want);
    // A local origin falls back to the shared root commit.
    let (_b, b) = tempdir();
    git(&b, &["clone", "-q", &a, "clone"]);
    let r = real_resolver();
    assert_eq!(r.key(&gopath::join(&[&b, "clone"])), r.key(&a));
    // A hosted origin names the repo, from any subdirectory.
    git(
        &a,
        &["remote", "add", "origin", "git@github.com:Foo/Bar.git"],
    );
    let sub = gopath::join(&[&a, "x", "y"]);
    std::fs::create_dir_all(&sub).unwrap();
    let r = real_resolver();
    assert_eq!(r.key(&a), "remote:github.com/foo/bar");
    assert_eq!(r.key(&sub), "remote:github.com/foo/bar");
}

#[test]
fn integration_local_remote_falls_back() {
    let (_a, a) = init_repo();
    let (_o, origin) = tempdir();
    git(&a, &["remote", "add", "origin", &origin]);
    let want = format!("root:{}", git(&a, &["rev-list", "--max-parents=0", "HEAD"]));
    assert_eq!(real_resolver().key(&a), want);
}

#[test]
fn integration_not_a_repo_and_empty_repo() {
    let (_d, dir) = tempdir();
    assert_eq!(real_resolver().key(&dir), "");
    assert_eq!(real_resolver().key(&gopath::join(&[&dir, "missing"])), "");
    git(&dir, &["init", "-q"]);
    assert_eq!(real_resolver().key(&dir), "");
    git(
        &dir,
        &["remote", "add", "origin", "https://gitlab.com/g/s/n.git"],
    );
    assert_eq!(real_resolver().key(&dir), "remote:gitlab.com/g/s/n");
}

#[test]
fn git_env_drops_repo_vars() {
    let env: Vec<(String, String)> = [
        "PATH=x",
        "GIT_DIR=/r",
        "GIT_WORK_TREE=/w",
        "GIT_AUTHOR_NAME=a",
        "HOME=h",
        "git_dir=low",
    ]
    .iter()
    .map(|kv| {
        let (k, v) = kv.split_once('=').unwrap();
        (k.to_string(), v.to_string())
    })
    .collect();
    let got: Vec<String> = git_env(env)
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    let mut want = vec!["PATH=x", "GIT_AUTHOR_NAME=a", "HOME=h"];
    if !cfg!(windows) {
        want.push("git_dir=low"); // names are case-sensitive off Windows
    }
    assert_eq!(got, want);
}

#[test]
fn exec_runs_git_untrimmed_and_reports_errors() {
    let (_d, dir) = new_repo("false");
    let sha = head(&dir);
    let c = Cancel::new();
    let out = Exec
        .run(&c, Duration::from_secs(15), &dir, &["rev-parse", "HEAD"])
        .unwrap();
    assert_eq!(out, format!("{sha}\n"), "stdout comes back untrimmed");
    let err = Exec
        .run(
            &c,
            Duration::from_secs(15),
            &dir,
            &["rev-parse", "--verify", "nope^{commit}"],
        )
        .unwrap_err();
    assert!(
        matches!(&err, GitError::Exit { code: 128, msg } if msg.starts_with("git rev-parse: exit status 128")),
        "{err:?}"
    );
    let canceled = Cancel::new();
    canceled.cancel();
    assert_eq!(
        Exec.run(&canceled, Duration::from_secs(15), &dir, &["status"]),
        Err(GitError::Canceled)
    );
}

#[test]
fn git_dir_dir_file_relative_and_bom() {
    let (_d, dir) = new_repo("false");
    assert!(gopath::base(&gitlink::git_dir(&dir).unwrap()).eq_ignore_ascii_case(".git"));
    let (_w, base) = tempdir();
    let wt = gopath::join(&[&base, "wt"]);
    git(&dir, &["worktree", "add", "-q", &wt]);
    let gd = gitlink::git_dir(&wt).unwrap();
    assert!(gd.to_ascii_lowercase().contains("worktrees"), "{gd}");
    // A relative gitdir with a BOM resolves against the root.
    let (_r, other) = tempdir();
    std::fs::write(
        gopath::join(&[&other, ".git"]),
        "\u{feff}gitdir: sub/gd\r\n",
    )
    .unwrap();
    assert_eq!(
        gitlink::git_dir(&other).unwrap(),
        gopath::join(&[&other, "sub", "gd"])
    );
    std::fs::write(gopath::join(&[&other, ".git"]), "nonsense\n").unwrap();
    assert!(gitlink::git_dir(&other).is_err());
}

#[test]
fn describe_and_new_commits_on_a_real_repo() {
    let (_d, dir) = new_repo("false");
    let first = head(&dir);
    write_file(&gopath::join(&[&dir, "a.txt"]), "a\n");
    write_file(&gopath::join(&[&dir, "dir", "é b.txt"]), "b\n");
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "two files"]);
    git(&dir, &["rm", "-q", "README.md"]);
    git(&dir, &["commit", "-q", "-m", "delete"]);
    let last = head(&dir);
    let c = Cancel::new();
    let t = Duration::from_secs(15);
    let d = gitlink::describe(&Exec, &c, t, &dir, &last).unwrap();
    assert_eq!(d.subject, "delete");
    assert_eq!(d.parents.len(), 1);
    let files = d.files.unwrap();
    assert_eq!(files.len(), 1);
    assert!(
        files[0].path == "README.md" && files[0].status == "D" && gitlink::zero_sha(&files[0].blob)
    );
    let root = gitlink::describe(&Exec, &c, t, &dir, &first).unwrap();
    assert!(root.parents.is_empty());
    assert_eq!(root.files.unwrap()[0].blob, gitsha::blob(b"hello\n"));
    // The movement from first to last brings two commits, oldest first.
    let log = std::fs::read(gopath::join(&[&dir, ".git", "logs", "HEAD"])).unwrap();
    let (entries, n) = gitlink::parse_reflog(&log);
    assert_eq!(n, log.len());
    let mv = gitlink::ReflogEntry {
        old: first.clone(),
        new: last.clone(),
        ..entries.last().unwrap().clone()
    };
    let (shas, skipped) = gitlink::new_commits(&Exec, &c, t, &dir, &mv, &|_| false).unwrap();
    assert_eq!(skipped, 0);
    assert_eq!(shas.len(), 2);
    assert_eq!(shas[1], last);
    let mid = gitlink::describe(&Exec, &c, t, &dir, &shas[0]).unwrap();
    let mut paths: Vec<String> = mid.files.unwrap().into_iter().map(|f| f.path).collect();
    paths.sort();
    assert_eq!(paths, ["a.txt", "dir/é b.txt"], "paths come back unquoted");
    // Seen commits are dropped; a seen new runs nothing.
    let (shas, _) = gitlink::new_commits(&Exec, &c, t, &dir, &mv, &|s| s == last).unwrap();
    assert!(shas.is_empty());
}

/// Blob ids from gitsha equal what git stores, with and without autocrlf.
#[test]
fn gitsha_matches_git_blobs() {
    for crlf in ["false", "true"] {
        let (_d, dir) = new_repo(crlf);
        let body = "line one\r\nline two\r\n";
        write_file(&gopath::join(&[&dir, "f.txt"]), body);
        git(&dir, &["add", "f.txt"]);
        let stored = git(&dir, &["rev-parse", ":f.txt"]);
        let (raw, lf) = gitsha::pair(body.as_bytes());
        if crlf == "true" {
            assert_eq!(stored, lf);
        } else {
            assert_eq!(stored, raw);
        }
    }
}
