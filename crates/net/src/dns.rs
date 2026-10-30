//! The endpoint's DNS (D3/D13: no third party). iroh's own default resolver
//! reads the system's nameservers and keeps n0's public resolvers behind
//! them as a fallback tier (iroh-dns 1.4.1 `FallbackMode::Deferred`), and
//! iroh's DNS address lookup asks TXT records of `dns.iroh.link`. Irori
//! wants neither, so every endpoint gets [`LocalDns`]:
//!
//! - an IP literal is itself;
//! - a host name (a home server address the person typed, Go's
//!   `net.Dial` of a hub `host:port`) goes to the operating system's
//!   resolver (`getaddrinfo`: the hosts file, then the nameservers the
//!   network set), never to a resolver iroh picks;
//! - TXT lookups (iroh's DNS/pkarr address lookup) and n0's names are
//!   refused.
//!
//! Every name asked is kept in a [`DnsLog`] for status lines and the
//! no-public-contact test.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex};

use iroh::dns::{BoxIter, DnsError, DnsResolver, Resolver, TxtRecordData};
use n0_future::boxed::BoxFuture;

/// n0's infrastructure names: never resolved, whatever asks.
const REFUSED_SUFFIXES: &[&str] = &["iroh.link", "n0.computer", "iroh.computer"];

/// The names an endpoint's resolver was asked, in order, with what kind of
/// lookup (`a`, `aaaa`, `txt`). Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct DnsLog(Arc<Mutex<Vec<(String, &'static str)>>>);

impl DnsLog {
    fn push(&self, name: &str, kind: &'static str) {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if g.len() < 1024 {
            g.push((name.to_string(), kind));
        }
    }

    /// Every lookup so far.
    pub fn asked(&self) -> Vec<(String, &'static str)> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// The resolver every Irori endpoint uses (see the module docs).
#[derive(Debug, Clone, Default)]
pub struct LocalDns {
    log: DnsLog,
}

impl LocalDns {
    pub fn new(log: DnsLog) -> LocalDns {
        LocalDns { log }
    }

    /// As iroh's [`DnsResolver`], for `Builder::dns_resolver`.
    pub fn resolver(log: DnsLog) -> DnsResolver {
        DnsResolver::custom(LocalDns::new(log))
    }
}

fn refused(what: String) -> DnsError {
    DnsError::from(n0_error::AnyError::from(what))
}

fn is_refused_name(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    REFUSED_SUFFIXES
        .iter()
        .any(|s| h == *s || h.ends_with(&format!(".{s}")))
}

/// The OS resolver's addresses for `host`. `localhost` is answered here
/// as 127.0.0.1 (the relays bind IPv4), so it never reaches a nameserver.
async fn lookup(host: String) -> Result<Vec<IpAddr>, DnsError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![ip]);
    }
    if is_refused_name(&host) {
        return Err(refused(format!("{host}: not resolved (no third party)")));
    }
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    if h == "localhost" {
        return Ok(vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]);
    }
    let addrs = tokio::net::lookup_host((h.as_str(), 0))
        .await
        .map_err(|e| refused(format!("{host}: {e}")))?;
    Ok(addrs.map(|a| a.ip()).collect())
}

impl Resolver for LocalDns {
    fn lookup_ipv4(&self, host: String) -> BoxFuture<Result<BoxIter<Ipv4Addr>, DnsError>> {
        self.log.push(&host, "a");
        Box::pin(async move {
            let v: Vec<Ipv4Addr> = lookup(host.clone())
                .await?
                .into_iter()
                .filter_map(|ip| match ip {
                    IpAddr::V4(v4) => Some(v4),
                    IpAddr::V6(_) => None,
                })
                .collect();
            if v.is_empty() {
                return Err(refused(format!("{host}: no IPv4 address")));
            }
            Ok(Box::new(v.into_iter()) as BoxIter<Ipv4Addr>)
        })
    }

    fn lookup_ipv6(&self, host: String) -> BoxFuture<Result<BoxIter<Ipv6Addr>, DnsError>> {
        self.log.push(&host, "aaaa");
        Box::pin(async move {
            let v: Vec<Ipv6Addr> = lookup(host.clone())
                .await?
                .into_iter()
                .filter_map(|ip| match ip {
                    IpAddr::V6(v6) => Some(v6),
                    IpAddr::V4(_) => None,
                })
                .collect();
            if v.is_empty() {
                return Err(refused(format!("{host}: no IPv6 address")));
            }
            Ok(Box::new(v.into_iter()) as BoxIter<Ipv6Addr>)
        })
    }

    fn lookup_txt(&self, host: String) -> BoxFuture<Result<BoxIter<TxtRecordData>, DnsError>> {
        self.log.push(&host, "txt");
        Box::pin(async move {
            Err(refused(format!(
                "{host}: TXT lookups are off (no DNS address lookup)"
            )))
        })
    }

    fn clear_cache(&self) {}

    fn reset(&self) -> Box<dyn Resolver> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn localhost_ips_and_refusals() {
        let log = DnsLog::default();
        let r = LocalDns::resolver(log.clone());
        let v4: Vec<_> = r
            .lookup_ipv4("localhost", std::time::Duration::from_secs(1))
            .await
            .unwrap()
            .collect();
        assert_eq!(v4, vec![Ipv4Addr::LOCALHOST]);
        assert!(
            r.lookup_ipv4("dns.iroh.link", std::time::Duration::from_secs(1))
                .await
                .is_err()
        );
        assert!(
            r.lookup_txt("_iroh.x.dns.iroh.link", std::time::Duration::from_secs(1))
                .await
                .is_err()
        );
        assert!(is_refused_name("USE1-1.relay.n0.iroh.link."));
        assert!(!is_refused_name("pc.example"));
        let asked = log.asked();
        assert_eq!(asked[0], ("localhost".to_string(), "a"));
        assert!(asked.iter().any(|(_, k)| *k == "txt"));
    }
}
