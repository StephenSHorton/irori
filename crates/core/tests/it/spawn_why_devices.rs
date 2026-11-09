//! Card #574: why a card has no agent yet, when the answer is about
//! another of the person's devices. Real Cores of one person on loopback
//! iroh: A asks, B is the other device.
//!
//! Some cases look at a card before A's sweep acts on it (the sweep would
//! send it off or start it within seconds). There A's spawner holds its
//! sweep in the spawner's trust check (see [`Hold`]): the sweep never gets
//! to the cards, while the why-line, asked from the test's thread, reads
//! what the sweep would do.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::thread::ThreadId;
use std::time::Duration;

use crate::common::mesh::{Dev, Edit, person, person_with};
use crate::common::{Rig, wait_for};
use coord::{NewTask, Task};
use irori_core::Options;
use irori_core::spawn::{SpawnReq, SpawnSetting, SpawnStarted, Spawner};
use irori_core::spawn_why::Why;
use store::{Filter, kinds};

/// The repo only B has a checkout of.
const REPO_B: &str = "remote:github.com/stephen/api";
/// A repo only A has a checkout of.
const REPO_A: &str = "remote:github.com/stephen/web";
const WAIT: u64 = 90;

/// A spawner that never gets to start anything, trusting as `trusted`.
fn idle_spawner(trusted: bool) -> Spawner {
    Spawner {
        spawn: Arc::new(|_: &SpawnReq| {
            Ok(SpawnStarted {
                name: "never".into(),
                prefix: "never".into(),
            })
        }),
        end: Arc::new(|_: &str| Ok(String::new())),
        trusted: Arc::new(move |_: &Path| trusted),
        list: None,
        ready: None,
    }
}

/// Holds A's sweep in its spawner's trust check until dropped. The test's
/// own thread is never held, so the why-line can ask it.
struct Hold {
    held: Arc<AtomicBool>,
    entered: Arc<AtomicBool>,
    test: ThreadId,
}

impl Hold {
    fn new() -> Hold {
        Hold {
            held: Arc::new(AtomicBool::new(true)),
            entered: Arc::new(AtomicBool::new(false)),
            test: std::thread::current().id(),
        }
    }

    /// A spawner that trusts everything, and holds any other thread.
    fn spawner(&self) -> Spawner {
        let (held, entered, test) = (self.held.clone(), self.entered.clone(), self.test);
        let mut sp = idle_spawner(true);
        sp.trusted = Arc::new(move |_: &Path| {
            if std::thread::current().id() != test {
                entered.store(true, Ordering::SeqCst);
                while held.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            true
        });
        sp
    }

    /// Installs the spawner on `d` and waits until its sweep is held. The
    /// first pass after it publishes the device's host row, which asks the
    /// spawner about each checkout here, so `d` needs one.
    fn install(&self, d: &Dev) {
        d.core.set_spawner(Some(self.spawner()));
        wait_for(WAIT, "A's sweep is held", || {
            self.entered.load(Ordering::SeqCst)
        });
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.held.store(false, Ordering::SeqCst);
    }
}

fn seen(d: &Dev, repo: &str, dir: &Path) {
    d.core
        .capture()
        .emit_row(
            "irori",
            capture::KIND_REPO_SEEN,
            &capture::RepoSeen {
                repo_key: repo.into(),
                root: dir.to_string_lossy().into_owned(),
                at: gocompat::time::GoTime::unix(1_790_000_000, 0),
            },
        )
        .unwrap();
}

fn switch_on(d: &Dev) {
    d.core
        .set_spawn_setting(SpawnSetting {
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    wait_for(WAIT, "the switch is on in the coord", || {
        d.core.coord().spawning()
    });
}

fn card(d: &Dev, by: &str, title: &str, repo: &str) -> Task {
    d.core
        .coord()
        .create_task(
            by,
            NewTask {
                title: title.into(),
                repo_key: repo.into(),
                ..NewTask::default()
            },
        )
        .unwrap()
}

/// A has B's host row with `repo` (trusted as said) and sees B online.
fn a_sees_b_host(a: &Dev, b: &Dev, repo: &str, trusted: bool) {
    let b_id = b.core.device_id().to_string();
    wait_for(WAIT, "A has B's spawn.host with the repo", || {
        a.core
            .spawn_hosts()
            .get(&b_id)
            .and_then(|h| h.repo(repo).map(|r| r.trusted))
            == Some(trusted)
    });
    wait_for(WAIT, "A sees B online", || {
        a.core
            .mesh()
            .devices(false)
            .iter()
            .any(|r| r.device_id == b_id && r.state != "offline")
    });
}

fn two() -> (Dev, Dev) {
    let mut st = person("stephen", 2);
    let b = st.pop().unwrap();
    let a = st.pop().unwrap();
    (a, b)
}

/// A card B made is B's to start; once B is removed, the line says so
/// without naming B (card #1240).
#[test]
fn a_card_from_a_removed_device_says_it_was_removed() {
    let (a, b) = two();
    switch_on(&a);
    let b_id = b.core.device_id().to_string();
    let t = card(&b, &b.user(), "Made on B", REPO_B);
    wait_for(WAIT, "A has the card", || {
        a.core.coord().get_task(&t.id).is_some()
    });
    match a.core.why_no_agent(&t.id, true) {
        Some(Why::Elsewhere { .. }) => {}
        other => panic!("before the removal: {other:?}"),
    }
    let b_name = a.core.device_label(&b_id);
    assert!(!b_name.is_empty(), "A names B before the removal");
    let rt = a.core.pairing_runtime().clone();
    rt.block_on(a.core.remove_device(&b_id)).unwrap();
    wait_for(WAIT, "A reads B as removed", || {
        matches!(a.core.why_no_agent(&t.id, true), Some(Why::DeviceRemoved))
    });
    let line = Why::DeviceRemoved.to_string();
    assert!(!line.contains(&b_id[..8]), "no id: {line}");
    assert!(!line.contains(&b_name), "no name: {line}");
}

/// A card handed out by a device that can't run agents (a headless node
/// here) says that device can't.
#[test]
fn a_card_from_a_node_says_it_cant_run_agents() {
    let r = Rig::new();
    r.core
        .set_spawn_setting(SpawnSetting {
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    let node = "node-574";
    let key = ed25519_dalek::SigningKey::from_bytes(&[57; 32]);
    let add = r
        .core
        .identity()
        .cert_for(
            node,
            &hex::encode(key.verifying_key().to_bytes()),
            "Pi",
            irori_identity::objects::ROLE_NODE,
            presence::now_local(),
        )
        .unwrap();
    r.core
        .store()
        .append("irori", kinds::PERSON_DEVICE_ADDED, &add)
        .unwrap();
    wait_for(WAIT, "the node is one of the person's devices", || {
        r.core
            .mesh()
            .devices(false)
            .iter()
            .any(|d| d.device_id == node && d.role == irori_identity::objects::ROLE_NODE)
    });
    let t = r
        .core
        .coord()
        .create_task(
            &format!("user:{node}"),
            NewTask {
                title: "Handed out by the node".into(),
                repo_key: REPO_B.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    assert_eq!(
        r.core.why_no_agent(&t.id, true),
        Some(Why::CantRun {
            device: "Pi".into()
        })
    );
}

/// A private card for a repo only B has, and B is at its cap: A doesn't
/// ask a full device elsewhere (spawn_device skips it), so the line comes
/// from the devices that have the repo, and names B's cap.
#[test]
fn a_card_for_a_full_device_names_its_cap() {
    let (a, b) = two();
    let dir = tempfile::tempdir().unwrap();
    seen(&b, REPO_B, dir.path());
    b.core.set_spawner(Some(idle_spawner(true)));
    b.core.set_spawn_cap(0).unwrap();
    a.core.set_spawner(Some(idle_spawner(true)));
    a_sees_b_host(&a, &b, REPO_B, true);
    // A reads B's cap before its switch is on, so its sweep never asks B.
    let b_id = b.core.device_id().to_string();
    wait_for(WAIT, "A has B's cap row", || {
        a.core
            .store()
            .list(&Filter {
                kinds: vec![kinds::SPAWN_CAP.into()],
                ..Filter::default()
            })
            .unwrap()
            .iter()
            .any(|e| e.device_id == b_id)
    });
    a.core.sweep_coord().unwrap();
    switch_on(&a);
    let t = card(&a, &a.user(), "For B", REPO_B);
    for _ in 0..20 {
        match a.core.why_no_agent(&t.id, true) {
            Some(Why::AtCap {
                device,
                running,
                cap,
            }) => {
                assert_eq!((running, cap), (0, 0));
                assert_eq!(device, b.core.device_label(&b_id));
            }
            other => panic!("{other:?}"),
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A private card for a repo B has but doesn't trust: A would ask B, and
/// the line says the folder isn't trusted there, from B's host row.
#[test]
fn a_card_for_an_untrusted_repo_elsewhere_names_that_device() {
    let (a, b) = two();
    let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    seen(&b, REPO_B, dir_b.path());
    seen(&a, REPO_A, dir_a.path());
    b.core.set_spawner(Some(idle_spawner(false)));
    a.core.set_spawn_cap(1).unwrap();
    a_sees_b_host(&a, &b, REPO_B, false);
    switch_on(&a);
    let hold = Hold::new();
    hold.install(&a);
    let t = card(&a, &a.user(), "For B", REPO_B);
    let b_id = b.core.device_id().to_string();
    assert_eq!(
        a.core.why_no_agent(&t.id, true),
        Some(Why::NeedsTrust {
            folder: dir_b.path().to_string_lossy().into_owned(),
            device: b.core.device_label(&b_id),
        })
    );
    drop(hold);
}

/// A clock that runs `skew` seconds ahead.
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

/// A private card for a repo only B has, and B is offline: it waits for
/// B, and the line names B in card #541's words.
#[test]
fn a_card_for_an_offline_device_waits_for_it() {
    let skew = Arc::new(AtomicI64::new(0));
    let sk = skew.clone();
    let mut st = person_with("stephen", 2, |i| (i == 0).then(|| skewed(sk.clone())));
    let b = st.pop().unwrap();
    let a = st.pop().unwrap();
    let dir = tempfile::tempdir().unwrap();
    seen(&b, REPO_B, dir.path());
    b.core.set_spawner(Some(idle_spawner(true)));
    a_sees_b_host(&a, &b, REPO_B, true);
    let b_id = b.core.device_id().to_string();
    let b_name = b.core.device_label(&b_id);
    b.core.close().unwrap();
    // Past the mesh's "recent" (mesh::node::RECENT_WITHIN, 10 minutes).
    skew.store(
        mesh::node::RECENT_WITHIN.as_secs() as i64 + 60,
        Ordering::SeqCst,
    );
    wait_for(WAIT, "A sees B offline", || {
        a.core
            .mesh()
            .devices(false)
            .iter()
            .any(|r| r.device_id == b_id && r.state == "offline")
    });
    a.core.set_spawner(Some(idle_spawner(true)));
    switch_on(&a);
    let t = card(&a, &a.user(), "For B", REPO_B);
    let offline = Some(Why::NoComputer(format!(
        "{b_name} is offline. It starts by itself when it's back"
    )));
    // Until #541's host view hears B is gone, it's "waiting for B".
    wait_for(WAIT, "the line says B is offline", || {
        a.core.why_no_agent(&t.id, true) == offline
    });
    for _ in 0..20 {
        assert_eq!(a.core.why_no_agent(&t.id, true), offline);
        std::thread::sleep(Duration::from_millis(50));
    }
    // An agent asking for one (card #575) is queued, not refused, and told
    // what it waits on.
    let c = tools::Caller {
        sender: inbox::Sender::new(&a.agent, "alpha", "claude"),
        path: inbox::path::MOD.into(),
    };
    let args = serde_json::json!({ "task_id": t.id });
    let out = a
        .core
        .tools()
        .call(&c, "agent_start", &serde_json::to_vec(&args).unwrap())
        .unwrap();
    assert!(
        out.contains("Asked for an agent") && out.contains(&format!("{b_name} is offline")),
        "{out}"
    );
}

/// A has one slot and nothing running. An older card goes to B (A
/// publishes no host row while its sweep is held, so only B's names its
/// repo, though A has a trusted checkout too). It takes none of A's slots:
/// a newer card for a repo only A has starts at the next pass, not after
/// the older one.
#[test]
fn an_older_card_sent_elsewhere_takes_no_slot_here() {
    let (a, b) = two();
    let (dir_a, dir_ab, dir_b) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    seen(&b, REPO_B, dir_b.path());
    seen(&a, REPO_A, dir_a.path());
    seen(&a, REPO_B, dir_ab.path());
    b.core.set_spawner(Some(idle_spawner(true)));
    a.core.set_spawn_cap(1).unwrap();
    a_sees_b_host(&a, &b, REPO_B, true);
    switch_on(&a);
    let hold = Hold::new();
    hold.install(&a);
    let a_id = a.core.device_id().to_string();
    assert!(
        !a.core.spawn_hosts().contains_key(&a_id),
        "A published no host row"
    );
    let older = card(&a, &a.user(), "Older, for B", REPO_B);
    std::thread::sleep(Duration::from_millis(5));
    let newer = card(&a, &a.user(), "Newer, here", REPO_A);
    for _ in 0..20 {
        assert_eq!(
            a.core.why_no_agent(&older.id, true),
            Some(Why::NextPass),
            "B has room"
        );
        assert_eq!(a.core.why_no_agent(&newer.id, true), Some(Why::NextPass));
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(hold);
}
