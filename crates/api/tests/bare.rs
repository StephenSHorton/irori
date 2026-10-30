//! A Backend with no coord, inbox or capture (Go's api fakeBackend): the
//! routes that need them answer 503 (I:internal/api/v03_test.go
//! TestV03RoutesWithoutCapture503, board_test.go
//! TestBoardRoutesWithoutServices503), and the events stream's resync and
//! end (events_test.go TestEventsStreamResyncAndClose).

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gocompat::sub::Queue;
use irori_api::{Backend, Member, Server};
use tokio_util::sync::CancellationToken;

const TOKEN: &str = "bare-test-token";

struct Bare {
    st: store::Store,
    /// When set, the next subscription reads this queue.
    live: Mutex<Option<Arc<Queue<store::Event>>>>,
    /// A mesh's device rows; every `detail` flag asked is recorded.
    mesh: Option<Vec<mesh::DeviceRow>>,
    detail: Mutex<Vec<bool>>,
    /// Shared boards, each with its detail; `None` is a Core without them.
    boards: Mutex<Option<Vec<irori_api::boards::BoardRow>>>,
}

impl Backend for Bare {
    fn version(&self) -> String {
        "0.0.0-test".into()
    }
    fn device(&self) -> irori_identity::DeviceJson {
        irori_identity::DeviceJson {
            id: "dev1".into(),
            name: "box".into(),
            os: "testos".into(),
            ..Default::default()
        }
    }
    fn agents(&self) -> Vec<presence::AgentCard> {
        vec![]
    }
    fn events(&self, f: &store::Filter) -> Result<Vec<store::Event>, String> {
        self.st.list(f).map_err(|e| e.to_string())
    }
    fn subscribe_events(&self) -> store::Subscription {
        match self.live.lock().unwrap().take() {
            Some(q) => store::Subscription::from_queue(q),
            None => self.st.subscribe(),
        }
    }
    fn inbox(&self) -> Option<Arc<inbox::Inbox>> {
        None
    }
    fn coord(&self) -> Option<Arc<coord::Coord>> {
        None
    }
    fn capture(&self) -> Option<capture::Capture> {
        None
    }
    fn join_member(&self, _: &str, _: &str) -> Result<Member, String> {
        Err("no members".into())
    }
    fn member_by_secret(&self, _: &str) -> Option<Member> {
        None
    }
    fn member_stream(&self, _: &str, _: bool) {}
    fn mesh_devices(&self, detail: bool) -> Option<Vec<mesh::DeviceRow>> {
        self.detail.lock().unwrap().push(detail);
        self.mesh.clone()
    }
    fn boards(&self, detail: bool) -> Option<Vec<irori_api::boards::BoardRow>> {
        let mut rows = self.boards.lock().unwrap().clone()?;
        if !detail {
            for r in &mut rows {
                r.detail = None;
            }
        }
        Some(rows)
    }
}

struct Host {
    b: Arc<Bare>,
    port: u16,
    person: String,
    rt: tokio::runtime::Runtime,
    stop: CancellationToken,
    _dir: tempfile::TempDir,
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

fn start() -> Host {
    start_with(None)
}

fn start_with(mesh: Option<Vec<mesh::DeviceRow>>) -> Host {
    let dir = tempfile::tempdir().unwrap();
    let st = store::Store::open(
        dir.path().join("events.db"),
        "dev1",
        Arc::new(store::Clock::new()),
    )
    .unwrap();
    let b = Arc::new(Bare {
        st,
        live: Mutex::new(None),
        mesh,
        detail: Mutex::new(Vec::new()),
        boards: Mutex::new(None),
    });
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let srv = Server::new(b.clone(), TOKEN);
    let person = srv.person_secret();
    let ln = rt.block_on(irori_api::listen()).unwrap();
    let port = ln.local_addr().unwrap().port();
    let stop = CancellationToken::new();
    let s = stop.clone();
    rt.spawn(async move { srv.serve(ln, s).await });
    Host {
        b,
        port,
        person,
        rt,
        stop,
        _dir: dir,
    }
}

/// Go `wantStatus`: GET as the person, a JSON error body with that status.
fn want_status(h: &Host, path: &str, code: u16) {
    let c = irori_api::client::Client::at(h.port, TOKEN);
    let r =
        h.rt.block_on(c.raw("GET", path, &[(irori_api::HEADER_PERSON, &h.person)], None))
            .unwrap();
    assert_eq!(
        r.status,
        code,
        "GET {path}: {}",
        String::from_utf8_lossy(&r.body)
    );
    let v: serde_json::Value = serde_json::from_slice(&r.body)
        .unwrap_or_else(|_| panic!("GET {path}: not JSON: {:?}", r.body));
    assert!(
        v["error"].as_str().is_some_and(|e| !e.is_empty()),
        "GET {path}: {v}"
    );
}

const CAP_KEY: &str = "dev1/cccccccc-3333-4333-8333-333333333333";

#[test]
fn v03_routes_without_capture_503() {
    let h = start();
    for p in [
        format!("/v1/timeline?agent={CAP_KEY}"),
        "/v1/edits".into(),
        "/v1/commits".into(),
        "/v1/commits/abcdef0".into(),
        format!("/v1/blobs/{}", "a".repeat(64)),
        "/v1/decisions".into(),
        "/v1/prs".into(),
        "/v1/capture/status".into(),
        "/v1/why?path=a.txt".into(),
    ] {
        want_status(&h, &p, 503);
    }
}

#[test]
fn board_routes_without_services_503() {
    let h = start();
    for p in [
        "/v1/board".to_string(),
        "/v1/tasks/x".into(),
        "/v1/search?q=a".into(),
        "/v1/show?ref=task:x".into(),
        format!("/v1/overview?agent={CAP_KEY}"),
    ] {
        want_status(&h, &p, 503);
    }
}

/// The next NDJSON line of a chunked body, or `None` once it ends.
fn next_line(r: &mut BufReader<std::net::TcpStream>, body: &mut String) -> Option<String> {
    loop {
        if let Some(i) = body.find('\n') {
            return Some(body.drain(..=i).collect());
        }
        let mut size = String::new();
        if r.read_line(&mut size).ok()? == 0 {
            return None;
        }
        let n = usize::from_str_radix(size.trim(), 16).ok()?;
        if n == 0 {
            return None; // the last chunk: the stream ended
        }
        let mut chunk = vec![0u8; n + 2];
        r.read_exact(&mut chunk).ok()?;
        body.push_str(&String::from_utf8_lossy(&chunk[..n]));
    }
}

#[test]
fn events_stream_resync_and_close() {
    let h = start();
    let q = Arc::new(Queue::new(4));
    *h.b.live.lock().unwrap() = Some(q.clone());
    let mut c = std::net::TcpStream::connect(("127.0.0.1", h.port)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(
        c,
        "GET /v1/events/stream HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {TOKEN}\r\n\r\n",
        h.port
    )
    .unwrap();
    let mut r = BufReader::new(c);
    let mut head = Vec::new();
    loop {
        let mut l = String::new();
        r.read_line(&mut l).unwrap();
        if l == "\r\n" {
            break;
        }
        head.push(l);
    }
    assert!(head[0].starts_with("HTTP/1.1 200"), "{head:?}");
    assert!(
        head.iter()
            .any(|l| l.eq_ignore_ascii_case("content-type: application/x-ndjson\r\n")),
        "{head:?}"
    );
    let mut body = String::new();
    q.push(store::Event::default());
    let line = next_line(&mut r, &mut body).expect("a resync line");
    assert_eq!(line, "{\"resync\":true}\n");
    // The store closed: the stream ends with no other line.
    q.close();
    let t = Instant::now();
    loop {
        match next_line(&mut r, &mut body) {
            None => break,
            Some(l) if l == "\n" => {} // a keepalive
            Some(l) => panic!("a line after the store closed: {l:?}"),
        }
    }
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
}

/// Go server.go `devices`: 503 without a mesh; with one, the rows as
/// `[]DeviceRow` and `detail` on for `1` or `true` only.
#[test]
fn devices_route_like_go() {
    let h = start();
    want_status(&h, "/v1/devices", 503);
    want_status(&h, "/v1/devices?detail=1", 503);

    let rows = vec![
        mesh::DeviceRow {
            device_id: "dev1".into(),
            name: "box".into(),
            role: "desktop".into(),
            os: "testos".into(),
            this: true,
            state: "this".into(),
            ..Default::default()
        },
        mesh::DeviceRow {
            device_id: "dev2".into(),
            name: "mac".into(),
            role: "desktop".into(),
            state: "offline".into(),
            revoked: true,
            ..Default::default()
        },
    ];
    let h = start_with(Some(rows));
    let c = irori_api::client::Client::at(h.port, TOKEN);
    for (q, want) in [
        ("", false),
        ("?detail=1", true),
        ("?detail=true", true),
        ("?detail=0", false),
        ("?detail=yes", false),
        ("?detail=TRUE", false),
    ] {
        let r =
            h.rt.block_on(c.raw("GET", &format!("/v1/devices{q}"), &[], None))
                .unwrap();
        assert_eq!(r.status, 200, "{q}");
        assert_eq!(
            String::from_utf8_lossy(&r.body),
            concat!(
                r#"[{"deviceId":"dev1","name":"box","role":"desktop","os":"testos","this":true,"state":"this"},"#,
                r#"{"deviceId":"dev2","name":"mac","role":"desktop","state":"offline","revoked":true}]"#,
                "
"
            ),
            "{q}"
        );
        assert_eq!(h.b.detail.lock().unwrap().pop(), Some(want), "{q}");
    }
}

/// `/v1/boards`: 503 until the Core holds shared boards; then the boards,
/// with members and the set-aside counts only under `detail`.
#[test]
fn boards_route_shows_set_aside_counts_under_detail() {
    use irori_api::boards::{BoardDetail, BoardMember, BoardRow};
    let h = start();
    want_status(&h, "/v1/boards", 503);
    want_status(&h, "/v1/boards?detail=1", 503);
    *h.b.boards.lock().unwrap() = Some(vec![BoardRow {
        board_id: "b0".into(),
        name: "mlg".into(),
        tag: "mlg".into(),
        people: vec!["Pat".into(), "Robin".into()],
        owner: true,
        devices: 3,
        cards: 4,
        ready: true,
        detail: Some(BoardDetail {
            members: vec![BoardMember {
                person_id: "p2".into(),
                name: "Robin".into(),
                fingerprint: "ab:cd".into(),
                role: "member".into(),
                state: "active".into(),
                devices: 1,
            }],
            set_aside: coord::SetAside {
                unresolved: 2,
                dropped: 1,
                ignored: 3,
                ignored_by: [("dev-b".to_string(), 3)].into_iter().collect(),
            },
            conflicts: vec![],
        }),
    }]);
    let c = irori_api::client::Client::at(h.port, TOKEN);
    let get = |q: &str| {
        let r =
            h.rt.block_on(c.raw("GET", &format!("/v1/boards{q}"), &[], None))
                .unwrap();
        assert_eq!(r.status, 200, "{q}");
        String::from_utf8_lossy(&r.body).into_owned()
    };
    let plain = r#"{"boardId":"b0","name":"mlg","tag":"mlg","people":["Pat","Robin"],"owner":true,"devices":3,"cards":4,"ready":true"#;
    assert_eq!(get(""), format!("[{plain}}}]\n"));
    assert_eq!(get("?detail=0"), format!("[{plain}}}]\n"));
    for q in ["?detail=1", "?detail=true"] {
        assert_eq!(
            get(q),
            format!(
                "[{plain},{}}}]\n",
                concat!(
                    r#""detail":{"members":[{"personId":"p2","name":"Robin","fingerprint":"ab:cd","role":"member","state":"active","devices":1}],"#,
                    r#""setAside":{"unresolved":2,"dropped":1,"ignored":3,"ignoredBy":{"dev-b":3}}}"#
                )
            ),
            "{q}"
        );
    }
}
