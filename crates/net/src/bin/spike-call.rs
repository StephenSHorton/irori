//! `spike-call`: does a voice call fit on iroh between the real devices? (#571)
//!
//!   spike-call listen
//!   spike-call dial <endpointId> [--secs N] [--size BYTES] [--relay-only] [--label TEXT]
//!
//! Voice-shaped traffic: one QUIC datagram every 20 ms (an Opus frame is
//! ~80-160 bytes at 32-64 kbit/s). The listener reflects each datagram at once;
//! the dialer measures round trip, loss and jitter per second, and which path
//! iroh is on (relay or direct). One-way delay is about RTT/2.
//!
//! Throwaway keys, n0's public relays and n0's address lookup (preset N0), so
//! a peer is found by its endpoint id alone. It never touches an Irori data
//! dir or the live app's keys.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointId, SecretKey};
use irori_net::path::{self, PathKind};

const ALPN: &[u8] = b"irori/spike-call/1";
const TICK: Duration = Duration::from_millis(20);

#[tokio::main]
async fn main() -> Result<()> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match argv.first().map(String::as_str) {
        Some("listen") => listen().await,
        Some("dial") => dial(&argv[1..]).await,
        _ => {
            eprintln!(
                "usage:\n  spike-call listen\n  spike-call dial <endpointId> [--secs N] [--size BYTES] [--relay-only] [--label TEXT]"
            );
            std::process::exit(2)
        }
    }
}

async fn bind(relay_only: bool) -> Result<Endpoint> {
    let mut b = Endpoint::builder(presets::N0)
        .secret_key(SecretKey::generate())
        .alpns(vec![ALPN.to_vec()]);
    if relay_only {
        b = b.clear_ip_transports();
    }
    let ep = b.bind().await.context("bind")?;
    tokio::time::timeout(Duration::from_secs(20), ep.online())
        .await
        .map_err(|_| anyhow!("no relay answered within 20 s"))?;
    Ok(ep)
}

async fn listen() -> Result<()> {
    let ep = bind(false).await?;
    let addr = ep.addr();
    println!("listen   endpoint {}", ep.id());
    println!(
        "listen   relay {}",
        addr.relay_urls()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!("listen   dial with: spike-call dial {}", ep.id());
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

struct Opts {
    to: EndpointId,
    secs: u64,
    size: usize,
    relay_only: bool,
    label: String,
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
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "--secs" => o.secs = it.next().context("--secs")?.parse()?,
            "--size" => o.size = it.next().context("--size")?.parse()?,
            "--relay-only" => o.relay_only = true,
            "--label" => o.label = it.next().context("--label")?.clone(),
            other => bail!("unknown argument {other}"),
        }
    }
    if o.size < 16 {
        bail!("--size must be at least 16");
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

async fn dial(args: &[String]) -> Result<()> {
    let o = parse(args)?;
    let ep = bind(o.relay_only).await?;
    println!(
        "dial     me {} -> {} ({}) label={:?}",
        ep.id().fmt_short(),
        o.to.fmt_short(),
        if o.relay_only {
            "relay only"
        } else {
            "direct allowed"
        },
        o.label
    );
    let t0 = Instant::now();
    let conn = tokio::time::timeout(Duration::from_secs(30), ep.connect(o.to, ALPN))
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

    let start = Instant::now();
    let total = o.secs * 50;
    let mut sent_at: HashMap<u64, Instant> = HashMap::new();
    let mut win = Window::default();
    let mut all_rtts: Vec<f64> = Vec::new();
    let mut per_kind: HashMap<&'static str, Vec<f64>> = HashMap::new();
    let (mut sent, mut got, mut late) = (0u64, 0u64, 0u64);
    let mut jitter = 0.0f64;
    let mut last_rtt: Option<f64> = None;
    let mut first_direct: Option<f64> = None;
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut sec = tokio::time::interval(Duration::from_secs(1));
    sec.tick().await;
    let mut seq = 0u64;
    let mut buf = vec![0u8; o.size];
    let mut kind_now: &'static str = "none";
    let drain_until = |s: &Instant| *s + Duration::from_secs(o.secs) + Duration::from_secs(2);

    loop {
        tokio::select! {
            _ = tick.tick(), if seq < total => {
                buf[..8].copy_from_slice(&seq.to_be_bytes());
                if conn.send_datagram(buf.clone().into()).is_ok() {
                    sent_at.insert(seq, Instant::now());
                    sent += 1;
                    win.sent += 1;
                }
                seq += 1;
            }
            d = conn.read_datagram() => {
                let d = d.context("read datagram")?;
                if d.len() < 8 { continue; }
                let s = u64::from_be_bytes(d[..8].try_into().unwrap());
                if let Some(at) = sent_at.remove(&s) {
                    let rtt = at.elapsed().as_secs_f64() * 1000.0;
                    // A frame back after 300 ms RTT is useless to a jitter buffer.
                    if rtt > 300.0 { late += 1; }
                    got += 1;
                    win.got += 1;
                    win.rtts.push(rtt);
                    all_rtts.push(rtt);
                    per_kind.entry(kind_now).or_default().push(rtt);
                    if let Some(prev) = last_rtt {
                        jitter += ((rtt - prev).abs() - jitter) / 16.0;
                    }
                    last_rtt = Some(rtt);
                }
            }
            _ = sec.tick() => {
                let r = path::report(&conn);
                kind_now = match r.kind { PathKind::Relay => "relay", PathKind::Direct => "direct", PathKind::None => "none" };
                if r.kind == PathKind::Direct && first_direct.is_none() {
                    first_direct = Some(start.elapsed().as_secs_f64());
                }
                let mut v = std::mem::take(&mut win.rtts);
                let loss = if win.sent > 0 { 100.0 * (1.0 - win.got as f64 / win.sent as f64) } else { 0.0 };
                println!(
                    "t={:>3}s path={:<6} rtt p50={:>6.1} p95={:>6.1} max={:>6.1}ms loss={:>5.1}% jitter={:>5.1}ms via={}",
                    start.elapsed().as_secs(), kind_now, pct(&mut v, 0.5), pct(&mut v, 0.95),
                    v.last().copied().unwrap_or(f64::NAN), loss.max(0.0), jitter, r.remote,
                );
                win = Window::default();
                if seq >= total && Instant::now() > drain_until(&start) { break; }
            }
        }
    }

    let lost = sent.saturating_sub(got);
    let mut a = all_rtts.clone();
    let mut kinds: Vec<String> = per_kind
        .iter_mut()
        .map(|(k, v)| format!("{k}:n={} p50={:.1}ms", v.len(), pct(v, 0.5)))
        .collect();
    kinds.sort();
    println!(
        "SUMMARY label={:?} mode={} connect={:.0}ms first_direct={} sent={sent} lost={lost} ({:.2}%) late>300ms={late} rtt p50={:.1} p95={:.1} p99={:.1} max={:.1}ms jitter={:.1}ms one_way~{:.0}ms paths=[{}]",
        o.label,
        if o.relay_only {
            "relay-only"
        } else {
            "direct-allowed"
        },
        connect_ms,
        first_direct
            .map(|s| format!("{s:.0}s"))
            .unwrap_or_else(|| "never".into()),
        100.0 * lost as f64 / sent.max(1) as f64,
        pct(&mut a, 0.5),
        pct(&mut a, 0.95),
        pct(&mut a, 0.99),
        a.last().copied().unwrap_or(f64::NAN),
        jitter,
        pct(&mut a, 0.5) / 2.0,
        kinds.join(" "),
    );
    conn.close(0u32.into(), b"done");
    ep.close().await;
    Ok(())
}
