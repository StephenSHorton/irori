//! Go internal/core/api_pair_test.go: the joining side of pairing on the
//! running Core's API (§3, §6.8), so `irori pair join` works while the app
//! runs. These drive it through the API on the joining Core, with the
//! person's side on the adding Core. Two Cores on fresh scratch data dirs
//! with fixture identities (file keystore) pair over iroh on 127.0.0.1
//! with LAN lookups that unicast to each other on 127.0.0.1; no home
//! server. Each serves its API on 127.0.0.1:0 and writes its lockfile.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use irori_api::Server;
use irori_api::client::{Client, Error};
use irori_api::pair::is_wrong_key;
use irori_core::{Core, Options};
use irori_net::admit::ChainAdmit;
use irori_net::endpoint::IpMode;
use irori_net::pairnet::PairLan;
use irori_net::transport::{IrohNet, NetConfig};
use irori_pair::{
    ERR_TEXT_NOT_THAT_KEY, MAX_MISSES, MODE_ADD, MODE_JOIN, STATE_CANCELED, STATE_CONFIRM,
    STATE_DONE, STATE_EXPIRED, STATE_FAILED, STATE_WAITING, STATE_WORDS, new_code,
};
use tokio_util::sync::CancellationToken;

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
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// One Core and its API (Go `hcore`).
struct HCore {
    core: Arc<Core>,
    data: tempfile::TempDir,
    claude: tempfile::TempDir,
    lan: SocketAddr,
    to: SocketAddr,
    stop: CancellationToken,
    rt: Arc<tokio::runtime::Runtime>,
}

impl HCore {
    /// A headless Core with a fixture identity whose pairing runs on
    /// loopback iroh and a loopback LAN, and no home server.
    fn options(
        data: &Path,
        claude: &Path,
        lan: SocketAddr,
        to: SocketAddr,
        rt: &tokio::runtime::Runtime,
        window: Duration,
    ) -> Options {
        Options {
            data_dir: data.to_path_buf(),
            claude_dir: claude.to_path_buf(),
            version: "0.4.2-test".into(),
            headless: true,
            keystore: Some(Box::new(irori_identity::keystore::FileKeystore)),
            runtime: Some(rt.handle().clone()),
            no_home: true,
            pair_window: window,
            pair_net: Some(Arc::new(move |ident, chain, guests| {
                let key = iroh::SecretKey::from_bytes(&ident.expose_device_seed());
                let pl = PairLan::new(guests.clone());
                let mut lc =
                    irori_lan::Config::new(key.public(), irori_net::book::lan_rooms(ident.clone()));
                lc.bind = Some(lan);
                lc.targets = Some(Arc::new(move || vec![to]));
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
            })),
            ..Options::default()
        }
    }

    /// Starts the Core with `window` (zero: the full one), serves its API
    /// and writes the lockfile.
    fn start(
        rt: &Arc<tokio::runtime::Runtime>,
        lan: SocketAddr,
        to: SocketAddr,
        window: Duration,
    ) -> HCore {
        let (data, claude) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let opts = Self::options(data.path(), claude.path(), lan, to, rt, window);
        let (core, stop) = Self::serve(rt, opts);
        HCore {
            core,
            data,
            claude,
            lan,
            to,
            stop,
            rt: rt.clone(),
        }
    }

    fn restart(&mut self, window: Duration) {
        let opts = Self::options(
            self.data.path(),
            self.claude.path(),
            self.lan,
            self.to,
            &self.rt,
            window,
        );
        (self.core, self.stop) = Self::serve(&self.rt, opts);
    }

    /// Serves the Core's API on 127.0.0.1:0 and writes the lockfile.
    fn serve(rt: &tokio::runtime::Runtime, opts: Options) -> (Arc<Core>, CancellationToken) {
        let core = Arc::new(Core::start(opts).unwrap());
        let stop = CancellationToken::new();
        let (c, s) = (core.clone(), stop.clone());
        rt.block_on(async move {
            let server = Server::new(c.clone(), c.token());
            let ln = irori_api::listen().await.unwrap();
            let port = ln.local_addr().unwrap().port();
            tokio::spawn(async move { server.serve(ln, s).await });
            c.write_lockfile(port).unwrap();
        });
        (core, stop)
    }

    fn stop(&self) {
        self.stop.cancel();
        self.core.close().unwrap();
    }

    fn client(&self) -> Client {
        self.rt.block_on(Client::dial(self.data.path())).unwrap()
    }

    fn block<T>(&self, f: impl std::future::Future<Output = T>) -> T {
        self.rt.block_on(f)
    }
}

fn rt() -> Arc<tokio::runtime::Runtime> {
    Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap(),
    )
}

/// Two Cores that find each other on the loopback LAN.
fn two(rt: &Arc<tokio::runtime::Runtime>) -> (HCore, HCore) {
    let (la, lb) = (free_udp(), free_udp());
    (
        HCore::start(rt, la, lb, Duration::ZERO),
        HCore::start(rt, lb, la, Duration::ZERO),
    )
}

fn code_of(e: &Error) -> u16 {
    match e {
        Error::Status { code, .. } => *code,
        _ => 0,
    }
}

fn api_state(h: &HCore, c: &Client, want: &str) -> bool {
    h.block(c.pair_status()).unwrap().state == want
}

/// A join through the API pairs the running Core: the words come back in
/// the status, a wrong key counts a miss, the right one finishes it.
#[test]
fn join_through_the_api_while_the_core_runs() {
    let rt = rt();
    let (pc, mac) = two(&rt);
    let c = mac.client();
    let add = pc
        .block(pc.core.pairing().add(irori_identity::ROLE_DESKTOP))
        .unwrap();
    let st = mac
        .block(c.pair_join(&add.code.to_lowercase(), ""))
        .unwrap();
    assert!(
        st.mode == MODE_JOIN && st.state == STATE_WAITING,
        "join status = {st:?}"
    );
    wait("the mac shows the words", || {
        api_state(&mac, &c, STATE_WORDS)
    });
    let st = mac.block(c.pair_status()).unwrap();
    assert!(
        st.words.len() == 5 && st.peer == pc.core.device().name,
        "words status = {st:?}"
    );
    wait("the pc asks for them", || {
        pc.core.pairing().status().state == STATE_CONFIRM
    });
    pc.block(pc.core.pairing().confirm(&st.words.join(" ")))
        .unwrap();
    let e = mac.block(c.pair_key("0000 0000 0000 0000")).unwrap_err();
    assert!(is_wrong_key(&e), "a wrong key: {e}");
    let st = mac.block(c.pair_status()).unwrap();
    assert!(
        st.state == STATE_WORDS && st.misses == 1,
        "after one wrong key = {st:?}"
    );
    mac.block(c.pair_key(&add.fingerprint)).unwrap();
    wait("paired", || {
        pc.core.pairing().status().state == STATE_DONE
            && mac.core.pairing().status().state == STATE_DONE
    });
    assert_eq!(
        mac.core.person().id,
        pc.core.person().id,
        "the mac didn't take the pc's person"
    );
    // A paired device can't be pulled into another person over the API.
    let e = mac.block(c.pair_join(&new_code(), "")).unwrap_err();
    assert_eq!(code_of(&e), 409, "a paired device joined again: {e}");
    mac.stop();
    pc.stop();
}

/// The second wrong key closes the window on both sides.
#[test]
fn join_through_the_api_wrong_key_twice() {
    let rt = rt();
    let (pc, mac) = two(&rt);
    let c = mac.client();
    let add = pc
        .block(pc.core.pairing().add(irori_identity::ROLE_DESKTOP))
        .unwrap();
    mac.block(c.pair_join(&add.code, "")).unwrap();
    wait("the mac shows the words", || {
        api_state(&mac, &c, STATE_WORDS)
    });
    for _ in 0..MAX_MISSES {
        let e = mac.block(c.pair_key("1111 2222 3333 4444")).unwrap_err();
        assert!(is_wrong_key(&e), "a wrong key: {e}");
    }
    let st = mac.block(c.pair_status()).unwrap();
    assert!(
        st.state == STATE_FAILED && st.error == ERR_TEXT_NOT_THAT_KEY,
        "after two wrong keys = {st:?}"
    );
    wait("the pc's window closes", || {
        !pc.core.pairing().status().open()
    });
    let e = mac.block(c.pair_key(&add.fingerprint)).unwrap_err();
    match &e {
        Error::Status { code, message, .. } => {
            assert_eq!(*code, 409, "{e}");
            assert_eq!(message, "no pairing is waiting for a key");
        }
        _ => panic!("a key after the window closed: {e}"),
    }
    assert!(
        pc.core.pairing().status().state != STATE_DONE
            && mac.core.person().id != pc.core.person().id,
        "paired after two wrong keys"
    );
    mac.stop();
    pc.stop();
}

/// A code nobody answers expires; cancel closes an open join.
#[test]
fn join_through_the_api_expires_and_cancels() {
    let rt = rt();
    let (la, lb) = (free_udp(), free_udp());
    let mut mac = HCore::start(&rt, la, lb, Duration::from_millis(300));
    let c = mac.client();
    let e = mac.block(c.pair_join("not a code", "")).unwrap_err();
    assert_eq!(code_of(&e), 400, "a bad code: {e}");
    let e = mac
        .block(c.pair_join(&new_code(), "no port here"))
        .unwrap_err();
    assert_eq!(code_of(&e), 400, "a bad server address: {e}");
    assert!(
        e.to_string()
            .ends_with("the home server address is a host and port, like 203.0.113.7:24817"),
        "{e}"
    );
    mac.block(c.pair_join(&new_code(), "")).unwrap();
    wait("the code expires", || api_state(&mac, &c, STATE_EXPIRED));

    mac.stop();
    mac.restart(Duration::ZERO); // the full window: only cancel closes it now
    let c = mac.client();
    // A closed loopback port stands in for Go's 203.0.113.7: nothing here
    // dials off the machine.
    mac.block(c.pair_join(&new_code(), "127.0.0.1:9")).unwrap();
    let e = mac.block(c.pair_join(&new_code(), "")).unwrap_err();
    assert_eq!(code_of(&e), 409, "a second join while one is open: {e}");
    let st = mac.block(c.pair_status()).unwrap();
    assert!(
        st.via == "127.0.0.1:9" && st.open(),
        "join from away = {st:?}"
    );
    let st = mac.block(c.pair_cancel()).unwrap();
    assert_eq!(st.state, STATE_CANCELED, "after cancel = {st:?}");
    mac.stop();
}

fn call(port: u16, token: &str, method: &str, path: &str, body: &[u8]) -> u16 {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let c = Client::at(port, token);
    rt.block_on(c.raw(method, path, &[], Some(body.to_vec())))
        .unwrap()
        .status
}

/// Nothing on the API opens a pairing window, types the words or removes a
/// device; while the person's Add window is open the API shows neither its
/// code nor its key, and its cancel leaves the window open.
#[test]
fn the_pairing_api_has_no_add_side() {
    let rt = rt();
    let (pc, mac) = two(&rt);
    // Go pairCores: pair them first, through the Pairers.
    let add = pc
        .block(pc.core.pairing().add(irori_identity::ROLE_DESKTOP))
        .unwrap();
    mac.block(mac.core.pairing().join(&add.code)).unwrap();
    wait("both reach the words", || {
        pc.core.pairing().status().state == STATE_CONFIRM
            && mac.core.pairing().status().state == STATE_WORDS
    });
    let words = mac.core.pairing().status().words.join(" ");
    pc.block(pc.core.pairing().confirm(&words)).unwrap();
    mac.block(mac.core.pairing().confirm_key(&add.fingerprint))
        .unwrap();
    wait("paired", || {
        pc.core.pairing().status().state == STATE_DONE
            && mac.core.pairing().status().state == STATE_DONE
    });

    let lf = irori_core::lockfile::read(pc.data.path()).unwrap();
    let port = lf.port as u16;
    let mac_id = mac.core.device_id().to_string();
    let body = format!(r#"{{"role":"desktop","words":"a b c d e","deviceId":"{mac_id}"}}"#);
    let paths = [
        "/v1/pair/add".to_string(),
        "/v1/pair/open".into(),
        "/v1/pair/code".into(),
        "/v1/pair/words".into(),
        "/v1/pair/confirm".into(),
        "/v1/pair/revoke".into(),
        "/v1/pair/remove".into(),
        "/v1/pairing/add".into(),
        "/v1/devices/add".into(),
        format!("/v1/devices/{mac_id}"),
        "/v1/devices/revoke".into(),
    ];
    for p in &paths {
        for m in ["GET", "POST", "PUT", "DELETE"] {
            let code = call(port, &lf.token, m, p, body.as_bytes());
            assert!(code >= 400, "{m} {p} = {code}");
        }
    }
    assert_eq!(
        call(port, &lf.token, "GET", "/v1/pair/add", b""),
        404,
        "GET /v1/pair/add is not a 404"
    );
    assert_eq!(
        call(port, &lf.token, "GET", "/v1/pair/join", b""),
        405,
        "GET /v1/pair/join is allowed"
    );
    let st = pc.core.pairing().status();
    assert!(
        !(st.mode == MODE_ADD && st.open()),
        "the API opened a window: {st:?}"
    );
    assert!(
        pc.core.chain().cert(&mac_id).is_some() && pc.core.chain().cut(&mac_id).is_none(),
        "the API removed the mac"
    );

    let add = pc
        .block(pc.core.pairing().add(irori_identity::ROLE_NODE))
        .unwrap();
    let c = pc.client();
    let st = pc.block(c.pair_status()).unwrap();
    assert!(
        st.mode == MODE_ADD
            && st.state == STATE_WAITING
            && st.code.is_empty()
            && st.fingerprint.is_empty()
            && st.server_addr.is_empty()
            && st.role.is_empty(),
        "the API shows the Add window = {st:?}"
    );
    assert!(
        pc.block(c.pair_key(&add.fingerprint)).is_err(),
        "a key typed into an Add window"
    );
    pc.block(c.pair_cancel()).unwrap();
    let got = pc.core.pairing().status();
    assert!(
        got.state == STATE_WAITING && got.code == add.code,
        "the API's cancel closed the Add window: {}",
        got.state
    );
    let e = pc.block(c.pair_join(&new_code(), "")).unwrap_err();
    assert_eq!(code_of(&e), 409, "joined while an Add window is open: {e}");
    mac.stop();
    pc.stop();
}
