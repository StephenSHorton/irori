//! Card #1075 (merge-desk §12.12 B8, the side door) through the Core, with
//! a fake spawner in place of `claude --bg`: a card an agent makes after it
//! read another person's text waits on "OK to start an agent on this?",
//! never runs Bypass, and its agent starts flagged; a session that never
//! read another person's text makes cards as before.

use crate::common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use coord::{BlockerAnswer, GIVEN_QUESTION, NewTask, Where};
use irori_core::spawn::{SpawnReq, SpawnSetting, SpawnStarted, Spawner};

const SPAWNED_SESS: &str = "cccccccc-3333-4333-8333-000000000003";

#[derive(Default)]
struct Fake {
    started: Mutex<Vec<SpawnReq>>,
}

fn spawner(f: &Arc<Fake>, sessions: PathBuf) -> Spawner {
    let a = f.clone();
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
        end: Arc::new(|_| Ok(String::new())),
        trusted: Arc::new(|_| true),
        list: None,
        stop: None,
    }
}

/// A rig with spawning on, its spawner faked, and a card made by `maker`
/// (an agent key of the rig) with a folder to start in.
struct Setup {
    r: Rig,
    fake: Arc<Fake>,
    card: coord::Task,
    _work: tempfile::TempDir,
}

fn setup(flag_maker: bool) -> Setup {
    let r = Rig::new();
    let work = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    if flag_maker {
        // What a delivered board row, a flagged send or a spawn with
        // others' text does (the inbox tests cover each of those).
        r.core.inbox().mark_saw_others(&r.key_a);
    }
    let card = r
        .core
        .coord()
        .create_task(
            &r.key_a,
            NewTask {
                title: "Look at the loader".into(),
                objective: "blake asked for the loader to be looked at".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    r.core
        .coord()
        .set_where(
            &card.id,
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
    Setup {
        r,
        fake,
        card,
        _work: work,
    }
}

#[test]
fn a_card_made_after_reading_others_text_waits_for_ok_to_start() {
    let s = setup(true);
    let (r, co) = (&s.r, s.r.core.coord());
    assert!(
        co.get_task(&s.card.id).unwrap().others,
        "the card is marked"
    );
    wait_for(60, "the OK to start question", || {
        co.get_task(&s.card.id).is_some_and(|t| {
            t.blockers
                .iter()
                .any(|b| b.open() && b.question == GIVEN_QUESTION)
        })
    });
    std::thread::sleep(Duration::from_millis(600));
    assert!(
        s.fake.started.lock().unwrap().is_empty(),
        "nothing starts before the yes"
    );

    // Their yes starts one agent, never on Bypass, whose brief says so and
    // which starts flagged itself.
    let t = co.get_task(&s.card.id).unwrap();
    let q = t
        .blockers
        .iter()
        .find(|b| b.question == GIVEN_QUESTION)
        .unwrap();
    co.answer(
        &format!("user:{}", r.core.device_id()),
        &t.id,
        &q.id,
        BlockerAnswer {
            pick: 1,
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    wait_for(60, "the agent after their yes", || {
        !s.fake.started.lock().unwrap().is_empty()
    });
    let reqs = s.fake.started.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1, "one yes, one agent: {reqs:?}");
    assert_ne!(reqs[0].permission_mode, "bypassPermissions");
    let brief = std::fs::read_to_string(&reqs[0].brief_path).unwrap();
    assert!(
        brief.contains("made or changed this card") && brief.contains("another person's text"),
        "{brief}"
    );
    let spawned = format!("{}/{SPAWNED_SESS}", r.core.device_id());
    wait_for(60, "the spawned agent flagged", || {
        r.core.inbox().saw_others(&spawned)
    });
}

#[test]
fn a_session_that_never_read_others_text_makes_cards_as_today() {
    let s = setup(false);
    let co = s.r.core.coord();
    assert!(!co.get_task(&s.card.id).unwrap().others);
    wait_for(60, "the agent with no question", || {
        !s.fake.started.lock().unwrap().is_empty()
    });
    let reqs = s.fake.started.lock().unwrap().clone();
    assert_eq!(reqs[0].permission_mode, "bypassPermissions");
    assert!(
        co.get_task(&s.card.id)
            .unwrap()
            .blockers
            .iter()
            .all(|b| b.question != GIVEN_QUESTION)
    );
}
