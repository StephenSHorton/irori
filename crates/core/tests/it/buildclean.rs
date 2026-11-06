//! Card #427: a settled card's worktree doesn't keep its `target/`.

use crate::common;

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::*;
use irori_core::buildclean::{CLEARING, Cleared, NO_TARGET, clear, settled_worktrees};

const HOUR: Duration = Duration::from_secs(3600);

/// A worktree Irori made, in a fresh repo under `root`, with a cargo
/// `target/`.
fn worktree(root: &Path, name: &str) -> PathBuf {
    let main = git_repo(&root.join(name));
    let w = git_tree(&main, name);
    git_target(&w);
    w
}

fn later() -> SystemTime {
    SystemTime::now() + 2 * HOUR
}

#[test]
fn an_idle_worktrees_target_goes_and_its_source_stays() {
    let d = tempfile::tempdir().unwrap();
    let w = worktree(d.path(), "w");
    assert_eq!(clear(&w, HOUR, later()).unwrap(), Cleared::Gone);
    assert!(!w.join("target").exists());
    assert!(!w.join(CLEARING).exists());
    assert!(w.join("src.txt").is_file() && w.join(".git").is_file());
}

#[test]
fn a_recent_build_a_held_lock_or_a_main_checkout_keeps_it() {
    let d = tempfile::tempdir().unwrap();
    // Built just now.
    let w = worktree(d.path(), "recent");
    assert_eq!(
        clear(&w, HOUR, SystemTime::now()).unwrap(),
        Cleared::Kept("built recently")
    );
    assert!(w.join("target/CACHEDIR.TAG").is_file());
    // A cargo building in it holds the lock.
    let w = worktree(d.path(), "building");
    let f = File::options()
        .read(true)
        .write(true)
        .open(w.join("target/debug/.cargo-lock"))
        .unwrap();
    f.lock().unwrap();
    assert_eq!(
        clear(&w, HOUR, later()).unwrap(),
        Cleared::Kept("a build holds its lock")
    );
    assert!(w.join("target/CACHEDIR.TAG").is_file());
    drop(f);
    assert_eq!(clear(&w, HOUR, later()).unwrap(), Cleared::Gone);
    // A main checkout: .git is a directory.
    let w = git_repo(&d.path().join("main"));
    git_target(&w);
    assert_eq!(
        clear(&w, HOUR, later()).unwrap(),
        Cleared::Kept("not a tree Irori made")
    );
    assert!(w.join("target/CACHEDIR.TAG").is_file());
    // A linked worktree the person made: not under .claude/worktrees.
    let main = git_repo(&d.path().join("mine"));
    let w = d.path().join("irori-release");
    git(
        &main,
        &["worktree", "add", "-q", "-b", "rel", &w.to_string_lossy()],
    );
    git_target(&w);
    assert_eq!(
        clear(&w, HOUR, later()).unwrap(),
        Cleared::Kept("not a tree Irori made")
    );
    assert!(w.join("target/CACHEDIR.TAG").is_file());
    // A target/ that isn't cargo's.
    let w = worktree(d.path(), "other");
    fs::remove_file(w.join("target/CACHEDIR.TAG")).unwrap();
    assert_eq!(clear(&w, HOUR, later()).unwrap(), Cleared::Kept(NO_TARGET));
    assert!(w.join("target/debug/deps/libx.rlib").is_file());
}

#[test]
fn a_leftover_from_a_stop_midway_is_deleted() {
    let d = tempfile::tempdir().unwrap();
    let w = worktree(d.path(), "w");
    fs::create_dir_all(w.join(CLEARING).join("debug")).unwrap();
    fs::remove_dir_all(w.join("target")).unwrap();
    assert_eq!(clear(&w, HOUR, later()).unwrap(), Cleared::Kept(NO_TARGET));
    assert!(!w.join(CLEARING).exists());
}

fn card(state: &str, who: &str, tree: &str) -> coord::Task {
    coord::Task {
        state: state.into(),
        where_: Some(coord::Where {
            who: who.into(),
            worktree: tree.into(),
            ..coord::Where::default()
        }),
        ..coord::Task::default()
    }
}

#[test]
fn only_settled_cards_here_whose_worktree_no_open_card_uses() {
    let mut awaiting = card(coord::COMPLETED, "dev/s1", "/w/awaiting");
    let mut check = coord::Blocker::default();
    check.done_check = true;
    awaiting.blockers.push(check);
    let tasks = [
        card(coord::COMPLETED, "dev/s1", "/w/done"),
        card(coord::CANCELED, "dev/s2", "/w/canceled"),
        card(coord::FAILED, "dev/s2", "/w/failed"),
        // Another device's worktree isn't ours to clear.
        card(coord::COMPLETED, "other/s1", "/w/theirs"),
        // Waiting on the person's Done: it may come back.
        awaiting,
        // A settled card's worktree that an open card now works in.
        card(coord::COMPLETED, "dev/s1", "/w/reused"),
        card(coord::WORKING, "dev/s3", "/w/reused"),
        card(coord::WORKING, "dev/s3", "/w/open"),
    ];
    let trees: Vec<String> = settled_worktrees(&tasks, "dev", &BTreeMap::new())
        .into_iter()
        .map(|t| t.worktree)
        .collect();
    assert_eq!(trees, vec!["/w/canceled", "/w/done", "/w/failed"]);
}

#[test]
fn the_core_clears_a_canceled_cards_build() {
    let d = tempfile::tempdir().unwrap();
    let w = worktree(d.path(), "card");
    let r = Rig::with(|o| o.clear_builds = Some(Duration::ZERO));
    let co = r.core.coord();
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "Build it".into(),
                objective: "why".into(),
                ..coord::NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap();
    co.set_where(
        &t.id,
        coord::Where {
            who: r.key_a.clone(),
            cwd: w.to_string_lossy().into(),
            worktree: w.to_string_lossy().into(),
            branch: "fix/x".into(),
            head: "1a2b3c4d5e".into(),
            ..coord::Where::default()
        },
    )
    .unwrap();
    co.set_task_state(&r.key_a, &t.id, coord::CANCELED, "not needed")
        .unwrap();
    r.core.sweep_coord().unwrap();
    // Within LOOK (30 s) of the sweep loop's last look, at most.
    wait_for(45, "the canceled card's target/ to go", || {
        !w.join("target").exists()
    });
    assert!(w.join("src.txt").is_file());
}
