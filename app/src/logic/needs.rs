//! Home's "Needs you" list and its progress rows (the React app's
//! lib/needs.ts, cards #261, #305, #347, #431, #448, #449, #471, #481,
//! #487, #488, #509, #520, #541, #557, #570): what waits on the person
//! first, then every other card in one of the row states, with the words
//! the card's own page uses.

use std::collections::{HashMap, HashSet};

use inbox::Delivery;
use irori_window::cards::{AgentRef, CardRef, CardSummary, GoneWait, Holding, WaitPr};

use super::agent_counts::NOT_ANSWERING;
use super::agent_status::ms;
use super::cards::{Work, ask, ask_line, busy, idle_unnamed, open_ask, over, over_label, running_job_now, state, work_label};
use super::names::display_name;
use super::plain::plain_line;
use super::spawn_line::{NOT_TRUSTED, SpawnState, spawn_line};
use super::wait_name::{wait_line, wait_name};

/// What a person acts on; Home groups its rows by it in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RowState {
    Needs,
    Stuck,
    Working,
    Waiting,
    Idle,
    Later,
    Done,
}

pub const ROW_GROUPS: [RowState; 7] = [
    RowState::Needs,
    RowState::Stuck,
    RowState::Working,
    RowState::Waiting,
    RowState::Idle,
    RowState::Later,
    RowState::Done,
];

impl RowState {
    pub fn label(self) -> &'static str {
        match self {
            RowState::Needs => "Needs you",
            RowState::Stuck => "Stuck",
            RowState::Working => "Working",
            RowState::Waiting => "Waiting",
            RowState::Idle => "Idle",
            RowState::Later => "Not yet",
            RowState::Done => "Done",
        }
    }
}

/// The row's and the card's words for idle_unnamed (card #305).
pub const IDLE_LINE: &str = "Idle, no reason given";

/// The card waits on the person: an open blocker or done check, its
/// question, or no computer can start its agent (#541).
pub fn needs_you(c: &CardSummary) -> bool {
    if over(&c.state) {
        return false;
    }
    c.open_blockers > 0 || c.ready || c.state == state::INPUT_REQUIRED || !c.no_agent.is_empty()
}

/// A row's state, its line, and whether an agent is busy on it (the
/// row's spinner).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowLine {
    pub state: RowState,
    pub line: String,
    pub spin: bool,
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// A holder isn't there to carry the card.
fn away(w: &AgentRef) -> bool {
    w.status == "gone" || w.status == NOT_ANSWERING || !w.live || w.offline
}

fn away_line(w: &AgentRef, h: Holding) -> String {
    let name = if w.key.is_empty() { "Its agent".to_string() } else { display_name(w) };
    if h == Holding::Silent || w.status == NOT_ANSWERING {
        return format!("{name} isn't answering");
    }
    if h == Holding::Offline || w.offline {
        return format!("{name} is on an offline device");
    }
    format!("{name} {}", if w.spawned { "finished" } else { "went away" })
}

fn row(c: &CardSummary, now: i64) -> (RowLine, bool) {
    let r = |state: RowState, line: String| (
        RowLine {
            state,
            line,
            spin: false,
        },
        false,
    );
    let nobody = |state: RowState, line: String| (
        RowLine {
            state,
            line,
            spin: false,
        },
        true,
    );
    if over(&c.state) {
        return r(RowState::Done, over_label(&c.state, c.ask.as_ref()).into());
    }
    if let Some(l) = &c.later {
        return r(RowState::Later, format!("Back {}", l.line));
    }
    if needs_you(c) {
        if !c.no_agent.is_empty() && !c.ready && c.open_blockers == 0 {
            return r(RowState::Needs, c.no_agent.clone());
        }
        if c.ready && c.open_blockers <= 1 {
            return r(RowState::Needs, "Says it's done: is it?".into());
        }
        if c.open_blockers > 1 {
            return r(RowState::Needs, format!("{} decisions", c.open_blockers));
        }
        let q = plain_line(&c.blocker_question);
        return r(RowState::Needs, if q.is_empty() { "A question for you".into() } else { q });
    }
    let held: Vec<&AgentRef> = c.working.iter().filter(|w| !w.assigned).collect();
    if let Some(a) = c.ask.as_ref().filter(|_| open_ask(c.ask.as_ref())) {
        let line = ask_line(a);
        if a.state == ask::WAITING {
            return r(RowState::Waiting, line);
        }
        if a.state != ask::TAKEN {
            return nobody(RowState::Stuck, line);
        }
        let gone = !held.is_empty() && held.iter().all(|w| away(w));
        return if gone {
            r(RowState::Stuck, away_line(held[0], c.holder.state))
        } else {
            r(RowState::Working, line)
        };
    }
    if c.paused {
        return r(RowState::Waiting, "Paused".into());
    }
    if let Some(pr) = &c.merge_pr {
        return r(RowState::Needs, merge_line(pr));
    }
    if let Some(g) = &c.gone_wait {
        return r(RowState::Needs, gone_line(g));
    }
    if c.state == state::IN_REVIEW {
        return r(RowState::Waiting, "In review".into());
    }
    if !c.subtask_blocked.is_empty() {
        return r(RowState::Waiting, "A subtask needs a call".into());
    }
    if c.waiting_on > 0 && !c.wait_you.is_empty() {
        return r(RowState::Needs, format!("Waits on {}", c.wait_you));
    }
    if !c.wait_loop.is_empty() {
        return r(RowState::Needs, loop_line(&c.wait_loop));
    }
    if let Some(end) = &c.wait_stuck {
        return r(RowState::Needs, format!("Waits on {}, which nobody is on", ref_of(end)));
    }
    if c.waiting_on > 0 && c.wait_gone {
        let n = wait_name(&c.wait_kind, &c.wait_on);
        return r(
            RowState::Stuck,
            format!("Waiting on {}, who is gone", if n.is_empty() { "an agent" } else { &n }),
        );
    }
    if c.waiting_on > 0 {
        return r(RowState::Waiting, wait_line(&c.wait_kind, &c.wait_on));
    }
    let sp = spawn_line(&SpawnState::from(c));
    if c.waiting > 0 && (!held.is_empty() || sp.is_none()) {
        return r(RowState::Waiting, "Your answer is with the agent".into());
    }
    let who = held.iter().map(|w| display_name(w)).take(2).collect::<Vec<_>>().join(", ");
    let w = Work::from(c);
    if busy(&w) {
        return (
            RowLine {
                state: RowState::Working,
                line: if who.is_empty() { "Working".into() } else { format!("{who} working") },
                spin: true,
            },
            false,
        );
    }
    let job = running_job_now(&w);
    if !job.is_empty() {
        return (
            RowLine {
                state: RowState::Working,
                line: format!(
                    "{} running a job",
                    job.iter().map(display_name).take(2).collect::<Vec<_>>().join(", ")
                ),
                spin: true,
            },
            false,
        );
    }
    if held.is_empty() {
        if let Some(sp) = sp {
            let st = if !c.starting.is_empty() {
                RowState::Working
            } else if !c.cant_start.is_empty() {
                RowState::Stuck
            } else {
                RowState::Waiting
            };
            return r(st, cap(&sp.text));
        }
        if c.subtasks > 0 {
            return r(RowState::Waiting, "Waiting on its subtasks".into());
        }
        if !c.working.is_empty() {
            let l = if work_label(&w) == "offered" { "Offered, not taken up yet" } else { "Not picked up" };
            return nobody(RowState::Stuck, l.into());
        }
        if let Some(ret) = &c.returned {
            return r(
                RowState::Stuck,
                format!(
                    "Nobody on it: {} {}",
                    display_name(ret),
                    if ret.spawned { "finished" } else { "went away" }
                ),
            );
        }
        return nobody(RowState::Stuck, "Nobody has it yet".into());
    }
    if held.iter().all(|w| away(w)) {
        return r(RowState::Stuck, away_line(held[0], c.holder.state));
    }
    if idle_unnamed(c, now) {
        return r(
            RowState::Idle,
            if who.is_empty() { IDLE_LINE.into() } else { format!("{IDLE_LINE} ({who})") },
        );
    }
    let there = held
        .iter()
        .filter(|w| !away(w))
        .map(|w| display_name(w))
        .take(2)
        .collect::<Vec<_>>()
        .join(", ");
    r(
        RowState::Working,
        if there.is_empty() { "Held, between turns".into() } else { format!("{there}, between turns") },
    )
}

/// The card's state and the line under its title.
pub fn row_state(c: &CardSummary, now: i64) -> RowLine {
    row(c, now).0
}

/// Rows split into Home's groups, in [`ROW_GROUPS`] order, each in the
/// order given; empty groups left out.
pub fn group_rows<T: Clone>(rows: &[T], state_of: impl Fn(&T) -> RowState) -> Vec<(RowState, Vec<T>)> {
    let mut by: HashMap<RowState, Vec<T>> = HashMap::new();
    for r in rows {
        by.entry(state_of(r)).or_default().push(r.clone());
    }
    ROW_GROUPS.iter().filter_map(|s| by.remove(s).map(|v| (*s, v))).collect()
}

/// The open cards Home files under Stuck because nobody took them.
pub fn nobody_took(c: &CardSummary, now: i64) -> bool {
    row(c, now).1
}

/// The "waits on PR #N" chip an in-review card shows on itself (#471).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrChip {
    pub text: String,
    pub title: String,
    pub url: String,
}

pub fn pr_chip(c: &CardSummary) -> Option<PrChip> {
    let pr = c.wait_pr.as_ref().filter(|_| !over(&c.state))?;
    let who = if pr.approval {
        "another member's agents approve it, then it merges"
    } else {
        "an agent asks you to merge it, then merges it"
    };
    Some(PrChip {
        text: format!("waits on PR #{}", pr.number),
        title: format!("Waits on PR #{} in {}: {who}. Opens {}", pr.number, pr.repo, pr.url),
        url: pr.url.clone(),
    })
}

/// A merge wait's line (#509): "Merge PR #41".
pub fn merge_line(pr: &WaitPr) -> String {
    format!("Merge PR #{}", pr.number)
}

/// A gone wait's line (#520).
pub fn gone_line(g: &GoneWait) -> String {
    let who = format!("{} (gone)", display_name(&g.agent));
    if g.waited_on || g.on.is_empty() {
        return format!("Waits on {who}");
    }
    format!("Waits on {}; {who} raised it", plain_line(&g.on))
}

/// A gone wait's Needs you title: "#73 waits on kestrel (gone)".
pub fn gone_title(c: &CardSummary, g: &GoneWait) -> String {
    let r = if c.short.is_empty() { plain_line(&c.title) } else { format!("#{}", plain_line(&c.short)) };
    format!("{r} waits on {} (gone)", display_name(&g.agent))
}

/// The card's agent can't start until the person trusts its repo.
pub fn waits_on_trust(c: &CardSummary) -> bool {
    !over(&c.state) && c.cant_start.starts_with(NOT_TRUSTED)
}

/// Irori's own sweep questions (card #456); they match the Rust constants.
pub const WAITING_PERMISSION: &str = "Waiting for your permission";
#[cfg(test)]
const CRASHED_QUESTION: &str = "Its agent crashed twice in a row: start another?";
pub const GIVEN_QUESTION: &str = "OK to start an agent on this?";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnNeed {
    Permission,
    Crashed,
    Given,
}

/// Whether a card's decision is one of Irori's sweep questions.
pub fn spawn_need(c: &CardSummary) -> Option<SpawnNeed> {
    let q = c.blocker_question.as_str();
    if q.starts_with(WAITING_PERMISSION) {
        return Some(SpawnNeed::Permission);
    }
    if q.starts_with("Its agent crashed") && q.ends_with(": start another?") {
        return Some(SpawnNeed::Crashed);
    }
    (q == GIVEN_QUESTION).then_some(SpawnNeed::Given)
}

/// Another card as the person reads it: "#392".
pub fn ref_of(r: &CardRef) -> String {
    if !r.short.is_empty() {
        return format!("#{}", plain_line(&r.short));
    }
    let t = plain_line(&r.title);
    if t.is_empty() { "a card".into() } else { t }
}

/// "a", "a and b", "a, b and c".
pub fn and_list(xs: &[String]) -> String {
    super::cards::names_line(xs)
}

/// A wait loop's line: "#392, #396 and #394 wait on each other".
pub fn loop_line(ring: &[CardRef]) -> String {
    format!("{} wait on each other", and_list(&ring.iter().map(ref_of).collect::<Vec<_>>()))
}

/// A chain's that ends at a card nobody is on.
pub fn stuck_line(end: &CardRef, cards: &[&CardSummary]) -> String {
    let who = and_list(
        &cards
            .iter()
            .map(|c| if c.short.is_empty() { plain_line(&c.title) } else { format!("#{}", plain_line(&c.short)) })
            .collect::<Vec<_>>(),
    );
    format!(
        "{who} {} on {}, which nobody is on",
        if cards.len() == 1 { "waits" } else { "wait" },
        ref_of(end)
    )
}

/// One item of the Needs you list.
#[derive(Clone, Debug)]
pub enum NeedKind<'a> {
    Check(&'a CardSummary),
    Decision(&'a CardSummary),
    Merge(&'a CardSummary, &'a WaitPr),
    Gone(&'a CardSummary, &'a GoneWait),
    NoAgent(&'a CardSummary),
    Trust(&'a CardSummary),
    Held(&'a Delivery),
    HeldQuiet(Vec<&'a Delivery>),
    Loop(&'a [CardRef], Vec<&'a CardSummary>),
    Stuck(&'a CardRef, Vec<&'a CardSummary>),
}

#[derive(Clone, Debug)]
pub struct Need<'a> {
    pub id: String,
    /// When it last changed, ms.
    pub at: i64,
    pub kind: NeedKind<'a>,
}

impl Need<'_> {
    /// Its kind as the React app named it ("check", "heldQuiet", ...).
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            NeedKind::Check(_) => "check",
            NeedKind::Decision(_) => "decision",
            NeedKind::Merge(..) => "merge",
            NeedKind::Gone(..) => "gone",
            NeedKind::NoAgent(_) => "noAgent",
            NeedKind::Trust(_) => "trust",
            NeedKind::Held(_) => "held",
            NeedKind::HeldQuiet(_) => "heldQuiet",
            NeedKind::Loop(..) => "loop",
            NeedKind::Stuck(..) => "stuck",
        }
    }
}

/// The id of Needs you's one line for the quiet holds (card #488).
pub const HELD_QUIET: &str = "held-quiet";

/// A held message is the person's to decide on its own row (#488).
pub fn asks_person(d: &Delivery) -> bool {
    if d.state != inbox::st::HELD || d.hold == inbox::hold::PAUSED {
        return false;
    }
    if d.to.starts_with("user:") || d.message.from.kind == "board" {
        return true;
    }
    d.hold == inbox::hold::UNOWNED && from_person(d)
}

/// The person at this app sent `d`.
pub fn from_person(d: &Delivery) -> bool {
    d.message.from.kind == "user" || d.message.from.key.starts_with("user:")
}

fn time(t: &gocompat::time::GoTime) -> i64 {
    ms(t).unwrap_or(0)
}

/// The Needs you list, newest first: every card waiting on the person and
/// every held message that asks them; the rest of the holds are one
/// "N held" line, always last.
pub fn needs_of<'a>(cards: &'a [CardSummary], held: &'a [Delivery]) -> Vec<Need<'a>> {
    let mut out: Vec<Need<'a>> = Vec::new();
    let need = |c: &CardSummary, kind| Need {
        id: c.id.clone(),
        at: time(&c.updated_at),
        kind,
    };
    for c in cards.iter().filter(|c| needs_you(c)) {
        let check = c.ready && c.open_blockers <= 1;
        if !c.no_agent.is_empty() && !check && c.open_blockers == 0 {
            out.push(need(c, NeedKind::NoAgent(c)));
            continue;
        }
        out.push(need(c, if check { NeedKind::Check(c) } else { NeedKind::Decision(c) }));
    }
    for c in cards {
        if let Some(pr) = c.merge_pr.as_ref().filter(|_| !over(&c.state) && !needs_you(c)) {
            out.push(need(c, NeedKind::Merge(c, pr)));
        }
    }
    for c in cards {
        if c.merge_pr.is_some() || over(&c.state) || needs_you(c) {
            continue;
        }
        if let Some(g) = &c.gone_wait {
            out.push(need(c, NeedKind::Gone(c, g)));
        }
    }
    for c in cards.iter().filter(|c| waits_on_trust(c) && !needs_you(c)) {
        out.push(need(c, NeedKind::Trust(c)));
    }
    let mut loops: Vec<Need<'a>> = Vec::new();
    let mut at_of: HashMap<String, usize> = HashMap::new();
    for c in cards.iter().filter(|c| !over(&c.state)) {
        if c.wait_loop.is_empty() && c.wait_stuck.is_none() {
            continue;
        }
        let id = if c.wait_loop.is_empty() {
            c.wait_stuck.as_ref().map(|e| e.id.clone()).unwrap_or_default()
        } else {
            c.wait_loop.iter().map(|r| r.id.as_str()).collect::<Vec<_>>().join(",")
        };
        let at = time(&c.updated_at);
        if let Some(&i) = at_of.get(&id) {
            let n = &mut loops[i];
            n.at = n.at.max(at);
            match &mut n.kind {
                NeedKind::Loop(_, cs) | NeedKind::Stuck(_, cs) => cs.push(c),
                _ => {}
            }
            continue;
        }
        at_of.insert(id.clone(), loops.len());
        let kind = match &c.wait_stuck {
            Some(end) if c.wait_loop.is_empty() => NeedKind::Stuck(end, vec![c]),
            _ => NeedKind::Loop(&c.wait_loop, vec![c]),
        };
        loops.push(Need { id, at, kind });
    }
    out.extend(loops);
    let mut seen = HashSet::new();
    let mut quiet: Vec<&Delivery> = Vec::new();
    for d in held {
        if !seen.insert(d.id.clone()) {
            continue;
        }
        if !asks_person(d) {
            quiet.push(d);
            continue;
        }
        out.push(Need {
            id: d.id.clone(),
            at: time(&d.created_at),
            kind: NeedKind::Held(d),
        });
    }
    out.sort_by(|a, b| b.at.cmp(&a.at));
    if !quiet.is_empty() {
        out.push(Need {
            id: HELD_QUIET.into(),
            at: quiet.iter().map(|d| time(&d.created_at)).max().unwrap_or(0),
            kind: NeedKind::HeldQuiet(quiet),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gocompat::time::GoTime;
    use irori_window::cards::{CardAsk, CardHolder};

    fn t(s: &str) -> GoTime {
        GoTime::parse_rfc3339(s).expect("time")
    }
    fn ms_of(s: &str) -> i64 {
        t(s).unix_milli_value()
    }
    fn card(f: impl FnOnce(&mut CardSummary)) -> CardSummary {
        let mut c = CardSummary {
            id: "c".into(),
            short: "1".into(),
            title: "A card".into(),
            state: state::WORKING.into(),
            scope: "mine".into(),
            scope_label: "My devices".into(),
            updated_at: t("2026-11-02T10:00:00Z"),
            ..CardSummary::default()
        };
        f(&mut c);
        c
    }
    fn delivery(f: impl FnOnce(&mut Delivery)) -> Delivery {
        let mut d = Delivery {
            id: "d".into(),
            message_id: "m".into(),
            to: "user:dev".into(),
            state: inbox::st::QUEUED.into(),
            created_at: t("2026-11-02T10:00:00Z"),
            updated_at: t("2026-11-02T10:00:00Z"),
            ..Delivery::default()
        };
        d.message.id = "m".into();
        d.message.from = inbox::Sender::new("dev/agent", "otter", "claude");
        d.message.text = "Hello".into();
        f(&mut d);
        d
    }
    fn holder(name: &str, status: &str, f: impl Fn(&mut AgentRef)) -> AgentRef {
        let mut a = AgentRef {
            key: "dev/s1".into(),
            name: name.into(),
            status: status.into(),
            live: true,
            where_: "Macbook Pro (this one)".into(),
            here: true,
            idle_since: t("2026-11-02T10:05:00Z"),
            ..AgentRef::default()
        };
        f(&mut a);
        a
    }
    fn held(name: &str, status: &str, fc: impl FnOnce(&mut CardSummary), fw: impl Fn(&mut AgentRef)) -> CardSummary {
        let w = holder(name, status, fw);
        card(|c| {
            c.working = vec![w.clone()];
            c.holder = CardHolder {
                state: Holding::Idle,
                who: w,
                others: 0,
            };
            fc(c)
        })
    }
    fn rl(state: RowState, line: &str) -> RowLine {
        RowLine {
            state,
            line: line.into(),
            spin: false,
        }
    }
    fn kinds(n: &[Need]) -> Vec<String> {
        n.iter().map(|n| format!("{}:{}", n.kind_name(), n.id)).collect()
    }

    #[test]
    fn idle_with_no_reason_given() {
        let h = |fc: fn(&mut CardSummary)| held("orca", "idle", fc, |_| {});
        assert_eq!(row_state(&h(|_| {}), ms_of("2026-11-02T10:10:00Z")), rl(RowState::Working, "orca, between turns"));
        assert_eq!(
            row_state(&h(|_| {}), ms_of("2026-11-02T10:15:00Z")),
            rl(RowState::Idle, &format!("{IDLE_LINE} (orca)"))
        );
        let c = h(|c| c.updated_at = GoTime::parse_rfc3339("2026-11-02T10:12:00Z").unwrap());
        assert_eq!(row_state(&c, ms_of("2026-11-02T10:16:00Z")).state, RowState::Working);
        assert_eq!(row_state(&c, ms_of("2026-11-02T10:22:00Z")).state, RowState::Idle);
        let late = ms_of("2026-11-02T11:00:00Z");
        assert_eq!(
            row_state(
                &h(|c| {
                    c.waiting_on = 1;
                    c.wait_on = "kestrel".into()
                }),
                late
            ),
            rl(RowState::Waiting, "Waiting on kestrel")
        );
        assert_eq!(
            row_state(
                &h(|c| {
                    c.open_blockers = 1;
                    c.blocker_question = "Which?".into()
                }),
                late
            )
            .state,
            RowState::Needs
        );
        assert_eq!(row_state(&held("orca", "busy", |_| {}, |_| {}), late).state, RowState::Working);
        assert_eq!(
            row_state(&held("orca", "gone", |_| {}, |a| a.live = false), late),
            rl(RowState::Stuck, "orca went away")
        );
        assert_eq!(
            row_state(&held("orca", "idle", |_| {}, |a| a.offline = true), late),
            rl(RowState::Stuck, "orca is on an offline device")
        );
        assert_eq!(row_state(&h(|c| c.state = state::IN_REVIEW.into()), late).state, RowState::Waiting);
    }

    #[test]
    fn a_holder_running_a_background_job() {
        let late = ms_of("2026-11-02T11:00:00Z");
        assert_eq!(
            row_state(&held("wonton", "shell", |_| {}, |_| {}), late),
            RowLine {
                state: RowState::Working,
                line: "wonton running a job".into(),
                spin: true
            }
        );
        assert_eq!(
            row_state(&held("wonton", "idle", |_| {}, |_| {}), ms_of("2026-11-02T10:06:00Z")),
            rl(RowState::Working, "wonton, between turns")
        );
        assert_eq!(
            row_state(
                &held(
                    "wonton",
                    "shell",
                    |c| {
                        c.open_blockers = 1;
                        c.blocker_question = "Which?".into()
                    },
                    |_| {}
                ),
                late
            )
            .state,
            RowState::Needs
        );
        assert_eq!(
            row_state(&held("wonton", "shell", |c| c.state = state::IN_REVIEW.into(), |_| {}), late),
            rl(RowState::Waiting, "In review")
        );
        assert!(!row_state(&held("wonton", "shell", |_| {}, |a| a.offline = true), late).line.contains("background"));
    }

    #[test]
    fn row_states() {
        assert_eq!(
            row_state(
                &card(|c| {
                    c.open_blockers = 1;
                    c.blocker_question = "Steam or relay?".into()
                }),
                0
            ),
            rl(RowState::Needs, "Steam or relay?")
        );
        assert_eq!(
            row_state(
                &card(|c| {
                    c.ready = true;
                    c.open_blockers = 1
                }),
                0
            )
            .state,
            RowState::Needs
        );
        assert_eq!(row_state(&card(|c| c.state = state::COMPLETED.into()), 0).state, RowState::Done);
        assert_eq!(row_state(&card(|c| c.state = state::IN_REVIEW.into()), 0).line, "In review");
        assert_eq!(row_state(&card(|_| {}), 0).state, RowState::Stuck);
        assert!(!needs_you(&card(|c| {
            c.state = state::COMPLETED.into();
            c.open_blockers = 1
        })));
    }

    #[test]
    fn progress_and_page_agree_by_the_live_roster() {
        use super::super::cards::{with_live, working_now};
        let oak = AgentRef {
            key: "d/oak".into(),
            name: "oak".into(),
            status: "idle".into(),
            live: true,
            ..AgentRef::default()
        };
        let c = card(|c| {
            c.working = vec![oak.clone()];
            c.holder = CardHolder {
                state: Holding::Idle,
                who: oak.clone(),
                others: 0,
            };
        });
        let live = HashMap::from([("d/oak".to_string(), "busy".to_string())]);
        let mut now = c.clone();
        now.working = with_live(&c.working, &live).unwrap();
        assert_ne!(row_state(&c, ms_of("2026-11-02T11:00:00Z")).state, RowState::Working);
        assert!(row_state(&now, 0).spin);
        assert!(!working_now(&Work::from(&now)).is_empty());
        let mut blocked = now.clone();
        blocked.open_blockers = 1;
        assert_eq!(row_state(&blocked, 0).state, RowState::Needs);
        assert!(working_now(&Work::from(&blocked)).is_empty());
    }

    #[test]
    fn needs_of_cards_and_held() {
        let cs = [
            card(|c| {
                c.id = "a".into();
                c.open_blockers = 1
            }),
            card(|c| {
                c.id = "b".into();
                c.ready = true;
                c.open_blockers = 1;
                c.updated_at = GoTime::parse_rfc3339("2026-11-02T10:05:00Z").unwrap()
            }),
            card(|c| c.id = "c".into()),
        ];
        let h = delivery(|d| {
            d.id = "h".into();
            d.to = String::new();
            d.repo_key = "root:5a3c".into();
            d.state = inbox::st::HELD.into();
            d.hold = inbox::hold::UNOWNED.into();
            d.created_at = GoTime::parse_rfc3339("2026-11-02T10:02:00Z").unwrap();
            d.message.from = inbox::Sender::new("user:dev", "you", "user");
        });
        let ds = [h.clone(), h];
        assert_eq!(kinds(&needs_of(&cs, &ds)), ["check:b", "held:h", "decision:a"]);
    }

    #[test]
    fn quiet_holds_are_one_line_last() {
        let cs = [card(|c| {
            c.id = "a".into();
            c.open_blockers = 1
        })];
        let hd = |id: &str, to: &str, hold: &str, from_user: bool| {
            delivery(|d| {
                d.id = id.into();
                d.to = to.into();
                d.state = inbox::st::HELD.into();
                d.hold = hold.into();
                if to.is_empty() {
                    d.repo_key = "root:5a3c".into();
                }
                if from_user {
                    d.message.from = inbox::Sender::new("user:dev", "you", "user");
                }
            })
        };
        let mut hop = hd("hop", "dev/heron", inbox::hold::HOP_LIMIT, false);
        hop.created_at = GoTime::parse_rfc3339("2026-11-02T10:09:00Z").unwrap();
        let mut person = hd("pal", "dev/heron", inbox::hold::CROSS_PERSON, false);
        person.message.from = inbox::Sender::new("board:b1/p2", "Sam", "board");
        let mine = hd("mine", "", inbox::hold::UNOWNED, true);
        let ds = [
            hop.clone(),
            hd("old", "dev/heron", inbox::hold::BYPASS_RECEIVER, false),
            hd("allow", "dev/heron", inbox::hold::NOT_ALLOWED, false),
            hd("p", "dev/heron", inbox::hold::PAUSED, false),
            hd("ar", "", inbox::hold::UNOWNED, false),
            mine.clone(),
            hd("mp", "dev/heron", inbox::hold::PAUSED, true),
            person,
        ];
        let got = needs_of(&cs, &ds);
        assert_eq!(kinds(&got), ["decision:a", "held:mine", "held:pal", &format!("heldQuiet:{HELD_QUIET}")]);
        let NeedKind::HeldQuiet(q) = &got[3].kind else { panic!("quiet") };
        assert_eq!(q.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["hop", "old", "allow", "p", "ar", "mp"]);
        assert!(!asks_person(&hop));
        let to_you = hd("ty", "user:dev", inbox::hold::NOT_ALLOWED, false);
        assert!(asks_person(&to_you));
        let mut tp = to_you.clone();
        tp.hold = inbox::hold::PAUSED.into();
        assert!(!asks_person(&tp));
        assert!(asks_person(&mine));
        let mut mq = mine.clone();
        mq.state = inbox::st::QUEUED.into();
        assert!(!asks_person(&mq));
        assert_eq!(kinds(&needs_of(&[], std::slice::from_ref(&hop))), [format!("heldQuiet:{HELD_QUIET}")]);
    }

    fn pr(n: i64) -> WaitPr {
        WaitPr {
            repo: "StephenSHorton/suzuri".into(),
            number: n,
            url: format!("https://github.com/StephenSHorton/suzuri/pull/{n}"),
            approval: false,
        }
    }

    #[test]
    fn what_waits_on_the_person() {
        assert_eq!(
            row_state(
                &card(|c| {
                    c.state = state::IN_REVIEW.into();
                    c.wait_pr = Some(pr(96))
                }),
                0
            ),
            rl(RowState::Waiting, "In review")
        );
        let c = card(|c| {
            c.state = state::BLOCKED.into();
            c.waiting_on = 1;
            c.wait_on = "#290".into();
            c.wait_you = "your answer on #290".into();
        });
        assert_eq!(row_state(&c, 0), rl(RowState::Needs, "Waits on your answer on #290"));
        let mut c2 = c.clone();
        c2.wait_on = "#334".into();
        c2.wait_you = String::new();
        assert_eq!(row_state(&c2, 0), rl(RowState::Waiting, "Waiting on #334"));
        let cs = [
            card(|c| {
                c.id = "330".into();
                c.state = state::IN_REVIEW.into();
                c.wait_pr = Some(pr(96))
            }),
            card(|c| {
                c.id = "290".into();
                c.state = state::BLOCKED.into();
                c.open_blockers = 1
            }),
            card(|c| {
                c.id = "329".into();
                c.state = state::BLOCKED.into();
                c.waiting_on = 1;
                c.wait_on = "#334".into()
            }),
        ];
        assert_eq!(kinds(&needs_of(&cs, &[])), ["decision:290"]);
        assert_eq!(
            pr_chip(&card(|c| {
                c.state = state::IN_REVIEW.into();
                c.wait_pr = Some(pr(96))
            })),
            Some(PrChip {
                text: "waits on PR #96".into(),
                title: "Waits on PR #96 in StephenSHorton/suzuri: an agent asks you to merge it, then merges it. Opens https://github.com/StephenSHorton/suzuri/pull/96".into(),
                url: pr(96).url
            })
        );
        let mut ap = pr(96);
        ap.approval = true;
        assert!(
            pr_chip(&card(|c| {
                c.state = state::IN_REVIEW.into();
                c.wait_pr = Some(ap.clone())
            }))
            .unwrap()
            .title
            .contains("another member's agents approve it")
        );
        assert!(pr_chip(&card(|c| c.state = state::IN_REVIEW.into())).is_none());
        assert!(
            pr_chip(&card(|c| {
                c.state = state::COMPLETED.into();
                c.wait_pr = Some(pr(96))
            }))
            .is_none()
        );
    }

    fn gone_agent(name: &str) -> AgentRef {
        AgentRef {
            key: format!("dev/{name}"),
            name: name.into(),
            status: "gone".into(),
            ..AgentRef::default()
        }
    }

    #[test]
    fn a_wait_no_agent_will_end() {
        let on_agent = card(|c| {
            c.id = "73".into();
            c.short = "73".into();
            c.state = state::BLOCKED.into();
            c.waiting_on = 1;
            c.wait_kind = "agent".into();
            c.wait_gone = true;
            c.gone_wait = Some(GoneWait {
                blocker_id: "b6".into(),
                agent: gone_agent("kestrel"),
                waited_on: true,
                question: "Waiting on kestrel's GUI spawn".into(),
                ..GoneWait::default()
            });
            c.updated_at = GoTime::parse_rfc3339("2026-11-05T06:23:00Z").unwrap();
        });
        let outside = card(|c| {
            c.id = "75".into();
            c.short = "75".into();
            c.state = state::BLOCKED.into();
            c.waiting_on = 1;
            c.wait_kind = "external".into();
            c.gone_wait = Some(GoneWait {
                blocker_id: "b1".into(),
                agent: gone_agent("piranha"),
                on: "Apple notarization".into(),
                question: "Waiting on notarization".into(),
                ..GoneWait::default()
            });
            c.updated_at = GoTime::parse_rfc3339("2026-11-05T05:00:00Z").unwrap();
        });
        assert_eq!(row_state(&on_agent, 0), rl(RowState::Needs, "Waits on kestrel (gone)"));
        assert_eq!(
            gone_line(outside.gone_wait.as_ref().unwrap()),
            "Waits on Apple notarization; piranha (gone) raised it"
        );
        assert_eq!(gone_title(&on_agent, on_agent.gone_wait.as_ref().unwrap()), "#73 waits on kestrel (gone)");
        assert_eq!(gone_title(&outside, outside.gone_wait.as_ref().unwrap()), "#75 waits on piranha (gone)");
        let both = [on_agent.clone(), outside];
        let ns = needs_of(&both, &[]);
        assert_eq!(kinds(&ns), ["gone:73", "gone:75"]);
        let mut m = on_agent.clone();
        m.merge_pr = Some(pr(41));
        assert_eq!(kinds(&needs_of(std::slice::from_ref(&m), &[])), ["merge:73"]);
        let mut d = on_agent.clone();
        d.open_blockers = 1;
        assert_eq!(kinds(&needs_of(std::slice::from_ref(&d), &[])), ["decision:73"]);
        let mut x = on_agent.clone();
        x.state = state::CANCELED.into();
        assert!(needs_of(std::slice::from_ref(&x), &[]).is_empty());
        let mut h = on_agent;
        h.gone_wait = None;
        h.state = state::SUBMITTED.into();
        assert!(needs_of(std::slice::from_ref(&h), &[]).is_empty());
    }

    #[test]
    fn a_merge_wait_is_the_persons() {
        let p = pr(41);
        let merge = card(|c| {
            c.id = "454".into();
            c.state = state::BLOCKED.into();
            c.waiting_on = 1;
            c.wait_kind = "external".into();
            c.wait_on = p.url.clone();
            c.merge_pr = Some(p.clone());
            c.updated_at = GoTime::parse_rfc3339("2026-11-05T06:23:00Z").unwrap();
        });
        assert_eq!(row_state(&merge, 0), rl(RowState::Needs, "Merge PR #41"));
        let cs = [
            merge.clone(),
            card(|c| {
                c.id = "290".into();
                c.state = state::BLOCKED.into();
                c.open_blockers = 1;
                c.updated_at = GoTime::parse_rfc3339("2026-11-05T05:00:00Z").unwrap();
            }),
            card(|c| {
                c.id = "500".into();
                c.state = state::BLOCKED.into();
                c.waiting_on = 1;
                c.wait_kind = "external".into();
                c.wait_on = "CI on the release".into();
            }),
            card(|c| {
                c.id = "330".into();
                c.state = state::IN_REVIEW.into();
                c.wait_pr = Some(p.clone())
            }),
        ];
        assert_eq!(kinds(&needs_of(&cs, &[])), ["merge:454", "decision:290"]);
        let mut d = merge.clone();
        d.open_blockers = 1;
        assert_eq!(kinds(&needs_of(std::slice::from_ref(&d), &[])), ["decision:454"]);
        let mut x = merge;
        x.state = state::COMPLETED.into();
        assert!(needs_of(std::slice::from_ref(&x), &[]).is_empty());
    }

    fn cref(id: &str, short: &str, title: &str) -> CardRef {
        CardRef {
            id: id.into(),
            short: short.into(),
            title: title.into(),
            ..CardRef::default()
        }
    }

    #[test]
    fn waits_nothing_will_end() {
        let ring = vec![cref("t392", "392", "Talk 4"), cref("t396", "396", "Talk 8"), cref("t394", "394", "Talk 6")];
        let cs: Vec<CardSummary> = (0..3)
            .map(|i| {
                card(|c| {
                    c.id = ring[i].id.clone();
                    c.short = ring[i].short.clone();
                    c.state = state::BLOCKED.into();
                    c.waiting_on = 1;
                    c.wait_on = format!("#{}", ring[(i + 1) % 3].short);
                    c.wait_loop = ring.clone();
                    c.updated_at = GoTime::parse_rfc3339(&format!("2026-11-03T1{i}:00:00Z")).unwrap();
                })
            })
            .collect();
        let got = needs_of(&cs, &[]);
        assert_eq!(kinds(&got), ["loop:t392,t396,t394"]);
        let NeedKind::Loop(_, members) = &got[0].kind else { panic!("loop") };
        assert_eq!(members.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["t392", "t396", "t394"]);
        assert_eq!(got[0].at, ms_of("2026-11-03T12:00:00Z"));
        assert_eq!(loop_line(&ring), "#392, #396 and #394 wait on each other");
        assert_eq!(row_state(&cs[0], 0), rl(RowState::Needs, "#392, #396 and #394 wait on each other"));
        let w = card(|c| {
            c.id = "t394".into();
            c.state = state::BLOCKED.into();
            c.waiting_on = 1;
            c.wait_on = "#392".into()
        });
        assert!(needs_of(std::slice::from_ref(&w), &[]).is_empty());
        let end = cref("t34", "34", "Vendor");
        let cs: Vec<CardSummary> = ["12", "13"]
            .iter()
            .map(|s| {
                card(|c| {
                    c.id = format!("t{s}");
                    c.short = s.to_string();
                    c.state = state::BLOCKED.into();
                    c.waiting_on = 1;
                    c.wait_on = "#34".into();
                    c.wait_stuck = Some(end.clone());
                })
            })
            .collect();
        let got = needs_of(&cs, &[]);
        assert_eq!(kinds(&got), ["stuck:t34"]);
        let NeedKind::Stuck(e, members) = &got[0].kind else { panic!("stuck") };
        assert_eq!(stuck_line(e, members), "#12 and #13 wait on #34, which nobody is on");
        assert_eq!(row_state(&cs[0], 0).line, "Waits on #34, which nobody is on");
        let done = card(|c| {
            c.id = "t392".into();
            c.state = state::COMPLETED.into();
            c.wait_loop = ring.clone()
        });
        assert!(needs_of(std::slice::from_ref(&done), &[]).is_empty());
    }

    #[test]
    fn trust() {
        let tr = card(|c| {
            c.id = "t".into();
            c.cant_start = "not trusted: /r/api on dev1".into();
            c.state = state::SUBMITTED.into()
        });
        let other = card(|c| {
            c.id = "o".into();
            c.cant_start = "no copy of x on Desktop Computer".into()
        });
        assert!(waits_on_trust(&tr));
        assert!(!waits_on_trust(&other));
        assert_eq!(kinds(&needs_of(&[tr, other], &[])), ["trust:t"]);
        let d = card(|c| {
            c.id = "t".into();
            c.cant_start = "not trusted: /r on d".into();
            c.open_blockers = 1
        });
        assert_eq!(kinds(&needs_of(std::slice::from_ref(&d), &[])), ["decision:t"]);
        let x = card(|c| {
            c.id = "t".into();
            c.cant_start = "not trusted: /r on d".into();
            c.state = state::COMPLETED.into()
        });
        assert!(needs_of(std::slice::from_ref(&x), &[]).is_empty());
    }

    #[test]
    fn spawned_agents_own_decisions() {
        let q = |s: &str| {
            spawn_need(&card(|c| {
                c.open_blockers = 1;
                c.blocker_question = s.into()
            }))
        };
        assert_eq!(q("Waiting for your permission: thyme is stopped on a prompt"), Some(SpawnNeed::Permission));
        assert_eq!(q(CRASHED_QUESTION), Some(SpawnNeed::Crashed));
        assert_eq!(q(GIVEN_QUESTION), Some(SpawnNeed::Given));
        assert_eq!(q("Its agent crashed: start another?"), Some(SpawnNeed::Crashed));
        assert_eq!(q("Its agent crashed 5 times in a row: start another?"), Some(SpawnNeed::Crashed));
        assert_eq!(q("Ship today?"), None);
    }

    fn heron() -> AgentRef {
        AgentRef {
            key: "d/heron".into(),
            name: "heron".into(),
            status: "idle".into(),
            live: true,
            where_: "This PC".into(),
            ..AgentRef::default()
        }
    }

    #[test]
    fn homes_groups() {
        let ask_card = |f: fn(&mut CardAsk)| {
            card(|c| {
                c.state = state::SUBMITTED.into();
                let mut a = CardAsk {
                    state: "open".into(),
                    told: 3,
                    ..CardAsk::default()
                };
                f(&mut a);
                c.ask = Some(a);
            })
        };
        let offered = |waiting: bool| {
            card(|c| {
                c.state = state::SUBMITTED.into();
                c.working = vec![AgentRef {
                    assigned: true,
                    waiting,
                    ..heron()
                }];
                c.holder = CardHolder {
                    state: if waiting { Holding::Waiting } else { Holding::Assigned },
                    who: heron(),
                    others: 0,
                };
            })
        };
        for (c, line) in [
            (card(|c| c.state = state::SUBMITTED.into()), "Nobody has it yet"),
            (offered(true), "Not picked up"),
            (offered(false), "Offered, not taken up yet"),
            (ask_card(|_| {}), "Told 3 agents; nobody took it yet"),
            (ask_card(|a| a.state = "unclaimed".into()), "Nobody took this yet"),
        ] {
            assert_eq!(row_state(&c, 0), rl(RowState::Stuck, line));
            assert!(nobody_took(&c, 0));
        }
        let taken = card(|c| {
            c.state = state::SUBMITTED.into();
            c.ask = Some(CardAsk {
                state: "taken".into(),
                told: 3,
                taker: Some(heron()),
                ..CardAsk::default()
            });
        });
        assert_eq!(row_state(&taken, 0), rl(RowState::Working, "heron took it"));
        assert!(!nobody_took(&taken, 0));
        let starting = card(|c| {
            c.state = state::SUBMITTED.into();
            c.starting = "This PC".into()
        });
        assert_eq!(row_state(&starting, 0), rl(RowState::Working, "Starting an agent on This PC"));
        assert!(!nobody_took(&starting, 0));

        assert_eq!(row_state(&card(|c| c.waiting = 1), 0), rl(RowState::Waiting, "Your answer is with the agent"));
        assert_eq!(
            row_state(
                &card(|c| {
                    c.waiting = 1;
                    c.subtasks = 2;
                    c.starting = "This PC".into()
                }),
                0
            ),
            rl(RowState::Working, "Starting an agent on This PC")
        );
        let lease = card(|c| {
            c.state = state::BLOCKED.into();
            c.waiting_on = 1;
            c.wait_kind = "lease".into();
            c.wait_on = "ww/dolphin-slot-1 ww/dolphin-slot-2 ww/dolphin-slot-3 ww/dolphin-slot-4".into();
        });
        assert_eq!(row_state(&lease, 0), rl(RowState::Waiting, "Waiting for a free Dolphin slot"));
        let gone = card(|c| {
            c.state = state::BLOCKED.into();
            c.waiting_on = 1;
            c.wait_kind = "agent".into();
            c.wait_on = "kestrel".into();
            c.wait_gone = true;
        });
        assert_eq!(row_state(&gone, 0), rl(RowState::Stuck, "Waiting on kestrel, who is gone"));
        assert!(!nobody_took(&gone, 0));

        let ret = card(|c| {
            c.returned = Some(AgentRef {
                status: "gone".into(),
                live: false,
                ..heron()
            })
        });
        assert_eq!(row_state(&ret, 0), rl(RowState::Stuck, "Nobody on it: heron went away"));
        assert!(!nobody_took(&ret, 0));

        let g = AgentRef {
            key: "d/a".into(),
            name: "a".into(),
            status: "gone".into(),
            live: false,
            ..heron()
        };
        let c = card(|c| {
            c.working = vec![g.clone(), heron()];
            c.holder = CardHolder {
                state: Holding::Gone,
                who: g.clone(),
                others: 1,
            };
        });
        assert_eq!(row_state(&c, ms_of("2026-11-02T10:01:00Z")), rl(RowState::Working, "heron, between turns"));

        let c = card(|c| {
            c.working = vec![AgentRef {
                steps: vec!["write tests".into()],
                ..heron()
            }]
        });
        assert_eq!(row_state(&c, ms_of("2026-11-02T10:01:00Z")).state, RowState::Working);
        assert!(!nobody_took(&c, 0));

        let c = card(|c| c.subtasks = 2);
        assert_eq!(row_state(&c, 0), rl(RowState::Waiting, "Waiting on its subtasks"));
        assert!(!nobody_took(&c, 0));

        let oak = AgentRef {
            key: "d/oak".into(),
            name: "oak".into(),
            offline: true,
            ..heron()
        };
        let c = card(|c| {
            c.ask = Some(CardAsk {
                state: "taken".into(),
                told: 2,
                taker: Some(oak.clone()),
                ..CardAsk::default()
            });
            c.working = vec![oak.clone()];
            c.holder = CardHolder {
                state: Holding::Offline,
                who: oak.clone(),
                others: 0,
            };
        });
        assert_eq!(row_state(&c, 0), rl(RowState::Stuck, "oak is on an offline device"));
        assert!(!nobody_took(&c, 0));
    }

    #[test]
    fn groups_in_order() {
        let rows = [
            ("a", RowState::Waiting),
            ("b", RowState::Stuck),
            ("c", RowState::Working),
            ("d", RowState::Stuck),
            ("e", RowState::Needs),
        ];
        let got: Vec<(RowState, Vec<&str>)> = group_rows(&rows, |r| r.1)
            .into_iter()
            .map(|(s, v)| (s, v.into_iter().map(|r| r.0).collect()))
            .collect();
        assert_eq!(
            got,
            [
                (RowState::Needs, vec!["e"]),
                (RowState::Stuck, vec!["b", "d"]),
                (RowState::Working, vec!["c"]),
                (RowState::Waiting, vec!["a"])
            ]
        );
    }

    #[test]
    fn no_agent_can_start() {
        let line = "No agent can start: Mac is offline. It starts by itself when it's back.";
        let c = card(|c| {
            c.id = "c1".into();
            c.no_agent = line.into()
        });
        assert_eq!(kinds(&needs_of(std::slice::from_ref(&c), &[])), ["noAgent:c1"]);
        let mut asked = c;
        asked.open_blockers = 1;
        assert_eq!(kinds(&needs_of(std::slice::from_ref(&asked), &[])), ["decision:c1"]);
    }
}
