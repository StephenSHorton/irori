//! Find a home server in a copy of `irori.db`: its newest `node.addr` row gives
//! where to dial (mesh.md §2.6, §3.1 "Servers.Addrs order"), and its
//! `person.device.added` cert gives its devicePub, which is its EndpointId (D2).
//!
//! Read-only: opened with `SQLITE_OPEN_READ_ONLY`, payload bytes parsed as they
//! are stored and never re-encoded. Spike limits, noted for M5: no row-signature
//! or Chain check (the real `Servers` fold ignores revoked writers and
//! uncertified certs), and `Heard`/`hub-addrs.json` hints are not consulted.

use std::net::IpAddr;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct NodeAddr {
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    lan: Vec<String>,
    #[serde(default)]
    v6: Vec<String>,
    #[serde(default)]
    public: Option<Public>,
    #[serde(default)]
    reach: String,
}

#[derive(Debug, Deserialize)]
struct Public {
    #[serde(default)]
    ip: String,
    #[serde(default)]
    tcp: Option<u16>,
}

#[derive(Debug, Deserialize)]
struct CertRow {
    cert: Cert,
}

#[derive(Debug, Deserialize)]
struct Cert {
    #[serde(rename = "deviceId", default)]
    device_id: String,
    #[serde(rename = "devicePub", default)]
    device_pub: String,
    #[serde(rename = "personId", default)]
    person_id: String,
}

/// Every devicePub certified for `person_id` (`person.device.added` rows), for
/// the spike relay's allow list. Spike: no person-signature or revoke check.
pub fn certified_devices(db: &Path, person_id: &str) -> Result<Vec<String>> {
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("open {} read-only", db.display()))?;
    let mut st =
        conn.prepare("SELECT payload FROM events WHERE kind = 'person.device.added' ORDER BY hlc")?;
    let mut rows = st.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let payload: Option<Vec<u8>> = r.get(0)?;
        let Some(payload) = payload else { continue };
        let Ok(c) = serde_json::from_slice::<CertRow>(&payload) else {
            continue;
        };
        if c.cert.person_id == person_id && !out.contains(&c.cert.device_pub) {
            out.push(c.cert.device_pub);
        }
    }
    Ok(out)
}

/// One home server as the log describes it.
#[derive(Debug, Clone)]
pub struct HomeServer {
    pub device_id: String,
    /// Hex devicePub = iroh EndpointId.
    pub device_pub: String,
    pub person_id: String,
    /// The port written in `node.addr` (Go's port N, 25817 on the PC).
    pub port: Option<u16>,
    /// Dial order: LAN IPv4s, then v6, then the public IP when reach is ok.
    pub hosts: Vec<IpAddr>,
    pub public_ip: Option<IpAddr>,
    pub public_tcp: Option<u16>,
    pub reach: String,
}

/// The newest `node.addr` row (by HLC) not written by `exclude_device` and not
/// `reach: off`, joined with that device's newest cert row.
pub fn home_server(db: &Path, exclude_device: Option<&str>) -> Result<HomeServer> {
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("open {} read-only", db.display()))?;
    let mut st = conn.prepare(
        "SELECT device_id, payload FROM events WHERE kind = 'node.addr' ORDER BY hlc DESC, device_id, seq DESC",
    )?;
    let mut rows = st.query([])?;
    let mut found = None;
    while let Some(r) = rows.next()? {
        let dev: String = r.get(0)?;
        if Some(dev.as_str()) == exclude_device {
            continue;
        }
        let payload: Option<Vec<u8>> = r.get(1)?;
        let Some(payload) = payload else { continue };
        let Ok(na) = serde_json::from_slice::<NodeAddr>(&payload) else {
            continue;
        };
        if na.reach == "off" {
            continue;
        }
        found = Some((dev, na));
        break;
    }
    let (dev, na) = found.ok_or_else(|| {
        anyhow!(
            "no node.addr row from a running home server in {}",
            db.display()
        )
    })?;

    let mut st = conn.prepare(
        "SELECT payload FROM events WHERE kind = 'person.device.added' ORDER BY hlc DESC, device_id, seq DESC",
    )?;
    let mut rows = st.query([])?;
    let mut cert = None;
    while let Some(r) = rows.next()? {
        let payload: Option<Vec<u8>> = r.get(0)?;
        let Some(payload) = payload else { continue };
        let Ok(c) = serde_json::from_slice::<CertRow>(&payload) else {
            continue;
        };
        if c.cert.device_id == dev {
            cert = Some(c.cert);
            break;
        }
    }
    let cert = cert.ok_or_else(|| anyhow!("no person.device.added cert for server {dev}"))?;
    if cert.device_pub.len() != 64 || hex::decode(&cert.device_pub).is_err() {
        bail!("server {dev}'s devicePub is not 32-byte hex");
    }

    let mut hosts: Vec<IpAddr> = Vec::new();
    for s in na.lan.iter().chain(na.v6.iter()) {
        if let Ok(ip) = s.parse::<IpAddr>()
            && !hosts.contains(&ip)
        {
            hosts.push(ip);
        }
    }
    let mut public_tcp = None;
    let mut public_ip = None;
    if na.reach == "ok"
        && let Some(p) = &na.public
        && let Ok(ip) = p.ip.parse::<IpAddr>()
    {
        if !hosts.contains(&ip) {
            hosts.push(ip);
        }
        public_ip = Some(ip);
        public_tcp = p.tcp;
    }
    Ok(HomeServer {
        device_id: dev,
        device_pub: cert.device_pub,
        person_id: cert.person_id,
        port: na.port,
        hosts,
        public_ip,
        public_tcp,
        reach: na.reach,
    })
}
