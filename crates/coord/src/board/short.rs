//! Short refs `#12`, `#12a`, `mlg#12` (I:internal/coord/short.go,
//! coord.md §3.9): derived by the fold from every task.created row's
//! claimed number, settled the same way on every device.

use std::collections::HashMap;

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

fn raise_top(tops: &mut HashMap<String, i64>, board: &str, n: i64) {
    let top = tops.entry(board.into()).or_default();
    *top = (*top).max(n);
}

/// Ambiguous, naming each card `short` could mean: boards that each have
/// the number (card #558), or devices apart that claimed it on one board.
pub(crate) fn ambiguous(short: &str, cards: &[&Task]) -> Error {
    let names: Vec<String> = cards
        .iter()
        .map(|t| {
            format!(
                "{} {} (task:{})",
                t.short_ref(),
                quote(&one_line_title(&t.title)),
                t.id
            )
        })
        .collect();
    let names = names.join(", ");
    let text = if cards.iter().all(|t| t.board == cards[0].board) {
        let tag = &cards[0].tag;
        format!(
            "cards made on devices that were apart each claimed {tag}#{short}, so it is now {names}"
        )
    } else {
        format!("#{short} is on more than one board: {names}")
    };
    Error::new(ErrorKind::Ambiguous, text)
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
    pub(crate) fn claim_short(&mut self, id: &str, at: RowPos, num: i64, board: &str) {
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
            board: board.into(),
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
        let key = (cl.board.clone(), cl.num);
        let g = self.short_groups.entry(key.clone()).or_default();
        let i = g.partition_point(|x| !cl.at.before(&x.at));
        g.insert(i, cl.clone());
        raise_top(&mut self.short_tops, &cl.board, cl.num);
        self.settle(&key);
    }

    /// Derives every task's short ref from the claims. A row with no
    /// number predates claims, and so predates boards: those count in one
    /// run whatever they say.
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
            raise_top(&mut self.short_tops, &cl.board, n);
            self.short_groups
                .entry((cl.board.clone(), n))
                .or_default()
                .push(cl.clone());
        }
        self.short_idx.clear();
        let keys: Vec<(String, i64)> = self.short_groups.keys().cloned().collect();
        for key in keys {
            self.settle(&key);
        }
    }

    /// Gives one board's claimants of one number their refs.
    fn settle(&mut self, key: &(String, i64)) {
        let g = self.short_groups.get(key).cloned().unwrap_or_default();
        let (board, base) = (key.0.clone(), key.1.to_string());
        self.short_idx.remove(&(board.clone(), base.clone()));
        for (i, cl) in g.iter().enumerate() {
            let (short, was) = if g.len() > 1 {
                (format!("{base}{}", short_suffix(i as i64)), base.clone())
            } else {
                (base.clone(), String::new())
            };
            let Some(old) = self.tasks.get(&cl.id).map(|t| t.short.clone()) else {
                continue;
            };
            self.short_idx
                .insert((board.clone(), short.clone()), cl.id.clone());
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

    /// The number a card created now on `board` claims: a run of its own,
    /// carrying on from the board's highest (card #558).
    pub(crate) fn next_short(&self, board: &str) -> i64 {
        self.short_tops.get(board).copied().unwrap_or(0) + 1
    }

    /// The cards `short` names, across the space's boards or only among
    /// those showing `tag`: a bare number also names the lettered cards
    /// that share it.
    ///
    /// Personal's default board has no tag, so a bare number is its cards'
    /// whole ref: it means them first, or `home#3` would leave private
    /// card `#3` with nothing that names it (card #558). A deleted one keeps
    /// the number, so a stale `#3` finds nothing rather than `home#3`.
    pub(crate) fn short_hits(&self, tag: &str, short: &str) -> Vec<&Task> {
        let bare = short_num(short) == short;
        let hits: Vec<&Task> = self
            .short_idx
            .iter()
            .filter(|((_, s), _)| s == short || (bare && short_num(s) == short))
            .filter_map(|(_, id)| self.tasks.get(id))
            .filter(|t| tag.is_empty() || t.tag == tag)
            .collect();
        if !tag.is_empty() || self.on_board {
            return hits;
        }
        if hits.iter().any(|t| t.board.is_empty()) {
            return hits.into_iter().filter(|t| t.board.is_empty()).collect();
        }
        if self.deleted_on("", short) {
            return Vec::new();
        }
        hits
    }

    /// Whether a deleted card's claim on `board` gave it exactly `short`.
    /// A claim outlives its card; `short_idx` doesn't keep one past a replay.
    fn deleted_on(&self, board: &str, short: &str) -> bool {
        let base = short_num(short);
        let Some(g) = base
            .parse::<i64>()
            .ok()
            .and_then(|n| self.short_groups.get(&(board.to_string(), n)))
        else {
            return false;
        };
        g.iter().enumerate().any(|(i, cl)| {
            let named = if g.len() > 1 {
                short == format!("{base}{}", short_suffix(i as i64))
            } else {
                short == base
            };
            named && self.deleted.contains(&cl.id)
        })
    }

    /// The card `#5a` was while a space numbered in one run (card #558):
    /// two boards' claimants of 5 had letters by row order, and now each is
    /// plain `#5` on its own board. Old comments still say it, and so does
    /// a device on an older Irori, which folds that way yet.
    fn one_run_claim(&self, tag: &str, short: &str) -> Option<&str> {
        let base = short_num(short);
        let letters = &short[base.len()..];
        if letters.is_empty() {
            return None;
        }
        let num = base.parse::<i64>().ok()?;
        let mut g: Vec<(&str, &ShortClaim)> = self
            .short_groups
            .iter()
            .filter(|((_, n), _)| *n == num)
            .flat_map(|((b, _), g)| g.iter().map(move |cl| (b.as_str(), cl)))
            .collect();
        g.sort_by(|a, b| a.1.at.cmp(&b.1.at));
        let i = (0..g.len()).find(|i| short_suffix(*i as i64) == letters)?;
        let (board, cl) = g[i];
        (tag.is_empty() || self.tag_of(board) == tag).then_some(cl.id.as_str())
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
        if !tag.is_empty() && !self.has_tag(&tag) {
            return Err(not_found(format!("{v} is a card on another board")));
        }
        match self.short_hits(&tag, &short).as_slice() {
            [t] => return Ok(t.id.clone()),
            [] => {}
            many => return Err(ambiguous(&short, many)),
        }
        let gone = self
            .short_groups
            .keys()
            .filter(|(b, _)| tag.is_empty() || self.tag_of(b) == tag)
            .any(|(b, _)| self.deleted_on(b, &short));
        if gone {
            return Err(not_found(format!("{v} was deleted")));
        }
        if let Some(id) = self.one_run_claim(&tag, &short) {
            if self.deleted.contains(id) {
                return Err(not_found(format!("{v} was deleted")));
            }
            if self.tasks.contains_key(id) {
                return Ok(id.to_string());
            }
        }
        if self.tasks.contains_key(&v) {
            return Ok(v);
        }
        Err(Error::bare(ErrorKind::NotFound))
    }

    /// Whether id's short ref changed at or after `since`.
    pub(crate) fn moved_since(&self, id: &str, since: &GoTime) -> bool {
        self.short_moved
            .get(id)
            .is_some_and(|at| !crate::gotime::before(at, since))
    }
}
