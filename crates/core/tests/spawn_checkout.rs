//! Card #480 through the Core: a card whose repo key is in another case
//! than the device's checkout still finds it; a card with no checkout here
//! gets one plain blocker instead of being asked for on every sweep, and
//! its agent starts once the person names a folder or a copy turns up; the
//! spawner failing three times in a row asks too. No line names a raw key
//! or a device id.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use coord::{BlockerAnswer, NewTask, Task};
use irori_core::spawn::{SpawnReq, SpawnSetting, SpawnStarted, Spawner};
use irori_core::spawn_stop::{START_FAILED, is_no_copy};

const SESS: &str = "dddddddd-4444-4444-8444-000000000004";
const MIXED: &str = "remote:github.com/StephenSHorton/irori";
const LOWER: &str = "remote:github.com/stephenshorton/irori";

#[derive(Default)]
struct Fake {
    started: Mutex<Vec<SpawnReq>>,
    tries: Mutex<u32>,
    fail: AtomicBool,
    long: Mutex<bool>,
}

fn spawner(f: &Arc<Fake>, sessions: PathBuf) -> Spawner {
    let a = f.clone();
    Spawner {
        ready: None,
        spawn: Arc::new(move |r: &SpawnReq| {
            *a.tries.lock().unwrap() += 1;
            if a.fail.load(Ordering::SeqCst) {
                if *a.long.lock().unwrap() {
                    return Err(format!(
                        "claude --bg failed: {}",
                        "output line\n".repeat(2000)
                    ));
                }
                return Err("claude: exit status 1".into());
            }
            a.started.lock().unwrap().push(r.clone());
            write_session(
                &sessions,
                "900004.json",
                SESS,
                &r.name,
                Path::new(&r.folder),
            );
            Ok(SpawnStarted {
                name: r.name.clone(),
                prefix: SESS[..8].into(),
            })
        }),
        end: Arc::new(|_| Ok(String::new())),
        trusted: Arc::new(|_| true),
        list: None,
    }
}

struct T {
    r: Rig,
    skew: Arc<AtomicI64>,
    fake: Arc<Fake>,
    person: String,
}

fn rig() -> T {
    let skew = Arc::new(AtomicI64::new(0));
    let sk = skew.clone();
    let r = Rig::with(move |o| {
        o.now = Some(Arc::new(move || {
            let d = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap();
            gocompat::time::GoTime::unix(
                d.as_secs() as i64 + sk.load(Ordering::SeqCst),
                i64::from(d.subsec_nanos()),
            )
        }));
    });
    r.core
        .set_spawn_setting(SpawnSetting {
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    let person = format!("user:{}", r.core.device_id());
    T {
        r,
        skew,
        fake: Arc::new(Fake::default()),
        person,
    }
}

impl T {
    fn start(&self) {
        self.r
            .core
            .set_spawner(Some(spawner(&self.fake, self.r.claude.join("sessions"))));
    }
    fn card(&self, title: &str, repo: &str) -> Task {
        self.r
            .core
            .coord()
            .create_task(
                &self.person,
                NewTask {
                    title: title.into(),
                    repo_key: repo.into(),
                    ..NewTask::default()
                },
            )
            .unwrap()
    }
    fn get(&self, id: &str) -> Task {
        self.r.core.coord().get_task(id).unwrap()
    }
    fn seen(&self, repo: &str, root: &Path) {
        self.r
            .core
            .capture()
            .emit_row(
                "irori",
                capture::KIND_REPO_SEEN,
                &capture::RepoSeen {
                    repo_key: repo.into(),
                    root: root.to_string_lossy().into_owned(),
                    at: gocompat::time::GoTime::unix(1_790_000_000, 0),
                },
            )
            .unwrap();
    }
    fn spawn_rows(&self) -> usize {
        self.r
            .core
            .store()
            .list(&store::Filter {
                kinds: vec![store::kinds::TASK_SPAWN.into()],
                ..Default::default()
            })
            .unwrap()
            .len()
    }
    /// Past the retry wait, a few sweeps.
    fn later(&self) {
        self.skew
            .fetch_add(coord::SPAWN_RETRY.as_secs() as i64 + 10, Ordering::SeqCst);
        for _ in 0..5 {
            self.r.core.set_spawn_cap(self.r.core.spawn_cap()).unwrap();
            std::thread::sleep(Duration::from_millis(100));
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    fn no_copy_blockers(&self, id: &str) -> Vec<coord::Blocker> {
        self.get(id)
            .blockers
            .into_iter()
            .filter(|b| b.open() && is_no_copy(b))
            .collect()
    }
    fn wait_blocked(&self, id: &str) -> coord::Blocker {
        wait_for(60, "the no-copy blocker", || {
            self.get(id).state == coord::BLOCKED && !self.no_copy_blockers(id).is_empty()
        });
        let bs = self.no_copy_blockers(id);
        assert_eq!(bs.len(), 1, "{bs:?}");
        bs[0].clone()
    }
    fn plain(&self, s: &str) {
        let me = self.r.core.device_id();
        assert!(!s.contains("remote:"), "a raw key: {s}");
        assert!(!s.contains("StephenSHorton"), "a raw key: {s}");
        assert!(!s.contains(me), "a device id: {s}");
    }
}

#[test]
fn a_mixed_case_card_starts_in_the_lowercase_checkout() {
    let t = rig();
    let dir = tempfile::tempdir().unwrap();
    t.seen(LOWER, dir.path());
    let c = t.card("Port the parser", MIXED);
    assert_eq!(c.repo_key, LOWER);
    t.start();
    wait_for(60, "its agent started", || {
        !t.fake.started.lock().unwrap().is_empty()
    });
    let req = t.fake.started.lock().unwrap()[0].clone();
    assert_eq!(Path::new(&req.folder), dir.path());
}

#[test]
fn no_checkout_asks_once_in_plain_words_then_starts_in_the_folder_named() {
    let t = rig();
    let c = t.card("Port the parser", MIXED);
    t.start();
    let b = t.wait_blocked(&c.id);
    let dev = t.r.core.device_label(t.r.core.device_id());
    assert!(!dev.is_empty());
    assert_eq!(
        b.question,
        format!("No device has a copy of irori. Choose its folder on {dev}?")
    );
    t.plain(&b.question);
    t.plain(&b.context);
    let sr = t.get(&c.id).spawn_request().cloned().unwrap();
    assert_eq!(sr.reason, format!("no copy of irori on {dev}"));
    t.plain(&sr.reason);
    let rows = t.spawn_rows();
    assert_eq!(rows, 2, "one requested and one failed");

    // It doesn't loop: past the retry wait, nothing more is written and
    // there is still one blocker.
    t.later();
    t.later();
    assert_eq!(t.spawn_rows(), rows, "no new task.spawn rows");
    assert_eq!(t.no_copy_blockers(&c.id).len(), 1);
    assert!(t.fake.started.lock().unwrap().is_empty());

    // A reply that isn't a folder here is said so, and asked again.
    let co = t.r.core.coord();
    co.answer(
        &t.person,
        &c.id,
        &b.id,
        BlockerAnswer {
            text: "I'll clone it later".into(),
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    wait_for(60, "asked again", || {
        let bs = t.no_copy_blockers(&c.id);
        bs.len() == 1 && bs[0].id != b.id && t.get(&c.id).state == coord::BLOCKED
    });
    let b2 = t.no_copy_blockers(&c.id)[0].clone();
    assert!(
        b2.context
            .starts_with("“I'll clone it later” doesn't name a folder"),
        "{}",
        b2.context
    );
    assert!(b2.question.contains("“I'll clone it later”"), "{b2:?}");
    t.plain(&b2.question);
    t.plain(&b2.context);

    // The folder's path: the agent starts there and the blocker closes.
    let dir = tempfile::tempdir().unwrap();
    co.answer(
        &t.person,
        &c.id,
        &b2.id,
        BlockerAnswer {
            text: format!("`{}`", dir.path().display()),
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    wait_for(60, "its agent started", || {
        !t.fake.started.lock().unwrap().is_empty()
    });
    let req = t.fake.started.lock().unwrap()[0].clone();
    assert_eq!(Path::new(&req.folder), dir.path());
    wait_for(60, "the blocker closed", || {
        t.no_copy_blockers(&c.id).is_empty()
    });
}

#[test]
fn a_copy_turning_up_lifts_the_blocker_and_starts_the_agent() {
    let t = rig();
    let c = t.card("Port the parser", LOWER);
    t.start();
    t.wait_blocked(&c.id);
    let dir = tempfile::tempdir().unwrap();
    t.seen(MIXED, dir.path());
    t.later();
    wait_for(60, "its agent started", || {
        !t.fake.started.lock().unwrap().is_empty()
    });
    let req = t.fake.started.lock().unwrap()[0].clone();
    assert_eq!(Path::new(&req.folder), dir.path());
    assert!(t.no_copy_blockers(&c.id).is_empty());
    let resolved = t
        .get(&c.id)
        .blockers
        .into_iter()
        .find(is_no_copy)
        .and_then(|b| b.resolved)
        .unwrap();
    assert!(
        resolved
            .answer
            .starts_with("Irori found a copy of irori on"),
        "{resolved:?}"
    );
    t.plain(&resolved.answer);
}

#[test]
fn a_checkout_seen_on_another_device_is_not_a_folder_here() {
    let t = rig();
    // Checkouts replicate: this root is another device's.
    t.seen(LOWER, Path::new("/nonexistent/other-device/irori"));
    let c = t.card("Port the parser", LOWER);
    t.start();
    let b = t.wait_blocked(&c.id);
    let dev = t.r.core.device_label(t.r.core.device_id());
    assert_eq!(
        b.question,
        format!("{dev} has no copy of irori. Choose its folder on {dev}?")
    );
}

#[test]
fn a_card_with_no_repo_asks_for_a_folder() {
    let t = rig();
    let c = t.card("A card with no repo", "");
    t.start();
    let b = t.wait_blocked(&c.id);
    assert!(
        b.question
            .starts_with("This card names no repo. Choose its folder on"),
        "{b:?}"
    );
    let sr = t.get(&c.id).spawn_request().cloned().unwrap();
    assert!(sr.reason.starts_with("this card names no repo"), "{sr:?}");
    t.plain(&sr.reason);
}

/// Card #531: the close that going back on the Board writes carries the
/// person's name and Irori's words. It is no reply: the question comes
/// back plain, quoting nothing.
#[test]
fn going_back_on_the_board_is_no_reply() {
    let t = rig();
    let c = t.card("A card with no repo", "");
    t.start();
    let b = t.wait_blocked(&c.id);
    let co = t.r.core.coord();
    assert!(co.back_on_board(&t.person, &c.id, "put back").unwrap());
    let closed = t.get(&c.id).blockers.into_iter().find(|x| x.id == b.id);
    assert!(closed.unwrap().resolved.unwrap().closed);
    wait_for(60, "asked again", || {
        t.r.core.set_spawn_cap(t.r.core.spawn_cap()).unwrap();
        let bs = t.no_copy_blockers(&c.id);
        bs.len() == 1 && bs[0].id != b.id
    });
    let b2 = t.no_copy_blockers(&c.id)[0].clone();
    assert_eq!(b2.question, b.question);
    assert_eq!(b2.context, b.context);
}

/// Card #531: a card with no repo is offered where this device's agents
/// work, and picking it starts the agent there.
#[test]
fn a_card_with_no_repo_is_offered_the_folder_agents_work_in() {
    let t = rig();
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().canonicalize().unwrap();
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(&dir)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&["commit", "-q", "--allow-empty", "-m", "first"]);
    let sess = "eeeeeeee-5555-4555-8555-000000000005";
    write_session(
        &t.r.claude.join("sessions"),
        "900005.json",
        sess,
        "gamma",
        &dir,
    );
    let key = format!("{}/{sess}", t.r.core.device_id());
    wait_for(60, "the agent listed with its repo", || {
        t.r.core
            .local_agents()
            .iter()
            .any(|a| a.key == key && !a.repo_key.is_empty())
    });
    let c = t.card("A card with no repo", "");
    t.start();
    let b = t.wait_blocked(&c.id);
    assert_eq!(b.options.len(), 1, "{b:?}");
    assert_eq!(Path::new(&b.options[0].answer).canonicalize().unwrap(), dir);
    assert!(b.context.contains("Pick the folder"), "{}", b.context);

    // As the window answers an Irori ask: picked, and resolved with it.
    t.r.core
        .coord()
        .answer(
            &t.person,
            &c.id,
            &b.id,
            BlockerAnswer {
                pick: 1,
                resolve: true,
                ..BlockerAnswer::default()
            },
        )
        .unwrap();
    wait_for(60, "its agent started", || {
        t.r.core.set_spawn_cap(t.r.core.spawn_cap()).unwrap();
        !t.fake.started.lock().unwrap().is_empty()
    });
    let req = t.fake.started.lock().unwrap()[0].clone();
    assert_eq!(Path::new(&req.folder).canonicalize().unwrap(), dir);
}

#[test]
fn a_spawner_failing_three_times_asks_once() {
    let t = rig();
    let dir = tempfile::tempdir().unwrap();
    t.seen(LOWER, dir.path());
    let c = t.card("Port the parser", LOWER);
    t.fake.fail.store(true, Ordering::SeqCst);
    t.start();
    wait_for(60, "the first try", || *t.fake.tries.lock().unwrap() >= 1);
    for _ in 0..4 {
        if t.get(&c.id).state == coord::BLOCKED {
            break;
        }
        t.later();
    }
    wait_for(60, "the start-failed blocker", || {
        t.get(&c.id)
            .blockers
            .iter()
            .any(|b| b.open() && b.question == START_FAILED)
    });
    let tries = *t.fake.tries.lock().unwrap();
    assert_eq!(tries, 3, "three tries, then it asks");
    t.later();
    t.later();
    assert_eq!(*t.fake.tries.lock().unwrap(), 3, "and no more");
    let b = t
        .get(&c.id)
        .blockers
        .into_iter()
        .find(|b| b.open() && b.question == START_FAILED)
        .unwrap();
    assert!(b.context.contains("claude: exit status 1"), "{}", b.context);
    t.plain(&b.context);

    // Try again, still failing: one more try, and it asks again at once.
    let co = t.r.core.coord();
    co.answer(
        &t.person,
        &c.id,
        &b.id,
        BlockerAnswer {
            pick: 1,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    wait_for(60, "asked again after one more try", || {
        t.get(&c.id)
            .blockers
            .iter()
            .any(|x| x.open() && x.question == START_FAILED && x.id != b.id)
    });
    assert_eq!(*t.fake.tries.lock().unwrap(), 4, "one more try");
}

/// A spawner error far longer than a blocker's context still raises it.
#[test]
fn a_long_spawner_error_still_asks() {
    let t = rig();
    let dir = tempfile::tempdir().unwrap();
    t.seen(LOWER, dir.path());
    let c = t.card("Port the parser", LOWER);
    t.fake.fail.store(true, Ordering::SeqCst);
    *t.fake.long.lock().unwrap() = true;
    t.start();
    wait_for(60, "the first try", || *t.fake.tries.lock().unwrap() >= 1);
    for _ in 0..4 {
        if t.get(&c.id).state == coord::BLOCKED {
            break;
        }
        t.later();
    }
    wait_for(60, "the start-failed blocker", || {
        t.get(&c.id)
            .blockers
            .iter()
            .any(|b| b.open() && b.question == START_FAILED)
    });
}
