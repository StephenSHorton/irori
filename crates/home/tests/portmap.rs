//! Ports of R:portmap/portmap_test.go. A NAT-PMP gateway fake runs on a
//! 127.0.0.1 UDP socket and the other gateways are in process; nothing does
//! SSDP or reads the real default gateway.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use home::portmap::natpmp::NatPmp;
use home::portmap::{
    self, BoxFut, Config, Gateway, MapError, Mapper, Port, STATE_FAILED, STATE_OK, Status, Timing,
};

fn ports() -> Vec<Port> {
    vec![
        Port {
            proto: "tcp",
            internal: 24817,
            external: 24817,
        },
        Port {
            proto: "udp",
            internal: 24817,
            external: 24817,
        },
    ]
}

fn fast() -> Timing {
    Timing {
        retry_failed: Duration::from_millis(50),
        recheck: Duration::from_millis(200),
        ip_poll: Duration::from_millis(20),
        min_renew: Duration::from_millis(10),
        call_timeout: Duration::from_secs(2),
        conflict_try: 4,
    }
}

fn config(gws: Vec<Arc<dyn Gateway>>) -> Config {
    let mut c = Config::new(ports());
    c.discover = Some(Arc::new(move || {
        let gws = gws.clone();
        Box::pin(async move { Ok(gws) })
    }));
    c.timing = fast();
    c
}

async fn wait_state(m: &Mapper, state: &str) -> Status {
    for _ in 0..500 {
        let s = m.status();
        if s.state == state {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("state {state} never came; status {:?}", m.status());
}

/// A fake NAT-PMP gateway on 127.0.0.1.
struct FakePmp {
    addr: SocketAddr,
    st: Arc<Mutex<PmpState>>,
}

#[derive(Default)]
struct PmpState {
    external: [u8; 4],
    mapped: BTreeMap<(u8, u16), u16>,
    result: u16,
    silent: bool,
    lifetime: u32,
    reqs: usize,
}

impl FakePmp {
    fn new(ext: [u8; 4]) -> FakePmp {
        let sock = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = sock.local_addr().unwrap();
        let st = Arc::new(Mutex::new(PmpState {
            external: ext,
            ..Default::default()
        }));
        let s2 = st.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 64];
            loop {
                let (n, from) = match sock.recv_from(&mut buf) {
                    Ok(x) => x,
                    // Windows reports an ICMP port unreachable for an earlier
                    // reply (to a client socket that gave up and closed) as a
                    // read error; Go's net turns that off (SIO_UDP_CONNRESET),
                    // so a loaded run would otherwise kill the gateway here.
                    Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => continue,
                    Err(_) => return,
                };
                let mut s = s2.lock().unwrap();
                s.reqs += 1;
                if s.silent || n < 2 {
                    continue;
                }
                let op = buf[1];
                let mut out = vec![0u8, 128 + op];
                out.extend_from_slice(&s.result.to_be_bytes());
                out.extend_from_slice(&1u32.to_be_bytes()); // epoch
                if op == 0 {
                    out.extend_from_slice(&s.external);
                } else if n >= 12 {
                    let internal = u16::from_be_bytes([buf[4], buf[5]]);
                    let ext = u16::from_be_bytes([buf[6], buf[7]]);
                    let life = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
                    let granted = if life == 0 {
                        s.mapped.remove(&(op, internal));
                        0
                    } else {
                        s.mapped.insert((op, internal), ext);
                        if s.lifetime > 0 { s.lifetime } else { life }
                    };
                    out.extend_from_slice(&internal.to_be_bytes());
                    out.extend_from_slice(&ext.to_be_bytes());
                    out.extend_from_slice(&granted.to_be_bytes());
                }
                drop(s);
                let _ = sock.send_to(&out, from);
            }
        });
        FakePmp { addr, st }
    }

    fn gateway(&self) -> NatPmp {
        let mut g = NatPmp::new(self.addr);
        g.first_wait = Duration::from_millis(20); // Go: natpmpFirstWait 20 ms
        g
    }

    fn mapped(&self) -> usize {
        self.st.lock().unwrap().mapped.len()
    }
}

#[tokio::test]
async fn natpmp_wire() {
    let f = FakePmp::new([198, 51, 100, 7]);
    let g = f.gateway();
    assert_eq!(
        g.external_ip().await.unwrap(),
        IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7))
    );
    let (got, life) = g
        .map(
            Port {
                proto: "udp",
                internal: 24817,
                external: 0,
            },
            Duration::from_secs(3600),
        )
        .await
        .unwrap();
    assert_eq!((got.external, life), (24817, Duration::from_secs(3600)));
    g.unmap(got).await.unwrap();
    assert_eq!(f.mapped(), 0);
    f.st.lock().unwrap().result = 2;
    let err = g
        .map(
            Port {
                proto: "tcp",
                internal: 1,
                external: 0,
            },
            Duration::from_secs(3600),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("refused"), "{err}");
    {
        let mut s = f.st.lock().unwrap();
        s.result = 0;
        s.silent = true;
    }
    assert_eq!(
        g.external_ip().await.unwrap_err(),
        "natpmp: no answer from the gateway"
    );
}

/// A gateway in process with conflicts, permanent-only leases, refusals,
/// stale entries and a changeable external address.
#[derive(Default)]
struct FakeIgd {
    st: Mutex<IgdState>,
}

#[derive(Default)]
struct IgdState {
    external: String,
    /// (proto, external) -> (owner, lease secs)
    maps: BTreeMap<(&'static str, u16), (String, u64)>,
    taken: Vec<(&'static str, u16)>,
    only_permanent: bool,
    refuse: bool,
}

impl FakeIgd {
    fn new(ext: &str) -> Arc<FakeIgd> {
        let f = FakeIgd::default();
        f.st.lock().unwrap().external = ext.into();
        Arc::new(f)
    }
    fn mapped(&self) -> BTreeMap<(&'static str, u16), (String, u64)> {
        self.st.lock().unwrap().maps.clone()
    }
}

const ME: &str = "127.0.0.1|irori";

impl Gateway for FakeIgd {
    fn method(&self) -> &str {
        "upnp"
    }
    fn external_ip(&self) -> BoxFut<'_, Result<IpAddr, String>> {
        Box::pin(async move {
            self.st
                .lock()
                .unwrap()
                .external
                .parse()
                .map_err(|_| "bad".into())
        })
    }
    fn map(&self, mut p: Port, life: Duration) -> BoxFut<'_, Result<(Port, Duration), MapError>> {
        Box::pin(async move {
            let mut s = self.st.lock().unwrap();
            if s.refuse {
                return Err(MapError::Other("refused".into()));
            }
            if p.external == 0 {
                p.external = p.internal;
            }
            if s.taken.contains(&(p.proto, p.external)) {
                return Err(MapError::Conflict);
            }
            let lease = if s.only_permanent { 0 } else { life.as_secs() };
            s.maps.insert((p.proto, p.external), (ME.into(), lease));
            Ok((p, Duration::from_secs(lease)))
        })
    }
    fn unmap(&self, p: Port) -> BoxFut<'_, Result<(), String>> {
        Box::pin(async move {
            self.st.lock().unwrap().maps.remove(&(p.proto, p.external));
            Ok(())
        })
    }
    fn unmap_stale(&self, p: Port) -> Option<BoxFut<'_, Result<(), String>>> {
        Some(Box::pin(async move {
            let mut s = self.st.lock().unwrap();
            if s.maps
                .get(&(p.proto, p.external))
                .is_some_and(|(o, _)| o == ME)
            {
                s.maps.remove(&(p.proto, p.external));
            }
            Ok(())
        }))
    }
}

#[tokio::test]
async fn mapper_maps_and_close_unmaps() {
    let f = FakeIgd::new("203.0.113.9");
    let m = Mapper::start(config(vec![f.clone()]));
    let s = wait_state(&m, STATE_OK).await;
    assert_eq!(s.method, "upnp");
    assert_eq!(s.external, Some("203.0.113.9".parse().unwrap()));
    assert!(!s.cgnat);
    assert_eq!(f.mapped().len(), 2);
    m.close().await;
    assert!(f.mapped().is_empty(), "close left {:?}", f.mapped());
    assert_eq!(m.status().state, portmap::STATE_NONE);
}

#[tokio::test]
async fn conflict_and_permanent() {
    let f = FakeIgd::new("100.64.3.4");
    {
        let mut s = f.st.lock().unwrap();
        s.taken.push(("tcp", 24817));
        s.only_permanent = true;
    }
    let m = Mapper::start(config(vec![f.clone()]));
    let s = wait_state(&m, STATE_OK).await;
    assert!(s.cgnat, "100.64/10 not reported as CGNAT");
    assert!(
        s.ports[0].external != 24817
            && (20000..=29999).contains(&s.ports[0].external)
            && s.ports[1].external == 24817,
        "ports after a conflict {:?}",
        s.ports
    );
    for (k, (_, lease)) in f.mapped() {
        assert_eq!(lease, 0, "{k:?} not permanent");
    }
    m.close().await;
}

#[tokio::test]
async fn removes_stale_mappings_of_this_machine_only() {
    let f = FakeIgd::new("203.0.113.9");
    {
        let mut s = f.st.lock().unwrap();
        s.maps.insert(("tcp", 25000), (ME.into(), 0));
        s.maps.insert(("udp", 25001), (ME.into(), 0));
        s.maps
            .insert(("tcp", 25002), ("192.168.1.9|irori".into(), 0));
    }
    let mut c = config(vec![f.clone()]);
    c.stale = vec![
        Port {
            proto: "tcp",
            internal: 0,
            external: 25000,
        },
        Port {
            proto: "udp",
            internal: 0,
            external: 25001,
        },
        Port {
            proto: "tcp",
            internal: 0,
            external: 25002,
        },
    ];
    let m = Mapper::start(c);
    wait_state(&m, STATE_OK).await;
    let got = f.mapped();
    assert!(
        !got.contains_key(&("tcp", 25000)),
        "this machine's old TCP mapping stayed"
    );
    assert!(
        !got.contains_key(&("udp", 25001)),
        "this machine's old UDP mapping stayed"
    );
    assert!(
        got.contains_key(&("tcp", 25002)),
        "another machine's mapping was removed"
    );
    m.close().await;
}

#[tokio::test]
async fn notices_a_new_external_address() {
    let f = FakeIgd::new("203.0.113.9");
    let m = Mapper::start(config(vec![f.clone()]));
    wait_state(&m, STATE_OK).await;
    f.st.lock().unwrap().external = "198.51.100.4".into();
    for _ in 0..500 {
        if m.status().external == Some("198.51.100.4".parse().unwrap()) {
            m.close().await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("status {:?}", m.status());
}

/// UPnP refused, NAT-PMP answers: the mapper falls through, and renews at
/// half the granted lifetime.
#[tokio::test]
async fn falls_back_and_renews() {
    let igd = FakeIgd::new("203.0.113.9");
    igd.st.lock().unwrap().refuse = true;
    let pmp = FakePmp::new([203, 0, 113, 9]);
    pmp.st.lock().unwrap().lifetime = 1;
    let m = Mapper::start(config(vec![igd.clone(), Arc::new(pmp.gateway())]));
    let s = wait_state(&m, STATE_OK).await;
    assert!(
        s.method == "natpmp" && pmp.mapped() == 2 && igd.mapped().is_empty(),
        "status {s:?}"
    );
    let before = pmp.st.lock().unwrap().reqs;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let after = pmp.st.lock().unwrap().reqs;
    assert!(
        after >= before + 2,
        "no renewal: {before} requests then {after}"
    );
    m.close().await;
    assert_eq!(pmp.mapped(), 0, "close left mappings");
}

#[tokio::test]
async fn fails_without_a_router() {
    let mut c = config(vec![]);
    c.discover = Some(Arc::new(|| {
        Box::pin(async { Err("no gateway".to_string()) })
    }));
    let m = Mapper::start(c);
    let s = wait_state(&m, STATE_FAILED).await;
    assert!(s.detail.contains("no gateway"), "detail {:?}", s.detail);
    m.close().await;

    let pmp = FakePmp::new([203, 0, 113, 9]);
    pmp.st.lock().unwrap().silent = true;
    let m2 = Mapper::start(config(vec![Arc::new(pmp.gateway())]));
    wait_state(&m2, STATE_FAILED).await;
    m2.close().await;
}

#[test]
fn cgnat_ranges() {
    for (a, want) in [
        ("100.64.0.1", true),
        ("100.127.255.254", true),
        ("192.168.0.1", true),
        ("10.1.1.1", true),
        ("203.0.113.9", false),
        ("8.8.8.8", false),
        ("100.128.0.1", false),
    ] {
        assert_eq!(portmap::cgnat(a.parse().unwrap()), want, "{a}");
    }
}
