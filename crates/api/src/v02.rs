//! v0.2 routes (I:internal/api/routes_v02.go, v02.go; surface.md §2.5):
//! attach, members, deliveries, tools, policy, inbox.

use serde::Serialize;

use crate::attach::{HEADER_ATTACH, HEADER_MEMBER, HEADER_ORIGIN};
use crate::backend::{Caller as ToolCaller, ToolError};
use crate::body;
use crate::http::{self, Resp};
use crate::server::Handler;

/// Caps `GET /v1/inbox?limit=`.
pub const MAX_INBOX_LIMIT: usize = 500;
/// Caps a member's display name (the inbox alias limit).
pub const MAX_MEMBER_NAME: usize = inbox::MAX_ALIAS_LEN;

/// Whether `path` accepts POST.
pub(crate) fn post_route(path: &str) -> bool {
    matches!(
        path,
        "/v1/members"
            | "/v1/policy"
            | "/v1/deliveries/delivered"
            | "/v1/deliveries/engaged"
            | "/v1/deliveries/turn-ended"
    ) || path.starts_with("/v1/tools/")
}

/// The routes here; `None` when the path is not one of them.
pub(crate) fn route(h: &Handler) -> Option<Resp> {
    let p = h.r.path.as_str();
    let ours = matches!(
        p,
        "/v1/attach" | "/v1/members" | "/v1/policy" | "/v1/inbox" | "/v1/tools"
    ) || p.starts_with("/v1/tools/")
        || p.starts_with("/v1/deliveries/");
    if !ours {
        return None;
    }
    let Some(ib) = h.s.b.inbox() else {
        return Some(http::error(503, "messaging is not running"));
    };
    let get = h.r.method == "GET";
    Some(match () {
        _ if p == "/v1/attach" && get => crate::stream::attach(h),
        _ if p == "/v1/members" && !get => join(h),
        _ if p == "/v1/policy" && get => http::json(200, &ib.policy()),
        _ if p == "/v1/policy" => set_policy(h, &ib),
        _ if p == "/v1/inbox" && get => inbox_list(h, &ib),
        _ if p == "/v1/tools" && get => tool_specs(h),
        _ if p.starts_with("/v1/tools/") && !get => tool(h, &p["/v1/tools/".len()..]),
        _ if p.starts_with("/v1/deliveries/") && !get => {
            control(h, &ib, &p["/v1/deliveries/".len()..])
        }
        _ if post_route(p) == get => {
            // A GET on a POST-only route or the reverse.
            let allow = if post_route(p) { "POST" } else { "GET" };
            http::error(405, "method not allowed").with_header("Allow", allow)
        }
        _ => http::error(404, "no such route"),
    })
}

pub(crate) fn forbidden() -> Resp {
    http::error(
        403,
        &format!("this route needs a current {HEADER_ATTACH} or {HEADER_MEMBER} secret"),
    )
}

/// Inbox errors as statuses (Go `inboxStatus`).
pub(crate) fn inbox_status(e: &inbox::Error) -> u16 {
    use inbox::ErrorKind as K;
    match e.kind() {
        K::NotFound | K::UnknownAgent => 404,
        K::NotYours => 403,
        K::BadState | K::Ambiguous => 409,
        K::RateLimited => 429,
        K::TooLong => 413,
        _ => 500,
    }
}

#[derive(Serialize)]
struct OkBody {
    ok: bool,
}

/// `POST /v1/deliveries/{delivered,engaged,turn-ended}`.
fn control(h: &Handler, ib: &inbox::Inbox, what: &str) -> Resp {
    if !matches!(what, "delivered" | "engaged" | "turn-ended") {
        return http::error(404, "no such route");
    }
    let c = match h.caller() {
        Some(c) if c.attach => c,
        _ => return forbidden(),
    };
    let d = match body::read(&h.r.body, h.r.body_too_large, &body::CONTROL) {
        Ok(d) => d,
        Err(e) => return http::error(400, &e),
    };
    let (token, turn_id, appended) = (d.str(0), d.str(1), d.bool(2));
    if token.trim_matches(gocompat::text::is_space).is_empty() {
        return http::error(400, "token is required");
    }
    let agent = c.sender.key.as_str();
    let res = match (what, appended) {
        ("delivered", true) => ib.mark_appended_delivered(agent, &token),
        ("delivered", false) => ib.mark_delivered(agent, &token),
        ("engaged", true) => ib.mark_appended_engaged(agent, &token, &turn_id),
        ("engaged", false) => ib.mark_engaged(agent, &token, &turn_id),
        _ => ib.turn_ended(agent, &token, &turn_id),
    };
    match res {
        Ok(()) => http::json(200, &OkBody { ok: true }),
        Err(e) => http::error(inbox_status(&e), &e.to_string()),
    }
}

fn tool_specs(h: &Handler) -> Resp {
    let client = h.r.query.get("client");
    let client = match client.as_str() {
        "" => "claude",
        "claude" | "grok" => client.as_str(),
        _ => return http::error(400, "client must be claude or grok"),
    };
    let specs = h.s.tools().map(|t| t.specs(client)).unwrap_or_default();
    http::json(200, &specs)
}

#[derive(Serialize)]
struct ToolResult {
    text: String,
}

fn tool(h: &Handler, name: &str) -> Resp {
    let Some(c) = h.caller() else {
        return forbidden();
    };
    let Some(ts) = h.s.tools() else {
        return http::error(503, "tools are not running");
    };
    let d = match body::read(&h.r.body, h.r.body_too_large, &body::TOOL) {
        Ok(d) => d,
        Err(e) => return http::error(400, &e),
    };
    let args = match d.raw(0) {
        Some(a) if !a.is_empty() && a != b"null" => a,
        _ => b"{}".to_vec(),
    };
    let caller = ToolCaller {
        sender: c.sender,
        path: c.path,
    };
    match ts.call(&caller, name, &args) {
        Ok(text) => http::json(200, &ToolResult { text }),
        Err(ToolError::UnknownTool) => http::error(
            404,
            &format!("no tool called {}", gocompat::quote::quote(name)),
        ),
        Err(ToolError::Other(e)) => http::error(400, &e),
    }
}

fn set_policy(h: &Handler, ib: &inbox::Inbox) -> Resp {
    let c = match h.caller() {
        Some(c) if c.attach && h.r.header(HEADER_ORIGIN) == "composer" => c,
        _ => {
            return http::error(
                403,
                &format!(
                    "policy changes need an attached mod and a person at the composer ({HEADER_ORIGIN}: composer)"
                ),
            );
        }
    };
    let d = match body::read(&h.r.body, h.r.body_too_large, &body::POLICY) {
        Ok(d) => d,
        Err(e) => return http::error(400, &e),
    };
    let p = inbox::Policy {
        mode: d.str(0),
        block: d.strings(1).unwrap_or_default(),
        allow: d.strings(2).unwrap_or_default(),
        paused: d.bool(3),
    };
    // Typed at the composer, but through the loopback API: only the GUI
    // mints "user" rows, so the actor still names the session.
    let by = format!("{}composer@{}", inbox::API_KEY_PREFIX, c.sender.key);
    if let Err(e) = ib.set_policy(p, &by) {
        return http::error(400, &e.to_string());
    }
    http::json(200, &ib.policy())
}

fn inbox_list(h: &Handler, ib: &inbox::Inbox) -> Resp {
    let Some(c) = h.caller() else {
        return forbidden();
    };
    let q = &h.r.query;
    let all = matches!(q.get("all").as_str(), "1" | "true");
    let mut limit = inbox::DEFAULT_INBOX_LIST;
    let v = q.get("limit");
    if !v.is_empty() {
        match go_atoi(&v) {
            Some(n) if n >= 1 => limit = (n as usize).min(MAX_INBOX_LIMIT),
            _ => return http::error(400, "limit must be a positive integer"),
        }
    }
    http::json(200, &ib.for_agent(&c.sender.key, all, limit))
}

#[derive(Serialize)]
struct MemberBody {
    #[serde(rename = "memberKey")]
    key: String,
    #[serde(rename = "memberSecret")]
    secret: String,
    name: String,
}

/// `POST /v1/members`.
fn join(h: &Handler) -> Resp {
    let d = match body::read(&h.r.body, h.r.body_too_large, &body::JOIN) {
        Ok(d) => d,
        Err(e) => return http::error(400, &e),
    };
    let (name, session) = (d.str(0), d.str(1));
    let name = name.trim_matches(gocompat::text::is_space);
    if name.is_empty()
        || name.chars().count() > MAX_MEMBER_NAME
        || name.chars().any(gocompat::text::is_control)
    {
        return http::error(
            400,
            &format!("name is required: 1-{MAX_MEMBER_NAME} characters, no control characters"),
        );
    }
    if session.len() > 200 || session.chars().any(gocompat::text::is_control) {
        return http::error(400, "session is too long or has control characters");
    }
    match h
        .s
        .b
        .join_member(name, session.trim_matches(gocompat::text::is_space))
    {
        Ok(m) => http::json(
            200,
            &MemberBody {
                key: m.key,
                secret: m.secret,
                name: m.name,
            },
        ),
        Err(e) => http::error(500, &e),
    }
}

/// Go `strconv.Atoi`: an optional sign and decimal digits, in range.
pub(crate) fn go_atoi(s: &str) -> Option<i64> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<i64>().ok()
}
