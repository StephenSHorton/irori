//! Shared by the home tests: a fake router, a counting awake holder, and
//! a node (fixture identity, store and server on 127.0.0.1), as
//! I:internal/home/home_test.go builds them.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use home::nodeaddr::NodeAddr;
use home::portmap::{BoxFut, Gateway, MapError, Port};
use home::{Config, Server};
use irori_identity::{Chain, DeviceJson, FileKeystore, Identity};
use store::Store;
use sys::awake::{Error as AwakeError, Hold, Holder};
use tokio_util::sync::CancellationToken;

/// A fake gateway in process: it maps whatever it is asked for, at `ext`,
/// unless `fail` is set.
#[derive(Default)]
pub struct Router {
    pub st: Mutex<RouterState>,
}

#[derive(Default)]
pub struct RouterState {
    pub ext: Option<IpAddr>,
    pub fail: Option<String>,
    pub mapped: BTreeMap<&'static str, Port>,
}

impl Router {
    pub fn new(ext: &str) -> Arc<Router> {
        let r = Router::default();
        r.st.lock().unwrap().ext = Some(ext.parse().unwrap());
        Arc::new(r)
    }
    pub fn count(&self) -> usize {
        self.st.lock().unwrap().mapped.len()
    }
    pub fn set_fail(&self, e: &str) {
        self.st.lock().unwrap().fail = Some(e.into());
    }
}

impl Gateway for Router {
    fn method(&self) -> &str {
        "upnp"
    }
    fn external_ip(&self) -> BoxFut<'_, Result<IpAddr, String>> {
        Box::pin(async move {
            let s = self.st.lock().unwrap();
            match &s.fail {
                Some(e) => Err(e.clone()),
                None => Ok(s.ext.unwrap()),
            }
        })
    }
    fn map(&self, mut p: Port, life: Duration) -> BoxFut<'_, Result<(Port, Duration), MapError>> {
        Box::pin(async move {
            let mut s = self.st.lock().unwrap();
            if let Some(e) = &s.fail {
                return Err(MapError::Other(e.clone()));
            }
            if p.external == 0 {
                p.external = p.internal;
            }
            s.mapped.insert(p.proto, p);
            Ok((p, life))
        })
    }
    fn unmap(&self, p: Port) -> BoxFut<'_, Result<(), String>> {
        Box::pin(async move {
            self.st.lock().unwrap().mapped.remove(p.proto);
            Ok(())
        })
    }
}

/// Counts holds taken and released, and fails while `fail` is set.
#[derive(Default)]
pub struct FakeAwake {
    pub st: Mutex<(Option<String>, usize, usize)>,
}

pub struct FakeHold {
    f: Arc<FakeAwake>,
    done: AtomicBool,
}

pub struct Holds(pub Arc<FakeAwake>);

impl Holder for Holds {
    fn acquire(&self) -> Result<Box<dyn Hold>, AwakeError> {
        let mut s = self.0.st.lock().unwrap();
        if let Some(e) = &s.0 {
            return Err(AwakeError(e.clone()));
        }
        s.1 += 1;
        Ok(Box::new(FakeHold {
            f: self.0.clone(),
            done: AtomicBool::new(false),
        }))
    }
}

impl Hold for FakeHold {
    fn release(&self) {
        if !self.done.swap(true, Ordering::SeqCst) {
            self.f.st.lock().unwrap().2 += 1;
        }
    }
}

impl FakeAwake {
    pub fn counts(&self) -> (usize, usize) {
        let s = self.st.lock().unwrap();
        (s.1, s.2)
    }
    pub fn set_fail(&self, e: Option<&str>) {
        self.st.lock().unwrap().0 = e.map(String::from);
    }
}

pub struct Node {
    pub _tmp: tempfile::TempDir,
    pub dir: PathBuf,
    pub ident: Arc<Identity>,
    pub st: Arc<Store>,
    pub gw: Arc<Router>,
    pub awake: Arc<FakeAwake>,
    pub wanted: Arc<AtomicBool>,
    pub cfg: Config,
    pub srv: Server,
    pub stop: CancellationToken,
    pub run: Option<tokio::task::JoinHandle<()>>,
}

impl Node {
    pub fn new(edit: impl FnOnce(&mut Config)) -> Node {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let dev = DeviceJson::load_or_create(&dir).unwrap();
        let ident = Arc::new(Identity::load(&dir, dev, Some(Box::new(FileKeystore))).unwrap());
        let me = ident.device_id();
        let person = ident.current();
        let chain = Arc::new(Chain::new(&person.id, &person.root_device_id, &me));
        let st = Arc::new(
            Store::open_with(
                dir.join("irori.db"),
                &me,
                Arc::new(store::Clock::new()),
                store::OpenOptions {
                    signer: Some(ident.clone()),
                    verifier: Some(chain.clone()),
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        ident
            .ensure_self_cert(&*st, &chain, irori_identity::objects::ROLE_NODE)
            .unwrap();
        let gw = Router::new("203.0.113.7");
        let awake = Arc::new(FakeAwake::default());
        let wanted = Arc::new(AtomicBool::new(true));
        let mut cfg = Config::new(dir.clone(), ident.clone(), chain, st.clone());
        let w = wanted.clone();
        cfg.wanted = Some(Arc::new(move || w.load(Ordering::SeqCst)));
        cfg.bind_ip = Some(IpAddr::V4(Ipv4Addr::LOCALHOST));
        cfg.lan_addrs = Some(Arc::new(|| vec![Ipv4Addr::LOCALHOST]));
        cfg.v6_addrs = Some(Arc::new(Vec::new));
        let g = gw.clone();
        cfg.discover = Some(Arc::new(move || {
            let g = g.clone();
            Box::pin(async move { Ok(vec![g as Arc<dyn Gateway>]) })
        }));
        cfg.on_public_network = Some(Arc::new(|_| false));
        cfg.awake = Some(Arc::new(Holds(awake.clone())));
        cfg.every = Duration::from_millis(50);
        edit(&mut cfg);
        let srv = Server::new(cfg.clone());
        Node {
            _tmp: tmp,
            dir,
            ident,
            st,
            gw,
            awake,
            wanted,
            cfg,
            srv,
            stop: CancellationToken::new(),
            run: None,
        }
    }

    pub fn run(&mut self) {
        let srv = self.srv.clone();
        let stop = self.stop.clone();
        self.run = Some(tokio::spawn(async move { srv.run(stop).await }));
    }

    pub async fn shutdown(mut self) {
        self.stop.cancel();
        if let Some(r) = self.run.take() {
            r.await.unwrap();
        }
        self.srv.stop().await;
    }

    /// Every node.addr this device wrote.
    pub fn rows(&self) -> Vec<NodeAddr> {
        self.st
            .list(&store::Filter {
                kinds: vec![store::kinds::NODE_ADDR.into()],
                limit: 1000,
                ..Default::default()
            })
            .unwrap()
            .iter()
            .map(|e| NodeAddr::parse(&e.payload).unwrap())
            .collect()
    }
}

pub async fn eventually(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for: {what}");
}
