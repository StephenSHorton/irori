//! Card #451 (spawned agents §3, part 7) through the Core: a helper's part
//! of a led card is spawned in its own worktree, branched from the lead's
//! card branch, never in the lead's folder.

use crate::common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use common::*;
use coord::{NewSubtask, NewTask, Where};
use irori_core::spawn::{SpawnReq, SpawnSetting, SpawnStarted, Spawner};

const HELPER_SESS: &str = "dddddddd-4444-4444-8444-000000000004";

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
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[derive(Default)]
struct Fake {
    started: Mutex<Vec<SpawnReq>>,
    ended: Mutex<Vec<String>>,
}

fn spawner(f: &Arc<Fake>, sessions: PathBuf) -> Spawner {
    let (a, b) = (f.clone(), f.clone());
    let s2 = sessions.clone();
    Spawner {
        ready: None,
        spawn: Arc::new(move |r: &SpawnReq| {
            a.started.lock().unwrap().push(r.clone());
            write_session(
                &sessions,
                "900004.json",
                HELPER_SESS,
                &r.name,
                Path::new(&r.folder),
            );
            Ok(SpawnStarted {
                name: r.name.clone(),
                prefix: HELPER_SESS[..8].into(),
            })
        }),
        end: Arc::new(move |sess: &str| {
            b.ended.lock().unwrap().push(sess.to_string());
            let _ = std::fs::remove_file(s2.join("900004.json"));
            Ok(String::new())
        }),
        trusted: Arc::new(|_| true),
        list: None,
    }
}

#[test]
fn a_helpers_part_starts_in_a_worktree_from_the_card_branch() {
    let r = Rig::new();
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("game-repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "first"]);
    git(&repo, &["checkout", "-q", "-b", "lead/451-card"]);
    std::fs::write(repo.join("plan.md"), "plan").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "plan"]);
    let plan = git(&repo, &["rev-parse", "HEAD"]);

    let fake = Arc::new(Fake::default());
    let me = r.core.device_id().to_string();
    let person = format!("user:{me}");
    let helper = format!("{me}/{HELPER_SESS}");
    let co = r.core.coord();
    let p = co
        .create_task(
            &person,
            NewTask {
                title: "big card".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &p.id).unwrap();
    co.set_where(
        &p.id,
        Where {
            who: r.key_a.clone(),
            cwd: repo.to_string_lossy().into(),
            branch: "lead/451-card".into(),
            ..Where::default()
        },
    )
    .unwrap();
    let (_, made) = co
        .split(
            &r.key_a,
            &p.id,
            vec![NewSubtask {
                title: "tests".into(),
                spawn: true,
                ..NewSubtask::default()
            }],
            vec![],
        )
        .unwrap();
    let k = &made[0];
    r.core
        .set_spawner(Some(spawner(&fake, r.claude.join("sessions"))));
    r.core
        .set_spawn_setting(SpawnSetting {
            board_id: String::new(),
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    wait_for(60, "the helper bound to its part", || {
        co.get_task(&k.id)
            .is_some_and(|t| t.holders.contains(&helper))
    });
    let reqs = fake.started.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1, "one helper: {reqs:?}");
    let folder = PathBuf::from(&reqs[0].folder).canonicalize().unwrap();
    let repo = repo.canonicalize().unwrap();
    assert_ne!(folder, repo, "never the lead's folder");
    assert!(
        folder.starts_with(repo.join(".claude/worktrees")),
        "{}",
        folder.display()
    );
    assert_eq!(
        git(&folder, &["rev-parse", "HEAD"]),
        plan,
        "from the card branch"
    );
    assert!(git(&folder, &["branch", "--show-current"]).starts_with("lead/451-card-"));
    // The lead holds the parent: nothing is spawned for it.
    assert!(reqs.iter().all(|q| q.task_id == k.id));

    // The helper's part is done: it asks the lead, and stays for the merge.
    co.set_task_state(&helper, &k.id, "completed", "tests pass @abc")
        .unwrap();
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert!(fake.ended.lock().unwrap().is_empty(), "kept for the merge");
    assert!(co.get_task(&k.id).unwrap().holders.contains(&helper));
    // The lead merges and accepts: now the helper goes.
    co.set_task_state(&r.key_a, &k.id, "completed", "merged")
        .unwrap();
    wait_for(60, "the helper ended after the merge", || {
        fake.ended
            .lock()
            .unwrap()
            .iter()
            .any(|s| HELPER_SESS.starts_with(s.as_str()) || s.starts_with(&HELPER_SESS[..8]))
    });
}

/// A card has no limit of its own on helpers (card #576): the device's cap
/// is what bounds them, and the rest queue for a slot.
#[test]
fn a_cards_helpers_are_bound_by_the_devices_cap() {
    let r = Rig::new();
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("game-repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "first"]);
    git(&repo, &["checkout", "-q", "-b", "lead/576-card"]);

    let fake = Arc::new(Fake::default());
    let person = format!("user:{}", r.core.device_id());
    let co = r.core.coord();
    let p = co
        .create_task(
            &person,
            NewTask {
                title: "big card".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &p.id).unwrap();
    co.set_where(
        &p.id,
        Where {
            who: r.key_a.clone(),
            cwd: repo.to_string_lossy().into(),
            branch: "lead/576-card".into(),
            ..Where::default()
        },
    )
    .unwrap();
    let parts = (0..7)
        .map(|i| NewSubtask {
            title: format!("part {i}"),
            spawn: true,
            ..NewSubtask::default()
        })
        .collect();
    co.split(&r.key_a, &p.id, parts, vec![]).unwrap();

    r.core.set_spawn_cap(3).unwrap();
    r.core
        .set_spawn_setting(SpawnSetting {
            board_id: String::new(),
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    // The switch on, every part waits for an agent: none is held back for
    // the card's sake.
    assert_eq!(co.cards_to_spawn().len(), 7, "no limit of the card's own");
    r.core
        .set_spawner(Some(spawner(&fake, r.claude.join("sessions"))));
    let started = || fake.started.lock().unwrap().len();
    let kick = || r.core.set_spawn_cap(r.core.spawn_cap()).unwrap();
    wait_for(60, "the cap's helpers started", || {
        kick();
        started() == 3
    });
    for _ in 0..5 {
        kick();
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    assert_eq!(started(), 3, "the cap holds");
    assert_eq!(r.core.spawn_queue().places.len(), 4, "the rest queue");

    r.core.set_spawn_cap(5).unwrap();
    wait_for(60, "a higher cap starts two more", || {
        kick();
        started() == 5
    });
}

/// Card #1129: a helper's part that needs the other machine (`needs:pc` on
/// a Mac, `needs:mac` on a PC) waits instead of starting an agent here that
/// can only hand it back, over and over; a part with no such need starts.
#[test]
fn a_helper_part_that_needs_the_other_machine_waits() {
    let r = Rig::new();
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("game-repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "first"]);
    git(&repo, &["checkout", "-q", "-b", "lead/1129-card"]);

    let fake = Arc::new(Fake::default());
    let person = format!("user:{}", r.core.device_id());
    let co = r.core.coord();
    let p = co
        .create_task(
            &person,
            NewTask {
                title: "big card".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &p.id).unwrap();
    co.set_where(
        &p.id,
        Where {
            who: r.key_a.clone(),
            cwd: repo.to_string_lossy().into(),
            branch: "lead/1129-card".into(),
            ..Where::default()
        },
    )
    .unwrap();
    // The machine this test is not running on.
    let other = if cfg!(windows) {
        "needs:mac"
    } else {
        "needs:pc"
    };
    let (_, made) = co
        .split(
            &r.key_a,
            &p.id,
            vec![
                NewSubtask {
                    title: format!("elsewhere {other}"),
                    spawn: true,
                    ..NewSubtask::default()
                },
                NewSubtask {
                    title: "anywhere".into(),
                    spawn: true,
                    ..NewSubtask::default()
                },
            ],
            vec![],
        )
        .unwrap();
    let (elsewhere, anywhere) = (&made[0], &made[1]);
    r.core
        .set_spawner(Some(spawner(&fake, r.claude.join("sessions"))));
    r.core
        .set_spawn_setting(SpawnSetting {
            board_id: String::new(),
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    wait_for(60, "the part with no need started", || {
        r.core.set_spawn_cap(r.core.spawn_cap()).unwrap();
        fake.started
            .lock()
            .unwrap()
            .iter()
            .any(|q| q.task_id == anywhere.id)
    });
    for _ in 0..5 {
        r.core.set_spawn_cap(r.core.spawn_cap()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    let reqs = fake.started.lock().unwrap().clone();
    assert!(
        reqs.iter().all(|q| q.task_id != elsewhere.id),
        "nothing started for the part that needs the other machine: {reqs:?}"
    );
    assert!(
        r.core.why_no_agent(&elsewhere.id, true).is_some(),
        "it says why it waits"
    );
}
