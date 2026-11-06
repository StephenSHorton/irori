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
        phone.core.set_runs_no_agents();
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
        self.mac
            .core
            .set_spawner(Some(spawner(&self.fake, self.mac.claude.join("sessions"))));
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
        r.phone
            .core
            .no_agent()
            .get(&t.id)
            .is_some_and(|l| l.contains(&label) && l.contains("offline"))
    });
    let line = r.phone.core.no_agent().remove(&t.id).unwrap();
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
        phone.core.no_agent().is_empty()
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
        r.phone
            .core
            .no_agent()
            .get(&t.id)
            .is_some_and(|l| l.contains("Claude Code isn't installed"))
    });
    assert_eq!(r.fake.started(), 0, "nothing is tried without claude");
    r.fake.missing.store(false, Ordering::SeqCst);
    r.mac.core.set_spawn_cap(r.mac.core.spawn_cap()).unwrap();
    wait_for(WAIT, "it starts once Claude Code is there", || {
        r.fake.started() == 1
    });
    r.close();
}

/// A computer whose Claude Code went away after it last said so: a request
/// another device wrote before it heard reaches it, and it's refused in
/// plain words, not as a trust or folder problem, and never tried. The PC
/// writes the request itself: its own sweep would hear first and not ask.
#[test]
fn a_request_reaching_a_computer_without_claude_is_refused_plainly() {
    let r = Rig::new();
    r.fake.missing.store(true, Ordering::SeqCst);
    r.mac_spawner();
    let t = r.thread_held_back("Rename the API's handlers");
    let (pc, mac) = (r.phone_id(), r.mac_id());
    wait_for(WAIT, "the Mac has the card", || {
        r.mac.core.coord().get_task(&t.id).is_some()
    });
    r.phone
        .core
        .coord()
        .request_spawn(
            &format!("user:{pc}"),
            coord::SpawnAsk {
                task_id: t.id.clone(),
                device: mac,
                repo_key: REPO.into(),
                name: "stale".into(),
                permission_mode: "acceptEdits".into(),
                ..coord::SpawnAsk::default()
            },
        )
        .unwrap();
    wait_for(WAIT, "the Mac says why", || {
        r.phone
            .core
            .coord()
            .get_task(&t.id)
            .and_then(|t| t.spawn_request().cloned())
            .is_some_and(|s| s.reason.contains("Claude Code isn't installed"))
    });
    assert_eq!(r.fake.started(), 0, "nothing is tried without claude");
    r.close();
}

/// The only computer with the repo has no Claude Code, and the one that has
/// it has no copy: the card waits and says so, rather than blaming a folder
/// on the computer that could run it. It starts once Claude Code is there.
#[test]
fn a_repo_only_on_a_computer_without_claude_waits_and_says_so() {
    let r = Rig::new();
    let pc = Arc::new(Fake::default());
    r.phone
        .core
        .set_spawner(Some(spawner(&pc, r.phone.claude.join("sessions"))));
    r.fake.missing.store(true, Ordering::SeqCst);
    r.mac_spawner();
    let mac = r.mac_id();
    wait_for(
        WAIT,
        "the PC knows the Mac has the repo and no Claude Code",
        || {
            r.phone.core.cant_run_devices().contains(&mac)
                && r.phone
                    .core
                    .spawn_hosts()
                    .get(&mac)
                    .is_some_and(|h| h.repo(REPO).is_some())
        },
    );
    let t = r.card("Rename the API's handlers");
    let label = r.phone.core.device_label(&mac);
    wait_for(WAIT, "the PC says why", || {
        r.phone
            .core
            .no_agent()
            .get(&t.id)
            .is_some_and(|l| l.contains("Claude Code isn't installed") && l.contains(&label))
    });
    let t2 = r.phone.core.coord().get_task(&t.id).unwrap();
    assert!(
        t2.spawn_request().is_none(),
        "no folder failure on the PC: {:?}",
        t2.spawns
    );
    r.fake.missing.store(false, Ordering::SeqCst);
    r.mac.core.set_spawn_cap(r.mac.core.spawn_cap()).unwrap();
    wait_for(WAIT, "it starts on the Mac", || r.fake.started() == 1);
    assert_eq!(pc.started(), 0, "the PC has no copy of the repo");
    r.close();
}

impl Rig {
    /// A card nobody's sweep starts: spawning is off on the PC.
    fn thread_held_back(&self, title: &str) -> Task {
        self.phone
            .core
            .set_spawn_setting(SpawnSetting::default())
            .unwrap();
        self.card(title)
    }
}

/// Two computers, both away: the thread goes to one, the other comes back
/// first, and it takes the thread rather than wait for the first (found by
/// #541's audit: an ask's owner was fixed once).
#[test]
fn a_thread_starts_on_whichever_computer_comes_back() {
    let mut devs = person("stephen", 3);
    let (b, a, phone) = (
        devs.pop().unwrap(),
        devs.pop().unwrap(),
        devs.pop().unwrap(),
    );
    phone.core.set_runs_no_agents();
    for d in [&phone, &a, &b] {
        d.core
            .set_spawn_setting(SpawnSetting {
                on: true,
                ..SpawnSetting::default()
            })
            .unwrap();
    }
    let (fa, fb) = (Arc::new(Fake::default()), Arc::new(Fake::default()));
    let folder = tempfile::tempdir().unwrap();
    for d in [&a, &b] {
        d.core
            .capture()
            .emit_row(
                "irori",
                capture::KIND_REPO_SEEN,
                &capture::RepoSeen {
                    repo_key: REPO.into(),
                    root: folder.path().to_string_lossy().into_owned(),
                    at: gocompat::time::GoTime::unix(1_790_000_000, 0),
                },
            )
            .unwrap();
    }
    a.core
        .set_spawner(Some(spawner(&fa, a.claude.join("sessions"))));
    b.core
        .set_spawner(Some(spawner(&fb, b.claude.join("sessions"))));
    let (ia, ib) = (
        a.core.device_id().to_string(),
        b.core.device_id().to_string(),
    );
    wait_for(WAIT, "the phone knows both computers", || {
        let h = phone.core.spawn_hosts();
        h.contains_key(&ia) && h.contains_key(&ib)
    });
    a.core.close().unwrap();
    b.core.close().unwrap();
    wait_for(WAIT, "the phone would send work to a computer", || {
        phone.core.coord().agent_home("").is_some()
    });
    let t = phone
        .core
        .coord()
        .post_ask(
            &format!("user:{}", phone.core.device_id()),
            coord::NewAsk {
                text: "Is the build green?".into(),
                thread: true,
                ..coord::NewAsk::default()
            },
            &phone.core.ask_agents(),
        )
        .unwrap()
        .0;
    // The one it went to stays away; the other one comes back.
    let owner = t.ask.as_ref().unwrap().owner.clone();
    assert!(owner == ia || owner == ib, "it went to a computer");
    let (other, fother) = if owner == ia { (b, fb) } else { (a, fa) };
    let other = restart(other);
    other
        .core
        .set_spawner(Some(spawner(&fother, other.claude.join("sessions"))));
    wait_for(WAIT, "the computer that's back starts it", || {
        fother.started() == 1
    });
    let id = other.core.device_id().to_string();
    wait_for(WAIT, "the phone hears who has it", || {
        phone
            .core
            .coord()
            .get_task(&t.id)
            .and_then(|t| t.ask)
            .is_some_and(|a| a.owner == id)
    });
    phone.core.close().unwrap();
    other.core.close().unwrap();
}

/// An ask the phone still owns, from before it knew a computer or from an
/// older Irori: the Mac takes it, the phone hears, and it survives a
/// restart of the phone (the row is folded, not only written).
#[test]
fn the_mac_takes_a_thread_the_phone_still_owns() {
    let r = Rig::new();
    let t = r.thread("Why is the sync stuck?");
    assert_eq!(t.ask.as_ref().unwrap().owner, r.phone_id());
    r.mac_spawner();
    wait_for(WAIT, "an agent starts on the Mac", || r.fake.started() == 1);
    let mac = r.mac_id();
    wait_for(WAIT, "the phone hears the Mac has it", || {
        r.phone
            .core
            .coord()
            .get_task(&t.id)
            .and_then(|t| t.ask)
            .is_some_and(|a| a.owner == mac)
    });
    let Rig {
        phone,
        mac: m,
        _checkout,
        ..
    } = r;
    let phone = restart(phone);
    let t2 = phone.core.coord().get_task(&t.id).unwrap();
    assert_eq!(t2.ask.unwrap().owner, mac, "kept after a restart");
    phone.core.close().unwrap();
    m.core.close().unwrap();
}

/// A computer between start and its spawner, or closing, says nothing about
/// what it can run: no row calling it Claude-less, no ask given away. Found
/// by #541's audit: "no spawner" read as "no Claude" at every launch.
#[test]
fn a_computer_without_its_spawner_yet_speaks_for_nobody() {
    let r = Rig::new();
    let mac = r.mac_id();
    r.mac_spawner();
    wait_for(WAIT, "the phone has the Mac's row", || {
        r.phone
            .core
            .spawn_hosts()
            .get(&mac)
            .is_some_and(|h| !h.no_agents && h.repo(REPO).is_some())
    });
    // Installing lifts publish_host's once-a-minute limit, so a sweep with
    // no spawner would publish at once; then the app quits.
    r.mac_spawner();
    r.mac.core.set_spawner(None);
    std::thread::sleep(std::time::Duration::from_secs(8));
    let h = r.mac.core.spawn_hosts().remove(&mac).unwrap();
    assert!(!h.no_agents, "closing isn't losing Claude Code");
    assert!(h.repo(REPO).is_some(), "nor its checkouts");
    r.close();
}

/// A shared board's thread from the phone with no computer anywhere: the
/// person is told as on the private board. Found by #541's audit.
#[test]
fn a_shared_boards_thread_says_why_too() {
    let mut devs = person("stephen", 1);
    let phone = devs.pop().unwrap();
    phone.core.set_runs_no_agents();
    let id = phone.core.create_board("Team", "team").unwrap();
    phone
        .core
        .set_spawn_setting(SpawnSetting {
            board_id: id.clone(),
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    let (t, _) = phone
        .core
        .board_ask(
            &id,
            coord::NewAsk {
                text: "Why is the sync stuck?".into(),
                thread: true,
                ..coord::NewAsk::default()
            },
        )
        .unwrap();
    wait_for(WAIT, "the phone says why", || {
        phone.core.no_agent().contains_key(&t.id)
    });
    phone.core.close().unwrap();
}

/// A shared board of the person's, spawning on, and a thread the phone
/// posts on it before the Mac can start anything, so the phone owns it.
fn board_thread(r: &Rig) -> (String, Task) {
    let id = r.phone.core.create_board("Team", "team").unwrap();
    wait_for(WAIT, "the Mac has the board", || {
        r.mac.core.boards().contains(&id)
    });
    for d in [&r.phone, &r.mac] {
        d.core
            .set_spawn_setting(SpawnSetting {
                board_id: id.clone(),
                on: true,
                ..SpawnSetting::default()
            })
            .unwrap();
    }
    let (t, _) = r
        .phone
        .core
        .board_ask(
            &id,
            coord::NewAsk {
                text: "Why is the sync stuck?".into(),
                thread: true,
                ..coord::NewAsk::default()
            },
        )
        .unwrap();
    assert_eq!(t.ask.as_ref().unwrap().owner, r.phone_id());
    wait_for(WAIT, "the Mac has the thread", || {
        board_owner(&r.mac, &id, &t.id).is_some()
    });
    (id, t)
}

fn board_owner(d: &Dev, id: &str, task: &str) -> Option<String> {
    d.core
        .board_coord(id)
        .ok()
        .and_then(|c| c.get_task(task))
        .and_then(|t| t.ask)
        .map(|a| a.owner)
}

/// A shared board's thread the phone still owns: the Mac has heard every
/// member device say it reads `board.ask.moved`, so it takes the thread.
#[test]
fn the_mac_takes_a_boards_thread_the_phone_still_owns() {
    let r = Rig::new();
    let (id, t) = board_thread(&r);
    r.mac_spawner();
    let mac = r.mac_id();
    wait_for(WAIT, "the Mac takes it", || {
        board_owner(&r.mac, &id, &t.id).as_deref() == Some(mac.as_str())
    });
    wait_for(WAIT, "an agent starts on the Mac", || r.fake.started() == 1);
    wait_for(WAIT, "the phone hears the Mac has it", || {
        board_owner(&r.phone, &id, &t.id).as_deref() == Some(mac.as_str())
    });
    r.close();
}

/// The same thread when the Mac hasn't heard the phone since it started,
/// as with an older Irori whose hello has no `moves`: a member that doesn't
/// read the row would refuse it and stall on the gap, so the Mac leaves the
/// thread where it is and starts nothing.
#[test]
fn a_boards_thread_stays_put_while_a_member_is_unheard() {
    let r = Rig::new();
    let (id, t) = board_thread(&r);
    let ph = r.phone_id();
    let Rig {
        phone,
        mac,
        fake,
        _checkout,
    } = r;
    phone.core.close().unwrap();
    let mac = restart(mac);
    assert_eq!(board_owner(&mac, &id, &t.id), Some(ph.clone()));
    mac.core
        .set_spawner(Some(spawner(&fake, mac.claude.join("sessions"))));
    std::thread::sleep(std::time::Duration::from_secs(15));
    assert_eq!(board_owner(&mac, &id, &t.id), Some(ph), "not taken");
    assert_eq!(fake.started(), 0, "nothing started");
    mac.core.close().unwrap();
}

/// A computer on an Irori from before card #541 (its spawn.host has no
/// `moves`) never stands in for the phone or takes its asks, so it is
/// never where phone work goes: the phone keeps saying no agent can start
/// rather than waiting on it for good. Found by #541's review.
#[test]
fn an_older_computer_is_never_where_phone_work_goes() {
    let r = Rig::new();
    let mac = r.mac_id();
    // The Mac has no spawner, so it publishes nothing of its own; this is
    // the row an older Irori wrote.
    r.mac
        .core
        .capture()
        .emit_row(
            "irori",
            "spawn.host",
            &serde_json::json!({"device": mac, "os": "macos", "repos": [], "at": "2026-11-01T00:00:00Z"}),
        )
        .unwrap();
    wait_for(WAIT, "the phone has the Mac's older row", || {
        r.phone
            .core
            .spawn_hosts()
            .get(&mac)
            .is_some_and(|h| h.moves == 0)
    });
    wait_for(WAIT, "the phone knows it can't run agents", || {
        r.phone.core.cant_run_devices().contains(&r.phone_id())
    });
    std::thread::sleep(std::time::Duration::from_secs(3));
    assert_eq!(r.phone.core.coord().agent_home(""), None, "not a home");
    let t = r.thread("Why is the sync stuck?");
    assert_eq!(
        t.ask.as_ref().unwrap().owner,
        r.phone_id(),
        "not given to it"
    );
    r.close();
}
