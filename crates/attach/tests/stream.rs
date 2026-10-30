//! The Go attach-stream tests over HTTP (I:internal/core/v02_test.go,
//! nextturn_test.go, untaken_test.go), through the real API server
//! (`irori_api::Server` with the Core's tools), the one `irori serve`
//! hosts.

mod common;

use std::time::Duration;

use attach::HEADER_ATTACH;
use common::{Rig, SESS_A, SESS_B, str_of, strs};
use inbox::{Address, SendOptions, addr, mode, st};
use serde_json::json;

fn to(name: &str) -> Address {
    Address::new(addr::AGENT, name)
}

#[tokio::test(flavor = "multi_thread")]
async fn two_sessions_talk() {
    let r = Rig::new().await;
    let (mut a, hello_a) = r.attach(SESS_A).await;
    let (mut b, hello_b) = r.attach(SESS_B).await;
    assert!(
        str_of(&hello_a, "agentKey") == r.key_a
            && str_of(&hello_a, "name") == "alpha"
            && str_of(&hello_b, "session") == SESS_B
            && hello_a["v"] == 2,
        "hellos {hello_a} {hello_b}"
    );
    a.next("inbox").await;
    b.next("inbox").await;
    let sa = str_of(&hello_a, "attachSecret").to_string();
    let sb = str_of(&hello_b, "attachSecret").to_string();

    // tools-send: alpha's irori_send through POST /v1/tools/send wakes beta.
    let (code, raw) = r
        .do_(
            "POST",
            "/v1/tools/send",
            &[(HEADER_ATTACH, &sa)],
            Some(json!({"args": {"to": "beta", "text": "via tools"}})),
        )
        .await;
    assert!(
        code == 200 && raw.contains("\"text\""),
        "tools send: {code} {raw}"
    );
    let ev = b.next("deliver").await;
    let t0 = str_of(&ev, "token").to_string();
    r.control(&sb, "delivered", &t0, "t0").await;
    r.control(&sb, "engaged", &t0, "t0").await;
    r.control(&sb, "turn-ended", &t0, "t0").await;

    let ib = r.core.inbox();
    let (_, d1) = ib
        .send(
            &r.sender(&r.key_a, "alpha"),
            &to("beta"),
            "ping-7f3e: please look",
            "",
        )
        .unwrap();
    let ev = b.next("deliver").await;
    let (tok, prompt) = (str_of(&ev, "token"), str_of(&ev, "prompt"));
    assert!(
        prompt.contains(tok)
            && prompt.contains("ping-7f3e")
            && prompt.contains("<irori-message from=\"alpha\"")
            && prompt.contains("trust=\"untrusted\""),
        "prompt {prompt:?}"
    );
    assert!(
        strs(&ev, "deliveryIds") == vec![d1.id.clone()]
            && strs(&ev, "messageIds").len() == 1
            && str_of(&ev, "from") == "alpha",
        "deliver {ev}"
    );
    r.control(&sb, "delivered", tok, "").await;
    assert_eq!(r.state(&d1.id), st::DELIVERED);
    r.control(&sb, "engaged", tok, "turn-1").await;
    assert_eq!(r.state(&d1.id), st::ENGAGED);

    // B's inbox over HTTP shows it.
    let (code, raw) = r
        .do_("GET", "/v1/inbox", &[(HEADER_ATTACH, &sb)], None)
        .await;
    let list: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(
        code == 200 && list[0]["id"] == d1.id.as_str(),
        "inbox: {code} {raw}"
    );

    ib.reply(&r.sender(&r.key_b, "beta"), &d1.id, "pong-91ac")
        .unwrap();
    r.control(&sb, "turn-ended", tok, "turn-1").await;
    assert_eq!(r.state(&d1.id), st::ANSWERED);
    let back = a.next("deliver").await;
    assert!(
        str_of(&back, "prompt").contains("pong-91ac")
            && str_of(&back, "prompt").contains(str_of(&back, "token"))
            && str_of(&back, "from") == "beta",
        "reply deliver {back}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn supersede() {
    let r = Rig::new().await;
    let ib = r.core.inbox();
    let (mut b1, h1) = r.attach(SESS_B).await;
    let (_, d1) = ib
        .send(&r.sender(&r.key_a, "alpha"), &to("beta"), "first", "")
        .unwrap();
    let w1 = b1.next("deliver").await;
    let tok1 = str_of(&w1, "token").to_string();

    // A reload before the mod confirmed: the new stream gets the same wake
    // again (same token), the old one is told it was superseded.
    let (mut b2, h2) = r.attach(SESS_B).await;
    b1.ended("superseded").await;
    let again = b2.next("deliver").await;
    assert!(
        str_of(&again, "token") == tok1 && strs(&again, "deliveryIds") == vec![d1.id.clone()],
        "re-sent wake {again}, want token {tok1}"
    );
    let (code, _) = r
        .do_(
            "POST",
            "/v1/deliveries/delivered",
            &[(HEADER_ATTACH, str_of(&h1, "attachSecret"))],
            Some(json!({"token": tok1})),
        )
        .await;
    assert_eq!(code, 403, "superseded secret");
    r.control(str_of(&h2, "attachSecret"), "delivered", &tok1, "")
        .await;

    // A reload after it confirmed: nothing is re-sent.
    let (mut b3, h3) = r.attach(SESS_B).await;
    b2.ended("superseded").await;
    b3.none("deliver", Duration::from_millis(700)).await;

    // A second message waits behind the outstanding wake, then goes alone.
    let (_, d2) = ib
        .send(&r.sender(&r.key_a, "alpha"), &to("beta"), "second", "")
        .unwrap();
    b3.none("deliver", Duration::from_millis(500)).await;
    let s3 = str_of(&h3, "attachSecret").to_string();
    r.control(&s3, "engaged", &tok1, "turn-1").await;
    let w2 = b3.next("deliver").await;
    assert!(
        str_of(&w2, "token") != tok1
            && strs(&w2, "deliveryIds") == vec![d2.id.clone()]
            && !str_of(&w2, "prompt").contains("first"),
        "second wake {w2}"
    );
    assert_eq!(r.state(&d1.id), st::ENGAGED);

    // Closing the live stream invalidates its secret.
    b3.close();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let (code, _) = r
            .do_("GET", "/v1/inbox", &[(HEADER_ATTACH, &s3)], None)
            .await;
        if code == 403 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "closed stream's secret still works: {code}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(r.host.server.open_streams(), 0);
    assert_eq!(ib.reachable(&r.key_b), (false, false));
}

#[tokio::test(flavor = "multi_thread")]
async fn keepalive() {
    let r = Rig::with(Duration::from_millis(50), |_| {}).await;
    let resp = common::send(
        r.host.port,
        "GET",
        &format!("/v1/attach?session={SESS_A}"),
        &r.token(),
        &[],
        None,
    )
    .await;
    assert_eq!(
        resp.headers()["content-type"],
        "application/x-ndjson",
        "content type"
    );
    assert_eq!(resp.headers()["cache-control"], "no-store");
    assert_eq!(resp.headers()["x-content-type-options"], "nosniff");
    use http_body_util::BodyExt;
    let mut body = resp.into_body();
    let mut buf = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let f = tokio::time::timeout_at(deadline, body.frame())
            .await
            .expect("a keepalive within 10 s")
            .expect("stream open")
            .unwrap();
        let Ok(d) = f.into_data() else { continue };
        buf.extend_from_slice(&d);
        let text = String::from_utf8_lossy(&buf);
        assert!(
            text.starts_with("{\"type\":\"hello\""),
            "first line {text:?}"
        );
        if text.split_inclusive('\n').skip(1).any(|l| l == "\n") {
            return; // a bare keepalive
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn auth() {
    let r = Rig::new().await;
    let (_a, h) = r.attach(SESS_A).await;
    let secret = str_of(&h, "attachSecret").to_string();
    let hdrs: Vec<Vec<(&str, &str)>> = vec![
        vec![],
        vec![(HEADER_ATTACH, "nope")],
        vec![(attach::HEADER_MEMBER, "nope")],
        vec![(HEADER_ATTACH, &secret), (attach::HEADER_MEMBER, "x")],
    ];
    for hdr in &hdrs {
        for path in ["/v1/deliveries/delivered", "/v1/tools/send"] {
            let (code, raw) = r
                .do_("POST", path, hdr, Some(json!({"token": "[irori-wake:x]"})))
                .await;
            assert_eq!(code, 403, "{path} with {hdr:?}: {raw}");
            if path.starts_with("/v1/deliveries") {
                assert_eq!(
                    raw,
                    "{\"error\":\"this route needs a current X-Irori-Attach or X-Irori-Member secret\"}\n"
                );
            }
        }
        let (code, _) = r.do_("GET", "/v1/inbox", hdr, None).await;
        assert_eq!(code, 403, "inbox with {hdr:?}");
    }
    let (code, raw) = r
        .do_("GET", "/v1/attach?session=not-a-session", &[], None)
        .await;
    assert_eq!(code, 404, "unknown session");
    assert_eq!(
        raw,
        "{\"error\":\"no live session \\\"not-a-session\\\" on this device (yet)\"}\n"
    );
    let (code, _) = r.do_("GET", "/v1/attach?member=nope", &[], None).await;
    assert_eq!(code, 403, "unknown member");
    for q in ["", "session=a&member=b"] {
        let (code, raw) = r.do_("GET", &format!("/v1/attach?{q}"), &[], None).await;
        assert!(
            code == 400 && raw.contains("pass exactly one of session or member"),
            "{q}: {code} {raw}"
        );
    }
    for bad in ["a%2Fb", "a%01b", &"x".repeat(201)] {
        let (code, raw) = r
            .do_("GET", &format!("/v1/attach?session={bad}"), &[], None)
            .await;
        assert!(
            code == 400 && raw.contains("bad session id"),
            "{bad}: {code} {raw}"
        );
    }
    // A blank token, and a body that isn't JSON.
    let ok = [(HEADER_ATTACH, secret.as_str())];
    let (code, raw) = r
        .do_(
            "POST",
            "/v1/deliveries/delivered",
            &ok,
            Some(json!({"token": "  "})),
        )
        .await;
    assert!(
        code == 400 && raw.contains("token is required"),
        "{code} {raw}"
    );
    let (code, raw) = r
        .do_("POST", "/v1/deliveries/engaged", &ok, Some(json!([1])))
        .await;
    assert!(
        code == 400 && raw.contains("body must be a JSON object"),
        "{code} {raw}"
    );
    // An unknown token is the inbox's not-found.
    let (code, _) = r
        .do_(
            "POST",
            "/v1/deliveries/delivered",
            &ok,
            Some(json!({"token": "[irori-wake:x]"})),
        )
        .await;
    assert_eq!(code, 404, "unknown token");
    // Go's policy-write and members checks belong to the API routes.
}

#[tokio::test(flavor = "multi_thread")]
async fn next_turn_append() {
    let r = Rig::new().await;
    let ib = r.core.inbox();
    let (mut b1, _) = r.attach_mod(SESS_B, attach::APPEND_SINCE).await;
    let (_, d) = ib
        .send_with(
            &r.core.user_sender(),
            &to("beta"),
            "look at the lexer when you're next up",
            SendOptions {
                ref_: "task:01LEX".into(),
                ..SendOptions::default()
            },
        )
        .unwrap();
    assert_eq!(d.mode, mode::NEXT_TURN);
    let ev = b1.next("append").await;
    let (tok, prompt) = (
        str_of(&ev, "token").to_string(),
        str_of(&ev, "prompt").to_string(),
    );
    assert!(
        str_of(&ev, "mode") == "next_turn"
            && tok.starts_with("[irori-append:")
            && strs(&ev, "deliveryIds") == vec![d.id.clone()]
            && strs(&ev, "messageIds").len() == 1
            && str_of(&ev, "from") == "you"
            && prompt.matches(tok.as_str()).count() == 1
            && prompt.contains("mode=\"next_turn\"")
            && prompt.contains("ref=\"task:01LEX\""),
        "append {ev}"
    );
    b1.none("deliver", Duration::from_millis(300)).await;

    // A reload before delivered: the same append again, same token.
    let (mut b2, h2) = r.attach_mod(SESS_B, "0.4.1").await;
    b1.ended("superseded").await;
    let again = b2.next("append").await;
    assert!(
        str_of(&again, "token") == tok && str_of(&again, "prompt") == prompt,
        "re-sent {again}"
    );
    r.control(str_of(&h2, "attachSecret"), "delivered", &tok, "")
        .await;
    assert_eq!(r.state(&d.id), st::DELIVERED);
    // After delivered a reload sends nothing.
    let (mut b3, h3) = r.attach_mod(SESS_B, "0.4.1").await;
    b2.ended("superseded").await;
    b3.none("append", Duration::from_millis(500)).await;

    // The next main-loop turn engages it; no running wake, turn-ended is a no-op.
    let s3 = str_of(&h3, "attachSecret").to_string();
    r.control(&s3, "engaged", &tok, "turn-p").await;
    assert_eq!(r.state(&d.id), st::ENGAGED);
    assert_eq!(ib.running_wake(&r.key_b), "");
    r.control(&s3, "turn-ended", &tok, "turn-p").await;

    // A second one goes out as soon as it is sent.
    let (_, d2) = ib
        .send_with(
            &r.core.user_sender(),
            &to("beta"),
            "and the parser",
            SendOptions::default(),
        )
        .unwrap();
    let ev2 = b3.next("append").await;
    assert!(
        str_of(&ev2, "token") != tok && strs(&ev2, "deliveryIds") == vec![d2.id.clone()],
        "second append {ev2}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn next_turn_rides_wake_for_old_mod() {
    let r = Rig::new().await;
    let ib = r.core.inbox();
    let (mut a, ha) = r.attach(SESS_A).await; // mod=0.2.0-test: before appends
    let (_, d) = ib
        .send_with(
            &r.core.user_sender(),
            &to("alpha"),
            "fyi-3c1d",
            SendOptions::default(),
        )
        .unwrap();
    a.none("append", Duration::from_millis(500)).await;
    assert_eq!(r.state(&d.id), st::QUEUED);
    // It rides the next wake.
    let (_, dw) = ib
        .send(&r.sender(&r.key_b, "beta"), &to("alpha"), "wake-88aa", "")
        .unwrap();
    let ev = a.next("deliver").await;
    assert!(
        str_of(&ev, "mode").is_empty()
            && strs(&ev, "deliveryIds") == vec![dw.id.clone(), d.id.clone()]
            && str_of(&ev, "prompt").contains("fyi-3c1d")
            && str_of(&ev, "prompt").contains("mode=\"next_turn\""),
        "deliver {ev}"
    );
    let sa = str_of(&ha, "attachSecret").to_string();
    r.control(&sa, "delivered", str_of(&ev, "token"), "").await;
    r.control(&sa, "engaged", str_of(&ev, "token"), "turn-1")
        .await;
    assert_eq!(r.state(&d.id), st::ENGAGED, "ride-along state");
}

/// Card #23 through a real stream: a wake the mod never confirms is
/// reclaimed and re-sent under a new token, and after
/// `inbox::MAX_WAKE_TRIES` it goes as an append, which the next turn
/// engages.
#[tokio::test(flavor = "multi_thread")]
async fn unconfirmed_wake_reaches_next_turn() {
    let grace = Duration::from_millis(300);
    let r = Rig::with(Duration::ZERO, |o| o.reattach_grace = grace).await;
    let ib = r.core.inbox();
    let (mut b, hb) = r.attach_mod(SESS_B, attach::APPEND_SINCE).await;
    let (_, d) = ib
        .send_with(
            &r.core.user_sender(),
            &to("beta"),
            "done",
            SendOptions {
                mode: mode::WAKE.into(),
                ref_: "task:01M62BP2MKFWJ4FK189419DNRV".into(),
                notice: true,
                reason: "answered".into(),
                ..SendOptions::default()
            },
        )
        .unwrap();
    let mut tokens = std::collections::HashSet::new();
    for i in 0..inbox::MAX_WAKE_TRIES {
        let ev = b.next("deliver").await;
        let tok = str_of(&ev, "token").to_string();
        assert!(
            strs(&ev, "deliveryIds") == vec![d.id.clone()] && tokens.insert(tok),
            "try {i}: deliver {ev}"
        );
        // The mod drops it and says nothing; the sweeper reclaims it.
        tokio::time::sleep(grace + Duration::from_millis(50)).await;
        ib.sweep().unwrap();
    }
    let ev = b.next("append").await;
    let tok = str_of(&ev, "token").to_string();
    assert!(
        tok.starts_with("[irori-append:")
            && strs(&ev, "deliveryIds") == vec![d.id.clone()]
            && str_of(&ev, "prompt").contains("reason=\"answered\""),
        "append {ev}"
    );
    b.none("deliver", Duration::from_millis(300)).await;
    let sb = str_of(&hb, "attachSecret").to_string();
    r.control(&sb, "delivered", &tok, "").await;
    assert_eq!(r.state(&d.id), st::DELIVERED);
    r.control(&sb, "engaged", &tok, "turn-human").await;
    assert_eq!(r.state(&d.id), st::SKIPPED);
}

/// The mod reports a wake it appended (its host refused the submit) with
/// `appended: true`, and the Core engages it without making it the
/// running wake.
#[tokio::test(flavor = "multi_thread")]
async fn appended_wake_engaged_over_http() {
    let r = Rig::new().await;
    let ib = r.core.inbox();
    let (mut b, hb) = r.attach_mod(SESS_B, attach::APPEND_SINCE).await;
    let (_, d) = ib
        .send(&r.core.user_sender(), &to("beta"), "hi", "")
        .unwrap();
    let ev = b.next("deliver").await;
    let sb = str_of(&hb, "attachSecret").to_string();
    let tok = str_of(&ev, "token").to_string();
    r.control(&sb, "delivered", &tok, "").await;
    let (code, raw) = r
        .do_(
            "POST",
            "/v1/deliveries/engaged",
            &[(HEADER_ATTACH, &sb)],
            Some(json!({"token": tok, "turnId": "turn-human", "appended": true})),
        )
        .await;
    assert!(
        code == 200 && raw == "{\"ok\":true}\n",
        "engaged: {code} {raw}"
    );
    assert_eq!(r.state(&d.id), st::ENGAGED);
    assert_eq!(ib.running_wake(str_of(&hb, "agentKey")), "");
}

/// The mod's own report bodies (mod/tests/irori.test.tsx asserts these
/// exact JSON texts) are accepted: a dropped submit put in as an append
/// is reported `{token, appended:true}`, then engaged
/// `{token, turnId, appended:true}`, then ended `{token, turnId}`.
#[tokio::test(flavor = "multi_thread")]
async fn mod_report_bodies() {
    let r = Rig::new().await;
    let ib = r.core.inbox();
    let (mut b, hb) = r.attach_mod(SESS_B, "0.4.1").await;
    let (_, d) = ib
        .send(&r.sender(&r.key_a, "alpha"), &to("beta"), "hi", "")
        .unwrap();
    let ev = b.next("deliver").await;
    let tok = str_of(&ev, "token").to_string();
    let sb = str_of(&hb, "attachSecret").to_string();
    let post = |what: &'static str, body: String| {
        let sb = sb.clone();
        let r = &r;
        async move {
            let resp = common::send(
                r.host.port,
                "POST",
                &format!("/v1/deliveries/{what}"),
                &r.token(),
                &[("content-type", "application/json"), (HEADER_ATTACH, &sb)],
                Some(serde_json::from_str(&body).unwrap()),
            )
            .await;
            resp.status().as_u16()
        }
    };
    let q = gocompat::json::quote_string(&tok, false);
    assert_eq!(
        post("delivered", format!("{{\"token\":{q},\"appended\":true}}")).await,
        200
    );
    assert_eq!(r.state(&d.id), st::DELIVERED);
    assert_eq!(
        post(
            "engaged",
            format!("{{\"token\":{q},\"turnId\":\"turn-h\",\"appended\":true}}")
        )
        .await,
        200
    );
    assert_eq!(r.state(&d.id), st::ENGAGED);
    assert_eq!(
        post(
            "turn-ended",
            format!("{{\"token\":{q},\"turnId\":\"turn-h\"}}")
        )
        .await,
        200
    );
}

/// Held and moved deliveries reach the stream as `held` and `moved`
/// events, with the counts after them.
#[tokio::test(flavor = "multi_thread")]
async fn held_and_counts() {
    let r = Rig::new().await;
    let ib = r.core.inbox();
    let (mut b, _) = r.attach(SESS_B).await;
    let first = b.next("inbox").await;
    assert_eq!(first, json!({"type": "inbox"}), "zero counts are omitted");
    let mut p = ib.policy();
    p.paused = true;
    ib.set_policy(p, "user:test").unwrap();
    let (_, d) = ib
        .send(
            &r.sender(&r.key_a, "alpha"),
            &to("beta"),
            "  held\n\ttext  ",
            "",
        )
        .unwrap();
    assert_eq!(d.state, st::HELD, "{d:?}");
    // The counts may go out before the held event: the policy change's
    // pass already sees the hold.
    let seen = b.collect(Duration::from_millis(700)).await;
    let held = json!({"type": "held", "deliveryId": d.id, "from": "alpha", "why": "paused", "preview": "held text"});
    assert!(seen.contains(&held), "{seen:?}");
    assert!(
        seen.contains(&json!({"type": "inbox", "held": 1})),
        "{seen:?}"
    );
}
