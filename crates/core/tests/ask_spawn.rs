//! Card #470 through the Core, with a fake spawner in place of `claude
//! --bg`: a thread the person starts that no agent with "Offer it cards"
//! on takes gets its own background agent. It starts in the repo of the
//! agent here active last (the window's threads name none), with a brief
//! for a thread; bound, it takes the thread, answers in it, and stays for
//! follow-ups until the person resolves it. An idle agent with "Offer it
//! cards" on still takes a thread first.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use irori_core::spawn::{SpawnReq, SpawnSetting, SpawnStarted, Spawner};

const SPAWNED_SESS: &str = "cccccccc-3333-4333-8333-000000000003";
const REPO_SESS: &str = "dddddddd-4444-4444-8444-000000000004";

#[derive(Default)]
struct Fake {
    started: Mutex<Vec<SpawnReq>>,
    ended: Mutex<Vec<String>>,
}

fn spawner(f: &Arc<Fake>, sessions: PathBuf) -> Spawner {
    let (a, b) = (f.clone(), f.clone());
    let s2 = sessions.clone();
    Spawner {
        spawn: Arc::new(move |r: &SpawnReq| {
            a.started.lock().unwrap().push(r.clone());
            write_session(
                &sessions,
                "900003.json",
                SPAWNED_SESS,
                &r.name,
                Path::new(&r.folder),
            );
            Ok(SpawnStarted {
                name: r.name.clone(),
                prefix: SPAWNED_SESS[..8].into(),
            })
        }),
        end: Arc::new(move |sess: &str| {
            b.ended.lock().unwrap().push(sess.to_string());
            let _ = std::fs::remove_file(s2.join("900003.json"));
            Ok(String::new())
        }),
        trusted: Arc::new(|_| true),
        list: None,
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
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

/// A git repo with one commit, and a terminal agent working in it.
fn agent_in_repo(r: &Rig) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().canonicalize().unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["commit", "-q", "--allow-empty", "-m", "first"]);
    write_session(
        &r.claude.join("sessions"),
        "900004.json",
        REPO_SESS,
        "gamma",
        &dir,
    );
    let me = r.core.device_id().to_string();
    let key = format!("{me}/{REPO_SESS}");
    wait_for(60, "the agent in the repo listed with its repo", || {
        r.core
            .local_agents()
            .iter()
            .any(|a| a.key == key && !a.repo_key.is_empty())
    });
    (d, dir)
}

fn spawning_on(r: &Rig) {
    r.core
        .set_spawn_setting(SpawnSetting {
            board_id: String::new(),
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
}

fn thread(r: &Rig, text: &str) -> coord::Task {
    let person = format!("user:{}", r.core.device_id());
    r.core
        .coord()
        .post_ask(
            &person,
            coord::NewAsk {
                text: text.into(),
                thread: true,
                ..coord::NewAsk::default()
            },
            &r.core.ask_agents(),
        )
        .unwrap()
        .0
}

#[test]
fn a_thread_nobody_takes_gets_a_background_agent_that_answers_it() {
    let r = Rig::new();
    let (_repo, dir) = agent_in_repo(&r);
    let fake = Arc::new(Fake::default());
    let me = r.core.device_id().to_string();
    let person = format!("user:{me}");
    let spawned = format!("{me}/{SPAWNED_SESS}");
    let co = r.core.coord();
    r.core
        .set_spawner(Some(spawner(&fake, r.claude.join("sessions"))));
    spawning_on(&r);

    // No agent has "Offer it cards" on.
    let tk = thread(&r, "What did we ship today?");
    wait_for(60, "the spawned agent took the thread", || {
        co.get_task(&tk.id)
            .and_then(|t| t.ask)
            .is_some_and(|a| a.taker == spawned)
    });
    let t = co.get_task(&tk.id).unwrap();
    assert_eq!(t.holders, std::slice::from_ref(&spawned));
    for _ in 0..3 {
        r.core.set_spawn_cap(r.core.spawn_cap()).unwrap();
    }
    std::thread::sleep(Duration::from_millis(300));
    let reqs = fake.started.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1, "one spawn: {reqs:?}");
    let req = &reqs[0];
    assert!(
        !req.name.is_empty() && !req.name.contains('-'),
        "a word name (#472): {req:?}"
    );
    assert_eq!(
        Path::new(&req.folder).canonicalize().unwrap(),
        dir,
        "the repo of the agent here"
    );
    let brief = std::fs::read_to_string(&req.brief_path).unwrap();
    assert!(
        brief.contains(&format!("board_comment {}", tk.short_ref()))
            && brief.contains("Don't ask_close it")
            && brief.contains("What did we ship today?"),
        "{brief}"
    );
    assert!(
        !brief.contains("board_status") && !brief.contains("Left off"),
        "a thread's brief, not a card's: {brief}"
    );

    // It answers: the thread is answered, and its agent stays for
    // follow-ups.
    co.add_comment(&spawned, &tk.id, "We shipped #12.", false)
        .unwrap();
    wait_for(60, "the thread answered", || {
        co.get_task(&tk.id)
            .and_then(|t| t.ask)
            .is_some_and(|a| a.reason == coord::TAKE_ANSWERED)
    });
    r.core.set_spawn_cap(r.core.spawn_cap()).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert!(fake.ended.lock().unwrap().is_empty(), "it stays");
    assert_eq!(fake.started.lock().unwrap().len(), 1, "no second agent");

    // The person resolves it: its agent is ended.
    co.resolve_thread(&person, &tk.id).unwrap();
    wait_for(60, "the thread's agent ended", || {
        !fake.ended.lock().unwrap().is_empty()
    });
    assert_eq!(*fake.ended.lock().unwrap(), [SPAWNED_SESS]);
}

#[test]
fn an_agent_with_offer_it_cards_on_takes_a_thread_first() {
    let r = Rig::new();
    let (_repo, _dir) = agent_in_repo(&r);
    let fake = Arc::new(Fake::default());
    let co = r.core.coord();
    r.core
        .set_spawner(Some(spawner(&fake, r.claude.join("sessions"))));
    spawning_on(&r);
    r.core.set_agent_offer(&r.key_a, true).unwrap();
    let tk = thread(&r, "Is CI green?");
    wait_for(60, "alpha has the thread", || {
        co.get_task(&tk.id)
            .and_then(|t| t.ask)
            .is_some_and(|a| a.taker == r.key_a)
    });
    for _ in 0..3 {
        r.core.set_spawn_cap(r.core.spawn_cap()).unwrap();
    }
    std::thread::sleep(Duration::from_millis(500));
    assert!(fake.started.lock().unwrap().is_empty(), "no agent started");
}

/// Card #518: an unclaimed ask counts as having an agent coming only while
/// this device's sweep would start one. With no spawner it won't, so the
/// Core drops coord's hold and "Nobody took" shows at once.
#[test]
fn an_ask_has_no_agent_coming_while_this_device_cannot_start_one() {
    let r = Rig::new();
    spawning_on(&r);
    let person = format!("user:{}", r.core.device_id());
    let tk = r
        .core
        .coord()
        .post_ask(
            &person,
            coord::NewAsk {
                text: "anyone?".into(),
                thread: true,
                wake: true,
                ..coord::NewAsk::default()
            },
            &[],
        )
        .unwrap()
        .0;
    let held = r.core.coord().ask_starting(&tk.id).expect("coord holds it");
    assert!(!held.requested && held.device == r.core.device_id());
    assert!(
        !r.core.asks_starting().contains_key(&tk.id),
        "no spawner: nothing is coming"
    );
}
