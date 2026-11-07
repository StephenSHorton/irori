//! A spawned agent's permission prompt (card #682): `POST
//! /v1/permission/wait` takes what Claude Code's `PermissionRequest` hook
//! said, as `irori permission-hook` passes it on, and holds the answer up
//! to [`irori_core::spawn_waiting::PERMISSION_POLL`] for the person's pick
//! on the card. The hook asks again with the same `id` while the answer
//! is `pending`.
//!
//! Body: `{"id", "sessionId", "toolName", "toolInput", "permissionMode",
//! "permissionSuggestions"}`. Answer: `{"decision": "pending" | "none" |
//! "allow" | "deny", "output"?}`, where `output` is the JSON the hook
//! prints for Claude Code. Only the bearer token's holder may call it, not
//! a member; it reports a prompt and waits, and never carries a decision
//! in: the person's pick is a board row.

use serde::{Deserialize, Serialize};

use crate::attach::HEADER_MEMBER;
use crate::http::{self, Resp};
use crate::server::Handler;

pub(crate) const PATH: &str = "/v1/permission/wait";

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Body {
    id: String,
    session_id: String,
    tool_name: String,
    tool_input: serde_json::Value,
    permission_mode: String,
    permission_suggestions: serde_json::Value,
}

#[derive(Serialize)]
struct Answer {
    decision: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<serde_json::Value>,
}

/// The route; `None` when the path is not it.
pub(crate) fn route(h: &Handler) -> Option<Resp> {
    if h.r.path != PATH {
        return None;
    }
    if h.r.method != "POST" {
        return Some(http::error(405, "method not allowed").with_header("Allow", "POST"));
    }
    if h.r.has_header(HEADER_MEMBER) {
        return Some(http::error(
            403,
            "only this device's agents report a permission prompt",
        ));
    }
    if h.r.body_too_large {
        return Some(http::error(400, "body: http: request body too large"));
    }
    let b: Body = match serde_json::from_slice(&h.r.body) {
        Ok(b) => b,
        Err(e) => return Some(http::error(400, &format!("body: {e}"))),
    };
    if b.id.trim().is_empty() || b.session_id.trim().is_empty() {
        return Some(http::error(400, "id and sessionId are required"));
    }
    let ask = irori_core::spawn_waiting::PermissionAsk {
        id: b.id.trim().into(),
        session_id: b.session_id.trim().into(),
        tool_name: b.tool_name,
        tool_input: b.tool_input,
        permission_mode: b.permission_mode,
        permission_suggestions: b.permission_suggestions,
    };
    let Some(d) = h.s.b.permission_wait(&ask) else {
        return Some(http::error(503, "this Core starts no agents"));
    };
    use irori_core::spawn_waiting::{PermissionDecision as D, hook_output};
    let decision = match &d {
        D::Pending => "pending",
        D::Nothing => "none",
        D::AllowOnce | D::AllowAlways { .. } => "allow",
        D::Deny => "deny",
    };
    Some(http::json(
        200,
        &Answer {
            decision,
            output: hook_output(&d),
        },
    ))
}
