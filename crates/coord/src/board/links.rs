//! Links out from a card (I:internal/coord/links.go, coord.md §7.10) and
//! the subset of Go `net/url.Parse` they need.

use std::sync::OnceLock;

use gocompat::quote::quote;
use regex::Regex;

use super::error::{Error, invalid};
use super::types::{LINK_COMMIT, LINK_PATH, LINK_URL, Link};
use crate::names::trim_space;

pub const MAX_LINKS: usize = 16;
const MAX_LINK_REF: usize = 1000;
const MAX_LABEL: usize = 60;

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("static regex"))
}

fn hex_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^[0-9a-fA-F]{7,40}$")
}
fn linear_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^/[^/]+/issue/([A-Za-z][A-Za-z0-9]*-[0-9]+)(?:/|$)")
}
fn github_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^/[^/]+/[^/]+/(?:issues|pull)/([0-9]+)(?:/|$)")
}
fn notion_id_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"-?[0-9a-f]{32}$")
}
fn scheme_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^[A-Za-z][A-Za-z0-9+-]+:")
}
fn pr_path_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // Go's \w is ASCII.
    re(
        &R,
        r"^/([0-9A-Za-z_.-]+)/([0-9A-Za-z_.-]+)/pull/([0-9]+)(?:/|$)",
    )
}

/// Classifies `s` as an http(s) URL, a commit sha or a repo path, and
/// labels it; anything else is Invalid.
pub fn parse_link(s: &str) -> Result<Link, Error> {
    let s = trim_space(s);
    if s.is_empty() || s.chars().count() > MAX_LINK_REF || s.contains(['\r', '\n', '\t']) {
        return Err(invalid(format!(
            "a link is a URL, a repo path or a commit sha (at most {MAX_LINK_REF} characters)"
        )));
    }
    if s.contains("://") || scheme_re().is_match(s) {
        let u = match url::parse(s) {
            Ok(u) if (u.scheme == "http" || u.scheme == "https") && !u.host.is_empty() => u,
            _ => return Err(invalid(format!("link {} is not an http(s) URL", quote(s)))),
        };
        if u.user {
            return Err(invalid(format!(
                "link {} has a user@ before its host; give the plain URL",
                quote(s)
            )));
        }
        return Ok(Link {
            kind: LINK_URL.into(),
            reference: s.into(),
            label: link_label(s),
        });
    }
    if hex_re().is_match(s) {
        let s = s.to_ascii_lowercase();
        return Ok(Link {
            kind: LINK_COMMIT.into(),
            label: link_label(&s),
            reference: s,
        });
    }
    let p = s.replace('\\', "/");
    if p.split('/').any(|seg| seg == "..") {
        return Err(invalid(format!("link path {} leaves the repo", quote(s))));
    }
    Ok(Link {
        kind: LINK_PATH.into(),
        label: link_label(&p),
        reference: p,
    })
}

/// The name a card shows for a link ref (at most 60 runes).
pub fn link_label(r: &str) -> String {
    let r = trim_space(r);
    if hex_re().is_match(r) {
        return format!("commit {}", r[..7].to_ascii_lowercase());
    }
    if !r.contains("://") {
        let p = r.replace('\\', "/");
        let p = p.trim_matches('/');
        let segs: Vec<&str> = p.split('/').collect();
        if segs.len() > 2 {
            return cap_label(&format!(".../{}", segs[segs.len() - 2..].join("/")));
        }
        return cap_label(p);
    }
    let u = match url::parse(r) {
        Ok(u) if !u.host.is_empty() => u,
        _ => return cap_label(r),
    };
    let host = gocompat::text::to_lower_str(&u.hostname());
    let host = host.strip_prefix("www.").unwrap_or(&host);
    if host == "linear.app" {
        if let Some(m) = linear_re().captures(&u.path) {
            return format!("Linear {}", gocompat::text::to_upper_str(&m[1]));
        }
        return "Linear".into();
    }
    if host == "github.com" {
        if let Some(m) = github_re().captures(&u.path) {
            return format!("GitHub #{}", &m[1]);
        }
        return "GitHub".into();
    }
    if host == "notion.so" || host == "notion.com" || host.ends_with(".notion.site") {
        let p = u.path.strip_suffix('/').unwrap_or(&u.path);
        let last = path_base(p);
        let stripped = notion_id_re().replace(&last, "");
        let title = stripped.replace('-', " ");
        let title = trim_space(&title);
        if title.is_empty() || title == "." || title == "/" {
            return "Notion".into();
        }
        return cap_label(&format!("Notion {title}"));
    }
    cap_label(host)
}

/// Go `path.Base`.
fn path_base(p: &str) -> String {
    if p.is_empty() {
        return ".".into();
    }
    let p = p.trim_end_matches('/');
    if p.is_empty() {
        return "/".into();
    }
    match p.rfind('/') {
        Some(i) => p[i + 1..].into(),
        None => p.into(),
    }
}

fn cap_label(s: &str) -> String {
    super::util::cut(s, MAX_LABEL)
}

/// A GitHub pull request out of a link: its `owner/name` and number.
pub fn pr_link(l: &Link) -> Option<(String, i64)> {
    if l.kind != LINK_URL {
        return None;
    }
    let u = url::parse(&l.reference).ok()?;
    let host = gocompat::text::to_lower_str(&u.hostname());
    if host.strip_prefix("www.").unwrap_or(&host) != "github.com" {
        return None;
    }
    let m = pr_path_re().captures(&u.path)?;
    let n: i64 = super::resource::go_atoi(&m[3])?;
    if n <= 0 {
        return None;
    }
    Some((format!("{}/{}", &m[1], &m[2]), n))
}

/// The subset of Go `net/url.Parse` links need: scheme, userinfo, host,
/// path, with Go's errors for control bytes, bad escapes, bad ports and
/// invalid host characters.
pub(crate) mod url {
    #[derive(Debug, Default)]
    pub struct Url {
        pub scheme: String,
        pub opaque: String,
        /// A userinfo (`user@`) was present.
        pub user: bool,
        pub host: String,
        pub path: String,
    }

    impl Url {
        /// `URL.Hostname()`.
        pub fn hostname(&self) -> String {
            let (host, _) = split_host_port(&self.host);
            host.to_string()
        }
    }

    #[derive(Debug)]
    pub struct ParseError;

    pub fn parse(raw: &str) -> Result<Url, ParseError> {
        let (u, frag) = match raw.split_once('#') {
            Some((u, f)) => (u, Some(f)),
            None => (raw, None),
        };
        let url = parse_inner(u)?;
        if let Some(f) = frag {
            unescape(f, Mode::Fragment)?;
        }
        Ok(url)
    }

    fn parse_inner(raw: &str) -> Result<Url, ParseError> {
        if raw.bytes().any(|b| b < 0x20 || b == 0x7f) {
            return Err(ParseError);
        }
        let mut url = Url::default();
        if raw == "*" {
            url.path = "*".into();
            return Ok(url);
        }
        let (scheme, rest) = get_scheme(raw)?;
        url.scheme = scheme.to_ascii_lowercase();
        let rest = if rest.ends_with('?') && rest.matches('?').count() == 1 {
            &rest[..rest.len() - 1]
        } else {
            rest.split_once('?').map(|(a, _)| a).unwrap_or(rest)
        };
        if !rest.starts_with('/') {
            if !url.scheme.is_empty() {
                url.opaque = rest.into();
                return Ok(url);
            }
            if let Some(colon) = rest.find(':')
                && rest.find('/').is_none_or(|slash| colon < slash)
            {
                return Err(ParseError);
            }
        }
        let mut rest = rest;
        if (!url.scheme.is_empty() || !rest.starts_with("///")) && rest.starts_with("//") {
            let after = &rest[2..];
            let (authority, r) = match after.find('/') {
                Some(i) => (&after[..i], &after[i..]),
                None => (after, ""),
            };
            parse_authority(authority, &mut url)?;
            rest = r;
        }
        url.path = unescape(rest, Mode::Path)?;
        Ok(url)
    }

    fn get_scheme(raw: &str) -> Result<(&str, &str), ParseError> {
        for (i, c) in raw.bytes().enumerate() {
            match c {
                b'a'..=b'z' | b'A'..=b'Z' => {}
                b'0'..=b'9' | b'+' | b'-' | b'.' => {
                    if i == 0 {
                        return Ok(("", raw));
                    }
                }
                b':' => {
                    if i == 0 {
                        return Err(ParseError);
                    }
                    return Ok((&raw[..i], &raw[i + 1..]));
                }
                _ => return Ok(("", raw)),
            }
        }
        Ok(("", raw))
    }

    fn parse_authority(authority: &str, url: &mut Url) -> Result<(), ParseError> {
        let host = match authority.rfind('@') {
            None => parse_host(authority)?,
            Some(i) => {
                let userinfo = &authority[..i];
                if !valid_userinfo(userinfo) {
                    return Err(ParseError);
                }
                match userinfo.split_once(':') {
                    None => {
                        unescape(userinfo, Mode::UserPassword)?;
                    }
                    Some((u, p)) => {
                        unescape(u, Mode::UserPassword)?;
                        unescape(p, Mode::UserPassword)?;
                    }
                }
                url.user = true;
                parse_host(&authority[i + 1..])?
            }
        };
        url.host = host;
        Ok(())
    }

    fn valid_userinfo(s: &str) -> bool {
        s.bytes().all(|r| {
            r.is_ascii_alphanumeric()
                || matches!(
                    r,
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
                || r >= 0x80
        })
    }

    fn parse_host(host: &str) -> Result<String, ParseError> {
        if host.starts_with('[') {
            let i = host.rfind(']').ok_or(ParseError)?;
            if !valid_optional_port(&host[i + 1..]) {
                return Err(ParseError);
            }
            // Zones (%25) are rare in links; the address part must unescape.
            unescape(&host[..=i], Mode::Host)?;
            return Ok(host.to_string());
        }
        if let Some(i) = host.rfind(':')
            && !valid_optional_port(&host[i..])
        {
            return Err(ParseError);
        }
        unescape(host, Mode::Host)
    }

    fn valid_optional_port(port: &str) -> bool {
        if port.is_empty() {
            return true;
        }
        let Some(digits) = port.strip_prefix(':') else {
            return false;
        };
        digits.bytes().all(|b| b.is_ascii_digit())
    }

    fn split_host_port(host_port: &str) -> (&str, &str) {
        let mut host = host_port;
        let mut port = "";
        if let Some(colon) = host.rfind(':')
            && valid_optional_port(&host[colon..])
        {
            port = &host[colon + 1..];
            host = &host[..colon];
        }
        if host.starts_with('[') && host.ends_with(']') {
            host = &host[1..host.len() - 1];
        }
        (host, port)
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mode {
        Path,
        Host,
        UserPassword,
        Fragment,
    }

    fn should_escape_host(c: u8) -> bool {
        if c.is_ascii_alphanumeric() {
            return false;
        }
        !matches!(
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
                | b'-'
                | b'_'
                | b'.'
                | b'~'
        )
    }

    fn unhex(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => 0,
        }
    }

    fn unescape(s: &str, mode: Mode) -> Result<String, ParseError> {
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'%' => {
                    if i + 2 >= b.len()
                        || !b[i + 1].is_ascii_hexdigit()
                        || !b[i + 2].is_ascii_hexdigit()
                    {
                        return Err(ParseError);
                    }
                    if mode == Mode::Host && unhex(b[i + 1]) < 8 && &s[i..i + 3] != "%25" {
                        return Err(ParseError);
                    }
                    i += 3;
                }
                c => {
                    if mode == Mode::Host && c < 0x80 && should_escape_host(c) {
                        return Err(ParseError);
                    }
                    i += 1;
                }
            }
        }
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' {
                out.push(unhex(b[i + 1]) << 4 | unhex(b[i + 2]));
                i += 3;
            } else {
                out.push(b[i]);
                i += 1;
            }
        }
        Ok(gocompat::utf8::to_valid_string(&out))
    }
}
