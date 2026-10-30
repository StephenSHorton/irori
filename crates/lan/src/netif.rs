//! This machine's networks, as go-room and the home server read them:
//! private IPv4 subnets on up interfaces (R:lan.go:439-474), their directed
//! broadcast addresses minus Windows Public networks (R:lan.go:476-504,
//! R:lan_windows.go), the default gateway (I:internal/home/home.go:821-875)
//! and global IPv6 addresses (home.go:877-902).

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One up IPv4 interface address with a private address (Go `LocalSubnet`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalSubnet {
    /// Interface index.
    pub index: u32,
    pub addr: Ipv4Addr,
    /// Prefix length, 8..=30.
    pub bits: u8,
}

impl LocalSubnet {
    /// Whether `ip` is inside this subnet.
    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        let mask = mask(self.bits);
        u32::from(self.addr) & mask == u32::from(ip) & mask
    }

    /// The subnet's directed broadcast address (192.168.1.255 for
    /// 192.168.1.20/24).
    pub fn broadcast(&self) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.addr) | !mask(self.bits))
    }
}

fn mask(bits: u8) -> u32 {
    if bits == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(bits))
    }
}

/// Go `netip.Addr.IsPrivate` for IPv4 (RFC 1918).
pub fn is_private_v4(ip: Ipv4Addr) -> bool {
    ip.is_private()
}

/// Go `netip.Addr.IsPrivate` for IPv6 (fc00::/7).
pub fn is_private_v6(ip: Ipv6Addr) -> bool {
    ip.segments()[0] & 0xfe00 == 0xfc00
}

/// Go `netip.Addr.IsGlobalUnicast` for IPv6: not unspecified, loopback,
/// multicast or link-local unicast.
pub fn is_global_unicast_v6(ip: Ipv6Addr) -> bool {
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || ip.segments()[0] & 0xffc0 == 0xfe80)
}

/// The up, non-loopback interfaces' private IPv4 addresses with a /8../30
/// prefix (Go `LocalSubnets`).
pub fn local_subnets() -> Vec<LocalSubnet> {
    let mut out = Vec::new();
    for ifc in netdev::get_interfaces() {
        if !ifc.is_up() || ifc.is_loopback() {
            continue;
        }
        for n in &ifc.ipv4 {
            let ip = n.addr();
            let bits = n.prefix_len();
            if !is_private_v4(ip) || !(8..=30).contains(&bits) {
                continue;
            }
            out.push(LocalSubnet {
                index: ifc.index,
                addr: ip,
                bits,
            });
        }
    }
    out
}

/// This machine's global IPv6 addresses (not link-local, not unique-local,
/// not 4in6) on up, non-loopback interfaces (Go `v6Addrs`).
pub fn v6_addrs() -> Vec<Ipv6Addr> {
    let mut out: Vec<Ipv6Addr> = Vec::new();
    for ifc in netdev::get_interfaces() {
        if !ifc.is_up() || ifc.is_loopback() {
            continue;
        }
        for n in &ifc.ipv6 {
            let ip = n.addr();
            if ip.to_ipv4_mapped().is_none()
                && is_global_unicast_v6(ip)
                && !is_private_v6(ip)
                && !out.contains(&ip)
            {
                out.push(ip);
            }
        }
    }
    out
}

/// The default gateway's IPv4 address, if one is known.
pub fn default_gateway_v4() -> Option<Ipv4Addr> {
    let gw = netdev::get_default_gateway().ok()?;
    gw.ipv4.first().copied()
}

static GW_CACHE: Mutex<Option<(Vec<Ipv4Addr>, Ipv4Addr)>> = Mutex::new(None);

/// The default gateway, looked up again only when `addrs` change; a failed
/// lookup is not cached (Go `gatewayFor`).
fn gateway_for(addrs: &[Ipv4Addr]) -> Option<Ipv4Addr> {
    let mut c = GW_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((key, gw)) = c.as_ref()
        && key.as_slice() == addrs
    {
        return Some(*gw);
    }
    let gw = default_gateway_v4()?;
    *c = Some((addrs.to_vec(), gw));
    Some(gw)
}

/// This machine's private IPv4 addresses on up interfaces on the default
/// gateway's subnet (all of them while the gateway is unknown): virtual
/// adapters (Hyper-V, WSL, Docker, VPNs) have private addresses too, which
/// devices on the home network can't reach (Go `lanAddrs`).
pub fn lan_addrs() -> Vec<Ipv4Addr> {
    let subs = local_subnets();
    let mut out: Vec<Ipv4Addr> = Vec::new();
    for s in &subs {
        if !out.contains(&s.addr) {
            out.push(s.addr);
        }
    }
    if let Some(gw) = gateway_for(&out) {
        let on_gw: Vec<Ipv4Addr> = subs
            .iter()
            .filter(|s| s.contains(gw))
            .map(|s| s.addr)
            .collect();
        if !on_gw.is_empty() {
            return on_gw;
        }
    }
    out
}

/// Where an announcement for everyone goes: each private subnet's directed
/// broadcast address on `port` (not 255.255.255.255, which Windows sends
/// out of one interface only), minus networks on the Public profile (Go
/// `broadcastTargets`).
pub fn broadcast_targets(port: u16) -> Vec<SocketAddr> {
    let public = public_interfaces();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for s in local_subnets() {
        if public.contains(&s.index) {
            continue;
        }
        let b = s.broadcast();
        if seen.insert(b) {
            out.push(SocketAddr::V4(SocketAddrV4::new(b, port)));
        }
    }
    out
}

/// Whether `ip` is this machine's address on a network Windows has on the
/// Public firewall profile (always false elsewhere), where the LAN lookup
/// stays silent and Windows Firewall blocks what a server listens for (Go
/// `OnPublicNetwork`).
pub fn on_public_network(ip: IpAddr) -> bool {
    let ip = match ip {
        IpAddr::V4(v4) => v4,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4,
            None => return false,
        },
    };
    let public = public_interfaces();
    local_subnets()
        .iter()
        .find(|s| s.addr == ip)
        .is_some_and(|s| public.contains(&s.index))
}

static PUBLIC_CACHE: Mutex<Option<(Instant, BTreeSet<u32>)>> = Mutex::new(None);

/// The indexes of interfaces on the Public profile, cached for 30 s. If the
/// Network List Manager can't be read, nothing is skipped: a home LAN that
/// goes silent for a COM error would be worse than the rare café that hears
/// a sealed, tag-rotating datagram (R:lan_windows.go:62-90).
pub fn public_interfaces() -> BTreeSet<u32> {
    let mut c = PUBLIC_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, set)) = c.as_ref()
        && at.elapsed() < Duration::from_secs(30)
    {
        return set.clone();
    }
    let mut set = BTreeSet::new();
    let guids = public_adapter_guids();
    if !guids.is_empty() {
        let by_name: BTreeMap<String, u32> = netdev::get_interfaces()
            .into_iter()
            .map(|i| (i.name.to_ascii_uppercase(), i.index))
            .collect();
        for g in guids {
            if let Some(idx) = by_name.get(&g) {
                set.insert(*idx);
            }
        }
    }
    *c = Some((Instant::now(), set.clone()));
    set
}

/// The adapter GUIDs (`{XXXXXXXX-...}`, upper case, which is the adapter
/// name netdev reports on Windows) whose network is on the Public profile.
#[cfg(windows)]
fn public_adapter_guids() -> Vec<String> {
    // COM runs on its own short-lived thread, so the caller's apartment
    // (a tokio worker, the UI thread) is never touched.
    std::thread::spawn(nlm::public_adapter_guids)
        .join()
        .unwrap_or_default()
}

#[cfg(not(windows))]
fn public_adapter_guids() -> Vec<String> {
    Vec::new()
}

#[cfg(windows)]
mod nlm {
    use windows::Win32::Networking::NetworkListManager::{
        INetworkConnection, INetworkListManager, NLM_NETWORK_CATEGORY_PUBLIC, NetworkListManager,
    };
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
    };

    pub(super) fn public_adapter_guids() -> Vec<String> {
        // SAFETY: COM is initialized on this fresh thread and torn down
        // before it returns; every interface is dropped before CoUninitialize.
        unsafe {
            if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
                return Vec::new();
            }
            let out = read().unwrap_or_default();
            CoUninitialize();
            out
        }
    }

    unsafe fn read() -> windows::core::Result<Vec<String>> {
        // SAFETY: called with COM initialized (see above).
        unsafe {
            let nlm: INetworkListManager = CoCreateInstance(&NetworkListManager, None, CLSCTX_ALL)?;
            let conns = nlm.GetNetworkConnections()?;
            let mut out = Vec::new();
            loop {
                let mut slot: [Option<INetworkConnection>; 1] = [None];
                let mut fetched = 0u32;
                if conns.Next(&mut slot, Some(&mut fetched)).is_err() || fetched == 0 {
                    break;
                }
                let Some(c) = slot[0].take() else { break };
                let Ok(net) = c.GetNetwork() else { continue };
                let Ok(cat) = net.GetCategory() else { continue };
                if cat != NLM_NETWORK_CATEGORY_PUBLIC {
                    continue;
                }
                if let Ok(g) = c.GetAdapterId() {
                    out.push(format!("{{{g:?}}}").to_ascii_uppercase());
                }
            }
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcast_and_contains() {
        let s = LocalSubnet {
            index: 1,
            addr: Ipv4Addr::new(192, 168, 1, 20),
            bits: 24,
        };
        assert_eq!(s.broadcast(), Ipv4Addr::new(192, 168, 1, 255));
        assert!(s.contains(Ipv4Addr::new(192, 168, 1, 1)));
        assert!(!s.contains(Ipv4Addr::new(192, 168, 2, 1)));
        let s = LocalSubnet {
            index: 1,
            addr: Ipv4Addr::new(10, 1, 2, 3),
            bits: 8,
        };
        assert_eq!(s.broadcast(), Ipv4Addr::new(10, 255, 255, 255));
    }

    #[test]
    fn v6_classes() {
        assert!(is_global_unicast_v6("2001:db8::7".parse().unwrap()));
        assert!(!is_global_unicast_v6("fe80::1".parse().unwrap()));
        assert!(is_private_v6("fd00::1".parse().unwrap()));
        assert!(!is_private_v6("2001:db8::1".parse().unwrap()));
    }

    #[test]
    fn reading_the_machine_does_not_panic() {
        let _ = local_subnets();
        let _ = v6_addrs();
        let _ = lan_addrs();
        let _ = broadcast_targets(38419);
        let _ = on_public_network(IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
}
