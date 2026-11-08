//! Card #739: the person reopens a thread whose agent has gone and writes
//! on it. Irori starts it a new agent, and while that one starts the
//! thread says so: it waits on an agent (not "Answered" by the gone one),
//! with the device it starts on, in the Talk list and on its card.

use crate::common;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use irori_core::spawn::{SpawnReq, SpawnSetting, SpawnStarted, Spawner};

#[test]
fn a_reopened_thread_says_its_new_agent_is_starting() {
    let r = Rig::new();
    // An agent here works in a git repo: the thread's agent starts there.
    let me = r.w.core.device_id().to_string();
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path().canonicalize().unwrap();
    for args in [
        &["init", "-q"][..],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "first",
        ][..],
    ] {
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(&dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(ok.success(), "git {args:?}");
    }
    let in_repo = "dddddddd-4444-4444-8444-000000000004";
    write_session(
        &r._dirs[1].path().join("sessions"),
        "900004.json",
        in_repo,
        "gamma",
        &dir,
        unix_ms(),
    );
    let key = format!("{me}/{in_repo}");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !r
        .w
        .core
        .local_agents()
        .iter()
        .any(|a| a.key == key && !a.repo_key.is_empty())
    {
        assert!(
            Instant::now() < deadline,
            "the agent in the repo never showed"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let started: Arc<Mutex<Vec<SpawnReq>>> = Arc::default();
    let s = started.clone();
    // Its agent never shows in presence: the request stays pending, as
    // #732's did for two and a half minutes.
    r.w.core.set_spawner(Some(Spawner {
        spawn: Arc::new(move |req: &SpawnReq| {
            s.lock().unwrap().push(req.clone());
            Ok(SpawnStarted {
                name: req.name.clone(),
                prefix: "eeeeeeee".into(),
            })
        }),
        end: Arc::new(|_| Ok(String::new())),
        trusted: Arc::new(|_: &Path| true),
        list: None,
        ready: None,
    }));

    // Answered by an agent that has gone since, then resolved.
    let gone = format!("{me}/ffffffff-6666-4666-8666-000000000006");
    let id =
        r.w.start_thread("Can we add shortcuts to the app?", &[], &[], "")
            .unwrap()
            .task
            .id;
    r.w.core
        .coord()
        .add_comment(&gone, &id, "Yes: #733 has them.", false)
        .unwrap();
    r.w.resolve_thread(&id).unwrap();
    r.w.reopen_thread(&id).unwrap();
    assert_eq!(r.w.threads("")[0].state, "answered");

    // Spawning on; the person writes, with nobody @mentioned.
    r.w.core
        .set_spawn_setting(SpawnSetting {
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    let c =
        r.w.comment(
            &id,
            "we're doing equivalent windows shortcuts right?",
            "wake",
            &[],
            &[],
        )
        .unwrap();
    assert!(c.told.to.is_empty(), "nobody live heard it: {:?}", c.told);

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        r.w.core.set_spawn_cap(r.w.core.spawn_cap()).unwrap();
        if !started.lock().unwrap().is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "no agent was started for it");
        std::thread::sleep(Duration::from_millis(100));
    }
    let req = started.lock().unwrap()[0].clone();
    assert_eq!(req.task_id, id);
    let brief = std::fs::read_to_string(&req.brief_path).unwrap();
    assert!(
        brief.contains("we're doing equivalent windows shortcuts right?"),
        "{brief}"
    );

    // While it starts, the thread waits on it and says where it starts.
    let t = r.w.threads("").into_iter().find(|t| t.id == id).unwrap();
    assert_eq!(t.state, "waiting", "{t:?}");
    assert!(t.starting.contains("this one"), "{t:?}");
    assert!(t.no_agent.is_empty(), "{t:?}");
    let card = r.w.card(&id).unwrap();
    assert!(
        card.summary.starting.contains("this one"),
        "{:?}",
        card.summary
    );

    // Its new agent answers: Answered at once, before any sweep.
    let fresh = format!("{me}/eeeeeeee-5555-4555-8555-000000000005");
    r.w.core
        .coord()
        .add_comment(&fresh, &id, "Yes: Ctrl in place of Cmd.", false)
        .unwrap();
    let t = r.w.threads("").into_iter().find(|t| t.id == id).unwrap();
    assert_eq!(t.state, "answered", "{t:?}");
}

/// A follow-up with Agents answer now off gets no agent: it stays
/// Answered, the person's to follow up, never Waiting for good.
#[test]
fn a_follow_up_with_agents_answer_now_off_stays_answered() {
    let r = Rig::new();
    let me = r.w.core.device_id().to_string();
    let gone = format!("{me}/ffffffff-6666-4666-8666-000000000006");
    let id =
        r.w.start_thread("Can we add shortcuts to the app?", &[], &[], "")
            .unwrap()
            .task
            .id;
    r.w.core
        .coord()
        .add_comment(&gone, &id, "Yes: #733 has them.", false)
        .unwrap();
    r.w.resolve_thread(&id).unwrap();
    r.w.reopen_thread(&id).unwrap();
    r.w.comment(&id, "And on Windows?", "wake", &[], &[])
        .unwrap();
    r.w.set_agents_now(&id, false).unwrap();
    let t = r.w.threads("").into_iter().find(|t| t.id == id).unwrap();
    assert_eq!(t.state, "answered", "{t:?}");
}
