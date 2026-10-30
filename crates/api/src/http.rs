//! Request and response plumbing with Go `net/http` and `net/url`
//! semantics: the decoded path, `URL.Query()`, `writeJSON`, `writeError`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use hyper::HeaderMap;
use serde::Serialize;

/// One request as the handlers read it.
pub(crate) struct Req {
    pub method: String,
    /// Go `r.URL.Path`: percent-decoded.
    pub path: String,
    pub query: Query,
    pub headers: HeaderMap,
    /// Go `r.Host`.
    pub host: String,
    /// The body, read up to the cap.
    pub body: Vec<u8>,
    /// The body was over the cap (Go's `MaxBytesReader` error).
    pub body_too_large: bool,
}

impl Req {
    /// Go `r.Header.Get`: the first value, `""` when absent.
    pub fn header(&self, name: &str) -> String {
        self.headers
            .get(name)
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .unwrap_or_default()
    }

    /// Go `r.Header.Values`.
    pub fn header_values(&self, name: &str) -> Vec<String> {
        self.headers
            .get_all(name)
            .iter()
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .collect()
    }

    pub fn has_header(&self, name: &str) -> bool {
        self.headers.contains_key(name)
    }
}

/// Go `url.Values` from `url.ParseQuery`, in order.
#[derive(Debug, Clone, Default)]
pub(crate) struct Query(Vec<(String, String)>);

impl Query {
    /// Go `parseQuery`: pairs split on `&`; a pair with `;` or a bad escape
    /// is skipped; `+` is a space.
    pub fn parse(raw: &str) -> Query {
        let mut out = Vec::new();
        for part in raw.split('&') {
            if part.is_empty() || part.contains(';') {
                continue;
            }
            let (k, v) = part.split_once('=').unwrap_or((part, ""));
            let (Some(k), Some(v)) = (unescape(k, true), unescape(v, true)) else {
                continue;
            };
            out.push((k, v));
        }
        Query(out)
    }

    /// Go `Values.Get`: the first value, `""` when absent.
    pub fn get(&self, name: &str) -> String {
        self.0
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }

    /// Go `q[name]`.
    pub fn all(&self, name: &str) -> Vec<String> {
        self.0
            .iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .collect()
    }

    pub fn has(&self, name: &str) -> bool {
        self.0.iter().any(|(k, _)| k == name)
    }
}

fn unhex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Go `url.QueryUnescape` (`plus`) or `url.PathUnescape`; `None` on a bad
/// `%` escape. Invalid UTF-8 after decoding becomes U+FFFD, as Go's string
/// conversion keeps the bytes and every later use of them reads them so.
pub(crate) fn unescape(s: &str, plus: bool) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let hi = unhex(*b.get(i + 1)?)?;
                let lo = unhex(*b.get(i + 2)?)?;
                out.push(hi << 4 | lo);
                i += 3;
            }
            b'+' if plus => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// What a handler answers.
pub(crate) enum Resp {
    Full {
        status: u16,
        headers: Vec<(&'static str, String)>,
        body: Vec<u8>,
    },
    /// 200 with NDJSON headers; `run` writes the body on its own thread.
    Stream { run: Box<dyn FnOnce(Sink) + Send> },
}

impl Resp {
    /// Adds a header to a full response.
    pub fn with_header(mut self, name: &'static str, value: &str) -> Resp {
        if let Resp::Full { headers, .. } = &mut self {
            headers.push((name, value.to_string()));
        }
        self
    }
}

/// Go `writeJSON`: `json.NewEncoder(w).Encode(v)` (trailing `\n`, `<>&`
/// escaped) with the no-store and nosniff headers.
pub(crate) fn json<T: Serialize + ?Sized>(status: u16, v: &T) -> Resp {
    match gocompat::json::encode_line(v, gocompat::json::Options::default()) {
        Ok(body) => Resp::Full {
            status,
            headers: json_headers(),
            body,
        },
        Err(e) => Resp::Full {
            status: 500,
            headers: json_headers(),
            body: gocompat::json::encode_line(
                &ErrorBody {
                    error: e.to_string(),
                },
                gocompat::json::Options::default(),
            )
            .unwrap_or_default(),
        },
    }
}

fn json_headers() -> Vec<(&'static str, String)> {
    vec![
        ("Content-Type", "application/json".into()),
        ("Cache-Control", "no-store".into()),
        ("X-Content-Type-Options", "nosniff".into()),
    ]
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

/// Go `writeError`: `{"error": msg}`.
pub(crate) fn error(status: u16, msg: &str) -> Resp {
    json(
        status,
        &ErrorBody {
            error: msg.to_string(),
        },
    )
}

/// The writing end of a stream body. Writes block while the client is slow
/// and fail once it has gone or the server stops.
pub(crate) struct Sink {
    pub(crate) tx: tokio::sync::mpsc::Sender<Bytes>,
    pub(crate) stop: Arc<AtomicBool>,
}

impl Sink {
    /// Writes and flushes one chunk.
    pub fn write(&self, b: Vec<u8>) -> Result<(), ()> {
        if self.done() {
            return Err(());
        }
        self.tx.blocking_send(Bytes::from(b)).map_err(|_| ())
    }

    /// One JSON value as a line (Go `json.Marshal` + `\n`).
    pub fn line<T: Serialize + ?Sized>(&self, v: &T) -> Result<(), ()> {
        let mut raw = gocompat::json::marshal(v).map_err(|_| ())?;
        raw.push(b'\n');
        self.write(raw)
    }

    /// A bare `\n`.
    pub fn keepalive(&self) -> Result<(), ()> {
        self.write(b"\n".to_vec())
    }

    /// The client went away or the server is stopping.
    pub fn done(&self) -> bool {
        self.tx.is_closed() || self.stop.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_like_go() {
        let q = Query::parse("a=1&b=x+y&a=2&c=%41%zz&d=%41&e;f=1&=v&g");
        assert_eq!(q.get("a"), "1");
        assert_eq!(q.all("a"), vec!["1", "2"]);
        assert_eq!(q.get("b"), "x y");
        assert!(!q.has("c"), "bad escape skips the pair");
        assert_eq!(q.get("d"), "A");
        assert!(!q.has("e") && !q.has("e;f"));
        assert!(q.has("g") && q.get("g").is_empty());
        assert_eq!(unescape("a%2Fb+c", false).unwrap(), "a/b+c");
        assert!(unescape("%4", false).is_none());
        assert!(unescape("%", false).is_none());
    }
}
