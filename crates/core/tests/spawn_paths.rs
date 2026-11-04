//! Card #447 (spawned agents §2, part 3) through the Core, with a fake
//! spawner: a spawned agent stays for DONE_GRACE after raising its done
//! check, so a quick "Not done" reaches the same session; after the grace
//! it is ended, and a later "Not done" puts the card back on the Board and
//! a fresh agent starts with the person's words in its brief.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use coord::{BlockerAnswer, NewTask, Task, Where};
use gocompat::time::GoTime;
use irori_core::spawn::{
    ClaudeAgent, DONE_GRACE, IDLE_LIMIT, RESTARTED, SpawnReq, SpawnSetting, SpawnStarted, Spawner,
};

/// The session id of the fake's nth spawn (1-based).
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
        spawn: Arc::new(move |r: &SpawnReq| {
            let n = {
                let mut v = a.started.lock().unwrap();
                v.push(r.clone());
                v.len()
            };
            let id = sess(n);
            write_session(
                &sessions,
                &format!("9100{n:02}.json"),
                &id,
                &r.name,
                Path::new(&r.folder),
            );
            Ok(SpawnStarted {
                name: r.name.clone(),
                prefix: id[..8].into(),
            })
        }),
        end: Arc::new(move |s: &str| {
            b.ended.lock().unwrap().push(s.to_string());
            if let Some(n) = s.rsplit('-').next().and_then(|x| x.parse::<usize>().ok()) {
                let _ = std::fs::remove_file(s2.join(format!("9100{n:02}.json")));
            }
            Ok(())
        }),
        trusted: Arc::new(|_| true),
        list: None,
    }
}

/// A rig whose clock runs `skew` seconds ahead of the wall.
fn rig(skew: &Arc<AtomicI64>) -> Rig {
    let s = skew.clone();
    Rig::with(move |o| {
        o.now = Some(Arc::new(move || {
            let d = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap();
            GoTime::unix(
                d.as_secs() as i64 + s.load(Ordering::SeqCst),
                i64::from(d.subsec_nanos()),
            )
        }));
    })
}

struct Setup {
    r: Rig,
    fake: Arc<Fake>,
    skew: Arc<AtomicI64>,
    person: String,
    t: Task,
    _work: tempfile::TempDir,
}

/// A card on the private board with spawning on, its first agent bound,
/// and that agent's done check raised.
fn at_done_check() -> Setup {
    let s = bound();
    let co = s.r.core.coord();
    co.set_task_state(&s.first(), &s.t.id, "completed", "done: saving works")
        .unwrap();
    s
}

impl Setup {
    /// The key of the fake's nth agent.
    fn agent(&self, n: usize) -> String {
        format!("{}/{}", self.r.core.device_id(), sess(n))
    }

    fn first(&self) -> String {
        self.agent(1)
    }
}

/// A card on the private board with spawning on and its first agent bound.
fn bound() -> Setup {
    let skew = Arc::new(AtomicI64::new(0));
    let r = rig(&skew);
    let work = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    let me = r.core.device_id().to_string();
    let person = format!("user:{me}");
    let co = r.core.coord();
    let t = co
        .create_task(
            &person,
            NewTask {
                title: "Fix the save".into(),
                objective: "saving works".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.set_where(
        &t.id,
        Where {
            who: r.key_a.clone(),
            cwd: work.path().to_string_lossy().into(),
            worktree: work.path().to_string_lossy().into(),
            ..Where::default()
        },
    )
    .unwrap();
    r.core
        .set_spawner(Some(spawner(&fake, r.claude.join("sessions"))));
    r.core
        .set_spawn_setting(SpawnSetting {
            board_id: String::new(),
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    let first = format!("{me}/{}", sess(1));
    wait_for(60, "the first agent bound", || {
        co.get_task(&t.id)
            .is_some_and(|t| t.holders.contains(&first))
    });
    Setup {
        r,
        fake,
        skew,
        person,
        t,
        _work: work,
    }
}

fn check_of(co: &coord::Coord, id: &str) -> String {
    co.get_task(id)
        .unwrap()
        .blockers
        .iter()
        .find(|b| b.open() && b.done_check)
        .expect("a done check")
        .id
        .clone()
}

fn not_done(s: &Setup, words: &str) {
    let co = s.r.core.coord();
    let b = check_of(co, &s.t.id);
    co.answer(
        &s.person,
        &s.t.id,
        &b,
        BlockerAnswer {
            text: words.into(),
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
}

fn past_grace(s: &Setup) {
    s.skew
        .fetch_add(DONE_GRACE.as_secs() as i64 + 10, Ordering::SeqCst);
    // A roster change kicks the sweep.
    s.r.core.set_spawn_cap(s.r.core.spawn_cap()).unwrap();
}

fn ended(s: &Setup, n: usize) -> bool {
    s.fake.ended.lock().unwrap().contains(&sess(n))
}

#[test]
fn the_agent_stays_through_the_grace_then_ends() {
    let s = at_done_check();
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!ended(&s, 1), "ended inside the grace");
    past_grace(&s);
    wait_for(60, "the agent ended after the grace", || ended(&s, 1));
}

#[test]
fn a_quick_not_done_reaches_the_same_agent() {
    let s = at_done_check();
    not_done(&s, "the save still 500s");
    past_grace(&s);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!ended(&s, 1), "it was ended with his answer to act on");
    let co = s.r.core.coord();
    let me = s.r.core.device_id().to_string();
    assert!(
        co.get_task(&s.t.id)
            .unwrap()
            .holders
            .contains(&format!("{me}/{}", sess(1)))
    );
    assert_eq!(s.fake.started.lock().unwrap().len(), 1, "no second agent");
}

#[test]
fn a_late_not_done_starts_a_fresh_agent_with_his_words() {
    let s = at_done_check();
    let co = s.r.core.coord();
    past_grace(&s);
    wait_for(60, "the first agent ended", || ended(&s, 1));
    wait_for(90, "the first agent off its card", || {
        co.get_task(&s.t.id).is_some_and(|t| t.holders.is_empty())
    });
    not_done(&s, "the save still 500s on the second tab");
    wait_for(60, "a fresh agent started", || {
        s.fake.started.lock().unwrap().len() == 2
    });
    let req = s.fake.started.lock().unwrap()[1].clone();
    let brief = std::fs::read_to_string(&req.brief_path).unwrap();
    assert!(
        brief.contains("the save still 500s on the second tab"),
        "{brief}"
    );
    // Card #476: it answers him on the check he sent back, by id, then
    // brings proof that says what changed.
    let check =
        s.r.core
            .coord()
            .sent_back(&s.t.id)
            .expect("the check he sent back")
            .id;
    assert!(brief.contains(&format!("blocker_id={check}")), "{brief}");
    assert!(brief.contains("Answer them on it first"), "{brief}");
    assert!(brief.contains("what changed"), "{brief}");
    let me = s.r.core.device_id().to_string();
    let second = format!("{me}/{}", sess(2));
    wait_for(60, "the fresh agent bound", || {
        co.get_task(&s.t.id)
            .is_some_and(|t| t.holders.contains(&second))
    });
}

fn skew_by(s: &Setup, d: Duration) {
    s.skew.fetch_add(d.as_secs() as i64 + 10, Ordering::SeqCst);
    s.r.core.set_spawn_cap(s.r.core.spawn_cap()).unwrap();
}

#[test]
fn stuck_on_a_question_past_the_idle_limit_it_goes_and_his_answer_brings_a_fresh_one() {
    let s = bound();
    let co = s.r.core.coord();
    let t = co.block(&s.first(), &s.t.id, "which port?", None).unwrap();
    let b = t.blockers.iter().find(|b| b.open()).unwrap().id.clone();
    // Inside the limit it waits for him.
    skew_by(&s, Duration::from_secs(60));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!ended(&s, 1), "ended inside the idle limit");
    // Its hand-off, which Irori asks for before it ends it.
    co.add_comment(
        &s.first(),
        &s.t.id,
        "Waiting on the port; the server code is on the branch.",
        false,
    )
    .unwrap();
    skew_by(&s, IDLE_LIMIT);
    wait_for(60, "the stuck agent ended", || ended(&s, 1));
    wait_for(90, "the stuck agent off its card", || {
        co.get_task(&s.t.id).is_some_and(|t| t.holders.is_empty())
    });
    co.answer(
        &s.person,
        &s.t.id,
        &b,
        BlockerAnswer {
            text: "use 8080, the 3000s are taken".into(),
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    wait_for(60, "a fresh agent started", || {
        s.fake.started.lock().unwrap().len() == 2
    });
    let req = s.fake.started.lock().unwrap()[1].clone();
    let brief = std::fs::read_to_string(&req.brief_path).unwrap();
    assert!(brief.contains("use 8080, the 3000s are taken"), "{brief}");
}

#[test]
fn pause_all_starts_no_agent_until_it_lifts() {
    let s = at_done_check();
    let co = s.r.core.coord();
    past_grace(&s);
    wait_for(60, "the first agent ended", || ended(&s, 1));
    wait_for(90, "the first agent off its card", || {
        co.get_task(&s.t.id).is_some_and(|t| t.holders.is_empty())
    });
    let inbox = s.r.core.inbox();
    let mut pol = inbox.policy();
    pol.paused = true;
    inbox.set_policy(pol.clone(), "user:test").unwrap();
    not_done(&s, "still broken");
    s.r.core.set_spawn_cap(s.r.core.spawn_cap()).unwrap();
    std::thread::sleep(Duration::from_millis(2000));
    assert_eq!(
        s.fake.started.lock().unwrap().len(),
        1,
        "nothing starts while paused"
    );
    pol.paused = false;
    inbox.set_policy(pol, "user:test").unwrap();
    s.r.core.set_spawn_cap(s.r.core.spawn_cap()).unwrap();
    wait_for(60, "the agent started once Pause all lifted", || {
        s.fake.started.lock().unwrap().len() == 2
    });
}

#[test]
fn after_a_restart_its_gone_agent_ends_as_finished_and_a_fresh_one_starts() {
    let s = bound();
    let co = s.r.core.coord();
    // The machine restarted: the --bg session is gone, and Claude Code
    // lists it as finished.
    let _ = std::fs::remove_file(s.r.claude.join("sessions").join("910001.json"));
    let mut sp = spawner(&s.fake, s.r.claude.join("sessions"));
    let gone = sess(1);
    sp.list = Some(Arc::new(move || {
        Ok(vec![ClaudeAgent {
            session_id: gone.clone(),
            live: false,
        }])
    }));
    s.r.core.set_spawner(Some(sp));
    wait_for(60, "the gone agent removed", || ended(&s, 1));
    let cs = co.comments(&s.t.id).unwrap_or_default();
    assert!(cs.iter().any(|c| c.text == RESTARTED), "{cs:?}");
    // It leaves as finished after the short grace, and a fresh agent
    // picks the card up.
    skew_by(&s, Duration::from_secs(40));
    wait_for(60, "a fresh agent started", || {
        s.fake.started.lock().unwrap().len() == 2
    });
    let t = co.get_task(&s.t.id).unwrap();
    assert!(
        t.blockers
            .iter()
            .all(|b| b.question != coord::CRASHED_QUESTION),
        "a restart isn't a crash: {:?}",
        t.blockers
    );
}

#[test]
fn a_live_agent_survives_the_cleanup() {
    let s = bound();
    let mut sp = spawner(&s.fake, s.r.claude.join("sessions"));
    let live = sess(1);
    sp.list = Some(Arc::new(move || {
        Ok(vec![ClaudeAgent {
            session_id: live.clone(),
            live: true,
        }])
    }));
    s.r.core.set_spawner(Some(sp));
    s.r.core.set_spawn_cap(s.r.core.spawn_cap()).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!ended(&s, 1));
    assert!(
        s.r.core
            .coord()
            .get_task(&s.t.id)
            .unwrap()
            .holders
            .contains(&s.first())
    );
}
