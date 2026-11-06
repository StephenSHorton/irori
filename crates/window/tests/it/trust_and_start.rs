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
