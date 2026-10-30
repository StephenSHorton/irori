//! The server (I:internal/api/server.go, api.go): the listener, the checks
//! every request passes in Go's order, the v0.1 routes and the attach
//! stream registry the caller checks read.
//!
//! Order (surface.md §2.4, §7.14): Origin → Host → method → `/v1/health`
//! → auth → v0.2 → v0.3 → Board → pairing → v0.1 → 404. Auth comes before
//! routing, so a caller without the token cannot map the routes.

use std::collections::HashMap;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::BodyExt;
use http_body_util::combinators::BoxBody;
use hyper::body::{Body, Frame, Incoming};
use hyper::{Request, Response};
use hyper_util::rt::{TokioIo, TokioTimer};
use inbox::Sender;
use serde::Serialize;
use subtle::ConstantTimeEq;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::attach::{self, HEADER_ATTACH, HEADER_MEMBER};
use crate::backend::{Backend, ToolSet};
use crate::body::MAX_REQUEST_BODY;
use crate::conn::{Deadlines, DoneBody, TimedIo, Timeouts};
use crate::http::{self, Query, Req, Resp, Sink};
use crate::{AGENT_VIEWER, HEADER_PERSON};

/// Event query limits for `GET /v1/events`.
pub const DEFAULT_EVENT_LIMIT: i64 = 100;
pub const MAX_EVENT_LIMIT: i64 = 1000;
/// How often an idle stream writes a bare `\n`.
pub const KEEPALIVE: Duration = Duration::from_secs(15);
/// How long shutdown waits for open requests.
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);
/// How often a waiting stream looks for a client that went away.
pub(crate) const POLL: Duration = Duration::from_millis(100);

/// The loopback API over one [`Backend`] (Go `api.Server`).
#[derive(Clone)]
pub struct Server {
    pub(crate) s: Arc<Shared>,
}

pub(crate) struct Shared {
    pub b: Arc<dyn Backend>,
    pub tools: RwLock<Option<Arc<dyn ToolSet>>>,
    token: Vec<u8>,
    /// The person's per-run secret (decision D10(a)): only in memory.
    person: Vec<u8>,
    pub streams: Streams,
    pub stop: Arc<AtomicBool>,
    pub keepalive: RwLock<Duration>,
    pub timeouts: RwLock<Timeouts>,
}

/// The `GET /v1/health` body.
#[derive(Serialize)]
struct Health {
    ok: bool,
    version: String,
    #[serde(rename = "deviceId")]
    device_id: String,
    /// The mesh (v0.4 §6.8); absent when the backend has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    net: Option<crate::backend::NetHealth>,
}

fn new_secret(n: usize) -> String {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("api: no randomness for a secret");
    hex::encode(b)
}

impl Server {
    /// A server that checks `token` on every authenticated route. It mints
    /// the person's secret for this run ([`Server::person_secret`]).
    pub fn new(b: Arc<dyn Backend>, token: &str) -> Server {
        Server {
            s: Arc::new(Shared {
                b,
                tools: RwLock::new(None),
                token: token.as_bytes().to_vec(),
                person: new_secret(32).into_bytes(),
                streams: Streams::default(),
                stop: Arc::new(AtomicBool::new(false)),
                keepalive: RwLock::new(KEEPALIVE),
                timeouts: RwLock::new(Timeouts::default()),
            }),
        }
    }

    /// The agent tools `GET /v1/tools` and `POST /v1/tools/{name}` serve;
    /// without them the list is `[]` and calls are 503.
    pub fn with_tools(self, t: Arc<dyn ToolSet>) -> Server {
        *self.s.tools.write().unwrap_or_else(|e| e.into_inner()) = Some(t);
        self
    }

    /// Tests: the keepalive cadence of both streams.
    pub fn set_keepalive(&self, d: Duration) {
        *self.s.keepalive.write().unwrap_or_else(|e| e.into_inner()) = d;
    }

    /// Tests: the connection deadlines (Go's Read/Write/Idle timeouts).
    pub fn set_timeouts(&self, t: Timeouts) {
        *self.s.timeouts.write().unwrap_or_else(|e| e.into_inner()) = t;
    }

    /// How many attach streams (mod sessions and Grok members) are live.
    pub fn open_streams(&self) -> usize {
        self.s.streams.lock().by_key.len()
    }

    /// Whether `secret` is a live stream's attach secret (a member secret
    /// never is).
    pub fn is_attach_secret(&self, secret: &str) -> bool {
        self.s.streams.by_secret(secret).is_some()
    }

    /// This run's person secret, for the window only. Never written to
    /// disk or logged.
    pub fn person_secret(&self) -> String {
        String::from_utf8_lossy(&self.s.person).into_owned()
    }

    /// Serves on `ln` until `stop` is cancelled, then stops the streams and
    /// waits [`SHUTDOWN_WAIT`] at most for open requests.
    pub async fn serve(&self, ln: TcpListener, stop: CancellationToken) -> std::io::Result<()> {
        let port = ln.local_addr()?.port();
        let mut conns = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = stop.cancelled() => break,
                acc = ln.accept() => {
                    let Ok((tcp, _)) = acc else { continue };
                    let s = self.s.clone();
                    let stop = stop.clone();
                    let t = *self.s.timeouts.read().unwrap_or_else(|e| e.into_inner());
                    conns.spawn(async move {
                        // Go's ReadHeader/Read/Write/Idle timeouts live on
                        // the socket (conn.rs), not in hyper's own timer.
                        let d = Deadlines::new(t);
                        let io = TimedIo::new(tcp, d.clone());
                        let svc = hyper::service::service_fn(move |req| {
                            let (s, d) = (s.clone(), d.clone());
                            async move { Ok::<_, Infallible>(serve_one(s, port, d, req).await) }
                        });
                        let conn = hyper::server::conn::http1::Builder::new()
                            .timer(TokioTimer::new())
                            .header_read_timeout(None)
                            .max_buf_size(64 << 10)
                            .serve_connection(TokioIo::new(io), svc);
                        tokio::pin!(conn);
                        tokio::select! {
                            _ = conn.as_mut() => {}
                            _ = stop.cancelled() => {
                                conn.as_mut().graceful_shutdown();
                                let _ = conn.await;
                            }
                        }
                    });
                }
            }
        }
        drop(ln);
        // Streams end at once; other requests get the shutdown wait.
        self.s.stop.store(true, Ordering::SeqCst);
        let _ = tokio::time::timeout(SHUTDOWN_WAIT, async {
            while conns.join_next().await.is_some() {}
        })
        .await;
        conns.abort_all();
        Ok(())
    }
}

/// Opens `127.0.0.1:0` (Go `api.Listen`).
pub async fn listen() -> std::io::Result<TcpListener> {
    TcpListener::bind("127.0.0.1:0").await
}

type RespBody = BoxBody<Bytes, Infallible>;

/// A stream body fed by a writer thread.
struct ChanBody(tokio::sync::mpsc::Receiver<Bytes>);

impl Body for ChanBody {
    type Data = Bytes;
    type Error = Infallible;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        self.0.poll_recv(cx).map(|o| o.map(|b| Ok(Frame::data(b))))
    }
}

async fn serve_one(
    s: Arc<Shared>,
    port: u16,
    d: Arc<Deadlines>,
    req: Request<Incoming>,
) -> Response<RespBody> {
    d.request_started();
    let (parts, body) = req.into_parts();
    let raw_path = parts.uri.path().to_string();
    let Some(path) = http::unescape(&raw_path, false) else {
        return done(plain(400, "400 Bad Request"), &d);
    };
    let host = parts
        .headers
        .get(hyper::header::HOST)
        .map(|h| String::from_utf8_lossy(h.as_bytes()).into_owned())
        .or_else(|| parts.uri.authority().map(|a| a.to_string()))
        .unwrap_or_default();
    let mut r = Req {
        method: parts.method.as_str().to_string(),
        path,
        query: Query::parse(parts.uri.query().unwrap_or("")),
        headers: parts.headers,
        host,
        body: Vec::new(),
        body_too_large: false,
    };
    // The checks every request passes come before its body is read, as Go
    // reads a body only inside the POST handlers: a caller without the
    // token never gets the server to read one.
    let resp = match precheck(&s, port, &r) {
        Some(resp) => resp,
        None => {
            if r.method == "POST" {
                match http_body_util::Limited::new(body, MAX_REQUEST_BODY)
                    .collect()
                    .await
                {
                    Ok(b) => r.body = b.to_bytes().to_vec(),
                    Err(e) if e.is::<http_body_util::LengthLimitError>() => {
                        r.body_too_large = true;
                    }
                    // A stalled or broken body (the read deadline passed):
                    // hyper drops the connection after this answer.
                    Err(_) => return done(plain(400, "400 Bad Request"), &d),
                }
            }
            d.body_read();
            let s2 = s.clone();
            match tokio::task::spawn_blocking(move || dispatch(&s2, port, &r)).await {
                Ok(r) => r,
                Err(_) => http::error(500, "internal error"),
            }
        }
    };
    match resp {
        Resp::Full {
            status,
            headers,
            body,
        } => {
            let mut b = Response::builder().status(status);
            for (k, v) in headers {
                b = b.header(k, v);
            }
            let r = b
                .body(http_body_util::Full::new(Bytes::from(body)).boxed())
                .unwrap_or_else(|_| plain(500, "internal error"));
            done(r, &d)
        }
        Resp::Stream { run } => {
            // Streams lift the read and write deadlines for themselves.
            d.lift();
            let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(16);
            let sink = Sink {
                tx,
                stop: s.stop.clone(),
            };
            std::thread::Builder::new()
                .name("irori-api-stream".into())
                .spawn(move || run(sink))
                .expect("spawn a stream thread");
            Response::builder()
                .status(200)
                .header("Content-Type", "application/x-ndjson")
                .header("Cache-Control", "no-store")
                .header("X-Content-Type-Options", "nosniff")
                .body(BodyExt::boxed(DoneBody::new(ChanBody(rx), d.clone())))
                .unwrap_or_else(|_| plain(500, "internal error"))
        }
    }
}

/// `r` with a body that marks the connection idle once it is written.
fn done(r: Response<RespBody>, d: &Arc<Deadlines>) -> Response<RespBody> {
    let (parts, body) = r.into_parts();
    Response::from_parts(parts, BodyExt::boxed(DoneBody::new(body, d.clone())))
}

fn plain(status: u16, text: &str) -> Response<RespBody> {
    let mut r = Response::new(http_body_util::Full::new(Bytes::from(text.to_string())).boxed());
    *r.status_mut() = hyper::StatusCode::from_u16(status).unwrap_or_default();
    r
}

/// The checks every request passes before any route, in Go's order
/// (`handler.ServeHTTP`): Origin, Host, method, `/v1/health`, the token.
/// `None`: the request may be routed ([`dispatch`]).
fn precheck(s: &Arc<Shared>, port: u16, r: &Req) -> Option<Resp> {
    // Browsers always send Origin on cross-origin requests and on any
    // non-GET; no legitimate client of this API is a browser.
    if r.has_header("origin") {
        return Some(http::error(403, "browser requests are not allowed"));
    }
    let host = gocompat::text::to_lower_str(&r.host);
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return Some(http::error(403, "bad host"));
    }
    let post = crate::v02::post_route(&r.path) || crate::pair::pair_post(&r.path);
    if r.method != "GET" && !(post && r.method == "POST") {
        let allow = if post { "GET, POST" } else { "GET" };
        return Some(http::error(405, "method not allowed").with_header("Allow", allow));
    }
    if r.path == "/v1/health" {
        let d = s.b.device();
        return Some(http::json(
            200,
            &Health {
                ok: true,
                version: s.b.version(),
                device_id: d.id,
                net: s.b.net_health(),
            },
        ));
    }
    if !s.authorized(r) {
        return Some(
            http::error(401, "missing or wrong token")
                .with_header("WWW-Authenticate", r#"Bearer realm="irori""#),
        );
    }
    None
}

/// An authenticated request to its route.
fn dispatch(s: &Arc<Shared>, port: u16, r: &Req) -> Resp {
    let h = Handler { s, port, r };
    if let Some(resp) = crate::v02::route(&h)
        .or_else(|| crate::v03::route(&h))
        .or_else(|| crate::board::route(&h))
        .or_else(|| crate::pair::route(&h))
    {
        return resp;
    }
    match r.path.as_str() {
        "/v1/device" => http::json(200, &s.b.device()),
        "/v1/agents" => http::json(200, &s.b.agents()),
        "/v1/events" => crate::events::events(&h),
        "/v1/events/stream" => crate::events::stream(&h),
        "/v1/devices" => devices(s, r),
        _ => http::error(404, "no such route"),
    }
}

/// Go `devices`: `GET /v1/devices[?detail=1]`, the person's devices with
/// this one first; `detail` (`1` or `true`) adds what is behind each state.
/// A backend without a mesh answers 503, as Go's does.
fn devices(s: &Arc<Shared>, r: &Req) -> Resp {
    let d = r.query.get("detail");
    match s.b.mesh_devices(d == "1" || d == "true") {
        Some(rows) => http::json(200, &rows),
        None => http::error(503, "this Core has no devices layer"),
    }
}

/// One authenticated request.
pub(crate) struct Handler<'a> {
    pub s: &'a Arc<Shared>,
    pub port: u16,
    pub r: &'a Req,
}

impl Shared {
    fn authorized(&self, r: &Req) -> bool {
        if self.token.is_empty() {
            return false; // never serve with an empty token
        }
        let auth = r.header("authorization");
        let Some(got) = auth.strip_prefix("Bearer ") else {
            return false;
        };
        got.len() == self.token.len() && bool::from(got.as_bytes().ct_eq(&self.token))
    }

    /// Reads the person header: whether one was sent, and whether it is
    /// this run's secret (constant-time).
    pub fn as_person(&self, r: &Req) -> (bool, bool) {
        let vals = r.header_values(HEADER_PERSON);
        if vals.is_empty() {
            return (false, false);
        }
        let ok = vals.len() == 1
            && !self.person.is_empty()
            && vals[0].len() == self.person.len()
            && bool::from(vals[0].as_bytes().ct_eq(&self.person));
        (true, ok)
    }

    pub fn token_str(&self) -> String {
        String::from_utf8_lossy(&self.token).into_owned()
    }

    pub fn keepalive(&self) -> Duration {
        *self.keepalive.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn tools(&self) -> Option<Arc<dyn ToolSet>> {
        self.tools.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Who a request acts as, from exactly one identity header.
#[derive(Debug, Clone)]
pub(crate) struct Caller {
    pub sender: Sender,
    pub path: String,
    /// Through a mod attach secret (not a member).
    pub attach: bool,
}

impl Handler<'_> {
    /// `X-Irori-Attach` / `X-Irori-Member`; `None` when neither (or both)
    /// is present or the secret is unknown, superseded or closed.
    pub fn caller(&self) -> Option<Caller> {
        let (a, m) = (self.r.header(HEADER_ATTACH), self.r.header(HEADER_MEMBER));
        if !a.is_empty() && m.is_empty() {
            let st = self.s.streams.by_secret(&a)?;
            if st.path != inbox::path::MOD {
                return None;
            }
            let mut snd = st.sender.clone();
            if let Some(c) = self
                .s
                .b
                .agents()
                .into_iter()
                .find(|c| c.key == snd.key && !c.name.is_empty())
            {
                snd.name = c.name; // a renamed session sends under its new name
            }
            return Some(Caller {
                sender: snd,
                path: inbox::path::MOD.into(),
                attach: true,
            });
        }
        if !m.is_empty() && a.is_empty() {
            let mb = self.s.b.member_by_secret(&m)?;
            return Some(Caller {
                sender: Sender::new(&mb.key, &mb.name, "grok"),
                path: inbox::path::CHANNEL.into(),
                attach: false,
            });
        }
        None
    }

    /// Whether rows read as the person (this run's `X-Irori-Person`);
    /// `Err` is the 403 for a wrong one.
    pub fn rows_view(&self) -> Result<bool, Resp> {
        let (present, valid) = self.s.as_person(self.r);
        if present && !valid {
            return Err(http::error(
                403,
                &format!("wrong {HEADER_PERSON} secret (only the Irori app holds it)"),
            ));
        }
        Ok(valid)
    }

    /// Who a browsing request reads as: `""` (the person), the caller's
    /// key, or [`AGENT_VIEWER`] with no identity header. A header that is
    /// sent but doesn't resolve, or two at once, is 403.
    pub fn viewer(&self) -> Result<String, Resp> {
        let agent_hdr = self.r.has_header(HEADER_ATTACH) || self.r.has_header(HEADER_MEMBER);
        let (present, ok) = self.s.as_person(self.r);
        if present {
            if !ok || agent_hdr {
                return Err(http::error(
                    403,
                    &format!(
                        "wrong {HEADER_PERSON} secret (only the Irori app holds it; send it alone)"
                    ),
                ));
            }
            return Ok(String::new());
        }
        if !agent_hdr {
            return Ok(AGENT_VIEWER.into());
        }
        match self.caller() {
            Some(c) => Ok(c.sender.key),
            None => Err(http::error(
                403,
                &format!(
                    "unknown or superseded {HEADER_ATTACH} / {HEADER_MEMBER} secret (send at most one)"
                ),
            )),
        }
    }
}

// ---- the attach stream registry --------------------------------------------

/// One open attach stream (a mod session or a Grok member).
pub(crate) struct Stream {
    pub id: u64,
    pub key: String,
    pub path: String,
    pub sender: Sender,
    /// The attach secret; `""` for member streams.
    pub secret: String,
    /// Set when a newer stream for the key supersedes this one.
    pub kicked: AtomicBool,
    /// A mod at [`attach::APPEND_SINCE`] or later.
    pub appends: bool,
    /// The name event last sent (or the hello's).
    pub named: Mutex<attach::Event>,
}

#[derive(Default)]
pub(crate) struct Reg {
    pub by_key: HashMap<String, Arc<Stream>>,
    pub secrets: HashMap<String, Arc<Stream>>,
}

impl Reg {
    /// Whether `st` is still its key's live stream.
    pub fn current(&self, st: &Stream) -> bool {
        self.by_key.get(&st.key).is_some_and(|x| x.id == st.id)
    }
}

/// One live stream per agent key. The mutex also serializes wake issuing,
/// so a superseded stream can't take a wake after its successor has looked
/// for outstanding ones.
#[derive(Default)]
pub(crate) struct Streams {
    pub mu: Mutex<Reg>,
    next: AtomicU64,
}

impl Streams {
    pub fn lock(&self) -> MutexGuard<'_, Reg> {
        self.mu.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn by_secret(&self, secret: &str) -> Option<Arc<Stream>> {
        self.lock().secrets.get(secret).cloned()
    }

    pub fn next_id(&self) -> u64 {
        self.next.fetch_add(1, Ordering::SeqCst) + 1
    }
}

/// An attach secret: 24 random bytes, hex.
pub(crate) fn attach_secret() -> String {
    new_secret(24)
}
