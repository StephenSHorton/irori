//! Ports of the I:internal/core tests that need neither the network nor
//! the window nor the API: board_test.go, notice_test.go, record_test.go,
//! reset_test.go (without the mesh), capture_test.go, mesh_test.go's lone
//! Core, and the Core's own wiring: start/stop and the lockfile, the
//! expiry timer (coord.md §6 "expiry timer fires without a row"), the
//! loops (follow, sweep, names), the change feed and the decision fold.

mod common;

use std::time::{Duration, Instant};

use common::*;
use coord::{BlockedBy, BlockerAnswer, BlockerOption, NewBlocker, NewTask, Step};
use gocompat::sub::TryRecv;
use inbox::{Address, SendOptions, Sender, addr, mode};
use irori_core::{Core, CoreEvent, StartError, Topic, lockfile, reset};
use store::{Filter, kinds};

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

/// A watcher hears what changed on a card from Irori at its next turn,
/// through the coord sweep; the person's answer reaches the raiser as a
/// wake when the person chose it.
#[test]
fn test_card_notices_route() {
    let r = Rig::new();
    let co = r.core.coord();
    let task = co.create_task(&r.key_a, nt("ship")).unwrap();
    co.claim_task(&r.key_a, &task.id).unwrap();
    co.watch(&r.key_b, &task.id).unwrap();
    co.set_steps(
        &r.key_a,
        &task.id,
        vec![Step {
            text: "tag it".into(),
            ..Step::default()
        }],
    )
    .unwrap();
    co.raise(
        &r.key_a,
        &task.id,
        NewBlocker {
            question: "Ship now?".into(),
            options: vec![BlockerOption {
                answer: "yes".into(),
                ..BlockerOption::default()
            }],
            ..NewBlocker::default()
        },
    )
    .unwrap();
    r.core.sweep_coord().unwrap();
    let ns = r.notices_to(&r.key_b);
    assert!(
        ns.len() == 1
            && ns[0].mode == mode::NEXT_TURN
            && ns[0].message.from.key == inbox::SYSTEM_KEY
            && ns[0].message.text.contains("raised a blocker: Ship now?")
            && ns[0].message.text.contains("set the steps"),
        "watcher notices {ns:?}"
    );
    let user = r.core.user_sender();
    let (_, n) = co
        .answer(
            &user.key,
            &task.id,
            "b1",
            BlockerAnswer {
                pick: 1,
                wake: true,
                ..BlockerAnswer::default()
            },
        )
        .unwrap();
    let n = n.into_iter().next().unwrap_or_default();
    r.core.route_notices(&user, &[n]).unwrap();
    let woke = r.notices_to(&r.key_a).iter().any(|d| {
        d.mode == mode::WAKE
            && d.message.ref_ == format!("task:{}", task.id)
            && d.message.text.contains("Answer: yes")
    });
    assert!(woke, "raiser notices {:?}", r.notices_to(&r.key_a));
}

/// Each recipient its own delivery, next_turn unless the notice wakes, as
/// a notice with its ref; an unreachable recipient is reported without
/// stopping the rest.
#[test]
fn test_route_notices() {
    let r = Rig::new();
    for (name, wake, want) in [
        ("next turn", false, mode::NEXT_TURN),
        ("wake", true, mode::WAKE),
    ] {
        let reference = format!("task:{}", name.replace(' ', "-"));
        let n = coord::Notice {
            kind: coord::NOTICE_COMMENT.into(),
            reference: reference.clone(),
            to: vec!["dev-x/not-here".into(), r.key_b.clone()],
            text: format!("a comment on {reference}"),
            wake,
            ..coord::Notice::default()
        };
        let err = r
            .core
            .route_notices(&r.core.user_sender(), &[n])
            .unwrap_err();
        assert!(err.contains("dev-x/not-here"), "{name}: err {err:?}");
        let found = r
            .core
            .inbox()
            .list(500)
            .into_iter()
            .find(|d| d.to == r.key_b && d.message.ref_ == reference)
            .unwrap_or_else(|| panic!("{name}: the reachable recipient got nothing"));
        assert!(
            found.mode == want && found.message.notice,
            "{name}: {found:?}"
        );
    }
}

/// recordGone puts a message whose recipient ended on the card its ref
/// names, marked gone; a ref that names no card here is NoCard.
#[test]
fn test_record_gone_on_the_card() {
    let r = Rig::new();
    let co = r.core.coord();
    let task = co.create_task(&r.key_a, nt("ship")).unwrap();
    let from = Sender::new(&r.key_b, "beta", "claude");
    let send = |reference: &str| {
        r.core
            .inbox()
            .send_with(
                &from,
                &Address::new(addr::AGENT, &r.key_a),
                &format!("about {reference}"),
                SendOptions {
                    ref_: reference.into(),
                    ..SendOptions::default()
                },
            )
            .unwrap()
            .0
    };
    let m = send(&format!("task:{}", task.id));
    assert_eq!(r.core.record_gone(&m, &r.key_a).unwrap(), task.id);
    let cm = co.get_comment(&m.id).expect("on the card");
    assert!(
        cm.gone && cm.msg_id == m.id && cm.by == r.key_b && cm.to == r.key_a && cm.text == m.text,
        "on the card {cm:?}"
    );
    // Again (the sweep after a crash): nothing new.
    r.core.record_gone(&m, &r.key_a).unwrap();
    assert_eq!(co.comments(&task.id).unwrap().len(), 1);
    // A comment ref goes to the comment's card.
    let (c, _) = co.add_comment(&r.key_a, &task.id, "first", false).unwrap();
    let got = r
        .core
        .record_gone(&send(&format!("comment:{}", c.id)), &r.key_a)
        .unwrap();
    assert_eq!(got, task.id);
    for reference in ["task:01NOPE", "comment:01NOPE", "pr:o/r#1", ""] {
        let e = r.core.record_gone(&send(reference), &r.key_a).unwrap_err();
        assert!(e.is(inbox::ErrorKind::NoCard), "ref {reference:?}: {e}");
    }
}

/// A blocker that resolves by itself (the lease is released) is closed by
/// the coord sweep, and its holder hears from Irori at its next turn.
#[test]
fn test_sweep_routes_notices() {
    let r = Rig::new();
    let co = r.core.coord();
    co.acquire(&r.key_b, "gpu/qwen", Duration::ZERO, false)
        .unwrap();
    let task = co.create_task(&r.key_a, nt("train")).unwrap();
    co.claim_task(&r.key_a, &task.id).unwrap();
    co.block(
        &r.key_a,
        &task.id,
        "gpu busy",
        Some(BlockedBy {
            kind: coord::BLOCKED_ON_LEASE.into(),
            reference: "gpu/qwen".into(),
        }),
    )
    .unwrap();
    co.release(&r.key_b, "gpu/qwen").unwrap();
    r.core.sweep_coord().unwrap();
    assert_ne!(
        co.get_task(&task.id).unwrap().state,
        coord::BLOCKED,
        "still blocked"
    );
    let ns = r.notices_to(&r.key_a);
    assert_eq!(ns.len(), 1, "notices {ns:?}");
    let d = &ns[0];
    assert!(
        d.mode == mode::NEXT_TURN
            && d.message.from.key == inbox::SYSTEM_KEY
            && d.message.ref_ == format!("task:{}", task.id),
        "notice {d:?}"
    );
    // A second sweep has nothing more to say.
    r.core.sweep_coord().unwrap();
    assert_eq!(r.notices_to(&r.key_a).len(), 1);
}

/// Go TestBoardToolsThroughCore: the Core's tool set routes a board
/// notice through its inbox, and search reads the Core's capture.
#[test]
fn test_board_tools_through_core() {
    let r = Rig::new();
    let a = tools::Caller {
        sender: Sender::new(&r.key_a, "alpha", "claude"),
        path: inbox::path::MOD.into(),
    };
    let call = |name: &str, args: serde_json::Value| -> String {
        let raw = serde_json::to_vec(&args).unwrap();
        let out = r.core.tools().call(&a, name, &raw);
        match out {
            Ok(t) if !t.starts_with("Error: ") => t,
            other => panic!("{name}: {other:?}"),
        }
    };
    let out = call(
        "board_create",
        serde_json::json!({"title": "hand-off", "assign_to": r.key_b}),
    );
    // beta is idle, so the hand-off wakes it (card #85).
    assert!(out.contains("Woke: beta"), "create: {out}");
    let ns = r.notices_to(&r.key_b);
    assert!(
        ns.len() == 1 && ns[0].mode == mode::WAKE && ns[0].message.from.key == r.key_a,
        "notices {ns:?}"
    );
    let out = call(
        "search",
        serde_json::json!({"query": "hand-off", "kinds": ["task"]}),
    );
    assert!(out.contains("task:"), "search: {out}");
    assert_eq!(
        r.core.tools().call(&a, "nope", b"{}"),
        Err(tools::CallError::UnknownTool("nope".into()))
    );
}

/// What the window calls: a person's comment defaults to the next turn,
/// and the Wake toggle makes it a wake.
#[test]
fn test_route_notice_from_the_person() {
    for (wake, want) in [(false, mode::NEXT_TURN), (true, mode::WAKE)] {
        let r = Rig::new();
        let co = r.core.coord();
        let task = co.create_task(&r.key_a, nt("docs")).unwrap();
        co.claim_task(&r.key_b, &task.id).unwrap();
        let user = r.core.user_sender();
        let (cm, n) = co
            .add_comment(&user.key, &task.id, "use the new heading style", wake)
            .unwrap();
        let rs = r.core.route_notice(&user, &n);
        assert!(irori_core::notice::notice_err(&rs).is_none() && rs.len() == 1);
        let d = &rs[0].delivery;
        assert!(
            rs[0].to == r.key_b
                && d.mode == want
                && d.message.ref_ == format!("comment:{}", cm.id)
                && d.message.notice
                && d.message.trust == inbox::trust::USER,
            "delivery {d:?}"
        );
    }
}

/// A session's mode learned after its agent.seen row reaches the other
/// devices on an agent.status row (and an unchanged update writes none).
#[test]
fn test_record_replicates_a_mode_learned_later() {
    let r = Rig::new();
    let me = r.core.device_id().to_string();
    let key = format!("{me}/cccccccc-3333-4333-8333-000000000003");
    let card = presence::AgentCard {
        key: key.clone(),
        device_id: me.clone(),
        agent: "claude".into(),
        name: "gamma".into(),
        status: "idle".into(),
        ..presence::AgentCard::default()
    };
    let mut moded = card.clone();
    moded.permission_mode = "bypassPermissions".into();
    // Feed the three changes through a queue the record loop reads.
    let q = gocompat::sub::Queue::new(4);
    for (kind, c) in [
        (presence::ChangeKind::Added, card.clone()),
        (presence::ChangeKind::Updated, card.clone()),
        (presence::ChangeKind::Updated, moded),
    ] {
        assert!(q.push(presence::Change { kind, card: c }));
    }
    q.close();
    r.core.record_from(&q);

    let evs = r
        .core
        .events(&Filter {
            kinds: vec![kinds::AGENT_STATUS.into()],
            limit: store::MAX_LIST_LIMIT,
            ..Filter::default()
        })
        .unwrap();
    let rows: Vec<serde_json::Value> = evs
        .iter()
        .filter_map(|e| serde_json::from_slice::<serde_json::Value>(&e.payload).ok())
        .filter(|p| p["key"] == key.as_str())
        .collect();
    assert!(
        rows.len() == 1
            && rows[0]["permissionMode"] == "bypassPermissions"
            && rows[0]["status"] == "idle"
            && rows[0]["name"] == "gamma",
        "status rows for the session: {rows:?}"
    );
    // Go map key order.
    let raw = evs
        .iter()
        .find(|e| String::from_utf8_lossy(&e.payload).contains(&key))
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&raw.payload),
        format!(
            r#"{{"key":"{key}","name":"gamma","permissionMode":"bypassPermissions","status":"idle"}}"#
        )
    );
}

/// Card #433: a card whose agent.seen row never landed after it went (the
/// Added change was dropped, or its append failed on a full disk) gets an
/// agent.seen on its next change, not an agent.status another device
/// drops for a card it doesn't hold; so the other devices list it again.
#[test]
fn test_record_rewrites_seen_after_a_lost_added() {
    let r = Rig::new();
    let me = r.core.device_id().to_string();
    let key = format!("{me}/dddddddd-4444-4444-8444-000000000433");
    let card = presence::AgentCard {
        key: key.clone(),
        device_id: me.clone(),
        agent: "claude".into(),
        name: "gudgeon".into(),
        status: "idle".into(),
        ..presence::AgentCard::default()
    };
    let mut gone = card.clone();
    gone.status = "gone".into();
    let mut busy = card.clone();
    busy.status = "busy".into();
    let q = gocompat::sub::Queue::new(8);
    for (kind, c) in [
        (presence::ChangeKind::Added, card.clone()),
        (presence::ChangeKind::Removed, gone),
        // Its Added back never reached the record loop.
        (presence::ChangeKind::Updated, busy.clone()),
        (presence::ChangeKind::Updated, busy),
    ] {
        assert!(q.push(presence::Change { kind, card: c }));
    }
    q.close();
    r.core.record_from(&q);

    let rows: Vec<String> = r
        .core
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
        .filter(|e| String::from_utf8_lossy(&e.payload).contains(&key))
        .map(|e| e.kind)
        .collect();
    assert_eq!(
        rows,
        [kinds::AGENT_SEEN, kinds::AGENT_GONE, kinds::AGENT_SEEN],
        "rows for the card"
    );

    // Another device folding this log lists it, busy.
    let other = coord::roster::Roster::new(coord::roster::Options {
        store: r.core.store().clone(),
        self_id: "other-device".into(),
        local: None,
        liveness: None,
        seen: None,
        revoked: None,
    })
    .unwrap();
    let c = other
        .agents()
        .into_iter()
        .find(|c| c.key == key)
        .expect("another device doesn't list the card");
    assert_eq!(c.status, "busy");
}

/// The live loop turns the rig's sessions into agent.seen rows, the
/// replicated card stripped of what means nothing on another machine.
#[test]
fn test_presence_rows_from_the_live_loop() {
    let r = Rig::new();
    let mut seen = Vec::new();
    wait_for(10, "agent.seen rows for both sessions", || {
        seen = r
            .core
            .events(&Filter {
                kinds: vec![kinds::AGENT_SEEN.into()],
                limit: 100,
                ..Filter::default()
            })
            .unwrap();
        let has = |k: &str| {
            seen.iter()
                .any(|e| String::from_utf8_lossy(&e.payload).contains(k))
        };
        has(&r.key_a) && has(&r.key_b)
    });
    for e in &seen {
        let v: serde_json::Value = serde_json::from_slice(&e.payload).unwrap();
        assert_eq!(v["pid"], 0, "pid replicated: {v}");
        assert_eq!(v["transcript"], "", "transcript replicated: {v}");
        assert!(v.get("aiTitle").is_none(), "aiTitle replicated: {v}");
        assert_eq!(e.actor, "irori");
    }
}

/// Start writes device.started; a second Core on the same data dir is
/// refused while the first holds the lockfile; Close removes it.
#[test]
fn test_start_lockfile_and_already_running() {
    let r = Rig::new();
    let started = r
        .core
        .events(&Filter {
            kinds: vec![kinds::DEVICE_STARTED.into()],
            limit: 10,
            ..Filter::default()
        })
        .unwrap();
    assert_eq!(started.len(), 1);
    let want = format!(
        r#"{{"headless":true,"name":{},"os":{},"version":"0.2.0-test"}}"#,
        serde_json::to_string(&r.core.device().name).unwrap(),
        serde_json::to_string(&r.core.device().os).unwrap()
    );
    assert_eq!(String::from_utf8_lossy(&started[0].payload), want);

    r.core.write_lockfile(4321).unwrap();
    let lf = lockfile::read(&r.data).unwrap();
    assert!(
        lf.port == 4321
            && lf.token == r.core.token()
            && lf.device_id == r.core.device_id()
            && lf.headless
            && lf.version == "0.2.0-test"
            && lf.pid == lockfile::own_pid(),
        "{lf:?}"
    );
    // Another process's lockfile (a live pid that isn't ours) refuses.
    let sleeper = Sleeper::start();
    let other = lockfile::Lockfile {
        pid: sleeper.pid(),
        ..lf.clone()
    };
    lockfile::write(&r.data, &other).unwrap();
    match Core::start(options(&r.data, &r.claude)) {
        Err(StartError::AlreadyRunning { pid }) => assert_eq!(pid, other.pid),
        Err(e) => panic!("want AlreadyRunning, got {e}"),
        Ok(_) => panic!("a second Core started"),
    }
    // Close leaves another process's lockfile alone, and removes its own.
    lockfile::write(&r.data, &lf).unwrap();
    r.core.close().unwrap();
    assert!(matches!(
        lockfile::read(&r.data),
        Err(lockfile::Error::NotRunning)
    ));
    drop(sleeper);
}

/// A live process that is not this one, killed when dropped.
struct Sleeper(std::process::Child);

impl Sleeper {
    fn start() -> Sleeper {
        let mut cmd = if cfg!(windows) {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "ping -n 30 127.0.0.1 >NUL"]);
            c
        } else {
            let mut c = std::process::Command::new("sleep");
            c.arg("30");
            c
        };
        Sleeper(
            cmd.stdout(std::process::Stdio::null())
                .spawn()
                .expect("start a child"),
        )
    }
    fn pid(&self) -> i64 {
        i64::from(self.0.id())
    }
}

impl Drop for Sleeper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Reset: the next start after request_reset is a new device with its own
/// person; the old log moves aside and the settings stay. The CLI's reset
/// refuses while a Core holds the data dir.
#[test]
fn test_reset_makes_a_new_device() {
    let data = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let c = Core::start(options(data.path(), claude.path())).unwrap();
    c.write_lockfile(1).unwrap();
    let (old_id, old_person) = (c.device_id().to_string(), c.person().id);
    std::fs::write(data.path().join("settings.json"), b"{}").unwrap();
    c.request_reset().unwrap();
    assert!(
        reset::reset(data.path()).is_err(),
        "reset ran while a Core holds the data dir"
    );
    c.close().unwrap();
    drop(c);
    let c2 = Core::start(options(data.path(), claude.path())).unwrap();
    assert!(
        c2.device_id() != old_id && c2.person().id != old_person,
        "after reset: the same device"
    );
    assert!(
        !data.path().join(reset::RESET_FILE).exists(),
        "reset marker left"
    );
    assert!(
        data.path().join("settings.json").exists(),
        "settings did not stay"
    );
    let moved: Vec<_> = std::fs::read_dir(data.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("reset-"))
        .filter(|e| e.path().join("irori.db").exists())
        .collect();
    assert_eq!(moved.len(), 1, "old log not moved aside");
    c2.close().unwrap();
    drop(c2);
    // The CLI's reset, with nothing running: moved again.
    reset::reset(data.path()).unwrap();
    assert!(!data.path().join("device.json").exists());
}

/// coord.md §6 "expiry timer fires without a row": a lease running out
/// reaches subscribers by itself; nobody calls sweep, leases or expire,
/// and no row is involved. The timer then re-arms for the next lease.
#[test]
fn test_expiry_timer_fires_without_a_row() {
    let r = Rig::new();
    let co = r.core.coord();
    let q = co.subscribe();
    let alice = r.key_a.as_str();
    co.acquire(alice, "gpu/long", Duration::from_secs(3600), false)
        .unwrap();
    co.acquire(alice, "gpu/short", Duration::from_millis(100), false)
        .unwrap();
    let rows_before = r.core.store().vector().unwrap();
    let next = |q: &gocompat::sub::Queue<coord::Change>| {
        q.recv_timeout(Duration::from_secs(3))
            .expect("no change within 3s")
            .expect("subscription closed")
    };
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let c = next(&q);
        if c.kind == coord::LEASE_EXPIRED {
            got.push(c);
            break;
        }
    }
    assert_eq!(
        got,
        vec![coord::Change {
            kind: coord::LEASE_EXPIRED.into(),
            resource: "gpu/short".into(),
            holder: alice.into(),
            ..coord::Change::default()
        }]
    );
    // The change came from the timer, not a row: the leases left are the
    // long one, and the timer is armed for it.
    let at = co.next_expiry().expect("timer re-armed for gpu/long");
    let in_50m = capture::gotime::add(&presence::now_local(), Duration::from_secs(50 * 60));
    assert!(capture::gotime::after(&at, &in_50m), "timer at {at}");
    let _ = rows_before;
}

/// Start returns at once over a large transcript (backfill runs inside
/// capture's run), and the lockfile token is scrubbed from captured
/// prompts.
#[test]
fn test_start_with_large_transcript() {
    let data = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let sid = "cccccccc-3333-4333-8333-000000000003";
    let path = claude
        .path()
        .join("projects")
        .join("C--work")
        .join(format!("{sid}.jsonl"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let stamp = presence::now_local().utc().format_rfc3339_nano();
    let cwd = claude.path().to_string_lossy().into_owned();
    {
        use std::io::Write as _;
        let mut w =
            std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&path).unwrap());
        let pad = "x".repeat(8 << 10);
        // ~48 MiB, mostly api-request rows (skipped before decoding).
        for i in 0..6000 {
            let row =
                serde_json::json!({"type": "api-request-log", "timestamp": stamp, "body": pad});
            writeln!(w, "{row}").unwrap();
            if i % 1000 == 0 {
                let u = serde_json::json!({"type": "user", "uuid": format!("u{i}"), "timestamp": stamp,
                    "sessionId": sid, "cwd": cwd, "message": {"role": "user", "content": "warm up"}});
                let d = serde_json::json!({"type": "system", "subtype": "turn_duration", "timestamp": stamp, "sessionId": sid, "cwd": cwd});
                writeln!(w, "{u}\n{d}").unwrap();
            }
        }
    }
    let start = Instant::now();
    let c = Core::start(options(data.path(), claude.path())).unwrap();
    c.write_lockfile(1).unwrap();
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "Start took {:?}",
        start.elapsed()
    );
    let lf = lockfile::read(data.path()).unwrap();

    // A prompt carrying the token, after backfill.
    {
        use std::io::Write as _;
        let now = presence::now_local().utc().format_rfc3339_nano();
        let b = serde_json::json!({"type": "user", "uuid": "u-token", "promptId": "p-token", "timestamp": now,
            "sessionId": sid, "cwd": cwd, "message": {"role": "user", "content": format!("the token is {}", lf.token)}});
        let d = serde_json::json!({"type": "system", "subtype": "turn_duration", "timestamp": now, "sessionId": sid, "cwd": cwd});
        let mut g = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(g, "{b}\n{d}").unwrap();
    }
    let key = format!("{}/{sid}", c.device_id());
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let tl = c.capture().timeline(&capture::TimelineQuery {
            key: key.clone(),
            limit: 100,
            ..Default::default()
        });
        let found = tl
            .entries
            .iter()
            .filter_map(|e| e.turn.as_ref())
            .find(|t| t.row.turn_id == "p-token")
            .cloned();
        if let Some(t) = found {
            assert!(
                !t.row.turn.prompt.contains(&lf.token)
                    && t.row.turn.prompt.contains("[redacted:exact]"),
                "the token reached the log"
            );
            let mut st = c.capture().status();
            while st.backfilling && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
                st = c.capture().status();
            }
            assert!(!st.backfilling && st.files == 1, "status {st:?}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "turn never captured; {:?}",
            c.capture().status()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    c.close().unwrap();
}

/// mesh_test.go TestALoneCoreOpensNoNet, as far as M4 goes: a Core on its
/// own opens no socket; no lockfile until the API writes one.
#[test]
fn test_a_lone_core_writes_no_lockfile_by_itself() {
    let r = Rig::new();
    assert!(matches!(
        lockfile::read(&r.data),
        Err(lockfile::Error::NotRunning)
    ));
}

/// core.go's home server rule: switched on, a lone device's server binds
/// nothing (no other device yet), and a Core without one says so.
#[test]
fn test_home_server_waits_for_another_device() {
    let r = Rig::with(|o| {
        o.home = Some(std::sync::Arc::new(|hc: &mut home::Config| {
            hc.bind_ip = Some(std::net::Ipv4Addr::LOCALHOST.into());
        }));
    });
    assert!(r.core.can_serve());
    assert!(!r.core.home_server().on);
    r.core.set_home_server(true, false, "").unwrap();
    let st = r.core.home_server();
    assert!(st.on && !st.running, "{st:?}");
    assert_eq!(r.core.home().and_then(|h| h.relay_port()), None);

    let r = Rig::with(|o| o.no_home = true);
    assert!(!r.core.can_serve());
    assert_eq!(
        r.core.set_home_server(true, false, "").unwrap_err(),
        irori_core::NO_HOME_SERVER
    );
}

/// The change feed: a Board mutation and an inbox send each reach a feed
/// of their topic; a feed of one topic never sees another's.
#[test]
fn test_feed_carries_service_changes() {
    let r = Rig::new();
    let board = r.core.feed(&[Topic::Board]);
    let ib = r.core.feed(&[Topic::Inbox]);
    let task = r.core.coord().create_task(&r.key_a, nt("feed")).unwrap();
    let ev = board.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
    match ev {
        CoreEvent::Board(c) => assert_eq!(c.task_id, task.id),
        other => panic!("board feed got {other:?}"),
    }
    r.core
        .inbox()
        .send_with(
            &r.core.user_sender(),
            &Address::new(addr::AGENT, &r.key_b),
            "hello",
            SendOptions::default(),
        )
        .unwrap();
    let ev = ib.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
    assert!(matches!(ev, CoreEvent::Inbox(_)), "inbox feed got {ev:?}");
    while let Some(e) = board.try_recv() {
        assert!(matches!(e, CoreEvent::Board(_)), "board feed got {e:?}");
    }
}

/// A slow feed gets a Resync in place of the next event that fits.
#[test]
fn test_feed_slow_reader_resync() {
    let hub = irori_core::Hub::new();
    let f = hub.subscribe(&[]);
    for _ in 0..irori_core::events::FEED_BUFFER + 3 {
        hub.publish(CoreEvent::Roster);
    }
    for _ in 0..irori_core::events::FEED_BUFFER {
        assert_eq!(f.try_recv(), Some(CoreEvent::Roster));
    }
    assert_eq!(f.try_recv(), None);
    hub.publish(CoreEvent::Asks("a1".into()));
    assert_eq!(f.try_recv(), Some(CoreEvent::Resync));
    hub.publish(CoreEvent::Asks("a2".into()));
    assert_eq!(f.try_recv(), Some(CoreEvent::Asks("a2".into())));
    drop(f);
    assert!(hub.is_empty());
}

/// The window's pump: a burst becomes one emit carrying all of it, and the
/// next emit waits `every` after the previous one.
#[test]
fn test_coalesce() {
    use std::sync::{Arc, Mutex};
    let q = Arc::new(gocompat::sub::Queue::new(64));
    let stop = sys::cancel::Cancel::new();
    type Emits = Arc<Mutex<Vec<(Instant, Vec<i32>)>>>;
    let got: Emits = Arc::default();
    // The first burst is queued before the pump starts, and the second is
    // queued by the first emit itself, so each burst is whole when the
    // pump looks (pushing them from this thread raced the pump on a loaded
    // machine: a preempted push split a burst in two emits).
    for i in 0..5 {
        q.push(i);
    }
    let h = {
        let (q, stop, got) = (q.clone(), stop.clone(), got.clone());
        std::thread::spawn(move || {
            let q2 = q.clone();
            irori_core::events::coalesce(&q, Duration::from_millis(200), &stop, |b| {
                let mut g = got.lock().unwrap();
                g.push((Instant::now(), b));
                if g.len() == 1 {
                    q2.push(5);
                    q2.push(6);
                }
            })
        })
    };
    wait_for(10, "the second emit", || got.lock().unwrap().len() >= 2);
    {
        let g = got.lock().unwrap();
        assert_eq!(g[0].1, vec![0, 1, 2, 3, 4]);
        assert_eq!(g[1].1, vec![5, 6]);
        assert!(
            g[1].0 - g[0].0 >= Duration::from_millis(190),
            "emits {:?} apart",
            g[1].0 - g[0].0
        );
    }
    stop.cancel();
    h.join().unwrap();
}

/// The decision fold follows the log: a decision.recorded row appended
/// after start lists, and a superseded one hides unless all.
#[test]
fn test_decisions_follow_the_log() {
    let r = Rig::new();
    let at = presence::now_local().utc();
    let d1 = coord::decision::Decision {
        id: "d-1".into(),
        by: coord::decision::Party {
            key: "user".into(),
            kind: "user".into(),
            ..Default::default()
        },
        statement: "use sqlite".into(),
        trust: "user".into(),
        at,
        ..Default::default()
    };
    let mut d2 = d1.clone();
    d2.id = "d-2".into();
    d2.statement = "use sqlite in WAL mode".into();
    d2.supersedes = vec!["d-1".into()];
    for d in [&d1, &d2] {
        r.core
            .store()
            .append("user", capture::KIND_DECISION, d)
            .unwrap();
    }
    let f = coord::decision::Filter::default();
    wait_for(5, "both decisions folded", || {
        r.core
            .list_decisions(&coord::decision::Filter {
                all: true,
                ..Default::default()
            })
            .len()
            == 2
    });
    let live = r.core.list_decisions(&f);
    assert_eq!(live.len(), 1, "{live:?}");
    assert_eq!(live[0].listed.decision.id, "d-2");
    assert!(live[0].status.is_none(), "no anchor, no status");
    let all = r.core.list_decisions(&coord::decision::Filter {
        all: true,
        ..Default::default()
    });
    assert_eq!(all.len(), 2);
    let j = gocompat::json::marshal(&all[1]).unwrap();
    assert!(
        String::from_utf8_lossy(&j).contains(r#""supersededBy":["d-2"]"#),
        "{}",
        String::from_utf8_lossy(&j)
    );
}

/// Device colours: the default for this device, a pick as a row, and a
/// bad pick refused.
#[test]
fn test_device_colors() {
    let r = Rig::new();
    let me = r.core.device_id().to_string();
    let cs = r.core.device_colors();
    assert_eq!(cs.len(), 1);
    assert!(coord::devcolor::valid(&cs[&me]));
    assert!(r.core.set_device_color(&me, "mauve").is_err());
    assert!(r.core.set_device_color("not-mine", "teal").is_err());
    r.core.set_device_color(&me, "pink").unwrap();
    wait_for(5, "the pick folded", || {
        r.core.device_colors()[&me] == "pink"
    });
}

/// Agent faces (card #407): a checked face is kept by name, lower case,
/// and an empty one goes back to the default.
#[test]
fn test_agent_avatars() {
    let r = Rig::new();
    let v = serde_json::json!({"version":1,"id":"box","name":"Box","body":{"path":"M20 20 H80 V80 H20 Z"},"eyes":{"x":50,"y":46,"spacing":15,"scale":1}});
    let face = coord::avatar::check(&v).unwrap();
    r.core.set_agent_avatar("Libra", Some(&face)).unwrap();
    assert!(r.core.agent_avatars()["libra"].contains("\"id\":\"box\""));
    r.core.set_agent_avatar("libra", None).unwrap();
    assert!(!r.core.agent_avatars().contains_key("libra"));
}

/// Device names (card #184): this device starts as its computer's name;
/// a rename is a `device.name` row that wins at once, is cleaned to one
/// short line, tells the window (devices:changed), and "" goes back.
#[test]
fn test_device_names() {
    let r = Rig::new();
    let me = r.core.device_id().to_string();
    let host = r.core.device().name.clone();
    assert_eq!(r.core.device_label(&me), host);
    assert!(r.core.set_device_name("not-mine", "PC").is_err());

    let feed = r.core.feed(&[irori_core::Topic::Devices]);
    assert_eq!(
        r.core.set_device_name(&me, "  Mac\nBook  ").unwrap(),
        "Mac Book"
    );
    assert_eq!(r.core.device_label(&me), "Mac Book", "folded at once");
    assert_eq!(r.core.device_labels()[&me], "Mac Book");
    wait_for(5, "devices:changed", || {
        matches!(feed.try_recv(), Some(CoreEvent::Devices))
    });
    let rows = r
        .core
        .store()
        .list(&store::Filter {
            kinds: vec![coord::devname::KIND.into()],
            ..store::Filter::default()
        })
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].actor, "user");

    r.core.set_device_name(&me, "").unwrap();
    assert_eq!(r.core.device_label(&me), host, "empty goes back");
}

/// The inbox's sweep and Follow run on their own: a send is folded, and a
/// sweep call through the Core works between the timed ones.
#[test]
fn test_sweeps_run() {
    let r = Rig::new();
    r.core.sweep_inbox().unwrap();
    r.core.sweep_coord().unwrap();
    assert!(r.core.ask_agents().iter().any(|a| a.key == r.key_b));
}

/// Close stops every loop within the close wait, and is idempotent.
#[test]
fn test_close_is_prompt_and_idempotent() {
    let r = Rig::new();
    let t = Instant::now();
    r.core.close().unwrap();
    assert!(
        t.elapsed() < Duration::from_secs(3),
        "close took {:?}",
        t.elapsed()
    );
    r.core.close().unwrap();
    assert!(matches!(
        r.core.coord().subscribe().try_recv(),
        TryRecv::Empty | TryRecv::Closed
    ));
}

/// A resync refolds capture's decisions from the log whole: a decision
/// the fold holds but the log no longer does (a revoke's cut) leaves it.
#[test]
fn test_decisions_refold_on_resync() {
    let r = Rig::new();
    let d = |id: &str| decision(id);
    r.core
        .store()
        .append("user", capture::KIND_DECISION, &d("d-1"))
        .unwrap();
    let all = coord::decision::Filter {
        all: true,
        ..Default::default()
    };
    let ids = |v: Vec<irori_core::DecisionView>| -> Vec<String> {
        v.into_iter().map(|l| l.listed.decision.id).collect()
    };
    wait_for(5, "d-1 folded", || r.core.list_decisions(&all).len() == 1);
    // Folded but not in the log: a row the follow loop saw, then cut.
    r.core
        .capture()
        .follow_decision(&store::Event {
            device_id: "gone-device".into(),
            seq: 1,
            kind: capture::KIND_DECISION.into(),
            payload: gocompat::json::marshal(&d("d-gone")).unwrap(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(r.core.list_decisions(&all).len(), 2);
    r.core.capture().refold_decisions().unwrap();
    assert_eq!(ids(r.core.list_decisions(&all)), vec!["d-1"]);
    assert_eq!(r.core.capture().decision_payloads().len(), 1);
}

/// This device's own decision is folded once although the follow loop
/// sees its row too.
#[test]
fn test_own_decision_folds_once() {
    let r = Rig::new();
    r.core
        .capture()
        .emit_row("user", capture::KIND_DECISION, &decision("d-own"))
        .unwrap();
    // A marker row after it: once it folds, the loop has seen d-own.
    r.core
        .store()
        .append("user", capture::KIND_DECISION, &decision("d-mark"))
        .unwrap();
    let all = coord::decision::Filter {
        all: true,
        ..Default::default()
    };
    wait_for(5, "the marker folded", || {
        r.core.list_decisions(&all).len() == 2
    });
    assert_eq!(r.core.capture().decision_payloads().len(), 2);
}

fn decision(id: &str) -> coord::decision::Decision {
    coord::decision::Decision {
        id: id.into(),
        by: coord::decision::Party {
            key: "user".into(),
            kind: "user".into(),
            ..Default::default()
        },
        statement: format!("statement {id}"),
        trust: "user".into(),
        at: presence::now_local().utc(),
        ..Default::default()
    }
}

/// The non-attach half of names_test.go
/// TestUnnamedSessionGetsANameAndARenameWins: an unnamed session gets an
/// Irori name on its card (presence → namer → mesh roster → presence, with
/// no deadlock), and a rename in Claude reaches the log as an agent.status
/// row carrying the new source.
#[test]
fn test_unnamed_session_gets_a_name_and_a_rename_wins() {
    let r = Rig::new();
    let sessions = r.claude.join("sessions");
    let cwd = tempfile::tempdir().unwrap();
    write_session(&sessions, "900001.json", SESS_A, "", cwd.path());
    let mut named = String::new();
    wait_for(10, "an Irori name on the card", || {
        r.core
            .local_agents()
            .into_iter()
            .find(|c| c.key == r.key_a && c.name_source == presence::SOURCE_IRORI)
            .map(|c| named = c.name)
            .is_some()
    });
    assert!(!named.is_empty());
    assert_eq!(r.core.names().assigned(SESS_A), named);

    write_session(&sessions, "900001.json", SESS_A, "Bob", cwd.path());
    wait_for(10, "an agent.status row with the rename", || {
        r.core
            .events(&Filter {
                kinds: vec![kinds::AGENT_STATUS.into()],
                limit: 500,
                ..Filter::default()
            })
            .unwrap()
            .iter()
            .any(|e| {
                let p = String::from_utf8_lossy(&e.payload);
                p.contains(r#""name":"Bob""#) && p.contains(r#""nameSource":"user""#)
            })
    });
}

/// Card #87: a branch wait fires when the core's sweep sees the branch's
/// ref move, and wakes the waiter once with the new tip.
#[test]
fn test_branch_wait_wakes_once() {
    let r = Rig::new();
    let co = r.core.coord();
    let repo = tempfile::tempdir().unwrap();
    let git = repo.path().join(".git");
    std::fs::create_dir_all(git.join("refs/heads")).unwrap();
    let a = "1".repeat(40);
    let b = "2".repeat(40);
    std::fs::write(git.join("refs/heads/main"), format!("{a}\n")).unwrap();
    let root = repo.path().to_string_lossy().into_owned();
    co.create_wait(
        &r.key_b,
        coord::WaitSpec {
            branch: "main".into(),
            root: root.clone(),
            refs: capture::gitlink::branch_tips(&root, "main").unwrap(),
            ..coord::WaitSpec::default()
        },
    )
    .unwrap();
    r.core.sweep_coord().unwrap();
    assert!(r.notices_to(&r.key_b).is_empty(), "nothing moved yet");

    std::fs::write(git.join("refs/heads/main"), format!("{b}\n")).unwrap();
    r.core.sweep_coord().unwrap();
    r.core.sweep_coord().unwrap();
    let ns: Vec<_> = r
        .notices_to(&r.key_b)
        .into_iter()
        .filter(|d| d.message.reason == "wait")
        .collect();
    assert!(
        ns.len() == 1
            && ns[0].mode == mode::WAKE
            && ns[0]
                .message
                .text
                .contains("main moved from 1111111 to 2222222"),
        "{ns:?}"
    );
    assert!(co.branch_waits().is_empty());
}

/// Talk (card #389): the person makes a channel and posts, mentions alpha
/// (woken with a mention notice, and now in the channel), DMs beta (woken
/// with a message whose id is the post's), and reads the channel; every
/// write tells the window (talk:changed). After a restart the Core folds
/// the same channels, posts, members and read cursor from the log.
#[test]
fn test_talk_survives_a_restart() {
    let r = Rig::new();
    let me = r.core.user_sender();
    let feed = r.core.feed(&[Topic::Talk]);
    let ch = r
        .core
        .talk_channel_create(&me.key, "general", "the room")
        .unwrap();
    wait_for(5, "talk:changed", || {
        matches!(feed.try_recv(), Some(CoreEvent::Talk))
    });
    let p1 = r
        .core
        .talk_post(&me, &ch.id, "@alpha hello room", &[], true, None)
        .unwrap();
    assert_eq!(p1.post.mentions, vec![r.key_a.clone()]);
    assert!(
        r.core.talk().is_member(&ch.id, &r.key_a),
        "mentioned: joined"
    );
    let ns = r.notices_to(&r.key_a);
    assert_eq!(ns.len(), 1, "{ns:?}");
    assert_eq!(ns[0].message.reason, coord::NOTICE_MENTIONED);
    assert!(
        r.core
            .talk_post(&me, &ch.id, "   ", &[], true, None)
            .is_err()
    );
    assert!(
        r.core
            .talk_post(&me, "ch_nope", "x", &[], true, None)
            .is_err()
    );

    let dm = r
        .core
        .talk_open_dm(
            &coord::talk::Member::of(&me.key),
            &coord::talk::Member::of(&r.key_b),
            &me.key,
        )
        .unwrap();
    let p2 = r
        .core
        .talk_post(&me, &dm, "can you look at #3?", &[], false, None)
        .unwrap();
    let sent = p2.sent.expect("a DM to an agent is a message");
    assert_eq!(sent.message_id, p2.post.post_id);
    assert_eq!(sent.mode, mode::WAKE);
    r.core
        .talk_mark_read(&ch.id, &p1.post.post_id, &me.key)
        .unwrap();

    let before = r.core.talk_conversations(&me.key).unwrap();
    let posts_ch = r.core.talk_posts(&me.key, &ch.id, None, 50).unwrap();
    let posts_dm = r.core.talk_posts(&me.key, &dm, None, 50).unwrap();
    assert_eq!(posts_ch.len(), 1);
    assert_eq!(posts_dm.len(), 1, "the message shows once: {posts_dm:?}");
    assert!(!posts_dm[0].agents_now);

    let (data, claude) = (r.data.clone(), r.claude.clone());
    r.core.close().unwrap();
    let c = Core::start(common::options(&data, &claude)).unwrap();
    assert_eq!(c.talk_posts(&me.key, &ch.id, None, 50).unwrap(), posts_ch);
    assert_eq!(c.talk_posts(&me.key, &dm, None, 50).unwrap(), posts_dm);
    assert_eq!(
        c.talk().cursor(&me.key, &ch.id),
        Some(p1.post.post_id.clone())
    );
    let after = c.talk_conversations(&me.key).unwrap();
    let pick = |cs: &[tools::talk::Conversation], id: &str| {
        cs.iter().find(|x| x.conv_id == id).cloned().unwrap()
    };
    for id in [&ch.id, &dm] {
        let (a, b) = (pick(&before, id), pick(&after, id));
        assert_eq!(a.members, b.members, "{id}");
        assert_eq!(
            (a.unread, a.name.clone()),
            (b.unread, b.name.clone()),
            "{id}"
        );
    }
    assert_eq!(pick(&after, &ch.id).name, "general");
    c.close().unwrap();
    drop(r);
}

/// Card #468: a message tells the window Talk changed only when it can
/// show as a DM. Board notices (most of what the inbox carries while
/// agents work) and stops don't, so they no longer make the window
/// rebuild Talk's list from the whole inbox; an agent's message still does.
#[test]
fn test_only_messages_that_show_as_dms_change_talk() {
    let r = Rig::new();
    let feed = r.core.feed(&[Topic::Talk]);
    let ib = r.core.inbox();
    let a = Sender::new(&r.key_a, "alpha", "agent");
    let to_a = Address::new(addr::AGENT, &r.key_a);
    let to_b = Address::new(addr::AGENT, &r.key_b);
    const NOTICES: usize = 20;
    for i in 0..NOTICES {
        ib.send_with(
            &inbox::system_sender(),
            &to_a,
            &format!("Board: #{i} was handed to you."),
            SendOptions {
                notice: true,
                ..SendOptions::default()
            },
        )
        .unwrap();
    }
    // The person's stop to alpha.
    ib.send_with(
        &r.core.user_sender(),
        &to_a,
        "stop",
        SendOptions {
            mode: mode::STOP.into(),
            ..SendOptions::default()
        },
    )
    .unwrap();
    // The agent's message is folded after them, so its event comes last.
    ib.send(&a, &to_b, "can you look at #3?", "").unwrap();
    let mut talk = 0;
    wait_for(5, "talk:changed for the agent's message", || {
        while let Some(e) = feed.try_recv() {
            if matches!(e, CoreEvent::Talk) {
                talk += 1;
            }
        }
        talk > 0
    });
    std::thread::sleep(Duration::from_millis(200));
    while let Some(e) = feed.try_recv() {
        if matches!(e, CoreEvent::Talk) {
            talk += 1;
        }
    }
    eprintln!("card #468: {NOTICES} notices, a stop and one agent message: {talk} Talk events");
    assert_eq!(talk, 1, "only the agent's message changes Talk");
    let convs = r
        .core
        .talk_conversations(&r.core.user_sender().key)
        .unwrap();
    assert!(
        convs.iter().any(|c| c.kind == "dm" && c.read_only),
        "the agents' DM shows: {convs:?}"
    );
}

/// Card #468's numbers: what one talk:changed costs the window, its
/// talk_conversations read, which mirrors agents' messages into DMs from
/// the whole inbox. 2,000 notices and 401 messages that show as DMs.
/// `cargo test -p irori-core --test core -- --ignored --nocapture talk_conversations_timing`
#[test]
#[ignore]
fn talk_conversations_timing() {
    let r = Rig::new();
    let ib = r.core.inbox();
    let a = Sender::new(&r.key_a, "alpha", "agent");
    let to_a = Address::new(addr::AGENT, &r.key_a);
    let to_b = Address::new(addr::AGENT, &r.key_b);
    for i in 0..2000 {
        ib.send_with(
            &inbox::system_sender(),
            &to_a,
            &format!("Board: #{i} was handed to you."),
            SendOptions {
                notice: true,
                ..SendOptions::default()
            },
        )
        .unwrap();
    }
    // The person's messages to the agents (agents' to each other are
    // rate-limited); each shows as a DM too.
    let me = r.core.user_sender();
    for i in 0..400 {
        let to = if i % 2 == 0 { &to_a } else { &to_b };
        ib.send(&me, to, &format!("can you look at #{i}?"), "")
            .unwrap();
    }
    ib.send(&a, &to_b, "and you?", "").unwrap();
    let me = me.key;
    let mut best = Duration::MAX;
    for _ in 0..20 {
        let t = Instant::now();
        std::hint::black_box(r.core.talk_conversations(&me).unwrap());
        best = best.min(t.elapsed());
    }
    eprintln!(
        "card #468: talk_conversations over {} deliveries: {best:?}",
        ib.list(usize::MAX).len()
    );
}
