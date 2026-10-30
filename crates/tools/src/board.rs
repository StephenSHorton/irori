//! The Board tools (I:internal/tools/board.go, route.go): a scratch pad for
//! live coordination, never a tracker. Every card line carries its ref,
//! and every call that someone should hear about routes coord's Notice
//! from the caller.

use std::fmt::Write;
use std::sync::Arc;
use std::time::Duration;

use coord::{
    BlockedBy, Blocker, BlockerAnswer, BlockerOption, BoardFilter, BoardView, Card, Comment,
    CommentInput, Coord, Link, NewBlocker, NewSubtask, NewTask, Step, StepUpdate, Task,
};
use gocompat::time::GoTime;

use crate::args::{Args, StepArg, Ty};
use crate::attachments::{attached_line, describe_coord, to_coord};
use crate::format::{MAX_FIELD_RUNES, ago, in_, one_line, q, quoted, trim};
use crate::{Caller, Inner, MAX_RESULT, Names, decode, err_text, map_err, untrusted};

/// A card's title in a list line.
pub(crate) const MAX_CARD_TITLE: usize = 160;

/// Board list lines show this much of a blocked card; the rest is counted.
const LIST_ITEMS: usize = 1;
const LIST_OPTIONS: usize = 3;

pub(crate) fn need_task_id() -> String {
    err_text(
        "give task_id: the card's short ref as the board tool shows it (#12), or its task:<id>.",
    )
}

/// A card in a list line: "#12 task:<id>".
fn card_refs(t: &Task) -> String {
    if t.short.is_empty() {
        return format!("task:{}", one_line(&t.id, 80));
    }
    format!(
        "{}#{} task:{}",
        one_line(&t.tag, 20),
        one_line(&t.short, 20),
        one_line(&t.id, 80)
    )
}

impl Inner {
    /// Resolves what the caller gave for a card. `Err` is the tool's answer
    /// when it can't be used: none given, or a number naming more than one
    /// card. A short ref no card has comes back as given.
    pub(crate) fn task_id(&self, v: &str) -> Result<String, String> {
        if trim(v).is_empty() {
            return Err(need_task_id());
        }
        match self.task_ref(v) {
            Ok(id) => Ok(id),
            Err(e) if e.is(coord::ErrorKind::Ambiguous) => Err(map_err(e)),
            Err(_) => Ok(trim(v).to_string()),
        }
    }

    /// Resolves `r` across the logs this backend holds.
    pub(crate) fn task_ref(&self, r: &str) -> Result<String, coord::Error> {
        match self.b.router() {
            Some(rt) => rt.task_ref(r),
            None => self.b.coord().task_ref(r),
        }
    }

    /// The card `id` names, in whichever log holds it.
    pub(crate) fn task(&self, id: &str) -> Option<Task> {
        let (co, _) = self.on(id, &[]).ok()?;
        co.get_task(id)
    }

    /// The coord a call on card `id` acts on, with `keys` as it knows them.
    pub(crate) fn on(
        &self,
        id: &str,
        keys: &[String],
    ) -> Result<(Arc<Coord>, Vec<String>), coord::Error> {
        match self.b.router() {
            Some(r) => r.task_coord(id, keys),
            None => Ok((self.b.coord().clone(), keys.to_vec())),
        }
    }

    /// A card's short ref by id ("#12"), or task:<id> when there is none.
    pub(crate) fn short_of(&self, id: &str) -> String {
        match self.task(id) {
            Some(t) => t.short_ref(),
            None => format!("task:{}", one_line(id, 80)),
        }
    }

    /// The caller's repoKey from its card ("" for Grok members).
    pub(crate) fn caller_repo(&self, c: &Caller) -> String {
        self.roster()
            .cards
            .get(&c.sender.key)
            .map(|c| c.repo_key.clone())
            .unwrap_or_default()
    }

    /// Adds each shared board's view of `f` to `v`.
    fn with_boards(&self, mut v: BoardView, f: &BoardFilter) -> BoardView {
        let Some(bv) = self.b.board_viewer() else {
            return v;
        };
        for o in bv.board_views(f) {
            v.needs_you.extend(o.needs_you);
            v.blocked.extend(o.blocked);
            v.working.extend(o.working);
            v.open.extend(o.open);
            v.done.extend(o.done);
            v.done_total += o.done_total;
            v.leases.extend(o.leases);
        }
        v
    }

    /// The Board for `repo` ("" every repo) as `viewer` sees it, with
    /// NeedsYou taken across every repo.
    pub(crate) fn board_view(&self, repo: &str, viewer: &str, done_limit: i64) -> BoardView {
        let co = self.b.coord();
        let f = BoardFilter {
            repo_key: repo.into(),
            viewer: viewer.into(),
            done_limit,
        };
        let mut v = self.with_boards(co.board(&f), &f);
        if !repo.is_empty() {
            let all = BoardFilter {
                repo_key: String::new(),
                viewer: viewer.into(),
                done_limit: 1,
            };
            v.needs_you = self.with_boards(co.board(&all), &all).needs_you;
        }
        v
    }

    pub(crate) fn card_line(
        &self,
        t: &Task,
        last: Option<&Comment>,
        n: &Names<'_>,
        now: &GoTime,
    ) -> String {
        self.card_text(t, last, n, now, false)
    }

    /// One card as list lines; `brief` (the board list) shows only the
    /// first open item and its first options.
    pub(crate) fn card_text(
        &self,
        t: &Task,
        last: Option<&Comment>,
        n: &Names<'_>,
        now: &GoTime,
        brief: bool,
    ) -> String {
        let mut b = String::new();
        let mut state = coord::effective_state(t).to_string();
        if t.open_ask().is_some() {
            state = "ask".into();
        }
        if let Some(r) = &t.rollup {
            state += &format!(", {}/{} subtasks", r.done, r.total);
        }
        let _ = write!(
            b,
            "- {} [{}] {}",
            card_refs(t),
            one_line(&state, 40),
            quoted(&one_line(&t.title, MAX_CARD_TITLE), MAX_CARD_TITLE)
        );
        if !t.parent.is_empty() {
            b += &format!("; part of {}", self.short_of(&t.parent));
        }
        if t.pinned {
            b += "; pinned by the person";
        }
        let holders: Vec<String> = t
            .holders
            .iter()
            .map(|h| {
                let mut w = n.who(h);
                if t.assigned.contains(h) {
                    w += " (assigned, not taken up)";
                }
                w
            })
            .collect();
        if !holders.is_empty() {
            b += &format!("; held by {}", holders.join(", "));
        } else {
            b += "; nobody holds it";
        }
        if t.shared {
            b += "; shared";
        }
        if !t.links.is_empty() {
            let ls: Vec<String> = t.links.iter().map(|l| one_line(&l.label, 60)).collect();
            b += &format!("; links: {}", ls.join(", "));
        }
        if t.comments > 0 {
            b += &format!("; {} comments", t.comments);
        }
        if !t.watchers.is_empty() {
            let ws: Vec<String> = t.watchers.iter().map(|w| n.who(w)).collect();
            b += &format!("; watched by {}", ws.join(", "));
        }
        b += &format!("; updated {}\n", ago(now, &t.updated_at));
        if let Some(r) = &t.rollup {
            let mut ks = vec![];
            if let Ok((co, _)) = self.on(&t.id, &[]) {
                for k in &t.subtasks {
                    if let Some(kt) = co.get_task(k) {
                        ks.push(format!("{} {}", kt.short_ref(), kt.state));
                    }
                }
            }
            b += &format!(
                "  subtasks: {}\n",
                one_line(&ks.join(", "), MAX_FIELD_RUNES)
            );
            if !r.blocked.is_empty() {
                b += &format!(
                    "  blocked by its subtask {}: {}\n",
                    self.short_of(&r.blocked),
                    quoted(&one_line(&r.blocked_question, 160), 160)
                );
            }
        }
        let (done, total) = t.progress();
        if total > 0 && t.rollup.is_none() {
            b += &format!("  steps {done}/{total}");
            for st in &t.steps {
                if st.doing {
                    b += &format!(
                        ", now {} {}",
                        one_line(&st.id, 20),
                        quoted(&one_line(&st.text, 120), 120)
                    );
                }
            }
            b += "\n";
        }
        if t.state == coord::BLOCKED {
            let mut open = t.open_blockers();
            if open.is_empty() {
                // A blocked task always has an item; this is only for safety.
                let mut bl = Blocker::default();
                bl.question = t.reason.clone();
                open = vec![bl];
            }
            let on = match &t.blocked_by {
                Some(bb) => format!(
                    " (on {} {})",
                    one_line(&bb.kind, 20),
                    one_line(&bb.reference, 160)
                ),
                None => String::new(),
            };
            let more = if brief && open.len() > LIST_ITEMS {
                open.len() - LIST_ITEMS
            } else {
                0
            };
            for (i, bl) in open[..open.len() - more].iter().enumerate() {
                b += "  blocked";
                if !bl.id.is_empty() {
                    b += &format!(" [{}]", one_line(&bl.id, 20));
                }
                b += &format!(": {}", quoted(&bl.question, MAX_FIELD_RUNES));
                if i == 0 {
                    b += &on;
                }
                if !bl.raised_by.is_empty() {
                    b += &format!(" raised by {}", n.short(&bl.raised_by));
                }
                match bl.status.as_str() {
                    coord::BLOCKER_ANSWERED => {
                        b += "; answered";
                        let to = if bl.sent_to.is_empty() {
                            &bl.raised_by
                        } else {
                            &bl.sent_to
                        };
                        if !to.is_empty() {
                            b += &format!(" (waiting on {}: resolve or reply)", n.short(to));
                        }
                    }
                    coord::BLOCKER_REPLIED => {
                        b += "; replied (with the person)";
                        if !bl.missing.is_empty() {
                            let ms: Vec<String> = bl
                                .missing
                                .iter()
                                .map(|m| quoted(m, coord::MAX_MISSING_TEXT))
                                .collect();
                            b += &format!(", missing: {}", ms.join(", "));
                        }
                    }
                    _ => {}
                }
                b += "\n";
                let opts: &[BlockerOption] = if brief && bl.options.len() > LIST_OPTIONS {
                    &bl.options[..LIST_OPTIONS]
                } else {
                    &bl.options
                };
                if !bl.links.is_empty() {
                    b += &format!("    links: {}\n", link_refs(&bl.links));
                }
                for f in &bl.attachments {
                    b += &format!("    file: {} (attachment_get)\n", describe_coord(f));
                }
                for (j, o) in opts.iter().enumerate() {
                    b += &format!(
                        "    {}. {} ({})",
                        j + 1,
                        quoted(&o.answer, 160),
                        n.short(&o.by)
                    );
                    if o.recommended {
                        b += " [recommended]";
                    }
                    if !o.why.is_empty() {
                        b += &format!(": {}", quoted(&o.why, 160));
                    }
                    if let Some(l) = &o.link {
                        b += &format!(" link: {}", link_refs(std::slice::from_ref(l)));
                    }
                    b += "\n";
                }
                if opts.len() < bl.options.len() {
                    b += &format!("    (+{} more answers)\n", bl.options.len() - opts.len());
                }
            }
            if more > 0 {
                let ids: Vec<String> = open[open.len() - more..]
                    .iter()
                    .map(|bl| one_line(&bl.id, 20))
                    .collect();
                b += &format!(
                    "  +{more} more open items ({}); show {} lists them all\n",
                    ids.join(", "),
                    t.short_ref()
                );
            }
        }
        if let Some(l) = last {
            b += &format!(
                "  last: comment:{} by {} {}: {}\n",
                one_line(&l.id, 80),
                n.who(&l.by),
                ago(now, &l.at),
                quoted(&l.text, 160)
            );
        }
        b
    }

    /// An agent name or key (live or gone, the caller too), or a person's
    /// "user:*" key passed through.
    pub(crate) fn assignee(&self, v: &str) -> Result<String, inbox::Error> {
        if self.is_owner(v) {
            return Ok(v.to_string());
        }
        self.resolve_agent(v)
    }

    fn line(&self, t: &Task) -> String {
        self.card_line(t, None, &self.roster(), &self.now())
    }
}

/// A card's ask, when it is open (Go `t.Ask.Open()` on a nil pointer).
fn ask_open(t: &Task) -> bool {
    t.ask.as_ref().is_some_and(|a| a.is_open())
}

// --- board ---

const BOARD_ARGS: &[(&str, Ty)] = &[("repo", Ty::Str), ("done", Ty::Bool)];

pub(crate) fn board(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, BOARD_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let done = a.bool("done");
    let mut repo = trim(a.str("repo")).to_string();
    match repo.as_str() {
        "*" | "all" => repo.clear(),
        "" => repo = s.caller_repo(c),
        _ => {}
    }
    let done_limit = if done { coord::DEFAULT_DONE_CARD } else { 1 };
    let me = &c.sender.key;
    let mut v = s.board_view(&repo, me, done_limit);
    let (mut asks, mut took) = ask_groups(&mut v, me, &repo);
    if !repo.is_empty() {
        // Asks from every repo: one with no repo is for anyone.
        let mut all = s.b.coord().board(&BoardFilter {
            repo_key: String::new(),
            viewer: me.clone(),
            done_limit: 1,
        });
        (asks, took) = ask_groups(&mut all, me, &repo);
    }
    let where_ = if repo.is_empty() {
        "every repo".to_string()
    } else {
        one_line(&repo, 160)
    };
    let open = asks.len()
        + took.len()
        + v.needs_you.len()
        + v.blocked.len()
        + v.working.len()
        + v.open.len();
    let head = format!(
        "Board for {where_}: {open} open cards, {} done",
        v.done_total
    );
    if open == 0 && v.leases.is_empty() && (!done || v.done_total == 0) {
        return head + ". Nothing live here; board_create puts up a card for work starting now.";
    }
    let n = s.roster();
    let now = s.now();
    let mut b = format!("{head}. {}\n", untrusted());
    const HINT_ROOM: usize = 200;
    let budget = MAX_RESULT - HINT_ROOM;
    let mut cut = 0;
    let group = |b: &mut String, name: &str, cards: &[Card], cut: &mut usize| {
        if cards.is_empty() {
            return;
        }
        let _ = write!(b, "\n{name} ({}):\n", cards.len());
        for cd in cards {
            let mut line = s.card_text(&cd.task, cd.last_comment.as_ref(), &n, &now, true);
            if !repo.is_empty() && cd.task.repo_key != repo {
                let w = if cd.task.repo_key.is_empty() {
                    "no repo".to_string()
                } else {
                    format!("repo {}", one_line(&cd.task.repo_key, 160))
                };
                line += &format!("  from {w}\n");
            }
            if cd.resolved {
                line += "  its blocker has gone away; it closes on the next sweep, or board_unblock now\n";
            }
            if !cd.gone.is_empty() {
                line += &format!(
                    "  what {} waited on has gone away; the next sweep closes just those, the other items still wait\n",
                    one_line(&cd.gone.join(", "), 80)
                );
            }
            if !cd.overlaps.is_empty() {
                line += &format!(
                    "  holders' leases overlap: {}\n",
                    one_line(&cd.overlaps.join("; "), MAX_FIELD_RUNES)
                );
            }
            if b.len() + line.len() > budget {
                *cut += 1;
                continue;
            }
            b.push_str(&line);
        }
    };
    group(
        &mut b,
        "Asks from the person, nobody has taken them (ask_claim one if you're free)",
        &asks,
        &mut cut,
    );
    group(
        &mut b,
        "Asks you took (ask_close each with what it became)",
        &took,
        &mut cut,
    );
    group(&mut b, "Needs you", &v.needs_you, &mut cut);
    group(&mut b, "Blocked", &v.blocked, &mut cut);
    group(&mut b, "Working", &v.working, &mut cut);
    group(&mut b, "Open", &v.open, &mut cut);
    if !v.leases.is_empty() && cut == 0 {
        let _ = write!(b, "\nLeases in the way ({}):\n", v.leases.len());
        for l in &v.leases {
            let mut line = format!(
                "- {} held by {}, expires {}",
                one_line(&l.lease.resource, MAX_FIELD_RUNES),
                n.who(&l.lease.holder),
                in_(&now, &l.lease.until)
            );
            if !l.blocks.is_empty() {
                let bs: Vec<String> = l.blocks.iter().map(|id| s.short_of(id)).collect();
                line += &format!("; blocks {}", one_line(&bs.join(", "), MAX_FIELD_RUNES));
            }
            let refused: Vec<String> = l
                .refused
                .iter()
                .map(|r| format!("{} {}", n.who(&r.by), ago(&now, &r.at)))
                .collect();
            if !refused.is_empty() {
                line += &format!("; refused to {}", refused.join(", "));
            }
            line += "\n";
            if b.len() + line.len() > budget {
                cut += 1;
                continue;
            }
            b.push_str(&line);
        }
    }
    if done {
        group(&mut b, "Done, newest first", &v.done, &mut cut);
    }
    if cut > 0 {
        let _ = write!(
            b,
            "\n({cut} more lines not shown; narrow with repo, or search kinds=[\"task\"] with words from the card)\n"
        );
    }
    if !done && v.done_total > 0 {
        let _ = write!(
            b,
            "\n{} done cards are collapsed: board done = true lists the newest {}. ",
            v.done_total,
            coord::DEFAULT_DONE_CARD
        );
    } else if done && v.done_total > v.done.len() as i64 {
        let _ = write!(
            b,
            "\n{} older done cards not listed: search kinds=[\"task\"] finds them. ",
            v.done_total - v.done.len() as i64
        );
    } else {
        b += "\n";
    }
    b += "show #<n> (or task:<id>) for a card's comments and links.\n";
    b
}

/// Splits the Board's asks off for the board tool: open asks the viewer
/// could take, then the ones it took. They come out of the other groups.
pub(crate) fn ask_groups(v: &mut BoardView, viewer: &str, repo: &str) -> (Vec<Card>, Vec<Card>) {
    let (mut open, mut mine) = (vec![], vec![]);
    for g in [
        &mut v.needs_you,
        &mut v.blocked,
        &mut v.working,
        &mut v.open,
    ] {
        let mut keep = vec![];
        for cd in g.drain(..) {
            match cd.task.ask.as_ref().filter(|a| a.is_open()) {
                None => keep.push(cd),
                Some(a) if a.taker == viewer => mine.push(cd),
                Some(a)
                    if a.taker.is_empty()
                        && (a.repo_key.is_empty() || repo.is_empty() || a.repo_key == repo) =>
                {
                    open.push(cd)
                }
                Some(_) => keep.push(cd),
            }
        }
        *g = keep;
    }
    (open, mine)
}

// --- board_create ---

const CREATE_ARGS: &[(&str, Ty)] = &[
    ("title", Ty::Str),
    ("objective", Ty::Str),
    ("done_when", Ty::Str),
    ("repo", Ty::Str),
    ("shared", Ty::Bool),
    ("links", Ty::Strs),
    ("assign_to", Ty::Str),
    ("steps", Ty::Steps),
    ("parent", Ty::Str),
];

/// Step arguments as coord's steps.
pub(crate) fn steps_of(v: &[StepArg]) -> Vec<Step> {
    v.iter()
        .map(|st| Step {
            id: st.id.clone(),
            text: st.text.clone(),
            done: st.done,
            doing: st.doing,
        })
        .collect()
}

pub(crate) fn board_create(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, CREATE_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    if trim(a.str("title")).is_empty() {
        return err_text("give the card a title: what is being worked on now.");
    }
    let mut repo = trim(a.str("repo")).to_string();
    if repo.is_empty() {
        repo = s.caller_repo(c);
    }
    let mut to = String::new();
    let v = trim(a.str("assign_to"));
    if !v.is_empty() {
        match s.assignee(v) {
            Ok(k) => to = k,
            Err(e) => return map_err(e),
        }
    }
    // A subtask goes in its parent's log, with the caller and assignee as
    // that log knows them.
    let mut co = s.b.coord().clone();
    let mut keys = vec![c.sender.key.clone(), to.clone()];
    let mut parent = trim(a.str("parent")).to_string();
    if !parent.is_empty() {
        match s.task_ref(&parent) {
            Ok(id) => parent = id,
            Err(e) => return map_err(e),
        }
        match s.on(&parent, &keys) {
            Ok((c2, k2)) => (co, keys) = (c2, k2),
            Err(e) => return map_err(e),
        }
    }
    let private = Arc::ptr_eq(&co, s.b.coord());
    let t = match co.create_task(
        &keys[0],
        NewTask {
            title: trim(a.str("title")).into(),
            objective: trim(a.str("objective")).into(),
            done_criteria: trim(a.str("done_when")).into(),
            repo_key: repo,
            shared: a.bool("shared") && private,
            links: a.strs("links"),
            steps: steps_of(&a.steps("steps")),
            parent,
            ..NewTask::default()
        },
    ) {
        Ok(t) => t,
        Err(e) => return map_err(e),
    };
    let n = s.roster();
    let mut out = if !t.parent.is_empty() {
        format!(
            "Created {} (task:{}), a subtask of {}.",
            t.short_ref(),
            t.id,
            s.short_of(&t.parent)
        )
    } else {
        format!("Created {} (task:{}).", t.short_ref(), t.id)
    };
    if t.steps.is_empty() {
        out += " Give it steps with board_steps once you know them, so the person sees how far along it is.";
    } else {
        out += " Tick its steps with board_step as you go.";
    }
    let mut t = t;
    if !to.is_empty() {
        match co.assign(&keys[0], &t.id, &keys[1]) {
            Err(e) => {
                return format!(
                    "{out} Assigning it failed: {}\n{}",
                    map_err(e).trim_start_matches("Error: "),
                    s.card_line(&t, None, &n, &s.now())
                );
            }
            Ok((at, notice)) => {
                t = at;
                out += &format!(
                    " Assigned to {}.{}",
                    n.who(&to),
                    s.told(&s.tell(&c.sender, notice), &n)
                );
            }
        }
    } else {
        out += " Take it up with board_join, or hand it over with board_assign.";
    }
    format!("{out}\n{}", s.card_line(&t, None, &n, &s.now()))
}

// --- board_comment ---

const COMMENT_ARGS: &[(&str, Ty)] = &[
    ("task_id", Ty::Str),
    ("text", Ty::Str),
    ("wake", Ty::Bool),
    ("attach", Ty::Strs),
    ("blocker_id", Ty::Str),
    ("missing", Ty::Strs),
];

pub(crate) fn board_comment(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, COMMENT_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    if trim(a.str("text")).is_empty() {
        return err_text("give the comment text.");
    }
    let bid = trim(a.str("blocker_id")).to_string();
    let missing = a.strs("missing");
    if !missing.is_empty() && bid.is_empty() {
        return err_text("missing needs blocker_id: which blocker's answer it is missing from.");
    }
    if missing.len() > coord::MAX_MISSING {
        return err_text(&format!(
            "missing takes at most {} lines.",
            coord::MAX_MISSING
        ));
    }
    let files = match s.attach_files(c, &a.strs("attach")) {
        Ok(f) => f,
        Err(m) => return m,
    };
    let (co, as_) = match s.on(&id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let (cm, notice) = match co.comment_on(
        &as_[0],
        &id,
        CommentInput {
            text: trim(a.str("text")).into(),
            wake: a.bool("wake"),
            files: files.iter().map(to_coord).collect(),
            blocker_id: bid.clone(),
            missing,
        },
    ) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    if !bid.is_empty() {
        // A reply on a blocker tells nobody: the person is the audience.
        let b = one_line(&bid, 20);
        return format!(
            "Replied on blocker {b} of {}{}; it stays open and goes back to the person. Resolve it with board_unblock blocker_id={b} note=… once you have what you need. If your reply answered their question and the choices are not what they were, revise it now with board_block blocker_id={b} (the question and options that fit, or none); don't leave the same options up.",
            s.short_of(&id),
            attached_line(&files)
        );
    }
    let n = s.roster();
    let mut out = format!(
        "Commented comment:{} on {}{}.",
        cm.id,
        s.short_of(&cm.task_id),
        attached_line(&files)
    );
    if let Some(t) = co.get_task(&id)
        && ask_open(&t)
        && t.ask.as_ref().is_some_and(|a| a.taker != as_[0])
        && !t.watchers.contains(&as_[0])
    {
        // Chiming in on an ask makes you a watcher.
        if co.watch(&as_[0], &id).is_ok() {
            out +=
                " You watch it now: you hear when it changes, and answer the person only if asked.";
        }
    }
    let rs = s.tell(&c.sender, notice);
    if !rs.is_empty() {
        out += &s.told(&rs, &n);
    } else {
        out += " Nobody else holds it, so nobody was told; it shows on the Board.";
    }
    out
}

// --- board_block / board_unblock ---

const OPTION_FIELDS: &[(&str, Ty)] = &[
    ("answer", Ty::Str),
    ("why", Ty::Str),
    ("recommended", Ty::Bool),
    ("link", Ty::Str),
];

const BLOCK_ARGS: &[(&str, Ty)] = &[
    ("task_id", Ty::Str),
    ("question", Ty::Str),
    ("reason", Ty::Str),
    ("context", Ty::OptStr),
    ("options", Ty::Structs(OPTION_FIELDS)),
    ("on", Ty::Str),
    ("links", Ty::Strs),
    ("attach", Ty::Strs),
    ("blocker_id", Ty::Str),
];

fn link_labels(ls: &[Link]) -> String {
    ls.iter()
        .map(|l| one_line(&l.label, 60))
        .collect::<Vec<_>>()
        .join(", ")
}

fn link_refs(ls: &[Link]) -> String {
    ls.iter()
        .map(|l| one_line(&l.reference, 300))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Text that asks more than one thing: two or more question marks.
fn many_questions(text: &str) -> bool {
    text.matches('?').count() > 1
}

const ONE_ASK_EACH: &str = "Raise each question with its own board_block (question, context for a newcomer, options when there are real choices, and recommended: true on the one you recommend), so the person can answer them one by one.";

impl Inner {
    /// board_block's on: "<kind> <ref>" or "<kind>:<ref>".
    fn parse_on(&self, v: &str) -> Result<Option<BlockedBy>, String> {
        let v = trim(v);
        if v.is_empty() {
            return Ok(None);
        }
        let (kind, r) = match v.find([':', ' ']) {
            Some(i) => (&v[..i], trim(&v[i + 1..]).to_string()),
            None => (v, String::new()),
        };
        let mut bb = BlockedBy {
            kind: gocompat::text::to_lower_str(kind),
            reference: r.clone(),
        };
        match bb.kind.as_str() {
            coord::BLOCKED_ON_LEASE => {
                if r.is_empty() {
                    return Err(err_text(
                        "on = lease needs the resource, e.g. lease path:<repoKey>:src/api/** (see the leases tool).",
                    ));
                }
            }
            coord::BLOCKED_ON_TASK => {
                if r.is_empty() {
                    return Err(err_text(
                        "on = task needs its short ref or id, e.g. task #12.",
                    ));
                }
                bb.reference = self.task_id(&r)?;
                if coord::parse_short(&bb.reference).is_some() {
                    return Err(err_text(&format!(
                        "on = task: no card is {}; the board tool lists them.",
                        q(&one_line(&r, 40))
                    )));
                }
            }
            coord::BLOCKED_ON_PERSON => {
                if !r.is_empty() && !self.is_owner(&r) {
                    return Err(err_text(
                        "on = person takes no name (whoever is at the app), or a user:* key.",
                    ));
                }
            }
            coord::BLOCKED_ON_AGENT => {
                if r.is_empty() {
                    return Err(err_text("on = agent needs the agent's name or key."));
                }
                bb.reference = self.resolve_agent(&r).map_err(map_err)?;
            }
            coord::BLOCKED_ON_EXTERNAL => {}
            _ => {
                return Err(err_text(&format!(
                    "on starts with what it waits on: lease, task, person, agent or external, not {}.",
                    q(&one_line(kind, 40))
                )));
            }
        }
        Ok(Some(bb))
    }
}

/// board_block's options as coord takes them.
fn block_options(options: &[Args]) -> Vec<BlockerOption> {
    let mut opts: Vec<BlockerOption> = options
        .iter()
        .map(|o| BlockerOption {
            answer: o.str("answer").into(),
            why: o.str("why").into(),
            recommended: o.bool("recommended"),
            link: if trim(o.str("link")).is_empty() {
                None
            } else {
                Some(Link {
                    reference: o.str("link").into(),
                    ..Link::default()
                })
            },
            ..BlockerOption::default()
        })
        .collect();
    if opts.len() == 1 {
        opts[0].recommended = true; // a lone answer is the one recommended
    }
    opts
}

/// board_block with blocker_id: revises that open blocker in place.
fn revise_blocker(s: &Inner, c: &Caller, a: &Args, id: &str, bid: &str) -> String {
    if !trim(a.str("reason")).is_empty()
        || !trim(a.str("on")).is_empty()
        || !a.strs("links").is_empty()
        || !a.strs("attach").is_empty()
    {
        return err_text(
            "with blocker_id, board_block revises that blocker: give question, context and options only (reason, on, links and attach stay as they are).",
        );
    }
    let options = a.structs("options");
    for o in &options {
        if trim(o.str("link")).is_empty() {
            continue;
        }
        if let Err(e) = coord::parse_link(o.str("link")) {
            return map_err(e);
        }
    }
    let (co, as_) = match s.on(id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let (t, item) = match co.revise(
        &as_[0],
        id,
        bid,
        coord::BlockerRevision {
            question: a.str("question").into(),
            context: a.opt_str("context"),
            options: block_options(&options),
        },
    ) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let mut out = format!(
        "Revised blocker {} on {}: it asks {}",
        item.id,
        t.short_ref(),
        q(&one_line(&item.question, 200))
    );
    if item.options.is_empty() {
        out += " with no options, so the person answers in their own words.";
    } else {
        let n = item.options.len();
        out += &format!(" with {n} option{}.", if n == 1 { "" } else { "s" });
    }
    out += " It shows under the person's last reply, and the blocker waits on them again. Resolve it with board_unblock once you have what you need.";
    if many_questions(&item.question) {
        out += &format!(" Its question asks more than one thing. {ONE_ASK_EACH}");
    }
    format!("{out}\n{}", s.line(&t))
}

pub(crate) fn board_block(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, BLOCK_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    let bid = trim(a.str("blocker_id")).to_string();
    if !bid.is_empty() {
        return revise_blocker(s, c, &a, &id, &bid);
    }
    let (mut qn, reason) = (
        trim(a.str("question")).to_string(),
        trim(a.str("reason")).to_string(),
    );
    if qn.is_empty() && reason.is_empty() {
        return err_text(
            "give question: the decision or thing in the way, one line, written for someone new to the work.",
        );
    }
    if !qn.is_empty() && !reason.is_empty() && qn != reason {
        return err_text(
            "give question or reason, not both (reason is the old name for question).",
        );
    }
    let mut bb = match s.parse_on(a.str("on")) {
        Ok(v) => v,
        Err(m) => return m,
    };
    let links = a.strs("links");
    if links.len() > coord::MAX_BLOCKER_LINKS {
        return err_text(&format!(
            "links takes at most {} URLs or repo paths.",
            coord::MAX_BLOCKER_LINKS
        ));
    }
    for l in &links {
        if trim(l).is_empty() {
            continue;
        }
        if let Err(e) = coord::parse_link(l) {
            return map_err(e);
        }
    }
    let options = a.structs("options");
    for o in &options {
        if trim(o.str("link")).is_empty() {
            continue;
        }
        if let Err(e) = coord::parse_link(o.str("link")) {
            return map_err(e);
        }
    }
    let files = match s.attach_files(c, &a.strs("attach")) {
        Ok(f) => f,
        Err(m) => return m,
    };
    let mut keys = vec![c.sender.key.clone()];
    if let Some(b) = &bb
        && (b.kind == coord::BLOCKED_ON_AGENT || b.kind == coord::BLOCKED_ON_PERSON)
        && !b.reference.is_empty()
    {
        keys.push(b.reference.clone());
    }
    let (co, as_) = match s.on(&id, &keys) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    if as_.len() > 1
        && let Some(b) = bb.as_mut()
    {
        b.reference = as_[1].clone();
    }
    let by = &as_[0];
    let context = a.opt_str("context").unwrap_or_default();
    let context = context.as_str();
    let res = if qn.is_empty()
        && trim(context).is_empty()
        && options.is_empty()
        && links.is_empty()
        && files.is_empty()
    {
        // The old reason-only form: one item that blocking again replaces.
        co.block(by, &id, &reason, bb.clone()).map(|t| {
            let item = t.open_blockers().into_iter().next().unwrap_or_default();
            (t, item)
        })
    } else {
        if qn.is_empty() {
            qn = reason.clone();
        }
        let opts = block_options(&options);
        co.raise(
            by,
            &id,
            NewBlocker {
                question: qn.clone(),
                context: context.into(),
                options: opts,
                blocked_by: bb.clone(),
                links: links.clone(),
                files: files.iter().map(to_coord).collect(),
            },
        )
    };
    let (t, item) = match res {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let mut out = format!("Blocked {}", t.short_ref());
    if !item.id.is_empty() {
        out += &format!(" on blocker {}", item.id);
    }
    out += &attached_line(&files);
    if !item.links.is_empty() {
        let join = if files.is_empty() { " with" } else { " and" };
        out += &format!("{join} links {}", link_labels(&item.links));
    }
    out += ".";
    if item.options.is_empty() {
        out += " Add your recommended answer with board_recommend so whoever decides can pick it.";
    } else if !item.options.iter().any(|o| o.recommended) {
        out += " None of its options is marked recommended: board_recommend with the answer you recommend marks it.";
    }
    if many_questions(&item.question) {
        out += &format!(" Its question asks more than one thing. {ONE_ASK_EACH}");
    }
    match &bb {
        Some(b) if b.kind == coord::BLOCKED_ON_LEASE || b.kind == coord::BLOCKED_ON_TASK => {
            out += &format!(" It unblocks by itself when that {} goes away.", b.kind);
        }
        _ => out += " Anyone may board_unblock it.",
    }
    format!("{out}\n{}", s.line(&t))
}

const UNBLOCK_ARGS: &[(&str, Ty)] = &[
    ("task_id", Ty::Str),
    ("note", Ty::Str),
    ("blocker_id", Ty::Str),
    ("pick", Ty::Int),
];

pub(crate) fn board_unblock(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, UNBLOCK_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    let (co, as_) = match s.on(&id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let pick = a.int("pick");
    let note = a.str("note");
    let bid = trim(a.str("blocker_id"));
    if !bid.is_empty() {
        if pick == 0 && trim(note).is_empty() {
            return err_text(
                "to resolve one blocker give pick (the number of a recommended answer) or note (your answer).",
            );
        }
        let (t, notice) = match co.answer(
            &as_[0],
            &id,
            bid,
            BlockerAnswer {
                pick,
                text: note.into(),
                resolve: true,
                ..BlockerAnswer::default()
            },
        ) {
            Ok(v) => v,
            Err(e) => return map_err(e),
        };
        let n = s.roster();
        let mut out = format!(
            "Resolved blocker {} on {}",
            one_line(bid, 20),
            t.short_ref()
        );
        if let Some(bl) = t.blockers.iter().find(|b| b.id == bid)
            && bl.resolved.as_ref().is_some_and(|r| !r.note.is_empty())
        {
            out += " with your note"; // beside the person's answer
        }
        out += "; ";
        if t.state == coord::BLOCKED {
            out += &format!("{} still open.", t.open_blockers().len());
        } else {
            out += &format!("it is {} again.", t.state);
        }
        return format!(
            "{out}{}\n{}",
            s.told(&s.tell(&c.sender, notice), &n),
            s.card_line(&t, None, &n, &s.now())
        );
    }
    if pick != 0 {
        return err_text("pick needs blocker_id: which blocker it answers.");
    }
    let (t, notice) = match co.unblock(&as_[0], &id, trim(note)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let n = s.roster();
    format!(
        "Unblocked {}; it is {} again.{}\n{}",
        t.short_ref(),
        t.state,
        s.told(&s.tell(&c.sender, notice), &n),
        s.card_line(&t, None, &n, &s.now())
    )
}

// --- board_recommend ---

const RECOMMEND_ARGS: &[(&str, Ty)] = &[
    ("task_id", Ty::Str),
    ("blocker_id", Ty::Str),
    ("answer", Ty::Str),
    ("why", Ty::Str),
];

pub(crate) fn board_recommend(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, RECOMMEND_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    if trim(a.str("blocker_id")).is_empty() {
        return err_text("give blocker_id: the blocker's id as the board tool shows it ([b1]).");
    }
    if trim(a.str("answer")).is_empty() {
        return err_text("give answer: the answer you recommend, one line.");
    }
    let (co, as_) = match s.on(&id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let t = match co.recommend(
        &as_[0],
        &id,
        trim(a.str("blocker_id")),
        a.str("answer"),
        a.str("why"),
    ) {
        Ok(t) => t,
        Err(e) => return map_err(e),
    };
    format!(
        "Recommended on blocker {} of {}. Whoever decides sees it beside the question.\n{}",
        one_line(a.str("blocker_id"), 20),
        t.short_ref(),
        s.line(&t)
    )
}

// --- board_steps / board_step ---

const STEPS_ARGS: &[(&str, Ty)] = &[("task_id", Ty::Str), ("steps", Ty::Steps)];

pub(crate) fn board_steps(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, STEPS_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    let (co, as_) = match s.on(&id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let t = match co.set_steps(&as_[0], &id, steps_of(&a.steps("steps"))) {
        Ok(t) => t,
        Err(e) => return map_err(e),
    };
    let (d, n) = t.progress();
    let mut b = format!(
        "{} has {n} steps, {d} done. Tick them with board_step as you go.\n",
        t.short_ref()
    );
    for st in &t.steps {
        let mark = if st.done {
            "[x]"
        } else if st.doing {
            "[>]"
        } else {
            "[ ]"
        };
        let _ = writeln!(
            b,
            "- {mark} {} {}",
            one_line(&st.id, 20),
            quoted(&st.text, MAX_FIELD_RUNES)
        );
    }
    b
}

const STEP_ARGS: &[(&str, Ty)] = &[
    ("task_id", Ty::Str),
    ("step_id", Ty::Str),
    ("done", Ty::OptBool),
    ("doing", Ty::OptBool),
];

pub(crate) fn board_step(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, STEP_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    if trim(a.str("step_id")).is_empty() {
        return err_text(
            "give step_id: the step's id as board_steps or the board tool shows it (s1).",
        );
    }
    let (done, doing) = (a.opt_bool("done"), a.opt_bool("doing"));
    if done.is_none() && doing.is_none() {
        return err_text(
            "give done (true ticks it, false unticks it) or doing (true makes it the current step).",
        );
    }
    let (co, as_) = match s.on(&id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let t = match co.set_step(
        &as_[0],
        &id,
        trim(a.str("step_id")),
        StepUpdate { done, doing },
    ) {
        Ok(t) => t,
        Err(e) => return map_err(e),
    };
    let (d, n) = t.progress();
    format!("{}: steps {d}/{n} done.\n{}", t.short_ref(), s.line(&t))
}

// --- board_watch / board_unwatch ---

const TASK_ARGS: &[(&str, Ty)] = &[("task_id", Ty::Str)];

pub(crate) fn board_watch(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    board_watching(s, c, raw, true)
}

pub(crate) fn board_unwatch(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    board_watching(s, c, raw, false)
}

fn board_watching(s: &Inner, c: &Caller, raw: &[u8], on: bool) -> String {
    let a = match decode(raw, TASK_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    let (co, as_) = match s.on(&id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let r = if on {
        co.watch(&as_[0], &id)
    } else {
        co.unwatch(&as_[0], &id)
    };
    let t = match r {
        Ok(t) => t,
        Err(e) => return map_err(e),
    };
    let out = if on {
        format!(
            "You watch {} now: you hear at your next turn when its state, steps, blockers or PR links change.",
            t.short_ref()
        )
    } else {
        format!("You no longer watch {}.", t.short_ref())
    };
    format!("{out}\n{}", s.line(&t))
}

// --- board_join / board_leave / board_assign ---

pub(crate) fn board_join(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, TASK_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    let (co, _) = match s.on(&id, &[]) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let Some(cur) = co.get_task(&id) else {
        return map_err(coord::Error::new(
            coord::ErrorKind::NotFound,
            format!("task {}", one_line(&id, 80)),
        ));
    };
    let (_, as_) = match s.on(&id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let r = if cur.shared {
        co.join(&as_[0], &id, &as_[0])
    } else {
        co.claim_task(&as_[0], &id)
    };
    let t = match r {
        Ok(t) => t,
        Err(e) => return map_err(e),
    };
    let mut out = format!("You hold {} now", t.short_ref());
    if t.shared && t.holders.len() > 1 {
        out += ", with the others on it; leases keep you off each other's files";
    }
    format!(
        "{out}. Post its state with board_status as you go.\n{}",
        s.line(&t)
    )
}

pub(crate) fn board_leave(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, TASK_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    let (co, as_) = match s.on(&id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let t = match co.leave(&as_[0], &id, &as_[0]) {
        Ok(t) => t,
        Err(e) => return map_err(e),
    };
    if ask_open(&t) {
        return format!(
            "You gave the ask {} back; the device that hands it out tells the room again.",
            t.short_ref()
        );
    }
    format!(
        "You left {}; it stays on the Board.\n{}",
        t.short_ref(),
        s.line(&t)
    )
}

const ASSIGN_ARGS: &[(&str, Ty)] = &[("task_id", Ty::Str), ("to", Ty::Str)];

pub(crate) fn board_assign(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, ASSIGN_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    let want = trim(a.str("to"));
    if want.is_empty() {
        return err_text("give to: the agent's name or key (see the agents tool).");
    }
    let to = match s.assignee(want) {
        Ok(k) => k,
        Err(e) => return map_err(e),
    };
    let (co, as_) = match s.on(&id, &[c.sender.key.clone(), to.clone()]) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let (t, notice) = match co.assign(&as_[0], &id, &as_[1]) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let n = s.roster();
    format!(
        "Assigned {} to {}.{}\n{}",
        t.short_ref(),
        n.who(&to),
        s.told(&s.tell(&c.sender, notice), &n),
        s.card_line(&t, None, &n, &s.now())
    )
}

// --- board_status / board_link ---

const STATUS_ARGS: &[(&str, Ty)] = &[("task_id", Ty::Str), ("state", Ty::Str), ("note", Ty::Str)];

const BOARD_STATES: [&str; 5] = [
    coord::WORKING,
    coord::INPUT_REQUIRED,
    coord::COMPLETED,
    coord::FAILED,
    coord::CANCELED,
];

pub(crate) fn board_status(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, STATUS_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    let st = gocompat::text::to_lower_str(trim(a.str("state")));
    if st == coord::BLOCKED {
        return err_text("use board_block for blocked: it takes the reason.");
    }
    if !BOARD_STATES.contains(&st.as_str()) {
        return err_text("give state: working, input_required, completed, failed or canceled.");
    }
    if st == coord::INPUT_REQUIRED {
        return s.input_required(&id, trim(a.str("note")));
    }
    let (co, as_) = match s.on(&id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let t = match co.set_task_state(&as_[0], &id, &st, trim(a.str("note"))) {
        Ok(t) => t,
        Err(e) => return map_err(e),
    };
    let mut out = format!("{} is {} now.", t.short_ref(), t.state);
    if coord::is_terminal(&t.state) {
        out += " It folds into Done on the Board.";
    }
    format!("{out}\n{}", s.line(&t))
}

impl Inner {
    /// board_status input_required: what the person must decide goes on
    /// the card as blockers, one per question.
    fn input_required(&self, id: &str, note: &str) -> String {
        let Ok((co, _)) = self.on(id, &[]) else {
            return map_err(coord::Error::new(
                coord::ErrorKind::NotFound,
                format!("task {}", one_line(id, 80)),
            ));
        };
        let Some(t) = co.get_task(id) else {
            return map_err(coord::Error::new(
                coord::ErrorKind::NotFound,
                format!("task {}", one_line(id, 80)),
            ));
        };
        let open = t.open_blockers();
        if open.is_empty() {
            let mut msg = format!(
                "a card needs your question on it as a blocker before it waits on the person: input_required alone shows them one note with nothing to pick. {ONE_ASK_EACH}"
            );
            if many_questions(note) {
                msg = format!("your note asks more than one thing; {msg}");
            }
            return err_text(&msg);
        }
        let ids: Vec<String> = open.iter().map(|b| one_line(&b.id, 20)).collect();
        let mut out = format!(
            "{} already waits on {} open blockers ({}); they are in the person's window, each answered on its own, so its state stays {}.",
            t.short_ref(),
            open.len(),
            ids.join(", "),
            t.state
        );
        if !note.is_empty() {
            out += " The note was not saved: put anything else the person needs to decide in its own board_block, or a board_comment.";
        }
        format!("{out}\n{}", self.line(&t))
    }
}

const LINK_ARGS: &[(&str, Ty)] = &[("task_id", Ty::Str), ("url_or_path", Ty::Str)];

pub(crate) fn board_link(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, LINK_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    let link = a.str("url_or_path");
    if trim(link).is_empty() {
        return err_text("give url_or_path: an http(s) URL or a repo path.");
    }
    let (co, as_) = match s.on(&id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let t = match co.add_link(&as_[0], &id, link) {
        Ok(t) => t,
        Err(e) => return map_err(e),
    };
    format!("Linked {}.\n{}", t.short_ref(), s.line(&t))
}

// --- leases ---

const LEASE_ARGS: &[(&str, Ty)] = &[
    ("resource", Ty::Str),
    ("ttl_minutes", Ty::OptInt),
    ("steal", Ty::Bool),
];

pub(crate) fn lease(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, LEASE_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let res = trim(a.str("resource"));
    if res.is_empty() {
        return err_text("give resource: path:<repoKey>:<glob> or a name like gpu/qwen.");
    }
    let max = (coord::MAX_TTL.as_secs() / 60) as i64;
    let mut ttl = Duration::ZERO;
    if let Some(m) = a.opt_int("ttl_minutes") {
        if m < 1 || m > max {
            return err_text(&format!("ttl_minutes must be 1 to {max}, not {m}."));
        }
        ttl = Duration::from_secs(m as u64 * 60);
    }
    let l = match s
        .b
        .coord()
        .acquire(&c.sender.key, res, ttl, a.bool("steal"))
    {
        Ok(l) => l,
        Err(e) => {
            if res.starts_with("path:") {
                let rk = s.caller_repo(c);
                if !rk.is_empty() {
                    return format!("{} Your repoKey is {}.", map_err(e), one_line(&rk, 160));
                }
            }
            return map_err(e);
        }
    };
    let n = s.roster();
    let mut out = format!(
        "Leased {} until {} ({}). Lease again to renew; release when done.",
        one_line(&l.resource, MAX_FIELD_RUNES),
        l.until.format_rfc3339(),
        in_(&s.now(), &l.until)
    );
    if !l.stolen_from.is_empty() {
        out += &format!(
            " Taken from {}; consider telling them.",
            n.who(&l.stolen_from)
        );
    }
    out
}

pub(crate) fn leases(s: &Inner, _: &Caller, raw: &[u8]) -> String {
    if let Err(e) = decode(raw, &[]) {
        return e;
    }
    let ls = s.b.coord().leases();
    if ls.is_empty() {
        return "No current leases.".into();
    }
    let n = s.roster();
    let now = s.now();
    let mut b = format!("{} current leases. {}\n", ls.len(), untrusted());
    for l in &ls {
        b += &format!(
            "- {} held by {}, expires {}",
            one_line(&l.resource, MAX_FIELD_RUNES),
            n.who(&l.holder),
            in_(&now, &l.until)
        );
        if !l.stolen_from.is_empty() {
            b += &format!(", taken from {}", n.who(&l.stolen_from));
        }
        b += "\n";
    }
    b
}

const RELEASE_ARGS: &[(&str, Ty)] = &[("resource", Ty::Str)];

pub(crate) fn release(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, RELEASE_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let res = trim(a.str("resource"));
    if res.is_empty() {
        return err_text("give resource, exactly as the leases tool shows it.");
    }
    if let Err(e) = s.b.coord().release(&c.sender.key, res) {
        return map_err(e);
    }
    format!("Released {}.", one_line(res, MAX_FIELD_RUNES))
}

// --- board_split ---

const SUBTASK_FIELDS: &[(&str, Ty)] = &[
    ("title", Ty::Str),
    ("objective", Ty::Str),
    ("done_when", Ty::Str),
    ("steps", Ty::Strs),
    ("assign_to", Ty::Str),
];

const SPLIT_ARGS: &[(&str, Ty)] = &[
    ("task_id", Ty::Str),
    ("subtasks", Ty::Structs(SUBTASK_FIELDS)),
    ("adopt", Ty::Strs),
];

pub(crate) fn board_split(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, SPLIT_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    let subtasks = a.structs("subtasks");
    let adopt = a.strs("adopt");
    if subtasks.is_empty() && adopt.is_empty() {
        return err_text(
            "give subtasks (each with a title and its steps) or adopt (existing cards).",
        );
    }
    let mut subs = Vec::with_capacity(subtasks.len());
    let mut to = vec![String::new(); subtasks.len()];
    for (i, st) in subtasks.iter().enumerate() {
        if trim(st.str("title")).is_empty() {
            return err_text(&format!("subtask {} needs a title.", i + 1));
        }
        let v = trim(st.str("assign_to"));
        if !v.is_empty() {
            match s.assignee(v) {
                Ok(k) => to[i] = k,
                Err(e) => return map_err(e),
            }
        }
        subs.push(NewSubtask {
            title: trim(st.str("title")).into(),
            objective: trim(st.str("objective")).into(),
            done_criteria: trim(st.str("done_when")).into(),
            steps: st.strs("steps"),
        });
    }
    // Subtasks go in the parent's log.
    let mut keys = vec![c.sender.key.clone()];
    keys.extend(to.iter().cloned());
    let (co, as_) = match s.on(&id, &keys) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let (mut p, made) = match co.split(&as_[0], &id, subs, adopt.clone()) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let n = s.roster();
    let mut b = format!(
        "{} is a parent now; its progress and state follow its subtasks, and you keep holding it.",
        p.short_ref()
    );
    for (i, k) in made.iter().enumerate() {
        b += &format!(
            "\nCreated {} {}.",
            k.short_ref(),
            quoted(&one_line(&k.title, MAX_CARD_TITLE), MAX_CARD_TITLE)
        );
        if to.get(i).is_none_or(|t| t.is_empty()) {
            continue;
        }
        match co.assign(&as_[0], &k.id, &as_[1 + i]) {
            Err(e) => {
                b += &format!(
                    " Assigning it failed: {}",
                    map_err(e).trim_start_matches("Error: ")
                );
            }
            Ok((_, notice)) => {
                b += &format!(
                    " Assigned to {}.{}",
                    n.who(&to[i]),
                    s.told(&s.tell(&c.sender, notice), &n)
                );
            }
        }
    }
    if !adopt.is_empty() {
        let refs: Vec<String> = adopt.iter().map(|r| one_line(r, 40)).collect();
        b += &format!("\nAdopted {}.", refs.join(", "));
    }
    if let Some(t) = co.get_task(&p.id) {
        p = t;
    }
    b += &format!("\n{}", s.card_line(&p, None, &n, &s.now()));
    b
}
