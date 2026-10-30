//! A small client for a running Irori (I:internal/api/client.go,
//! client_events.go; surface.md §2.7, §3): found through `irori.json`,
//! loopback only, no proxy, no redirects. One connection per request.
//!
//! Errors never carry the token or a secret: a path's query is dropped
//! from every error text.

use std::path::Path;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use irori_core::lockfile::{self, Lockfile};

/// Probe and dial timeout.
pub const DIAL_TIMEOUT: Duration = Duration::from_secs(2);
/// A whole call's timeout (streams excepted).
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Response body cap.
pub const MAX_RESPONSE: usize = 32 << 20;
/// The longest events-stream line read whole (Go `maxEventLine`).
pub const MAX_EVENT_LINE: usize = 8 << 20;

/// Why a call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No Irori runs for that data dir (Go `discovery.ErrNotRunning`).
    NotRunning,
    /// The route answered with an error status (Go `api.StatusError`).
    Status {
        path: String,
        code: u16,
        message: String,
    },
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotRunning => f.write_str("irori is not running"),
            Error::Status {
                path,
                code,
                message,
            } => write!(f, "irori api {path}: {code} {message}"),
            Error::Other(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for Error {}

/// The path without its query (which may carry a secret).
fn bare(path: &str) -> String {
    path.split('?').next().unwrap_or("").to_string()
}

/// One answer.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A client for one Core.
#[derive(Clone)]
pub struct Client {
    port: u16,
    token: String,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").field("port", &self.port).finish()
    }
}

impl Client {
    /// A client from a lockfile, without probing it.
    pub fn new(lf: &Lockfile) -> Client {
        Client {
            port: lf.port as u16,
            token: lf.token.clone(),
        }
    }

    /// A client for `127.0.0.1:<port>` with `token` (tests).
    pub fn at(port: u16, token: &str) -> Client {
        Client {
            port,
            token: token.to_string(),
        }
    }

    /// Reads the lockfile in `data_dir` and checks `/v1/health`.
    pub async fn dial(data_dir: &Path) -> Result<Client, Error> {
        let lf = match lockfile::read(data_dir) {
            Ok(lf) => lf,
            Err(lockfile::Error::NotRunning) => return Err(Error::NotRunning),
            Err(e) => return Err(Error::Other(e.to_string())),
        };
        let c = Client::new(&lf);
        match tokio::time::timeout(DIAL_TIMEOUT, c.raw("GET", "/v1/health", &[], None)).await {
            Ok(Ok(r)) if r.status == 200 => Ok(c),
            _ => Err(Error::NotRunning),
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The bearer token. A secret: never log it.
    pub fn token(&self) -> &str {
        &self.token
    }

    async fn send(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<Vec<u8>>,
    ) -> Result<hyper::Response<Incoming>, Error> {
        let fail = |what: &str, e: &dyn std::fmt::Display| {
            Error::Other(format!("irori api {}: {what}: {e}", bare(path)))
        };
        let tcp = tokio::time::timeout(
            DIAL_TIMEOUT,
            tokio::net::TcpStream::connect(("127.0.0.1", self.port)),
        )
        .await
        .map_err(|e| fail("dial", &e))?
        .map_err(|e| fail("dial", &e))?;
        let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tcp))
            .await
            .map_err(|e| fail("handshake", &e))?;
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let mut b = hyper::Request::builder().method(method).uri(path);
        let mut has_host = false;
        let mut has_auth = false;
        for (k, v) in headers {
            has_host |= k.eq_ignore_ascii_case("host");
            has_auth |= k.eq_ignore_ascii_case("authorization");
            b = b.header(*k, *v);
        }
        if !has_host {
            b = b.header("Host", format!("127.0.0.1:{}", self.port));
        }
        if !has_auth {
            b = b.header("Authorization", format!("Bearer {}", self.token));
        }
        if body.is_some() {
            b = b.header("Content-Type", "application/json");
        }
        let req = b
            .body(http_body_util::Full::new(Bytes::from(
                body.unwrap_or_default(),
            )))
            .map_err(|e| fail("request", &e))?;
        sender.send_request(req).await.map_err(|e| fail("send", &e))
    }

    /// Any request, headers as given (a `Host` or `Authorization` given here
    /// replaces the default one), the answer whatever its status.
    pub async fn raw(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<Vec<u8>>,
    ) -> Result<Response, Error> {
        let resp = self.send(method, path, headers, body).await?;
        let status = resp.status().as_u16();
        let hs = resp
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect();
        let body = http_body_util::Limited::new(resp.into_body(), MAX_RESPONSE)
            .collect()
            .await
            .map_err(|e| Error::Other(format!("irori api {}: read: {e}", bare(path))))?
            .to_bytes()
            .to_vec();
        Ok(Response {
            status,
            headers: hs,
            body,
        })
    }

    /// A JSON call: the body on 200, else [`Error::Status`] with Go's
    /// `{"error"}` text.
    pub async fn call(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, Error> {
        let r = tokio::time::timeout(CALL_TIMEOUT, self.raw(method, path, headers, body))
            .await
            .map_err(|_| Error::Other(format!("irori api {}: timed out", bare(path))))??;
        if r.status == 200 {
            return Ok(r.body);
        }
        let message = serde_json::from_slice::<serde_json::Value>(&r.body)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
            .unwrap_or_else(|| String::from_utf8_lossy(&r.body).trim().to_string());
        Err(Error::Status {
            path: bare(path),
            code: r.status,
            message,
        })
    }

    /// Opens a stream whatever the answer: the lines and the headers on
    /// 200, else the whole refusal.
    pub async fn open(&self, path: &str, headers: &[(&str, &str)]) -> Result<Opened, Error> {
        let resp = self.send("GET", path, headers, None).await?;
        let status = resp.status().as_u16();
        let hs: Vec<(String, String)> = resp
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect();
        if status == 200 {
            return Ok(Opened::Stream(
                Lines {
                    body: resp.into_body(),
                    buf: Vec::new(),
                    done: false,
                },
                hs,
            ));
        }
        let body = resp
            .into_body()
            .collect()
            .await
            .map(|b| b.to_bytes().to_vec())
            .unwrap_or_default();
        Ok(Opened::Refused(Response {
            status,
            headers: hs,
            body,
        }))
    }

    /// Opens an NDJSON stream (`/v1/events/stream`, `/v1/attach`).
    pub async fn stream(&self, path: &str, headers: &[(&str, &str)]) -> Result<Lines, Error> {
        let resp = self.send("GET", path, headers, None).await?;
        let status = resp.status().as_u16();
        if status != 200 {
            let body = resp
                .into_body()
                .collect()
                .await
                .map(|b| b.to_bytes().to_vec())
                .unwrap_or_default();
            let message = serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
                .unwrap_or_default();
            return Err(Error::Status {
                path: bare(path),
                code: status,
                message,
            });
        }
        Ok(Lines {
            body: resp.into_body(),
            buf: Vec::new(),
            done: false,
        })
    }

    /// This device's id (Go `Client.Device`'s `ID`, `GET /v1/device`).
    pub async fn device_id(&self) -> Result<String, Error> {
        let raw = self.call("GET", "/v1/device", &[], None).await?;
        let v: serde_json::Value = serde_json::from_slice(&raw)
            .map_err(|e| Error::Other(format!("irori api /v1/device: {e}")))?;
        Ok(v.get("id")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string())
    }

    /// The newest `limit` rows (0: the route's default) of `kinds`, newest
    /// first (Go `Client.Events`: `GET /v1/events?kind=…&limit=N`).
    pub async fn events(&self, kinds: &[String], limit: i64) -> Result<Vec<store::Event>, Error> {
        // url.Values.Encode: keys sorted, a key's values in order.
        let mut q: Vec<String> = kinds
            .iter()
            .map(|k| format!("kind={}", query_escape(k)))
            .collect();
        if limit > 0 {
            q.push(format!("limit={limit}"));
        }
        let mut path = "/v1/events".to_string();
        if !q.is_empty() {
            path = format!("{path}?{}", q.join("&"));
        }
        let raw = self.call("GET", &path, &[], None).await?;
        let rows: Option<Vec<Row>> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Other(format!("irori api /v1/events: {e}")))?;
        Ok(rows
            .unwrap_or_default()
            .into_iter()
            .map(Row::into_event)
            .collect())
    }

    /// The newest seq of `device_id`'s rows, 0 when it has none (Go
    /// `LastSeq`): a `since` that resumes after every row written.
    pub async fn last_seq(&self, device_id: &str) -> Result<i64, Error> {
        let raw = self
            .call(
                "GET",
                &format!("/v1/events?limit=1&device={}", query_escape(device_id)),
                &[],
                None,
            )
            .await?;
        let v: serde_json::Value =
            serde_json::from_slice(&raw).map_err(|e| Error::Other(e.to_string()))?;
        Ok(v.get(0)
            .and_then(|e| e.get("seq"))
            .and_then(|s| s.as_i64())
            .unwrap_or(0))
    }
}

/// Go `url.QueryEscape`.
pub fn query_escape(s: &str) -> String {
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

/// What [`Client::open`] got.
pub enum Opened {
    /// 200: the lines and the response headers.
    Stream(Lines, Vec<(String, String)>),
    Refused(Response),
}

/// The lines of an NDJSON stream, keepalives included (as empty lines).
pub struct Lines {
    body: Incoming,
    buf: Vec<u8>,
    done: bool,
}

impl Lines {
    /// The next line without its `\n`; `None` at the end of the stream.
    pub async fn next(&mut self) -> Option<Vec<u8>> {
        loop {
            if let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=i).collect();
                return Some(line[..line.len() - 1].to_vec());
            }
            if self.done {
                if self.buf.is_empty() {
                    return None;
                }
                return Some(std::mem::take(&mut self.buf));
            }
            match self.body.frame().await {
                Some(Ok(f)) => {
                    if let Ok(d) = f.into_data() {
                        self.buf.extend_from_slice(&d);
                    }
                }
                _ => self.done = true,
            }
        }
    }

    /// The next line that isn't a keepalive, waiting at most `d`.
    pub async fn next_json(&mut self, d: Duration) -> Option<Vec<u8>> {
        tokio::time::timeout(d, async {
            loop {
                match self.next().await {
                    Some(l) if l.iter().all(|b| b.is_ascii_whitespace()) => continue,
                    other => return other,
                }
            }
        })
        .await
        .ok()
        .flatten()
    }
}

/// `since` for [`Client::events_stream`] that replays nothing (Go
/// `NoReplay`).
pub const NO_REPLAY: i64 = -1;

/// One row of the events stream, as `GET /v1/events` writes it.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Row {
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub seq: i64,
    pub hlc: u64,
    pub actor: String,
    pub kind: String,
    /// The payload's JSON as sent.
    pub payload: Box<serde_json::value::RawValue>,
}

impl Row {
    /// The row as a [`store::Event`] (no signature; JSON never carries it).
    pub fn into_event(self) -> store::Event {
        store::Event {
            device_id: self.device_id,
            seq: self.seq,
            hlc: store::Timestamp(self.hlc),
            actor: self.actor,
            kind: self.kind,
            payload: self.payload.get().as_bytes().to_vec(),
            sig: Vec::new(),
        }
    }
}

/// One item of [`Client::events_stream`] (Go `StreamItem`): a row, a
/// resync (rows were dropped: re-read), or an error just before the end.
/// A resync for a line over [`MAX_EVENT_LINE`] names the skipped row when
/// its head was readable.
#[derive(Debug, Default)]
pub struct StreamItem {
    pub row: Option<Row>,
    pub resync: bool,
    pub err: Option<String>,
    pub skipped_device: String,
    pub skipped_seq: i64,
}

impl Client {
    /// Opens `GET /v1/events/stream`: `kinds` exact or prefixes ending in
    /// `.`; `since >= 0` replays this device's rows after that seq,
    /// [`NO_REPLAY`] starts live. A refusal comes back at once; then items
    /// arrive on the channel until the stream ends (drop it to stop).
    pub async fn events_stream(
        &self,
        kinds: &[String],
        since: i64,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamItem>, Error> {
        let since = if since >= 0 {
            since.to_string()
        } else {
            String::new()
        };
        self.events_stream_q(kinds, &since).await
    }

    /// [`Client::events_stream`] with a vector `since`: each device named
    /// is replayed after its seq, the rest arrive live.
    pub async fn events_stream_from(
        &self,
        kinds: &[String],
        since: &std::collections::BTreeMap<String, i64>,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamItem>, Error> {
        let v: Vec<String> = since.iter().map(|(d, s)| format!("{d}:{s}")).collect();
        self.events_stream_q(kinds, &v.join(",")).await
    }

    async fn events_stream_q(
        &self,
        kinds: &[String],
        since: &str,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamItem>, Error> {
        // url.Values.Encode: keys sorted.
        let mut q = Vec::new();
        if !kinds.is_empty() {
            q.push(format!("kinds={}", query_escape(&kinds.join(","))));
        }
        if !since.is_empty() {
            q.push(format!("since={}", query_escape(since)));
        }
        let mut path = "/v1/events/stream".to_string();
        if !q.is_empty() {
            path = format!("{path}?{}", q.join("&"));
        }
        let mut lines = self
            .stream(&path, &[("Accept", "application/x-ndjson")])
            .await?;
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        tokio::spawn(async move {
            loop {
                let (line, skipped, end) = lines.next_capped(MAX_EVENT_LINE).await;
                if let Some(head) = skipped {
                    let (d, s) = line_head(&head);
                    let it = StreamItem {
                        resync: true,
                        skipped_device: d,
                        skipped_seq: s,
                        ..Default::default()
                    };
                    if tx.send(it).await.is_err() {
                        return;
                    }
                }
                if let Some(line) = line {
                    let line = line.trim_ascii();
                    if !line.is_empty() {
                        let it = parse_stream_line(line).unwrap_or_else(|e| StreamItem {
                            err: Some(format!("irori api /v1/events/stream: bad line: {e}")),
                            ..Default::default()
                        });
                        let bad = it.err.is_some();
                        if tx.send(it).await.is_err() || bad {
                            return;
                        }
                    }
                }
                if end {
                    return;
                }
            }
        });
        Ok(rx)
    }
}

fn parse_stream_line(line: &[u8]) -> Result<StreamItem, String> {
    #[derive(serde::Deserialize)]
    struct Head {
        #[serde(default)]
        resync: bool,
    }
    let h: Head = serde_json::from_slice(line).map_err(|e| e.to_string())?;
    if h.resync {
        return Ok(StreamItem {
            resync: true,
            ..Default::default()
        });
    }
    let row: Row = serde_json::from_slice(line).map_err(|e| e.to_string())?;
    Ok(StreamItem {
        row: Some(row),
        ..Default::default()
    })
}

impl Lines {
    /// Go `readEventLine`: the next line, or for one longer than `cap` its
    /// first 64 KiB as `skipped` (the rest is read past); `end` once the
    /// stream is over.
    pub async fn next_capped(&mut self, cap: usize) -> (Option<Vec<u8>>, Option<Vec<u8>>, bool) {
        const HEAD: usize = 64 << 10;
        let mut over: Option<Vec<u8>> = None;
        loop {
            if let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
                if over.is_some() {
                    self.buf.drain(..=i);
                    return (None, over, false);
                }
                if i + 1 > cap {
                    let head = self.buf[..HEAD.min(i)].to_vec();
                    self.buf.drain(..=i);
                    return (None, Some(head), false);
                }
                let line: Vec<u8> = self.buf.drain(..=i).collect();
                return (Some(line[..line.len() - 1].to_vec()), None, false);
            }
            if over.is_none() && self.buf.len() > cap {
                over = Some(self.buf[..HEAD.min(self.buf.len())].to_vec());
            }
            if over.is_some() {
                self.buf.clear();
            }
            if self.done {
                if over.is_some() {
                    return (None, over, true);
                }
                let rest = std::mem::take(&mut self.buf);
                return (if rest.is_empty() { None } else { Some(rest) }, None, true);
            }
            match self.body.frame().await {
                Some(Ok(f)) => {
                    if let Ok(d) = f.into_data() {
                        self.buf.extend_from_slice(&d);
                    }
                }
                _ => self.done = true,
            }
        }
    }
}

/// Go `lineHead`: `deviceId` and `seq` from the start of a truncated
/// events line, in either order, before any nested value; `("", 0)` when
/// the start does not have both.
pub fn line_head(head: &[u8]) -> (String, i64) {
    line_head_opt(head).unwrap_or_default()
}

fn skip_ws(b: &[u8], i: &mut usize) {
    while *i < b.len() && matches!(b[*i], b' ' | b'\t' | b'\n' | b'\r') {
        *i += 1;
    }
}

/// A whole JSON string literal starting at `b[*i] == '"'`.
fn json_string(b: &[u8], i: &mut usize) -> Option<String> {
    let start = *i;
    *i += 1;
    while *i < b.len() {
        match b[*i] {
            b'\\' => *i += 2,
            b'"' => {
                *i += 1;
                return serde_json::from_slice::<String>(&b[start..*i]).ok();
            }
            _ => *i += 1,
        }
    }
    None
}

fn line_head_opt(b: &[u8]) -> Option<(String, i64)> {
    let mut i = 0;
    skip_ws(b, &mut i);
    if b.get(i) != Some(&b'{') {
        return None;
    }
    i += 1;
    let (mut dev, mut seq): (Option<String>, Option<i64>) = (None, None);
    loop {
        if let (Some(d), Some(s)) = (&dev, seq) {
            return Some((d.clone(), s));
        }
        skip_ws(b, &mut i);
        if b.get(i) == Some(&b',') {
            i += 1;
            skip_ws(b, &mut i);
        }
        if b.get(i) != Some(&b'"') {
            return None;
        }
        let key = json_string(b, &mut i)?;
        skip_ws(b, &mut i);
        if b.get(i) != Some(&b':') {
            return None;
        }
        i += 1;
        skip_ws(b, &mut i);
        match *b.get(i)? {
            b'"' => {
                let v = json_string(b, &mut i)?;
                match key.as_str() {
                    "deviceId" => dev = Some(v),
                    "seq" => return None,
                    _ => {}
                }
            }
            b'{' | b'[' => return None, // a nested value before both fields
            _ => {
                let start = i;
                while i < b.len() && !matches!(b[i], b',' | b'}' | b' ' | b'\t' | b'\n' | b'\r') {
                    i += 1;
                }
                if i >= b.len() {
                    return None; // the value may be cut short
                }
                let lit = std::str::from_utf8(&b[start..i]).ok()?;
                match key.as_str() {
                    "seq" => seq = Some(lit.parse().ok()?),
                    "deviceId" => return None,
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // I:internal/api/events_test.go TestLineHead.
    #[test]
    fn line_head_table() {
        for (head, dev, seq) in [
            (
                r#"{"deviceId":"dev1","seq":7,"hlc":1,"payload":{"name":"xx"#,
                "dev1",
                7,
            ),
            (r#"{"seq":9,"deviceId":"dev2","pay"#, "dev2", 9),
            (r#"{"deviceId":"dev1","payload":{"seq":3},"seq":4}"#, "", 0),
            (r#"{"deviceId":"dev1","se"#, "", 0),
            ("not json", "", 0),
        ] {
            assert_eq!(line_head(head.as_bytes()), (dev.to_string(), seq), "{head}");
        }
    }

    #[test]
    fn query_escape_like_go() {
        assert_eq!(query_escape("a b/c:d,e~"), "a+b%2Fc%3Ad%2Ce~");
    }
}
