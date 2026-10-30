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
//! - Shared boards (stage 2): one [`mesh::boardmesh::BoardMesh`] per open
//!   board on `irori/board/1`, beside the private Node. Opening a board
//!   counts its verifier at the endpoint gate (`NetConfig::boards`, the
//!   `MemberAdmit` registry), feeds its member devices to the board's own
//!   address book, and starts its mesh; every refeed re-checks each
//!   board's live connections against its verifier.

use std::sync::{Arc, Mutex};

use irori_identity::{Chain, Identity};
use irori_net::book::{ServerAddrs, feed};
use irori_net::transport::IrohNet;
use mesh::boardmesh::{BoardMesh, BoardMeshOpts};
use store::Store;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// The Core's mesh Node, its endpoint and the home-server directory.
pub(crate) struct Mesh {
    pub node: Arc<mesh::Node>,
    pub net: IrohNet,
    pub servers: Arc<mesh::Servers>,
    /// The attachment store the Node and every board mesh share.
    files: Option<Arc<blob::Files>>,
    chain: Arc<Chain>,
    own_key: iroh::EndpointId,
    /// This Core's version, for board hellos.
    version: String,
    /// This device's own home relay port, while its server runs.
    local: Mutex<Option<u16>>,
    refeed: Arc<Notify>,
    /// The other devices' home servers fed to the endpoint as relays: the
    /// relay URLs of each and when this device started using it (Go
    /// roomnet `hubs` and their `since`; see crate::meshview).
    pub(crate) hubs: Mutex<std::collections::BTreeMap<String, Hub>>,
    /// Open shared boards' meshes, by board id.
    boards: Mutex<std::collections::BTreeMap<String, Arc<BoardMesh>>>,
    /// Direct addresses heard for other members' devices (an invite's
    /// ticket, #44), by device id.
    board_hints: Mutex<std::collections::BTreeMap<String, Vec<std::net::SocketAddr>>>,
    /// This device's home server, once the Core made it (B1: it writes
    /// `board.server` into each board).
    home: std::sync::OnceLock<home::Server>,
}

/// How often the address book is fed again with nothing else happening:
/// a home server's public address moves without a row here.
const REFEED_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// What [`Mesh::open_board`] runs a shared board's mesh on: the board's
/// log, its verifier (the log's store Verifier) and its secrets.
pub struct BoardParts {
    pub board_id: String,
    pub person_id: String,
    pub log: Arc<Store>,
    pub verifier: Arc<irori_identity::BoardVerifier>,
    pub secrets: Arc<irori_identity::BoardSecrets>,
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
            version: p.version.clone(),
            files: p.files.clone(),
            data_dir: Some(p.data_dir),
            intervals: mesh::Intervals::default(),
            trust_any_peer: false,
            ignored: p.ignored,
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
            files: p.files,
            chain: p.chain,
            own_key,
            version: p.version,
            local: Mutex::new(None),
            refeed,
            hubs: Mutex::new(std::collections::BTreeMap::new()),
            boards: Mutex::new(std::collections::BTreeMap::new()),
            board_hints: Mutex::new(std::collections::BTreeMap::new()),
            home: std::sync::OnceLock::new(),
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

    /// This device's home server (set once, after the Core makes it).
    pub(crate) fn set_home(&self, h: home::Server) {
        let _ = self.home.set(h);
        self.refeed.notify_one();
    }

    /// B1 (v0.5 §6.2): a server device writes its `board.server` row into
    /// each board it is a counted member device of, only when it changes.
    fn write_board_servers(&self, boards: &[Arc<BoardMesh>]) {
        let Some(h) = self.home.get() else { return };
        let Ok(row) = h.board_server() else {
            return; // never served here: not a server device
        };
        let me = self.net.self_id();
        for m in boards {
            if m.verifier().cert(me).is_none() || m.verifier().cut(me).is_some() {
                continue;
            }
            if let Err(e) = mesh::boardmesh::write_board_server(m.log(), &row) {
                crate::log(&format!("board {}: board.server: {e}", m.board_id()));
            }
        }
    }

    /// Each board's `board.server` relays (B1), by person: the servers'
    /// public addresses, pinned to the key the board certifies for each.
    fn board_relays(
        m: &BoardMesh,
    ) -> std::collections::BTreeMap<String, Vec<irori_net::endpoint::RelaySpec>> {
        let mut out: std::collections::BTreeMap<String, Vec<_>> = Default::default();
        let Ok(servers) = mesh::boardmesh::board_servers(m.log(), m.verifier()) else {
            return out;
        };
        for s in servers {
            let Some(p) = s.public.as_ref().filter(|p| !p.ip.is_empty() && p.tcp > 0) else {
                continue;
            };
            let host = if p.ip.contains(':') && !p.ip.starts_with('[') {
                format!("[{}]:{}", p.ip, p.tcp)
            } else {
                format!("{}:{}", p.ip, p.tcp)
            };
            let Some((url, port)) = irori_net::book::relay_of(&host) else {
                continue;
            };
            let (Some(cert), Some(person)) = (
                m.verifier().cert(&s.device_id),
                m.verifier().person_of(&s.device_id),
            ) else {
                continue;
            };
            let Some(key) = hex::decode(&cert.device_pub)
                .ok()
                .and_then(|b| <[u8; 32]>::try_from(b).ok())
                .and_then(|b| iroh::EndpointId::from_bytes(&b).ok())
            else {
                continue;
            };
            let quic_port = u16::try_from(p.udp).ok().filter(|u| *u > 0).unwrap_or(port);
            out.entry(person)
                .or_default()
                .push(irori_net::endpoint::RelaySpec {
                    url,
                    quic_port,
                    server_pub: key,
                });
        }
        out
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
        let boards = self.board_list();
        self.write_board_servers(&boards);
        let mut relays = Vec::new();
        for m in &boards {
            let by_person = Self::board_relays(m);
            self.feed_board(m, &by_person);
            m.recheck();
            relays.push(irori_net::board::BoardRelays {
                board_id: m.board_id().to_string(),
                servers: by_person.into_values().flatten().collect(),
            });
        }
        if let Err(e) = self.net.set_board_servers(relays).await {
            crate::log(&format!("board servers: {}", e.0));
        }
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

    pub(crate) fn board_list(&self) -> Vec<Arc<BoardMesh>> {
        self.boards
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect()
    }

    /// Board `m`'s member devices as its address book: each by its
    /// certified key. Addresses come from the lookups (the private book's
    /// for own devices, the LAN lookup), the hints of
    /// [`Mesh::board_hint`] and, with B1, `board.server` relays.
    fn feed_board(
        &self,
        m: &BoardMesh,
        relays: &std::collections::BTreeMap<String, Vec<irori_net::endpoint::RelaySpec>>,
    ) {
        let hints = self
            .board_hints
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let peers = m
            .member_devices()
            .into_iter()
            .filter_map(|d| {
                let b: [u8; 32] = hex::decode(&d.device_pub).ok()?.try_into().ok()?;
                Some(irori_net::transport::PeerAddr {
                    id: iroh::EndpointId::from_bytes(&b).ok()?,
                    // B1: the device's person's home server, from its
                    // board.server row.
                    relay: relays
                        .get(&d.person_id)
                        .and_then(|r| r.first())
                        .map(|r| r.url.clone()),
                    direct: hints.get(&d.device_id).cloned().unwrap_or_default(),
                    device_id: d.device_id,
                })
            })
            .collect();
        self.net.set_board_peers(m.board_id(), peers);
    }

    /// Opens shared board `p.board_id` on the network: its members count at
    /// the endpoint gate, its member devices are dialled on
    /// `irori/board/1`, and its rows move only on those connections.
    /// Opening an open board again is a no-op.
    pub(crate) async fn open_board(self: &Arc<Self>, p: BoardParts) -> Result<(), String> {
        let verifier = p.verifier.clone();
        let own: mesh::boardmesh::OwnDevicesFn = {
            let (chain, person) = (self.chain.clone(), p.person_id.clone());
            Arc::new(move || {
                chain
                    .devices()
                    .into_iter()
                    .filter(|d| !d.revoked)
                    .map(|d| mesh::boardmesh::MemberDevice {
                        person_id: person.clone(),
                        device_id: d.cert.device_id,
                        device_pub: d.cert.device_pub,
                    })
                    .collect()
            })
        };
        let m = Arc::new(BoardMesh::new(BoardMeshOpts {
            board_id: p.board_id.clone(),
            self_id: self.net.self_id().to_string(),
            person_id: p.person_id,
            core_version: self.version.clone(),
            log: p.log,
            verifier: p.verifier,
            secrets: p.secrets,
            transport: Arc::new(self.net.clone()),
            own_devices: Some(own),
            files: self.files.clone(),
        }));
        m.set_log(Arc::new(|l: &str| crate::log(l)));
        {
            // Rows from a member may move the membership: refeed.
            let r = self.refeed.clone();
            m.on_rows(Arc::new(move || r.notify_one()));
        }
        {
            // One check and insert, so two openers can't both start it.
            let mut g = self.boards.lock().unwrap_or_else(|e| e.into_inner());
            if g.contains_key(&p.board_id) {
                return Ok(());
            }
            g.insert(p.board_id.clone(), m.clone());
        }
        self.net.board_members().add(&p.board_id, verifier);
        self.feed_board(&m, &Self::board_relays(&m));
        self.refeed.notify_one(); // board.server rows and the relays
        if let Err(e) = m.start().await {
            self.close_board(&p.board_id, irori_meshseam::reason::LEAVING);
            return Err(e.to_string());
        }
        Ok(())
    }

    /// Closes shared board `board_id` on the network (left, removed,
    /// forgotten): its connections close with `why` and its members no
    /// longer count at the gate.
    pub(crate) fn close_board(&self, board_id: &str, why: &str) {
        let m = self
            .boards
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(board_id);
        if let Some(m) = m {
            m.close();
        }
        self.net.board_members().remove(board_id);
        let _ = self.net.drop_board(board_id, why);
    }

    /// Direct addresses for another member's device (an invite ticket's),
    /// used by every board it is a member device of.
    pub(crate) fn board_hint(&self, device_id: &str, addrs: Vec<std::net::SocketAddr>) {
        self.board_hints
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(device_id.to_string(), addrs);
        self.refeed.notify_one();
    }

    /// Every open board's mesh pulls now (a board opened or a row landed).
    pub(crate) fn kick_boards(&self) {
        let boards: Vec<_> = self
            .boards
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        for b in boards {
            b.kick();
        }
    }

    /// Board `board_id`'s mesh: pull now after a local write, or recheck
    /// after a local membership change.
    pub(crate) fn board(&self, board_id: &str) -> Option<Arc<BoardMesh>> {
        self.boards
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(board_id)
            .cloned()
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
                        _ = tokio::time::sleep(REFEED_EVERY) => {}
                    }
                }
            })
        };
        self.node.run(cancel.clone()).await;
        let _ = follow.await;
        let _ = book.await;
        for m in self.board_list() {
            m.close();
        }
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
