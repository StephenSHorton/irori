//! Reading the person's message for Needs you (decision 69, plan
//! docs/design/task-proposals.md §5.2): "accept the first three, reject
//! the rest, tell Spencer it's too big" becomes Start, Start, Start and
//! GiveBack grants on the exact cards given to them.
//!
//! Every target resolves against the [`Snapshot`] Core took when the turn
//! started, never against tool output, the screen or the model's words, so
//! a `#12` written inside a task can't become one. Like the rest of
//! [`super::assistant_writes`], it is narrow on purpose: a message that
//! talks about Needs you but says more than this understands grants
//! nothing, and the assistant asks the person to say it plainly.
//!
//! Names in the snapshot are other members' own choice, so a name made of
//! words the reader knows (a filler, an opener, a hold-back word, a verb,
//! "the", "me") never matches, and a contraction ("let's", "that's") is
//! never a possessive.

use super::assistant_writes::{
    Grant, HOLD_BACK, ItemState, NOT_A_NAME, OPENERS, SnapItem, Snapshot, is_ref, norm_ref, opening,
};

/// What [`read`] made of a message.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Read {
    /// It isn't about Needs you: the other readers look at it.
    Not,
    /// It is about Needs you but isn't plain enough: no grant at all.
    Refused,
    Grants(Vec<Grant>),
}

/// One word of the message, where it sits in the text, and the stop that
/// ends it ('.', '!', '?', ',', ';', ':' or none).
#[derive(Debug, Clone)]
struct Tok {
    s: usize,
    e: usize,
    w: String,
    stop: Option<char>,
}

/// Apostrophes as the person's keyboard may type them.
fn apostrophes(s: &str) -> String {
    s.replace(['\u{2019}', '\u{2018}', '\u{02bc}', '\u{2032}', '`'], "'")
}

fn toks(text: &str) -> Vec<Tok> {
    let mut out = vec![];
    let mut at: Option<usize> = None;
    let push = |s: usize, e: usize, out: &mut Vec<Tok>| {
        let raw = &text[s..e];
        let core =
            raw.trim_end_matches(|c: char| ")]}\"'*_`\u{201c}\u{201d}\u{2018}\u{2019}".contains(c));
        // The run of stops at its end: a '?' anywhere in it makes it a
        // question ("start #12?!").
        let tail: String = core
            .chars()
            .rev()
            .take_while(|c| ".!?,;:".contains(*c))
            .collect();
        let stop = if tail.contains('?') {
            Some('?')
        } else if tail.chars().count() > 1 && tail.chars().all(|c| c == '.') {
            // "..." trails off; it ends nothing.
            None
        } else {
            tail.chars().next()
        };
        let w = apostrophes(&raw.to_lowercase())
            .trim_matches(|c: char| ",.!?;:()[]{}\"*_\u{201c}\u{201d}".contains(c))
            .to_string();
        if !w.is_empty() || stop.is_some() {
            out.push(Tok { s, e, w, stop });
        }
    };
    for (i, ch) in text.char_indices() {
        match (ch.is_whitespace(), at) {
            (false, None) => at = Some(i),
            (true, Some(s)) => {
                push(s, i, &mut out);
                at = None;
            }
            _ => {}
        }
    }
    if let Some(s) = at {
        push(s, text.len(), &mut out);
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    Start,
    NotNow,
    GiveBack,
    Give,
}

const NUMBERS: &[&str] = &[
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "twenty",
];

fn number(w: &str) -> Option<usize> {
    if !w.is_empty() && w.len() <= 3 && w.bytes().all(|b| b.is_ascii_digit()) {
        return w.parse().ok();
    }
    NUMBERS.iter().position(|n| *n == w)
}

/// Words that may stand anywhere in a Needs you clause and say nothing.
const FILLER: &[&str] = &[
    "the", "card", "cards", "task", "tasks", "one", "ones", "an", "a", "agent", "agents", "on",
    "for", "me", "please", "now", "too", "also", "then", "and", "both", "ok", "okay",
];

/// Words that pick cards out by place or by set.
const SET_WORDS: &[&str] = &[
    "all",
    "everything",
    "first",
    "last",
    "rest",
    "others",
    "other",
    "it",
    "this",
    "that",
    "them",
    "those",
    "these",
    "of",
    "back",
    "to",
    "tell",
    "over",
    "with",
    "ahead",
];

/// Words that never open a possessive, however a member names themselves.
const CONTRACTIONS: &[&str] = &[
    "let's",
    "that's",
    "it's",
    "what's",
    "here's",
    "there's",
    "he's",
    "she's",
    "who's",
    "where's",
    "how's",
    "when's",
    "why's",
    "one's",
    "everyone's",
    "someone's",
];

/// Words in a reason that take the request back or make it conditional:
/// the whole message grants nothing.
const REASON_HOLD: &[&str] = &[
    "wait",
    "actually",
    "unless",
    "until",
    "if",
    "maybe",
    "instead",
    "nevermind",
    "cancel",
];

/// The longest reason a give-back carries (as a message or comment does).
const MAX_REASON: usize = 4000;

/// A word that is a verb's own start, and how many words it takes.
fn verb_at(w: &[&str], i: usize) -> Option<(Verb, usize)> {
    let at = |k: usize, s: &str| w.get(k).is_some_and(|x| *x == s);
    Some(match w.get(i).copied()? {
        "not" if at(i + 1, "now") => (Verb::NotNow, 2),
        "later" | "park" => (Verb::NotNow, 1),
        "start" | "accept" | "take" => (Verb::Start, 1),
        "go" if at(i + 1, "ahead") && at(i + 2, "with") => (Verb::Start, 3),
        "reject" | "decline" => (Verb::GiveBack, 1),
        "pass" => (Verb::GiveBack, 1),
        "give" | "send" | "hand" => (Verb::Give, 1),
        _ => return None,
    })
}

/// Whether a word of a member's name is one the reader gives a meaning:
/// such a name never matches.
fn reserved(part: &str) -> bool {
    FILLER.contains(&part)
        || OPENERS.contains(&part)
        || HOLD_BACK.contains(&part)
        || NOT_A_NAME.contains(&part)
        || SET_WORDS.contains(&part)
        || REASON_HOLD.contains(&part)
        || CONTRACTIONS.contains(&part)
        || verb_at(&[part], 0).is_some()
        || ["not", "now", "because", "since", "later", "park"].contains(&part)
        || number(part).is_some()
        || is_ref(part)
        || !part.chars().any(char::is_alphabetic)
}

/// A member's name as words the reader can match, or `None` when it holds
/// a word the reader gives a meaning to.
fn name_parts(name: &str) -> Option<Vec<String>> {
    let parts: Vec<String> = apostrophes(&name.to_lowercase())
        .split_whitespace()
        .map(str::to_string)
        .collect();
    (!parts.is_empty() && !parts.iter().any(|p| reserved(p))).then_some(parts)
}

/// Whether the snapshot's people or givers include `name` as these words
/// start at `i`: the person's key and how many words the name took. A
/// first name alone counts when no one else has it; a full name beats
/// another's first name. Two that fit equally are `Err`: which one isn't
/// known.
fn name_at(
    w: &[&str],
    i: usize,
    people: &[(String, String)],
    possessive: bool,
) -> Option<Result<(String, usize), ()>> {
    if possessive && w.get(i).is_some_and(|x| CONTRACTIONS.contains(x)) {
        return None;
    }
    let mut hits: Vec<(String, usize)> = vec![];
    for (key, name) in people {
        let Some(parts) = name_parts(name) else {
            continue;
        };
        let fits = |n: usize| -> bool {
            (0..n).all(|k| {
                let Some(x) = w.get(i + k) else {
                    return false;
                };
                let want = &parts[k];
                if k + 1 == n && possessive {
                    *x == format!("{want}'s") || (want.ends_with('s') && *x == format!("{want}'"))
                } else {
                    x == want
                }
            })
        };
        if fits(parts.len()) {
            hits.push((key.clone(), parts.len()));
        } else if parts.len() > 1 && fits(1) {
            hits.push((key.clone(), 1));
        }
    }
    hits.sort();
    hits.dedup();
    match hits.as_slice() {
        [] => None,
        [one] => Some(Ok(one.clone())),
        _ => {
            let best = hits.iter().map(|h| h.1).max().unwrap_or(0);
            let top: Vec<_> = hits.iter().filter(|h| h.1 == best).collect();
            match top.as_slice() {
                [one] if best > 1 => Some(Ok((*one).clone())),
                _ => Some(Err(())),
            }
        }
    }
}

/// Which cards a set word ("all", "the first two", "the rest") counts
/// over.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Scope {
    /// Everything given to the person.
    All,
    /// What one giver gave ("of Spencer's").
    Giver(String),
    /// The cards named last ("of them", "of those").
    Named,
}

/// What a clause acts on, before it is resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Ref(String),
    All(Scope),
    First(usize, Scope),
    Last(usize, Scope),
    Rest(Scope),
    It,
    Them,
}

impl Target {
    /// A set word: it stands alone in its clause.
    fn is_set(&self) -> bool {
        !matches!(self, Target::Ref(_) | Target::It | Target::Them)
    }
}

#[derive(Debug, Default)]
struct Clause {
    verb: Option<Verb>,
    targets: Vec<Target>,
    /// The person a Give is to.
    to: Option<String>,
    back: bool,
    reason: Option<String>,
    /// "tell Spencer …": only ever a reason for Spencer's give-backs.
    tell: Option<String>,
}

struct Reader<'a> {
    text: &'a str,
    snap: &'a Snapshot,
    givers: Vec<(String, String)>,
    people: Vec<(String, String)>,
}

impl Reader<'_> {
    /// The verbatim words of `t[a..b]`, without the closing stop.
    fn said(&self, t: &[Tok], a: usize, b: usize) -> String {
        if a >= b {
            return String::new();
        }
        self.text[t[a].s..t[b - 1].e]
            .trim()
            .trim_end_matches(['.', ',', ';', '!', '?'])
            .trim()
            .to_string()
    }

    /// Whether a new clause starts at word `j` (after "and", a comma or
    /// a semicolon): a verb, "tell <giver>", or targets then a verb before
    /// the next stop ("#13 not now").
    fn starts_clause(&self, t: &[Tok], w: &[&str], mut j: usize, end: usize) -> bool {
        let (t, w) = (&t[..end], &w[..end]);
        while j < end && ["and", "then", "also", "please"].contains(&w[j]) {
            j += 1;
        }
        if j >= end {
            return false;
        }
        if verb_at(w, j).is_some() {
            return true;
        }
        if w[j] == "tell" && name_at(w, j + 1, &self.givers, false).is_some() {
            return true;
        }
        if !(is_ref(w[j]) || ["the", "all", "first", "last", "rest", "others"].contains(&w[j])) {
            return false;
        }
        let mut k = j;
        while k < end {
            if verb_at(w, k).is_some() {
                return true;
            }
            if t[k].stop.is_some_and(|c| c == ',' || c == ';') {
                return false;
            }
            k += 1;
        }
        false
    }

    /// Splits sentence `t[a..b]` into clauses. A "because", colon or dash
    /// reason runs to the end of the sentence, so nothing after it is a
    /// clause. A "tell <giver> …" reason ends only at a comma or semicolon
    /// before the next verb.
    fn clauses(&self, t: &[Tok], w: &[&str], a: usize, b: usize) -> Vec<(usize, usize)> {
        let (t, w) = (&t[..b], &w[..b]);
        let mut out = vec![];
        let mut from = a;
        let mut k = a;
        while k < b {
            let tell = w[from..]
                .iter()
                .find(|x| !OPENERS.contains(x))
                .is_some_and(|x| *x == "tell");
            if !tell && (is_reason_mark(w[k]) || t[k].stop == Some(':')) {
                break;
            }
            // After a comma or a semicolon.
            if t[k].stop.is_some_and(|c| c == ',' || c == ';') && self.starts_clause(t, w, k + 1, b)
            {
                out.push((from, k + 1));
                from = k + 1;
            } else if !tell && k > from && w[k] == "and" && self.starts_clause(t, w, k + 1, b) {
                out.push((from, k));
                from = k + 1;
            }
            k += 1;
        }
        if from < b {
            out.push((from, b));
        }
        out
    }

    /// The giver's possessive ("Spencer's").
    fn whose(&self, w: &[&str], j: usize) -> Option<Result<(String, usize), ()>> {
        name_at(w, j, &self.givers, true)
    }

    /// "of them", "of Spencer's" after a set word: what it counts over,
    /// and the words that took.
    fn of(&self, w: &[&str], j: usize) -> Result<(Scope, usize), ()> {
        if w.get(j) != Some(&"of") {
            return Ok((Scope::All, 0));
        }
        if matches!(w.get(j + 1), Some(&"them" | &"these" | &"those")) {
            return Ok((Scope::Named, 2));
        }
        match self.whose(w, j + 1) {
            Some(Ok((k, n))) => Ok((Scope::Giver(k), n + 1)),
            _ => Err(()),
        }
    }

    /// One clause, `t[a..b]`. `None`: words it doesn't understand.
    fn clause(&self, t: &[Tok], w: &[&str], a: usize, b: usize) -> Option<Clause> {
        // Nothing past the clause is looked at.
        let (t, w) = (&t[..b], &w[..b]);
        let mut c = Clause::default();
        let mut j = a;
        while j < b && OPENERS.contains(&w[j]) {
            j += 1;
        }
        // "tell Spencer it's too big": the rest is the reason.
        if w.get(j) == Some(&"tell") {
            let (key, n) = name_at(w, j + 1, &self.givers, false)?.ok()?;
            let mut r = j + 1 + n;
            if t.get(r - 1).is_some_and(|x| x.stop == Some(':')) {
                // "tell Spencer: it's too big"
            } else if w.get(r) == Some(&"that") {
                r += 1;
            }
            c.tell = Some(key);
            c.reason = Some(self.said(t, r, b));
            return Some(c);
        }
        while j < b {
            let x = w[j];
            // A reason runs to the end of the clause, as typed.
            if is_reason_mark(x) {
                c.reason = Some(self.said(t, j + 1, b));
                break;
            }
            if let Some((v, n)) = verb_at(w, j) {
                if c.verb.is_some() {
                    return None;
                }
                c.verb = Some(v);
                if v == Verb::GiveBack && x == "pass" {
                    // "pass on #12"
                    if w.get(j + 1) != Some(&"on") {
                        return None;
                    }
                    j += 1;
                }
                let colon = t[j + n - 1].stop == Some(':');
                j += n;
                if colon {
                    c.reason = Some(self.said(t, j, b));
                    break;
                }
                continue;
            }
            let start = j;
            let n = if is_ref(x) {
                c.targets.push(Target::Ref(norm_ref(x)));
                1
            } else if x == "back" {
                c.back = true;
                1
            } else if x == "to" && c.verb == Some(Verb::Give) && c.to.is_none() {
                let (k, n) = name_at(w, j + 1, &self.people, false)?.ok()?;
                c.to = Some(k);
                1 + n
            } else if x == "all" || x == "everything" {
                let (scope, n) = match self.of(w, j + 1).ok()? {
                    (Scope::All, 0) => match self.whose(w, j + 1) {
                        Some(Ok((k, n))) => (Scope::Giver(k), n),
                        Some(Err(())) => return None,
                        None => (Scope::All, 0),
                    },
                    v => v,
                };
                c.targets.push(Target::All(scope));
                1 + n
            } else if x == "first" || x == "last" {
                let (count, m) = match w.get(j + 1).and_then(|n| number(n)) {
                    Some(n) => (n, 1),
                    None => (1, 0),
                };
                if count == 0 {
                    return None;
                }
                let (scope, o) = self.of(w, j + 1 + m).ok()?;
                c.targets.push(if x == "first" {
                    Target::First(count, scope)
                } else {
                    Target::Last(count, scope)
                });
                1 + m + o
            } else if x == "rest"
                || x == "others"
                || (x == "other" && w.get(j + 1) == Some(&"ones"))
            {
                let m = usize::from(x == "other");
                let (scope, o) = self.of(w, j + 1 + m).ok()?;
                c.targets.push(Target::Rest(scope));
                1 + m + o
            } else if ["it", "this", "that"].contains(&x) {
                c.targets.push(Target::It);
                1
            } else if ["them", "those", "these"].contains(&x) {
                c.targets.push(Target::Them);
                1
            } else if FILLER.contains(&x) || x == "over" {
                1
            } else if let Some(r) = self.whose(w, j) {
                let (k, n) = r.ok()?;
                c.targets.push(Target::All(Scope::Giver(k)));
                n
            } else if c.verb == Some(Verb::Give) && c.to.is_none() {
                // "give Spencer #12"
                let (k, n) = name_at(w, j, &self.people, false)?.ok()?;
                c.to = Some(k);
                n
            } else {
                return None;
            };
            j += n;
            // A colon on the word just read: the rest is the reason.
            if t[start..j].iter().any(|x| x.stop == Some(':')) {
                if t[j - 1].stop != Some(':') {
                    return None;
                }
                c.reason = Some(self.said(t, j, b));
                break;
            }
        }
        if c.back {
            match c.verb {
                Some(Verb::Give) => c.verb = Some(Verb::GiveBack),
                Some(Verb::GiveBack) => {}
                _ => return None,
            }
        }
        if c.verb.is_none() || c.targets.is_empty() {
            return None;
        }
        // A set word stands alone: "Spencer's #12", "#13 first" and "them
        // all" say two things at once.
        if c.targets.iter().any(Target::is_set) && c.targets.len() > 1 {
            return None;
        }
        // Only a give-back carries the person's words.
        if c.reason.is_some() && c.verb != Some(Verb::GiveBack) {
            return None;
        }
        Some(c)
    }

    fn item_refs(&self, r: &str) -> Vec<usize> {
        self.snap
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.refs.iter().any(|x| x == r))
            .map(|(i, _)| i)
            .collect()
    }

    fn of_giver(&self, g: &str) -> Vec<usize> {
        self.snap
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.giver_key == g)
            .map(|(i, _)| i)
            .collect()
    }

    /// Whether `reason` can go to a giver as the person's words: it says
    /// something, names no card given to them, takes nothing back, and
    /// isn't too long.
    fn reason_ok(&self, reason: &str) -> bool {
        let ws = toks(reason);
        reason.chars().count() <= MAX_REASON
            && ws.iter().any(|x| {
                x.w.chars().any(char::is_alphabetic)
                    && !is_ref(&x.w)
                    && !["and", "or", "to"].contains(&x.w.as_str())
            })
            && !ws.iter().any(|x| {
                REASON_HOLD.contains(&x.w.as_str())
                    || (is_ref(&x.w) && !self.item_refs(&norm_ref(&x.w)).is_empty())
            })
    }
}

/// The refs `text` names, as they resolve in the snapshot (each exactly
/// one item), or `None` when one doesn't.
fn earlier_items(r: &Reader, text: &str) -> Option<Vec<usize>> {
    let mut out = vec![];
    for t in toks(text) {
        if is_ref(&t.w) {
            match r.item_refs(&norm_ref(&t.w)).as_slice() {
                [i] => {
                    if !out.contains(i) {
                        out.push(*i);
                    }
                }
                _ => return None,
            }
        }
    }
    Some(out)
}

/// Whether the message is about Needs you at all: a Needs you verb and
/// something in the snapshot it could act on.
fn about_needs(r: &Reader, w: &[&str]) -> bool {
    let verb = (0..w.len()).any(|i| verb_at(w, i).is_some());
    let given = !r.snap.items.is_empty();
    let target = w.iter().enumerate().any(|(i, x)| {
        (is_ref(x) && !r.item_refs(&norm_ref(x)).is_empty())
            || (given
                && [
                    "all",
                    "everything",
                    "first",
                    "last",
                    "rest",
                    "others",
                    "it",
                    "them",
                    "those",
                    "these",
                ]
                .contains(x))
            || r.whose(w, i).is_some()
    });
    let give = w.iter().any(|x| ["give", "send", "hand"].contains(x))
        && w.iter()
            .any(|x| is_ref(x) || ["it", "this", "that"].contains(x))
        && (0..w.len()).any(|i| name_at(w, i, &r.people, false).is_some());
    verb && (target || give)
}

/// Reads `text` for Needs you. `earlier` is what the person typed before,
/// oldest first ("start it" means the one they last named).
pub(crate) fn read(text: &str, earlier: &[&str], snap: &Snapshot) -> Read {
    let mut givers: Vec<(String, String)> = vec![];
    for it in &snap.items {
        let g = (it.giver_key.clone(), it.giver_name.clone());
        if !givers.contains(&g) {
            givers.push(g);
        }
    }
    let r = Reader {
        text,
        snap,
        givers,
        people: snap
            .people
            .iter()
            .map(|p| (p.person_key.clone(), p.name.clone()))
            .collect(),
    };
    let t = toks(text);
    let w: Vec<&str> = t.iter().map(|x| x.w.as_str()).collect();
    if !about_needs(&r, &w) {
        return Read::Not;
    }
    match parse(&r, &t, &w, earlier) {
        Some(g) if !g.is_empty() => Read::Grants(g),
        _ => Read::Refused,
    }
}

/// The cards the person's last message that named any card named; all of
/// them must be given to them.
fn earlier_named(r: &Reader, earlier: &[&str]) -> Option<Vec<usize>> {
    let e = earlier
        .iter()
        .rev()
        .find(|e| toks(e).iter().any(|x| is_ref(&x.w)))?;
    earlier_items(r, e)
}

fn parse(r: &Reader, t: &[Tok], w: &[&str], earlier: &[&str]) -> Option<Vec<Grant>> {
    // Sentences: split at '.', '!' and '?'.
    let mut sentences: Vec<(usize, usize, bool)> = vec![];
    let mut from = 0;
    for (k, x) in t.iter().enumerate() {
        if let Some(c @ ('.' | '!' | '?')) = x.stop {
            sentences.push((from, k + 1, c == '?'));
            from = k + 1;
        }
    }
    if from < t.len() {
        sentences.push((from, t.len(), false));
    }
    // Each item's verb, in the order they were named.
    let mut said: Vec<(usize, Verb, Option<String>)> = vec![];
    let mut gives: Vec<(String, Vec<String>)> = vec![];
    let mut tells: Vec<(String, String)> = vec![];
    let mut last: Vec<usize> = vec![];
    // The one card the last Give named, for "give it to Spencer".
    let mut last_card: Option<String> = None;
    // Where the person's own reasons are: hold-back words there are theirs
    // to say.
    let mut reasons: Vec<(usize, usize)> = vec![];
    for (a, b, question) in sentences {
        let (i, asked) = opening(&w[a..b].iter().map(|s| s.to_string()).collect::<Vec<_>>());
        // A question in a message about Needs you leaves it unsure:
        // "start all of Spencer's. Or just the first two?"
        if question && !asked {
            return None;
        }
        // "go ahead with #12": the opening took "go ahead".
        let mut a2 = a + i;
        if w.get(a2) == Some(&"with") && a2 >= 2 && w[a2 - 2] == "go" && w[a2 - 1] == "ahead" {
            a2 -= 2;
        }
        for (ca, cb) in r.clauses(t, w, a2, b) {
            let c = r.clause(t, w, ca, cb)?;
            reasons.push((reason_start(t, w, ca, cb, &c), cb));
            if let Some(why) = &c.reason
                && !r.reason_ok(why)
            {
                return None;
            }
            if let Some(g) = c.tell {
                if tells.iter().any(|(k, _)| *k == g) {
                    return None;
                }
                tells.push((g, c.reason.unwrap_or_default()));
                continue;
            }
            let verb = c.verb?;
            if verb == Verb::Give {
                let to = c.to.clone()?;
                let mut cards = vec![];
                for tg in &c.targets {
                    match tg {
                        Target::Ref(x) => cards.push(x.clone()),
                        // "it" is the one card this message last gave, or,
                        // when it named nothing yet, the earlier one.
                        Target::It => cards.push(match &last_card {
                            Some(x) => x.clone(),
                            None if last.is_empty() && gives.is_empty() => {
                                earlier_one_ref(earlier)?
                            }
                            None => return None,
                        }),
                        _ => return None,
                    }
                }
                // A card given to them goes back or starts; it is never
                // passed on, and no card goes to two people.
                for x in &cards {
                    if !r.item_refs(x).is_empty()
                        || gives.iter().any(|(k, v)| *k != to && v.contains(x))
                    {
                        return None;
                    }
                }
                last_card = cards.last().cloned().filter(|_| cards.len() == 1);
                match gives.iter_mut().find(|(k, _)| *k == to) {
                    Some((_, v)) => {
                        for x in cards {
                            if !v.contains(&x) {
                                v.push(x);
                            }
                        }
                    }
                    None => gives.push((to, cards)),
                }
                continue;
            }
            if c.to.is_some() {
                return None;
            }
            let named: Vec<usize> = said.iter().map(|x| x.0).collect();
            // What "them", "it" and "of those" point at.
            let pointed = || -> Option<Vec<usize>> {
                if last.is_empty() {
                    earlier_named(r, earlier)
                } else {
                    Some(last.clone())
                }
            };
            let scoped = |s: &Scope| -> Option<Vec<usize>> {
                match s {
                    Scope::All => Some((0..r.snap.items.len()).collect()),
                    Scope::Giver(g) => Some(r.of_giver(g)),
                    Scope::Named => pointed(),
                }
            };
            let mut now: Vec<usize> = vec![];
            for tg in &c.targets {
                let got: Vec<usize> = match tg {
                    Target::Ref(x) => match r.item_refs(x).as_slice() {
                        [i] => vec![*i],
                        _ => return None,
                    },
                    Target::All(s) => scoped(s)?,
                    Target::First(n, s) => {
                        let v = scoped(s)?;
                        if *n > v.len() {
                            return None;
                        }
                        v[..*n].to_vec()
                    }
                    Target::Last(n, s) => {
                        let v = scoped(s)?;
                        if *n > v.len() {
                            return None;
                        }
                        v[v.len() - n..].to_vec()
                    }
                    Target::Rest(s) => {
                        if named.is_empty() {
                            return None;
                        }
                        scoped(s)?
                            .into_iter()
                            .filter(|i| !named.contains(i))
                            .collect()
                    }
                    Target::It | Target::Them => {
                        let v = pointed()?;
                        if *tg == Target::It && v.len() != 1 {
                            return None;
                        }
                        v
                    }
                };
                if got.is_empty() {
                    return None;
                }
                for i in got {
                    if !now.contains(&i) {
                        now.push(i);
                    }
                }
            }
            for &i in &now {
                match said.iter().find(|x| x.0 == i) {
                    Some(x) if x.1 != verb => return None,
                    Some(_) => {}
                    None => said.push((i, verb, c.reason.clone())),
                }
            }
            last = now;
        }
    }
    // A hold-back word anywhere but a reason takes the whole message
    // ("Start #12. But only if it passes.").
    for (k, x) in w.iter().enumerate() {
        let not_now = *x == "not" && w.get(k + 1) == Some(&"now");
        if HOLD_BACK.contains(x) && !not_now && !reasons.iter().any(|(a, b)| (*a..*b).contains(&k))
        {
            return None;
        }
    }
    // "tell Spencer …" is the reason for Spencer's give-backs, and only
    // ever that.
    for (g, why) in &tells {
        let mut any = false;
        for x in said.iter_mut() {
            let item: &SnapItem = &r.snap.items[x.0];
            if x.1 == Verb::GiveBack && item.giver_key == *g {
                if x.2.is_some() {
                    return None;
                }
                x.2 = Some(why.clone());
                any = true;
            }
        }
        if !any {
            return None;
        }
    }
    let mut out = vec![];
    for (i, verb, why) in said {
        let it = &r.snap.items[i];
        let (card, refs) = (it.id.clone(), it.refs.clone());
        out.push(match verb {
            Verb::Start => Grant::Start { card, refs },
            Verb::NotNow => Grant::NotNow { card, refs },
            Verb::GiveBack => {
                // An agent is on it or starting: stop it first.
                if it.state == ItemState::Starting {
                    return None;
                }
                Grant::GiveBack {
                    card,
                    refs,
                    reason: why.unwrap_or_default(),
                }
            }
            Verb::Give => return None,
        });
    }
    for (to, cards) in gives {
        if cards.is_empty() {
            return None;
        }
        let name = r
            .people
            .iter()
            .find(|(k, _)| *k == to)
            .map(|(_, n)| n.clone())
            .unwrap_or_default();
        out.push(Grant::Give { to, name, cards });
    }
    Some(out)
}

/// Where clause `c`'s reason starts in `t[a..b]` (`b` when it has none).
fn reason_start(t: &[Tok], w: &[&str], a: usize, b: usize, c: &Clause) -> usize {
    if c.reason.is_none() {
        return b;
    }
    for k in a..b {
        if c.tell.is_some() && w[k] == "tell" {
            return k;
        }
        if is_reason_mark(w[k]) {
            return k + 1;
        }
        if t[k].stop == Some(':') {
            return k + 1;
        }
    }
    b
}

/// A word after which the rest of the clause is the person's reason.
/// "since" isn't one: "reject everything since Monday" is about when.
fn is_reason_mark(w: &str) -> bool {
    ["because", "-", "--", "\u{2014}", "\u{2013}"].contains(&w)
}

/// The one card the person's earlier messages named last.
fn earlier_one_ref(earlier: &[&str]) -> Option<String> {
    for e in earlier.iter().rev() {
        let mut refs: Vec<String> = vec![];
        for t in toks(e) {
            if is_ref(&t.w) {
                let r = norm_ref(&t.w);
                if !refs.contains(&r) {
                    refs.push(r);
                }
            }
        }
        match refs.len() {
            0 => continue,
            1 => return refs.pop(),
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::assistant_writes::{SnapPerson, grants_for};
    use super::*;

    const SPENCER: &str = "person:pSpencer";
    const BLAKE: &str = "person:pBlake";

    fn item(id: &str, short: &str, giver: (&str, &str), state: ItemState) -> (SnapItem, String) {
        (
            SnapItem {
                id: id.into(),
                giver_key: giver.0.into(),
                giver_name: giver.1.into(),
                batch: "b1".into(),
                state,
                ..SnapItem::default()
            },
            short.into(),
        )
    }

    fn people() -> Vec<SnapPerson> {
        [
            (SPENCER, "Spencer"),
            (BLAKE, "Blake Moss"),
            ("person:pAnn", "Ann"),
        ]
        .iter()
        .map(|(k, n)| SnapPerson {
            person_key: (*k).into(),
            name: (*n).into(),
        })
        .collect()
    }

    /// Spencer gave ops#12 to ops#15; Blake gave mlg#7.
    fn snap() -> Snapshot {
        let s = (SPENCER, "Spencer");
        Snapshot::new(
            vec![
                item("t12", "ops#12", s, ItemState::Asked),
                item("t13", "ops#13", s, ItemState::Asked),
                item("t14", "ops#14", s, ItemState::NotNow),
                item("t15", "ops#15", s, ItemState::Asked),
                item("t7", "mlg#7", (BLAKE, "Blake Moss"), ItemState::Asked),
            ],
            people(),
        )
    }

    /// Only Spencer's four.
    fn spencers() -> Snapshot {
        let mut s = snap();
        s.items.retain(|it| it.giver_key == SPENCER);
        s
    }

    fn refs(id: &str) -> Vec<String> {
        let n = id.trim_start_matches('t');
        let tag = if n == "7" { "mlg" } else { "ops" };
        vec![format!("{tag}#{n}"), format!("#{n}")]
    }
    fn start(id: &str) -> Grant {
        Grant::Start {
            card: id.into(),
            refs: refs(id),
        }
    }
    fn not_now(id: &str) -> Grant {
        Grant::NotNow {
            card: id.into(),
            refs: refs(id),
        }
    }
    fn back(id: &str, why: &str) -> Grant {
        Grant::GiveBack {
            card: id.into(),
            refs: refs(id),
            reason: why.into(),
        }
    }

    fn g(text: &str, s: &Snapshot) -> Vec<Grant> {
        grants_for(text, &[], s)
    }

    #[test]
    fn the_snapshot_gives_a_bare_number_only_when_it_is_unique() {
        let s = snap();
        assert_eq!(s.items[0].refs, vec!["ops#12", "#12"]);
        let two = Snapshot::new(
            vec![
                item("a", "ops#12", (SPENCER, "Spencer"), ItemState::Asked),
                item("b", "mlg#12", (BLAKE, "Blake"), ItemState::Asked),
            ],
            vec![],
        );
        assert_eq!(two.items[0].refs, vec!["ops#12"]);
        assert_eq!(two.items[1].refs, vec!["mlg#12"]);
        // A bare #12 there names two items: nothing.
        assert_eq!(g("start #12", &two), vec![]);
    }

    /// The examples table in task-proposals.md §5.2.
    #[test]
    fn the_plans_examples_grant_what_they_say() {
        let s = spencers();
        assert_eq!(
            g(
                "accept the first three, reject the rest, tell Spencer it's too big",
                &s
            ),
            vec![
                start("t12"),
                start("t13"),
                start("t14"),
                back("t15", "it's too big")
            ]
        );
        assert_eq!(
            g("tell Spencer it's too big, give back #15", &s),
            vec![back("t15", "it's too big")]
        );
        assert_eq!(g("not now #12", &s), vec![not_now("t12")]);
        assert_eq!(g("#12 not now", &s), vec![not_now("t12")]);
        assert_eq!(
            g("give back #15 because it's not mine", &s),
            vec![back("t15", "it's not mine")]
        );
        assert_eq!(g("start #12, but not #13", &s), vec![]);
        assert_eq!(
            g("start all of Spencer's", &snap()),
            vec![start("t12"), start("t13"), start("t14"), start("t15")]
        );
    }

    #[test]
    fn plain_needs_asks_grant_what_they_name() {
        let s = snap();
        let all_spencers = vec![start("t12"), start("t13"), start("t14"), start("t15")];
        for (text, want) in [
            ("start ops#13", vec![start("t13")]),
            (
                "Please accept #12 and #13.",
                vec![start("t12"), start("t13")],
            ),
            ("can you start ops#13?", vec![start("t13")]),
            ("go ahead with #12", vec![start("t12")]),
            ("ok, start an agent on #12", vec![start("t12")]),
            ("take #7", vec![start("t7")]),
            ("park #12", vec![not_now("t12")]),
            ("#12 later", vec![not_now("t12")]),
            (
                "start #12 and #13 later",
                vec![start("t12"), not_now("t13")],
            ),
            ("decline #13 - too vague", vec![back("t13", "too vague")]),
            (
                "reject #15: duplicate of #9",
                vec![back("t15", "duplicate of #9")],
            ),
            ("give #15 back", vec![back("t15", "")]),
            ("send back #15", vec![back("t15", "")]),
            ("pass on #15", vec![back("t15", "")]),
            ("start Spencer's", all_spencers.clone()),
            ("start all of Blake Moss's", vec![start("t7")]),
            (
                "accept the last two of Spencer's",
                vec![start("t14"), start("t15")],
            ),
            (
                "start #12, give the others back",
                vec![
                    start("t12"),
                    back("t13", ""),
                    back("t14", ""),
                    back("t15", ""),
                    back("t7", ""),
                ],
            ),
            (
                "give back #15 because it's big, and old",
                vec![back("t15", "it's big, and old")],
            ),
        ] {
            assert_eq!(g(text, &s), want, "{text}");
        }
    }

    #[test]
    fn anything_unclear_about_needs_grants_nothing() {
        let s = snap();
        for text in [
            "don't start #12",
            "maybe start #12",
            "start #12 when the tests pass",
            "start #12 if it's small",
            "should I start #12?",
            "start #12. Or not?",
            "Start #12. But only if it passes.",
            "start #12\nand #13",
            "start #99",
            "start #12, tell Spencer thanks",
            "start #12 and close #5",
            "close #5. start #12",
            "take a look at #12",
            "start the first six",
            "reject the rest",
            "start it",
            "accept them all quickly",
            "start #12 and give #12 back",
            "start Bob's",
            "tell Spencer hi, start #12",
        ] {
            assert_eq!(g(text, &s), vec![], "{text}");
        }
    }

    /// Each of these would grant more than the plain reading allows.
    #[test]
    fn edge_cases_never_grant_more_than_was_said() {
        let s = snap();
        // "it" is the card last named, even when that isn't given to them.
        assert_eq!(
            grants_for("start it", &["what's ops#12?", "look at #5"], &s),
            vec![]
        );
        assert_eq!(
            grants_for("start it", &["what's ops#12?"], &s),
            vec![start("t12")]
        );
        assert_eq!(
            grants_for("start them", &["what are #12 and #13?"], &s),
            vec![start("t12"), start("t13")]
        );
        // Words to an agent stay the agent's message.
        assert_eq!(
            g("tell ringo to start the first test", &s),
            vec![Grant::Send {
                to: "ringo".into(),
                text: "start the first test".into()
            }]
        );
        // A reason runs to the end of its sentence: nothing in it acts.
        assert_eq!(
            g("give back #15 because I can't do this and start #14", &s),
            vec![],
            "a reason that names a card given to them is unclear"
        );
        assert_eq!(
            g(
                "give back #15 because I can't do this and start it later",
                &s
            ),
            vec![back("t15", "I can't do this and start it later")]
        );
        // A give-back on a card whose agent is starting: stop it first.
        let mut busy = spencers();
        busy.items[3].state = ItemState::Starting;
        assert_eq!(g("give back #15", &busy), vec![]);
        assert_eq!(g("start #15", &busy), vec![start("t15")]);
    }

    #[test]
    fn give_names_a_person_and_the_persons_own_cards() {
        let s = snap();
        let give = |to: &str, name: &str, cards: &[&str]| Grant::Give {
            to: to.into(),
            name: name.into(),
            cards: cards.iter().map(|c| c.to_string()).collect(),
        };
        assert_eq!(
            g("give #3 and #4 to Blake", &s),
            vec![give(BLAKE, "Blake Moss", &["#3", "#4"])]
        );
        assert_eq!(
            g("send Spencer #3", &s),
            vec![give(SPENCER, "Spencer", &["#3"])]
        );
        assert_eq!(
            g("hand ops#40 to Ann", &s),
            vec![give("person:pAnn", "Ann", &["ops#40"])]
        );
        assert_eq!(
            grants_for("give it to Ann", &["what's #3?"], &s),
            vec![give("person:pAnn", "Ann", &["#3"])]
        );
        for text in [
            "give #3 to Bob",
            "give the first two to Ann",
            "give #3 to Ann because she asked",
            "don't give #3 to Ann",
        ] {
            assert_eq!(g(text, &s), vec![], "{text}");
        }
        // Two people go by Ann: which one isn't known.
        let mut two = snap();
        two.people.push(SnapPerson {
            person_key: "person:pAnn2".into(),
            name: "Ann".into(),
        });
        assert_eq!(g("give #3 to Ann", &two), vec![]);
    }

    /// The older readers keep their answers with things given to the
    /// person.
    #[test]
    fn close_tick_send_and_comment_read_as_before() {
        let s = snap();
        for text in [
            "close #5",
            "tick s2 on #5",
            "tell ringo the build is green",
            "comment on #1182: start all of it",
            "close #5 and #6",
        ] {
            assert_eq!(
                grants_for(text, &[], &s),
                grants_for(text, &[], &Snapshot::default()),
                "{text}"
            );
            assert!(!grants_for(text, &[], &s).is_empty(), "{text}");
        }
    }

    use super::super::assistant_writes::{
        Grants, TOOL_CLOSE, TOOL_GIVE_BACK, TOOL_NOT_NOW, TOOL_START,
    };

    /// Refs resolve as Core would: tagged refs and ids; a bare #3 is a
    /// Personal card.
    fn resolve(r: &str) -> Option<String> {
        let r = r.strip_prefix("task:").unwrap_or(r);
        match r {
            "ops#12" | "t12" => Some("t12".into()),
            "ops#15" | "t15" => Some("t15".into()),
            "#3" | "p3" => Some("p3".into()),
            "#4" | "p4" => Some("p4".into()),
            _ => None,
        }
    }

    #[test]
    fn a_needs_grant_is_spent_once_on_its_own_card() {
        let gs = Grants::default();
        let run = gs.begin(g("start #12, not now #13", &spencers()));
        // Not by another tool, nor on another card.
        assert_eq!(gs.take(TOOL_CLOSE, "ops#12", "", &resolve), None);
        assert_eq!(gs.take(TOOL_NOT_NOW, "ops#12", "", &resolve), None);
        assert_eq!(gs.take(TOOL_START, "ops#15", "", &resolve), None);
        assert_eq!(gs.take(TOOL_START, "", "", &resolve), None);
        // By the snapshot's ref, the id or a ref that resolves to it.
        assert_eq!(
            gs.take(TOOL_START, "#12", "", &resolve),
            Some("task:t12".into())
        );
        assert_eq!(gs.take(TOOL_START, "task:t12", "", &resolve), None, "spent");
        assert_eq!(
            gs.take(TOOL_NOT_NOW, "ops#13", "", &resolve),
            Some("task:t13".into())
        );
        assert_eq!(gs.open_count(), 0);
        gs.end(run);

        let run = gs.begin(g("give back #15 because it's not mine", &spencers()));
        assert_eq!(gs.take(TOOL_START, "#15", "", &resolve), None);
        assert_eq!(gs.take_text(TOOL_GIVE_BACK, "#12", &resolve), None);
        assert_eq!(
            gs.take_text(TOOL_GIVE_BACK, "task:t15", &resolve),
            Some(("task:t15".into(), "it's not mine".into()))
        );
        assert_eq!(gs.take_text(TOOL_GIVE_BACK, "#15", &resolve), None);
        gs.end(run);
    }

    #[test]
    fn a_give_is_spent_only_for_exactly_its_cards_and_person() {
        let gs = Grants::default();
        let run = gs.begin(g("give #3 and #4 to Blake", &snap()));
        let ids = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(gs.take_give("Ann", &ids(&["#3", "#4"]), &resolve), None);
        assert_eq!(gs.take_give(BLAKE, &ids(&["#3"]), &resolve), None);
        assert_eq!(
            gs.take_give(BLAKE, &ids(&["#3", "#4", "ops#12"]), &resolve),
            None
        );
        assert_eq!(gs.take_give(BLAKE, &ids(&["#3", "#3"]), &resolve), None);
        assert_eq!(gs.take_give(BLAKE, &[], &resolve), None);
        assert_eq!(
            gs.take_give("blake moss", &ids(&["p4", "#3"]), &resolve),
            Some((BLAKE.into(), ids(&["task:p4", "task:p3"])))
        );
        assert_eq!(gs.take_give(BLAKE, &ids(&["#3", "#4"]), &resolve), None);
        gs.end(run);
    }

    /// The adversarial pass on #1226: each of these once granted more than
    /// the person plainly said. Each now grants nothing (or only what was
    /// said).
    #[test]
    fn nothing_grants_more_than_the_person_plainly_said() {
        let s = snap();
        let sp = (SPENCER, "Spencer");
        for text in [
            // A set word mixed with refs, or two set words.
            "give back Spencer's #15 because it's not mine",
            "start Spencer's #12",
            "start Spencer's first two",
            "start #13 first",
            "start #13 last",
            "first start #13 and reject #15",
            "start all of these: #12 and #13",
            // A card given to them is never passed on, nor to two people.
            "start ops#13, give ops#12 to Ann, reject the rest",
            "start #12 and give #12 to Blake",
            "give #12 back and give #12 to Blake",
            "give #3 to Ann, give #3 to Blake",
            "give #3 and #4 to Ann, give #4 to Blake",
            // Reasons that take it back, make it conditional, or name cards.
            "give back #15 - actually no, wait, keep it",
            "reject #15, tell Spencer it's too big, but wait until Friday",
            "reject #15 because it's too big, unless Spencer splits it",
            "reject #13 - #15",
            "reject everything since Monday",
            "reject #15, tell Spencer to start #12 himself and #13 later",
            // Questions and their spellings.
            "start all of Spencer's. Or just the first two?",
            "Start #12. Or should I hold off?",
            "Start #12. Or is it not now?",
            "Start #12. Or don\u{2018}t?",
            "start #12?!",
            // A tell to a giver is a reason, never a Send.
            "tell Spencer thanks",
        ] {
            assert_eq!(g(text, &s), vec![], "{text}");
        }
        for sep in ["\r", "\u{2028}", "\u{2029}", "\u{0085}", "\u{000b}"] {
            assert_eq!(g(&format!("start #12{sep}and #13"), &s), vec![], "{sep:?}");
            assert_eq!(g(&format!("close #5{sep}and #6"), &s), vec![], "{sep:?}");
        }
        let long = format!("give back #15 because {}", "x".repeat(MAX_REASON + 1));
        assert_eq!(g(&long, &s), vec![]);
        // "it" in a give is never a card from before when this message
        // named one.
        for (text, before) in [
            ("accept #12 and hand it to Ann", "what's #3?"),
            ("start #12, then give it to Ann", "what's #3?"),
            ("start #12 and give it to Ann", "what's #3?"),
            ("give #3 and #4 to Blake, give it to Ann", "what's #5?"),
            ("reject them: #13 and #15", "what's #12?"),
        ] {
            assert_eq!(grants_for(text, &[before], &s), vec![], "{text}");
        }
        // "of them" and "them all" count over the cards named last.
        let named = ["what are #12 and #13?"];
        assert_eq!(
            grants_for("start all of them", &named, &s),
            vec![start("t12"), start("t13")]
        );
        assert_eq!(grants_for("start them all", &named, &s), vec![]);
        assert_eq!(
            grants_for(
                "accept the first two of those",
                &["what are ops#13, ops#14 and ops#15?"],
                &s
            ),
            vec![start("t13"), start("t14")]
        );

        // Names other members chose never make a target or a recipient.
        let with = |extra: Vec<(SnapItem, String)>, people: Vec<(&str, &str)>| {
            let mut items = vec![
                item("t12", "ops#12", sp, ItemState::Asked),
                item("t13", "ops#13", sp, ItemState::Asked),
            ];
            items.extend(extra);
            let mut ps = super::tests::people();
            for (k, n) in people {
                ps.push(SnapPerson {
                    person_key: k.into(),
                    name: n.into(),
                });
            }
            Snapshot::new(items, ps)
        };
        let lets = with(
            vec![item(
                "x1",
                "dev#1",
                ("person:pLet", "Let"),
                ItemState::Asked,
            )],
            vec![],
        );
        assert_eq!(g("let's start ops#13", &lets), vec![]);
        let that = with(
            vec![item(
                "x1",
                "dev#1",
                ("person:pThat", "That Guy"),
                ItemState::Asked,
            )],
            vec![],
        );
        assert_eq!(g("start #12 and #13, that's all", &that), vec![]);
        let ringo = with(
            vec![item(
                "x1",
                "dev#1",
                ("person:pRingo", "Ringo"),
                ItemState::Asked,
            )],
            vec![],
        );
        assert_eq!(
            g("tell ringo I'm done, give back everything", &ringo),
            vec![]
        );
        let spencer_that = Snapshot::new(
            vec![
                item("t12", "ops#12", sp, ItemState::Asked),
                item("t13", "ops#13", sp, ItemState::Asked),
                item("t14", "ops#14", sp, ItemState::Asked),
                item("t15", "ops#15", sp, ItemState::Asked),
                item(
                    "x1",
                    "dev#1",
                    ("person:pM", "Spencer That"),
                    ItemState::Asked,
                ),
            ],
            people(),
        );
        let got = g(
            "accept the first three, reject the rest, tell Spencer that it's too big",
            &spencer_that,
        );
        assert!(
            !got.iter().any(|x| matches!(x, Grant::GiveBack { card, reason, .. } if card == "x1" && !reason.is_empty())),
            "{got:?}"
        );
        let names = with(
            vec![],
            vec![
                ("person:pAP", "Ann Please"),
                ("person:pTeam", "The Team"),
                ("person:pMe", "Me"),
                ("person:pSome", "Someone"),
            ],
        );
        let give = |to: &str, name: &str, cards: &[&str]| Grant::Give {
            to: to.into(),
            name: name.into(),
            cards: cards.iter().map(|c| c.to_string()).collect(),
        };
        assert_eq!(
            g("give #3 and #4 to Ann please", &names),
            vec![give("person:pAnn", "Ann", &["#3", "#4"])]
        );
        for text in [
            "give #3 and #4 to the team",
            "give #3 to me",
            "give #3 to someone",
        ] {
            assert_eq!(g(text, &names), vec![], "{text}");
        }
    }
}
