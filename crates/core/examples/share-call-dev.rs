//! share-call-dev (#658): time a screen share from capture to the viewer's
//! window channel, over iroh, direct or relay only.
//!
//! ```sh
//! cargo run --release -p irori-core --example share-call-dev -- [--relay-only] [--secs 30] [--size 1920x1080]
//! ```
//!
//! Two endpoints in this process connect on n0's public relays as Irori
//! uses them (irori_net::public, no preset); `--relay-only` drops the IP
//! transports, so every byte goes through a relay (on this Mac: up to the
//! relay and back down over the same Wi-Fi). Each side runs the real
//! [`ShareHub`]: the sharer's encoder thread on the test picture through
//! the hardware encoder (no capture, no grant), its per-viewer GOP sender
//! and rate ramp; the viewer's GOP reader and AVCC conversion, ending at
//! the sink the window's channel would be.
//!
//! Delay is from just before the encoder starts plus the frame's timestamp
//! to the sink: an upper bound, since the encoder's start is counted too.

#![cfg_attr(
    not(any(target_os = "macos", target_os = "windows")),
    allow(dead_code, unused_imports)
)]

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn main() {
    eprintln!("share-call-dev needs the Mac's or the PC's hardware encoder");
}

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iroh::endpoint::Builder;
use iroh::{Endpoint, SecretKey};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use irori_core::share_dev::{Picture, ShareHub, spawn_encoder};
use irori_meshseam::{read_frame, write_frame};
use irori_net::share::{ShareInfo, VIEW_CHUNK, VIEW_CONFIG};

const ALPN: &[u8] = b"irori/share-dev/1";

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("share-call-dev: {e}");
        std::process::exit(1);
    }
}

async fn bind(relay_only: bool) -> Result<Endpoint, String> {
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
    let ep = b.bind().await.map_err(|e| format!("bind: {e}"))?;
    tokio::time::timeout(Duration::from_secs(20), ep.online())
        .await
        .map_err(|_| "no relay answered within 20 s".to_string())?;
    Ok(ep)
}

fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s[((s.len() - 1) as f64 * p).round() as usize]
}

struct Got {
    at: Instant,
    ts_us: u64,
    key: bool,
    bytes: usize,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let relay_only = args.iter().any(|a| a == "--relay-only");
    let val = |k: &str| {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let secs: u64 = val("--secs")
        .map_or(Ok(30), |v| v.parse())
        .map_err(|_| "--secs N")?;
    let (w, h) = match val("--size") {
        Some(s) => {
            let (w, h) = s.split_once('x').ok_or("--size WxH")?;
            (
                w.parse::<u32>().map_err(|_| "--size WxH")?,
                h.parse::<u32>().map_err(|_| "--size WxH")?,
            )
        }
        None => (1920, 1080),
    };

    let (sharer, viewer) = tokio::try_join!(bind(relay_only), bind(relay_only))?;
    let mut to = viewer.addr();
    if relay_only {
        to.addrs.retain(|a| a.is_relay());
    }
    let accept = {
        let viewer = viewer.clone();
        tokio::spawn(async move {
            let inc = viewer.accept().await.ok_or("no incoming")?;
            let conn = inc.await.map_err(|e| e.to_string())?;
            let (send, mut recv) = conn.accept_bi().await.map_err(|e| e.to_string())?;
            read_frame(&mut recv, 64, false)
                .await
                .map_err(|e| e.to_string())?;
            Ok::<_, String>((conn, send, recv))
        })
    };
    let t_conn = Instant::now();
    let a_conn = sharer
        .connect(to, ALPN)
        .await
        .map_err(|e| format!("connect: {e}"))?;
    let (mut a_send, a_recv) = a_conn.open_bi().await.map_err(|e| e.to_string())?;
    write_frame(&mut a_send, b"hi", 64)
        .await
        .map_err(|e| e.to_string())?;
    let (b_conn, b_send, b_recv) = accept.await.map_err(|e| e.to_string())??;
    let connect_ms = t_conn.elapsed().as_secs_f64() * 1000.0;

    let rt = tokio::runtime::Handle::current();
    let a_hub = ShareHub::new("a".into(), rt.clone(), Arc::new(|| {}));
    let b_hub = ShareHub::new("b".into(), rt.clone(), Arc::new(|| {}));
    a_hub.add_peer_stream("b", a_conn.clone(), (a_send, a_recv));
    b_hub.add_peer_stream("a", b_conn.clone(), (b_send, b_recv));

    let got: Arc<Mutex<Vec<Got>>> = Arc::default();
    let configs = Arc::new(Mutex::new(Vec::<String>::new()));
    {
        let (got, configs) = (got.clone(), configs.clone());
        b_hub.attach(Arc::new(move |b: Vec<u8>| {
            let at = Instant::now();
            match b[0] {
                VIEW_CONFIG => {
                    let n = u16::from_be_bytes([b[1], b[2]]) as usize;
                    configs
                        .lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&b[3..3 + n]).into_owned());
                }
                VIEW_CHUNK => got.lock().unwrap().push(Got {
                    at,
                    key: b[1] & 1 == 1,
                    ts_us: u64::from_be_bytes(b[2..10].try_into().unwrap()),
                    bytes: b.len() - 10,
                }),
                _ => {}
            }
            true
        }));
    }

    let mut id = [0u8; 16];
    getrandom::fill(&mut id).map_err(|e| e.to_string())?;
    let info = ShareInfo {
        v: 1,
        share_id: hex::encode(id),
        started_ms: 1,
        kind: "window".into(),
        title: "Test picture".into(),
        app: "share-call-dev".into(),
        fps: 10,
        ..ShareInfo::default()
    };
    let ctl = a_hub.begin(info)?;
    let t0 = Instant::now();
    {
        let hub = Arc::downgrade(&a_hub);
        tokio::task::spawn_blocking(move || spawn_encoder(hub, ctl, Picture::Synthetic(w, h)))
            .await
            .map_err(|e| e.to_string())??;
    }
    let mut sec = 0;
    while sec < secs {
        tokio::time::sleep(Duration::from_secs(1)).await;
        sec += 1;
        let g = got.lock().unwrap();
        let recent: Vec<f64> = g
            .iter()
            .filter(|x| x.at.duration_since(t0) > Duration::from_secs(sec - 1))
            .map(|x| delay_ms(t0, x))
            .collect();
        let bytes: usize = g
            .iter()
            .filter(|x| x.at.duration_since(t0) > Duration::from_secs(sec - 1))
            .map(|x| x.bytes)
            .sum();
        let r = irori_net::path::report(&a_conn);
        let kbps = a_hub.view().0.map_or(0, |n| n.info.kbps);
        println!(
            "t={sec:>3}s path={:?} frames={:>2} kbit/s in={:>5} target={kbps} delay p50={:.0} max={:.0} ms",
            r.kind,
            recent.len(),
            bytes * 8 / 1000,
            pct(&recent, 0.5),
            pct(&recent, 1.0),
        );
    }
    let g = std::mem::take(&mut *got.lock().unwrap());
    let all: Vec<f64> = g.iter().map(|x| delay_ms(t0, x)).collect();
    let keys: Vec<f64> = g
        .iter()
        .filter(|x| x.key)
        .map(|x| delay_ms(t0, x))
        .collect();
    let key_kb: Vec<f64> = g
        .iter()
        .filter(|x| x.key)
        .map(|x| x.bytes as f64 / 1000.0)
        .collect();
    let over = all.iter().filter(|d| **d > 1000.0).count();
    let r = irori_net::path::report(&a_conn);
    println!(
        "SUMMARY mode={} path={:?} size={w}x{h} connect={connect_ms:.0}ms frames={} (of ~{}) config={:?} delay p50={:.0} p95={:.0} max={:.0} ms over1s={over} keyframes={} key delay p50={:.0} max={:.0} ms key size p50={:.0} KB",
        if relay_only {
            "relay-only"
        } else {
            "direct-allowed"
        },
        r.kind,
        g.len(),
        secs * 10,
        configs.lock().unwrap(),
        pct(&all, 0.5),
        pct(&all, 0.95),
        pct(&all, 1.0),
        keys.len(),
        pct(&keys, 0.5),
        pct(&keys, 1.0),
        pct(&key_kb, 0.5),
    );
    a_hub.end(None, None);
    a_conn.close(0u32.into(), b"done");
    sharer.close().await;
    viewer.close().await;
    Ok(())
}

fn delay_ms(t0: Instant, x: &Got) -> f64 {
    let made = t0 + Duration::from_micros(x.ts_us);
    x.at.saturating_duration_since(made).as_secs_f64() * 1000.0
}
