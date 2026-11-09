//! Script slots (card #437): `POST /v1/slots` takes one of N slot leases on
//! a name for a run (a script, not an agent),
//! and `POST /v1/slots/release` gives it back. The token is enough: the
//! holder is this device's run holder `<device>/run-<run>`, which the gone
//! sweep leaves alone, so a run renews its lease while it lives (a short
//! `ttl`) and a run that dies frees its slot when the lease ends.
//!
//! Arguments are query parameters: `resource` (a plain name such as
//! `check/mac`), `run` (`[a-z0-9._-]`, at most 64), and for a take `slots`
//! (1 to [`coord::MAX_SLOTS`]) and `ttl` (seconds, 10 to
//! [`coord::MAX_TTL`]). A take answers 200 with the lease, or 409 with
//! `held` (the other runs' names) while every slot is taken.

use std::time::Duration;

use serde::Serialize;

use crate::http::{self, Resp};
use crate::server::Handler;

const MAX_RUN: usize = 64;
const MIN_TTL: u64 = 10;

/// Whether `path` is a slots route (POST only).
pub(crate) fn post_route(path: &str) -> bool {
    matches!(path, "/v1/slots" | "/v1/slots/release")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Taken {
    resource: String,
    holder: String,
    until: String,
}

#[derive(Serialize)]
struct Held {
    error: String,
    held: Vec<String>,
}

#[derive(Serialize)]
struct Released {
    released: bool,
}

/// The routes here; `None` when the path is not one of them.
pub(crate) fn route(h: &Handler) -> Option<Resp> {
    let p = h.r.path.as_str();
    if !post_route(p) {
        return None;
    }
    if h.r.method != "POST" {
        return Some(http::error(405, "method not allowed").with_header("Allow", "POST"));
    }
    let Some(c) = h.s.b.coord() else {
        return Some(http::error(503, "the board is not running"));
    };
    let q = &h.r.query;
    let run = q.get("run");
    if run.is_empty()
        || run.len() > MAX_RUN
        || !run
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
    {
        return Some(http::error(
            400,
            &format!("run is required: 1-{MAX_RUN} of a-z 0-9 . _ -"),
        ));
    }
    let holder = format!("{}/{}{run}", h.s.b.device().id, coord::RUN_HOLDER);
    let resource = q.get("resource");
    if p == "/v1/slots/release" {
        return Some(match c.release_slot(&holder, &resource) {
            Ok(released) => http::json(200, &Released { released }),
            Err(e) => http::error(400, &e.to_string()),
        });
    }
    let slots = match q.get("slots").parse::<usize>() {
        Ok(n) if (1..=coord::MAX_SLOTS).contains(&n) => n,
        _ => {
            return Some(http::error(
                400,
                &format!("slots must be 1 to {}", coord::MAX_SLOTS),
            ));
        }
    };
    let max = coord::MAX_TTL.as_secs();
    let ttl = match q.get("ttl").parse::<u64>() {
        Ok(n) if (MIN_TTL..=max).contains(&n) => Duration::from_secs(n),
        _ => {
            return Some(http::error(
                400,
                &format!("ttl must be {MIN_TTL} to {max} seconds"),
            ));
        }
    };
    Some(match c.acquire_slot(&holder, &resource, slots, ttl) {
        Ok(l) => http::json(
            200,
            &Taken {
                resource: l.resource,
                holder: l.holder,
                until: l.until.format_rfc3339(),
            },
        ),
        Err(e) if e.kind() == coord::ErrorKind::Held => {
            let base = format!("{}/slot-", resource.trim());
            let mut held: Vec<String> = c
                .leases()
                .into_iter()
                .filter(|l| l.resource.starts_with(&base) && l.holder != holder)
                .map(|l| run_name(&l.holder))
                .collect();
            held.dedup();
            http::json(
                409,
                &Held {
                    error: e.to_string(),
                    held,
                },
            )
        }
        Err(e) => http::error(400, &e.to_string()),
    })
}

/// A holder as a person reads it: a run holder's run, else the key.
fn run_name(key: &str) -> String {
    match key.split_once('/') {
        Some((_, rest)) if rest.starts_with(coord::RUN_HOLDER) => {
            rest[coord::RUN_HOLDER.len()..].to_string()
        }
        _ => key.to_string(),
    }
}
