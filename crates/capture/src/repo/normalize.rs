//! I:internal/repo/normalize.go: a remote URL into `host/owner/name`.
//!
//! The URL form goes through a port of the parts of Go's `net/url.Parse`
//! the rules depend on (capture.md §7 trap 9): control bytes refused, the
//! scheme, `?` and `#` split off, authority with userinfo and port checks,
//! the path's `%` escapes checked, and `EscapedPath` (the raw path when it
//! is a valid encoding, else the decoded path escaped again).

use gocompat::text::{is_control, to_lower_str};

/// URL schemes that name a hosted repository.
const REMOTE_SCHEMES: &[&str] = &[
    "http",
    "https",
    "ssh",
    "git",
    "git+ssh",
    "ssh+git",
    "git+https",
];

/// Turns a remote URL into `host/owner/name`, or "" when it can't be
/// parsed (a local path, an unknown scheme, fewer than two segments, ...).
pub fn normalize_remote(raw: &str) -> String {
    let s = crate::jsonx::trim_space(raw);
    if s.is_empty() || is_local_path(s) {
        return String::new();
    }
    let (host, path);
    if s.contains("://") {
        let Some(u) = parse_url(s) else {
            return String::new();
        };
        if !REMOTE_SCHEMES.contains(&u.scheme.as_str()) {
            return String::new();
        }
        host = u.hostname;
        // The decoded path would let %2F pass as a separator: decode each
        // escaped segment and refuse one that holds a slash.
        let mut segs = Vec::new();
        for seg in u.escaped_path.split('/') {
            let Some(dec) = unescape(seg) else {
                return String::new();
            };
            if dec.contains('/') || dec.contains('\\') {
                return String::new();
            }
            segs.push(dec);
        }
        path = segs.join("/");
    } else {
        // scp-like: [user@]host:path. The colon must come before any slash.
        let Some(i) = s.find(':') else {
            return String::new();
        };
        if let Some(j) = s.find('/')
            && j < i
        {
            return String::new();
        }
        let mut h = &s[..i];
        path = s[i + 1..].to_string();
        if let Some(at) = h.rfind('@') {
            h = &h[at + 1..];
        }
        // A one-letter "host" is a Windows drive (C:foo).
        if h.len() <= 1 {
            return String::new();
        }
        host = h.to_string();
    }
    let lower = to_lower_str(&host);
    let host = lower.strip_suffix('.').unwrap_or(&lower);
    if host.is_empty() || host.contains(['/', '\\', ' ']) {
        return String::new();
    }
    let Some(segs) = split_path(&to_lower_str(&path)) else {
        return String::new();
    };
    let (host, segs) = fold_azure(host.to_string(), segs);
    if segs.len() < 2 {
        return String::new();
    }
    format!("{host}/{}", segs.join("/"))
}

/// Drops empty segments and a trailing `.git`; `None` for `.`/`..`
/// segments or control characters.
fn split_path(p: &str) -> Option<Vec<String>> {
    let mut segs: Vec<String> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" => continue,
            "." | ".." => return None,
            _ => {}
        }
        if seg.chars().any(is_control) {
            return None;
        }
        segs.push(seg.to_string());
    }
    if let Some(last) = segs.last_mut() {
        let t = last.strip_suffix(".git").unwrap_or(last).to_string();
        if t.is_empty() {
            segs.pop();
        } else {
            *last = t;
        }
    }
    if segs.is_empty() { None } else { Some(segs) }
}

/// Maps every Azure DevOps spelling onto dev.azure.com/<org>/<project>/_git/<repo>.
fn fold_azure(host: String, segs: Vec<String>) -> (String, Vec<String>) {
    let (host, mut segs) = fold_azure_host(host, segs);
    let n = segs.len();
    if host == "dev.azure.com" && n >= 2 && segs[n - 2] == "_ssh" {
        segs[n - 2] = "_git".into();
    }
    (host, segs)
}

fn fold_azure_host(host: String, segs: Vec<String>) -> (String, Vec<String>) {
    if host == "ssh.dev.azure.com" || host == "vs-ssh.visualstudio.com" {
        if segs.len() == 4 && segs[0] == "v3" {
            return (
                "dev.azure.com".into(),
                vec![
                    segs[1].clone(),
                    segs[2].clone(),
                    "_git".into(),
                    segs[3].clone(),
                ],
            );
        }
    } else if let Some(org) = host.strip_suffix(".visualstudio.com") {
        let mut rest = segs;
        if rest.first().is_some_and(|s| s == "defaultcollection") {
            rest.remove(0);
        }
        let mut out = vec![org.to_string()];
        out.extend(rest);
        return ("dev.azure.com".into(), out);
    }
    (host, segs)
}

/// Paths git accepts as a local remote.
fn is_local_path(s: &str) -> bool {
    if to_lower_str(s).starts_with("file:") {
        return true;
    }
    if s.starts_with('/') || s.starts_with('\\') || s.starts_with('.') || s.starts_with('~') {
        return true;
    }
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        return b.len() == 2 || b[2] == b'\\' || b[2] == b'/';
    }
    false
}

// --- the net/url subset ---

struct Url {
    scheme: String,
    hostname: String,
    escaped_path: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Path,
    Host,
}

fn should_escape(c: u8, mode: Mode) -> bool {
    if c.is_ascii_alphanumeric() {
        return false;
    }
    if mode == Mode::Host
        && matches!(
            c,
            b'!' | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b':'
                | b'['
                | b']'
                | b'<'
                | b'>'
                | b'"'
        )
    {
        return false;
    }
    match c {
        b'-' | b'_' | b'.' | b'~' => false,
        b'$' | b'&' | b'+' | b',' | b'/' | b':' | b';' | b'=' | b'?' | b'@' => {
            mode == Mode::Path && c == b'?' || mode != Mode::Path
        }
        _ => true,
    }
}

fn unhex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

/// Go `unescape(s, encodePathSegment/encodePath)`: `%XX` decoded, a bad
/// escape is an error. Invalid UTF-8 after decoding is kept as Go keeps it
/// (lossily here; such a segment is never a valid repo name anyway).
fn unescape(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hi = unhex(*b.get(i + 1)?)?;
            let lo = unhex(*b.get(i + 2)?)?;
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// Go `escape(s, encodePath)`.
fn escape_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &c in s.as_bytes() {
        if should_escape(c, Mode::Path) {
            out.push_str(&format!("%{c:02X}"));
        } else {
            out.push(c as char);
        }
    }
    out
}

/// Go `validEncoded(s, encodePath)`.
fn valid_encoded(s: &str) -> bool {
    s.bytes().all(|c| {
        matches!(
            c,
            b'!' | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b':'
                | b'@'
                | b'['
                | b']'
                | b'%'
        ) || !should_escape(c, Mode::Path)
    })
}

fn valid_optional_port(port: &str) -> bool {
    if port.is_empty() {
        return true;
    }
    let Some(digits) = port.strip_prefix(':') else {
        return false;
    };
    digits.bytes().all(|c| c.is_ascii_digit())
}

fn valid_userinfo(s: &str) -> bool {
    s.bytes().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(
                c,
                b'-' | b'.'
                    | b'_'
                    | b':'
                    | b'~'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
                    | b'%'
                    | b'@'
            )
    })
}

/// Go `parseHost`, then `unescape(host, encodeHost)` (a `%` in a host is
/// refused here; Go allows a few zone and high-byte forms).
fn parse_host(host: &str) -> Option<String> {
    if host.starts_with('[') {
        let i = host.rfind(']')?;
        if !valid_optional_port(&host[i + 1..]) {
            return None;
        }
    } else if let Some(i) = host.rfind(':')
        && !valid_optional_port(&host[i..])
    {
        return None;
    }
    for &c in host.as_bytes() {
        if c == b'%' || (c < 0x80 && should_escape(c, Mode::Host)) {
            return None;
        }
    }
    Some(host.to_string())
}

/// Go `URL.Hostname()`.
fn hostname(host_port: &str) -> String {
    let mut host = host_port;
    if let Some(colon) = host_port.rfind(':')
        && valid_optional_port(&host_port[colon..])
    {
        host = &host_port[..colon];
    }
    if host.starts_with('[') && host.ends_with(']') && host.len() >= 2 {
        host = &host[1..host.len() - 1];
    }
    host.to_string()
}

fn parse_url(raw: &str) -> Option<Url> {
    let (u, frag) = match raw.find('#') {
        Some(i) => (&raw[..i], &raw[i + 1..]),
        None => (raw, ""),
    };
    if u.bytes().any(|c| c < 0x20 || c == 0x7f) {
        return None;
    }
    // getScheme
    let mut scheme = String::new();
    let mut rest = u;
    for (i, c) in u.bytes().enumerate() {
        if c.is_ascii_alphabetic() {
            continue;
        }
        if c.is_ascii_digit() || c == b'+' || c == b'-' || c == b'.' {
            if i == 0 {
                break;
            }
            continue;
        }
        if c == b':' {
            if i == 0 {
                return None; // missing protocol scheme
            }
            scheme = to_lower_str(&u[..i]);
            rest = &u[i + 1..];
        }
        break;
    }
    let rest = if rest.ends_with('?') && rest.matches('?').count() == 1 {
        &rest[..rest.len() - 1]
    } else {
        match rest.find('?') {
            Some(i) => &rest[..i],
            None => rest,
        }
    };
    let mut host_port = String::new();
    let mut path = rest;
    if !rest.starts_with('/') {
        if !scheme.is_empty() {
            // Opaque: no host.
            unescape(frag)?;
            return Some(Url {
                scheme,
                hostname: String::new(),
                escaped_path: String::new(),
            });
        }
    } else if rest.starts_with("//") && !scheme.is_empty() {
        let after = &rest[2..];
        let (authority, p) = match after.find('/') {
            Some(i) => (&after[..i], &after[i..]),
            None => (after, ""),
        };
        path = p;
        let h = match authority.rfind('@') {
            Some(i) => {
                let userinfo = &authority[..i];
                if !valid_userinfo(userinfo) {
                    return None;
                }
                unescape(userinfo)?;
                &authority[i + 1..]
            }
            None => authority,
        };
        host_port = parse_host(h)?;
    }
    let decoded = unescape(path)?;
    unescape(frag)?;
    let escaped_path = if valid_encoded(path) && unescape(path).as_deref() == Some(&decoded) {
        path.to_string()
    } else {
        escape_path(&decoded)
    };
    Some(Url {
        scheme,
        hostname: hostname(&host_port),
        escaped_path,
    })
}
