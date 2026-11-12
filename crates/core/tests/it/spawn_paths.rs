//! Card #447 (spawned agents §2, part 3) through the Core, with a fake
//! spawner: a spawned agent stays for DONE_GRACE after raising its done
//! check, so a quick "Not done" reaches the same session; after the grace
//! it is ended, and a later "Not done" puts the card back on the Board and
//! a fresh agent starts with the person's words in its brief.

use crate::common;

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
        ready: None,
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
    bound_weighed("")
}

/// [`bound`] for a card the person weighed `level` first.
fn bound_weighed(level: &str) -> Setup {
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
    if !level.is_empty() {
        co.set_weight(&person, &t.id, level, "").unwrap();
    }
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
    // Already heavy: nothing to move up to, so the session stays (#966).
    let s = bound_weighed("heavy");
    s.r.core
        .coord()
        .set_task_state(&s.first(), &s.t.id, "completed", "done: saving works")
        .unwrap();
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
            ..ClaudeAgent::default()
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
            ..ClaudeAgent::default()
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

/// Card #565: `rm` keeps saying kept about a folder that isn't a worktree
/// any more, and Claude Code keeps listing the job. It is asked once: 92
/// log lines came from asking at every reap.
#[test]
fn a_job_rm_kept_is_asked_about_once() {
    let s = at_done_check();
    *s.fake.says.lock().unwrap() =
        "kept f83d401f, its worktree is still at /r/.claude/worktrees/506-solid-popovers".into();
    let mut sp = spawner(&s.fake, s.r.claude.join("sessions"));
    let left = sess(1);
    sp.list = Some(Arc::new(move || {
        Ok(vec![ClaudeAgent {
            session_id: left.clone(),
            live: false,
            ..ClaudeAgent::default()
        }])
    }));
    s.r.core.set_spawner(Some(sp));
    past_grace(&s);
    wait_for(60, "the agent ended", || ended(&s, 1));
    let asked = ends(&s);
    for _ in 0..3 {
        skew_by(&s, irori_core::spawn::REAP_EVERY);
        std::thread::sleep(Duration::from_millis(700));
    }
    assert_eq!(ends(&s), asked, "rm was asked again");
    assert_eq!(gone_rows(&s, &s.first()), 1);
    let lines = s.r.core.coord().comments(&s.t.id).unwrap_or_default();
    let said = lines
        .iter()
        .filter(|c| c.text.starts_with(irori_core::spawn::KEPT_WORKTREE))
        .count();
    assert_eq!(said, 1, "{lines:?}");
    let book = irori_core::treeclean::Book::load(s.r.core.data_dir());
    assert!(book.kept_jobs.contains(&s.first()), "it outlives a restart");
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
            ..ClaudeAgent::default()
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
            ..ClaudeAgent::default()
        }])
    }));
    s.r.core.set_spawner(Some(sp));
    for _ in 0..2 {
        skew_by(&s, irori_core::spawn::REAP_EVERY);
        std::thread::sleep(Duration::from_millis(700));
    }
    assert_eq!(ends(&s), tries, "a running agent was stopped");
}

/// Card #565 finding A: an end that only stopped the job (its tree is on
/// disk) is an end, says nothing on the card, and isn't a kept job: the
/// reap asks again once the tree goes.
#[test]
fn an_end_that_only_stopped_is_an_end_and_not_a_kept_job() {
    let s = at_done_check();
    *s.fake.says.lock().unwrap() = format!(
        "{} its folder /r/.claude/worktrees/card-1-1 is a worktree Irori removes itself",
        irori_core::spawn::STOPPED_ONLY
    );
    past_grace(&s);
    wait_for(60, "the agent ended", || ended(&s, 1));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(gone_rows(&s, &s.first()), 1);
    let lines = s.r.core.coord().comments(&s.t.id).unwrap_or_default();
    assert!(
        !lines
            .iter()
            .any(|c| c.text.starts_with(irori_core::spawn::KEPT_WORKTREE)),
        "{lines:?}"
    );
    let book = irori_core::treeclean::Book::load(s.r.core.data_dir());
    assert!(!book.kept_jobs.contains(&s.first()));
}

/// Card #565 finding A: `claude rm` deletes a job's worktree without
/// Irori's checks, so the reaper leaves a finished job whose folder is a
/// tree still on disk, and asks again once the tree has gone.
#[test]
fn the_reaper_waits_for_a_jobs_tree_to_go() {
    let s = at_done_check();
    s.fake.fail.store(usize::MAX, Ordering::SeqCst);
    past_grace(&s);
    let tries = irori_core::spawn::END_TRIES as usize;
    for n in 1..=tries {
        wait_for(60, "a try", || ends(&s) >= n);
        skew_by(&s, irori_core::spawn::END_RETRY);
    }
    s.fake.fail.store(0, Ordering::SeqCst);
    let d = tempfile::tempdir().unwrap();
    let tree = d.path().join("repo/.claude/worktrees/card-9-1");
    std::fs::create_dir_all(tree.join("src")).unwrap();
    let mut sp = spawner(&s.fake, s.r.claude.join("sessions"));
    let left = sess(1);
    let cwd = tree.join("src").to_string_lossy().into_owned();
    sp.list = Some(Arc::new(move || {
        Ok(vec![ClaudeAgent {
            session_id: left.clone(),
            live: false,
            cwd: cwd.clone(),
        }])
    }));
    s.r.core.set_spawner(Some(sp));
    // The cleanup after a new spawner ends the listed job once more, the
    // usual way (the window's end then only stops it, as its tree is on
    // disk); the reaper doesn't ask again while the tree is there.
    std::thread::sleep(Duration::from_millis(1500));
    let base = ends(&s);
    assert!(base <= tries + 1, "{base}");
    for _ in 0..2 {
        skew_by(&s, irori_core::spawn::REAP_EVERY);
        std::thread::sleep(Duration::from_millis(700));
    }
    assert_eq!(ends(&s), base, "rm was asked while its tree was there");
    assert!(tree.join("src").is_dir());
    std::fs::remove_dir_all(&tree).unwrap();
    skew_by(&s, irori_core::spawn::REAP_EVERY);
    wait_for(60, "the job removed once its tree went", || ends(&s) > base);
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

/// Calls tool `name` as the rig's agent A.
fn tool_as_a(s: &Setup, name: &str, args: serde_json::Value) -> String {
    let c = tools::Caller {
        sender: inbox::Sender::new(&s.r.key_a, "alpha", "claude"),
        path: inbox::path::MOD.into(),
    };
    s.r.core
        .tools()
        .call(&c, name, &serde_json::to_vec(&args).unwrap())
        .unwrap()
}

/// Card #665: with Pause agents on, the person's End starts no fresh agent
/// and a new card starts none; turning it off (with no other kick) starts
/// both, the older card first.
#[test]
fn pause_agents_holds_the_fresh_agent_after_end_and_new_cards_until_it_lifts() {
    let s = bound();
    let co = s.r.core.coord();
    let p = s.r.core.set_agents_paused(true).unwrap();
    assert!(p.on && !p.since.is_zero(), "{p:?}");
    assert_eq!(p.by, s.person);
    // End: the hand-off wait passes, the agent goes and leaves its card.
    s.r.core.end_spawned(&s.first()).unwrap();
    s.skew.fetch_add(
        irori_core::spawn::HANDOFF_WAIT.as_secs() as i64 + 10,
        Ordering::SeqCst,
    );
    wait_for(60, "the agent ended", || ended(&s, 1));
    s.skew.fetch_add(40, Ordering::SeqCst);
    wait_for(90, "the agent off its card", || {
        co.get_task(&s.t.id).is_some_and(|t| t.holders.is_empty())
    });
    let other = co
        .create_task(
            &s.person,
            NewTask {
                title: "Another".into(),
                objective: "more".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.set_where(
        &other.id,
        Where {
            who: s.r.key_a.clone(),
            cwd: s._work.path().to_string_lossy().into(),
            worktree: s._work.path().to_string_lossy().into(),
            ..Where::default()
        },
    )
    .unwrap();
    s.r.core.set_spawn_cap(s.r.core.spawn_cap()).unwrap();
    std::thread::sleep(Duration::from_millis(2000));
    assert_eq!(
        s.fake.started.lock().unwrap().len(),
        1,
        "nothing starts while agents are paused"
    );
    let why = s.r.core.why_no_agent(&other.id, true).unwrap().to_string();
    assert!(why.contains("Stop here is on for"), "{why}");
    // Off, with nothing else kicking the sweep.
    let p = s.r.core.set_agents_paused(false).unwrap();
    assert!(!p.on);
    wait_for(60, "both cards started once Pause agents lifted", || {
        s.fake.started.lock().unwrap().len() == 3
    });
    let started = s.fake.started.lock().unwrap().clone();
    assert_eq!(started[1].task_id, s.t.id, "the older card first");
    assert_eq!(started[2].task_id, other.id);
}

/// Card #665: an agent pauses and unpauses with agents_pause, and reads it.
#[test]
fn an_agent_pauses_and_unpauses_agent_starts() {
    let s = bound();
    let said = tool_as_a(&s, "agents_pause", serde_json::json!({}));
    assert!(said.contains("Pause agents is off"), "{said}");
    let said = tool_as_a(&s, "agents_pause", serde_json::json!({ "on": true }));
    assert!(said.contains("Pause agents is on"), "{said}");
    let p = s.r.core.agents_pause();
    assert!(p.on);
    assert_eq!(p.by, s.r.key_a, "the row names the agent");
    let said = tool_as_a(&s, "agents_pause", serde_json::json!({}));
    assert!(
        said.contains("Pause agents is on for") && said.contains("by alpha"),
        "{said}"
    );
    let said = tool_as_a(&s, "agents_pause", serde_json::json!({ "on": false }));
    assert!(said.contains("Pause agents is off"), "{said}");
    assert!(!s.r.core.agents_pause().on);
}

/// Card #1196 follow-up: an agent's agents_pause off lifts the old inbox
/// Pause all too, as the person's Stop here off does, even with Pause
/// agents already off. Before, an agent asked to get cards going again
/// turned starts "on" while the old pause still held every wake and start.
#[test]
fn an_agents_pause_off_lifts_the_old_pause_all() {
    let s = bound();
    let inbox = s.r.core.inbox();
    let mut pol = inbox.policy();
    pol.paused = true;
    inbox.set_policy(pol, "user:test").unwrap();
    assert!(s.r.core.inbox().policy().paused);
    assert!(!s.r.core.agents_pause().on);
    // Pause on leaves it alone.
    let said = tool_as_a(&s, "agents_pause", serde_json::json!({ "on": true }));
    assert!(said.contains("Pause agents is on"), "{said}");
    assert!(s.r.core.inbox().policy().paused, "on doesn't lift it");
    let said = tool_as_a(&s, "agents_pause", serde_json::json!({ "on": false }));
    assert!(said.contains("Pause agents is off"), "{said}");
    assert!(!s.r.core.inbox().policy().paused, "off lifts it");
}

/// The agents tool lists the person's devices with Stop here, so a device
/// with no agents doesn't read as offline.
#[test]
fn the_agents_tool_lists_devices_with_stop_here() {
    let s = bound();
    let said = tool_as_a(&s, "agents", serde_json::json!({}));
    assert!(said.contains("The person's devices"), "{said}");
    assert!(
        said.contains(": online, this device, Stop here off, "),
        "{said}"
    );
    s.r.core.set_agents_paused(true).unwrap();
    // A filter that matches no agent still lists the devices.
    let said = tool_as_a(&s, "agents", serde_json::json!({ "status": "nobody" }));
    assert!(
        said.contains(": online, this device, Stop here on (by the person"),
        "{said}"
    );
}

/// The person's wind-down to the agent bound in `s` (card #787).
fn wind_down_first(s: &Setup) -> inbox::Delivery {
    s.r.core
        .inbox()
        .send_with(
            &s.r.core.user_sender(),
            &inbox::Address::new(inbox::addr::AGENT, &s.first()),
            "Wind down",
            inbox::SendOptions {
                mode: inbox::mode::WAKE.into(),
                reason: inbox::REASON_WIND_DOWN.into(),
                ..inbox::SendOptions::default()
            },
        )
        .unwrap()
        .1
}

/// It posts its hand-off and leaves its card, as a wind-down asks, before
/// it replies.
fn hand_off_and_leave(s: &Setup) {
    let co = s.r.core.coord();
    let k = s.first();
    co.add_comment(&k, &s.t.id, "Where: pushed. Next: the save.", false)
        .unwrap();
    co.leave(&k, &s.t.id, &k).unwrap();
    s.r.core.set_spawn_cap(s.r.core.spawn_cap()).unwrap();
}

/// Card #787: an agent that left its card while winding down is ended once
/// it replied "wound down", not before (its row would stay winding).
#[test]
fn an_agent_winding_down_is_ended_once_it_answered() {
    let s = bound();
    s.r.core.set_agents_paused(true).unwrap();
    let d = wind_down_first(&s);
    hand_off_and_leave(&s);
    std::thread::sleep(Duration::from_millis(2000));
    assert!(!ended(&s, 1), "ended before it answered the wind-down");
    s.r.core
        .inbox()
        .reply(
            &inbox::Sender::new(&s.first(), "alpha", "claude"),
            &d.id,
            "wound down; left the card",
        )
        .unwrap();
    s.r.core.set_spawn_cap(s.r.core.spawn_cap()).unwrap();
    wait_for(60, "the agent ended once it answered", || ended(&s, 1));
    let got = s.r.core.inbox().get(&d.id).unwrap();
    assert_eq!(got.state, inbox::st::ANSWERED);
}

/// Card #787: one that never answers is ended after the hand-off wait, and
/// its wind-down closes then, so Wind down's row reads no answer.
#[test]
fn an_agent_ended_without_answering_closes_its_wind_down() {
    let s = bound();
    s.r.core.set_agents_paused(true).unwrap();
    let d = wind_down_first(&s);
    hand_off_and_leave(&s);
    // The sweep notes its leaving on its own time: the clock moves past the
    // wait again until it is ended, so a late sweep can't restart the wait.
    wait_for(60, "the agent ended after the wait", || {
        if ended(&s, 1) {
            return true;
        }
        skew_by(&s, irori_core::spawn::HANDOFF_WAIT);
        s.r.core.set_spawn_cap(s.r.core.spawn_cap()).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        false
    });
    wait_for(10, "its wind-down closed", || {
        s.r.core
            .inbox()
            .get(&d.id)
            .is_some_and(|x| x.state == inbox::st::SKIPPED)
    });
}

/// Card #787: while Stop everywhere is on, an agent's agents_pause off is
/// refused, so its device starts nothing until the person turns it off.
#[test]
fn an_agent_cant_lift_stop_everywhere() {
    let s = bound();
    let me = s.r.core.device_id().to_string();
    s.r.core.wind_down_pause(std::slice::from_ref(&me)).unwrap();
    let said = tool_as_a(&s, "agents_pause", serde_json::json!({ "on": false }));
    assert!(said.contains("Stop everywhere is on"), "{said}");
    assert!(s.r.core.agents_pause().on, "still paused");
    assert!(s.r.core.agents_paused_on(&me));
    // The person's Off lifts it.
    s.r.core.wind_down_lift().unwrap();
    assert!(!s.r.core.agents_paused_on(&me));
    let said = tool_as_a(&s, "agents_pause", serde_json::json!({}));
    assert!(said.contains("Pause agents is off"), "{said}");
}

/// The model each started agent got, in order.
fn models(s: &Setup) -> Vec<(String, String)> {
    s.fake
        .started
        .lock()
        .unwrap()
        .iter()
        .map(|r| (r.model.clone(), r.effort.clone()))
        .collect()
}

fn sonnet_then_opus() -> Vec<(String, String)> {
    vec![
        ("sonnet".to_string(), "medium".to_string()),
        ("opus".to_string(), "high".to_string()),
    ]
}

/// Card #966: an agent moves its card up: left_off goes on the card, the
/// agent is ended, and a fresh one starts one weight higher from it.
#[test]
fn an_agent_moves_its_card_up_and_the_next_carries_on_from_left_off() {
    let s = bound();
    let co = s.r.core.coord();
    co.set_steps(
        &s.first(),
        &s.t.id,
        vec![coord::Step {
            text: "fix it".into(),
            ..coord::Step::default()
        }],
    )
    .unwrap();
    let got = co
        .move_weight(
            &s.first(),
            &s.t.id,
            true,
            "the cause is unknown",
            "reproduced it; the lock is next",
        )
        .unwrap();
    assert_eq!(got.weight.as_ref().unwrap().level, "heavy");
    assert!(!got.holders.contains(&s.first()), "it left the card");
    wait_for(60, "the first agent ended", || {
        s.skew.fetch_add(30, Ordering::SeqCst);
        s.r.core.set_spawn_cap(s.r.core.spawn_cap()).unwrap();
        ended(&s, 1)
    });
    wait_for(60, "a fresh agent", || {
        s.fake.started.lock().unwrap().len() == 2
    });
    assert_eq!(models(&s), sonnet_then_opus());
    let brief = std::fs::read_to_string(&s.fake.started.lock().unwrap()[1].brief_path).unwrap();
    assert!(brief.contains("reproduced it; the lock is next"), "{brief}");
    let t = co.get_task(&s.t.id).unwrap();
    assert_eq!(t.spawn_request().unwrap().weight, "heavy");
    assert_eq!(t.spawn_crashes, 0, "a move is no crash");
}

/// Card #966: the person's Not done ends the agent still waiting on it and
/// starts the next attempt one step higher; the tally counts it.
#[test]
fn a_card_sent_back_restarts_one_step_higher() {
    let s = at_done_check();
    not_done(&s, "the save still 500s");
    wait_for(60, "the agent ended for the higher weight", || ended(&s, 1));
    wait_for(60, "the next attempt", || {
        s.fake.started.lock().unwrap().len() == 2
    });
    assert_eq!(models(&s), sonnet_then_opus());
    let co = s.r.core.coord();
    let t = co.get_task(&s.t.id).unwrap();
    assert_eq!(t.weight, None, "the weight itself is untouched");
    assert_eq!(t.run_level(), "heavy");
    let brief = std::fs::read_to_string(&s.fake.started.lock().unwrap()[1].brief_path).unwrap();
    assert!(brief.contains("This attempt runs heavy"), "{brief}");
    assert!(brief.contains("the save still 500s"), "{brief}");
    let tally = co.run_tally();
    assert_eq!(tally.len(), 1, "{tally:?}");
    assert_eq!(
        (
            tally[0].weight.as_str(),
            tally[0].model.as_str(),
            tally[0].runs,
            tally[0].sent_back,
            tally[0].first_try
        ),
        ("normal", "sonnet", 1, 1, 0)
    );
}

/// Card #966: the person's own weight stays theirs; the next attempt still
/// starts higher than it.
#[test]
fn a_persons_weight_stays_while_the_attempt_steps_up() {
    let s = bound_weighed("light");
    let co = s.r.core.coord();
    co.set_task_state(&s.first(), &s.t.id, "completed", "done")
        .unwrap();
    not_done(&s, "no");
    wait_for(60, "the next attempt", || {
        s.fake.started.lock().unwrap().len() == 2
    });
    let t = co.get_task(&s.t.id).unwrap();
    let w = t.weight.as_ref().unwrap();
    assert_eq!((w.level.as_str(), w.by_person()), ("light", true));
    assert_eq!(t.run_level(), "normal");
    assert_eq!(s.fake.started.lock().unwrap()[1].model, "sonnet");
}

/// Card #966: a crash starts the next agent one step higher.
#[test]
fn a_crash_starts_the_next_agent_one_step_higher() {
    let s = bound();
    std::fs::remove_file(s.r.claude.join("sessions").join("910001.json")).unwrap();
    wait_for(60, "the next agent after the crash", || {
        s.skew.fetch_add(30, Ordering::SeqCst);
        s.r.core.set_spawn_cap(s.r.core.spawn_cap()).unwrap();
        s.fake.started.lock().unwrap().len() == 2
    });
    assert_eq!(models(&s), sonnet_then_opus());
    let co = s.r.core.coord();
    assert_eq!(co.get_task(&s.t.id).unwrap().spawn_crashes, 1);
}

/// Card #966: Done on the first try counts in the tally as that.
#[test]
fn done_on_the_first_try_is_tallied() {
    let s = at_done_check();
    let co = s.r.core.coord();
    let b = check_of(co, &s.t.id);
    // A Not done they take back with Done on the same check doesn't count.
    not_done(&s, "wait");
    co.say_done(&s.person, &s.t.id, &b, &[]).unwrap();
    assert_eq!(co.get_task(&s.t.id).unwrap().run_level(), "normal");
    let tally = co.run_tally();
    assert_eq!(tally.len(), 1, "{tally:?}");
    assert_eq!(
        (tally[0].runs, tally[0].first_try, tally[0].sent_back),
        (1, 1, 0)
    );
}
