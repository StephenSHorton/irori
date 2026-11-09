//! Card #1208: a card made from a thread goes first to that thread's live
//! spawned agent; with the agent gone it gets a fresh spawn as before; and
//! a thread agent whose card is still open outlives the idle limit.

use crate::common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use coord::NewTask;
use gocompat::time::GoTime;
use irori_core::spawn::{IDLE_LIMIT, SpawnReq, SpawnSetting, SpawnStarted, Spawner};

fn sess(n: usize) -> String {
    format!("cccccccc-3333-4333-8333-{n:012}")
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
            let n = {
                let mut v = a.started.lock().unwrap();
                v.push(r.clone());
                v.len()
            };
            write_session(
                &sessions,
                &format!("9000{n}.json"),
                &sess(n),
                &r.name,
                Path::new(&r.folder),
            );
            Ok(SpawnStarted {
                name: r.name.clone(),
                prefix: sess(n)[..8].into(),
            })
        }),
        end: Arc::new(move |id: &str| {
            b.ended.lock().unwrap().push(id.to_string());
            for e in std::fs::read_dir(&s2).unwrap().flatten() {
                if std::fs::read_to_string(e.path()).is_ok_and(|s| s.contains(id)) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
            Ok(String::new())
        }),
        trusted: Arc::new(|_| true),
        list: None,
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// A git repo with one commit and a terminal agent working in it, so a
/// thread's agent has somewhere to start.
fn agent_in_repo(r: &Rig) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().canonicalize().unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["commit", "-q", "--allow-empty", "-m", "first"]);
    write_session(
        &r.claude.join("sessions"),
        "800004.json",
        "dddddddd-4444-4444-8444-000000000004",
        "gamma",
        &dir,
    );
    let key = format!(
        "{}/dddddddd-4444-4444-8444-000000000004",
        r.core.device_id()
    );
    wait_for(60, "the agent in the repo listed with its repo", || {
        r.core
            .local_agents()
            .iter()
            .any(|a| a.key == key && !a.repo_key.is_empty())
    });
    d
}

struct Setup {
    _repo: tempfile::TempDir,
    r: Rig,
    fake: Arc<Fake>,
    skew: Arc<AtomicI64>,
    person: String,
    thread: coord::Task,
    agent: String,
}

/// A thread with its spawned agent bound and the thread answered.
fn setup() -> Setup {
    let skew = Arc::new(AtomicI64::new(0));
    let s = skew.clone();
    let r = Rig::with(move |o| {
        o.now = Some(Arc::new(move || {
            let d = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap();
            GoTime::unix(
                d.as_secs() as i64 + s.load(Ordering::SeqCst),
                i64::from(d.subsec_nanos()),
            )
        }));
    });
    let repo = agent_in_repo(&r);
    let fake = Arc::new(Fake::default());
    let me = r.core.device_id().to_string();
    let person = format!("user:{me}");
    let co = r.core.coord();
    r.core
        .set_spawner(Some(spawner(&fake, r.claude.join("sessions"))));
    r.core
        .set_spawn_setting(SpawnSetting {
            board_id: String::new(),
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    let thread = co
        .post_ask(
            &person,
            coord::NewAsk {
                text: "Can the board sort by age?".into(),
                thread: true,
                ..coord::NewAsk::default()
            },
            &r.core.ask_agents(),
        )
        .unwrap()
        .0;
    let agent = format!("{me}/{}", sess(1));
    wait_for(60, "the thread's agent took it", || {
        co.get_task(&thread.id)
            .and_then(|t| t.ask)
            .is_some_and(|a| a.taker == agent)
    });
    co.add_comment(&agent, &thread.id, "Yes, it is small.", false)
        .unwrap();
    Setup {
        _repo: repo,
        r,
        fake,
        skew,
        person,
        thread,
        agent,
    }
}

fn card_from(s: &Setup, title: &str) -> coord::Task {
    s.r.core
        .coord()
        .create_task(
            &s.person,
            NewTask {
                title: title.into(),
                objective: "sort it".into(),
                from: s.thread.id.clone(),
                ..NewTask::default()
            },
        )
        .unwrap()
}

fn skew_by(s: &Setup, d: Duration) {
    s.skew.fetch_add(d.as_secs() as i64 + 10, Ordering::SeqCst);
    s.r.core.set_spawn_cap(s.r.core.spawn_cap()).unwrap();
}

#[test]
fn a_card_made_from_a_thread_goes_to_the_threads_live_agent() {
    let s = setup();
    let co = s.r.core.coord();
    let c = card_from(&s, "Sort the board by age");
    wait_for(60, "the card offered to the thread's agent", || {
        co.get_task(&c.id)
            .is_some_and(|t| t.assigned.contains(&s.agent) || t.holders.contains(&s.agent))
    });
    s.r.core.set_spawn_cap(s.r.core.spawn_cap()).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        s.fake.started.lock().unwrap().len(),
        1,
        "no fresh agent for the card"
    );
}

#[test]
fn with_the_threads_agent_gone_the_card_gets_a_fresh_spawn() {
    let s = setup();
    let co = s.r.core.coord();
    // The thread agent's session is gone (Claude Code dropped it).
    std::fs::remove_file(s.r.claude.join("sessions").join("90001.json")).unwrap();
    wait_for(60, "the thread agent off presence", || {
        !s.r.core.local_agents().iter().any(|a| a.key == s.agent)
    });
    let c = card_from(&s, "Sort the board by age");
    wait_for(90, "a fresh agent for the card", || {
        s.r.core.set_spawn_cap(s.r.core.spawn_cap()).unwrap();
        s.fake.started.lock().unwrap().iter().any(|r| {
            std::fs::read_to_string(&r.brief_path).is_ok_and(|b| {
                b.contains("Irori started you to work card") && b.contains("Sort the board by age")
            })
        })
    });
    assert!(!co.get_task(&c.id).unwrap().assigned.contains(&s.agent));
}

#[test]
fn a_thread_agent_with_an_open_card_outlives_the_idle_limit() {
    let s = setup();
    let co = s.r.core.coord();
    let c = card_from(&s, "Sort the board by age");
    wait_for(60, "the card offered to the thread's agent", || {
        co.get_task(&c.id)
            .is_some_and(|t| t.assigned.contains(&s.agent) || t.holders.contains(&s.agent))
    });
    // Even with the thread resolved, the card it holds keeps it.
    co.resolve_thread(&s.person, &s.thread.id).unwrap();
    skew_by(&s, IDLE_LIMIT);
    std::thread::sleep(Duration::from_millis(2500));
    assert!(
        s.fake.ended.lock().unwrap().is_empty(),
        "wanted, so kept past the idle limit"
    );
    // The card closes: nothing wants the agent, and the short clock runs.
    co.set_task_state(&s.person, &c.id, "canceled", "not needed")
        .unwrap();
    skew_by(&s, IDLE_LIMIT);
    wait_for(60, "the unwanted thread agent ended", || {
        s.r.core.set_spawn_cap(s.r.core.spawn_cap()).unwrap();
        !s.fake.ended.lock().unwrap().is_empty()
    });
}

#[test]
fn an_unwanted_thread_agent_keeps_the_short_clock() {
    let s = setup();
    skew_by(&s, IDLE_LIMIT);
    wait_for(60, "the quiet thread agent ended", || {
        s.r.core.set_spawn_cap(s.r.core.spawn_cap()).unwrap();
        !s.fake.ended.lock().unwrap().is_empty()
    });
}
