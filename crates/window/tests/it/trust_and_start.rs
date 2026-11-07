//! Trust and start (card #449, docs/design/spawned-agents.md §7 "The
//! pre-trust flow"): a card whose repo isn't trusted for agents shows the
//! reason and what trusting does; the person's Trust and start records
//! trust for the repo's main root in `.claude.json` and the agent starts.

use crate::common;

use std::path::Path;
use std::sync::{Arc, Mutex};

use common::*;
use coord::{NewTask, Where};
use irori_core::spawn::{SpawnReq, SpawnSetting, SpawnStarted, Spawner};
use irori_window::agents_trust::{NOT_TRUSTED, trust_folder};
use irori_window::claude_trust;

#[test]
fn an_untrusted_repo_waits_on_the_person_then_starts() {
    let r = Rig::new();
    let w = &r.w;
    let co = w.core.coord().clone();
    let person = format!("user:{}", w.core.device_id());

    // A repo with a subfolder the card works in, and a .claude.json that
    // trusts neither.
    let dirs = tempfile::tempdir().unwrap();
    let repo = dirs.path().join("api");
    let sub = repo.join("server");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(&sub).unwrap();
    let file = dirs.path().join(".claude.json");
    std::fs::write(&file, r#"{"numStartups": 3, "projects": {}}"#).unwrap();

    let started: Arc<Mutex<Vec<SpawnReq>>> = Arc::default();
    let (s2, f2) = (started.clone(), file.clone());
    w.core.set_spawner(Some(Spawner {
        ready: None,
        spawn: Arc::new(move |req: &SpawnReq| {
            s2.lock().unwrap().push(req.clone());
            Ok(SpawnStarted {
                name: req.name.clone(),
                prefix: "dddddddd".into(),
            })
        }),
        end: Arc::new(|_| Ok(String::new())),
        trusted: Arc::new(move |dir: &Path| {
            claude_trust::is_trusted(&f2, &trust_folder(&dir.to_string_lossy()))
        }),
        list: None,
    }));
    w.core
        .set_spawn_setting(SpawnSetting {
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();

    let t = co
        .create_task(
            &person,
            NewTask {
                title: "Add the endpoint".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.set_where(
        &t.id,
        Where {
            who: r.key_a.clone(),
            cwd: sub.to_string_lossy().into(),
            worktree: sub.to_string_lossy().into(),
            ..Where::default()
        },
    )
    .unwrap();

    // The spawn is refused for trust, and nothing started.
    wait("the not-trusted refusal", || {
        co.get_task(&t.id)
            .and_then(|t| t.spawn_request().cloned())
            .is_some_and(|s| s.op == coord::SPAWN_FAILED && s.reason.starts_with(NOT_TRUSTED))
    });
    assert!(started.lock().unwrap().is_empty());

    // The card says why, for Needs you and the working line.
    let cs = w.cards();
    let c = cs.iter().find(|c| c.id == t.id).expect("the card");
    assert!(c.cant_start.starts_with(NOT_TRUSTED), "{:?}", c.cant_start);

    // The window shows what trusting does, for the repo's root, here.
    let need = w.agents_trust_need(&t.id).unwrap().expect("a trust need");
    let root = repo.to_string_lossy().into_owned();
    assert_eq!(
        need.folder, root,
        "the repo's root, not the card's subfolder"
    );
    assert!(need.here);
    assert!(need.what.contains(&root), "{}", need.what);
    assert!(
        need.what.contains("without asking you first"),
        "{}",
        need.what
    );

    // Trust and start: recorded for the root only, and the agent starts.
    let done = w.trust_and_start_in(&file, &t.id).unwrap();
    assert_eq!((done.folder.as_str(), done.already), (root.as_str(), false));
    assert!(claude_trust::is_trusted(&file, &root));
    assert!(!claude_trust::is_trusted(&file, &sub.to_string_lossy()));
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(v["numStartups"], 3, "nothing else in the file changed");
    wait("the agent starts", || !started.lock().unwrap().is_empty());
    assert_eq!(started.lock().unwrap()[0].task_id, t.id);

    // It no longer waits on trust, so a second click does nothing.
    wait("the new request", || {
        w.agents_trust_need(&t.id).unwrap().is_none()
    });
    assert!(w.trust_and_start_in(&file, &t.id).is_err());
}

/// A card with no trust problem has no trust need, and Trust and start
/// refuses it (no trust is written).
#[test]
fn a_card_not_waiting_on_trust_is_left_alone() {
    let r = Rig::new();
    let w = &r.w;
    let person = format!("user:{}", w.core.device_id());
    let t = w
        .core
        .coord()
        .create_task(
            &person,
            NewTask {
                title: "x".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    assert!(w.agents_trust_need(&t.id).unwrap().is_none());
    let dirs = tempfile::tempdir().unwrap();
    let file = dirs.path().join(".claude.json");
    assert!(w.trust_and_start_in(&file, &t.id).is_err());
    assert!(!file.exists(), "nothing written");
}

/// Settings › Agents' permissions round-trip and leave the switch alone.
#[test]
fn a_boards_agent_permissions_round_trip() {
    let r = Rig::new();
    let w = &r.w;
    w.core
        .set_spawn_setting(SpawnSetting {
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    let mut p = w.agents_permissions("");
    assert_eq!(p.permission, "", "bypass, guarded, until he picks");
    p.permission = "limited".into();
    p.allow = vec!["Bash(cargo test:*)".into()];
    p.merge
        .insert("remote:github.com/me/irori".into(), "main".into());
    let got = w.agents_permissions_set(p.clone()).unwrap();
    assert_eq!(got, p);
    assert!(w.core.spawn_setting("").on, "the switch stays on");
    p.permission = "anything".into();
    assert!(w.agents_permissions_set(p).is_err());
    assert_eq!(w.agents_permissions("").permission, "limited");
}

/// A card refused for trust, on a rig whose spawner records what it starts
/// and reads trust from a scratch `.claude.json` (card #717).
struct Untrusted {
    r: Rig,
    dirs: tempfile::TempDir,
    file: std::path::PathBuf,
    started: Arc<Mutex<Vec<SpawnReq>>>,
    card: coord::Task,
}

fn untrusted() -> Untrusted {
    let r = Rig::new();
    let dirs = tempfile::tempdir().unwrap();
    let repo = dirs.path().join("api");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let file = dirs.path().join(".claude.json");
    std::fs::write(&file, r#"{"projects": {}}"#).unwrap();
    let started: Arc<Mutex<Vec<SpawnReq>>> = Arc::default();
    let (s2, f2) = (started.clone(), file.clone());
    r.w.core.set_spawner(Some(Spawner {
        ready: None,
        spawn: Arc::new(move |req: &SpawnReq| {
            s2.lock().unwrap().push(req.clone());
            Ok(SpawnStarted {
                name: req.name.clone(),
                prefix: "dddddddd".into(),
            })
        }),
        end: Arc::new(|_| Ok(String::new())),
        trusted: Arc::new(move |dir: &Path| {
            claude_trust::is_trusted(&f2, &trust_folder(&dir.to_string_lossy()))
        }),
        list: None,
    }));
    r.w.core
        .set_spawn_setting(SpawnSetting {
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    let card = in_folder(&r, "Add the endpoint", &repo);
    let co = r.w.core.coord().clone();
    wait("the not-trusted refusal", || {
        co.get_task(&card.id)
            .and_then(|t| t.spawn_request().cloned())
            .is_some_and(|s| s.op == coord::SPAWN_FAILED && s.reason.starts_with(NOT_TRUSTED))
    });
    assert!(started.lock().unwrap().is_empty());
    Untrusted {
        r,
        dirs,
        file,
        started,
        card,
    }
}

/// A new card whose agent starts in `dir`.
fn in_folder(r: &Rig, title: &str, dir: &Path) -> coord::Task {
    let co = r.w.core.coord().clone();
    let person = format!("user:{}", r.w.core.device_id());
    let t = co
        .create_task(
            &person,
            NewTask {
                title: title.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.set_where(
        &t.id,
        Where {
            who: r.key_a.clone(),
            cwd: dir.to_string_lossy().into(),
            worktree: dir.to_string_lossy().into(),
            ..Where::default()
        },
    )
    .unwrap();
    t
}

fn cant_start(r: &Rig, id: &str) -> String {
    r.w.cards()
        .into_iter()
        .find(|c| c.id == id)
        .expect("the card")
        .cant_start
}

/// Card #717: a live agent takes a card waiting on trust. It no longer
/// waits, so it leaves Needs you; Trust and start on it (the row a beat
/// behind) still records trust and says the holder has it, starting nothing.
#[test]
fn a_card_an_agent_takes_no_longer_waits_on_trust() {
    let u = untrusted();
    let (r, w) = (&u.r, &u.r.w);
    let co = w.core.coord().clone();
    assert!(cant_start(r, &u.card.id).starts_with(NOT_TRUSTED));
    assert!(w.agents_trust_need(&u.card.id).unwrap().is_some());

    co.claim_task(&r.key_a, &u.card.id).unwrap();
    assert_eq!(cant_start(r, &u.card.id), "", "out of Needs you");
    assert!(w.agents_trust_need(&u.card.id).unwrap().is_none());

    let done = w.trust_and_start_in(&u.file, &u.card.id).unwrap();
    let name = w.core.agent_name(&r.key_a);
    assert!(!name.is_empty());
    assert_eq!(
        done.said,
        format!("Trusted. {name} already has it, so no new agent starts.")
    );
    assert!(claude_trust::is_trusted(&u.file, &done.folder));
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(u.started.lock().unwrap().is_empty(), "nothing started");
}

/// Card #717: Trust and start on a device at its cap says it waits for a
/// slot, with how many are in use, and the card shows its place in the
/// queue instead of the old trust reason.
#[test]
fn trust_and_start_at_the_cap_says_it_waits_for_a_slot() {
    let u = untrusted();
    let (r, w) = (&u.r, &u.r.w);
    w.core.set_spawn_cap(1).unwrap();
    // Another card, in a trusted repo, takes the one slot.
    let other = u.dirs.path().join("web");
    std::fs::create_dir_all(other.join(".git")).unwrap();
    claude_trust::record_trust(&u.file, &other.to_string_lossy()).unwrap();
    let busy = in_folder(r, "Fix the page", &other);
    wait("the other card's agent", || {
        u.started
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.task_id == busy.id)
    });

    let done = w.trust_and_start_in(&u.file, &u.card.id).unwrap();
    assert_eq!(
        done.said,
        "Trusted. It starts when a slot frees (1 of 1 in use)."
    );
    assert_eq!(cant_start(r, &u.card.id), "", "out of Needs you");
    let c = w.cards().into_iter().find(|c| c.id == u.card.id).unwrap();
    assert_eq!(c.queue_place, 1, "{c:?}");
    assert!(w.agents_trust_need(&u.card.id).unwrap().is_none());
    assert!(
        !u.started
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.task_id == u.card.id)
    );
}

/// Card #717: with a slot free, Trust and start says its agent is starting.
#[test]
fn trust_and_start_with_a_slot_says_it_starts() {
    let u = untrusted();
    let done = u.r.w.trust_and_start_in(&u.file, &u.card.id).unwrap();
    assert_eq!(done.said, "Trusted. Its agent is starting.");
    wait("the agent starts", || {
        u.started
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.task_id == u.card.id)
    });
}

/// Card #717: with Pause agents on, Trust and start says so, and the card
/// leaves Needs you: the person has asked, and it starts when they turn
/// the pause off.
#[test]
fn trust_and_start_while_paused_says_so_and_leaves_needs_you() {
    let u = untrusted();
    let w = &u.r.w;
    w.core.set_agents_paused(true).unwrap();
    let done = w.trust_and_start_in(&u.file, &u.card.id).unwrap();
    assert_eq!(
        done.said,
        "Trusted. Its agent starts when you turn off Pause agents."
    );
    assert_eq!(cant_start(&u.r, &u.card.id), "", "out of Needs you");
    assert!(w.agents_trust_need(&u.card.id).unwrap().is_none());
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(u.started.lock().unwrap().is_empty(), "nothing while paused");
    w.core.set_agents_paused(false).unwrap();
    wait("the agent starts once unpaused", || {
        u.started
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.task_id == u.card.id)
    });
}
