//! NAT-PMP (RFC 6886), go-room's own client (R:portmap/natpmp.go): three
//! requests over UDP to the gateway, version 0.
//!
//! ```text
//! external address: op 0          -> op 128, result, epoch, address
//! map UDP / TCP:    op 1 / 2, internal port, suggested external, lifetime
//!                                 -> op 129 / 130, result, epoch, internal, external, lifetime
//! ```
//!
//! A lifetime of 0 removes the mapping. Requests are retried from 250 ms,
//! doubling, up to five tries.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use tokio::net::UdpSocket;

use super::{BoxFut, Gateway, MapError, Port};

/// The gateway port NAT-PMP uses.
pub const PORT: u16 = 5351;

const FIRST_WAIT: Duration = Duration::from_millis(250);
const TRIES: u32 = 5;

/// A [`Gateway`] speaking NAT-PMP to `gw` (the app: the default gateway on
/// [`PORT`]; tests: a fake on 127.0.0.1).
#[derive(Debug, Clone)]
pub struct NatPmp {
    pub gw: SocketAddr,
    /// The first retry wait (doubling); tests shorten it.
    pub first_wait: Duration,
}

fn result_text(rc: u16) -> String {
    match rc {
        1 => "unsupported version".into(),
        2 => "not authorized or refused".into(),
        3 => "network failure".into(),
        4 => "out of resources".into(),
        5 => "unsupported opcode".into(),
        n => format!("result {n}"),
    }
}

impl NatPmp {
    pub fn new(gw: SocketAddr) -> Self {
        NatPmp {
            gw,
            first_wait: FIRST_WAIT,
        }
    }

    async fn call(&self, req: &[u8], resp_len: usize) -> Result<Vec<u8>, String> {
        let local: SocketAddr = match self.gw.ip() {
            IpAddr::V4(v4) if v4.is_loopback() => (Ipv4Addr::LOCALHOST, 0).into(),
            _ => (Ipv4Addr::UNSPECIFIED, 0).into(),
        };
        let c = UdpSocket::bind(local).await.map_err(|e| e.to_string())?;
        c.connect(self.gw).await.map_err(|e| e.to_string())?;
        let mut wait = self.first_wait;
        let mut buf = [0u8; 64];
        for _ in 0..TRIES {
            c.send(req).await.map_err(|e| e.to_string())?;
            let deadline = tokio::time::Instant::now() + wait;
            loop {
                let r = tokio::time::timeout_at(deadline, c.recv(&mut buf)).await;
                let n = match r {
                    Err(_) => break, // timeout: send again
                    Ok(Err(_)) => break,
                    Ok(Ok(n)) => n,
                };
                if n < resp_len || buf[0] != 0 || buf[1] != 128 + req[1] {
                    continue; // not the answer to this request
                }
                let rc = u16::from_be_bytes([buf[2], buf[3]]);
                if rc != 0 {
                    return Err(format!("natpmp: {}", result_text(rc)));
                }
                return Ok(buf[..n].to_vec());
            }
            wait *= 2;
        }
        Err("natpmp: no answer from the gateway".into())
    }

    async fn map_req(&self, p: Port, life: Duration) -> Result<(Port, Duration), String> {
        let op = match p.proto {
            "udp" => 1u8,
            "tcp" => 2u8,
            other => return Err(format!("natpmp: unknown protocol {other:?}")),
        };
        let mut req = [0u8; 12];
        req[1] = op;
        req[4..6].copy_from_slice(&p.internal.to_be_bytes());
        req[6..8].copy_from_slice(&p.external.to_be_bytes());
        req[8..12].copy_from_slice(&(life.as_secs() as u32).to_be_bytes());
        let resp = self.call(&req, 16).await?;
        let got = Port {
            proto: p.proto,
            internal: u16::from_be_bytes([resp[8], resp[9]]),
            external: u16::from_be_bytes([resp[10], resp[11]]),
        };
        let secs = u32::from_be_bytes([resp[12], resp[13], resp[14], resp[15]]);
        Ok((got, Duration::from_secs(u64::from(secs))))
    }
}

impl Gateway for NatPmp {
    fn method(&self) -> &str {
        "natpmp"
    }

    fn external_ip(&self) -> BoxFut<'_, Result<IpAddr, String>> {
        Box::pin(async move {
            let resp = self.call(&[0, 0], 12).await?;
            Ok(IpAddr::V4(Ipv4Addr::new(
                resp[8], resp[9], resp[10], resp[11],
            )))
        })
    }

    /// The router picks another external port itself if the one asked for
    /// is taken, so a conflict never surfaces here.
    fn map(&self, mut p: Port, life: Duration) -> BoxFut<'_, Result<(Port, Duration), MapError>> {
        Box::pin(async move {
            if p.external == 0 {
                p.external = p.internal;
            }
            let life = life.max(Duration::from_secs(1));
            let (got, granted) = self.map_req(p, life).await.map_err(MapError::Other)?;
            if granted.is_zero() {
                return Err(MapError::Other(
                    "natpmp: the gateway granted no lifetime".into(),
                ));
            }
            Ok((got, granted))
        })
    }

    fn unmap(&self, p: Port) -> BoxFut<'_, Result<(), String>> {
        Box::pin(async move {
            self.map_req(
                Port {
                    proto: p.proto,
                    internal: p.internal,
                    external: 0,
                },
                Duration::ZERO,
            )
            .await
            .map(|_| ())
        })
    }
}
