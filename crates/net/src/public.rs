//! n0's public iroh relays as the meeting point (card #328,
//! docs/design/public-relays.md).
//!
//! With the switch on, every endpoint's relay map holds these four relays
//! and nothing else, so each device homes on the nearest one and can be
//! reached there from any network. Every dial adds all four to the peer's
//! address, since there is no directory saying which one it homes on. Only
//! the relays are used: n0's DNS/pkarr address lookup stays off, and
//! [`crate::dns`] resolves these four host names and no other n0 name.
//!
//! The switch is a shared [`PublicRelays`] flag, off by default so tests
//! and a fresh `NetConfig` never touch the internet.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use iroh::{EndpointAddr, RelayConfig, RelayUrl};

/// The four n0 production relay hosts (iroh 1.4.1 `defaults::prod`).
pub const HOSTS: [&str; 4] = [
    "use1-1.relay.n0.iroh.link",
    "usw1-1.relay.n0.iroh.link",
    "euc1-1.relay.n0.iroh.link",
    "aps1-1.relay.n0.iroh.link",
];

/// Whether public relays are on. Cheap to clone; every clone sees `set`.
#[derive(Debug, Clone, Default)]
pub struct PublicRelays(Arc<AtomicBool>);

impl PublicRelays {
    pub fn new(on: bool) -> Self {
        Self(Arc::new(AtomicBool::new(on)))
    }
    pub fn on(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    pub fn set(&self, on: bool) {
        self.0.store(on, Ordering::SeqCst);
    }
}

/// True when `host` is one of [`HOSTS`] (case-insensitive, trailing dot
/// allowed).
pub fn is_public_host(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    HOSTS.contains(&h.as_str())
}

/// The relay configs for the map: n0's own (QAD on its default port).
pub fn configs() -> Vec<RelayConfig> {
    iroh::defaults::prod::default_relay_map()
        .relays::<Vec<_>>()
        .into_iter()
        .map(|c| (*c).clone())
        .collect()
}

/// The four relay URLs.
pub fn urls() -> Vec<RelayUrl> {
    configs().into_iter().map(|c| c.url).collect()
}

/// True when `url` is one of n0's public relays.
pub fn is_public_url(url: &RelayUrl) -> bool {
    url.host_str().is_some_and(is_public_host)
}

/// `addr` with the four public relays added when `on`: one of them is the
/// peer's home relay, and iroh tries every relay address it is given.
pub fn with_public(addr: EndpointAddr, on: bool) -> EndpointAddr {
    if !on {
        return addr;
    }
    urls().into_iter().fold(addr, |a, u| a.with_relay_url(u))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_match_iroh_defaults() {
        let got: Vec<String> = urls()
            .iter()
            .map(|u| u.host_str().unwrap().trim_end_matches('.').to_string())
            .collect();
        for h in HOSTS {
            assert!(
                got.iter().any(|g| g == h),
                "{h} not in iroh's defaults: {got:?}"
            );
        }
        assert_eq!(got.len(), 4);
    }

    #[test]
    fn host_match_is_exact() {
        assert!(is_public_host("USE1-1.relay.n0.iroh.link."));
        assert!(!is_public_host("dns.iroh.link"));
        assert!(!is_public_host("evil.use1-1.relay.n0.iroh.link"));
        assert!(!is_public_host("pc.example"));
    }

    #[test]
    fn with_public_adds_four_only_when_on() {
        let id = iroh::SecretKey::from_bytes(&[7u8; 32]).public();
        let a = EndpointAddr::new(id);
        assert_eq!(with_public(a.clone(), false).relay_urls().count(), 0);
        assert_eq!(with_public(a, true).relay_urls().count(), 4);
    }
}
