//! The invite room on loopback iroh (card #44): two made-up devices of
//! different people, each endpoint admitting nobody it doesn't know, meet on
//! `irori/invite/1` only through an invite window and its room key. Every
//! socket is on 127.0.0.1.
//!
//! - the invitee dials the inviter with the room key; both get a
//!   connection with the same fingerprints, and messages move both ways;
//! - a dialer with another code is refused and the invite stays open;
//! - with no invite open the endpoint refuses the ALPN;
//! - the first dialer takes the invite: a second one is refused.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::EndpointId;
use irori_meshseam::{Conn, Secret};
use irori_net::endpoint::IpMode;
use irori_net::invite::{InviteAddr, InviteHandler};
use irori_net::transport::{IrohNet, NetConfig};

const WAIT: Duration = Duration::from_secs(20);

/// The gate of a device that knows nobody: only the invite window lets a
/// stranger in.
#[derive(Debug)]
struct Nobody;

impl irori_net::admit::Admit for Nobody {
    fn admits(&self, _alpn: &[u8], _id: &EndpointId) -> bool {
        false
    }
}

struct Dev {
    net: IrohNet,
    id: EndpointId,
}

fn dev(name: &str) -> Dev {
    let mut s = [0u8; 32];
    getrandom::fill(&mut s).unwrap();
    let key = iroh::SecretKey::from_bytes(&s);
    let id = key.public();
    let mut cfg = NetConfig::new(name, key, Arc::new(Nobody));
    cfg.ip = IpMode::Loopback;
    cfg.gossip = false;
    cfg.connect_timeout = Duration::from_secs(10);
    Dev {
        net: IrohNet::new(cfg),
        id,
    }
}

fn room(n: u8) -> Secret {
    Secret {
        key: [n; 32],
        topic: format!("irori-invite-v1/test-{n}"),
        epoch: 0,
    }
}

impl Dev {
    async fn addr(&self) -> InviteAddr {
        let ep = self.net.bind().await.unwrap();
        let direct = ep.bound_sockets();
        for a in &direct {
            assert!(a.ip().is_loopback(), "non-loopback socket {a}");
        }
        InviteAddr {
            id: self.id,
            relay: None,
            relay_quic_port: 0,
            relay_pub: None,
            direct,
        }
    }

    /// Opens an invite for `room`; what the handler is handed lands in the
    /// returned slot.
    fn invite(&self, room: Secret) -> Arc<Mutex<Vec<Arc<dyn Conn>>>> {
        let got: Arc<Mutex<Vec<Arc<dyn Conn>>>> = Arc::default();
        let g = got.clone();
        let h: InviteHandler = Arc::new(move |c| g.lock().unwrap().push(c));
        self.net.set_invite_handler(Some(room), Some(h)).unwrap();
        got
    }
}

async fn handed(got: &Arc<Mutex<Vec<Arc<dyn Conn>>>>) -> Arc<dyn Conn> {
    let start = std::time::Instant::now();
    loop {
        if let Some(c) = got.lock().unwrap().first().cloned() {
            return c;
        }
        assert!(
            start.elapsed() < WAIT,
            "the inviter was never handed a connection"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_invitee_with_the_code_reaches_the_inviter() {
    let (a, b) = (dev("inviter-dev"), dev("invitee-dev"));
    let got = a.invite(room(1));
    let to = a.addr().await;
    let cb = b.net.dial_invite(&to, &room(1)).await.unwrap();
    let ca = handed(&got).await;
    assert_eq!(cb.id(), "inviter-dev");
    assert_eq!(ca.id(), "invitee-dev");
    assert_eq!(ca.fingerprints(), cb.fingerprints());
    assert!(!ca.fingerprints().0.is_empty());
    cb.send(b"hello from b").await.unwrap();
    assert_eq!(ca.recv().await.unwrap(), b"hello from b");
    ca.send(b"hello from a").await.unwrap();
    assert_eq!(cb.recv().await.unwrap(), b"hello from a");
    a.net.close().await;
    b.net.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn another_code_is_refused_and_the_invite_stays_open() {
    let (a, b) = (dev("inviter-dev"), dev("invitee-dev"));
    let got = a.invite(room(1));
    let to = a.addr().await;
    assert!(b.net.dial_invite(&to, &room(2)).await.is_err());
    assert!(got.lock().unwrap().is_empty());
    let cb = b.net.dial_invite(&to, &room(1)).await.unwrap();
    let ca = handed(&got).await;
    assert_eq!(ca.fingerprints(), cb.fingerprints());
    a.net.close().await;
    b.net.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_invite_open_refuses_the_alpn() {
    let (a, b) = (dev("inviter-dev"), dev("invitee-dev"));
    let to = a.addr().await;
    assert!(b.net.dial_invite(&to, &room(1)).await.is_err());
    // Opened and stopped again: still refused.
    let got = a.invite(room(1));
    a.net.set_invite_handler(None, None).unwrap();
    assert!(b.net.dial_invite(&to, &room(1)).await.is_err());
    assert!(got.lock().unwrap().is_empty());
    a.net.close().await;
    b.net.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_first_invitee_takes_the_invite() {
    let (a, b, c) = (dev("inviter-dev"), dev("invitee-dev"), dev("late-dev"));
    let got = a.invite(room(1));
    let to = a.addr().await;
    let _cb = b.net.dial_invite(&to, &room(1)).await.unwrap();
    handed(&got).await;
    assert!(c.net.dial_invite(&to, &room(1)).await.is_err());
    assert_eq!(got.lock().unwrap().len(), 1);
    for d in [a, b, c] {
        d.net.close().await;
    }
}
