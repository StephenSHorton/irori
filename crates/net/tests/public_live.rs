//! Card #328, live: two relay-only endpoints, built with Irori's relay TLS
//! (WebPKI for n0's hosts) and DNS (only those four names), meet through
//! n0's public relays with no address but the four relay URLs.
//! Needs the internet, so it's ignored by default:
//! `cargo test -p irori-net --test public_live -- --ignored --nocapture`

use std::time::Duration;

use iroh::endpoint::Builder;
use iroh::{Endpoint, EndpointAddr, RelayMap, RelayMode, SecretKey, Watcher as _};
use irori_net::dns::{DnsLog, LocalDns};
use irori_net::public::{self, PublicRelays};
use irori_net::tls::{PinSet, shared_ca_config_public};

const ALPN: &[u8] = b"irori/public-live/0";

async fn endpoint(seed: u8, log: DnsLog) -> Endpoint {
    let flag = PublicRelays::new(true);
    Builder::empty()
        .crypto_provider(irori_net::tls::provider())
        .secret_key(SecretKey::from_bytes(&[seed; 32]))
        .alpns(vec![ALPN.to_vec()])
        .ca_tls_config(shared_ca_config_public(PinSet::default(), flag.clone()))
        .dns_resolver(LocalDns::resolver_public(log, flag))
        .relay_mode(RelayMode::Custom(RelayMap::from_iter(public::configs())))
        .clear_ip_transports()
        .bind()
        .await
        .expect("bind")
}

#[tokio::test]
#[ignore = "needs the internet (n0's public relays)"]
async fn two_endpoints_meet_through_n0() {
    let (la, lb) = (DnsLog::default(), DnsLog::default());
    let a = endpoint(1, la.clone()).await;
    let b = endpoint(2, lb.clone()).await;
    tokio::time::timeout(Duration::from_secs(30), b.online())
        .await
        .expect("b reaches an n0 relay");
    let home: Vec<_> = b
        .home_relay_status()
        .get()
        .iter()
        .map(|s| s.url().to_string())
        .collect();
    println!("b homes on {home:?}");

    let accept = {
        let b = b.clone();
        tokio::spawn(async move {
            let inc = b.accept().await.expect("incoming");
            let conn = inc.await.expect("accept");
            let mut r = conn.accept_uni().await.expect("uni");
            let got = r.read_to_end(64).await.expect("read");
            (got, conn)
        })
    };

    // Only B's id and the four relay URLs: no directory says which one.
    let addr = public::with_public(EndpointAddr::new(b.id()), true);
    let conn = tokio::time::timeout(Duration::from_secs(30), a.connect(addr, ALPN))
        .await
        .expect("connect in time")
        .expect("connect");
    let mut s = conn.open_uni().await.unwrap();
    s.write_all(b"hello over n0").await.unwrap();
    s.finish().unwrap();
    let got = tokio::time::timeout(Duration::from_secs(15), accept)
        .await
        .expect("read in time")
        .unwrap();
    let (got, _bconn) = got;
    println!("a to b: {:?}", irori_net::path::report(&conn));
    conn.close(0u32.into(), b"done");
    assert_eq!(got, b"hello over n0");

    for (who, log) in [("a", &la), ("b", &lb)] {
        for (name, kind) in log.asked() {
            assert!(
                public::is_public_host(&name) && kind != "txt",
                "{who} asked {kind} {name}"
            );
        }
    }
    a.close().await;
    b.close().await;
}
