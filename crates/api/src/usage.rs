//! Claude usage left (card #599): `POST /v1/usage/limits` takes the plan's
//! rate-limit windows a Claude Code session measured, from the Irori mod's
//! `session.measure` hook. Only an attached mod may post (its current
//! `X-Irori-Attach` secret, as the deliveries routes): a member secret, no
//! secret or a dead one is 403.
//!
//! Body: `{"rateLimits": [{"kind", "percentUsed", "resetsAt"?}]}`, Claude
//! Code's `SessionRateLimit`s as they come. The Core checks, merges and
//! keeps them ([`irori_core::usage`]); a refused reading is 400 with why.

use serde::{Deserialize, Serialize};

use crate::http::{self, Resp};
use crate::server::Handler;

pub(crate) const PATH: &str = "/v1/usage/limits";

/// Whether `path` is the usage route (POST only).
pub(crate) fn post_route(path: &str) -> bool {
    path == PATH
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Body {
    #[serde(default)]
    rate_limits: Vec<irori_core::usage::RateLimit>,
}

#[derive(Serialize)]
struct OkBody {
    ok: bool,
}

/// The route; `None` when the path is not it.
pub(crate) fn route(h: &Handler) -> Option<Resp> {
    if !post_route(&h.r.path) {
        return None;
    }
    if h.r.method != "POST" {
        return Some(http::error(405, "method not allowed").with_header("Allow", "POST"));
    }
    if !matches!(h.caller(), Some(c) if c.attach) {
        return Some(crate::v02::forbidden(h));
    }
    if h.r.body_too_large {
        return Some(http::error(400, "body: http: request body too large"));
    }
    let body: Body = match serde_json::from_slice(&h.r.body) {
        Ok(b) => b,
        Err(e) => return Some(http::error(400, &format!("body: {e}"))),
    };
    Some(match h.s.b.record_usage_limits(&body.rate_limits) {
        None => http::error(503, "this Core keeps no usage limits"),
        Some(Ok(())) => http::json(200, &OkBody { ok: true }),
        Some(Err(e)) => http::error(400, &e),
    })
}
