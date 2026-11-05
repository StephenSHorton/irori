//! Card #540: answering Irori's crash question in the window. The window
//! resolves an Irori ask with the answer, so nothing puts the card back on
//! the Board and the reply is on no comment: "Start another agent" still
//! starts one, the person's own words reach its brief, and the window
//! doesn't say nobody will act on it.

mod common;

use base64::Engine;
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use common::*;
use coord::{Blocker, HOLDER_GRACE, MAX_SPAWN_CRASHES, NewTask, SUBMITTED, Task, is_crash_question};
use irori_core::spawn::{SpawnReq, SpawnSetting, SpawnStarted, Spawner};
use irori_window::Window;

struct T {
    r: Rig,
    /// The brief of each agent started, in order.
    briefs: Arc<Mutex<Vec<String>>>,
    /// How far this rig's clock runs ahead of the real one, in seconds.
    ahead: Arc<AtomicI64>,
}

fn rig() -> T {
    let ahead = Arc::new(AtomicI64::new(0));
    let by = ahead.clone();
    let r = Rig::with(move |o| {
        o.now = Some(Arc::new(move || {
            let ms = unix_ms() + by.load(Ordering::SeqCst) * 1000;
            gocompat::time::GoTime::unix_milli(ms)
        }));
    });
    let briefs: Arc<Mutex<Vec<String>>> = Arc::default();
    let b = briefs.clone();
    r.w.core.set_spawner(Some(Spawner {
        spawn: Arc::new(move |req: &SpawnReq| {
            b.lock()
                .unwrap()
                .push(std::fs::read_to_string(&req.brief_path).unwrap());
            Ok(SpawnStarted {
                name: req.name.clone(),
                prefix: "dddddddd".into(),
            })
        }),
        end: Arc::new(|_| Ok(String::new())),
        trusted: Arc::new(|_: &Path| true),
        list: None,
    }));
    r.w.core
        .set_spawn_setting(SpawnSetting {
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    T { r, briefs, ahead }
}

impl T {
    fn w(&self) -> &Window {
        &self.r.w
    }
    fn sweep(&self) {
        self.w().core.set_spawn_cap(self.w().core.spawn_cap()).ok();
    }
    fn get(&self, id: &str) -> Task {
        self.w().core.coord().get_task(id).unwrap()
    }
    fn started(&self) -> usize {
        self.briefs.lock().unwrap().len()
    }
    /// Waits for agent number `n` to start.
    fn wait_started(&self, n: usize) {
        wait("the agent started", || {
            self.sweep();
            self.started() >= n
        });
    }
    /// A card with a checkout here, whose agent is started by the real
    /// sweep and goes away, MAX_SPAWN_CRASHES times.
    fn crashed_card(&self) -> (Task, Blocker) {
        let co = self.w().core.coord();
        let person = format!("user:{}", self.w().core.device_id());
        let repo = "remote:github.com/example/crashy";
        let dir = self.r._dirs[0].path().display().to_string();
        let c = co
            .create_task(
                &person,
                NewTask {
                    title: "Kills its agent".into(),
                    repo_key: repo.into(),
                    ..NewTask::default()
                },
            )
            .unwrap();
        // The reply to its "no copy" question names a folder here.
        wait("the no-copy blocker", || {
            self.sweep();
            self.get(&c.id).blockers.iter().any(|b| b.open())
        });
        let asked = self.get(&c.id).blockers[0].id.clone();
        self.w()
            .answer(&c.id, &asked, -1, &dir, true, &[])
            .unwrap();
        for n in 1..=MAX_SPAWN_CRASHES as usize {
            self.wait_started(n);
            self.crash(&c.id, n);
        }
        let got = self.get(&c.id);
        let q = got
            .blockers
            .iter()
            .find(|b| b.open() && is_crash_question(b))
            .expect("the person is asked")
            .clone();
        (got, q)
    }
    /// The agent started for card `id` binds, takes it and drops off the
    /// roster for good.
    fn crash(&self, id: &str, n: usize) {
        let co = self.w().core.coord();
        let key = format!("{}/gone-{n}", self.w().core.device_id());
        let g = self.get(id).spawn_request().unwrap().generation;
        assert!(co.bind_spawn(id, g, &key).unwrap());
        co.claim_task(&key, id).unwrap();
        co.sweep_notices().unwrap();
        self.ahead
            .fetch_add(HOLDER_GRACE.as_secs() as i64 + 1, Ordering::SeqCst);
        co.sweep_notices().unwrap();
        assert!(self.get(id).holders.is_empty(), "{:?}", self.get(id));
    }
}

#[test]
fn start_another_agent_starts_one() {
    let t = rig();
    let (c, q) = t.crashed_card();
    t.sweep();
    assert_eq!(t.started(), 2, "no third agent before the answer");

    let res = t.w().answer(&c.id, &q.id, 0, "", true, &[]).unwrap();
    assert!(res.irori_acts, "the window says Irori has it, not nobody");

    let got = t.get(&c.id);
    assert_eq!(got.state, SUBMITTED, "{got:?}");
    assert_eq!(got.spawn_crashes, 0);
    t.wait_started(3);
    let brief = t.briefs.lock().unwrap()[2].clone();
    assert!(!brief.contains(coord::CRASHED_START), "{brief}");
}

#[test]
fn an_own_words_reply_reaches_the_next_brief() {
    let t = rig();
    let (c, q) = t.crashed_card();
    let said = "try it with the smaller fixture";
    let res = t.w().answer(&c.id, &q.id, -1, said, true, &[]).unwrap();
    assert!(res.irori_acts);
    t.wait_started(3);
    let brief = t.briefs.lock().unwrap()[2].clone();
    assert!(brief.contains(said), "{brief}");
}

/// An answer with a file goes on the thread and puts the card back on the
/// Board before any resolve: the same outcome by the other road.
#[test]
fn a_reply_with_a_file_starts_one_too() {
    let t = rig();
    let (c, q) = t.crashed_card();
    let file = irori_window::board::UploadFile {
        name: "log.txt".into(),
        data: base64::engine::general_purpose::STANDARD.encode(b"here"),
    };
    let said = "this log shows the crash";
    let res = t
        .w()
        .answer_files(&c.id, &q.id, -1, said, true, &[], &[file])
        .unwrap();
    assert!(res.irori_acts);
    t.wait_started(3);
    let brief = t.briefs.lock().unwrap()[2].clone();
    assert!(brief.contains(said), "{brief}");
}

/// A card someone took while it waited is theirs: Irori won't start one.
#[test]
fn a_card_with_a_holder_does_not_promise_an_agent() {
    let t = rig();
    let (c, q) = t.crashed_card();
    t.w().core.coord().claim_task(&t.r.key_a, &c.id).unwrap();
    let res = t.w().answer(&c.id, &q.id, 0, "", true, &[]).unwrap();
    assert!(!res.irori_acts);
}
