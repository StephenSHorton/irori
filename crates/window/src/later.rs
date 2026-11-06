//! Not yet and order on cards (card #570, docs/design/later-order.md). A
//! held card carries [`CardLater`]: why, what brings it back in the
//! person's words and who held it. A card in a run of cards that follow
//! each other carries [`CardOrder`]. The person holds a card back and
//! brings it back from here; only agents set the order for now.

use std::collections::{HashMap, HashSet};

use coord::{Back, Task};
use gocompat::time::GoTime;
use serde::Serialize;
use specta::Type;

use crate::Window;
use crate::cards::{CardSummary, People};

/// What brings a held card back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum LaterBack {
    /// Another card finishing.
    Card,
    /// A day coming.
    Date,
    /// A device running an Irori version.
    Version,
    /// The person's word.
    #[default]
    Word,
}

/// A held card's Not yet (card #570).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardLater {
    /// Why it was held, as it was given.
    pub why: String,
    pub back: LaterBack,
    /// What brings it back, plainly: "when #12 is done", "on 1 Dec",
    /// "when this Mac runs v0.13.0", "when you say".
    pub line: String,
    /// Who held it, by name.
    pub by: String,
    /// The card it waits on, for a card hold.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub card: String,
}

/// Where a step of an order stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum OrderState {
    Done,
    Working,
    #[default]
    Waiting,
    Later,
}

/// One card of an order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OrderStep {
    pub id: String,
    /// The short ref without its #: "12", "mlg#12" keeps its tag.
    pub short: String,
    pub title: String,
    pub state: OrderState,
}

/// The run of cards a card is in (card #570): each after the one before.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardOrder {
    /// First to last; the card itself among them.
    pub steps: Vec<OrderStep>,
    /// The card it comes after, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub previous: Option<OrderStep>,
    /// The card that comes after it, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub next: Option<OrderStep>,
}

/// How far an order is followed each way from a card.
const MAX_STEPS: usize = 24;

/// What the person's hold says when they give no reason.
fn held_why(name: &str) -> String {
    match name.trim() {
        "" => "The person held it back".into(),
        n => format!("{n} held it back"),
    }
}

const NOT_YET: &str = "Not yet: ";

/// Card t's Not yet as the person reads it; a card hold's line names its
/// card later, in [`fill_later_card`], where the log is at hand.
pub(crate) fn later_of(t: &Task, p: &People) -> Option<CardLater> {
    let b = t.parked()?;
    let back = b.back().unwrap_or(Back::Word);
    let why = b.question.strip_prefix(NOT_YET).unwrap_or(&b.question);
    let by = p.name_of(&b.raised_by);
    let (kind, line, card) = match &back {
        Back::Card(id) => (
            LaterBack::Card,
            "when another card is done".into(),
            id.clone(),
        ),
        Back::Date(d) => (
            LaterBack::Date,
            format!("on {}", day_line(d, &now(), &sys::tz::local_offset)),
            String::new(),
        ),
        Back::Version(r) => (LaterBack::Version, version_line(p, r), String::new()),
        Back::Word => (LaterBack::Word, "when you say".into(), String::new()),
    };
    Some(CardLater {
        why: why.trim().to_string(),
        back: kind,
        line,
        by,
        card,
    })
}

/// Names a card hold's card: "when #12 is done".
pub(crate) fn fill_later_card(co: &coord::Coord, s: &mut CardSummary) {
    if let Some(l) = s.later.as_mut().filter(|l| l.back == LaterBack::Card)
        && let Some(k) = co.get_task(&l.card)
    {
        l.line = format!("when {} is done", k.short_ref());
    }
}

fn now() -> GoTime {
    GoTime::from_system(std::time::SystemTime::now())
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// "1 Dec" in the person's local time, with the year when it isn't this
/// year's.
pub(crate) fn day_line(d: &GoTime, now: &GoTime, offset: &dyn Fn(i64) -> i32) -> String {
    let at = d.with_offset(offset(d.unix_nano().div_euclid(1_000_000_000)));
    let here = now.with_offset(offset(now.unix_nano().div_euclid(1_000_000_000)));
    let (y, m, day, ..) = at.civil();
    let month = MONTHS[(m as usize).saturating_sub(1).min(11)];
    if y == here.civil().0 {
        format!("{day} {month}")
    } else {
        format!("{day} {month} {y}")
    }
}

/// "when this Mac runs v0.13.0": the devices by name, this one as this
/// Mac (or PC).
fn version_line(p: &People, reference: &str) -> String {
    let (min, devs) = coord::parse_version_wait(reference);
    let mut names: Vec<String> = Vec::new();
    for d in &devs {
        let n = if *d == p.self_id() {
            this_device().to_string()
        } else {
            p.device_name(d).unwrap_or("another device").to_string()
        };
        if !names.contains(&n) {
            names.push(n);
        }
    }
    let who = match names.as_slice() {
        [] => "this device".to_string(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    let verb = if names.len() > 1 { "run" } else { "runs" };
    format!("when {who} {verb} v{min}")
}

fn this_device() -> &'static str {
    match std::env::consts::OS {
        "macos" => "this Mac",
        "windows" => "this PC",
        _ => "this device",
    }
}

// ---- order ----

/// The cards that come after each card, read once per list (card #570):
/// open cards' plain after items, followers by id, so the first made
/// leads.
#[derive(Default)]
pub(crate) struct Followers {
    after: HashMap<String, Vec<String>>,
}

impl Followers {
    pub(crate) fn of<'a>(tasks: impl IntoIterator<Item = &'a Task>) -> Followers {
        let mut after: HashMap<String, Vec<String>> = HashMap::new();
        for t in tasks {
            if coord::is_terminal(&t.state) {
                continue;
            }
            for k in t.after_cards() {
                after.entry(k).or_default().push(t.id.clone());
            }
        }
        for v in after.values_mut() {
            v.sort();
            v.dedup();
        }
        Followers { after }
    }

    /// The followers of `co`'s open cards, for one card's read.
    pub(crate) fn read(co: &coord::Coord) -> Followers {
        let open = co.list_tasks(&coord::TaskFilter {
            open: true,
            ..Default::default()
        });
        Followers::of(open.iter())
    }

    fn first(&self, id: &str, seen: &HashSet<String>) -> Option<&String> {
        self.after
            .get(id)
            .and_then(|v| v.iter().find(|k| !seen.contains(*k)))
    }
}

fn step_of(t: &Task) -> OrderStep {
    let state = if coord::is_terminal(coord::effective_state(t)) {
        OrderState::Done
    } else if t.parked().is_some() {
        OrderState::Later
    } else if !t.holders.is_empty()
        || !t.co_holders.is_empty()
        || coord::effective_state(t) == coord::WORKING
    {
        OrderState::Working
    } else {
        OrderState::Waiting
    };
    let r = t.short_ref();
    OrderStep {
        id: t.id.clone(),
        short: r.strip_prefix('#').unwrap_or(&r).to_string(),
        title: t.title.clone(),
        state,
    }
}

/// The card t comes after: its first open after item, else, once that
/// card is done, the first it came after.
fn before_of(co: &coord::Coord, t: &Task, seen: &HashSet<String>) -> Option<Task> {
    let open = t.after_cards();
    if !open.is_empty() {
        return open
            .iter()
            .filter(|id| !seen.contains(*id))
            .find_map(|id| co.get_task(id));
    }
    t.after_cards_ended()
        .iter()
        .filter(|id| !seen.contains(*id))
        .filter_map(|id| co.get_task(id))
        .find(|k| coord::is_terminal(coord::effective_state(k)) && k.state == coord::COMPLETED)
}

/// The order card t is in: back along what it comes after, forward along
/// what comes after it, one path each way. None when it is in no order or
/// in a loop (the loop shows instead).
pub(crate) fn order_of(
    co: &coord::Coord,
    follows: &Followers,
    loops: &coord::OrderLoops,
    t: &Task,
) -> Option<CardOrder> {
    if loops.contains(&t.id) || coord::is_terminal(&t.state) {
        return None;
    }
    let mut seen: HashSet<String> = HashSet::from([t.id.clone()]);
    let mut back: Vec<OrderStep> = Vec::new();
    let mut cur = t.clone();
    while back.len() < MAX_STEPS {
        let Some(k) = before_of(co, &cur, &seen) else {
            break;
        };
        seen.insert(k.id.clone());
        back.push(step_of(&k));
        // A done card's own order is history: the run starts at it.
        if coord::is_terminal(&k.state) {
            break;
        }
        cur = k;
    }
    let mut ahead: Vec<OrderStep> = Vec::new();
    let mut at = t.id.clone();
    while ahead.len() < MAX_STEPS {
        let Some(id) = follows.first(&at, &seen).cloned() else {
            break;
        };
        seen.insert(id.clone());
        let Some(k) = co.get_task(&id) else { break };
        ahead.push(step_of(&k));
        at = id;
    }
    if back.is_empty() && ahead.is_empty() {
        return None;
    }
    let previous = back.first().cloned();
    let next = ahead.first().cloned();
    back.reverse();
    let mut steps = back;
    steps.push(step_of(t));
    steps.extend(ahead);
    Some(CardOrder {
        steps,
        previous,
        next,
    })
}

// ---- holding back and bringing back ----

/// What "until" says, read: a card (#83, mlg#4), a version (v0.13.0, on
/// this device), a date (1 Dec, 2026-12-01: the start of that day here),
/// or nothing (until the person says).
pub(crate) fn parse_until(
    text: &str,
    device: &str,
    now: &GoTime,
    offset: &dyn Fn(i64) -> i32,
) -> Result<Back, String> {
    let s = text.trim();
    if s.is_empty() {
        return Ok(Back::Word);
    }
    if s.contains('#') || s.starts_with("task:") {
        return Ok(Back::Card(s.to_string()));
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return Ok(Back::Card(format!("#{s}")));
    }
    let v = s.strip_prefix(['v', 'V']).unwrap_or(s);
    if v.starts_with(|c: char| c.is_ascii_digit())
        && v.contains('.')
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && !looks_like_iso(v)
    {
        return Ok(Back::Version(format!("{v} {device}")));
    }
    let (y, m, d) = parse_day(s, now, offset).ok_or_else(|| {
        "Hold it until a card (#83), a day (1 Dec) or a version (v0.13.0); left empty, it waits for you".to_string()
    })?;
    Ok(Back::Date(local_midnight(y, m, d, offset)))
}

fn looks_like_iso(s: &str) -> bool {
    let p: Vec<&str> = s.split('-').collect();
    p.len() == 3 && p[0].len() == 4 && p.iter().all(|x| x.chars().all(|c| c.is_ascii_digit()))
}

fn month_of(word: &str) -> Option<u32> {
    let w = word.to_ascii_lowercase();
    if w.len() < 3 {
        return None;
    }
    const FULL: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    FULL.iter()
        .position(|m| m.starts_with(&w) || (w == "sept" && *m == "september"))
        .map(|i| i as u32 + 1)
}

fn days_in(y: i64, m: u32) -> u32 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// A day: 2026-12-01, or a day of the month and a month name in either
/// order with an optional year. With no year it is the next such day
/// from today here.
fn parse_day(s: &str, now: &GoTime, offset: &dyn Fn(i64) -> i32) -> Option<(i64, u32, u32)> {
    let today = now
        .with_offset(offset(now.unix_nano().div_euclid(1_000_000_000)))
        .civil();
    let (y, m, d) = if looks_like_iso(s) {
        let p: Vec<&str> = s.split('-').collect();
        (p[0].parse().ok()?, p[1].parse().ok()?, p[2].parse().ok()?)
    } else {
        let words: Vec<&str> = s
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|w| !w.is_empty())
            .collect();
        let (mut day, mut month, mut year) = (None, None, None);
        for w in &words {
            let num = w.trim_end_matches(|c: char| c.is_ascii_alphabetic());
            if w.chars().all(|c| c.is_ascii_digit()) && w.len() == 4 && year.is_none() {
                year = w.parse::<i64>().ok();
            } else if !num.is_empty()
                && num.chars().all(|c| c.is_ascii_digit())
                && num.len() <= 2
                && day.is_none()
            {
                day = num.parse::<u32>().ok();
            } else if let Some(mo) = month_of(w).filter(|_| month.is_none()) {
                month = Some(mo);
            } else {
                return None;
            }
        }
        let (d, m) = (day?, month?);
        let y = match year {
            Some(y) => y,
            None if (m, d) < (today.1, today.2) => today.0 + 1,
            None => today.0,
        };
        (y, m, d)
    };
    (1..=12).contains(&m).then_some(())?;
    (1..=days_in(y, m)).contains(&d).then_some(())?;
    Some((y, m, d))
}

/// The start of day y-m-d in the local zone.
fn local_midnight(y: i64, m: u32, d: u32, offset: &dyn Fn(i64) -> i32) -> GoTime {
    let utc = GoTime::parse_rfc3339(&format!("{y:04}-{m:02}-{d:02}T00:00:00Z"))
        .unwrap_or_default()
        .unix_nano()
        .div_euclid(1_000_000_000);
    let first = offset(utc);
    let mut at = utc - i64::from(first);
    let second = offset(at);
    if second != first {
        at = utc - i64::from(second);
    }
    GoTime::unix(at, 0)
}

impl Window {
    /// BoardService.NotYet (card #570): the person holds card `id` back
    /// until `until` ([`parse_until`]); `why` may be empty. The agents
    /// taken off it are told. A held card's hold changes.
    pub fn not_yet(&self, id: &str, until: &str, why: &str) -> Result<(), String> {
        let id = crate::trim(id);
        let back = parse_until(until, self.core.device_id(), &now(), &|s| {
            sys::tz::local_offset(s)
        })?;
        let (co, keys) = self.on(id, &[])?;
        let me = self.core.user_sender();
        let why = match crate::trim(why) {
            "" => held_why(&me.name),
            w => w.to_string(),
        };
        let (_, ns) = co
            .park(&keys[0], id, &why, back)
            .map_err(|e| e.to_string())?;
        self.core
            .route_notices(&me, &ns)
            .map_err(|e| format!("It is held back, but not every agent on it heard: {e}"))
    }

    /// BoardService.StartNow (card #570): the person brings held card
    /// `id` back; it goes out as if just posted.
    pub fn start_now(&self, id: &str) -> Result<(), String> {
        let id = crate::trim(id);
        let (co, keys) = self.on(id, &[])?;
        let (_, ns) = co
            .bring_back(&keys[0], id, "the person started it now")
            .map_err(|e| e.to_string())?;
        self.core
            .route_notices(&self.core.user_sender(), &ns)
            .map_err(|e| format!("It is back, but not every agent heard: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> GoTime {
        GoTime::parse_rfc3339(s).unwrap()
    }

    // New York in November: five hours behind.
    fn ny(_: i64) -> i32 {
        -5 * 3600
    }

    fn date(s: &str) -> String {
        match parse_until(s, "dev1", &at("2026-11-06T15:00:00Z"), &ny).unwrap() {
            Back::Date(d) => d.utc().format_rfc3339(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn until_reads_cards_versions_and_nothing() {
        let now = at("2026-11-06T15:00:00Z");
        let p = |s: &str| parse_until(s, "dev1", &now, &ny).unwrap();
        assert_eq!(p(""), Back::Word);
        assert_eq!(p("  "), Back::Word);
        assert_eq!(p("#83"), Back::Card("#83".into()));
        assert_eq!(p("mlg#4"), Back::Card("mlg#4".into()));
        assert_eq!(p("83"), Back::Card("#83".into()));
        assert_eq!(p("v0.13.0"), Back::Version("0.13.0 dev1".into()));
        assert_eq!(p("0.13.0"), Back::Version("0.13.0 dev1".into()));
    }

    #[test]
    fn until_reads_days_at_the_start_of_the_day_here() {
        assert_eq!(date("2026-12-01"), "2026-12-01T05:00:00Z");
        assert_eq!(date("1 Dec"), "2026-12-01T05:00:00Z");
        assert_eq!(date("Dec 1"), "2026-12-01T05:00:00Z");
        assert_eq!(date("1 December 2027"), "2027-12-01T05:00:00Z");
        assert_eq!(date("1st dec"), "2026-12-01T05:00:00Z");
        // A day already gone this year is next year's.
        assert_eq!(date("3 Mar"), "2027-03-03T05:00:00Z");
        // Today stays today: coord says it has passed.
        assert_eq!(date("6 Nov"), "2026-11-06T05:00:00Z");
    }

    #[test]
    fn until_refuses_what_it_cant_read() {
        let now = at("2026-11-06T15:00:00Z");
        for s in ["soon", "31 Feb", "2026-13-01", "Dec", "1 2"] {
            let e = parse_until(s, "dev1", &now, &ny).unwrap_err();
            assert!(e.contains("a card (#83)"), "{s}: {e}");
        }
    }

    #[test]
    fn day_line_is_local_and_drops_this_year() {
        let now = at("2026-11-06T15:00:00Z");
        assert_eq!(day_line(&at("2026-12-01T05:00:00Z"), &now, &ny), "1 Dec");
        // Midnight UTC is still the day before in New York.
        assert_eq!(day_line(&at("2026-12-01T00:00:00Z"), &now, &ny), "30 Nov");
        assert_eq!(
            day_line(&at("2027-01-02T05:00:00Z"), &now, &ny),
            "2 Jan 2027"
        );
    }
}
