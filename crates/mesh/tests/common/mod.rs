//! The multi-Core harness (I:internal/mesh/harness_test.go): devices in one
//! process, each with a real store, real keys (the file keystore) and a
//! cert chain, joined by FakeNet. Nothing opens a socket.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use gocompat::time::GoTime;
use irori_identity::{Chain, DeviceJson, FileKeystore, Identity, PERSON_FILE};
use mesh::{FakeNet, Intervals, Node, Options, PeerState};
use store::{Event, Filter, Store, Vector};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub fn fast_intervals() -> Intervals {
    Intervals {
        presence: Duration::from_millis(40),
        ping: Duration::from_millis(60),
        vector: Duration::from_millis(80),
        backoff_min: Duration::from_millis(20),
        backoff_max: Duration::from_millis(200),
    }
}

/// A device's wall clock: real time plus an offset tests move.
#[derive(Default)]
pub struct Clock {
    off: Mutex<Duration>,
}

impl Clock {
    pub fn now(&self) -> SystemTime {
        SystemTime::now() + *self.off.lock().unwrap()
    }

    pub fn set(&self, off: Duration) {
        *self.off.lock().unwrap() = off;
    }
}

pub struct Dev {
    pub name: String,
    _tmp: tempfile::TempDir,
    pub dir: PathBuf,
    pub net: FakeNet,
    pub clk: Arc<Clock>,
    pub ident: Arc<Identity>,
    pub chain: Arc<Chain>,
    pub st: Arc<Store>,
    pub files: Arc<blob::Files>,
    pub node: Option<Arc<Node>>,
    pub trust: bool,
    cancel: Option<CancellationToken>,
    task: Option<JoinHandle<()>>,
}

fn open_parts(
    dir: &Path,
    name: &str,
    clk: &Arc<Clock>,
) -> (Arc<Identity>, Arc<Chain>, Arc<Store>, Arc<blob::Files>) {
    let mut dv = DeviceJson::load_or_create(dir).unwrap();
    dv.name = name.into();
    dv.save(dir).unwrap();
    let ident = Arc::new(Identity::load(dir, dv.clone(), Some(Box::new(FileKeystore))).unwrap());
    let p = ident.current();
    let chain = Arc::new(Chain::new(&p.id, &p.root_device_id, &dv.id));
    let c1 = clk.clone();
    let c2 = clk.clone();
    let clock = Arc::new(store::Clock::with_now(Box::new(move || c1.now())));
    let st = Arc::new(
        Store::open_with(
            dir.join("irori.db"),
            &dv.id,
            clock,
            store::OpenOptions {
                signer: Some(ident.clone()),
                verifier: Some(chain.clone()),
                now: Some(Arc::new(move || c2.now())),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    ident
        .ensure_self_cert(&*st, &chain, irori_identity::objects::ROLE_DESKTOP)
        .unwrap();
    let files = Arc::new(blob::Files::open(dir.join("files")).unwrap());
    (ident, chain, st, files)
}

impl Dev {
    /// A device with its own first-run person.
    pub fn new(net: &FakeNet, name: &str) -> Dev {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let clk = Arc::new(Clock::default());
        let (ident, chain, st, files) = open_parts(&dir, name, &clk);
        Dev {
            name: name.into(),
            _tmp: tmp,
            dir,
            net: net.clone(),
            clk,
            ident,
            chain,
            st,
            files,
            node: None,
            trust: false,
            cancel: None,
            task: None,
        }
    }

    pub fn id(&self) -> String {
        self.ident.device_id()
    }

    pub fn node(&self) -> &Arc<Node> {
        self.node.as_ref().expect("started")
    }

    pub fn options(&self, net: Arc<dyn mesh::Transport>) -> Options {
        let name = self.name.clone();
        Options {
            store: self.st.clone(),
            ident: self.ident.clone(),
            chain: self.chain.clone(),
            net,
            version: "0.4.0-test".into(),
            files: Some(self.files.clone()),
            data_dir: Some(self.dir.clone()),
            intervals: fast_intervals(),
            trust_any_peer: self.trust,
            ignored: None,
            logf: Some(Arc::new(move |m: &str| eprintln!("{name}: {m}"))),
            now: Some({
                let c = self.clk.clone();
                Arc::new(move || c.now())
            }),
        }
    }

    /// Runs the node, as the Core does: it waits for a second device.
    pub fn start(&mut self) {
        assert!(self.task.is_none(), "{} already started", self.name);
        let node = Node::new(self.options(Arc::new(self.net.keyed(self.ident.device_pub()))));
        self.start_node(node);
    }

    pub fn start_node(&mut self, node: Arc<Node>) {
        let cancel = CancellationToken::new();
        let (n, c) = (node.clone(), cancel.clone());
        self.task = Some(tokio::spawn(async move { n.run(c).await }));
        self.node = Some(node);
        self.cancel = Some(cancel);
    }

    /// Ends the node (the device goes offline); the store stays open.
    pub async fn stop(&mut self) {
        if let Some(c) = self.cancel.take() {
            c.cancel();
        }
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
    }

    pub async fn close(&mut self) {
        self.stop().await;
        let _ = self.st.close();
    }

    pub fn open(&mut self) {
        let (ident, chain, st, files) = open_parts(&self.dir, &self.name, &self.clk);
        self.ident = ident;
        self.chain = chain;
        self.st = st;
        self.files = files;
    }

    /// Closes and reopens the store and identity (a restart).
    pub async fn reopen(&mut self) {
        self.close().await;
        self.open();
    }

    /// Makes this device one of root's, as pairing does: it takes root's
    /// person.json, root writes its cert, and it learns root's person rows.
    pub async fn join(&mut self, root: &Dev) {
        self.close().await;
        std::fs::copy(root.dir.join(PERSON_FILE), self.dir.join(PERSON_FILE)).unwrap();
        self.open();
        let add = root
            .ident
            .cert_for(
                &self.id(),
                &hex::encode(self.ident.device_pub()),
                &self.name,
                irori_identity::objects::ROLE_DESKTOP,
                GoTime::from_system(SystemTime::now()),
            )
            .unwrap();
        root.st
            .append("irori", irori_identity::kinds::PERSON_DEVICE_ADDED, &add)
            .unwrap();
        self.learn(root);
    }

    /// Folds every person row `src` holds into this device's chain.
    pub fn learn(&self, src: &Dev) {
        let evs = src
            .st
            .list(&Filter {
                kinds: vec!["person.".into()],
                limit: 1000,
                ..Filter::default()
            })
            .unwrap();
        for _ in 0..evs.len() {
            for ev in &evs {
                let _ = self.chain.learn(ev.into());
            }
        }
        if let Some(n) = &self.node {
            n.kick();
        }
    }

    pub fn append(&self, kind: &str, payload: serde_json::Value) -> Event {
        self.st
            .append("irori", kind, &payload)
            .unwrap_or_else(|e| panic!("{} append {kind}: {e}", self.name))
    }

    pub fn vector(&self) -> Vector {
        self.st.vector().unwrap()
    }

    pub fn has(&self, ev: &Event) -> bool {
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

    pub fn peer(&self, id: &str) -> PeerState {
        self.node()
            .status()
            .peer_list()
            .iter()
            .find(|p| p.device_id == id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn seed(&self, epoch: i64) -> Vec<u8> {
        self.ident
            .person_seed(epoch)
            .unwrap_or_else(|| panic!("{} holds no epoch {epoch} seed", self.name))
            .to_vec()
    }

    /// The data-dir files only (the WAL included), as Go's copyDir does.
    pub fn copy_dir(src: &Path, dst: &Path) {
        for e in std::fs::read_dir(src).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                continue;
            }
            std::fs::copy(e.path(), dst.join(e.file_name())).unwrap();
        }
        for e in std::fs::read_dir(dst).unwrap() {
            let e = e.unwrap();
            if !e.file_type().unwrap().is_dir() && !src.join(e.file_name()).exists() {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

impl Drop for Dev {
    fn drop(&mut self) {
        if let Some(c) = self.cancel.take() {
            c.cancel();
        }
    }
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

/// Checks that `ok` stays false for a while.
pub async fn never(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_millis(400);
    while std::time::Instant::now() < deadline {
        if ok() {
            panic!("happened, and must not: {what}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub fn converged(devs: &[&Dev]) -> bool {
    let v = devs[0].vector();
    devs[1..].iter().all(|d| d.vector() == v)
}

/// PC (the root), Mac and node, paired and synced.
pub async fn trio(net: &FakeNet) -> (Dev, Dev, Dev) {
    let (pc, mut mac, mut nd) = (
        Dev::new(net, "pc"),
        Dev::new(net, "mac"),
        Dev::new(net, "node"),
    );
    mac.join(&pc).await;
    nd.join(&pc).await;
    mac.learn(&pc);
    let (mut pc, mut mac, mut nd) = (pc, mac, nd);
    pc.start();
    mac.start();
    nd.start();
    eventually("three devices converge", || converged(&[&pc, &mac, &nd])).await;
    (pc, mac, nd)
}
