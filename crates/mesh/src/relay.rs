//! The advanced TURN relay setting, `relay.json` (mesh.md §2.5, §3.1;
//! I:internal/mesh/relay.go). Under iroh it has no meaning (D4): the
//! setting is kept readable and writable so Settings and the API work, and
//! the transport reports it as `none` with a note.

use std::path::Path;
use std::sync::LazyLock;

use gocompat::json::{self, Schema};
use regex::Regex;
use serde::Serialize;

/// The file name in the data dir; device-local.
pub const RELAY_FILE: &str = "relay.json";

/// Go `Relay`: a TURN REST credential endpoint and, optionally, TURN URLs
/// that replace the ones it returns. The zero value is none.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Relay {
    #[serde(rename = "credentialUrl", skip_serializing_if = "String::is_empty")]
    pub credential_url: String,
    #[serde(rename = "turnUrls", skip_serializing_if = "Vec::is_empty")]
    pub turn_urls: Vec<String>,
}

/// How the relay is doing: none, pending, ok or error.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RelayStatus {
    pub state: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

/// A transport whose relay can change while it runs (Go `RelaySetter`).
pub trait RelaySetter: Send + Sync {
    fn set_relay(&self, r: Relay);
    fn relay_status(&self) -> RelayStatus;
}

/// The scheme and host of a URL as Go's `net/url.Parse` reads them, enough
/// for [`Relay::check`].
fn scheme_and_host(raw: &str) -> Option<(String, String)> {
    let colon = raw.find(':')?;
    let scheme = &raw[..colon];
    let first = scheme.chars().next()?;
    if !first.is_ascii_alphabetic()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    let rest = raw[colon + 1..].strip_prefix("//")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let auth = &rest[..end];
    let host = auth.rsplit_once('@').map(|(_, h)| h).unwrap_or(auth);
    Some((scheme.to_ascii_lowercase(), host.to_string()))
}

/// Go `url.URL.Hostname`: the host without its port or IPv6 brackets.
fn hostname(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    match host.rfind(':') {
        Some(i) => &host[..i],
        None => host,
    }
}

impl Relay {
    /// No relay is set.
    pub fn direct(&self) -> bool {
        self.credential_url.is_empty()
    }

    /// Go `Check`: the credential URL must be https, except on loopback,
    /// and TURN URLs must be `turn:` or `turns:`.
    pub fn check(&self) -> Result<(), &'static str> {
        if self.direct() {
            if !self.turn_urls.is_empty() {
                return Err("relay: TURN URLs need a credential URL");
            }
            return Ok(());
        }
        let Some((scheme, host)) =
            scheme_and_host(&self.credential_url).filter(|(_, h)| !h.is_empty())
        else {
            return Err("relay: the credential URL is not a URL");
        };
        let h = hostname(&host);
        let lo = h == "127.0.0.1" || h == "::1" || h == "localhost";
        if scheme != "https" && !(scheme == "http" && lo) {
            return Err("relay: the credential URL must be https");
        }
        for t in &self.turn_urls {
            let l = t.trim().to_ascii_lowercase();
            if !l.starts_with("turn:") && !l.starts_with("turns:") {
                return Err("relay: TURN URLs start with turn: or turns:");
            }
        }
        Ok(())
    }
}

/// Go `LoadRelay`: none set is the zero value.
pub fn load_relay(data_dir: &Path) -> Result<Relay, String> {
    let raw = match std::fs::read(data_dir.join(RELAY_FILE)) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Relay::default()),
        Err(e) => return Err(e.to_string()),
    };
    let schema = Schema::object(vec![
        ("credentialUrl", Schema::String),
        ("turnUrls", Schema::Slice(Box::new(Schema::String))),
    ]);
    let o = json::unmarshal(&raw, &schema).map_err(|_| "relay: relay.json is unreadable")?;
    Ok(Relay {
        credential_url: o.get_str("credentialUrl").to_string(),
        turn_urls: o
            .get("turnUrls")
            .map(|v| v.items().iter().map(|s| s.str().to_string()).collect())
            .unwrap_or_default(),
    })
}

/// Go `SaveRelay`: checks and writes the setting; the zero value removes
/// the file.
pub fn save_relay(data_dir: &Path, r: &Relay) -> Result<(), String> {
    r.check().map_err(str::to_string)?;
    let path = data_dir.join(RELAY_FILE);
    if r.direct() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    }
    let out = json::marshal_indent(r, "", "  ").map_err(|e| e.to_string())?;
    crate::fsutil::write_replace(&path, &out).map_err(|e| e.to_string())
}

/// A URL's query string and fragment, where TURN REST providers put their
/// API key. Go's `\s` is ASCII whitespace.
static URL_QUERY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(https?://[^\t\n\f\r ?#"']*)[?#][^\t\n\f\r "']*"#).unwrap());

/// Go `ScrubURLs`: cuts the query off every URL in `s`, so a credential
/// URL's key never reaches a log, a status line or the window.
pub fn scrub_urls(s: &str) -> String {
    URL_QUERY.replace_all(s, "$1?…").into_owned()
}
