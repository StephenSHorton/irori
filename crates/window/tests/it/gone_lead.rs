//! Card #557: a split card whose lead is gone. The person's "Merge it" on
//! the lead's "Can I merge?" stays open and brings a fresh lead that reads
//! it in its brief, and a helper's finished part waits on the next lead,
//! not on the gone one.

use crate::common;

use std::path::Path;
use std::sync::{Arc, Mutex};

use common::*;
use coord::{
    BLOCKED, BLOCKER_ANSWERED, BlockerOption, COMPLETED, NewBlocker, NewSubtask, NewTask, Task,
};
use irori_core::spawn::{SpawnReq, SpawnSetting, SpawnStarted, Spawner};
use irori_window::Window;

const REPO: &str = "remote:github.com/example/split";

struct T {
    r: Rig,
    /// The brief of each agent started, in order.
    briefs: Arc<Mutex<Vec<String>>>,
}

/// A spawning rig with a checkout of REPO here, starting nobody until
/// `start` (its cap is 0).
fn rig() -> T {
    let r = Rig::new();
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
        ready: None,
    }));
    r.w.core
        .set_spawn_setting(SpawnSetting {
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    r.w.core.set_spawn_cap(0).unwrap();
    r.w.core
        .capture()
        .emit_row(
            "irori",
            capture::KIND_REPO_SEEN,
            &capture::RepoSeen {
                repo_key: REPO.into(),
                root: r._dirs[0].path().display().to_string(),
                at: gocompat::time::GoTime::unix(1_790_000_000, 0),
            },
        )
        .unwrap();
    T { r, briefs }
}

impl T {
    fn w(&self) -> &Window {
        &self.r.w
    }
    fn key(&self, name: &str) -> String {
        format!("{}/{name}", self.w().core.device_id())
    }
    fn get(&self, id: &str) -> Task {
        self.w().core.coord().get_task(id).unwrap()
    }
    /// A card the lead holds, split into two parts, each taken by a helper.
    fn led(&self) -> (Task, Vec<Task>) {
        let co = self.w().core.coord();
        let person = format!("user:{}", self.w().core.device_id());
        let p = co
            .create_task(
                &person,
                NewTask {
                    title: "Big card".into(),
                    repo_key: REPO.into(),
                    ..NewTask::default()
                },
            )
            .unwrap();
        co.claim_task(&self.key("lead"), &p.id).unwrap();
        let part = |title: &str| NewSubtask {
            title: title.into(),
            spawn: true,
            ..NewSubtask::default()
        };
        let (p, made) = co
            .split(&self.key("lead"), &p.id, vec![part("a"), part("b")], vec![])
            .unwrap();
        for (i, k) in made.iter().enumerate() {
            co.claim_task(&self.key(&format!("helper-{i}")), &k.id)
                .unwrap();
        }
        (p, made)
    }
    fn start(&self) {
        self.w().core.set_spawn_cap(5).unwrap();
    }
    fn wait_started(&self, n: usize) {
        wait("the agent started", || {
            self.w().core.set_spawn_cap(5).ok();
            self.briefs.lock().unwrap().len() >= n
        });
    }
}

#[test]
fn merge_it_on_a_card_whose_lead_went_reaches_a_fresh_lead() {
    let t = rig();
    let co = t.w().core.coord();
    let (p, made) = t.led();
    let lead = t.key("lead");
    for (i, k) in made.iter().enumerate() {
        co.set_task_state(
            &t.key(&format!("helper-{i}")),
            &k.id,
            COMPLETED,
            "on my branch",
        )
        .unwrap();
        co.set_task_state(&lead, &k.id, COMPLETED, "merged")
            .unwrap();
    }
    let opt = |a: &str| BlockerOption {
        answer: a.into(),
        ..BlockerOption::default()
    };
    let (_, b) = co
        .raise(
            &lead,
            &p.id,
            NewBlocker {
                question: "Can I merge PR #7 (covers #1)?".into(),
                options: vec![opt("Merge it"), opt("Not yet")],
                ..NewBlocker::default()
            },
        )
        .unwrap();
    co.leave(&lead, &p.id, &lead).unwrap();
    assert_eq!(coord::effective_state(&t.get(&p.id)), BLOCKED);

    let res = t.w().answer(&p.id, &b.id, 0, "", true, &[]).unwrap();
    assert!(res.irori_acts, "the window says Irori has it, not nobody");
    let got = t.get(&p.id);
    let kept = got.blockers.iter().find(|x| x.id == b.id).unwrap();
    assert!(
        kept.open() && kept.status == BLOCKER_ANSWERED,
        "left open for the lead: {kept:?}"
    );

    t.start();
    t.wait_started(1);
    let brief = t.briefs.lock().unwrap()[0].clone();
    assert!(brief.contains("you are its new lead"), "{brief}");
    assert!(brief.contains("Merge it"), "{brief}");
}

#[test]
fn a_finished_part_waits_on_the_next_lead_not_the_gone_one() {
    let t = rig();
    let co = t.w().core.coord();
    let (p, made) = t.led();
    co.set_task_state(&t.key("helper-0"), &made[0].id, COMPLETED, "on my branch")
        .unwrap();
    co.leave(&t.key("lead"), &p.id, &t.key("lead")).unwrap();

    let all = t.w().cards();
    let at = |id: &str| all.iter().find(|c| c.id == id).unwrap();
    let part = at(&made[0].id);
    assert!(!part.wait_gone, "a lead is coming: {part:?}");
    assert_eq!(part.wait_on, "its next lead");
    // The part's own page says the same.
    let d = t.w().card(&made[0].id).unwrap();
    let w = d
        .blockers
        .iter()
        .find_map(|b| b.waits_on.as_ref())
        .expect("a wait row");
    assert!(!w.gone && w.label == "its next lead", "{w:?}");
    let parent = at(&p.id);
    assert!(
        parent.subtask_blocked.is_empty(),
        "the part waits on its lead, not the person: {:?}",
        parent.subtask_blocked
    );
}
