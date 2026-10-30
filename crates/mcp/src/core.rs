//! The slice of the Core's loopback API `irori mcp` uses
//! (I:internal/mcpserver/core.go): [`Core`], and [`HttpCore`] over a
//! minimal HTTP/1.1 client on 127.0.0.1.
//!
//! The client never follows redirects, never uses a proxy, dials in 2 s,
//! gives each call 30 s and reads at most 4 MiB of a JSON answer. The member
//! secret rides in `/v1/attach?member=`, so no error built here carries a
//! URL with its query.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use gocompat::json::RawJson;
use serde::{Deserialize, Serialize};

/// One dial (Go `dialTimeout`).
pub const DIAL_TIMEOUT: Duration = Duration::from_secs(2);
/// One non-streaming request (Go `callTimeout`).
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// A JSON answer read from the Core (Go `maxCoreBody`).
pub const MAX_CORE_BODY: usize = 4 << 20;

/// POST /v1/members' answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Member {
    #[serde(rename = "memberKey", default)]
    pub key: String,
    #[serde(rename = "memberSecret", default)]
    pub secret: String,
    #[serde(default)]
    pub name: String,
}

/// Why a Core call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// A non-2xx answer (Go `api.StatusError`).
    Status {
        path: String,
        code: u16,
        message: String,
    },
    /// The request never reached a server (a dial error), so even a
    /// mutating call may be retried.
    Unsent(String),
    /// No Core answered on this device (Go `ErrNotRunning`).
    NotRunning(String),
    Other(String),
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreError::Status {
                path,
                code,
                message,
            } => {
                let msg = if message.is_empty() {
                    status_text(*code)
                } else {
                    message
                };
                write!(f, "irori api {path}: {code} {msg}")
            }
            CoreError::Unsent(s) | CoreError::Other(s) => f.write_str(s),
            CoreError::NotRunning(s) => {
                if s.is_empty() {
                    f.write_str("irori is not running")
                } else {
                    write!(f, "irori is not running: {s}")
                }
            }
        }
    }
}

impl std::error::Error for CoreError {}

impl CoreError {
    /// The HTTP status, or 0 for transport failures.
    pub fn code(&self) -> u16 {
        match self {
            CoreError::Status { code, .. } => *code,
            _ => 0,
        }
    }

    /// The request never reached a server.
    pub fn unsent(&self) -> bool {
        matches!(self, CoreError::Unsent(_) | CoreError::NotRunning(_))
    }
}

fn status_text(code: u16) -> &'static str {
    match code {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Request Entity Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "",
    }
}

/// A member's wake stream: NDJSON lines until it ends.
pub trait EventStream: Send {
    /// The next line (without its newline); `None` at the end.
    fn next_line(&mut self) -> Option<Result<Vec<u8>, String>>;
}

/// Ends a stream from another thread.
pub type Closer = Arc<dyn Fn() + Send + Sync>;

/// The Core API `irori mcp` uses (Go `mcpserver.Core`).
pub trait Core: Send + Sync {
    /// GET /v1/tools?client=.
    fn tools(&self, client: &str) -> Result<Vec<tools::Spec>, CoreError>;
    /// POST /v1/tools/{name} `{"args"}`; `member` "" sends no identity.
    fn call_tool(&self, member: &str, name: &str, args: &[u8]) -> Result<String, CoreError>;
    /// POST /v1/members.
    fn join(&self, name: &str, session: &str) -> Result<Member, CoreError>;
    /// GET /v1/attach?member=.
    fn attach(&self, member_secret: &str) -> Result<(Box<dyn EventStream>, Closer), CoreError>;
    /// The Core run this client talks to: base + token, which changes
    /// exactly when the Core restarts.
    fn run_id(&self) -> String;
}

/// Connects to the running Core (any error means "not running").
pub type Dialer = Arc<dyn Fn() -> Result<Arc<dyn Core>, String> + Send + Sync>;

/// The API base for a lockfile port.
pub fn loopback_url(port: i64) -> String {
    format!("http://127.0.0.1:{port}")
}

/// [`Core`] over the loopback API at `base` (`http://127.0.0.1:<port>`)
/// with the lockfile's bearer token.
pub struct HttpCore {
    base: String,
    addr: String,
    token: String,
}

impl HttpCore {
    pub fn new(base: &str, token: &str) -> HttpCore {
        let base = base.trim_end_matches('/').to_string();
        let addr = base
            .strip_prefix("http://")
            .unwrap_or(&base)
            .split('/')
            .next()
            .unwrap_or("")
            .to_string();
        HttpCore {
            base,
            addr,
            token: token.into(),
        }
    }

    /// GET /v1/health with no token: whether an Irori answers with `ok`,
    /// and its device id.
    pub fn health(&self) -> Result<(bool, String), CoreError> {
        let (code, body) =
            self.roundtrip("GET", "/v1/health", "", false, "", None, DIAL_TIMEOUT)?;
        if !(200..300).contains(&code) {
            return Err(status_error("/v1/health", code, &body));
        }
        #[derive(Deserialize)]
        struct H {
            #[serde(default)]
            ok: bool,
            #[serde(rename = "deviceId", default)]
            device_id: String,
        }
        let h: H = serde_json::from_slice(&body)
            .map_err(|e| CoreError::Other(format!("irori api /v1/health: bad response: {e}")))?;
        Ok((h.ok, h.device_id))
    }

    fn connect(&self, path: &str, timeout: Duration) -> Result<TcpStream, CoreError> {
        let addr: SocketAddr = self
            .addr
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.next())
            .ok_or_else(|| CoreError::Unsent(format!("irori api {path}: bad address")))?;
        let s = TcpStream::connect_timeout(&addr, DIAL_TIMEOUT)
            .map_err(|e| CoreError::Unsent(format!("irori api {path}: dial tcp {addr}: {e}")))?;
        let _ = s.set_nodelay(true);
        let t = (!timeout.is_zero()).then_some(timeout);
        let _ = s.set_read_timeout(t);
        let _ = s.set_write_timeout(Some(CALL_TIMEOUT));
        Ok(s)
    }

    #[allow(clippy::too_many_arguments)]
    fn write_request(
        &self,
        s: &mut TcpStream,
        method: &str,
        target: &str,
        member: &str,
        auth: bool,
        accept: &str,
        body: Option<&[u8]>,
    ) -> std::io::Result<()> {
        let mut req = format!("{method} {target} HTTP/1.1\r\nHost: {}\r\n", self.addr);
        if auth {
            req += &format!("Authorization: Bearer {}\r\n", self.token);
        }
        if !accept.is_empty() {
            req += &format!("Accept: {accept}\r\n");
        }
        if !member.is_empty() {
            req += &format!("X-Irori-Member: {member}\r\n");
        }
        if let Some(b) = body {
            req += &format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                b.len()
            );
        }
        req += "Connection: close\r\n\r\n";
        s.write_all(req.as_bytes())?;
        if let Some(b) = body {
            s.write_all(b)?;
        }
        s.flush()
    }

    #[allow(clippy::too_many_arguments)]
    fn roundtrip(
        &self,
        method: &str,
        path: &str,
        query: &str,
        auth: bool,
        member: &str,
        body: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<(u16, Vec<u8>), CoreError> {
        let mut s = self.connect(path, timeout)?;
        let target = if query.is_empty() {
            path.to_string()
        } else {
            format!("{path}?{query}")
        };
        self.write_request(
            &mut s,
            method,
            &target,
            member,
            auth,
            "application/json",
            body,
        )
        .map_err(|e| CoreError::Other(format!("irori api {path}: {e}")))?;
        let mut r = BufReader::new(s);
        let (code, chunked, len) =
            read_head(&mut r).map_err(|e| CoreError::Other(format!("irori api {path}: {e}")))?;
        let body = read_body(&mut r, chunked, len, MAX_CORE_BODY)
            .map_err(|e| CoreError::Other(format!("irori api {path}: {e}")))?;
        Ok((code, body))
    }

    fn do_json(
        &self,
        method: &str,
        path: &str,
        query: &str,
        member: &str,
        body: Option<&[u8]>,
    ) -> Result<Vec<u8>, CoreError> {
        let (code, data) = self.roundtrip(method, path, query, true, member, body, CALL_TIMEOUT)?;
        if !(200..300).contains(&code) {
            return Err(status_error(path, code, &data));
        }
        Ok(data)
    }
}

fn status_error(path: &str, code: u16, body: &[u8]) -> CoreError {
    #[derive(Deserialize)]
    struct E {
        #[serde(default)]
        error: String,
    }
    let message = serde_json::from_slice::<E>(body)
        .map(|e| e.error)
        .unwrap_or_default();
    CoreError::Status {
        path: path.into(),
        code,
        message,
    }
}

/// Reads the status line and headers: (status, chunked, content length).
fn read_head(r: &mut impl BufRead) -> std::io::Result<(u16, bool, Option<usize>)> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "EOF before the status line",
        ));
    }
    let code: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| std::io::Error::other(format!("malformed status line {line:?}")))?;
    let (mut chunked, mut len) = (false, None);
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        let Some((k, v)) = h.split_once(':') else {
            continue;
        };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        if k == "transfer-encoding" && v.eq_ignore_ascii_case("chunked") {
            chunked = true;
        } else if k == "content-length" {
            len = v.parse().ok();
        }
    }
    Ok((code, chunked, len))
}

fn read_body(
    r: &mut impl BufRead,
    chunked: bool,
    len: Option<usize>,
    max: usize,
) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    if chunked {
        let mut c = Chunked {
            r,
            left: 0,
            done: false,
        };
        c.by_ref().take(max as u64).read_to_end(&mut out)?;
    } else if let Some(n) = len {
        r.take(n.min(max) as u64).read_to_end(&mut out)?;
    } else {
        r.take(max as u64).read_to_end(&mut out)?;
    }
    Ok(out)
}

/// A chunked body as a plain reader.
struct Chunked<R> {
    r: R,
    left: usize,
    done: bool,
}

impl<R: BufRead> Read for Chunked<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            let mut line = String::new();
            if self.r.read_line(&mut line)? == 0 {
                self.done = true;
                return Ok(0);
            }
            let size = line.trim().split(';').next().unwrap_or("");
            let n = usize::from_str_radix(size, 16)
                .map_err(|_| std::io::Error::other(format!("bad chunk size {size:?}")))?;
            if n == 0 {
                self.done = true;
                return Ok(0);
            }
            self.left = n;
        }
        let want = buf.len().min(self.left);
        let got = self.r.read(&mut buf[..want])?;
        if got == 0 {
            self.done = true;
            return Ok(0);
        }
        self.left -= got;
        if self.left == 0 {
            let mut crlf = String::new();
            self.r.read_line(&mut crlf)?;
        }
        Ok(got)
    }
}

/// Go `url.QueryEscape`.
pub(crate) fn query_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Go `url.PathEscape`.
fn path_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b'$'
            | b'&'
            | b'+'
            | b','
            | b':'
            | b'='
            | b'@' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

struct HttpStream {
    r: BufReader<Chunked<BufReader<TcpStream>>>,
}

impl EventStream for HttpStream {
    fn next_line(&mut self) -> Option<Result<Vec<u8>, String>> {
        let mut line = Vec::new();
        match self.r.read_until(b'\n', &mut line) {
            Ok(0) => None,
            Ok(_) => {
                if line.last() == Some(&b'\n') {
                    line.pop();
                }
                Some(Ok(line))
            }
            Err(e) => Some(Err(e.to_string())),
        }
    }
}

struct PlainStream {
    r: BufReader<TcpStream>,
}

impl EventStream for PlainStream {
    fn next_line(&mut self) -> Option<Result<Vec<u8>, String>> {
        let mut line = Vec::new();
        match self.r.read_until(b'\n', &mut line) {
            Ok(0) => None,
            Ok(_) => {
                if line.last() == Some(&b'\n') {
                    line.pop();
                }
                Some(Ok(line))
            }
            Err(e) => Some(Err(e.to_string())),
        }
    }
}

impl Core for HttpCore {
    fn tools(&self, client: &str) -> Result<Vec<tools::Spec>, CoreError> {
        let data = self.do_json(
            "GET",
            "/v1/tools",
            &format!("client={}", query_escape(client)),
            "",
            None,
        )?;
        tools::parse_specs(&data)
            .map_err(|e| CoreError::Other(format!("irori api /v1/tools: bad response: {e}")))
    }

    fn call_tool(&self, member: &str, name: &str, args: &[u8]) -> Result<String, CoreError> {
        #[derive(Serialize)]
        struct Body {
            args: RawJson,
        }
        let trimmed = args.trim_ascii();
        let args = if trimmed.is_empty() {
            b"{}" as &[u8]
        } else {
            trimmed
        };
        let body = gocompat::json::marshal(&Body {
            args: RawJson(args.to_vec()),
        })
        .map_err(|e| CoreError::Other(e.to_string()))?;
        let path = format!("/v1/tools/{}", path_escape(name));
        let data = self.do_json("POST", &path, "", member, Some(&body))?;
        #[derive(Deserialize)]
        struct Out {
            #[serde(default)]
            text: String,
        }
        let o: Out = serde_json::from_slice(&data)
            .map_err(|e| CoreError::Other(format!("irori api {path}: bad response: {e}")))?;
        Ok(o.text)
    }

    fn join(&self, name: &str, session: &str) -> Result<Member, CoreError> {
        #[derive(Serialize)]
        struct In<'a> {
            name: &'a str,
            #[serde(skip_serializing_if = "str::is_empty")]
            session: &'a str,
        }
        let body = gocompat::json::marshal(&In { name, session })
            .map_err(|e| CoreError::Other(e.to_string()))?;
        let data = self.do_json("POST", "/v1/members", "", "", Some(&body))?;
        let m: Member = serde_json::from_slice(&data)
            .map_err(|e| CoreError::Other(format!("irori api /v1/members: bad response: {e}")))?;
        if m.secret.is_empty() {
            return Err(CoreError::Other(
                "irori api /v1/members: answer has no memberSecret".into(),
            ));
        }
        Ok(m)
    }

    fn attach(&self, member_secret: &str) -> Result<(Box<dyn EventStream>, Closer), CoreError> {
        let path = "/v1/attach";
        let mut s = self.connect(path, Duration::ZERO)?;
        let target = format!("{path}?member={}", query_escape(member_secret));
        // Errors name the path only: the query holds the member secret.
        self.write_request(
            &mut s,
            "GET",
            &target,
            "",
            true,
            "application/x-ndjson",
            None,
        )
        .map_err(|e| CoreError::Other(format!("irori api {path}: {e}")))?;
        let closer_sock = s
            .try_clone()
            .map_err(|e| CoreError::Other(format!("irori api {path}: {e}")))?;
        let closer: Closer = Arc::new(move || {
            let _ = closer_sock.shutdown(Shutdown::Both);
        });
        let mut r = BufReader::new(s);
        let (code, chunked, len) =
            read_head(&mut r).map_err(|e| CoreError::Other(format!("irori api {path}: {e}")))?;
        if !(200..300).contains(&code) {
            let body = read_body(&mut r, chunked, len, 64 << 10).unwrap_or_default();
            return Err(status_error(path, code, &body));
        }
        let stream: Box<dyn EventStream> = if chunked {
            Box::new(HttpStream {
                r: BufReader::new(Chunked {
                    r,
                    left: 0,
                    done: false,
                }),
            })
        } else {
            Box::new(PlainStream { r })
        };
        Ok((stream, closer))
    }

    fn run_id(&self) -> String {
        format!("{}|{}", self.base, self.token)
    }
}
