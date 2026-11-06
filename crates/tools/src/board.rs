//! The Board tools (I:internal/tools/board.go, route.go): a scratch pad for
//! live coordination, never a tracker. Every card line carries its ref,
//! and every call that someone should hear about routes coord's Notice
//! from the caller.

use std::fmt::Write;
use std::sync::Arc;
use std::time::Duration;

use coord::{
    BlockedBy, Blocker, BlockerAnswer, BlockerOption, BoardFilter, BoardView, Card, Comment,
    CommentInput, Coord, Link, NewBlocker, NewSubtask, NewTask, Step, StepUpdate, Task, TaskEdit,
};
use gocompat::time::GoTime;

use crate::args::{Args, StepArg, Ty};
use crate::attachments::{attached_line, describe_coord, to_coord};
use crate::format::{MAX_FIELD_RUNES, ago, in_, one_line, q, quoted, trim};
use crate::later::Order;
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
            v.later.extend(o.later);
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
        self.card_text_in(t, last, n, now, brief, None)
    }

    /// The order among the open cards of card `t`'s log, for one card's
    /// lines.
    fn order_near(&self, t: &Task) -> Option<Order> {
        if coord::is_terminal(&t.state) {
            return None;
        }
        let (co, _) = self.on(&t.id, &[]).ok()?;
        let all = co.list_tasks(&coord::TaskFilter {
            open: true,
            ..coord::TaskFilter::default()
        });
        Some(Order::of(&all))
    }

    /// [`Self::card_text`] with the order of the cards around it, found
    /// once for a whole list (`None`: from this card's log).
    pub(crate) fn card_text_in(
        &self,
        t: &Task,
        last: Option<&Comment>,
        n: &Names<'_>,
        now: &GoTime,
        brief: bool,
        order: Option<&Order>,
    ) -> String {
        let mut b = String::new();
        let mut state = coord::effective_state(t).to_string();
        let held = t.parked().is_some();
        if held && state == coord::BLOCKED {
            // Card #570: put aside, not stuck.
            state = "not yet".into();
        } else if state == coord::BLOCKED && t.all_answered() {
            // The person answered every open item (card #204): its holder's turn.
            state = "working, answered".into();
        }
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
        // Its work/personal side (card #494, #496).
        if !t.side.is_empty() {
            b += &format!("; {} side", one_line(&t.side, 40));
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
                    w += " (offered, not taken up yet)";
                }
                w
            })
            .collect();
        if !holders.is_empty() {
            b += &format!("; held by {}", holders.join(", "));
        } else {
            b += "; nobody holds it";
        }
        // Step owners are co-holders while their steps are open (card #86).
        let co: Vec<String> = t
            .co_holders
            .iter()
            .map(|k| {
                let ids: Vec<&str> = t
                    .steps
                    .iter()
                    .filter(|st| !st.done && st.owner == *k)
                    .map(|st| st.id.as_str())
                    .collect();
                format!("{} ({})", n.who(k), ids.join(", "))
            })
            .collect();
        if !co.is_empty() {
            b += &format!("; with {} on steps", co.join(", "));
        }
        if t.shared {
            b += "; shared";
        }
        // A card on a side its device doesn't run waits (card #495).
        if !t.side_wait.is_empty() {
            b += &format!("; waits: {}", one_line(&t.side_wait, 80));
        }
        if !t.links.is_empty() {
            let ls: Vec<String> = t.links.iter().map(|l| one_line(&l.label, 60)).collect();
            b += &format!("; links: {}", ls.join(", "));
        }
        if t.comments > 0 {
            b += &format!("; {} comments", t.comments);
        }
        // Gone watchers are left out (card #485).
        let co = self.on(&t.id, &[]).ok().map(|(co, _)| co);
        let ws: Vec<String> = t
            .watchers
            .iter()
            .filter(|w| !co.as_ref().is_some_and(|co| co.key_gone(w)))
            .map(|w| n.who(w))
            .collect();
        if !ws.is_empty() {
            b += &format!("; watched by {}", ws.join(", "));
        }
        // Waits (card #87): who waits on this card, and what its holders
        // wait on.
        if let Ok((co, _)) = self.on(&t.id, &[]) {
            let all = co.waits("");
            let on_it: Vec<String> = all
                .iter()
                .filter(|w| w.task_id == t.id)
                .map(|w| format!("{} (until {})", n.who(&w.by), coord::until_text(&w.until)))
                .collect();
            if !on_it.is_empty() {
                b += &format!("; waited on by {}", on_it.join(", "));
            }
            if !coord::is_terminal(&t.state) {
                let theirs: Vec<String> = all
                    .iter()
                    .filter(|w| t.holders.contains(&w.by))
                    .map(|w| {
                        let r = if w.task_id.is_empty() {
                            String::new()
                        } else {
                            self.short_of(&w.task_id)
                        };
                        one_line(&coord::wait_line(w, &r), 120)
                    })
                    .collect();
                if !theirs.is_empty() {
                    b += &format!("; waiting on {}", theirs.join(", "));
                }
            }
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
        // Where the work is and where it left off (card #137).
        if !coord::is_terminal(&t.state) {
            // Only a git place helps an agent pick the work up.
            if let Some(w) = t.where_.as_ref().filter(|w| !w.worktree.is_empty()) {
                b += &format!("  where: {}\n", one_line(&w.line(), 200));
            }
            if let Some(l) = &t.left_off {
                b += &format!("  left off: {}\n", quoted(&one_line(&l.text, 200), 200));
            }
        }
        if !coord::is_terminal(&t.state) {
            let own;
            let order = match order {
                Some(o) => Some(o),
                None => {
                    own = self.order_near(t);
                    own.as_ref()
                }
            };
            if let Some(l) = order.and_then(|o| o.line(self, t)) {
                b += &l;
            }
        }
        if t.state == coord::BLOCKED {
            // Card #570: a Not yet item says what brings the card back.
            for bl in t.open_blockers().iter().filter(|b| !b.later.is_empty()) {
                b += &self.not_yet_line(bl);
            }
            let mut open: Vec<Blocker> = t
                .open_blockers()
                .into_iter()
                .filter(|b| b.later.is_empty())
                .collect();
            if open.is_empty() && !held {
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
                if bl.options_aside && bl.done_check {
                    b += "    Not done: the person said why; answer them in the thread (board_comment blocker_id), then set completed again with proof that says what changed\n";
                } else if bl.options_aside {
                    b += "    options set aside: the person answered in their own words; answer in the thread, then revise with board_block blocker_id\n";
                }
                let opts = if bl.options_aside { &[][..] } else { opts };
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
                if !bl.options_aside && opts.len() < bl.options.len() {
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

    /// Writes where `key`'s work on card t is (card #137) right away, when
    /// it holds the card: its latest working directory (capture's, else
    /// its session's) as a git place. The Core's sweep keeps it current.
    pub(crate) fn note_where(&self, key: &str, t: &Task) {
        if t.holder != key || t.shared || coord::is_terminal(&t.state) {
            return;
        }
        let dir = self
            .b
            .capture()
            .and_then(|c| c.session_dir(key))
            .or_else(|| {
                self.b
                    .agents()
                    .into_iter()
                    .find(|a| a.key == key)
                    .map(|a| a.cwd)
            })
            .unwrap_or_default();
        // Only a git place is worth a row: a folder outside git says
        // nothing about where the work is.
        let Some(g) = capture::gitlink::place(&dir) else {
            return;
        };
        let w = coord::Where {
            who: key.to_string(),
            cwd: dir,
            worktree: g.worktree,
            branch: g.branch,
            head: g.head,
            pushed: g.pushed,
            ..coord::Where::default()
        };
        if let Ok((co, _)) = self.on(&t.id, &[]) {
            let _ = co.set_where(&t.id, w);
        }
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
    let mut repo = coord::canonical_repo_key(trim(a.str("repo")));
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
        + v.open.len()
        + v.later.len();
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
    // Card #496: which side this device runs; other sides' cards are read.
    if !v.side_line.is_empty() {
        let _ = writeln!(b, "{}", v.side_line);
    }
    // Card #436: a red main holds every landing; it goes first.
    for r in coord::main_reds(&v) {
        let who = if r.task.holders.is_empty() {
            "nobody holds it yet".to_string()
        } else {
            format!("{} is on it", n.who(&r.task.holders[0]))
        };
        let _ = writeln!(
            b,
            "Main is red: {} fails on clean main; {} {} on {} ({who}). Hit it too? board_block on = main-red {} waits on that card instead of fixing it again.",
            one_line(&r.test, 80),
            r.waiting,
            if r.waiting == 1 {
                "card waits"
            } else {
                "cards wait"
            },
            r.task.short_ref(),
            one_line(&r.test, 80),
        );
    }
    const HINT_ROOM: usize = 200;
    let budget = MAX_RESULT - HINT_ROOM;
    let mut cut = 0;
    // Card #570: where each card sits in its order, found once.
    let order = Order::of(
        [
            &asks,
            &took,
            &v.needs_you,
            &v.blocked,
            &v.working,
            &v.open,
            &v.later,
        ]
        .into_iter()
        .flatten()
        .map(|cd| &cd.task),
    );
    let group = |b: &mut String, name: &str, cards: &[Card], cut: &mut usize| {
        if cards.is_empty() {
            return;
        }
        let _ = write!(b, "\n{name} ({}):\n", cards.len());
        for cd in cards {
            let mut line = s.card_text_in(
                &cd.task,
                cd.last_comment.as_ref(),
                &n,
                &now,
                true,
                Some(&order),
            );
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
            // Items whose agent is gone on a card nobody holds (card #485).
            if !cd.stranded.is_empty() {
                let n = s.roster();
                let items: Vec<String> = cd
                    .stranded
                    .iter()
                    .map(|s| {
                        let who = n.who(&s.agent);
                        if s.waited_on {
                            format!("{} waits on {who}, gone", s.blocker_id)
                        } else {
                            format!("{} was raised by {who}, gone", s.blocker_id)
                        }
                    })
                    .collect();
                line += &format!(
                    "  {}; nobody holds it, so nothing moves it: the person can hand it to an agent, or board_join it and pick up from its thread\n",
                    one_line(&items.join("; "), MAX_FIELD_RUNES)
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
        "Asks you hold (ask_close each with what it became; board_leave passes one on)",
        &took,
        &mut cut,
    );
    group(&mut b, "Needs you", &v.needs_you, &mut cut);
    group(&mut b, "Blocked", &v.blocked, &mut cut);
    // Card #191: cards waiting on their review come out of Working.
    let (reviewing, working): (Vec<Card>, Vec<Card>) = std::mem::take(&mut v.working)
        .into_iter()
        .partition(|cd| coord::effective_state(&cd.task) == coord::IN_REVIEW);
    group(&mut b, "Working", &working, &mut cut);
    group(
        &mut b,
        "In review (the work waits on its PR or review; the holder is free for other work)",
        &reviewing,
        &mut cut,
    );
    group(&mut b, "Open", &v.open, &mut cut);
    // Card #570: put aside until something brings them back.
    group(
        &mut b,
        "Not yet (put aside: don't start or join these; each comes back by itself, or when the person brings it back)",
        &v.later,
        &mut cut,
    );
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
                // The person's threads aren't work (card #116): agents hear
                // them through notices and read them with show.
                Some(a) if a.thread => {}
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
    ("board", Ty::Str),
    ("shared", Ty::Bool),
    ("links", Ty::Strs),
    ("assign_to", Ty::Str),
    ("steps", Ty::Steps),
    ("parent", Ty::Str),
    ("from", Ty::Str),
    ("waits_on", Ty::Str),
    ("after", Ty::Strs),
    ("later", Ty::Struct(LATER_FIELDS)),
];

/// board_create's later: what brings the card back, and why it waits.
const LATER_FIELDS: &[(&str, Ty)] = &[("until", Ty::Str), ("why", Ty::Str)];

/// board_create's after and later (card #570): the cards it comes after,
/// and what brings it back when it is Not yet from the start, with why.
pub(crate) fn order_of(
    s: &Inner,
    a: &Args,
) -> Result<(Vec<String>, Option<(coord::Back, String)>), String> {
    let after: Vec<String> = a
        .strs("after")
        .iter()
        .map(|r| trim(r).to_string())
        .filter(|r| !r.is_empty())
        .collect();
    let later = match a.object("later") {
        None => None,
        Some(l) => Some((s.back_of(l.str("until"))?, trim(l.str("why")).to_string())),
    };
    Ok((after, later))
}

/// What board_create says about a card that waits from the start (card
/// #570), "" when it doesn't.
fn waits_from_start(s: &Inner, t: &Task) -> String {
    if let Some(b) = t.parked() {
        return format!(
            " It is Not yet: it {}. Nobody starts it until then.",
            s.back_text(b)
        );
    }
    let af = crate::later::afters(t);
    if af.is_empty() {
        return String::new();
    }
    let names: Vec<String> = af.iter().map(|id| s.short_of(id)).collect();
    format!(
        " It comes after {}: Irori hands it out once {} done.",
        names.join(", "),
        if af.len() == 1 { "that is" } else { "they are" }
    )
}

/// board_create's waits_on (card #400): a task (#12, or task #12) or a
/// lease (lease <resource>) the new card waits on, as the blocker it is
/// created with. The card starts blocked, so Irori hands it out only once
/// that work is done.
pub(crate) fn waits_on(s: &Inner, v: &str) -> Result<Option<(String, BlockedBy)>, String> {
    let v = trim(v);
    if v.is_empty() {
        return Ok(None);
    }
    let on = if v.starts_with('#') || v.starts_with("task:") && !v.starts_with("task: ") {
        format!("task {v}")
    } else {
        v.to_string()
    };
    let Some(bb) = s.parse_on(&on)? else {
        return Ok(None);
    };
    let reason = match bb.kind.as_str() {
        coord::BLOCKED_ON_TASK => format!("Waits on {}", s.short_of(&bb.reference)),
        coord::BLOCKED_ON_LEASE => format!("Waits on the lease {}", bb.reference),
        _ => {
            return Err(err_text(
                "waits_on takes a card (#12) or a lease (lease <resource>); for a question, create the card and raise it with board_block.",
            ));
        }
    };
    Ok(Some((reason, bb)))
}

/// Step arguments as coord's steps.
pub(crate) fn steps_of(v: &[StepArg]) -> Vec<Step> {
    v.iter()
        .map(|st| Step {
            id: st.id.clone(),
            text: st.text.clone(),
            done: st.done,
            doing: st.doing,
            owner: st.owner.clone(),
            spawn: st.spawn,
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
    let mut repo = coord::canonical_repo_key(trim(a.str("repo")));
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
    let waits = match waits_on(s, a.str("waits_on")) {
        Ok(w) => w,
        Err(m) => return m,
    };
    let (after, later) = match order_of(s, &a) {
        Ok(v) => v,
        Err(m) => return m,
    };
    let ordered = !after.is_empty() || later.is_some();
    let board = trim(a.str("board"));
    if !board.is_empty() {
        return crate::shared::create_on_board(s, c, &a, board, repo, to, waits, after, later);
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
            from: trim(a.str("from")).into(),
            block_reason: match &later {
                Some((_, why)) => why.clone(),
                None => waits.as_ref().map(|w| w.0.clone()).unwrap_or_default(),
            },
            blocked_by: waits.as_ref().map(|w| w.1.clone()),
            after,
            later: later.map(|l| l.0),
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
    if !t.from.is_empty() {
        out += &format!(
            " Made from the thread {}: the thread shows it.",
            co.get_task(&t.from)
                .map(|th| th.short_ref())
                .unwrap_or_else(|| format!("task:{}", t.from))
        );
    }
    if t.steps.is_empty() {
        out += " Give it steps with board_steps once you know them, so the person sees how far along it is.";
    } else {
        out += " Tick its steps with board_step as you go.";
    }
    let mut t = t;
    let waits_now = if ordered {
        waits_from_start(s, &t)
    } else {
        String::new()
    };
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
    } else if !waits_now.is_empty() {
        // Card #570: Not yet, or after other cards: nobody is handed it
        // until it is free.
        out += &waits_now;
    } else if let Some((reason, _)) = waits.as_ref().filter(|_| private && !t.shared) {
        // Blocked from the start (card #400): nobody is handed it until the
        // work it waits on is done; then it goes out like a new card.
        out += &format!(
            " It starts blocked ({reason}): Irori hands it to a free agent once that is done."
        );
    } else if private && !t.shared {
        // Pull, not push (#82): Irori hands it to one free agent at a time;
        // the poster doesn't pick.
        match co.offer_card(&keys[0], &t.id, &s.b.room_agents()) {
            Ok(offer) if !offer.to.is_empty() => {
                let to = offer.to[0].clone();
                out += &format!(
                    " Irori handed it to {}, the free agent idle longest; if it can't take it on, it goes to the next.{}",
                    n.who(&to),
                    // From Irori, not the poster: it hands the card out,
                    // and the poster's messages to that agent aren't spent.
                    s.told(&s.tell(&inbox::system_sender(), offer), &n)
                );
                if let Some(nt) = co.get_task(&t.id) {
                    t = nt;
                }
            }
            // A card with no repo has no checkout to start in (card #467:
            // v0.11.0 looped on one), so say so instead of promising one.
            Ok(_) if co.spawning() && t.repo_key.is_empty() && t.where_.is_none() => {
                out += " It names no repo, so Irori has no checkout to start an agent in: take it up with board_join, or post it again with repo set.";
            }
            Ok(_) if co.spawning() => {
                out += " Irori starts an agent for it.";
            }
            Ok(_) => {
                out += " No agent takes offered cards now: it waits for one the person offers cards to. Take it yourself with board_join only if you hold no card in progress; board_assign it when one agent is clearly the right one.";
            }
            Err(_) => {
                out += " Take it up with board_join, or hand it over with board_assign.";
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
    ("mention", Ty::Strs),
    ("verdict", Ty::Str),
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
    let verdict = gocompat::text::to_lower_str(trim(a.str("verdict")));
    if !verdict.is_empty() && !bid.is_empty() {
        return err_text("a verdict is its own comment: give verdict or blocker_id, not both.");
    }
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
    let res = s.resolve_mentions(c, &id, a.str("text"), &a.strs("mention"));
    let (logm, shown) = s.mentions_on(&id, &res.mentions);
    let (cm, mut notices) = match co.comment_on(
        &as_[0],
        &id,
        CommentInput {
            text: trim(a.str("text")).into(),
            wake: a.bool("wake"),
            files: files.iter().map(to_coord).collect(),
            blocker_id: bid.clone(),
            missing,
            mentions: logm,
            verdict: verdict.clone(),
        },
    ) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let notice = if notices.is_empty() {
        coord::Notice::default()
    } else {
        notices.remove(0)
    };
    let (_, mres) = s.tell_all(c, notices);
    let mline = crate::mention::mention_line(
        &crate::mention::outcomes(&shown, &mres),
        &res.misses,
        &s.roster(),
    );
    if !bid.is_empty() {
        // A reply on a blocker tells nobody: the person is the audience.
        let b = one_line(&bid, 20);
        // A done check (card #476): the reply is the agent's answer to a
        // Not done; completed with new proof replaces the check, never
        // board_unblock or board_block.
        if s.task(&id)
            .is_some_and(|t| t.blockers.iter().any(|x| x.id == bid && x.done_check))
        {
            return format!(
                "Answered the person on done check {b} of {}{}. When the work is done, set completed again with proof that says what changed since they sent it back; that check replaces this one.{mline}",
                s.short_of(&id),
                attached_line(&files)
            );
        }
        // After the person's answer, a reply without missing acts on it
        // (card #204): the blocker waits on the agent, which resolves it.
        let acted = s.task(&id).is_some_and(|t| {
            t.blockers
                .iter()
                .any(|x| x.id == bid && x.open() && x.status == coord::BLOCKER_ANSWERED)
        });
        if acted {
            return format!(
                "Replied on blocker {b} of {}{}; the person answered it, so it now waits on you, not them. Resolve it now with board_unblock blocker_id={b} note=… (what you'll do), in this turn. If your reply answered their question and the choices are not what they were, revise it now with board_block blocker_id={b} (the question and options that fit, or none); don't leave the same options up. To ask them something back, reply with missing.{mline}",
                s.short_of(&id),
                attached_line(&files)
            );
        }
        return format!(
            "Replied on blocker {b} of {}{}; it stays open and goes back to the person. Resolve it with board_unblock blocker_id={b} note=… once you have what you need. If your reply answered their question and the choices are not what they were, revise it now with board_block blocker_id={b} (the question and options that fit, or none); don't leave the same options up.{mline}",
            s.short_of(&id),
            attached_line(&files)
        );
    }
    let n = s.roster();
    let verdict_note = match verdict.as_str() {
        "" => "",
        coord::VERDICT_CHANGES => " (verdict: changes asked)",
        _ => " (verdict: approved)",
    };
    let mut out = format!(
        "Commented comment:{} on {}{verdict_note}{}.",
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
    } else if shown.iter().all(|m| m.gone) {
        out += " Nobody else holds it, so nobody was told; it shows on the Board.";
    }
    out + &mline
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
    ("mention", Ty::Strs),
    ("for", Ty::Str),
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
/// The word in a card's done-when that leaves a choice to the person
/// (card #354): "Stephen picks what becomes cards". A verb after can,
/// lets or able ("the person can pick a theme") is a feature, not a choice.
fn choice_left(done_when: &str) -> Option<&'static str> {
    const VERBS: [(&str, &str); 12] = [
        ("pick", "pick"),
        ("picks", "pick"),
        ("choose", "choose"),
        ("chooses", "choose"),
        ("decide", "decide"),
        ("decides", "decide"),
        ("select", "select"),
        ("selects", "select"),
        ("approve", "approve"),
        ("approves", "approve"),
        ("prioritise", "prioritise"),
        ("prioritize", "prioritize"),
    ];
    const FEATURE: [&str; 7] = ["can", "could", "let", "lets", "allow", "allows", "able"];
    let words: Vec<String> = done_when
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    words.iter().enumerate().find_map(|(i, w)| {
        let verb = VERBS.iter().find(|(v, _)| v == w)?.1;
        let feature = words[i.saturating_sub(3)..i]
            .iter()
            .any(|p| FEATURE.contains(&p.as_str()));
        (!feature).then_some(verb)
    })
}

fn many_questions(text: &str) -> bool {
    text.matches('?').count() > 1
}

const ONE_ASK_EACH: &str = "Raise each question with its own board_block (question, context for a newcomer, options when there are real choices, and recommended: true on the one you recommend), so the person can answer them one by one.";

impl Inner {
    /// board_block's for (card #264): the person who decides a question on
    /// shared card `id`, as that board writes them (person:<id>). It takes
    /// another member's name as the board shows it, or their person: key;
    /// this person (a user: key of theirs) is `None`, the default.
    fn decider(&self, id: &str, v: &str) -> Result<Option<(String, String)>, String> {
        let v = trim(v);
        if v.is_empty() || self.is_owner(v) {
            return Ok(None);
        }
        let board = self.b.shared_boards().and_then(|sb| {
            let b = sb.board_of(id)?;
            Some((b.clone(), sb.board_mentionables(&b.board_id)))
        });
        let Some((b, cards)) = board else {
            return Err(err_text(
                "for names who decides on a shared board's card; a private card's questions are for your person, so leave for out.",
            ));
        };
        let people: Vec<_> = cards
            .iter()
            .filter(|c| c.key.starts_with(store::kinds::PERSON_ACTOR_PREFIX))
            .collect();
        if let Some(p) = people.iter().find(|p| p.key == v) {
            return Ok(Some((p.key.clone(), p.name.clone())));
        }
        let named: Vec<_> = people
            .iter()
            .filter(|p| p.name.eq_ignore_ascii_case(v))
            .collect();
        match named.as_slice() {
            [one] => Ok(Some((one.key.clone(), one.name.clone()))),
            [] => {
                let names: Vec<String> = people.iter().map(|p| q(&one_line(&p.name, 40))).collect();
                Err(err_text(&format!(
                    "for: nobody on {} is named {}; {}. Leave for out when the question is for the card's owner.",
                    one_line(&b.name, 60),
                    q(&one_line(v, 40)),
                    if names.is_empty() {
                        "it has no other members".to_string()
                    } else {
                        format!("its other members are {}", names.join(", "))
                    }
                )))
            }
            _ => Err(err_text(&format!(
                "for: more than one member of {} is named {}; give their person: key.",
                one_line(&b.name, 60),
                q(&one_line(v, 40))
            ))),
        }
    }

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
            // A least app version on named devices (card #485).
            coord::BLOCKED_ON_VERSION => {
                bb.reference = self.version_wait(&r)?;
            }
            // Main is red (card #436): board_block turns it into a wait on
            // the one card for that test.
            MAIN_RED => {
                coord::parse_main_red(&r).map_err(map_err)?;
            }
            _ => {
                return Err(err_text(&format!(
                    "on starts with what it waits on: lease, task, person, agent, external, version or main-red, not {}.",
                    q(&one_line(kind, 40))
                )));
            }
        }
        Ok(Some(bb))
    }

    /// on = version's ref (card #485): `<min> [device …]`, each device by
    /// its name (MacBook, DESKTOP-STEPHEN; any case, or the start of one
    /// name) or id, turned into `<min> <id> …`. No device leaves it to the
    /// coord: this device.
    pub(crate) fn version_wait(&self, r: &str) -> Result<String, String> {
        let mut it = r.split_whitespace();
        let Some(min) = it.next() else {
            return Err(err_text(
                "on = version needs the least app version, then the devices by name: version 0.11.0 MacBook (none: this device).",
            ));
        };
        if !coord::valid_version(min) {
            return Err(err_text(&format!(
                "on = version: {} is not a version; give X.Y.Z, e.g. version 0.11.0 MacBook.",
                q(&one_line(min, 40))
            )));
        }
        let labels = self.b.device_labels();
        let listed = || {
            labels
                .iter()
                .map(|(id, l)| {
                    if l.is_empty() {
                        id.clone()
                    } else {
                        format!("{l} ({})", short_id(id))
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut out = vec![min.strip_prefix('v').unwrap_or(min).to_string()];
        for name in it {
            let low = gocompat::text::to_lower_str(name);
            let exact: Vec<&String> = labels
                .iter()
                .filter(|(id, l)| **id == name || gocompat::text::to_lower_str(l) == low)
                .map(|(id, _)| id)
                .collect();
            let found = if exact.len() == 1 {
                exact
            } else {
                labels
                    .iter()
                    .filter(|(id, l)| {
                        id.starts_with(name)
                            || !low.is_empty() && gocompat::text::to_lower_str(l).contains(&low)
                    })
                    .map(|(id, _)| id)
                    .collect()
            };
            match found.as_slice() {
                [id] => {
                    if !out.contains(id) {
                        out.push((*id).clone());
                    }
                }
                [] => {
                    return Err(err_text(&format!(
                        "on = version: no device is called {}; the person's devices are {}.",
                        q(&one_line(name, 40)),
                        listed()
                    )));
                }
                _ => {
                    return Err(err_text(&format!(
                        "on = version: more than one device matches {}; the person's devices are {}. Give its full name or id.",
                        q(&one_line(name, 40)),
                        listed()
                    )));
                }
            }
        }
        Ok(out.join(" "))
    }
}

/// The first 8 characters of a device id, as lines print it.
fn short_id(id: &str) -> &str {
    &id[..id.len().min(8)]
}

/// The `on` kind for a test that fails on clean main (card #436).
const MAIN_RED: &str = "main-red";

/// What board_block says about a main-red wait: the card it joined, or the
/// card it made and who Irori handed it to.
fn main_red_line(s: &Inner, c: &Caller, co: &coord::Coord, rt: &Task, made: bool) -> String {
    let n = s.roster();
    if !made {
        let who = if rt.holders.is_empty() {
            "nobody holds it yet".to_string()
        } else {
            format!("{} is on it", n.who(&rt.holders[0]))
        };
        return format!(
            " Main is already red for that test: it waits on {} ({who}) instead of fixing it here, and wakes when that card completes.",
            rt.short_ref()
        );
    }
    let mut out = format!(
        " Main is red: Irori made {} for that test, and this card waits on it, waking when it completes.",
        rt.short_ref()
    );
    match co.offer_card(&c.sender.key, &rt.id, &s.b.room_agents()) {
        Ok(offer) if !offer.to.is_empty() => {
            let to = offer.to[0].clone();
            out += &format!(
                " Irori handed it to {}, the free agent idle longest.{}",
                n.who(&to),
                s.told(&s.tell(&inbox::system_sender(), offer), &n)
            );
        }
        _ if co.spawning() => out += " Irori starts an agent for it.",
        _ => {
            out += " No agent takes offered cards now: it waits for one the person offers cards to."
        }
    }
    out
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
        || !trim(a.str("for")).is_empty()
        || !a.strs("links").is_empty()
        || !a.strs("attach").is_empty()
    {
        return err_text(
            "with blocker_id, board_block revises that blocker: give question, context and options only (reason, on, for, links and attach stay as they are).",
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
    out += &s.mention_only(c, id, &item.question, a);
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
    let mut for_name = String::new();
    match s.decider(&id, a.str("for")) {
        Err(m) => return m,
        Ok(None) => {}
        Ok(Some((who, name))) => match bb.as_mut() {
            None => {
                for_name = name;
                bb = Some(BlockedBy {
                    kind: coord::BLOCKED_ON_PERSON.into(),
                    reference: who,
                })
            }
            Some(b) if b.kind == coord::BLOCKED_ON_PERSON && b.reference.is_empty() => {
                for_name = name;
                b.reference = who
            }
            Some(_) => {
                return err_text(
                    "for names the person who decides; it goes with on = person (or no on), not with a lease, task, agent or external wait.",
                );
            }
        },
    }
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
    // Main is red (card #436): one card per failing test; this card waits
    // on it like any task wait, and a card made now is handed out below.
    let mut red: Option<(Task, bool)> = None;
    if let Some(b) = bb.as_mut().filter(|b| b.kind == MAIN_RED) {
        let (test, commit) = match coord::parse_main_red(&b.reference) {
            Ok(v) => v,
            Err(e) => return map_err(e),
        };
        match co.main_red(by, &id, &test, &commit) {
            Ok((rt, made)) => {
                b.kind = coord::BLOCKED_ON_TASK.into();
                b.reference = rt.id.clone();
                red = Some((rt, made));
            }
            Err(e) => return map_err(e),
        }
    }
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
    if !for_name.is_empty() {
        out += &format!(
            ", a question for {}: it shows under their Needs a decision, and as waiting on them for everyone else (anyone can still answer)",
            one_line(&for_name, 60)
        );
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
    if let Some((rt, made)) = &red {
        out += &main_red_line(s, c, &co, rt, *made);
    }
    // What it waits on as stored: an external wait that names leases held
    // now became a wait on those leases (card #339).
    let stored = item
        .on
        .clone()
        .or_else(|| t.blocked_by.clone())
        .or(bb.clone());
    match &stored {
        Some(b)
            if b.kind == coord::BLOCKED_ON_LEASE
                && bb.as_ref().is_some_and(|i| i.kind != b.kind) =>
        {
            out += &format!(
                " It names leases held now, so it waits on them ({}) and unblocks by itself, telling you, when the last is released.",
                one_line(&b.reference, 200)
            );
        }
        Some(b) if b.kind == coord::BLOCKED_ON_LEASE || b.kind == coord::BLOCKED_ON_TASK => {
            out += &format!(" It unblocks by itself when that {} goes away.", b.kind);
        }
        _ => out += " Anyone may board_unblock it.",
    }
    out += &s.mention_only(c, &id, &item.question, &a);
    format!("{out}\n{}", s.line(&t))
}

const UNBLOCK_ARGS: &[(&str, Ty)] = &[
    ("task_id", Ty::Str),
    ("note", Ty::Str),
    ("blocker_id", Ty::Str),
    ("pick", Ty::Int),
    ("mention", Ty::Strs),
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
        let res = s.resolve_mentions(c, &id, note, &a.strs("mention"));
        let (logm, shown) = s.mentions_on(&id, &res.mentions);
        let (t, notices) = match co.answer(
            &as_[0],
            &id,
            bid,
            BlockerAnswer {
                pick,
                text: note.into(),
                resolve: true,
                mentions: logm,
                ..BlockerAnswer::default()
            },
        ) {
            Ok(v) => v,
            Err(e) => return map_err(e),
        };
        let (told, mres) = s.tell_all(c, notices);
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
        let mline =
            crate::mention::mention_line(&crate::mention::outcomes(&shown, &mres), &res.misses, &n);
        return format!(
            "{out}{}{mline}\n{}",
            s.told(&told, &n),
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
    let (told, mline) = s.mention_with(c, &id, note, &a, Some(notice));
    let n = s.roster();
    format!(
        "Unblocked {}; it is {} again.{}{mline}\n{}",
        t.short_ref(),
        t.state,
        s.told(&told, &n),
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
    // Owners by name or key, as the board's coord knows them (card #86).
    let mut steps = steps_of(&a.steps("steps"));
    let asked: Vec<String> = steps
        .iter()
        .filter(|st| st.spawn)
        .map(|st| trim(&st.text).to_string())
        .collect();
    let mut keys = vec![c.sender.key.clone()];
    for st in &steps {
        let o = trim(&st.owner);
        if o.is_empty() {
            continue;
        }
        match s.assignee(o) {
            Ok(k) => keys.push(k),
            Err(e) => return map_err(e),
        }
    }
    let (co, as_) = match s.on(&id, &keys) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let mut owners = as_[1..].iter();
    for st in steps.iter_mut() {
        if !trim(&st.owner).is_empty() {
            st.owner = owners.next().cloned().unwrap_or_default();
        }
    }
    let (t, notices) = match co.set_steps_notice(&as_[0], &id, steps) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let names = s.roster();
    let mut told = String::new();
    for nt in notices {
        told.push_str(&s.told(&s.tell(&c.sender, nt), &names));
    }
    let (d, n) = t.progress();
    let mut b = format!(
        "{} has {n} steps, {d} done. Tick them with board_step as you go.{told}\n",
        t.short_ref()
    );
    if asked
        .iter()
        .any(|x| !t.steps.iter().any(|st| st.spawn && st.text == *x))
    {
        b.push_str(SPAWN_REFUSED_TOOL);
    }
    for st in &t.steps {
        let mark = if st.done {
            "[x]"
        } else if st.doing {
            "[>]"
        } else {
            "[ ]"
        };
        let mut owner = if st.owner.is_empty() {
            String::new()
        } else {
            format!(" (owner: {})", names.who(&st.owner))
        };
        if st.spawn {
            owner.push_str(" (spawn)");
        }
        let _ = writeln!(
            b,
            "- {mark} {} {}{owner}",
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
    ("owner", Ty::OptStr),
    ("left_off", Ty::Str),
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
    let owner = a.opt_str("owner").map(|o| trim(&o).to_string());
    if done.is_none() && doing.is_none() && owner.is_none() {
        return err_text(
            "give done (true ticks it, false unticks it), doing (true makes it the current step) or owner (the agent doing it; empty takes it off them).",
        );
    }
    let mut keys = vec![c.sender.key.clone()];
    if let Some(o) = owner.as_deref().filter(|o| !o.is_empty()) {
        match s.assignee(o) {
            Ok(k) => keys.push(k),
            Err(e) => return map_err(e),
        }
    }
    let (co, as_) = match s.on(&id, &keys) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let sid = trim(a.str("step_id"));
    let names = s.roster();
    let mut told = String::new();
    let mut t = None;
    if let Some(o) = &owner {
        let key = if o.is_empty() { "" } else { as_[1].as_str() };
        match co.set_step_owner(&as_[0], &id, sid, key) {
            Ok((got, ns)) => {
                for nt in ns {
                    told.push_str(&s.told(&s.tell(&c.sender, nt), &names));
                }
                t = Some(got);
            }
            Err(e) => return map_err(e),
        }
    }
    let left_off = trim(a.str("left_off"));
    if done.is_some() || doing.is_some() {
        match co.set_step_left_off(&as_[0], &id, sid, StepUpdate { done, doing }, left_off) {
            Ok(got) => t = Some(got),
            Err(e) => return map_err(e),
        }
    }
    let t = t.expect("one change was asked");
    s.note_where(&c.sender.key, &t);
    let (d, n) = t.progress();
    let mut nudge = String::new();
    if done == Some(true) && left_off.is_empty() {
        nudge = format!(
            "\nSay where you left off: board_step task_id={} step_id={} done=true left_off=\"<one line: what's done, what's next>\". It shows on the card for whoever picks it up if this machine stops. Commit and push your branch (WIP is fine) at each step for the same reason.",
            t.short_ref(),
            one_line(sid, 20)
        );
    }
    format!(
        "{}: steps {d}/{n} done.{told}{nudge}\n{}",
        t.short_ref(),
        s.line(&t)
    )
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
    // Where the last holder's work is and where it left off (card #137),
    // read before this agent's own place can replace it.
    let handover = if cur.holders.iter().any(|h| *h == as_[0]) {
        String::new()
    } else {
        cur.handover_lines()
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
    if !handover.is_empty() {
        out += &format!(
            ". Picking up where it was:{handover}\nWork from that branch (fetch it if it's pushed), not a fresh one"
        );
    }
    // Card #476: the person sent its last done check back and nobody has
    // answered them yet.
    if let Some(sb) = co.sent_back(&t.id).filter(|sb| !sb.answered) {
        out += &format!(
            ". The person sent done check {} back: {}. Answer them on it first (board_comment task_id={} blocker_id={}), then set completed again with proof that says what changed",
            sb.id,
            quoted(&sb.reason.text, coord::MAX_ANSWER),
            t.short_ref(),
            sb.id
        );
    }
    s.note_where(&c.sender.key, &t);
    let t = co.get_task(&t.id).unwrap_or(t);
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
            "You gave the ask {} back; Irori hands it to the next idle agent.",
            t.short_ref()
        );
    }
    format!(
        "You left {}; it stays on the Board.\n{}",
        t.short_ref(),
        s.line(&t)
    )
}

const ASSIGN_ARGS: &[(&str, Ty)] = &[("task_id", Ty::Str), ("to", Ty::Str), ("note", Ty::Str)];

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
    let (t, notice) = match co.hand_off(&as_[0], &id, &as_[1], trim(a.str("note"))) {
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

const STATUS_ARGS: &[(&str, Ty)] = &[
    ("task_id", Ty::Str),
    ("state", Ty::Str),
    ("note", Ty::Str),
    ("proof", Ty::Str),
    ("attach", Ty::Strs),
];

const BOARD_STATES: [&str; 6] = [
    coord::WORKING,
    coord::INPUT_REQUIRED,
    coord::IN_REVIEW,
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
        return err_text(
            "give state: working, input_required, in_review, completed, failed or canceled.",
        );
    }
    if st == coord::INPUT_REQUIRED {
        return s.input_required(&id, trim(a.str("note")));
    }
    let proof = trim(a.str("proof"));
    let paths = a.strs("attach");
    let completing = st == coord::COMPLETED;
    if !completing && (!proof.is_empty() || paths.iter().any(|p| !trim(p).is_empty())) {
        return err_text("proof and attach go with completed.");
    }
    let (co, as_) = match s.on(&id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    // A done check is for work the person checks, not a choice still
    // theirs (card #354): a card whose done-when has them pick or decide
    // asks that as a blocker first. board_edit's done-when lifts it.
    if completing
        && let Some(t) = co.get_task(&id)
        && !t.blockers.iter().any(|b| !b.done_check)
        && let Some(w) = choice_left(&t.done_criteria)
    {
        return err_text(&format!(
            "{}'s done-when has the person {w} (\"{}\"): a done check isn't where they choose, and a Done there settles nothing. Raise a board_block that asks for that choice, with the options and your pick recommended: true; act on their answer, then set completed. If done-when describes something already settled, board_edit it to say what is done.",
            t.short_ref(),
            one_line(&t.done_criteria, 160)
        ));
    }
    // An agent's completed waits on the person's Done, so it brings proof
    // they can decide on without digging (card #140): the text is the done
    // check's context, the files its attachments. Coord refuses one without
    // proof after its own refusals (not the holder, open subtasks, an ask).
    if proof.chars().count() > coord::MAX_OBJECTIVE {
        return err_text(&format!(
            "proof longer than {} characters.",
            coord::MAX_OBJECTIVE
        ));
    }
    let files = if completing && !proof.is_empty() {
        match s.attach_files(c, &paths) {
            Ok(f) => f,
            Err(m) => return m,
        }
    } else {
        Vec::new()
    };
    // With the done check on, completed's text is the proof alone: a note
    // doesn't stand in for it.
    let note = if completing && (!proof.is_empty() || co.done_check()) {
        proof
    } else {
        trim(a.str("note"))
    };
    let t = match co.set_task_state_with(
        &as_[0],
        &id,
        &st,
        note,
        files.iter().map(to_coord).collect(),
        completing,
    ) {
        Ok(t) => t,
        Err(e) => return map_err(e),
    };
    // An agent's completed waits on the person's Done (card #112).
    if st == coord::COMPLETED
        && t.state != coord::COMPLETED
        && let Some(b) = t.blockers.iter().find(|b| b.open() && b.done_check)
    {
        {
            return format!(
                "{} waits on the person to confirm it's done (done check {}{}). Their Done closes it; if they say why it isn't done, you hear it as an answered blocker: answer them on it first (board_comment task_id={} blocker_id={}), so they see you heard them, then fix it and set completed again with proof that says what changed. Keep holding it until then.\n{}",
                t.short_ref(),
                one_line(&b.id, 20),
                attached_line(&files),
                t.short_ref(),
                one_line(&b.id, 20),
                s.line(&t)
            );
        }
    }
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

// --- board_edit ---

const EDIT_ARGS: &[(&str, Ty)] = &[
    ("task_id", Ty::Str),
    ("title", Ty::OptStr),
    ("objective", Ty::OptStr),
    ("done_when", Ty::OptStr),
    ("from", Ty::Str),
    ("after", Ty::Strs),
    ("not_after", Ty::Strs),
];

/// A list of card refs, blanks dropped.
fn refs_in(v: Vec<String>) -> Vec<String> {
    v.iter()
        .map(|r| trim(r).to_string())
        .filter(|r| !r.is_empty())
        .collect()
}

pub(crate) fn board_edit(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, EDIT_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    let e = TaskEdit {
        title: a.opt_str("title"),
        objective: a.opt_str("objective"),
        done_when: a.opt_str("done_when"),
        from: trim(a.str("from")).into(),
    };
    let (after, not_after) = (refs_in(a.strs("after")), refs_in(a.strs("not_after")));
    let words = e.title.is_some() || e.objective.is_some() || e.done_when.is_some();
    if !words && after.is_empty() && not_after.is_empty() {
        return err_text(
            "give title, objective or done_when (what the card is for now), or after or not_after (its order).",
        );
    }
    let (co, as_) = match s.on(&id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let mut done: Vec<String> = Vec::new();
    let mut t = None;
    if words {
        match co.edit_task(&as_[0], &id, e) {
            Ok(nt) => t = Some(nt),
            Err(e) => return map_err(e),
        }
        done.push("its thread says what changed. If the plan changed too, give it new steps with board_steps".into());
    }
    // Card #570: its order. Each list is checked whole before it is
    // written; a circle is refused with the cards in it.
    let so_far = |done: &[String]| {
        if done.is_empty() {
            String::new()
        } else {
            format!(" (the rest went through: {})", done.join("; "))
        }
    };
    if !after.is_empty() {
        match co.add_after(&as_[0], &id, &after) {
            Ok(nt) => {
                done.push(format!("it comes after {}", ref_list(&after)));
                t = Some(nt);
            }
            Err(e) => return format!("{}{}", map_err(e), so_far(&done)),
        }
    }
    if !not_after.is_empty() {
        match co.remove_after(&as_[0], &id, &not_after) {
            Ok(nt) => {
                done.push(format!("it no longer comes after {}", ref_list(&not_after)));
                t = Some(nt);
            }
            Err(e) => return format!("{}{}", map_err(e), so_far(&done)),
        }
    }
    let Some(t) = t else {
        return need_task_id();
    };
    format!(
        "Edited {}; {}.\n{}",
        t.short_ref(),
        done.join("; "),
        s.line(&t)
    )
}

/// Refs as the caller gave them, for a result line.
fn ref_list(v: &[String]) -> String {
    one_line(&v.join(", "), 200)
}

// --- leases ---

const LEASE_ARGS: &[(&str, Ty)] = &[
    ("resource", Ty::Str),
    ("ttl_minutes", Ty::OptInt),
    ("steal", Ty::Bool),
    ("pr", Ty::OptInt),
    ("head_sha", Ty::Str),
    ("base_sha", Ty::Str),
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
    // A lease in a shared card's repo goes on that board first, where
    // another person's lease refuses it.
    let mirrored = match s.b.router() {
        Some(rt) => match rt.mirror_acquire(&c.sender.key, res, ttl, a.bool("steal")) {
            Ok(tags) => tags,
            Err(e) => return map_err(e),
        },
        None => Vec::new(),
    };
    let l = match s
        .b
        .coord()
        .acquire(&c.sender.key, res, ttl, a.bool("steal"))
    {
        Ok(l) => l,
        Err(e) => {
            if !mirrored.is_empty()
                && let Some(rt) = s.b.router()
            {
                rt.mirror_release(&c.sender.key, res);
            }
            if res.starts_with("path:") {
                let rk = s.caller_repo(c);
                if !rk.is_empty() {
                    return format!("{} Your repoKey is {}.", map_err(e), one_line(&rk, 160));
                }
            }
            return map_err(e);
        }
    };
    let mut run_line = String::new();
    if coord::is_desk(&l.resource)
        && let Some(pr) = a.opt_int("pr")
    {
        let co = s.b.coord();
        let rk = co
            .desks()
            .into_iter()
            .map(|d| d.settings.repo_key)
            .find(|k| {
                coord::github_repo(k)
                    .map(|r| coord::desk_resource(&r))
                    .as_deref()
                    == Some(l.resource.as_str())
            });
        match rk.map(|rk| {
            co.record_desk_run(&c.sender.key, &rk, pr, a.str("head_sha"), a.str("base_sha"))
        }) {
            Some(Ok(())) => run_line = format!(" The desk shows PR #{pr} as Merging."),
            Some(Err(e)) => return format!("Leased {}, but {}", l.resource, map_err(e)),
            None => {}
        }
    }
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
    if !mirrored.is_empty() {
        out += &format!(
            " Mirrored on {}, where the other members' agents see it.",
            mirrored.join(", ")
        );
    }
    out + &run_line
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

const RELEASE_ARGS: &[(&str, Ty)] = &[("resource", Ty::Str), ("reason", Ty::Str)];

pub(crate) fn release(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, RELEASE_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let res = trim(a.str("resource"));
    if res.is_empty() {
        return err_text("give resource, exactly as the leases tool shows it.");
    }
    let r = s.b.coord().release(&c.sender.key, res);
    if let Some(rt) = s.b.router() {
        rt.mirror_release(&c.sender.key, res);
    }
    if let Err(e) = r {
        // Not yours: another person's lease on a shared board is never
        // stolen, so its holder is asked to release it instead (§5.4).
        if let Some(out) = release_request(s, c, res, a.str("reason")) {
            return out;
        }
        return map_err(e);
    }
    format!("Released {}.", one_line(res, MAX_FIELD_RUNES))
}

/// A release request for another person's lease on a shared board, `None`
/// when no board holds one on res that isn't the caller's own person's.
fn release_request(s: &Inner, c: &Caller, res: &str, why: &str) -> Option<String> {
    let rt = s.b.router()?;
    let (_, cm, _) = rt.request_release(&c.sender.key, res, why).ok()?;
    let mut out = format!(
        "{} isn't yours: another person's agent holds it on a shared board, and nobody steals another person's lease. Asked its holder to release it on {} (comment:{}); they hear at their next turn, and the lease is yours to take once they release it or it expires.",
        one_line(res, MAX_FIELD_RUNES),
        s.short_of(&cm.task_id),
        cm.id
    );
    if let Some(b) = s.b.shared_boards().and_then(|sb| sb.board_of(&cm.task_id)) {
        out += &format!("\n{}", b.visible_to());
    }
    Some(out)
}

// --- board_split ---

/// What a caller hears when its spawn marks were set aside (spawned
/// agents §3): the edit itself went through.
const SPAWN_REFUSED_TOOL: &str = "Your spawn marks were set aside: only the card's owner or their agents ask for helpers. The rest of the change went through.\n";

const SUBTASK_FIELDS: &[(&str, Ty)] = &[
    ("title", Ty::Str),
    ("objective", Ty::Str),
    ("done_when", Ty::Str),
    ("steps", Ty::Strs),
    ("assign_to", Ty::Str),
    ("spawn", Ty::Bool),
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
    let asked: Vec<bool> = subtasks.iter().map(|st| st.bool("spawn")).collect();
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
            spawn: st.bool("spawn"),
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
    if made.iter().zip(&asked).any(|(k, a)| *a && !k.spawn) {
        b += &format!("\n{}", SPAWN_REFUSED_TOOL.trim_end());
    }
    for (i, k) in made.iter().enumerate() {
        b += &format!(
            "\nCreated {} {}{}.",
            k.short_ref(),
            quoted(&one_line(&k.title, MAX_CARD_TITLE), MAX_CARD_TITLE),
            if k.spawn { " (spawn)" } else { "" }
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
