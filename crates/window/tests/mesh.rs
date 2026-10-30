//! Settings → Devices on the running mesh (Go devices_service.go over
//! `core.Mesh()`): two and three Cores on scratch data dirs with fixture
//! identities (file keystore), paired through the window's own calls over
//! iroh on 127.0.0.1, with LAN lookups that unicast to each other on
//! 127.0.0.1. The home server binds 127.0.0.1 with no router and a fake
//! awake hold. No public relay, STUN, DNS or port mapping.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use blob::FileRef;
use irori_core::{Core, Options};
use irori_net::admit::ChainAdmit;
use irori_net::endpoint::IpMode;
use irori_net::pairnet::PairLan;
use irori_net::transport::{IrohNet, NetConfig};
use irori_window::devices::DeviceView;
use irori_window::seams::FetchError;
use irori_window::{Config, Window};

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
            start.elapsed() < Duration::from_secs(60),
            "timed out waiting for: {what}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// An awake hold that holds nothing.
struct NoSleepHold;
impl sys::awake::Holder for NoSleepHold {
    fn acquire(&self) -> Result<Box<dyn sys::awake::Hold>, sys::awake::Error> {
        Ok(Box::new(Released))
    }
}
struct Released;
impl sys::awake::Hold for Released {
    fn release(&self) {}
}

/// One device: its Core, its window and its scratch dirs.
struct Dev {
    w: Arc<Window>,
    _dirs: Vec<tempfile::TempDir>,
}

impl Dev {
    fn core(&self) -> &Arc<Core> {
        &self.w.core
    }

    fn id(&self) -> String {
        self.core().device_id().to_string()
    }

    /// This window's row for `id`.
    fn row(&self, id: &str) -> Option<DeviceView> {
        self.w.devices().into_iter().find(|d| d.device_id == id)
    }

    fn state_of(&self, id: &str) -> String {
        self.row(id).map(|d| d.state).unwrap_or_default()
    }
}

impl Drop for Dev {
    fn drop(&mut self) {
        let _ = self.w.core.close();
    }
}

/// A device whose mesh and pairing run on loopback iroh and a LAN lookup
/// on `lan` that sends to `to`; `home` lets it run a home server on
/// 127.0.0.1.
fn dev(lan: SocketAddr, to: Vec<SocketAddr>, home: bool) -> Dev {
    let (d, c, dl) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    std::fs::create_dir_all(c.path().join("sessions")).unwrap();
    let mut o = Options {
        data_dir: d.path().to_path_buf(),
        claude_dir: c.path().to_path_buf(),
        version: "0.6.0-test".into(),
        headless: true,
        keystore: Some(Box::new(irori_identity::keystore::FileKeystore)),
        no_home: !home,
        ..Options::default()
    };
    if home {
        o.home = Some(Arc::new(|hc: &mut home::Config| {
            hc.bind_ip = Some(IpAddr::V4(Ipv4Addr::LOCALHOST));
            hc.lan_addrs = Some(Arc::new(|| vec![Ipv4Addr::LOCALHOST]));
            hc.v6_addrs = Some(Arc::new(Vec::new));
            hc.discover = Some(Arc::new(|| Box::pin(async { Ok(vec![]) })));
            hc.on_public_network = Some(Arc::new(|_| false));
            hc.awake = Some(Arc::new(NoSleepHold));
            hc.every = Duration::from_millis(100);
        }));
    }
    o.pair_net = Some(Arc::new(move |ident, chain, guests| {
        let key = iroh::SecretKey::from_bytes(&ident.expose_device_seed());
        let pl = PairLan::new(guests.clone());
        let mut lc =
            irori_lan::Config::new(key.public(), irori_net::book::lan_rooms(ident.clone()));
        lc.bind = Some(lan);
        let to = to.clone();
        lc.targets = Some(Arc::new(move || to.clone()));
        lc.every = Duration::from_millis(200);
        pl.wire(&mut lc);
        let l = irori_lan::LanLookup::start(lc);
        pl.attach(l.clone());
        let admit = Arc::new(ChainAdmit {
            chain: chain.clone(),
            self_id: ident.device_id(),
        });
        let mut nc = NetConfig::new(ident.device_id(), key, admit);
        nc.ip = IpMode::Loopback;
        nc.lan = Some(l);
        nc.guests = guests.clone();
        nc.pair_lan = pl;
        IrohNet::new(nc)
    }));
    let core = Arc::new(Core::start(o).unwrap());
    let downloads = dl.path().to_path_buf();
    let w = Arc::new(Window::new(
        core.clone(),
        Window::default_seams(&core),
        Config {
            claude_dir: c.path().to_path_buf(),
            downloads: Box::new(move || Ok(downloads.clone())),
        },
    ));
    Dev {
        w,
        _dirs: vec![d, c, dl],
    }
}

/// `joiner` joins `host`'s person through the Devices calls.
fn pair(host: &Dev, joiner: &Dev) {
    let (hp, jp) = (&host.w.seams.pairing, &joiner.w.seams.pairing);
    let st = hp.add(irori_identity::ROLE_DESKTOP).unwrap();
    jp.join(&st.code, "").unwrap();
    wait("both reach the words", || {
        hp.status().state == irori_pair::STATE_CONFIRM
            && jp.status().state == irori_pair::STATE_WORDS
    });
    hp.confirm_words(&jp.status().words.join(" ")).unwrap();
    jp.confirm_key(&st.fingerprint).unwrap();
    wait("both paired", || {
        hp.status().state == irori_pair::STATE_DONE && jp.status().state == irori_pair::STATE_DONE
    });
}

/// `n` LAN sockets, each device sending to all the others.
fn lans(n: usize) -> Vec<(SocketAddr, Vec<SocketAddr>)> {
    let all: Vec<SocketAddr> = (0..n).map(|_| free_udp()).collect();
    all.iter()
        .map(|a| (*a, all.iter().filter(|b| *b != a).copied().collect()))
        .collect()
}

fn file_ref(d: &Dev, body: &[u8]) -> FileRef {
    let src = d._dirs[0].path().join("attach.txt");
    std::fs::write(&src, body).unwrap();
    let mut r = d.core().files().put_file(&src.to_string_lossy()).unwrap();
    r.device = d.id();
    r.name = "attach.txt".into();
    r
}

fn holders_reach(d: &Dev, sha: &str, who: &str) -> bool {
    d.core()
        .mesh()
        .holders(sha)
        .is_ok_and(|h| h.iter().any(|x| x == who))
}

#[test]
fn two_devices_live_state_files_ignored_rows_and_the_home_server() {
    let l = lans(2);
    let a = dev(l[0].0, l[0].1.clone(), true);
    let b = dev(l[1].0, l[1].1.clone(), false);
    let (ai, bi) = (a.id(), b.id());

    // Before pairing: one device, the mesh off, no other server.
    let rows = a.w.devices();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].this && rows[0].state == "this" && rows[0].device_id == ai);
    let hs = a.w.home_server();
    assert!(hs.can && !hs.elsewhere && hs.remote.is_empty());
    assert!(!b.w.home_server().can, "b runs no home server");

    pair(&a, &b);

    // Live state: each sees the other connected, this device first, with
    // the cert's label and role (Go Devices via Mesh().Devices).
    wait("a sees b connected", || a.state_of(&bi) == "connected");
    wait("b sees a connected", || b.state_of(&ai) == "connected");
    let rows = a.w.devices();
    assert_eq!(rows[0].device_id, ai);
    assert!(rows[0].this && rows[0].state == "this");
    let rb = &rows[1];
    let cert = a.core().chain().cert(&bi).unwrap();
    assert_eq!(
        (rb.name.as_str(), rb.role.as_str()),
        (cert.label.as_str(), "desktop")
    );
    assert!(!rb.this && rb.warning.is_empty() && !rb.last_seen.is_zero());
    assert_eq!(a.core().net_status().state, mesh::STATE_ON);
    // Both LAN lookups have somewhere to send: no LAN line.
    assert_eq!(a.w.home_server().lan, "");

    // A file held on b only reaches a through the mesh (Go FetchFile).
    let r = file_ref(&b, b"held on b");
    b.w.seams.mesh.offer(&r).unwrap();
    wait("b's offer reaches a", || holders_reach(&a, &r.sha256, &bi));
    assert!(!a.core().files().has(&r.sha256));
    a.w.seams.mesh.fetch_file(&r).unwrap();
    assert!(a.core().files().has(&r.sha256));

    // Rows the folds ignore (written by the wrong device): b claims an
    // agent of a's. a's row for b says so and offers Remove.
    b.core()
        .store()
        .append(
            "irori",
            store::kinds::AGENT_SEEN,
            &serde_json::json!({"key": format!("{ai}/not-b"), "agent": "claude"}),
        )
        .unwrap();
    wait("a counts b's ignored row", || {
        a.row(&bi).is_some_and(|d| {
            d.warning
                == "1 of its rows were ignored: written by the wrong device. Remove it if you don't know why."
                && d.action == "remove"
        })
    });

    // The home server on a: b sees it elsewhere, uses its relay, and it is
    // up (Go OtherServer, RemoteServers).
    let v = a.w.set_home_server(true, false, "").unwrap();
    assert!(v.on);
    wait("a's server runs", || a.w.home_server().running);
    let hs = a.w.home_server();
    assert_eq!(hs.reach, "home");
    assert!(!hs.elsewhere, "a's own server isn't elsewhere");
    wait("b sees a's server", || b.w.home_server().elsewhere);
    wait("b's endpoint is up on a's relay", || {
        b.core().hubs().iter().any(|h| h.id == ai && h.up)
    });
    assert!(b.core().remote_servers().iter().all(|s| !s.unreachable));
    assert_eq!(b.w.home_server().remote, "");

    // a goes away: b's row for a stops being connected, a's server is down
    // (the line waits out the grace), and a file only a holds is offline,
    // named by its device.
    let ra = file_ref(&a, b"held on a");
    let a_label = b.core().chain().cert(&ai).unwrap().label;
    drop(a);
    wait("b sees a gone", || b.state_of(&ai) != "connected");
    assert_eq!(b.state_of(&ai), "recent");
    wait("a's server is down for b", || {
        b.core().hubs().iter().any(|h| h.id == ai && !h.up)
    });
    match b.w.seams.mesh.fetch_file(&ra) {
        Err(FetchError::Offline(on)) => assert_eq!(on, a_label),
        other => panic!("fetch from a gone device: {other:?}"),
    }
    // Attached here, held nowhere: no device to name.
    let mut lost = ra.clone();
    lost.device = bi.clone();
    match b.w.seams.mesh.fetch_file(&lost) {
        Err(FetchError::Offline(on)) => assert_eq!(on, "no device that is connected"),
        other => panic!("fetch of a lost file: {other:?}"),
    }
}

#[test]
fn three_devices_each_see_the_other_two() {
    let l = lans(3);
    let a = dev(l[0].0, l[0].1.clone(), false);
    let b = dev(l[1].0, l[1].1.clone(), false);
    let c = dev(l[2].0, l[2].1.clone(), false);
    pair(&a, &b);
    pair(&a, &c);
    let devs = [&a, &b, &c];
    let ids: Vec<String> = devs.iter().map(|d| d.id()).collect();
    for (i, d) in devs.iter().enumerate() {
        for (j, id) in ids.iter().enumerate() {
            if i != j {
                wait(&format!("device {i} sees device {j} connected"), || {
                    d.state_of(id) == "connected"
                });
            }
        }
        let rows = d.w.devices();
        assert_eq!(rows.len(), 3, "device {i}: {rows:?}");
        assert_eq!(rows[0].device_id, ids[i]);
        assert!(rows.iter().all(|r| r.warning.is_empty()), "{rows:?}");
    }

    // b and c met through the mesh only: a file b holds reaches c.
    let r = file_ref(&b, b"from b to c");
    b.w.seams.mesh.offer(&r).unwrap();
    wait("b's offer reaches c", || {
        holders_reach(&c, &r.sha256, &ids[1])
    });
    c.w.seams.mesh.fetch_file(&r).unwrap();
    assert!(c.core().files().has(&r.sha256));

    // c goes away: a and b both stop showing it connected; b still sees a.
    drop(c);
    for d in [&a, &b] {
        wait("c gone", || d.state_of(&ids[2]) != "connected");
    }
    assert_eq!(b.state_of(&ids[0]), "connected");
}
