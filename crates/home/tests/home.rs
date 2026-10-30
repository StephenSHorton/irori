//! Ports of I:internal/home/home_test.go and awake_test.go. Every listener
//! is on 127.0.0.1, the router is a fake gateway in process, the awake
//! holder is a counting fake, and nothing reaches the internet. Identities
//! are fresh fixtures in temp dirs (file keystore), never a real device key.

mod common;

use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use common::{Node, eventually};
use home::Server;
use home::nodeaddr::{REACH_CGNAT, REACH_OFF, REACH_OK, REACH_UNMAPPED};
use home::server::{REACH_FORWARD, REACH_HOME, REACH_OUTSIDE, REACH_SHARED};
use home::settings::{self, PORT_MAX, PORT_MIN, Settings};

/// The server starts, maps its port, and writes node.addr once; renewals
/// and re-checks with nothing changed write nothing more. Switching it off
/// writes one "off" row and removes the mappings.
#[tokio::test(flavor = "multi_thread")]
async fn server_writes_its_address_only_on_change() {
    let mut n = Node::new(|_| {});
    n.run();
    n.srv.set(true, true, "").unwrap();
    eventually("a reachable node.addr", || {
        let r = n.rows();
        r.len() == 1 && r[0].reach == REACH_OK
    })
    .await;
    let a = n.rows()[0].clone();
    let port = n.srv.settings().port;
    assert!((i64::from(PORT_MIN)..=i64::from(PORT_MAX)).contains(&port));
    let p = a.public.clone().unwrap();
    assert!(
        a.port == port
            && a.via == "upnp"
            && p.ip == "203.0.113.7"
            && p.tcp == port
            && p.udp == port
            && a.lan == vec!["127.0.0.1".to_string()],
        "node.addr {a:?} (port {port})"
    );
    let st = n.srv.status();
    assert!(
        st.running
            && st.reach == REACH_OUTSIDE
            && st.address == format!("203.0.113.7:{port}")
            && st.line.is_empty(),
        "status {st:?}"
    );
    tokio::time::sleep(Duration::from_millis(300)).await; // several re-checks
    n.srv.kick();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(n.rows().len(), 1, "rows with nothing changed");
    assert_eq!(n.gw.count(), 2, "want TCP and UDP mapped");
    n.srv.set(false, true, "").unwrap();
    eventually("an off row", || {
        let r = n.rows();
        r.len() == 2 && r[1].reach == REACH_OFF && r[1].dial().is_empty()
    })
    .await;
    eventually("the mappings removed", || n.gw.count() == 0).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(n.rows().len(), 2, "rows after switching off");
    assert!(!n.srv.status().running, "still running after switching off");
    // The off row's exact bytes (Go: public null, via "").
    let raw = &n
        .st
        .list(&store::Filter {
            kinds: vec![store::kinds::NODE_ADDR.into()],
            desc: true,
            limit: 1,
            ..Default::default()
        })
        .unwrap()[0]
        .payload;
    let at = n.rows()[1].at;
    assert_eq!(
        String::from_utf8_lossy(raw),
        format!(r#"{{"v":1,"public":null,"via":"","reach":"off","at":{at}}}"#)
    );
    n.shutdown().await;
}

/// A restart with nothing changed writes no new row: the comparison is with
/// the newest row in the log, not with memory.
#[tokio::test(flavor = "multi_thread")]
async fn restart_writes_no_new_row() {
    let n = Node::new(|_| {});
    n.srv.set(true, true, "").unwrap();
    for _ in 0..300 {
        n.srv.reconcile().await; // no Run loop: the test drives it
        if n.rows().len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(n.rows().len(), 1);
    n.srv.stop().await;
    let again = Server::new(n.cfg.clone());
    for _ in 0..300 {
        again.reconcile().await;
        if again.status().reach == REACH_OUTSIDE {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(again.status().reach, REACH_OUTSIDE);
    again.reconcile().await;
    again.stop().await;
    assert_eq!(n.rows().len(), 1, "rows after a restart: {:?}", n.rows());
}

/// The Devices lines on the server come from the mapper's states.
#[tokio::test(flavor = "multi_thread")]
async fn status_mapping_failed() {
    let mut n = Node::new(|_| {});
    n.gw.set_fail("no IGD");
    n.run();
    n.srv.set(true, true, "").unwrap();
    eventually("an unmapped row", || {
        let r = n.rows();
        r.len() == 1 && r[0].reach == REACH_UNMAPPED && r[0].public.is_none()
    })
    .await;
    let st = n.srv.status();
    assert!(
        st.reach == REACH_FORWARD
            && st.line.contains(&format!(
                "forward TCP and UDP port {} to 127.0.0.1",
                st.port
            ))
            && st.address.is_empty(),
        "status {st:?}"
    );
    n.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn status_cgnat() {
    let mut n = Node::new(|_| {});
    n.gw.st.lock().unwrap().ext = Some("100.64.3.4".parse().unwrap());
    n.run();
    n.srv.set(true, true, "").unwrap();
    eventually("a cgnat row", || {
        let r = n.rows();
        r.len() == 1 && r[0].reach == REACH_CGNAT && r[0].outside().is_empty()
    })
    .await;
    let st = n.srv.status();
    assert!(
        st.reach == REACH_SHARED && st.line.contains("CGNAT"),
        "status {st:?}"
    );
    n.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn status_home_only() {
    let mut n = Node::new(|_| {});
    n.run();
    n.srv.set(true, false, "").unwrap();
    eventually("an unmapped row", || {
        let r = n.rows();
        r.len() == 1 && r[0].reach == REACH_UNMAPPED
    })
    .await;
    assert_eq!(n.gw.count(), 0, "mapped with outside reach off");
    let st = n.srv.status();
    assert!(
        st.reach == REACH_HOME && st.line.is_empty(),
        "status {st:?}"
    );
    n.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn status_manual_address() {
    let mut n = Node::new(|_| {});
    n.run();
    n.srv.set(true, true, "pc.example:443").unwrap();
    eventually("a manual row", || {
        let r = n.rows();
        r.len() == 1 && r[0].via == "manual" && r[0].outside() == "pc.example:443"
    })
    .await;
    assert_eq!(n.gw.count(), 0, "mapped with a manual address");
    let st = n.srv.status();
    assert!(
        st.reach == REACH_OUTSIDE && st.address == "pc.example:443",
        "status {st:?}"
    );
    assert!(
        n.srv.set(true, true, "not an address/").is_err(),
        "a bad address was taken"
    );
    n.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn status_public_network() {
    let mut n = Node::new(|c| c.on_public_network = Some(Arc::new(|_| true)));
    n.run();
    n.srv.set(true, false, "").unwrap();
    eventually("running", || n.srv.status().running).await;
    let st = n.srv.status();
    assert!(st.line.contains("Public"), "status {st:?}");
    n.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn status_port_taken() {
    let busy = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let old = i64::from(busy.local_addr().unwrap().port());
    let mut n = Node::new(|_| {});
    settings::save(
        &n.dir,
        &Settings {
            on: true,
            port: old,
            ..Default::default()
        },
    )
    .unwrap();
    n.srv = Server::new(n.cfg.clone());
    n.run();
    eventually("a row on a new port", || {
        let r = n.rows();
        r.len() == 1 && r[0].port != old
    })
    .await;
    let st = n.srv.status();
    assert!(
        st.line.contains(&format!("Port {old} was taken")) && st.port != old,
        "status {st:?}"
    );
    assert_eq!(
        settings::load(&n.dir).unwrap().port,
        st.port,
        "the new port isn't saved"
    );
    drop(busy);
    n.shutdown().await;
}

/// Global IPv6 addresses go into node.addr only while "Reachable from
/// outside home" is on.
#[tokio::test(flavor = "multi_thread")]
async fn ipv6_only_with_outside() {
    let v6: Ipv6Addr = "2001:db8::7".parse().unwrap();
    let mut n = Node::new(|c| c.v6_addrs = Some(Arc::new(move || vec![v6])));
    n.run();
    n.srv.set(true, false, "").unwrap();
    eventually("a home-only row", || {
        let r = n.rows();
        r.len() == 1 && r[0].reach == REACH_UNMAPPED
    })
    .await;
    assert!(n.rows()[0].v6.is_empty(), "home-only row lists IPv6");
    n.srv.set(true, true, "").unwrap();
    eventually("a row with IPv6", || {
        let r = n.rows();
        r.len() == 2 && r[1].v6 == vec![v6.to_string()]
    })
    .await;
    n.shutdown().await;
}

/// A server with no other device and no pairing window binds nothing.
#[tokio::test(flavor = "multi_thread")]
async fn server_waits_for_another_device() {
    let mut n = Node::new(|_| {});
    n.wanted.store(false, Ordering::SeqCst);
    n.run();
    n.srv.set(true, true, "").unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let st = n.srv.status();
    assert!(
        !st.running && n.rows().is_empty() && n.gw.count() == 0,
        "a lone server started: {st:?}"
    );
    n.wanted.store(true, Ordering::SeqCst);
    n.srv.kick();
    eventually("it starts once wanted", || n.srv.status().running).await;
    n.shutdown().await;
}

/// The hold is taken when the server starts serving, once however often it
/// re-checks, released when the setting goes off and taken again when it
/// comes back on, and released when the server stops.
#[tokio::test(flavor = "multi_thread")]
async fn keep_awake_while_serving() {
    let n = Node::new(|_| {});
    let want = |what: &str, a: usize, r: usize| {
        assert_eq!(n.awake.counts(), (a, r), "{what}");
    };
    assert!(n.srv.settings().awake_on(), "keep awake is off by default");
    n.srv.reconcile().await;
    want("before the server is switched on", 0, 0);
    n.srv.set(true, false, "").unwrap();
    n.srv.reconcile().await;
    assert!(n.srv.status().running, "not running");
    want("started", 1, 0);
    let st = n.srv.status();
    assert!(
        st.awake && st.awake_line == "Keeping this PC awake",
        "status {st:?}"
    );
    n.srv.reconcile().await;
    n.srv.reconcile().await;
    want("re-checks", 1, 0);

    n.srv.set_awake(false).unwrap();
    // Devices reads the status straight back, before the Run loop has
    // released the hold: it must not still say it keeps the PC awake.
    let st = n.srv.status();
    assert!(
        !st.awake && st.awake_line.is_empty(),
        "status right after the switch went off {st:?}"
    );
    n.srv.reconcile().await;
    want("setting off", 1, 1);
    let st = n.srv.status();
    assert!(
        st.running && !st.awake && st.awake_line.is_empty(),
        "status with the setting off {st:?}"
    );
    let saved = settings::load(&n.dir).unwrap();
    assert!(!saved.awake_on(), "saved setting {saved:?}");

    n.srv.set_awake(true).unwrap();
    n.srv.reconcile().await;
    want("setting back on", 2, 1);

    n.srv.stop().await;
    want("stopped", 2, 2);
    n.srv.stop().await;
    want("stopped twice", 2, 2);
}

/// Switching the server off releases the hold through the Run loop.
#[tokio::test(flavor = "multi_thread")]
async fn keep_awake_released_when_switched_off() {
    let mut n = Node::new(|_| {});
    n.run();
    n.srv.set(true, false, "").unwrap();
    eventually("the hold", || n.awake.counts().0 == 1).await;
    n.srv.set(false, false, "").unwrap();
    eventually("the release", || n.awake.counts().1 == 1).await;
    assert_eq!(n.awake.counts().0, 1, "acquired more than once");
    n.shutdown().await;
}

/// A hold that can't be taken leaves the server serving, says why, and is
/// taken at a later check once it can be.
#[tokio::test(flavor = "multi_thread")]
async fn keep_awake_failure_keeps_serving() {
    let n = Node::new(|_| {});
    n.awake.set_fail(Some("no power API"));
    n.srv.set(true, false, "").unwrap();
    n.srv.reconcile().await;
    let st = n.srv.status();
    assert!(
        st.running && st.awake_line == "Can't keep this PC awake: no power API",
        "status {st:?}"
    );
    n.awake.set_fail(None);
    n.srv.reconcile().await;
    assert_eq!(n.awake.counts().0, 1);
    assert_eq!(n.srv.status().awake_line, "Keeping this PC awake");
    n.srv.stop().await;
    assert_eq!(n.awake.counts().1, 1);
}

/// `home-server.json` keeps Go's bytes: tri-state fields absent until set,
/// two-space indent, and an absent `outside` is never written as false.
#[test]
fn settings_file_bytes() {
    let s = Settings {
        on: true,
        port: 25817,
        ..Default::default()
    };
    assert_eq!(
        String::from_utf8(s.to_file_bytes()).unwrap(),
        "{\n  \"on\": true,\n  \"port\": 25817\n}"
    );
    let s = Settings {
        on: true,
        port: 25817,
        outside: Some(false),
        public: "pc.example".into(),
        tcp_external: 1,
        udp_external: 2,
        awake: Some(true),
    };
    assert_eq!(
        String::from_utf8(s.to_file_bytes()).unwrap(),
        "{\n  \"on\": true,\n  \"port\": 25817,\n  \"outside\": false,\n  \"public\": \"pc.example\",\n  \"tcpExternal\": 1,\n  \"udpExternal\": 2,\n  \"awake\": true\n}"
    );
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(settings::load(tmp.path()).unwrap(), Settings::default());
    std::fs::write(
        tmp.path().join(settings::FILE),
        b"{\"on\":true,\"port\":25817}",
    )
    .unwrap();
    let got = settings::load(tmp.path()).unwrap();
    assert!(got.on && got.port == 25817 && got.outside_on() && got.awake_on());
    settings::save(tmp.path(), &got).unwrap();
    assert_eq!(
        std::fs::read(tmp.path().join(settings::FILE)).unwrap(),
        b"{\n  \"on\": true,\n  \"port\": 25817\n}"
    );
    std::fs::write(tmp.path().join(settings::FILE), b"{\"on\":\"yes\"}").unwrap();
    assert_eq!(
        settings::load(tmp.path()).unwrap_err().to_string(),
        "home: home-server.json is unreadable"
    );
}

/// A certified device's relay connection is admitted; once it is revoked,
/// the server drops it and refuses it from then on (Go
/// TestRevokeDropsTheHubConnectionAndTURN, as relay Deny). The other
/// device is a fixture key; its endpoint is relay-only on loopback, pinned
/// to the server's device key.
#[tokio::test(flavor = "multi_thread")]
async fn revoke_drops_the_relay_connection() {
    let mut n = Node::new(|_| {});
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).unwrap();
    let key = iroh::SecretKey::from_bytes(&seed);
    let other = "mac00000";
    let now = gocompat::time::GoTime::from_system(SystemTime::now());
    let add = n
        .ident
        .cert_for(
            other,
            &hex::encode(key.public().as_bytes()),
            "mac",
            irori_identity::objects::ROLE_DESKTOP,
            now,
        )
        .unwrap();
    n.st.append("irori", store::kinds::PERSON_DEVICE_ADDED, &add)
        .unwrap();
    n.run();
    n.srv.set(true, false, "").unwrap();
    eventually("running", || n.srv.relay_port().is_some()).await;
    let port = n.srv.relay_port().unwrap();
    let server_id = iroh::PublicKey::from_bytes(&n.ident.device_pub()).unwrap();

    // A stranger (no cert) is refused outright.
    let stranger = client(key_from_random(), port, server_id).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(n.srv.relay_clients().is_empty(), "a stranger got in");

    let ep = client(key.clone(), port, server_id).await;
    eventually("the certified device is in", || {
        n.srv.relay_clients().contains(&key.public())
    })
    .await;
    let rev = n.ident.revoke_of(other, 0, "", "test", now).unwrap();
    n.st.append("irori", store::kinds::PERSON_DEVICE_REVOKED, &rev)
        .unwrap();
    eventually("the revoked device is dropped", || {
        !n.srv.relay_clients().contains(&key.public())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await; // the client's reconnect is refused
    assert!(
        !n.srv.relay_clients().contains(&key.public()),
        "the revoked device got back in"
    );
    ep.close().await;
    stranger.close().await;
    n.shutdown().await;
}

fn key_from_random() -> iroh::SecretKey {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).unwrap();
    iroh::SecretKey::from_bytes(&seed)
}

/// A relay-only iroh endpoint whose home relay is the server's, pinned to
/// the server's device key.
async fn client(key: iroh::SecretKey, port: u16, server: iroh::EndpointId) -> iroh::Endpoint {
    let url: iroh::RelayUrl = format!("https://127.0.0.1:{port}/").parse().unwrap();
    let map = iroh::RelayMap::from_iter([iroh::RelayConfig::new(
        url,
        Some(iroh_relay::RelayQuicConfig::new(port)),
    )]);
    let mut nr = iroh::endpoint::NetReportConfig::default();
    nr.captive_portal_check = false;
    iroh::endpoint::Builder::empty()
        .crypto_provider(irori_net::tls::provider())
        .secret_key(key)
        .alpns(vec![b"irori/home-test/1".to_vec()])
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .net_report_config(nr)
        .clear_ip_transports()
        .ca_tls_config(irori_net::tls::pinned_ca_config(&[server]))
        .relay_mode(iroh::RelayMode::Custom(map))
        .bind()
        .await
        .unwrap()
}

/// A pairing window's guest key: a fresh room secret.
fn window_secret() -> irori_meshseam::Secret {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).unwrap();
    irori_meshseam::Secret {
        key,
        topic: format!("irori/pair/1/{}", hex::encode(&key[..8])),
        epoch: 0,
    }
}

/// A relay-only endpoint that presents a pairing window's guest token.
async fn guest(
    port: u16,
    server: iroh::EndpointId,
    token: &str,
) -> (iroh::SecretKey, iroh::Endpoint) {
    let key = key_from_random();
    let url: iroh::RelayUrl = format!("https://127.0.0.1:{port}/").parse().unwrap();
    let cfg = iroh::RelayConfig::new(url, Some(iroh_relay::RelayQuicConfig::new(port)))
        .with_auth_token(token.to_string());
    let mut nr = iroh::endpoint::NetReportConfig::default();
    nr.captive_portal_check = false;
    let ep = iroh::endpoint::Builder::empty()
        .crypto_provider(irori_net::tls::provider())
        .secret_key(key.clone())
        .alpns(vec![b"irori/home-test/1".to_vec()])
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .net_report_config(nr)
        .clear_ip_transports()
        .ca_tls_config(irori_net::tls::pinned_ca_config(&[server]))
        .relay_mode(iroh::RelayMode::Custom(iroh::RelayMap::from_iter([cfg])))
        .bind()
        .await
        .unwrap();
    (key, ep)
}

/// A server with guest windows, running, and its relay's port and key.
async fn guest_server(g: &irori_net::guest::GuestWindows) -> (Node, u16, iroh::EndpointId) {
    let gw = g.clone();
    let mut n = Node::new(move |c| c.guests = Some(gw));
    n.run();
    n.srv.set(true, false, "").unwrap();
    eventually("running", || n.srv.relay_port().is_some()).await;
    let port = n.srv.relay_port().unwrap();
    let id = iroh::PublicKey::from_bytes(&n.ident.device_pub()).unwrap();
    (n, port, id)
}

/// Architecture §8 M7: the relay drops a guest once its pairing window
/// closes, and refuses it after; at most `MAX_GUESTS` guests are in at a
/// time, and the closed window's guests free their slots for the next one.
#[tokio::test(flavor = "multi_thread")]
async fn guests_leave_with_their_window_and_keep_the_cap() {
    use irori_net::guest::{GuestWindows, MAX_GUESTS, guest_token};
    let g = GuestWindows::default();
    let (n, port, server) = guest_server(&g).await;
    let owner = key_from_random().public();
    let s1 = window_secret();
    let w1 = g
        .open_window(owner, s1.clone(), Duration::from_secs(60))
        .unwrap();
    let tok = guest_token(&s1.key);
    let mut eps = Vec::new();
    for _ in 0..MAX_GUESTS {
        eps.push(guest(port, server, &tok).await);
    }
    eventually("the guests are in", || {
        n.srv.relay_clients().len() == MAX_GUESTS
    })
    .await;
    let extra = guest(port, server, &tok).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(
        !n.srv.relay_clients().contains(&extra.0.public()),
        "a guest over the cap got in"
    );

    // The window closes: every guest is dropped and stays out.
    g.close_window(w1);
    n.srv.kick();
    eventually("the closed window's guests are dropped", || {
        n.srv.relay_clients().is_empty()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(n.srv.relay_clients().is_empty(), "a guest got back in");

    // A new window's guest gets a slot the old ones held.
    let s2 = window_secret();
    g.open_window(owner, s2.clone(), Duration::from_secs(60))
        .unwrap();
    let fresh = guest(port, server, &guest_token(&s2.key)).await;
    eventually("the new window's guest is in", || {
        n.srv.relay_clients().contains(&fresh.0.public())
    })
    .await;
    fresh.1.close().await;
    extra.1.close().await;
    for (_, ep) in eps {
        ep.close().await;
    }
    n.shutdown().await;
}

/// A window that expires (its TTL, with no close) drops its guests within
/// the guest sweep. Windows per member are capped too.
#[tokio::test(flavor = "multi_thread")]
async fn guests_leave_when_their_window_expires() {
    use irori_net::guest::{GuestWindows, MAX_WINDOWS_PER_MEMBER, guest_token};
    let g = GuestWindows::default();
    let (n, port, server) = guest_server(&g).await;
    let owner = key_from_random().public();
    let s = window_secret();
    g.open_window(owner, s.clone(), Duration::from_secs(3))
        .unwrap();
    for _ in 1..MAX_WINDOWS_PER_MEMBER {
        g.open_window(owner, window_secret(), Duration::from_secs(3))
            .unwrap();
    }
    assert!(
        g.open_window(owner, window_secret(), Duration::from_secs(3))
            .is_none(),
        "a member opened more than its windows"
    );
    let (k, ep) = guest(port, server, &guest_token(&s.key)).await;
    eventually("the guest is in", || {
        n.srv.relay_clients().contains(&k.public())
    })
    .await;
    // No close, no kick: the TTL ends it.
    let deadline =
        std::time::Instant::now() + Duration::from_secs(3) + 2 * home::server::GUEST_SWEEP;
    while n.srv.relay_clients().contains(&k.public()) {
        assert!(
            std::time::Instant::now() < deadline + Duration::from_secs(2),
            "an expired window's guest stayed"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    ep.close().await;
    n.shutdown().await;
}

/// B1 (boards stage 2, card #43): the relay admits a counted member device
/// of a board this server holds, refuses everyone else, and drops the
/// member (refusing it after) once no board held here counts it.
#[tokio::test(flavor = "multi_thread")]
async fn the_relay_admits_board_members_and_drops_a_removed_one() {
    use irori_net::boardadmit::{HeldBoards, MemberKeys};
    let held = HeldBoards::default();
    let members = MemberKeys::default();
    let member = key_from_random();
    members.add("blakepc0", &member.public());
    held.hold("board-a", Arc::new(members.clone()));
    // A board held here whose members don't include the stranger either.
    held.hold("board-b", Arc::new(MemberKeys::default()));
    let relay_admit = held.relay_admit();
    let mut n = Node::new(move |c| c.board_admit = Some(relay_admit));
    n.run();
    n.srv.set(true, false, "").unwrap();
    eventually("running", || n.srv.relay_port().is_some()).await;
    let port = n.srv.relay_port().unwrap();
    let server_id = iroh::PublicKey::from_bytes(&n.ident.device_pub()).unwrap();

    let stranger_key = key_from_random();
    let stranger = client(stranger_key.clone(), port, server_id).await;
    let ep = client(member.clone(), port, server_id).await;
    eventually("the board member is in", || {
        n.srv.relay_clients().contains(&member.public())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !n.srv.relay_clients().contains(&stranger_key.public()),
        "a stranger got in"
    );

    // Removed from the board: the guest sweep drops it, and it stays out.
    members.remove(&member.public());
    eventually("the removed member is dropped", || {
        !n.srv.relay_clients().contains(&member.public())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        !n.srv.relay_clients().contains(&member.public()),
        "the removed member got back in"
    );
    ep.close().await;
    stranger.close().await;
    n.shutdown().await;
}

/// Without `board_admit` nothing changes: a key no person certified is
/// refused even if some board elsewhere counts it.
#[tokio::test(flavor = "multi_thread")]
async fn without_board_admit_the_relay_stays_own_devices_only() {
    let mut n = Node::new(|_| {});
    n.run();
    n.srv.set(true, false, "").unwrap();
    eventually("running", || n.srv.relay_port().is_some()).await;
    let port = n.srv.relay_port().unwrap();
    let server_id = iroh::PublicKey::from_bytes(&n.ident.device_pub()).unwrap();
    let k = key_from_random();
    let ep = client(k.clone(), port, server_id).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(n.srv.relay_clients().is_empty(), "an unknown key got in");
    ep.close().await;
    n.shutdown().await;
}

/// The invites part's guest hook (card #44): an invite window admits a
/// client presenting its invite token only while it is open; the pairing
/// token of the same key opens nothing, and closing the window drops the
/// invitee.
#[tokio::test(flavor = "multi_thread")]
async fn an_invite_window_admits_its_invitee_only_while_open() {
    use irori_net::guest::{GuestWindows, WindowKind, guest_token, invite_token};
    let g = GuestWindows::default();
    let (n, port, server) = guest_server(&g).await;
    let s = window_secret();
    let tok = invite_token(&s.key);

    let early = guest(port, server, &tok).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(
        !n.srv.relay_clients().contains(&early.0.public()),
        "an invitee got in before the window opened"
    );
    early.1.close().await;

    let owner = key_from_random().public();
    let id = g
        .open_invite_window(owner, s.clone(), Duration::from_secs(60))
        .unwrap();
    assert_eq!(g.kind(id), Some(WindowKind::Invite));
    let wrong = guest(port, server, &guest_token(&s.key)).await;
    let invitee = guest(port, server, &tok).await;
    eventually("the invitee is in", || {
        n.srv.relay_clients().contains(&invitee.0.public())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !n.srv.relay_clients().contains(&wrong.0.public()),
        "the pairing token opened an invite window"
    );

    g.close_window(id);
    n.srv.kick();
    eventually("the invitee is dropped with its window", || {
        !n.srv.relay_clients().contains(&invitee.0.public())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        !n.srv.relay_clients().contains(&invitee.0.public()),
        "the invitee got back in"
    );
    invitee.1.close().await;
    wrong.1.close().await;
    n.shutdown().await;
}

/// `board.server` (v0.5 §6.2): the port and the public address only, and
/// `public: None` until outside reach is on and the router mapped it.
#[tokio::test(flavor = "multi_thread")]
async fn the_board_server_row_names_only_the_public_address() {
    let mut n = Node::new(|_| {});
    assert!(n.srv.board_server().is_err(), "a server that never ran");
    n.run();
    n.srv.set(true, false, "").unwrap();
    eventually("running", || n.srv.relay_port().is_some()).await;
    let port = i64::from(n.srv.relay_port().unwrap());
    let me = n.ident.device_id();
    let row = n.srv.board_server().unwrap();
    assert_eq!(
        row,
        mesh::boardmesh::BoardServer {
            device_id: me.clone(),
            port,
            public: None,
        },
        "outside reach off"
    );
    n.srv.set(true, true, "").unwrap();
    eventually("a public board.server", || {
        n.srv.board_server().is_ok_and(|r| r.public.is_some())
    })
    .await;
    let row = n.srv.board_server().unwrap();
    let p = row.public.clone().unwrap();
    let port = n.srv.settings().port;
    assert!(
        row.device_id == me && row.port == port && p.ip == "203.0.113.7" && p.tcp == port,
        "{row:?}"
    );
    // The JSON never carries LAN or v6 addresses.
    let j = serde_json::to_value(&row).unwrap();
    let keys: Vec<&str> = j.as_object().unwrap().keys().map(|k| k.as_str()).collect();
    assert_eq!(keys, vec!["deviceId", "port", "public"]);
    n.srv.set(false, true, "").unwrap();
    eventually("off: public null", || {
        n.srv.board_server().is_ok_and(|r| r.public.is_none())
    })
    .await;
    n.shutdown().await;
}
