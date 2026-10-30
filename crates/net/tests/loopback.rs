//! Loopback-only iroh tests (architecture §6.5): every socket is on 127.0.0.1,
//! keys are generated here, the relay is embedded in the test process.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iroh::unstable_net_report::{NetReport, Probe};
use iroh::{Endpoint, EndpointAddr, EndpointId, Watcher as _};
use irori_net::endpoint::{self, EndpointOpts, IpMode, RelaySpec};
use irori_net::identity::{DeviceIdentity, IrohKey as _};
use irori_net::path::{self, PathKind};
use irori_net::relay::{self, AllowList, EmbeddedRelay};

const LO: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

fn ident() -> DeviceIdentity {
    let mut s = [0u8; 32];
    getrandom::fill(&mut s).unwrap();
    DeviceIdentity::from_seed_for_tests(s)
}

fn assert_loopback(ep: &Endpoint) {
    for a in ep.bound_sockets() {
        assert!(a.ip().is_loopback(), "non-loopback socket {a}");
    }
}

struct Net {
    server: DeviceIdentity,
    relay: EmbeddedRelay,
    allow: AllowList,
}

impl Net {
    async fn new() -> Self {
        let server = ident();
        let allow = AllowList::new([server.endpoint_id()]);
        let relay = relay::spawn(&server, LO, 0, allow.clone()).await.unwrap();
        assert!(relay.server.https_addr().unwrap().ip().is_loopback());
        assert!(relay.server.http_addr().unwrap().ip().is_loopback());
        assert!(relay.server.quic_addr().unwrap().ip().is_loopback());
        Net {
            server,
            relay,
            allow,
        }
    }
    fn spec(&self) -> RelaySpec {
        RelaySpec {
            url: self.relay.url_for(LO),
            quic_port: self.relay.port,
            server_pub: self.server.endpoint_id(),
        }
    }
    async fn endpoint(
        &self,
        id: &DeviceIdentity,
        ip: IpMode,
        allow: Option<AllowList>,
    ) -> Endpoint {
        let mut o = EndpointOpts::new(ip);
        o.relays = vec![self.spec()];
        o.allow = allow;
        let ep = endpoint::bind(id, o).await.unwrap();
        assert_loopback(&ep);
        ep
    }
}

async fn wait_relay(ep: &Endpoint, within: Duration) -> Result<(), String> {
    let start = Instant::now();
    loop {
        let st = ep.home_relay_status().get();
        if st.iter().any(|s| s.is_connected()) {
            return Ok(());
        }
        if start.elapsed() > within {
            return Err(format!(
                "{:?}",
                st.iter()
                    .map(|s| (
                        s.url().to_string(),
                        s.auth_denied_reason().map(str::to_string),
                        s.last_error().map(|e| format!("{e:#}"))
                    ))
                    .collect::<Vec<_>>()
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn spawn_echo(ep: &Endpoint) {
    tokio::spawn(endpoint::serve_echo(ep.clone(), |_| {}));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_only_echo_through_the_embedded_relay() {
    let net = Net::new().await;
    let (a, b) = (ident(), ident());
    net.allow.allow(a.endpoint_id());
    net.allow.allow(b.endpoint_id());
    let eb = net
        .endpoint(
            &b,
            IpMode::RelayOnly,
            Some(AllowList::new([a.endpoint_id()])),
        )
        .await;
    let ea = net.endpoint(&a, IpMode::RelayOnly, None).await;
    assert!(
        eb.bound_sockets().is_empty(),
        "relay-only binds no IP socket"
    );
    wait_relay(&eb, Duration::from_secs(10))
        .await
        .expect("B on relay (TLS pinned to device key)");
    spawn_echo(&eb);

    let to = EndpointAddr::new(b.endpoint_id()).with_relay_url(net.relay.url_for(LO));
    let conn = tokio::time::timeout(
        Duration::from_secs(15),
        ea.connect(to, irori_net::SPIKE_ALPN),
    )
    .await
    .expect("connect timeout")
    .expect("connect");
    assert_eq!(conn.remote_id(), b.endpoint_id());
    for i in 0..3 {
        let msg = format!("hello {i}");
        let back = endpoint::echo_once(&conn, msg.as_bytes()).await.unwrap();
        assert_eq!(back, msg.as_bytes());
    }
    let r = path::report(&conn);
    println!("relay-only: {r}");
    assert_eq!(r.kind, PathKind::Relay);
    assert!(r.rtt.is_some());
    ea.close().await;
    eb.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn direct_paths_allowed_reports_the_path_type() {
    let net = Net::new().await;
    let (a, b) = (ident(), ident());
    net.allow.allow(a.endpoint_id());
    net.allow.allow(b.endpoint_id());
    let eb = net
        .endpoint(
            &b,
            IpMode::Loopback,
            Some(AllowList::new([a.endpoint_id()])),
        )
        .await;
    let ea = net.endpoint(&a, IpMode::Loopback, None).await;
    wait_relay(&eb, Duration::from_secs(10))
        .await
        .expect("B on relay");
    wait_relay(&ea, Duration::from_secs(10))
        .await
        .expect("A on relay");
    spawn_echo(&eb);

    // Only the relay URL: any direct path must come from iroh's hole punching.
    let to = EndpointAddr::new(b.endpoint_id()).with_relay_url(net.relay.url_for(LO));
    let conn = tokio::time::timeout(
        Duration::from_secs(15),
        ea.connect(to, irori_net::SPIKE_ALPN),
    )
    .await
    .expect("connect timeout")
    .expect("connect");
    let first = path::report(&conn);
    println!("direct-allowed, first: {first}");
    let start = Instant::now();
    let mut last = first.clone();
    while start.elapsed() < Duration::from_secs(10) {
        endpoint::echo_once(&conn, b"ping").await.unwrap();
        last = path::report(&conn);
        if last.kind == PathKind::Direct {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    println!("direct-allowed, after {:?}: {last}", start.elapsed());
    assert_eq!(
        last.kind,
        PathKind::Direct,
        "loopback hole punch should go direct"
    );
    assert!(last.remote.starts_with("127.0.0.1:"));
    ea.close().await;
    eb.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_endpoint_off_the_allow_list_is_refused() {
    let net = Net::new().await;
    let (a, b, c) = (ident(), ident(), ident());
    net.allow.allow(a.endpoint_id());
    net.allow.allow(b.endpoint_id());
    let eb = net
        .endpoint(
            &b,
            IpMode::Loopback,
            Some(AllowList::new([a.endpoint_id()])),
        )
        .await;
    wait_relay(&eb, Duration::from_secs(10)).await.unwrap();
    spawn_echo(&eb);

    // 1. The relay refuses C (AccessControl::on_connect -> Deny).
    let ec = net.endpoint(&c, IpMode::RelayOnly, None).await;
    let start = Instant::now();
    let reason = loop {
        let st = ec.home_relay_status().get();
        if let Some(r) = st
            .iter()
            .find_map(|s| s.auth_denied_reason().map(str::to_string))
        {
            break r;
        }
        assert!(
            !st.iter().any(|s| s.is_connected()),
            "C must not be admitted"
        );
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "no denial seen: {st:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    println!("relay denied C: {reason}");
    assert!(reason.contains("not a certified device"));
    let to = EndpointAddr::new(b.endpoint_id()).with_relay_url(net.relay.url_for(LO));
    let r = tokio::time::timeout(
        Duration::from_secs(5),
        ec.connect(to, irori_net::SPIKE_ALPN),
    )
    .await;
    assert!(!matches!(r, Ok(Ok(_))), "C reached B through the relay");

    // 2. B's after_handshake hook refuses C even on a direct path.
    let ec2 = {
        let mut o = EndpointOpts::new(IpMode::Loopback);
        o.extra_pins = vec![net.server.endpoint_id()];
        endpoint::bind(&c, o).await.unwrap()
    };
    assert_loopback(&ec2);
    let b_ip = *eb.bound_sockets().first().unwrap();
    let to = EndpointAddr::new(b.endpoint_id()).with_ip_addr(b_ip);
    let res = tokio::time::timeout(Duration::from_secs(10), async {
        let conn = ec2.connect(to, irori_net::SPIKE_ALPN).await?;
        endpoint::echo_once(&conn, b"x").await
    })
    .await
    .expect("timeout");
    let err = res.expect_err("hook must refuse C");
    println!("hook refused C: {err:#}");
    ec.close().await;
    ec2.close().await;
    eb.close().await;
}

/// One TLS handshake with the relay's HTTPS port through `PinnedDeviceKey`,
/// the verifier `pinned_ca_config` hands iroh.
fn pinned_handshake(addr: SocketAddr, pin: EndpointId) -> Result<(), rustls::Error> {
    let p = irori_net::tls::provider();
    let cfg = rustls::ClientConfig::builder_with_provider(p.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(irori_net::tls::PinnedDeviceKey::new(
            &[*pin.as_bytes()],
            p,
        )))
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from("irori-relay").unwrap();
    let mut conn = rustls::ClientConnection::new(Arc::new(cfg), name).unwrap();
    let mut sock = std::net::TcpStream::connect(addr).unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    while conn.is_handshaking() {
        if let Err(e) = conn.complete_io(&mut sock) {
            return Err(e
                .get_ref()
                .and_then(|i| i.downcast_ref::<rustls::Error>())
                .cloned()
                .unwrap_or_else(|| panic!("handshake I/O error, not a TLS one: {e}")));
        }
    }
    Ok(())
}

async fn wait_report(ep: &Endpoint, within: Duration) -> Option<NetReport> {
    let mut w = ep.net_report();
    tokio::time::timeout(within, w.initialized()).await.ok()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_tls_refuses_a_server_with_another_key() {
    let net = Net::new().await;
    let wrong = ident().endpoint_id();
    let https = net.relay.server.https_addr().unwrap();

    // 1. The verifier itself: the relay's device-key cert fails the wrong pin
    // with the pin error and passes the right one.
    let got = tokio::task::spawn_blocking(move || pinned_handshake(https, wrong))
        .await
        .unwrap();
    println!("wrong pin handshake: {got:?}");
    assert_eq!(
        got,
        Err(rustls::Error::InvalidCertificate(
            rustls::CertificateError::ApplicationVerificationFailure
        ))
    );
    let right = net.server.endpoint_id();
    tokio::task::spawn_blocking(move || pinned_handshake(https, right))
        .await
        .unwrap()
        .expect("right pin handshake");

    // 2. iroh with the wrong pin: every relay probe (HTTPS and QAD) fails TLS,
    // so the net report has no latency for the relay, iroh never picks it as
    // home relay (home_relay_status stays empty), and it never connects.
    let a = ident();
    net.allow.allow(a.endpoint_id());
    let mut o = EndpointOpts::new(IpMode::Loopback);
    let mut spec = net.spec();
    spec.server_pub = wrong;
    o.relays = vec![spec];
    let ea = endpoint::bind(&a, o).await.unwrap();
    let report = wait_report(&ea, Duration::from_secs(15))
        .await
        .expect("a net report runs even when every probe fails");
    println!("wrong pin: report = {report:?}");
    assert_eq!(report.relay_latency.iter().count(), 0, "{report:?}");
    assert!(!report.udp_v4, "QAD must fail TLS: {report:?}");
    assert_eq!(report.global_v4, None);
    assert_eq!(report.preferred_relay, None);
    let err = wait_relay(&ea, Duration::from_secs(4))
        .await
        .expect_err("must not connect");
    println!("wrong pin: home relay status = {err}");
    ea.close().await;

    // 3. Same relay, same client key, right pin: the same probes succeed and
    // it connects, so the custom verifier is the one iroh uses.
    let ok = net.endpoint(&a, IpMode::Loopback, None).await;
    wait_relay(&ok, Duration::from_secs(10))
        .await
        .expect("right pin connects");
    ok.close().await;
}

/// mesh.md §8.1: the one `ca_tls_config` covers QAD as well as the relay's
/// HTTPS. A QAD IPv4 round trip completes against the embedded relay's
/// device-key cert and reports this endpoint's own loopback socket.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qad_probe_succeeds_through_the_pinned_ca_config() {
    let net = Net::new().await;
    let a = ident();
    net.allow.allow(a.endpoint_id());
    let ea = net.endpoint(&a, IpMode::Loopback, None).await;
    let url = net.relay.url_for(LO);
    let start = Instant::now();
    let report = loop {
        if let Some(r) = ea.net_report().get()
            && r.udp_v4
        {
            break r;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "no QAD round trip: {:?}",
            ea.net_report().get()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    println!("QAD: {report:?}");
    assert!(
        report
            .relay_latency
            .iter()
            .any(|(p, u, _)| matches!(p, Probe::QadIpv4) && *u == url),
        "no QAD IPv4 latency for {url}: {report:?}"
    );
    let own = ea.bound_sockets()[0];
    assert_eq!(report.global_v4.map(SocketAddr::V4), Some(own));
    ea.close().await;
}

/// Can an endpoint bind with `RelayMode::Custom` over an empty map (a relay
/// transport with no relay yet), and does `insert_relay` then work?
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn custom_empty_map_then_insert_relay() {
    let net = Net::new().await;
    let (a, b) = (ident(), ident());
    net.allow.allow(a.endpoint_id());
    net.allow.allow(b.endpoint_id());
    let bound = iroh::endpoint::Builder::empty()
        .crypto_provider(irori_net::tls::provider())
        .secret_key(a.secret_key())
        .alpns(vec![irori_net::SPIKE_ALPN.to_vec()])
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .unwrap()
        .relay_mode(iroh::RelayMode::Custom(iroh::RelayMap::empty()))
        .ca_tls_config(irori_net::tls::pinned_ca_config(&[net
            .server
            .endpoint_id()]))
        .bind()
        .await;
    // mesh.md §8.1 rests on this bind succeeding: no skip on failure.
    let ea = bound.expect("RelayMode::Custom(RelayMap::empty()) must bind");
    assert_loopback(&ea);
    ea.insert_relay(
        net.relay.url_for(LO),
        std::sync::Arc::new(relay::client_relay_config(
            net.relay.url_for(LO),
            net.relay.port,
        )),
    )
    .await;
    let r = wait_relay(&ea, Duration::from_secs(10)).await;
    println!("Custom(empty) + insert_relay: home relay = {r:?}");
    spawn_echo(&ea);
    let eb = net.endpoint(&b, IpMode::RelayOnly, None).await;
    let to = EndpointAddr::new(a.endpoint_id()).with_relay_url(net.relay.url_for(LO));
    let c = tokio::time::timeout(
        Duration::from_secs(10),
        eb.connect(to, irori_net::SPIKE_ALPN),
    )
    .await
    .expect("timeout")
    .expect("connect via relay inserted into an empty Custom map");
    assert_eq!(endpoint::echo_once(&c, b"ok").await.unwrap(), b"ok");
    assert!(r.is_ok());
    ea.close().await;
    eb.close().await;
}

/// Critic finding 2: does `insert_relay` work on an endpoint bound with
/// `RelayMode::Disabled`? Also checks the alternative of binding with a
/// non-empty Custom map and inserting a second relay later.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn insert_relay_after_bind() {
    let net = Net::new().await;
    let (a, b) = (ident(), ident());
    net.allow.allow(a.endpoint_id());
    net.allow.allow(b.endpoint_id());

    // Disabled at bind, then insert_relay.
    let ea = iroh::endpoint::Builder::empty()
        .crypto_provider(irori_net::tls::provider())
        .secret_key(a.secret_key())
        .alpns(vec![irori_net::SPIKE_ALPN.to_vec()])
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .unwrap()
        .relay_mode(iroh::RelayMode::Disabled)
        .ca_tls_config(irori_net::tls::pinned_ca_config(&[net
            .server
            .endpoint_id()]))
        .bind()
        .await
        .unwrap();
    assert_loopback(&ea);
    let prev = ea
        .insert_relay(
            net.relay.url_for(LO),
            std::sync::Arc::new(relay::client_relay_config(
                net.relay.url_for(LO),
                net.relay.port,
            )),
        )
        .await;
    assert!(prev.is_none());
    let disabled = wait_relay(&ea, Duration::from_secs(6)).await;
    println!(
        "Disabled + insert_relay: home relay = {disabled:?}, status = {:?}",
        ea.home_relay_status().get()
    );
    spawn_echo(&ea);
    let eb = net.endpoint(&b, IpMode::RelayOnly, None).await;
    let to = EndpointAddr::new(a.endpoint_id()).with_relay_url(net.relay.url_for(LO));
    let reach = tokio::time::timeout(Duration::from_secs(6), async {
        let c = eb.connect(to, irori_net::SPIKE_ALPN).await?;
        endpoint::echo_once(&c, b"via inserted relay").await
    })
    .await;
    let disabled_reach = matches!(reach, Ok(Ok(_)));
    println!("Disabled + insert_relay: reachable through relay = {disabled_reach}");
    assert!(
        disabled.is_err() && !disabled_reach,
        "Disabled + insert_relay unexpectedly worked; update mesh.md §8"
    );
    ea.close().await;

    // Custom map with a placeholder relay, then insert the real one.
    let a2 = ident();
    net.allow.allow(a2.endpoint_id());
    let mut o = EndpointOpts::new(IpMode::Loopback);
    o.relays = vec![RelaySpec {
        url: relay::relay_url(LO, 9), // discard port: nothing answers
        quic_port: 9,
        server_pub: net.server.endpoint_id(),
    }];
    let ea2 = endpoint::bind(&a2, o).await.unwrap();
    ea2.insert_relay(
        net.relay.url_for(LO),
        std::sync::Arc::new(relay::client_relay_config(
            net.relay.url_for(LO),
            net.relay.port,
        )),
    )
    .await;
    let custom = wait_relay(&ea2, Duration::from_secs(10)).await;
    println!("Custom(placeholder) + insert_relay: home relay = {custom:?}");
    spawn_echo(&ea2);
    let to = EndpointAddr::new(a2.endpoint_id()).with_relay_url(net.relay.url_for(LO));
    let c = tokio::time::timeout(
        Duration::from_secs(10),
        eb.connect(to, irori_net::SPIKE_ALPN),
    )
    .await
    .expect("timeout")
    .expect("connect via inserted relay");
    assert_eq!(endpoint::echo_once(&c, b"ok").await.unwrap(), b"ok");
    assert!(custom.is_ok());
    ea2.close().await;
    eb.close().await;
}
