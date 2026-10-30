//! Port mapping for the home server's port N (mesh.md §3.3, §7 trap 16):
//! a port of go-room's `portmap` (R:portmap/portmap.go). It asks the home
//! router to forward TCP N and UDP N to this machine over UPnP-IGD
//! ([`upnp`], through igd-next) or NAT-PMP ([`natpmp`], RFC 6886), keeps the
//! mappings renewed, and removes them on [`Mapper::close`].
//!
//! iroh's own portmapper maps only the endpoint's UDP socket, and
//! `node.addr` needs the external IP, both external ports and the method,
//! so this is separate. Discovery (SSDP for an IGD at the default gateway,
//! then NAT-PMP there) runs only in the app; tests hand [`Config::discover`]
//! their own gateways (fakes in process or on 127.0.0.1).

pub mod natpmp;
pub mod upnp;

use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

/// A boxed future, so [`Gateway`] can be a trait object.
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One mapping: `proto` `"tcp"` or `"udp"`, the port on this machine, and
/// the port on the router's external address (0 = ask for `internal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Port {
    pub proto: &'static str,
    pub internal: u16,
    pub external: u16,
}

/// Why `Map` failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapError {
    /// The external port is taken by another mapping; the mapper then asks
    /// for another one (Go `ErrConflict`).
    Conflict,
    Other(String),
}

impl std::fmt::Display for MapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MapError::Conflict => f.write_str("portmap: external port already mapped"),
            MapError::Other(s) => f.write_str(s),
        }
    }
}

/// One router protocol endpoint (Go `portmap.Gateway`).
pub trait Gateway: Send + Sync {
    /// `"upnp"` or `"natpmp"`.
    fn method(&self) -> &str;
    /// The router's own external address.
    fn external_ip(&self) -> BoxFut<'_, Result<IpAddr, String>>;
    /// Asks for `p` for `life`; returns the mapping as granted (the external
    /// port may differ) and its lifetime (zero = permanent).
    fn map(&self, p: Port, life: Duration) -> BoxFut<'_, Result<(Port, Duration), MapError>>;
    /// Removes a mapping `map` made.
    fn unmap(&self, p: Port) -> BoxFut<'_, Result<(), String>>;
    /// Removes a mapping an earlier run of this app left, after checking it
    /// is this machine's own; `None` when the protocol can't (NAT-PMP
    /// mappings always expire).
    fn unmap_stale(&self, _p: Port) -> Option<BoxFut<'_, Result<(), String>>> {
        None
    }
}

/// Mapping states.
pub const STATE_NONE: &str = "none";
pub const STATE_PENDING: &str = "pending";
pub const STATE_OK: &str = "ok";
pub const STATE_FAILED: &str = "failed";

/// How mapping is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub state: &'static str,
    pub method: String,
    /// The router's external address.
    pub external: Option<IpAddr>,
    /// As granted (`external` may differ from `internal`).
    pub ports: Vec<Port>,
    /// `external` is in 100.64.0.0/10 or private space.
    pub cgnat: bool,
    pub detail: String,
}

impl Status {
    fn of(state: &'static str) -> Status {
        Status {
            state,
            method: String::new(),
            external: None,
            ports: vec![],
            cgnat: false,
            detail: String::new(),
        }
    }

    fn failed(detail: String) -> Status {
        Status {
            detail,
            ..Status::of(STATE_FAILED)
        }
    }
}

/// Whether a router's external address is shared: in 100.64.0.0/10
/// (RFC 6598) or private space (a router behind a router).
pub fn cgnat(a: IpAddr) -> bool {
    let a = match a {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        v4 => v4,
    };
    match a {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            (o[0] == 100 && (o[1] & 0xc0) == 64) || v4.is_private() || v4.is_loopback()
        }
        IpAddr::V6(v6) => v6.segments()[0] & 0xfe00 == 0xfc00 || v6.is_loopback(),
    }
}

/// The timing, so tests run fast (Go's package variables).
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// After nothing could be mapped.
    pub retry_failed: Duration,
    /// A permanent mapping is asked for again this often.
    pub recheck: Duration,
    /// The router's external address is re-read this often.
    pub ip_poll: Duration,
    pub min_renew: Duration,
    pub call_timeout: Duration,
    /// Other external ports tried after a conflict.
    pub conflict_try: u32,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            retry_failed: Duration::from_secs(5 * 60),
            recheck: Duration::from_secs(30 * 60),
            ip_poll: Duration::from_secs(3 * 60),
            min_renew: Duration::from_secs(1),
            call_timeout: Duration::from_secs(10),
            conflict_try: 4,
        }
    }
}

pub type DiscoverFn =
    Arc<dyn Fn() -> BoxFut<'static, Result<Vec<Arc<dyn Gateway>>, String>> + Send + Sync>;
pub type OnChangeFn = Arc<dyn Fn(&Status) + Send + Sync>;
pub type LogFn = Arc<dyn Fn(&str) + Send + Sync>;

/// Sets up a [`Mapper`].
#[derive(Clone)]
pub struct Config {
    /// Ports to map (Irori: TCP N and UDP N).
    pub ports: Vec<Port>,
    /// Finds gateways, tried in order. `None`: [`upnp::discover_default`].
    pub discover: Option<DiscoverFn>,
    /// Lifetime asked for (default 1 h); renewed at half of what was granted.
    pub lifetime: Duration,
    /// The UPnP mapping description (Irori: `"irori"`).
    pub description: String,
    /// Mappings an earlier run may have left on the router: each is removed
    /// from the first gateway mapped on, if it still points at this machine
    /// with this description.
    pub stale: Vec<Port>,
    pub on_change: Option<OnChangeFn>,
    pub logf: Option<LogFn>,
    pub timing: Timing,
}

impl Config {
    pub fn new(ports: Vec<Port>) -> Self {
        Config {
            ports,
            discover: None,
            lifetime: Duration::from_secs(3600),
            description: "irori".into(),
            stale: vec![],
            on_change: None,
            logf: None,
            timing: Timing::default(),
        }
    }
}

struct State {
    status: Status,
    gw: Option<Arc<dyn Gateway>>,
    mapped: Vec<Port>,
    stale_done: bool,
}

struct Inner {
    cfg: Config,
    quit: CancellationToken,
    st: Mutex<State>,
}

/// Keeps the mappings up (Go `portmap.Mapper`).
pub struct Mapper {
    inner: Arc<Inner>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
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

    fn set(&self, s: Status) {
        let changed = {
            let mut st = self.lock();
            let changed = st.status != s;
            st.status = s.clone();
            changed
        };
        if changed && let Some(f) = &self.cfg.on_change {
            f(&s);
        }
    }

    /// Runs one router call with the call timeout, cut short by close.
    async fn call<T>(&self, f: impl Future<Output = Result<T, String>>) -> Result<T, String> {
        tokio::select! {
            _ = self.quit.cancelled() => Err("closed".into()),
            r = tokio::time::timeout(self.cfg.timing.call_timeout, f) => {
                r.unwrap_or_else(|_| Err("timed out".into()))
            }
        }
    }
}

impl Mapper {
    /// Begins mapping in the background (inside a tokio runtime).
    pub fn start(cfg: Config) -> Mapper {
        let inner = Arc::new(Inner {
            cfg,
            quit: CancellationToken::new(),
            st: Mutex::new(State {
                status: Status::of(STATE_PENDING),
                gw: None,
                mapped: vec![],
                stale_done: false,
            }),
        });
        let task = tokio::spawn(run(inner.clone()));
        Mapper {
            inner,
            task: Mutex::new(Some(task)),
        }
    }

    /// The current status.
    pub fn status(&self) -> Status {
        self.inner.lock().status.clone()
    }

    /// Stops renewing and removes the mappings.
    pub async fn close(&self) {
        self.inner.quit.cancel();
        let t = self.task.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(t) = t {
            let _ = t.await;
        }
    }
}

impl Drop for Mapper {
    fn drop(&mut self) {
        self.inner.quit.cancel();
    }
}

async fn run(m: Arc<Inner>) {
    m.set(Status::of(STATE_PENDING));
    'outer: loop {
        if m.quit.is_cancelled() {
            break;
        }
        let wait = cycle(&m).await;
        let next = tokio::time::Instant::now() + wait;
        // Between renewals, the external address is re-read: an ISP can
        // change it at any time, and devices away from home dial it.
        loop {
            let now = tokio::time::Instant::now();
            if now >= next {
                break;
            }
            let d = (next - now).min(m.cfg.timing.ip_poll);
            tokio::select! {
                _ = m.quit.cancelled() => break 'outer,
                _ = tokio::time::sleep(d) => {}
            }
            if tokio::time::Instant::now() < next {
                recheck_ip(&m).await;
            }
        }
    }
    unmap_all(&m).await;
    m.set(Status::of(STATE_NONE));
}

async fn recheck_ip(m: &Inner) {
    let (g, st) = {
        let s = m.lock();
        (s.gw.clone(), s.status.clone())
    };
    let Some(g) = g else { return };
    if st.state != STATE_OK {
        return;
    }
    let Ok(ext) = m.call(g.external_ip()).await else {
        return;
    };
    if Some(ext) == st.external {
        return;
    }
    m.log(&format!(
        "portmap: the router's external address changed to {ext}"
    ));
    let next = {
        let s = m.lock();
        if !s.gw.as_ref().is_some_and(|x| Arc::ptr_eq(x, &g)) {
            return;
        }
        let mut st = s.status.clone();
        st.external = Some(ext);
        st.cgnat = cgnat(ext);
        st
    };
    m.set(next);
}

async fn unmap_stale(m: &Inner, g: &Arc<dyn Gateway>) {
    {
        let mut s = m.lock();
        if s.stale_done {
            return;
        }
        s.stale_done = true;
    }
    for p in m.cfg.stale.iter().filter(|p| p.external != 0) {
        let Some(f) = g.unmap_stale(*p) else { return };
        if let Err(e) = m.call(f).await {
            m.log(&format!(
                "portmap: remove an old {} {}: {e}",
                p.proto, p.external
            ));
        }
    }
}

/// Maps (or renews) everything once and says when to run again.
async fn cycle(m: &Inner) -> Duration {
    let gw = m.lock().gw.clone();
    if let Some(gw) = gw {
        // Renew on the gateway that worked, asking for what was granted.
        match map_on(m, &gw, true).await {
            Ok(wait) => return wait,
            Err(e) => {
                m.log(&format!("portmap: renew on {}: {e}", gw.method()));
                unmap_all(m).await;
            }
        }
    }
    let discover = m
        .cfg
        .discover
        .clone()
        .unwrap_or_else(|| upnp::discover_default(m.cfg.description.clone()));
    let found = m.call(discover()).await;
    let gws = match found {
        Err(e) => {
            m.set(Status::failed(format!("no router answered: {e}")));
            return m.cfg.timing.retry_failed;
        }
        Ok(g) if g.is_empty() => {
            m.set(Status::failed("no router answered UPnP or NAT-PMP".into()));
            return m.cfg.timing.retry_failed;
        }
        Ok(g) => g,
    };
    let mut errs = Vec::new();
    for g in &gws {
        match map_on(m, g, false).await {
            Ok(wait) => return wait,
            Err(e) => {
                errs.push(format!("{}: {e}", g.method()));
                unmap_all(m).await;
            }
        }
    }
    m.set(Status::failed(format!(
        "the router refused: [{}]",
        errs.join(" ")
    )));
    m.cfg.timing.retry_failed
}

/// Maps every port on `g`; on success records `g` and the status.
async fn map_on(m: &Inner, g: &Arc<dyn Gateway>, renew: bool) -> Result<Duration, String> {
    let want = if renew {
        m.lock().mapped.clone()
    } else {
        m.cfg.ports.clone()
    };
    if !renew {
        unmap_stale(m, g).await;
    }
    let mut got = Vec::new();
    let mut shortest = Duration::ZERO;
    for p in want {
        match map_one(m, g, p).await {
            Ok((mp, granted)) => {
                got.push(mp);
                if granted > Duration::ZERO && (shortest.is_zero() || granted < shortest) {
                    shortest = granted;
                }
            }
            Err(e) => {
                if !renew {
                    // So unmap_all removes what this attempt made.
                    let mut s = m.lock();
                    s.gw = Some(g.clone());
                    s.mapped = got;
                }
                return Err(e);
            }
        }
    }
    let ext = m.call(g.external_ip()).await;
    let mut st = Status {
        method: g.method().to_string(),
        ports: got.clone(),
        ..Status::of(STATE_OK)
    };
    match ext {
        Err(e) => st.detail = format!("the router didn't say its external address: {e}"),
        Ok(a) => {
            st.external = Some(a);
            st.cgnat = cgnat(a);
        }
    }
    {
        let mut s = m.lock();
        s.gw = Some(g.clone());
        s.mapped = got;
    }
    m.set(st);
    if shortest.is_zero() {
        return Ok(m.cfg.timing.recheck);
    }
    Ok((shortest / 2).max(m.cfg.timing.min_renew))
}

/// Maps one port, trying other external ports after a conflict.
async fn map_one(m: &Inner, g: &Arc<dyn Gateway>, p: Port) -> Result<(Port, Duration), String> {
    let mut try_p = p;
    let mut i = 0;
    loop {
        let r = tokio::select! {
            _ = m.quit.cancelled() => Err(MapError::Other("closed".into())),
            r = tokio::time::timeout(m.cfg.timing.call_timeout, g.map(try_p, m.cfg.lifetime)) => {
                r.unwrap_or_else(|_| Err(MapError::Other("timed out".into())))
            }
        };
        match r {
            Ok(x) => return Ok(x),
            Err(MapError::Conflict) if i < m.cfg.timing.conflict_try => {
                try_p.external = 20000 + random_below(10000) as u16;
                m.log(&format!(
                    "portmap: {} {} taken on the router; trying {}",
                    p.proto, p.external, try_p.external
                ));
                i += 1;
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn random_below(n: u32) -> u32 {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).expect("getrandom");
    u32::from_le_bytes(b) % n
}

async fn unmap_all(m: &Inner) {
    let (g, ports) = {
        let mut s = m.lock();
        (s.gw.take(), std::mem::take(&mut s.mapped))
    };
    let Some(g) = g else { return };
    for p in ports {
        match tokio::time::timeout(Duration::from_secs(3), g.unmap(p)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => m.log(&format!("portmap: unmap {} {}: {e}", p.proto, p.external)),
            Err(_) => m.log(&format!(
                "portmap: unmap {} {}: timed out",
                p.proto, p.external
            )),
        }
    }
}
