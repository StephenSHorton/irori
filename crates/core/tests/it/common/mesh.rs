//! Cores on loopback iroh, as the five-Core harness started them (card
//! #48): one person on `n` devices paired as pairing does, each device a
//! Core with one live Claude session. Shared by five_cores.rs and
//! spawn_devices.rs (card #455), which also edits a device's options (its
//! clock) and keeps that edit across a restart.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use irori_core::{Core, Options};
use store::Filter;
use store::kinds::{self, PERSON_ACTOR_PREFIX};

use super::wait_for;

const PAIR_WAIT: u64 = 45;

/// An edit to a Core's options before it starts (its clock, say).
pub type Edit = Arc<dyn Fn(&mut Options) + Send + Sync>;

pub fn free_udp() -> SocketAddr {
    std::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
}

/// One device: a Core with one live Claude session.
pub struct Dev {
    pub core: Core,
    pub agent: String,
    pub data: std::path::PathBuf,
    pub claude: std::path::PathBuf,
    pub lan: SocketAddr,
    pub to: Vec<SocketAddr>,
    pub edit: Option<Edit>,
    pub _dirs: Vec<tempfile::TempDir>,
}

impl Dev {
    pub fn user(&self) -> String {
        self.core.user_sender().key
    }

    pub fn person_actor(&self) -> String {
        format!("{PERSON_ACTOR_PREFIX}{}", self.core.person().id)
    }

    pub fn tool(&self, name: &str, args: serde_json::Value) -> String {
        let caller = tools::Caller {
            sender: inbox::Sender::new(&self.agent, "agent", "claude"),
            path: String::new(),
        };
        self.core
            .tools()
            .call(&caller, name, &serde_json::to_vec(&args).unwrap())
            .unwrap()
    }
}

/// A Core whose mesh runs on loopback iroh with a LAN lookup unicast to
/// `to` (as tests/pair.rs), and one live session `sess`.
pub fn dev(
    data: tempfile::TempDir,
    name: &str,
    sess: &str,
    lan: SocketAddr,
    to: Vec<SocketAddr>,
    edit: Option<Edit>,
) -> Dev {
    let (cl, w) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let sessions = cl.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    super::write_session(&sessions, "900001.json", sess, name, w.path());
    let core = start(data.path(), cl.path(), lan, to.clone(), edit.as_ref());
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
        to,
        edit,
        _dirs: vec![data, cl, w],
    }
}

/// `d` stopped and started again on the same data and sessions (and the
/// same options edit): its boards' logs replay from disk, as at a restart
/// of the app.
pub fn restart(d: Dev) -> Dev {
    let Dev {
        core,
        agent,
        data,
        claude,
        lan,
        to,
        edit,
        _dirs,
    } = d;
    core.close().unwrap();
    let core = start(&data, &claude, lan, to.clone(), edit.as_ref());
    wait_for(60, "presence lists the session again", || {
        core.local_agents().iter().any(|a| a.key == agent)
    });
    Dev {
        core,
        agent,
        data,
        claude,
        lan,
        to,
        edit,
        _dirs,
    }
}

/// A Core on `data` and Claude dir `cl` whose mesh runs on loopback iroh
/// with a LAN lookup on `lan` unicast to `to`.
pub fn start(
    data: &std::path::Path,
    cl: &std::path::Path,
    lan: SocketAddr,
    to: Vec<SocketAddr>,
    edit: Option<&Edit>,
) -> Core {
    let mut o = super::options(data, cl);
    o.pair_net = Some(Arc::new(move |ident, chain, guests| {
        let mut nc = irori_core::devices::default_net_config(ident, chain, guests);
        nc.ip = irori_net::endpoint::IpMode::Loopback;
        nc.gossip = false;
        let lc = nc.lan_config.as_mut().expect("a LAN lookup");
        lc.bind = Some(lan);
        let to = to.clone();
        lc.targets = Some(Arc::new(move || to.clone()));
        lc.every = Duration::from_millis(200);
        irori_net::transport::IrohNet::new(nc)
    }));
    if let Some(e) = edit {
        e(&mut o);
    }
    Core::start(o).unwrap()
}

/// One person on `n` devices: the first certifies the others as pairing
/// does, and the call returns once every device owns every other's key.
pub fn person(who: &str, n: usize) -> Vec<Dev> {
    person_with(who, n, |_| None)
}

/// [`person`] with device i's options edited by `edit(i)`.
pub fn person_with(who: &str, n: usize, edit: impl Fn(usize) -> Option<Edit>) -> Vec<Dev> {
    let lans: Vec<SocketAddr> = (0..n).map(|_| free_udp()).collect();
    let others = |i: usize| -> Vec<SocketAddr> {
        lans.iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, a)| *a)
            .collect()
    };
    let sess = |i: usize| format!("{:08x}-1111-4111-8111-{:012x}", i + 1, i + 1);
    let first = dev(
        tempfile::tempdir().unwrap(),
        &format!("{who}1"),
        &sess(0),
        lans[0],
        others(0),
        edit(0),
    );
    let mut devs = vec![first];
    for (i, lan) in lans.iter().enumerate().skip(1) {
        let d = tempfile::tempdir().unwrap();
        std::fs::copy(
            devs[0].data.join(irori_identity::PERSON_FILE),
            d.path().join(irori_identity::PERSON_FILE),
        )
        .unwrap();
        let next = dev(
            d,
            &format!("{who}{}", i + 1),
            &sess(i),
            *lan,
            others(i),
            edit(i),
        );
        let add = devs[0]
            .core
            .identity()
            .cert_for(
                next.core.device_id(),
                &hex::encode(next.core.identity().device_pub()),
                &format!("{who}{}", i + 1),
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
            ..Default::default()
        })
        .unwrap();
    for d in &devs[1..] {
        for _ in 0..2 {
            for ev in &rows {
                let _ = d.core.chain().learn(ev.into());
            }
        }
        d.core.mesh().kick();
    }
    wait_for(PAIR_WAIT, &format!("{who}'s devices pair"), || {
        devs.iter().all(|x| {
            devs.iter()
                .all(|y| x.core.is_owner(&format!("user:{}", y.core.device_id())))
        })
    });
    devs
}

/// Alice's board, with Bob invited from a link and the words checked.
pub fn invited(a: &Dev, b: &Dev) -> String {
    let id = a.core.create_board("MLG farming", "mlg").unwrap();
    let ra = a.core.pairing_runtime().clone();
    let link = ra.block_on(a.core.invite(&id)).unwrap().link;
    let rb = b.core.pairing_runtime().clone();
    rb.block_on(b.core.join_invite(&link)).unwrap();
    wait_for(PAIR_WAIT, "both screens show the words", || {
        a.core.invite_status().status.state == "words"
            && b.core.invite_status().status.state == "words"
    });
    let words = b.core.invite_status().status.words.join(" ");
    a.core.confirm_invite_words(&words).unwrap();
    wait_for(PAIR_WAIT, "the invite is done on both sides", || {
        a.core.invite_status().status.state == "done"
            && b.core.invite_status().status.state == "done"
    });
    assert!(b.core.boards().contains(&id));
    id
}
