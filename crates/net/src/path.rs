//! Relay vs direct, and RTT, for a live connection.
//!
//! iroh 1.4.1: `Connection::paths()` is a snapshot `PathList`; each `Path` has
//! `is_selected()`, `is_relay()`, `is_ip()`, `remote_addr() -> &TransportAddr`,
//! `rtt()` (= `stats().rtt`) and `stats() -> PathStats` (noq-proto 1.4.0:
//! `rtt`, `udp_tx`/`udp_rx`, `frame_tx`/`frame_rx`, `cwnd`, `bandwidth_estimate`,
//! `congestion_events`, `sent_packets`, `lost_packets`, …). `PathStats` itself has
//! no relay/direct flag: the kind comes from `Path::is_relay()` / `TransportAddr`.

use std::fmt;
use std::time::Duration;

use iroh::TransportAddr;
use iroh::endpoint::Connection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    Relay,
    Direct,
    /// No path is selected (yet).
    None,
}

impl fmt::Display for PathKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PathKind::Relay => "relay",
            PathKind::Direct => "direct",
            PathKind::None => "none",
        })
    }
}

#[derive(Debug, Clone)]
pub struct PathReport {
    pub kind: PathKind,
    pub rtt: Option<Duration>,
    /// The selected path's remote address (relay URL or ip:port).
    pub remote: String,
    /// Every open path, `kind@addr`, selected first.
    pub open: Vec<String>,
}

impl fmt::Display for PathReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rtt = self
            .rtt
            .map(|d| format!("{:.1}ms", d.as_secs_f64() * 1000.0))
            .unwrap_or_else(|| "-".into());
        write!(
            f,
            "path={} rtt={} via={} open=[{}]",
            self.kind,
            rtt,
            self.remote,
            self.open.join(", ")
        )
    }
}

fn addr_str(a: &TransportAddr) -> String {
    match a {
        TransportAddr::Ip(s) => s.to_string(),
        TransportAddr::Relay(u) => u.to_string(),
        other => format!("{other:?}"),
    }
}

/// The selected path's kind and RTT (mesh.md §5.2 "Path / RTT").
pub fn report(conn: &Connection) -> PathReport {
    let paths = conn.paths();
    let mut open = Vec::new();
    let mut sel = None;
    for p in paths.iter() {
        let kind = if p.is_relay() {
            PathKind::Relay
        } else if p.is_ip() {
            PathKind::Direct
        } else {
            PathKind::None
        };
        let s = format!("{kind}@{}", addr_str(p.remote_addr()));
        if p.is_selected() {
            sel = Some((kind, p.rtt(), addr_str(p.remote_addr())));
            open.insert(0, format!("*{s}"));
        } else {
            open.push(s);
        }
    }
    match sel {
        Some((kind, rtt, remote)) => PathReport {
            kind,
            rtt: Some(rtt),
            remote,
            open,
        },
        None => PathReport {
            kind: PathKind::None,
            rtt: None,
            remote: "-".into(),
            open,
        },
    }
}
