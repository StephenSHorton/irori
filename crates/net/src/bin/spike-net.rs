//! `spike-net`: the M1 networking spike (architecture §8 M1, docs/m1-mac.md).
//!
//!   spike-net relay --home <dir> --port <devport> [--public] [--allow <id>]... [--allow-db <irori.db copy>]
//!   spike-net dial  --home <dir> --relay <url> --to <endpointId> [--relay-pub <id>] [--direct | --direct-loopback] [--secs N]
//!   spike-net dial  --home <dir> --from-db <irori.db copy> [--port <devport>] [--host <ip>] [--direct] [--secs N]
//!   spike-net fixture --out <new dir>
//!
//! `--home` must be a copy of a data dir (never the live one); it is only read.
//! Dial is relay-only by default (no IP socket). `--direct` binds UDP 0.0.0.0 for
//! real direct paths; `--direct-loopback` binds 127.0.0.1. The relay binds
//! 127.0.0.1 unless `--public`.

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use iroh::{EndpointAddr, EndpointId, RelayUrl, Watcher as _};
use irori_net::endpoint::{self, EndpointOpts, IpMode, RelaySpec};
use irori_net::identity::IrohKey as _;
use irori_net::relay::{self, AllowList};
use irori_net::{fromdb, identity, path};

struct Args(Vec<String>);

impl Args {
    fn flag(&mut self, name: &str) -> bool {
        if let Some(i) = self.0.iter().position(|a| a == name) {
            self.0.remove(i);
            true
        } else {
            false
        }
    }
    fn opt(&mut self, name: &str) -> Result<Option<String>> {
        let Some(i) = self.0.iter().position(|a| a == name) else {
            return Ok(None);
        };
        if i + 1 >= self.0.len() {
            bail!("{name} needs a value");
        }
        let v = self.0.remove(i + 1);
        self.0.remove(i);
        Ok(Some(v))
    }
    fn many(&mut self, name: &str) -> Result<Vec<String>> {
        let mut out = vec![];
        while let Some(v) = self.opt(name)? {
            out.push(v);
        }
        Ok(out)
    }
    fn done(&self) -> Result<()> {
        if !self.0.is_empty() {
            bail!("unknown arguments: {:?}", self.0);
        }
        Ok(())
    }
}

fn usage() -> ! {
    eprintln!(
        "usage:\n  spike-net relay --home <dir> --port <devport> [--public] [--allow <id>]... [--allow-db <irori.db copy>]\n  spike-net dial --home <dir> (--relay <url> --to <id> [--relay-pub <id>] | --from-db <irori.db copy> [--port <devport>] [--host <ip>]) [--direct] [--secs N]"
    );
    std::process::exit(2)
}

/// Refuse the live data dir: the spike reads copies only.
fn check_home(home: &Path) -> Result<()> {
    let live = std::env::var_os("LOCALAPPDATA")
        .map(|d| PathBuf::from(d).join("irori"))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".irori")));
    if let Some(live) = live
        && same_dir(home, &live)?
    {
        bail!("--home is the live data dir {}; use a copy", live.display());
    }
    Ok(())
}

/// Whether two spellings name one dir. `canonicalize` resolves symlinks,
/// 8.3 short names and trailing separators when both exist; otherwise the
/// absolute paths' components are compared (case-insensitively on Windows),
/// which also ignores trailing separators and `.`.
fn same_dir(a: &Path, b: &Path) -> Result<bool> {
    if let (Ok(ca), Ok(cb)) = (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        return Ok(norm(&ca) == norm(&cb));
    }
    Ok(norm(&std::path::absolute(a)?) == norm(&std::path::absolute(b)?))
}

fn norm(p: &Path) -> Vec<String> {
    p.components()
        .filter(|c| !matches!(c, std::path::Component::CurDir))
        .map(|c| {
            let s = c.as_os_str().to_string_lossy();
            if cfg!(windows) {
                s.to_lowercase()
            } else {
                s.into_owned()
            }
        })
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var_os("RUST_LOG").is_some() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(std::io::stderr)
            .init();
    }
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        usage();
    }
    let cmd = argv.remove(0);
    let mut a = Args(argv);
    match cmd.as_str() {
        "relay" => cmd_relay(&mut a).await,
        "dial" => cmd_dial(&mut a).await,
        "fixture" => cmd_fixture(&mut a),
        _ => usage(),
    }
}

/// `spike-net fixture --out <new dir>`: a throwaway data dir with a fresh random
/// device key sealed by this OS's keystore, for loopback runs of the CLI.
/// Refuses a dir that already holds a device.json. Prints only public values.
fn cmd_fixture(a: &mut Args) -> Result<()> {
    let out = PathBuf::from(a.opt("--out")?.unwrap_or_else(|| usage()));
    a.done()?;
    check_home(&out)?;
    if out.join("device.json").exists() || out.join("device.key").exists() {
        bail!("{} already holds an identity", out.display());
    }
    std::fs::create_dir_all(&out)?;
    let mut seed = zeroize::Zeroizing::new([0u8; 32]);
    let k = iroh::SecretKey::generate();
    seed.copy_from_slice(&k.to_bytes());
    let pubk = k.public().to_string();
    let wrapped = identity::wrap_for_tests(&*seed, None)?;
    let dev = format!(
        "{{\n  \"id\": \"{}\",\n  \"name\": \"spike-fixture\",\n  \"os\": \"{}\",\n  \"devicePub\": \"{pubk}\"\n}}",
        &pubk[..32],
        if cfg!(target_os = "macos") {
            "darwin"
        } else {
            std::env::consts::OS
        }
    );
    std::fs::write(out.join("device.key"), wrapped)?;
    std::fs::write(out.join("device.json"), dev)?;
    println!("fixture {} endpoint {pubk}", out.display());
    Ok(())
}

async fn cmd_relay(a: &mut Args) -> Result<()> {
    let home = PathBuf::from(a.opt("--home")?.unwrap_or_else(|| usage()));
    let port: u16 = a
        .opt("--port")?
        .unwrap_or_else(|| usage())
        .parse()
        .context("--port")?;
    let public = a.flag("--public");
    let allow_ids = a.many("--allow")?;
    let allow_db = a.opt("--allow-db")?;
    a.done()?;
    check_home(&home)?;

    let id = identity::load(&home)?;
    let me = id.endpoint_id();
    let allow = AllowList::new([me]);
    for s in &allow_ids {
        allow.allow(endpoint::parse_id(s)?);
    }
    if let Some(db) = &allow_db {
        if id.device.person_id.is_empty() {
            bail!("device.json has no personId; cannot read certified devices");
        }
        for p in fromdb::certified_devices(Path::new(db), &id.device.person_id)? {
            allow.allow(endpoint::parse_id(&p)?);
        }
    }
    let bind_ip = if public {
        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    } else {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    };
    let r = relay::spawn(&id, bind_ip, port, allow.clone()).await?;
    let lo = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let own = RelaySpec {
        url: r.url_for(lo),
        quic_port: r.port,
        server_pub: me,
    };
    let mut o = EndpointOpts::new(if public {
        IpMode::AnyV4
    } else {
        IpMode::Loopback
    });
    o.relays = vec![own];
    o.allow = Some(allow.clone());
    let ep = endpoint::bind(&id, o).await?;

    println!("endpoint {me}");
    println!("device   {} ({})", id.device.id, id.device.name);
    println!(
        "relay    https {} quic {} (plain http {} loopback only)",
        r.server
            .https_addr()
            .map(|a| a.to_string())
            .unwrap_or_default(),
        r.server
            .quic_addr()
            .map(|a| a.to_string())
            .unwrap_or_default(),
        r.server
            .http_addr()
            .map(|a| a.to_string())
            .unwrap_or_default()
    );
    println!(
        "dial     spike-net dial --home <copy> --relay {} --to {me}",
        r.url_for(lo)
    );
    println!("sockets  {:?}", ep.bound_sockets());

    let ep2 = ep.clone();
    tokio::spawn(endpoint::serve_echo(ep.clone(), move |conn| {
        let rid = conn.remote_id();
        println!("conn     from {rid}");
        let weak = conn.weak_handle();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let Some(c) = weak.upgrade() else { break };
                if c.close_reason().is_some() {
                    println!(
                        "conn     {} closed: {:?}",
                        rid.fmt_short(),
                        c.close_reason()
                    );
                    break;
                }
                println!("conn     {} {}", rid.fmt_short(), path::report(&c));
            }
        });
    }));
    let mut last = String::new();
    loop {
        let st = ep2.home_relay_status().get();
        let line = st
            .iter()
            .map(|s| {
                format!(
                    "{} {}{}",
                    s.url(),
                    if s.is_connected() {
                        "connected"
                    } else {
                        "not connected"
                    },
                    s.last_error()
                        .map(|e| format!(" ({e:#})"))
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        if line != last {
            println!(
                "status   home relay: {}",
                if line.is_empty() {
                    "none yet".into()
                } else {
                    line.clone()
                }
            );
            last = line;
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            _ = tokio::signal::ctrl_c() => { ep2.close().await; return Ok(()); }
        }
    }
}

async fn cmd_dial(a: &mut Args) -> Result<()> {
    let home = PathBuf::from(a.opt("--home")?.unwrap_or_else(|| usage()));
    let relay_s = a.opt("--relay")?;
    let to_s = a.opt("--to")?;
    let relay_pub_s = a.opt("--relay-pub")?;
    let from_db = a.opt("--from-db")?;
    let port_s = a.opt("--port")?;
    let host_s = a.opt("--host")?;
    let direct_lo = a.flag("--direct-loopback");
    let direct = a.flag("--direct") || direct_lo;
    let secs: u64 = a
        .opt("--secs")?
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(0);
    a.done()?;
    check_home(&home)?;
    let id = identity::load(&home)?;

    let (url, quic_port, to, relay_pub): (RelayUrl, u16, EndpointId, EndpointId) =
        if let Some(db) = from_db {
            let hs = fromdb::home_server(Path::new(&db), Some(&id.device.id))?;
            println!(
                "from-db  server {} devicePub {} reach {} node.addr port {:?} hosts {:?}",
                hs.device_id, hs.device_pub, hs.reach, hs.port, hs.hosts
            );
            if !id.device.person_id.is_empty() && hs.person_id != id.device.person_id {
                println!(
                    "warning  server's cert names person {} but this device is {}",
                    hs.person_id, id.device.person_id
                );
            }
            let host: IpAddr = match host_s {
                Some(h) => h.parse().context("--host")?,
                None => *hs
                    .hosts
                    .first()
                    .ok_or_else(|| anyhow!("node.addr row has no address"))?,
            };
            let port: u16 = match port_s {
                Some(p) => p.parse().context("--port")?,
                None if Some(host) == hs.public_ip && hs.public_tcp.is_some() => {
                    hs.public_tcp.unwrap_or(0)
                }
                None => hs
                    .port
                    .ok_or_else(|| anyhow!("node.addr row has no port; pass --port"))?,
            };
            let pk = endpoint::parse_id(&hs.device_pub)?;
            (relay::relay_url(host, port), port, pk, pk)
        } else {
            let url: RelayUrl = relay_s
                .unwrap_or_else(|| usage())
                .parse()
                .map_err(|e| anyhow!("--relay: {e}"))?;
            let to = endpoint::parse_id(&to_s.unwrap_or_else(|| usage()))?;
            let rp = match relay_pub_s {
                Some(s) => endpoint::parse_id(&s)?,
                None => to,
            };
            let port = url
                .port()
                .ok_or_else(|| anyhow!("--relay needs an explicit port"))?;
            (url, port, to, rp)
        };

    let mut o = EndpointOpts::new(if direct_lo {
        IpMode::Loopback
    } else if direct {
        IpMode::AnyV4
    } else {
        IpMode::RelayOnly
    });
    o.relays = vec![RelaySpec {
        url: url.clone(),
        quic_port,
        server_pub: relay_pub,
    }];
    let ep = endpoint::bind(&id, o).await?;
    println!(
        "endpoint {} dialing {to} via {url} ({})",
        ep.id(),
        if direct {
            "direct allowed"
        } else {
            "relay only"
        }
    );

    let t0 = Instant::now();
    let conn = tokio::time::timeout(
        Duration::from_secs(30),
        ep.connect(
            EndpointAddr::new(to).with_relay_url(url),
            irori_net::SPIKE_ALPN,
        ),
    )
    .await
    .map_err(|_| anyhow!("connect timed out after 30 s"))??;
    println!("connected in {:?}", t0.elapsed());
    let mut n = 0u64;
    loop {
        let msg = format!("echo {n}");
        let s = Instant::now();
        let back = endpoint::echo_once(&conn, msg.as_bytes()).await?;
        if back != msg.as_bytes() {
            bail!("echo mismatch");
        }
        println!(
            "echo {n} ok in {:.1}ms  {}",
            s.elapsed().as_secs_f64() * 1000.0,
            path::report(&conn)
        );
        n += 1;
        if secs > 0 && t0.elapsed() >= Duration::from_secs(secs) {
            break;
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    conn.close(0u32.into(), b"leaving");
    ep.close().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::same_dir;
    use std::path::Path;

    #[test]
    fn other_spellings_of_one_dir_match() {
        let t = tempfile::tempdir().unwrap();
        let live = t.path().join("irori");
        std::fs::create_dir(&live).unwrap();
        let sep = std::path::MAIN_SEPARATOR;
        let s = live.to_string_lossy();
        for spelling in [
            format!("{s}{sep}"),
            format!("{s}{sep}{sep}"),
            format!("{s}{sep}.{sep}"),
            format!("{s}{sep}..{sep}irori"),
        ] {
            assert!(same_dir(Path::new(&spelling), &live).unwrap(), "{spelling}");
        }
        if cfg!(windows) {
            let upper = s.to_uppercase();
            assert!(same_dir(Path::new(&upper), &live).unwrap(), "{upper}");
        }
        // A dir that doesn't exist yet still compares by its components.
        let missing = t.path().join("gone");
        assert!(same_dir(&missing.join("."), &missing).unwrap());
        let other = t.path().join("copy");
        std::fs::create_dir(&other).unwrap();
        assert!(!same_dir(&other, &live).unwrap());
    }
}
