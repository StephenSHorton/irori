//! `call-dev`: a real call between machines, outside the app (#654).
//!
//!   call-dev [--dial <endpointId>]... [--mute] [--probe] [--say] [--no-aec] [--no-ns] [--no-agc] [--secs N]
//!
//! Each run is one participant: it prints its endpoint id, accepts calls,
//! and dials every --dial id, so three runs where the later ones dial the
//! earlier ones make a full mesh. Real mic and speaker (cpal), the real
//! engine (Opus, FEC, DTX, jitter buffer, echo cancelling), QUIC datagrams
//! over iroh. Throwaway keys and n0's relays and address lookup, like
//! spike-call; it never touches an Irori data dir.
//!
//! Mouth-to-ear: put two devices side by side, run the far one with
//! --mute, and this one with --probe. Every 2.5 s it clicks its own speaker
//! and times the click's return from the far speaker at its own mic (see
//! src/probe.rs). Turn the volume up on both.
//!
//! Echo cancelling: run the far one with --say (a stand-in voice it sends
//! every other 1.5 s) and this one plain, on laptop speakers. The far one
//! prints how loud this one comes back (peer level): the voice's echo,
//! picked up by this mic, should be far below the level --say sends. Run
//! this one again with --no-aec to hear the difference.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use call::dsp::DspConfig;
use call::{Config, Engine, Inlet, Link};
use iroh::endpoint::{Connection, presets};
use iroh::{Endpoint, EndpointId, SecretKey};

const ALPN: &[u8] = b"irori/call-dev/1";

struct Opts {
    dial: Vec<EndpointId>,
    cfg: Config,
    secs: u64,
}

fn parse() -> Result<Opts> {
    let mut o = Opts {
        dial: Vec::new(),
        cfg: Config::default(),
        secs: 0,
    };
    let mut dsp = DspConfig::default();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dial" => o.dial.push(
                it.next()
                    .context("--dial needs an endpoint id")?
                    .parse()
                    .map_err(|e| anyhow!("endpoint id: {e}"))?,
            ),
            "--mute" => o.cfg.muted = true,
            "--probe" => o.cfg.probe = true,
            "--say" => o.cfg.say = true,
            "--no-aec" => dsp.echo_cancel = false,
            "--no-ns" => dsp.noise_suppress = false,
            "--no-agc" => dsp.auto_gain = false,
            "--secs" => o.secs = it.next().context("--secs")?.parse()?,
            "-h" | "--help" => {
                eprintln!(
                    "usage: call-dev [--dial <endpointId>]... [--mute] [--probe] [--say] [--no-aec] [--no-ns] [--no-agc] [--secs N]"
                );
                std::process::exit(2)
            }
            other => bail!("unknown argument {other}"),
        }
    }
    o.cfg.dsp = dsp;
    Ok(o)
}

/// Sends on an iroh connection's datagrams.
struct Datagrams(Connection);

impl Link for Datagrams {
    fn send(&self, d: &[u8]) -> bool {
        self.0.send_datagram(d.to_vec().into()).is_ok()
    }
}

/// One connection's life: add the peer, pump its datagrams in, remove it
/// when it closes.
async fn serve(conn: Connection, engine: Arc<Engine>, inlet: Inlet) {
    let id = conn.remote_id().fmt_short().to_string();
    match conn.max_datagram_size() {
        Some(n) => println!("call-dev peer {id} joined (datagrams up to {n} B)"),
        None => {
            println!("call-dev peer {id} can't take datagrams; closing");
            conn.close(1u32.into(), b"no datagrams");
            return;
        }
    }
    engine.add_peer(&id, Arc::new(Datagrams(conn.clone())));
    let reporter = {
        let conn = conn.clone();
        let id = id.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                println!("call-dev path {id}: {}", irori_net::path::report(&conn));
            }
        })
    };
    while let Ok(d) = conn.read_datagram().await {
        inlet.receive(&id, d.to_vec());
    }
    reporter.abort();
    engine.remove_peer(&id);
    println!("call-dev peer {id} left");
}

#[tokio::main]
async fn main() -> Result<()> {
    let o = parse()?;
    let ep = Endpoint::builder(presets::N0)
        .secret_key(SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .context("bind")?;
    tokio::time::timeout(Duration::from_secs(20), ep.online())
        .await
        .map_err(|_| anyhow!("no relay answered within 20 s"))?;
    println!("call-dev endpoint {}", ep.id());
    println!("call-dev others join with: --dial {}", ep.id());

    let engine = Arc::new(
        tokio::task::spawn_blocking({
            let cfg = o.cfg;
            move || Engine::start(cfg, call::device::default_devices())
        })
        .await?
        .map_err(|e| anyhow!(e))?,
    );
    let inlet = engine.inlet();

    {
        let ep = ep.clone();
        let (engine, inlet) = (engine.clone(), inlet.clone());
        tokio::spawn(async move {
            while let Some(inc) = ep.accept().await {
                let (engine, inlet) = (engine.clone(), inlet.clone());
                tokio::spawn(async move {
                    match inc.await {
                        Ok(c) => serve(c, engine, inlet).await,
                        Err(e) => println!("call-dev accept failed: {e:#}"),
                    }
                });
            }
        });
    }
    for id in o.dial.clone() {
        let (ep, engine, inlet) = (ep.clone(), engine.clone(), inlet.clone());
        tokio::spawn(async move {
            match ep.connect(id, ALPN).await {
                Ok(c) => serve(c, engine, inlet).await,
                Err(e) => println!("call-dev dial {} failed: {e:#}", id.fmt_short()),
            }
        });
    }

    let t0 = Instant::now();
    let mut seen = 0;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
        let st = engine.stats();
        let echo = match st.echo.erle_db {
            Some(e) => format!("{e:.1} dB"),
            None => "-".into(),
        };
        println!(
            "call-dev t={:>3}s sent={} level={:.0}dBFS speaking={} muted={} echo-removed={} reduction={} aec-delay={:?}ms mic={}Hz/{:.1}ms spk={}Hz/{:.1}ms underruns={}",
            t0.elapsed().as_secs(),
            st.sent,
            st.sent_level_dbfs,
            st.speaking,
            st.muted,
            echo,
            st.echo_reduction_db
                .map_or("-".into(), |d| format!("{d:.1} dB")),
            st.echo.delay_ms,
            st.mic_rate,
            st.input_latency_ms,
            st.speaker_rate,
            st.output_latency_ms,
            st.underruns
        );
        for p in &st.peers {
            let j = &p.jitter;
            println!(
                "call-dev   peer {} level={:.0}dBFS recv={} lost={} late={} fec={} concealed={} target={:.0}ms jitter={:.0}ms depth={:.0}ms speaking={} muted={}",
                p.id,
                p.level_dbfs,
                j.received,
                j.lost,
                j.late,
                j.fec,
                j.concealed,
                j.target_ms,
                j.jitter_ms,
                j.depth_ms,
                p.speaking,
                j.muted
            );
        }
        for ms in &st.mouth_to_ear_ms[seen..] {
            println!("call-dev mouth-to-ear {ms:.1} ms");
        }
        seen = st.mouth_to_ear_ms.len();
        if o.secs > 0 && t0.elapsed() >= Duration::from_secs(o.secs) {
            break;
        }
    }
    let st = engine.stats();
    if !st.mouth_to_ear_ms.is_empty() {
        let mut v = st.mouth_to_ear_ms.clone();
        v.sort_by(|a, b| a.total_cmp(b));
        println!(
            "call-dev mouth-to-ear n={} min={:.1} median={:.1} max={:.1} ms",
            v.len(),
            v[0],
            v[v.len() / 2],
            v[v.len() - 1]
        );
    }
    ep.close().await;
    Ok(())
}
