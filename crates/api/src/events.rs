//! `GET /v1/events` and `GET /v1/events/stream` (I:internal/api/server.go
//! events, routes_events.go; surface.md §2.7). Rows read whole only with
//! this run's person secret; otherwise as the events tool shows them.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::Serialize;
use store::{Event, Filter, Vector};

use crate::http::{self, Resp, Sink};
use crate::server::{DEFAULT_EVENT_LIMIT, Handler, MAX_EVENT_LIMIT, POLL};
use crate::v02::go_atoi;

/// `ev` as the reader may see it (Go `viewRow`).
fn view_row(mut ev: Event, person: bool) -> Event {
    if !person {
        ev.payload = crate::redact::redact(&ev.kind, &ev.payload);
    }
    ev
}

/// `GET /v1/events?kind=&limit=&device=`: newest first.
pub(crate) fn events(h: &Handler) -> Resp {
    let person = match h.rows_view() {
        Ok(p) => p,
        Err(r) => return r,
    };
    let q = &h.r.query;
    let mut f = Filter {
        limit: DEFAULT_EVENT_LIMIT,
        desc: true,
        ..Default::default()
    };
    f.kinds = q
        .all("kind")
        .into_iter()
        .filter(|k| !k.is_empty())
        .collect();
    f.device_id = q.get("device");
    let v = q.get("limit");
    if !v.is_empty() {
        match go_atoi(&v) {
            Some(n) if n >= 1 => f.limit = n.min(MAX_EVENT_LIMIT),
            _ => return http::error(400, "limit must be a positive integer"),
        }
    }
    match h.s.b.events(&f) {
        Ok(evs) => {
            let evs: Vec<Event> = evs.into_iter().map(|e| view_row(e, person)).collect();
            http::json(200, &evs)
        }
        Err(e) => http::error(500, &e),
    }
}

/// Go `parseKinds`: a comma list, empty entries dropped.
pub fn parse_kinds(s: &str) -> Vec<String> {
    s.split(',')
        .map(|k| k.trim_matches(gocompat::text::is_space))
        .filter(|k| !k.is_empty())
        .map(str::to_string)
        .collect()
}

/// Go `matchKind`: exact, or a prefix ending in `.`; no kinds passes all.
pub fn match_kind(kinds: &[String], kind: &str) -> bool {
    kinds.is_empty()
        || kinds
            .iter()
            .any(|k| k == kind || (k.ends_with('.') && kind.starts_with(k.as_str())))
}

const SINCE_FORM: &str = "since must be a non-negative integer or a vector of <deviceId>:<seq>";

/// Go `parseSince`: a bare seq is `self`'s alone (the v0.1 form), else a
/// vector of `<deviceId>:<seq>` entries.
pub fn parse_since(v: &str, self_id: &str) -> Result<Vector, String> {
    if let Some(n) = go_atoi(v) {
        if n < 0 {
            return Err(SINCE_FORM.into());
        }
        return Ok(BTreeMap::from([(self_id.to_string(), n)]));
    }
    let mut out = Vector::new();
    for part in v.split(',') {
        let part = part.trim_matches(gocompat::text::is_space);
        if part.is_empty() {
            continue;
        }
        let i = match part.rfind(':') {
            Some(i) if i > 0 => i,
            _ => {
                return Err(format!(
                    "since entry {} is not <deviceId>:<seq>",
                    gocompat::quote::quote(part)
                ));
            }
        };
        let n = match go_atoi(&part[i + 1..]) {
            Some(n) if n >= 0 => n,
            _ => {
                return Err(format!(
                    "since entry {} needs a non-negative seq",
                    gocompat::quote::quote(part)
                ));
            }
        };
        let dev = &part[..i];
        if out.contains_key(dev) {
            return Err(format!("since names {} twice", gocompat::quote::quote(dev)));
        }
        out.insert(dev.to_string(), n);
    }
    if out.is_empty() {
        return Err(SINCE_FORM.into());
    }
    Ok(out)
}

#[derive(Serialize)]
struct ResyncLine {
    resync: bool,
}

/// `GET /v1/events/stream?kinds=&since=`: replay what `since` names, then
/// live rows, with keepalives filling silence.
pub(crate) fn stream(h: &Handler) -> Resp {
    let q = &h.r.query;
    let kinds = parse_kinds(&q.get("kinds"));
    let person = match h.rows_view() {
        Ok(p) => p,
        Err(r) => return r,
    };
    let self_id = h.s.b.device().id;
    // None: replay nothing.
    let since = if q.has("since") {
        match parse_since(&q.all("since")[0], &self_id) {
            Ok(v) => Some(v),
            Err(e) => return http::error(400, &e),
        }
    } else {
        None
    };

    // Subscribe before the replay query, so a row appended in between is
    // in one or the other (or both, deduped below), never in neither.
    let live = h.s.b.subscribe_events();
    let mut replay: Vec<Event> = Vec::new();
    let mut last = Vector::new(); // per device: the newest seq replayed (or since)
    for (dev, seq) in since.iter().flatten() {
        last.insert(dev.clone(), *seq);
        loop {
            let f = Filter {
                kinds: kinds.clone(),
                device_id: dev.clone(),
                after_seq: last[dev],
                limit: MAX_EVENT_LIMIT,
                ..Default::default()
            };
            let evs = match h.s.b.events(&f) {
                Ok(e) => e,
                Err(e) => return http::error(500, &e),
            };
            let n = evs.len();
            if let Some(l) = evs.last() {
                last.insert(dev.clone(), l.seq);
            }
            replay.extend(evs);
            if (n as i64) < MAX_EVENT_LIMIT {
                break;
            }
        }
    }
    let keepalive = h.s.keepalive();
    Resp::Stream {
        run: Box::new(move |out: Sink| {
            for ev in replay {
                if out.line(&view_row(ev, person)).is_err() {
                    return;
                }
            }
            let mut tick = Instant::now() + keepalive;
            loop {
                let wait = tick
                    .saturating_duration_since(Instant::now())
                    .min(POLL)
                    .max(Duration::from_millis(1));
                let got = live.recv_timeout(wait);
                if out.done() {
                    return;
                }
                let ev = match got {
                    Err(()) => {
                        if Instant::now() >= tick {
                            if out.keepalive().is_err() {
                                return;
                            }
                            tick = Instant::now() + keepalive;
                        }
                        continue;
                    }
                    Ok(None) => return, // the store closed
                    Ok(Some(ev)) => ev,
                };
                let res = if ev.is_resync() {
                    out.line(&ResyncLine { resync: true })
                } else if last.get(&ev.device_id).is_some_and(|s| ev.seq <= *s) {
                    continue; // replayed already, or at or before since
                } else if !match_kind(&kinds, &ev.kind) {
                    continue;
                } else {
                    out.line(&view_row(ev, person))
                };
                if res.is_err() {
                    return;
                }
                tick = Instant::now() + keepalive; // keepalives only fill silence
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_since() {
        assert_eq!(parse_kinds(" a., ,b "), vec!["a.", "b"]);
        let k = parse_kinds("agent.,message.sent");
        assert!(match_kind(&k, "agent.seen") && match_kind(&k, "message.sent"));
        assert!(!match_kind(&k, "message.sentx") && !match_kind(&k, "agentx"));
        assert!(match_kind(&[], "x"));
        assert_eq!(
            parse_since("5", "me").unwrap(),
            Vector::from([("me".into(), 5)])
        );
        assert_eq!(parse_since("-1", "me").unwrap_err(), SINCE_FORM);
        assert_eq!(
            parse_since("a:1, b:c:2", "me").unwrap(),
            Vector::from([("a".into(), 1), ("b:c".into(), 2)])
        );
        assert_eq!(
            parse_since("a:1,a:2", "me").unwrap_err(),
            r#"since names "a" twice"#
        );
        assert_eq!(
            parse_since(":1", "me").unwrap_err(),
            r#"since entry ":1" is not <deviceId>:<seq>"#
        );
        assert_eq!(
            parse_since("a:-1", "me").unwrap_err(),
            r#"since entry "a:-1" needs a non-negative seq"#
        );
        assert_eq!(parse_since(" , ", "me").unwrap_err(), SINCE_FORM);
    }
}
