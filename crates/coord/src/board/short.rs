//! Short refs `#12`, `#12a`, `mlg#12` (I:internal/coord/short.go,
//! coord.md §3.9): derived by the fold from every task.created row's
//! claimed number, settled the same way on every device.

use gocompat::quote::quote;
use gocompat::time::GoTime;

use super::error::{Error, ErrorKind, Result, not_found};
use super::fold::{Fold, ShortClaim};
use super::types::*;
use super::util::{SWEEPER, collapse_space};
use crate::names::trim_space;

/// The letter(s) of the i-th claimant of a shared number: a..z, aa, ab...
pub fn short_suffix(i: i64) -> String {
    let mut k = i.max(0) + 1;
    let mut out = Vec::new();
    while k > 0 {
        k -= 1;
        out.insert(0, b'a' + (k % 26) as u8);
        k /= 26;
    }
    String::from_utf8(out).expect("ascii")
}

/// A short ref's number without its letters.
pub(crate) fn short_num(short: &str) -> &str {
    short.trim_end_matches(|c: char| c.is_ascii_lowercase())
}

/// Reads `#12`, `12`, `#12b` (any case, spaces trimmed).
pub fn parse_short(r: &str) -> Option<String> {
    let s = gocompat::text::to_lower_str(trim_space(r));
    let s = s.strip_prefix('#').unwrap_or(&s);
    if s.is_empty() || s.len() > 12 {
        return None;
    }
    let b = s.as_bytes();
    let i = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if i == 0 || b[0] == b'0' {
        return None;
    }
    if !s[i..].chars().all(|c| c.is_ascii_lowercase()) {
        return None;
    }
    Some(s.to_string())
}

/// Reads a short ref with its board tag: `mlg#12` is ("mlg", "12").
pub fn parse_ref(r: &str) -> Option<(String, String)> {
    let mut s = trim_space(r);
    let mut tag = String::new();
    if let Some(i) = s.rfind('#')
        && i > 0
    {
        tag = gocompat::text::to_lower_str(&s[..i]);
        if !valid_tag(&tag) {
            return None;
        }
        s = &s[i..];
    }
    Some((tag, parse_short(s)?))
}

/// A board tag: a lowercase letter, then up to 15 of `a-z 0-9 -`.
pub(crate) fn valid_tag(t: &str) -> bool {
    let b = t.as_bytes();
    if b.is_empty() || b.len() > 16 || !b[0].is_ascii_lowercase() {
        return false;
    }
    b[1..]
        .iter()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// A title cut to one short line for an error's text.
fn one_line_title(s: &str) -> String {
    let s = collapse_space(s);
    if s.chars().count() > 60 {
        let mut out: String = s.chars().take(59).collect();
        out.push('…');
        return out;
    }
    s
}

impl Fold {
    /// Records a task.created row's claim.
    pub(crate) fn claim_short(&mut self, id: &str, at: RowPos, num: i64) {
        if self.claimed.contains(id) {
            if !self.replaying {
                self.renumber();
            }
            return;
        }
        let cl = ShortClaim {
            id: id.into(),
            at,
            num: num.max(0),
        };
        self.claimed.insert(id.into());
        self.claims.push(cl.clone());
        if self.replaying {
            return;
        }
        if cl.num == 0 {
            self.renumber();
            return;
        }
        let g = self.short_groups.entry(cl.num).or_default();
        let i = g.partition_point(|x| !cl.at.before(&x.at));
        g.insert(i, cl.clone());
        self.short_top = self.short_top.max(cl.num);
        self.settle(cl.num);
    }

    /// Derives every task's short ref from the claims.
    pub(crate) fn renumber(&mut self) {
        self.claims.sort_by(|a, b| a.at.cmp(&b.at));
        self.short_groups.clear();
        let mut legacy = 0;
        for cl in &self.claims {
            let mut n = cl.num;
            if n == 0 {
                legacy += 1;
                n = legacy;
            }
            self.short_groups.entry(n).or_default().push(cl.clone());
            self.short_top = self.short_top.max(n);
        }
        self.short_idx.clear();
        let ns: Vec<i64> = self.short_groups.keys().copied().collect();
        for n in ns {
            self.settle(n);
        }
    }

    /// Gives number n's claimants their refs.
    fn settle(&mut self, n: i64) {
        let g = self.short_groups.get(&n).cloned().unwrap_or_default();
        let base = n.to_string();
        self.short_idx.remove(&base);
        for (i, cl) in g.iter().enumerate() {
            let (short, was) = if g.len() > 1 {
                (format!("{base}{}", short_suffix(i as i64)), base.clone())
            } else {
                (base.clone(), String::new())
            };
            let Some(old) = self.tasks.get(&cl.id).map(|t| t.short.clone()) else {
                continue;
            };
            self.short_idx.insert(short.clone(), cl.id.clone());
            // As Go: the move is noted while the card still has its old
            // ref, and the new one is set after.
            if old != short && !old.is_empty() && !self.replaying {
                self.short_moved_to(&cl.id, &old, &short);
            }
            if let Some(t) = self.tasks.get_mut(&cl.id) {
                t.short = short;
                t.short_was = was;
            }
        }
    }

    /// Notes that a card's short ref moved, outside replay.
    pub(crate) fn short_moved_to(&mut self, id: &str, from: &str, to: &str) {
        let now = self.stamp();
        self.short_moved.insert(id.into(), now);
        let Some(t) = self.tasks.get(id) else { return };
        let holder = t.holder.clone();
        let mut also = t.holders.clone();
        also.push(t.created_by.clone());
        let has_watchers = !t.watchers.is_empty();
        self.note(Change {
            kind: TASK_CHANGED.into(),
            task_id: id.into(),
            holder,
            ..Change::default()
        });
        let mut what = format!("renumbered it #{to} (it was #{from})");
        let num = short_num(to);
        if num != to {
            what.push_str(&format!(
                ": more than one card made on devices that were apart claimed #{num}, so #{num} alone names none of them now"
            ));
        }
        self.queue_watch(id, has_watchers, SWEEPER, &what, also);
    }

    /// The number a card created now claims.
    pub(crate) fn next_short(&self) -> i64 {
        self.short_top + 1
    }

    /// Resolves what someone typed for a card to its task id.
    pub(crate) fn task_ref_locked(&self, r: &str) -> Result<String> {
        let mut v = trim_space(r).to_string();
        if let Some((k, id)) = v.split_once(':')
            && gocompat::text::equal_fold(trim_space(k), "task")
        {
            v = trim_space(id).to_string();
        }
        let Some((tag, short)) = parse_ref(&v) else {
            return Ok(v);
        };
        if !tag.is_empty() && tag != self.tag() {
            return Err(not_found(format!("{v} is a card on another board")));
        }
        if let Some(id) = self.short_idx.get(&short) {
            if self.deleted.contains(id) {
                return Err(not_found(format!("{v} was deleted")));
            }
            return Ok(id.clone());
        }
        if self.tasks.contains_key(&v) {
            return Ok(v);
        }
        if let Some(e) = self.ambiguous(&short) {
            return Err(e);
        }
        Err(Error::bare(ErrorKind::NotFound))
    }

    /// Ambiguous, naming each card, when short is a bare number more than
    /// one card claimed.
    pub(crate) fn ambiguous(&self, short: &str) -> Option<Error> {
        let n: i64 = super::resource::go_atoi(short)?;
        let g = self.short_groups.get(&n)?;
        if g.len() < 2 {
            return None;
        }
        let names: Vec<String> = g
            .iter()
            .filter_map(|cl| self.tasks.get(&cl.id))
            .map(|t| {
                format!(
                    "{} {} (task:{})",
                    t.short_ref(),
                    quote(&one_line_title(&t.title)),
                    t.id
                )
            })
            .collect();
        Some(Error::new(
            ErrorKind::Ambiguous,
            format!(
                "cards made on devices that were apart each claimed {}#{short}, so it is now {}",
                self.tag(),
                names.join(", ")
            ),
        ))
    }

    /// Whether id's short ref changed at or after `since`.
    pub(crate) fn moved_since(&self, id: &str, since: &GoTime) -> bool {
        self.short_moved
            .get(id)
            .is_some_and(|at| !crate::gotime::before(at, since))
    }
}
