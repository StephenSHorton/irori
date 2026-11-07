//! A card's derived words and states (the derivation half of the React
//! app's lib/cards.ts, and the pure parts of cards/bits.tsx): who is busy
//! on it, its state pill, holder and ask lines, its #ref chip status, the
//! people rows of Who's on it, device chips. The Rust types are
//! crates/window's; nothing here reads the backend.

use std::collections::{HashMap, HashSet};

use irori_window::cards::{AgentRef, CardAsk, CardDetail, CardHolder, CardSummary, CardWait, Holding};

use super::agent_status::{Tone as StatusTone, Who, agent_status, ms};
use super::later;
use super::names::display_name;
use super::plain::plain_line;
use super::wait_name::{Wait, wait_label, wait_tip};

/// TaskState's words (crates/window ts.rs).
pub mod state {
    pub const SUBMITTED: &str = "submitted";
    pub const WORKING: &str = "working";
    pub const INPUT_REQUIRED: &str = "input_required";
    pub const COMPLETED: &str = "completed";
    pub const FAILED: &str = "failed";
    pub const CANCELED: &str = "canceled";
    pub const BLOCKED: &str = "blocked";
    pub const IN_REVIEW: &str = "in_review";
}

/// CardBlocker.state's words: open, answered (waits on the raiser),
/// replied (an agent asked back), resolved, closed.
pub mod blocker {
    pub const OPEN: &str = "open";
    pub const ANSWERED: &str = "answered";
    pub const REPLIED: &str = "replied";
    pub const RESOLVED: &str = "resolved";
    pub const CLOSED: &str = "closed";
}

/// CardAsk.state's words (board.md "Asks").
pub mod ask {
    pub const OPEN: &str = "open";
    pub const WAITING: &str = "waiting";
    pub const UNCLAIMED: &str = "unclaimed";
    pub const TAKEN: &str = "taken";
    pub const HANDED: &str = "handed";
    pub const CLOSED: &str = "closed";
}

/// The roster status of a session between turns with a background job
/// still running (card #431).
pub const RUNNING_JOB: &str = "shell";

/// A colour family for chips, pills, bars and dots (the React app's
/// badge variants and status tones).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tone {
    Info,
    Destructive,
    Success,
    Warning,
    Slate,
    /// A plain outline badge.
    Outline,
    /// A filled grey badge.
    Secondary,
}

/// What busy, working-now and the work label read off a card.
#[derive(Clone, Copy, Debug)]
pub struct Work<'a> {
    pub state: &'a str,
    pub open_blockers: i64,
    pub working: &'a [AgentRef],
}

impl<'a> From<&'a CardSummary> for Work<'a> {
    fn from(c: &'a CardSummary) -> Work<'a> {
        Work {
            state: &c.state,
            open_blockers: c.open_blockers,
            working: &c.working,
        }
    }
}

/// The card's progress in percent, `None` with no steps.
pub fn percent(done: i64, total: i64) -> Option<i64> {
    (total > 0).then(|| ((100 * done) as f64 / total as f64).round() as i64)
}

/// The card is finished: done, canceled or failed.
pub fn over(state: &str) -> bool {
    matches!(state, state::COMPLETED | state::CANCELED | state::FAILED)
}

/// A card can be worked on now: not over, not waiting on the person, not
/// in review (card #191).
fn workable(w: &Work) -> bool {
    if matches!(
        w.state,
        state::COMPLETED | state::CANCELED | state::FAILED | state::INPUT_REQUIRED | state::IN_REVIEW
    ) {
        return false;
    }
    w.open_blockers <= 0
}

/// This holder is working right now: live, online, taken up, busy.
pub fn agent_busy(w: &AgentRef) -> bool {
    w.status == "busy" && w.live && !w.offline && !w.assigned
}

/// This holder is between turns with a background job running.
pub fn agent_running_job(w: &AgentRef) -> bool {
    w.status == RUNNING_JOB && w.live && !w.offline && !w.assigned
}

/// A holder is working on the card right now (the card's spinner).
pub fn busy(w: &Work) -> bool {
    workable(w) && w.working.iter().any(agent_busy)
}

fn once<'a>(rs: impl Iterator<Item = &'a AgentRef>) -> Vec<AgentRef> {
    let mut seen = HashSet::new();
    rs.filter(|r| seen.insert(r.key.clone())).cloned().collect()
}

/// Who holds the card and runs a background job between turns, on a card
/// no holder is busy on (card #431).
pub fn running_job_now(w: &Work) -> Vec<AgentRef> {
    if busy(w) || !workable(w) {
        return vec![];
    }
    once(w.working.iter().filter(|r| agent_running_job(r)))
}

/// Who is working on the card right now, by busy's rule.
pub fn working_now(w: &Work) -> Vec<AgentRef> {
    if !busy(w) {
        return vec![];
    }
    once(w.working.iter().filter(|r| agent_busy(r)))
}

/// A card's holders with the live roster's status of now (card #317): an
/// agent the roster doesn't have keeps its status; the person stays.
/// `None` when nothing changed.
pub fn with_live(working: &[AgentRef], live: &HashMap<String, String>) -> Option<Vec<AgentRef>> {
    if live.is_empty() {
        return None;
    }
    let mut changed = false;
    let out = working
        .iter()
        .map(|w| match live.get(&w.key) {
            Some(s) if !s.is_empty() && *s != w.status && w.status != "person" => {
                changed = true;
                AgentRef {
                    status: s.clone(),
                    ..w.clone()
                }
            }
            _ => w.clone(),
        })
        .collect();
    changed.then_some(out)
}

/// "oak", "oak and elm", "oak, elm and ash".
pub fn names_line<S: AsRef<str>>(names: &[S]) -> String {
    match names {
        [] => String::new(),
        [one] => one.as_ref().to_string(),
        [rest @ .., last] => format!(
            "{} and {}",
            rest.iter().map(|s| s.as_ref()).collect::<Vec<_>>().join(", "),
            last.as_ref()
        ),
    }
}

/// A finished card's state word: "Answered" for an ask its taker
/// answered, "Closed" for one nothing came of, else Done, Canceled, Failed.
pub fn over_label(state: &str, a: Option<&CardAsk>) -> &'static str {
    let kind = a
        .filter(|a| a.state == ask::CLOSED)
        .and_then(|a| a.outcome.as_ref())
        .map(|o| o.kind.as_str())
        .unwrap_or("");
    match state {
        state::COMPLETED if kind == "answered" => "Answered",
        state::COMPLETED => "Done",
        state::FAILED => "Failed",
        _ if kind == "none" => "Closed",
        _ => "Canceled",
    }
}

/// How long a held card's holders may sit idle with nothing named before
/// it stops reading as calm Waiting (card #305).
pub const IDLE_UNNAMED_MS: i64 = 10 * 60_000;

/// A working card is held, every holder idle (live, online, taken up) for
/// [`IDLE_UNNAMED_MS`] with nothing saying why: "Idle, no reason given".
pub fn idle_unnamed(c: &CardSummary, now: i64) -> bool {
    if over(&c.state) || c.state != state::WORKING {
        return false;
    }
    if c.open_blockers > 0 || c.waiting_on > 0 || c.waiting > 0 || c.paused {
        return false;
    }
    if !c.subtask_blocked.is_empty() || open_ask(c.ask.as_ref()) {
        return false;
    }
    let held: Vec<&AgentRef> = c.working.iter().filter(|w| !w.assigned).collect();
    if held.is_empty() || !held.iter().all(|w| w.status == "idle" && w.live && !w.offline) {
        return false;
    }
    let since = held
        .iter()
        .filter_map(|w| ms(&w.idle_since))
        .chain(ms(&c.updated_at))
        .max()
        .unwrap_or(0);
    since > 0 && now - since >= IDLE_UNNAMED_MS
}

/// The word beside a card's working faces: "working" while busy,
/// "offered" or "not picked up" before anyone took it up, nothing once
/// it's over, else "on it".
pub fn work_label(w: &Work) -> &'static str {
    if over(w.state) || w.working.is_empty() {
        return "";
    }
    if w.working.iter().all(|r| r.assigned) {
        return if w.working.iter().any(|r| r.waiting) { "not picked up" } else { "offered" };
    }
    if busy(w) { "working" } else { "on it" }
}

/// A notification's ref: "task:<id>" opens the card, "task:<id>/comment:<c>"
/// opens it at that comment.
pub fn open_target(r: &str) -> Option<(String, String)> {
    let rest = r.strip_prefix("task:")?;
    let (card, comment) = match rest.find("/comment:") {
        Some(i) => (&rest[..i], &rest[i + "/comment:".len()..]),
        None => (rest, ""),
    };
    (!card.is_empty()).then(|| (card.to_string(), comment.to_string()))
}

/// A card with steps also shows its holder line whenever nobody is busy on
/// it (card #16).
pub fn holder_line_shows(h: &CardHolder) -> bool {
    h.state != Holding::Active && h.state != Holding::Person
}

/// Nobody live is on the card: its holder went away, or Irori took it off.
pub fn nobody_on(c: &CardSummary) -> bool {
    c.returned.is_some() || c.holder.state == Holding::Gone
}

/// An ask still finding its taker or its outcome.
pub fn open_ask(a: Option<&CardAsk>) -> bool {
    a.is_some_and(|a| a.state != ask::CLOSED)
}

/// An open ask's line in place of the holder line.
pub fn ask_line(a: &CardAsk) -> String {
    let taker = a.taker.as_ref().map(display_name);
    match a.state.as_str() {
        ask::TAKEN => format!("{} took it", taker.unwrap_or_else(|| "An agent".into())),
        ask::HANDED => format!("Handed to {}", taker.unwrap_or_else(|| "an agent".into())),
        ask::WAITING => {
            let o = plain_line(&a.owner);
            format!(
                "Waiting for {} to hand it out",
                if o.is_empty() { "the device it was posted from".to_string() } else { o }
            )
        }
        ask::UNCLAIMED => "Nobody took this yet".into(),
        ask::CLOSED => {
            let (kind, note) = a
                .outcome
                .as_ref()
                .map(|o| (o.kind.as_str(), plain_line(&o.note)))
                .unwrap_or(("", String::new()));
            match kind {
                "edit" => format!(
                    "Changed {}: {note}",
                    if a.outcome_ref.is_empty() { "a card" } else { &a.outcome_ref }
                ),
                "answered" => format!("Answered: {note}"),
                _ => format!("Closed: {note}"),
            }
        }
        _ if a.told == 0 => "Nobody was free to hear it".into(),
        _ => format!("Told {} {}; nobody took it yet", a.told, if a.told == 1 { "agent" } else { "agents" }),
    }
}

/// Whether an ask's line reads as a warning.
pub fn ask_warns(a: &CardAsk) -> bool {
    a.state == ask::UNCLAIMED || a.state == ask::WAITING || (a.state == ask::OPEN && a.told == 0)
}

/// What a #ref's chip says of its card (card #343).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefStatus {
    pub label: &'static str,
    pub tone: Tone,
    /// Done and total steps, when it has some.
    pub progress: Option<(i64, i64)>,
}

/// What [`ref_status`] reads: a card (live or looked up) or a thread.
#[derive(Clone, Debug, Default)]
pub struct RefOf {
    pub thread: bool,
    pub done: bool,
    pub state: String,
    pub blocked: bool,
    pub ready: bool,
    /// `None`: not known (a looked-up card).
    pub working: Option<bool>,
    pub progress: Option<(i64, i64)>,
}

pub fn ref_status(x: &RefOf) -> RefStatus {
    let progress = x.progress.filter(|(_, t)| !x.thread && *t > 0);
    let s = |label, tone| RefStatus { label, tone, progress };
    if x.thread {
        return if x.done { s("Resolved", Tone::Slate) } else { s("Open thread", Tone::Info) };
    }
    match x.state.as_str() {
        state::COMPLETED => return s("Done", Tone::Success),
        state::FAILED => return s("Failed", Tone::Warning),
        state::CANCELED => return s("Canceled", Tone::Slate),
        _ => {}
    }
    if x.ready {
        return s("Ready", Tone::Success);
    }
    if x.blocked || x.state == state::INPUT_REQUIRED || x.state == state::BLOCKED {
        return s("Blocked", Tone::Destructive);
    }
    if x.state == state::IN_REVIEW {
        return s("In review", Tone::Warning);
    }
    if x.state == state::WORKING && x.working != Some(false) {
        return s("Working", Tone::Info);
    }
    s("Open", Tone::Slate)
}

/// [`ref_status`] for a card the window holds.
pub fn card_status(c: &CardSummary) -> RefStatus {
    ref_status(&RefOf {
        thread: false,
        done: over(&c.state),
        state: c.state.clone(),
        blocked: (c.open_blockers > 0 && !(c.ready && c.open_blockers == 1)) || c.state == state::INPUT_REQUIRED,
        ready: c.ready && c.open_blockers == 1,
        working: Some(!c.working.is_empty()),
        progress: Some((c.progress.done, c.progress.total)),
    })
}

/// A card's status colour, shared by its pill and its progress bar:
/// waiting on the person red, done green, closed or unheld grey, else blue.
pub fn card_tone(c: &CardSummary) -> Tone {
    if c.ready && c.open_blockers == 1 {
        return Tone::Success;
    }
    if c.state == state::INPUT_REQUIRED || c.open_blockers > 0 {
        return Tone::Destructive;
    }
    match c.state.as_str() {
        state::COMPLETED => Tone::Success,
        state::CANCELED | state::FAILED => Tone::Slate,
        _ if c.working.is_empty() => Tone::Slate,
        _ => Tone::Info,
    }
}

/// A card's state pill: its words, colour, tooltip ("" for none), and
/// whether it is round (a calm state) rather than square.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pill {
    pub label: String,
    pub tone: Tone,
    pub tip: String,
    pub round: bool,
}

/// The card's first badge (bits.tsx StatePill).
pub fn state_pill(c: &CardSummary, now: i64) -> Pill {
    let p = |label: &str, tone, tip: &str, round| Pill {
        label: label.into(),
        tone,
        tip: tip.into(),
        round,
    };
    if c.later.is_some() {
        return p("Not yet", Tone::Slate, "", true);
    }
    if open_ask(c.ask.as_ref()) {
        let unclaimed = c.ask.as_ref().is_some_and(|a| a.state == ask::UNCLAIMED);
        return p(
            "Ask",
            if unclaimed { Tone::Warning } else { Tone::Info },
            "You asked the room; one agent takes it",
            true,
        );
    }
    if c.state == state::INPUT_REQUIRED {
        return p("Needs your input", Tone::Destructive, "", false);
    }
    if c.ready && c.open_blockers == 1 {
        return p(
            "Ready for you",
            Tone::Success,
            "The agent says it's done: mark it Done, or say why it isn't",
            false,
        );
    }
    if c.open_blockers > 0 {
        let n = c.open_blockers;
        return p(
            &format!("{n} {}", if n == 1 { "blocker" } else { "blockers" }),
            Tone::Destructive,
            "",
            false,
        );
    }
    if c.state == state::IN_REVIEW {
        return p(
            "In review",
            Tone::Warning,
            "The work is done and waits on its pull request's review; its agent is free for other work",
            false,
        );
    }
    if !c.subtask_blocked.is_empty() && !over(&c.state) {
        return p("Subtask blocked", Tone::Warning, &plain_line(&c.subtask_blocked), false);
    }
    if c.waiting_on > 0 {
        let w = Wait::from(c);
        return Pill {
            label: wait_label(&w),
            tone: Tone::Slate,
            tip: wait_tip(&w),
            round: true,
        };
    }
    if c.waiting > 0 {
        return p("Waiting on agent", Tone::Slate, "", true);
    }
    if over(&c.state) {
        let tone = match c.state.as_str() {
            state::COMPLETED => Tone::Success,
            state::FAILED => Tone::Warning,
            _ => Tone::Slate,
        };
        return p(over_label(&c.state, c.ask.as_ref()), tone, "", false);
    }
    if c.working.is_empty() {
        return p("Open", Tone::Slate, "", false);
    }
    match c.holder.state {
        Holding::Waiting => return p("Not picked up", Tone::Warning, "", true),
        Holding::Assigned => return p("Offered", Tone::Slate, "", true),
        _ => {}
    }
    if idle_unnamed(c, now) {
        return p(
            "Idle, no reason given",
            Tone::Warning,
            "Its holder ended its turn without saying what the card waits on. Irori asked it to name the wait (a blocker) or keep going.",
            true,
        );
    }
    p("In progress", Tone::Info, "", false)
}

/// The holder line's words: who holds the card and what that agent is up
/// to, or who it waits for (card #489). `status` and `since` are the live
/// roster's when known (it wins over the card's).
pub fn holder_text(h: &CardHolder, since: Option<i64>, status: &str, now: i64) -> String {
    let who = if h.who.key.is_empty() { String::new() } else { display_name(&h.who) };
    let more = if h.others > 0 { format!(" (+{} more)", h.others) } else { String::new() };
    match h.state {
        Holding::Nobody => "Nobody holds it yet".into(),
        Holding::Person => format!("You hold it{more}"),
        Holding::Waiting => format!("Waiting for {who}{more}: the hand-over hasn't reached it yet"),
        Holding::Assigned => format!("Offered to {who}{more}, not taken up yet"),
        Holding::Active | Holding::Idle | Holding::Silent => {
            let st = if status.is_empty() { h.who.status.as_str() } else { status };
            format!("{who}{more} · {}", agent_status(&Who::status(st), since, now).text)
        }
        Holding::Offline => format!("{who}{more} · its device is offline"),
        Holding::Gone if h.who.spawned => format!("{who}{more} · agent finished"),
        Holding::Gone => format!("{who}{more} · gone"),
    }
}

/// The holder line as it shows: its text, whether it warns, and whether
/// it wears the green working dot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HolderLine {
    pub text: String,
    pub warn: bool,
    pub dot: bool,
}

/// The holder line for a card (bits.tsx HolderLine), with the live
/// roster's status and when it began.
pub fn holder_line(h: &CardHolder, returned: Option<&AgentRef>, live: Option<(&str, Option<i64>)>, now: i64) -> HolderLine {
    let (status, since) = live.unwrap_or(("", None));
    let finished = h.state == Holding::Gone && h.who.spawned;
    let tone = matches!(h.state, Holding::Active | Holding::Idle | Holding::Silent).then(|| {
        let st = if status.is_empty() { h.who.status.as_str() } else { status };
        agent_status(&Who::status(st), None, now).tone
    });
    let warn = returned.is_some_and(|r| !r.spawned)
        || h.state == Holding::Waiting
        || (h.state == Holding::Gone && !finished)
        || tone == Some(StatusTone::Warn)
        || h.state == Holding::Offline;
    let text = match returned {
        Some(r) if h.state == Holding::Nobody => {
            if r.spawned {
                format!("Nobody on it: {} finished", display_name(r))
            } else {
                format!("Nobody on it: {} went away", display_name(r))
            }
        }
        _ => holder_text(h, since, status, now),
    };
    HolderLine {
        text,
        warn,
        dot: matches!(tone, Some(StatusTone::Working | StatusTone::Job)),
    }
}

/// A card's short ref as text: "#12", or "mlg#12" on a shared board.
pub fn ref_text(tag: &str, short: &str) -> String {
    format!("{}#{}", plain_line(tag), plain_line(short))
}

/// One agent in a card's Who's on it (card #490): its strongest role, the
/// titles of the open steps it owns, and its waits.
#[derive(Clone, Debug, PartialEq)]
pub struct PeopleRow {
    pub who: AgentRef,
    pub role: Role,
    pub step_titles: Vec<String>,
    pub waits: Vec<CardWait>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Working,
    Watching,
    Waiting,
}

/// A step's title for the person, never its id.
pub fn step_title(text: Option<&str>) -> String {
    let t = plain_line(text.unwrap_or(""));
    if t.is_empty() { "a step".into() } else { t }
}

/// One row per agent: working (holders, then step owners), watchers, then
/// agents that only wait. `step_text` finds a step's text by id.
pub fn people_rows(
    working: &[AgentRef],
    watching: &[AgentRef],
    waits: &[CardWait],
    step_text: impl Fn(&str) -> Option<String>,
) -> Vec<PeopleRow> {
    let mut rows: Vec<PeopleRow> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    let mut add = |who: &AgentRef, role: Role, rows: &mut Vec<PeopleRow>| -> usize {
        match at.get(&who.key) {
            None => {
                rows.push(PeopleRow {
                    who: who.clone(),
                    role,
                    step_titles: vec![],
                    waits: vec![],
                });
                at.insert(who.key.clone(), rows.len() - 1);
                rows.len() - 1
            }
            Some(&i) => {
                let r = &mut rows[i];
                if r.role == Role::Working && role == Role::Working {
                    r.who.assigned |= who.assigned;
                    r.who.waiting |= who.waiting;
                    r.who.spawned |= who.spawned;
                    if r.who.handoff.is_empty() {
                        r.who.handoff = who.handoff.clone();
                    }
                    r.who.steps.extend(who.steps.iter().cloned());
                }
                i
            }
        }
    };
    for w in working {
        add(w, Role::Working, &mut rows);
    }
    for w in watching {
        add(w, Role::Watching, &mut rows);
    }
    for w in waits {
        let i = add(&w.who, Role::Waiting, &mut rows);
        rows[i].waits.push(w.clone());
    }
    for r in &mut rows {
        let mut seen = HashSet::new();
        r.step_titles = r
            .who
            .steps
            .iter()
            .filter(|id| seen.insert(id.to_string()))
            .map(|id| step_title(step_text(id).as_deref()))
            .collect();
    }
    rows
}

/// [`people_rows`] for a card's page.
pub fn people_rows_of(d: &CardDetail) -> Vec<PeopleRow> {
    people_rows(&d.summary.working, &d.summary.watching, &d.waits, |id| {
        d.steps.iter().find(|s| s.id == id).map(|s| s.text.clone())
    })
}

/// One device in the grid's device filter (card #22).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceChip {
    pub id: String,
    pub name: String,
    pub color: String,
}

/// The devices the cards' agents run on, for the filter row: only once
/// there is more than one.
pub fn device_chips<'a>(cards: impl IntoIterator<Item = &'a CardSummary>) -> Vec<DeviceChip> {
    let mut seen: Vec<DeviceChip> = Vec::new();
    for c in cards {
        for w in &c.working {
            if !w.device_id.is_empty() && !w.color.is_empty() && !seen.iter().any(|d| d.id == w.device_id) {
                seen.push(DeviceChip {
                    id: w.device_id.clone(),
                    name: w.where_.clone(),
                    color: w.color.clone(),
                });
            }
        }
    }
    seen.sort_by(|a, b| super::spaces::name_order(&a.name, &b.name));
    if seen.len() > 1 { seen } else { vec![] }
}

/// A card's Not yet line, when it is held (later.ts whenLater).
pub fn later_line(c: &CardSummary) -> Option<String> {
    c.later.as_ref().map(later::when_later)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn agent(status: &str) -> AgentRef {
        AgentRef {
            key: "d/s".into(),
            name: "a".into(),
            status: status.into(),
            live: status != "gone",
            where_: "Macbook Pro (this one)".into(),
            here: true,
            ..AgentRef::default()
        }
    }
    fn with(mut a: AgentRef, f: impl Fn(&mut AgentRef)) -> AgentRef {
        f(&mut a);
        a
    }
    fn w<'a>(state: &'a str, working: &'a [AgentRef], ob: i64) -> Work<'a> {
        Work {
            state,
            open_blockers: ob,
            working,
        }
    }

    #[test]
    fn busy_rules() {
        assert!(busy(&w(state::WORKING, &[agent("busy")], 0)));
        assert!(!busy(&w(state::WORKING, &[agent("idle")], 0)));
        assert!(!busy(&w(state::WORKING, &[agent("gone")], 0)));
        assert!(!busy(&w(state::WORKING, &[with(agent("busy"), |a| a.offline = true)], 0)));
        assert!(!busy(&w(state::WORKING, &[with(agent("busy"), |a| a.assigned = true)], 0)));
        assert!(!busy(&w(state::WORKING, &[], 0)));
        assert!(!busy(&w(state::INPUT_REQUIRED, &[agent("busy")], 0)));
        assert!(!busy(&w(state::BLOCKED, &[agent("busy")], 1)));
        assert!(!busy(&w(state::COMPLETED, &[agent("busy")], 0)));
        assert!(!busy(&w(state::CANCELED, &[agent("busy")], 0)));
        assert!(!busy(&w(state::IN_REVIEW, &[agent("busy")], 0)));
        assert!(busy(&w(state::WORKING, &[agent("idle"), agent("busy")], 0)));
    }

    #[test]
    fn with_live_and_working_now() {
        let live = |k: &str, s: &str| HashMap::from([(k.to_string(), s.to_string())]);
        let idle = [agent("idle")];
        assert!(!busy(&w(state::WORKING, &idle, 0)));
        let now = with_live(&idle, &live("d/s", "busy")).unwrap();
        assert!(busy(&w(state::WORKING, &now, 0)));
        let was = [agent("busy")];
        let now = with_live(&was, &live("d/s", "idle")).unwrap();
        assert!(!busy(&w(state::WORKING, &now, 0)));
        assert!(with_live(&was, &live("other/x", "idle")).is_none());
        assert!(with_live(&was, &HashMap::new()).is_none());

        let oak = with(agent("busy"), |a| {
            a.key = "d/oak".into();
            a.name = "oak".into()
        });
        let elm = with(agent("idle"), |a| {
            a.key = "d/elm".into();
            a.name = "elm".into()
        });
        let ash = with(agent("busy"), |a| {
            a.key = "d/ash".into();
            a.name = "ash".into();
            a.assigned = true
        });
        let all = [oak.clone(), elm, ash, oak.clone()];
        let names: Vec<String> = working_now(&w(state::WORKING, &all, 0)).into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["oak"]);
        let o = [oak];
        assert!(working_now(&w(state::WORKING, &o, 1)).is_empty());
        assert!(working_now(&w(state::IN_REVIEW, &o, 0)).is_empty());
        assert!(working_now(&w(state::COMPLETED, &o, 0)).is_empty());
    }

    #[test]
    fn names_lines() {
        assert_eq!(names_line::<&str>(&[]), "");
        assert_eq!(names_line(&["oak"]), "oak");
        assert_eq!(names_line(&["oak", "elm"]), "oak and elm");
        assert_eq!(names_line(&["oak", "elm", "ash"]), "oak, elm and ash");
    }

    #[test]
    fn work_labels() {
        assert_eq!(work_label(&w(state::WORKING, &[agent("busy")], 0)), "working");
        assert_eq!(work_label(&w(state::WORKING, &[agent("idle")], 0)), "on it");
        assert_eq!(work_label(&w(state::BLOCKED, &[agent("busy")], 1)), "on it");
        assert_eq!(work_label(&w(state::INPUT_REQUIRED, &[agent("busy")], 0)), "on it");
        assert_eq!(work_label(&w(state::COMPLETED, &[agent("busy")], 0)), "");
        assert_eq!(work_label(&w(state::WORKING, &[with(agent("idle"), |a| a.assigned = true)], 0)), "offered");
        assert_eq!(
            work_label(&w(
                state::WORKING,
                &[with(agent("idle"), |a| {
                    a.assigned = true;
                    a.waiting = true
                })],
                0
            )),
            "not picked up"
        );
        assert_eq!(work_label(&w(state::WORKING, &[], 0)), "");
        for st in [state::COMPLETED, state::CANCELED, state::FAILED] {
            assert_eq!(work_label(&w(st, &[agent("busy")], 0)), "");
            assert_eq!(work_label(&w(st, &[with(agent("idle"), |a| a.assigned = true)], 0)), "");
            assert!(!busy(&w(st, &[agent("busy")], 0)));
        }
    }

    pub(crate) fn an_ask(f: impl Fn(&mut CardAsk)) -> CardAsk {
        let mut a = CardAsk {
            state: "open".into(),
            told: 3,
            owner: "MacBook".into(),
            ..CardAsk::default()
        };
        f(&mut a);
        a
    }
    fn closed(kind: &str) -> CardAsk {
        an_ask(|a| {
            a.state = "closed".into();
            a.outcome = Some(Default::default());
            let o = a.outcome.as_mut().unwrap();
            o.kind = kind.into();
            o.note = "x".into();
        })
    }

    #[test]
    fn asks() {
        assert_eq!(ask_line(&an_ask(|_| {})), "Told 3 agents; nobody took it yet");
        assert_eq!(ask_line(&an_ask(|a| a.told = 1)), "Told 1 agent; nobody took it yet");
        assert_eq!(ask_line(&an_ask(|a| a.told = 0)), "Nobody was free to hear it");
        assert_eq!(ask_line(&an_ask(|a| a.state = "waiting".into())), "Waiting for MacBook to hand it out");
        assert_eq!(ask_line(&an_ask(|a| a.state = "unclaimed".into())), "Nobody took this yet");
        let heron = with(agent("idle"), |a| a.name = "sample-heron".into());
        assert_eq!(
            ask_line(&an_ask(|a| {
                a.state = "taken".into();
                a.taker = Some(heron.clone())
            })),
            "sample-heron took it"
        );
        assert_eq!(
            ask_line(&an_ask(|a| {
                a.state = "handed".into();
                a.taker = Some(heron.clone())
            })),
            "Handed to sample-heron"
        );
        assert!(open_ask(Some(&an_ask(|_| {}))));
        assert!(!open_ask(Some(&closed("none"))));
        assert!(!open_ask(None));
        assert_eq!(over_label(state::COMPLETED, Some(&closed("answered"))), "Answered");
        assert_eq!(over_label(state::CANCELED, Some(&closed("none"))), "Closed");
        assert_eq!(over_label(state::COMPLETED, Some(&closed("edit"))), "Done");
        assert_eq!(over_label(state::COMPLETED, None), "Done");
        assert_eq!(over_label(state::CANCELED, None), "Canceled");
        assert_eq!(over_label(state::FAILED, None), "Failed");
        assert_eq!(ask_line(&closed("answered")), "Answered: x");
        assert_eq!(ask_line(&closed("none")), "Closed: x");
    }

    #[test]
    fn gone_holders() {
        let h = |s: Holding| CardHolder {
            state: s,
            ..CardHolder::default()
        };
        assert!(!holder_line_shows(&h(Holding::Active)));
        assert!(!holder_line_shows(&h(Holding::Person)));
        for s in [Holding::Idle, Holding::Gone, Holding::Offline, Holding::Nobody, Holding::Waiting] {
            assert!(holder_line_shows(&h(s)));
        }
        let c = |s: Holding, returned: Option<AgentRef>| CardSummary {
            holder: h(s),
            returned,
            ..CardSummary::default()
        };
        assert!(nobody_on(&c(Holding::Gone, None)));
        assert!(nobody_on(&c(Holding::Nobody, Some(agent("gone")))));
        assert!(!nobody_on(&c(Holding::Nobody, None)));
        assert!(!nobody_on(&c(Holding::Idle, None)));
    }

    #[test]
    fn device_colours() {
        let pc = with(agent("busy"), |a| {
            a.device_id = "pc".into();
            a.color = "teal".into()
        });
        let mac = with(agent("idle"), |a| {
            a.key = "mac/s".into();
            a.device_id = "mac".into();
            a.where_ = "Desktop Computer".into();
            a.here = false;
            a.color = "orange".into()
        });
        let c = |w: Vec<AgentRef>| CardSummary {
            working: w,
            ..CardSummary::default()
        };
        assert!(device_chips(&[c(vec![pc.clone()]), c(vec![pc.clone()])]).is_empty());
        assert_eq!(
            device_chips(&[c(vec![pc.clone()]), c(vec![mac, pc])]),
            [
                DeviceChip {
                    id: "mac".into(),
                    name: "Desktop Computer".into(),
                    color: "orange".into()
                },
                DeviceChip {
                    id: "pc".into(),
                    name: "Macbook Pro (this one)".into(),
                    color: "teal".into()
                },
            ]
        );
        let nocolor = |id: &str| with(agent("busy"), |a| a.device_id = id.into());
        assert!(device_chips(&[c(vec![nocolor("pc")]), c(vec![nocolor("mac")])]).is_empty());
    }

    #[test]
    fn open_targets() {
        assert_eq!(open_target("task:t1"), Some(("t1".into(), "".into())));
        assert_eq!(open_target("task:t1/comment:c9"), Some(("t1".into(), "c9".into())));
        assert_eq!(open_target("msg:x"), None);
        assert_eq!(open_target("task:"), None);
        assert_eq!(open_target(""), None);
    }

    #[test]
    fn ref_statuses() {
        let c = |st: &str, blocked: bool, f: &dyn Fn(&mut RefOf)| {
            let mut x = RefOf {
                state: st.into(),
                blocked,
                ..RefOf::default()
            };
            f(&mut x);
            ref_status(&x)
        };
        let n = &|_: &mut RefOf| {};
        assert_eq!((c(state::SUBMITTED, false, n).label, c(state::SUBMITTED, false, n).tone), ("Open", Tone::Slate));
        assert_eq!((c(state::WORKING, false, n).label, c(state::WORKING, false, n).tone), ("Working", Tone::Info));
        assert_eq!(c(state::WORKING, false, &|x| x.working = Some(false)).label, "Open");
        assert_eq!(c(state::WORKING, true, n).tone, Tone::Destructive);
        assert_eq!(c(state::INPUT_REQUIRED, false, n).label, "Blocked");
        assert_eq!(c(state::IN_REVIEW, false, n).tone, Tone::Warning);
        assert_eq!(c(state::COMPLETED, false, n).tone, Tone::Success);
        assert_eq!(c(state::FAILED, false, n).label, "Failed");
        assert_eq!(c(state::CANCELED, false, n).label, "Canceled");
        assert_eq!(c(state::WORKING, false, &|x| x.ready = true).label, "Ready");
        assert_eq!(c(state::COMPLETED, true, n).label, "Done");
        assert_eq!(c(state::WORKING, false, &|x| x.progress = Some((2, 5))).progress, Some((2, 5)));
        assert_eq!(c(state::WORKING, false, &|x| x.progress = Some((0, 0))).progress, None);
        let t = |done: bool| {
            ref_status(&RefOf {
                thread: true,
                done,
                state: "working".into(),
                ..RefOf::default()
            })
            .label
        };
        assert_eq!(t(false), "Open thread");
        assert_eq!(t(true), "Resolved");
    }

    #[test]
    fn people_rows_cases() {
        let named = |key: &str, name: &str| {
            with(agent("idle"), |a| {
                a.key = key.into();
                a.name = name.into()
            })
        };
        let otter = named("d/otter", "otter");
        let heron = named("d/heron", "heron");
        let wren = named("d/wren", "wren");
        let steps = |id: &str| match id {
            "s1" => Some("Plan".to_string()),
            "s3" => Some("Sync".to_string()),
            _ => None,
        };
        let wait = |id: &str, who: &AgentRef, text: &str, on_this: bool| CardWait {
            id: id.into(),
            who: who.clone(),
            text: text.into(),
            on_this,
        };

        let rows = people_rows(
            &[otter.clone()],
            &[],
            &[wait("w1", &named("d/otter", ""), "branch main moves", false)],
            steps,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].role, Role::Working);
        assert_eq!(rows[0].who.name, "otter");
        assert_eq!(rows[0].waits[0].text, "branch main moves");

        let rows = people_rows(
            &[],
            &[wren.clone()],
            &[wait("w1", &wren, "until done", true), wait("w2", &wren, "#3", false)],
            steps,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].role, rows[0].waits.len()), (Role::Watching, 2));

        let rows = people_rows(&[otter.clone()], &[heron.clone()], &[wait("w1", &wren, "until done", true)], steps);
        let roles: Vec<Role> = rows.iter().map(|r| r.role).collect();
        assert_eq!(roles, [Role::Working, Role::Watching, Role::Waiting]);

        let h2 = with(heron.clone(), |a| a.steps = vec!["s3".into(), "s9".into()]);
        let rows = people_rows(&[otter.clone(), h2], &[], &[], steps);
        assert_eq!(rows[1].step_titles, ["Sync", "a step"]);

        let rows = people_rows(
            &[with(otter.clone(), |a| a.assigned = true), with(otter.clone(), |a| a.steps = vec!["s1".into()])],
            &[otter.clone()],
            &[],
            steps,
        );
        assert_eq!(rows.len(), 1);
        assert!(rows[0].who.assigned);
        assert_eq!(rows[0].step_titles, ["Plan"]);
    }

    #[test]
    fn holder_texts() {
        let h = |s: Holding, status: &str| CardHolder {
            who: with(agent(status), |a| a.name = "oak".into()),
            state: s,
            others: 0,
        };
        assert_eq!(holder_text(&h(Holding::Nobody, "idle"), None, "", 0), "Nobody holds it yet");
        assert_eq!(holder_text(&h(Holding::Active, "shell"), None, "", 0), "oak · running a job");
        assert_eq!(holder_text(&h(Holding::Idle, "idle"), None, "busy", 0), "oak · working");
        assert_eq!(holder_text(&h(Holding::Offline, "idle"), None, "", 0), "oak · its device is offline");
        let mut g = h(Holding::Gone, "gone");
        assert_eq!(holder_text(&g, None, "", 0), "oak · gone");
        g.who.spawned = true;
        assert_eq!(holder_text(&g, None, "", 0), "oak · agent finished");
        let l = holder_line(&h(Holding::Active, "busy"), None, None, 0);
        assert!(l.dot && !l.warn);
        let r = with(agent("gone"), |a| a.name = "elm".into());
        let l = holder_line(&h(Holding::Nobody, "idle"), Some(&r), None, 0);
        assert_eq!(l.text, "Nobody on it: elm went away");
        assert!(l.warn);
    }

    #[test]
    fn pills() {
        let c = |f: &dyn Fn(&mut CardSummary)| {
            let mut c = CardSummary {
                state: state::WORKING.into(),
                working: vec![agent("busy")],
                ..CardSummary::default()
            };
            f(&mut c);
            state_pill(&c, 0)
        };
        assert_eq!(c(&|_| {}).label, "In progress");
        assert_eq!(c(&|c| c.open_blockers = 2).label, "2 blockers");
        assert_eq!(
            c(&|c| {
                c.open_blockers = 1;
                c.ready = true
            })
            .tone,
            Tone::Success
        );
        assert_eq!(c(&|c| c.state = state::INPUT_REQUIRED.into()).label, "Needs your input");
        assert_eq!(c(&|c| c.working.clear()).label, "Open");
        assert_eq!(c(&|c| c.state = state::COMPLETED.into()).label, "Done");
        let p = c(&|c| {
            c.waiting_on = 2;
            c.wait_kind = "agent".into();
            c.wait_on = "kestrel".into()
        });
        assert_eq!((p.label.as_str(), p.round), ("Waiting on kestrel +1", true));
        assert_eq!(c(&|c| c.holder.state = Holding::Assigned).label, "Offered");
    }
}
