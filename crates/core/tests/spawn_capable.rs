//! Card #541 through two real Cores of one person on loopback iroh: A (the
//! phone: no spawner, as on Android) and B (the Mac, with a fake spawner and
//! a checkout). Work that lands on A goes to B and never starts on A; with
//! B away the person is told why and it starts when B is back; a computer
//! without Claude Code behaves the same way.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use common::mesh::{Dev, person, restart};
use common::{wait_for, write_session};
use coord::{NewTask, Task};
use irori_core::spawn::{SpawnReq, SpawnSetting, SpawnStarted, Spawner};

const REPO: &str = "remote:github.com/stephen/api";
const SESS: &str = "bbbbbbbb-5555-4555-8555-00000000000b";
const WAIT: u64 = 90;

#[derive(Default)]
struct Fake {
    started: Mutex<Vec<SpawnReq>>,
    /// `claude` isn't installed.
    missing: AtomicBool,
}

impl Fake {
    fn started(&self) -> usize {
        self.started.lock().unwrap().len()
    }
}

fn spawner(f: &Arc<Fake>, sessions: PathBuf) -> Spawner {
    let (a, b) = (f.clone(), f.clone());
    Spawner {
        ready: Some(Arc::new(move || !b.missing.load(Ordering::SeqCst))),
        spawn: Arc::new(move |r: &SpawnReq| {
            a.started.lock().unwrap().push(r.clone());
            write_session(
                &sessions,
                "900003.json",
                SESS,
                &r.name,
                Path::new(&r.folder),
            );
            Ok(SpawnStarted {
                name: r.name.clone(),
                prefix: SESS[..8].into(),
            })
        }),
        end: Arc::new(|_: &str| Ok(String::new())),
        trusted: Arc::new(|_| true),
        list: None,
    }
}

struct Rig {
    phone: Dev,
    mac: Dev,
    fake: Arc<Fake>,
    _checkout: tempfile::TempDir,
}

impl Rig {
    fn new() -> Rig {
        let mut devs = person("stephen", 2);
        let mac = devs.pop().unwrap();
        let phone = devs.pop().unwrap();
        let checkout = tempfile::tempdir().unwrap();
        mac.core
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
        let r = Rig {
            phone,
            mac,
            fake: Arc::new(Fake::default()),
            _checkout: checkout,
        };
        for d in [&r.phone, &r.mac] {
            d.core
                .set_spawn_setting(SpawnSetting {
                    on: true,
                    ..SpawnSetting::default()
                })
                .unwrap();
        }
        r
    }

    fn phone_id(&self) -> String {
        self.phone.core.device_id().to_string()
    }
    fn mac_id(&self) -> String {
        self.mac.core.device_id().to_string()
    }

    fn mac_spawner(&self) {
        self.mac.core.set_spawner(Some(spawner(
            &self.fake,
            self.mac.claude.join("sessions"),
        )));
    }

    /// The phone knows the Mac's host row, and says it can't run agents.
    fn phone_sees_mac(&self) {
        let (mac, phone) = (self.mac_id(), self.phone_id());
        wait_for(WAIT, "the phone has the Mac's spawn.host", || {
            self.phone
                .core
                .spawn_hosts()
                .get(&mac)
                .is_some_and(|h| h.repo(REPO).is_some())
        });
        wait_for(WAIT, "the Mac has the phone's spawn.host", || {
            self.mac
                .core
                .spawn_hosts()
                .get(&phone)
                .is_some_and(|h| h.no_agents)
        });
        wait_for(WAIT, "the phone knows it can't run agents", || {
            self.phone.core.cant_run_devices().contains(&phone)
        });
    }

    fn thread(&self, text: &str) -> Task {
        self.phone
            .core
            .coord()
            .post_ask(
                &format!("user:{}", self.phone_id()),
                coord::NewAsk {
                    text: text.into(),
                    thread: true,
                    ..coord::NewAsk::default()
                },
                &self.phone.core.ask_agents(),
            )
            .unwrap()
            .0
    }

    fn card(&self, title: &str) -> Task {
        self.phone
            .core
            .coord()
            .create_task(
                &format!("user:{}", self.phone_id()),
                NewTask {
                    title: title.into(),
                    repo_key: REPO.into(),
                    ..NewTask::default()
                },
            )
            .unwrap()
    }

    fn close(self) {
        self.phone.core.close().unwrap();
        self.mac.core.close().unwrap();
    }
}

/// Every spawn request any device wrote for card `id` named `dev`.
fn asked_only(d: &Dev, id: &str, dev: &str) {
    let t = d.core.coord().get_task(id).unwrap();
    assert!(
        t.spawns.iter().all(|s| s.device == dev),
        "only {dev} is ever asked: {:?}",
        t.spawns
    );
}

/// The Done when's first half: with no agent free, a thread from the phone
/// gets an agent on the computer, and nothing is tried on the phone.
#[test]
fn a_thread_from_the_phone_starts_on_the_mac() {
    let r = Rig::new();
    r.mac_spawner();
    r.phone_sees_mac();
    let t = r.thread("Why is the sync stuck?");
    assert_eq!(
        t.ask.as_ref().unwrap().owner,
        r.mac_id(),
        "the phone gives it to the Mac as it posts"
    );
    wait_for(WAIT, "an agent starts on the Mac", || r.fake.started() == 1);
    asked_only(&r.mac, &t.id, &r.mac_id());
    r.close();
}

/// A card made on the phone needs nothing more from the phone: it can be
/// asleep, and the Mac stands in for it.
#[test]
fn a_card_from_a_sleeping_phone_starts_on_the_mac() {
    let r = Rig::new();
    r.phone_sees_mac_without_spawner_on_mac();
    let t = r.card("Rename the API's handlers");
    let mac = r.mac_id();
    wait_for(WAIT, "the Mac has the card", || {
        r.mac.core.coord().get_task(&t.id).is_some()
    });
    r.phone.core.close().unwrap();
    r.mac_spawner();
    wait_for(WAIT, "an agent starts on the Mac", || r.fake.started() == 1);
    asked_only(&r.mac, &t.id, &mac);
    r.mac.core.close().unwrap();
}

impl Rig {
    /// The Mac has no spawner yet, so it can't start anything, but both
    /// know each other: the phone can't run agents.
    fn phone_sees_mac_without_spawner_on_mac(&self) {
        let phone = self.phone_id();
        wait_for(WAIT, "the Mac has the phone's spawn.host", || {
            self.mac
                .core
                .spawn_hosts()
                .get(&phone)
                .is_some_and(|h| h.no_agents)
        });
    }
}

/// The Done when's second half: with every computer offline the phone says
/// why and what happens next, and the thread starts when the Mac is back.
#[test]
fn with_the_mac_offline_the_phone_says_so_and_the_thread_starts_when_it_is_back() {
    let r = Rig::new();
    r.mac_spawner();
    r.phone_sees_mac();
    let label = r.phone.core.device_label(&r.mac_id());
    r.mac.core.close().unwrap();
    let t = r.thread("Is the build green?");
    wait_for(WAIT, "the phone says why", || {
        let (line, ids) = r.phone.core.no_agent();
        ids.contains(&t.id) && line.contains(&label) && line.contains("offline")
    });
    let (line, _) = r.phone.core.no_agent();
    assert!(!line.contains('?') && !line.contains("repo"), "{line}");
    assert!(line.contains("starts by itself"), "{line}");
    // After NO_AGENT_AFTER, on real clocks: a phone clock run ahead would
    // have the Mac refuse its rows.
    wait_for(WAIT, "it reaches the person's notifications", || {
        r.phone.core.notifications().iter().any(|n| {
            n.posted.kind == coord::notifications::KIND_NO_AGENT && n.posted.task_id == t.id
        })
    });
    let Rig {
        phone,
        mac,
        fake,
        _checkout,
        ..
    } = r;
    let mac = restart(mac);
    mac.core
        .set_spawner(Some(spawner(&fake, mac.claude.join("sessions"))));
    wait_for(WAIT, "the agent starts on the Mac", || fake.started() == 1);
    wait_for(WAIT, "the phone stops saying it", || {
        phone.core.no_agent().0.is_empty()
    });
    phone.core.close().unwrap();
    mac.core.close().unwrap();
}

/// A computer without Claude Code is passed over the same way: nothing is
/// tried there, the person is told, and it starts once Claude is installed.
#[test]
fn a_computer_without_claude_starts_nothing_until_it_is_installed() {
    let r = Rig::new();
    r.fake.missing.store(true, Ordering::SeqCst);
    r.mac_spawner();
    r.phone_sees_mac();
    let mac = r.mac_id();
    wait_for(WAIT, "the phone hears the Mac has no Claude Code", || {
        r.phone
            .core
            .spawn_hosts()
            .get(&mac)
            .is_some_and(|h| h.no_agents)
    });
    let t = r.thread("Why is the sync stuck?");
    wait_for(WAIT, "the phone says why", || {
        let (line, ids) = r.phone.core.no_agent();
        ids.contains(&t.id) && line.contains("Claude Code isn't installed")
    });
    assert_eq!(r.fake.started(), 0, "nothing is tried without claude");
    r.fake.missing.store(false, Ordering::SeqCst);
    r.mac.core.set_spawn_cap(r.mac.core.spawn_cap()).unwrap();
    wait_for(WAIT, "it starts once Claude Code is there", || {
        r.fake.started() == 1
    });
    r.close();
}

/// A computer whose Claude Code went away after it last said so: another
/// computer's request reaches it, and it's refused in plain words, not as a
/// trust or folder problem, and never tried.
#[test]
fn a_request_reaching_a_computer_without_claude_is_refused_plainly() {
    let r = Rig::new();
    let pc = Arc::new(Fake::default());
    r.phone
        .core
        .set_spawner(Some(spawner(&pc, r.phone.claude.join("sessions"))));
    r.mac_spawner();
    let mac = r.mac_id();
    wait_for(WAIT, "the PC has the Mac's spawn.host", || {
        r.phone
            .core
            .spawn_hosts()
            .get(&mac)
            .is_some_and(|h| h.repo(REPO).is_some() && !h.no_agents)
    });
    wait_for(WAIT, "the PC sees the Mac online", || {
        r.phone
            .core
            .mesh()
            .devices(false)
            .iter()
            .any(|d| d.device_id == mac && d.state != "offline")
    });
    r.fake.missing.store(true, Ordering::SeqCst);
    let t = r.card("Rename the API's handlers");
    std::thread::sleep(std::time::Duration::from_secs(20)); eprintln!("DBG pc {:?}\n mac {:?}\n hosts {:?}\n cant {:?}", r.phone.core.coord().get_task(&t.id).map(|t| (t.state.clone(), t.spawns.clone(), t.holders.clone())), r.mac.core.coord().get_task(&t.id).map(|t| t.spawns.clone()), r.phone.core.spawn_hosts().keys().collect::<Vec<_>>(), r.phone.core.cant_run_devices());
    wait_for(WAIT, "the Mac says why", || {
        r.mac
            .core
            .coord()
            .get_task(&t.id)
            .and_then(|t| t.spawn_request().cloned())
            .is_some_and(|s| s.reason.contains("Claude Code isn't installed"))
    });
    assert_eq!(r.fake.started(), 0);
    assert_eq!(pc.started(), 0, "the PC has no copy of the repo");
    r.close();
}
