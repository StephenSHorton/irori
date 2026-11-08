//! One assistant, card 1: the recorded assistant's computer (one-assistant.md
//! §3). A private `assistant.home` row names it: the first automatic claim
//! wins, a person's pick beats it, a removal ends older rows, and nothing
//! about a computer being offline or missing Claude Code moves it.

use crate::common;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::mesh::{Dev, Edit, free_udp, person, person_with};
use common::wait_for;
use gocompat::time::GoTime;
use irori_core::assistant_home::HomeRow;
use irori_core::spawn::{SpawnStarted, Spawner};
use irori_core::SpawnHost;
use irori_core::{Core, CoreEvent, Options, Topic};
use store::kinds;
use store::{Event, Filter};

const WAIT: u64 = 90;

/// A spawner that can start `claude` while `ready` holds.
fn spawner(ready: &Arc<AtomicBool>) -> Spawner {
    let r = ready.clone();
    Spawner {
        ready: Some(Arc::new(move || r.load(Ordering::SeqCst))),
        spawn: Arc::new(|r| {
            Ok(SpawnStarted {
                name: r.name.clone(),
                prefix: String::new(),
            })
        }),
        end: Arc::new(|_| Ok(String::new())),
        trusted: Arc::new(|_| true),
        list: None,
    }
}

/// `d` can run the assistant's turn; flip the flag to take Claude Code away.
fn able(d: &Dev) -> Arc<AtomicBool> {
    let ready = Arc::new(AtomicBool::new(true));
    d.core.set_spawner(Some(spawner(&ready)));
    ready
}

/// Every `assistant.home` row in `c`'s log, in store order, with its writer.
fn home_rows(c: &Core) -> Vec<(String, HomeRow)> {
    let mut evs: Vec<Event> = c
        .store()
        .list(&Filter {
            kinds: vec![kinds::ASSISTANT_HOME.into()],
            ..Filter::default()
        })
        .unwrap();
    evs.sort_by_key(|e| (e.hlc, e.device_id.clone(), e.seq));
    evs.into_iter()
        .map(|e| {
            let r: HomeRow = serde_json::from_slice(&e.payload).unwrap();
            (e.device_id, r)
        })
        .collect()
}

fn id(d: &Dev) -> String {
    d.core.device_id().to_string()
}

fn home_is(d: &Dev, want: &str) -> bool {
    d.core.assistant_home().as_deref() == Some(want)
}

/// Writes a row as the person's `user` actor, whatever `set_assistant_home`
/// would say.
fn write_row(c: &Core, device: &str, how: &str, at: GoTime) {
    let row = HomeRow {
        device: device.into(),
        how: how.into(),
        by: format!("user:{}", c.device_id()),
        at,
    };
    c.store()
        .append("user", kinds::ASSISTANT_HOME, &row)
        .unwrap();
}

/// Waits `secs` and checks `cond` held the whole time.
fn holds_for(secs: u64, what: &str, cond: impl Fn() -> bool) {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    while std::time::Instant::now() < end {
        assert!(cond(), "{what}");

        std::thread::sleep(Duration::from_millis(250));
    }
}

// ---- Cores that can be kept apart, then meet ----

type Links = Arc<Mutex<Vec<SocketAddr>>>;

fn start_dyn(
    data: &std::path::Path,
    cl: &std::path::Path,
    lan: SocketAddr,
    links: Links,
) -> Core {
    let mut o = common::options(data, cl);
    o.pair_net = Some(Arc::new(move |ident, chain, guests| {
        let mut nc = irori_core::devices::default_net_config(ident, chain, guests);
        nc.ip = irori_net::endpoint::IpMode::Loopback;
        nc.gossip = false;
        let lc = nc.lan_config.as_mut().expect("a LAN lookup");
        lc.bind = Some(lan);
        let links = links.clone();
        lc.targets = Some(Arc::new(move || links.lock().unwrap().clone()));
        lc.every = Duration::from_millis(200);
        irori_net::transport::IrohNet::new(nc)
    }));
    Core::start(o).unwrap()
}

fn dev_dyn(
    data: tempfile::TempDir,
    name: &str,
    sess: &str,
    lan: SocketAddr,
    links: Links,
) -> Dev {
    let (cl, w) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let sessions = cl.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    common::write_session(&sessions, "900001.json", sess, name, w.path());
    let core = start_dyn(data.path(), cl.path(), lan, links);
    let agent = format!("{}/{sess}", core.device_id());
    wait_for(60, "presence lists the session", || {
        core.local_agents().iter().any(|a| a.key == agent)
    });
    Dev {
        core,
        agent,
        data: data.path().to_path_buf(),
        claude: cl.path().to_path_buf(),
        lan,
        to: Vec::new(),
        edit: None,
        _dirs: vec![data, cl, w],
    }
}

/// One person on two devices that know each other but can't reach each
/// other until [`meet`].
fn apart(who: &str) -> (Vec<Dev>, Vec<Links>, Vec<SocketAddr>) {
    let lans: Vec<SocketAddr> = (0..2).map(|_| free_udp()).collect();
    let links: Vec<Links> = (0..2).map(|_| Links::default()).collect();
    let sess = |i: usize| format!("{:08x}-1111-4111-8111-{:012x}", i + 1, i + 1);
    let first = dev_dyn(
        tempfile::tempdir().unwrap(),
        &format!("{who}1"),
        &sess(0),
        lans[0],
        links[0].clone(),
    );
    let d = tempfile::tempdir().unwrap();
    std::fs::copy(
        first.data.join(irori_identity::PERSON_FILE),
        d.path().join(irori_identity::PERSON_FILE),
    )
    .unwrap();
    let second = dev_dyn(d, &format!("{who}2"), &sess(1), lans[1], links[1].clone());
    let add = first
        .core
        .identity()
        .cert_for(
            second.core.device_id(),
            &hex::encode(second.core.identity().device_pub()),
            &format!("{who}2"),
            irori_identity::ROLE_DESKTOP,
            presence::now_local(),
        )
        .unwrap();
    first
        .core
        .store()
        .append("irori", kinds::PERSON_DEVICE_ADDED, &add)
        .unwrap();
    let rows = first
        .core
        .store()
        .list(&Filter {
            kinds: vec!["person.".into()],
            ..Filter::default()
        })
        .unwrap();
    for _ in 0..2 {
        for ev in &rows {
            let _ = second.core.chain().learn(ev.into());
        }
    }
    wait_for(45, "the devices own each other's keys", || {
        let devs = [&first, &second];
        devs.iter().all(|x| {
            devs.iter()
                .all(|y| x.core.is_owner(&format!("user:{}", y.core.device_id())))
        })
    });
    // Nothing is dialed: each one's lookup has no targets yet.
    (vec![first, second], links, lans)
}

fn meet(links: &[Links], lans: &[SocketAddr]) {
    for (i, l) in links.iter().enumerate() {
        *l.lock().unwrap() = lans
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, a)| *a)
            .collect();
    }
}

/// A clock that runs `skew` seconds ahead.
fn skewed(skew: Arc<AtomicI64>) -> Edit {
    Arc::new(move |o: &mut Options| {
        let sk = skew.clone();
        o.now = Some(Arc::new(move || {
            let d = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap();
            GoTime::unix(
                d.as_secs() as i64 + sk.load(Ordering::SeqCst),
                i64::from(d.subsec_nanos()),
            )
        }));
    })
}

// ---- the tests ----

/// Two computers that claim while they can't see each other each think they
/// are the assistant's computer. When they meet the first claim, by the
/// store's order, wins on both.
#[test]
fn two_cores_that_claim_apart_agree_on_the_first_claim_when_they_meet() {
    let (devs, links, lans) = apart("stephen");
    let (a, b) = (&devs[0], &devs[1]);
    let (ra, rb) = (able(a), able(b));
    wait_for(WAIT, "each claims for itself", || {
        home_is(a, &id(a)) && home_is(b, &id(b))
    });
    assert!(a.core.is_assistant_home() && b.core.is_assistant_home());
    meet(&links, &lans);
    wait_for(WAIT, "both Cores hold both claims", || {
        home_rows(&a.core).len() == 2 && home_rows(&b.core).len() == 2
    });
    let first = home_rows(&a.core)[0].0.clone();
    assert_eq!(home_rows(&b.core)[0].0, first, "one store order");
    wait_for(WAIT, "both agree on the first claim", || {
        home_is(a, &first) && home_is(b, &first)
    });
    assert_eq!(
        [a, b].iter().filter(|d| d.core.is_assistant_home()).count(),
        1
    );
    // Nobody writes a third.
    holds_for(12, "a claim was written after the meeting", || {
        home_rows(&a.core).len() == 2 && home_rows(&b.core).len() == 2
    });
    drop((ra, rb));
    a.core.close().unwrap();
    b.core.close().unwrap();
}

/// A person's row beats an earlier automatic claim, the newest of their rows
/// wins by the store's order even when its `at` is smaller, and a change of
/// the answer is `assistant:home`.
#[test]
fn a_persons_pick_beats_a_claim_and_the_newest_pick_wins() {
    let mut devs = person("stephen", 2);
    let (pc, mac) = (devs.pop().unwrap(), devs.pop().unwrap());
    let feed = pc.core.feed(&[Topic::AssistantHome]);
    let _ready = able(&mac);
    wait_for(WAIT, "the Mac claims and both agree", || {
        home_is(&mac, &id(&mac)) && home_is(&pc, &id(&mac))
    });
    assert_eq!(home_rows(&mac.core).len(), 1);
    assert_eq!(home_rows(&mac.core)[0].1.how, "auto");
    assert!(
        std::iter::from_fn(|| feed.try_recv()).any(|e| matches!(e, CoreEvent::AssistantHome)),
        "the PC heard assistant:home when the Mac claimed"
    );

    // The person picks the PC, from the Mac.
    assert!(mac.core.set_assistant_home("nope").is_err());
    mac.core.set_assistant_home(&id(&pc)).unwrap();
    wait_for(WAIT, "both follow the pick", || {
        home_is(&mac, &id(&pc)) && home_is(&pc, &id(&pc))
    });
    assert!(pc.core.is_assistant_home() && !mac.core.is_assistant_home());

    // A later pick with an `at` long before: store order decides.
    write_row(&pc.core, &id(&mac), "person", GoTime::unix(1, 0));
    wait_for(WAIT, "the newest pick wins on both", || {
        home_is(&mac, &id(&mac)) && home_is(&pc, &id(&mac))
    });
    // A claim written now doesn't move it.
    write_row(&pc.core, &id(&pc), "auto", presence::now_local());
    std::thread::sleep(Duration::from_secs(1));
    assert!(home_is(&mac, &id(&mac)) && home_is(&pc, &id(&mac)));
    mac.core.close().unwrap();
    pc.core.close().unwrap();
}

/// The assistant's computer stopped: another able Core never claims.
#[test]
fn a_stopped_home_is_never_replaced() {
    let mut devs = person("stephen", 2);
    let (pc, mac) = (devs.pop().unwrap(), devs.pop().unwrap());
    let _r = able(&mac);
    wait_for(WAIT, "the Mac is the assistant's computer", || {
        home_is(&pc, &id(&mac))
    });
    mac.core.close().unwrap();
    let _r2 = able(&pc);
    holds_for(14, "the PC took over from a stopped Mac", || {
        home_is(&pc, &id(&mac)) && !pc.core.is_assistant_home() && home_rows(&pc.core).len() == 1
    });
    pc.core.close().unwrap();
}

/// A computer whose `assistantTurn` drops stays the assistant's computer,
/// no other Core claims, and the status says Claude Code is missing there.
#[test]
fn a_home_that_loses_claude_code_stays_home() {
    let mut devs = person("stephen", 2);
    let (pc, mac) = (devs.pop().unwrap(), devs.pop().unwrap());
    let ready = able(&mac);
    wait_for(WAIT, "the Mac is the assistant's computer", || {
        home_is(&pc, &id(&mac))
    });
    let st = pc.core.assistant_home_status();
    assert_eq!((st.device.as_str(), st.here, st.claude), (&*id(&mac), false, true));
    assert!(!st.name.is_empty());
    ready.store(false, Ordering::SeqCst);
    mac.core.set_spawner(Some(spawner(&ready)));
    let _r2 = able(&pc);
    wait_for(WAIT, "the PC hears the Mac lost Claude Code", || {
        pc.core
            .spawn_hosts()
            .get(&id(&mac))
            .is_some_and(|h| !h.assistant_turn)
    });
    holds_for(12, "the assistant's computer moved", || {
        home_is(&pc, &id(&mac)) && home_is(&mac, &id(&mac)) && home_rows(&pc.core).len() == 1
    });
    let st = pc.core.assistant_home_status();
    assert_eq!(st.device, id(&mac));
    assert!(!st.claude, "the status says Claude Code is missing there");
    assert!(st.none.is_empty());
    mac.core.close().unwrap();
    pc.core.close().unwrap();
}

/// A claim, the person's pick of another computer, that computer removed: no
/// assistant's computer. Then a connected third claims and the first claim
/// doesn't come back. A row naming the removed computer, written after, is
/// ignored.
#[test]
fn a_removed_computer_ends_older_rows() {
    let mut devs = person("stephen", 3);
    let (pc2, pc, mac) = (
        devs.pop().unwrap(),
        devs.pop().unwrap(),
        devs.pop().unwrap(),
    );
    let feed = mac.core.feed(&[Topic::AssistantHome]);
    let ready = able(&mac);
    wait_for(WAIT, "the Mac claims; all three agree", || {
        [&mac, &pc, &pc2].iter().all(|d| home_is(d, &id(&mac)))
    });
    mac.core.set_assistant_home(&id(&pc)).unwrap();
    wait_for(WAIT, "all three follow the pick of the PC", || {
        [&mac, &pc, &pc2].iter().all(|d| home_is(d, &id(&pc)))
    });
    // The Mac can no longer claim (it must not be the one that does).
    ready.store(false, Ordering::SeqCst);
    mac.core.set_spawner(Some(spawner(&ready)));
    wait_for(WAIT, "the Mac's host row drops assistantTurn", || {
        pc2.core
            .spawn_hosts()
            .get(&id(&mac))
            .is_some_and(|h| !h.assistant_turn)
    });
    let _ = std::iter::from_fn(|| feed.try_recv()).count();

    // The Mac removes the PC. No row names a new computer, yet the answer
    // changes: the removal ends the rows before it.
    let rt = mac.core.pairing_runtime().clone();
    rt.block_on(mac.core.mesh().revoke(&id(&pc), "test"))
        .unwrap();
    wait_for(WAIT, "the answer clears on the Mac and on the PC2", || {
        mac.core.assistant_home().is_none() && pc2.core.assistant_home().is_none()
    });
    assert!(
        std::iter::from_fn(|| feed.try_recv()).any(|e| matches!(e, CoreEvent::AssistantHome)),
        "a removal that changed the answer published assistant:home"
    );

    // A row naming the removed PC, written after its removal, never counts.
    write_row(&mac.core, &id(&pc), "person", presence::now_local());
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(mac.core.assistant_home(), None);
    assert!(mac.core.set_assistant_home(&id(&pc)).is_err());

    // The PC2, connected, claims; the Mac's old claim stays gone.
    let _r2 = able(&pc2);
    wait_for(WAIT, "the PC2 claims and the Mac agrees", || {
        home_is(&pc2, &id(&pc2)) && home_is(&mac, &id(&pc2))
    });
    assert!(pc2.core.is_assistant_home());
    mac.core.close().unwrap();
    pc2.core.close().unwrap();
    pc.core.close().unwrap();
}

/// A Core that is connected to another of the person's devices but hasn't
/// caught up with it doesn't claim: the other's rows may hold the claim.
#[test]
fn a_core_that_is_not_caught_up_does_not_claim() {
    let (skew_mac, skew_pc) = (Arc::new(AtomicI64::new(0)), Arc::new(AtomicI64::new(0)));
    let sks = [skew_mac.clone(), skew_pc.clone()];
    let mut devs = person_with("stephen", 2, |i| Some(skewed(sks[i].clone())));
    let (pc, mac) = (devs.pop().unwrap(), devs.pop().unwrap());
    // The Mac's clock runs ten minutes ahead: the PC holds its rows back
    // until its own clock gets there, so it is connected but behind.
    skew_mac.store(600, Ordering::SeqCst);
    let _rm = able(&mac);
    wait_for(WAIT, "the Mac claims for itself", || {
        home_is(&mac, &id(&mac))
    });
    wait_for(WAIT, "the PC is connected to the Mac", || {
        pc.core
            .mesh()
            .devices(false)
            .iter()
            .any(|r| r.device_id == id(&mac) && r.state == "connected")
    });
    let _rp = able(&pc);
    holds_for(14, "the PC claimed while behind the Mac", || {
        home_rows(&pc.core).iter().all(|(w, _)| *w != id(&pc))
            && !pc.core.is_assistant_home()
    });
    // Its clock catches up: the Mac's claim lands, and it is the answer.
    skew_pc.store(600, Ordering::SeqCst);
    wait_for(WAIT, "the PC has the Mac's claim", || home_is(&pc, &id(&mac)));
    assert!(home_rows(&pc.core).iter().all(|(w, _)| *w != id(&pc)));
    mac.core.close().unwrap();
    pc.core.close().unwrap();
}

/// The status of an assistant's computer that doesn't exist yet reads the
/// `spawn.host` rows: hand-written here, `assistantChat` included.
#[test]
fn the_status_with_no_computer_reads_the_host_rows() {
    let mut devs = person("stephen", 1);
    let d = devs.pop().unwrap();
    let me = id(&d);
    let none = |d: &Dev| d.core.assistant_home_status().none;
    let host = |turn: bool, chat: bool| {
        d.core
            .store()
            .append(
                "irori",
                kinds::SPAWN_HOST,
                &SpawnHost {
                    device: me.clone(),
                    assistant_turn: turn,
                    assistant_chat: chat,
                    no_agents: true,
                    ..SpawnHost::default()
                },
            )
            .unwrap();
    };
    // Settle the sweep's own first row before writing ours.
    std::thread::sleep(Duration::from_secs(6));
    host(false, false);
    assert_eq!(none(&d), "no_claude");
    host(true, false);
    assert_eq!(none(&d), "needs_update");
    host(true, true);
    assert_eq!(none(&d), "setting_up");
    host(false, true);
    assert_eq!(none(&d), "no_claude");
    let st = d.core.assistant_home_status();
    assert!(st.device.is_empty() && !st.here);
    d.core.close().unwrap();
}
