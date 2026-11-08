//! Card #470 through the Core, with a fake spawner in place of `claude
//! --bg`: a thread the person starts that no agent with "Offer it cards"
//! on takes gets its own background agent. It starts in the repo of the
//! agent here active last (the window's threads name none), with a brief
//! for a thread; bound, it takes the thread, answers in it, and stays for
//! follow-ups until the person resolves it. An idle agent with "Offer it
//! cards" on still takes a thread first.

use crate::common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use irori_core::spawn::{DEVICE_FREE, SpawnReq, SpawnSetting, SpawnStarted, Spawner};

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
        ready: None,
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
    assert!(brief.contains(DEVICE_FREE), "{brief}");
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

/// Card #631 through the Core: a thread an agent opens itself starts no
/// agent, with spawning on, while it holds it and after it gives it back;
/// the person's post in it goes to the opener while it lives.
#[test]
fn an_agents_own_thread_starts_no_agent_until_the_person_writes() {
    let r = Rig::new();
    let (_repo, _dir) = agent_in_repo(&r);
    let fake = Arc::new(Fake::default());
    let co = r.core.coord();
    r.core
        .set_spawner(Some(spawner(&fake, r.claude.join("sessions"))));
    spawning_on(&r);
    let (tk, _) = co
        .post_agent_thread(
            &r.key_a,
            coord::NewAsk {
                text: "Should the beta go out today?".into(),
                thread: true,
                ..coord::NewAsk::default()
            },
        )
        .unwrap();
    let nothing_starts = |why: &str| {
        for _ in 0..3 {
            r.core.set_spawn_cap(r.core.spawn_cap()).unwrap();
        }
        std::thread::sleep(Duration::from_millis(500));
        assert!(fake.started.lock().unwrap().is_empty(), "{why}");
    };
    nothing_starts("held by its opener");
    assert_eq!(co.get_task(&tk.id).unwrap().ask.unwrap().taker, r.key_a);
    co.leave(&r.key_a, &tk.id, &r.key_a).unwrap();
    nothing_starts("given back");
    // The person writes while its opener is live: the post goes to the
    // opener, and still no other agent starts.
    let person = format!("user:{}", r.core.device_id());
    let (_, ns) = co
        .person_comment(&person, &tk.id, "Yes, today.", true, &[], None)
        .unwrap();
    assert!(
        ns.iter().any(|n| n.to.contains(&r.key_a)),
        "its opener hears it: {ns:?}"
    );
    nothing_starts("the person wrote, its opener live");
}

// Card #454 (spawned-agents.md §4, §14 part 10): a thread started from a
// message in a Talk channel. It names its channel and the channel's repo;
// its agent starts in that repo (not the repo of the agent here active
// last), and its brief carries the channel's newest lines as data, the
// idle limit, and the note to propose.

/// A second repo, with a GitHub remote, that capture knows (as it would
/// from an agent that worked there) but that no agent here works in now:
/// so the repo of the agent here active last is another one.
fn known_repo(r: &Rig, name: &str) -> (tempfile::TempDir, PathBuf, String) {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().canonicalize().unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["commit", "-q", "--allow-empty", "-m", "first"]);
    let repo_key = format!("remote:github.com/t/{name}");
    r.core
        .capture()
        .emit_row(
            "irori",
            capture::KIND_REPO_SEEN,
            &capture::RepoSeen {
                repo_key: repo_key.clone(),
                root: dir.to_string_lossy().into_owned(),
                at: gocompat::time::GoTime::unix(1_790_000_000, 0),
            },
        )
        .unwrap();
    assert!(!r.core.capture().repo_roots(&repo_key).is_empty());
    (d, dir, repo_key)
}

#[test]
fn a_thread_from_a_repo_channel_starts_its_agent_in_that_repo_with_the_channel() {
    let r = Rig::new();
    let (_game, game_dir, game_key) = known_repo(&r, "game-repo");
    let (_repo, _dir) = agent_in_repo(&r);
    let fake = Arc::new(Fake::default());
    let me = r.core.device_id().to_string();
    let person = format!("user:{me}");
    let spawned = format!("{me}/{SPAWNED_SESS}");
    let co = r.core.coord();

    // The person's Personal channel about the game repo, with a few lines.
    let ch = r
        .core
        .talk_channel_create(&person, "game-repo", "the game")
        .unwrap();
    for (by, text) in [
        (person.as_str(), "main is red after #212"),
        (r.key_a.as_str(), "the save test times out on Windows"),
    ] {
        r.core
            .talk()
            .post(coord::talk::NewPost {
                conv_id: ch.id.clone(),
                text: text.into(),
                by: by.into(),
                ..coord::talk::NewPost::default()
            })
            .unwrap();
    }
    // A post of files alone (card #643) reads as Talk's one line for it.
    r.core
        .talk()
        .post(coord::talk::NewPost {
            conv_id: ch.id.clone(),
            by: person.clone(),
            files: vec![blob::FileRef {
                sha256: "a".repeat(64),
                name: "save-trace.log".into(),
                size: 10,
                ..blob::FileRef::default()
            }],
            ..coord::talk::NewPost::default()
        })
        .unwrap();

    r.core
        .set_spawner(Some(spawner(&fake, r.claude.join("sessions"))));
    spawning_on(&r);
    let (tk, _) = co
        .post_ask(
            &person,
            coord::NewAsk {
                text: "Why is main red?\n\n> main is red after #212\n(you, in #game-repo)".into(),
                thread: true,
                channel: ch.id.clone(),
                repo_key: game_key.clone(),
                ..coord::NewAsk::default()
            },
            &r.core.ask_agents(),
        )
        .unwrap();
    wait_for(60, "the spawned agent took the thread", || {
        co.get_task(&tk.id)
            .and_then(|t| t.ask)
            .is_some_and(|a| a.taker == spawned)
    });
    let reqs = fake.started.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1, "one spawn: {reqs:?}");
    let req = &reqs[0];
    assert_eq!(
        Path::new(&req.folder).canonicalize().unwrap(),
        game_dir,
        "the channel's repo, not the repo of the agent active last"
    );
    let brief = std::fs::read_to_string(&req.brief_path).unwrap();
    assert!(
        brief.contains("The newest lines in #game-repo")
            && brief.contains("the save test times out on Windows")
            && brief.contains("doesn't make you one")
            && brief.contains("📎 save-trace.log"),
        "the channel's lines, as data: {brief}"
    );
    assert!(
        brief.contains("notes_propose") && brief.contains("quiet for 30 minutes"),
        "{brief}"
    );
    // Its agent isn't made a member of the channel.
    assert!(
        !r.core
            .talk()
            .members(&ch.id)
            .iter()
            .any(|m| m.key == spawned)
    );
}

#[test]
fn a_thread_the_person_makes_a_card_gets_a_card_agent_in_the_threads_home() {
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
    let tk = thread(&r, "Can the board sort by age?");
    wait_for(60, "the thread's agent took it", || {
        co.get_task(&tk.id)
            .and_then(|t| t.ask)
            .is_some_and(|a| a.taker == spawned)
    });
    co.add_comment(&spawned, &tk.id, "Not yet; it's small.", false)
        .unwrap();
    wait_for(60, "the thread answered", || {
        co.get_task(&tk.id)
            .and_then(|t| t.ask)
            .is_some_and(|a| a.reason == coord::TAKE_ANSWERED)
    });
    co.ask_close(
        &person,
        &tk.id,
        coord::AskOutcome {
            kind: coord::OUTCOME_CARD.into(),
            title: "Sort the board by age".into(),
            ..coord::AskOutcome::default()
        },
    )
    .unwrap();
    // The thread's agent goes; the card, naming no repo, gets a card's
    // agent where the thread's started, not a "no copy" blocker.
    wait_for(60, "the thread's agent ended", || {
        !fake.ended.lock().unwrap().is_empty()
    });
    wait_for(60, "a card agent asked for", || {
        r.core.set_spawn_cap(r.core.spawn_cap()).unwrap();
        fake.started.lock().unwrap().len() == 2
    });
    let reqs = fake.started.lock().unwrap().clone();
    assert_eq!(
        Path::new(&reqs[1].folder).canonicalize().unwrap(),
        dir,
        "the thread's home"
    );
    let brief = std::fs::read_to_string(&reqs[1].brief_path).unwrap();
    assert!(
        brief.contains("Irori started you to work card") && brief.contains("Sort the board by age"),
        "a card's brief: {brief}"
    );
    let t = co.get_task(&tk.id).unwrap();
    assert!(
        t.blockers.iter().all(|b| !b.open()),
        "no blocker: {:?}",
        t.blockers
    );
}
