//! Card #433: an attached session presence listed as gone (its session
//! file was unreadable when last read, as on a full disk) comes back on
//! activity: a tool call through its attach secret, or its attach
//! stream's keepalive or a new hello. Back, it is listed live on this
//! device and on the others (an agent.seen row), a mention of it is
//! delivered and it can join a card. A session whose process ended stays
//! gone whatever its stream sends.
//!
//! The session's process is a fake pid whose liveness the test sets; both
//! presence intervals are an hour and exit waits are off, so only the
//! activity paths (or a session-file event) can rescan.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use irori_api::client::Opened;
use serde_json::{Value, json};
use store::{Filter, kinds};

const SESS_G: &str = "dddddddd-4333-4433-8433-000000000433";
const FAKE_PID: i64 = (1 << 30) + 433;
const HOUR: Duration = Duration::from_secs(3600);

struct Fx {
    rig: common::Rig,
    alive: Arc<AtomicBool>,
    key_g: String,
}

fn fixture() -> Fx {
    let alive = Arc::new(AtomicBool::new(true));
    let a = alive.clone();
    let rig = common::Rig::with(move |o| {
        let a = a.clone();
        o.presence = Some(Arc::new(move |p: &mut presence::Options| {
            let def = presence::internal::default_live();
            let a = a.clone();
            p.live = Some(Arc::new(move |pid, start| {
                if pid == FAKE_PID {
                    a.load(Ordering::SeqCst)
                } else {
                    def(pid, start)
                }
            }));
            p.watch_exit = Some(Arc::new(|_, _| None));
            p.interval = HOUR;
            p.safety_interval = HOUR;
        }));
    });
    let sessions = rig.claude.join("sessions");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let m = json!({
        "pid": FAKE_PID, "sessionId": SESS_G, "cwd": rig.cwd.to_string_lossy(),
        "name": "gudgeon", "status": "idle", "kind": "bg", "entrypoint": "cli",
        "version": "2.1.311", "startedAt": now, "updatedAt": now,
    });
    std::fs::write(
        sessions.join(format!("{FAKE_PID}.json")),
        serde_json::to_vec(&m).unwrap(),
    )
    .unwrap();
    // The session-file event lists it; nudge in case the watcher is slow.
    let key_g = format!("{}/{SESS_G}", rig.core.device_id());
    let deadline = Instant::now() + Duration::from_secs(30);
    while !listed(&rig, &key_g) {
        assert!(Instant::now() < deadline, "gudgeon never listed");
        rig.core.presence().rescan();
        std::thread::sleep(Duration::from_millis(50));
    }
    Fx { rig, alive, key_g }
}

fn listed(rig: &common::Rig, key: &str) -> bool {
    rig.core.local_agents().iter().any(|c| c.key == key)
}

/// Opens an attach stream for `session` and returns its hello; the rest
/// drains on the rig's runtime for the test's life.
fn attach(rig: &common::Rig, session: &str) -> Result<Value, u16> {
    rig.rt.block_on(async {
        let path = format!("/v1/attach?session={session}&mod=0.4.1");
        match rig.client.open(&path, &[]).await.unwrap() {
            Opened::Stream(mut lines, _) => {
                let hello = lines
                    .next_json(Duration::from_secs(5))
                    .await
                    .expect("hello");
                tokio::spawn(async move { while lines.next().await.is_some() {} });
                Ok(serde_json::from_slice(&hello).unwrap())
            }
            Opened::Refused(r) => Err(r.status),
        }
    })
}

fn secret(hello: &Value) -> String {
    hello["attachSecret"]
        .as_str()
        .expect("attach secret")
        .to_string()
}

/// A tool call through an attach secret: the tool's text.
fn tool(rig: &common::Rig, secret: &str, name: &str, args: Value) -> String {
    let body = serde_json::to_vec(&json!({ "args": args })).unwrap();
    let out = rig
        .rt
        .block_on(rig.client.call(
            "POST",
            &format!("/v1/tools/{name}"),
            &[(irori_api::attach::HEADER_ATTACH, secret)],
            Some(body),
        ))
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    let v: Value = serde_json::from_slice(&out).unwrap();
    v["text"].as_str().unwrap().to_string()
}

/// The agent rows for `key`, oldest first.
fn rows(rig: &common::Rig, key: &str) -> Vec<String> {
    rig.core
        .events(&Filter {
            kinds: vec![
                kinds::AGENT_SEEN.into(),
                kinds::AGENT_STATUS.into(),
                kinds::AGENT_GONE.into(),
            ],
            limit: store::MAX_LIST_LIMIT,
            ..Filter::default()
        })
        .unwrap()
        .into_iter()
        .filter(|e| String::from_utf8_lossy(&e.payload).contains(key))
        .map(|e| e.kind)
        .collect()
}

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Whether another device folding this device's log lists `key`.
fn others_list(rig: &common::Rig, key: &str) -> bool {
    coord::roster::Roster::new(coord::roster::Options {
        store: rig.core.store().clone(),
        self_id: "other-device".into(),
        local: None,
        liveness: None,
        seen: None,
        via: None,
        revoked: None,
    })
    .unwrap()
    .agents()
    .iter()
    .any(|c| c.key == key)
}

/// Presence lists it gone: its process is dead at the next scan.
fn go_gone(fx: &Fx) {
    fx.alive.store(false, Ordering::SeqCst);
    fx.rig.core.presence().rescan();
    assert!(!listed(&fx.rig, &fx.key_g), "still listed after it went");
    wait_for("agent.gone", || {
        rows(&fx.rig, &fx.key_g).last().map(String::as_str) == Some(kinds::AGENT_GONE)
    });
    assert!(!others_list(&fx.rig, &fx.key_g));
}

#[test]
fn a_gone_session_active_again_is_live_reachable_and_can_join() {
    let fx = fixture();
    let rig = &fx.rig;
    let g = secret(&attach(rig, SESS_G).expect("gudgeon attach"));
    let b = secret(&attach(rig, common::SESS_B).expect("beta attach"));
    let person = format!("user:{}", rig.core.device_id());
    let co = rig.core.coord();
    let task = co
        .create_task(
            &person,
            coord::NewTask {
                title: "the card #391 was".into(),
                objective: "keep it with gudgeon".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let mention = |text: &str| {
        tool(
            rig,
            &b,
            "board_comment",
            json!({"task_id": task.id, "text": text}),
        )
    };

    // Gone (the session file read as nothing while its process ran on).
    go_gone(&fx);
    let out = mention("@gudgeon are you there?");
    assert!(out.contains("\"@gudgeon\" names no live agent"), "{out}");

    // Let the Core's own reaction to the change (the namer settling, which
    // may rescan) pass while the process is still dead; then it is alive
    // again, but nothing moves on disk, so nothing lists it yet.
    std::thread::sleep(Duration::from_millis(300));
    fx.alive.store(true, Ordering::SeqCst);
    assert!(!listed(rig, &fx.key_g), "listed with no activity");

    // One tool call through its attach secret brings it back before the
    // call runs: it joins the card.
    let out = tool(rig, &g, "board_join", json!({"task_id": task.id}));
    assert!(out.starts_with("You hold"), "board_join: {out}");
    assert!(listed(rig, &fx.key_g), "not listed after a tool call");
    assert_eq!(
        co.get_task(&task.id).unwrap().holders,
        [fx.key_g.as_str()],
        "holders"
    );
    // The other devices list it again from its agent.seen row.
    wait_for("agent.seen after agent.gone", || {
        rows(rig, &fx.key_g).last().map(String::as_str) == Some(kinds::AGENT_SEEN)
    });
    assert!(
        others_list(rig, &fx.key_g),
        "another device doesn't list it"
    );
    // A mention of it is delivered.
    let out = mention("@gudgeon are you there?");
    assert!(!out.contains("names no live agent"), "{out}");
    assert!(out.contains("Mentioned: gudgeon"), "{out}");
    wait_for("the mention's notice", || {
        rig.core
            .inbox()
            .for_agent(&fx.key_g, true, 0)
            .iter()
            .any(|d| d.message.text.contains("are you there"))
    });

    // Truly ended: the process is gone; calls through its old secret
    // don't bring it back.
    std::thread::sleep(Duration::from_millis(2100)); // past the stream's ask limit
    go_gone(&fx);
    let _ = tool(rig, &g, "board", json!({}));
    assert!(!listed(rig, &fx.key_g), "an ended session came back");
    let out = mention("@gudgeon still?");
    assert!(out.contains("\"@gudgeon\" names no live agent"), "{out}");
    assert!(!others_list(rig, &fx.key_g));
}

#[test]
fn the_attach_stream_brings_a_gone_session_back() {
    let fx = fixture();
    let rig = &fx.rig;
    rig.server.set_keepalive(Duration::from_millis(40));
    attach(rig, SESS_G).expect("gudgeon attach");
    go_gone(&fx);

    // A new attach while its process is dead is refused.
    assert_eq!(attach(rig, SESS_G).err(), Some(404), "attach while dead");

    // Alive again: the open stream's keepalive brings it back.
    fx.alive.store(true, Ordering::SeqCst);
    wait_for("listed from the keepalive", || listed(rig, &fx.key_g));
    wait_for("agent.seen", || {
        rows(rig, &fx.key_g).last().map(String::as_str) == Some(kinds::AGENT_SEEN)
    });

    // Gone again, then a fresh attach (a restarted `irori attach`) brings
    // it back with its hello.
    go_gone(&fx);
    fx.alive.store(true, Ordering::SeqCst);
    let hello = attach(rig, SESS_G).expect("attach after it came back");
    assert_eq!(hello["name"], "gudgeon", "{hello}");
    assert!(listed(rig, &fx.key_g));
}
