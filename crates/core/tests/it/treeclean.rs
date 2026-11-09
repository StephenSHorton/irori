//! Card #565: a settled card's worktree goes, build output and all, and
//! nothing but a clean, pushed tree Irori made ever does. Every removal
//! here is in a temp git repo.

use crate::common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use common::*;
use irori_core::buildclean::settled_worktrees;
use irori_core::spawn::{ClaudeAgent, SpawnStarted, Spawner};
use irori_core::treeclean::{
    Book, Check, KEPT_EVERY, Verdict, any_under, aside_path, begun_path, end_holder, holders, key,
    lsof_cwds, names_tree, only_temp, ours, pick_root, regenerable, remove, reopen, restore_all,
    temp_root, tree_of, under,
};
use irori_core::trees::{BUSY_SAY, KEPT_TREE, busy_plan, say_kept};

const HOUR: Duration = Duration::from_secs(3600);

fn later() -> SystemTime {
    SystemTime::now() + 2 * HOUR
}

fn judge(tree: &Path) -> Verdict {
    remove(
        tree,
        &Check {
            live: &[],
            idle: HOUR,
            now: later(),
            recheck: None,
        },
    )
}

fn setup(name: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("repo"));
    let w = git_tree(&main, name);
    (d, main, w)
}

fn is_work(v: &Verdict) -> bool {
    matches!(v, Verdict::Work(_))
}

fn commit(w: &Path, file: &str) {
    fs::write(w.join(file), "x\n").unwrap();
    git(w, &["add", "."]);
    git(w, &["commit", "-q", "-m", "work"]);
}

/// A tracked, pushed `package.json` in `dir` of tree `w`, so a
/// `node_modules/` beside it is a package manager's.
fn package_json(w: &Path, dir: &str) {
    fs::create_dir_all(w.join(dir)).unwrap();
    fs::write(w.join(dir).join("package.json"), "{}\n").unwrap();
    git(w, &["add", "."]);
    git(w, &["commit", "-q", "-m", "package"]);
    git(w, &["push", "-q", "origin", "HEAD"]);
}

/// Moves tree `w` of `main` aside with git, as a removal that stopped
/// part way leaves it.
fn strand(main: &Path, w: &Path) -> PathBuf {
    let aside = aside_path(w);
    git(
        main,
        &[
            "worktree",
            "move",
            &w.to_string_lossy(),
            &aside.to_string_lossy(),
        ],
    );
    assert!(aside.join(".git").is_file() && !w.exists());
    aside
}

#[test]
fn a_clean_pushed_tree_goes_with_its_build_and_keeps_its_branch() {
    let (_d, main, w) = setup("done");
    git_target(&w);
    // Read-only files, as a git object or a cargo registry copy is.
    let ro = w.join("target/debug/deps/ro.bin");
    fs::write(&ro, "x").unwrap();
    let mut p = fs::metadata(&ro).unwrap().permissions();
    p.set_readonly(true);
    fs::set_permissions(&ro, p).unwrap();
    assert_eq!(judge(&w), Verdict::Removed);
    assert!(!w.exists());
    assert!(!git(&main, &["worktree", "list"]).contains("done"));
    assert!(git(&main, &["branch", "--list", "done"]).contains("done"));
    assert!(main.join("src.txt").is_file());
}

#[test]
fn a_changed_file_keeps_the_tree() {
    let (_d, _main, w) = setup("dirty");
    fs::write(w.join("src.txt"), "changed\n").unwrap();
    assert!(is_work(&judge(&w)));
    assert_eq!(fs::read_to_string(w.join("src.txt")).unwrap(), "changed\n");
}

#[test]
fn an_untracked_file_keeps_the_tree_and_an_ignored_build_doesnt() {
    let (_d, _main, w) = setup("untracked");
    git_target(&w);
    fs::write(w.join("notes.txt"), "mine\n").unwrap();
    assert!(is_work(&judge(&w)));
    assert!(w.join("notes.txt").is_file());
    assert!(w.join("target/CACHEDIR.TAG").is_file(), "only judged");
    fs::remove_file(w.join("notes.txt")).unwrap();
    assert_eq!(judge(&w), Verdict::Removed);
}

#[test]
fn an_unpushed_commit_keeps_the_tree_until_it_is_pushed() {
    let (_d, _main, w) = setup("ahead");
    commit(&w, "new.txt");
    assert!(is_work(&judge(&w)));
    assert!(w.join("new.txt").is_file());
    git(&w, &["push", "-q", "origin", "ahead"]);
    assert_eq!(judge(&w), Verdict::Removed);
}

#[test]
fn a_detached_head_goes_on_a_pushed_commit_and_stays_on_an_unpushed_one() {
    let (_d, _main, w) = setup("detached");
    git(&w, &["checkout", "-q", "--detach"]);
    assert_eq!(judge(&w), Verdict::Removed);

    let (_d, _main, w) = setup("detached-ahead");
    git(&w, &["checkout", "-q", "--detach"]);
    commit(&w, "new.txt");
    assert!(is_work(&judge(&w)));
    assert!(w.join("new.txt").is_file());
}

#[test]
fn a_worktree_the_person_made_is_never_touched() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("repo"));
    let w = d.path().join("irori-release");
    git(
        &main,
        &["worktree", "add", "-q", "-b", "rel", &w.to_string_lossy()],
    );
    git_target(&w);
    assert_eq!(judge(&w), Verdict::NotOurs);
    assert!(ours(&w).is_none());
    assert!(w.join("target/CACHEDIR.TAG").is_file());
    assert!(w.join("src.txt").is_file());
    assert!(git(&main, &["worktree", "list"]).contains("rel"));
}

#[test]
fn a_tree_of_another_repo_at_our_path_isnt_ours() {
    // A worktree of repo B that happens to sit in repo A's worktrees folder.
    let d = tempfile::tempdir().unwrap();
    let a = git_repo(&d.path().join("a"));
    let b = git_repo(&d.path().join("b"));
    let w = a.join(".claude/worktrees/stray");
    git(
        &b,
        &["worktree", "add", "-q", "-b", "stray", &w.to_string_lossy()],
    );
    assert_eq!(judge(&w), Verdict::NotOurs);
    assert!(w.join("src.txt").is_file());
}

#[test]
fn a_main_checkout_is_never_touched() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("repo"));
    git_target(&main);
    assert_eq!(judge(&main), Verdict::NotOurs);
    assert!(ours(&main).is_none());
    assert!(main.join("target/CACHEDIR.TAG").is_file());
    assert!(main.join(".git").is_dir());
}

#[test]
fn a_live_agent_keeps_the_tree_however_its_path_is_written() {
    let (_d, _main, w) = setup("live");
    let inside = w.join("crates").to_string_lossy().replace('/', "\\");
    let live = [if cfg!(windows) {
        inside.to_uppercase()
    } else {
        w.join("crates").to_string_lossy().into_owned()
    }];
    let c = Check {
        live: &live,
        idle: HOUR,
        now: later(),
        recheck: None,
    };
    assert!(matches!(remove(&w, &c), Verdict::Busy(_)));
    assert!(w.is_dir());
}

#[test]
fn a_recent_build_or_a_held_cargo_lock_keeps_the_tree() {
    let (_d, _main, w) = setup("building");
    git_target(&w);
    let c = Check {
        live: &[],
        idle: HOUR,
        now: SystemTime::now(),
        recheck: None,
    };
    assert_eq!(remove(&w, &c), Verdict::Busy("built recently".into()));
    let f = File::options()
        .read(true)
        .write(true)
        .open(w.join("target/debug/.cargo-lock"))
        .unwrap();
    f.lock().unwrap();
    assert!(matches!(judge(&w), Verdict::Busy(_)));
    assert!(w.join("target/CACHEDIR.TAG").is_file());
    drop(f);
    assert_eq!(judge(&w), Verdict::Removed);
}

/// A process's cwd can't be listed without admin on Windows, so the rename
/// is what finds it, and a refusal is a kept tree.
#[cfg(windows)]
#[test]
fn a_process_sitting_in_the_tree_keeps_it_on_windows() {
    let (_d, _main, w) = setup("in-use");
    let mut child = std::process::Command::new("ping")
        .args(["-n", "30", "127.0.0.1"])
        .current_dir(&w)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let v = judge(&w);
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(v, Verdict::Busy("in use".into()));
    assert!(w.join("src.txt").is_file() && w.join(".git").is_file());
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(judge(&w), Verdict::Removed);
}

/// Review: a lock is the person saying keep it, so it is work, said once,
/// and the folder is never renamed to probe it.
#[test]
fn a_locked_worktree_stays() {
    let (_d, main, w) = setup("locked");
    git(&main, &["worktree", "lock", &w.to_string_lossy()]);
    let v = judge(&w);
    assert!(
        matches!(&v, Verdict::Work(why) if why.contains("lock")),
        "{v:?}"
    );
    assert!(w.is_dir());
    git(&main, &["worktree", "unlock", &w.to_string_lossy()]);
    assert_eq!(judge(&w), Verdict::Removed);
}

#[test]
fn a_folder_that_isnt_a_worktree_any_more_is_broken_and_left() {
    // Its admin entry is gone: the `.git` file points nowhere.
    let (_d, main, w) = setup("stale");
    let admin = main.join(".git/worktrees/stale");
    fs::remove_dir_all(&admin).unwrap();
    assert!(matches!(judge(&w), Verdict::Broken(_)));
    assert!(w.join("src.txt").is_file());
    // Its `.git` file is gone: git must not answer for the repo around it.
    let (_d, main, w) = setup("no-git");
    fs::remove_file(w.join(".git")).unwrap();
    assert!(matches!(judge(&w), Verdict::Broken(_)));
    assert!(w.join("src.txt").is_file());
    assert!(main.join(".git").is_dir());
}

fn card(id: &str, state: &str, tree: &str) -> coord::Task {
    coord::Task {
        id: id.into(),
        state: state.into(),
        where_: (!tree.is_empty()).then(|| coord::Where {
            who: "dev/s1".into(),
            worktree: tree.into(),
            cwd: tree.into(),
            ..coord::Where::default()
        }),
        ..coord::Task::default()
    }
}

fn book(trees: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
    trees
        .iter()
        .map(|(t, ids)| {
            let ids = ids.iter().map(|s| s.to_string()).collect();
            (key(t), ids)
        })
        .collect()
}

#[test]
fn an_unsettled_card_sharing_the_tree_keeps_it() {
    let t = "/r/.claude/worktrees/x";
    let tasks = [
        card("a", coord::COMPLETED, t),
        card("b", coord::WORKING, "/R/.claude/worktrees/x/crates/core"),
    ];
    // By Where: the open card's cwd is inside it.
    let mut b = tasks[1].clone();
    b.where_.as_mut().unwrap().worktree.clear();
    let open_cwd = [tasks[0].clone(), b];
    if cfg!(windows) {
        assert!(settled_worktrees(&open_cwd, "dev", &BTreeMap::new()).is_empty());
    }
    let same = [card("a", coord::COMPLETED, t), card("b", coord::WORKING, t)];
    assert!(settled_worktrees(&same, "dev", &BTreeMap::new()).is_empty());
    // By the book: card b has no Where at all.
    let no_where = [
        card("a", coord::COMPLETED, t),
        card("b", coord::WORKING, ""),
    ];
    let m = book(&[(t, &["a", "b"])]);
    assert!(settled_worktrees(&no_where, "dev", &m).is_empty());
}

#[test]
fn the_book_reaches_a_tree_no_where_names_and_nothing_else_does() {
    let t = "/r/.claude/worktrees/mentions";
    let tasks = [
        card("a", coord::COMPLETED, ""),
        card("b", coord::FAILED, ""),
    ];
    let got = settled_worktrees(&tasks, "dev", &book(&[(t, &["a", "b"])]));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].worktree, key(t));
    assert_eq!(got[0].cards, vec!["a", "b"]);
    // A tree nothing ties to a card is left alone.
    assert!(settled_worktrees(&tasks, "dev", &BTreeMap::new()).is_empty());
    // A card this device can't see can't be called settled.
    assert!(settled_worktrees(&tasks, "dev", &book(&[(t, &["a", "gone"])])).is_empty());
}

#[test]
fn the_book_survives_a_restart() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(Book::load(d.path()), Book::default());
    let mut b = Book::default();
    assert!(b.note(r"C:\R\.claude\worktrees\x\", "card"));
    assert!(!b.note("c:/r/.claude/worktrees/x", "card") || !cfg!(windows));
    b.kept_jobs.insert("dev/s1".into());
    b.save(d.path()).unwrap();
    assert_eq!(Book::load(d.path()), b);
}

#[test]
fn paths_compare_the_way_git_and_claude_write_them() {
    assert!(under(
        "/r/.claude/worktrees/x/src",
        "/r/.claude/worktrees/x/"
    ));
    assert!(!under("/r/.claude/worktrees/x2", "/r/.claude/worktrees/x"));
    assert!(!under("/r", ""));
    if cfg!(windows) {
        assert!(under(r"C:\R\A\b", "c:/r/a"));
        assert!(under(r"\\?\C:\R\A", "c:/r/a"));
    }
    assert_eq!(
        tree_of("/r/.claude/worktrees/x/crates/core").as_deref(),
        Some("/r/.claude/worktrees/x")
    );
    assert_eq!(tree_of("/r/.claude/worktrees/"), None);
    assert_eq!(tree_of("/r/crates"), None);
    assert!(temp_root("/h/.claude/jobs/47fb4a11/tmp/irori-439"));
    assert!(!temp_root("/h/projects/irori"));
}

#[test]
fn a_real_checkout_is_picked_and_a_temp_clone_never_is() {
    let d = tempfile::tempdir().unwrap();
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let clone = d.path().join("a/.claude/jobs/47fb4a11/tmp/irori-439");
    fs::create_dir_all(clone.join(".git")).unwrap();
    let linked = d.path().join("b-linked");
    fs::create_dir_all(&linked).unwrap();
    fs::write(linked.join(".git"), "gitdir: x\n").unwrap();
    let real = d.path().join("c-real");
    fs::create_dir_all(real.join(".git")).unwrap();
    let gone = d.path().join("0-gone");
    let all = [s(&gone), s(&clone), s(&linked), s(&real)];
    assert_eq!(pick_root(&all), Some(s(&real)));
    assert_eq!(pick_root(&all[..3]), Some(s(&linked)));
    assert_eq!(pick_root(&all[..2]), None, "only a temp clone: none");
    assert_eq!(only_temp(&all[..2]), Some(s(&clone)));
    assert_eq!(pick_root(&all[..1]), None);
    assert_eq!(only_temp(&all[..1]), None);
}

fn settled_card(r: &Rig, title: &str, w: Option<&Path>) -> coord::Task {
    let co = r.core.coord();
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: title.into(),
                objective: "why".into(),
                ..coord::NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap();
    if let Some(w) = w {
        let p: String = w.to_string_lossy().into();
        co.set_where(
            &t.id,
            coord::Where {
                who: r.key_a.clone(),
                cwd: p.clone(),
                worktree: p,
                branch: "b".into(),
                head: "1a2b3c4d5e".into(),
                ..coord::Where::default()
            },
        )
        .unwrap();
    }
    co.set_task_state(&r.key_a, &t.id, coord::CANCELED, "not needed")
        .unwrap();
    t
}

fn kept_lines(r: &Rig, id: &str) -> usize {
    let cs = r.core.coord().comments(id).unwrap_or_default();
    cs.iter().filter(|c| c.text.starts_with(KEPT_TREE)).count()
}

/// The saying's only memory is the card, so a restart can't repeat it.
#[test]
fn kept_is_said_once_however_often_and_whatever_the_reason() {
    let r = Rig::new();
    let t = settled_card(&r, "Kept", None);
    let co = r.core.coord();
    say_kept(co, &t.id, "/r/.claude/worktrees/x", "it has 2 files");
    say_kept(co, &t.id, "/r/.claude/worktrees/x", "it has 2 files");
    say_kept(
        co,
        &t.id,
        "/r/.claude/worktrees/x",
        "it has unpushed commits",
    );
    assert_eq!(kept_lines(&r, &t.id), 1);
    let cs = co.comments(&t.id).unwrap_or_default();
    assert!(
        cs.iter().any(|c| c.text.contains("it has 2 files.")),
        "{cs:?}"
    );
}

fn lister() -> Spawner {
    Spawner {
        spawn: Arc::new(|r| {
            Ok(SpawnStarted {
                name: r.name.clone(),
                prefix: String::new(),
            })
        }),
        end: Arc::new(|_| Ok(String::new())),
        trusted: Arc::new(|_| true),
        list: Some(Arc::new(|| Ok(Vec::<ClaudeAgent>::new()))),
        ready: None,
    }
}

#[test]
fn the_core_removes_a_settled_tree_and_says_why_it_kept_another() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("repo"));
    let (clean, dirty) = (git_tree(&main, "clean"), git_tree(&main, "dirty"));
    fs::write(dirty.join("notes.txt"), "mine\n").unwrap();
    // Its build going is the pass's last step: wait for it, or the pass's
    // git still runs in the tree when the temp folder goes.
    git_target(&dirty);
    let r = Rig::with(|o| o.clear_builds = Some(Duration::ZERO));
    r.core.set_spawner(Some(lister()));
    settled_card(&r, "Clean", Some(&clean));
    let kept = settled_card(&r, "Dirty", Some(&dirty));
    r.core.sweep_coord().unwrap();
    wait_for(45, "the clean tree to go", || !clean.exists());
    wait_for(45, "the kept line", || kept_lines(&r, &kept.id) == 1);
    wait_for(45, "the kept tree's build to go", || {
        !dirty.join("target").exists() && !dirty.join(irori_core::buildclean::CLEARING).exists()
    });
    assert!(dirty.join("notes.txt").is_file());
    let cs = r.core.coord().comments(&kept.id).unwrap_or_default();
    assert!(cs.iter().any(|c| c.text.contains("uncommitted")), "{cs:?}");
}

#[test]
fn with_no_agent_list_a_tree_stays_and_only_its_build_goes() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("repo"));
    let w = git_tree(&main, "unlisted");
    git_target(&w);
    let r = Rig::with(|o| o.clear_builds = Some(Duration::ZERO));
    settled_card(&r, "Unlisted", Some(&w));
    r.core.sweep_coord().unwrap();
    wait_for(45, "its target/ to go", || !w.join("target").exists());
    assert!(w.join("src.txt").is_file());
}

/// Ignores `pattern` in `main` and every tree of it, as a repo's own
/// `.gitignore` would, without a commit.
fn ignore(main: &Path, pattern: &str) {
    let f = main.join(".git/info/exclude");
    let mut text = fs::read_to_string(&f).unwrap_or_default();
    text.push_str(pattern);
    text.push('\n');
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(f, text).unwrap();
}

/// Finding 1: an ignored file that isn't build output is work.
#[test]
fn an_ignored_file_that_isnt_build_output_keeps_the_tree() {
    let (_d, main, w) = setup("ignored-env");
    ignore(&main, ".env");
    ignore(&main, "scratch/");
    fs::write(w.join(".env"), "TOKEN=mine\n").unwrap();
    let v = judge(&w);
    assert!(is_work(&v), "{v:?}");
    assert!(
        matches!(&v, Verdict::Work(why) if why.contains(".env")),
        "{v:?}"
    );
    assert_eq!(fs::read_to_string(w.join(".env")).unwrap(), "TOKEN=mine\n");
    fs::remove_file(w.join(".env")).unwrap();
    fs::create_dir_all(w.join("scratch")).unwrap();
    fs::write(w.join("scratch/plan.md"), "mine\n").unwrap();
    assert!(is_work(&judge(&w)));
    assert!(w.join("scratch/plan.md").is_file());
}

/// Finding 1's other half: build output and fetched packages still go.
#[test]
fn ignored_build_output_and_packages_go_with_the_tree() {
    let (_d, main, w) = setup("ignored-build");
    ignore(&main, "node_modules/");
    ignore(&main, "/frontend/dist/");
    ignore(&main, "/app/gen/*");
    ignore(&main, "__pycache__/");
    git_target(&w);
    package_json(&w, "frontend");
    for f in [
        "frontend/node_modules/x/index.js",
        "frontend/dist/app.js",
        "app/gen/schemas/capabilities.json",
        "tools/__pycache__/a.pyc",
    ] {
        fs::create_dir_all(w.join(f).parent().unwrap()).unwrap();
        fs::write(w.join(f), "x").unwrap();
    }
    assert_eq!(judge(&w), Verdict::Removed);
    assert!(!w.exists());
}

#[test]
fn only_known_build_folders_count_as_regenerable() {
    let d = tempfile::tempdir().unwrap();
    let t = d.path();
    for dir in ["target", "crates/x/target"] {
        fs::create_dir_all(t.join(dir)).unwrap();
        fs::write(t.join(dir).join("CACHEDIR.TAG"), "Signature: x\n").unwrap();
    }
    fs::create_dir_all(t.join("frontend")).unwrap();
    fs::write(t.join("frontend/package.json"), "{}\n").unwrap();
    for p in [
        "target/",
        "crates/x/target/",
        "crates/x/target/debug/",
        "frontend/node_modules/",
        "frontend/node_modules/x/index.js",
        "frontend/dist/",
        "frontend/dist/app.js",
        "app/gen/schemas/",
        "app/gen/schemas/capabilities.json",
        "target/debug/x",
        "target.irori-clearing/",
        "a/__pycache__/",
    ] {
        assert!(regenerable(t, p), "{p}");
    }
    for p in [
        ".env",
        "target",
        "notes.local",
        "scratch/",
        "dist/",
        "src/dist/",
        "app/gen/",
        "app/gen/apple/project.yml",
        "my-target/",
        ".claude/",
        ".claude/worktrees/inner/",
        // Finding B: by name alone they aren't build output.
        "docs/target/",
        "docs/target/notes.md",
        "node_modules/",
        "lib/node_modules/x.js",
        "a/target.irori-clearing/",
        "../target/",
    ] {
        assert!(!regenerable(t, p), "{p}");
    }
}

/// Card #565 finding B: an ignored folder named `target` that cargo didn't
/// make (no `CACHEDIR.TAG`), or a `node_modules` with no `package.json`
/// beside it, is work, not build output.
#[test]
fn a_hand_made_target_or_node_modules_keeps_the_tree() {
    let (_d, main, w) = setup("hand-target");
    ignore(&main, "target/");
    fs::create_dir_all(w.join("docs/target")).unwrap();
    fs::write(w.join("docs/target/notes.md"), "mine\n").unwrap();
    let v = judge(&w);
    assert!(
        matches!(&v, Verdict::Work(why) if why.contains("docs/target")),
        "{v:?}"
    );
    assert!(w.join("docs/target/notes.md").is_file());
    // A cargo target beside it is fine; the hand-made one still keeps it.
    git_target(&w);
    assert!(is_work(&judge(&w)));
    fs::remove_dir_all(w.join("docs")).unwrap();
    assert_eq!(judge(&w), Verdict::Removed);

    let (_d, main, w) = setup("hand-modules");
    ignore(&main, "node_modules/");
    fs::create_dir_all(w.join("node_modules")).unwrap();
    fs::write(w.join("node_modules/plan.md"), "mine\n").unwrap();
    let v = judge(&w);
    assert!(
        matches!(&v, Verdict::Work(why) if why.contains("node_modules")),
        "{v:?}"
    );
    assert!(w.join("node_modules/plan.md").is_file());
    package_json(&w, ".");
    assert_eq!(judge(&w), Verdict::Removed);
}

/// Card #565 finding C: with no HEAD reflog, which commits only the tree
/// holds can't be known, so it stays and says why.
#[test]
fn a_tree_with_no_head_reflog_stays() {
    let (_d, _main, w) = setup("no-reflog");
    let log = PathBuf::from(git(&w, &["rev-parse", "--git-path", "logs/HEAD"]));
    let log = if log.is_absolute() { log } else { w.join(log) };
    assert!(log.is_file(), "{}", log.display());
    fs::remove_file(&log).unwrap();
    let v = judge(&w);
    assert!(
        matches!(&v, Verdict::Work(why) if why.contains("reflog")),
        "{v:?}"
    );
    assert!(w.join("src.txt").is_file() && w.join(".git").is_file());
}

/// Finding 3: `status.showUntrackedFiles=no` hides nothing from the gate.
#[test]
fn untracked_work_stays_seen_when_the_repo_hides_untracked_files() {
    let (_d, main, w) = setup("hidden-untracked");
    git(&main, &["config", "status.showUntrackedFiles", "no"]);
    fs::create_dir_all(w.join("new")).unwrap();
    fs::write(w.join("new/work.rs"), "fn main() {}\n").unwrap();
    assert!(
        git(&w, &["status", "--porcelain"]).is_empty(),
        "git hides it"
    );
    assert!(is_work(&judge(&w)));
    assert!(w.join("new/work.rs").is_file());
}

/// Finding 2: a repo or a worktree inside the tree is never deleted with
/// it, ignored or not.
#[test]
fn a_repo_or_worktree_inside_the_tree_keeps_it() {
    // Another repo, in ignored build output so status can't see it.
    let (_d, main, w) = setup("nested-repo");
    ignore(&main, "node_modules/");
    package_json(&w, ".");
    let inner = w.join("node_modules/lib");
    fs::create_dir_all(&inner).unwrap();
    git(&inner, &["init", "-q"]);
    fs::write(inner.join("work.txt"), "uncommitted\n").unwrap();
    let v = judge(&w);
    assert!(
        matches!(&v, Verdict::Work(why) if why.contains("inside")),
        "{v:?}"
    );
    assert!(inner.join("work.txt").is_file());

    // Another worktree of the same repo, in the tree's own ignored target/.
    let (_d, main, w) = setup("nested-tree");
    git_target(&w);
    let inner = w.join("target/inner");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "inner",
            &inner.to_string_lossy(),
        ],
    );
    fs::write(inner.join("work.txt"), "uncommitted\n").unwrap();
    let v = judge(&w);
    assert!(
        matches!(&v, Verdict::Work(why) if why.contains("inside")),
        "{v:?}"
    );
    assert!(inner.join("work.txt").is_file());
    assert!(w.join("src.txt").is_file());
}

/// Finding 4: a commit only the tree's HEAD reflog reaches stays.
#[test]
fn a_commit_only_the_trees_history_reaches_keeps_it() {
    let (_d, _main, w) = setup("reset-away");
    commit(&w, "lost.txt");
    let lost = git(&w, &["rev-parse", "HEAD"]);
    git(&w, &["reset", "-q", "--hard", "HEAD~1"]);
    // HEAD is on origin/main now: the old gate let this go.
    assert!(!git(&w, &["branch", "-r", "--contains", "HEAD"]).is_empty());
    let v = judge(&w);
    assert!(
        matches!(&v, Verdict::Work(why) if why.contains("remote")),
        "{v:?}"
    );
    git(
        &w,
        &["push", "-q", "origin", &format!("{lost}:refs/heads/saved")],
    );
    assert_eq!(judge(&w), Verdict::Removed);

    // A detached commit left behind by a checkout.
    let (_d, _main, w) = setup("detached-left");
    git(&w, &["checkout", "-q", "--detach"]);
    commit(&w, "left.txt");
    git(&w, &["checkout", "-q", "detached-left"]);
    assert!(is_work(&judge(&w)));
}

/// Finding 5: a tree a removal left at its aside name is moved back, by
/// git, not lost to every later look.
#[test]
fn a_tree_left_at_its_aside_name_is_moved_back() {
    let (_d, main, w) = setup("stranded");
    fs::write(w.join("src.txt"), "changed\n").unwrap();
    let aside = strand(&main, &w);
    assert!(is_work(&judge(&w)), "back, and judged");
    assert!(!aside.exists());
    assert_eq!(fs::read_to_string(w.join("src.txt")).unwrap(), "changed\n");
    assert!(git(&main, &["worktree", "list"]).contains("stranded"));
    git(&w, &["checkout", "--", "src.txt"]);
    strand(&main, &w);
    assert_eq!(judge(&w), Verdict::Removed);
    assert!(!aside.exists() && !w.exists());
    assert!(git(&main, &["branch", "--list", "stranded"]).contains("stranded"));

    // Both there: nothing is touched.
    let (_d, _main, w) = setup("both");
    let aside = aside_path(&w);
    fs::create_dir_all(&aside).unwrap();
    fs::write(aside.join("x"), "x").unwrap();
    assert!(matches!(judge(&w), Verdict::Busy(_)));
    assert!(aside.join("x").is_file() && w.join("src.txt").is_file());

    // A plain folder at the aside name, no worktree: left alone.
    let (_d, _main, w) = setup("plain-aside");
    let aside = aside_path(&w);
    fs::rename(&w, &aside).unwrap();
    assert!(matches!(judge(&w), Verdict::Busy(_)));
    assert!(aside.join("src.txt").is_file() && !w.exists());
}

/// Card #565 finding D: the last checks and the removal run on the tree
/// moved aside by git, so nothing new can start at its path meanwhile, and
/// a tree whose last checks fail is moved back whole, still a worktree.
#[test]
fn the_last_checks_run_aside_and_a_failing_one_moves_it_back() {
    use std::sync::Mutex;
    let (_d, main, w) = setup("late-work");
    let seen: Mutex<Option<bool>> = Mutex::new(None);
    let path = w.clone();
    // The recheck is the last word before the move: work made then is
    // found on the moved tree, and the tree comes back.
    let recheck = |_: &str| -> Result<(), String> {
        fs::write(path.join("late.txt"), "mine\n").unwrap();
        Ok(())
    };
    let v = remove(
        &w,
        &Check {
            live: &[],
            idle: HOUR,
            now: later(),
            recheck: Some(&recheck),
        },
    );
    assert!(is_work(&v), "{v:?}");
    assert!(w.join("late.txt").is_file() && w.join(".git").is_file());
    assert!(!aside_path(&w).exists());
    assert!(git(&w, &["status", "--porcelain"]).contains("late.txt"));
    fs::remove_file(w.join("late.txt")).unwrap();

    // While the checks run, the old path is gone: a removal that passes
    // leaves neither name, and keeps the branch.
    let watch = w.clone();
    let check_gone = |_: &str| -> Result<(), String> {
        *seen.lock().unwrap() = Some(watch.exists());
        Ok(())
    };
    let v = remove(
        &w,
        &Check {
            live: &[],
            idle: HOUR,
            now: later(),
            recheck: Some(&check_gone),
        },
    );
    assert_eq!(v, Verdict::Removed);
    assert_eq!(*seen.lock().unwrap(), Some(true), "rechecked at its name");
    assert!(!w.exists() && !aside_path(&w).exists());
    assert!(!git(&main, &["worktree", "list"]).contains("late-work"));
    assert!(git(&main, &["branch", "--list", "late-work"]).contains("late-work"));
}

/// Card #565 finding D: a reopened card's tree that a removal stranded
/// aside is moved back for it, on its branch, with what was in it.
#[test]
fn reopening_a_card_moves_its_stranded_tree_back() {
    let (_d, main, w) = setup("reopen-aside");
    fs::write(w.join("src.txt"), "changed\n").unwrap();
    let aside = strand(&main, &w);
    reopen(&main, &w, "reopen-aside").unwrap();
    assert!(!aside.exists() && w.join(".git").is_file());
    assert_eq!(git(&w, &["branch", "--show-current"]), "reopen-aside");
    assert_eq!(fs::read_to_string(w.join("src.txt")).unwrap(), "changed\n");
    // A removal git has begun on isn't a tree to reopen.
    let (_d, main, w) = setup("reopen-begun");
    strand(&main, &w);
    fs::write(begun_path(&w), "").unwrap();
    assert!(reopen(&main, &w, "reopen-begun").is_err());
    assert!(aside_path(&w).join(".git").is_file());
}

fn link_dir(link: &Path, to: &Path) {
    #[cfg(windows)]
    {
        let out = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(to.to_string_lossy().replace('/', "\\"))
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(to, link).unwrap();
}

/// Finding 9: a link (a junction on Windows) anywhere in the tree keeps
/// it, and nothing behind the link is touched.
#[test]
fn a_link_in_the_tree_keeps_it_and_what_it_points_at() {
    let (d, main, w) = setup("linked");
    let shared = d.path().join("shared");
    fs::create_dir_all(&shared).unwrap();
    fs::write(shared.join("keep.txt"), "shared\n").unwrap();
    // In ignored build output, so only the walk can see it.
    ignore(&main, "node_modules/");
    package_json(&w, ".");
    fs::create_dir_all(w.join("node_modules")).unwrap();
    link_dir(&w.join("node_modules/shared"), &shared);
    let v = judge(&w);
    assert!(
        matches!(&v, Verdict::Work(why) if why.contains("link")),
        "{v:?}"
    );
    assert!(shared.join("keep.txt").is_file());
    assert!(w.join("src.txt").is_file() && w.join(".git").is_file());

    // A target/ that is a link to a shared build folder.
    let (d, _main, w) = setup("linked-target");
    let builds = d.path().join("builds");
    fs::create_dir_all(builds.join("debug")).unwrap();
    fs::write(
        builds.join("CACHEDIR.TAG"),
        "Signature: 8a477f597d28d172789f06886806bc55\n",
    )
    .unwrap();
    fs::write(builds.join("debug/libshared.rlib"), "x").unwrap();
    link_dir(&w.join("target"), &builds);
    let v = judge(&w);
    assert!(
        matches!(&v, Verdict::Work(why) if why.contains("link")),
        "{v:?}"
    );
    assert_eq!(
        irori_core::buildclean::clear(&w, Duration::ZERO, later()),
        Ok(irori_core::buildclean::Cleared::Kept(
            irori_core::buildclean::TARGET_LINK
        ))
    );
    assert!(builds.join("debug/libshared.rlib").is_file());
}

/// Finding 7: the last look before the folder moves can still keep it.
#[test]
fn the_last_look_keeps_a_tree_whose_card_came_back() {
    let (_d, _main, w) = setup("came-back");
    git_target(&w);
    let asked = std::sync::atomic::AtomicUsize::new(0);
    let no = |_: &str| -> Result<(), String> {
        asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err("a card that works in it isn't settled any more".into())
    };
    let c = Check {
        live: &[],
        idle: HOUR,
        now: later(),
        recheck: Some(&no),
    };
    assert!(matches!(remove(&w, &c), Verdict::Busy(why) if why.contains("settled")));
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(w.join("src.txt").is_file() && w.join(".git").is_file());
    let yes = |_: &str| -> Result<(), String> { Ok(()) };
    let c = Check {
        recheck: Some(&yes),
        ..c
    };
    assert_eq!(remove(&w, &c), Verdict::Removed);
}

/// Finding 8: a line about one tree doesn't count as saying another's.
#[test]
fn a_line_names_a_tree_only_by_its_whole_name() {
    assert!(names_tree(
        "Irori kept: worktree kept at C:\\r\\.claude\\worktrees\\card-5-1 (unpushed)",
        "c:/r/.claude/worktrees/card-5-1"
    ));
    assert!(names_tree(
        "kept /r/.claude/worktrees/card-5-1.",
        "/r/.claude/worktrees/card-5-1"
    ));
    assert!(!names_tree(
        "kept /r/.claude/worktrees/card-5-12 (unpushed)",
        "/r/.claude/worktrees/card-5-1"
    ));
    assert!(!names_tree(
        "kept /r/.claude/worktrees/xcard-5-1",
        "/r/.claude/worktrees/card-5-1"
    ));
    assert!(!names_tree(
        "kept /r/.claude/worktrees/card-5-1.irori-removing",
        "/r/.claude/worktrees/card-5-1"
    ));
}

#[test]
fn kept_is_said_for_each_tree_of_a_card() {
    let r = Rig::new();
    let t = settled_card(&r, "Two trees", None);
    let co = r.core.coord();
    say_kept(co, &t.id, "/r/.claude/worktrees/card-5-1", "it has 2 files");
    say_kept(
        co,
        &t.id,
        "/r/.claude/worktrees/card-5-12",
        "it has 1 commit",
    );
    say_kept(co, &t.id, "/r/.claude/worktrees/card-5-1", "it has 2 files");
    assert_eq!(kept_lines(&r, &t.id), 2);
}

/// Finding 10: where a process's folder can be listed (Linux, macOS),
/// one in the tree keeps it.
#[test]
fn a_folder_list_finds_a_process_in_the_tree() {
    let text = "p123\nfcwd\nn/r/.claude/worktrees/x/crates\np9\nfcwd\nn/home\n";
    let cwds = lsof_cwds(text);
    assert_eq!(cwds, ["/r/.claude/worktrees/x/crates", "/home"]);
    assert!(any_under(&cwds, "/r/.claude/worktrees/x"));
    assert!(!any_under(&cwds, "/r/.claude/worktrees/y"));
}

#[cfg(unix)]
#[test]
fn a_process_sitting_in_the_tree_keeps_it_on_unix() {
    let (_d, _main, w) = setup("in-use-unix");
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .current_dir(w.join("."))
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let v = judge(&w);
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        matches!(&v, Verdict::Busy(why) if why.starts_with("held by sleep (pid ")),
        "{v:?}"
    );
    assert!(w.join("src.txt").is_file());
    assert_eq!(judge(&w), Verdict::Removed);
}

/// Card #1118: the holder is named, found by `holders`, and `end_holder`
/// ends it (a fake holder: `sleep` with its folder in the tree), after
/// which the tree goes. A pid outside the tree is refused.
#[cfg(unix)]
#[test]
fn a_holder_is_named_and_can_be_ended_on_unix() {
    let (_d, _main, w) = setup("holder-unix");
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .current_dir(&w)
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let tree = w.to_string_lossy().into_owned();
    let h = holders(&tree).unwrap();
    assert_eq!(h.len(), 1, "{h:?}");
    assert_eq!(h[0].pid, child.id());
    assert_eq!(h[0].name, "sleep");
    assert_eq!(held_by(&h), format!("held by sleep (pid {})", child.id()));
    assert!(end_holder(&tree, std::process::id()).is_err());
    let other = tempfile::tempdir().unwrap();
    assert!(end_holder(&other.path().to_string_lossy(), child.id()).is_err());
    let ended = end_holder(&tree, child.id()).unwrap();
    assert_eq!(ended.pid, child.id());
    child.wait().unwrap();
    assert!(holders(&tree).unwrap().is_empty());
    assert_eq!(judge(&w), Verdict::Removed);
}

fn lister_with(f: impl Fn() -> Vec<ClaudeAgent> + Send + Sync + 'static) -> Spawner {
    Spawner {
        list: Some(Arc::new(move || Ok(f()))),
        ..lister()
    }
}

/// Finding 7 end to end: the card reopens while the pass runs, after its
/// first look; the pass's last look keeps the tree.
#[test]
fn a_card_reopened_while_the_pass_runs_keeps_its_tree() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("repo"));
    let w = git_tree(&main, "reopened");
    let r = Rig::with(|o| o.clear_builds = Some(Duration::ZERO));
    let t = settled_card(&r, "Reopened", Some(&w));
    let (co, by, id) = (r.core.coord().clone(), r.key_a.clone(), t.id.clone());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let n = calls.clone();
    r.core.set_spawner(Some(lister_with(move || {
        // The pass's first list, then the last look's: reopen between.
        if n.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            co.reopen(&by, &id).unwrap();
        }
        Vec::new()
    })));
    r.core.sweep_coord().unwrap();
    wait_for(45, "the last look", || {
        calls.load(std::sync::atomic::Ordering::SeqCst) >= 2
    });
    std::thread::sleep(Duration::from_secs(3));
    assert!(w.join("src.txt").is_file() && w.join(".git").is_file());
    assert_eq!(kept_lines(&r, &t.id), 0, "busy, not kept for work");
}

/// Finding 7: an agent that starts in the tree after the pass's first list
/// is seen by the last look.
#[test]
fn an_agent_that_starts_in_the_tree_mid_pass_keeps_it() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("repo"));
    let w = git_tree(&main, "joined");
    let r = Rig::with(|o| o.clear_builds = Some(Duration::ZERO));
    settled_card(&r, "Joined", Some(&w));
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (n, cwd) = (calls.clone(), w.to_string_lossy().into_owned());
    r.core.set_spawner(Some(lister_with(move || {
        if n.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            return Vec::new();
        }
        vec![ClaudeAgent {
            session_id: "s".into(),
            live: true,
            cwd: cwd.clone(),
        }]
    })));
    r.core.sweep_coord().unwrap();
    wait_for(45, "the last look", || {
        calls.load(std::sync::atomic::Ordering::SeqCst) >= 2
    });
    std::thread::sleep(Duration::from_secs(2));
    assert!(w.join("src.txt").is_file() && w.join(".git").is_file());
}

/// Finding 11: a live agent `claude agents` can't place keeps every tree.
#[test]
fn a_live_agent_with_no_folder_keeps_every_tree() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("repo"));
    let w = git_tree(&main, "unplaced");
    git_target(&w);
    let r = Rig::with(|o| o.clear_builds = Some(Duration::ZERO));
    settled_card(&r, "Unplaced", Some(&w));
    r.core.set_spawner(Some(lister_with(|| {
        vec![ClaudeAgent {
            session_id: "s".into(),
            live: true,
            cwd: String::new(),
        }]
    })));
    r.core.sweep_coord().unwrap();
    wait_for(45, "its target/ to go", || !w.join("target").exists());
    assert!(w.join("src.txt").is_file() && w.join(".git").is_file());
}

/// Finding 8 for `claude rm`'s kept lines: another tree's line on the same
/// card is said; the same line again isn't.
#[test]
fn a_kept_worktree_line_is_said_once_per_line_not_once_per_card() {
    use irori_core::spawn::{KEPT_WORKTREE, kept_said};
    let first = format!("{KEPT_WORKTREE}kept /r/.claude/worktrees/card-5-1: 2 unpushed commits");
    let second = format!("{KEPT_WORKTREE}kept /r/.claude/worktrees/card-5-2: 1 unpushed commit");
    let comment = |by: &str, text: &str| {
        let mut c = coord::Comment::default();
        c.by = by.into();
        c.text = text.into();
        c
    };
    let on_card = [comment("irori", &first)];
    assert!(kept_said(&on_card, &first));
    assert!(!kept_said(&on_card, &second));
    let by_someone = [comment("user:x", &first)];
    assert!(!kept_said(&by_someone, &first));
}

/// Finding 9's limit: on Linux and macOS a symlink to somewhere inside the
/// tree (`node_modules/.bin`) goes with it; one pointing out keeps it.
#[cfg(unix)]
#[test]
fn a_symlink_within_the_tree_goes_with_it_on_unix() {
    let (_d, main, w) = setup("bin-links");
    ignore(&main, "node_modules/");
    package_json(&w, ".");
    fs::create_dir_all(w.join("node_modules/pkg/bin")).unwrap();
    fs::write(w.join("node_modules/pkg/bin/x"), "#!/bin/sh\n").unwrap();
    fs::create_dir_all(w.join("node_modules/.bin")).unwrap();
    std::os::unix::fs::symlink("../pkg/bin/x", w.join("node_modules/.bin/x")).unwrap();
    assert_eq!(judge(&w), Verdict::Removed);
}

/// Finding 4's message: a pushed HEAD with reset-away history says so.
#[test]
fn a_pushed_head_with_history_left_behind_says_which() {
    let (_d, _main, w) = setup("amended");
    commit(&w, "a.txt");
    git(&w, &["reset", "-q", "--hard", "HEAD~1"]);
    let v = judge(&w);
    assert!(
        matches!(&v, Verdict::Work(why) if why.contains("HEAD is pushed")),
        "{v:?}"
    );
    let (_d, _main, w) = setup("ahead-plain");
    commit(&w, "a.txt");
    let v = judge(&w);
    assert!(
        matches!(&v, Verdict::Work(why) if !why.contains("HEAD is pushed")),
        "{v:?}"
    );
}

/// `remove` with `during` run as its last look, after the clear: a stand-in
/// for whatever happens in the tree while a big `target/` is deleted.
fn judge_with(tree: &Path, during: impl Fn() + Sync) -> Verdict {
    let f = |_: &str| -> Result<(), String> {
        during();
        Ok(())
    };
    remove(
        tree,
        &Check {
            live: &[],
            idle: HOUR,
            now: later(),
            recheck: Some(&f),
        },
    )
}

/// Review: the work gates run again after the long clear, so work written
/// while it ran (an ignored file, a detached commit) keeps the tree.
#[test]
fn work_made_while_the_build_clears_keeps_the_tree() {
    let (_d, main, w) = setup("late-notes");
    git_target(&w);
    ignore(&main, "notes.local");
    let v = judge_with(&w, || fs::write(w.join("notes.local"), "mine\n").unwrap());
    assert!(
        matches!(&v, Verdict::Work(why) if why.contains("notes.local")),
        "{v:?}"
    );
    assert_eq!(fs::read_to_string(w.join("notes.local")).unwrap(), "mine\n");
    assert!(!w.join("target").exists(), "the build still went");

    let (_d, _main, w) = setup("late-commit");
    let v = judge_with(&w, || {
        git(&w, &["checkout", "-q", "--detach"]);
        commit(&w, "late.txt");
    });
    assert!(
        matches!(&v, Verdict::Work(why) if why.contains("remote")),
        "{v:?}"
    );
    assert!(w.join("late.txt").is_file());
}

/// Review: no `git worktree prune`, so a worktree the person made whose
/// folder is missing right now (an unmounted drive) keeps its entry, and
/// so does a tree of ours stranded at its aside name.
#[test]
fn removing_a_tree_never_prunes_another_entry() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("repo"));
    let theirs = d.path().join("elsewhere/mine");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "mine",
            &theirs.to_string_lossy(),
        ],
    );
    let gone = d.path().join("unmounted");
    fs::rename(d.path().join("elsewhere"), &gone).unwrap();
    let stuck = git_tree(&main, "stuck");
    strand(&main, &stuck);
    let done = git_tree(&main, "done-one");
    assert_eq!(judge(&done), Verdict::Removed);
    assert!(main.join(".git/worktrees/mine").is_dir(), "theirs kept");
    assert!(main.join(".git/worktrees/stuck").is_dir(), "ours kept");
    // Theirs comes back whole; ours is moved back and is a worktree still.
    fs::rename(&gone, d.path().join("elsewhere")).unwrap();
    assert!(git(&theirs, &["status", "--porcelain"]).is_empty());
    assert_eq!(judge(&stuck), Verdict::Removed);
}

/// Review: a change git is told not to look at shows in no status, so a
/// skip-worktree or assume-unchanged file keeps the tree.
#[test]
fn a_file_git_is_told_to_ignore_the_changes_of_keeps_the_tree() {
    for flag in ["--skip-worktree", "--assume-unchanged"] {
        let (_d, _main, w) = setup("hidden");
        git(&w, &["update-index", flag, "src.txt"]);
        fs::write(w.join("src.txt"), "mine\n").unwrap();
        assert!(git(&w, &["status", "--porcelain"]).is_empty());
        let v = judge(&w);
        assert!(
            matches!(&v, Verdict::Work(why) if why.contains("src.txt")),
            "{flag}: {v:?}"
        );
        assert_eq!(fs::read_to_string(w.join("src.txt")).unwrap(), "mine\n");
    }
}

/// Review: refs that live in the tree's own admin folder go with it, so a
/// commit only one of them holds keeps the tree.
#[test]
fn a_commit_only_the_trees_own_refs_hold_keeps_it() {
    for r in ["refs/worktree/checkpoint", "refs/bisect/bad"] {
        let (_d, _main, w) = setup("own-ref");
        let tree = git(&w, &["rev-parse", "HEAD^{tree}"]);
        let lone = git(
            &w,
            &["commit-tree", &tree, "-p", "HEAD", "-m", "checkpoint"],
        );
        git(&w, &["update-ref", r, &lone]);
        let v = judge(&w);
        assert!(
            matches!(&v, Verdict::Work(why) if why.contains("own refs")),
            "{r}: {v:?}"
        );
        git(
            &w,
            &["push", "-q", "origin", &format!("{lone}:refs/heads/kept")],
        );
        assert_eq!(judge(&w), Verdict::Removed, "{r}");
    }
}

/// Builds what git leaves when its delete fails part way: the tree aside,
/// entry and `.git` file gone, some files left, the mark written before
/// git began.
fn half_removed(name: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let (d, main, w) = setup(name);
    fs::create_dir_all(w.join("left/deep")).unwrap();
    fs::write(w.join("left/deep/file.o"), "x").unwrap();
    let aside = strand(&main, &w);
    std::thread::sleep(Duration::from_millis(50));
    fs::write(begun_path(&w), "").unwrap();
    fs::remove_dir_all(main.join(".git/worktrees").join(name)).unwrap();
    fs::remove_file(aside.join(".git")).unwrap();
    fs::remove_file(aside.join("src.txt")).unwrap();
    (d, main, w)
}

/// Review: a removal git left half done is finished by the next look, not
/// called "not a worktree any more" and kept for good.
#[test]
fn a_removal_git_left_half_done_is_finished() {
    let (_d, main, w) = half_removed("half");
    assert_eq!(judge(&w), Verdict::Removed);
    assert!(!w.exists() && !aside_path(&w).exists() && !begun_path(&w).exists());
    assert!(git(&main, &["branch", "--list", "half"]).contains("half"));

    // Something written there since: left, and said.
    let (_d, _main, w) = half_removed("half-new");
    std::thread::sleep(Duration::from_millis(50));
    let aside = aside_path(&w);
    fs::write(aside.join("new.txt"), "mine\n").unwrap();
    let v = judge(&w);
    assert!(
        matches!(&v, Verdict::Broken(why) if why.contains("new.txt")),
        "{v:?}"
    );
    assert!(aside.join("new.txt").is_file());
    // And a pass that puts trees back leaves it be.
    let home = aside.parent().unwrap().to_path_buf();
    assert_eq!(restore_all(&home).len(), 1);
    assert!(aside.join("new.txt").is_file() && !w.exists());

    // A mark beside a tree git never began on: dropped, the tree moved
    // back and judged.
    let (_d, main, w) = setup("never-began");
    strand(&main, &w);
    fs::write(begun_path(&w), "").unwrap();
    assert_eq!(judge(&w), Verdict::Removed);
    assert!(!begun_path(&w).exists() && !aside_path(&w).exists());
    let (_d, _main, w) = setup("never-began-here");
    fs::write(begun_path(&w), "").unwrap();
    assert_eq!(judge(&w), Verdict::Removed);
    assert!(!begun_path(&w).exists());
}

/// Review: a tree left at its aside name is moved back on the next pass
/// whatever its card's state, not only while its card is settled.
#[test]
fn every_tree_left_at_its_aside_name_is_moved_back() {
    let (_d, main, w) = setup("reopened-probe");
    let aside = strand(&main, &w);
    let home = main.join(".claude/worktrees");
    assert!(restore_all(&home).is_empty());
    assert!(w.join(".git").is_file() && !aside.exists());
    // A mark left beside a tree that is gone goes; one beside a tree stays.
    let orphan = begun_path(&home.join("gone-already"));
    fs::write(&orphan, "").unwrap();
    fs::write(begun_path(&w), "").unwrap();
    assert!(restore_all(&home).is_empty());
    assert!(!orphan.exists() && begun_path(&w).exists());
    fs::remove_file(begun_path(&w)).unwrap();

    // End to end: an open card's tree, stuck aside, is put back by a sweep.
    let r = Rig::with(|o| o.clear_builds = Some(Duration::ZERO));
    let co = r.core.coord();
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "Open".into(),
                objective: "why".into(),
                ..coord::NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap();
    let p: String = w.to_string_lossy().into();
    co.set_where(
        &t.id,
        coord::Where {
            who: r.key_a.clone(),
            cwd: p.clone(),
            worktree: p,
            branch: "reopened-probe".into(),
            ..coord::Where::default()
        },
    )
    .unwrap();
    strand(&main, &w);
    r.core.sweep_coord().unwrap();
    wait_for(45, "the tree to be moved back", || {
        w.join(".git").is_file() && !aside.exists()
    });
}

/// Review: a tree found in use is said on its card after a few looks in a
/// row, then looked at less and less often.
#[test]
fn a_tree_in_use_look_after_look_is_said_and_backs_off() {
    for n in 1..BUSY_SAY {
        assert_eq!(busy_plan(n), (false, None), "{n}");
    }
    let (say, first) = busy_plan(BUSY_SAY);
    assert!(say);
    let first = first.unwrap();
    let (_, second) = busy_plan(BUSY_SAY + 1);
    assert_eq!(second.unwrap(), first * 2);
    assert_eq!(busy_plan(BUSY_SAY + 30).1, Some(KEPT_EVERY));
}

/// Review: a Git Bash (MSYS) process opens its folder so it can still be
/// renamed, so the rename can't see it; Git's own `/proc` list does.
#[cfg(windows)]
#[test]
fn a_git_bash_shell_sitting_in_the_tree_keeps_it() {
    let exec = git(Path::new("."), &["--exec-path"]);
    let sh = Path::new(&exec)
        .ancestors()
        .nth(3)
        .unwrap()
        .join("usr/bin/sh.exe");
    if !sh.is_file() {
        eprintln!("no Git sh here: skipped");
        return;
    }
    let (_d, _main, w) = setup("bash-in-it");
    let mut child = std::process::Command::new(&sh)
        .args(["-c", "exec sleep 30"])
        .current_dir(&w)
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let v = judge(&w);
    let h = holders(&w.to_string_lossy()).unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        matches!(&v, Verdict::Busy(why) if why.starts_with("held by sleep (pid ")),
        "{v:?}"
    );
    assert!(h.iter().any(|h| h.name == "sleep"), "{h:?}");
    assert!(w.join("src.txt").is_file());
}

/// Card #1118, the PC: the Git Bash `sleep` is named, with the Windows
/// pid, and `end_holder` ends it so the tree goes.
#[cfg(windows)]
#[test]
fn a_git_bash_holder_is_named_and_can_be_ended() {
    let exec = git(Path::new("."), &["--exec-path"]);
    let sh = Path::new(&exec)
        .ancestors()
        .nth(3)
        .unwrap()
        .join("usr/bin/sh.exe");
    if !sh.is_file() {
        eprintln!("no Git sh here: skipped");
        return;
    }
    let (_d, _main, w) = setup("bash-holder");
    let mut child = std::process::Command::new(&sh)
        .args(["-c", "exec sleep 30"])
        .current_dir(&w)
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let tree = w.to_string_lossy().into_owned();
    let h = holders(&tree).unwrap();
    // `exec` in MSYS starts the program as a process of its own, so its
    // Windows pid isn't the shell's.
    let one = h
        .iter()
        .find(|h| h.name == "sleep")
        .expect("sleep is named");
    assert!(end_holder(&tree, std::process::id()).is_err());
    assert_eq!(end_holder(&tree, one.pid).unwrap().pid, one.pid);
    child.wait().unwrap();
    assert!(holders(&tree).unwrap().is_empty());
    assert_eq!(judge(&w), Verdict::Removed);
}

/// Card #565, last check: a clone an agent made inside `target/` is work,
/// not build output. Neither the tree nor its `target/` goes, whether by
/// the tree's removal or by clearing build output alone.
#[test]
fn a_repo_cloned_into_target_keeps_the_tree_and_its_build() {
    let (_d, _main, w) = setup("clone-in-target");
    git_target(&w);
    let clone = w.join("target/tmp/upstream");
    fs::create_dir_all(&clone).unwrap();
    git(&clone, &["init", "-q", "-b", "main"]);
    fs::write(clone.join("edit.rs"), "mine\n").unwrap();
    let v = judge(&w);
    assert!(
        matches!(&v, Verdict::Work(why) if why.contains("target/tmp/upstream/.git")),
        "{v:?}"
    );
    assert!(clone.join("edit.rs").is_file());
    assert_eq!(
        irori_core::buildclean::clear(&w, HOUR, later()).unwrap(),
        irori_core::buildclean::Cleared::Kept(irori_core::buildclean::HOLDS_REPO)
    );
    assert!(clone.join("edit.rs").is_file());
    // A leftover half-cleared target holding one is kept too.
    fs::rename(w.join("target"), w.join("target.irori-clearing")).unwrap();
    assert!(is_work(&judge(&w)));
    assert!(
        w.join("target.irori-clearing/tmp/upstream/edit.rs")
            .is_file()
    );
    fs::remove_dir_all(w.join("target.irori-clearing")).unwrap();
    assert_eq!(judge(&w), Verdict::Removed);
}

/// A build script's `OUT_DIR` is cargo's: whisper-rs-sys copies
/// whisper.cpp there with its `.git`, and that must not keep every tree
/// that built voice. The same `.git` anywhere else in `target/` still
/// does (above).
#[test]
fn a_repo_a_build_script_copied_into_its_out_dir_is_build_output() {
    let (_d, _main, w) = setup("build-out-repo");
    git_target(&w);
    for out in [
        "target/debug/build/whisper-rs-sys-8b97011b/out/whisper.cpp",
        "target/x86_64-pc-windows-msvc/release/build/foo-1a2b/out",
    ] {
        let vendored = w.join(out);
        fs::create_dir_all(&vendored).unwrap();
        git(&vendored, &["init", "-q", "-b", "main"]);
    }
    assert!(irori_core::treeclean::in_build_out(Path::new(
        "debug/build/x-1/out/a/.git"
    )));
    assert!(!irori_core::treeclean::in_build_out(Path::new(
        "tmp/build/x/out/.git"
    )));
    assert!(!irori_core::treeclean::in_build_out(Path::new(
        "tmp/upstream/.git"
    )));
    assert!(!irori_core::treeclean::in_build_out(Path::new(
        "debug/build/x-1/.git"
    )));
    assert_eq!(judge(&w), Verdict::Removed);
    assert!(!w.exists());
}

/// A checkout made with `--separate-git-dir` has a `.git` file but is a
/// main checkout all the same; a linked worktree isn't.
#[test]
fn a_separate_git_dir_checkout_is_a_main_checkout() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("repo"));
    let sep = d.path().join("sep");
    git(
        d.path(),
        &[
            "init",
            "-q",
            "--separate-git-dir",
            &d.path().join("sep.git").to_string_lossy(),
            &sep.to_string_lossy(),
        ],
    );
    assert!(sep.join(".git").is_file());
    assert!(irori_core::treeclean::is_main_checkout(&sep));
    assert!(irori_core::treeclean::is_main_checkout(&main));
    let w = git_tree(&main, "linked");
    assert!(!irori_core::treeclean::is_main_checkout(&w));
    assert!(!irori_core::treeclean::is_main_checkout(d.path()));
}

/// Card #1118: a canceled card's tree, kept for work in it, is listed as
/// canceled and the person may delete it whole; a card still open's tree is
/// not, and is refused.
#[test]
fn a_canceled_cards_tree_can_be_deleted_anyway_and_an_open_ones_cannot() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("repo"));
    let (gone, open) = (git_tree(&main, "canceled"), git_tree(&main, "open"));
    fs::write(gone.join("notes.txt"), "mine\n").unwrap();
    fs::write(open.join("notes.txt"), "mine\n").unwrap();
    let r = Rig::new();
    r.core.set_spawner(Some(lister()));
    settled_card(&r, "Canceled", Some(&gone));
    // An open card: claimed, never settled.
    let co = r.core.coord();
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "Open".into(),
                objective: "why".into(),
                ..coord::NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap();
    let p: String = open.to_string_lossy().into();
    co.set_where(
        &t.id,
        coord::Where {
            who: r.key_a.clone(),
            cwd: p.clone(),
            worktree: p,
            branch: "b".into(),
            head: "1a2b3c4d5e".into(),
            ..coord::Where::default()
        },
    )
    .unwrap();
    let trees = r.core.build_trees();
    let of = |w: &Path| {
        trees
            .iter()
            .find(|t| t.tree == key(&w.to_string_lossy()))
            .unwrap()
            .clone()
    };
    assert!(of(&gone).canceled && of(&gone).settled);
    assert!(!of(&open).canceled && !of(&open).settled);
    let why = r
        .core
        .delete_canceled_tree(&open.to_string_lossy())
        .unwrap_err();
    assert!(why.contains("isn't canceled"), "{why}");
    assert!(open.join("notes.txt").is_file());
    r.core
        .delete_canceled_tree(&gone.to_string_lossy())
        .unwrap();
    assert!(!gone.exists());
    // The branch stays: only the folder went.
    assert!(!git(&main, &["branch", "--list", "canceled"]).is_empty());
}

/// Card #1118: a fake holder shows in the Core's tree list with its name,
/// and `end_tree_holder` ends it; a pid that isn't in the tree is refused.
#[cfg(unix)]
#[test]
fn the_tree_list_names_a_holder_and_ends_it() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("repo"));
    let w = git_tree(&main, "held");
    let r = Rig::new();
    settled_card(&r, "Held", Some(&w));
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .current_dir(&w)
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let t = r
        .core
        .build_trees()
        .into_iter()
        .find(|t| t.tree == key(&w.to_string_lossy()))
        .unwrap();
    assert_eq!(t.holders.len(), 1, "{t:?}");
    assert_eq!(t.holders[0].name, "sleep");
    assert!(
        r.core
            .end_tree_holder(&w.to_string_lossy(), std::process::id())
            .is_err()
    );
    r.core
        .end_tree_holder(&w.to_string_lossy(), child.id())
        .unwrap();
    child.wait().unwrap();
}
