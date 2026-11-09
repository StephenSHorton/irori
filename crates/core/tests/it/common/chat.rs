//! The one assistant in tests (one-assistant cards 1 and 3): a computer that
//! can run it, the chat worker's fake turns, and one person's devices that
//! can be kept apart and then meet.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gocompat::time::GoTime;
use irori_core::assistant_chat::{ChatDone, ChatTurnFn};
use irori_core::spawn::{SpawnStarted, Spawner};
use irori_core::{Core, Options};
use store::{Filter, kinds};

use super::mesh::{Dev, Edit, free_udp};
use super::wait_for;

/// A spawner that can start `claude` while `ready` holds.
pub fn spawner(ready: &Arc<AtomicBool>) -> Spawner {
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
        stop: None,
    }
}

/// A chat turn that answers "ok" at once.
pub fn idle_turn() -> ChatTurnFn {
    Arc::new(|_, _| ChatDone {
        text: "ok".into(),
        ..ChatDone::default()
    })
}

/// `core` can be the assistant's computer: Claude Code is found and the
/// chat turn is installed. Flip the flag (and set the spawner again) to
/// take Claude Code away.
pub fn able(core: &Core) -> Arc<AtomicBool> {
    let ready = Arc::new(AtomicBool::new(true));
    core.set_chat_turn(Some(idle_turn()));
    core.set_spawner(Some(spawner(&ready)));
    ready
}

/// Waits `secs` and checks `cond` held the whole time.
pub fn holds_for(secs: u64, what: &str, cond: impl Fn() -> bool) {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    while std::time::Instant::now() < end {
        assert!(cond(), "{what}");
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// A clock that runs `skew` seconds ahead (behind, when negative).
pub fn skewed(skew: Arc<AtomicI64>) -> Edit {
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

// ---- Devices that can be kept apart, then meet ----

/// Who a device's LAN lookup dials.
pub type Links = Arc<Mutex<Vec<SocketAddr>>>;

pub fn start_dyn(
    data: &std::path::Path,
    cl: &std::path::Path,
    lan: SocketAddr,
    links: Links,
    edit: Option<&Edit>,
) -> Core {
    let mut o = super::options(data, cl);
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
    if let Some(e) = edit {
        e(&mut o);
    }
    Core::start(o).unwrap()
}

fn dev_dyn(
    data: tempfile::TempDir,
    name: &str,
    sess: &str,
    lan: SocketAddr,
    links: Links,
    edit: Option<Edit>,
) -> Dev {
    let (cl, w) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let sessions = cl.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    super::write_session(&sessions, "900001.json", sess, name, w.path());
    let core = start_dyn(data.path(), cl.path(), lan, links, edit.as_ref());
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
        edit,
        _dirs: vec![data, cl, w],
    }
}

/// One person on `n` desktops that know each other but can't reach each
/// other until [`meet`] or [`link`]. The first certified the rest.
pub fn apart(who: &str, n: usize) -> (Vec<Dev>, Vec<Links>, Vec<SocketAddr>) {
    apart_with(who, n, |_| None)
}

/// [`apart`], with `edit(i)` applied to device `i`'s options.
pub fn apart_with(
    who: &str,
    n: usize,
    edit: impl Fn(usize) -> Option<Edit>,
) -> (Vec<Dev>, Vec<Links>, Vec<SocketAddr>) {
    let lans: Vec<SocketAddr> = (0..n).map(|_| free_udp()).collect();
    let links: Vec<Links> = (0..n).map(|_| Links::default()).collect();
    let sess = |i: usize| format!("{:08x}-1111-4111-8111-{:012x}", i + 1, i + 1);
    let first = dev_dyn(
        tempfile::tempdir().unwrap(),
        &format!("{who}1"),
        &sess(0),
        lans[0],
        links[0].clone(),
        edit(0),
    );
    let mut devs = vec![first];
    for i in 1..n {
        let d = tempfile::tempdir().unwrap();
        std::fs::copy(
            devs[0].data.join(irori_identity::PERSON_FILE),
            d.path().join(irori_identity::PERSON_FILE),
        )
        .unwrap();
        let name = format!("{who}{}", i + 1);
        let next = dev_dyn(d, &name, &sess(i), lans[i], links[i].clone(), edit(i));
        let add = devs[0]
            .core
            .identity()
            .cert_for(
                next.core.device_id(),
                &hex::encode(next.core.identity().device_pub()),
                &name,
                irori_identity::ROLE_DESKTOP,
                presence::now_local(),
            )
            .unwrap();
        devs[0]
            .core
            .store()
            .append("irori", kinds::PERSON_DEVICE_ADDED, &add)
            .unwrap();
        devs.push(next);
    }
    let rows = devs[0]
        .core
        .store()
        .list(&Filter {
            kinds: vec!["person.".into()],
            ..Filter::default()
        })
        .unwrap();
    for d in &devs[1..] {
        for _ in 0..2 {
            for ev in &rows {
                let _ = d.core.chain().learn(ev.into());
            }
        }
    }
    wait_for(45, "the devices own each other's keys", || {
        devs.iter().all(|x| {
            devs.iter()
                .all(|y| x.core.is_owner(&format!("user:{}", y.core.device_id())))
        })
    });
    // Nothing is dialed: each one's lookup has no targets yet.
    (devs, links, lans)
}

/// Every device dials every other.
pub fn meet(links: &[Links], lans: &[SocketAddr]) {
    let pairs: Vec<(usize, usize)> = (0..links.len())
        .flat_map(|i| {
            (0..links.len())
                .filter(move |j| *j != i)
                .map(move |j| (i, j))
        })
        .collect();
    link(links, lans, &pairs);
}

/// Device `i` dials device `j`, for each `(i, j)`, on top of what each
/// already dials.
pub fn link(links: &[Links], lans: &[SocketAddr], pairs: &[(usize, usize)]) {
    for (i, j) in pairs {
        let mut l = links[*i].lock().unwrap();
        if !l.contains(&lans[*j]) {
            l.push(lans[*j]);
        }
    }
}
