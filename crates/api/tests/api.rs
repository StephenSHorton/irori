//! The Go api tests the golden replay does not cover (I:internal/api
//! api_test.go, events_test.go, v04_test.go, client_v02_test.go; core
//! v02_test.go): stream timing, replay modes, resync, shutdown, the client.

mod common;

use std::time::{Duration, Instant};

use irori_api::client::{Client, Error, NO_REPLAY, Opened, StreamItem};
use tokio::sync::mpsc::Receiver;

fn next(rig: &common::Rig, rx: &mut Receiver<StreamItem>) -> StreamItem {
    rig.rt
        .block_on(async { tokio::time::timeout(Duration::from_secs(10), rx.recv()).await })
        .expect("no stream item in 10 s")
        .expect("the stream ended")
}

fn append(rig: &common::Rig, kind: &str) -> i64 {
    rig.core
        .store()
        .append("irori", kind, &serde_json::json!({}))
        .unwrap()
        .seq
}

fn last_own_seq(rig: &common::Rig) -> i64 {
    rig.rt
        .block_on(rig.client.last_seq(rig.core.device_id()))
        .unwrap()
}

#[test]
fn events_stream_replay_then_live() {
    let rig = common::Rig::new();
    let base = last_own_seq(&rig);
    let a = append(&rig, "agent.seen");
    let _ = append(&rig, "device.started");
    let c = append(&rig, "agent.status");
    assert_eq!(a, base + 1);
    let mut rx = rig
        .rt
        .block_on(rig.client.events_stream(&["agent.".into()], a))
        .unwrap();
    let it = next(&rig, &mut rx);
    let row = it.row.expect("a row");
    assert_eq!((row.seq, row.kind.as_str()), (c, "agent.status"));
    let d = append(&rig, "agent.gone");
    let _ = append(&rig, "device.started");
    let f = append(&rig, "agent.status");
    for want in [d, f] {
        assert_eq!(next(&rig, &mut rx).row.expect("a row").seq, want);
    }
}

#[test]
fn events_stream_replay_modes() {
    // since and the replayed rows, counted from the rows before the two.
    let cases: [(Option<i64>, &[i64]); 4] = [
        (None, &[]),
        (Some(0), &[1, 2]),
        (Some(1), &[2]),
        (Some(2), &[]),
    ];
    for (since, want) in cases {
        let rig = common::Rig::new();
        let base = last_own_seq(&rig);
        append(&rig, "agent.seen");
        append(&rig, "agent.status");
        let since = since.map_or(NO_REPLAY, |s| base + s);
        let mut rx = rig
            .rt
            .block_on(rig.client.events_stream(&["agent.".into()], since))
            .unwrap();
        for s in want {
            assert_eq!(
                next(&rig, &mut rx).row.expect("a row").seq,
                base + s,
                "since {since}"
            );
        }
        // The stream is live: a new row arrives next, and nothing older.
        let live = append(&rig, "agent.status");
        assert_eq!(
            next(&rig, &mut rx).row.expect("a row").seq,
            live,
            "since {since}"
        );
    }
}

#[test]
fn events_stream_vector_since() {
    let rig = common::Rig::new();
    let me = rig.core.device_id().to_string();
    let base = last_own_seq(&rig);
    append(&rig, "agent.seen");
    let since = std::collections::BTreeMap::from([(me.clone(), base), ("other".to_string(), 0)]);
    let mut rx = rig
        .rt
        .block_on(rig.client.events_stream_from(&["agent.".into()], &since))
        .unwrap();
    let row = next(&rig, &mut rx).row.expect("a row");
    assert_eq!((row.device_id.as_str(), row.seq), (me.as_str(), base + 1));
}

#[test]
fn events_stream_keepalive_fills_silence() {
    let rig = common::Rig::new();
    rig.server.set_keepalive(Duration::from_millis(40));
    let first = rig.rt.block_on(async {
        let Opened::Stream(mut lines, hs) =
            rig.client.open("/v1/events/stream", &[]).await.unwrap()
        else {
            panic!("refused")
        };
        assert!(
            hs.iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v == "application/x-ndjson")
        );
        // The rig's own rows (agent.seen) may come first; skip them.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match lines.next().await {
                    Some(l) if !l.is_empty() => continue,
                    other => break other,
                }
            }
        })
        .await
        .unwrap()
    });
    assert_eq!(first.as_deref(), Some(&b""[..]), "a bare \\n keepalive");
}

#[test]
fn events_stream_resync_when_the_reader_falls_behind() {
    let rig = common::Rig::new();
    let opened = rig
        .rt
        .block_on(rig.client.open("/v1/events/stream?kinds=agent.status", &[]))
        .unwrap();
    let Opened::Stream(mut lines, _) = opened else {
        panic!("refused")
    };
    // Nobody reads while far more than the store's 256 slots of rows (and
    // the socket buffers' worth) are appended.
    let big = "x".repeat(32 << 10);
    for _ in 0..1200 {
        rig.core
            .store()
            .append("irori", "agent.status", &serde_json::json!({ "name": big }))
            .unwrap();
    }
    let found = rig.rt.block_on(async {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_secs(5), lines.next()).await {
                Ok(Some(l)) if l == br#"{"resync":true}"# => return true,
                Ok(Some(_)) => continue,
                _ => return false,
            }
        }
        false
    });
    assert!(found, "no resync line after the reader fell behind");
}

#[test]
fn events_stream_oversized_row_names_itself() {
    let rig = common::Rig::new();
    let base = last_own_seq(&rig);
    let big = rig
        .core
        .store()
        .append(
            "irori",
            "agent.status",
            &serde_json::json!({ "name": "x".repeat(irori_api::client::MAX_EVENT_LINE) }),
        )
        .unwrap();
    assert_eq!(big.seq, base + 1);
    append(&rig, "agent.seen");
    let mut rx = rig
        .rt
        .block_on(rig.client.events_stream(&[], base))
        .unwrap();
    let it = next(&rig, &mut rx);
    assert!(it.resync, "{it:?}");
    assert_eq!(
        (it.skipped_device.as_str(), it.skipped_seq),
        (rig.core.device_id(), big.seq)
    );
    assert_eq!(next(&rig, &mut rx).row.expect("a row").seq, big.seq + 1);
}

#[test]
fn last_seq() {
    let rig = common::Rig::new();
    let base = last_own_seq(&rig);
    assert!(base > 0, "the Core's start rows");
    append(&rig, "agent.seen");
    assert_eq!(last_own_seq(&rig), base + 1);
    assert_eq!(rig.rt.block_on(rig.client.last_seq("nobody")).unwrap(), 0);
}

#[test]
fn attach_keepalive_ticks() {
    let rig = common::Rig::new();
    rig.server.set_keepalive(Duration::from_millis(40));
    let got = rig.rt.block_on(async {
        let path = format!("/v1/attach?session={}&mod=0.4.1", common::SESS_A);
        let Opened::Stream(mut lines, _) = rig.client.open(&path, &[]).await.unwrap() else {
            panic!("refused")
        };
        let mut seen = Vec::new();
        for _ in 0..3 {
            seen.push(
                tokio::time::timeout(Duration::from_secs(5), lines.next())
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
        seen
    });
    assert!(got[0].starts_with(br#"{"type":"hello""#));
    assert_eq!(got[1], br#"{"type":"inbox"}"#);
    assert_eq!(got[2], b"", "then a keepalive");
}

#[test]
fn member_card_while_streaming_then_for_the_grace() {
    let rig = common::Rig::new();
    let joined = rig
        .rt
        .block_on(rig.client.call(
            "POST",
            "/v1/members",
            &[],
            Some(br#"{"name":"g","session":"s"}"#.to_vec()),
        ))
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&joined).unwrap();
    let (key, secret) = (
        v["memberKey"].as_str().unwrap().to_string(),
        v["memberSecret"].as_str().unwrap().to_string(),
    );
    let listed = |rig: &common::Rig| rig.core.local_agents().iter().any(|c| c.key == key);
    assert!(!listed(&rig), "a join alone lists no card");
    rig.rt.block_on(async {
        let Opened::Stream(mut lines, _) = rig
            .client
            .open(&format!("/v1/attach?member={secret}"), &[])
            .await
            .unwrap()
        else {
            panic!("refused")
        };
        let hello = lines.next().await.unwrap();
        assert!(hello.starts_with(br#"{"type":"hello""#));
        assert!(
            !String::from_utf8_lossy(&hello).contains("attachSecret"),
            "a member hello has no attach secret"
        );
    });
    // The stream closed when its reader went away; the card stays for the
    // reattach grace.
    let deadline = Instant::now() + Duration::from_secs(5);
    while rig.core.inbox().reachable(&key).1 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(listed(&rig), "the card stays for the grace");
    // Coord and capture see the member through the Core's roster too.
    assert!(
        rig.core
            .roster()
            .agents()
            .iter()
            .any(|c| c.key == key && c.agent == "grok")
    );
}

#[test]
fn serve_stops_on_cancel_with_a_stream_open() {
    let rig = common::Rig::new();
    let server = irori_api::Server::new(rig.core.clone(), rig.core.token());
    let ln = rig.rt.block_on(irori_api::listen()).unwrap();
    let port = ln.local_addr().unwrap().port();
    let stop = tokio_util::sync::CancellationToken::new();
    let done = {
        let (server, stop) = (server.clone(), stop.clone());
        rig.rt.spawn(async move { server.serve(ln, stop).await })
    };
    let c = Client::at(port, rig.core.token());
    let opened = rig.rt.block_on(c.open("/v1/events/stream", &[])).unwrap();
    assert!(matches!(opened, Opened::Stream(..)));
    let t = Instant::now();
    stop.cancel();
    rig.rt
        .block_on(async { tokio::time::timeout(Duration::from_secs(6), done).await })
        .expect("serve did not return")
        .unwrap()
        .unwrap();
    assert!(t.elapsed() < Duration::from_secs(6));
    drop(opened);
}

#[test]
fn dial_finds_a_running_core_and_only_that() {
    let rig = common::Rig::new();
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(
        rig.rt.block_on(Client::dial(empty.path())).unwrap_err(),
        Error::NotRunning
    );
    // A lockfile naming a closed port is not a running Irori either.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    rig.core.write_lockfile(port).unwrap();
    assert_eq!(
        rig.rt.block_on(Client::dial(&rig.data)).unwrap_err(),
        Error::NotRunning
    );
    rig.core.write_lockfile(rig.port).unwrap();
    let c = rig.rt.block_on(Client::dial(&rig.data)).unwrap();
    let dev = rig
        .rt
        .block_on(c.call("GET", "/v1/device", &[], None))
        .unwrap();
    assert!(String::from_utf8_lossy(&dev).contains(rig.core.device_id()));
}

#[test]
fn stream_error_hides_the_query() {
    let rig = common::Rig::new();
    let ln = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = ln.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for c in ln.incoming() {
            drop(c); // EOF before any response
        }
    });
    let c = Client::at(port, "tok");
    let err = rig
        .rt
        .block_on(c.stream("/v1/attach?member=SECRET123", &[]))
        .err()
        .expect("a closing listener refused nothing");
    let text = err.to_string();
    assert!(
        !text.contains("SECRET123"),
        "the error leaks the secret: {text}"
    );
    assert!(
        text.contains("/v1/attach"),
        "the error lost its context: {text}"
    );
}

#[test]
fn token_alone_reads_as_an_agent() {
    let rig = common::Rig::new();
    let me = rig.core.device_id().to_string();
    rig.core
        .inbox()
        .send(
            &inbox::Sender::new(&rig.key_b, "beta", "claude"),
            &inbox::Address::new("agent", &rig.key_a),
            "secret words",
            "why",
        )
        .unwrap();
    let get = |hs: &[(&str, &str)]| {
        String::from_utf8(
            rig.rt
                .block_on(
                    rig.client
                        .call("GET", "/v1/events?kind=message.sent", hs, None),
                )
                .unwrap(),
        )
        .unwrap()
    };
    let agent = get(&[]);
    assert!(!agent.contains("secret words") && agent.contains(r#""text":"[left out]""#));
    assert!(agent.contains(r#""reason":"[left out]""#));
    let person = rig.server.person_secret();
    let whole = get(&[("X-Irori-Person", &person)]);
    assert!(whole.contains("secret words"), "{whole}");
    assert!(whole.contains(&me));
}

// ---- connection deadlines (Go http.Server's Read/Write/Idle timeouts) ----

mod deadlines {
    use super::common;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};

    // The short deadlines keep their order (header < idle, read < idle)
    // with seconds between what must fit inside one and the deadline, so a
    // loaded machine (a parallel build) can't stall a step past it.
    const IDLE: Duration = Duration::from_secs(4);
    /// How long a test waits for what must come; only a failing run waits
    /// it out.
    const WAIT: Duration = Duration::from_secs(20);

    fn short(rig: &common::Rig) {
        rig.server.set_timeouts(irori_api::Timeouts {
            read_header: Duration::from_millis(400),
            read: Duration::from_millis(800),
            write: Duration::from_secs(5),
            idle: IDLE,
        });
    }

    fn dial(rig: &common::Rig) -> TcpStream {
        let c = TcpStream::connect(("127.0.0.1", rig.port)).unwrap();
        c.set_read_timeout(Some(WAIT)).unwrap();
        c
    }

    fn get(rig: &common::Rig, path: &str) -> String {
        format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\n\r\n",
            rig.port, rig.token
        )
    }

    /// Reads one response whose body has a Content-Length.
    fn response(c: &mut TcpStream) -> String {
        let mut buf = Vec::new();
        let mut b = [0u8; 4096];
        loop {
            let n = c.read(&mut b).unwrap();
            assert!(
                n > 0,
                "closed mid-response: {:?}",
                String::from_utf8_lossy(&buf)
            );
            buf.extend_from_slice(&b[..n]);
            let text = String::from_utf8_lossy(&buf).into_owned();
            if let Some(i) = text.find("\r\n\r\n") {
                let len: usize = text[..i]
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                if buf.len() >= i + 4 + len {
                    return text;
                }
            }
        }
    }

    /// The server closes the connection, within `within`.
    fn closed(c: &mut TcpStream, within: Duration) -> Duration {
        let t = Instant::now();
        let mut b = [0u8; 4096];
        c.set_read_timeout(Some(within)).unwrap();
        loop {
            match c.read(&mut b) {
                // The read timeout: open all along.
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    panic!("still open after {within:?}")
                }
                Ok(0) | Err(_) => return t.elapsed(),
                Ok(_) => {}
            }
            assert!(t.elapsed() < within, "still open after {within:?}");
        }
    }

    #[test]
    fn idle_keepalive_closes_after_idle_timeout() {
        let rig = common::Rig::new();
        short(&rig);
        let mut c = dial(&rig);
        c.write_all(get(&rig, "/v1/device").as_bytes()).unwrap();
        assert!(response(&mut c).starts_with("HTTP/1.1 200"));
        // Idle well past the header timeout and well short of the idle
        // one: the connection still serves.
        std::thread::sleep(Duration::from_millis(1200));
        let sent = Instant::now();
        c.write_all(get(&rig, "/v1/device").as_bytes()).unwrap();
        assert!(response(&mut c).starts_with("HTTP/1.1 200"));
        // Then the idle timeout closes it: not before (its wait starts
        // after the response, so after `sent`), and not never.
        closed(&mut c, WAIT);
        let after = sent.elapsed();
        assert!(after >= IDLE, "closed {after:?} after the request");
    }

    #[test]
    fn a_stalled_head_or_body_is_cut() {
        let rig = common::Rig::new();
        short(&rig);
        // Half a request head.
        let mut c = dial(&rig);
        c.write_all(b"GET /v1/device HTTP/1.1\r\nHost: 127")
            .unwrap();
        closed(&mut c, WAIT);
        // A body that never comes, from a caller with the token.
        let mut c = dial(&rig);
        let head = format!(
            "POST /v1/policy HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{{\"a\"",
            rig.port, rig.token
        );
        c.write_all(head.as_bytes()).unwrap();
        closed(&mut c, WAIT);
    }

    #[test]
    fn no_body_is_read_before_the_token_check() {
        let rig = common::Rig::new();
        let mut c = dial(&rig);
        // Without the token: answered at once, though the body never comes.
        let head = format!(
            "POST /v1/policy HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 100\r\n\r\n",
            rig.port
        );
        let t = Instant::now();
        c.write_all(head.as_bytes()).unwrap();
        let r = response(&mut c);
        assert!(r.starts_with("HTTP/1.1 401"), "{r}");
        // Well before the 10 s read timeout a body read would wait for.
        assert!(t.elapsed() < Duration::from_secs(6), "{:?}", t.elapsed());
    }

    #[test]
    fn streams_lift_the_deadlines() {
        let rig = common::Rig::new();
        short(&rig);
        rig.server.set_keepalive(Duration::from_secs(60));
        let mut c = dial(&rig);
        c.write_all(get(&rig, "/v1/events/stream?kinds=agent.status").as_bytes())
            .unwrap();
        let mut b = [0u8; 4096];
        let n = c.read(&mut b).unwrap();
        assert!(String::from_utf8_lossy(&b[..n]).starts_with("HTTP/1.1 200"));
        // Well past every deadline, the stream still delivers.
        std::thread::sleep(IDLE + Duration::from_millis(600));
        rig.core
            .store()
            .append(
                "irori",
                "agent.status",
                &serde_json::json!({"name": "late-row"}),
            )
            .unwrap();
        let mut got = String::new();
        let t = Instant::now();
        while !got.contains("late-row") {
            assert!(t.elapsed() < WAIT, "no row: {got:?}");
            let n = c.read(&mut b).unwrap();
            assert!(n > 0, "stream closed: {got:?}");
            got.push_str(&String::from_utf8_lossy(&b[..n]));
        }
    }
}

/// `/v1/devices` on a Core is its mesh's rows (Go `Mesh().Devices`): a
/// lone device lists itself, first, as "this".
#[test]
fn devices_lists_this_device_first() {
    let rig = common::Rig::new();
    for q in ["", "?detail=1"] {
        let r = rig
            .rt
            .block_on(rig.client.raw("GET", &format!("/v1/devices{q}"), &[], None))
            .unwrap();
        assert_eq!(r.status, 200, "{q}: {}", String::from_utf8_lossy(&r.body));
        let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
        let rows = v.as_array().expect("an array");
        assert_eq!(rows.len(), 1, "{q}: {v}");
        assert_eq!(rows[0]["deviceId"], rig.core.device_id(), "{q}");
        assert_eq!(rows[0]["this"], true, "{q}");
        assert_eq!(rows[0]["state"], "this", "{q}");
        assert!(rows[0].get("detail").is_none(), "{q}: {v}");
    }
}
