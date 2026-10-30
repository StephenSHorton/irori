//! Grok member streams through the real API server and the Core's member
//! table (I:internal/core/v02_test.go TestGrokMember,
//! TestGrokMemberSupersede): `POST /v1/members`, then `GET
//! /v1/attach?member=`. Go moves the member clock past the reattach grace;
//! here the rig's Core runs with a short member grace instead.

mod common;

use std::time::{Duration, Instant};

use attach::{HEADER_ATTACH, HEADER_MEMBER};
use common::{Rig, str_of, strs};
use inbox::{Address, addr, st};
use serde_json::{Value, json};

const GRACE: Duration = Duration::from_millis(1500);

async fn rig() -> Rig {
    Rig::with(Duration::ZERO, |o| o.member_grace = GRACE).await
}

async fn join(r: &Rig, name: &str, session: &str) -> Value {
    let (code, raw) = r
        .do_(
            "POST",
            "/v1/members",
            &[],
            Some(json!({"name": name, "session": session})),
        )
        .await;
    let m: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    assert!(
        code == 200 && !str_of(&m, "memberSecret").is_empty(),
        "join: {code} {raw}"
    );
    m
}

fn listed(r: &Rig, key: &str) -> Option<presence::AgentCard> {
    r.core.local_agents().into_iter().find(|c| c.key == key)
}

async fn wait_closed(r: &Rig, key: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while r.core.member_open_streams(key) != 0 {
        assert!(
            Instant::now() < deadline,
            "open count {} after the member streams closed",
            r.core.member_open_streams(key)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_unlisted(r: &Rig, key: &str) {
    let deadline = Instant::now() + GRACE + Duration::from_secs(5);
    while listed(r, key).is_some() {
        assert!(Instant::now() < deadline, "grok card outlived the grace");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn grok_member() {
    let r = rig().await;
    let m1 = join(&r, "grokky", "grok-session-1").await;
    let m = join(&r, "grokky", "grok-session-1").await;
    let (key, secret) = (
        str_of(&m, "memberKey").to_string(),
        str_of(&m, "memberSecret").to_string(),
    );
    assert!(
        key == str_of(&m1, "memberKey")
            && secret != str_of(&m1, "memberSecret")
            && key.starts_with(&format!("{}/grok-", r.core.device_id())),
        "rejoin {m} after {m1}"
    );
    let (code, _) = r
        .do_(
            "GET",
            "/v1/inbox",
            &[(HEADER_MEMBER, str_of(&m1, "memberSecret"))],
            None,
        )
        .await;
    assert_eq!(code, 403, "retired member secret");
    assert!(
        listed(&r, &key).is_none(),
        "grok card listed after a join with no stream"
    );

    let mut s = r
        .open(&format!("member={secret}"))
        .await
        .unwrap_or_else(|c| panic!("member stream: {c}"));
    let hello = s.next("hello").await;
    assert!(
        str_of(&hello, "agentKey") == key
            && str_of(&hello, "name") == "grokky"
            && hello.get("attachSecret").is_none(),
        "member hello {hello}"
    );
    let c = listed(&r, &key).expect("a grok card while the stream is open");
    assert!(
        c.agent == "grok" && c.status == "idle" && c.name == "grokky",
        "card {c:?}"
    );
    assert!(!r.host.server.is_attach_secret(&secret));

    let (_, d) = r
        .core
        .inbox()
        .send(
            &r.sender(&r.key_a, "alpha"),
            &Address::new(addr::AGENT, "grokky"),
            "hello grok 5c1d",
            "",
        )
        .unwrap();
    let ev = s.next("deliver").await;
    assert!(
        strs(&ev, "deliveryIds") == vec![d.id.clone()]
            && str_of(&ev, "prompt").contains("hello grok 5c1d")
            && str_of(&ev, "from") == "alpha",
        "member deliver {ev}"
    );
    assert_eq!(
        r.state(&d.id),
        st::QUEUED,
        "a channel push isn't delivered yet"
    );
    // Control routes are for the mod only.
    let (code, _) = r
        .do_(
            "POST",
            "/v1/deliveries/delivered",
            &[(HEADER_MEMBER, &secret)],
            Some(json!({"token": str_of(&ev, "token")})),
        )
        .await;
    assert_eq!(code, 403, "member on control route");
    let (code, raw) = r
        .do_("GET", "/v1/inbox", &[(HEADER_MEMBER, &secret)], None)
        .await;
    assert!(
        code == 200 && raw.contains(&d.id),
        "member inbox: {code} {raw}"
    );
    r.core.inbox().ack(&key, &d.id).unwrap();
    assert_eq!(r.state(&d.id), st::ENGAGED, "after ack");

    // The card outlives the stream by the grace only.
    s.close();
    wait_closed(&r, &key).await;
    assert!(
        listed(&r, &key).is_some(),
        "grok card gone before the grace ran out"
    );
    wait_unlisted(&r, &key).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn grok_member_supersede() {
    let r = rig().await;
    let m = join(&r, "grokky", "grok-session-2").await;
    let (key, secret) = (
        str_of(&m, "memberKey").to_string(),
        str_of(&m, "memberSecret").to_string(),
    );
    let q = format!("member={secret}");
    let mut s1 = r.open(&q).await.unwrap_or_else(|c| panic!("stream 1: {c}"));
    s1.next("hello").await;
    let mut s2 = r.open(&q).await.unwrap_or_else(|c| panic!("stream 2: {c}"));
    s2.next("hello").await;
    s1.ended("superseded").await;
    assert_eq!(r.host.server.open_streams(), 1);
    s2.close();
    wait_closed(&r, &key).await;
    assert_eq!(r.core.inbox().reachable(&key), (false, false));
    wait_unlisted(&r, &key).await;
    assert_eq!(r.host.server.open_streams(), 0);

    // An attach header with a member secret is not a caller.
    let (code, _) = r
        .do_("GET", "/v1/inbox", &[(HEADER_ATTACH, &secret)], None)
        .await;
    assert_eq!(code, 403);
}
