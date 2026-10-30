//! The little of the loopback API client `irori attach` needs (Go
//! `api.Dial` and `Client.OpenStream`, I:internal/api/client.go,
//! client_v02.go): read the lockfile, probe `/v1/health`, open
//! `/v1/attach`. HTTP/1.1 straight over a 127.0.0.1 socket: no proxy, a
//! 2 s connect, no redirects followed.
//!
//! Merge note: when the API crate's client lands, the runner can take its
//! `Dial`/`OpenStream` instead; the error split below ([`Error::Status`]
//! for an HTTP answer, everything else an outage) is what the runner
//! branches on.

use std::path::Path;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;

/// Bounds the connect and the health probe (Go `probeTimeout`).
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Caps an error or health body the client reads.
const MAX_SMALL_BODY: usize = 64 << 10;

/// A non-2xx answer (Go `api.StatusError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusError {
    pub path: String,
    pub code: u16,
    /// The server's `{"error"}` text, if any.
    pub message: String,
}

impl std::fmt::Display for StatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = if self.message.is_empty() {
            hyper::StatusCode::from_u16(self.code)
                .ok()
                .and_then(|c| c.canonical_reason())
                .unwrap_or("")
                .to_string()
        } else {
            self.message.clone()
        };
        write!(f, "irori api {}: {} {msg}", self.path, self.code)
    }
}

/// Why a dial or an open failed.
#[derive(Debug)]
pub enum Error {
    /// The server answered with an error status.
    Status(StatusError),
    /// No Core (no lockfile, a dead pid, nothing on the port, an unreadable
    /// lockfile) or a broken connection: an outage to the runner.
    Down(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Status(s) => s.fmt(f),
            Error::Down(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for Error {}

/// A Core found through its lockfile.
pub struct Client {
    port: u16,
    /// The bearer token. A secret: never logged or printed.
    token: String,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

fn down(e: impl std::fmt::Display) -> Error {
    Error::Down(e.to_string())
}

/// Go `api.Dial`: the lockfile in `data_dir`, then a health probe that must
/// answer ok for the lockfile's device.
pub async fn dial(data_dir: &Path) -> Result<Client, Error> {
    let lf = irori_core::lockfile::read(data_dir).map_err(down)?;
    let port = u16::try_from(lf.port).map_err(|_| down(format!("bad port {}", lf.port)))?;
    let c = Client {
        port,
        token: lf.token,
    };
    let health = tokio::time::timeout(PROBE_TIMEOUT, c.get_small("/v1/health", false)).await;
    let (status, body) = match health {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            return Err(down(format!(
                "irori is not running: no answer on port {port}: {e}"
            )));
        }
        Err(_) => {
            return Err(down(format!(
                "irori is not running: no answer on port {port}: timed out"
            )));
        }
    };
    if !(200..300).contains(&status) {
        return Err(Error::Status(status_error("/v1/health", status, &body)));
    }
    use gocompat::json::lenient::{self, Out, Schema};
    let schema = Schema::object(vec![("ok", Schema::Bool), ("deviceId", Schema::String)]);
    let h = lenient::unmarshal(&body, &schema)
        .map_err(|e| down(format!("irori api /v1/health: bad response: {e}")))?;
    let ok = matches!(h.get("ok"), Some(Out::Bool(true)));
    if !ok || (!lf.device_id.is_empty() && h.get_str("deviceId") != lf.device_id) {
        return Err(down(format!(
            "irori is not running: port {port} is not the irori in {}",
            data_dir.display()
        )));
    }
    Ok(c)
}

fn status_error(path: &str, code: u16, body: &[u8]) -> StatusError {
    use gocompat::json::lenient::{self, Schema};
    let schema = Schema::object(vec![("error", Schema::String)]);
    let message = lenient::unmarshal(body, &schema)
        .map(|o| o.get_str("error").to_string())
        .unwrap_or_default();
    StatusError {
        path: path.into(),
        code,
        message,
    }
}

/// Go `url.QueryEscape`.
fn query_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &c in s.as_bytes() {
        match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(c as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{c:02X}")),
        }
    }
    out
}

/// Go `url.Values.Encode`: keys sorted, values escaped.
pub fn encode_query(pairs: &[(&str, &str)]) -> String {
    let mut v: Vec<_> = pairs.to_vec();
    v.sort_by(|a, b| a.0.cmp(b.0));
    v.iter()
        .map(|(k, val)| format!("{}={}", query_escape(k), query_escape(val)))
        .collect::<Vec<_>>()
        .join("&")
}

impl Client {
    pub fn port(&self) -> u16 {
        self.port
    }

    async fn request(
        &self,
        path_q: &str,
        auth: bool,
        accept: &str,
    ) -> Result<hyper::Response<Incoming>, Error> {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], self.port));
        let tcp = tokio::time::timeout(PROBE_TIMEOUT, TcpStream::connect(addr))
            .await
            .map_err(|_| down("dial tcp: i/o timeout"))?
            .map_err(down)?;
        let (mut send, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tcp))
            .await
            .map_err(down)?;
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let mut req = hyper::Request::get(path_q)
            .header("Host", format!("127.0.0.1:{}", self.port))
            .header("Accept", accept)
            .header("User-Agent", "irori-rust-dev");
        if auth {
            req = req.header("Authorization", format!("Bearer {}", self.token));
        }
        let req = req.body(Empty::<Bytes>::new()).map_err(down)?;
        send.send_request(req).await.map_err(down)
    }

    /// A GET whose whole body is small.
    async fn get_small(&self, path: &str, auth: bool) -> Result<(u16, Vec<u8>), Error> {
        let resp = self.request(path, auth, "application/json").await?;
        let status = resp.status().as_u16();
        let body = read_capped(resp.into_body(), MAX_SMALL_BODY).await?;
        Ok((status, body))
    }

    /// Go `Client.OpenStream`: `GET /v1/attach?<query>`, its body on 200.
    /// The query may hold a member secret, so no error text carries it.
    pub async fn open_stream(&self, query: &str) -> Result<Incoming, Error> {
        let resp = self
            .request(&format!("/v1/attach?{query}"), true, "application/x-ndjson")
            .await
            .map_err(|e| match e {
                Error::Down(s) => Error::Down(format!("irori api /v1/attach: {s}")),
                other => other,
            })?;
        let status = resp.status().as_u16();
        if status != 200 {
            let body = read_capped(resp.into_body(), MAX_SMALL_BODY)
                .await
                .unwrap_or_default();
            return Err(Error::Status(status_error("/v1/attach", status, &body)));
        }
        Ok(resp.into_body())
    }
}

async fn read_capped(mut body: Incoming, cap: usize) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    while let Some(f) = body.frame().await {
        let f = f.map_err(down)?;
        if let Ok(d) = f.into_data() {
            let room = cap.saturating_sub(out.len());
            out.extend_from_slice(&d[..d.len().min(room)]);
            if out.len() >= cap {
                break;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::encode_query;

    #[test]
    fn queries_encode_like_go() {
        assert_eq!(
            encode_query(&[("session", "a b/c"), ("mod", "0.4.1")]),
            "mod=0.4.1&session=a+b%2Fc"
        );
    }
}
