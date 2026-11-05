//! Short refs `#12`, `#12a`, `mlg#12` (I:internal/coord/short.go,
//! coord.md §3.9): derived by the fold from every task.created row's
//! claimed number, settled the same way on every device.

use std::collections::{BTreeSet, HashMap, HashSet};

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

/// Hands a deleted board's cards the next numbers on the first board, in
/// the order they got there.
fn displace(
    board: &str,
    at: &RowPos,
    live: &mut HashMap<String, ShortClaim>,
    tops: &mut HashMap<String, i64>,
    hist: &mut Vec<(String, i64, String)>,
) {
    let mut on: Vec<ShortClaim> = live.values().filter(|c| c.board == board).cloned().collect();
    on.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
    for was in on {
        let num = tops.get("").copied().unwrap_or(0) + 1;
        raise_top(tops, "", num);
        live.insert(
            was.id.clone(),
            ShortClaim {
                id: was.id.clone(),
                at: at.clone(),
                num,
                board: String::new(),
                moved: false,
                displaced: true,
            },
        );
        hist.push((was.board, was.num, was.id));
    }
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
            board: self.own_board(board),
            moved: false,
            displaced: false,
        };
        self.claimed.insert(id.into());
        self.claims.push(cl.clone());
        if self.replaying {
            return;
        }
        if cl.num == 0 || self.needs_walk(&cl) {
            self.renumber();
            return;
        }
        let key = (cl.board.clone(), cl.num);
        let g = self.short_groups.entry(key.clone()).or_default();
        let i = g.partition_point(|x| !cl.at.before(&x.at));
        g.insert(i, cl.clone());
        raise_top(&mut self.short_tops, &cl.board, cl.num);
        if self.walked_to.as_ref().is_none_or(|w| w.before(&cl.at)) {
            self.walked_to = Some(cl.at.clone());
        }
        self.settle(&key);
    }

    /// Whether a claim can change what the last walk gave, so only another
    /// walk will do: it sits before the walk's end, or names a deleted
    /// board. With no moves or deletes, claims don't touch each other.
    fn needs_walk(&self, cl: &ShortClaim) -> bool {
        if !self.any_moves && self.board_view.deleted.is_empty() {
            return false;
        }
        self.walked_to.as_ref().is_some_and(|w| cl.at.before(w))
            || self.board_view.deleted.iter().any(|(b, _)| *b == cl.board)
    }

    /// Records a move row's claim (card #563), whether or not its card has
    /// landed yet; false for a repeat. The walk decides what it does, so
    /// arrival order can't.
    pub(crate) fn fold_move(&mut self, id: &str, at: RowPos, board: &str, num: i64) -> bool {
        if id.is_empty() || self.claims.iter().any(|c| c.moved && c.at == at) {
            return false;
        }
        self.claims.push(ShortClaim {
            id: id.into(),
            at,
            num: num.max(0),
            board: self.own_board(board),
            moved: true,
            displaced: false,
        });
        self.any_moves = true;
        if !self.replaying {
            self.renumber();
        }
        true
    }

    /// The thread's line for a move to `board`.
    pub(crate) fn moved_line(&self, board: &str) -> String {
        let tag = self.tag_of(board);
        if tag.is_empty() {
            return "moved it to the default board".to_string();
        }
        format!("moved it to {tag}")
    }

    /// `""` for the space's first board, by whichever name a row gave it.
    pub(crate) fn own_board(&self, board: &str) -> String {
        if !board.is_empty() && board == self.board_view.own {
            return String::new();
        }
        board.to_string()
    }

    /// Reads the board list. True when it changed.
    pub(crate) fn read_board_view(&mut self) -> bool {
        let Some(f) = &self.board_view_fn else {
            return false;
        };
        let mut v = f();
        v.deleted.sort_by(|a, b| a.1.cmp(&b.1));
        if v == self.board_view {
            return false;
        }
        self.board_view = v;
        true
    }

    /// After a board or member row: reads the board list again, and
    /// derives every card's place and tag from it (card #563). A tag
    /// changes with the list, so none is taken from a create row for good.
    pub(crate) fn refresh_boards(&mut self) {
        if self.replaying {
            return;
        }
        if self.read_board_view() {
            self.renumber();
        }
        let mut tags: HashMap<String, String> = HashMap::new();
        let boards: BTreeSet<String> = self.tasks.values().map(|t| t.board.clone()).collect();
        for b in boards {
            let tag = self.tag_of(&b);
            tags.insert(b, tag);
        }
        let mut changed = Vec::new();
        for t in self.tasks.values_mut() {
            if let Some(tag) = tags.get(&t.board)
                && *tag != t.tag
            {
                t.tag = tag.clone();
                changed.push((t.id.clone(), t.holder.clone()));
            }
        }
        for (task_id, holder) in changed {
            self.note(Change {
                kind: TASK_CHANGED.into(),
                task_id,
                holder,
                ..Change::default()
            });
        }
    }

    /// Derives every card's board and short ref by walking the claims and
    /// the board deletes in RowPos order, so every device gets the same
    /// answer whatever order the rows came in (card #563).
    ///
    /// A card sits where its newest claim put it, and a number it left
    /// stays in `short_hist`. A delete hands each card on the board the
    /// next number on the first, at the delete's own place, and a claim
    /// naming a board already deleted takes one the same way: a number
    /// comes from what sits before it, never from what came after.
    ///
    /// A row with no number predates claims, and so predates boards: those
    /// count in one run whatever they say.
    pub(crate) fn renumber(&mut self) {
        self.claims.sort_by(|a, b| a.at.cmp(&b.at));
        self.short_groups.clear();
        self.short_hist.clear();
        let dels = self.board_view.deleted.clone();
        let mut tops: HashMap<String, i64> = HashMap::new();
        let mut live: HashMap<String, ShortClaim> = HashMap::new();
        let mut gone: HashSet<String> = HashSet::new();
        let mut hist: Vec<(String, i64, String)> = Vec::new();
        let (mut legacy, mut d) = (0, 0);
        let mut end = None;
        for cl in &self.claims {
            while let Some((b, at)) = dels.get(d).filter(|(_, at)| at.before(&cl.at)) {
                displace(b, at, &mut live, &mut tops, &mut hist);
                gone.insert(b.clone());
                d += 1;
            }
            let mut c = cl.clone();
            if gone.contains(&c.board) {
                c.board.clear();
                c.num = 0;
                c.displaced = true;
            }
            if c.num == 0 {
                if c.moved || c.displaced {
                    c.num = tops.get(&c.board).copied().unwrap_or(0) + 1;
                } else {
                    legacy += 1;
                    c.num = legacy;
                }
            }
            raise_top(&mut tops, &c.board, c.num);
            if let Some(was) = live.insert(c.id.clone(), c) {
                hist.push((was.board, was.num, was.id));
            }
            end = Some(cl.at.clone());
        }
        for (b, at) in &dels[d..] {
            displace(b, at, &mut live, &mut tops, &mut hist);
            end = Some(at.clone());
        }
        for (b, n) in tops {
            raise_top(&mut self.short_tops, &b, n);
        }
        for (board, num, id) in hist {
            let ids = self.short_hist.entry((board, num)).or_default();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        let mut claims: Vec<ShortClaim> = live.into_values().collect();
        claims.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
        for cl in claims {
            self.short_groups
                .entry((cl.board.clone(), cl.num))
                .or_default()
                .push(cl);
        }
        self.walked_to = end;
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
        self.short_idx
            .retain(|(b, s), _| *b != board || short_num(s) != base);
        for (i, cl) in g.iter().enumerate() {
            let (short, was) = if g.len() > 1 {
                (format!("{base}{}", short_suffix(i as i64)), base.clone())
            } else {
                (base.clone(), String::new())
            };
            let Some((old, on)) = self
                .tasks
                .get(&cl.id)
                .map(|t| (t.short.clone(), t.board.clone()))
            else {
                continue;
            };
            self.short_idx
                .insert((board.clone(), short.clone()), cl.id.clone());
            let moved = on != board;
            // As Go: the move is noted while the card still has its old
            // ref, and the new one is set after.
            if !self.replaying {
                if moved {
                    self.short_left(&cl.id, cl, &short);
                } else if old != short && !old.is_empty() {
                    self.short_moved_to(&cl.id, &old, &short);
                }
            }
            let tag = moved.then(|| self.tag_of(&board));
            if let Some(t) = self.tasks.get_mut(&cl.id) {
                t.short = short;
                t.short_was = was;
                if let Some(tag) = tag {
                    t.board = board.clone();
                    t.tag = tag;
                }
            }
        }
    }

    /// Notes that a card went to another board, outside replay: a move's
    /// row says so itself, and a deleted board's card hears it here.
    fn short_left(&mut self, id: &str, cl: &ShortClaim, to: &str) {
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
        if cl.displaced {
            let what = format!("its board was deleted, so it moved to the default board as #{to}");
            self.queue_watch(id, has_watchers, SWEEPER, &what, also);
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
    /// card `#3` with nothing that names it (card #558).
    pub(crate) fn short_hits(&self, tag: &str, short: &str) -> Vec<&Task> {
        let bare = short_num(short) == short;
        let hits: Vec<&Task> = self
            .short_idx
            .iter()
            .filter(|((_, s), _)| s == short || (bare && short_num(s) == short))
            .filter_map(|(_, id)| self.tasks.get(id))
            .filter(|t| tag.is_empty() || t.tag == tag)
            .collect();
        if tag.is_empty() && !self.on_board && hits.iter().any(|t| t.board.is_empty()) {
            return hits.into_iter().filter(|t| t.board.is_empty()).collect();
        }
        hits
    }

    /// The cards that once held `short`, on the boards showing `tag`: a
    /// moved card's old ref still finds it, whatever became of that board
    /// (card #563).
    fn old_hits(&self, tag: &str, short: &str) -> Vec<&Task> {
        let Ok(num) = short_num(short).parse::<i64>() else {
            return Vec::new();
        };
        let named = |b: &str| {
            tag.is_empty()
                || if b.is_empty() {
                    tag == self.tag()
                } else {
                    self.board_view
                        .tags
                        .iter()
                        .any(|(t, id)| t == tag && id == b)
                }
        };
        let mut ids: Vec<&String> = self
            .short_hist
            .iter()
            .filter(|((b, n), _)| *n == num && named(b))
            .flat_map(|(_, ids)| ids)
            .collect();
        ids.sort();
        ids.dedup();
        ids.iter().filter_map(|id| self.tasks.get(*id)).collect()
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
        match self.old_hits(&tag, &short).as_slice() {
            [t] => return Ok(t.id.clone()),
            [] => {}
            many => return Err(ambiguous(&short, many)),
        }
        let gone = self
            .short_idx
            .iter()
            .any(|((_, s), id)| *s == short && self.deleted.contains(id));
        if gone {
            return Err(not_found(format!("{v} was deleted")));
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
