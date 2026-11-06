//! `spike-call`: does a voice call, and a screen share, fit on iroh between the
//! real devices? (#571, #653)
//!
//!   spike-call listen [--home-relay-port P | --home-relay https://HOST:P/ --relay-pub <id>]
//!   spike-call dial <endpointId> [--secs N] [--size BYTES] [--relay-only] [--label TEXT]
//!                   [--home-relay https://HOST:P/ [--relay-pub <id>]]
//!                   [--share [--kbps 2500] [--fps 10] [--gop-secs 2] [--key-kb 200]]
//!
//! Voice (the default): one QUIC datagram every 20 ms (an Opus frame is
//! ~80-160 bytes at 32-64 kbit/s). The listener reflects each datagram at once;
//! the dialer measures round trip, loss and jitter per second, and which path
//! iroh is on (relay or direct). One-way delay is about RTT/2.
//!
//! Share (`--share`): a 1080p screen share's shape. Frames at `--fps`, one QUIC
//! stream each (a lost packet stalls only its own frame), sized so the average
//! is `--kbps`, with a keyframe of `--key-kb` every `--gop-secs`. The listener
//! answers each whole frame with an 8-byte ack; the dialer times send start to
//! ack. Like an encoder, it skips a frame when 2 s of frames are still in
//! flight. The voice datagrams keep running beside it, so the summary also
//! shows what the share does to voice on the same connection.
//!
//! Relays: by default n0's four public relays as Irori uses them
//! (irori_net::public, decision 43; no preset, no address lookup), so a peer
//! is found through the relays by its endpoint id alone. `listen --home-relay-port P` also runs an
//! embedded Irori home relay (`irori_net::relay`, the home server's code, open
//! to any client) on 0.0.0.0:P with its own pinned key, and homes the listener
//! on it; `dial --home-relay https://HOST:P/` goes through it. A listener on
//! another machine can home on that relay too (`listen --home-relay URL
//! --relay-pub <relay key>`), so either end can be the sharer. Throwaway keys
//! either way: it never touches an Irori data dir or the live app's keys.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use iroh::endpoint::{Builder, Connection};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl, SecretKey};
use iroh_relay::server::{Access, AccessControl, ClientRequest};
use irori_net::endpoint::{self as irori_ep, EndpointOpts, IpMode, RelaySpec};
use irori_net::identity::{DeviceIdentity, IrohKey as _};
use irori_net::path::{self, PathKind};
use irori_net::relay;

const ALPN: &[u8] = b"irori/spike-call/1";
const TICK: Duration = Duration::from_millis(20);
/// Frame header: seq (u64 BE) and a key flag.
const HDR: usize = 9;

#[tokio::main]
async fn main() -> Result<()> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match argv.first().map(String::as_str) {
        Some("listen") => listen(&argv[1..]).await,
        Some("dial") => dial(&argv[1..]).await,
        _ => {
            eprintln!(
                "usage:\n  spike-call listen [--home-relay-port P | --home-relay https://HOST:P/ --relay-pub <id>]\n  spike-call dial <endpointId> [--secs N] [--size BYTES] [--relay-only] [--label TEXT]\n                  [--home-relay https://HOST:P/ [--relay-pub <id>]]\n                  [--share [--kbps 2500] [--fps 10] [--gop-secs 2] [--key-kb 200]]"
            );
            std::process::exit(2)
        }
    }
}

/// The spike's home relay admits anyone: its keys are throwaway and it runs
/// only for the length of a test.
#[derive(Debug, Clone)]
struct OpenAccess;

impl AccessControl for OpenAccess {
    async fn on_connect(&self, _request: &ClientRequest) -> Access {
        Access::Allow
    }
}

fn throwaway_identity() -> DeviceIdentity {
    DeviceIdentity::from_seed_for_tests(SecretKey::generate().to_bytes())
}

async fn online(ep: &Endpoint) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(20), ep.online())
        .await
        .map_err(|_| anyhow!("no relay answered within 20 s"))
}

/// n0's public relays as Irori uses them (irori_net::public, decision 43):
/// no preset and no address lookup.
async fn bind_n0(relay_only: bool) -> Result<Endpoint> {
    let mut b = Builder::empty()
        .crypto_provider(irori_net::tls::provider())
        .secret_key(SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(iroh::RelayMode::Custom(iroh::RelayMap::from_iter(
            irori_net::public::configs(),
        )));
    if relay_only {
        b = b.clear_ip_transports();
    }
    let ep = b.bind().await.context("bind")?;
    online(&ep).await?;
    Ok(ep)
}

/// Irori's own endpoint construction with one pinned home relay.
async fn bind_home(
    id: &DeviceIdentity,
    url: RelayUrl,
    relay_pub: EndpointId,
    relay_only: bool,
) -> Result<Endpoint> {
    let port = url
        .port()
        .ok_or_else(|| anyhow!("--home-relay needs an explicit port"))?;
    let mut o = EndpointOpts::new(if relay_only {
        IpMode::RelayOnly
    } else {
        IpMode::AnyV4
    });
    o.alpns = vec![ALPN.to_vec()];
    o.relays = vec![RelaySpec {
        url,
        quic_port: port,
        server_pub: relay_pub,
    }];
    let ep = irori_ep::bind(id, o).await?;
    online(&ep).await?;
    Ok(ep)
}

async fn listen(args: &[String]) -> Result<()> {
    let mut home_port: Option<u16> = None;
    let mut remote: Option<RelayUrl> = None;
    let mut remote_pub: Option<EndpointId> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--home-relay-port" => {
                home_port = Some(it.next().context("--home-relay-port")?.parse()?)
            }
            "--home-relay" => {
                remote = Some(
                    it.next()
                        .context("--home-relay")?
                        .parse()
                        .map_err(|e| anyhow!("--home-relay: {e}"))?,
                )
            }
            "--relay-pub" => {
                remote_pub = Some(
                    it.next()
                        .context("--relay-pub")?
                        .parse()
                        .map_err(|e| anyhow!("--relay-pub: {e}"))?,
                )
            }
            other => bail!("unknown argument {other}"),
        }
    }
    // Kept alive for the life of the listener.
    let mut _relay = None;
    let ep = match (home_port, &remote) {
        (Some(_), Some(_)) => bail!("give --home-relay-port or --home-relay, not both"),
        (None, Some(url)) => {
            // Home on another machine's spike home relay.
            let pubk = remote_pub.context("--home-relay on listen needs --relay-pub")?;
            bind_home(&throwaway_identity(), url.clone(), pubk, false).await?
        }
        (None, None) => bind_n0(false).await?,
        (Some(p), None) => {
            let id = throwaway_identity();
            let r = relay::spawn_with(&id, IpAddr::V4(Ipv4Addr::UNSPECIFIED), p, OpenAccess)
                .await
                .context("home relay")?;
            println!(
                "listen   home relay on 0.0.0.0:{} (TCP and UDP), key {}",
                r.port,
                id.endpoint_id()
            );
            let local = relay::relay_url(IpAddr::V4(Ipv4Addr::LOCALHOST), r.port);
            let ep = bind_home(&id, local, id.endpoint_id(), false).await?;
            _relay = Some(r);
            ep
        }
    };
    let addr = ep.addr();
    println!("listen   endpoint {}", ep.id());
    println!(
        "listen   relay {}",
        addr.relay_urls()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    );
    match (home_port, &remote) {
        (Some(p), _) => println!(
            "listen   dial with: spike-call dial {} --home-relay https://<this machine's IP>:{p}/",
            ep.id()
        ),
        (None, Some(url)) => println!(
            "listen   dial with: spike-call dial {} --home-relay {url} --relay-pub {}",
            ep.id(),
            remote_pub.map(|k| k.to_string()).unwrap_or_default()
        ),
        (None, None) => println!("listen   dial with: spike-call dial {}", ep.id()),
    }
    while let Some(inc) = ep.accept().await {
        tokio::spawn(async move {
            let conn = match inc.await {
                Ok(c) => c,
                Err(e) => {
                    println!("listen   accept failed: {e:#}");
                    return;
                }
            };
            let rid = conn.remote_id();
            println!("listen   conn from {}", rid.fmt_short());
            tokio::spawn(ack_frames(conn.clone()));
            let t0 = Instant::now();
            let mut n = 0u64;
            while let Ok(d) = conn.read_datagram().await {
                if conn.send_datagram(d).is_err() {
                    break;
                }
                n += 1;
                if n.is_multiple_of(250) {
                    println!(
                        "listen   {} t={:>4}s reflected={n} {}",
                        rid.fmt_short(),
                        t0.elapsed().as_secs(),
                        path::report(&conn)
                    );
                }
            }
            println!("listen   {} gone after {n} datagrams", rid.fmt_short());
        });
    }
    Ok(())
}

/// Share frames: read each stream to its end, answer with its 8-byte seq.
async fn ack_frames(conn: Connection) {
    let mut frames = 0u64;
    let bytes = Arc::new(AtomicUsize::new(0));
    let t0 = Instant::now();
    while let Ok((mut send, mut recv)) = conn.accept_bi().await {
        frames += 1;
        if frames.is_multiple_of(100) {
            println!(
                "listen   {} t={:>4}s frames={frames} ~{:.2} Mbps {}",
                conn.remote_id().fmt_short(),
                t0.elapsed().as_secs(),
                bytes.load(Ordering::Relaxed) as f64 * 8.0 / t0.elapsed().as_secs_f64() / 1e6,
                path::report(&conn)
            );
        }
        let bytes = bytes.clone();
        tokio::spawn(async move {
            if let Ok(buf) = recv.read_to_end(8 << 20).await
                && buf.len() >= HDR
            {
                bytes.fetch_add(buf.len(), Ordering::Relaxed);
                let _ = send.write_all(&buf[..8]).await;
                let _ = send.finish();
            }
        });
    }
}

struct Opts {
    to: EndpointId,
    secs: u64,
    size: usize,
    relay_only: bool,
    label: String,
    home_relay: Option<RelayUrl>,
    relay_pub: Option<EndpointId>,
    share: bool,
    kbps: u64,
    fps: u64,
    gop_secs: u64,
    key_kb: u64,
}

fn parse(args: &[String]) -> Result<Opts> {
    let mut it = args.iter();
    let to = it
        .next()
        .ok_or_else(|| anyhow!("dial needs an endpoint id"))?
        .parse::<EndpointId>()
        .map_err(|e| anyhow!("endpoint id: {e}"))?;
    let mut o = Opts {
        to,
        secs: 60,
        size: 160,
        relay_only: false,
        label: String::new(),
        home_relay: None,
        relay_pub: None,
        share: false,
        kbps: 2500,
        fps: 10,
        gop_secs: 2,
        key_kb: 200,
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "--secs" => o.secs = it.next().context("--secs")?.parse()?,
            "--size" => o.size = it.next().context("--size")?.parse()?,
            "--relay-only" => o.relay_only = true,
            "--label" => o.label = it.next().context("--label")?.clone(),
            "--home-relay" => {
                o.home_relay = Some(
                    it.next()
                        .context("--home-relay")?
                        .parse()
                        .map_err(|e| anyhow!("--home-relay: {e}"))?,
                )
            }
            "--relay-pub" => {
                o.relay_pub = Some(
                    it.next()
                        .context("--relay-pub")?
                        .parse()
                        .map_err(|e| anyhow!("--relay-pub: {e}"))?,
                )
            }
            "--share" => o.share = true,
            "--kbps" => o.kbps = it.next().context("--kbps")?.parse()?,
            "--fps" => o.fps = it.next().context("--fps")?.parse()?,
            "--gop-secs" => o.gop_secs = it.next().context("--gop-secs")?.parse()?,
            "--key-kb" => o.key_kb = it.next().context("--key-kb")?.parse()?,
            other => bail!("unknown argument {other}"),
        }
    }
    if o.size < 16 {
        bail!("--size must be at least 16");
    }
    if o.fps == 0 || o.gop_secs == 0 {
        bail!("--fps and --gop-secs must be at least 1");
    }
    Ok(o)
}

#[derive(Default)]
struct Window {
    sent: u64,
    got: u64,
    rtts: Vec<f64>,
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[i]
}

/// Lost packets over the whole connection (every path, closed ones too),
/// congestion events summed over the open paths, and the selected path's cwnd.
fn path_counters(conn: &Connection) -> (u64, u64, u64) {
    let lost = conn.stats().lost_packets;
    let (mut cong, mut cwnd) = (0, 0);
    for p in conn.paths().iter() {
        let s = p.stats();
        cong += s.congestion_events;
        if p.is_selected() {
            cwnd = s.cwnd;
        }
    }
    (lost, cong, cwnd)
}

/// Voice datagram bookkeeping, shared by both modes.
struct Voice {
    sent_at: HashMap<u64, Instant>,
    win: Window,
    all: Vec<f64>,
    per_kind: HashMap<&'static str, Vec<f64>>,
    sent: u64,
    got: u64,
    late: u64,
    jitter: f64,
    last: Option<f64>,
}

impl Voice {
    fn new() -> Self {
        Self {
            sent_at: HashMap::new(),
            win: Window::default(),
            all: vec![],
            per_kind: HashMap::new(),
            sent: 0,
            got: 0,
            late: 0,
            jitter: 0.0,
            last: None,
        }
    }
    fn send(&mut self, conn: &Connection, buf: &mut [u8], seq: u64) {
        buf[..8].copy_from_slice(&seq.to_be_bytes());
        if conn.send_datagram(buf.to_vec().into()).is_ok() {
            self.sent_at.insert(seq, Instant::now());
            self.sent += 1;
            self.win.sent += 1;
        }
    }
    fn recv(&mut self, d: &[u8], kind: &'static str) {
        if d.len() < 8 {
            return;
        }
        let s = u64::from_be_bytes(d[..8].try_into().unwrap());
        if let Some(at) = self.sent_at.remove(&s) {
            let rtt = at.elapsed().as_secs_f64() * 1000.0;
            // A frame back after 300 ms RTT is useless to a jitter buffer.
            if rtt > 300.0 {
                self.late += 1;
            }
            self.got += 1;
            self.win.got += 1;
            self.win.rtts.push(rtt);
            self.all.push(rtt);
            self.per_kind.entry(kind).or_default().push(rtt);
            if let Some(prev) = self.last {
                self.jitter += ((rtt - prev).abs() - self.jitter) / 16.0;
            }
            self.last = Some(rtt);
        }
    }
    /// (p50, p95, max, loss%) of the last second, and resets the window.
    fn second(&mut self) -> (f64, f64, f64, f64) {
        let mut v = std::mem::take(&mut self.win.rtts);
        let loss = if self.win.sent > 0 {
            100.0 * (1.0 - self.win.got as f64 / self.win.sent as f64)
        } else {
            0.0
        };
        self.win = Window::default();
        let (p50, p95) = (pct(&mut v, 0.5), pct(&mut v, 0.95));
        (
            p50,
            p95,
            v.last().copied().unwrap_or(f64::NAN),
            loss.max(0.0),
        )
    }
    fn summary(&mut self) -> String {
        let lost = self.sent.saturating_sub(self.got);
        let mut a = self.all.clone();
        let mut kinds: Vec<String> = self
            .per_kind
            .iter_mut()
            .map(|(k, v)| format!("{k}:n={} p50={:.1}ms", v.len(), pct(v, 0.5)))
            .collect();
        kinds.sort();
        format!(
            "sent={} lost={lost} ({:.2}%) late>300ms={} rtt p50={:.1} p95={:.1} p99={:.1} max={:.1}ms jitter={:.1}ms one_way~{:.0}ms paths=[{}]",
            self.sent,
            100.0 * lost as f64 / self.sent.max(1) as f64,
            self.late,
            pct(&mut a, 0.5),
            pct(&mut a, 0.95),
            pct(&mut a, 0.99),
            a.last().copied().unwrap_or(f64::NAN),
            self.jitter,
            pct(&mut a, 0.5) / 2.0,
            kinds.join(" "),
        )
    }
}

fn kind_of(conn: &Connection) -> (&'static str, path::PathReport) {
    let r = path::report(conn);
    let k = match r.kind {
        PathKind::Relay => "relay",
        PathKind::Direct => "direct",
        PathKind::None => "none",
    };
    (k, r)
}

async fn dial(args: &[String]) -> Result<()> {
    let o = parse(args)?;
    let (ep, dest, relay_name) = match &o.home_relay {
        None => (
            bind_n0(o.relay_only).await?,
            irori_net::public::with_public(EndpointAddr::new(o.to), true),
            "n0",
        ),
        Some(url) => {
            let id = throwaway_identity();
            let pubk = o.relay_pub.unwrap_or(o.to);
            let ep = bind_home(&id, url.clone(), pubk, o.relay_only).await?;
            (
                ep,
                EndpointAddr::new(o.to).with_relay_url(url.clone()),
                "home",
            )
        }
    };
    println!(
        "dial     me {} -> {} ({}, {} relay{}) label={:?}",
        ep.id().fmt_short(),
        o.to.fmt_short(),
        if o.relay_only {
            "relay only"
        } else {
            "direct allowed"
        },
        relay_name,
        if o.share { ", share" } else { "" },
        o.label
    );
    let t0 = Instant::now();
    let conn = tokio::time::timeout(Duration::from_secs(30), ep.connect(dest, ALPN))
        .await
        .map_err(|_| anyhow!("connect timed out after 30 s"))?
        .context("connect")?;
    let connect_ms = t0.elapsed().as_secs_f64() * 1000.0;
    println!(
        "dial     connected in {connect_ms:.0}ms {}",
        path::report(&conn)
    );
    if let Some(max) = conn.max_datagram_size() {
        println!("dial     max datagram {max} bytes");
        if max < o.size {
            bail!("datagram {} > max {max}", o.size);
        }
    }
    let mode = format!(
        "{}{}",
        if o.relay_only {
            "relay-only"
        } else {
            "direct-allowed"
        },
        if o.home_relay.is_some() {
            "/home-relay"
        } else {
            "/n0"
        }
    );
    if o.share {
        share(&o, &conn, &mode, connect_ms).await?;
    } else {
        voice(&o, &conn, &mode, connect_ms).await?;
    }
    conn.close(0u32.into(), b"done");
    ep.close().await;
    Ok(())
}

async fn voice(o: &Opts, conn: &Connection, mode: &str, connect_ms: f64) -> Result<()> {
    let start = Instant::now();
    let total = o.secs * 50;
    let mut v = Voice::new();
    let mut first_direct: Option<f64> = None;
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut sec = tokio::time::interval(Duration::from_secs(1));
    sec.tick().await;
    let mut seq = 0u64;
    let mut buf = vec![0u8; o.size];
    let mut kind_now: &'static str = "none";
    let drain_until = start + Duration::from_secs(o.secs) + Duration::from_secs(2);

    loop {
        tokio::select! {
            _ = tick.tick(), if seq < total => {
                v.send(conn, &mut buf, seq);
                seq += 1;
            }
            d = conn.read_datagram() => {
                let d = d.context("read datagram")?;
                v.recv(&d, kind_now);
            }
            _ = sec.tick() => {
                let (k, r) = kind_of(conn);
                kind_now = k;
                if r.kind == PathKind::Direct && first_direct.is_none() {
                    first_direct = Some(start.elapsed().as_secs_f64());
                }
                let (p50, p95, max, loss) = v.second();
                println!(
                    "t={:>3}s path={:<6} rtt p50={:>6.1} p95={:>6.1} max={:>6.1}ms loss={:>5.1}% jitter={:>5.1}ms via={}",
                    start.elapsed().as_secs(), kind_now, p50, p95, max, loss, v.jitter, r.remote,
                );
                if seq >= total && Instant::now() > drain_until { break; }
            }
        }
    }
    println!(
        "SUMMARY label={:?} mode={mode} connect={connect_ms:.0}ms first_direct={} {}",
        o.label,
        first_direct
            .map(|s| format!("{s:.0}s"))
            .unwrap_or_else(|| "never".into()),
        v.summary(),
    );
    Ok(())
}

struct FrameDone {
    key: bool,
    size: usize,
    sent: Instant,
    ms: Option<f64>,
}

async fn send_frame(
    conn: Connection,
    payload: Arc<Vec<u8>>,
    seq: u64,
    key: bool,
    size: usize,
) -> Option<f64> {
    let t = Instant::now();
    let (mut send, mut recv) = conn.open_bi().await.ok()?;
    let mut hdr = [0u8; HDR];
    hdr[..8].copy_from_slice(&seq.to_be_bytes());
    hdr[8] = key as u8;
    send.write_all(&hdr).await.ok()?;
    send.write_all(&payload[..size - HDR]).await.ok()?;
    send.finish().ok()?;
    let ack = recv.read_to_end(64).await.ok()?;
    if ack.len() != 8 || ack[..] != seq.to_be_bytes() {
        return None;
    }
    Some(t.elapsed().as_secs_f64() * 1000.0)
}

async fn share(o: &Opts, conn: &Connection, mode: &str, connect_ms: f64) -> Result<()> {
    let gop = o.fps * o.gop_secs;
    let gop_bytes = o.kbps * 125 * o.gop_secs;
    let key_bytes = (o.key_kb * 1000) as usize;
    if gop < 2 || gop_bytes <= key_bytes as u64 {
        bail!(
            "--key-kb must be smaller than one GOP's bytes ({gop_bytes}) and a GOP at least 2 frames"
        );
    }
    let delta_bytes = ((gop_bytes - key_bytes as u64) / (gop - 1)) as usize;
    let (key_bytes, delta_bytes) = (key_bytes.max(HDR), delta_bytes.max(HDR));
    println!(
        "share    {} kbps at {} fps: keyframe {} KB every {} s, other frames {:.1} KB",
        o.kbps,
        o.fps,
        key_bytes / 1000,
        o.gop_secs,
        delta_bytes as f64 / 1000.0
    );
    let payload = Arc::new(vec![0x5au8; key_bytes.max(delta_bytes)]);
    let frames = o.secs * o.fps;
    let max_inflight = (o.fps * 2) as usize;
    let inflight = Arc::new(AtomicUsize::new(0));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<FrameDone>();

    let start = Instant::now();
    let mut v = Voice::new();
    let mut vbuf = vec![0u8; o.size];
    let mut vseq = 0u64;
    let vtotal = o.secs * 50;
    let mut vtick = tokio::time::interval(TICK);
    vtick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut ftick = tokio::time::interval(Duration::from_secs_f64(1.0 / o.fps as f64));
    ftick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut sec = tokio::time::interval(Duration::from_secs(1));
    sec.tick().await;
    let mut kind_now: &'static str = "none";

    let mut fseq = 0u64;
    let (mut f_sent, mut f_acked, mut f_failed, mut f_skipped) = (0u64, 0u64, 0u64, 0u64);
    let (mut keys_skipped, mut late500, mut late1000) = (0u64, 0u64, 0u64);
    let (mut key_ms, mut delta_ms): (Vec<f64>, Vec<f64>) = (vec![], vec![]);
    let mut acked_bytes = 0u64;
    let mut sec_bytes = 0u64;
    let mut sec_ms: Vec<f64> = vec![];
    let mut last_ack: Option<Instant> = None;
    let mut max_gap = 0.0f64;
    let mut path_rtts: Vec<f64> = vec![];
    let mut first_direct: Option<f64> = None;
    let mut need_key = false;
    let hard_stop = start + Duration::from_secs(o.secs) + Duration::from_secs(15);
    let (mut prev_lost, mut prev_cong) = (0u64, 0u64);
    // One finished frame into the totals. A failed frame counts as late by its age.
    let mut record = |f: &FrameDone| match f.ms {
        None => {
            f_failed += 1;
            let age = f.sent.elapsed().as_secs_f64() * 1000.0;
            if age > 500.0 {
                late500 += 1;
            }
            if age > 1000.0 {
                late1000 += 1;
            }
        }
        Some(ms) => {
            f_acked += 1;
            acked_bytes += f.size as u64;
            if ms > 500.0 {
                late500 += 1;
            }
            if ms > 1000.0 {
                late1000 += 1;
            }
            if f.key {
                key_ms.push(ms)
            } else {
                delta_ms.push(ms)
            }
            let now = f.sent + Duration::from_secs_f64(ms / 1000.0);
            // The wait before the first ack counts as a gap too.
            let prev = last_ack.unwrap_or(start);
            max_gap = max_gap.max(now.saturating_duration_since(prev).as_secs_f64() * 1000.0);
            last_ack = Some(last_ack.map_or(now, |p| p.max(now)));
        }
    };

    loop {
        tokio::select! {
            _ = vtick.tick(), if vseq < vtotal => {
                v.send(conn, &mut vbuf, vseq);
                vseq += 1;
            }
            d = conn.read_datagram() => {
                let d = d.context("read datagram")?;
                v.recv(&d, kind_now);
            }
            _ = ftick.tick(), if fseq < frames => {
                // A real encoder sends a keyframe after it had to skip one.
                let scheduled = fseq.is_multiple_of(gop);
                let key = scheduled || need_key;
                fseq += 1;
                if inflight.load(Ordering::Relaxed) >= max_inflight {
                    f_skipped += 1;
                    // Keyframes the GOP called for, not each retry of an owed one.
                    if scheduled { keys_skipped += 1; }
                    need_key = need_key || key;
                    continue;
                }
                need_key = false;
                let size = if key { key_bytes } else { delta_bytes };
                inflight.fetch_add(1, Ordering::Relaxed);
                f_sent += 1;
                let (c, p, tx, inf, seq) = (conn.clone(), payload.clone(), tx.clone(), inflight.clone(), fseq - 1);
                tokio::spawn(async move {
                    let sent = Instant::now();
                    let ms = send_frame(c, p, seq, key, size).await;
                    inf.fetch_sub(1, Ordering::Relaxed);
                    let _ = tx.send(FrameDone { key, size, sent, ms });
                });
            }
            Some(f) = rx.recv() => {
                if f.ms.is_some() { sec_bytes += f.size as u64; }
                if let Some(ms) = f.ms { sec_ms.push(ms); }
                record(&f);
            }
            _ = sec.tick() => {
                let (k, r) = kind_of(conn);
                kind_now = k;
                if r.kind == PathKind::Direct && first_direct.is_none() {
                    first_direct = Some(start.elapsed().as_secs_f64());
                }
                if let Some(rtt) = r.rtt { path_rtts.push(rtt.as_secs_f64() * 1000.0); }
                let (vp50, vp95, _, vloss) = v.second();
                let mut m = std::mem::take(&mut sec_ms);
                let (lost, cong, cwnd) = path_counters(conn);
                println!(
                    "t={:>3}s path={:<6} share {:>5.2} Mbps frames={:>2} p50={:>6.1} max={:>6.1}ms inflight={:>2} skipped_total={} | voice p50={:>6.1} p95={:>6.1}ms loss={:>5.1}% | lost_pkts/s={} cong/s={} cwnd={}KB via={}",
                    start.elapsed().as_secs(), kind_now, sec_bytes as f64 * 8.0 / 1e6, m.len(),
                    pct(&mut m, 0.5), m.last().copied().unwrap_or(f64::NAN),
                    inflight.load(Ordering::Relaxed), f_skipped, vp50, vp95, vloss,
                    lost.saturating_sub(prev_lost), cong.saturating_sub(prev_cong), cwnd / 1000, r.remote,
                );
                (prev_lost, prev_cong) = (lost, cong);
                sec_bytes = 0;
                let done_sending = fseq >= frames && vseq >= vtotal;
                if (done_sending && inflight.load(Ordering::Relaxed) == 0
                    && start.elapsed() > Duration::from_secs(o.secs) + Duration::from_secs(2))
                    || Instant::now() > hard_stop
                {
                    break;
                }
            }
        }
    }
    // Frames that finished after the last tick, then frames still stuck at a
    // hard stop: those waited at least 13 s, so they count as late.
    while let Ok(f) = rx.try_recv() {
        record(&f);
    }
    let stuck = inflight.load(Ordering::Relaxed) as u64;
    late500 += stuck;
    late1000 += stuck;
    // Measure to the end of sending at least, so a stall at the end shows in
    // goodput and max_gap instead of vanishing after the last ack.
    let send_end = start + Duration::from_secs(o.secs);
    max_gap = max_gap.max(
        send_end
            .saturating_duration_since(last_ack.unwrap_or(start))
            .as_secs_f64()
            * 1000.0,
    );
    let span = last_ack
        .map_or(send_end, |l| l.max(send_end))
        .duration_since(start)
        .as_secs_f64();
    let (lost, cong, _) = path_counters(conn);
    println!(
        "SUMMARY label={:?} mode={mode}/share connect={connect_ms:.0}ms first_direct={} target={}kbps fps={} key={}KB/{}s frames sent={f_sent} acked={f_acked} failed={f_failed} skipped={f_skipped} (keys {keys_skipped}) still_in_flight={} goodput={:.2}Mbps frame ms delta p50={:.0} p95={:.0} p99={:.0} max={:.0} key p50={:.0} p95={:.0} max={:.0} late>500ms={late500} late>1s={late1000} max_gap={:.0}ms path_rtt p50={:.0}ms lost_pkts={lost} cong_events={cong} | voice {}",
        o.label,
        first_direct
            .map(|s| format!("{s:.0}s"))
            .unwrap_or_else(|| "never".into()),
        o.kbps,
        o.fps,
        key_bytes / 1000,
        o.gop_secs,
        inflight.load(Ordering::Relaxed),
        acked_bytes as f64 * 8.0 / span / 1e6,
        pct(&mut delta_ms, 0.5),
        pct(&mut delta_ms, 0.95),
        pct(&mut delta_ms, 0.99),
        delta_ms.last().copied().unwrap_or(f64::NAN),
        pct(&mut key_ms, 0.5),
        pct(&mut key_ms, 0.95),
        key_ms.last().copied().unwrap_or(f64::NAN),
        max_gap,
        pct(&mut path_rtts, 0.5),
        v.summary(),
    );
    Ok(())
}
