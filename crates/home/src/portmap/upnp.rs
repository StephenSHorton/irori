//! UPnP-IGD through igd-next (R:portmap/upnp.go), and the default
//! discovery: an IGD by SSDP at the default gateway, then NAT-PMP there.
//!
//! Only an IGD at the default gateway's own address is kept: any host on
//! the LAN can answer SSDP, and one that isn't the router could otherwise
//! take the mapping and name any address as the "external" one.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use igd_next::aio::Gateway as IgdGateway;
use igd_next::aio::tokio::Tokio;
use igd_next::{AddPortError, PortMappingProtocol, SearchOptions};

use super::natpmp::{self, NatPmp};
use super::{BoxFut, DiscoverFn, Gateway, MapError, Port};

/// A [`Gateway`] on one IGD's WAN connection service.
pub struct Upnp {
    gw: IgdGateway<Tokio>,
    /// This machine's address on the router's network.
    local: Ipv4Addr,
    desc: String,
}

fn proto(p: &str) -> PortMappingProtocol {
    if p == "tcp" {
        PortMappingProtocol::TCP
    } else {
        PortMappingProtocol::UDP
    }
}

impl Gateway for Upnp {
    fn method(&self) -> &str {
        "upnp"
    }

    fn external_ip(&self) -> BoxFut<'_, Result<IpAddr, String>> {
        Box::pin(async move { self.gw.get_external_ip().await.map_err(|e| e.to_string()) })
    }

    fn map(&self, mut p: Port, life: Duration) -> BoxFut<'_, Result<(Port, Duration), MapError>> {
        Box::pin(async move {
            if p.external == 0 {
                p.external = p.internal;
            }
            let local = SocketAddr::new(IpAddr::V4(self.local), p.internal);
            let mut lease = life.as_secs() as u32;
            let mut r = self
                .gw
                .add_port(proto(p.proto), p.external, local, lease, &self.desc)
                .await;
            if matches!(r, Err(AddPortError::OnlyPermanentLeasesSupported)) {
                lease = 0;
                r = self
                    .gw
                    .add_port(proto(p.proto), p.external, local, 0, &self.desc)
                    .await;
            }
            match r {
                Ok(()) => Ok((p, Duration::from_secs(u64::from(lease)))),
                Err(AddPortError::PortInUse) => Err(MapError::Conflict),
                Err(e) => Err(MapError::Other(e.to_string())),
            }
        })
    }

    fn unmap(&self, p: Port) -> BoxFut<'_, Result<(), String>> {
        Box::pin(async move {
            self.gw
                .remove_port(proto(p.proto), p.external)
                .await
                .map_err(|e| e.to_string())
        })
    }

    /// Removes a mapping an earlier run left (a crash leaves a permanent one
    /// forever), only if the router lists it as pointing at this machine
    /// with this app's description. igd-next has no
    /// GetSpecificPortMappingEntry, so the generic list is walked.
    fn unmap_stale(&self, p: Port) -> Option<BoxFut<'_, Result<(), String>>> {
        Some(Box::pin(async move {
            for i in 0..256u32 {
                let Ok(e) = self.gw.get_generic_port_mapping_entry(i).await else {
                    return Ok(()); // end of the list (or the router won't say)
                };
                let same_proto = e.protocol == proto(p.proto);
                if e.external_port != p.external || !same_proto {
                    continue;
                }
                if e.internal_client != self.local.to_string()
                    || e.port_mapping_description != self.desc
                {
                    return Ok(()); // someone else's
                }
                return self
                    .gw
                    .remove_port(proto(p.proto), p.external)
                    .await
                    .map_err(|e| e.to_string());
            }
            Ok(())
        }))
    }
}

/// The address this machine uses to reach `gw`: a connected UDP socket
/// sends nothing, it only picks the route.
fn local_addr_for(gw: Ipv4Addr) -> Option<Ipv4Addr> {
    let c = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    c.connect((gw, 1900)).ok()?;
    match c.local_addr().ok()?.ip() {
        IpAddr::V4(v4) => Some(v4),
        IpAddr::V6(_) => None,
    }
}

/// Finds an IGD by SSDP from the interface that reaches `gw`, kept only if
/// it answers from `gw` itself. The app only, never tests.
async fn discover_upnp(desc: &str, gw: Ipv4Addr) -> Vec<Arc<dyn Gateway>> {
    let Some(local) = local_addr_for(gw) else {
        return vec![];
    };
    let opts = SearchOptions {
        bind_addr: SocketAddr::new(IpAddr::V4(local), 0),
        timeout: Some(Duration::from_secs(2)),
        single_search_timeout: Some(Duration::from_secs(2)),
        ..SearchOptions::default()
    };
    match igd_next::aio::tokio::search_gateway(opts).await {
        Ok(g) if g.addr.ip() == IpAddr::V4(gw) => vec![Arc::new(Upnp {
            gw: g,
            local,
            desc: desc.to_string(),
        }) as Arc<dyn Gateway>],
        _ => vec![],
    }
}

/// `Config::discover`'s default: an IGD by SSDP at the default gateway, then
/// NAT-PMP there.
pub fn discover_default(desc: String) -> DiscoverFn {
    Arc::new(move || {
        let desc = desc.clone();
        Box::pin(async move {
            let gw = irori_lan::netif::default_gateway_v4()
                .ok_or_else(|| "no default gateway".to_string())?;
            let mut gws = discover_upnp(&desc, gw).await;
            gws.push(Arc::new(NatPmp::new(SocketAddr::new(
                IpAddr::V4(gw),
                natpmp::PORT,
            ))));
            Ok(gws)
        })
    })
}
