//! Part 7 (spawned agents §3): a helper's worktree is branched from the
//! card's branch, and making it again reuses it or carries on from the
//! part's branch.

use std::path::Path;
use std::process::Command;

use irori_core::helpertree::{WORKTREES, helper_branch, make, slug};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A repo on main with a card branch one commit ahead (the lead's plan).
fn repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let r = d.path().join("game-repo");
    std::fs::create_dir(&r).unwrap();
    git(&r, &["init", "-q", "-b", "main"]);
    std::fs::write(r.join("a.txt"), "a").unwrap();
    git(&r, &["add", "."]);
    git(&r, &["commit", "-q", "-m", "first"]);
    git(&r, &["checkout", "-q", "-b", "toffee/451-card"]);
    std::fs::write(r.join("plan.md"), "plan").unwrap();
    git(&r, &["add", "."]);
    git(&r, &["commit", "-q", "-m", "plan"]);
    git(&r, &["checkout", "-q", "main"]);
    (d, r)
}

#[test]
fn names_are_safe() {
    assert_eq!(slug("#12"), "12");
    assert_eq!(slug("wind#4"), "wind-4");
    assert_eq!(slug("s3"), "s3");
    assert_eq!(slug("../x y"), "x-y");
    assert_eq!(helper_branch("toffee/451-card", "s3"), "toffee/451-card-s3");
}

#[test]
fn a_helper_starts_from_the_card_branch() {
    let (_d, r) = repo();
    let card_head = git(&r, &["rev-parse", "toffee/451-card"]);
    let h = make(&r, "toffee/451-card", "s3").unwrap();
    assert!(h.made);
    assert_eq!(h.branch, "toffee/451-card-s3");
    assert_eq!(h.path, r.join(WORKTREES).join("451-card-s3"));
    assert_eq!(git(&h.path, &["rev-parse", "HEAD"]), card_head);
    assert_eq!(git(&h.path, &["branch", "--show-current"]), h.branch);
    assert!(h.path.join("plan.md").exists(), "the lead's plan is there");
    // Three parts, three worktrees.
    let others: Vec<_> = ["s4", "s5"]
        .iter()
        .map(|p| make(&r, "toffee/451-card", p).unwrap().path)
        .collect();
    assert_ne!(others[0], others[1]);
    assert_ne!(others[0], h.path);
    // Asked again: the same worktree.
    let again = make(&r, "toffee/451-card", "s3").unwrap();
    assert!(!again.made);
    assert_eq!(
        again.path.canonicalize().unwrap(),
        h.path.canonicalize().unwrap()
    );
}

#[test]
fn a_fresh_helper_carries_on_from_the_parts_branch() {
    let (_d, r) = repo();
    let h = make(&r, "toffee/451-card", "s3").unwrap();
    std::fs::write(h.path.join("tests.rs"), "t").unwrap();
    git(&h.path, &["add", "."]);
    git(&h.path, &["commit", "-q", "-m", "helper work"]);
    let theirs = git(&h.path, &["rev-parse", "HEAD"]);
    git(&r, &["worktree", "remove", h.path.to_str().unwrap()]);
    let h2 = make(&r, "toffee/451-card", "s3").unwrap();
    assert!(h2.made);
    assert_eq!(git(&h2.path, &["rev-parse", "HEAD"]), theirs);
}

#[test]
fn the_card_branch_from_origin_or_a_reason() {
    let (_d, r) = repo();
    let e = make(&r, "nobody/no-branch", "s1").unwrap_err();
    assert!(e.contains("isn't here or on origin"), "{e}");
    assert!(make(&r, "toffee/451-card", "#").is_err());
    // Only on origin: a clone has it as origin/<branch>.
    let d2 = tempfile::tempdir().unwrap();
    let c = d2.path().join("clone");
    git(
        d2.path(),
        &["clone", "-q", r.to_str().unwrap(), c.to_str().unwrap()],
    );
    let h = make(&c, "toffee/451-card", "s1").unwrap();
    assert!(h.path.join("plan.md").exists());
}
