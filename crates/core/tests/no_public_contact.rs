//! D3/D13 on the Core's own transport settings (mesh.md §6
//! `TestICENeverListsPublicSTUN`, `TestNothingOpensBeforeTheFirstJoin`):
//! a Core with default settings opens nothing while it is alone, starts
//! the LAN lookup only when something needs the network (a pairing window
//! here, Go roomnet `start`), and its endpoint names no public relay, STUN
//! (QAD) server, DNS resolver or n0 address lookup.
//!
//! The transport is [`devices::default_net_config`] with only the sockets
//! moved to loopback (IP transport on 127.0.0.1, the LAN lookup bound and
//! sending on 127.0.0.1). On Windows a socket guard then reads this
//! process's rows from `netstat` and requires every local and remote
//! address to be loopback. Alone in its file so this process holds no
//! other test's sockets.

mod common;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iroh::Watcher as _;
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
            start.elapsed() < Duration::from_secs(30),
            "timed out waiting for: {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// This process's socket rows from `netstat -ano`: (proto, local, remote).
#[cfg(windows)]
fn own_sockets() -> Vec<(String, String, String)> {
    let out = std::process::Command::new("netstat")
        .arg("-ano")
        .output()
        .expect("netstat");
    let pid = std::process::id().to_string();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            match f.as_slice() {
                [p @ "TCP", local, remote, _, id] | [p @ "UDP", local, remote, id]
                    if *id == pid =>
                {
                    Some((p.to_string(), local.to_string(), remote.to_string()))
                }
                _ => None,
            }
        })
        .collect()
}

/// `127.0.0.1:5`, `[::1]:5`, `[::1%1]:5` or `*:*` as an IP (`None` for `*`).
#[cfg(windows)]
fn ip_of(addr: &str) -> Option<std::net::IpAddr> {
    let (host, _) = addr.rsplit_once(':')?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let host = host.split('%').next().unwrap_or(host);
    host.parse().ok()
}

#[test]
fn a_default_core_opens_nothing_alone_and_never_names_a_public_server() {
    let (data, claude) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (lan_at, lan_to) = (free_udp(), free_udp());
    let net_slot: Arc<Mutex<Option<IrohNet>>> = Arc::default();
    let mut o = common::options(data.path(), claude.path());
    {
        let slot = net_slot.clone();
        o.pair_net = Some(Arc::new(move |ident, chain, guests| {
            let mut c = devices::default_net_config(ident, chain, guests);
            // The defaults this test keeps: every IPv4 interface, the LAN
            // lookup on its fixed port with a wildcard bind and broadcasts.
            assert_eq!(c.ip, IpMode::AnyV4);
            assert!(c.lan.is_none(), "no lookup is started up front");
            let lc = c
                .lan_config
                .as_mut()
                .expect("the default carries a LAN lookup");
            assert_eq!(lc.port, irori_lan::LAN_PORT);
            assert!(lc.bind.is_none() && lc.targets.is_none());
            // Only the sockets move to loopback.
            c.ip = IpMode::Loopback;
            lc.bind = Some(lan_at);
            lc.targets = Some(Arc::new(move || vec![lan_to]));
            let n = IrohNet::new(c);
            *slot.lock().unwrap() = Some(n.clone());
            n
        }));
    }
    let core = Core::start(o).unwrap();
    let net = net_slot
        .lock()
        .unwrap()
        .clone()
        .expect("the Core built its net");
    let rt = core.pairing_runtime().clone();

    // The home server is off by default (no UPnP, no NAT-PMP, no relay).
    let hs = core.home_server();
    assert!(!hs.on && !hs.running, "{hs:?}");

    // Alone: nothing binds, not the endpoint and not UDP 38419.
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        core.net().endpoint().is_none(),
        "a lone device bound an endpoint"
    );
    assert!(
        core.net().lan().is_none(),
        "a lone device started the LAN lookup"
    );
    assert_eq!(core.net().lan_error(), "");
    #[cfg(windows)]
    assert_eq!(own_sockets(), vec![], "a lone device holds sockets");

    // A pairing window needs the network: the endpoint binds and the LAN
    // lookup starts with it.
    rt.block_on(devices::add_device(&core, irori_identity::ROLE_DESKTOP))
        .unwrap();
    wait("the LAN lookup binds", || {
        core.net().lan().and_then(|l| l.local_addr()) == Some(lan_at)
    });
    let ep = core
        .net()
        .endpoint()
        .expect("the window bound the endpoint")
        .clone();
    // Let iroh's own start-up (net report, relay actor, address publish) run.
    std::thread::sleep(Duration::from_secs(3));

    // No relay, so no QAD probe and no relay connection: public or n0's.
    assert!(ep.home_relay_status().get().is_empty());
    assert_eq!(ep.addr().relay_urls().count(), 0);
    // Address lookups: the address book, the LAN lookup and iroh-gossip's
    // in-memory one (fed by gossip messages); no DNS, pkarr or mDNS.
    assert_eq!(ep.address_lookup().unwrap().len(), 3);
    for a in ep.bound_sockets() {
        assert!(a.ip().is_loopback(), "non-loopback endpoint socket {a}");
    }
    // Nothing asked the resolver, and it is Irori's: no TXT, no n0 names.
    assert!(
        net.dns_log().asked().is_empty(),
        "{:?}",
        net.dns_log().asked()
    );
    let dns = ep.dns_resolver().unwrap().clone();
    rt.block_on(async {
        let t = Duration::from_secs(2);
        assert!(
            dns.lookup_txt("_iroh.example.dns.iroh.link", t)
                .await
                .is_err()
        );
        assert!(
            dns.lookup_ipv4("use1-1.relay.n0.iroh.link", t)
                .await
                .is_err()
        );
        assert!(dns.lookup_ipv4("dns.iroh.link", t).await.is_err());
    });

    #[cfg(windows)]
    {
        let rows = own_sockets();
        assert!(
            !rows.is_empty(),
            "netstat shows none of this process's sockets"
        );
        for (p, local, remote) in &rows {
            let l = ip_of(local);
            assert!(
                l.is_some_and(|ip| ip.is_loopback()),
                "{p} socket on a non-loopback address: {local} -> {remote}"
            );
            if let Some(r) = ip_of(remote) {
                assert!(
                    r.is_loopback() || r.is_unspecified(),
                    "{p} socket talks to {remote}"
                );
            }
        }
        let lan = format!("{lan_at}");
        assert!(
            rows.iter().any(|(p, l, _)| p == "UDP" && *l == lan),
            "the LAN lookup's socket is missing: {rows:?}"
        );
    }

    core.pairing().cancel();
    core.close().unwrap();
}
