//! Card #455 through two real Cores of one person on loopback iroh: A (the
//! PC, where the cards are made) and B (the Mac, the only one with a
//! checkout of the cards' repo). Each has a fake spawner in place of
//! `claude --bg` that writes a live session file the way Claude Code does
//! (and End removes it). A's sweep asks B by name to start a card's agent
//! and B starts it; the person's End on A ends it on B; a request B's log
//! held when it started is never started there; a repo B doesn't trust
//! fails there, and the person's "trust it" from A is asked in B's window,
//! while a trust request B wrote itself or one naming someone else is
//! ignored.
//!
//! A is never restarted: capture replays every device's repo.seen, and B's
//! checkout is on the same disk, so a restarted A would find B's checkout
//! as its own.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::mesh::{Dev, Edit, invited, person, person_with, restart};
use common::{wait_for, write_session};
use coord::{NewTask, Task};
use irori_core::Options;
use irori_core::spawn::{NOT_TRUSTED_PREFIX, SpawnReq, SpawnSetting, SpawnStarted, Spawner};
use store::{Filter, kinds};

const REPO: &str = "remote:github.com/stephen/api";
const SESS_A: &str = "aaaaaaaa-5555-4555-8555-00000000000a";
const SESS_B: &str = "bbbbbbbb-5555-4555-8555-00000000000b";
const WAIT: u64 = 90;

#[derive(Default)]
struct Fake {
    started: Mutex<Vec<SpawnReq>>,
    ended: Mutex<Vec<String>>,
    untrusted: AtomicBool,
}

impl Fake {
    fn started(&self) -> usize {
        self.started.lock().unwrap().len()
    }
    fn ended(&self) -> Vec<String> {
        self.ended.lock().unwrap().clone()
    }
}

/// A spawner whose agent is session `sess` in `sessions`, named as asked.
fn spawner(f: &Arc<Fake>, sessions: PathBuf, sess: &'static str) -> Spawner {
    let (a, b, c) = (f.clone(), f.clone(), f.clone());
    let s2 = sessions.clone();
    Spawner {
        spawn: Arc::new(move |r: &SpawnReq| {
            a.started.lock().unwrap().push(r.clone());
            write_session(
                &sessions,
                "900003.json",
                sess,
                &r.name,
                Path::new(&r.folder),
            );
            Ok(SpawnStarted {
                name: r.name.clone(),
                prefix: sess[..8].into(),
            })
        }),
        end: Arc::new(move |s: &str| {
            b.ended.lock().unwrap().push(s.to_string());
            let _ = std::fs::remove_file(s2.join("900003.json"));
            Ok(String::new())
        }),
        trusted: Arc::new(move |_| !c.untrusted.load(Ordering::SeqCst)),
        list: None,
    }
}

/// Options whose clock runs `skew` seconds ahead.
fn skewed(skew: Arc<AtomicI64>) -> Edit {
    Arc::new(move |o: &mut Options| {
        let sk = skew.clone();
        o.now = Some(Arc::new(move || {
            let d = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap();
            gocompat::time::GoTime::unix(
                d.as_secs() as i64 + sk.load(Ordering::SeqCst),
                i64::from(d.subsec_nanos()),
            )
        }));
    })
}

struct Two {
    a: Dev,
    b: Dev,
    /// A's and B's clocks, ahead by this many seconds (kept across a
    /// restart).
    skew_a: Arc<AtomicI64>,
    skew_b: Arc<AtomicI64>,
    fa: Arc<Fake>,
    fb: Arc<Fake>,
    _checkout: tempfile::TempDir,
}

impl Two {
    /// A and B paired; B has seen a checkout of REPO (A hasn't).
    fn new() -> Two {
        let skew_a = Arc::new(AtomicI64::new(0));
        let skew_b = Arc::new(AtomicI64::new(0));
        let sks = [skew_a.clone(), skew_b.clone()];
        let mut devs = person_with("stephen", 2, |i| Some(skewed(sks[i].clone())));
        let b = devs.pop().unwrap();
        let a = devs.pop().unwrap();
        let checkout = tempfile::tempdir().unwrap();
        b.core
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
        Two {
            a,
            b,
            skew_a,
            skew_b,
            fa: Arc::new(Fake::default()),
            fb: Arc::new(Fake::default()),
            _checkout: checkout,
        }
    }

    fn a_id(&self) -> String {
        self.a.core.device_id().to_string()
    }
    fn b_id(&self) -> String {
        self.b.core.device_id().to_string()
    }

    fn spawner_a(&self) {
        self.a.core.set_spawner(Some(spawner(
            &self.fa,
            self.a.claude.join("sessions"),
            SESS_A,
        )));
    }
    fn spawner_b(&self) {
        self.b.core.set_spawner(Some(spawner(
            &self.fb,
            self.b.claude.join("sessions"),
            SESS_B,
        )));
    }

    /// A knows what B can start agents in and sees B online: only then
    /// may A's choice name B (otherwise it would try on A, which has no
    /// copy, and wait out the retry).
    fn a_sees_b_host(&self, trusted: bool) {
        let b = self.b_id();
        wait_for(WAIT, "A has B's spawn.host with the repo", || {
            self.a
                .core
                .spawn_hosts()
                .get(&b)
                .and_then(|h| h.repo(REPO).map(|r| r.trusted))
                == Some(trusted)
        });
        wait_for(WAIT, "A sees B online", || {
            self.a
                .core
                .mesh()
                .devices(false)
                .iter()
                .any(|r| r.device_id == b && r.state != "offline")
        });
        assert!(
            self.a
                .core
                .spawn_hosts()
                .get(&self.a_id())
                .is_none_or(|h| h.repo(REPO).is_none()),
            "A published no checkout of the repo"
        );
    }

    fn spawning_on_a(&self) {
        self.a
            .core
            .set_spawn_setting(SpawnSetting {
                on: true,
                ..SpawnSetting::default()
            })
            .unwrap();
    }

    fn card(&self, title: &str) -> Task {
        self.a
            .core
            .coord()
            .create_task(
                &format!("user:{}", self.a_id()),
                NewTask {
                    title: title.into(),
                    repo_key: REPO.into(),
                    ..NewTask::default()
                },
            )
            .unwrap()
    }

    fn on_a(&self, id: &str) -> Task {
        self.a.core.coord().get_task(id).unwrap()
    }

    /// A few sweeps on both, then a moment for them to run.
    fn kick(&self) {
        for d in [&self.a, &self.b] {
            for _ in 0..3 {
                d.core.set_spawn_cap(d.core.spawn_cap()).unwrap();
            }
        }
        // More than one coord sweep interval (5 s).
        std::thread::sleep(Duration::from_secs(7));
    }

    fn close(self) {
        self.a.core.close().unwrap();
        self.b.core.close().unwrap();
    }
}

fn rows(d: &Dev, kind: &str) -> Vec<store::Event> {
    d.core
        .store()
        .list(&Filter {
            kinds: vec![kind.into()],
            ..Filter::default()
        })
        .unwrap()
}

/// The stand-in for the live PC-to-Mac spawn: a card made on A for a repo
/// only B has is started on B, once, and is bound to B's session; then the
/// person's End on A ends it on B.
#[test]
fn a_card_made_on_the_pc_starts_on_the_mac_and_ends_from_the_pc() {
    let t2 = Two::new();
    let (a_id, b_id) = (t2.a_id(), t2.b_id());
    t2.spawner_b();
    t2.spawner_a();
    t2.a_sees_b_host(true);

    let t = t2.card("Fix the API's pagination");
    t2.spawning_on_a();
    wait_for(WAIT, "A's request names B", || {
        t2.on_a(&t.id)
            .spawn_request()
            .is_some_and(|s| s.device == b_id)
    });
    let spawned = format!("{b_id}/{SESS_B}");
    wait_for(WAIT, "the agent on B bound to the card", || {
        t2.on_a(&t.id).holders.contains(&spawned)
    });
    // The request is A's, for B; the bind is B's.
    let spawn_rows = rows(&t2.a, kinds::TASK_SPAWN);
    assert!(
        spawn_rows.iter().any(|e| e.device_id == a_id),
        "A wrote the request: {:?}",
        spawn_rows
            .iter()
            .map(|e| e.device_id.clone())
            .collect::<Vec<_>>()
    );
    t2.kick();
    assert_eq!(t2.fb.started(), 1, "B started it once");
    assert_eq!(t2.fa.started(), 0, "A started nothing");
    let req = t2.fb.started.lock().unwrap()[0].clone();
    assert_eq!(req.task_id, t.id);
    assert_eq!(
        Path::new(&req.folder),
        t2._checkout.path(),
        "B starts it in its own checkout"
    );
    let s = t2.on_a(&t.id).spawn_request().cloned().unwrap();
    assert_eq!(s.generation, 1, "not asked again: {s:?}");
    assert_eq!(s.device, b_id);
    assert_eq!(s.name, req.name);
    assert_eq!(s.key, spawned);
    assert_eq!(t2.b.core.coord().spawned_card(&spawned), Some(t.id.clone()));

    // The person's End on A: B asks its agent for its hand-off, and ends
    // it after the hand-off wait. B's clock runs back for the ask, so the
    // wait is over when it is put right (a clock ahead by as much would
    // have A's store refuse B's rows: store::DEFAULT_MAX_AHEAD).
    t2.skew_b.store(
        -(irori_core::spawn::HANDOFF_WAIT.as_secs() as i64 + 10),
        Ordering::SeqCst,
    );
    t2.a.core.request_end(&spawned).unwrap();
    wait_for(WAIT, "B asks its agent for its hand-off", || {
        t2.b.core
            .inbox()
            .for_agent(&spawned, true, 0)
            .iter()
            .any(|d| d.message.text.contains("Irori is ending you"))
    });
    assert!(t2.fb.ended().is_empty(), "ended before its hand-off wait");
    t2.skew_b.store(0, Ordering::SeqCst);
    wait_for(WAIT, "B ended its agent", || {
        t2.fb.ended().contains(&SESS_B.to_string())
    });
    wait_for(WAIT, "the agent off the card on A", || {
        !t2.on_a(&t.id).holders.contains(&spawned)
    });
    let gone = rows(&t2.b, kinds::AGENT_GONE);
    assert!(
        gone.iter()
            .any(|e| e.device_id == b_id && String::from_utf8_lossy(&e.payload).contains(SESS_B)),
        "B wrote agent.gone for it"
    );
    assert!(
        t2.fa.ended().is_empty(),
        "A ended nothing: {:?}",
        t2.fa.ended()
    );
    t2.close();
}

/// A request B's log already held when B's Core started (B was away and
/// comes back) is never started there: whether it went stale is A's call
/// (no double start after a restart, no clock-skew miss). The person's
/// Start from A asks B again, and that newer request, which reached B after
/// it started, starts once.
#[test]
fn a_request_held_when_the_mac_comes_back_starts_nothing_until_asked_again() {
    let t2 = Two::new();
    let b_id = t2.b_id();
    t2.spawner_b();
    t2.spawner_a();
    t2.a_sees_b_host(true);
    // B stops starting agents (its window away).
    t2.b.core.set_spawner(None);

    let t = t2.card("Rename the API's handlers");
    // A's request is written already past its grace by A's clock, run
    // back for it (a clock ahead by as much would have B's store refuse
    // A's rows: store::DEFAULT_MAX_AHEAD).
    t2.skew_a.store(
        -(coord::SPAWN_GRACE.as_secs() as i64 + 10),
        Ordering::SeqCst,
    );
    t2.spawning_on_a();
    wait_for(WAIT, "A's request names B", || {
        t2.on_a(&t.id)
            .spawn_request()
            .is_some_and(|s| s.device == b_id && s.pending())
    });
    t2.skew_a.store(0, Ordering::SeqCst);
    wait_for(WAIT, "B has the request", || {
        t2.b.core
            .coord()
            .get_task(&t.id)
            .and_then(|t| t.spawn_request().cloned())
            .is_some_and(|s| s.device == b_id && s.pending())
    });

    // B comes back, its log holding the request.
    let Two {
        a,
        b,
        skew_a,
        skew_b,
        fa,
        fb,
        _checkout,
    } = t2;
    let b = restart(b);
    let t2 = Two {
        a,
        b,
        skew_a,
        skew_b,
        fa,
        fb,
        _checkout,
    };
    t2.spawner_b();
    t2.kick();
    t2.kick();
    assert_eq!(t2.fb.started(), 0, "B started a request it held at start");
    assert_eq!(t2.fa.started(), 0, "A started nothing");
    let s = t2.on_a(&t.id).spawn_request().cloned().unwrap();
    assert_eq!(s.generation, 1, "{s:?}");

    // A's request is stale; B, asked lately, isn't asked again before the
    // retry, and A (no copy) never takes it itself. The person's "Start an
    // agent" asks B again, and B starts that newer request.
    t2.kick();
    let s = t2.on_a(&t.id).spawn_request().cloned().unwrap();
    assert_eq!(
        (s.generation, s.device.as_str()),
        (1, b_id.as_str()),
        "{s:?}"
    );
    t2.a.core.start_agent(&t.id, "").unwrap();
    wait_for(WAIT, "A asks B again", || {
        t2.on_a(&t.id)
            .spawn_request()
            .is_some_and(|s| s.generation == 2 && s.device == b_id)
    });
    let spawned = format!("{b_id}/{SESS_B}");
    wait_for(WAIT, "the newer request's agent bound on B", || {
        t2.on_a(&t.id).holders.contains(&spawned)
    });
    t2.kick();
    let reqs = t2.fb.started.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1, "{reqs:?}");
    let s = t2.on_a(&t.id).spawn_request().cloned().unwrap();
    assert_eq!((s.generation, s.name.as_str()), (2, reqs[0].name.as_str()));
    assert_eq!(t2.fa.started(), 0, "A started nothing");
    t2.close();
}

/// B doesn't trust its checkout: B fails the request as not trusted; the
/// person's Trust from A is asked in B's window, naming A. A trust request
/// B wrote about itself, one whose `by` isn't its writer, and one from an
/// agent are ignored.
#[test]
fn a_repo_the_mac_doesnt_trust_is_asked_there_only_by_the_person() {
    let t2 = Two::new();
    let (a_id, b_id) = (t2.a_id(), t2.b_id());
    t2.fb.untrusted.store(true, Ordering::SeqCst);
    t2.spawner_b();
    t2.spawner_a();
    t2.a_sees_b_host(false);

    let cards: Vec<Task> = ["valid", "from B itself", "by someone else", "by an agent"]
        .iter()
        .map(|w| t2.card(&format!("Trust case: {w}")))
        .collect();
    t2.spawning_on_a();
    for c in &cards {
        wait_for(WAIT, "B fails the request as not trusted", || {
            t2.on_a(&c.id).spawn_request().is_some_and(|s| {
                s.op == coord::SPAWN_FAILED
                    && s.device == b_id
                    && s.reason.starts_with(NOT_TRUSTED_PREFIX)
            })
        });
    }
    assert_eq!(t2.fb.started(), 0);
    assert_eq!(t2.fa.started(), 0);
    let failed = |c: &Task| t2.on_a(&c.id).spawn_request().cloned().unwrap();
    let row = |c: &Task, by: &str| {
        let s = failed(c);
        serde_json::json!({
            "taskId": c.id, "gen": s.generation, "device": b_id,
            "repoKey": REPO, "folder": "x", "by": by,
        })
    };
    // B about itself, as the person on B.
    t2.b.core
        .store()
        .append(
            "user",
            kinds::TRUST_REQUESTED,
            &row(&cards[1], &format!("user:{b_id}")),
        )
        .unwrap();
    // A, naming B's person key as `by`.
    t2.a.core
        .store()
        .append(
            "user",
            kinds::TRUST_REQUESTED,
            &row(&cards[2], &format!("user:{b_id}")),
        )
        .unwrap();
    // An agent on A.
    t2.a.core
        .store()
        .append(
            &t2.a.agent,
            kinds::TRUST_REQUESTED,
            &row(&cards[3], &format!("user:{a_id}")),
        )
        .unwrap();
    wait_for(WAIT, "B has the three trust rows", || {
        rows(&t2.b, kinds::TRUST_REQUESTED).len() >= 3
    });
    t2.kick();
    t2.kick();
    for c in &cards[1..] {
        assert_eq!(
            t2.b.core.trust_asked_by(&c.id),
            None,
            "{} was asked",
            c.title
        );
    }

    // On B itself the person trusts it there.
    assert!(t2.b.core.request_trust(&cards[0].id).is_err());
    // From A: B asks, naming A.
    assert_eq!(t2.a.core.request_trust(&cards[0].id), Ok(b_id.clone()));
    wait_for(WAIT, "B asks the person, naming A", || {
        t2.b.core.trust_asked_by(&cards[0].id) == Some(a_id.clone())
    });
    assert_eq!(t2.a.core.trust_asked_by(&cards[0].id), None);
    for c in &cards[1..] {
        assert_eq!(t2.b.core.trust_asked_by(&c.id), None, "{}", c.title);
    }
    // Nothing started anywhere, and A didn't ask B again.
    t2.kick();
    assert_eq!(t2.fb.started(), 0);
    assert_eq!(t2.fa.started(), 0);
    for c in &cards {
        assert_eq!(failed(c).generation, 1, "{}", c.title);
    }
    t2.close();
}

/// Shared boards (§9 "Shared boards"): Stephen's own card on a board whose
/// switch he turned on gets one agent on his device, held on the board as
/// an `agent:<cardId>` that Bob's device sees marked as started by Irori,
/// with no bypass. Bob's card on the same board, even with a request
/// naming Stephen's device, starts nothing there.
#[test]
fn a_shared_board_card_gets_its_makers_agent_and_never_another_members() {
    let mut st = person("stephen", 1);
    let mut bo = person("bob", 1);
    let (s, b) = (st.remove(0), bo.remove(0));
    let board = invited(&s, &b);
    let s_dev = s.core.device_id().to_string();
    let checkout = tempfile::tempdir().unwrap();
    s.core
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
    let fs = Arc::new(Fake::default());
    s.core
        .set_spawner(Some(spawner(&fs, s.claude.join("sessions"), SESS_A)));
    s.core
        .set_spawn_setting(SpawnSetting {
            board_id: board.clone(),
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    let (sco, bco) = (
        s.core.board_coord(&board).unwrap(),
        b.core.board_coord(&board).unwrap(),
    );
    let kick = || {
        for _ in 0..3 {
            s.core.set_spawn_cap(s.core.spawn_cap()).unwrap();
        }
        std::thread::sleep(Duration::from_secs(7));
    };

    // Bob's card first, with a request of his naming Stephen's device.
    let theirs = bco
        .create_task(
            &b.person_actor(),
            NewTask {
                title: "Bob's card".into(),
                repo_key: REPO.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    let asked = bco
        .request_spawn(
            &b.person_actor(),
            coord::SpawnAsk {
                task_id: theirs.id.clone(),
                device: s_dev.clone(),
                repo_key: REPO.into(),
                permission_mode: "acceptEdits".into(),
                name: "heron".into(),
                ..coord::SpawnAsk::default()
            },
        )
        .unwrap();
    assert!(asked.is_some(), "Bob's request was written");
    wait_for(WAIT, "Stephen's device has Bob's card", || {
        sco.get_task(&theirs.id).is_some()
    });
    kick();
    kick();
    assert_eq!(
        fs.started(),
        0,
        "Bob's card started on Stephen's device (Bob's request written: {asked:?})"
    );

    // Stephen's own card.
    let mine = sco
        .create_task(
            &s.person_actor(),
            NewTask {
                title: "Stephen's board card".into(),
                repo_key: REPO.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    wait_for(WAIT, "Stephen's device starts its agent", || {
        fs.started() >= 1
    });
    let holder = || {
        sco.get_task(&mine.id)
            .and_then(|t| t.holders.first().cloned())
    };
    wait_for(WAIT, "the card held on the board", || holder().is_some());
    let key = holder().unwrap();
    assert!(
        key.starts_with(kinds::AGENT_ACTOR_PREFIX),
        "held by {key}, not an agent:<cardId>"
    );
    wait_for(WAIT, "Bob's device sees it held by the agent", || {
        bco.get_task(&mine.id)
            .is_some_and(|t| t.holders.contains(&key))
    });
    wait_for(
        WAIT,
        "Bob's device knows the agent was started by Irori",
        || bco.spawned(&key).is_some_and(|m| m.task_id == mine.id),
    );
    let card_id = key.strip_prefix(kinds::AGENT_ACTOR_PREFIX).unwrap();
    let bstore = b.core.board_store(&board).unwrap();
    wait_for(
        WAIT,
        "Bob's device has its agent card marked spawned",
        || {
            bstore
                .list(&Filter {
                    kinds: vec!["agent.card".into()],
                    ..Filter::default()
                })
                .unwrap()
                .iter()
                .filter_map(|e| serde_json::from_slice::<serde_json::Value>(&e.payload).ok())
                .any(|v| v["cardId"] == card_id && v["spawned"] == serde_json::Value::Bool(true))
        },
    );
    let session = format!("{s_dev}/{SESS_A}");
    assert!(
        s.core.coord().is_spawned(&session),
        "marked in Stephen's own log"
    );

    kick();
    let reqs = fs.started.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1, "one agent: {reqs:?}");
    let req = &reqs[0];
    assert_eq!(req.task_id, mine.id);
    assert_eq!(Path::new(&req.folder), checkout.path());
    assert_ne!(req.permission_mode, "bypassPermissions", "{req:?}");
    assert!(
        coord::PERMISSION_MODES.contains(&req.permission_mode.as_str()),
        "{req:?}"
    );
    let sr = sco
        .get_task(&mine.id)
        .unwrap()
        .spawn_request()
        .cloned()
        .unwrap();
    assert_eq!(sr.permission_mode, req.permission_mode);
    assert_eq!(sr.generation, 1);
    let brief = std::fs::read_to_string(&req.brief_path).unwrap();
    assert!(brief.contains("Stephen's board card"), "{brief}");
    assert!(
        !brief.contains("Bob's card"),
        "the brief carries another card: {brief}"
    );
    // Bob's card stays as it was: nothing of Stephen's on it.
    let t = sco.get_task(&theirs.id).unwrap();
    assert!(t.holders.is_empty(), "{:?}", t.holders);
    s.core.close().unwrap();
    b.core.close().unwrap();
}
