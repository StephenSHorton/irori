//! Card #565 at the start of an agent: a fresh card gets a tree of its own
//! that Irori makes and its card owns (never `claude -w`, so `claude rm`
//! never owns it), builds land in it, and nothing new starts in a dead
//! job's temp clone.

use crate::common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use common::*;
use coord::{NewTask, Where};
use irori_core::spawn::{BUILD_OUTPUT, SpawnReq, SpawnSetting, SpawnStarted, Spawner};
use irori_core::treeclean::{Book, Check, Verdict, key, remove};

const K: &str = "remote:github.com/acme/app";
const SESS: &str = "eeeeeeee-5555-4555-8555-000000000005";

#[derive(Default)]
struct Fake {
    started: Mutex<Vec<SpawnReq>>,
    /// Whether the folder was a git worktree when `claude` was asked to
    /// start in it.
    was_tree: Mutex<Vec<bool>>,
}

fn spawner(f: &Arc<Fake>, sessions: PathBuf) -> Spawner {
    let a = f.clone();
    Spawner {
        spawn: Arc::new(move |r: &SpawnReq| {
            a.was_tree
                .lock()
                .unwrap()
                .push(Path::new(&r.folder).join(".git").is_file());
            a.started.lock().unwrap().push(r.clone());
            write_session(
                &sessions,
                "900005.json",
                SESS,
                &r.name,
                Path::new(&r.folder),
            );
            Ok(SpawnStarted {
                name: r.name.clone(),
                prefix: SESS[..8].into(),
            })
        }),
        end: Arc::new(|_| Ok(String::new())),
        trusted: Arc::new(|_| true),
        list: None,
        ready: None,
    }
}

fn see_root(r: &Rig, root: &Path) {
    r.core
        .capture()
        .emit_row(
            "irori",
            capture::KIND_REPO_SEEN,
            &capture::RepoSeen {
                repo_key: K.into(),
                root: root.to_string_lossy().into(),
                at: gocompat::time::GoTime::unix(1_790_000_000, 0),
            },
        )
        .unwrap();
}

/// Starts an agent for a new card of repo K and gives its request.
fn start(r: &Rig, place: Option<&Path>) -> (coord::Task, SpawnReq) {
    let (t, req, _) = start_with(r, place);
    (t, req)
}

/// [`start`], with the fake spawner too.
fn start_with(r: &Rig, place: Option<&Path>) -> (coord::Task, SpawnReq, Arc<Fake>) {
    let w = place.map(|p| Where {
        cwd: p.to_string_lossy().into(),
        worktree: p.to_string_lossy().into(),
        ..Where::default()
    });
    start_at_with(r, w)
}

/// [`start`] with the card's Where as given (its `who` filled in).
fn start_at(r: &Rig, place: Option<Where>) -> (coord::Task, SpawnReq) {
    let (t, req, _) = start_at_with(r, place);
    (t, req)
}

fn start_at_with(r: &Rig, place: Option<Where>) -> (coord::Task, SpawnReq, Arc<Fake>) {
    let fake = Arc::new(Fake::default());
    let me = r.core.device_id().to_string();
    let co = r.core.coord();
    let t = co
        .create_task(
            &format!("user:{me}"),
            NewTask {
                title: "App card".into(),
                repo_key: K.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    if let Some(w) = place {
        co.set_where(
            &t.id,
            Where {
                who: format!("{me}/{SESS}"),
                ..w
            },
        )
        .unwrap();
    }
    r.core
        .set_spawner(Some(spawner(&fake, r.claude.join("sessions"))));
    r.core
        .set_spawn_setting(SpawnSetting {
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    wait_for(60, "the agent started", || {
        !fake.started.lock().unwrap().is_empty()
    });
    let req = fake.started.lock().unwrap()[0].clone();
    (t, req, fake)
}

fn settings_target(req: &SpawnReq) -> String {
    let f = req.settings_file.as_ref().expect("a settings file");
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(f).unwrap()).unwrap();
    v["env"]["CARGO_TARGET_DIR"].as_str().unwrap().to_string()
}

/// The leaf of a card tree `folder` under `main`, checked to be one Irori
/// made there: `.claude/worktrees/card-*`, a worktree on a branch of the
/// same name.
fn card_tree(main: &Path, folder: &str) -> String {
    let home = key(&main.join(".claude/worktrees").to_string_lossy());
    let k = key(folder);
    let name = k
        .strip_prefix(&format!("{home}/"))
        .unwrap_or_else(|| panic!("{folder} isn't under {home}"))
        .to_string();
    assert!(name.starts_with("card-") && !name.contains('/'), "{name}");
    let p = Path::new(folder);
    assert!(p.join(".git").is_file(), "a worktree");
    assert_eq!(
        git(p, &["branch", "--show-current"]).to_lowercase(),
        name,
        "on its own branch"
    );
    name
}

/// Card #565 finding A: Irori makes the tree with git before `claude`
/// starts, and `claude` is started in it, with no `-w`.
#[test]
fn a_fresh_card_gets_a_tree_its_card_owns_and_builds_land_in_it() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("app"));
    let r = Rig::new();
    see_root(&r, &main);
    let (t, req, fake) = start_with(&r, None);
    assert_eq!(
        *fake.was_tree.lock().unwrap(),
        [true],
        "the tree was there before claude started"
    );
    card_tree(&main, &req.folder);
    let tree = key(&req.folder);
    let dir = format!("{tree}/target");
    assert_eq!(req.env, vec![("CARGO_TARGET_DIR".to_string(), dir.clone())]);
    assert_eq!(settings_target(&req), dir);
    let book = Book::load(r.core.data_dir());
    assert!(book.trees[&tree].contains(&t.id), "{book:?}");
    let brief = std::fs::read_to_string(&req.brief_path).unwrap();
    assert!(brief.contains(BUILD_OUTPUT), "{brief}");
}

#[test]
fn a_card_resuming_in_its_tree_stays_there_and_builds_there() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("app"));
    let w = git_tree(&main, "card-1");
    let r = Rig::new();
    see_root(&r, &main);
    let (t, req) = start(&r, Some(&w));
    assert_eq!(req.folder, w.to_string_lossy(), "no second tree");
    let tree = key(&w.to_string_lossy());
    assert_eq!(settings_target(&req), format!("{tree}/target"));
    assert!(Book::load(r.core.data_dir()).trees[&tree].contains(&t.id));
}

#[test]
fn a_card_resuming_in_a_main_checkout_gets_no_tree() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("app"));
    let r = Rig::new();
    see_root(&r, &main);
    let (_, req) = start(&r, Some(&main));
    assert_eq!(req.folder, main.to_string_lossy());
    assert!(!main.join(".claude/worktrees").exists(), "no tree made");
    assert!(req.env.is_empty());
}

#[test]
fn a_folder_that_isnt_a_repo_gets_no_tree() {
    let d = tempfile::tempdir().unwrap();
    let r = Rig::new();
    see_root(&r, d.path());
    let (_, req) = start(&r, None);
    assert_eq!(key(&req.folder), key(&d.path().to_string_lossy()));
    assert!(req.env.is_empty());
}

#[test]
fn a_spawn_passes_a_temp_job_clone_for_a_real_checkout() {
    let d = tempfile::tempdir().unwrap();
    // Sorts first, as it did on the PC.
    let clone = git_repo(&d.path().join("a/.claude/jobs/47fb4a11/tmp/irori-439"));
    let main = git_repo(&d.path().join("z-app"));
    let r = Rig::new();
    see_root(&r, &clone);
    see_root(&r, &main);
    let (_, req) = start(&r, None);
    card_tree(&main, &req.folder);
}

/// Card #565: a job's temp clone goes with its job, so nothing new starts
/// in one, even when it is the only checkout: the card gets the same
/// "no checkout here" failure as with none.
#[test]
fn with_only_a_temp_clone_nothing_starts_there() {
    let d = tempfile::tempdir().unwrap();
    let clone = git_repo(&d.path().join("a/.claude/jobs/47fb4a11/tmp/irori-439"));
    let r = Rig::new();
    see_root(&r, &clone);
    let fake = Arc::new(Fake::default());
    let me = r.core.device_id().to_string();
    let co = r.core.coord();
    let t = co
        .create_task(
            &format!("user:{me}"),
            NewTask {
                title: "App card".into(),
                repo_key: K.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    r.core
        .set_spawner(Some(spawner(&fake, r.claude.join("sessions"))));
    r.core
        .set_spawn_setting(SpawnSetting {
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    wait_for(60, "the start to fail", || {
        co.get_task(&t.id)
            .and_then(|t| t.spawn_request().cloned())
            .is_some_and(|s| s.op == coord::SPAWN_FAILED)
    });
    assert!(fake.started.lock().unwrap().is_empty());
    assert!(clone.join("src.txt").is_file());
}

/// The tree Irori removed when the card settled, as the card's Where still
/// names it.
fn removed_tree(main: &Path, name: &str) -> PathBuf {
    let w = git_tree(main, name);
    git(main, &["worktree", "remove", &w.to_string_lossy()]);
    assert!(!w.exists());
    w
}

/// Card #565 finding 6: a reopened card whose tree went gets it back, on
/// its branch, with its builds in it; not the main checkout.
#[test]
fn a_reopened_card_whose_tree_went_gets_it_back_on_its_branch() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("app"));
    let w = removed_tree(&main, "fix-save");
    let r = Rig::new();
    see_root(&r, &main);
    let p: String = w.to_string_lossy().into();
    let (t, req) = start_at(
        &r,
        Some(Where {
            cwd: p.clone(),
            worktree: p,
            branch: "fix-save".into(),
            ..Where::default()
        }),
    );
    assert_eq!(
        key(&req.folder),
        key(&w.to_string_lossy()),
        "its own tree, not a second"
    );
    assert!(w.join(".git").is_file());
    assert_eq!(git(&w, &["branch", "--show-current"]), "fix-save");
    let tree = key(&w.to_string_lossy());
    let dir = format!("{tree}/target");
    assert_eq!(req.env, vec![("CARGO_TARGET_DIR".to_string(), dir.clone())]);
    assert_eq!(settings_target(&req), dir);
    assert!(Book::load(r.core.data_dir()).trees[&tree].contains(&t.id));
}

/// Finding 6 with no branch to go back to (or one checked out elsewhere):
/// a fresh tree, still never the main checkout bare.
#[test]
fn a_reopened_card_whose_tree_cant_come_back_gets_a_fresh_one() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("app"));
    let w = removed_tree(&main, "fix-load");
    // Its branch is checked out in another tree now.
    git_tree(&main, "other");
    git(
        &main.join(".claude/worktrees/other"),
        &["checkout", "-q", "fix-load"],
    );
    for branch in ["", "fix-load"] {
        let r = Rig::new();
        see_root(&r, &main);
        let p: String = w.to_string_lossy().into();
        let (t, req) = start_at(
            &r,
            Some(Where {
                cwd: p.clone(),
                worktree: p,
                branch: branch.into(),
                ..Where::default()
            }),
        );
        assert!(
            r.core.coord().get_task(&t.id).unwrap().where_.is_some(),
            "the Where took"
        );
        let name = card_tree(&main, &req.folder);
        assert_eq!(req.env.len(), 1, "{:?}", req.env);
        assert!(req.env[0].1.contains(&name), "{:?}", req.env);
    }
}

/// Card #565 finding A: when the tree can't be made, the card doesn't
/// start, and never bare in the main checkout.
#[test]
fn a_card_whose_tree_cant_be_made_doesnt_start() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("app"));
    // A file where the trees go: git can't make one.
    std::fs::create_dir_all(main.join(".claude")).unwrap();
    std::fs::write(main.join(".claude/worktrees"), "in the way").unwrap();
    let r = Rig::new();
    see_root(&r, &main);
    let fake = Arc::new(Fake::default());
    let me = r.core.device_id().to_string();
    let co = r.core.coord();
    let t = co
        .create_task(
            &format!("user:{me}"),
            NewTask {
                title: "App card".into(),
                repo_key: K.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    r.core
        .set_spawner(Some(spawner(&fake, r.claude.join("sessions"))));
    r.core
        .set_spawn_setting(SpawnSetting {
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    wait_for(60, "the start to fail", || {
        co.get_task(&t.id)
            .and_then(|t| t.spawn_request().cloned())
            .is_some_and(|s| s.op == coord::SPAWN_FAILED)
    });
    assert!(fake.started.lock().unwrap().is_empty(), "nothing started");
    let why = co
        .get_task(&t.id)
        .unwrap()
        .spawn_request()
        .unwrap()
        .reason
        .clone();
    assert!(why.contains("worktree"), "{why}");
}

/// Card #565 finding A: ending the agent leaves its tree (the Core's end
/// is `claude stop`, and `rm` waits while the tree is on disk); the tree
/// goes only by Irori's own cleanup, once its gates hold.
#[test]
fn the_tree_outlives_its_agent_and_goes_only_by_the_cleanup() {
    let d = tempfile::tempdir().unwrap();
    let main = git_repo(&d.path().join("app"));
    let r = Rig::new();
    see_root(&r, &main);
    let (_, req) = start(&r, None);
    let tree = PathBuf::from(&req.folder);
    card_tree(&main, &req.folder);
    let check = |live: &[String]| {
        remove(
            &tree,
            &Check {
                live,
                idle: std::time::Duration::from_secs(3600),
                now: std::time::SystemTime::now() + std::time::Duration::from_secs(7200),
                recheck: None,
            },
        )
    };
    // While the agent is in it, nothing goes.
    let live = vec![req.folder.clone()];
    assert!(matches!(check(&live), Verdict::Busy(_)));
    assert!(tree.join(".git").is_file());
    // Work in it keeps it after the agent ends.
    std::fs::write(tree.join(".env"), "TOKEN=1\n").unwrap();
    std::fs::write(main.join(".gitignore"), "/target\n.env\n").unwrap();
    std::fs::write(tree.join(".gitignore"), "/target\n.env\n").unwrap();
    git(&tree, &["add", ".gitignore"]);
    git(&tree, &["commit", "-q", "-m", "ignore env"]);
    let v = check(&[]);
    assert!(matches!(&v, Verdict::Work(_)), "{v:?}");
    assert!(tree.join(".env").is_file());
    // Pushed and clean: the cleanup removes it, branch kept.
    std::fs::remove_file(tree.join(".env")).unwrap();
    let branch = git(&tree, &["branch", "--show-current"]);
    git(&tree, &["push", "-q", "origin", &branch]);
    assert_eq!(check(&[]), Verdict::Removed);
    assert!(!tree.exists());
    assert!(git(&main, &["branch", "--list", &branch]).contains(&branch));
}
