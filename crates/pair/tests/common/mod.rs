//! The flow-test harness (I:internal/pair/pair_test.go `dev`): each device
//! has a real store, keys (file keystore) and chain, a mesh Node and a
//! Pairer, all on one `mesh::FakeNet`. Nothing opens a socket.
//!
//! The Pairer and the Node share the device's FakeNet handle (keyed by its
//! device key, as an `IrohNet` is), so both rooms are on one network, as in
//! Go.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use irori_identity::{Chain, DeviceJson, FileKeystore, Identity};
use irori_pair::{Options, Pairer};
use mesh::{FakeNet, Intervals, Node};
use store::{Event, Filter, Store};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub fn fast() -> Intervals {
    Intervals {
        presence: Duration::from_millis(40),
        ping: Duration::from_millis(60),
        vector: Duration::from_millis(80),
        backoff_min: Duration::from_millis(20),
        backoff_max: Duration::from_millis(200),
    }
}

pub const HELLO: Duration = Duration::from_millis(30);

pub struct Dev {
    pub name: String,
    _tmp: tempfile::TempDir,
    pub dir: PathBuf,
    pub net: FakeNet,
    pub ident: Arc<Identity>,
    pub chain: Arc<Chain>,
    pub st: Arc<Store>,
    pub node: Arc<Node>,
    pub pair: Pairer,
    /// The node is built but not run: no sync.
    pub no_net: bool,
    window: Duration,
    stop: Option<CancellationToken>,
    task: Option<JoinHandle<()>>,
}

impl Dev {
    pub fn new(net: &FakeNet, name: &str, window: Duration) -> Dev {
        Dev::with(net, name, window, false)
    }

    pub fn with(net: &FakeNet, name: &str, window: Duration, no_net: bool) -> Dev {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let (ident, chain, st, node, pair) = open(&dir, name, net, window);
        let mut d = Dev {
            name: name.into(),
            _tmp: tmp,
            dir,
            net: net.clone(),
            ident,
            chain,
            st,
            node,
            pair,
            no_net,
            window,
            stop: None,
            task: None,
        };
        d.run();
        d
    }

    fn run(&mut self) {
        if self.no_net {
            return;
        }
        let c = CancellationToken::new();
        let (n, c2) = (self.node.clone(), c.clone());
        self.task = Some(tokio::spawn(async move { n.run(c2).await }));
        self.stop = Some(c);
    }

    pub async fn close(&mut self) {
        self.pair.cancel();
        if let Some(c) = self.stop.take() {
            c.cancel();
        }
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
        let _ = self.st.close();
    }

    pub fn open(&mut self, window: Duration) {
        let (ident, chain, st, node, pair) = open(&self.dir, &self.name, &self.net, window);
        self.ident = ident;
        self.chain = chain;
        self.st = st;
        self.node = node;
        self.pair = pair;
        self.window = window;
        self.run();
    }

    pub fn id(&self) -> String {
        self.ident.device_id()
    }

    pub fn state(&self) -> String {
        self.pair.status().state
    }

    pub fn held(&self, ev: &Event) -> bool {
        match self.st.list(&Filter {
            device_id: ev.device_id.clone(),
            after_seq: ev.seq - 1,
            limit: 1,
            ..Filter::default()
        }) {
            Ok(evs) => {
                evs.len() == 1
                    && evs[0].seq == ev.seq
                    && store::row_hash(&evs[0]) == store::row_hash(ev)
            }
            Err(_) => false,
        }
    }
}

impl Drop for Dev {
    fn drop(&mut self) {
        self.pair.cancel();
        if let Some(c) = self.stop.take() {
            c.cancel();
        }
    }
}

fn open(
    dir: &std::path::Path,
    name: &str,
    net: &FakeNet,
    window: Duration,
) -> (Arc<Identity>, Arc<Chain>, Arc<Store>, Arc<Node>, Pairer) {
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
    irori_pair::learn_saved(dir, &st, &chain).unwrap();
    let n = name.to_string();
    let node = Node::new(mesh::Options {
        store: st.clone(),
        ident: ident.clone(),
        chain: chain.clone(),
        net: Arc::new(net.keyed(ident.device_pub())),
        version: "0.4.0-test".into(),
        files: None,
        data_dir: Some(dir.to_path_buf()),
        intervals: fast(),
        trust_any_peer: false,
        ignored: None,
        boards: None,
        logf: Some(Arc::new(move |m: &str| eprintln!("{n}: {m}"))),
        now: None,
    });
    let mut o = Options::new(
        Arc::new(net.keyed(ident.device_pub())),
        ident.clone(),
        chain.clone(),
        st.clone(),
    );
    o.data_dir = Some(dir.to_path_buf());
    let n2 = node.clone();
    o.kick = Some(Arc::new(move || n2.kick()));
    o.window = window;
    o.hello_every = HELLO;
    let n = name.to_string();
    o.logf = Some(Arc::new(move |m: &str| eprintln!("{n}: {m}")));
    (ident, chain, st, node, Pairer::new(o))
}

pub async fn eventually(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for: {what}");
}
