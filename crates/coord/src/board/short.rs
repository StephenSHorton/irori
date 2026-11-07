//! Short refs `#12`, `#12a`, `mlg#12` (I:internal/coord/short.go,
//! coord.md §3.9): derived by the fold from every task.created row's
//! claimed number, settled the same way on every device.

use std::collections::{BTreeSet, HashMap, HashSet};

use gocompat::quote::quote;
use gocompat::time::GoTime;

use super::error::{Error, ErrorKind, Result, not_allowed, not_found};
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

/// A tag in a ref: a lowercase letter, then up to 15 of `a-z 0-9 -`.
/// Wider than a new board's tag (MAX_BOARD_TAG): boards made before
/// card #590 keep tags up to 16, and refs to them must still read.
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
    hist: &mut Vec<(String, i64, String, RowPos)>,
) {
    let mut on: Vec<ShortClaim> = live
        .values()
        .filter(|c| c.board == board)
        .cloned()
        .collect();
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
        hist.push((was.board, was.num, was.id, was.at));
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

/// Ambiguous, for an old ref more than one card has left.
fn ambiguous_old(typed: &str, cards: &[&Task]) -> Error {
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
    Error::new(
        ErrorKind::Ambiguous,
        format!("{typed} was more than one card, now {names}"),
    )
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
            self.walk();
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
            self.walk();
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

    /// Walks now; a replay walks once, when it ends.
    fn walk(&mut self) {
        if self.replaying {
            return;
        }
        self.renumber();
    }

    /// Whether a claim can change what the last walk gave, so only another
    /// walk will do: it sits before the walk's end, its card has a move
    /// waiting, or it names a deleted board. With no moves or deletes,
    /// claims don't touch each other.
    fn needs_walk(&self, cl: &ShortClaim) -> bool {
        if self.moved_ids.is_empty() && self.board_view.deleted.is_empty() {
            return false;
        }
        self.moved_ids.contains(&cl.id)
            || self.walked_to.as_ref().is_some_and(|w| cl.at.before(w))
            || self.board_view.deleted.iter().any(|(b, _)| *b == cl.board)
    }

    /// Records a move row's claim (card #563), whether or not its card has
    /// landed yet; false for a repeat. The walk decides what it does, so
    /// arrival order can't.
    pub(crate) fn fold_move(&mut self, id: &str, at: RowPos, board: &str, num: i64) -> bool {
        if id.is_empty() || !self.moved_rows.insert((at.dev.clone(), at.seq)) {
            return false;
        }
        self.moved_ids.insert(id.into());
        self.claims.push(ShortClaim {
            id: id.into(),
            at,
            num: num.max(0),
            board: self.own_board(board),
            moved: true,
            displaced: false,
        });
        self.walk();
        true
    }

    /// The thread's line for a move to `board`. It names the board the row
    /// named, even one deleted since, or history would change at a restart;
    /// a board the log doesn't hold yet isn't guessed at.
    pub(crate) fn moved_line(&self, board: &str) -> String {
        let b = self.own_board(board);
        let named = self.board_view.tags.iter().rfind(|(_, id)| *id == b);
        let tag = match named {
            _ if b.is_empty() => self.tag(),
            Some((t, _)) => t.clone(),
            None => match self.board_tag(&b) {
                Some(t) => t,
                None => return "moved it to another board".to_string(),
            },
        };
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

    /// Whether `board` is finished (card #690). The first board never is.
    pub(crate) fn board_finished(&self, board: &str) -> bool {
        let b = self.own_board(board);
        !b.is_empty() && self.board_view.finished.contains(&b)
    }

    /// The refusal for writing onto finished board `board`.
    pub(crate) fn finished_refusal(&self, board: &str) -> Error {
        let tag = self.board_tag(board).unwrap_or_else(|| board.to_string());
        not_allowed(format!(
            "board {} is finished: reopen it first, or pick another board",
            quote(&tag)
        ))
    }

    /// See [`crate::Coord::open_on_board`].
    pub(crate) fn open_on_board(&self, board: &str) -> Vec<Task> {
        let b = self.own_board(board);
        let mut out: Vec<Task> = self
            .tasks
            .values()
            .filter(|t| t.board == b && !t.withdrawn && !is_terminal(effective_state(t)))
            .map(|t| self.shown(t))
            .collect();
        out.sort_by(|a, b| {
            let pa = if a.parent.is_empty() {
                &a.id
            } else {
                &a.parent
            };
            let pb = if b.parent.is_empty() {
                &b.id
            } else {
                &b.parent
            };
            (pa, !a.parent.is_empty(), &a.id).cmp(&(pb, !b.parent.is_empty(), &b.id))
        });
        out
    }

    /// Reads the board list. True when it changed.
    pub(crate) fn read_board_view(&mut self) -> bool {
        let Some(mut v) = self.board_view_fn.as_ref().and_then(|f| f()) else {
            return false;
        };
        v.deleted.sort_by(|a, b| a.1.cmp(&b.1));
        v.finished.sort();
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
        let mut hist: Vec<(String, i64, String, RowPos)> = Vec::new();
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
                // The ref its writer saw still finds it.
                if c.num > 0 {
                    hist.push((c.board.clone(), c.num, c.id.clone(), c.at.clone()));
                }
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
                hist.push((was.board, was.num, was.id, was.at));
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
        for (board, num, id, at) in hist {
            let old = self.short_hist.entry((board, num)).or_default();
            if !old.iter().any(|(_, x)| *x == id) {
                old.push((at, id));
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
        // A group only grows between walks, so these are all it held.
        self.short_idx.remove(&(board.clone(), base.clone()));
        for i in 0..g.len() {
            let short = format!("{base}{}", short_suffix(i as i64));
            self.short_idx.remove(&(board.clone(), short));
        }
        let by_move = g.iter().any(|c| c.moved || c.displaced);
        for (i, cl) in g.iter().enumerate() {
            let (short, was) = if g.len() > 1 {
                // A card that came by a move never held the bare number.
                let held = !cl.moved && !cl.displaced;
                let was = if held { base.clone() } else { String::new() };
                (format!("{base}{}", short_suffix(i as i64)), was)
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
                    self.short_moved_to(&cl.id, &old, &short, by_move);
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
            let tag = self.tag();
            let what =
                format!("its board was deleted, so it moved to the default board as {tag}#{to}");
            self.queue_watch(id, has_watchers, SWEEPER, &what, also);
        }
    }

    /// Notes that a card's short ref moved, outside replay.
    pub(crate) fn short_moved_to(&mut self, id: &str, from: &str, to: &str, by_move: bool) {
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
        if num != to && by_move {
            what.push_str(&format!(
                ": a card that came from another board took #{num} too, so #{num} alone names none of them now"
            ));
        } else if num != to {
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
        // And one that left it keeps that ref (card #563), or `home#3`
        // would take over the `#3` a moved card's links still say.
        if bare
            && let Ok(n) = short.parse::<i64>()
            && let Some(ids) = self.short_hist.get(&(String::new(), n))
        {
            let old: Vec<&Task> = ids
                .iter()
                .filter_map(|(_, id)| self.tasks.get(id))
                .collect();
            if !old.is_empty() {
                return old;
            }
        }
        hits
    }

    /// The cards that once held `short`, on the boards showing `tag`: a
    /// moved card's old ref still finds it, whatever became of that board
    /// (card #563). A bare number means whoever left it. A lettered ref
    /// means one claimant: the letters went by RowPos among everyone who
    /// held the number, so the same order finds the same card.
    pub(crate) fn old_hits(&self, tag: &str, short: &str) -> Vec<&Task> {
        let base = short_num(short);
        let letters = &short[base.len()..];
        let Ok(num) = base.parse::<i64>() else {
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
        let mut ids: Vec<&String> = Vec::new();
        for ((b, n), old) in &self.short_hist {
            if *n != num || !named(b) {
                continue;
            }
            if letters.is_empty() {
                ids.extend(old.iter().map(|(_, id)| id));
                continue;
            }
            let mut all: Vec<(&RowPos, &String)> = old.iter().map(|(at, id)| (at, id)).collect();
            if let Some(g) = self.short_groups.get(&(b.clone(), num)) {
                all.extend(g.iter().map(|c| (&c.at, &c.id)));
            }
            all.sort();
            if all.len() < 2 {
                continue;
            }
            let hit = (0..all.len()).find(|i| short_suffix(*i as i64) == letters);
            ids.extend(hit.map(|i| all[i].1));
        }
        ids.sort();
        ids.dedup();
        ids.iter().filter_map(|id| self.tasks.get(*id)).collect()
    }

    /// The cards a ref in text means: who holds it now, else who held it.
    pub(crate) fn ref_hits(&self, tag: Option<&str>, short: &str) -> Vec<&Task> {
        let tag = tag.unwrap_or("");
        let hits = self.short_hits(tag, short);
        if hits.is_empty() {
            return self.old_hits(tag, short);
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
        // Each card's create claim, where the one run numbered it: a move
        // (card #563) came after that run and changes none of its letters.
        let mut made: Vec<&ShortClaim> = self.claims.iter().filter(|c| !c.moved).collect();
        made.sort_by(|a, b| a.at.cmp(&b.at));
        let mut legacy = 0;
        let mut g: Vec<&ShortClaim> = Vec::new();
        for cl in made {
            let n = if cl.num == 0 {
                legacy += 1;
                legacy
            } else {
                cl.num
            };
            if n == num {
                g.push(cl);
            }
        }
        let i = (0..g.len()).find(|i| short_suffix(*i as i64) == letters)?;
        let cl = g[i];
        (tag.is_empty() || self.tag_of(&cl.board) == tag).then_some(cl.id.as_str())
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
            many => return Err(ambiguous_old(&v, many)),
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

    /// [`Self::task_ref_locked`] that also names a deleted card, for taking
    /// a wait on it off (card #570).
    pub(crate) fn removal_ref(&self, r: &str) -> Result<String> {
        // Only a card not found may be a deleted one: an ambiguous ref
        // stays ambiguous.
        let e = match self.task_ref_locked(r) {
            Ok(id) => return Ok(id),
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(e),
            Err(e) => e,
        };
        let mut v = trim_space(r).to_string();
        if let Some((k, id)) = v.split_once(':')
            && gocompat::text::equal_fold(trim_space(k), "task")
        {
            v = trim_space(id).to_string();
        }
        let Some((tag, short)) = parse_ref(&v) else {
            return Err(e);
        };
        let base = short_num(&short);
        let Ok(n) = base.parse::<i64>() else {
            return Err(e);
        };
        // A claim outlives its card, so it still names a deleted one. More
        // than one deleted card with that ref names none of them.
        let mut ids: Vec<&str> = Vec::new();
        for ((b, num), g) in &self.short_groups {
            if *num != n || !(tag.is_empty() || self.tag_of(b) == tag) {
                continue;
            }
            for (i, cl) in g.iter().enumerate() {
                let named = if g.len() > 1 {
                    short == format!("{base}{}", short_suffix(i as i64))
                } else {
                    short == base
                };
                if named && self.deleted.contains(&cl.id) {
                    ids.push(&cl.id);
                }
            }
        }
        match ids.as_slice() {
            [id] => Ok(id.to_string()),
            _ => Err(e),
        }
    }

    /// Whether id's short ref changed at or after `since`.
    pub(crate) fn moved_since(&self, id: &str, since: &GoTime) -> bool {
        self.short_moved
            .get(id)
            .is_some_and(|at| !crate::gotime::before(at, since))
    }
}
