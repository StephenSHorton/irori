//! The home server on this device (I:internal/home/home.go, awake.go;
//! mesh.md §3.3, §5.4): on the always-on PC it runs the embedded iroh
//! relay on port N, maps N on the router, holds off idle sleep, and
//! publishes where it can be reached as `node.addr` rows.
//!
//! It runs only where it is switched on, and only once the device has
//! another device or an open pairing window ([`Config::wanted`]): a server
//! on its own binds nothing. Rows are written only on change, compared
//! with the newest row in the log, not with memory.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use iroh::EndpointId;
use irori_identity::{Chain, Identity};
use serde::{Deserialize, Serialize};
use store::Store;
use sys::awake::{Hold, Holder};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::nodeaddr::{NodeAddr, PublicAddr, REACH_CGNAT, REACH_OFF, REACH_OK, REACH_UNMAPPED};
use crate::portmap::{self, Mapper, Port};
use crate::relay::{AdmitFn, Relay, SpawnError};
use crate::settings::{self, PORT_MAX, PORT_MIN, Settings};

/// Reachability, the one-word summary in Devices.
/// Devices at home reach it; away from home they can't.
pub const REACH_HOME: &str = "home";
/// Reachable from outside home too.
pub const REACH_OUTSIDE: &str = "outside";
/// Away from home needs a port forward (mapping failed).
pub const REACH_FORWARD: &str = "forward";
/// The ISP shares the public address: nothing from outside helps.
pub const REACH_SHARED: &str = "cgnat";

/// How often relay guests are checked against their pairing window, so
/// one whose window expired is dropped (architecture §8 M7).
pub const GUEST_SWEEP: Duration = Duration::from_secs(2);

pub type WantedFn = Arc<dyn Fn() -> bool + Send + Sync>;
pub type OnRelayFn = Arc<dyn Fn(Option<RelayUp>) + Send + Sync>;

/// The relay as [`Config::on_relay`] is told it when the server starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayUp {
    /// The port N it serves on (TCP and UDP).
    pub port: u16,
    /// Where other devices on the LAN reach it: the primary LAN address
    /// (the one the server restarts on when it changes), or
    /// [`Config::bind_ip`] when that names one address. `None`: no LAN.
    pub lan_ip: Option<IpAddr>,
}
pub type LogFn = Arc<dyn Fn(&str) + Send + Sync>;

/// Sets up a [`Server`] (Go `home.Config`).
#[derive(Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub ident: Arc<Identity>,
    pub chain: Arc<Chain>,
    pub store: Arc<Store>,
    /// The §4.5 condition: another certified device, or an open pairing
    /// window (default: always).
    pub wanted: Option<WantedFn>,
    /// Told the relay's port N and LAN address when the server starts
    /// serving and `None` when it stops: the net crate's endpoint adds (or
    /// drops) its own relay, the iroh form of Go's `Net.SetLocal(hub, ice)`,
    /// and the pairing bridge names it at the LAN address.
    pub on_relay: Option<OnRelayFn>,
    /// Pairing guest windows (M7, `irori_net::guest`): the relay admits a
    /// joiner whose auth token opens one, and the server runs while one is
    /// open on it, as Go's runs for an open pairing window.
    pub guests: Option<irori_net::guest::GuestWindows>,
    /// B1 (stage 2, card #43): member devices of boards held here, admitted
    /// to the relay beside this person's devices. Interface only: not read
    /// yet.
    pub board_admit: Option<crate::relay::BoardAdmitFn>,

    // Tests replace these.
    /// The address port N binds. `None` is the point of the server: the
    /// wildcard `0.0.0.0`, plus TCP `[::]` while "Reachable from outside
    /// home" is on. Tests: 127.0.0.1.
    pub bind_ip: Option<IpAddr>,
    /// This machine's private IPv4 addresses on the home network, the first
    /// being the primary (default: [`irori_lan::netif::lan_addrs`]).
    pub lan_addrs: Option<Arc<dyn Fn() -> Vec<Ipv4Addr> + Send + Sync>>,
    /// Global IPv6 addresses (default: [`irori_lan::netif::v6_addrs`]).
    pub v6_addrs: Option<Arc<dyn Fn() -> Vec<Ipv6Addr> + Send + Sync>>,
    /// Finds the router (default: SSDP, then NAT-PMP at the default gateway).
    pub discover: Option<portmap::DiscoverFn>,
    pub portmap_timing: portmap::Timing,
    /// Whether an address is on a network Windows has as Public.
    pub on_public_network: Option<Arc<dyn Fn(IpAddr) -> bool + Send + Sync>>,
    /// Holds off idle system sleep while the server runs (default:
    /// [`sys::awake::system`]).
    pub awake: Option<Arc<dyn Holder>>,
    /// How often addresses are checked for a change (default 30 s).
    pub every: Duration,
    pub now: Option<Arc<dyn Fn() -> SystemTime + Send + Sync>>,
    pub logf: Option<LogFn>,
}

impl Config {
    pub fn new(
        data_dir: PathBuf,
        ident: Arc<Identity>,
        chain: Arc<Chain>,
        store: Arc<Store>,
    ) -> Self {
        Config {
            data_dir,
            ident,
            chain,
            store,
            wanted: None,
            on_relay: None,
            guests: None,
            board_admit: None,
            bind_ip: None,
            lan_addrs: None,
            v6_addrs: None,
            discover: None,
            portmap_timing: portmap::Timing::default(),
            on_public_network: None,
            awake: None,
            every: Duration::from_secs(30),
            now: None,
            logf: None,
        }
    }
}

/// The home server as Devices shows it on the server itself (Go
/// `home.Status`), fields in Go's order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub on: bool,
    pub outside: bool,
    /// The manual public address, as typed.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub public: String,
    pub running: bool,
    #[serde(skip_serializing_if = "is_zero")]
    pub port: i64,
    /// [`REACH_HOME`], [`REACH_OUTSIDE`], [`REACH_FORWARD`] or
    /// [`REACH_SHARED`] while it runs.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reach: String,
    /// Where a device away from home dials it; "" when it isn't reachable.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub address: String,
    /// The one status line, only when something is wrong.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub line: String,
    /// "Keep this PC awake while it's the server".
    pub awake: bool,
    /// "Keeping this PC awake" while it runs and holds it, or why it can't.
    #[serde(rename = "awakeLine", skip_serializing_if = "String::is_empty")]
    pub awake_line: String,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// Everything a started server holds.
struct Running {
    port: u16,
    /// The default listeners took IPv6 too (outside on).
    dual: bool,
    lan_ip: Option<Ipv4Addr>,
    relay: Arc<Relay>,
    mapper: Option<Arc<Mapper>>,
    /// Held while it serves and the setting is on.
    awake: Option<Box<dyn Hold>>,
    /// Why the hold couldn't be taken.
    awake_err: String,
    /// Bumped per start, so a stale `Running` is told apart.
    epoch: u64,
}

struct State {
    set: Settings,
    run: Option<Running>,
    /// Why the last start failed.
    err: String,
    /// Port N was taken at a start and this is the old one.
    moved: i64,
    last: Option<NodeAddr>,
    epoch: u64,
}

struct Inner {
    cfg: Config,
    kick: Arc<Notify>,
    st: Mutex<State>,
    /// Serializes reconcile and stop (Go runs them on one goroutine, and
    /// tests call them directly).
    op: tokio::sync::Mutex<()>,
}

/// The home server (Go `home.Server`).
#[derive(Clone)]
pub struct Server {
    inner: Arc<Inner>,
}

/// Go `CheckDevice` by key: `pubkey` is a device other than `self_id` with a
/// valid, unrevoked cert for this person naming it. The one copy of the
/// rule is `irori_net::admit` (Go home.go:435 calls `mesh.CheckDevice`).
pub fn check_device_by_key(chain: &Chain, self_id: &str, pubkey: &[u8; 32]) -> bool {
    irori_net::admit::admitted_device(chain, self_id, pubkey).is_some()
}

fn pick_port() -> i64 {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).expect("getrandom");
    let span = u32::from(PORT_MAX - PORT_MIN) + 1;
    i64::from(u32::from(PORT_MIN) + u32::from_le_bytes(b) % span)
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn log(&self, msg: &str) {
        if let Some(l) = &self.cfg.logf {
            l(msg);
        }
    }

    fn now_ms(&self) -> i64 {
        let t = self.cfg.now.as_ref().map_or_else(SystemTime::now, |f| f());
        t.duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64)
    }

    fn wanted(&self) -> bool {
        self.cfg.wanted.as_ref().is_none_or(|f| f())
            || self.cfg.guests.as_ref().is_some_and(|g| g.any_open())
    }

    fn lan_addrs(&self) -> Vec<Ipv4Addr> {
        match &self.cfg.lan_addrs {
            Some(f) => f(),
            None => irori_lan::netif::lan_addrs(),
        }
    }

    fn v6_addrs(&self) -> Vec<Ipv6Addr> {
        match &self.cfg.v6_addrs {
            Some(f) => f(),
            None => irori_lan::netif::v6_addrs(),
        }
    }

    fn primary(&self) -> Option<Ipv4Addr> {
        self.lan_addrs().first().copied()
    }

    fn default_listen(&self) -> bool {
        self.cfg.bind_ip.is_none()
    }

    fn on_public(&self, ip: IpAddr) -> bool {
        match &self.cfg.on_public_network {
            Some(f) => f(ip),
            None => irori_lan::netif::on_public_network(ip),
        }
    }

    fn self_id(&self) -> String {
        self.cfg.ident.device_id()
    }

    fn save(&self, set: &Settings) {
        if let Err(e) = settings::save(&self.cfg.data_dir, set) {
            self.log(&format!("home: save {}: {e}", settings::FILE));
        }
    }

    fn admit(&self) -> AdmitFn {
        let chain = self.cfg.chain.clone();
        let self_id = self.self_id();
        let self_pub = self.cfg.ident.device_pub();
        Arc::new(move |id: &EndpointId| {
            // The server's own endpoint uses its own relay, so the others
            // reach it there.
            *id.as_bytes() == self_pub || check_device_by_key(&chain, &self_id, id.as_bytes())
        })
    }
}

impl Server {
    /// Go `home.New`: reads `home-server.json` (an unreadable one keeps the
    /// server off). [`Server::run`] starts it.
    pub fn new(cfg: Config) -> Server {
        let set = match settings::load(&cfg.data_dir) {
            Ok(s) => s,
            Err(e) => {
                if let Some(l) = &cfg.logf {
                    l(&format!("home: {e} (the server stays off)"));
                }
                Settings::default()
            }
        };
        Server {
            inner: Arc::new(Inner {
                cfg,
                kick: Arc::new(Notify::new()),
                st: Mutex::new(State {
                    set,
                    run: None,
                    err: String::new(),
                    moved: 0,
                    last: None,
                    epoch: 0,
                }),
                op: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// Re-checks at once whether the server should run: a pairing window
    /// opened or closed, or a setting changed.
    pub fn kick(&self) {
        self.inner.kick.notify_one();
    }

    /// Keeps the server in the state its setting and §4.5 ask for until
    /// `stop` is cancelled, then stops it (its port mappings are removed).
    pub async fn run(&self, stop: CancellationToken) {
        let sub = self.inner.cfg.store.subscribe();
        let mut tick = tokio::time::interval(self.inner.cfg.every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await;
        // Guests whose pairing window expired are dropped within this.
        let mut sweep = tokio::time::interval(GUEST_SWEEP);
        sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        sweep.tick().await;
        self.reconcile().await;
        loop {
            tokio::select! {
                _ = stop.cancelled() => break,
                ev = sub.recv_async() => {
                    let Some(ev) = ev else { break };
                    if ev.is_resync() || store::is_person_kind(&ev.kind) {
                        self.drop_revoked();
                        self.reconcile().await;
                    }
                }
                // A window opened or closed (the Pairer's notify), or a
                // setting changed: a closed window's guests go at once.
                _ = self.inner.kick.notified() => {
                    self.drop_revoked();
                    self.reconcile().await;
                }
                // A new DHCP lease, interface or network.
                _ = tick.tick() => self.reconcile().await,
                _ = sweep.tick() => self.drop_revoked(),
            }
        }
        sub.cancel();
        self.stop().await;
    }

    /// The current setting.
    pub fn settings(&self) -> Settings {
        self.inner.lock().set.clone()
    }

    /// Changes the setting (Settings → Devices → Home server, GUI only): on,
    /// "Reachable from outside home", and a public address for when mapping
    /// fails ("" for none).
    pub fn set(&self, on: bool, outside: bool, public: &str) -> Result<(), String> {
        if !public.is_empty() {
            crate::hostport::split_public(public, 1)?;
        }
        {
            let mut st = self.inner.lock();
            let mut set = st.set.clone();
            set.on = on;
            set.outside = Some(outside);
            set.public = public.to_string();
            settings::save(&self.inner.cfg.data_dir, &set).map_err(|e| e.to_string())?;
            st.set = set;
        }
        self.kick();
        Ok(())
    }

    /// Changes "Keep this PC awake while it's the server" (GUI only).
    pub fn set_awake(&self, on: bool) -> Result<(), String> {
        {
            let mut st = self.inner.lock();
            let mut set = st.set.clone();
            set.awake = Some(on);
            settings::save(&self.inner.cfg.data_dir, &set).map_err(|e| e.to_string())?;
            st.set = set;
        }
        self.kick();
        Ok(())
    }

    /// The relay's port N while it serves.
    /// The `board.server` row this server would write into each board its
    /// person is in (B1, v0.5 §6.2): its public address and mapped ports
    /// only, `public: None` while off or outside reach is off. Interface
    /// only (card #43).
    pub fn board_server(&self) -> Result<mesh::boardmesh::BoardServer, String> {
        Err("boards2: transport not built yet".into())
    }

    pub fn relay_port(&self) -> Option<u16> {
        self.inner.lock().run.as_ref().map(|r| r.port)
    }

    /// Endpoints with a live connection to the relay.
    pub fn relay_clients(&self) -> Vec<EndpointId> {
        let relay = self.inner.lock().run.as_ref().map(|r| r.relay.clone());
        relay.map(|r| r.connected()).unwrap_or_default()
    }

    /// Closes the relay connection of every client that is no longer
    /// admitted: a device no longer certified (Go `dropRevoked`), or a guest
    /// whose pairing window closed or expired (go-room's hub drops a
    /// window's guests with it). Admission then refuses its reconnect.
    pub fn drop_revoked(&self) {
        let relay = self.inner.lock().run.as_ref().map(|r| r.relay.clone());
        if let Some(r) = relay {
            let n = r.drop_refused();
            if n > 0 {
                self.inner.log(&format!(
                    "home: dropped {n} relay client(s) no longer admitted"
                ));
            }
        }
    }

    /// Starts, restarts or stops the server and writes `node.addr` when it
    /// changed (Go `reconcile`; the Run loop calls it, tests may too).
    pub async fn reconcile(&self) {
        let _op = self.inner.op.lock().await;
        let i = &self.inner;
        let (set, running, lan_ip, dual) = {
            let st = i.lock();
            let r = st.run.as_ref();
            (
                st.set.clone(),
                r.is_some(),
                r.and_then(|r| r.lan_ip),
                r.is_some_and(|r| r.dual),
            )
        };
        if !set.on || !i.wanted() {
            if running {
                self.stop_locked().await;
            }
            if !set.on {
                // Switched off: devices stop dialing it.
                if let Some(cur) = self.last_row()
                    && cur.reach != REACH_OFF
                {
                    self.write(NodeAddr {
                        v: 1,
                        reach: REACH_OFF.into(),
                        ..Default::default()
                    });
                }
            }
            return;
        }
        let mut running = running;
        if running && i.primary() != lan_ip {
            // Mappings point at the LAN address.
            i.log("home: the LAN address changed, restarting the server");
            self.stop_locked().await;
            running = false;
        }
        if running && i.default_listen() && dual != set.outside_on() {
            // IPv6 is listened on only while outside is on.
            self.stop_locked().await;
            running = false;
        }
        if !running && let Err(e) = self.start().await {
            i.log(&format!("home: {e}"));
            i.lock().err = e;
            return;
        }
        self.sync_mapper().await;
        self.sync_awake();
        if let Some(a) = self.addr() {
            match self.last_row() {
                Some(cur) if cur.same(&a) => {}
                _ => self.write(a),
            }
        }
    }

    /// Binds port N (another one if N is taken), then the relay.
    async fn start(&self) -> Result<(), String> {
        let i = &self.inner;
        let set = i.lock().set.clone();
        let mut port = if set.port == 0 { pick_port() } else { set.port };
        let dual = set.outside_on();
        let default = i.default_listen();
        let bind_ip = i.cfg.bind_ip.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        let seed = i.cfg.ident.expose_device_seed();
        let mut last_err = String::new();
        let mut relay = None;
        for attempt in 0..8 {
            if attempt > 0 {
                port = pick_port();
            }
            if !(1..=65535).contains(&port) {
                last_err = format!("port {port} out of range");
                continue;
            }
            let guest = i
                .cfg
                .guests
                .clone()
                .map(|g| Arc::new(move |t: &str| g.token_ok(t)) as crate::relay::GuestFn);
            match Relay::spawn(
                &seed,
                bind_ip,
                port as u16,
                dual && default,
                i.admit(),
                guest,
            )
            .await
            {
                Ok(r) => {
                    relay = Some(r);
                    break;
                }
                Err(SpawnError::Bind(e)) => last_err = e,
                Err(SpawnError::Other(e)) => return Err(format!("relay: {e}")),
            }
        }
        let Some(relay) = relay else {
            return Err(format!("can't listen on a port: {last_err}"));
        };
        if port != set.port {
            let saved = {
                let mut st = i.lock();
                if st.set.port != 0 {
                    st.moved = st.set.port; // a manual port forward must change
                }
                st.set.port = port;
                st.set.clone()
            };
            i.save(&saved);
        }
        let lan_ip = i.primary();
        {
            let mut st = i.lock();
            st.epoch += 1;
            let epoch = st.epoch;
            st.run = Some(Running {
                port: port as u16,
                dual,
                lan_ip,
                relay: Arc::new(relay),
                mapper: None,
                awake: None,
                awake_err: String::new(),
                epoch,
            });
            st.err.clear();
        }
        if let Some(f) = &i.cfg.on_relay {
            let at = match i.cfg.bind_ip {
                Some(ip) if !ip.is_unspecified() => Some(ip),
                _ => lan_ip.map(IpAddr::V4),
            };
            f(Some(RelayUp {
                port: port as u16,
                lan_ip: at,
            }));
        }
        i.log(&format!("home: the server runs on port {port}"));
        Ok(())
    }

    /// Starts or stops port mapping to match "Reachable from outside home".
    /// A manual public address needs no mapping.
    async fn sync_mapper(&self) {
        let i = &self.inner;
        let (want, has, port, set, moved) = {
            let st = i.lock();
            let Some(r) = st.run.as_ref() else { return };
            (
                st.set.outside_on() && st.set.public.is_empty(),
                r.mapper.is_some(),
                r.port,
                st.set.clone(),
                st.moved,
            )
        };
        if want == has {
            return;
        }
        if !want {
            let m = i.lock().run.as_mut().and_then(|r| r.mapper.take());
            if let Some(m) = m {
                m.close().await; // removes the mappings
            }
            return;
        }
        let ext = |v: u16| if v == 0 { port } else { v };
        // What an earlier run mapped may still be on the router: a crash or
        // a kill skips the removal, and a permanent lease stays forever.
        let mut stale = Vec::new();
        for (proto, e) in [("tcp", set.tcp_external), ("udp", set.udp_external)] {
            if e != 0 {
                stale.push(Port {
                    proto,
                    internal: 0,
                    external: e,
                });
            }
        }
        if moved != 0 {
            for proto in ["tcp", "udp"] {
                stale.push(Port {
                    proto,
                    internal: 0,
                    external: moved as u16,
                });
            }
        }
        let mut cfg = portmap::Config::new(vec![
            Port {
                proto: "tcp",
                internal: port,
                external: ext(set.tcp_external),
            },
            Port {
                proto: "udp",
                internal: port,
                external: ext(set.udp_external),
            },
        ]);
        cfg.discover = i.cfg.discover.clone();
        cfg.stale = stale;
        cfg.timing = i.cfg.portmap_timing;
        let kick = i.kick.clone();
        cfg.on_change = Some(Arc::new(move |_| kick.notify_one()));
        if let Some(l) = &i.cfg.logf {
            let l = l.clone();
            cfg.logf = Some(Arc::new(move |m: &str| l(m)));
        }
        let m = Arc::new(Mapper::start(cfg));
        let unused = {
            let mut st = i.lock();
            match st.run.as_mut() {
                Some(r) if r.mapper.is_none() => {
                    r.mapper = Some(m);
                    None
                }
                _ => Some(m),
            }
        };
        if let Some(m) = unused {
            m.close().await;
        }
    }

    /// Takes or releases the hold to match the setting while the server
    /// runs (stop releases it). A hold that can't be taken leaves the server
    /// running, with the reason in Status, and is tried again at the next
    /// check.
    fn sync_awake(&self) {
        let i = &self.inner;
        let (want, epoch) = {
            let mut st = i.lock();
            let want = st.set.awake_on();
            let Some(r) = st.run.as_mut() else { return };
            if want == r.awake.is_some() {
                return;
            }
            if !want {
                let hold = r.awake.take();
                r.awake_err.clear();
                drop(st);
                if let Some(h) = hold {
                    h.release();
                }
                return;
            }
            (want, r.epoch)
        };
        debug_assert!(want);
        let holder = i.cfg.awake.clone().unwrap_or_else(sys::awake::system);
        let got = holder.acquire();
        let mut st = i.lock();
        match got {
            Err(e) => {
                if let Some(r) = st.run.as_mut() {
                    if r.awake_err.is_empty() {
                        i.log(&format!("home: can't keep this PC awake: {e}"));
                    }
                    r.awake_err = e.to_string();
                }
            }
            Ok(hold) => match st.run.as_mut() {
                Some(r) if r.epoch == epoch => {
                    r.awake = Some(hold);
                    r.awake_err.clear();
                    i.log("home: keeping this PC awake while it's the server");
                }
                _ => {
                    drop(st);
                    hold.release(); // stopped meanwhile
                }
            },
        }
    }

    /// Closes everything start opened and removes the port mappings.
    pub async fn stop(&self) {
        let _op = self.inner.op.lock().await;
        self.stop_locked().await;
    }

    async fn stop_locked(&self) {
        let i = &self.inner;
        let Some(mut r) = i.lock().run.take() else {
            return;
        };
        if let Some(f) = &i.cfg.on_relay {
            f(None);
        }
        if let Some(h) = r.awake.take() {
            h.release();
        }
        if let Some(m) = r.mapper.take() {
            m.close().await;
        }
        r.relay.close().await;
    }

    /// The `node.addr` row the server's state gives now; `None` while the
    /// router is still being asked, so a start doesn't write an "unmapped"
    /// row only to replace it a moment later.
    fn addr(&self) -> Option<NodeAddr> {
        let i = &self.inner;
        let (port, set, mst) = {
            let st = i.lock();
            let r = st.run.as_ref()?;
            (
                r.port,
                st.set.clone(),
                r.mapper.as_ref().map(|m| m.status()),
            )
        };
        let mut a = NodeAddr {
            v: 1,
            port: i64::from(port),
            reach: REACH_UNMAPPED.into(),
            at: i.now_ms(),
            ..Default::default()
        };
        a.lan = i.lan_addrs().iter().map(|ip| ip.to_string()).collect();
        if set.outside_on() {
            // Global IPv6 addresses are dialable from outside: listed (and
            // listened on) only when the person asked for that.
            a.v6 = i.v6_addrs().iter().map(|ip| ip.to_string()).collect();
        }
        if !set.public.is_empty() {
            let (host, p) =
                crate::hostport::split_public(&set.public, i64::from(port)).unwrap_or_default();
            a.public = Some(PublicAddr {
                ip: host,
                tcp: p,
                udp: p,
            });
            a.via = "manual".into();
            a.reach = REACH_OK.into();
        } else if let Some(mst) = mst {
            match mst.state {
                portmap::STATE_OK => {
                    let mut pubaddr = PublicAddr {
                        ip: mst.external.map(|e| e.to_string()).unwrap_or_default(),
                        ..Default::default()
                    };
                    for p in &mst.ports {
                        if p.proto == "tcp" {
                            pubaddr.tcp = i64::from(p.external);
                        } else {
                            pubaddr.udp = i64::from(p.external);
                        }
                    }
                    self.keep_external(pubaddr.tcp as u16, pubaddr.udp as u16);
                    a.public = Some(pubaddr);
                    a.via = mst.method.clone();
                    a.reach = if mst.cgnat { REACH_CGNAT } else { REACH_OK }.into();
                }
                portmap::STATE_FAILED => {}
                _ => return None,
            }
        }
        Some(a)
    }

    /// Saves the router ports granted, to ask for them again.
    fn keep_external(&self, tcp: u16, udp: u16) {
        let saved = {
            let mut st = self.inner.lock();
            if st.set.tcp_external == tcp && st.set.udp_external == udp {
                return;
            }
            st.set.tcp_external = tcp;
            st.set.udp_external = udp;
            st.set.clone()
        };
        self.inner.save(&saved);
    }

    /// This device's newest `node.addr`: the one it wrote this run, or the
    /// one in the log from an earlier run.
    fn last_row(&self) -> Option<NodeAddr> {
        let i = &self.inner;
        if let Some(a) = i.lock().last.clone() {
            return Some(a);
        }
        let rows = i
            .cfg
            .store
            .list(&store::Filter {
                kinds: vec![store::kinds::NODE_ADDR.into()],
                device_id: i.self_id(),
                limit: 1,
                desc: true,
                ..Default::default()
            })
            .ok()?;
        let a = NodeAddr::parse(&rows.first()?.payload).unwrap_or_default();
        i.lock().last = Some(a.clone());
        Some(a)
    }

    fn write(&self, mut a: NodeAddr) {
        let i = &self.inner;
        if a.at == 0 {
            a.at = i.now_ms();
        }
        if let Err(e) = i.cfg.store.append("irori", store::kinds::NODE_ADDR, &a) {
            i.log(&format!("home: write node.addr: {e}"));
            return;
        }
        i.lock().last = Some(a);
    }

    /// The server for Devices (Go `Status`).
    pub fn status(&self) -> Status {
        let i = &self.inner;
        let (set, run, err, moved) = {
            let st = i.lock();
            let run = st.run.as_ref().map(|r| {
                (
                    r.port,
                    r.lan_ip,
                    r.mapper.as_ref().map(|m| m.status()),
                    if st.set.awake_on() {
                        awake_status(r)
                    } else {
                        String::new()
                    },
                )
            });
            (st.set.clone(), run, st.err.clone(), st.moved)
        };
        let mut s = Status {
            on: set.on,
            outside: set.outside_on(),
            public: set.public.clone(),
            running: run.is_some(),
            port: set.port,
            awake: set.awake_on(),
            ..Default::default()
        };
        if !set.on {
            return s;
        }
        let Some((port, lan_ip, mst, awake_line)) = run else {
            if !err.is_empty() {
                s.line = format!("The home server couldn't start: {err}");
            }
            return s;
        };
        s.awake_line = awake_line;
        let lan = lan_ip.map_or_else(|| "this PC".to_string(), |ip| ip.to_string());
        s.address = self.addr().map(|a| a.outside()).unwrap_or_default();
        let (ok, failed, shared) = match &mst {
            Some(m) => (
                m.state == portmap::STATE_OK,
                m.state == portmap::STATE_FAILED,
                m.state == portmap::STATE_OK && m.cgnat,
            ),
            None => (false, false, false),
        };
        if !set.public.is_empty() {
            s.reach = REACH_OUTSIDE.into();
            s.line = "Irori can't test the address you entered from inside your home. If your public address changes, it goes stale.".into();
        } else if !set.outside_on() {
            s.reach = REACH_HOME.into();
        } else if shared {
            s.reach = REACH_SHARED.into();
            s.line = "Your internet provider shares your public address (CGNAT), so devices away from home can't reach this PC, and a port forward won't help.".into();
        } else if ok {
            s.reach = REACH_OUTSIDE.into();
        } else if failed {
            s.reach = REACH_FORWARD.into();
            s.line = format!(
                "Away from home your devices can't reach this PC. Turn on UPnP on your router, or forward TCP and UDP port {port} to {lan}."
            );
        } else {
            s.reach = REACH_HOME.into();
        }
        if moved != 0 && moved != i64::from(port) {
            s.line = format!(
                "Port {moved} was taken, so the server moved to port {port}. If you forwarded port {moved} on your router, forward {port} instead."
            );
        }
        if let Some(ip) = lan_ip
            && i.on_public(IpAddr::V4(ip))
        {
            s.line = "Windows has this network as Public, so it keeps your other devices from reaching this PC. In Windows Settings, open Network & internet and set this network to Private.".into();
        }
        s
    }
}

/// The Devices line for the hold.
fn awake_status(r: &Running) -> String {
    if r.awake.is_some() {
        "Keeping this PC awake".into()
    } else if !r.awake_err.is_empty() {
        format!("Can't keep this PC awake: {}", r.awake_err)
    } else {
        String::new()
    }
}
