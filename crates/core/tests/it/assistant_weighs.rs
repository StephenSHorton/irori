//! Card #965 through the Core, with a fake weigher in place of the desktop
//! app's `claude -p --model haiku`: a new card gets a weight as Irori before
//! its agent starts; a failure, the switch or a slow turn lets it run as it
//! would have; a weight the person set is left alone; an agent's hint may
//! change, saying why; and Undo gives the old weight back.

use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use crate::common::*;
use coord::{NewTask, Task, Where};
use gocompat::time::GoTime;
use irori_core::spawn::{SpawnReq, SpawnSetting, SpawnStarted, Spawner};
use irori_core::thread_names::{NameAsk, Namer};

fn sess(n: usize) -> String {
    format!("eeeeeeee-4444-4444-8444-{n:012}")
}

struct Setup {
    r: Rig,
    started: Arc<Mutex<Vec<SpawnReq>>>,
    asked: Arc<Mutex<Vec<NameAsk>>>,
    skew: Arc<AtomicI64>,
    person: String,
    work: tempfile::TempDir,
}

impl Setup {
    fn new() -> Setup {
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
        let person = format!("user:{}", r.core.device_id());
        r.core
            .set_spawn_setting(SpawnSetting {
                on: true,
                ..SpawnSetting::default()
            })
            .unwrap();
        Setup {
            r,
            started: Arc::default(),
            asked: Arc::default(),
            skew,
            person,
            work: tempfile::tempdir().unwrap(),
        }
    }

    fn install_spawner(&self) {
        let (a, sessions) = (self.started.clone(), self.r.claude.join("sessions"));
        self.r.core.set_spawner(Some(Spawner {
            spawn: Arc::new(move |r: &SpawnReq| {
                let n = {
                    let mut v = a.lock().unwrap();
                    v.push(r.clone());
                    v.len()
                };
                let id = sess(n);
                write_session(
                    &sessions,
                    &format!("9300{n:02}.json"),
                    &id,
                    &r.name,
                    Path::new(&r.folder),
                );
                Ok(SpawnStarted {
                    name: r.name.clone(),
                    prefix: id[..8].into(),
                })
            }),
            end: Arc::new(|_: &str| Ok(String::new())),
            trusted: Arc::new(|_| true),
            list: None,
            ready: None,
        }));
    }

    /// A weigher that answers `reply`.
    fn weigher(&self, reply: &'static str) {
        let asked = self.asked.clone();
        let w: Namer = Arc::new(move |a: &NameAsk| {
            asked.lock().unwrap().push(a.clone());
            Ok(reply.to_string())
        });
        self.r.core.set_weigher(Some(w));
    }

    /// A weigher that says so when asked, and answers once told to.
    fn slow_weigher(&self, reply: &'static str) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (asked_tx, asked_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let (asked_tx, go_rx) = (Mutex::new(asked_tx), Mutex::new(go_rx));
        self.r
            .core
            .set_weigher(Some(Arc::new(move |_: &NameAsk| {
                let _ = asked_tx.lock().unwrap().send(());
                let _ = go_rx.lock().unwrap().recv_timeout(Duration::from_secs(60));
                Ok(reply.to_string())
            })));
        (asked_rx, go_tx)
    }

    fn make(&self, by: &str, nt: NewTask) -> Task {
        let co = self.r.core.coord();
        let t = co.create_task(by, nt).unwrap();
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

    fn card(&self, title: &str) -> Task {
        self.make(
            &self.person,
            NewTask {
                title: title.into(),
                objective: format!("{title}, done"),
                done_criteria: "it works".into(),
                ..NewTask::default()
            },
        )
    }

    fn get(&self, id: &str) -> Task {
        self.r.core.coord().get_task(id).unwrap()
    }

    fn weight(&self, id: &str) -> Option<coord::Weight> {
        self.get(id).weight
    }

    fn lines(&self, id: &str) -> Vec<(String, String)> {
        self.r
            .core
            .coord()
            .activity(id)
            .unwrap()
            .into_iter()
            .map(|a| (a.by, a.what))
            .collect()
    }

    fn kick(&self) {
        let cap = self.r.core.spawn_cap();
        self.r.core.set_spawn_cap(cap).unwrap();
    }

    /// Kicks sweeps for a moment and checks no agent started.
    fn none_start(&self) {
        for _ in 0..5 {
            self.kick();
            std::thread::sleep(Duration::from_millis(150));
        }
        assert_eq!(self.started.lock().unwrap().len(), 0);
    }

    fn started_model(&self) -> String {
        wait_for(30, "the agent started", || {
            self.kick();
            !self.started.lock().unwrap().is_empty()
        });
        self.started.lock().unwrap()[0].model.clone()
    }
}

#[test]
fn a_new_card_gets_a_weight_and_reason_as_irori() {
    let s = Setup::new();
    s.weigher("heavy\nDeciding how to split the import.");
    assert!(s.r.core.weighs_on(), "on unless the person turns it off");
    let t = s.card("Plan the import");
    wait_for(10, "the card weighed", || s.weight(&t.id).is_some());
    let w = s.weight(&t.id).unwrap();
    assert_eq!(w.level, "heavy");
    assert_eq!(w.reason, "Deciding how to split the import.");
    assert_eq!(w.set_by, coord::SET_BY_ASSISTANT);
    assert_eq!(w.was_level, "", "it had none");
    assert!(
        s.lines(&t.id).iter().any(|(by, what)| by == "irori"
            && what == "weighed it heavy: Deciding how to split the import."),
        "{:?}",
        s.lines(&t.id)
    );
    let asked = s.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1, "asked once");
    assert!(asked[0].prompt.contains("Title: Plan the import"));
    assert!(asked[0].prompt.contains("Done when: it works"));
}

#[test]
fn its_agent_waits_for_the_weight_then_starts_on_it() {
    let s = Setup::new();
    let (asked, go) = s.slow_weigher("heavy\nA design call.");
    s.install_spawner();
    let t = s.card("Design the sync");
    asked.recv_timeout(Duration::from_secs(30)).unwrap();
    s.none_start();
    go.send(()).unwrap();
    assert_eq!(s.started_model(), "opus", "heavy's model");
    assert_eq!(s.weight(&t.id).unwrap().level, "heavy");
}

#[test]
fn a_failed_turn_lets_the_card_run_as_normal() {
    let s = Setup::new();
    s.r.core
        .set_weigher(Some(Arc::new(|_: &NameAsk| Err("signed out".to_string()))));
    s.install_spawner();
    let t = s.card("Fix the login");
    s.started_model();
    assert_eq!(s.weight(&t.id), None);
}

#[test]
fn an_answer_that_is_no_weight_lets_the_card_run() {
    let s = Setup::new();
    s.weigher("I can't tell from this card.");
    s.install_spawner();
    let t = s.card("Fix the login");
    s.started_model();
    assert_eq!(s.weight(&t.id), None);
}

#[test]
fn a_slow_turn_never_holds_the_card_past_the_limit() {
    let s = Setup::new();
    let (asked, _go) = s.slow_weigher("heavy\nToo late.");
    s.install_spawner();
    let _t = s.card("Fix the login");
    asked.recv_timeout(Duration::from_secs(30)).unwrap();
    s.none_start();
    s.skew
        .fetch_add(irori_core::card_weights::HOLD_SECS, Ordering::SeqCst);
    s.started_model();
}

#[test]
fn turned_off_it_neither_weighs_nor_holds_and_a_held_card_goes() {
    let s = Setup::new();
    let (asked, _go) = s.slow_weigher("heavy\nA design call.");
    s.install_spawner();
    let held = s.card("Design the sync");
    asked.recv_timeout(Duration::from_secs(30)).unwrap();
    s.none_start();
    s.r.core.set_weighs_on(false).unwrap();
    assert!(!s.r.core.weighs_on());
    s.started_model();
    let _ = held;

    let s = Setup::new();
    s.weigher("heavy\nA design call.");
    s.r.core.set_weighs_on(false).unwrap();
    s.install_spawner();
    let t = s.card("Design the sync");
    s.started_model();
    assert_eq!(s.weight(&t.id), None);
    assert!(s.asked.lock().unwrap().is_empty());
}

#[test]
fn no_weigher_means_no_hold() {
    let s = Setup::new();
    s.install_spawner();
    s.card("Fix the login");
    s.started_model();
}

#[test]
fn a_weight_the_person_set_is_left_alone() {
    let s = Setup::new();
    s.weigher("heavy\nA design call.");
    let t = s.make(
        &s.person,
        NewTask {
            title: "Mine".into(),
            objective: "mine".into(),
            weight: "light".into(),
            weight_reason: "I say so".into(),
            ..NewTask::default()
        },
    );
    std::thread::sleep(Duration::from_millis(800));
    assert!(s.asked.lock().unwrap().is_empty(), "never asked");
    assert_eq!(s.weight(&t.id).unwrap().level, "light");
}

#[test]
fn a_weight_the_person_sets_during_the_turn_wins() {
    let s = Setup::new();
    let (asked, go) = s.slow_weigher("heavy\nA design call.");
    let t = s.card("Design the sync");
    asked.recv_timeout(Duration::from_secs(30)).unwrap();
    let co = s.r.core.coord();
    co.set_weight(&s.person, &t.id, "light", "").unwrap();
    go.send(()).unwrap();
    std::thread::sleep(Duration::from_millis(800));
    let w = s.weight(&t.id).unwrap();
    assert_eq!((w.level.as_str(), w.set_by.as_str()), ("light", coord::SET_BY_PERSON));
    assert!(s.lines(&t.id).iter().all(|(_, w)| !w.starts_with("weighed it")));
}

#[test]
fn it_may_change_an_agents_hint_and_says_why() {
    let s = Setup::new();
    s.weigher("heavy\nThe cause is unknown, so this needs real digging.");
    let t = s.make(
        &s.r.key_a,
        NewTask {
            title: "Why does sync stall".into(),
            objective: "Find out".into(),
            weight: "light".into(),
            weight_reason: "looks small".into(),
            ..NewTask::default()
        },
    );
    wait_for(10, "the card weighed", || {
        s.weight(&t.id).is_some_and(|w| w.by_assistant())
    });
    let w = s.weight(&t.id).unwrap();
    assert_eq!((w.level.as_str(), w.was_level.as_str()), ("heavy", "light"));
    assert_eq!(w.was_reason, "looks small");
    assert!(s.asked.lock().unwrap()[0].prompt.contains("<hint>looks small</hint>"));
    assert!(
        s.lines(&t.id).iter().any(|(_, what)| what
            == "changed the weight from light to heavy: The cause is unknown, so this needs real digging."),
        "{:?}",
        s.lines(&t.id)
    );
}

#[test]
fn undo_clears_it_and_irori_does_not_weigh_it_again() {
    let s = Setup::new();
    s.weigher("heavy\nA design call.");
    let t = s.card("Design the sync");
    wait_for(10, "the card weighed", || s.weight(&t.id).is_some());
    // What the window's Undo writes.
    s.r.core
        .coord()
        .set_weight(&s.person, &t.id, "", "")
        .unwrap();
    wait_for(10, "the weight gone", || s.weight(&t.id).is_none());
    // Another change to the card doesn't bring Irori back.
    s.r.core
        .coord()
        .rename_task(&s.person, &t.id, "Design the sync, again")
        .unwrap();
    s.kick();
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(s.weight(&t.id), None);
    assert_eq!(s.asked.lock().unwrap().len(), 1);
}

#[test]
fn a_thread_is_not_weighed() {
    let s = Setup::new();
    s.weigher("heavy\nA design call.");
    let (t, _) = s
        .r
        .core
        .coord()
        .post_ask(
            &s.person,
            coord::NewAsk {
                text: "the login page is broken".into(),
                thread: true,
                ..coord::NewAsk::default()
            },
            &s.r.core.ask_agents(),
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(s.weight(&t.id), None);
    assert!(s.asked.lock().unwrap().is_empty());
}
