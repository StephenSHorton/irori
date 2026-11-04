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
    /// How many more ends fail (card #474), as `claude rm` did.
    fail: std::sync::atomic::AtomicUsize,
    /// What a successful end says (`rm`'s output).
    says: Mutex<String>,
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
            if b.fail
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
            {
                return Err(format!("claude rm {s}: something went wrong"));
            }
            if let Some(n) = s.rsplit('-').next().and_then(|x| x.parse::<usize>().ok()) {
                let _ = std::fs::remove_file(s2.join(format!("9100{n:02}.json")));
            }
            Ok(b.says.lock().unwrap().clone())
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

/// The `agent.gone` rows for agent `key`.
fn gone_rows(s: &Setup, key: &str) -> usize {
    s.r.core
        .events(&store::Filter {
            kinds: vec![store::kinds::AGENT_GONE.into()],
            limit: store::MAX_LIST_LIMIT,
            ..store::Filter::default()
        })
        .unwrap()
        .into_iter()
        .filter(|e| {
            let p = String::from_utf8_lossy(&e.payload);
            // Irori's own; presence writes one more when the session goes.
            p.contains(key) && p.contains("\"finished\"")
        })
        .count()
}

fn ends(s: &Setup) -> usize {
    s.fake.ended.lock().unwrap().len()
}

/// Card #474: `claude rm` failing (it said "No job matching" for hours)
/// left the end in place, and every sweep wrote another `agent.gone` and
/// tried again. Now the row is written once, and stop and rm are tried
/// again only after END_RETRY, until they work.
#[test]
fn a_failing_end_writes_agent_gone_once_and_is_tried_again_later() {
    let s = at_done_check();
    s.fake.fail.store(usize::MAX, Ordering::SeqCst);
    past_grace(&s);
    wait_for(60, "the first end tried", || ends(&s) >= 1);
    // Many sweeps inside END_RETRY: no second try, one row.
    for _ in 0..5 {
        s.r.core.set_spawn_cap(s.r.core.spawn_cap()).unwrap();
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(ends(&s), 1, "tried again inside END_RETRY");
    assert_eq!(gone_rows(&s, &s.first()), 1);
    // After END_RETRY it is tried again, still with one row.
    skew_by(&s, irori_core::spawn::END_RETRY);
    wait_for(60, "the second try", || ends(&s) >= 2);
    assert_eq!(gone_rows(&s, &s.first()), 1);
    // It works the third time, and then it is left alone.
    s.fake.fail.store(0, Ordering::SeqCst);
    skew_by(&s, irori_core::spawn::END_RETRY);
    wait_for(60, "the third try", || ends(&s) >= 3);
    for _ in 0..2 {
        skew_by(&s, irori_core::spawn::END_RETRY);
        std::thread::sleep(Duration::from_millis(500));
    }
    assert_eq!(ends(&s), 3, "ended, then left alone");
    assert_eq!(gone_rows(&s, &s.first()), 1);
}

/// Card #474: an end that never works is given up after END_TRIES, so it
/// doesn't run all day.
#[test]
fn a_failing_end_is_given_up_after_its_tries() {
    let s = at_done_check();
    s.fake.fail.store(usize::MAX, Ordering::SeqCst);
    past_grace(&s);
    let tries = irori_core::spawn::END_TRIES as usize;
    for n in 1..=tries {
        wait_for(60, "a try", || ends(&s) >= n);
        skew_by(&s, irori_core::spawn::END_RETRY);
    }
    for _ in 0..3 {
        skew_by(&s, irori_core::spawn::END_RETRY);
        std::thread::sleep(Duration::from_millis(300));
    }
    // With no `claude agents` list the reaper never tries either.
    assert_eq!(ends(&s), tries, "given up after {tries}");
    assert_eq!(gone_rows(&s, &s.first()), 1);
}

/// Card #474: a worktree `rm` kept (unpushed work) is said once on the
/// card, with what rm said.
#[test]
fn a_kept_worktree_is_said_on_the_card() {
    let s = at_done_check();
    *s.fake.says.lock().unwrap() =
        "kept worktree /r/.claude/worktrees/fix-save: 2 unpushed commits".into();
    past_grace(&s);
    wait_for(60, "the agent ended", || ended(&s, 1));
    let co = s.r.core.coord();
    wait_for(30, "the kept line", || {
        co.comments(&s.t.id).unwrap_or_default().iter().any(|c| {
            c.text.starts_with(irori_core::spawn::KEPT_WORKTREE) && c.text.contains("fix-save")
        })
    });
}

/// Card #474: a job Irori ended that Claude Code still lists (its stop and
/// rm failed before) is removed by the sweep, with no new `agent.gone`.
#[test]
fn a_leftover_job_of_an_ended_agent_is_removed() {
    let s = at_done_check();
    s.fake.fail.store(usize::MAX, Ordering::SeqCst);
    past_grace(&s);
    let tries = irori_core::spawn::END_TRIES as usize;
    for n in 1..=tries {
        wait_for(60, "a try", || ends(&s) >= n);
        skew_by(&s, irori_core::spawn::END_RETRY);
    }
    // Claude Code still lists it, finished; rm works now.
    s.fake.fail.store(0, Ordering::SeqCst);
    let mut sp = spawner(&s.fake, s.r.claude.join("sessions"));
    let left = sess(1);
    sp.list = Some(Arc::new(move || {
        Ok(vec![ClaudeAgent {
            session_id: left.clone(),
            live: false,
        }])
    }));
    s.r.core.set_spawner(Some(sp));
    skew_by(&s, irori_core::spawn::REAP_EVERY);
    wait_for(60, "the leftover removed", || ends(&s) > tries);
    assert_eq!(gone_rows(&s, &s.first()), 1);
}

/// Card #474: an agent the log says Irori ended but that still runs (the
/// old build's ends all failed, and it went on working) is never stopped
/// by the reaper.
#[test]
fn the_reaper_leaves_a_running_agent_alone() {
    let s = at_done_check();
    s.fake.fail.store(usize::MAX, Ordering::SeqCst);
    past_grace(&s);
    let tries = irori_core::spawn::END_TRIES as usize;
    for n in 1..=tries {
        wait_for(60, "a try", || ends(&s) >= n);
        skew_by(&s, irori_core::spawn::END_RETRY);
    }
    s.fake.fail.store(0, Ordering::SeqCst);
    let mut sp = spawner(&s.fake, s.r.claude.join("sessions"));
    let running = sess(1);
    sp.list = Some(Arc::new(move || {
        Ok(vec![ClaudeAgent {
            session_id: running.clone(),
            live: true,
        }])
    }));
    s.r.core.set_spawner(Some(sp));
    for _ in 0..2 {
        skew_by(&s, irori_core::spawn::REAP_EVERY);
        std::thread::sleep(Duration::from_millis(700));
    }
    assert_eq!(ends(&s), tries, "a running agent was stopped");
}

/// A card the person posts that gets no spawned agent of its own (shared),
/// for a card to wait on.
fn other_card(s: &Setup) -> Task {
    s.r.core
        .coord()
        .create_task(
            &s.person,
            NewTask {
                title: "Land the API".into(),
                shared: true,
                ..NewTask::default()
            },
        )
        .unwrap()
}

/// Blocks the card on `other`, then on a question the person answers.
fn wait_on_card_and_answer(s: &Setup, other: &Task) {
    let co = s.r.core.coord();
    co.raise(
        &s.first(),
        &s.t.id,
        coord::NewBlocker {
            question: "waits on the API landing".into(),
            blocked_by: Some(coord::BlockedBy {
                kind: coord::BLOCKED_ON_TASK.into(),
                reference: other.id.clone(),
            }),
            ..coord::NewBlocker::default()
        },
    )
    .unwrap();
    let (_, b) = co
        .raise(
            &s.first(),
            &s.t.id,
            coord::NewBlocker {
                question: "which port?".into(),
                ..coord::NewBlocker::default()
            },
        )
        .unwrap();
    let q = b.id;
    assert_eq!(
        co.get_task(&s.t.id)
            .unwrap()
            .blockers
            .iter()
            .filter(|b| b.open())
            .count(),
        2
    );
    co.answer(
        &s.person,
        &s.t.id,
        &q,
        BlockerAnswer {
            text: "8080".into(),
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
}

/// Card #474: an agent parked on other work (its card blocked on another
/// card, nothing for it to act on) hands off and goes after the idle
/// limit; when that card is done its card goes back on the Board and a
/// fresh agent starts.
#[test]
fn parked_on_another_card_past_the_idle_limit_it_goes_and_a_fresh_one_starts_when_that_lands() {
    let s = bound();
    let co = s.r.core.coord();
    let other = other_card(&s);
    // The person answered a question, and the agent acted on it since (its
    // comment) but left it open: that doesn't keep it.
    wait_on_card_and_answer(&s, &other);
    std::thread::sleep(Duration::from_millis(50));
    co.add_comment(
        &s.first(),
        &s.t.id,
        "Using 8080; waiting on the API card now. The branch is pushed.",
        false,
    )
    .unwrap();
    // Inside the limit it stays.
    skew_by(&s, Duration::from_secs(60));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!ended(&s, 1), "ended inside the idle limit");
    skew_by(&s, IDLE_LIMIT);
    wait_for(60, "the parked agent ended", || ended(&s, 1));
    wait_for(90, "the parked agent off its card", || {
        co.get_task(&s.t.id).is_some_and(|t| t.holders.is_empty())
    });
    let cs = co.comments(&s.t.id).unwrap_or_default();
    assert!(
        cs.iter().all(|c| c.text != irori_core::spawn::NO_HANDOFF),
        "it handed off: {cs:?}"
    );
    assert_eq!(
        s.fake.started.lock().unwrap().len(),
        1,
        "no agent while it waits"
    );
    // The card it waited on is done: a fresh agent starts.
    co.set_task_state(&s.person, &other.id, "completed", "landed")
        .unwrap();
    wait_for(90, "a fresh agent started", || {
        s.fake.started.lock().unwrap().len() == 2
    });
}

/// Card #474: the person's answer it hasn't acted on yet keeps an agent
/// whose card also waits on another card.
#[test]
fn an_answer_to_act_on_keeps_an_agent_waiting_on_another_card() {
    let s = bound();
    let other = other_card(&s);
    wait_on_card_and_answer(&s, &other);
    skew_by(&s, IDLE_LIMIT);
    std::thread::sleep(Duration::from_millis(2000));
    assert!(!ended(&s, 1), "ended with his answer to act on");
}
