//! v0.3 capture routes (I:internal/api/routes_v03.go, v03.go; surface.md
//! §2.5): GET-only reads, 503 without capture, limits capped, lists `[]`.

use gocompat::time::GoTime;

use crate::http::{self, Query, Resp};
use crate::server::Handler;
use crate::v02::go_atoi;

/// Limits for the v0.3 list routes. Larger limits are capped, not refused.
pub const MAX_TIMELINE_LIMIT: i64 = 100;
pub const MAX_CAPTURE_LIMIT: i64 = 500;

pub(crate) fn route(h: &Handler) -> Option<Resp> {
    let p = h.r.path.as_str();
    let ours = matches!(
        p,
        "/v1/timeline"
            | "/v1/edits"
            | "/v1/commits"
            | "/v1/decisions"
            | "/v1/prs"
            | "/v1/capture/status"
            | "/v1/why"
    ) || p.starts_with("/v1/commits/")
        || p.starts_with("/v1/blobs/");
    if !ours {
        return None;
    }
    let Some(c) = h.s.b.capture() else {
        return Some(http::error(503, "capture is not running"));
    };
    let q = &h.r.query;
    Some(match p {
        "/v1/timeline" => timeline(q, &c),
        "/v1/edits" => edits(q, &c),
        "/v1/commits" => commits(q, &c),
        "/v1/decisions" => decisions(q, &c),
        "/v1/prs" => http::json(
            200,
            &c.prs(q.get("repo").trim_matches(gocompat::text::is_space)),
        ),
        "/v1/capture/status" => http::json(200, &c.status()),
        "/v1/why" => why(q, &c),
        _ if p.starts_with("/v1/commits/") => commit(&c, &p["/v1/commits/".len()..]),
        _ => blob(&c, &p["/v1/blobs/".len()..]),
    })
}

fn trim(s: &str) -> String {
    s.trim_matches(gocompat::text::is_space).to_string()
}

/// `?limit=`: `def` when absent, capped at `hi`; `Err` is the 400.
pub(crate) fn limit_param(q: &Query, def: i64, hi: i64) -> Result<i64, Resp> {
    positive_param(q, "limit", def, hi)
}

/// [`limit_param`] for any parameter name.
pub(crate) fn positive_param(q: &Query, name: &str, def: i64, hi: i64) -> Result<i64, Resp> {
    let v = q.get(name);
    if v.is_empty() {
        return Ok(def);
    }
    match go_atoi(&v) {
        Some(n) if n >= 1 => Ok(n.min(hi)),
        _ => Err(http::error(
            400,
            &format!("{name} must be a positive integer"),
        )),
    }
}

pub(crate) fn bool_param(q: &Query, name: &str) -> bool {
    matches!(q.get(name).as_str(), "1" | "true")
}

/// Go `time.Parse(time.RFC3339Nano, v)`.
pub(crate) fn parse_time(v: &str) -> Option<GoTime> {
    GoTime::parse_rfc3339(v)
}

fn timeline(q: &Query, c: &capture::Capture) -> Resp {
    let mut tq = capture::TimelineQuery {
        key: trim(&q.get("agent")),
        subagents: bool_param(q, "subagents"),
        ..Default::default()
    };
    if tq.key.is_empty() {
        return http::error(400, "agent is required");
    }
    let v = q.get("before");
    if !v.is_empty() {
        match parse_time(&v) {
            Some(t) => tq.before = t,
            None => return http::error(400, "before must be an RFC 3339 time"),
        }
    }
    match limit_param(q, 20, MAX_TIMELINE_LIMIT) {
        Ok(n) => tq.limit = n as usize,
        Err(r) => return r,
    }
    http::json(200, &c.timeline(&tq))
}

fn edits(q: &Query, c: &capture::Capture) -> Resp {
    let mut eq = capture::EditQuery {
        repo_key: trim(&q.get("repo")),
        path: q.get("path"),
        key: trim(&q.get("agent")),
        ..Default::default()
    };
    match limit_param(q, 50, MAX_CAPTURE_LIMIT) {
        Ok(n) => eq.limit = n as usize,
        Err(r) => return r,
    }
    http::json(200, &c.edits(&eq))
}

fn commits(q: &Query, c: &capture::Capture) -> Resp {
    let limit = match limit_param(q, 50, MAX_CAPTURE_LIMIT) {
        Ok(n) => n as usize,
        Err(r) => return r,
    };
    http::json(200, &c.commits(&trim(&q.get("repo")), limit))
}

/// Whether `s` is `lo..=hi` hex digits, either case.
fn is_hex(s: &str, lo: usize, hi: usize) -> bool {
    (lo..=hi).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn commit(c: &capture::Capture, sha: &str) -> Resp {
    if !is_hex(sha, 7, 64) {
        return http::error(400, "sha must be 7 to 64 hex digits");
    }
    match c.commit(sha) {
        Some(v) => http::json(200, &v),
        None => http::error(
            404,
            &format!("no commit {sha} (or the prefix is ambiguous)"),
        ),
    }
}

/// blob's sentinel texts (I:internal/blob; crates/blob/src/error.rs).
const BLOB_BAD_ID: &str = "blob: id must be 64 lowercase hex characters";
const BLOB_NOT_FOUND: &str = "blob: not found";

fn blob(c: &capture::Capture, id: &str) -> Resp {
    match c.blob(id) {
        Ok(data) => Resp::Full {
            status: 200,
            headers: vec![
                ("Content-Type", "text/plain; charset=utf-8".into()),
                ("Cache-Control", "no-store".into()),
                ("X-Content-Type-Options", "nosniff".into()),
                ("Content-Length", data.len().to_string()),
            ],
            body: data,
        },
        Err(e) if e.starts_with(BLOB_BAD_ID) => http::error(400, &e),
        Err(e) if e.starts_with(BLOB_NOT_FOUND) => http::error(404, "no such blob"),
        Err(e) => http::error(500, &e),
    }
}

fn decisions(q: &Query, c: &capture::Capture) -> Resp {
    let mut f = coord::decision::Filter {
        repo_key: trim(&q.get("repo")),
        path: q.get("path").replace('\\', "/"),
        query: q.get("query"),
        all: bool_param(q, "all"),
        ..Default::default()
    };
    match limit_param(q, 50, MAX_CAPTURE_LIMIT) {
        Ok(n) => f.limit = n,
        Err(r) => return r,
    }
    http::json(200, &c.decisions(&f))
}

fn why(q: &Query, c: &capture::Capture) -> Resp {
    let mut wq = capture::WhyQuery {
        path: trim(&q.get("path")),
        repo_key: trim(&q.get("repo")),
        cwd: trim(&q.get("cwd")),
        ..Default::default()
    };
    if wq.path.is_empty() {
        return http::error(400, "path is required");
    }
    let v = q.get("line");
    if !v.is_empty() {
        match go_atoi(&v) {
            Some(n) if n >= 1 => wq.line = n,
            _ => return http::error(400, "line must be a positive integer"),
        }
    }
    match c.why(&wq) {
        Ok(res) => http::json(200, &res),
        Err(capture::Error::NoRepo) => http::error(404, &capture::Error::NoRepo.to_string()),
        Err(e) => http::error(500, &e.to_string()),
    }
}
