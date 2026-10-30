//! Go pair_cmd_test.go. The flow tests run two Cores on fresh scratch data
//! dirs with fixture identities (file keystore), pairing over iroh on
//! 127.0.0.1 with LAN lookups that unicast to each other on 127.0.0.1; no
//! home server, nothing leaves the machine.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use irori_net::admit::ChainAdmit;
use irori_net::endpoint::IpMode;
use irori_net::pairnet::PairLan;
use irori_net::transport::{IrohNet, NetConfig};
use irori_pair::{MODE_ADD, STATE_CANCELED, STATE_CONFIRM, STATE_FAILED};

use super::*;

/// Go `syncBuf`: join_loop's output while the test reads it.
#[derive(Clone, Default)]
struct SyncBuf(Arc<Mutex<Vec<u8>>>);

impl SyncBuf {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl Write for SyncBuf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(45),
            "timed out waiting for: {what}"
        );
        std::thread::sleep(Duration::from_millis(15));
    }
}

fn free_udp() -> SocketAddr {
    std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// One test device: scratch data and Claude dirs, a loopback LAN address,
/// and the peer's.
struct Dev {
    data: tempfile::TempDir,
    claude: tempfile::TempDir,
    lan: SocketAddr,
    to: SocketAddr,
}

fn pair_devs() -> (Dev, Dev) {
    let (la, lb) = (free_udp(), free_udp());
    let d = |lan, to| Dev {
        data: tempfile::tempdir().unwrap(),
        claude: tempfile::tempdir().unwrap(),
        lan,
        to,
    };
    (d(la, lb), d(lb, la))
}

/// Go `pairOpts`: a headless Core with a fixture identity whose pairing
/// runs on loopback iroh and a loopback LAN, and no home server.
fn pair_opts(rt: &tokio::runtime::Runtime, d: &Dev) -> Options {
    let (lan, to) = (d.lan, d.to);
    Options {
        data_dir: d.data.path().to_path_buf(),
        claude_dir: d.claude.path().to_path_buf(),
        version: "0.4.2-test".into(),
        headless: true,
        keystore: Some(Box::new(irori_identity::keystore::FileKeystore)),
        runtime: Some(rt.handle().clone()),
        no_home: true,
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

/// Go `startCore`: a running Irori (Core, API, lockfile).
fn start_core(rt: &tokio::runtime::Runtime, d: &Dev) -> Hosted {
    host::start_with(rt, pair_opts(rt, d)).unwrap_or_else(|e| panic!("start: {e}"))
}

/// Go `joinFlow`: runs `irori pair join`'s loop on the joining device with
/// the lines piped in (no terminal), while the test plays the person at the
/// adding device: it types the words the joining side printed.
fn join_flow(
    side: Side,
    joiner: impl Fn() -> Status,
    pc: &Core,
    lines: Sender<String>,
    rx: Receiver<String>,
) -> (String, Result<(), String>) {
    let add = pc
        .pairing_runtime()
        .block_on(pc.pairing().add(irori_identity::ROLE_DESKTOP))
        .unwrap();
    lines.send("0000 0000 0000 0000".into()).unwrap(); // a wrong key first
    lines.send(add.fingerprint.clone()).unwrap();
    let out = SyncBuf::default();
    let (done_tx, done) = channel();
    {
        let (code, mut out) = (add.code.clone(), out.clone());
        std::thread::spawn(move || {
            let stop = CancellationToken::new();
            let r = join_loop(
                &side,
                &code,
                "",
                &rx,
                &mut out,
                Duration::from_millis(20),
                &stop,
            );
            let _ = done_tx.send(r);
            drop(lines);
        });
    }
    wait_until("the words are printed", || {
        out.text().contains("Type these words")
    });
    let words = joiner().words.join(" ");
    assert!(
        out.text().contains(&words),
        "the words aren't printed: {:?}",
        out.text()
    );
    wait_until("the pc asks for them", || {
        pc.pairing().status().state == STATE_CONFIRM
    });
    pc.pairing_runtime()
        .block_on(pc.pairing().confirm(&words))
        .unwrap();
    let r = done
        .recv_timeout(Duration::from_secs(45))
        .expect("the join never finished");
    (out.text(), r)
}

/// While Irori runs here, `irori pair join` drives that Core through the
/// API instead of failing with "already running".
#[test]
fn pair_join_drives_the_running_core() {
    let rt = rt();
    let (dp, dm) = pair_devs();
    let pc = start_core(&rt, &dp);
    let mac = start_core(&rt, &dm);
    let (side, own) = open_join(&rt, dm.data.path(), pair_opts(&rt, &dm)).unwrap();
    assert!(own.is_none());
    assert!(
        matches!(side, Side::Api(_)),
        "side is not the running Core's API"
    );
    let (tx, rx) = channel();
    let m = mac.core.clone();
    let (out, r) = join_flow(side, move || m.pairing().status(), &pc.core, tx, rx);
    r.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(
        out.contains("That isn't the key. Try once more") && out.contains("Paired."),
        "output:\n{out}"
    );
    assert_eq!(
        mac.core.person().id,
        pc.core.person().id,
        "the mac didn't take the pc's person"
    );
    mac.close().unwrap();
    pc.close().unwrap();
}

/// With no Irori running, it starts its own Core, as before.
#[test]
fn pair_join_starts_its_own_core() {
    let rt = rt();
    let (dp, dm) = pair_devs();
    let pc = start_core(&rt, &dp);
    let (side, own) = open_join(&rt, dm.data.path(), pair_opts(&rt, &dm)).unwrap();
    let Side::Local(LocalJoin(core)) = &side else {
        panic!("side is not a Core of its own");
    };
    let core = core.clone();
    let own = own.expect("no Core of its own to close");
    let (tx, rx) = channel();
    let c = core.clone();
    let (out, r) = join_flow(side, move || c.pairing().status(), &pc.core, tx, rx);
    r.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(out.contains("Paired."), "output:\n{out}");
    assert_eq!(core.person().id, pc.core.person().id);
    drop(core);
    own.close().unwrap();
    pc.close().unwrap();
}

/// Stdin ending before a key cancels the running Core's join.
#[test]
fn pair_join_cancels_when_stdin_ends() {
    let rt = rt();
    let (dp, dm) = pair_devs();
    let pc = start_core(&rt, &dp);
    let mac = start_core(&rt, &dm);
    let (side, _) = open_join(&rt, dm.data.path(), pair_opts(&rt, &dm)).unwrap();
    let add = pc
        .core
        .pairing_runtime()
        .block_on(pc.core.pairing().add(irori_identity::ROLE_DESKTOP))
        .unwrap();
    let (tx, rx) = channel::<String>();
    drop(tx);
    let err = join_loop(
        &side,
        &add.code,
        "",
        &rx,
        &mut SyncBuf::default(),
        Duration::from_millis(20),
        &CancellationToken::new(),
    )
    .unwrap_err();
    assert_eq!(err, "no key typed");
    assert_eq!(mac.core.pairing().status().state, STATE_CANCELED);
    mac.close().unwrap();
    pc.close().unwrap();
}

/// The prompts take the code and the server address from piped lines.
#[test]
fn ask_join_reads_piped_lines() {
    let (tx, rx) = channel();
    tx.send("abcd-efgh-jkmn-pqrs".to_string()).unwrap();
    tx.send(" 203.0.113.7:24817 ".to_string()).unwrap();
    let mut out = SyncBuf::default();
    let (code, server) = ask_join(&rx, &mut out).unwrap();
    assert_eq!(
        (code.as_str(), server.as_str()),
        ("abcd-efgh-jkmn-pqrs", "203.0.113.7:24817")
    );
    assert_eq!(
        out.text(),
        "Pairing code (from Settings → Devices → Pair a device on the other device): Home server address, only when joining from outside your home (Enter to skip): "
    );
    let (tx, rx) = channel();
    tx.send("abcd".to_string()).unwrap();
    tx.send("no port".to_string()).unwrap();
    assert_eq!(
        ask_join(&rx, &mut SyncBuf::default()).unwrap_err(),
        "the home server address is a host and port, like 203.0.113.7:24817"
    );
    let (tx, rx) = channel::<String>();
    drop(tx);
    assert_eq!(
        ask_join(&rx, &mut SyncBuf::default()).unwrap_err(),
        "no code typed"
    );
    // At home: Enter skips the address, and so does stdin ending there.
    let (tx, rx) = channel();
    tx.send("abcd".to_string()).unwrap();
    drop(tx);
    assert_eq!(
        ask_join(&rx, &mut SyncBuf::default()).unwrap(),
        ("abcd".to_string(), String::new())
    );
}

/// Go `fakeJoin`: a side that plays back statuses, for the ways out of
/// join_loop that a real pairing can't stage on cue.
struct FakeJoin {
    first: Status,
    /// Status returns these in turn, then the last.
    sts: Mutex<Vec<Status>>,
    key_err: Option<KeyError>,
    cancels: Mutex<usize>,
}

impl FakeJoin {
    fn new(first: Status, sts: Vec<Status>, key_err: Option<KeyError>) -> FakeJoin {
        FakeJoin {
            first,
            sts: Mutex::new(sts),
            key_err,
            cancels: Mutex::new(0),
        }
    }
    fn cancels(&self) -> usize {
        *self.cancels.lock().unwrap()
    }
}

impl JoinSide for FakeJoin {
    fn join(&self, _: &str, _: &str) -> Result<Status, String> {
        Ok(self.first.clone())
    }
    fn status(&self) -> Result<Status, String> {
        let mut s = self.sts.lock().unwrap();
        let st = s[0].clone();
        if s.len() > 1 {
            s.remove(0);
        }
        Ok(st)
    }
    fn key(&self, _: &str) -> Result<(), KeyError> {
        match &self.key_err {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }
    fn cancel(&self) {
        *self.cancels.lock().unwrap() += 1;
    }
}

fn exp(secs: u64) -> Option<gocompat::time::GoTime> {
    Some(gocompat::time::GoTime::from_system(
        std::time::SystemTime::now() + Duration::from_secs(secs),
    ))
}

fn st(mode: &str, state: &str, expires: Option<gocompat::time::GoTime>) -> Status {
    Status {
        mode: mode.into(),
        state: state.into(),
        expires,
        ..Status::default()
    }
}

fn run_fake(f: &FakeJoin, lines: &[&str], out: &mut SyncBuf) -> Result<(), String> {
    let (tx, rx) = channel();
    for l in lines {
        tx.send(l.to_string()).unwrap();
    }
    // Go passes a nil channel where no key is typed: keep it open.
    let r = join_loop(
        f,
        "code",
        "",
        &rx,
        out,
        Duration::from_millis(1),
        &CancellationToken::new(),
    );
    drop(tx);
    r
}

/// Any way out of an open join but "Paired." closes it, so the running
/// Core isn't left holding it; a closed one isn't canceled again.
#[test]
fn pair_join_cancels_on_every_way_out() {
    let e = exp(60);
    let mut words = st(MODE_JOIN, STATE_WORDS, e);
    words.words = vec!["a".into(), "b".into()];
    // A key the API refused for another reason than a wrong key.
    let f = FakeJoin::new(
        st(MODE_JOIN, STATE_WAITING, e),
        vec![words.clone()],
        Some(KeyError::Other("boom".into())),
    );
    let r = run_fake(&f, &["1111 2222 3333 4444"], &mut SyncBuf::default());
    assert!(
        r.is_err() && f.cancels() == 1,
        "a refused key: {r:?}, cancels {}",
        f.cancels()
    );
    // The pairing failed on its own: nothing left to cancel.
    let mut failed = st(MODE_JOIN, STATE_FAILED, e);
    failed.error = "nope".into();
    let f = FakeJoin::new(words.clone(), vec![failed], None);
    let r = run_fake(&f, &[], &mut SyncBuf::default());
    assert_eq!(r, Err("nope".to_string()));
    assert_eq!(f.cancels(), 0);
    // Paired: nothing to cancel.
    let f = FakeJoin::new(words.clone(), vec![st(MODE_JOIN, STATE_DONE, e)], None);
    assert_eq!(run_fake(&f, &[], &mut SyncBuf::default()), Ok(()));
    assert_eq!(f.cancels(), 0);
    // Interrupted (Go's ctx canceled): the join closes.
    let f = FakeJoin::new(words.clone(), vec![st(MODE_JOIN, STATE_WAITING, e)], None);
    let stop = CancellationToken::new();
    stop.cancel();
    let (_tx, rx) = channel();
    let r = join_loop(
        &f,
        "code",
        "",
        &rx,
        &mut SyncBuf::default(),
        Duration::from_millis(1),
        &stop,
    );
    assert_eq!(r, Err("context canceled".to_string()));
    assert_eq!(f.cancels(), 1);
}

/// The running Core's status is shared with the app: once this join ended,
/// an Add or a newer Join there is not this command's pairing, so it
/// neither follows it to "Paired." nor cancels it.
#[test]
fn pair_join_follows_only_its_own_join() {
    let e = exp(60);
    let mine = st(MODE_JOIN, STATE_WAITING, e);
    let newer = exp(61);
    for (name, other) in [
        ("an add", st(MODE_ADD, STATE_DONE, None)),
        ("a newer join", st(MODE_JOIN, STATE_DONE, newer)),
    ] {
        let mut out = SyncBuf::default();
        let f = FakeJoin::new(mine.clone(), vec![mine.clone(), other], None);
        let r = run_fake(&f, &[], &mut out);
        assert_eq!(r, Err("the pairing closed".to_string()), "{name}");
        assert!(!out.text().contains("Paired."), "{name}: {}", out.text());
        assert_eq!(f.cancels(), 0, "{name}");
    }
}

/// The words, the key prompt and the waiting line, as Go prints them.
#[test]
fn pair_join_texts() {
    let e = exp(60);
    let mut words = st(MODE_JOIN, STATE_WORDS, e);
    words.words = vec!["apple".into(), "brick".into()];
    words.peer = "PC".into();
    let mut ok = words.clone();
    ok.key_ok = true;
    let f = FakeJoin::new(
        st(MODE_JOIN, STATE_WAITING, e),
        vec![words, ok, st(MODE_JOIN, STATE_DONE, e)],
        None,
    );
    let mut out = SyncBuf::default();
    run_fake(&f, &["1111 2222 3333 4444"], &mut out).unwrap();
    assert_eq!(
        out.text(),
        "Looking for the other device...\nType these words on PC:\n\n    apple brick\n\nThen type the key PC shows beside its code: Waiting for PC (Ctrl+C cancels).\nPaired. Keep Irori running here (or `irori serve --headless`) to keep the devices in sync.\n"
    );
}

/// An unreadable irori.json doesn't stop `irori pair join`: with no Irori
/// answering it starts its own Core, as before the API path.
#[test]
fn pair_join_past_an_unreadable_lockfile() {
    let rt = rt();
    let (dm, _) = pair_devs();
    std::fs::write(dm.data.path().join("irori.json"), b"{not json").unwrap();
    let (side, own) = open_join(&rt, dm.data.path(), pair_opts(&rt, &dm)).unwrap();
    assert!(
        matches!(side, Side::Local(_)),
        "side is not a Core of its own"
    );
    drop(side);
    own.unwrap().close().unwrap();
}
