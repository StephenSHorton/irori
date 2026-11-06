//! Two devices of one person (Go pairedCores, newHCore): Cores whose
//! meshes run on loopback iroh, B certified by A as pairing does. Shared by
//! tests/boards.rs and tests/spawn_settings_devices.rs.

use std::time::Duration;

use irori_core::Core;
use store::Filter;
use store::kinds;

use super::wait_for;

/// One device of the pair: its Core and its one live session.
pub struct Dev {
    pub core: Core,
    pub agent: String,
    pub data: std::path::PathBuf,
    _dirs: Vec<tempfile::TempDir>,
}

impl Dev {
    pub fn user(&self) -> String {
        self.core.user_sender().key
    }
}

pub fn free_udp() -> std::net::SocketAddr {
    std::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
}

/// A Core with one live session `name`/`sess`, whose mesh runs on loopback
/// iroh with a LAN lookup unicast to `to` (as tests/pair.rs).
pub fn dev(
    data: tempfile::TempDir,
    name: &str,
    sess: &str,
    lan: std::net::SocketAddr,
    to: std::net::SocketAddr,
) -> Dev {
    let (cl, w) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let sessions = cl.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    super::write_session(&sessions, "900001.json", sess, name, w.path());
    let mut o = super::options(data.path(), cl.path());
    o.pair_net = Some(std::sync::Arc::new(move |ident, chain, guests| {
        let mut nc = irori_core::devices::default_net_config(ident, chain, guests);
        nc.ip = irori_net::endpoint::IpMode::Loopback;
        let lc = nc.lan_config.as_mut().expect("a LAN lookup");
        lc.bind = Some(lan);
        lc.targets = Some(std::sync::Arc::new(move || vec![to]));
        lc.every = Duration::from_millis(200);
        irori_net::transport::IrohNet::new(nc)
    }));
    let core = Core::start(o).unwrap();
    let agent = format!("{}/{sess}", core.device_id());
    wait_for(10, "presence lists the session", || {
        core.local_agents().iter().any(|a| a.key == agent)
    });
    Dev {
        core,
        agent,
        data: data.path().to_path_buf(),
        _dirs: vec![data, cl, w],
    }
}

/// Two Cores of one person, B certified by A as pairing does, waiting
/// until each owns the other's user key.
pub fn paired() -> (Dev, Dev) {
    let (la, lb) = (free_udp(), free_udp());
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = dev(da, "alpha", "aaaaaaaa-1111-4111-8111-00000000000a", la, lb);
    std::fs::copy(
        a.data.join(irori_identity::PERSON_FILE),
        db.path().join(irori_identity::PERSON_FILE),
    )
    .unwrap();
    let b = dev(db, "beta", "bbbbbbbb-2222-4222-8222-00000000000b", lb, la);
    let add = a
        .core
        .identity()
        .cert_for(
            b.core.device_id(),
            &hex::encode(b.core.identity().device_pub()),
            "mac",
            irori_identity::ROLE_DESKTOP,
            presence::now_local(),
        )
        .unwrap();
    a.core
        .store()
        .append("irori", kinds::PERSON_DEVICE_ADDED, &add)
        .unwrap();
    let person = a
        .core
        .store()
        .list(&Filter {
            kinds: vec!["person.".into()],
            ..Default::default()
        })
        .unwrap();
    for _ in 0..2 {
        for ev in &person {
            let _ = b.core.chain().learn(ev.into());
        }
    }
    b.core.mesh().kick();
    wait_for(45, "a and b pair", || {
        a.core.is_owner(&format!("user:{}", b.core.device_id()))
            && b.core.is_owner(&format!("user:{}", a.core.device_id()))
    });
    (a, b)
}
