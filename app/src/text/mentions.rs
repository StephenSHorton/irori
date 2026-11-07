//! Mentions (card #77, docs/design/board.md "Mentions"): the pure part of
//! the @name picker and the #card picker (the React app's lib/mentions.ts).
//! The text keeps a readable "@name"; the key beside it, recorded when the
//! person picks a row, decides who hears. The window never resolves a name
//! to a key by itself, except the one case the design allows: a query that
//! is exactly one live agent's name when the person types a space or sends.
//!
//! Offsets are UTF-8 bytes into the text (a `TextEdit`'s caret).

use irori_window::cards::{AgentRef, CardSummary, Holding};
use irori_window::mentions::MentionOutcome;

/// One agent a box names (lib/mentions.ts `Mention`, the backend's
/// `ts::Mention`): what the person picked, the key deciding who hears.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Mention {
    pub key: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gone: bool,
}

impl Mention {
    /// As the backend's calls take it.
    pub fn to_ts(&self) -> irori_window::ts::Mention {
        irori_window::ts::Mention {
            key: self.key.clone(),
            name: self.name.clone(),
            gone: self.gone,
        }
    }
}

/// Mentions as the backend's calls take them.
pub fn to_ts(ms: &[Mention]) -> Vec<irori_window::ts::Mention> {
    ms.iter().map(Mention::to_ts).collect()
}

/// One picker row: lib/mentions.ts `MentionCandidate`, the backend's
/// `MentionCandidate` plus `card`: a "the agent on #N" row (card #452),
/// the agent holding that card now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Candidate {
    pub who: AgentRef,
    pub cwd: String,
    pub repo: String,
    /// Another person on a shared board, or one of their agents: the board
    /// they can be mentioned on (card #177). "" for your own.
    pub board: String,
    /// "#N" or "tag#N" for a card row, else "".
    pub card: String,
}

impl From<irori_window::mentions::MentionCandidate> for Candidate {
    fn from(c: irori_window::mentions::MentionCandidate) -> Candidate {
        Candidate {
            who: c.who,
            cwd: c.cwd,
            repo: c.repo,
            board: c.board,
            card: String::new(),
        }
    }
}

/// A card row's words in the picker.
pub fn agent_on_label(card: &str) -> String {
    format!("the agent on {card}")
}

/// What a row is found by and shows as: its name, or for a card row
/// "the agent on #N".
pub fn row_name(c: &Candidate) -> String {
    if c.card.is_empty() {
        c.who.name.clone()
    } else {
        agent_on_label(&c.card)
    }
}

/// The "the agent on #N" rows: one per card an agent holds and is live on
/// (not the person, not gone), with the card the box sits on (`here`)
/// first, then the newest. The holder's own row from `cands` is reused so
/// its device and colour show the same.
pub fn agent_on_rows(cards: &[CardSummary], cands: &[Candidate], here: &str) -> Vec<Candidate> {
    let mut rows: Vec<(bool, i64, Candidate)> = Vec::new();
    for c in cards {
        let who = &c.holder.who;
        if c.short.is_empty() || who.key.is_empty() || who.key.starts_with("user:") {
            continue;
        }
        if matches!(c.holder.state, Holding::Person | Holding::Gone | Holding::Nobody) {
            continue;
        }
        if is_gone(who) {
            continue;
        }
        let known = cands.iter().find(|x| x.who.key == who.key && x.card.is_empty());
        rows.push((
            c.id == here,
            c.short.parse().unwrap_or(0),
            Candidate {
                who: known.map_or_else(|| who.clone(), |k| k.who.clone()),
                cwd: known.map(|k| k.cwd.clone()).unwrap_or_default(),
                repo: known.map(|k| k.repo.clone()).unwrap_or_default(),
                board: known.map(|k| k.board.clone()).unwrap_or_default(),
                card: format!("{}#{}", c.tag, c.short),
            },
        ));
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    rows.into_iter().map(|r| r.2).collect()
}

/// The candidates a box on board `board` may mention: your own agents
/// everywhere, another person and their agents on their board.
pub fn board_rows(cands: &[Candidate], board: &str) -> Vec<Candidate> {
    cands
        .iter()
        .filter(|c| c.board.is_empty() || c.board == board)
        .cloned()
        .collect()
}

/// coord's MAX_MENTIONS.
pub const MAX_MENTIONS: usize = 10;
/// How many rows the picker shows.
pub const MAX_ROWS: usize = 20;

/// A token's "@" starts the text or follows whitespace or "(".
fn before(c: char) -> bool {
    c.is_whitespace() || c == '('
}

/// A mention in the text ends at the end, whitespace or this punctuation.
pub fn after(c: char) -> bool {
    c.is_whitespace() || ".,;:!?)'\"".contains(c)
}

/// The "@query" (or "#query") the caret is in: where its sign is, and
/// what follows it up to the caret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub query: String,
}

fn tok(start: usize, query: &str) -> Token {
    Token {
        start,
        query: query.to_string(),
    }
}

/// `query` without closing punctuation at its end ("@oak," reads oak).
fn bare(query: &str) -> &str {
    query.trim_end_matches(|c: char| ".,;:!?)'\"".contains(c))
}

/// The agent has left (not merely offline).
pub fn is_gone(who: &AgentRef) -> bool {
    who.status == "gone"
}

fn char_before(text: &str, i: usize) -> Option<char> {
    text.get(..i).and_then(|h| h.chars().last())
}

fn char_at(text: &str, i: usize) -> Option<char> {
    text.get(i..).and_then(|t| t.chars().next())
}

/// The token for `sign` at the caret: the last `sign` only (an earlier
/// one's query would hold this one); a space ends it unless some of
/// `names` still holds the query.
fn token_for(text: &str, caret: usize, sign: char, names: &[String], card: bool) -> Option<Token> {
    let head = text.get(..caret.min(text.len()))?;
    let at = head.rfind(sign)?;
    if at > 0 && !char_before(head, at).is_some_and(before) {
        return None;
    }
    let query = &head[at + 1..];
    if query.contains('\n') || query.chars().count() > 64 || (card && query.contains('@')) {
        return None;
    }
    if !query.chars().any(char::is_whitespace) {
        return Some(tok(at, query));
    }
    let q = query.to_lowercase();
    names
        .iter()
        .any(|n| n.to_lowercase().contains(&q))
        .then(|| tok(at, query))
}

/// The "@query" the caret is in, or None. The query runs from the "@" to
/// the caret; a space or newline ends it, unless some candidate's name
/// still holds the query with the space in it (names may hold spaces).
pub fn token_at(text: &str, caret: usize, names: &[String]) -> Option<Token> {
    token_for(text, caret, '@', names, false)
}

/// The rows for `query`: names holding it, ignoring case. Live agents
/// first, gone ones last; among each, names that start with the query, then
/// this device's agents (no device chip), then by name. At most MAX_ROWS.
pub fn filter_candidates(cands: &[Candidate], query: &str) -> Vec<Candidate> {
    let q = bare(query).to_lowercase();
    let qn = q.strip_prefix('#').unwrap_or(&q);
    let matches = |c: &Candidate| {
        if c.card.is_empty() {
            return c.who.name.to_lowercase().contains(&q);
        }
        let card = c.card.to_lowercase();
        let num = card.rsplit('#').next().unwrap_or("");
        agent_on_label(&c.card).to_lowercase().contains(&q) || (!q.is_empty() && num.starts_with(qn))
    };
    let rank = |c: &Candidate| {
        let n = row_name(c).to_lowercase();
        [
            u8::from(is_gone(&c.who)),
            u8::from(q.is_empty() && c.card.is_empty()),
            u8::from(!(n.starts_with(&q) || (!c.card.is_empty() && q.starts_with('#')))),
            u8::from(!c.who.device.is_empty()),
        ]
    };
    let mut rows: Vec<(usize, [u8; 4], &Candidate)> = cands
        .iter()
        .filter(|c| matches(c))
        .enumerate()
        .map(|(i, c)| (i, rank(c), c))
        .collect();
    rows.sort_by(|a, b| {
        a.1.cmp(&b.1).then_with(|| {
            if !a.2.card.is_empty() && !b.2.card.is_empty() {
                return a.0.cmp(&b.0);
            }
            let (x, y) = (row_name(a.2), row_name(b.2));
            x.to_lowercase().cmp(&y.to_lowercase()).then(x.cmp(&y))
        })
    });
    rows.into_iter().take(MAX_ROWS).map(|r| r.2.clone()).collect()
}

/// The next highlighted row: -1 is none (the first row is then the
/// default). Forward from none is the first row, back from none the last;
/// both wrap.
pub fn move_highlight(i: i32, count: usize, dir: i32) -> i32 {
    let n = count as i32;
    if n <= 0 {
        return -1;
    }
    if i < 0 || i >= n {
        return if dir > 0 { 0 } else { n - 1 };
    }
    (i + dir + n) % n
}

/// `ins` (plus a space unless one follows) in place of the token and the
/// rest of the word the caret is in: the new text and where the caret goes.
fn insert_word(text: &str, t: &Token, caret: usize, ins: &str) -> (String, usize) {
    let mut end = caret.min(text.len());
    while let Some(c) = char_at(text, end) {
        if after(c) {
            break;
        }
        end += c.len_utf8();
    }
    let rest = &text[end..];
    let space = !rest.starts_with(char::is_whitespace);
    let ins = if space { format!("{ins} ") } else { ins.to_string() };
    let out = format!("{}{}{}", &text[..t.start], ins, rest);
    let caret = (t.start + ins.len() + usize::from(!space)).min(out.len());
    (out, caret)
}

/// Puts "@name " in place of the token and the rest of the word the caret
/// is in; returns the text and where the caret goes.
pub fn insert_mention(text: &str, t: &Token, caret: usize, name: &str) -> (String, usize) {
    insert_word(text, t, caret, &format!("@{name}"))
}

/// `text` holds "@name" at `i` as a whole mention.
fn mention_at(text: &str, i: usize, name: &str) -> bool {
    if i > 0 && !char_before(text, i).is_some_and(before) {
        return false;
    }
    let end = i + 1 + name.len();
    end == text.len() || char_at(text, end).is_some_and(after)
}

fn starts_at(text: &str, i: usize, s: &str) -> bool {
    text.get(i..).is_some_and(|t| t.starts_with(s))
}

/// How many whole "@name" spans `text` holds. An "@name" that is the start
/// of a longer recorded one ("@oak" in "@oak tree") doesn't count.
pub fn count_mention(text: &str, name: &str, longer: &[String]) -> usize {
    if name.is_empty() {
        return 0;
    }
    let needle = format!("@{name}");
    let over: Vec<&String> = longer
        .iter()
        .filter(|n| n.len() > name.len() && n.starts_with(name))
        .collect();
    text.match_indices(&needle)
        .filter(|&(i, _)| {
            mention_at(text, i, name)
                && !over.iter().any(|o| starts_at(text, i + 1, o) && mention_at(text, i, o))
        })
        .count()
}

/// `text` still holds "@name" (exact, case as inserted).
pub fn has_mention(text: &str, name: &str, longer: &[String]) -> bool {
    count_mention(text, name, longer) > 0
}

/// Drops the mentions whose "@name" left the text. Two mentions with one
/// name (two agents called oak) need two "@oak" spans: with fewer, the
/// latest-added go first.
pub fn keep_mentions(mentions: &[Mention], text: &str) -> Vec<Mention> {
    let names: Vec<String> = mentions.iter().map(|m| m.name.clone()).collect();
    let mut left: Vec<(String, usize)> = Vec::new();
    mentions
        .iter()
        .filter(|m| {
            let i = match left.iter().position(|(n, _)| *n == m.name) {
                Some(i) => i,
                None => {
                    left.push((m.name.clone(), count_mention(text, &m.name, &names)));
                    left.len() - 1
                }
            };
            if left[i].1 == 0 {
                return false;
            }
            left[i].1 -= 1;
            true
        })
        .cloned()
        .collect()
}

/// `mentions` holds MAX_MENTIONS already.
pub fn is_full(mentions: &[Mention]) -> bool {
    mentions.len() >= MAX_MENTIONS
}

/// Records `m`, once per key, at most MAX_MENTIONS.
pub fn add_mention(mentions: &[Mention], m: Mention) -> Vec<Mention> {
    let mut out = mentions.to_vec();
    if !mentions.iter().any(|x| x.key == m.key) && mentions.len() < MAX_MENTIONS {
        out.push(m);
    }
    out
}

/// The one live candidate named `query` exactly (ignoring case), or None
/// when none or several are.
pub fn exact_unique<'a>(query: &str, cands: &'a [Candidate]) -> Option<&'a Candidate> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return None;
    }
    let mut hits = cands
        .iter()
        .filter(|c| !is_gone(&c.who) && c.who.name.to_lowercase() == q);
    let hit = hits.next()?;
    hits.next().is_none().then_some(hit)
}

fn mention(key: &str, name: &str) -> Mention {
    Mention {
        key: key.to_string(),
        name: name.to_string(),
        gone: false,
    }
}

/// Records the token at the caret when it is exactly one live agent's name
/// (a space typed after it, or the text sent). The name is kept as typed.
pub fn settle(text: &str, caret: usize, mentions: &[Mention], cands: &[Candidate]) -> Vec<Mention> {
    let names: Vec<String> = cands.iter().map(|c| c.who.name.clone()).collect();
    let Some(t) = token_at(text, caret, &names) else {
        return mentions.to_vec();
    };
    let name = bare(&t.query);
    match exact_unique(name, cands) {
        Some(hit) => add_mention(mentions, mention(&hit.who.key, name)),
        None => mentions.to_vec(),
    }
}

/// Records each whole "@name" in `text` whose "@" is in [from, to) and
/// whose name is exactly one live agent's (pasted text, or a mention undo
/// brought back). Longest names first, so "@oak tree" wins over "@oak".
pub fn settle_range(
    text: &str,
    from: usize,
    to: usize,
    mentions: &[Mention],
    cands: &[Candidate],
) -> Vec<Mention> {
    let mut names: Vec<&str> = Vec::new();
    for c in cands.iter().filter(|c| !is_gone(&c.who)) {
        if !c.who.name.is_empty() && !names.contains(&c.who.name.as_str()) {
            names.push(&c.who.name);
        }
    }
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    let lower = text.to_lowercase();
    let mut out = mentions.to_vec();
    for (i, _) in text.match_indices('@') {
        if i < from || i >= to {
            continue;
        }
        if i > 0 && !char_before(text, i).is_some_and(before) {
            continue;
        }
        let Some(n) = names
            .iter()
            .find(|x| starts_at(&lower, i + 1, &x.to_lowercase()) && mention_at(text, i, x))
        else {
            continue;
        };
        let Some(written) = text.get(i + 1..i + 1 + n.len()) else { continue };
        if let Some(hit) = exact_unique(written, cands)
            && !out.iter().any(|m| m.key == hit.who.key)
        {
            out = add_mention(&out, mention(&hit.who.key, written));
        }
    }
    out
}

/// What a key does with the picker open: `Hold` swallows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKey {
    Next,
    Prev,
    Pick,
    Close,
    Hold,
}

/// Maps a key press (its DOM name: "Tab", "ArrowDown", "ArrowUp", "Enter",
/// "Escape") to the picker: only while it is open, and never while an IME
/// is composing. With no rows (a typo, or the list still loading) Enter is
/// held, so it doesn't send what the person can see is unfinished; Tab
/// moves focus as usual.
pub fn menu_key(key: &str, shift: bool, composing: bool, open: bool, rows: usize) -> Option<MenuKey> {
    if !open || composing {
        return None;
    }
    if key == "Escape" {
        return Some(MenuKey::Close);
    }
    if rows == 0 {
        return (key == "Enter" && !shift).then_some(MenuKey::Hold);
    }
    match key {
        "Tab" if shift => Some(MenuKey::Prev),
        "Tab" | "ArrowDown" => Some(MenuKey::Next),
        "ArrowUp" => Some(MenuKey::Prev),
        "Enter" if !shift => Some(MenuKey::Pick),
        _ => None,
    }
}

/// Anything a mention span can name: a [`Mention`], or a richer type.
pub trait Named {
    fn name(&self) -> &str;
}

impl Named for Mention {
    fn name(&self) -> &str {
        &self.name
    }
}

/// A piece of text: plain, or one mention's "@name".
#[derive(Clone, Debug, PartialEq)]
pub enum MentionSpan<'a, M> {
    Text(String),
    Mention(String, &'a M),
}

/// Cuts text around each "@name" a mention names (longest name first, so
/// "@oak tree" wins over "@oak"). Any other "@word" stays text. Same-named
/// mentions go out in order: the k-th "@oak" is the k-th mention named oak.
pub fn split_mentions<'a, M: Named>(text: &str, mentions: &'a [M]) -> Vec<MentionSpan<'a, M>> {
    let mut named: Vec<&M> = mentions.iter().filter(|m| !m.name().is_empty()).collect();
    named.sort_by_key(|m| std::cmp::Reverse(m.name().len()));
    if named.is_empty() || !text.contains('@') {
        return vec![MentionSpan::Text(text.to_string())];
    }
    let mut used: Vec<(&str, usize)> = Vec::new();
    let mut out = Vec::new();
    let mut from = 0;
    for (i, _) in text.match_indices('@') {
        if i < from {
            continue;
        }
        let Some(hit) = named
            .iter()
            .find(|x| starts_at(text, i + 1, x.name()) && mention_at(text, i, x.name()))
        else {
            continue;
        };
        let name = hit.name();
        let all: Vec<&M> = mentions.iter().filter(|x| x.name() == name).collect();
        let k = match used.iter_mut().find(|(n, _)| *n == name) {
            Some((_, k)) => {
                *k += 1;
                *k - 1
            }
            None => {
                used.push((name, 1));
                0
            }
        };
        let m = all[k.min(all.len() - 1)];
        if i > from {
            out.push(MentionSpan::Text(text[from..i].to_string()));
        }
        let end = i + 1 + name.len();
        out.push(MentionSpan::Mention(text[i..end].to_string(), m));
        from = end;
    }
    if from < text.len() {
        out.push(MentionSpan::Text(text[from..].to_string()));
    }
    out
}

/// The line under a box after a send: each mention that didn't reach its
/// agent, or "" when all did.
pub fn not_delivered(outcomes: &[MentionOutcome]) -> String {
    let parts: Vec<String> = outcomes
        .iter()
        .filter(|o| o.state != "woke")
        .map(|o| match o.state.as_str() {
            "gone" => format!("{} is gone", o.name),
            "unknown" => format!("no agent named {}", o.name),
            "held" if o.note.is_empty() => format!("{} is held", o.name),
            "held" => format!("{} is held ({})", o.name, o.note),
            _ if o.note.is_empty() => format!("{} failed", o.name),
            _ => format!("{}: {}", o.name, o.note),
        })
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("Not delivered: {}.", parts.join("; "))
    }
}

/// The ask box's line while mentions target it: "Asking @oak only",
/// "Asking @oak and @fern only".
pub fn asking_line(mentions: &[Mention]) -> String {
    let names: Vec<String> = mentions.iter().map(|m| format!("@{}", m.name)).collect();
    match names.as_slice() {
        [] => String::new(),
        [one] => format!("Asking {one} only"),
        [head @ .., last] => format!("Asking {} and {last} only", head.join(", ")),
    }
}

/// How long the "Not delivered" line stays under a box.
pub const MISSED_MS: u64 = 8000;

/// One card ref in text (card #114): "#12" (tag None) or "mlg#12", as the
/// Core's card_refs reads them: a whole word, after the start, whitespace
/// or "(", and not followed by a letter, digit or "_".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardRef {
    pub start: usize,
    pub end: usize,
    pub tag: Option<String>,
    pub n: String,
}

fn tag_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

pub fn card_refs(text: &str) -> Vec<CardRef> {
    let mut out = Vec::new();
    for (p, _) in text.match_indices('#') {
        let digits = text[p + 1..].bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            continue;
        }
        let end = p + 1 + digits;
        if char_at(text, end).is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let tag_len: usize = text[..p]
            .chars()
            .rev()
            .take_while(|&c| tag_char(c))
            .map(char::len_utf8)
            .sum();
        let start = p - tag_len;
        if start > 0 && !char_before(text, start).is_some_and(before) {
            continue;
        }
        out.push(CardRef {
            start,
            end,
            tag: (tag_len > 0).then(|| text[start..p].to_string()),
            n: text[p + 1..end].to_string(),
        });
    }
    out
}

/// What a card ref names among what the window already holds (card #144).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefHit {
    pub thread: bool,
    pub id: String,
    pub title: String,
}

/// A card or thread a ref may name: its id, number, title and tag (or a
/// thread's space).
#[derive(Clone, Debug, Default)]
pub struct RefRow<'a> {
    pub id: &'a str,
    pub short: &'a str,
    pub title: &'a str,
    /// A card's tag, a thread's space id ("" for none).
    pub tag: &'a str,
}

impl<'a> From<&'a CardSummary> for RefRow<'a> {
    fn from(c: &'a CardSummary) -> RefRow<'a> {
        RefRow {
            id: &c.id,
            short: &c.short,
            title: &c.title,
            tag: &c.tag,
        }
    }
}

/// Resolves `r` against the live cards, then the threads (own board, so
/// only an untagged ref). More than one live card leaves it to the lookup;
/// so does a bare ref whose only live card is on a shared board (card
/// #343), and a space's thread (card #558).
pub fn local_ref(r: &CardRef, cards: &[RefRow], threads: &[RefRow]) -> Option<RefHit> {
    let c: Vec<&RefRow> = cards
        .iter()
        .filter(|x| x.short == r.n && r.tag.as_deref().is_none_or(|t| x.tag == t))
        .collect();
    if c.len() == 1 && (r.tag.is_some() || c[0].tag.is_empty()) {
        return Some(RefHit {
            thread: false,
            id: c[0].id.to_string(),
            title: c[0].title.to_string(),
        });
    }
    if !c.is_empty() || r.tag.is_some() {
        return None;
    }
    let t: Vec<&RefRow> = threads
        .iter()
        .filter(|x| x.short == r.n && x.tag.is_empty())
        .collect();
    (t.len() == 1).then(|| RefHit {
        thread: true,
        id: t[0].id.to_string(),
        title: t[0].title.to_string(),
    })
}

/// A ref as the backend reads it: "#12" or "mlg#12".
pub fn ref_text(r: &CardRef) -> String {
    format!("{}#{}", r.tag.as_deref().unwrap_or(""), r.n)
}

/// The "#query" the caret is in (card #114), or None: a "#" at the start or
/// after whitespace or "(", up to the caret, no newline or "@". A space
/// ends it unless some card title still holds the query.
pub fn card_token_at(text: &str, caret: usize, titles: &[String]) -> Option<Token> {
    token_for(text, caret, '#', titles, true)
}

/// One row of the # picker.
#[derive(Clone, Debug, PartialEq)]
pub struct CardCandidate {
    pub id: String,
    /// "#12" or "mlg#12".
    pub r#ref: String,
    pub short: String,
    pub title: String,
}

/// The # picker's rows for `query`: a number matches the cards whose number
/// starts with it, words match titles. Number matches first, then title
/// starts, then the rest in the list's order.
pub fn filter_cards(cards: &[CardCandidate], query: &str) -> Vec<CardCandidate> {
    let q = bare(query).to_lowercase();
    let q = q.trim();
    if q.is_empty() {
        return cards.iter().take(MAX_ROWS).cloned().collect();
    }
    let rank = |c: &CardCandidate| {
        if c.short.starts_with(q) {
            0
        } else if c.title.to_lowercase().starts_with(q) {
            1
        } else {
            2
        }
    };
    let mut rows: Vec<(u8, &CardCandidate)> = cards
        .iter()
        .filter(|c| c.short.starts_with(q) || c.title.to_lowercase().contains(q))
        .map(|c| (rank(c), c))
        .collect();
    rows.sort_by_key(|r| r.0);
    rows.into_iter().take(MAX_ROWS).map(|r| r.1.clone()).collect()
}

/// Puts "#12 " (or "mlg#12 ") in place of the token and the rest of the
/// word the caret is in.
pub fn insert_card_ref(text: &str, t: &Token, caret: usize, r: &str) -> (String, usize) {
    insert_word(text, t, caret, r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(key: &str, name: &str, status: &str, device: &str) -> Candidate {
        Candidate {
            who: AgentRef {
                key: key.into(),
                name: name.into(),
                status: status.into(),
                live: status != "gone",
                where_: if device.is_empty() {
                    "Macbook Pro (this one)".into()
                } else {
                    device.into()
                },
                here: device.is_empty(),
                device: device.into(),
                ..AgentRef::default()
            },
            ..Candidate::default()
        }
    }

    struct Fx {
        all: Vec<Candidate>,
        names: Vec<String>,
    }

    fn fx() -> Fx {
        let oak = cand("agent:oak", "oak", "idle", "");
        let oak_mac = cand("agent:oak-mac", "oak", "idle", "Mac");
        let oaktree = cand("agent:oaktree", "oak tree", "idle", "");
        let fern = cand("agent:fern", "fern", "idle", "");
        let cedar = cand("agent:cedar", "cedar", "gone", "");
        let boak = cand("agent:boak", "boak", "idle", "");
        let all = vec![fern, cedar, oak_mac, boak, oaktree, oak];
        let names = all.iter().map(|c| c.who.name.clone()).collect();
        Fx { all, names }
    }

    fn keys(cs: &[Candidate]) -> Vec<String> {
        cs.iter().map(|c| c.who.key.clone()).collect()
    }

    fn t(start: usize, q: &str) -> Option<Token> {
        Some(tok(start, q))
    }

    fn sv(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn token_at_starts_after_whitespace_or_paren() {
        assert_eq!(token_at("@oa", 3, &[]), t(0, "oa"));
        assert_eq!(token_at("hi @", 4, &[]), t(3, ""));
        assert_eq!(token_at("hi\n@f", 5, &[]), t(3, "f"));
        assert_eq!(token_at("(@f", 3, &[]), t(1, "f"));
        assert_eq!(token_at("mail a@b", 8, &[]), None);
        assert_eq!(token_at("@fern and more", 3, &[]), t(0, "fe"));
    }

    #[test]
    fn a_space_ends_it_unless_a_name_holds_it() {
        let f = fx();
        assert_eq!(token_at("@oak hello", 10, &f.names), None);
        assert_eq!(token_at("@oak ", 5, &sv(&["oak"])), None);
        assert_eq!(token_at("@oak t", 6, &f.names), t(0, "oak t"));
        assert_eq!(token_at("@oak\n", 5, &f.names), None);
    }

    #[test]
    fn filter_candidates_orders_rows() {
        let f = fx();
        assert_eq!(
            keys(&filter_candidates(&f.all, "OAK")),
            sv(&["agent:oak", "agent:oaktree", "agent:oak-mac", "agent:boak"])
        );
        assert_eq!(
            keys(&filter_candidates(&f.all, "")),
            sv(&["agent:boak", "agent:fern", "agent:oak", "agent:oaktree", "agent:oak-mac", "agent:cedar"])
        );
        let many: Vec<Candidate> = (0..30).map(|i| cand(&format!("k{i}"), &format!("a{i}"), "idle", "")).collect();
        assert_eq!(filter_candidates(&many, "a").len(), 20);
    }

    #[test]
    fn move_highlight_steps_and_wraps() {
        let mut i = -1;
        let mut seen = Vec::new();
        for _ in 0..4 {
            i = move_highlight(i, 3, 1);
            seen.push(i);
        }
        assert_eq!(seen, vec![0, 1, 2, 0]);
        assert_eq!(move_highlight(-1, 3, -1), 2);
        assert_eq!(move_highlight(0, 3, -1), 2);
        assert_eq!(move_highlight(2, 3, -1), 1);
        assert_eq!(move_highlight(0, 0, 1), -1);
    }

    #[test]
    fn menu_key_guards_enter() {
        use MenuKey::*;
        assert_eq!(menu_key("Tab", false, false, true, 2), Some(Next));
        assert_eq!(menu_key("Tab", true, false, true, 2), Some(Prev));
        assert_eq!(menu_key("ArrowDown", false, false, true, 2), Some(Next));
        assert_eq!(menu_key("ArrowUp", false, false, true, 2), Some(Prev));
        assert_eq!(menu_key("Enter", false, false, true, 2), Some(Pick));
        assert_eq!(menu_key("Escape", false, false, true, 2), Some(Close));
        assert_eq!(menu_key("Enter", false, false, false, 2), None);
        assert_eq!(menu_key("Tab", false, false, false, 2), None);
        assert_eq!(menu_key("Enter", false, false, true, 0), Some(Hold));
        assert_eq!(menu_key("Enter", true, false, true, 0), None);
        assert_eq!(menu_key("Tab", false, false, true, 0), None);
        assert_eq!(menu_key("Escape", false, false, true, 0), Some(Close));
        assert_eq!(menu_key("Enter", true, false, true, 2), None);
        assert_eq!(menu_key("Enter", false, true, true, 2), None);
        assert_eq!(menu_key("Escape", false, true, true, 2), None);
    }

    #[test]
    fn insert_mention_replaces_the_query_and_word() {
        assert_eq!(insert_mention("hi @oa", &tok(3, "oa"), 6, "oak"), ("hi @oak ".into(), 8));
        assert_eq!(insert_mention("@f there", &tok(0, "f"), 2, "fern"), ("@fern there".into(), 6));
        assert_eq!(insert_mention("@foak hello", &tok(0, "f"), 2, "fern"), ("@fern hello".into(), 6));
        assert_eq!(insert_mention("@oak", &tok(0, "oa"), 3, "oak"), ("@oak ".into(), 5));
        assert_eq!(insert_mention("@fx, hi", &tok(0, "f"), 2, "fern"), ("@fern , hi".into(), 6));
    }

    #[test]
    fn punctuation_after_a_name() {
        let f = fx();
        assert_eq!(settle("@fern,", 6, &[], &f.all), vec![mention("agent:fern", "fern")]);
        assert_eq!(keys(&filter_candidates(&f.all, "fern.")), sv(&["agent:fern"]));
    }

    #[test]
    fn same_named_mentions() {
        let ms = vec![mention("agent:oak", "oak"), mention("agent:oak-mac", "oak")];
        assert_eq!(keep_mentions(&ms, "@oak hi"), vec![ms[0].clone()]);
        assert_eq!(keep_mentions(&ms, "@oak and @oak"), ms);
        assert!(keep_mentions(&ms, "hi").is_empty());
        let got: Vec<String> = split_mentions("@oak and @oak", &ms)
            .into_iter()
            .filter_map(|x| match x {
                MentionSpan::Mention(_, m) => Some(m.key.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(got, sv(&["agent:oak", "agent:oak-mac"]));
    }

    #[test]
    fn settle_range_records_exact_unique_names() {
        let f = fx();
        assert_eq!(settle_range("@fern please review", 0, 19, &[], &f.all), vec![mention("agent:fern", "fern")]);
        assert!(settle_range("@oak @cedar", 0, 11, &[], &f.all).is_empty());
        assert_eq!(settle_range("@fern and @boak", 6, 15, &[], &f.all), vec![mention("agent:boak", "boak")]);
        let oaktree = f.all[4].clone();
        let fern = f.all[0].clone();
        assert_eq!(
            settle_range("@oak tree hi", 0, 12, &[], &[oaktree, fern]),
            vec![mention("agent:oaktree", "oak tree")]
        );
    }

    #[test]
    fn token_tracking_on_edit() {
        let ms = vec![mention("agent:oak", "oak"), mention("agent:fern", "fern")];
        assert_eq!(keep_mentions(&ms, "@oak and @fern"), ms);
        assert_eq!(keep_mentions(&ms, "@oak and "), vec![ms[0].clone()]);
        assert_eq!(keep_mentions(&ms, "@oa and @fern"), vec![ms[1].clone()]);
        assert!(keep_mentions(&ms, "").is_empty());
        assert!(!has_mention("@oakx", "oak", &[]));
        assert!(!has_mention("@Oak", "oak", &[]));
        assert!(has_mention("(@oak).", "oak", &[]));
        assert!(!has_mention("x@oak", "oak", &[]));
        assert!(has_mention("@oak tree", "oak tree", &[]));
        assert_eq!(add_mention(&ms, mention("agent:oak", "oak")), ms);
        let mut many = Vec::new();
        for i in 0..12 {
            many = add_mention(&many, mention(&format!("k{i}"), &format!("n{i}")));
        }
        assert_eq!(many.len(), MAX_MENTIONS);
    }

    #[test]
    fn exact_names() {
        let f = fx();
        assert_eq!(exact_unique("FERN", &f.all).map(|c| c.who.key.as_str()), Some("agent:fern"));
        assert!(exact_unique("oak", &f.all).is_none());
        assert!(exact_unique("cedar", &f.all).is_none());
        assert!(exact_unique("", &f.all).is_none());
        assert_eq!(settle("hi @Fern", 8, &[], &f.all), vec![mention("agent:fern", "Fern")]);
        assert!(settle("hi @fer", 7, &[], &f.all).is_empty());
        let ms = vec![mention("agent:fern", "fern")];
        assert_eq!(settle("@fern", 5, &ms, &f.all), ms);
    }

    #[derive(Debug, PartialEq)]
    struct Nm(&'static str, u8);
    impl Named for Nm {
        fn name(&self) -> &str {
            self.0
        }
    }

    #[test]
    fn split_mentions_cuts_longest_first() {
        let ms = [Nm("oak", 1), Nm("oak tree", 2)];
        assert_eq!(
            split_mentions("ask @oak tree and @oak.", &ms),
            vec![
                MentionSpan::Text("ask ".into()),
                MentionSpan::Mention("@oak tree".into(), &ms[1]),
                MentionSpan::Text(" and ".into()),
                MentionSpan::Mention("@oak".into(), &ms[0]),
                MentionSpan::Text(".".into()),
            ]
        );
        assert_eq!(
            split_mentions("hi @fern, a@oak", &ms),
            vec![MentionSpan::Text("hi @fern, a@oak".into())]
        );
        let none: [Nm; 0] = [];
        assert_eq!(split_mentions("plain", &none), vec![MentionSpan::Text("plain".into())]);
    }

    fn outcome(name: &str, state: &str) -> MentionOutcome {
        MentionOutcome {
            name: name.into(),
            state: state.into(),
            ..MentionOutcome::default()
        }
    }

    #[test]
    fn lines() {
        assert_eq!(
            not_delivered(&[outcome("oak", "woke"), outcome("fern", "gone"), outcome("cedr", "unknown")]),
            "Not delivered: fern is gone; no agent named cedr."
        );
        assert_eq!(not_delivered(&[outcome("oak", "woke")]), "");
        assert_eq!(not_delivered(&[]), "");
        assert_eq!(asking_line(&[mention("a", "oak")]), "Asking @oak only");
        assert_eq!(
            asking_line(&[mention("a", "oak"), mention("b", "fern"), mention("c", "elm")]),
            "Asking @oak, @fern and @elm only"
        );
    }

    #[test]
    fn oak_inside_a_recorded_oak_tree_doesnt_keep_oak() {
        let ms = vec![mention("agent:oak", "oak"), mention("agent:oaktree", "oak tree")];
        assert_eq!(keep_mentions(&ms, "@oak tree hi"), vec![ms[1].clone()]);
        assert_eq!(keep_mentions(&ms, "@oak tree and @oak"), ms);
    }

    #[test]
    fn card_mentions() {
        let r: Vec<(Option<String>, String)> = card_refs("see #12, (mlg#3) not a.#4 or #5a; #67")
            .into_iter()
            .map(|x| (x.tag, x.n))
            .collect();
        assert_eq!(
            r,
            vec![(None, "12".into()), (Some("mlg".into()), "3".into()), (None, "67".into())]
        );
        assert_eq!(card_token_at("hi #1", 5, &[]), t(3, "1"));
        assert_eq!(card_token_at("a#1", 3, &[]), None);
        assert_eq!(card_token_at("#subtask lay", 12, &sv(&["Subtask layout"])), t(0, "subtask lay"));
        assert_eq!(card_token_at("#sub lay", 8, &sv(&["Other"])), None);
        let cc = |id: &str, short: &str, title: &str| CardCandidate {
            id: id.into(),
            r#ref: format!("#{short}"),
            short: short.into(),
            title: title.into(),
        };
        let cs = vec![cc("a", "21", "Two"), cc("b", "3", "Has 2 in title"), cc("c", "2", "Second")];
        let ids = |v: Vec<CardCandidate>| v.into_iter().map(|c| c.id).collect::<Vec<_>>();
        assert_eq!(ids(filter_cards(&cs, "2")), sv(&["a", "c", "b"]));
        assert_eq!(ids(filter_cards(&cs, "sec")), sv(&["c"]));
        assert_eq!(insert_card_ref("see #su", &tok(4, "su"), 7, "#12"), ("see #12 ".into(), 8));
    }

    fn rr<'a>(id: &'a str, short: &'a str, title: &'a str, tag: &'a str) -> RefRow<'a> {
        RefRow { id, short, title, tag }
    }

    #[test]
    fn local_ref_resolves() {
        let cards = [
            rr("c12", "12", "Live card", ""),
            rr("m1", "1", "MLG card", "mlg"),
            rr("x7", "7", "One", ""),
            rr("y7", "7", "Two", "web"),
        ];
        let threads = [rr("t136", "136", "A thread", ""), rr("t12", "12", "Same number", "")];
        let r = |s: &str| card_refs(s).remove(0);
        let on_space = [rr("s9", "9", "x", "b-mlg")];
        assert_eq!(local_ref(&r("#9"), &cards, &on_space), None);
        assert_eq!(
            local_ref(&r("see #12"), &cards, &threads),
            Some(RefHit { thread: false, id: "c12".into(), title: "Live card".into() })
        );
        assert_eq!(
            local_ref(&r("mentioned this in #136"), &cards, &threads),
            Some(RefHit { thread: true, id: "t136".into(), title: "A thread".into() })
        );
        assert_eq!(local_ref(&r("mlg#136"), &cards, &threads), None);
        assert_eq!(local_ref(&r("mlg#1"), &cards, &threads).map(|h| h.id), Some("m1".into()));
        assert_eq!(local_ref(&r("#7"), &cards, &threads), None);
        assert_eq!(local_ref(&r("#99"), &cards, &threads), None);
        assert_eq!(local_ref(&r("#1"), &cards, &threads), None);
        assert_eq!(ref_text(&r("in #136")), "#136");
        assert_eq!(ref_text(&r("mlg#12")), "mlg#12");
    }

    #[test]
    fn board_rows_filter_by_board() {
        let row = |key: &str, board: &str| Candidate {
            who: AgentRef { key: key.into(), name: key.into(), ..AgentRef::default() },
            board: board.into(),
            ..Candidate::default()
        };
        let rows = vec![row("d/me", ""), row("person:blake", "b1"), row("agent:c1", "b1"), row("agent:c2", "b2")];
        assert_eq!(keys(&board_rows(&rows, "")), sv(&["d/me"]));
        assert_eq!(keys(&board_rows(&rows, "b1")), sv(&["d/me", "person:blake", "agent:c1"]));
    }

    fn held(id: &str, short: &str, who: &AgentRef, state: Holding, tag: &str) -> CardSummary {
        let mut c = CardSummary { id: id.into(), short: short.into(), tag: tag.into(), ..CardSummary::default() };
        c.holder.who = who.clone();
        c.holder.state = state;
        c
    }

    #[test]
    fn the_agent_on_n() {
        let fern = cand("dev/fern", "fern", "idle", "");
        let clover = cand("dev/clover", "clover", "idle", "");
        let you = AgentRef { key: "user:me".into(), name: "You".into(), status: "person".into(), ..AgentRef::default() };
        let gone = AgentRef { key: "dev/gone".into(), name: "sorrel".into(), status: "gone".into(), ..AgentRef::default() };
        let cards = vec![
            held("c57", "57", &clover.who, Holding::Active, ""),
            held("c61", "61", &fern.who, Holding::Active, ""),
            held("c62", "62", &you, Holding::Person, ""),
            held("c63", "63", &gone, Holding::Gone, ""),
            held("c70", "4", &fern.who, Holding::Active, "wind"),
        ];
        let base = vec![fern.clone(), clover.clone()];
        let rows = agent_on_rows(&cards, &base, "c61");
        assert_eq!(rows.iter().map(|r| r.card.clone()).collect::<Vec<_>>(), sv(&["#61", "#57", "wind#4"]));
        assert_eq!(
            rows.iter().map(row_name).collect::<Vec<_>>(),
            sv(&["the agent on #61", "the agent on #57", "the agent on wind#4"])
        );
        assert_eq!(rows[0].who.key, "dev/fern");

        let mut all = base.clone();
        all.extend(agent_on_rows(&cards, &base, "c61"));
        assert_eq!(row_name(&filter_candidates(&all, "")[0]), "the agent on #61");
        assert_eq!(filter_candidates(&all, "fer").iter().map(row_name).collect::<Vec<_>>(), sv(&["fern"]));

        let mut all = base.clone();
        all.extend(agent_on_rows(&cards, &base, ""));
        let names = |q: &str| filter_candidates(&all, q).iter().map(row_name).collect::<Vec<_>>();
        assert_eq!(names("#57"), sv(&["the agent on #57"]));
        assert_eq!(names("57"), sv(&["the agent on #57"]));
        assert_eq!(names("the agent on #6"), sv(&["the agent on #61"]));
        let rn: Vec<String> = all.iter().map(row_name).collect();
        assert_eq!(token_at("ask @the agent on #6", 20, &rn), t(4, "the agent on #6"));
    }
}
