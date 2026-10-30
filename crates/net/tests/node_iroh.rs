//! The real mesh Node (mesh.md §1-§3: session, hello, rotation, restore
//! and fork) on the iroh transport, on loopback (architecture §6.5).
//!
//! Each device is the mesh crate's FakeNet harness device (a real store,
//! keys and Chain), but its Node runs on its own [`IrohNet`] through
//! the seam: rows, `person.secret` frames, the rotation tie-break,
//! `provesOwn` and `row.at` all cross real QUIC connections. The address
//! book is fed as the Core's would be: every bound endpoint's EndpointId
//! and loopback sockets (or only the home relay).

#[path = "../../mesh/tests/common/mod.rs"]
mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{Dev, converged, eventually, never};
use irori_identity::DeviceIdentity;
use irori_net::admit::{ChainAccess, ChainAdmit};
use irori_net::book::{self, ServerAddrs};
use irori_net::endpoint::{IpMode, RelaySpec};
use irori_net::identity::IrohKey as _;
use irori_net::relay::{self, EmbeddedRelay};
use irori_net::transport::{IrohNet, NetConfig, PeerAddr};
use mesh::{FakeNet, Node, mesh_secret};
use serde_json::json;
use store::Event;

const LO: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// The devices' address book: each bound endpoint, pushed to every other
/// device's `set_peers` whenever one (re)binds. A cut pair leaves each
/// other out and drops their connections (a partition: the loopback
/// endpoints have no other way to find each other).
#[derive(Default, Clone)]
struct Book {
    nets: Arc<Mutex<BTreeMap<String, (IrohNet, PeerAddr)>>>,
    cut: Arc<Mutex<BTreeSet<(String, String)>>>,
}

impl Book {
    fn put(&self, net: IrohNet, addr: PeerAddr) {
        self.nets
            .lock()
            .unwrap()
            .insert(addr.device_id.clone(), (net, addr));
        self.push();
    }

    fn is_cut(&self, a: &str, b: &str) -> bool {
        let g = self.cut.lock().unwrap();
        g.contains(&(a.to_string(), b.to_string())) || g.contains(&(b.to_string(), a.to_string()))
    }

    fn push(&self) {
        let g = self.nets.lock().unwrap();
        for (id, (n, _)) in g.iter() {
            n.set_peers(
                g.iter()
                    .filter(|(o, _)| *o != id && !self.is_cut(id, o))
                    .map(|(_, (_, a))| a.clone())
                    .collect(),
            );
        }
    }

    fn partition(&self, a: &str, b: &str) {
        self.cut
            .lock()
            .unwrap()
            .insert((a.to_string(), b.to_string()));
        self.push();
        let g = self.nets.lock().unwrap();
        if let Some((n, _)) = g.get(a) {
            n.drop_device(b, "leaving");
        }
        if let Some((n, _)) = g.get(b) {
            n.drop_device(a, "leaving");
        }
    }

    fn heal(&self) {
        self.cut.lock().unwrap().clear();
        self.push();
    }
}

/// One device: the harness device and the iroh transport its Node runs on.
struct IDev {
    d: Dev,
    net: Option<IrohNet>,
    ip: IpMode,
    relay: Option<RelaySpec>,
    book: Book,
}

impl IDev {
    fn new(fake: &FakeNet, name: &str, book: &Book, ip: IpMode) -> IDev {
        IDev {
            d: Dev::new(fake, name),
            net: None,
            ip,
            relay: None,
            book: book.clone(),
        }
    }

    fn id(&self) -> String {
        self.d.id()
    }

    /// Starts the Node on a fresh endpoint keyed by the device key, gated by
    /// this (re)opened Chain, and puts it in the book.
    async fn start(&mut self) {
        if let Some(old) = self.net.take() {
            old.close().await;
        }
        let key = iroh::SecretKey::from_bytes(&self.d.ident.expose_device_seed());
        let admit = Arc::new(ChainAdmit {
            chain: self.d.chain.clone(),
            self_id: self.id(),
        });
        let mut cfg = NetConfig::new(self.id(), key, admit);
        cfg.ip = self.ip;
        cfg.dial_grace = Duration::from_millis(300);
        cfg.connect_timeout = Duration::from_secs(10);
        cfg.gossip = false;
        let net = IrohNet::new(cfg);
        if let Some(r) = &self.relay {
            net.set_servers(vec![r.clone()]).await;
        }
        let ep = net.bind().await.expect("bind");
        let direct = ep.bound_sockets();
        for a in &direct {
            assert!(a.ip().is_loopback(), "non-loopback socket {a}");
        }
        self.book.put(
            net.clone(),
            PeerAddr {
                device_id: self.id(),
                id: iroh::EndpointId::from_bytes(&self.d.ident.device_pub()).unwrap(),
                relay: self.relay.as_ref().map(|r| r.url.clone()),
                direct,
            },
        );
        let node = Node::new(self.d.options(Arc::new(net.clone())));
        self.net = Some(net);
        self.d.start_node(node);
    }

    async fn stop(&mut self) {
        self.d.stop().await;
    }

    async fn close(&mut self) {
        self.d.close().await;
        if let Some(n) = self.net.take() {
            n.close().await;
        }
    }

    fn rooms(&self) -> Vec<i64> {
        self.net.as_ref().map(|n| n.rooms()).unwrap_or_default()
    }

    fn connected(&self, peer: &str) -> bool {
        self.net
            .as_ref()
            .is_some_and(|n| n.conns().iter().any(|c| c.peer == peer))
    }
}

fn seen(d: &Dev, tag: &str) -> serde_json::Value {
    json!({"key": format!("{}/{tag}", d.id()), "agent": "a"})
}

/// PC (the root) and `n - 1` more devices, paired, started and synced.
async fn person(names: &[&str], ip: IpMode) -> (FakeNet, Book, Vec<IDev>) {
    let (fake, book) = (FakeNet::new(), Book::default());
    let mut devs: Vec<IDev> = names
        .iter()
        .map(|n| IDev::new(&fake, n, &book, ip))
        .collect();
    let (root, rest) = devs.split_first_mut().unwrap();
    for d in rest.iter_mut() {
        d.d.join(&root.d).await;
    }
    for d in rest.iter() {
        d.d.learn(&root.d);
    }
    for d in devs.iter_mut() {
        d.start().await;
    }
    {
        let all: Vec<&Dev> = devs.iter().map(|d| &d.d).collect();
        eventually("every device converges over iroh", || converged(&all)).await;
    }
    assert_eq!(fake.joins(), 0, "a Node ran on FakeNet, not iroh");
    (fake, book, devs)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_node_syncs_live_rows_and_history_over_iroh() {
    let (fake, book) = (FakeNet::new(), Book::default());
    let (mut a, mut b) = (
        IDev::new(&fake, "pc", &book, IpMode::Loopback),
        IDev::new(&fake, "mac", &book, IpMode::Loopback),
    );
    let before = b.d.append(store::kinds::AGENT_SEEN, seen(&b.d, "s1"));
    b.d.join(&a.d).await;
    a.start().await;
    b.start().await;
    a.d.node().kick();
    eventually("a and b converge", || converged(&[&a.d, &b.d])).await;
    assert!(a.d.has(&before), "b's row from before pairing");
    let ea = a.d.append(store::kinds::AGENT_SEEN, seen(&a.d, "s2"));
    let eb = b.d.append(store::kinds::AGENT_SEEN, seen(&b.d, "s3"));
    eventually("live rows cross", || a.d.has(&eb) && b.d.has(&ea)).await;
    eventually("b is connected", || a.d.peer(&b.id()).state == "connected").await;
    let p = a.d.peer(&b.id());
    assert_eq!(p.path, "direct", "loopback is a direct path: {p:?}");
}

/// Relay-only endpoints: the Node's session crosses the home relay (pinned
/// to the server's device key) with no direct path at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_node_syncs_through_the_home_relay_alone() {
    let (fake, book) = (FakeNet::new(), Book::default());
    let mut devs = [
        IDev::new(&fake, "pc", &book, IpMode::RelayOnly),
        IDev::new(&fake, "mac", &book, IpMode::RelayOnly),
    ];
    let (pc, mac) = devs.split_at_mut(1);
    mac[0].d.join(&pc[0].d).await;
    // The PC is the home server: its relay admits the person's devices.
    let ident = DeviceIdentity::from_seed_for_tests(*pc[0].d.ident.expose_device_seed());
    let access = ChainAccess {
        admit: ChainAdmit {
            chain: pc[0].d.chain.clone(),
            self_id: pc[0].id(),
        },
        self_pub: ident.endpoint_id(),
    };
    let r: EmbeddedRelay = relay::spawn_with(&ident, LO, 0, access).await.unwrap();
    let spec = RelaySpec {
        url: r.url_for(LO),
        quic_port: r.port,
        server_pub: ident.endpoint_id(),
    };
    for d in devs.iter_mut() {
        d.relay = Some(spec.clone());
        d.start().await;
    }
    let (a, b) = (&devs[0], &devs[1]);
    let e = b.d.append(store::kinds::AGENT_SEEN, seen(&b.d, "relayed"));
    eventually("rows cross the relay", || {
        a.d.has(&e) && converged(&[&a.d, &b.d])
    })
    .await;
    assert_eq!(a.d.peer(&b.id()).path, "relay");
}

/// The home server reaches its peers through its own relay (Go
/// `Net.SetLocal`): the PC's endpoint is fed as the Core feeds it while its
/// server runs (`book::own_relay`, 127.0.0.1), the Mac only knows the
/// server by a typed host name (`localhost:N`, a `hub-addrs.json` hint or a
/// manual public address), so the two are homed on two URLs of one relay
/// process. Both are relay-only, and the PC is the lower id, so the PC
/// dials the Mac.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_server_reaches_peers_through_its_own_relay_and_a_name_finds_it() {
    let fake = FakeNet::new();
    let (mut pc, mut mac) = loop {
        let (pc, mac) = (Dev::new(&fake, "pc"), Dev::new(&fake, "mac"));
        if pc.id() < mac.id() {
            break (pc, mac);
        }
    };
    mac.join(&pc).await;
    let ident = DeviceIdentity::from_seed_for_tests(*pc.ident.expose_device_seed());
    let access = ChainAccess {
        admit: ChainAdmit {
            chain: pc.chain.clone(),
            self_id: pc.id(),
        },
        self_pub: ident.endpoint_id(),
    };
    let r: EmbeddedRelay = relay::spawn_with(&ident, LO, 0, access).await.unwrap();
    let net_for = |d: &Dev| {
        let key = iroh::SecretKey::from_bytes(&d.ident.expose_device_seed());
        let admit = Arc::new(ChainAdmit {
            chain: d.chain.clone(),
            self_id: d.id(),
        });
        let mut cfg = NetConfig::new(d.id(), key, admit);
        cfg.ip = IpMode::RelayOnly;
        cfg.dial_grace = Duration::from_secs(30); // only the lower id dials
        cfg.connect_timeout = Duration::from_secs(10);
        cfg.gossip = false;
        IrohNet::new(cfg)
    };
    let (pc_net, mac_net) = (net_for(&pc), net_for(&mac));
    // The PC runs no node.addr row here: only its own relay.
    book::feed(
        &pc_net,
        &[],
        &pc.chain,
        Some(book::own_relay(r.port, ident.endpoint_id())),
    )
    .await;
    book::feed(
        &mac_net,
        &[ServerAddrs {
            device_id: pc.id(),
            addrs: vec![format!("localhost:{}", r.port)],
        }],
        &mac.chain,
        None,
    )
    .await;
    for (d, n) in [(&mut pc, &pc_net), (&mut mac, &mac_net)] {
        let ep = n.bind().await.expect("bind");
        assert!(
            ep.bound_sockets().is_empty(),
            "relay-only binds no IP socket"
        );
        d.start_node(Node::new(d.options(Arc::new(n.clone()))));
    }
    pc.node().kick();
    let (e1, e2) = (
        pc.append(store::kinds::AGENT_SEEN, seen(&pc, "from-server")),
        mac.append(store::kinds::AGENT_SEEN, seen(&mac, "to-server")),
    );
    eventually("rows cross both ways through the server's relay", || {
        mac.has(&e1) && pc.has(&e2)
    })
    .await;
    let conns = pc_net.conns();
    assert_eq!(conns.len(), 1, "{conns:?}");
    assert_eq!(conns[0].dialer, pc.id(), "the server dialed: {conns:?}");
    assert_eq!(conns[0].path, "relay", "{conns:?}");
    assert_eq!(pc.peer(&mac.id()).path, "relay");
    // Each is homed on its own URL of the one relay.
    let home = |n: &IrohNet| -> Vec<String> {
        n.endpoint()
            .unwrap()
            .addr()
            .relay_urls()
            .map(|u| u.to_string())
            .collect()
    };
    assert_eq!(home(&pc_net), vec![r.url_for(LO).to_string()]);
    assert_eq!(
        home(&mac_net),
        vec![format!("https://localhost:{}/", r.port)]
    );
    // The name went to Irori's resolver (answered locally), the PC asked none.
    assert!(
        pc_net.dns_log().asked().is_empty(),
        "{:?}",
        pc_net.dns_log().asked()
    );
    let asked = mac_net.dns_log().asked();
    assert!(
        !asked.is_empty() && asked.iter().all(|(n, k)| n == "localhost" && *k != "txt"),
        "{asked:?}"
    );
    pc.close().await;
    mac.close().await;
    pc_net.close().await;
    mac_net.close().await;
}

/// Remove device on the real session: the PC revokes the Mac and rotates;
/// the node takes the revoke and the new key (`person.secret` on the
/// wire), the old epoch's room closes, and the Mac can't come back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_rotates_the_key_over_iroh_and_the_mac_stays_out() {
    let (_f, _b, mut devs) = person(&["pc", "mac", "node"], IpMode::Loopback).await;
    devs[1].stop().await; // the Mac is away
    let (pc, nd) = (&devs[0], &devs[2]);
    let mac_id = devs[1].id();
    pc.d.node().revoke(&mac_id, "lost").await.unwrap();
    assert_eq!(pc.d.ident.current().epoch, 2);
    eventually("the node takes the revoke and the new key", || {
        nd.d.chain.cut(&mac_id).is_some()
            && nd.d.ident.current().epoch == 2
            && nd.d.ident.current().pub_hex == pc.d.ident.current().pub_hex
    })
    .await;
    eventually("both close the epoch-1 room", || {
        pc.rooms() == vec![2] && nd.rooms() == vec![2]
    })
    .await;
    let e = pc.d.append(store::kinds::AGENT_SEEN, seen(&pc.d, "after"));
    eventually("sync runs on epoch 2", || nd.d.has(&e)).await;
    // The Mac comes back on its old key: refused at the handshake.
    devs[1].start().await;
    let (pc, mac, nd) = (&devs[0], &devs[1], &devs[2]);
    never("the revoked Mac connects", || {
        pc.connected(&mac.id()) || nd.connected(&mac.id()) || mac.connected(&pc.id())
    })
    .await;
}

/// The PC and the node can't reach each other and each removes the lost
/// Mac: two rotations. Once they meet over iroh the lower device id's
/// rotation wins on both (the `person.secret` frames and the tie-break run
/// on the real session), and they share one epoch-2 room.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_devices_that_revoke_apart_settle_on_one_key_over_iroh() {
    let (_f, book, mut devs) = person(&["pc", "mac", "node"], IpMode::Loopback).await;
    devs[1].stop().await;
    let (pc, mac_id, nd) = (&devs[0], devs[1].id(), &devs[2]);
    book.partition(&pc.id(), &nd.id());
    eventually("the PC and the node are apart", || {
        !pc.connected(&nd.id()) && !nd.connected(&pc.id())
    })
    .await;
    pc.d.node().revoke(&mac_id, "lost").await.unwrap();
    nd.d.node().revoke(&mac_id, "lost").await.unwrap();
    assert_ne!(
        pc.d.ident.current().pub_hex,
        nd.d.ident.current().pub_hex,
        "the two rotations made one key"
    );
    let winner = if nd.id() < pc.id() { nd } else { pc };
    let want = winner.d.ident.current().pub_hex;
    let topic = mesh_secret(&winner.d.seed(2), &pc.d.ident.current().id, 2).topic;
    assert!(topic.starts_with("irori/mesh/1/"));
    book.heal();
    eventually("both hold the winning key", || {
        pc.d.ident.current().pub_hex == want
            && nd.d.ident.current().pub_hex == want
            && pc.d.ident.current().epoch == 2
            && nd.d.ident.current().epoch == 2
    })
    .await;
    eventually("both sit in the one epoch-2 room", || {
        pc.rooms() == vec![2] && nd.rooms() == vec![2]
    })
    .await;
    let e = pc.d.append(store::kinds::AGENT_SEEN, seen(&pc.d, "after"));
    eventually("sync runs on it", || nd.d.has(&e)).await;
}

async fn restored_pair() -> (FakeNet, Book, Vec<IDev>, tempfile::TempDir) {
    let (f, book, mut devs) = person(&["pc", "mac"], IpMode::Loopback).await;
    devs[1].close().await;
    let backup = tempfile::tempdir().unwrap();
    Dev::copy_dir(&devs[1].d.dir, backup.path());
    devs[1].d.open();
    devs[1].start().await;
    (f, book, devs, backup)
}

/// A device restored from a backup proves its key in hello (`provesOwn`),
/// holds its appends, and gets its own later rows back over iroh.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restored_device_gets_its_own_rows_back_over_iroh() {
    let (_f, _book, mut devs, backup) = restored_pair().await;
    let later: Vec<Event> = (0..5)
        .map(|i| {
            devs[1].d.append(
                store::kinds::AGENT_SEEN,
                json!({"key": format!("{}/later", devs[1].id()), "i": i}),
            )
        })
        .collect();
    eventually("a has b's later rows", || devs[0].d.has(&later[4])).await;
    devs[1].close().await;
    Dev::copy_dir(backup.path(), &devs[1].d.dir);
    devs[1].d.open();
    assert!(devs[1].d.vector().get(&devs[1].id()).copied().unwrap_or(0) < later[4].seq);
    devs[1].start().await;
    let (a, b) = (&devs[0], &devs[1]);
    eventually("b gets its own rows back", || b.d.has(&later[4])).await;
    let mut ev = None;
    eventually("b can append again", || {
        ev =
            b.d.st
                .append("irori", store::kinds::AGENT_SEEN, &seen(&b.d, "new"))
                .ok();
        ev.is_some()
    })
    .await;
    let ev = ev.unwrap();
    assert_eq!(ev.seq, later[4].seq + 1, "b's next row");
    eventually("a takes it", || a.d.has(&ev)).await;
    assert!(a.d.peer(&b.id()).stopped.is_empty());
}

/// A restored device that wrote over its old rows before syncing is a
/// fork: the session's `row.at` check finds it on the real connection.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restored_device_that_wrote_over_its_rows_is_a_fork_over_iroh() {
    let (_f, _book, mut devs, backup) = restored_pair().await;
    let later = devs[1]
        .d
        .append(store::kinds::AGENT_SEEN, seen(&devs[1].d, "later"));
    eventually("a has it", || devs[0].d.has(&later)).await;
    devs[1].close().await;
    Dev::copy_dir(backup.path(), &devs[1].d.dir);
    devs[1].d.open();
    let other = devs[1]
        .d
        .append(store::kinds::AGENT_SEEN, seen(&devs[1].d, "other"));
    assert_eq!(other.seq, later.seq);
    devs[1].start().await;
    let (a, b) = (&devs[0], &devs[1]);
    eventually("a flags b as signing two histories", || {
        !a.d.peer(&b.id()).stopped.is_empty()
    })
    .await;
    assert!(!a.d.st.forks().unwrap().is_empty(), "no fork recorded on a");
}

/// An attachment offered on each device is fetched by the other over
/// `blob/` streams of the real Node's Files, both ways at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attachments_cross_both_ways_over_iroh() {
    let (_f, _b, devs) = person(&["pc", "mac"], IpMode::Loopback).await;
    let (a, b) = (&devs[0], &devs[1]);
    let tmp = tempfile::tempdir().unwrap();
    let attach = |d: &Dev, name: &str, size: usize| {
        let mut data = vec![0u8; size];
        getrandom::fill(&mut data).unwrap();
        let p = tmp.path().join(name);
        std::fs::write(&p, &data).unwrap();
        let mut r = d.files.put_file(p.to_str().unwrap()).unwrap();
        r.device = d.id();
        d.node().offer(&r).unwrap();
        (r, data)
    };
    let (ra, da) = attach(&a.d, "a.bin", (2 << 20) + 17);
    let (rb, db) = attach(&b.d, "b.bin", 4096);
    eventually("the offers replicate", || converged(&[&a.d, &b.d])).await;
    let (fa, fb) = tokio::join!(b.d.node().fetch_file(&ra), a.d.node().fetch_file(&rb));
    assert_eq!(fa.expect("b fetches a's file"), a.id());
    assert_eq!(fb.expect("a fetches b's file"), b.id());
    let read = |d: &Dev, sha: &str| {
        let mut v = Vec::new();
        std::io::Read::read_to_end(&mut d.files.open_file(sha).unwrap(), &mut v).unwrap();
        v
    };
    assert!(read(&b.d, &ra.sha256) == da, "b's copy differs");
    assert!(read(&a.d, &rb.sha256) == db, "a's copy differs");
}
