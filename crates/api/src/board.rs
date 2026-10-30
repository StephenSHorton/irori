//! Board and browsing routes (I:internal/api/routes_board.go, board.go,
//! board_overview.go; surface.md §2.5): board, tasks, search, show,
//! overview. GET only; who is asking decides what they show
//! ([`Handler::viewer`]).

use std::collections::HashMap;

use serde::Serialize;

use crate::attach::{HEADER_ATTACH, HEADER_MEMBER};
use crate::http::{self, Query, Resp};
use crate::server::Handler;
use crate::v02::go_atoi;
use crate::v03::{limit_param, parse_time, positive_param};
use crate::{AGENT_VIEWER, COUNT_LIMIT};

/// The `GET /v1/tasks/{id}` body.
#[derive(Serialize)]
pub struct TaskDetail {
    pub task: coord::Task,
    pub comments: Vec<coord::Comment>,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// The `GET /v1/search` body.
#[derive(Serialize)]
pub struct SearchResult {
    pub hits: Vec<search::Hit>,
    /// Another page exists.
    pub more: bool,
    /// Its offset, when `more`.
    #[serde(skip_serializing_if = "is_zero")]
    pub next: i64,
}

pub(crate) fn route(h: &Handler) -> Option<Resp> {
    let p = h.r.path.as_str();
    let ours = matches!(p, "/v1/board" | "/v1/search" | "/v1/show" | "/v1/overview")
        || p.starts_with("/v1/tasks/");
    if !ours {
        return None;
    }
    let viewer = match h.viewer() {
        Ok(v) => v,
        Err(r) => return Some(r),
    };
    Some(match p {
        "/v1/board" => board_view(h, &board_for(h, &viewer)),
        "/v1/search" => search_route(h, &viewer),
        "/v1/show" => show(h, &viewer),
        "/v1/overview" => overview(h, &viewer),
        _ => task(h, &p["/v1/tasks/".len()..]),
    })
}

/// The Board's viewer key: the agent's, or for the person the window's own
/// (`user:<device>`), so tasks assigned to the person land in NeedsYou.
fn board_for(h: &Handler, viewer: &str) -> String {
    if !viewer.is_empty() {
        return viewer.to_string();
    }
    format!("user:{}", h.s.b.device().id)
}

/// A non-negative `?name=` (`def` when absent); `Err` is the 400.
fn int_param(q: &Query, name: &str, def: i64) -> Result<i64, Resp> {
    let v = q.get(name);
    if v.is_empty() {
        return Ok(def);
    }
    match go_atoi(&v) {
        Some(n) if n >= 0 => Ok(n),
        _ => Err(http::error(
            400,
            &format!("{name} must be a non-negative integer"),
        )),
    }
}

fn trim(s: &str) -> String {
    s.trim_matches(gocompat::text::is_space).to_string()
}

fn board_unavailable() -> Resp {
    http::error(503, "the board is not running")
}

fn capture_unavailable() -> Resp {
    http::error(503, "capture is not running")
}

fn board_view(h: &Handler, for_key: &str) -> Resp {
    let Some(co) = h.s.b.coord() else {
        return board_unavailable();
    };
    let q = &h.r.query;
    let done = match positive_param(q, "done", coord::DEFAULT_DONE_CARD, coord::MAX_DONE_CARD) {
        Ok(n) => n,
        Err(r) => return r,
    };
    let v = co.board(&coord::BoardFilter {
        repo_key: trim(&q.get("repo")),
        viewer: for_key.to_string(),
        done_limit: done,
    });
    http::json(200, &v)
}

fn task(h: &Handler, id: &str) -> Resp {
    let Some(co) = h.s.b.coord() else {
        return board_unavailable();
    };
    if id.is_empty() || id.contains('/') {
        return http::error(404, "no such route");
    }
    let Some(t) = co.get_task(id) else {
        return http::error(404, &format!("no task {}", gocompat::quote::quote(id)));
    };
    let comments = co.comments(id).unwrap_or_default();
    http::json(200, &TaskDetail { task: t, comments })
}

/// `?kinds=` (repeatable, comma-separated); `Err` is the 400.
fn search_kinds(q: &Query) -> Result<Vec<search::Kind>, Resp> {
    let mut out: Vec<search::Kind> = Vec::new();
    for v in q.all("kinds") {
        for k in v.split(',') {
            let k = gocompat::text::to_lower_str(k.trim_matches(gocompat::text::is_space));
            if k.is_empty() {
                continue;
            }
            let kind = search::KINDS
                .iter()
                .copied()
                .find(|x| x.as_str() == k && *x != search::Kind::Agent);
            let Some(kind) = kind else {
                return Err(http::error(
                    400,
                    &format!(
                        "kinds: {} is not a searchable kind",
                        gocompat::quote::quote(&k)
                    ),
                ));
            };
            if !out.contains(&kind) {
                out.push(kind);
            }
        }
    }
    Ok(out)
}

fn search_route(h: &Handler, viewer: &str) -> Resp {
    let Some(c) = h.s.b.capture() else {
        return capture_unavailable();
    };
    let q = &h.r.query;
    let mut sq = search::Query {
        text: q.get("q"),
        repo_key: trim(&q.get("repo")),
        agent: trim(&q.get("agent")),
        viewer: viewer.to_string(),
        ..Default::default()
    };
    match search_kinds(q) {
        Ok(k) => sq.kinds = k,
        Err(r) => return r,
    }
    let v = q.get("before");
    if !v.is_empty() {
        match parse_time(&v) {
            Some(t) => sq.before = t,
            None => return http::error(400, "before must be an RFC 3339 time"),
        }
    }
    match int_param(q, "offset", 0) {
        Ok(n) => sq.offset = n,
        Err(r) => return r,
    }
    let limit = match limit_param(q, search::DEFAULT_LIMIT, search::MAX_LIMIT) {
        Ok(n) => n,
        Err(r) => return r,
    };
    sq.limit = limit + 1; // one more says whether another page exists
    let mut hits = match c.search(&sq) {
        Ok(h) => h,
        Err(e) => return http::error(500, &e.to_string()),
    };
    let mut res = SearchResult {
        hits: vec![],
        more: false,
        next: 0,
    };
    if hits.len() as i64 > limit {
        hits.truncate(limit as usize);
        res.more = true;
        res.next = sq.offset + limit;
    }
    res.hits = hits;
    http::json(200, &res)
}

fn show(h: &Handler, viewer: &str) -> Resp {
    let Some(c) = h.s.b.capture() else {
        return capture_unavailable();
    };
    let q = &h.r.query;
    let r = trim(&q.get("ref"));
    if r.is_empty() {
        return http::error(400, "ref is required");
    }
    let mut o = search::ShowOptions {
        viewer: viewer.to_string(),
        ..Default::default()
    };
    match int_param(q, "offset", 0) {
        Ok(n) => o.offset = n,
        Err(r) => return r,
    }
    match limit_param(q, search::DEFAULT_SHOW, search::MAX_SHOW) {
        Ok(n) => o.limit = n,
        Err(r) => return r,
    }
    match c.show(&r, &o) {
        Ok(sh) => http::json(200, &sh),
        Err(e @ search::Error::BadRef) => http::error(400, &e.to_string()),
        Err(search::Error::NotFound | search::Error::NotFoundRef(_)) => http::error(
            404,
            &format!("nothing has the ref {}", gocompat::quote::quote(&r)),
        ),
        Err(e) => http::error(500, &e.to_string()),
    }
}

fn overview(h: &Handler, viewer: &str) -> Resp {
    let mut agent = trim(&h.r.query.get("agent"));
    if viewer == AGENT_VIEWER {
        return http::error(
            403,
            &format!(
                "an overview carries an agent's inbox: send that agent's {HEADER_ATTACH} or {HEADER_MEMBER} secret"
            ),
        );
    }
    if !viewer.is_empty() && !agent.is_empty() && agent != viewer {
        return http::error(403, "an agent reads only its own overview");
    }
    if viewer.is_empty() && agent.is_empty() {
        return http::error(400, "agent is required");
    }
    if !viewer.is_empty() {
        agent = viewer.to_string();
    }
    let Some(c) = h.s.b.capture() else {
        return capture_unavailable();
    };
    let Some(co) = h.s.b.coord() else {
        return board_unavailable();
    };
    let Some(ib) = h.s.b.inbox() else {
        return http::error(503, "messaging is not running");
    };
    let input = overview_input(&co, &ib, &h.s.b.agents(), &agent);
    http::json(200, &c.overview(&agent, &input))
}

/// What `Capture::overview` copies in from coord and the inbox for agent
/// `key` (Go `overviewInput`): its newest open deliveries, the blocked
/// tasks in its repo, the other open ones there (its own first), and each
/// holder's last-changed task.
pub fn overview_input(
    co: &coord::Coord,
    ib: &inbox::Inbox,
    cards: &[presence::AgentCard],
    key: &str,
) -> capture::OverviewInput {
    let mut input = capture::OverviewInput::default();
    if let Some(c) = cards.iter().find(|c| c.key == key) {
        input.repo_key = c.repo_key.clone();
    }

    let open = ib.for_agent(key, false, COUNT_LIMIT); // newest first
    input.inbox_open = open.len() as i64;
    for d in open.iter().take(capture::OVERVIEW_LINES) {
        let from = &d.message.from.name;
        let text = if from.is_empty() {
            d.message.text.clone()
        } else {
            format!("{from}: {}", d.message.text)
        };
        input.inbox.push(capture::Line {
            r#ref: search::make_ref(search::Kind::Msg, &d.message_id),
            text,
            at: d.created_at,
        });
    }
    input.inbox_held = ib
        .list(COUNT_LIMIT)
        .iter()
        .filter(|d| d.to == key && d.state == inbox::st::HELD)
        .count() as i64;

    let mut tasks = co.list_tasks(&coord::TaskFilter {
        open: true,
        ..Default::default()
    });
    input.board_count = tasks.len() as i64;
    // Newest change first, so Current keeps each holder's latest task and
    // the lists read newest first (a stable sort, as Go's).
    tasks.sort_by_key(|t| std::cmp::Reverse(t.updated_at.unix_nano()));
    let mut current: HashMap<String, capture::Line> = HashMap::new();
    let (mut mine, mut others) = (Vec::new(), Vec::new());
    for t in &tasks {
        let mut line = capture::Line {
            r#ref: search::make_ref(search::Kind::Task, &t.id),
            text: t.title.clone(),
            at: t.updated_at,
        };
        for holder in &t.holders {
            if !current.contains_key(holder) && !t.assigned.contains(holder) {
                current.insert(holder.clone(), line.clone());
            }
        }
        if !input.repo_key.is_empty() && t.repo_key != input.repo_key {
            continue;
        }
        if t.state == coord::BLOCKED {
            if input.blockers.len() < capture::OVERVIEW_LINES {
                line.text = format!("{} (blocked: {})", t.title, t.reason);
                input.blockers.push(line);
            }
            continue;
        }
        if t.holders.iter().any(|x| x == key) {
            mine.push(line);
        } else {
            others.push(line);
        }
    }
    input.current = current;
    // capture caps Tasks and counts the rest into MoreTasks.
    mine.extend(others);
    input.tasks = mine;
    input
}
