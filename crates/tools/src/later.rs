//! Not yet and order (card #570, docs/design/later-order.md), the tools'
//! side: board_later holds a card back and brings it back, `until` reads
//! as coord's [`coord::Back`], and list lines say what brings a held card
//! back and where a card sits in its order.

use std::collections::{HashMap, HashSet};

use coord::{Back, Blocker, Task};
use gocompat::time::GoTime;

use crate::args::Ty;
use crate::format::{one_line, q, quoted, trim};
use crate::{Caller, Inner, decode, err_text, map_err};

/// What a held card's question starts with (coord's `NOT_YET`).
const NOT_YET: &str = "Not yet: ";

/// How far a chain is walked for a card's place in it.
const MAX_CHAIN: usize = 64;

/// What `until` takes, said in its refusal.
const UNTIL_TAKES: &str = "until takes a card (#12 or tag#12), a date (2026-12-01), version 0.13.0 [device …], or nothing: until the person says.";

/// The cards `t` comes after: its open plain waits on cards. A hold
/// until a card is no order, and a question with options is no wait.
pub(crate) fn afters(t: &Task) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for b in &t.blockers {
        if !b.open() || !b.later.is_empty() || !b.options.is_empty() {
            continue;
        }
        let Some(on) = b.on.as_ref().filter(|on| on.kind == coord::BLOCKED_ON_TASK) else {
            continue;
        };
        if !on.reference.is_empty() && !out.contains(&on.reference) {
            out.push(on.reference.clone());
        }
    }
    out
}

/// The order among a set of open cards, found once per list.
#[derive(Debug, Default)]
pub(crate) struct Order {
    after: HashMap<String, Vec<String>>,
    next: HashMap<String, Vec<String>>,
}

impl Order {
    pub(crate) fn of<'a>(ts: impl IntoIterator<Item = &'a Task>) -> Order {
        let mut o = Order::default();
        for t in ts {
            if coord::is_terminal(&t.state) {
                continue;
            }
            let af = afters(t);
            for a in &af {
                o.next.entry(a.clone()).or_default().push(t.id.clone());
            }
            if !af.is_empty() {
                o.after.insert(t.id.clone(), af);
            }
        }
        for v in o.next.values_mut() {
            v.sort();
            v.dedup();
        }
        o
    }

    fn one<'a>(m: &'a HashMap<String, Vec<String>>, id: &str) -> Option<&'a String> {
        match m.get(id).map(Vec::as_slice) {
            Some([one]) => Some(one),
            _ => None,
        }
    }

    /// Steps along a plain chain from `id`, one card each way: `step` is
    /// the map walked, `back` the other way's, which must name one card.
    fn walk(&self, id: &str, step: &HashMap<String, Vec<String>>, back: &HashMap<String, Vec<String>>) -> usize {
        let mut seen: HashSet<&str> = HashSet::from([id]);
        let mut cur = id;
        let mut n = 0;
        while n < MAX_CHAIN {
            let Some(p) = Self::one(step, cur) else { break };
            if Self::one(back, p).map(String::as_str) != Some(cur) || !seen.insert(p) {
                break;
            }
            cur = p;
            n += 1;
        }
        n
    }

    /// Where card `t` sits: the cards it comes after, its place in a
    /// plain chain of three or more, and the cards that come next.
    pub(crate) fn line(&self, s: &Inner, t: &Task) -> Option<String> {
        let af = afters(t);
        let nx = self.next.get(&t.id).cloned().unwrap_or_default();
        if af.is_empty() && nx.is_empty() {
            return None;
        }
        let names = |ids: &[String]| {
            ids.iter()
                .map(|id| s.short_of(id))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut parts = Vec::new();
        if !af.is_empty() {
            parts.push(format!("after {}", names(&af)));
        }
        let before = self.walk(&t.id, &self.after, &self.next);
        let later = self.walk(&t.id, &self.next, &self.after);
        let total = before + later + 1;
        if total >= 3 {
            parts.push(format!("{} of {total}", before + 1));
        }
        if !nx.is_empty() {
            parts.push(format!("next {}", names(&nx)));
        }
        Some(format!("  order: {}\n", one_line(&parts.join("; "), 300)))
    }
}

impl Inner {
    /// board_later's `until` as what brings a card back. A date is the
    /// start of that day on this device's clock.
    pub(crate) fn back_of(&self, until: &str) -> Result<Back, String> {
        let u = trim(until);
        if u.is_empty() {
            return Ok(Back::Word);
        }
        let low = gocompat::text::to_lower_str(u);
        if let Some(rest) = low.strip_prefix("version ") {
            let rest = &u[u.len() - rest.len()..];
            return Ok(Back::Version(self.version_wait(trim(rest))?));
        }
        let first = u.split_whitespace().next().unwrap_or_default();
        if !u.contains('#') && coord::valid_version(first) {
            return Ok(Back::Version(self.version_wait(u)?));
        }
        if is_day(u) {
            let Some(day) = GoTime::parse_rfc3339(&format!("{u}T00:00:00Z")) else {
                return Err(err_text(&format!(
                    "{} is not a date: give it as 2026-12-01.",
                    q(&one_line(u, 40))
                )));
            };
            let off = i64::from(self.now().offset);
            return Ok(Back::Date(GoTime::unix(day.secs - off, 0)));
        }
        let card = u.strip_prefix("task ").map(trim).unwrap_or(u);
        if card.contains('#')
            || card.starts_with("task:")
            || coord::parse_short(card).is_some()
            || self.task_ref(card).is_ok()
        {
            return Ok(Back::Card(card.to_string()));
        }
        Err(err_text(UNTIL_TAKES))
    }

    /// What brings a held card back, for a list line.
    pub(crate) fn back_text(&self, b: &Blocker) -> String {
        match b.back() {
            Some(Back::Card(id)) => format!("comes back when {} is done", self.short_of(&id)),
            Some(Back::Date(d)) => {
                let (y, m, day, ..) = d.with_offset(self.now().offset).civil();
                format!("comes back on {y:04}-{m:02}-{day:02}")
            }
            Some(Back::Version(r)) => {
                let (min, devs) = coord::parse_version_wait(&r);
                let labels = self.b.device_labels();
                let devs: Vec<String> = devs
                    .iter()
                    .map(|d| {
                        labels
                            .get(d)
                            .filter(|l| !l.is_empty())
                            .cloned()
                            .unwrap_or_else(|| d.chars().take(8).collect())
                    })
                    .collect();
                format!(
                    "comes back when {} {} {min}",
                    one_line(&devs.join(", "), 120),
                    if devs.len() == 1 { "runs" } else { "run" }
                )
            }
            _ => "comes back when the person says".into(),
        }
    }

    /// A held card's line: what brings it back, and why.
    pub(crate) fn not_yet_line(&self, b: &Blocker) -> String {
        let why = b.question.strip_prefix(NOT_YET).unwrap_or(&b.question);
        let mut out = format!("  not yet [{}]: {}", one_line(&b.id, 30), self.back_text(b));
        if !trim(why).is_empty() {
            out += &format!("; why: {}", quoted(why, 200));
        }
        out += "\n";
        out
    }
}

/// `2026-12-01`.
fn is_day(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

// --- board_later ---

const LATER_ARGS: &[(&str, Ty)] = &[
    ("task_id", Ty::Str),
    ("until", Ty::Str),
    ("why", Ty::Str),
    ("back", Ty::Bool),
    ("note", Ty::Str),
];

pub(crate) fn board_later(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, LATER_ARGS) {
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
    let n = s.roster();
    if a.bool("back") {
        if !trim(a.str("until")).is_empty() || !trim(a.str("why")).is_empty() {
            return err_text(
                "back = true brings it back now: leave out until and why, and give note if you like.",
            );
        }
        let (t, ns) = match co.bring_back(&as_[0], &id, trim(a.str("note"))) {
            Ok(v) => v,
            Err(e) => return map_err(e),
        };
        let (told, _) = s.tell_all(c, ns);
        return format!(
            "Brought {} back: it is {} and goes out again as if just posted.{}\n{}",
            t.short_ref(),
            t.state,
            s.told(&told, &n),
            s.card_line(&t, None, &n, &s.now())
        );
    }
    if !trim(a.str("note")).is_empty() {
        return err_text("note goes with back = true; to hold the card, give why.");
    }
    let why = trim(a.str("why"));
    if why.is_empty() {
        return err_text("give why: one line on why it waits.");
    }
    let back = match s.back_of(a.str("until")) {
        Ok(b) => b,
        Err(m) => return m,
    };
    let was = co.get_task(&id).is_some_and(|t| t.parked().is_some());
    let (t, ns) = match co.park(&as_[0], &id, why, back) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let (told, _) = s.tell_all(c, ns);
    let when = t
        .parked()
        .map(|b| s.back_text(b))
        .unwrap_or_else(|| "comes back when the person says".into());
    format!(
        "{} {} Not yet: it {}{when}. Nobody starts it until then; it comes back by itself, or when the person brings it back.{}\n{}",
        t.short_ref(),
        if was { "stays" } else { "is" },
        if was { "now " } else { "" },
        s.told(&told, &n),
        s.card_line(&t, None, &n, &s.now())
    )
}
