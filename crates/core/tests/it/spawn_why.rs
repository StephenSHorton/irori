//! Card #574 (GitHub #66): an agent that asks for an agent is told why none
//! has started, in the tool result and in `board` and `show`, and `wait`
//! works on shared-board cards. Real Cores: one for the private board's
//! reasons, then two people on loopback iroh for a shared board's.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::common::mesh::{invited, person, restart};
use crate::common::{Rig, wait_for};
use coord::{NewTask, SPAWN_FAILED, SpawnAsk, Task};
use irori_core::spawn::{SpawnReq, SpawnSetting, SpawnStarted, Spawner};
use irori_core::spawn_why::Why;
use mesh::boardmesh::{KIND_BOARD_HELLO, SavedHello};

const REPO: &str = "remote:github.com/stephen/api";

/// A spawner that never gets to start anything here.
fn idle_spawner() -> Spawner {
    Spawner {
        spawn: Arc::new(|_: &SpawnReq| {
            Ok(SpawnStarted {
                name: "never".into(),
                prefix: "never".into(),
            })
        }),
        end: Arc::new(|_: &str| Ok(String::new())),
        trusted: Arc::new(|_: &Path| true),
        list: None,
        ready: None,
    }
}

fn card(r: &Rig, title: &str) -> Task {
    let person = format!("user:{}", r.core.device_id());
    r.core
        .coord()
        .create_task(
            &person,
            NewTask {
                title: title.into(),
                repo_key: REPO.into(),
                ..NewTask::default()
            },
        )
        .unwrap()
}

fn switch(r: &Rig, on: bool) {
    r.core
        .set_spawn_setting(SpawnSetting {
            on,
            ..SpawnSetting::default()
        })
        .unwrap();
}

/// The card's next request, ended with `reason`.
fn failed(r: &Rig, t: &Task, reason: &str) {
    let co = r.core.coord();
    let g = co
        .request_spawn(
            "irori",
            SpawnAsk {
                task_id: t.id.clone(),
                device: r.core.device_id().to_string(),
                repo_key: REPO.into(),
                permission_mode: "acceptEdits".into(),
                name: "heron".into(),
                ..SpawnAsk::default()
            },
        )
        .unwrap()
        .unwrap();
    co.end_spawn("irori", &t.id, g, SPAWN_FAILED, reason)
        .unwrap();
}

#[test]
fn the_switch_is_said_only_to_who_asked() {
    let r = Rig::new();
    let t = card(&r, "A card");
    assert_eq!(r.core.why_no_agent(&t.id, false), None, "ordinary, unasked");
    assert_eq!(r.core.why_no_agent(&t.id, true), Some(Why::SwitchOff));

    let out = r
        .core
        .tools()
        .call(
            &tools::Caller {
                sender: inbox::Sender::new(&r.key_a, "agent", "claude"),
                path: String::new(),
            },
            "board_steps",
            &serde_json::to_vec(&serde_json::json!({
                "task_id": t.id,
                "steps": [{"text": "the work", "spawn": true}],
            }))
            .unwrap(),
        )
        .unwrap();
    assert!(
        out.contains("no agent yet: this board doesn't start agents"),
        "{out}"
    );
    assert_eq!(
        r.core.why_no_agent(&t.id, false),
        Some(Why::SwitchOff),
        "the mark asks for it from now on"
    );
    let board = r
        .core
        .tools()
        .call(
            &tools::Caller {
                sender: inbox::Sender::new(&r.key_a, "agent", "claude"),
                path: String::new(),
            },
            "board",
            b"{}",
        )
        .unwrap();
    assert!(board.contains("no agent yet: this board"), "{board}");
}

#[test]
fn no_spawner_here_means_this_device_cant_run_agents() {
    let r = Rig::new();
    switch(&r, true);
    let t = card(&r, "A card");
    match r.core.why_no_agent(&t.id, true) {
        Some(Why::CantRun { .. }) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_pending_request_says_where_it_starts() {
    let r = Rig::new();
    switch(&r, true);
    let t = card(&r, "A card");
    let co = r.core.coord();
    co.request_spawn(
        "irori",
        SpawnAsk {
            task_id: t.id.clone(),
            device: r.core.device_id().to_string(),
            repo_key: REPO.into(),
            permission_mode: "acceptEdits".into(),
            name: "heron".into(),
            ..SpawnAsk::default()
        },
    )
    .unwrap()
    .unwrap();
    match r.core.why_no_agent(&t.id, false) {
        Some(Why::Starting { .. }) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_failed_request_says_trust_folder_or_the_refusal() {
    let r = Rig::new();
    switch(&r, true);
    r.core.set_spawner(Some(idle_spawner()));
    r.core.set_spawn_cap(0).unwrap();
    let dev = r.core.device_id().to_string();

    let t = card(&r, "Trust");
    failed(&r, &t, &format!("not trusted: /work/api on {dev}"));
    match r.core.why_no_agent(&t.id, false) {
        Some(Why::NeedsTrust { folder, device }) => {
            assert_eq!(folder, "/work/api");
            assert_ne!(device, dev, "the device by name, never its id");
        }
        other => panic!("{other:?}"),
    }

    let t = card(&r, "Folder");
    failed(&r, &t, "no copy of api on Mac");
    match r.core.why_no_agent(&t.id, false) {
        Some(Why::NoFolder(w)) => assert!(w.starts_with("no copy of api on "), "{w}"),
        other => panic!("{other:?}"),
    }

    let t = card(&r, "Refused");
    failed(&r, &t, "couldn't start: no such command");
    assert_eq!(
        r.core.why_no_agent(&t.id, false),
        Some(Why::Refused("couldn't start: no such command".into()))
    );

    // #541's refusals arrive the same way, in their own words.
    let t = card(&r, "No Claude");
    failed(&r, &t, "Claude Code isn't installed on Mac");
    assert_eq!(
        r.core.why_no_agent(&t.id, false),
        Some(Why::Refused("Claude Code isn't installed on Mac".into()))
    );
}

#[test]
fn a_full_device_says_n_of_m_and_pause_all_says_so() {
    let r = Rig::new();
    switch(&r, true);
    r.core.set_spawner(Some(idle_spawner()));
    r.core.set_spawn_cap(0).unwrap();
    let t = card(&r, "A card");
    match r.core.why_no_agent(&t.id, false) {
        Some(Why::AtCap { running, cap, .. }) => assert_eq!((running, cap), (0, 0)),
        other => panic!("{other:?}"),
    }

    let inbox = r.core.inbox();
    let mut pol = inbox.policy();
    pol.paused = true;
    inbox.set_policy(pol, "user:test").unwrap();
    match r.core.why_no_agent(&t.id, false) {
        Some(Why::Paused { .. }) => {}
        other => panic!("{other:?}"),
    }
}

/// A card with no checkout here, or an untrusted one, says so whether the
/// sweep has tried yet (the line looks for itself) or not (its failed row
/// says the same).
#[test]
fn a_card_hears_folder_and_trust_with_or_without_a_try() {
    let r = Rig::new();
    switch(&r, true);
    let mut sp = idle_spawner();
    sp.trusted = Arc::new(|_: &Path| false);
    r.core.set_spawner(Some(sp));
    let t = card(&r, "No copy here");
    match r.core.why_no_agent(&t.id, true) {
        Some(Why::NoFolder(w)) => assert!(w.starts_with("no copy of api on "), "{w}"),
        other => panic!("{other:?}"),
    }

    let d = tempfile::tempdir().unwrap();
    const OTHER: &str = "remote:github.com/stephen/seen";
    r.core
        .capture()
        .emit_row(
            "irori",
            capture::KIND_REPO_SEEN,
            &capture::RepoSeen {
                repo_key: OTHER.into(),
                root: d.path().to_string_lossy().into_owned(),
                at: gocompat::time::GoTime::unix(1_790_000_000, 0),
            },
        )
        .unwrap();
    let person = format!("user:{}", r.core.device_id());
    let t = r
        .core
        .coord()
        .create_task(
            &person,
            NewTask {
                title: "Not trusted here".into(),
                repo_key: OTHER.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    wait_for(60, "the folder is named as untrusted", || {
        matches!(
            r.core.why_no_agent(&t.id, true),
            Some(Why::NeedsTrust { .. })
        )
    });
}

#[test]
fn a_shared_card_says_so_whatever_the_switch() {
    let r = Rig::new();
    let person = format!("user:{}", r.core.device_id());
    let t = r
        .core
        .coord()
        .create_task(
            &person,
            NewTask {
                title: "Join it".into(),
                shared: true,
                ..NewTask::default()
            },
        )
        .unwrap();
    assert_eq!(r.core.why_no_agent(&t.id, true), Some(Why::Joinable));
    assert_eq!(r.core.why_no_agent(&t.id, false), None, "not asked");
}

#[test]
fn a_card_no_device_hands_out_says_to_take_it_up() {
    let r = Rig::new();
    switch(&r, true);
    r.core.set_spawner(Some(idle_spawner()));
    let t = r
        .core
        .coord()
        .create_task(
            "irori",
            NewTask {
                title: "Irori made this".into(),
                repo_key: REPO.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    assert_eq!(r.core.why_no_agent(&t.id, true), Some(Why::NoOrigin));
}

#[test]
fn a_closed_card_says_nothing() {
    let r = Rig::new();
    let t = card(&r, "A card");
    let co = r.core.coord();
    co.claim_task(&r.key_a, &t.id).unwrap();
    co.set_steps_notice(
        &r.key_a,
        &t.id,
        vec![coord::Step {
            text: "a part".into(),
            spawn: true,
            ..coord::Step::default()
        }],
    )
    .unwrap();
    co.set_task_state(&r.key_a, &t.id, coord::CANCELED, "")
        .unwrap();
    assert_eq!(r.core.why_no_agent(&t.id, true), None);
}

#[test]
fn a_step_mark_on_a_held_card_says_to_split_it() {
    let r = Rig::new();
    let t = card(&r, "A card");
    let co = r.core.coord();
    co.claim_task(&r.key_a, &t.id).unwrap();
    assert_eq!(r.core.why_no_agent(&t.id, true), None, "it has its agent");
    co.set_steps_notice(
        &r.key_a,
        &t.id,
        vec![coord::Step {
            text: "a part".into(),
            spawn: true,
            ..coord::Step::default()
        }],
    )
    .unwrap();
    assert_eq!(r.core.why_no_agent(&t.id, false), Some(Why::StepOnly));
}

/// Stephen and Bob on one board, on real Cores. Bob's card asked about from
/// Stephen's side is Bob's to start; Stephen's own card waits on his switch;
/// an agent of Stephen's waiting on Bob's card is woken when it's done, and
/// the wait never reaches Bob's device, nor the waiting agent his board.
/// Once Bob's device is known to read no agent starts, the line names him.
#[test]
fn a_shared_board_says_whose_card_and_wakes_a_waiter() {
    let mut st = person("stephen", 1);
    let mut bo = person("bob", 1);
    let (s, b) = (st.remove(0), bo.remove(0));
    let board = invited(&s, &b);
    let (sco, bco) = (
        s.core.board_coord(&board).unwrap(),
        b.core.board_coord(&board).unwrap(),
    );
    let new = |co: &coord::Coord, by: String, title: &str| {
        co.create_task(
            &by,
            NewTask {
                title: title.into(),
                repo_key: REPO.into(),
                ..NewTask::default()
            },
        )
        .unwrap()
    };

    let theirs = new(&bco, b.person_actor(), "Bob's card");
    wait_for(90, "Stephen's device has Bob's card", || {
        sco.get_task(&theirs.id).is_some()
    });
    assert_eq!(s.core.why_no_agent(&theirs.id, false), None, "not asked");
    match s.core.why_no_agent(&theirs.id, true) {
        Some(Why::OthersCard { .. }) => {}
        other => panic!("{other:?}"),
    }

    let mine = new(&sco, s.person_actor(), "Stephen's card");
    assert_eq!(s.core.why_no_agent(&mine.id, true), Some(Why::SwitchOff));

    let on_board = |c: &irori_core::Core| {
        c.board_agents()
            .iter()
            .filter(|a| a.board_id == board && a.person_id == s.core.person().id)
            .count()
    };
    let before = on_board(&s.core);
    let out = s.tool(
        "wait",
        serde_json::json!({"task_id": theirs.id, "until": ["done"], "note": "private words"}),
    );
    assert!(out.contains("Only you see this wait"), "{out}");
    assert_eq!(
        on_board(&s.core),
        before,
        "waiting puts no agent of Stephen's on the board"
    );
    assert!(
        s.tool("board", serde_json::json!({"repo": "*"}))
            .contains("(until done)"),
        "the card shows who waits"
    );

    let holder = b.agent.clone();
    let bkey = b.core.board_key(&board, &holder).unwrap();
    bco.claim_task(&bkey, &theirs.id).unwrap();
    bco.set_task_state(&bkey, &theirs.id, coord::CANCELED, "")
        .unwrap();
    wait_for(90, "Stephen's agent is woken", || {
        s.core
            .inbox()
            .for_agent(&s.agent, true, 0)
            .iter()
            .any(|d| d.message.notice && d.message.text.contains("Your wait went off"))
    });
    assert!(s.core.coord().waits("").is_empty(), "it fired once");
    assert!(
        bco.waits("").is_empty() && b.core.coord().waits("").is_empty(),
        "nothing of the wait reached Bob"
    );

    s.core
        .set_spawn_setting(SpawnSetting {
            board_id: board.clone(),
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    let bob_device = b.core.device_id().to_string();
    let (bob_person, bob_name) = (b.core.person().id, b.core.person().name);
    drop(sco);
    drop(bco);
    drop(b);
    std::thread::sleep(Duration::from_millis(500));
    // What Stephen heard live from Bob wins over what was saved, so a
    // restart, then Bob's device saved as reading no agent starts (an
    // older Irori, as card #577 remembers it): the line names Bob.
    let s = restart(s);
    s.core.set_spawner(Some(idle_spawner()));
    s.core
        .board_store(&board)
        .unwrap()
        .append_local(
            "irori",
            KIND_BOARD_HELLO,
            &SavedHello {
                device_id: bob_device,
                person_id: bob_person,
                core_version: "0.10.2".into(),
                seq: i64::MAX,
                ..Default::default()
            },
        )
        .unwrap();
    wait_for(60, "the board reads as not ready", || {
        matches!(
            s.core.why_no_agent(&mine.id, true),
            Some(Why::OlderMember { .. })
        )
    });
    match s.core.why_no_agent(&mine.id, true) {
        Some(Why::OlderMember { people }) => assert_eq!(people, vec![bob_name]),
        other => panic!("{other:?}"),
    }
}

/// A card another of the person's devices made is that device's to start.
#[test]
fn another_of_my_devices_starts_what_it_made() {
    let mut st = person("stephen", 2);
    let (a, b) = (st.remove(0), st.remove(0));
    for d in [&a, &b] {
        d.core
            .set_spawn_setting(SpawnSetting {
                on: true,
                ..SpawnSetting::default()
            })
            .unwrap();
    }
    let t = b
        .core
        .coord()
        .create_task(
            &b.user(),
            NewTask {
                title: "Made on B".into(),
                repo_key: REPO.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    wait_for(90, "A has the card", || {
        a.core.coord().get_task(&t.id).is_some()
    });
    wait_for(60, "the switch reached A's coord", || {
        a.core.coord().spawning()
    });
    match a.core.why_no_agent(&t.id, true) {
        Some(Why::Elsewhere { .. }) => {}
        other => panic!("{other:?}"),
    }
}

/// A blocked card says it's blocked, not its old failure: Trust and start
/// starts nothing until the blocker clears.
#[test]
fn a_blocked_card_says_blocked_before_an_old_failure() {
    let r = Rig::new();
    switch(&r, true);
    r.core.set_spawner(Some(idle_spawner()));
    let dev = r.core.device_id().to_string();
    let first = card(&r, "First");
    let t = card(&r, "Untrusted, then blocked");
    failed(&r, &t, &format!("not trusted: /work/api on {dev}"));
    assert!(matches!(
        r.core.why_no_agent(&t.id, false),
        Some(Why::NeedsTrust { .. })
    ));
    r.core
        .coord()
        .raise(
            &r.key_a,
            &t.id,
            coord::NewBlocker {
                question: "waits".into(),
                blocked_by: Some(coord::BlockedBy {
                    kind: coord::BLOCKED_ON_TASK.into(),
                    reference: first.id.clone(),
                }),
                ..coord::NewBlocker::default()
            },
        )
        .unwrap();
    assert_eq!(r.core.why_no_agent(&t.id, false), Some(Why::Blocked));
}

/// A request past its grace with no agent says none showed up, not that
/// the device looks full.
#[test]
fn a_request_with_no_agent_says_none_showed_up() {
    let ahead = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let a = ahead.clone();
    let r = Rig::with(move |o| {
        o.now = Some(Arc::new(move || {
            let by = Duration::from_secs(a.load(std::sync::atomic::Ordering::SeqCst));
            gocompat::time::GoTime::from_system(std::time::SystemTime::now() + by)
        }));
    });
    switch(&r, true);
    r.core.set_spawner(Some(idle_spawner()));
    // At cap 0 the sweep asks nobody again, so the request stays stale.
    r.core.set_spawn_cap(0).unwrap();
    let t = card(&r, "A card");
    r.core
        .coord()
        .request_spawn(
            "irori",
            SpawnAsk {
                task_id: t.id.clone(),
                device: r.core.device_id().to_string(),
                repo_key: REPO.into(),
                permission_mode: "acceptEdits".into(),
                name: "heron".into(),
                ..SpawnAsk::default()
            },
        )
        .unwrap()
        .unwrap();
    assert!(matches!(
        r.core.why_no_agent(&t.id, false),
        Some(Why::Starting { .. })
    ));
    ahead.store(
        coord::SPAWN_GRACE.as_secs() + 10,
        std::sync::atomic::Ordering::SeqCst,
    );
    match r.core.why_no_agent(&t.id, false) {
        Some(Why::NoShow { .. }) => {}
        other => panic!("{other:?}"),
    }
}

/// What's on the card itself (its side, its lead) is said whether or not
/// the switch is on, but on its own only when the board starts agents.
#[test]
fn a_part_says_its_lead_and_a_card_its_side() {
    let r = Rig::new();
    r.core.set_spawner(Some(idle_spawner()));
    r.core.set_spawn_cap(0).unwrap();
    let co = r.core.coord();
    let person = format!("user:{}", r.core.device_id());
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
    co.claim_task(&r.key_a, &p.id).unwrap();
    let sub = |title: &str, spawn: bool| coord::NewSubtask {
        title: title.into(),
        spawn,
        ..coord::NewSubtask::default()
    };
    let (_, parts) = co
        .split(
            &r.key_a,
            &p.id,
            vec![sub("plain", false), sub("helper", true)],
            vec![],
        )
        .unwrap();
    let plain = &parts[0];
    assert_eq!(r.core.why_no_agent(&plain.id, false), None, "switch off");
    assert_eq!(r.core.why_no_agent(&plain.id, true), Some(Why::ForLead));
    switch(&r, true);
    assert_eq!(r.core.why_no_agent(&plain.id, false), Some(Why::ForLead));

    r.core.set_device_sides(&["front".into()]).unwrap();
    let t = co
        .create_task(
            &person,
            NewTask {
                title: "Back end".into(),
                repo_key: REPO.into(),
                side: "back".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    match r.core.why_no_agent(&t.id, false) {
        Some(Why::OffSide(w)) => assert!(w.contains("back"), "{w}"),
        other => panic!("{other:?}"),
    }
}
