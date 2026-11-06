//! Card #575 through the Core and the tools: agents start, list and end
//! agents inside the limit the person sets in the app, can't raise it or
//! chain past it, and get less than the person's own Start.

mod common;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use coord::{NewTask, Task};
use inbox::Sender;
use irori_core::spawn::{SpawnReq, SpawnSetting, SpawnStarted, Spawner};
use irori_core::spawn_agents::{AGENT_STARTS_MAX, AGENT_STARTS_ON};
use irori_core::spawn_why::Why;
use store::kinds;

const REPO: &str = "remote:github.com/stephen/api";
const CHILD: &str = "dddddddd-4444-4444-8444-000000000004";

/// A spawner whose agent shows up in presence as `CHILD`.
fn spawner(claude: &Path, started: Arc<Mutex<Vec<String>>>) -> Spawner {
    let sessions = claude.join("sessions");
    let gone = sessions.clone();
    let ended = started.clone();
    Spawner {
        spawn: Arc::new(move |r: &SpawnReq| {
            started.lock().unwrap().push(r.task_id.clone());
            write_session(
                &sessions,
                "900004.json",
                CHILD,
                &r.name,
                Path::new(&r.folder),
            );
            Ok(SpawnStarted {
                name: r.name.clone(),
                prefix: CHILD[..8].into(),
            })
        }),
        end: Arc::new(move |_: &str| {
            ended.lock().unwrap().push("ended".into());
            let _ = std::fs::remove_file(gone.join("900004.json"));
            Ok(String::new())
        }),
        trusted: Arc::new(|_: &Path| true),
        list: None,
    }
}

/// A rig on a spawning private board, its device at cap 0 so nothing starts
/// until a test raises it.
fn rig() -> (Rig, tempfile::TempDir, Arc<Mutex<Vec<String>>>) {
    let r = Rig::new();
    let checkout = tempfile::tempdir().unwrap();
    r.core
        .capture()
        .emit_row(
            "irori",
            capture::KIND_REPO_SEEN,
            &capture::RepoSeen {
                repo_key: REPO.into(),
                root: checkout.path().to_string_lossy().into_owned(),
                at: gocompat::time::GoTime::unix(1_790_000_000, 0),
            },
        )
        .unwrap();
    r.core.set_spawn_cap(0).unwrap();
    r.core
        .set_spawn_setting(SpawnSetting {
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    let started = Arc::new(Mutex::new(Vec::new()));
    r.core
        .set_spawner(Some(spawner(&r.claude, started.clone())));
    (r, checkout, started)
}

fn card_by(r: &Rig, by: &str, title: &str) -> Task {
    r.core
        .coord()
        .create_task(
            by,
            NewTask {
                title: title.into(),
                repo_key: REPO.into(),
                ..NewTask::default()
            },
        )
        .unwrap()
}

fn card(r: &Rig, title: &str) -> Task {
    card_by(r, &format!("user:{}", r.core.device_id()), title)
}

fn tool(r: &Rig, key: &str, name: &str, args: serde_json::Value) -> String {
    let c = tools::Caller {
        sender: Sender::new(key, "alpha", "claude"),
        path: inbox::path::MOD.into(),
    };
    r.core
        .tools()
        .call(&c, name, &serde_json::to_vec(&args).unwrap())
        .unwrap()
}

fn start(r: &Rig, t: &Task) -> String {
    tool(
        r,
        &r.key_a,
        "agent_start",
        serde_json::json!({ "task_id": t.id }),
    )
}

/// A row the sweep reads, and time for it to run.
fn sweep(r: &Rig) {
    let cap = r.core.spawn_cap();
    r.core.set_spawn_cap(cap).unwrap();
    std::thread::sleep(Duration::from_millis(400));
}

#[test]
fn on_with_a_maximum_of_100_for_a_new_person() {
    let r = Rig::new();
    let u = r.core.agent_starts();
    assert_eq!((u.on, u.max), (AGENT_STARTS_ON, AGENT_STARTS_MAX));
    assert_eq!((u.on, u.max, u.used), (true, 100, 0));
}

#[test]
fn switched_off_an_agent_is_told_and_nothing_is_asked() {
    let (r, _dir, _) = rig();
    r.core.set_agent_starts(false, 100).unwrap();
    let t = card(&r, "One");
    let out = start(&r, &t);
    assert!(out.starts_with("Error: "), "{out}");
    assert!(out.contains("Settings › Agents"), "{out}");
    assert_eq!(r.core.agent_starts().used, 0);
    let listed = tool(&r, &r.key_a, "agents_started", serde_json::json!({}));
    assert!(listed.contains("turned off"), "{listed}");
}

#[test]
fn past_the_maximum_is_refused_with_n_of_m() {
    let (r, _dir, _) = rig();
    r.core.set_agent_starts(true, 1).unwrap();
    let (a, b) = (card(&r, "One"), card(&r, "Two"));
    let out = start(&r, &a);
    assert!(out.contains("1 of the person's 1"), "{out}");
    let out = start(&r, &b);
    assert!(out.starts_with("Error: ") && out.contains("(1/1)"), "{out}");
    // Asking twice for one card takes no second slot.
    let out = start(&r, &a);
    assert!(!out.starts_with("Error: "), "{out}");
    assert_eq!(r.core.agent_starts().used, 1);
}

#[test]
fn no_tool_changes_the_limit_and_a_forged_row_is_ignored() {
    let (r, _dir, _) = rig();
    r.core.set_agent_starts(true, 2).unwrap();
    // Every actor but the person: an agent, and Irori's own sweeper.
    for actor in [r.key_a.as_str(), "irori"] {
        let forged = serde_json::json!({
            "device": "agents", "cap": 1000, "on": true, "by": actor,
        });
        r.core
            .store()
            .append(actor, kinds::SPAWN_CAP, &forged)
            .unwrap();
    }
    sweep(&r);
    assert_eq!(r.core.agent_starts().max, 2);
    // No tool names the setting, and none takes a maximum.
    let specs: serde_json::Value =
        serde_json::from_slice(&r.core.tools().specs_json("claude")).unwrap();
    for s in specs.as_array().unwrap() {
        let name = s["name"].as_str().unwrap();
        assert!(
            !name.contains("limit") && !name.contains("setting"),
            "{name}"
        );
        if name.starts_with("agent") {
            let props = s["inputSchema"]["properties"].as_object().unwrap();
            assert!(!props.keys().any(|k| k == "max" || k == "on"), "{name}");
        }
    }
    assert!(r.core.set_agent_starts(true, -1).is_err());
    assert!(r.core.set_agent_starts(true, 1001).is_err());
}

#[test]
fn a_card_waiting_on_the_person_is_refused_and_stays_put() {
    let (r, _dir, _) = rig();
    let t = card(&r, "One");
    let co = r.core.coord();
    let out = tool(
        &r,
        &r.key_a,
        "board_block",
        serde_json::json!({ "task_id": t.id, "question": "Which way?" }),
    );
    assert!(!out.starts_with("Error: "), "{out}");
    let was = co.get_task(&t.id).unwrap().state;
    let out = start(&r, &t);
    assert!(out.starts_with("Error: "), "{out}");
    assert_eq!(co.get_task(&t.id).unwrap().state, was);
    assert_eq!(r.core.agent_starts().used, 0);
}

#[test]
fn the_board_switch_is_honoured() {
    let (r, _dir, _) = rig();
    r.core
        .set_spawn_setting(SpawnSetting {
            on: false,
            ..SpawnSetting::default()
        })
        .unwrap();
    let t = card(&r, "One");
    let out = start(&r, &t);
    assert!(
        out.starts_with("Error: ") && out.contains("doesn't start agents"),
        "{out}"
    );
}

#[test]
fn the_chain_counts_and_a_started_agent_cant_post_past_the_limit() {
    let (r, _dir, started) = rig();
    r.core.set_agent_starts(true, 1).unwrap();
    let parent = card(&r, "Parent");
    assert!(!start(&r, &parent).starts_with("Error: "));
    r.core.set_spawn_cap(5).unwrap();
    let me = r.core.device_id().to_string();
    let child = format!("{me}/{CHILD}");
    let co = r.core.coord();
    wait_for(10, "the started agent is bound", || co.is_spawned(&child));
    assert!(co.spawned(&child).unwrap().counted);
    assert_eq!(r.core.agent_starts().used, 1);

    // It posts a card: that is the chain, and the limit is full.
    let posted = card_by(&r, &child, "Grandchild");
    sweep(&r);
    assert_eq!(started.lock().unwrap().len(), 1, "nothing past the limit");
    match r.core.why_no_agent(&posted.id, true) {
        Some(Why::AgentStartsFull { used: 1, max: 1 }) => {}
        other => panic!("{other:?}"),
    }
    // And the tool says the same to it.
    let out = tool(
        &r,
        &child,
        "agent_start",
        serde_json::json!({ "task_id": posted.id }),
    );
    assert!(out.contains("(1/1)"), "{out}");

    // The person's own Start is never held by the limit.
    r.core.start_agent(&posted.id, "").unwrap();
    sweep(&r);
    assert_eq!(started.lock().unwrap().len(), 2);
    let t = co.get_task(&posted.id).unwrap();
    assert!(!t.spawn_request().unwrap().counted);
}

#[test]
fn a_card_the_person_holds_is_refused_as_theirs() {
    let (r, _dir, started) = rig();
    let t = card(&r, "Mine to do");
    let me = format!("user:{}", r.core.device_id());
    r.core.coord().claim_task(&me, &t.id).unwrap();
    let out = start(&r, &t);
    assert!(out.contains("held by a person"), "{out}");
    r.core.set_spawn_cap(5).unwrap();
    sweep(&r);
    assert!(started.lock().unwrap().is_empty());
    assert_eq!(r.core.agent_starts().used, 0);
}

#[test]
fn a_card_an_ordinary_agent_posts_is_handed_out_as_before() {
    let (r, _dir, started) = rig();
    r.core.set_agent_starts(false, 0).unwrap();
    let t = card_by(&r, &r.key_a, "Normal hand-out");
    r.core.set_spawn_cap(5).unwrap();
    wait_for(10, "it starts", || started.lock().unwrap().len() == 1);
    let t = r.core.coord().get_task(&t.id).unwrap();
    assert!(!t.spawn_request().unwrap().counted);
}

#[test]
fn only_agents_irori_started_are_ended_and_not_yourself() {
    let (r, _dir, started) = rig();
    let parent = card(&r, "Parent");
    assert!(!start(&r, &parent).starts_with("Error: "));
    r.core.set_spawn_cap(5).unwrap();
    let me = r.core.device_id().to_string();
    let child = format!("{me}/{CHILD}");
    let co = r.core.coord();
    wait_for(10, "bound", || co.is_spawned(&child));
    wait_for(10, "on the roster", || {
        tool(&r, &r.key_a, "agents_started", serde_json::json!({})).contains(CHILD)
    });

    // A terminal agent the person started isn't Irori's to end.
    let out = tool(
        &r,
        &r.key_a,
        "agent_end",
        serde_json::json!({ "agent": r.key_b }),
    );
    assert!(out.contains("didn't start"), "{out}");
    // Nor can an agent end itself.
    let out = tool(
        &r,
        &child,
        "agent_end",
        serde_json::json!({ "agent": child }),
    );
    assert!(out.contains("that's you"), "{out}");
    // A card can't go to an agent started for another card, and the refusal
    // comes before anything is ended.
    let out = tool(
        &r,
        &r.key_a,
        "agent_end",
        serde_json::json!({ "agent": child, "give_to": child }),
    );
    assert!(out.contains("can't take another"), "{out}");
    assert!(co.spawned_live(&child));
    // Switched off, an agent ends nothing.
    r.core.set_agent_starts(false, 100).unwrap();
    let out = tool(
        &r,
        &r.key_a,
        "agent_end",
        serde_json::json!({ "agent": child }),
    );
    assert!(out.starts_with("Error: "), "{out}");
    r.core.set_agent_starts(true, 100).unwrap();
    let out = tool(
        &r,
        &r.key_a,
        "agent_end",
        serde_json::json!({ "agent": child }),
    );
    assert!(out.starts_with("Ending it"), "{out}");
    // Its hand-off, then the end the person's End runs.
    let said = tool(
        &r,
        &child,
        "board_comment",
        serde_json::json!({ "task_id": parent.id, "text": "Handing off." }),
    );
    assert!(!said.starts_with("Error: "), "{said}");
    wait_for(30, "it was ended", || {
        started.lock().unwrap().iter().any(|s| s == "ended")
    });
}

#[test]
fn a_counted_agent_that_goes_frees_its_slot() {
    let (r, _dir, _) = rig();
    r.core.set_agent_starts(true, 1).unwrap();
    let parent = card(&r, "Parent");
    assert!(!start(&r, &parent).starts_with("Error: "));
    r.core.set_spawn_cap(5).unwrap();
    let child = format!("{}/{CHILD}", r.core.device_id());
    let co = r.core.coord();
    wait_for(10, "bound", || co.is_spawned(&child));
    wait_for(10, "counted", || r.core.agent_starts().used == 1);
    // It exits by itself: nobody calls End, and no restart happens.
    r.core.set_spawn_cap(0).unwrap();
    std::fs::remove_file(r.claude.join("sessions/900004.json")).unwrap();
    wait_for(20, "its slot frees", || r.core.agent_starts().used == 0);
}

#[test]
fn an_agents_ask_is_written_and_lowering_the_maximum_keeps_the_order() {
    let (r, _dir, started) = rig();
    r.core.set_agent_starts(true, 2).unwrap();
    let (a, b) = (card(&r, "One"), card(&r, "Two"));
    assert!(!start(&r, &a).starts_with("Error: "));
    std::thread::sleep(Duration::from_millis(5));
    assert!(!start(&r, &b).starts_with("Error: "));
    // On a row, so a restart keeps the ask and its count.
    let rows = r
        .core
        .store()
        .list(&store::Filter {
            kinds: vec![kinds::SPAWN_START_REQUESTED.into()],
            ..store::Filter::default()
        })
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(String::from_utf8_lossy(&rows[0].payload).contains("\"agent\":true"));
    // Lowered under the asks waiting: the older still goes, not neither.
    r.core.set_agent_starts(true, 1).unwrap();
    r.core.set_spawn_cap(5).unwrap();
    wait_for(10, "the older ask starts", || {
        started.lock().unwrap().first() == Some(&a.id)
    });
    sweep(&r);
    assert_eq!(started.lock().unwrap().len(), 1, "the newer waits");
}
