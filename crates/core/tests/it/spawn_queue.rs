//! Card #450 (spawned agents part 6, docs/design/spawned-agents.md §6)
//! through the Core, with a fake spawner: over the cap cards queue in
//! order (what the person asked for first, then the rest by age) and each
//! knows its place; a Claude usage limit, whether the spawn fails on it or
//! the agent stops on it, queues the card without counting as a crash and
//! holds new spawns until the reset.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::common::*;
use coord::{NewTask, Task, Where};
use gocompat::time::GoTime;
use irori_core::spawn::{SpawnCap, SpawnReq, SpawnSetting, SpawnStarted, Spawner};
use irori_core::spawn_limits::USAGE_STOPPED;
use irori_core::spawn_stop::START_FAILED;
use store::kinds;

/// The session id of the fake's nth spawn (1-based).
fn sess(n: usize) -> String {
    format!("dddddddd-4444-4444-8444-{n:012}")
}

#[derive(Default)]
struct Fake {
    started: Mutex<Vec<SpawnReq>>,
    /// While set, a spawn fails with this (what `claude --bg` printed).
    fail: Mutex<Option<String>>,
}

fn spawner(f: &Arc<Fake>, sessions: PathBuf) -> Spawner {
    let (a, s2) = (f.clone(), sessions.clone());
    Spawner {
        spawn: Arc::new(move |r: &SpawnReq| {
            if let Some(e) = a.fail.lock().unwrap().clone() {
                return Err(e);
            }
            let n = {
                let mut v = a.started.lock().unwrap();
                v.push(r.clone());
                v.len()
            };
            let id = sess(n);
            write_session(
                &sessions,
                &format!("9200{n:02}.json"),
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
            if let Some(n) = s.rsplit('-').next().and_then(|x| x.parse::<usize>().ok()) {
                let _ = std::fs::remove_file(s2.join(format!("9200{n:02}.json")));
            }
            Ok(String::new())
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
    work: tempfile::TempDir,
}

impl Setup {
    fn new() -> Setup {
        let skew = Arc::new(AtomicI64::new(0));
        let r = rig(&skew);
        let person = format!("user:{}", r.core.device_id());
        // The switch on, no spawner yet: cards wait until it's installed.
        r.core
            .set_spawn_setting(SpawnSetting {
                on: true,
                ..SpawnSetting::default()
            })
            .unwrap();
        Setup {
            r,
            fake: Arc::new(Fake::default()),
            skew,
            person,
            work: tempfile::tempdir().unwrap(),
        }
    }

    /// A card made by `by`, a second younger than the one before.
    fn card(&self, by: &str, title: &str) -> Task {
        self.skew.fetch_add(1, Ordering::SeqCst);
        let co = self.r.core.coord();
        let t = co
            .create_task(
                by,
                NewTask {
                    title: title.into(),
                    objective: format!("{title}, done"),
                    ..NewTask::default()
                },
            )
            .unwrap();
        let w: String = self.work.path().to_string_lossy().into();
        co.set_where(
            &t.id,
            Where {
                who: self.r.key_a.clone(),
                cwd: w.clone(),
                worktree: w,
                ..Where::default()
            },
        )
        .unwrap();
        t
    }

    fn install(&self) {
        self.r
            .core
            .set_spawner(Some(spawner(&self.fake, self.r.claude.join("sessions"))));
    }

    /// Runs a sweep (the cap's write kicks one).
    fn kick(&self) {
        let cap = self.r.core.spawn_cap();
        self.r.core.set_spawn_cap(cap).unwrap();
    }

    fn started(&self) -> Vec<String> {
        self.fake
            .started
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.task_id.clone())
            .collect()
    }

    fn agent(&self, n: usize) -> String {
        format!("{}/{}", self.r.core.device_id(), sess(n))
    }

    /// Kicks sweeps for a moment and checks nothing new started.
    fn settle(&self, started: usize) {
        for _ in 0..5 {
            self.kick();
            std::thread::sleep(Duration::from_millis(150));
        }
        assert_eq!(self.started().len(), started, "{:?}", self.started());
    }
}

/// Ten cards at once on a device capped at six (the PC's default): six
/// run, four queue, each with its place. The person's own cards go first,
/// oldest first, then the cards agents made, oldest first, even though
/// the agents' cards are older.
#[test]
fn ten_cards_run_six_and_queue_four_in_order() {
    let s = Setup::new();
    s.r.core.set_spawn_cap(6).unwrap();
    let agents: Vec<Task> = (1..=3)
        .map(|i| s.card(&s.r.key_a, &format!("agent card {i}")))
        .collect();
    let mine: Vec<Task> = (1..=7)
        .map(|i| s.card(&s.person, &format!("my card {i}")))
        .collect();
    let co = s.r.core.coord();
    let order: Vec<String> = co.cards_to_spawn().into_iter().map(|t| t.id).collect();
    let want: Vec<String> = mine.iter().chain(&agents).map(|t| t.id.clone()).collect();
    assert_eq!(order, want, "the queue order");

    s.install();
    wait_for(60, "six agents bound", || {
        (1..=6).all(|n| co.spawned(&s.agent(n)).is_some())
            && mine[..6]
                .iter()
                .all(|t| co.get_task(&t.id).is_some_and(|t| !t.holders.is_empty()))
    });
    s.settle(6);
    let first_six: Vec<String> = mine[..6].iter().map(|t| t.id.clone()).collect();
    assert_eq!(s.started(), first_six, "the person's six oldest start");

    let q = s.r.core.spawn_queue();
    assert_eq!((q.running, q.cap), (6, 6));
    assert_eq!(q.held_until, None);
    assert_eq!(q.places.len(), 4, "{:?}", q.places);
    assert_eq!(q.places[&mine[6].id], 1);
    for (i, t) in agents.iter().enumerate() {
        assert_eq!(q.places[&t.id], i + 2, "{}", t.title);
    }
    // Still queued, not failed: no card says it can't start.
    for t in agents.iter().chain(&mine[6..]) {
        let t = co.get_task(&t.id).unwrap();
        assert!(t.spawn_request().is_none(), "{:?}", t.spawn_request());
    }

    // A slot frees (the person closes a card; its agent is asked for its
    // hand-off and ended): the 1st in the queue starts and everyone moves
    // up.
    co.set_task_state(&s.person, &mine[0].id, "canceled", "not needed")
        .unwrap();
    s.kick();
    std::thread::sleep(Duration::from_millis(500));
    s.skew.fetch_add(
        irori_core::spawn::HANDOFF_WAIT.as_secs() as i64 + 1,
        Ordering::SeqCst,
    );
    wait_for(60, "the 1st in the queue starts", || {
        s.kick();
        s.started().contains(&mine[6].id)
    });
    let q = s.r.core.spawn_queue();
    for (i, t) in agents.iter().enumerate() {
        assert_eq!(q.places.get(&t.id), Some(&(i + 1)), "{}", t.title);
    }
}

/// A raised cap lets queued cards start at once.
#[test]
fn a_higher_cap_starts_the_queue() {
    let s = Setup::new();
    s.r.core.set_spawn_cap(1).unwrap();
    let a = s.card(&s.person, "first");
    let b = s.card(&s.person, "second");
    s.install();
    s.settle(1);
    assert_eq!(s.started(), vec![a.id.clone()]);
    assert_eq!(s.r.core.spawn_queue().places.get(&b.id), Some(&1));
    s.r.core.set_spawn_cap(2).unwrap();
    wait_for(60, "the second starts", || s.started().len() == 2);
    assert_eq!(s.started()[1], b.id);
}

/// `claude --bg` failing on a usage limit isn't a failure: the card waits
/// in the queue as "usage limit" (no "can't start", nothing toward the
/// spawner's three tries), nothing else starts until the reset, and then
/// it does.
#[test]
fn a_spawn_that_hits_the_usage_limit_queues_and_waits_for_the_reset() {
    let s = Setup::new();
    let co = s.r.core.coord();
    *s.fake.fail.lock().unwrap() = Some(
        "claude --bg failed: You've hit your 5-hour limit · resets 4:30pm (America/Denver)".into(),
    );
    let a = s.card(&s.person, "hits the limit");
    let b = s.card(&s.person, "waits behind it");
    s.install();
    wait_for(60, "the request ends on the usage limit", || {
        s.kick();
        co.get_task(&a.id)
            .and_then(|t| t.spawn_request().cloned())
            .is_some_and(|r| coord::usage_limited(&r))
    });
    let q = s.r.core.spawn_queue();
    let until = q.held_until.expect("spawns are held");
    let now = GoTime::from_system(std::time::SystemTime::now());
    assert!(
        until.secs > now.secs && until.secs <= now.secs + 24 * 3600,
        "{until:?}"
    );
    // The limit lifts in Claude Code, but Irori waits for the reset.
    *s.fake.fail.lock().unwrap() = None;
    s.settle(0);
    // b was never tried: the hold stopped the pass.
    assert!(co.get_task(&b.id).unwrap().spawn_request().is_none());
    for t in [&a, &b] {
        let t = co.get_task(&t.id).unwrap();
        assert!(
            t.blockers
                .iter()
                .all(|x| x.question != irori_core::spawn_stop::START_FAILED),
            "{:?}",
            t.blockers
        );
    }
    // Past the reset both start, a (asked first) first.
    s.skew.fetch_add(25 * 3600, Ordering::SeqCst);
    wait_for(60, "both start after the reset", || {
        s.kick();
        s.started().len() == 2
    });
    assert_eq!(s.started(), vec![a.id.clone(), b.id.clone()]);
}

/// A spawned agent that stops on a usage limit (its transcript's last row
/// is the limit) is ended as not a crash: a line on the card, no crash
/// count, no crash question, the card queued; and after the reset a fresh
/// agent starts for it.
#[test]
fn an_agent_that_stops_on_the_usage_limit_is_no_crash() {
    let s = Setup::new();
    let co = s.r.core.coord();
    let t = s.card(&s.person, "works until the limit");
    s.install();
    let first = s.agent(1);
    wait_for(60, "the first agent bound", || {
        co.get_task(&t.id)
            .is_some_and(|t| t.holders.contains(&first))
    });
    co.set_task_state(&first, &t.id, "working", "on it")
        .unwrap();

    // The limit row Claude Code writes, with its reset an hour on.
    let resets = GoTime::from_system(std::time::SystemTime::now()).secs + 3600;
    let proj = s.r.claude.join("projects").join("C--work");
    std::fs::create_dir_all(&proj).unwrap();
    let said = r#"{"type":"assistant","timestamp":"2026-11-04T19:00:00Z","isSidechain":false,"message":{"id":"m0","role":"assistant","model":"claude-sonnet-5-5","content":[{"type":"text","text":"Starting on it."}]}}"#;
    let limit = format!(
        r#"{{"type":"assistant","timestamp":"2026-11-04T19:01:00Z","isSidechain":false,"message":{{"id":"m1","role":"assistant","model":"<synthetic>","content":[{{"type":"text","text":"You've hit your 5-hour limit · resets 4:30pm (America/Denver)"}}]}},"apiError":"usage_limit_reached","apiErrorParams":{{"rate_limit_info":{{"status":"rejected","resetsAt":{resets},"rateLimitType":"five_hour"}}}},"isApiErrorMessage":true}}"#
    );
    std::fs::write(
        proj.join(format!("{}.jsonl", sess(1))),
        format!("{said}\n{limit}\n"),
    )
    .unwrap();

    wait_for(60, "Irori ends it on the usage limit", || {
        s.kick();
        co.ended_by_irori(&first)
    });
    let q = s.r.core.spawn_queue();
    assert_eq!(q.held_until.map(|h| h.secs), Some(resets));
    let gone =
        s.r.core
            .store()
            .list(&store::Filter {
                kinds: vec![store::kinds::AGENT_GONE.into()],
                ..store::Filter::default()
            })
            .unwrap();
    assert!(
        gone.iter().any(|e| {
            let v: serde_json::Value = serde_json::from_slice(&e.payload).unwrap();
            v["key"] == first.as_str() && v["reason"] == coord::GONE_USAGE_LIMIT
        }),
        "an agent.gone as a usage limit"
    );
    wait_for(60, "it comes off the card", || {
        co.get_task(&t.id).is_some_and(|t| t.holders.is_empty())
    });
    let now = co.get_task(&t.id).unwrap();
    assert_eq!(now.spawn_crashes, 0, "a usage limit is no crash");
    assert!(
        now.blockers
            .iter()
            .all(|b| b.question != coord::CRASHED_QUESTION),
        "{:?}",
        now.blockers
    );
    let comments = co.comments(&t.id).unwrap();
    assert!(
        comments.iter().any(|c| c.text.starts_with(USAGE_STOPPED)),
        "{:?}",
        comments.iter().map(|c| &c.text).collect::<Vec<_>>()
    );
    // Held: no fresh agent yet, and the card waits in the queue.
    s.settle(1);
    assert_eq!(
        co.cards_to_spawn().first().map(|c| c.id.clone()),
        Some(t.id.clone())
    );

    // After the reset a fresh agent starts from Where and Left off.
    s.skew.fetch_add(3601, Ordering::SeqCst);
    wait_for(60, "a fresh agent after the reset", || {
        s.kick();
        s.started().len() == 2
    });
    assert_eq!(s.started()[1], t.id);
}

/// A cap set for a device is checked before it is stored.
#[test]
fn a_devices_cap_is_checked() {
    let s = Setup::new();
    assert!(s.r.core.set_spawn_cap_of("", 3).is_err());
    assert!(s.r.core.set_spawn_cap_of("other-device", 999).is_err());
    s.r.core.set_spawn_cap_of("other-device", 12).unwrap();
    assert_eq!(s.r.core.spawn_cap_of("other-device"), 12);
    assert!(s.r.core.spawn_cap_set("other-device"));
}

/// A changed cap holds from the next pass, with no restart (card #576).
#[test]
fn a_changed_limit_holds_without_a_restart() {
    let s = Setup::new();
    s.r.core.set_spawn_cap(0).unwrap();

    // A cap of 0 starts nothing, and raising it starts one, untouched.
    let plain = s.card(&s.person, "a card with a folder");
    s.install();
    s.settle(0);
    s.r.core.set_spawn_cap(1).unwrap();
    wait_for(60, "one starts", || s.started().len() == 1);
    assert_eq!(s.started(), vec![plain.id.clone()]);
    s.settle(1);
}

/// The person's number of failed starts before Irori asks (card #576).
#[test]
fn failed_starts_ask_at_the_persons_number() {
    let s = Setup::new();
    s.r.core
        .set_spawn_setting(SpawnSetting {
            on: true,
            start_tries: 1,
            ..SpawnSetting::default()
        })
        .unwrap();
    *s.fake.fail.lock().unwrap() = Some("claude: not logged in".into());
    let t = s.card(&s.person, "can't start");
    s.install();
    wait_for(60, "the first failure asks", || {
        s.r.core
            .coord()
            .get_task(&t.id)
            .unwrap()
            .blockers
            .iter()
            .any(|b| b.open() && b.question == START_FAILED)
    });
}

/// Limits past what Settings offers are refused, and a stored row with one
/// falls back to the defaults (card #576).
#[test]
fn the_new_limits_are_checked() {
    let s = Setup::new();
    let set = |f: fn(&mut SpawnSetting)| {
        let mut v = SpawnSetting {
            on: true,
            ..SpawnSetting::default()
        };
        f(&mut v);
        s.r.core.set_spawn_setting(v)
    };
    assert!(set(|v| v.crash_limit = 21).is_err());
    assert!(set(|v| v.start_tries = -1).is_err());
    set(|v| v.crash_limit = 20).unwrap();
}

/// Decision 23: a limit is the person's. A `spawn.cap` or `spawn.set` row
/// written as anything but the app's "user" is ignored.
#[test]
fn an_agent_cant_change_a_limit() {
    let s = Setup::new();
    s.r.core.set_spawn_cap(2).unwrap();
    let me = s.r.core.device_id().to_string();
    let agent = s.r.key_a.clone();
    let st = s.r.core.store();
    st.append(
        &agent,
        kinds::SPAWN_CAP,
        &SpawnCap {
            device: me,
            cap: 50,
            on: None,
            by: agent.clone(),
            at: GoTime::unix(1_790_000_000, 0),
        },
    )
    .unwrap();
    st.append(
        &agent,
        kinds::SPAWN_SET,
        &serde_json::json!({ "on": true, "crashLimit": 9, "startTries": 9 }),
    )
    .unwrap();
    s.kick();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(s.r.core.spawn_cap(), 2);
    let set = s.r.core.spawn_setting("");
    assert_eq!((set.crash_limit, set.start_tries), (0, 0));
}

/// A usage hold isn't only the reset's to end (card #576): the person can
/// end it, and a limit still standing holds again at the first start.
#[test]
fn the_person_can_end_a_usage_hold() {
    let s = Setup::new();
    let co = s.r.core.coord();
    *s.fake.fail.lock().unwrap() = Some("You've hit your 5-hour limit · resets 4:30pm".into());
    let a = s.card(&s.person, "hits the limit");
    s.install();
    wait_for(60, "held", || {
        s.kick();
        s.r.core.spawn_queue().held_until.is_some()
    });
    // Still limited: it tries once and holds again.
    s.r.core.release_usage_hold();
    wait_for(60, "it holds again", || {
        s.kick();
        s.r.core.spawn_queue().held_until.is_some()
    });
    assert!(
        co.get_task(&a.id)
            .unwrap()
            .spawn_request()
            .unwrap()
            .generation
            >= 2
    );
    // He bought more: the next try starts.
    *s.fake.fail.lock().unwrap() = None;
    s.r.core.release_usage_hold();
    wait_for(60, "it starts", || {
        s.kick();
        s.started() == vec![a.id.clone()]
    });
    assert_eq!(s.r.core.spawn_queue().held_until, None);
}
