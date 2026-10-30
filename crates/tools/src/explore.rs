//! Progressive disclosure (I:internal/tools/explore.go): overview, then
//! short lists with refs (search, commits, prs, board), then show(ref).
//! The caller's key is always the Viewer, never an argument.

use std::collections::HashMap;
use std::fmt::Write;

use coord::Task;
use gocompat::time::GoTime;
use search::{Hit, Kind, Query, ShowOptions};

use crate::args::Ty;
use crate::board::MAX_CARD_TITLE;
use crate::capturetools::{CAPTURED_UNTRUSTED, capture_off};
use crate::format::{after, ago, local, one_line, q, quoted, trim};
use crate::{Caller, Inner, MAX_RESULT, Names, ToolErr, decode, err_text, map_err, untrusted};

const DEFAULT_COMMIT_LIMIT: i64 = 10;
const MAX_COMMIT_LIMIT: i64 = 50;
const MAX_PRS: usize = 30;
/// Deliveries scanned for the held count.
const OVERVIEW_INBOX_SCAN: usize = 500;

/// Each agent's open card it holds (and has taken up) that changed last.
pub(crate) fn current(ts: &[Task]) -> HashMap<String, capture::Line> {
    let mut out: HashMap<String, capture::Line> = HashMap::new();
    let mut at: HashMap<String, GoTime> = HashMap::new();
    for t in ts {
        for h in &t.holders {
            if t.assigned.contains(h) {
                continue; // assigned, not taken up
            }
            if let Some(prev) = at.get(h)
                && !after(&t.updated_at, prev)
            {
                continue;
            }
            at.insert(h.clone(), t.updated_at);
            out.insert(
                h.clone(),
                capture::Line {
                    r#ref: search::make_ref(Kind::Task, &t.id),
                    text: format!(
                        "on the Board: {} {}",
                        t.short_ref(),
                        one_line(&t.title, MAX_CARD_TITLE)
                    ),
                    at: t.updated_at,
                },
            );
        }
    }
    out
}

fn task_overview_line(t: &Task, n: &Names<'_>) -> capture::Line {
    let mut text = format!(
        "{} [{}] {}",
        t.short_ref(),
        t.state,
        one_line(&t.title, MAX_CARD_TITLE)
    );
    if t.state == coord::BLOCKED {
        text += &format!(": {}", one_line(&t.reason, 80));
    } else if let Some(h) = t.holders.first() {
        text += &format!(", {}", n.who(h));
    }
    capture::Line {
        r#ref: search::make_ref(Kind::Task, &t.id),
        text,
        at: t.updated_at,
    }
}

impl Inner {
    /// What capture's Overview doesn't own: the caller's inbox and the
    /// Board in its repo.
    fn overview_input(&self, c: &Caller, n: &Names<'_>) -> capture::OverviewInput {
        let key = &c.sender.key;
        let repo = n
            .cards
            .get(key)
            .map(|c| c.repo_key.clone())
            .unwrap_or_default();
        let (ib, co) = (self.b.inbox(), self.b.coord());
        let mut input = capture::OverviewInput {
            repo_key: repo.clone(),
            ..capture::OverviewInput::default()
        };
        let open = ib.for_agent(key, false, inbox::DEFAULT_INBOX_LIST);
        input.inbox_open = open.len() as i64;
        for d in &open {
            if input.inbox.len() == capture::OVERVIEW_LINES {
                break;
            }
            let mut text = format!("from {}: ", n.sender(&d.message.from));
            let r = trim(&d.message.reason);
            if !r.is_empty() {
                text += &one_line(r, 80);
            } else {
                text += &one_line(&d.message.text, 80);
            }
            input.inbox.push(capture::Line {
                r#ref: search::make_ref(Kind::Msg, &d.message.id),
                text,
                at: d.created_at,
            });
        }
        for d in ib.list(OVERVIEW_INBOX_SCAN) {
            if d.to == *key && d.state == inbox::st::HELD {
                input.inbox_held += 1;
            }
        }
        let all = co.list_tasks(&coord::TaskFilter {
            open: true,
            ..coord::TaskFilter::default()
        });
        input.board_count = all.len() as i64;
        input.current = current(&all);
        let v = self.board_view(&repo, key, 1);
        let mut tasks = vec![];
        for cd in &v.needs_you {
            if cd.task.state == coord::BLOCKED {
                input.blockers.push(task_overview_line(&cd.task, n));
            } else {
                tasks.push(task_overview_line(&cd.task, n));
            }
        }
        for cd in &v.blocked {
            input.blockers.push(task_overview_line(&cd.task, n));
        }
        let (mut mine, mut others) = (vec![], vec![]);
        for cd in v.working.iter().chain(v.open.iter()) {
            if cd.task.holders.contains(key) {
                mine.push(task_overview_line(&cd.task, n));
            } else {
                others.push(task_overview_line(&cd.task, n));
            }
        }
        tasks.extend(mine);
        tasks.extend(others);
        input.blockers.truncate(capture::OVERVIEW_LINES);
        if tasks.len() > capture::OVERVIEW_LINES {
            input.more_tasks = (tasks.len() - capture::OVERVIEW_LINES) as i64;
            tasks.truncate(capture::OVERVIEW_LINES);
        }
        input.tasks = tasks;
        input
    }
}

fn write_lines(b: &mut String, head: &str, ls: &[capture::Line], now: &GoTime) {
    if ls.is_empty() {
        return;
    }
    let _ = writeln!(b, "{head}:");
    for l in ls {
        let _ = write!(
            b,
            "- {} {}",
            one_line(&l.r#ref, 200),
            one_line(&l.text, capture::OVERVIEW_LINE_MAX)
        );
        if !l.at.is_zero() {
            let _ = write!(b, " ({})", ago(now, &l.at));
        }
        *b += "\n";
    }
}

pub(crate) fn overview(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    if let Err(e) = decode(raw, &[]) {
        return e;
    }
    let Some(cp) = s.b.capture() else {
        return capture_off();
    };
    let n = s.roster();
    let now = s.now();
    let ov = cp.overview(&c.sender.key, &s.overview_input(c, &n));
    let where_ = if ov.repo_key.is_empty() {
        "no repo".to_string()
    } else {
        format!("repo {}", one_line(&ov.repo_key, 160))
    };
    let mut b = format!(
        "Overview for {}, {where_}. {CAPTURED_UNTRUSTED}\n",
        n.who(&c.sender.key)
    );
    let _ = write!(
        b,
        "\nAgents live ({}):\n",
        ov.agents.len() as i64 + ov.more_agents
    );
    for ag in &ov.agents {
        let you = if ag.key == c.sender.key { " (you)" } else { "" };
        let _ = write!(
            b,
            "- {}{you} [{}], key {}",
            one_line(&ag.name, 80),
            one_line(&ag.status, 30),
            one_line(&ag.key, 120)
        );
        if !ag.repo_key.is_empty() && ag.repo_key != ov.repo_key {
            b += &format!(", repo {}", one_line(&ag.repo_key, 160));
        }
        let d = one_line(&ag.doing.text, capture::OVERVIEW_LINE_MAX);
        if !d.is_empty() {
            let _ = write!(
                b,
                ": {} ({})",
                quoted(&d, capture::OVERVIEW_LINE_MAX),
                one_line(&ag.doing.r#ref, 200)
            );
        } else if !ag.doing.r#ref.is_empty() {
            let _ = write!(b, " ({})", one_line(&ag.doing.r#ref, 200));
        }
        b += "\n";
    }
    if ov.more_agents > 0 {
        let _ = writeln!(
            b,
            "({} more: the agents tool lists them all)",
            ov.more_agents
        );
    }
    let _ = write!(b, "\nInbox: {} open", ov.inbox_open);
    if ov.inbox_held > 0 {
        let _ = write!(b, ", {} held for the person to release", ov.inbox_held);
    }
    b += ".\n";
    write_lines(&mut b, "Newest", &ov.inbox, &now);
    if !ov.blockers.is_empty() || !ov.tasks.is_empty() {
        b += "\n";
    }
    write_lines(&mut b, "Blocked", &ov.blockers, &now);
    write_lines(&mut b, "On the Board", &ov.tasks, &now);
    if ov.more_tasks > 0 {
        let _ = writeln!(
            b,
            "({} more cards: the board tool lists them)",
            ov.more_tasks
        );
    }
    if !ov.commits.is_empty() || !ov.prs.is_empty() {
        b += "\n";
    }
    write_lines(&mut b, "Recent commits", &ov.commits, &now);
    write_lines(&mut b, "Pull requests", &ov.prs, &now);
    let _ = writeln!(b, "\n{}.", one_line(&ov.hint, 300));
    b
}

// --- search ---

const SEARCH_ARGS: &[(&str, Ty)] = &[
    ("query", Ty::Str),
    ("kinds", Ty::Strs),
    ("repo", Ty::Str),
    ("agent", Ty::Str),
    ("offset", Ty::Int),
    ("limit", Ty::Int),
];

pub(crate) fn search(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, SEARCH_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Some(cp) = s.b.capture() else {
        return capture_off();
    };
    let offset = a.int("offset");
    if offset < 0 {
        return err_text("offset is how many hits to skip: 0 or more.");
    }
    let mut qy = Query {
        text: trim(a.str("query")).into(),
        repo_key: trim(a.str("repo")).into(),
        offset,
        viewer: c.sender.key.clone(),
        ..Query::default()
    };
    for k in a.strs("kinds") {
        let name = gocompat::text::to_lower_str(trim(&k));
        if name.is_empty() {
            continue;
        }
        match Kind::from_name(&name) {
            Some(kind) if kind != Kind::Agent && search::KINDS.contains(&kind) => {
                qy.kinds.push(kind)
            }
            _ => {
                return err_text(&format!(
                    "kinds are turn, edit, commit, msg, task, comment, pr and decision, not {}.",
                    q(&one_line(&k, 40))
                ));
            }
        }
    }
    let ag = trim(a.str("agent"));
    if !ag.is_empty() {
        match s.resolve_agent(ag) {
            Ok(k) => qy.agent = k,
            Err(e) => return map_err(e),
        }
    }
    let mut limit = a.int("limit");
    if limit <= 0 {
        limit = search::DEFAULT_LIMIT;
    }
    let limit = limit.min(search::MAX_LIMIT);
    qy.limit = limit + 1;
    let hits = match cp.search(&qy) {
        Ok(h) => h,
        Err(e) => return err_text(&format!("search failed: {e}. Use plain words.")),
    };
    let mut hits = s.short_hit(&qy, hits);
    let mut more = hits.len() as i64 > limit;
    if more {
        hits.truncate(limit as usize);
    }
    let what = if qy.text.is_empty() {
        "newest items".to_string()
    } else {
        format!("hits for {}", q(&one_line(&qy.text, 120)))
    };
    if hits.is_empty() {
        if offset > 0 {
            return format!("No more {what} past offset {offset}.");
        }
        return format!("No {what}. Try fewer or other words, or drop kinds, repo or agent.");
    }
    let n = s.roster();
    let now = s.now();
    let mut b = format!("{} {what}", hits.len());
    if offset > 0 {
        let _ = write!(b, " (from offset {offset})");
    }
    let _ = writeln!(b, ". {CAPTURED_UNTRUSTED}");
    let budget = MAX_RESULT - 200;
    let mut shown = 0;
    for h in &hits {
        let mut r = one_line(&h.r#ref, 200);
        if h.kind == Kind::Task
            && let Ok((_, id)) = search::parse_ref(&h.r#ref)
            && let Some(t) = s.task(&id)
            && !t.short.is_empty()
        {
            r = format!("{} {r}", t.short_ref());
        }
        let mut line = format!(
            "- {r} {} {}",
            short2(&now, &h.at),
            quoted(&h.title, search::MAX_TITLE)
        );
        if !h.agent.is_empty() {
            line += &format!(", {}", n.who(&h.agent));
        }
        line += "\n";
        let sn = trim(&h.snippet);
        if !sn.is_empty() && sn != trim(&h.title) {
            line += &format!("  {}\n", quoted(sn, search::MAX_SNIPPET));
        }
        if b.len() + line.len() > budget {
            more = true;
            break;
        }
        b += &line;
        shown += 1;
    }
    if more {
        let _ = writeln!(
            b,
            "(more: offset={}; search again with the same arguments and offset = that value)",
            offset + shown
        );
    }
    b += "show <ref> for any hit.\n";
    b
}

impl Inner {
    /// Puts the card a query names by its short ref first on the first
    /// page, when the query is only that ref and the filters allow it.
    fn short_hit(&self, qy: &Query, hits: Vec<Hit>) -> Vec<Hit> {
        if qy.offset > 0 || (!qy.kinds.is_empty() && !qy.kinds.contains(&Kind::Task)) {
            return hits;
        }
        if coord::parse_ref(&qy.text).is_none() || !qy.text.contains('#') {
            return hits;
        }
        let Ok(id) = self.task_ref(&qy.text) else {
            return hits;
        };
        let Some(t) = self.task(&id) else {
            return hits;
        };
        if (!qy.repo_key.is_empty() && t.repo_key != qy.repo_key)
            || (!qy.agent.is_empty() && t.holder != qy.agent)
        {
            return hits;
        }
        let r = search::make_ref(Kind::Task, &t.id);
        let mut out = vec![Hit {
            r#ref: r.clone(),
            kind: Kind::Task,
            at: t.created_at,
            title: t.title.clone(),
            snippet: String::new(),
            agent: t.holder.clone(),
            repo_key: t.repo_key.clone(),
        }];
        out.extend(hits.into_iter().filter(|h| h.r#ref != r));
        out
    }
}

/// A hit's time: the date and clock, and how long ago.
fn short2(now: &GoTime, t: &GoTime) -> String {
    if t.is_zero() {
        return "(no time)".into();
    }
    let full = local(t, now).format_date_time();
    format!("{} ({})", &full[..full.len() - 3], ago(now, t))
}

// --- show ---

const SHOW_ARGS: &[(&str, Ty)] = &[("ref", Ty::Str), ("offset", Ty::Int)];

pub(crate) fn show(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, SHOW_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Some(cp) = s.b.capture() else {
        return capture_off();
    };
    let mut r = trim(a.str("ref")).to_string();
    if r.is_empty() {
        return err_text(
            "give ref, exactly as a list line printed it (#12, task:…, msg:…, turn:…, commit:…).",
        );
    }
    if coord::parse_ref(&r).is_some() {
        match s.task_ref(&r) {
            Ok(id) => r = search::make_ref(Kind::Task, &id),
            Err(e) if e.is(coord::ErrorKind::Ambiguous) => return map_err(e),
            Err(_) => {
                return err_text(&format!(
                    "no card is {}. The board tool lists them with their short refs.",
                    one_line(&r, 40)
                ));
            }
        }
    }
    let offset = a.int("offset");
    if offset < 0 {
        return err_text("offset is where to continue: 0 or more.");
    }
    let sh = match cp.show(
        &r,
        &ShowOptions {
            offset,
            viewer: c.sender.key.clone(),
            ..ShowOptions::default()
        },
    ) {
        Ok(sh) => sh,
        Err(e) if e.is_not_found() => {
            return err_text(&format!(
                "nothing you may read has the ref {}. search finds refs; a message shows only to its sender and recipients.",
                q(&one_line(&r, 200))
            ));
        }
        Err(e) => return map_err(ToolErr::Search(e)),
    };
    let mut b = format!(
        "{} {}",
        one_line(&sh.r#ref, 200),
        quoted(&sh.title, search::MAX_TITLE)
    );
    if offset > 0 {
        let _ = write!(b, " (from offset {offset})");
    }
    let _ = write!(b, ". {CAPTURED_UNTRUSTED}\n\n");
    let _ = writeln!(b, "{}", sh.text.trim_end_matches('\n'));
    if sh.more {
        let _ = write!(
            b,
            "\n(more: offset={}; show ref = {} with offset = that value continues)\n",
            sh.next,
            one_line(&sh.r#ref, 200)
        );
    }
    if !sh.related.is_empty() {
        let rel: Vec<String> = sh.related.iter().map(|r| one_line(r, 200)).collect();
        let _ = writeln!(b, "Related: {}", rel.join(", "));
    }
    b
}

// --- commits / prs ---

const COMMITS_ARGS: &[(&str, Ty)] = &[("repo", Ty::Str), ("path", Ty::Str), ("limit", Ty::Int)];

pub(crate) fn commits(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, COMMITS_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Some(cp) = s.b.capture() else {
        return capture_off();
    };
    let mut repo = trim(a.str("repo")).to_string();
    if repo.is_empty() {
        repo = s.caller_repo(c);
    }
    if repo.is_empty() {
        return err_text(
            "give repo: a repoKey from the agents tool (Irori knows no repo for you).",
        );
    }
    let mut limit = a.int("limit");
    if limit <= 0 {
        limit = DEFAULT_COMMIT_LIMIT;
    }
    let limit = limit.min(MAX_COMMIT_LIMIT) as usize;
    let p = trim(a.str("path")).replace('\\', "/");
    let p = p.trim_matches('/');
    let path = p.strip_prefix("./").unwrap_or(p).to_string();
    let scan = if path.is_empty() { limit + 1 } else { 500 };
    let mut list: Vec<capture::CommitView> = cp
        .commits(&repo, scan)
        .into_iter()
        .filter(|cv| path.is_empty() || touches(cv, &path))
        .collect();
    let mut where_ = one_line(&repo, 160);
    if !path.is_empty() {
        where_ += &format!(", touching {}", one_line(&path, 200));
    }
    if list.is_empty() {
        return format!(
            "No commits seen in {where_}. Irori sees commits made in worktrees it watches."
        );
    }
    let more = list.len() > limit;
    list.truncate(limit);
    let n = s.roster();
    let now = s.now();
    let mut b = format!(
        "{} commits in {where_}, newest first. {CAPTURED_UNTRUSTED}\n",
        list.len()
    );
    let budget = MAX_RESULT - 240;
    let mut cut = 0;
    for (i, cv) in list.iter().enumerate() {
        let cm = &cv.row.commit;
        let mut line = format!(
            "- commit:{} {} {}",
            one_line(&cm.sha, 64),
            short2(&now, &cm.at),
            quoted(&cm.subject, 160)
        );
        let br = one_line(&cv.row.branch, 80);
        if !br.is_empty() {
            line += &format!(" on {br}");
        }
        let au = one_line(&cm.author, 80);
        if !au.is_empty() {
            line += &format!(", author {au}");
        }
        if let Some(k) = cv.link.as_ref().and_then(|l| l.committer.as_ref()) {
            line += &format!(", committed by {}", n.who(&k.key));
        }
        line += "\n";
        if b.len() + line.len() > budget {
            cut = list.len() - i;
            break;
        }
        b += &line;
    }
    if cut > 0 {
        let _ = writeln!(
            b,
            "({cut} more not shown; narrow with path, or search kinds=[\"commit\"] with words from the subject)"
        );
    } else if more {
        let _ = writeln!(
            b,
            "(more: raise limit, up to {MAX_COMMIT_LIMIT}; or search kinds=[\"commit\"] with words from the subject)"
        );
    }
    b += "show commit:<sha> for its files and the turns behind it.\n";
    b
}

/// A commit changed `path`, or something under it.
fn touches(cv: &capture::CommitView, path: &str) -> bool {
    cv.row
        .commit
        .files
        .iter()
        .flatten()
        .any(|f| f.path == path || f.path.starts_with(&format!("{path}/")))
}

const PRS_ARGS: &[(&str, Ty)] = &[("repo", Ty::Str)];

pub(crate) fn prs(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, PRS_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Some(cp) = s.b.capture() else {
        return capture_off();
    };
    let mut repo = trim(a.str("repo")).to_string();
    if repo.is_empty() {
        repo = s.caller_repo(c);
    }
    let gh = if repo.contains(':') {
        ledger::repo_from_key(&repo).to_string()
    } else {
        repo.clone()
    };
    if gh.is_empty() || !gh.contains('/') {
        return err_text(
            "give repo: a GitHub repo as remote:github.com/<owner>/<name> or <owner>/<name>.",
        );
    }
    // ForRepo is newest PR number first, then best owner first.
    let mut keys: Vec<String> = vec![];
    let mut by_key: HashMap<String, Vec<ledger::Entry>> = HashMap::new();
    for e in cp.prs(&gh) {
        if !by_key.contains_key(&e.key) {
            keys.push(e.key.clone());
        }
        by_key.entry(e.key.clone()).or_default().push(e);
    }
    if keys.is_empty() {
        return format!(
            "No pull requests linked to agents in {}.",
            one_line(&gh, 160)
        );
    }
    let more = keys.len() > MAX_PRS;
    keys.truncate(MAX_PRS);
    let n = s.roster();
    let mut b = format!(
        "{} pull requests in {}, newest first. {}\n",
        keys.len(),
        one_line(&gh, 160),
        untrusted()
    );
    for k in &keys {
        let es = &by_key[k];
        let who: Vec<String> = es
            .iter()
            .map(|e| format!("{} {}", one_line(e.role.as_str(), 20), n.who(&e.session)))
            .collect();
        let _ = write!(b, "- {}: {}", one_line(k, 200), who.join(", "));
        for e in es {
            let u = one_line(&e.url, 200);
            if !u.is_empty() {
                b += &format!(" {u}");
                break;
            }
        }
        b += "\n";
    }
    if more {
        b += "(more not shown: search kinds=[\"pr\"] finds older ones)\n";
    }
    b += "show pr:<owner>/<name>#<n> for one.\n";
    b
}
