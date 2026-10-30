//! M7's check on one machine: a fresh dev data dir pairs with another
//! Core's fixture identity (file keystore, scratch dirs; never a real key),
//! over iroh on 127.0.0.1 with LAN lookups that unicast to each other on
//! 127.0.0.1, through the Devices calls the window makes. After pairing,
//! the two Cores' mesh Nodes sync over the same endpoint, found through
//! the LAN lookup alone (mesh.md §6 TwoNodesSyncOverTheLANAlone), and
//! Remove device runs on the Node.

mod common;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use irori_core::{Core, devices};
use irori_net::endpoint::IpMode;
use irori_net::transport::IrohNet;

fn free_udp() -> SocketAddr {
    std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
}

fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(45),
            "timed out waiting for: {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A Core on `data` whose pairing runs on loopback iroh and a loopback LAN:
/// the default transport ([`devices::default_net_config`], the LAN lookup
/// started when the endpoint binds) with its sockets moved to 127.0.0.1.
fn core(data: &std::path::Path, claude: &std::path::Path, lan: SocketAddr, to: SocketAddr) -> Core {
    let mut o = common::options(data, claude);
    o.pair_net = Some(Arc::new(move |ident, chain, guests| {
        let mut nc = devices::default_net_config(ident, chain, guests);
        nc.ip = IpMode::Loopback;
        let lc = nc.lan_config.as_mut().expect("a LAN lookup");
        lc.bind = Some(lan);
        lc.targets = Some(Arc::new(move || vec![to]));
        lc.every = Duration::from_millis(200);
        IrohNet::new(nc)
    }));
    Core::start(o).unwrap()
}

#[test]
fn a_fresh_data_dir_pairs_with_a_fixture_identity() {
    let (da, db, ca, cb) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let (la, lb) = (free_udp(), free_udp());
    let a = core(da.path(), ca.path(), la, lb);
    let b = core(db.path(), cb.path(), lb, la);
    let before_b = b.person();
    assert_ne!(before_b.id, a.person().id);
    let rt = a.pairing_runtime().clone();

    // The window's calls, with Go's texts.
    let st = rt
        .block_on(devices::add_device(&a, irori_identity::ROLE_DESKTOP))
        .unwrap();
    assert_eq!(st.state, irori_pair::STATE_WAITING);
    assert_eq!(
        rt.block_on(devices::join_with_code(&b, &st.code, "no port here"))
            .unwrap_err(),
        "the home server address is a host and port, like 203.0.113.7:24817"
    );
    rt.block_on(devices::join_with_code(&b, &st.code.to_lowercase(), ""))
        .unwrap();
    wait("both reach the words", || {
        a.pairing().status().state == irori_pair::STATE_CONFIRM
            && b.pairing().status().state == irori_pair::STATE_WORDS
    });
    assert_eq!(
        rt.block_on(devices::confirm_words(&a, "wrong words"))
            .unwrap_err(),
        "those aren't the words the other device shows"
    );
    let words = b.pairing().status().words.join(" ");
    rt.block_on(devices::confirm_words(&a, &words)).unwrap();
    assert_eq!(
        rt.block_on(devices::confirm_key(&b, "0000 0000 0000 0000"))
            .unwrap_err(),
        "that isn't the key the other device shows"
    );
    rt.block_on(devices::confirm_key(&b, &st.fingerprint))
        .unwrap();
    wait("both paired", || {
        a.pairing().status().state == irori_pair::STATE_DONE
            && b.pairing().status().state == irori_pair::STATE_DONE
    });
    assert_eq!(b.person().id, a.person().id, "b took another person");
    assert_eq!(b.person().root_device_id, a.device_id());
    let b_id = b.device_id().to_string();
    assert!(a.chain().cert(&b_id).is_some(), "a holds no cert for b");
    assert!(
        !a.guest_windows().any_open(),
        "a's guest window outlived the pairing"
    );
    assert!(db.path().join(irori_pair::ROWS_FILE).exists());

    // B restarts before any sync: the saved rows still certify it.
    b.close().unwrap();
    drop(b);
    let b = core(db.path(), cb.path(), lb, la);
    assert!(
        b.chain().cert(&b_id).is_some(),
        "after a restart b lost its cert"
    );
    assert_eq!(b.person().id, a.person().id);
    let rt_b = b.pairing_runtime().clone();
    assert!(
        rt_b.block_on(devices::join_with_code(&b, &irori_pair::new_code(), ""))
            .unwrap_err()
            .contains("already one of a person's devices")
    );

    // M5 on one machine: the two Cores' mesh Nodes sync over the same
    // endpoint pairing used, finding each other through the LAN lookup
    // alone (no home server, no address rows).
    let has = |c: &Core, ev: &store::Event| {
        c.store()
            .list(&store::Filter {
                device_id: ev.device_id.clone(),
                after_seq: ev.seq - 1,
                limit: 1,
                ..store::Filter::default()
            })
            .is_ok_and(|v| v.len() == 1 && v[0].seq == ev.seq)
    };
    let seen = |c: &Core, tag: &str| {
        c.store()
            .append(
                "irori",
                store::kinds::AGENT_SEEN,
                &serde_json::json!({"key": format!("{}/{tag}", c.device_id()), "agent": "a"}),
            )
            .unwrap()
    };
    let (ea, eb) = (seen(&a, "from-a"), seen(&b, "from-b"));
    wait("rows cross between the Cores over the mesh", || {
        has(&b, &ea) && has(&a, &eb)
    });

    // The roster reads the mesh's liveness (Go core.go:188): b's agent shows
    // on a with b's device connected, not offline.
    let b_agent = format!("{}/from-b", b.device_id());
    wait("a's roster sees b's agent on a live device", || {
        a.roster()
            .remote()
            .iter()
            .any(|c| c.key == b_agent && c.device_state != "offline")
    });

    // Remove device runs on the Node: a revokes b and rotates its key.
    let epoch = a.person().epoch;
    rt.block_on(a.remove_device(&b_id)).unwrap();
    assert!(a.chain().cut(&b_id).is_some(), "a did not revoke b");
    assert_eq!(a.person().epoch, epoch + 1, "a's person key did not rotate");
    a.close().unwrap();
    b.close().unwrap();
}

/// The bridge's relay (M7, `PairLan::set_relay`): while the home server
/// runs, this device's LAN lookup names its relay at the LAN address with
/// the device key, for a joiner at home whose code is an away member's;
/// once the server stops it names none.
#[test]
fn the_home_server_names_its_relay_to_the_bridge() {
    let (d, c) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (lan, to) = (free_udp(), free_udp());
    let mut o = common::options(d.path(), c.path());
    o.pair_net = Some(Arc::new(move |ident, chain, guests| {
        let mut nc = devices::default_net_config(ident, chain, guests);
        nc.ip = IpMode::Loopback;
        let lc = nc.lan_config.as_mut().expect("a LAN lookup");
        lc.bind = Some(lan);
        lc.targets = Some(Arc::new(move || vec![to]));
        IrohNet::new(nc)
    }));
    o.home = Some(Arc::new(|hc: &mut home::Config| {
        hc.bind_ip = Some(Ipv4Addr::LOCALHOST.into());
    }));
    let core = Core::start(o).unwrap();
    let rt = core.pairing_runtime().clone();
    core.set_home_server(true, false, "").unwrap();
    assert_eq!(core.net().pair_lan().relay(), None, "named before it runs");

    // An open pairing window is the §4.5 condition: the server starts.
    rt.block_on(devices::add_device(&core, irori_identity::ROLE_DESKTOP))
        .unwrap();
    let port = {
        let mut port = None;
        wait("the home server runs", || {
            port = core.home().and_then(|h| h.relay_port());
            port.is_some()
        });
        port.unwrap()
    };
    let key = iroh::EndpointId::from_bytes(&core.identity().device_pub()).unwrap();
    wait("the bridge names the relay", || {
        core.net().pair_lan().relay()
            == Some((
                irori_net::relay::relay_url(Ipv4Addr::LOCALHOST.into(), port),
                hex::encode(key.as_bytes()),
            ))
    });

    // The window closes and no other device is certified: it stops.
    core.pairing().cancel();
    wait("the home server stops", || {
        core.home().and_then(|h| h.relay_port()).is_none()
    });
    wait("the bridge names no relay", || {
        core.net().pair_lan().relay().is_none()
    });
    core.close().unwrap();
}

/// The address under the pairing code (Go core.go:288 `ServerAddr`,
/// home.go:107): while the home server runs, Add's status carries where a
/// device away from home dials it; with no server running, nothing.
#[test]
fn add_shows_the_home_server_address() {
    let (d, c) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (lan, to) = (free_udp(), free_udp());
    let mut o = common::options(d.path(), c.path());
    o.pair_net = Some(Arc::new(move |ident, chain, guests| {
        let mut nc = devices::default_net_config(ident, chain, guests);
        nc.ip = IpMode::Loopback;
        let lc = nc.lan_config.as_mut().expect("a LAN lookup");
        lc.bind = Some(lan);
        lc.targets = Some(Arc::new(move || vec![to]));
        IrohNet::new(nc)
    }));
    o.home = Some(Arc::new(|hc: &mut home::Config| {
        hc.bind_ip = Some(Ipv4Addr::LOCALHOST.into());
    }));
    let core = Core::start(o).unwrap();
    let rt = core.pairing_runtime().clone();

    // No server: the window opens with no address under the code.
    let st = rt
        .block_on(devices::add_device(&core, irori_identity::ROLE_DESKTOP))
        .unwrap();
    assert_eq!(st.state, irori_pair::STATE_WAITING);
    assert_eq!(st.server_addr, "", "an address with no server running");
    core.pairing().cancel();

    // A typed public address: the open window starts the server, and the
    // address follows it once it runs.
    core.set_home_server(true, false, "203.0.113.7:24817")
        .unwrap();
    rt.block_on(devices::add_device(&core, irori_identity::ROLE_DESKTOP))
        .unwrap();
    wait("the home server runs", || {
        core.home().is_some_and(|h| h.status().running)
    });
    let want = core.home().unwrap().status().address;
    assert!(!want.is_empty(), "the running server names no address");
    wait("Add's status carries the address", || {
        core.pairing().status().server_addr == want
    });
    core.pairing().cancel();
    core.close().unwrap();
}
