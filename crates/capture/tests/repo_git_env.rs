//! Ports of I:internal/repo/resolver_test.go's two tests that change
//! process state: TestIntegrationIgnoresInheritedGitDir (GIT_DIR and
//! GIT_WORK_TREE set, as under a git hook) and
//! TestIntegrationRelativeDirFollowsCwd. They live in their own test binary,
//! as one test, so no other test sees the environment or the working
//! directory move.

mod common;

use capture::repo::Resolver;
use common::*;

fn init_repo() -> (tempfile::TempDir, String) {
    let (d, dir) = tempdir();
    git(&dir, &["init", "-q"]);
    git(&dir, &["commit", "-q", "--allow-empty", "-m", "first"]);
    (d, dir)
}

/// The resolver over the real git, with capture's git limit instead of the
/// resolver's 2 s, which spawning git can take on a loaded machine.
fn real_git() -> Resolver {
    let mut r = Resolver::new();
    r.set_timeout(capture::GIT_TIMEOUT);
    r
}

#[test]
fn integration_inherited_git_dir_and_relative_dir() {
    let (_a, a) = init_repo();
    let (_b, b) = init_repo();
    git(&b, &["remote", "add", "origin", "git@github.com:x/y.git"]);

    // RelativeDirFollowsCwd: "." is resolved against the cwd of each call.
    let was = std::env::current_dir().unwrap();
    let r = real_git();
    std::env::set_current_dir(&a).unwrap();
    let ka = r.key(".");
    std::env::set_current_dir(&b).unwrap();
    let kb = r.key(".");
    std::env::set_current_dir(&was).unwrap();
    assert!(
        !ka.is_empty() && ka != "remote:github.com/x/y",
        "in a: {ka:?}"
    );
    assert_eq!(kb, "remote:github.com/x/y", "in b");

    // IgnoresInheritedGitDir: as if launched from a git hook in repo a.
    // SAFETY: this binary runs this one test, so no other thread reads the
    // environment while it changes.
    unsafe {
        std::env::set_var("GIT_DIR", format!("{a}/.git"));
        std::env::set_var("GIT_WORK_TREE", &a);
    }
    let r = real_git();
    let got_b = r.key(&b);
    let got_a = r.key(&a);
    unsafe {
        std::env::remove_var("GIT_DIR");
        std::env::remove_var("GIT_WORK_TREE");
    }
    assert_eq!(got_b, "remote:github.com/x/y", "b");
    assert!(!got_a.is_empty() && got_a != got_b, "a: {got_a:?}");
}
