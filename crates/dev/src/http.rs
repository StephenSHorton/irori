//! A minimal HTTP/1.1 client for a Core's loopback API
//! (I:internal/api: `127.0.0.1:<port>`, `Authorization: Bearer <token>`).
//! The token is read from `irori.json` and goes only into the request; no
//! error or log line carries it.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

/// The two `irori.json` fields the client needs (I:internal/discovery).
/// No `Debug`, so the token cannot be formatted by accident.
pub struct Lock {
    pub port: u16,
    token: String,
}

impl Lock {
    pub fn read(path: &Path) -> Result<Lock, String> {
        let raw = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let v: serde_json::Value =
            serde_json::from_slice(&raw).map_err(|_| format!("{} is not JSON", path.display()))?;
        let port = v
            .get("port")
            .and_then(|p| p.as_u64())
            .and_then(|p| u16::try_from(p).ok())
            .ok_or_else(|| format!("{} has no port", path.display()))?;
        let token = v
            .get("token")
            .and_then(|t| t.as_str())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| format!("{} has no token", path.display()))?
            .to_string();
        Ok(Lock { port, token })
    }
}

/// GET `path` and return the status and body.
pub fn get(lock: &Lock, path: &str) -> Result<(u16, Vec<u8>), String> {
    request(lock, "GET", path, &[], None, false)
}

/// One request with the bearer token and `headers`; a `body` is sent with
/// its Content-Length. With `head_only` the response is read up to its
/// header end and the connection dropped (a stream route), and the body
/// comes back empty.
pub fn request(
    lock: &Lock,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
    head_only: bool,
) -> Result<(u16, Vec<u8>), String> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, lock.port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .map_err(|e| format!("connect 127.0.0.1:{}: {e}", lock.port))?;
    s.set_read_timeout(Some(Duration::from_secs(15))).ok();
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nUser-Agent: irori-rust-dev\r\nAccept: application/json\r\nConnection: close\r\n",
        lock.port, lock.token
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        req.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    req.push_str("\r\n");
    let mut out = req.into_bytes();
    if let Some(b) = body {
        out.extend_from_slice(b);
    }
    s.write_all(&out)
        .map_err(|e| format!("send request: {e}"))?;
    drop(out);
    let mut raw = Vec::new();
    if head_only {
        let mut buf = [0u8; 4096];
        while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = s
                .read(&mut buf)
                .map_err(|e| format!("read response: {e}"))?;
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&buf[..n]);
        }
    } else {
        s.read_to_end(&mut raw)
            .map_err(|e| format!("read response: {e}"))?;
    }
    let end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("response has no header end")?;
    let head = String::from_utf8_lossy(&raw[..end]).to_string();
    let body = &raw[end + 4..];
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or("response has no status")?;
    if head_only {
        return Ok((status, Vec::new()));
    }
    let chunked = head.lines().any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("transfer-encoding:") && l.contains("chunked")
    });
    let body = if chunked {
        dechunk(body)?
    } else {
        body.to_vec()
    };
    Ok((status, body))
}

fn dechunk(mut b: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let eol = b
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("bad chunk header")?;
        let size_txt = String::from_utf8_lossy(&b[..eol]);
        let size_txt = size_txt.split(';').next().unwrap_or("").trim();
        let n = usize::from_str_radix(size_txt, 16).map_err(|_| "bad chunk size")?;
        b = &b[eol + 2..];
        if n == 0 {
            return Ok(out);
        }
        if b.len() < n + 2 {
            return Err("short chunk".into());
        }
        out.extend_from_slice(&b[..n]);
        b = &b[n + 2..];
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn dechunks() {
        let got = super::dechunk(b"3\r\nabc\r\n2;x=y\r\nde\r\n0\r\n\r\n").unwrap();
        assert_eq!(got, b"abcde");
    }
}
