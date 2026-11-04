//! Cards: steps, blocker items, watchers, activity and watch digests
//! (I:internal/coord/cards.go, cards_impl.go; coord.md §3.5, §3.6).

use std::collections::HashMap;

use gocompat::quote::quote;
use gocompat::time::GoTime;
use irori_identity::owner;
use store::{Event, kinds};

use super::error::{Result, bad_state, invalid, not_allowed, not_found, subtask};
use super::fold::{Fold, Touch, WatchLine, line_of, restore_state, sync_co_holders, take_up};
use super::links::parse_link;
use super::rows::*;
use super::types::*;
use super::util::{
    MAX_NOTICE_FIELD, SWEEPER, caller, check_key, check_text, cmp_or, cmp_str, contains, cut,
    cut_show, deeper, multiline, new_id, remove_str, runes,
};
use super::{
    ASKED_AGAIN, DONE_ANSWER, DONE_CONTEXT, DONE_QUESTION, KEEP_RESOLVED, MAX_ANSWER, MAX_ATTACHMENTS,
    MAX_BLOCKER_LINKS, MAX_COMMENT, MAX_CONTEXT, MAX_LEFT_OFF, MAX_MISSING, MAX_MISSING_TEXT,
    MAX_OBJECTIVE, MAX_OPEN_BLOCKERS, MAX_OPTIONS, MAX_OWN_ANSWER, MAX_QUESTION, MAX_STEP_TEXT,
    MAX_STEPS, MAX_WHY, NEEDS_PROOF,
};
use crate::names::trim_space;

const MAX_WATCH_LINES: usize = 8;
const MAX_WATCH_PENDING: usize = 32;
const MAX_ACTIVITY: usize = super::MAX_ACTIVITY;

const WATCH_HEAD: &str = "Update on ";
const WATCH_HEAD_END: &str = ", a card you hold, watch or raised a blocker on:\n";
const WATCH_FOOT: &str = "\nNow: ";

fn earlier_line(n: usize) -> String {
    format!("- ({n} earlier changes)\n")
}

/// The number of a short id (`b12` with prefix `b` is 12).
pub(crate) fn seq_of(id: &str, prefix: &str) -> i64 {
    match id.strip_prefix(prefix) {
        Some(n) => super::resource::go_atoi(n).unwrap_or(0),
        None => 0,
    }
}

pub(crate) fn blocker_index(t: &Task, id: &str) -> Option<usize> {
    t.blockers.iter().position(|b| b.id == id)
}

pub(crate) fn open_with_question(t: &Task, q: &str) -> Option<usize> {
    t.blockers.iter().position(|b| b.open() && b.question == q)
}

pub(crate) fn count_open(t: &Task) -> usize {
    t.blockers.iter().filter(|b| b.open()).count()
}

/// Reason is the first open item's reason, BlockedBy the first open
/// item's On.
pub(crate) fn sync_reason(t: &mut Task) {
    t.reason.clear();
    t.blocked_by = None;
    for b in t.blockers.iter().filter(|b| b.open()) {
        if t.reason.is_empty() {
            t.reason = b.reason.clone();
        }
        if t.blocked_by.is_none() && b.on.is_some() {
            t.blocked_by = b.on.clone();
        }
    }
}

/// Resolves every open item as closed, with note as the answer.
pub(crate) fn close_open(t: &mut Task, by: &str, note: &str, at: GoTime) {
    for b in t.blockers.iter_mut().filter(|b| b.open()) {
        b.resolved = Some(Resolution {
            answer: note.into(),
            option: -1,
            by: by.into(),
            at,
            closed: true,
            note: String::new(),
        });
    }
    trim_resolved(t);
}

/// The person's completed answers t's open done checks with Done (card
/// #315), as their Done button would have.
pub(crate) fn say_done_open(t: &mut Task, by: &str, at: GoTime) {
    for b in t.blockers.iter_mut().filter(|b| b.open() && b.done_check) {
        let Some(i) = b.options.iter().position(|o| o.answer == DONE_ANSWER) else {
            continue;
        };
        b.resolved = Some(Resolution {
            answer: DONE_ANSWER.into(),
            option: i as i64,
            by: by.into(),
            at,
            closed: false,
            note: String::new(),
        });
    }
}

/// Keeps the newest KEEP_RESOLVED resolved items.
fn trim_resolved(t: &mut Task) {
    let n = t.blockers.len() - count_open(t);
    if n <= KEEP_RESOLVED {
        return;
    }
    let mut drop = n - KEEP_RESOLVED;
    t.blockers.retain(|b| {
        if !b.open() && drop > 0 {
            drop -= 1;
            return false;
        }
        true
    });
}

pub(crate) fn add_links(mut ls: Vec<Link>, more: &[Link]) -> Vec<Link> {
    for l in more {
        if !ls
            .iter()
            .any(|x| x.kind == l.kind && x.reference == l.reference)
        {
            ls.push(l.clone());
        }
    }
    ls
}

pub(crate) fn add_files(mut fs: Vec<FileRef>, more: &[FileRef]) -> Vec<FileRef> {
    for f in more {
        if !fs.iter().any(|x| x.sha256 == f.sha256) {
            fs.push(f.clone());
        }
    }
    fs
}

fn capped<T>(mut v: Vec<T>, n: usize) -> Vec<T> {
    v.truncate(n);
    v
}

fn brings_new(b: &Blocker, links: &[Link], files: &[FileRef]) -> bool {
    add_links(b.links.clone(), links).len() > b.links.len()
        || add_files(b.attachments.clone(), files).len() > b.attachments.len()
}

pub(crate) fn option_index(os: &[BlockerOption], answer: &str) -> Option<usize> {
    os.iter()
        .position(|o| gocompat::text::equal_fold(&o.answer, answer))
}

/// Derives item `bid`'s Status, SentTo and Missing from its blocker
/// comments in log order.
pub(crate) fn refresh_blocker(
    comments: &HashMap<String, Comment>,
    ids: Option<&Vec<String>>,
    t: &mut Task,
    bid: &str,
) {
    let Some(i) = blocker_index(t, bid) else {
        return;
    };
    let asked = t.blockers[i].asked.clone();
    let mine = || {
        ids.into_iter()
            .flatten()
            .filter_map(|id| comments.get(id))
            .filter(|cm| cm.blocker_id == bid)
    };
    let mut cs: Vec<&Comment> = mine().filter(|cm| !cm.pos().before(&asked)).collect();
    cs.sort_by_key(|c| c.pos());
    // The person's own words since the options were offered set them aside
    // (card #90); a revision with new options brings options back.
    let since = t.blockers[i].options_at.clone();
    let aside = !t.blockers[i].options.is_empty()
        && mine().any(|cm| cm.person && cm.pick <= 0 && !cm.pos().before(&since));
    let b = &mut t.blockers[i];
    b.options_aside = aside;
    b.status = BLOCKER_OPEN.into();
    b.sent_to.clear();
    b.missing.clear();
    let n = cs.len();
    if n == 0 {
        return;
    }
    let Some(last_person) = cs.iter().rposition(|c| c.person) else {
        return;
    };
    if last_person == n - 1 || cs[n - 1].missing.is_empty() {
        // The person's answer stands, or an agent replied after it with
        // nothing missing: it acted on the answer (card #204), so the item
        // waits on the agent to resolve it, not on the person.
        b.status = BLOCKER_ANSWERED.into();
        b.sent_to = cs[last_person].to.clone();
    } else {
        // An agent asked back: what it still needs puts it before the person.
        b.status = BLOCKER_REPLIED.into();
        b.missing = cs[n - 1].missing.clone();
    }
}

/// Keeps at most one doing (the first) and no done step doing.
fn norm_steps(ss: Vec<Step>) -> Vec<Step> {
    let mut doing = false;
    ss.into_iter()
        .map(|mut s| {
            if s.done || doing {
                s.doing = false;
            }
            doing = doing || s.doing;
            s
        })
        .collect()
}

/// `ss` with `u` applied to step i.
fn apply_step(ss: &[Step], i: usize, u: StepUpdate) -> Vec<Step> {
    let mut out = ss.to_vec();
    if let Some(done) = u.done {
        out[i].done = done;
        if done {
            out[i].doing = false;
        }
    }
    if let Some(doing) = u.doing {
        if doing {
            for s in out.iter_mut() {
                s.doing = false;
            }
            out[i].doing = true;
            out[i].done = false;
        } else {
            out[i].doing = false;
        }
    }
    out
}

/// Trims a step list in place and checks it.
pub(crate) fn check_steps(input: &mut [Step]) -> Result<()> {
    if input.len() > MAX_STEPS {
        return Err(invalid(format!("more than {MAX_STEPS} steps")));
    }
    let mut doing = 0;
    for s in input.iter_mut() {
        s.text = trim_space(&s.text).to_string();
        s.id = trim_space(&s.id).to_string();
        check_text("a step", &s.text, 1, MAX_STEP_TEXT)?;
        if multiline(&s.text) {
            return Err(invalid("a step is one line"));
        }
        if s.done && s.doing {
            return Err(invalid(format!(
                "step {} is both done and in progress",
                quote(&s.text)
            )));
        }
        if s.doing {
            doing += 1;
        }
    }
    if doing > 1 {
        return Err(invalid("only one step can be in progress"));
    }
    Ok(())
}

/// Mints s1, s2, ... for a new task's checked steps.
pub(crate) fn new_steps(input: &[Step]) -> Vec<Step> {
    input
        .iter()
        .enumerate()
        .map(|(i, s)| Step {
            id: format!("s{}", i + 1),
            text: s.text.clone(),
            done: s.done,
            doing: s.doing,
            // board_create gives no owners; board_steps does (card #86).
            owner: String::new(),
            spawn: false,
        })
        .collect()
}

/// A Block reason as an item's question and, when longer, its context.
pub(crate) fn block_question(reason: &str) -> (String, String) {
    if runes(reason) <= MAX_QUESTION {
        return (reason.into(), String::new());
    }
    (cut(reason, MAX_QUESTION), reason.into())
}

fn picked_line(opt: i64, rec_by: &str, rec: bool) -> String {
    if !rec_by.is_empty() && rec {
        return format!("\n(That is the answer {rec_by} recommended.)");
    }
    if !rec_by.is_empty() {
        return format!("\n(That is option {}, offered by {rec_by}.)", opt + 1);
    }
    String::new()
}

/// The person's latest answer on item bid since its question was last
/// replaced.
fn last_answer(f: &Fold, t: &Task, bid: &str) -> Option<Comment> {
    let asked = blocker_index(t, bid)
        .map(|i| t.blockers[i].asked.clone())
        .unwrap_or_default();
    let mut last: Option<&Comment> = None;
    for id in f.by_task.get(&t.id).into_iter().flatten() {
        if let Some(cm) = f.comments.get(id)
            && cm.blocker_id == bid
            && cm.person
            && !cm.pos().before(&asked)
            && last.is_none_or(|l| l.pos().before(&cm.pos()))
        {
            last = Some(cm);
        }
    }
    last.cloned()
}

/// The done check the person sent back (card #476): the newest done check
/// on t, when the person's last word on it is their own (Not done, with
/// why), open or not. `answered` is whether an agent has answered them
/// on it since: a reply on its thread, or resolving it with a note.
#[derive(Debug, Clone)]
pub struct SentBack {
    pub id: String,
    pub reason: Comment,
    pub proof: String,
    pub answered: bool,
}

pub(crate) fn sent_back(f: &Fold, t: &Task) -> Option<SentBack> {
    let b = t
        .blockers
        .iter()
        .filter(|b| b.done_check)
        .max_by_key(|b| b.at.unix_nano())?;
    let reason = last_answer(f, t, &b.id).filter(|c| c.pick <= 0)?;
    let replied = f
        .by_task
        .get(&t.id)
        .into_iter()
        .flatten()
        .filter_map(|id| f.comments.get(id))
        .any(|cm| cm.blocker_id == b.id && !cm.person && reason.pos().before(&cm.pos()));
    let noted = b.resolved.as_ref().is_some_and(|r| {
        !r.closed && !r.note.is_empty() && r.note != ASKED_AGAIN && !f.is_owner(&r.by)
    });
    Some(SentBack {
        id: b.id.clone(),
        reason,
        proof: b.context.clone(),
        answered: replied || noted,
    })
}

/// What folding a blocked row did: its activity line, the item it raised
/// or revised, and whether it was a revision.
#[derive(Debug, Default)]
pub(crate) struct Folded {
    pub what: String,
    pub raised: String,
    pub revised: bool,
}

impl Folded {
    fn line(what: String) -> Folded {
        Folded {
            what,
            ..Folded::default()
        }
    }

    fn raised(what: String, id: &str) -> Folded {
        Folded {
            what,
            raised: id.into(),
            revised: false,
        }
    }
}

/// A row's options as the item keeps them: deduplicated, capped, dated.
fn fold_options(input: &[BlockerOption], at: GoTime) -> Vec<BlockerOption> {
    let mut opts: Vec<BlockerOption> = Vec::new();
    for o in input {
        if opts.len() < MAX_OPTIONS && option_index(&opts, &o.answer).is_none() {
            let mut o = o.clone();
            o.at = cmp_or(o.at, at);
            opts.push(o);
        }
    }
    opts
}

/// Whether two option lists offer the same answers, recommended alike.
pub(crate) fn same_options(a: &[BlockerOption], b: &[BlockerOption]) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|o| option_index(b, &o.answer).is_some_and(|j| b[j].recommended == o.recommended))
}

/// The activity line of a revision: what it changed on item b.
fn revision_line(b: &Blocker, br: &BlockerRow, opts: &[BlockerOption]) -> String {
    let mut parts = Vec::new();
    if br.question != b.question {
        parts.push(format!("asks now “{}”", br.question));
    }
    if br.context != b.context {
        parts.push(if br.context.is_empty() {
            "dropped the context".to_string()
        } else {
            "changed the context".to_string()
        });
    }
    if !same_options(opts, &b.options) {
        if opts.is_empty() {
            parts.push("no options now, a plain question".into());
        } else {
            let list: Vec<String> = opts
                .iter()
                .map(|o| {
                    let mut a = cut(&o.answer, 80);
                    if o.recommended {
                        a.push_str(" (recommended)");
                    }
                    a
                })
                .collect();
            parts.push(format!("options now: {}", list.join(" | ")));
        }
    }
    if parts.is_empty() {
        return format!("revised blocker {}", b.id);
    }
    format!("revised blocker {}: {}", b.id, parts.join("; "))
}

impl Fold {
    /// Records activity on t from row ev and queues a watch line.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn touch(
        &mut self,
        ev: &Event,
        t: &Touch,
        at: GoTime,
        by: &str,
        what: &str,
        watch: bool,
        also: Vec<String>,
    ) {
        self.touch_line(t, line_of(ev, at, by, what), watch, also);
    }

    /// The line goes in at its row's position; past MAX_ACTIVITY the
    /// lowest positions drop.
    pub(crate) fn touch_line(&mut self, t: &Touch, a: Activity, watch: bool, also: Vec<String>) {
        self.touch_line_wake(t, a, watch, also, false, false);
    }

    /// A state row's line: always watched, and `wake` wakes the parent's
    /// holders (card #89). `wake_holders` wakes the card's own holders when
    /// another agent changed its state or raised a blocker on it (card #320).
    pub(crate) fn touch_parent(&mut self, t: &Touch, a: Activity, wake: bool, wake_holders: bool) {
        self.touch_line_wake(t, a, true, Vec::new(), wake, wake_holders);
    }

    /// Whether every subtask of `id`'s parent has ended, now that `id` has.
    pub(crate) fn last_subtask(&self, id: &str) -> bool {
        let Some(p) = self.tasks.get(id).and_then(|t| self.tasks.get(&t.parent)) else {
            return false;
        };
        p.subtasks
            .iter()
            .all(|k| self.tasks.get(k).is_none_or(|kt| is_terminal(&kt.state)))
    }

    /// The line goes on t's activity and, when watched, on t's digest and
    /// its open parent's digest; `wake` wakes the parent's holders. A watched
    /// line by an agent that doesn't work the card also goes to the ones who
    /// do, woken when `wake_holders` (card #320: any agent may change any
    /// card, and its holders hear who changed what).
    fn touch_line_wake(
        &mut self,
        t: &Touch,
        a: Activity,
        watch: bool,
        mut also: Vec<String>,
        wake: bool,
        wake_holders: bool,
    ) {
        if a.what.is_empty() {
            return;
        }
        let (by, what) = (a.by.clone(), a.what.clone());
        let acts = self.activity.entry(t.id.clone()).or_default();
        let pos = a.pos();
        let i = acts.partition_point(|x| !pos.before(&x.pos()));
        acts.insert(i, a);
        if acts.len() > MAX_ACTIVITY {
            let n = acts.len() - MAX_ACTIVITY;
            acts.drain(..n);
        }
        if watch {
            let mut woken = Vec::new();
            if self.is_agent_key(&by) && !contains(&t.holders, &by) {
                for h in &t.holders {
                    if !contains(&also, h) {
                        also.push(h.clone());
                    }
                }
                if wake_holders {
                    woken = t.holders.clone();
                }
            }
            self.queue_watch_wake(&t.id, t.has_watchers, &by, &what, also, woken);
            if let Some(p) = self.tasks.get(&t.parent)
                && !is_terminal(&p.state)
            {
                let (pid, pw, pref, ph) = (
                    p.id.clone(),
                    !p.watchers.is_empty(),
                    p.short_ref(),
                    p.holders.clone(),
                );
                let line = format!("{} (part of {pref}): {what}", t.short_ref);
                let woken = if wake { ph.clone() } else { Vec::new() };
                self.queue_watch_wake(&pid, pw, &by, &line, ph, woken);
            }
        }
    }

    /// Queues a watch line for a task's watchers and `also`, outside replay.
    pub(crate) fn queue_watch(
        &mut self,
        id: &str,
        has_watchers: bool,
        by: &str,
        what: &str,
        also: Vec<String>,
    ) {
        self.queue_watch_wake(id, has_watchers, by, what, also, Vec::new());
    }

    /// [`Self::queue_watch`], waking `wake` (a subset of `also`).
    fn queue_watch_wake(
        &mut self,
        id: &str,
        has_watchers: bool,
        by: &str,
        what: &str,
        also: Vec<String>,
        wake: Vec<String>,
    ) {
        if self.replaying || !has_watchers && also.is_empty() {
            return;
        }
        let q = self.watchq.entry(id.to_string()).or_default();
        q.lines.push(WatchLine {
            by: by.into(),
            what: what.into(),
            also,
            wake,
        });
        if q.lines.len() > MAX_WATCH_PENDING {
            let n = q.lines.len() - MAX_WATCH_PENDING;
            q.dropped += n;
            q.lines.drain(..n);
        }
    }

    /// Applies a blocked task.status: adds (or replaces) an item. Returns
    /// the activity phrase and, for a raise, the new item's id.
    pub(crate) fn fold_blocked(
        &mut self,
        t: &mut Task,
        p: &TaskStateRow,
        ev: &Event,
        was_blocked: bool,
        at: GoTime,
    ) -> Folded {
        let pos = RowPos::of(ev);
        let none = Folded::default;
        if let Some(br) = p.blocker.as_ref().filter(|b| !b.id.is_empty()) {
            if let Some(i) = blocker_index(t, &br.id)
                && br.replace
                && t.blockers[i].open()
            {
                if pos.before(&t.blockers[i].asked) {
                    return none();
                }
                let person = self.is_owner(&p.by);
                let b = &mut t.blockers[i];
                let mut what = String::new();
                if br.revise {
                    let opts = fold_options(&br.options, at);
                    what = revision_line(b, br, &opts);
                    b.options = opts;
                    b.options_at = pos.clone();
                }
                b.question = br.question.clone();
                b.context = br.context.clone();
                b.reason = cmp_str(&p.reason, &br.question);
                // A person's revision leaves the answer going to the raiser.
                if !(br.revise && person) {
                    b.raised_by = p.by.clone();
                }
                b.at = at;
                b.on = br.on.clone();
                b.links = capped(add_links(Vec::new(), &br.links), MAX_BLOCKER_LINKS);
                b.attachments = capped(add_files(Vec::new(), &br.attachments), MAX_ATTACHMENTS);
                b.asked = pos;
                let (id, q) = (b.id.clone(), b.question.clone());
                refresh_blocker(&self.comments, self.by_task.get(&t.id), t, &id);
                if br.revise {
                    return Folded {
                        what,
                        raised: id,
                        revised: true,
                    };
                }
                return Folded::line(format!("changed the blocker: {q}"));
            }
            if blocker_index(t, &br.id).is_some() {
                return none();
            }
            if let Some(i) = open_with_question(t, &br.question) {
                let b = &mut t.blockers[i];
                b.on = br.on.clone();
                if !brings_new(b, &br.links, &br.attachments) {
                    return none();
                }
                b.links = capped(add_links(b.links.clone(), &br.links), MAX_BLOCKER_LINKS);
                b.attachments = capped(
                    add_files(b.attachments.clone(), &br.attachments),
                    MAX_ATTACHMENTS,
                );
                return Folded::line(format!("added to the blocker: {}", b.question));
            }
            let seq = self.blocker_seq.entry(t.id.clone()).or_default();
            *seq = (*seq).max(seq_of(&br.id, "b"));
            let opts = fold_options(&br.options, at);
            let mut nb = Blocker {
                id: br.id.clone(),
                question: br.question.clone(),
                context: br.context.clone(),
                raised_by: p.by.clone(),
                at,
                options: opts,
                reason: cmp_str(&p.reason, &br.question),
                legacy: br.reason_only,
                on: br.on.clone(),
                links: capped(add_links(Vec::new(), &br.links), MAX_BLOCKER_LINKS),
                attachments: capped(add_files(Vec::new(), &br.attachments), MAX_ATTACHMENTS),
                done_check: br.done_check,
                transfer: br.transfer,
                ..Blocker::default()
            };
            if br.replace {
                nb.asked = pos.clone();
            }
            if br.revise {
                nb.options_at = pos;
            }
            t.blockers.push(nb);
            return Folded::raised(format!("raised a blocker: {}", br.question), &br.id);
        }
        // A reason-only row (written before items existed).
        let (q, ctx) = if runes(&p.reason) > MAX_QUESTION {
            (cut(&p.reason, MAX_QUESTION), p.reason.clone())
        } else {
            (p.reason.clone(), String::new())
        };
        if was_blocked && let Some(i) = t.blockers.iter().position(|b| b.open() && b.legacy) {
            if pos.before(&t.blockers[i].asked) {
                return none();
            }
            let b = &mut t.blockers[i];
            b.question = q.clone();
            b.context = ctx;
            b.reason = p.reason.clone();
            b.raised_by = p.by.clone();
            b.at = at;
            b.on = p.blocked_by.clone();
            b.asked = pos;
            let id = b.id.clone();
            refresh_blocker(&self.comments, self.by_task.get(&t.id), t, &id);
            return Folded::line(format!("changed the blocker: {q}"));
        }
        let seq = self.blocker_seq.entry(t.id.clone()).or_default();
        *seq += 1;
        let bid = format!("b{seq}");
        t.blockers.push(Blocker {
            id: bid.clone(),
            question: q.clone(),
            context: ctx,
            raised_by: p.by.clone(),
            at,
            reason: p.reason.clone(),
            legacy: true,
            on: p.blocked_by.clone(),
            ..Blocker::default()
        });
        Folded::raised(format!("raised a blocker: {q}"), &bid)
    }

    pub(crate) fn next_blocker_id(&self, task_id: &str) -> String {
        format!(
            "b{}",
            self.blocker_seq.get(task_id).copied().unwrap_or(0) + 1
        )
    }

    /// Folds the cards' own rows.
    pub(crate) fn apply_card(&mut self, ev: &Event, at: &dyn Fn(GoTime) -> GoTime) {
        let task_id;
        match ev.kind.as_str() {
            kinds::BLOCKER_OPTION => {
                let Some(p) = row::<BlockerOptionRow>(&ev.payload) else {
                    return;
                };
                if p.answer.is_empty() {
                    return;
                }
                let Some(t) = self.tasks.get_mut(&p.task_id) else {
                    return;
                };
                let Some(i) = blocker_index(t, &p.blocker_id).filter(|&i| t.blockers[i].open())
                else {
                    return;
                };
                // A recommendation for the options a revision replaced. Only a
                // revision: a plain replace keeps the options, and so the
                // recommendation, wherever the row lands in the fold.
                if RowPos::of(ev).before(&t.blockers[i].options_at) {
                    return;
                }
                let when = at(p.at);
                let b = &mut t.blockers[i];
                match option_index(&b.options, &p.answer) {
                    Some(j) => {
                        if b.options[j].recommended {
                            return;
                        }
                        b.options[j].recommended = true;
                    }
                    None if b.options.len() >= MAX_OPTIONS => return,
                    None => b.options.push(BlockerOption {
                        answer: p.answer.clone(),
                        why: p.why.clone(),
                        by: p.by.clone(),
                        at: when,
                        recommended: true,
                        link: None,
                    }),
                }
                let what = format!("recommended “{}” for “{}”", p.answer, b.question);
                let raiser = b.raised_by.clone();
                t.updated_at = when;
                let ti = Touch::of(t);
                self.touch(ev, &ti, when, &p.by, &what, true, vec![raiser]);
                task_id = p.task_id;
            }
            kinds::BLOCKER_RESOLVED => {
                let Some(p) = row::<BlockerResolvedRow>(&ev.payload) else {
                    return;
                };
                let Some(t) = self.tasks.get_mut(&p.task_id) else {
                    return;
                };
                let Some(i) = blocker_index(t, &p.blocker_id).filter(|&i| t.blockers[i].open())
                else {
                    return;
                };
                let when = at(p.at);
                let mut opt = p.option;
                if opt < -1 || opt >= t.blockers[i].options.len() as i64 {
                    opt = -1;
                }
                let q = t.blockers[i].question.clone();
                t.blockers[i].resolved = Some(Resolution {
                    answer: p.answer.clone(),
                    option: opt,
                    by: p.by.clone(),
                    at: when,
                    closed: p.closed,
                    note: p.note.clone(),
                });
                if t.state == BLOCKED && count_open(t) == 0 {
                    let mut s = p.restored.clone();
                    if !valid_state(&s) || s == BLOCKED || is_terminal(&s) {
                        s = restore_state(t).into();
                    }
                    t.state = s;
                }
                trim_resolved(t);
                sync_reason(t);
                t.updated_at = when;
                t.updated_by = p.by.clone();
                let ti = Touch::of(t);
                let what = if p.closed {
                    format!("closed “{q}”: {}", p.answer)
                } else if !p.note.is_empty() {
                    format!("resolved “{q}”: {}", p.note)
                } else {
                    format!("answered “{q}”: {}", p.answer)
                };
                self.touch(ev, &ti, when, &p.by, &what, true, Vec::new());
                task_id = p.task_id;
            }
            kinds::TASK_STEPS => {
                let Some(p) = row::<TaskStepsRow>(&ev.payload) else {
                    return;
                };
                if !self.tasks.contains_key(&p.task_id) {
                    return;
                }
                let mut ss: Vec<Step> = Vec::new();
                for s in &p.steps {
                    if s.id.is_empty() || ss.len() >= MAX_STEPS || ss.iter().any(|o| o.id == s.id) {
                        continue;
                    }
                    let seq = self.step_seq.entry(p.task_id.clone()).or_default();
                    *seq = (*seq).max(seq_of(&s.id, "s"));
                    ss.push(s.clone());
                }
                // Spawn marks only from the card's owner's side (§3).
                let refused = {
                    let t = &self.tasks[&p.task_id];
                    !self.may_spawn_on(t, &ev.device_id)
                        && super::parts::keep_marks(&t.steps, &mut ss)
                };
                let t = self.tasks.get_mut(&p.task_id).expect("checked");
                let was = std::mem::replace(&mut t.steps, norm_steps(ss));
                let when = at(p.at);
                t.updated_at = when;
                t.updated_by = p.by.clone();
                take_up(t, &p.by);
                sync_co_holders(t);
                let (d, n) = t.progress();
                let mut what = format!("{} ({d}/{n} done)", steps_change(&was, &t.steps));
                if refused {
                    what = format!("{what}; {}", super::parts::SPAWN_REFUSED);
                }
                let ti = Touch::of(t);
                self.touch(ev, &ti, when, &p.by, &what, true, Vec::new());
                task_id = p.task_id;
            }
            kinds::TASK_STEP => {
                let Some(p) = row::<TaskStepRow>(&ev.payload) else {
                    return;
                };
                let Some(t) = self.tasks.get_mut(&p.task_id) else {
                    return;
                };
                let Some(i) = t.steps.iter().position(|s| s.id == p.step_id) else {
                    return;
                };
                let before = t.steps[i].clone();
                t.steps = apply_step(
                    &t.steps,
                    i,
                    StepUpdate {
                        done: p.done,
                        doing: p.doing,
                    },
                );
                if let Some(o) = &p.owner {
                    t.steps[i].owner = trim_space(o).to_string();
                }
                let lo = trim_space(&p.left_off);
                if !lo.is_empty() {
                    t.left_off = Some(LeftOff {
                        text: lo.to_string(),
                        by: p.by.clone(),
                        at: at(p.at),
                    });
                }
                sync_co_holders(t);
                let after = t.steps[i].clone();
                let when = at(p.at);
                t.updated_at = when;
                t.updated_by = p.by.clone();
                take_up(t, &p.by);
                let mut what = if after.done && !before.done {
                    format!("ticked “{}”", after.text)
                } else if !after.done && before.done {
                    format!("unticked “{}”", after.text)
                } else if after.doing && !before.doing {
                    format!("started “{}”", after.text)
                } else if !after.doing && before.doing {
                    format!("paused “{}”", after.text)
                } else if after.owner != before.owner && !after.owner.is_empty() {
                    format!("gave “{}” to {}", after.text, after.owner)
                } else if after.owner != before.owner {
                    format!("took {} off “{}”", before.owner, after.text)
                } else {
                    String::new()
                };
                if !what.is_empty() {
                    let (d, n) = t.progress();
                    what.push_str(&format!(" ({d}/{n})"));
                }
                let ti = Touch::of(t);
                self.touch(ev, &ti, when, &p.by, &what, true, Vec::new());
                task_id = p.task_id;
            }
            kinds::TASK_WATCHED => {
                let Some(p) = row::<TaskWatchedRow>(&ev.payload) else {
                    return;
                };
                if p.who.is_empty() {
                    return;
                }
                let Some(t) = self.tasks.get_mut(&p.task_id) else {
                    return;
                };
                if contains(&t.watchers, &p.who) == p.watching {
                    return;
                }
                let what = if p.watching {
                    t.watchers.push(p.who.clone());
                    "started watching"
                } else {
                    t.watchers = remove_str(&t.watchers, &p.who);
                    "stopped watching"
                };
                let ti = Touch::of(t);
                self.touch(ev, &ti, at(p.at), &p.who, what, false, Vec::new());
                task_id = p.task_id;
            }
            _ => return,
        }
        self.note(Change {
            kind: TASK_CHANGED.into(),
            task_id,
            ..Change::default()
        });
    }

    /// Drains the watch queue into one digest per card and set of
    /// recipients reading the same lines.
    pub(crate) fn watch_notices(&mut self) -> Vec<Notice> {
        let q = std::mem::take(&mut self.watchq);
        let mut out: Vec<Notice> = Vec::new();
        for (id, q) in q {
            let Some(t) = self.tasks.get(&id) else {
                continue;
            };
            let mut who: Vec<String> = Vec::new();
            let add = |k: &str, who: &mut Vec<String>| {
                let dev = owner::device_of(k);
                if !dev.is_empty() && dev != self.dev {
                    return;
                }
                if !k.is_empty() && !self.is_owner(k) && k != SWEEPER && !contains(who, k) {
                    who.push(k.into());
                }
            };
            for w in &t.watchers {
                add(w, &mut who);
            }
            for l in &q.lines {
                for a in &l.also {
                    add(a, &mut who);
                }
            }
            let mut by_text: HashMap<(String, bool), usize> = HashMap::new();
            for r in who {
                let kept: Vec<&WatchLine> = q
                    .lines
                    .iter()
                    .filter(|l| {
                        !(l.by == r || !contains(&t.watchers, &r) && !contains(&l.also, &r))
                    })
                    .collect();
                // Someone else's line about a subtask that matters wakes
                // the parent's holder; their own never does.
                let wake = kept.iter().any(|l| contains(&l.wake, &r));
                let lines: Vec<String> = kept
                    .into_iter()
                    .map(|l| {
                        let actor = if l.by == SWEEPER {
                            "Irori".to_string()
                        } else {
                            self.named(&l.by)
                        };
                        format!("{actor} {}", l.what)
                    })
                    .collect();
                if lines.is_empty() {
                    continue;
                }
                let text = watch_text(t, &lines, q.dropped);
                let key = (text.clone(), wake);
                if let Some(&i) = by_text.get(&key) {
                    out[i].to.push(r);
                    continue;
                }
                by_text.insert(key, out.len());
                out.push(Notice {
                    kind: NOTICE_WATCHED.into(),
                    task_id: id.clone(),
                    reference: format!("task:{id}"),
                    title: t.title.clone(),
                    to: vec![r],
                    by: SWEEPER.into(),
                    text,
                    wake,
                    ..Notice::default()
                });
            }
        }
        out
    }

    // ---- raise, recommend, answer ----

    pub(crate) fn raise(&mut self, by: &str, id: &str, nb: NewBlocker) -> Result<(Task, Blocker)> {
        let by = caller(by)?;
        let q = trim_space(&nb.question).to_string();
        let cx = trim_space(&nb.context).to_string();
        check_text("the question", &q, 1, MAX_QUESTION)?;
        check_text("the context", &cx, 0, MAX_CONTEXT)?;
        let now = self.stamp();
        let opts = clean_options(&nb.options, &by, now)?;
        let links = clean_blocker_links(&nb.links, &nb.files)?;
        let t = self.open_task(id)?.clone();
        let bb = nb.blocked_by.clone();
        self.raise_locked(
            &by, &t, &q, &q, &cx, opts, bb, false, now, links, nb.files, false, false,
        )
    }

    /// An agent's completed (card #112): the card waits on the person with
    /// a done check. Done closes it; their words go back to the agent. An
    /// earlier check on the card, open or answered, gives way to this one.
    pub(crate) fn ask_done(
        &mut self,
        by: &str,
        t: &Task,
        note: &str,
        files: Vec<FileRef>,
        need_proof: bool,
    ) -> Result<Task> {
        if files.len() > MAX_ATTACHMENTS {
            return Err(invalid(format!("at most {MAX_ATTACHMENTS} attachments")));
        }
        let earlier: Vec<String> = t
            .blockers
            .iter()
            .filter(|b| b.open() && b.done_check)
            .map(|b| b.id.clone())
            .collect();
        // A window that predates done checks answers Done without closing
        // the card (and the agent may have resolved it since): if the
        // person's last word on the latest check was Done, it stands, and
        // the card closes now.
        let said_done = t
            .blockers
            .iter()
            .rev()
            .find(|b| b.done_check)
            .is_some_and(|b| {
                b.options.first().is_some_and(|o| o.answer == DONE_ANSWER)
                    && last_answer(self, t, &b.id).is_some_and(|a| a.pick == 1)
            });
        // Card #140: the person decides Done on the proof, so a check with
        // none is refused here, after every other refusal and before any
        // write. Once they said Done it closes without one.
        if need_proof && !said_done && note.is_empty() {
            return Err(invalid(NEEDS_PROOF));
        }
        // Card #476: the person said why it isn't done. The agent answers
        // them on that check first, so the exchange stays on the card and
        // the next check shows what changed, then brings new proof.
        if let Some(sb) = sent_back(self, t).filter(|_| !said_done) {
            if !sb.answered {
                return Err(invalid(format!(
                    "the person sent done check {bid} back: {why}. Answer them on it first, in your own words: what you did about it (board_comment task_id={id} blocker_id={bid}). Then set completed again with proof that says what changed since.",
                    bid = sb.id,
                    why = quote(&cut(&sb.reason.text, MAX_ANSWER)),
                    id = t.short_ref(),
                )));
            }
            if !note.is_empty() && cut(note, MAX_OBJECTIVE) == sb.proof {
                return Err(invalid(format!(
                    "that is the proof the person sent back on {}: say what changed since their answer",
                    sb.id
                )));
            }
        }
        // The person already said Done: the completed row below closes the
        // checks itself (card #315), with no stop at working on the way.
        for bid in earlier.into_iter().filter(|_| !said_done) {
            self.answer(
                by,
                &t.id,
                &bid,
                BlockerAnswer {
                    text: if said_done {
                        "the person said Done".into()
                    } else {
                        ASKED_AGAIN.into()
                    },
                    resolve: true,
                    ..BlockerAnswer::default()
                },
            )?;
        }
        // The person already said Done: the card closes now, and the
        // proof's files have nothing to wait on (the note stays on the row).
        if said_done {
            let at = self.stamp();
            self.write(
                by,
                kinds::TASK_STATUS,
                &TaskStateRow {
                    task_id: t.id.clone(),
                    state: COMPLETED.into(),
                    note: cut(note, MAX_OBJECTIVE),
                    by: by.into(),
                    at,
                    ..TaskStateRow::default()
                },
            )?;
            return Ok(self.tasks[&t.id].clone());
        }
        let t = self.open_task(&t.id)?.clone();
        let now = self.stamp();
        // The note is the agent's proof (card #140): what changed, how it
        // was checked, the commit. It was held to MAX_OBJECTIVE already;
        // MAX_CONTEXT would cut exactly what the person reads to decide.
        let cx = if note.is_empty() {
            DONE_CONTEXT.to_string()
        } else {
            cut(note, MAX_OBJECTIVE)
        };
        let opts = clean_options(
            &[BlockerOption {
                answer: DONE_ANSWER.into(),
                why: "Closes the card.".into(),
                recommended: true,
                ..BlockerOption::default()
            }],
            by,
            now,
        )?;
        let bb = Some(BlockedBy {
            kind: BLOCKED_ON_PERSON.into(),
            reference: String::new(),
        });
        let (nt, _) = self.raise_locked(
            by,
            &t,
            DONE_QUESTION,
            DONE_QUESTION,
            &cx,
            opts,
            bb,
            false,
            now,
            Vec::new(),
            files,
            true,
            false,
        )?;
        Ok(nt)
    }

    /// Who decides a question on a shared board (card #264): a person
    /// blocker that names nobody (a done check too) is for the card's
    /// owner's person (#429), or whoever raised it when the owner is
    /// unknown, so it shows under that person's Needs a decision and as "Waiting on
    /// <name>" for the board's other members. Off a board it stays "".
    fn decider_default(&self, by: &str, t: &Task, nb: &mut Option<BlockedBy>) {
        let Some(b) = nb.as_mut() else {
            return;
        };
        if b.kind != BLOCKED_ON_PERSON || !b.reference.is_empty() {
            return;
        }
        // The card's owner decides (card #429): the person who made it, or
        // whose agent did, or whoever it was handed to since (card #432).
        // Another person's agent finishing it, or asking on it, doesn't
        // make it that agent's person's to handle.
        let mut p = self.owner_person(t);
        if p.is_empty() {
            p = self.person_of(by);
        }
        if !p.is_empty() {
            b.reference = format!("{}{p}", kinds::PERSON_ACTOR_PREFIX);
        }
    }

    /// Checks bb and writes the blocked row that raises an item.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn raise_locked(
        &mut self,
        by: &str,
        t: &Task,
        reason: &str,
        q: &str,
        cx: &str,
        opts: Vec<BlockerOption>,
        bb: Option<BlockedBy>,
        reason_only: bool,
        now: GoTime,
        links: Vec<Link>,
        files: Vec<FileRef>,
        done_check: bool,
        transfer: bool,
    ) -> Result<(Task, Blocker)> {
        if t.open_ask().is_some() {
            return Err(super::asks::ask_refusal(
                t,
                "a choice only the person can make goes on a card: ask_close it with outcome card (it stays yours), then board_block that card, one blocker per question; a question about what they meant goes on its thread (board_comment)",
            ));
        }
        let mut nb = self.check_blocker(bb.as_ref(), &t.id)?;
        self.decider_default(by, t, &mut nb);
        let same_q = open_with_question(t, q);
        if let Some(i) = same_q
            && t.state == BLOCKED
            && (nb.is_none() && !reason_only || t.blockers[i].on == nb)
            && !brings_new(&t.blockers[i], &links, &files)
        {
            return Ok((t.clone(), t.blockers[i].clone()));
        }
        if let Some(i) = same_q {
            let b = &t.blockers[i];
            let n = add_links(b.links.clone(), &links).len();
            if n > MAX_BLOCKER_LINKS {
                return Err(invalid(format!(
                    "blocker {} would have {n} links (at most {MAX_BLOCKER_LINKS})",
                    b.id
                )));
            }
            let n = add_files(b.attachments.clone(), &files).len();
            if n > MAX_ATTACHMENTS {
                return Err(invalid(format!(
                    "blocker {} would have {n} attachments (at most {MAX_ATTACHMENTS})",
                    b.id
                )));
            }
        }
        if let Some(i) = same_q
            && t.state == BLOCKED
            && nb.is_none()
            && !reason_only
        {
            nb = t.blockers[i].on.clone();
        }
        if nb.is_none() && !reason_only {
            nb = Some(BlockedBy {
                kind: BLOCKED_ON_PERSON.into(),
                reference: String::new(),
            });
            self.decider_default(by, t, &mut nb);
        }
        let mut row_b = BlockerRow {
            id: self.next_blocker_id(&t.id),
            question: q.into(),
            context: cx.into(),
            options: opts,
            reason_only,
            on: nb.clone(),
            links,
            attachments: files,
            done_check,
            transfer,
            ..BlockerRow::default()
        };
        if reason_only
            && t.state == BLOCKED
            && let Some(b) = t.blockers.iter().find(|b| b.open() && b.legacy)
        {
            row_b.id = b.id.clone();
            row_b.replace = true;
        }
        if !row_b.replace && same_q.is_none() && count_open(t) >= MAX_OPEN_BLOCKERS {
            return Err(invalid(format!(
                "task {} already has {MAX_OPEN_BLOCKERS} open blockers",
                t.id
            )));
        }
        let bid = row_b.id.clone();
        self.write(
            by,
            kinds::TASK_STATUS,
            &TaskStateRow {
                task_id: t.id.clone(),
                state: BLOCKED.into(),
                note: t.note.clone(),
                reason: reason.into(),
                blocked_by: nb,
                blocker: Some(row_b),
                by: by.into(),
                at: now,
                ..TaskStateRow::default()
            },
        )?;
        let nt = self.tasks[&t.id].clone();
        let i = blocker_index(&nt, &bid).or_else(|| open_with_question(&nt, q));
        let out = i.map(|i| nt.blockers[i].clone()).unwrap_or_default();
        Ok((nt, out))
    }

    /// Revises open item bid: what it asks now, its context and its
    /// options (none makes it a plain question). The raiser, a holder or a
    /// person may. The person's earlier answer stays in the thread, and the
    /// item waits on the person again.
    pub(crate) fn revise(
        &mut self,
        by: &str,
        id: &str,
        bid: &str,
        r: BlockerRevision,
    ) -> Result<(Task, Blocker)> {
        let by = caller(by)?;
        let bid = trim_space(bid).to_string();
        let t = self.open_task(id)?.clone();
        let Some(i) = blocker_index(&t, &bid) else {
            return Err(not_found(format!(
                "task {id} has no blocker {}",
                quote(&bid)
            )));
        };
        let b = t.blockers[i].clone();
        if !b.open() {
            return Err(bad_state(format!("blocker {bid} is already resolved")));
        }
        let mut q = trim_space(&r.question).to_string();
        if q.is_empty() {
            q = b.question.clone();
        }
        let cx = match &r.context {
            Some(c) => trim_space(c).to_string(),
            None => b.context.clone(),
        };
        check_text("the question", &q, 1, MAX_QUESTION)?;
        // A done check's context is the agent's proof, up to MAX_OBJECTIVE
        // (card #140).
        let max_cx = if b.done_check {
            MAX_OBJECTIVE
        } else {
            MAX_CONTEXT
        };
        check_text("the context", &cx, 0, max_cx)?;
        let now = self.stamp();
        let opts = clean_options(&r.options, &by, now)?;
        if q == b.question && cx == b.context && same_options(&opts, &b.options) {
            return Err(invalid(format!(
                "that asks the same question with the same options: answer the person on the blocker (board_comment blocker_id={bid}), then resolve it (board_unblock) or revise it with options that fit what you know now, or none"
            )));
        }
        self.write(
            &by,
            kinds::TASK_STATUS,
            &TaskStateRow {
                task_id: t.id.clone(),
                state: BLOCKED.into(),
                note: t.note.clone(),
                reason: q.clone(),
                blocked_by: b.on.clone(),
                blocker: Some(BlockerRow {
                    id: bid.clone(),
                    question: q,
                    context: cx,
                    options: opts,
                    replace: true,
                    revise: true,
                    on: b.on.clone(),
                    links: b.links.clone(),
                    attachments: b.attachments.clone(),
                    ..BlockerRow::default()
                }),
                by: by.clone(),
                at: now,
                ..TaskStateRow::default()
            },
        )?;
        let nt = self.tasks[&t.id].clone();
        let out = blocker_index(&nt, &bid)
            .map(|i| nt.blockers[i].clone())
            .unwrap_or_default();
        Ok((nt, out))
    }

    pub(crate) fn recommend(
        &mut self,
        by: &str,
        id: &str,
        bid: &str,
        answer: &str,
        why: &str,
    ) -> Result<Task> {
        let by = caller(by)?;
        let answer = trim_space(answer).to_string();
        let why = trim_space(why).to_string();
        check_text("the answer", &answer, 1, MAX_ANSWER)?;
        check_text("the why", &why, 0, MAX_WHY)?;
        let t = self.open_task(id)?.clone();
        let Some(i) = blocker_index(&t, trim_space(bid)) else {
            return Err(not_found(format!(
                "task {id} has no blocker {}",
                quote(bid)
            )));
        };
        let b = &t.blockers[i];
        if !b.open() {
            return Err(bad_state(format!("blocker {} is already resolved", b.id)));
        }
        match option_index(&b.options, &answer) {
            Some(j) if b.options[j].recommended => return Ok(t),
            None if b.options.len() >= MAX_OPTIONS => {
                return Err(invalid(format!(
                    "blocker {} already has {MAX_OPTIONS} recommended answers",
                    b.id
                )));
            }
            _ => {}
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::BLOCKER_OPTION,
            &BlockerOptionRow {
                task_id: id.into(),
                blocker_id: b.id.clone(),
                answer,
                why,
                by: by.clone(),
                at,
            },
        )?;
        Ok(self.tasks[id].clone())
    }

    /// Puts a card nobody holds back on the Board when the person answers on
    /// it (spawned agents §2, card #447): a "Not done" on its done check, or
    /// an answer to a blocker, waits on an agent, and with no holder there is
    /// nobody to act on it. SUBMITTED makes it offerable (and, on a spawning
    /// board, starts a fresh agent); the answer stays in the blocker's
    /// thread and Left off is kept, for whoever picks it up.
    pub(crate) fn back_on_board(&mut self, by: &str, id: &str, why: &str) -> Result<bool> {
        let Some(t) = self.tasks.get(id) else {
            return Ok(false);
        };
        if !t.holders.is_empty()
            || !t.co_holders.is_empty()
            || !t.assigned.is_empty()
            || is_terminal(&t.state)
            || t.state == SUBMITTED
            || t.state == WORKING
            || t.open_ask().is_some()
            || !t.subtasks.is_empty()
        {
            return Ok(false);
        }
        let at = self.stamp();
        self.write(
            by,
            kinds::TASK_STATUS,
            &TaskStateRow {
                task_id: id.into(),
                state: SUBMITTED.into(),
                note: format!("back on the Board: {why}"),
                by: by.into(),
                at,
                ..TaskStateRow::default()
            },
        )?;
        Ok(true)
    }

    pub(crate) fn answer(
        &mut self,
        by: &str,
        id: &str,
        bid: &str,
        a: BlockerAnswer,
    ) -> Result<(Task, Vec<Notice>)> {
        let by = caller(by)?;
        let ms = self.check_mentions(&by, &a.mentions)?;
        if !a.resolve && !self.is_owner(&by) {
            return Err(invalid(
                "only the person answers a blocker; resolve it (with a note once the person answered) or reply on it",
            ));
        }
        let mut text = trim_space(&a.text).to_string();
        if a.files.len() > MAX_ATTACHMENTS {
            return Err(invalid(format!("at most {MAX_ATTACHMENTS} attachments")));
        }
        if a.resolve && !a.files.is_empty() {
            return Err(invalid(
                "files go with an answer, not a resolve; answer with them first",
            ));
        }
        let t = self.open_task(id)?.clone();
        let Some(i) = blocker_index(&t, trim_space(bid)) else {
            return Err(not_found(format!(
                "task {id} has no blocker {}",
                quote(bid)
            )));
        };
        let b = t.blockers[i].clone();
        if !b.open() {
            return Err(bad_state(format!("blocker {} is already resolved", b.id)));
        }
        // An ownership item (card #432): Accept or Decline moves the card
        // (or not) and resolves it; only the person it asks may.
        if b.transfer
            && self
                .pending_transfer(&t)
                .is_some_and(|tr| tr.blocker_id == b.id)
        {
            let picked = usize::try_from(a.pick - 1)
                .ok()
                .and_then(|i| b.options.get(i))
                .map(|o| o.answer.clone());
            match picked.as_deref() {
                Some(super::owner::OWNER_ACCEPT) | Some(super::owner::OWNER_DECLINE) => {
                    let accept = picked.as_deref() == Some(super::owner::OWNER_ACCEPT);
                    self.answer_owner(&by, id, &b.id, accept)?;
                    return self.answer(&by, id, bid, BlockerAnswer { resolve: true, ..a });
                }
                _ if !self.is_owner(&by) => {
                    return Err(not_allowed(
                        "an ownership change is the person's to accept or decline: reply on it instead",
                    ));
                }
                _ => {}
            }
        }
        let last = if a.resolve {
            last_answer(self, &t, &b.id)
        } else {
            None
        };
        let (mut opt, mut rec_by, mut rec, mut note) = (-1i64, String::new(), false, String::new());
        if let Some(last) = last.as_ref().filter(|_| a.pick == 0) {
            if !self.is_owner(&by) {
                if text.is_empty() {
                    return Err(invalid(
                        "the person already answered; give note: one line on what you will do now",
                    ));
                }
                if multiline(&text) {
                    return Err(invalid("the note is one line"));
                }
            }
            if self.is_owner(&by) {
                super::comments::check_person_text("the note", &text, 0)?;
            } else {
                check_text("the note", &text, 0, MAX_ANSWER)?;
            }
            note = text;
            text = last.text.clone();
            opt = last.pick - 1;
            if opt >= b.options.len() as i64 {
                opt = -1;
            }
        } else if last.is_some() && !self.is_owner(&by) {
            return Err(invalid(
                "the person already answered; give note: one line on what you will do now",
            ));
        } else if a.pick < 0 || a.pick > b.options.len() as i64 {
            return Err(invalid(format!(
                "blocker {} has {} answers to pick from, not {}",
                b.id,
                b.options.len(),
                a.pick
            )));
        } else if a.pick > 0 {
            opt = a.pick - 1;
            let o = &b.options[opt as usize];
            text = o.answer.clone();
            rec_by = o.by.clone();
            rec = o.recommended;
        } else if self.is_owner(&by) {
            super::comments::check_person_text("the answer", &text, 1)?;
        } else {
            check_text("the answer", &text, 1, MAX_OWN_ANSWER)?;
        }
        let mut n = Notice {
            kind: NOTICE_ANSWERED.into(),
            task_id: id.into(),
            reference: format!("task:{id}"),
            title: t.title.clone(),
            by: by.clone(),
            wake: a.wake,
            ..Notice::default()
        };
        let r = &b.raised_by;
        if *r != by && !r.is_empty() && !self.is_owner(r) && r != SWEEPER {
            n.to = vec![r.clone()];
        }
        for h in self.recipients(&t, &by) {
            if !n.to.contains(&h) {
                n.to.push(h);
            }
        }
        // A mentioned raiser or holder hears this notice itself, woken
        // (mention_into), not a plain mention without the question.
        if !a.resolve {
            let at = self.stamp();
            let actor = self.actor_for(&by);
            self.write_comment(
                &actor,
                TaskCommentRow {
                    task_id: id.into(),
                    comment_id: new_id(&at),
                    by: by.clone(),
                    to: b.raised_by.clone(),
                    text: text.clone(),
                    wake: a.wake,
                    blocker_id: b.id.clone(),
                    pick: opt + 1,
                    at,
                    mentions: ms.clone(),
                    attachments: a.files.clone(),
                    ..TaskCommentRow::default()
                },
            )?;
            n.attachments = a.files.clone();
            let mut s = format!(
                "{} answered the blocker on {} ({}, blocker {}).\nQuestion: {}\nAnswer: {}",
                self.named(&by),
                quote(&t.title),
                t.refs(),
                b.id,
                b.question,
                cut_show(&text, MAX_NOTICE_FIELD, &format!("task:{id}"))
            );
            s.push_str(&picked_line(opt, &rec_by, rec));
            if b.done_check {
                // Card #476: Not done on a done check. The answer goes on
                // its thread, where the person reads it beside their
                // reason, and the next check says what changed.
                s.push_str(&format!(
                    "\nThat is a Not done on your done check: it goes back to you. Answer them on it now, in your own words, so they see you heard them: what you'll do about it (board_comment task_id={id} blocker_id={bid}). When it is done, set completed again with proof that says what changed since; that check replaces this one, and completed is refused until you have answered here or if the proof is the same.",
                    bid = b.id
                ));
                s.push_str(&deeper(id));
                n.text = s;
                let mut mn =
                    self.mention_notice(&by, id, &ms, &text, &format!("task:{id}"), "")?;
                super::mention::mention_into(&mut n, &mut mn);
                let mut ns = vec![n];
                ns.extend(mn);
                if self.is_owner(&by) {
                    self.back_on_board(&by, id, "the person answered and nobody is on it")?;
                }
                return Ok((self.tasks[id].clone(), ns));
            }
            if opt < 0 && !b.options.is_empty() {
                s.push_str(&format!(
                    "\nThe person answered in their own words, so the options you offered are set aside: their window shows only the thread and a reply box. Answer them in the thread (board_comment task_id={id} blocker_id={}). Once you can see the choices that fit now, revise with board_block task_id={id} blocker_id={} with new options and your recommendation marked, or none to keep it a conversation.",
                    b.id, b.id
                ));
            }
            s.push_str(&format!(
                "\nIt is not resolved yet. When you have what you need, resolve it now, in the turn you act on it: board_unblock task_id={id} blocker_id={} note=\"<one line on what you will do now>\"; an answered blocker left open waits on you. If the answer doesn't settle it: board_comment task_id={id} blocker_id={} with missing (what you still need from them), which puts it before the person again. If it is a question or changes what is being decided, answer it there, then resolve the blocker if that settles it, or revise it with board_block task_id={id} blocker_id={} with the question and options that fit now (or none); never offer the same options again.",
                b.id, b.id, b.id
            ));
            s.push_str(&deeper(id));
            n.text = s;
            let mut mn = self.mention_notice(&by, id, &ms, &text, &format!("task:{id}"), "")?;
            super::mention::mention_into(&mut n, &mut mn);
            let mut ns = vec![n];
            ns.extend(mn);
            if self.is_owner(&by) {
                self.back_on_board(&by, id, "the person answered and nobody is on it")?;
            }
            return Ok((self.tasks[id].clone(), ns));
        }
        let restored = if count_open(&t) == 1 {
            self.restore_for(&t).to_string()
        } else {
            String::new()
        };
        let at = self.stamp();
        self.write(
            &by,
            kinds::BLOCKER_RESOLVED,
            &BlockerResolvedRow {
                task_id: id.into(),
                blocker_id: b.id.clone(),
                answer: text.clone(),
                option: opt,
                by: by.clone(),
                restored: restored.clone(),
                note: note.clone(),
                at,
                ..BlockerResolvedRow::default()
            },
        )?;
        let nt = self.tasks[id].clone();
        let mut s = String::new();
        if last.is_some() {
            s.push_str(&format!(
                "{} resolved blocker {} on {} ({})",
                self.named(&by),
                b.id,
                quote(&t.title),
                t.refs()
            ));
            if !note.is_empty() {
                s.push_str(&format!(": {note}"));
            }
            s.push_str(&format!(
                "\nQuestion: {}\nThe person's answer: {}",
                b.question,
                cut_show(&text, MAX_NOTICE_FIELD, &format!("task:{id}"))
            ));
        } else {
            s.push_str(&format!(
                "{} answered the blocker on {} ({}, blocker {}).\nQuestion: {}\nAnswer: {}",
                self.named(&by),
                quote(&t.title),
                t.refs(),
                b.id,
                b.question,
                cut_show(&text, MAX_NOTICE_FIELD, &format!("task:{id}"))
            ));
            s.push_str(&picked_line(opt, &rec_by, rec));
        }
        if !restored.is_empty() {
            s.push_str(&format!("\nNo blockers remain: it is {} again.", nt.state));
        } else {
            s.push_str(&format!(
                "\n{} more blockers are still open.",
                count_open(&nt)
            ));
        }
        s.push_str(&deeper(id));
        n.text = s;
        let said = if note.is_empty() { &text } else { &note };
        let mut mn = self.mention_notice(&by, id, &ms, said, &format!("task:{id}"), "")?;
        super::mention::mention_into(&mut n, &mut mn);
        let mut ns = vec![n];
        ns.extend(mn);
        Ok((self.tasks[id].clone(), ns))
    }

    /// The person's Done on a done check (card #315): one completed row,
    /// which answers the check Done as it folds, so the card goes from
    /// waiting on the person straight to completed, never through working.
    /// The notice says it is completed.
    pub(crate) fn say_done(
        &mut self,
        by: &str,
        id: &str,
        bid: &str,
        mentions: &[Mention],
    ) -> Result<(Task, Vec<Notice>)> {
        let by = caller(by)?;
        if !self.is_owner(&by) {
            return Err(not_allowed("only the person says Done on a done check"));
        }
        let ms = self.check_mentions(&by, mentions)?;
        let t = self.open_task(id)?.clone();
        let Some(b) = t.blockers.iter().find(|b| b.id == bid).cloned() else {
            return Err(not_found(format!(
                "task {id} has no blocker {}",
                quote(bid)
            )));
        };
        if !b.open() {
            return Err(bad_state(format!("blocker {} is already resolved", b.id)));
        }
        if !b.done_check || !b.options.iter().any(|o| o.answer == DONE_ANSWER) {
            return Err(invalid(format!("blocker {} isn't a done check", b.id)));
        }
        let t = self.set_task_state(&by, id, COMPLETED, "")?;
        let mut n = Notice {
            kind: NOTICE_ANSWERED.into(),
            task_id: id.into(),
            reference: format!("task:{id}"),
            title: t.title.clone(),
            by: by.clone(),
            ..Notice::default()
        };
        let r = &b.raised_by;
        if *r != by && !r.is_empty() && !self.is_owner(r) && r != SWEEPER {
            n.to = vec![r.clone()];
        }
        for h in self.recipients(&t, &by) {
            if !n.to.contains(&h) {
                n.to.push(h);
            }
        }
        n.text = format!(
            "The person said Done on {} ({}): it is completed.{}",
            quote(&t.title),
            t.refs(),
            deeper(id)
        );
        let mut mn = self.mention_notice(&by, id, &ms, DONE_ANSWER, &format!("task:{id}"), "")?;
        super::mention::mention_into(&mut n, &mut mn);
        let mut ns = vec![n];
        ns.extend(mn);
        Ok((t, ns))
    }

    pub(crate) fn comment_on(
        &mut self,
        by: &str,
        id: &str,
        input: CommentInput,
    ) -> Result<(Comment, Vec<Notice>)> {
        if !trim_space(&input.verdict).is_empty() {
            if !input.mentions.is_empty() {
                return Err(invalid(
                    "a verdict can't carry mentions: comment separately to mention someone",
                ));
            }
            let (cm, n) = self.review_verdict(by, id, input)?;
            return Ok((cm, vec![n]));
        }
        let by = caller(by)?;
        let ms = self.check_mentions(&by, &input.mentions)?;
        let bid = trim_space(&input.blocker_id).to_string();
        if bid.is_empty() {
            let (cm, mut n) =
                self.add_comment_full(&by, id, &input.text, input.wake, input.files, "", &ms)?;
            super::mention::without_mentioned(&mut n, &ms);
            let mut ns = vec![n];
            ns.extend(self.mention_notice(
                &by,
                id,
                &ms,
                &cm.text,
                &format!("comment:{}", cm.id),
                "",
            )?);
            return Ok((cm, ns));
        }
        if self.is_owner(&by) {
            return Err(invalid(
                "a person answers a blocker instead of replying on it",
            ));
        }
        let text = trim_space(&input.text).to_string();
        check_text("the reply", &text, 1, MAX_COMMENT)?;
        if input.missing.len() > MAX_MISSING {
            return Err(invalid(format!("at most {MAX_MISSING} missing lines")));
        }
        let mut missing = Vec::new();
        for m in &input.missing {
            let m = trim_space(m).to_string();
            check_text("a missing line", &m, 1, MAX_MISSING_TEXT)?;
            missing.push(m);
        }
        if input.files.len() > MAX_ATTACHMENTS {
            return Err(invalid(format!("at most {MAX_ATTACHMENTS} attachments")));
        }
        let t = self.any_task(id)?;
        let Some(i) = blocker_index(t, &bid) else {
            return Err(not_found(format!(
                "task {id} has no blocker {}",
                quote(&bid)
            )));
        };
        // The done check the person sent back stays answerable after the
        // next check (or the Board) closed it (card #476): a fresh agent
        // answers their reason there.
        if !t.blockers[i].open() && sent_back(self, t).is_none_or(|sb| sb.id != bid) {
            return Err(bad_state(format!("blocker {bid} is already resolved")));
        }
        let at = self.stamp();
        let actor = self.actor_for(&by);
        let cm = self.write_comment(
            &actor,
            TaskCommentRow {
                task_id: id.into(),
                comment_id: new_id(&at),
                by,
                text,
                at,
                attachments: input.files,
                blocker_id: bid,
                missing,
                mentions: ms.clone(),
                ..TaskCommentRow::default()
            },
        )?;
        let mut ns = vec![Notice::default()];
        ns.extend(self.mention_notice(
            &cm.by,
            id,
            &ms,
            &cm.text,
            &format!("comment:{}", cm.id),
            "",
        )?);
        Ok((cm, ns))
    }

    // ---- steps ----

    pub(crate) fn set_steps(&mut self, by: &str, id: &str, mut input: Vec<Step>) -> Result<Task> {
        let by = caller(by)?;
        check_steps(&mut input)?;
        let t = self.open_task(id)?.clone();
        if !t.subtasks.is_empty() {
            return Err(subtask(format!(
                "{} is a parent: its work lives in its subtasks (board_create parent={} adds one)",
                t.short_ref(),
                t.short_ref()
            )));
        }
        let may_spawn = self.may_spawn_by(&t, &by);
        let mut seq = self.step_seq.get(id).copied().unwrap_or(0);
        let mut used: Vec<String> = Vec::new();
        let mut out = Vec::with_capacity(input.len());
        for s in &input {
            let mut sid = String::new();
            if !s.id.is_empty() && t.steps.iter().any(|o| o.id == s.id) && !used.contains(&s.id) {
                sid = s.id.clone();
            }
            if sid.is_empty()
                && let Some(o) = t.steps.iter().find(|o| {
                    o.text == s.text && !used.contains(&o.id) && !input.iter().any(|x| x.id == o.id)
                })
            {
                sid = o.id.clone();
            }
            if sid.is_empty() {
                seq += 1;
                sid = format!("s{seq}");
            }
            used.push(sid.clone());
            // A mark stays once set (spawned agents §3); only the card's
            // owner's side adds one.
            let had = t.steps.iter().any(|o| o.id == sid && o.spawn);
            out.push(Step {
                spawn: had || (s.spawn && may_spawn),
                id: sid,
                text: s.text.clone(),
                done: s.done,
                doing: s.doing,
                owner: self.check_owner(&s.owner)?,
            });
        }
        if out == t.steps {
            return Ok(t);
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_STEPS,
            &TaskStepsRow {
                task_id: id.into(),
                steps: out,
                by: by.clone(),
                at,
            },
        )?;
        Ok(self.tasks[id].clone())
    }

    /// Where the holder's work on card id is (card #137), written by the
    /// holder's device only when the place changed (not when only the time
    /// it was seen did), so a sweep doesn't fill the log.
    pub(crate) fn set_where(&mut self, id: &str, w: Where) -> Result<bool> {
        let t = self.open_task(id)?.clone();
        if let Some(o) = &t.where_ {
            // A git place is never replaced by a place outside git: a new
            // holder still in its home folder, or one that stepped out,
            // mustn't hide where the work is.
            if o.same_place(&w) || (w.worktree.is_empty() && !o.worktree.is_empty()) {
                return Ok(false);
            }
        }
        let at = self.stamp();
        self.write(
            SWEEPER,
            kinds::TASK_WHERE,
            &TaskWhereRow {
                task_id: id.into(),
                who: w.who,
                cwd: w.cwd,
                worktree: w.worktree,
                branch: w.branch,
                head: w.head,
                pushed: w.pushed,
                by: SWEEPER.into(),
                at,
            },
        )?;
        Ok(true)
    }

    pub(crate) fn set_step(
        &mut self,
        by: &str,
        id: &str,
        sid: &str,
        u: StepUpdate,
        left_off: &str,
    ) -> Result<Task> {
        let left_off = trim_space(left_off).to_string();
        if runes(&left_off) > MAX_LEFT_OFF {
            return Err(invalid(format!(
                "left_off is one line of at most {MAX_LEFT_OFF} characters"
            )));
        }
        let by = caller(by)?;
        let sid = trim_space(sid);
        if u.done.is_none() && u.doing.is_none() {
            return Err(invalid("give done or doing"));
        }
        if u.done == Some(true) && u.doing == Some(true) {
            return Err(invalid("a step can't be done and in progress"));
        }
        let t = self.open_task(id)?.clone();
        let Some(i) = t.steps.iter().position(|s| s.id == sid) else {
            return Err(not_found(format!("task {id} has no step {}", quote(sid))));
        };
        let new_left_off =
            !left_off.is_empty() && t.left_off.as_ref().is_none_or(|l| l.text != left_off);
        if apply_step(&t.steps, i, u) == t.steps && !new_left_off {
            return Ok(t);
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_STEP,
            &TaskStepRow {
                task_id: id.into(),
                step_id: sid.into(),
                done: u.done,
                doing: u.doing,
                left_off,
                by: by.clone(),
                at,
                ..TaskStepRow::default()
            },
        )?;
        Ok(self.tasks[id].clone())
    }

    /// A step owner: an agent's key, or "" for none. The person and Irori
    /// don't own steps.
    pub(crate) fn check_owner(&self, owner: &str) -> Result<String> {
        let o = trim_space(owner).to_string();
        if o.is_empty() {
            return Ok(o);
        }
        check_key(&o)?;
        if self.is_owner(&o) || o == SWEEPER {
            return Err(invalid("a step's owner is an agent, not the person"));
        }
        Ok(o)
    }

    /// Gives step `sid` of card `id` to `owner` ("" takes it off anyone);
    /// whoever may set the card's steps may (card #86).
    pub(crate) fn set_step_owner(
        &mut self,
        by: &str,
        id: &str,
        sid: &str,
        owner: &str,
    ) -> Result<(Task, Vec<Notice>)> {
        let by = caller(by)?;
        let sid = trim_space(sid);
        let owner = self.check_owner(owner)?;
        let t = self.open_task(id)?.clone();
        let Some(s) = t.steps.iter().find(|s| s.id == sid) else {
            return Err(not_found(format!("task {id} has no step {}", quote(sid))));
        };
        if s.owner == owner {
            return Ok((t, Vec::new()));
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_STEP,
            &TaskStepRow {
                task_id: id.into(),
                step_id: sid.into(),
                owner: Some(owner),
                by: by.clone(),
                at,
                ..TaskStepRow::default()
            },
        )?;
        let after = self.tasks[id].clone();
        let ns = self.owner_notices(&by, &t, &after);
        Ok((after, ns))
    }

    /// [`Self::set_steps`] and a notice for each agent it newly gives a
    /// step to.
    pub(crate) fn set_steps_notice(
        &mut self,
        by: &str,
        id: &str,
        input: Vec<Step>,
    ) -> Result<(Task, Vec<Notice>)> {
        let before = self.open_task(id)?.clone();
        let after = self.set_steps(by, id, input)?;
        let by = trim_space(by).to_string();
        let ns = self.owner_notices(&by, &before, &after);
        Ok((after, ns))
    }

    /// One notice per agent that owns an open step on `after` it didn't
    /// own on `before`: woken when idle, else at its next turn.
    fn owner_notices(&self, by: &str, before: &Task, after: &Task) -> Vec<Notice> {
        let mut by_owner: Vec<(String, Vec<&Step>)> = Vec::new();
        for s in after
            .steps
            .iter()
            .filter(|s| !s.done && !s.owner.is_empty())
        {
            let had = before
                .steps
                .iter()
                .any(|o| o.id == s.id && o.owner == s.owner);
            if had || s.owner == by || self.is_owner(&s.owner) {
                continue;
            }
            match by_owner.iter_mut().find(|(o, _)| *o == s.owner) {
                Some((_, v)) => v.push(s),
                None => by_owner.push((s.owner.clone(), vec![s])),
            }
        }
        by_owner
            .into_iter()
            .map(|(o, ss)| {
                let list: Vec<String> = ss
                    .iter()
                    .map(|s| format!("{} {}", s.id, quote(&s.text)))
                    .collect();
                let holder = if after.holder.is_empty() {
                    "nobody".to_string()
                } else {
                    self.named(&after.holder)
                };
                Notice {
                    kind: NOTICE_STEP_OWNER.into(),
                    task_id: after.id.clone(),
                    reference: format!("task:{}", after.id),
                    title: after.title.clone(),
                    to: vec![o.clone()],
                    by: by.into(),
                    wake: self.idle(&o),
                    text: format!(
                        "{} gave you {} on {} ({}): {}. You hear the card like a holder until your steps are done; tick each with board_step. {holder} holds the card.{}",
                        self.named(by),
                        if ss.len() == 1 { "a step" } else { "steps" },
                        quote(&after.title),
                        after.refs(),
                        list.join(", "),
                        deeper(&after.id)
                    ),
                    ..Notice::default()
                }
            })
            .collect()
    }

    // ---- watch ----

    pub(crate) fn watch(&mut self, by: &str, id: &str, on: bool) -> Result<Task> {
        let by = caller(by)?;
        if self.is_owner(&by) || by == SWEEPER {
            return Err(invalid("people see every card; only agents watch"));
        }
        check_key(&by)?;
        let t = self.any_task(id)?;
        if contains(&t.watchers, &by) == on {
            return Ok(t.clone());
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_WATCHED,
            &TaskWatchedRow {
                task_id: id.into(),
                who: by.clone(),
                watching: on,
                by: by.clone(),
                at,
            },
        )?;
        Ok(self.tasks[id].clone())
    }

    /// A task's newest activity lines, newest first.
    pub(crate) fn activity_of(&self, id: &str) -> Option<Vec<Activity>> {
        self.tasks.get(id)?;
        let mut out = self.activity.get(id).cloned().unwrap_or_default();
        out.reverse();
        Some(out)
    }

    /// An open task, or NotFound / BadState.
    pub(crate) fn open_task(&self, id: &str) -> Result<&Task> {
        let Some(t) = self.tasks.get(id) else {
            return Err(not_found(format!("task {}", quote(id))));
        };
        if is_terminal(&t.state) {
            return Err(bad_state(format!("task {id} is {}", t.state)));
        }
        Ok(t)
    }

    /// A task in any state.
    pub(crate) fn any_task(&self, id: &str) -> Result<&Task> {
        self.tasks
            .get(id)
            .ok_or_else(|| not_found(format!("task {}", quote(id))))
    }

    /// The creator, any holder or a person.
    /// board_edit: the holder, the creator or the person renames a card or
    /// changes its objective or done-when, so it says what it is for now.
    pub(crate) fn edit_task(&mut self, by: &str, id: &str, e: TaskEdit) -> Result<Task> {
        let by = caller(by)?;
        let title = e.title.map(|s| trim_space(&s).to_string());
        let obj = e.objective.map(|s| trim_space(&s).to_string());
        let done = e.done_when.map(|s| trim_space(&s).to_string());
        if title.is_none() && obj.is_none() && done.is_none() {
            return Err(invalid(
                "give title, objective or done_when: what the card is now",
            ));
        }
        if title.as_deref() == Some("") {
            return Err(invalid("a card needs a title"));
        }
        if title
            .as_deref()
            .is_some_and(|t| runes(t) > super::MAX_TITLE)
        {
            return Err(invalid(format!(
                "title longer than {} characters",
                super::MAX_TITLE
            )));
        }
        if [&obj, &done].iter().any(|x| {
            x.as_deref()
                .is_some_and(|v| runes(v) > super::MAX_OBJECTIVE)
        }) {
            return Err(invalid(format!(
                "objective or done_when longer than {} characters",
                super::MAX_OBJECTIVE
            )));
        }
        let t = self.open_task(id)?.clone();
        if t.open_ask().is_some() {
            return Err(bad_state(format!(
                "{} is an open ask: ask_close it with outcome card and the title it should have",
                t.short_ref()
            )));
        }
        let same = title.as_ref().is_none_or(|v| *v == t.title)
            && obj.as_ref().is_none_or(|v| *v == t.objective)
            && done.as_ref().is_none_or(|v| *v == t.done_criteria);
        if same {
            return Ok(t);
        }
        let at = self.stamp();
        let mut from = self.thread_from(&by, &e.from)?;
        if from == id {
            from.clear();
        }
        self.write(
            &by,
            self.ask_kind(kinds::TASK_EDITED),
            &TaskEditedRow {
                task_id: id.into(),
                title: title.clone(),
                objective: obj.clone(),
                done_criteria: done.clone(),
                by: by.clone(),
                from,
                at,
                ..TaskEditedRow::default()
            },
        )?;
        // The fold may hold an edit back (card #318): say so, never "Edited".
        let now = &self.tasks[id];
        let landed = title.as_ref().is_none_or(|v| *v == now.title)
            && obj.as_ref().is_none_or(|v| *v == now.objective)
            && done.as_ref().is_none_or(|v| *v == now.done_criteria);
        if !landed {
            return Err(super::error::Error::raw(
                super::error::ErrorKind::BadState,
                format!(
                    "coord: edit {}: the change didn't apply, so the card still says what it did; tell the person on the card",
                    now.short_ref()
                ),
            ));
        }
        Ok(now.clone())
    }
}

/// Trims and checks a raise's options.
fn clean_options(input: &[BlockerOption], by: &str, at: GoTime) -> Result<Vec<BlockerOption>> {
    if input.len() > MAX_OPTIONS {
        return Err(invalid(format!(
            "more than {MAX_OPTIONS} recommended answers"
        )));
    }
    let mut out: Vec<BlockerOption> = Vec::new();
    for o in input {
        let a = trim_space(&o.answer).to_string();
        let why = trim_space(&o.why).to_string();
        check_text("a recommended answer", &a, 1, MAX_ANSWER)?;
        check_text("a recommendation's why", &why, 0, MAX_WHY)?;
        let mut link = None;
        if let Some(l) = &o.link
            && !trim_space(&l.reference).is_empty()
        {
            link = Some(parse_link(&l.reference)?);
        }
        if let Some(j) = option_index(&out, &a) {
            out[j].recommended = out[j].recommended || o.recommended;
            if out[j].link.is_none() {
                out[j].link = link;
            }
            continue;
        }
        out.push(BlockerOption {
            answer: a,
            why,
            by: by.into(),
            at,
            recommended: o.recommended,
            link,
        });
    }
    if out.iter().filter(|o| o.recommended).count() > 1 {
        return Err(invalid("mark at most one answer as recommended"));
    }
    Ok(out)
}

/// Parses a raise's links and checks its files' count.
fn clean_blocker_links(refs: &[String], files: &[FileRef]) -> Result<Vec<Link>> {
    let mut out = Vec::new();
    for r in refs {
        if trim_space(r).is_empty() {
            continue;
        }
        let l = parse_link(r)?;
        out = add_links(out, &[l]);
    }
    if out.len() > MAX_BLOCKER_LINKS {
        return Err(invalid(format!(
            "at most {MAX_BLOCKER_LINKS} links on a blocker"
        )));
    }
    if files.len() > MAX_ATTACHMENTS {
        return Err(invalid(format!("at most {MAX_ATTACHMENTS} attachments")));
    }
    Ok(out)
}

/// A watch digest's text for `lines` ("<actor> <what>") and `dropped`
/// earlier changes (byte-exact: `merge_watch` parses it, also from Go).
pub fn watch_text(t: &Task, lines: &[String], dropped: usize) -> String {
    let mut b = format!(
        "{WATCH_HEAD}{} ({}){WATCH_HEAD_END}",
        quote(&t.title),
        t.refs()
    );
    let shown = &lines[lines.len().saturating_sub(MAX_WATCH_LINES)..];
    let more = lines.len() - shown.len() + dropped;
    if more > 0 {
        b.push_str(&earlier_line(more));
    }
    for l in shown {
        b.push_str(&format!("- {}\n", cut(l, MAX_NOTICE_FIELD)));
    }
    b.push_str(&format!("Now: {}", effective_state(t)));
    if let Some(r) = &t.rollup {
        b.push_str(&format!(", subtasks {}/{} done", r.done, r.total));
    } else {
        let (d, n) = t.progress();
        if n > 0 {
            b.push_str(&format!(", steps {d}/{n}"));
        }
    }
    let n = count_open(t);
    if n > 0 {
        b.push_str(&format!(", {n} open blockers"));
    }
    b.push('.');
    b.push_str(&deeper(&t.id));
    b
}

/// Folds an earlier unread watch digest into a newer one for the same
/// card: the newer head and "Now:" line, the earlier changes first, at most
/// eight lines (the rest counted). `None` when either isn't a digest.
pub fn merge_watch(earlier: &str, newer: &str) -> Option<String> {
    let (o_lines, o_more, _, _) = watch_parts(earlier)?;
    let (n_lines, n_more, head, foot) = watch_parts(newer)?;
    let mut lines = o_lines;
    lines.extend(n_lines);
    let mut more = o_more + n_more;
    if lines.len() > MAX_WATCH_LINES {
        more += lines.len() - MAX_WATCH_LINES;
        lines.drain(..lines.len() - MAX_WATCH_LINES);
    }
    let mut b = head;
    if more > 0 {
        b.push_str(&earlier_line(more));
    }
    for l in lines {
        b.push_str(&l);
    }
    b.push_str(&foot);
    Some(b)
}

fn watch_parts(s: &str) -> Option<(Vec<String>, usize, String, String)> {
    let i = s.find(WATCH_HEAD_END)?;
    let j = s.rfind(WATCH_FOOT)?;
    if !s.starts_with(WATCH_HEAD) || j + 1 < i + WATCH_HEAD_END.len() {
        return None;
    }
    let head = s[..i + WATCH_HEAD_END.len()].to_string();
    let foot = s[j + 1..].to_string();
    let mut lines: Vec<String> = Vec::new();
    let mut more = 0usize;
    let body = &s[i + WATCH_HEAD_END.len()..j + 1];
    for l in body.split_inclusive('\n') {
        if l.is_empty() {
            continue;
        }
        if l.starts_with("- (") && lines.is_empty() && more == 0 {
            if let Some(n) = scan_earlier(l) {
                more = n;
                continue;
            }
            lines.push(l.into());
        } else if !l.starts_with("- ") && !lines.is_empty() {
            let last = lines.len() - 1;
            lines[last].push_str(l);
        } else {
            lines.push(l.into());
        }
    }
    Some((lines, more, head, foot))
}

/// `fmt.Sscanf(l, "- (%d earlier changes)\n", &n)`.
fn scan_earlier(l: &str) -> Option<usize> {
    let rest = l.strip_prefix("- (")?;
    let rest = rest.trim_start_matches([' ', '\t', '\r']);
    let end = rest
        .char_indices()
        .find(|(i, c)| !(c.is_ascii_digit() || *i == 0 && (*c == '+' || *c == '-')))
        .map(|(i, _)| i)
        .unwrap_or(rest.len());
    let n: i64 = rest[..end].parse().ok()?;
    let tail = &rest[end..];
    if !tail.starts_with(" earlier changes)") {
        return None;
    }
    usize::try_from(n).ok()
}

/// What a steps row changed, from was to now: the steps named, so the
/// thread shows the plan and how it moved, not only a count.
fn steps_change(was: &[Step], now: &[Step]) -> String {
    let q = |s: &Step| {
        format!(
            "“{}”",
            super::util::cut(&s.text, super::asks::MAX_LOG_QUOTE)
        )
    };
    if was.is_empty() {
        let names: Vec<String> = now.iter().map(q).collect();
        return if names.is_empty() {
            "set the steps".into()
        } else {
            format!("set the steps: {}", names.join(", "))
        };
    }
    let mut parts: Vec<String> = Vec::new();
    let mut renamed: Vec<String> = Vec::new();
    for s in now {
        match was.iter().find(|o| o.id == s.id) {
            Some(o) if o.text != s.text => renamed.push(format!("{} to {}", q(o), q(s))),
            Some(_) => {}
            None => parts.push(format!("added {}", q(s))),
        }
    }
    let removed: Vec<String> = was
        .iter()
        .filter(|o| !now.iter().any(|s| s.id == o.id))
        .map(q)
        .collect();
    if !removed.is_empty() {
        parts.push(format!("removed {}", removed.join(", ")));
    }
    if !renamed.is_empty() {
        parts.push(format!("renamed {}", renamed.join(", ")));
    }
    let ticked: Vec<String> = now
        .iter()
        .filter(|s| s.done && was.iter().any(|o| o.id == s.id && !o.done))
        .map(q)
        .collect();
    if !ticked.is_empty() {
        parts.push(format!("ticked {}", ticked.join(", ")));
    }
    if parts.is_empty() {
        return "reordered the steps".into();
    }
    format!("changed the steps: {}", parts.join("; "))
}
