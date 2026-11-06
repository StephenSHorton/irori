//! Pairing on iroh, loopback only (architecture §6.5; the iroh twins of
//! I:internal/mesh/roomnet/pair_test.go): real identities, stores and
//! Chains, `irori_net::transport::IrohNet` on 127.0.0.1, LAN lookups that
//! unicast to each other on 127.0.0.1, and the home server's real relay
//! (`home::relay::Relay`) on 127.0.0.1 with guest windows.
//!
//! - on the LAN by the code alone (A announces the code-derived room);
//! - away from home as a guest of A's relay (B types `127.0.0.1:N`);
//! - through the server's bridge (A away on S's relay only, B on S's LAN
//!   only).
//!
//! Each ends with both devices meeting in the person's mesh room on iroh,
//! admitted by the Chain alone (the M5 mesh session then syncs, as the
//! FakeNet flow test shows).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use gocompat::time::GoTime;
use iroh::{EndpointId, SecretKey};
use irori_identity::{Chain, DeviceJson, FileKeystore, Identity, PERSON_FILE};
use irori_meshseam::{Conn, RoomConfig, Transport, mesh_secret};
use irori_net::admit::{Admit, ChainAdmit, admitted_device, check_device};
use irori_net::endpoint::{IpMode, RelaySpec};
use irori_net::guest::GuestWindows;
use irori_net::pairnet::PairLan;
use irori_net::transport::{IrohNet, NetConfig, PeerAddr};
use irori_pair::*;
use store::{Filter, Store};

const LO: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

async fn eventually(what: &str, within: Duration, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        if start.elapsed() > within {
            panic!("timed out waiting for: {what}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn free_udp() -> SocketAddr {
    std::net::UdpSocket::bind((LO, 0))
        .unwrap()
        .local_addr()
        .unwrap()
}

/// A port free on both TCP and UDP.
fn free_port() -> u16 {
    loop {
        let t = std::net::TcpListener::bind((LO, 0)).unwrap();
        let p = t.local_addr().unwrap().port();
        if std::net::UdpSocket::bind((LO, p)).is_ok() {
            return p;
        }
    }
}

struct Dev {
    name: String,
    _tmp: tempfile::TempDir,
    dir: std::path::PathBuf,
    ident: Arc<Identity>,
    chain: Arc<Chain>,
    st: Arc<Store>,
    net: IrohNet,
    guests: GuestWindows,
    pair: Pairer,
    lan: Option<irori_lan::LanLookup>,
    relay: Option<home::relay::Relay>,
    conns: Arc<Mutex<Vec<Arc<dyn Conn>>>>,
}

struct How {
    ip: IpMode,
    /// The LAN port to bind and the ports to unicast to.
    lan: Option<(SocketAddr, Vec<SocketAddr>)>,
    /// Join `root`'s person before the net is built (a certified member).
    member_of: Option<(std::path::PathBuf, Arc<Identity>, Arc<Store>)>,
}

fn open_parts(dir: &std::path::Path, name: &str) -> (Arc<Identity>, Arc<Chain>, Arc<Store>) {
    let mut dv = DeviceJson::load_or_create(dir).unwrap();
    dv.name = name.into();
    dv.save(dir).unwrap();
    let ident = Arc::new(Identity::load(dir, dv.clone(), Some(Box::new(FileKeystore))).unwrap());
    let p = ident.current();
    let chain = Arc::new(Chain::new(&p.id, &p.root_device_id, &dv.id));
    let st = Arc::new(
        Store::open_with(
            dir.join("irori.db"),
            &dv.id,
            Arc::new(store::Clock::with_now(Box::new(SystemTime::now))),
            store::OpenOptions {
                signer: Some(ident.clone()),
                verifier: Some(chain.clone()),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    ident
        .ensure_self_cert(&*st, &chain, irori_identity::ROLE_DESKTOP)
        .unwrap();
    (ident, chain, st)
}

fn learn_person(chain: &Chain, from: &Store) {
    let evs = from
        .list(&Filter {
            kinds: vec!["person.".into()],
            limit: 1000,
            ..Filter::default()
        })
        .unwrap();
    for _ in 0..evs.len() {
        for ev in &evs {
            let _ = chain.learn(ev.into());
        }
    }
}

impl Dev {
    fn new(name: &str, how: How) -> Dev {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        if let Some((root_dir, ..)) = &how.member_of {
            std::fs::copy(root_dir.join(PERSON_FILE), dir.join(PERSON_FILE)).unwrap();
        }
        let (ident, chain, st) = open_parts(&dir, name);
        if let Some((_, root, root_st)) = &how.member_of {
            let add = root
                .cert_for(
                    &ident.device_id(),
                    &hex::encode(ident.device_pub()),
                    name,
                    irori_identity::ROLE_DESKTOP,
                    GoTime::from_system(SystemTime::now()),
                )
                .unwrap();
            root_st
                .append("irori", irori_identity::kinds::PERSON_DEVICE_ADDED, &add)
                .unwrap();
            // The root's log, as a sync would have brought it.
            let rows = root_st
                .list(&Filter {
                    device_id: root.device_id(),
                    limit: 1000,
                    ..Filter::default()
                })
                .unwrap();
            let (_, r) = st.insert_batch(&rows);
            r.unwrap();
            learn_person(&chain, root_st);
        }
        let key = SecretKey::from_bytes(&ident.expose_device_seed());
        let guests = GuestWindows::default();
        let pl = PairLan::new(guests.clone());
        let lan = match how.lan {
            Some((bind, to)) => {
                let mut c = irori_lan::Config::new(key.public(), Arc::new(Vec::new));
                c.bind = Some(bind);
                c.targets = Some(Arc::new(move || to.clone()));
                c.every = Duration::from_millis(200);
                pl.wire(&mut c);
                let l = irori_lan::LanLookup::start(c);
                pl.attach(l.clone());
                Some(l)
            }
            None => None,
        };
        let admit: Arc<dyn Admit> = Arc::new(ChainAdmit {
            chain: chain.clone(),
            self_id: ident.device_id(),
        });
        let mut nc = NetConfig::new(ident.device_id(), key, admit);
        nc.ip = how.ip;
        nc.dial_grace = Duration::from_millis(300);
        nc.connect_timeout = Duration::from_secs(10);
        nc.lan = lan.clone();
        nc.guests = guests.clone();
        nc.pair_lan = pl;
        let net = IrohNet::new(nc);
        let mut o = Options::new(
            Arc::new(net.clone()),
            ident.clone(),
            chain.clone(),
            st.clone(),
        );
        o.data_dir = Some(dir.clone());
        o.hello_every = Duration::from_millis(200);
        let n = name.to_string();
        o.logf = Some(Arc::new(move |m: &str| eprintln!("{n}: {m}")));
        let pair = Pairer::new(o);
        Dev {
            name: name.into(),
            _tmp: tmp,
            dir,
            ident,
            chain,
            st,
            net,
            guests,
            pair,
            lan,
            relay: None,
            conns: Arc::default(),
        }
    }

    fn id(&self) -> String {
        self.ident.device_id()
    }

    fn eid(&self) -> EndpointId {
        EndpointId::from_bytes(&self.ident.device_pub()).unwrap()
    }

    fn state(&self) -> String {
        self.pair.status().state
    }

    /// Runs the home server's relay on 127.0.0.1 with this device's guest
    /// windows, and uses it as this device's own relay.
    async fn serve(&mut self) -> RelaySpec {
        let chain = self.chain.clone();
        let me = self.id();
        let self_pub = self.ident.device_pub();
        let admit: home::relay::AdmitFn = Arc::new(move |id: &EndpointId| {
            *id.as_bytes() == self_pub || admitted_device(&chain, &me, id.as_bytes()).is_some()
        });
        let g = self.guests.clone();
        let guest: home::relay::GuestFn = Arc::new(move |t: &str| g.token_ok(t));
        let seed = self.ident.expose_device_seed();
        let r = loop {
            match home::relay::Relay::spawn(
                &seed,
                LO,
                free_port(),
                false,
                admit.clone(),
                Some(guest.clone()),
            )
            .await
            {
                Ok(r) => break r,
                Err(home::relay::SpawnError::Bind(_)) => continue,
                Err(e) => panic!("relay: {e}"),
            }
        };
        let spec = RelaySpec {
            url: irori_net::relay::relay_url(LO, r.port()),
            quic_port: r.port(),
            server_pub: self.eid(),
        };
        self.net.set_servers(vec![spec.clone()]).await;
        self.relay = Some(r);
        spec
    }

    fn hub_addr(&self) -> String {
        format!("127.0.0.1:{}", self.relay.as_ref().unwrap().port())
    }

    /// Joins the person's mesh room on iroh, admitted by the Chain alone.
    async fn mesh(&self) -> Arc<dyn irori_meshseam::Room> {
        let p = self.ident.current();
        let seed = self.ident.person_seed(p.epoch).unwrap();
        let mut cfg = RoomConfig::new(self.id());
        let (chain, me) = (self.chain.clone(), self.id());
        cfg.accept = Arc::new(move |peer, k| check_device(&chain, &me, peer, k));
        let conns = self.conns.clone();
        cfg.on_peer = Arc::new(move |c| conns.lock().unwrap().push(c));
        self.net
            .join(mesh_secret(&seed[..], &p.id, p.epoch), cfg)
            .await
            .unwrap()
    }

    fn peer_addr(&self, relay: Option<&RelaySpec>) -> PeerAddr {
        PeerAddr {
            device_id: self.id(),
            id: self.eid(),
            relay: relay.map(|r| r.url.clone()),
            direct: self
                .net
                .endpoint()
                .map(|e| e.bound_sockets())
                .unwrap_or_default(),
        }
    }

    fn meets(&self, peer: &str) -> bool {
        self.conns
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.id() == peer && c.err().is_none())
    }

    async fn close(&self) {
        self.pair.cancel();
        self.net.close().await;
        if let Some(l) = &self.lan {
            l.close().await;
        }
        if let Some(r) = &self.relay {
            r.close().await;
        }
        let _ = self.st.close();
    }
}

const PAIR_WAIT: Duration = Duration::from_secs(45);

/// One pairing from `a` (A) to `b` (B) to the end: B joins with the code
/// (and server, when set), both reach the words, the person types them into
/// A and A's key into B.
async fn pair_up(a: &Dev, b: &Dev, server: &str) -> Status {
    let st = a.pair.add(irori_identity::ROLE_DESKTOP).await.unwrap();
    b.pair.join_via(&st.code, server).await.unwrap();
    eventually("both reach the words", PAIR_WAIT, || {
        a.state() == STATE_CONFIRM && b.state() == STATE_WORDS
    })
    .await;
    a.pair
        .confirm(&b.pair.status().words.join(" "))
        .await
        .unwrap();
    b.pair.confirm_key(&st.fingerprint).await.unwrap();
    eventually("both paired", PAIR_WAIT, || {
        a.state() == STATE_DONE && b.state() == STATE_DONE
    })
    .await;
    assert_eq!(
        b.ident.current().id,
        a.ident.current().id,
        "b didn't take a's person"
    );
    assert!(a.chain.cert(&b.id()).is_some(), "a holds no cert for b");
    assert!(
        b.chain.cert(&b.id()).is_some(),
        "b doesn't hold its own cert"
    );
    st
}

fn assert_loopback(d: &Dev) {
    if let Some(ep) = d.net.endpoint() {
        for s in ep.bound_sockets() {
            assert!(s.ip().is_loopback(), "{}: non-loopback socket {s}", d.name);
        }
    }
}

/// Pairing at home by the code alone: A announces the code's room on the
/// LAN, B hears it and dials; the words cover the TLS exporter of that one
/// QUIC connection; the two then meet in the person's mesh room.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pair_on_the_lan() {
    let (la, lb) = (free_udp(), free_udp());
    let a = Dev::new(
        "pc",
        How {
            ip: IpMode::Loopback,
            lan: Some((la, vec![lb])),
            member_of: None,
        },
    );
    let b = Dev::new(
        "mac",
        How {
            ip: IpMode::Loopback,
            lan: Some((lb, vec![la])),
            member_of: None,
        },
    );
    pair_up(&a, &b, "").await;
    assert_loopback(&a);
    assert_loopback(&b);
    assert!(
        !a.net.pairing() && !b.net.pairing(),
        "a pairing room stayed open"
    );
    assert!(!a.guests.any_open(), "a's guest window stayed open");
    let (ra, rb) = (a.mesh().await, b.mesh().await);
    a.net.set_peers(vec![b.peer_addr(None)]);
    b.net.set_peers(vec![a.peer_addr(None)]);
    ra.connect(&b.id());
    rb.connect(&a.id());
    eventually("a and b meet in the mesh room", PAIR_WAIT, || {
        a.meets(&b.id()) && b.meets(&a.id())
    })
    .await;
    a.close().await;
    b.close().await;
}

/// Joining from away from home: B has no LAN in common with A, which runs
/// the home server. B types the server's address; its relay admits B as a
/// guest of the pairing window only, and after pairing B is a member there.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pair_as_a_guest_of_the_hub() {
    let mut a = Dev::new(
        "pc",
        How {
            ip: IpMode::RelayOnly,
            lan: None,
            member_of: None,
        },
    );
    let b = Dev::new(
        "mac",
        How {
            ip: IpMode::RelayOnly,
            lan: None,
            member_of: None,
        },
    );
    let spec = a.serve().await;
    a.net.bind().await.unwrap();
    let st = pair_up(&a, &b, &a.hub_addr()).await;
    // The window is closed to guests again: the code's token opens nothing.
    let token = irori_net::guest::guest_token(&room_secret(&st.code).unwrap().key);
    assert!(
        !a.guests.token_ok(&token),
        "the guest window outlived the pairing"
    );
    // B now reaches A's relay as a member and meets A in the mesh room.
    b.net.set_servers(vec![spec.clone()]).await;
    let (ra, rb) = (a.mesh().await, b.mesh().await);
    a.net.set_peers(vec![b.peer_addr(Some(&spec))]);
    b.net.set_peers(vec![a.peer_addr(Some(&spec))]);
    ra.connect(&b.id());
    rb.connect(&a.id());
    eventually(
        "a and b meet in the mesh room through a's relay",
        PAIR_WAIT,
        || a.meets(&b.id()) && b.meets(&a.id()),
    )
    .await;
    a.close().await;
    b.close().await;
}

/// The bridge: A is away (on the server's relay only), B is at home with
/// only the code (on the server's LAN only), and the home server S carries
/// A's pairing room between its LAN and its relay.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pair_through_the_servers_bridge() {
    let (ls, lb) = (free_udp(), free_udp());
    let mut s = Dev::new(
        "node",
        How {
            ip: IpMode::RelayOnly,
            lan: Some((ls, vec![lb])),
            member_of: None,
        },
    );
    let a = Dev::new(
        "pc",
        How {
            ip: IpMode::RelayOnly,
            lan: None,
            member_of: Some((s.dir.clone(), s.ident.clone(), s.st.clone())),
        },
    );
    learn_person(&s.chain, &s.st);
    assert!(s.chain.cert(&a.id()).is_some(), "s doesn't hold a's cert");
    let b = Dev::new(
        "mac",
        How {
            ip: IpMode::RelayOnly,
            lan: Some((lb, vec![ls])),
            member_of: None,
        },
    );
    let spec = s.serve().await;
    s.net.bind().await.unwrap();
    a.net.set_servers(vec![spec.clone()]).await;
    a.net.bind().await.unwrap();
    // The bridge names the server's relay at the address the LAN reaches.
    let pl = test_pair_lan(&s);
    pl.set_relay(Some((spec.url.clone(), s.eid())));
    pair_up(&a, &b, "").await;
    assert_eq!(
        b.ident.current().root_device_id,
        s.id(),
        "b joined another person"
    );
    assert!(
        !s.guests.any_open(),
        "s kept a's guest window after the pairing"
    );
    a.close().await;
    b.close().await;
    s.close().await;
}

/// The server's PairLan (the one its LAN lookup was wired with).
fn test_pair_lan(d: &Dev) -> PairLan {
    d.net.pair_lan().clone()
}
