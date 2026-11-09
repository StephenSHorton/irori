//! Live tasks on cards (card #1249): `POST /v1/runs` takes the Irori mod's
//! reports of an agent's runs: a background shell, Agent or Workflow run
//! starting or ending, and a foreground shell call (design §2.1). Only an
//! attached mod may post (its current `X-Irori-Attach` secret, as the
//! usage route): a member secret, no secret or a dead one is 403. The run
//! is the attached agent's, by its key; the body never names the agent.
//!
//! Not a registered tool: the model never sees it in `/v1/tools`, and it
//! only records. Nothing on this route stops a run.
//!
//! Body: `{"event": "start|end|fg-start|fg", "kind", "taskId",
//! "toolUseId", "command", "outputFile", "startedAt", "endedAt",
//! "status", "exitCode", …}` ([`irori_core::runs::RunReport`]). A report
//! the Core can't take (bad JSON, an unknown event or kind, no id) is 400
//! with why; a repeat is 200 that changes nothing.

use serde::Serialize;

use crate::http::{self, Resp};
use crate::server::Handler;

pub(crate) const PATH: &str = "/v1/runs";

/// Whether `path` is the runs route (POST only).
pub(crate) fn post_route(path: &str) -> bool {
    path == PATH
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
    let caller = match h.caller() {
        Some(c) if c.attach => c,
        _ => return Some(crate::v02::forbidden(h)),
    };
    if h.r.body_too_large {
        return Some(http::error(400, "body: http: request body too large"));
    }
    let body: irori_core::runs::RunReport = match serde_json::from_slice(&h.r.body) {
        Ok(b) => b,
        Err(e) => return Some(http::error(400, &format!("body: {e}"))),
    };
    Some(match h.s.b.record_run(&caller.sender.key, &body) {
        None => http::error(503, "this Core keeps no runs"),
        Some(Ok(())) => http::json(200, &OkBody { ok: true }),
        Some(Err(e)) => http::error(400, &e),
    })
}
