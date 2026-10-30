//! The home-server directory (mesh.md §2.6, §3.1 "Servers.Addrs order";
//! I:internal/mesh/servers.go): the `node.addr` fold, the LAN address each
//! server's datagrams came from, the last address that worked, and pairing
//! hints. Under iroh the addresses become relay URLs and direct-address
//! hints (mesh.md §5.3); the fold and its order port unchanged.

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gocompat::json::{self, Out, Schema};
use identity::Chain;
use serde::Serialize;
use store::{Event, Filter, MAX_LIST_LIMIT, Store};
use tokio_util::sync::CancellationToken;

/// The LAN carrier's fixed UDP port on every device.
pub const LAN_PORT: u16 = 38419;

/// Reach values of a node.addr row.
pub const REACH_OK: &str = "ok";
pub const REACH_CGNAT: &str = "cgnat";
pub const REACH_UNMAPPED: &str = "unmapped";
pub const REACH_OFF: &str = "off";

/// The kind of the row this fold reads.
pub const KIND_NODE_ADDR: &str = "node.addr";

/// Go `NodeAddr`, a node.addr payload. `public` has no `omitempty` (a nil
/// one is `null`), and `via`, `reach`, `at` are always written.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct NodeAddr {
    pub v: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub port: i64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lan: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub v6: Vec<String>,
    pub public: Option<PublicAddr>,
    pub via: String,
    pub reach: String,
    pub at: i64,
}

/// The router's external address and the mapped ports, or an address the
/// person entered (then `ip` may be a host name).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PublicAddr {
    pub ip: String,
    pub tcp: i64,
    pub udp: i64,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// Go `net.JoinHostPort`.
pub fn join_host_port(host: &str, port: impl std::fmt::Display) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Go `net.SplitHostPort`, the port half (None on a malformed address).
fn split_port(hp: &str) -> Option<&str> {
    if let Some(rest) = hp.strip_prefix('[') {
        let (_, after) = rest.split_once(']')?;
        return after.strip_prefix(':');
    }
    let (host, port) = hp.rsplit_once(':')?;
    if host.contains(':') {
        return None; // too many colons
    }
    Some(port)
}

static NODE_ADDR_SCHEMA: std::sync::LazyLock<Schema> = std::sync::LazyLock::new(|| {
    Schema::object(vec![
        ("v", Schema::Int(64)),
        ("port", Schema::Int(64)),
        ("lan", Schema::Slice(Box::new(Schema::String))),
        ("v6", Schema::Slice(Box::new(Schema::String))),
        (
            "public",
            Schema::Ptr(Box::new(Schema::object(vec![
                ("ip", Schema::String),
                ("tcp", Schema::Int(64)),
                ("udp", Schema::Int(64)),
            ]))),
        ),
        ("via", Schema::String),
        ("reach", Schema::String),
        ("at", Schema::Int(64)),
    ])
});

fn int(o: &Out, name: &str) -> i64 {
    match o.get(name) {
        Some(Out::Int(n)) => *n,
        _ => 0,
    }
}

fn strs(o: &Out, name: &str) -> Vec<String> {
    o.get(name)
        .map(|v| v.items().iter().map(|s| s.str().to_string()).collect())
        .unwrap_or_default()
}

impl NodeAddr {
    /// A node.addr payload as Go's `json.Unmarshal` reads it.
    pub fn parse(raw: &[u8]) -> Option<NodeAddr> {
        let o = json::unmarshal(raw, &NODE_ADDR_SCHEMA).ok()?;
        let public = match o.get("public") {
            Some(Out::Ptr(Some(p))) => Some(PublicAddr {
                ip: p.get_str("ip").to_string(),
                tcp: int(p, "tcp"),
                udp: int(p, "udp"),
            }),
            _ => None,
        };
        Some(NodeAddr {
            v: int(&o, "v"),
            port: int(&o, "port"),
            lan: strs(&o, "lan"),
            v6: strs(&o, "v6"),
            public,
            via: o.get_str("via").to_string(),
            reach: o.get_str("reach").to_string(),
            at: int(&o, "at"),
        })
    }

    /// The payload bytes Go writes.
    pub fn marshal(&self) -> Vec<u8> {
        json::marshal(self).expect("a node.addr always marshals")
    }

    /// Go `Same`: whether two rows say the same thing, ignoring when.
    pub fn same(&self, b: &NodeAddr) -> bool {
        let (mut a, mut b) = (self.clone(), b.clone());
        a.at = 0;
        b.at = 0;
        a.marshal() == b.marshal()
    }

    fn public_ok(&self) -> Option<&PublicAddr> {
        self.public
            .as_ref()
            .filter(|p| !p.ip.is_empty() && p.tcp > 0 && self.reach == REACH_OK)
    }

    /// Go `Dial`: every host:port the row names, LAN first.
    pub fn dial(&self) -> Vec<String> {
        if self.reach == REACH_OFF || self.port == 0 {
            return Vec::new();
        }
        let mut out: Vec<String> = self
            .lan
            .iter()
            .chain(&self.v6)
            .map(|ip| join_host_port(ip, self.port))
            .collect();
        if let Some(p) = self.public_ok() {
            out.push(join_host_port(&p.ip, p.tcp));
        }
        out
    }

    /// Go `Outside`: the address a device away from home dials, or "".
    pub fn outside(&self) -> String {
        self.public_ok()
            .map(|p| join_host_port(&p.ip, p.tcp))
            .unwrap_or_default()
    }
}

/// The per-server cache file: the last address that worked and pairing
/// hints until the server's node.addr row arrives.
pub const SERVERS_FILE: &str = "hub-addrs.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
struct ServerCache {
    #[serde(skip_serializing_if = "String::is_empty")]
    last: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    hint: Vec<String>,
}

#[derive(Debug, Clone)]
struct ServerRow {
    seq: i64,
    addr: NodeAddr,
}

/// One home server this device knows of and where to dial it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub id: String,
    pub addrs: Vec<String>,
}

type ChangeFn = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct State {
    rows: BTreeMap<String, ServerRow>,
    heard: BTreeMap<String, IpAddr>,
    cache: BTreeMap<String, ServerCache>,
    watch: Vec<ChangeFn>,
}

/// Go `Servers`: the directory of home servers.
pub struct Servers {
    self_id: String,
    chain: Option<Arc<Chain>>,
    dir: Option<PathBuf>,
    st: Mutex<State>,
}

fn load_cache(dir: &Path) -> BTreeMap<String, ServerCache> {
    let Ok(raw) = std::fs::read(dir.join(SERVERS_FILE)) else {
        return BTreeMap::new();
    };
    let schema = Schema::Map(Box::new(Schema::object(vec![
        ("last", Schema::String),
        ("hint", Schema::Slice(Box::new(Schema::String))),
    ])));
    match json::unmarshal(&raw, &schema) {
        Ok(Out::Map(Some(m))) => m
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    ServerCache {
                        last: v.get_str("last").to_string(),
                        hint: strs(&v, "hint"),
                    },
                )
            })
            .collect(),
        _ => BTreeMap::new(),
    }
}

/// Go `foldAddr`: each writer's newest row by seq.
fn fold_addr(rows: &mut BTreeMap<String, ServerRow>, ev: &Event) {
    let Some(a) = NodeAddr::parse(&ev.payload) else {
        return;
    };
    if let Some(cur) = rows.get(&ev.device_id)
        && cur.seq > ev.seq
    {
        return;
    }
    rows.insert(
        ev.device_id.clone(),
        ServerRow {
            seq: ev.seq,
            addr: a,
        },
    );
}

impl Servers {
    /// Go `NewServers`: an empty directory; [`Servers::load`] fills it.
    pub fn new(self_id: &str, chain: Option<Arc<Chain>>, data_dir: Option<&Path>) -> Servers {
        let cache = data_dir.map(load_cache).unwrap_or_default();
        Servers {
            self_id: self_id.to_string(),
            chain,
            dir: data_dir.map(Path::to_path_buf),
            st: Mutex::new(State {
                cache,
                ..State::default()
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Go `OnChange`: `f` runs on its own thread whenever the list of
    /// servers or their addresses may have changed.
    pub fn on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        self.lock().watch.push(Arc::new(f));
    }

    fn changed(&self) {
        let fns = self.lock().watch.clone();
        for f in fns {
            std::thread::spawn(move || f());
        }
    }

    /// Go `Load`: folds every node.addr row in `st`.
    pub fn load(&self, st: &Store) -> store::Result<()> {
        let mut rows = BTreeMap::new();
        let mut f = Filter {
            kinds: vec![KIND_NODE_ADDR.into()],
            limit: MAX_LIST_LIMIT,
            ..Filter::default()
        };
        loop {
            let evs = st.list(&f)?;
            for ev in &evs {
                fold_addr(&mut rows, ev);
            }
            if (evs.len() as i64) < f.limit {
                break;
            }
            f.after = evs.last().map(Event::cursor).unwrap_or_default();
        }
        self.lock().rows = rows;
        self.changed();
        Ok(())
    }

    /// Go `Follow`: keeps the fold current until `cancel`. It folds the log
    /// again once subscribed, so a row written between the caller's load
    /// and the subscription is in it.
    pub async fn follow(&self, st: Arc<Store>, cancel: CancellationToken) {
        let sub = st.subscribe();
        let _ = self.load(&st);
        loop {
            let ev = tokio::select! {
                _ = cancel.cancelled() => return,
                ev = sub.recv_async() => ev,
            };
            let Some(ev) = ev else { return };
            if ev.seq == 0 {
                let _ = self.load(&st);
            } else if ev.kind == KIND_NODE_ADDR {
                fold_addr(&mut self.lock().rows, &ev);
                self.changed();
            } else if store::is_person_kind(&ev.kind) {
                self.changed(); // a revoke takes a server out of the list
            }
        }
    }

    fn revoked(&self, dev: &str) -> bool {
        self.chain.as_ref().is_some_and(|c| c.cut(dev).is_some())
    }

    /// Go `Row`: the newest node.addr `dev` wrote, ignoring a revoked
    /// writer's.
    pub fn row(&self, dev: &str) -> Option<NodeAddr> {
        if dev != self.self_id && self.revoked(dev) {
            return None;
        }
        self.lock().rows.get(dev).map(|r| r.addr.clone())
    }

    /// Go `List`: every other device that runs a home server, with the
    /// addresses to dial it at, by id.
    pub fn list(&self) -> Vec<Server> {
        let ids: BTreeSet<String> = {
            let g = self.lock();
            g.rows
                .keys()
                .cloned()
                .chain(
                    g.cache
                        .iter()
                        .filter(|(_, c)| !c.hint.is_empty())
                        .map(|(k, _)| k.clone()),
                )
                .collect()
        };
        ids.into_iter()
            .filter(|id| *id != self.self_id && !self.revoked(id))
            .filter_map(|id| {
                let addrs = self.addrs(&id);
                (!addrs.is_empty()).then_some(Server { id, addrs })
            })
            .collect()
    }

    /// Go `Addrs`: where to dial server `id` now: the heard LAN IP with the
    /// row's port (or a hint's), the row's `dial()`, the last address that
    /// worked, then hints while there is no row. De-duplicated.
    pub fn addrs(&self, id: &str) -> Vec<String> {
        let g = self.lock();
        let row = g.rows.get(id);
        if row.is_some_and(|r| r.addr.reach == REACH_OFF) {
            return Vec::new();
        }
        let mut out: Vec<String> = Vec::new();
        let mut add = |a: &str| {
            if !a.is_empty() && !out.iter().any(|x| x == a) {
                out.push(a.to_string());
            }
        };
        let empty = ServerCache::default();
        let c = g.cache.get(id).unwrap_or(&empty);
        let mut port = row.map(|r| r.addr.port).unwrap_or(0);
        if port == 0 {
            for h in &c.hint {
                if let Some(p) = split_port(h) {
                    port = p.parse().unwrap_or(0);
                    break;
                }
            }
        }
        if let Some(ip) = g.heard.get(id)
            && port > 0
        {
            add(&join_host_port(&ip.to_string(), port));
        }
        if let Some(r) = row {
            for a in r.addr.dial() {
                add(&a);
            }
        }
        add(&c.last);
        if row.is_none() {
            for h in &c.hint {
                add(h);
            }
        }
        out
    }

    /// Go `Heard`: the LAN address a server's authentic datagram came from.
    /// Only a device that runs a server (has a row or a hint) is kept.
    pub fn heard(&self, id: &str, ip: IpAddr) {
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
            v4 => v4,
        };
        {
            let mut g = self.lock();
            let known = g.rows.contains_key(id) || g.cache.contains_key(id);
            if id == self.self_id || !known || g.heard.get(id) == Some(&ip) {
                return;
            }
            g.heard.insert(id.to_string(), ip);
        }
        self.changed();
    }

    /// Go `Worked`: the address a connection to server `id` came up on.
    pub fn worked(&self, id: &str, addr: &str) {
        {
            let mut g = self.lock();
            let c = g.cache.entry(id.to_string()).or_default();
            if id.is_empty() || c.last == addr {
                if c == &ServerCache::default() {
                    g.cache.remove(id);
                }
                return;
            }
            c.last = addr.to_string();
        }
        self.save();
    }

    /// Go `Hint`: addresses for server `id` a pairing handed this device,
    /// until its node.addr row arrives.
    pub fn hint(&self, id: &str, addrs: &[String]) {
        if id.is_empty() || id == self.self_id || addrs.is_empty() {
            return;
        }
        self.lock().cache.entry(id.to_string()).or_default().hint = addrs.to_vec();
        self.save();
        self.changed();
    }

    /// Go `Hints`: every server this device can hand a device it pairs, and
    /// this device's own addresses when it is a server.
    pub fn hints(&self) -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> =
            self.list().into_iter().map(|s| (s.id, s.addrs)).collect();
        if let Some(r) = self.row(&self.self_id) {
            let d = r.dial();
            if !d.is_empty() {
                out.insert(self.self_id.clone(), d);
            }
        }
        out
    }

    fn save(&self) {
        let Some(dir) = &self.dir else { return };
        let raw = json::marshal_indent(&self.lock().cache, "", "  ").unwrap_or_default();
        let _ = crate::fsutil::write_replace(&dir.join(SERVERS_FILE), &raw);
    }
}
