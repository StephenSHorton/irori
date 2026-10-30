//! A Core with no network, for rehearsals on copies of a real data dir
//! (architecture.md 5.2, `tools/rehearse`).
//!
//! A copy holds this device's real key and its Chain names the other
//! devices, so a Core started on it with the default transport would join
//! the mesh and dial them as this device, forking its history. The
//! transport [`offline_net`] builds is closed before anything binds: every
//! bind fails ("transport closed"), so no socket opens, nothing is dialed
//! and no LAN lookup or relay runs. Pairing windows fail the same way. With
//! [`crate::Options::no_home`] set too, the Core opens no port but the
//! loopback API the host serves.

use std::sync::Arc;

use crate::devices::PairNetFn;

/// The env var that asks the app for an offline Core (`irori serve
/// --headless` and the window): `1` turns the network off.
pub const ENV: &str = "IRORI_DEV_OFFLINE";

/// Whether `value` (the [`ENV`] value) asks for an offline Core.
pub fn requested(value: Option<&str>) -> bool {
    value == Some("1")
}

/// A transport builder whose transport never binds.
pub fn offline_net() -> PairNetFn {
    Arc::new(|ident, chain, guests| {
        let key = iroh::SecretKey::from_bytes(&ident.expose_device_seed());
        let boards = irori_net::board::BoardMembers::new();
        let admit = Arc::new(irori_net::board::MemberAdmit {
            chain: irori_net::admit::ChainAdmit {
                chain: chain.clone(),
                self_id: ident.device_id(),
            },
            boards: boards.clone(),
        });
        let mut cfg = irori_net::transport::NetConfig::new(ident.device_id(), key, admit);
        cfg.boards = boards;
        cfg.ip = irori_net::endpoint::IpMode::Loopback;
        cfg.gossip = false;
        cfg.lan = None;
        cfg.guests = guests.clone();
        let net = irori_net::transport::IrohNet::new(cfg);
        close_now(&net);
        net
    })
}

/// Closes a transport that never bound. Its `close` then has nothing to
/// await (no endpoint, gossip, rooms or LAN lookup), so one poll finishes
/// it without a runtime.
fn close_now(net: &irori_net::transport::IrohNet) {
    let mut fut = std::pin::pin!(net.close());
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(
        fut.as_mut().poll(&mut cx).is_ready(),
        "offline transport: close did not finish at once"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_turns_it_on() {
        assert!(requested(Some("1")));
        for v in [None, Some(""), Some("0"), Some("true")] {
            assert!(!requested(v), "{v:?}");
        }
    }
}
