//! The mesh in the Core (M5; Go core.go's `mesh.New` and `Net`): one
//! [`IrohNet`] keyed by the device key carries the mesh Node and pairing
//! alike (both on the `irori-meshseam` seam), so the device has one
//! endpoint and one EndpointId (`irori/mesh/1` and `irori/pair/1` on the
//! same socket).
//!
//! - The Node joins nothing until there is a second certified device, so a
//!   lone device binds no socket (`TestNothingOpensBeforeTheFirstJoin`).
//! - The address book follows the mesh's `Servers` fold: on every
//!   `node.addr` or person row the home relays and the device list are fed
//!   to the endpoint ([`irori_net::book::feed`]) and live connections the
//!   Chain no longer admits close (`recheck`, Go `dropRevoked`).
//! - The home server's own relay port (`on_relay`, Go `Net.SetLocal`) makes
//!   this device's endpoint use its own relay too, and the pairing bridge
//!   name it at the LAN address.

use std::sync::{Arc, Mutex};

use irori_identity::{Chain, Identity};
use irori_net::book::{ServerAddrs, feed};
use irori_net::transport::IrohNet;
use store::Store;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// The Core's mesh Node, its endpoint and the home-server directory.
pub(crate) struct Mesh {
    pub node: Arc<mesh::Node>,
    pub net: IrohNet,
    pub servers: Arc<mesh::Servers>,
    chain: Arc<Chain>,
    own_key: iroh::EndpointId,
    /// This device's own home relay port, while its server runs.
    local: Mutex<Option<u16>>,
    refeed: Arc<Notify>,
    /// The other devices' home servers fed to the endpoint as relays: the
    /// relay URLs of each and when this device started using it (Go
    /// roomnet `hubs` and their `since`; see crate::meshview).
    pub(crate) hubs: Mutex<std::collections::BTreeMap<String, Hub>>,
}

/// One other device's home server as the address book feeds it.
pub(crate) struct Hub {
    pub urls: Vec<iroh::RelayUrl>,
    pub since: std::time::Instant,
}

/// What [`Mesh::new`] builds the Node from.
pub(crate) struct Parts {
    pub ident: Arc<Identity>,
    pub chain: Arc<Chain>,
    pub store: Arc<Store>,
    pub files: Option<Arc<blob::Files>>,
    pub data_dir: std::path::PathBuf,
    pub version: String,
    pub now: Option<mesh::node::NowFn>,
    /// Per writing device, the rows the folds ignored (Go `ignoredBy`).
    pub ignored: Option<mesh::node::IgnoredFn>,
}

impl Mesh {
    pub(crate) fn new(net: IrohNet, p: Parts) -> Arc<Mesh> {
        let me = p.ident.device_id();
        let own_key = iroh::EndpointId::from_bytes(&p.ident.device_pub()).expect("device key");
        let servers = Arc::new(mesh::Servers::new(
            &me,
            Some(p.chain.clone()),
            Some(&p.data_dir),
        ));
        let node = mesh::Node::new(mesh::Options {
            store: p.store,
            ident: p.ident,
            chain: p.chain.clone(),
            net: Arc::new(net.clone()),
            version: p.version,
            files: p.files,
            data_dir: Some(p.data_dir),
            intervals: mesh::Intervals::default(),
            trust_any_peer: false,
            ignored: p.ignored,
            boards: None,
            logf: Some(Arc::new(|l: &str| crate::log(l))),
            now: p.now,
        });
        // The LAN lookup's sightings (Go roomnet OnHeard → Servers.Heard):
        // the sender's key is its certified devicePub (D2).
        {
            let (s, chain) = (servers.clone(), p.chain.clone());
            net.set_on_heard(Arc::new(move |id, from| {
                let hex_id = hex::encode(id.as_bytes());
                if let Some(d) = chain
                    .devices()
                    .into_iter()
                    .find(|d| !d.revoked && d.cert.device_pub == hex_id)
                {
                    s.heard(&d.cert.device_id, from.ip());
                }
            }));
        }
        let refeed = Arc::new(Notify::new());
        {
            let r = refeed.clone();
            servers.on_change(move || r.notify_one());
        }
        Arc::new(Mesh {
            node,
            net,
            servers,
            chain: p.chain,
            own_key,
            local: Mutex::new(None),
            refeed,
            hubs: Mutex::new(std::collections::BTreeMap::new()),
        })
    }

    /// Home's `on_relay`: the port this device's own relay serves on and
    /// its LAN address, or `None` once it stops. The endpoint homes on it
    /// (the book, at 127.0.0.1), and the pairing bridge names it at the LAN
    /// address to a joiner at home whose code is an away member's
    /// ([`irori_net::pairnet::PairLan::set_relay`]; none without a LAN).
    pub(crate) fn set_local_relay(&self, up: Option<home::RelayUp>) {
        *self.local.lock().unwrap_or_else(|e| e.into_inner()) = up.map(|u| u.port);
        let bridge = up.and_then(|u| {
            u.lan_ip
                .map(|ip| (irori_net::relay::relay_url(ip, u.port), self.own_key))
        });
        self.net.pair_lan().set_relay(bridge);
        self.refeed.notify_one();
    }

    /// Remove device (Go `Mesh().Revoke`).
    pub(crate) fn revoke_fn(self: &Arc<Self>) -> crate::devices::RevokeFn {
        let m = self.clone();
        Arc::new(move |dev: String, why: String| {
            let m = m.clone();
            Box::pin(async move { m.node.revoke(&dev, &why).await })
        })
    }

    async fn feed_now(&self) {
        let list: Vec<ServerAddrs> = self
            .servers
            .list()
            .into_iter()
            .map(|s| ServerAddrs {
                device_id: s.id,
                addrs: s.addrs,
            })
            .collect();
        self.note_hubs(&list);
        let local = *self.local.lock().unwrap_or_else(|e| e.into_inner());
        let own = local.map(|port| irori_net::book::own_relay(port, self.own_key));
        feed(&self.net, &list, &self.chain, own).await;
        self.net.recheck();
        // A dial tried before the book had the device failed with no
        // address: the Node dials again now that it has one.
        self.node.kick();
    }

    /// Keeps [`Mesh::hubs`] to the servers the book feeds as relays: a new
    /// one starts its clock now, one gone is dropped.
    fn note_hubs(&self, list: &[ServerAddrs]) {
        let me = self.net.self_id().to_string();
        let mut hubs = self.hubs.lock().unwrap_or_else(|e| e.into_inner());
        let mut keep = std::collections::BTreeSet::new();
        for s in list {
            let urls: Vec<iroh::RelayUrl> =
                irori_net::book::relays_for(std::slice::from_ref(s), &self.chain, &me)
                    .into_iter()
                    .map(|r| r.url)
                    .collect();
            if urls.is_empty() {
                continue;
            }
            keep.insert(s.device_id.clone());
            hubs.entry(s.device_id.clone())
                .and_modify(|h| h.urls = urls.clone())
                .or_insert_with(|| Hub {
                    urls,
                    since: std::time::Instant::now(),
                });
        }
        hubs.retain(|id, _| keep.contains(id));
    }

    /// Runs the Node, the `Servers` fold and the address book until
    /// `cancel`; closes the endpoint after.
    pub(crate) async fn run(self: Arc<Self>, st: Arc<Store>, cancel: CancellationToken) {
        let _ = self.servers.load(&st);
        let follow = {
            let (s, c) = (self.servers.clone(), cancel.clone());
            tokio::spawn(async move { s.follow(st, c).await })
        };
        let book = {
            let (m, c) = (self.clone(), cancel.clone());
            tokio::spawn(async move {
                loop {
                    m.feed_now().await;
                    tokio::select! {
                        _ = c.cancelled() => return,
                        _ = m.refeed.notified() => {}
                    }
                }
            })
        };
        self.node.run(cancel.clone()).await;
        let _ = follow.await;
        let _ = book.await;
        self.net.close().await;
    }
}

/// The mesh's `Servers` as the Pairer's hints (Go passes `m.Servers()`).
struct Hints(Arc<mesh::Servers>);

impl irori_pair::ServerHints for Hints {
    fn hints(&self) -> std::collections::BTreeMap<String, Vec<String>> {
        self.0.hints()
    }

    fn hint(&self, id: &str, addrs: &[String]) {
        self.0.hint(id, addrs)
    }
}

impl Mesh {
    /// The pairing's server hints: this mesh's `Servers`.
    pub(crate) fn hints(&self) -> Arc<dyn irori_pair::ServerHints> {
        Arc::new(Hints(self.servers.clone()))
    }
}
