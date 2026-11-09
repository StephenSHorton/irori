//! Reading the person's message for Needs you (decision 69, plan
//! docs/design/task-proposals.md §5.2): "accept #12 and #13, reject the
//! rest, tell Spencer it's too big" becomes Start, Start and GiveBack
//! grants on the exact cards given to them.
//!
//! Every target resolves against the [`Snapshot`] Core took when the turn
//! started, never against tool output, the screen or the model's words, so
//! a `#12` written inside a task can't become one. Like the rest of
//! [`super::assistant_writes`], it is narrow on purpose: a message that
//! talks about Needs you but says more than this understands grants
//! nothing, and the assistant asks the person to say it plainly.
//!
//! What it reads, and only that:
//! - verbs: start, accept, take, go ahead with; not now, later, park;
//!   give back, send back, reject, decline, pass on; give, send, hand
//!   (with a person);
//! - targets: refs; "all", "all of Spencer's", "Spencer's"; "the rest",
//!   "the others"; "it", "them", "those" for what was named just before;
//! - a reason, only after "because" (never with a set word) or in a
//!   "tell <giver> …" clause, and only as one plain clause: no ref, no
//!   member's name, no verb, no punctuation but '.' or '-' inside a word.
//!
//! "The first three" is not read: the window lists given cards space by
//! space in its own order, so a position could name a card the person
//! didn't see there.
//!
//! Names in the snapshot are other members' own choice, so a name made of
//! words the reader knows never matches, a contraction is never a
//! possessive, a name never runs across a comma, and a longer name never
//! beats another member's whole name.

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

/// Words a period follows without ending the sentence.
const ABBREVIATIONS: &[&str] = &[
    "i.e", "e.g", "etc", "vs", "approx", "mr", "mrs", "ms", "dr", "cf", "incl",
];

/// Apostrophes as the person's keyboard may type them.
fn apostrophes(s: &str) -> String {
    s.replace(
        [
            '\u{2019}', '\u{2018}', '\u{02bc}', '\u{2032}', '\u{00b4}', '`',
        ],
        "'",
    )
}

fn toks(text: &str) -> Vec<Tok> {
    let mut out = vec![];
    let mut at: Option<usize> = None;
    let push = |s: usize, e: usize, out: &mut Vec<Tok>| {
        let raw = &text[s..e];
        let core = raw.trim_end_matches(|c: char| {
            ")]}\"'*_`\u{201c}\u{201d}\u{2018}\u{2019}".contains(c)
                || !c.is_ascii() && !c.is_alphanumeric()
        });
        // The run of stops at its end: a '?' anywhere in it makes it a
        // question ("start #12?!").
        let tail: String = core
            .chars()
            .rev()
            .take_while(|c| ".!?,;:".contains(*c))
            .collect();
        let body = core
            .trim_end_matches(|c: char| ".!?,;:".contains(c))
            .to_lowercase();
        let stop = if tail.contains('?') || raw.contains(['?', '\u{ff1f}']) {
            Some('?')
        } else if tail.chars().count() > 1 && tail.chars().all(|c| c == '.') {
            // "..." trails off; it ends nothing.
            None
        } else if tail == "." && ABBREVIATIONS.contains(&body.as_str()) {
            // "e.g.", "etc.": an abbreviation, not a sentence's end.
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

fn number(w: &str) -> bool {
    (!w.is_empty() && w.bytes().all(|b| b.is_ascii_digit())) || NUMBERS.contains(&w)
}

/// Words that may stand anywhere in a Needs you clause and say nothing.
const FILLER: &[&str] = &[
    "the", "card", "cards", "task", "tasks", "one", "ones", "an", "a", "agent", "agents", "on",
    "for", "me", "please", "now", "too", "also", "then", "and", "both", "ok", "okay",
];

/// Words that pick cards out by set, or tie a clause together.
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

/// Words about when, which a member's name may not be ("today's").
const TIME_WORDS: &[&str] = &[
    "today",
    "tomorrow",
    "yesterday",
    "tonight",
    "morning",
    "evening",
    "week",
    "weekend",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "now",
    "soon",
    "asap",
];

/// Words that, in a reason, take the request back, make it conditional,
/// carve something out of it or say something else to do: the whole
/// message grants nothing. "not" and "can't" stay the person's own
/// ("it's not mine").
const REASON_DENY: &[&str] = &[
    "wait",
    "actually",
    "unless",
    "until",
    "if",
    "maybe",
    "instead",
    "nevermind",
    "cancel",
    "but",
    "except",
    "excluding",
    "besides",
    "never",
    "don't",
    "dont",
    "no",
    "nope",
    "nah",
    "keep",
    "hold",
    "scratch",
    "nvm",
    "jk",
    "thought",
    "sorry",
    "later",
    "tomorrow",
    "tonight",
    "after",
    "before",
    "once",
    "when",
    "whether",
    "or",
    "should",
    "would",
    "could",
    "tell",
    "start",
    "accept",
    "take",
    "reject",
    "decline",
    "give",
    "send",
    "hand",
    "park",
    "pass",
    "stop",
    "close",
    "undo",
    "retract",
    "ignore",
];

/// The longest reason a give-back carries: what the card keeps of it.
const MAX_REASON: usize = 1000;

/// Words that open another reader's instruction: close, tick, comment.
const OTHER_VERBS: &[&str] = &[
    "close", "resolve", "complete", "finish", "mark", "tick", "check", "comment", "add", "post",
    "leave", "write", "let", "message",
];

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
        || REASON_DENY.contains(&part)
        || CONTRACTIONS.contains(&part)
        || TIME_WORDS.contains(&part)
        || OTHER_VERBS.contains(&part)
        || verb_at(&[part], 0).is_some()
        || [
            "not", "go", "because", "since", "review", "done", "yes", "trash", "bin", "backlog",
            "personal", "archive", "inbox", "board", "nobody", "void",
        ]
        .contains(&part)
        // A possessive ("Spencer's Assistant") is someone else's.
        || part.ends_with("'s")
        || part.ends_with('\'')
        || number(part)
        || is_ref(part)
        || !part.chars().any(char::is_alphabetic)
        || !part
            .chars()
            .all(|c| c.is_alphanumeric() || c == '\'' || c == '-' || c == '.')
}

fn name_words(name: &str) -> Vec<String> {
    apostrophes(&name.to_lowercase())
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Letters and digits only, for comparing names however they are
/// accented or decorated: "José" and "Jo🙂" become "jos" and "jo".
fn skeleton(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Who a name at word `i` is, among everyone the snapshot knows (people
/// and givers together): their key and how many words it took. One
/// matcher for every name, and any doubt is `Err`:
/// - a second candidate of any kind (a whole name, a first name, a
///   member whose name the reader can't match but who looks the same);
/// - a hit outside `allowed` (a non-giver for "Spencer's" or a tell);
/// - a name that would run across a stop.
///
/// `max` is how many words the name may take: a tell takes one.
fn name_at(
    w: &[&str],
    stops: &[Option<char>],
    i: usize,
    everyone: &[(String, String)],
    allowed: &[String],
    possessive: bool,
    max: usize,
) -> Option<Result<(String, usize), ()>> {
    let first = *w.get(i)?;
    if possessive && CONTRACTIONS.contains(&first) {
        return None;
    }
    let bare = first
        .strip_suffix("'s")
        .or_else(|| first.strip_suffix('\''))
        .filter(|_| possessive)
        .unwrap_or(first);
    let mut hits: Vec<(String, usize)> = vec![];
    let mut shadow = false;
    for (key, name) in everyone {
        let parts = name_words(name);
        if parts.is_empty() {
            continue;
        }
        if parts.iter().any(|p| reserved(p)) {
            // Unmatchable, but "Ann" still means two people when one of
            // them is "Ann 2", "Anné" or "Ann🙂".
            let (a, b) = (skeleton(&parts[0]), skeleton(bare));
            if !a.is_empty() && !b.is_empty() && (a.starts_with(&b) || b.starts_with(&a)) {
                shadow = true;
            }
            continue;
        }
        if parts[0].chars().any(|c| !c.is_ascii()) {
            // A name typed with letters a keyboard may not have: only an
            // exact match counts, and a near one is doubt.
            let (a, b) = (skeleton(&parts[0]), skeleton(bare));
            if parts[0] != bare && !a.is_empty() && (a.starts_with(&b) || b.starts_with(&a)) {
                shadow = true;
            }
        }
        let fits = |n: usize| -> bool {
            n <= max
                && (0..n).all(|k| {
                    let Some(x) = w.get(i + k) else {
                        return false;
                    };
                    // No stop inside a name: "Ann, close #5" isn't "Ann Close".
                    if k + 1 < n && stops.get(i + k).copied().flatten().is_some() {
                        return false;
                    }
                    let want = &parts[k];
                    if k + 1 == n && possessive {
                        *x == format!("{want}'s")
                            || (want.ends_with('s') && *x == format!("{want}'"))
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
    hits.dedup_by(|a, b| a.0 == b.0);
    match (hits.as_slice(), shadow) {
        ([], false) => None,
        ([one], false) if allowed.contains(&one.0) => Some(Ok(one.clone())),
        _ => Some(Err(())),
    }
}

/// Which cards a set word ("all", "the rest") counts over.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Scope {
    /// Everything given to the person.
    All,
    /// What one giver gave ("of Spencer's").
    Giver(String),
    /// The cards named just before ("of them", "of those").
    Named,
}

/// What a clause acts on, before it is resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Ref(String),
    All(Scope),
    Rest(Scope),
    It,
    Them,
}

impl Target {
    /// A set word: it stands alone in its clause.
    fn is_set(&self) -> bool {
        matches!(self, Target::All(_) | Target::Rest(_))
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
    people: Vec<(String, String)>,
    /// People and givers together, once each: every name is matched
    /// against all of them.
    everyone: Vec<(String, String)>,
    giver_keys: Vec<String>,
    people_keys: Vec<String>,
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
        let stops: Vec<Option<char>> = t.iter().map(|x| x.stop).collect();
        while j < end && ["and", "then", "also", "please"].contains(&w[j]) {
            j += 1;
        }
        if j >= end {
            return false;
        }
        if verb_at(w, j).is_some() {
            return true;
        }
        if w[j] == "tell" && self.teller(w, &stops, j + 1).is_some() {
            return true;
        }
        if !(is_ref(w[j]) || ["the", "all", "rest", "others"].contains(&w[j])) {
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

    /// Splits sentence `t[a..b]` into clauses. A "because" reason runs to
    /// the end of the sentence, so nothing after it is a clause. A "tell
    /// <giver> …" reason ends only at a comma or semicolon before the next
    /// verb.
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
            if !tell && w[k] == "because" {
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

    /// The giver a tell names, by one word: "tell Spencer Little chance…"
    /// is Spencer, and "Little" is the person's words.
    fn teller(
        &self,
        w: &[&str],
        stops: &[Option<char>],
        j: usize,
    ) -> Option<Result<(String, usize), ()>> {
        name_at(w, stops, j, &self.everyone, &self.giver_keys, false, 1)
    }

    /// The person a give is to.
    fn person(
        &self,
        w: &[&str],
        stops: &[Option<char>],
        j: usize,
    ) -> Option<Result<(String, usize), ()>> {
        name_at(
            w,
            stops,
            j,
            &self.everyone,
            &self.people_keys,
            false,
            usize::MAX,
        )
    }

    /// The giver's possessive ("Spencer's").
    fn whose(
        &self,
        w: &[&str],
        stops: &[Option<char>],
        j: usize,
    ) -> Option<Result<(String, usize), ()>> {
        name_at(
            w,
            stops,
            j,
            &self.everyone,
            &self.giver_keys,
            true,
            usize::MAX,
        )
    }

    /// "of them", "of Spencer's" after a set word: what it counts over,
    /// and the words that took.
    fn of(&self, w: &[&str], stops: &[Option<char>], j: usize) -> Result<(Scope, usize), ()> {
        if w.get(j) != Some(&"of") {
            return Ok((Scope::All, 0));
        }
        if matches!(w.get(j + 1), Some(&"them" | &"these" | &"those")) {
            return Ok((Scope::Named, 2));
        }
        match self.whose(w, stops, j + 1) {
            Some(Ok((k, n))) => Ok((Scope::Giver(k), n + 1)),
            _ => Err(()),
        }
    }

    /// One clause, `t[a..b]`. `None`: words it doesn't understand.
    fn clause(&self, t: &[Tok], w: &[&str], a: usize, b: usize) -> Option<Clause> {
        // Nothing past the clause is looked at.
        let (t, w) = (&t[..b], &w[..b]);
        let stops: Vec<Option<char>> = t.iter().map(|x| x.stop).collect();
        let mut c = Clause::default();
        let mut j = a;
        while j < b && OPENERS.contains(&w[j]) {
            j += 1;
        }
        // "tell Spencer it's too big": the rest is the reason.
        if w.get(j) == Some(&"tell") {
            let (key, n) = self.teller(w, &stops, j + 1)?.ok()?;
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
        let mut verb_end = None;
        while j < b {
            let x = w[j];
            // A reason runs to the end of the clause, as typed.
            if x == "because" {
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
                j += n;
                verb_end = Some(j);
                // A colon after a verb starts nothing it reads.
                if t[j - 1].stop == Some(':') {
                    return None;
                }
                continue;
            }
            let n = if is_ref(x) {
                c.targets.push(Target::Ref(norm_ref(x)));
                1
            } else if x == "back" {
                c.back = true;
                1
            } else if x == "to" && c.verb == Some(Verb::Give) && c.to.is_none() {
                let (k, n) = self.person(w, &stops, j + 1)?.ok()?;
                c.to = Some(k);
                1 + n
            } else if (x == "all" || x == "everything") && t[j].stop.is_some() {
                c.targets.push(Target::All(Scope::All));
                1
            } else if x == "all" || x == "everything" {
                let (scope, n) = match self.of(w, &stops, j + 1).ok()? {
                    (Scope::All, 0) => match self.whose(w, &stops, j + 1) {
                        Some(Ok((k, n))) => (Scope::Giver(k), n),
                        Some(Err(())) => return None,
                        None => (Scope::All, 0),
                    },
                    v => v,
                };
                c.targets.push(Target::All(scope));
                1 + n
            } else if x == "rest"
                || x == "others"
                || (x == "other" && w.get(j + 1) == Some(&"ones"))
            {
                let m = usize::from(x == "other");
                let (scope, o) = self.of(w, &stops, j + 1 + m).ok()?;
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
            } else if let Some(r) = self.whose(w, &stops, j) {
                let (k, n) = r.ok()?;
                c.targets.push(Target::All(Scope::Giver(k)));
                n
            } else if c.verb == Some(Verb::Give) && c.to.is_none() && verb_end == Some(j) {
                // "give Spencer #12": the person right after the verb.
                let (k, n) = self.person(w, &stops, j)?.ok()?;
                c.to = Some(k);
                n
            } else {
                return None;
            };
            j += n;
            // A colon inside the clause starts nothing it reads.
            if t[j - n..j].iter().any(|x| x.stop == Some(':')) {
                return None;
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
        // A set word stands alone: "Spencer's #12" and "them all" say two
        // things at once.
        if c.targets.iter().any(Target::is_set) && c.targets.len() > 1 {
            return None;
        }
        // Cards were given while the question waited: the list the person
        // read isn't the snapshot, so only their refs count.
        if self.snap.partial && c.targets.iter().any(Target::is_set) {
            return None;
        }
        // Only a give-back carries the person's words.
        if c.reason.is_some() && c.verb != Some(Verb::GiveBack) {
            return None;
        }
        // A "because" reason goes with the cards it names, never with
        // "all", "the rest" or "Spencer's": it may carve some out.
        if c.reason.is_some() && c.targets.iter().any(Target::is_set) {
            return None;
        }
        // "reject #15 for now" is a Not now, not a give-back.
        if c.verb == Some(Verb::GiveBack)
            && w[a..].windows(2).any(|p| p[0] == "for" && p[1] == "now")
        {
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
    /// something, asks nothing, names no card given to them, takes nothing
    /// back or carves nothing out, and fits what the card keeps.
    fn reason_ok(&self, reason: &str) -> bool {
        if reason.chars().count() > MAX_REASON {
            return false;
        }
        // One plain clause: letters, digits, apostrophes and spaces, with a
        // '.' or '-' only inside a word. No ref, no question, no second
        // sentence, list, quote or aside.
        let folded = apostrophes(reason);
        let cs: Vec<char> = folded.chars().collect();
        for (k, c) in cs.iter().enumerate() {
            let inside = k > 0
                && cs[k - 1].is_alphanumeric()
                && cs.get(k + 1).is_some_and(|n| n.is_alphanumeric());
            let ok = c.is_alphanumeric()
                || *c == ' '
                || *c == '\''
                || ((*c == '.' || *c == '-') && inside);
            if !ok {
                return false;
            }
        }
        // No Needs you verb: "because not now" says something else to do.
        let low = folded.to_lowercase();
        let ws: Vec<&str> = low.split(' ').filter(|x| !x.is_empty()).collect();
        if (0..ws.len()).any(|k| verb_at(&ws, k).is_some()) {
            return false;
        }
        // No member's name: the reason goes to one giver, and a name in it
        // may be meant for someone else.
        let names: Vec<String> = self
            .everyone
            .iter()
            .flat_map(|(_, n)| name_words(n))
            .filter(|p| !reserved(p))
            .collect();
        if folded.to_lowercase().split(' ').any(|x| {
            let x = x
                .strip_suffix("'s")
                .or_else(|| x.strip_suffix('\''))
                .unwrap_or(x);
            names.iter().any(|n| n == x)
        }) {
            return false;
        }
        // Word by word, whatever punctuation is stuck to them.
        let folded = apostrophes(&reason.to_lowercase());
        let words: Vec<&str> = folded
            .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '#'))
            .filter(|x| !x.is_empty())
            .collect();
        words
            .iter()
            .any(|x| x.chars().any(char::is_alphabetic) && !is_ref(x))
            && !words.iter().any(|x| {
                REASON_DENY.contains(x) || (is_ref(x) && !self.item_refs(&norm_ref(x)).is_empty())
            })
    }
}

/// The refs `text` names, as they resolve in the snapshot (each exactly
/// one item), or `None` when one doesn't.
fn earlier_items(r: &Reader, text: &str) -> Option<Vec<usize>> {
    let mut out = vec![];
    for t in toks(text) {
        let w = earlier_ref(&t.w)?;
        if is_ref(w) {
            match r.item_refs(&norm_ref(w)).as_slice() {
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
/// something in the snapshot it could act on, and it doesn't open as
/// another reader's instruction ("close #5 since #12 will take over").
fn about_needs(r: &Reader, t: &[Tok], w: &[&str]) -> bool {
    let words: Vec<String> = w.iter().map(|s| s.to_string()).collect();
    let (i, _) = opening(&words);
    if w.get(i).is_some_and(|x| OTHER_VERBS.contains(x)) {
        return false;
    }
    let stops: Vec<Option<char>> = t.iter().map(|x| x.stop).collect();
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
            || r.whose(w, &stops, i).is_some()
    });
    let give = w.iter().any(|x| ["give", "send", "hand"].contains(x))
        && w.iter()
            .any(|x| is_ref(x) || ["it", "this", "that"].contains(x))
        && (0..w.len()).any(|i| r.person(w, &stops, i).is_some());
    verb && (target || give)
}

/// Reads `text` for Needs you. `earlier` is what the person typed before,
/// oldest first ("start it" means the one they named just before).
pub(crate) fn read(text: &str, earlier: &[&str], snap: &Snapshot) -> Read {
    let mut givers: Vec<(String, String)> = vec![];
    for it in &snap.items {
        let g = (it.giver_key.clone(), it.giver_name.clone());
        if !givers.contains(&g) {
            givers.push(g);
        }
    }
    let people: Vec<(String, String)> = snap
        .people
        .iter()
        .map(|p| (p.person_key.clone(), p.name.clone()))
        .collect();
    let mut everyone = people.clone();
    for g in &givers {
        if !everyone.iter().any(|(k, _)| *k == g.0) {
            everyone.push(g.clone());
        }
    }
    let r = Reader {
        text,
        snap,
        giver_keys: givers.iter().map(|g| g.0.clone()).collect(),
        people_keys: people.iter().map(|p| p.0.clone()).collect(),
        people,
        everyone,
    };
    let t = toks(text);
    let w: Vec<&str> = t.iter().map(|x| x.w.as_str()).collect();
    if !about_needs(&r, &t, &w) {
        return Read::Not;
    }
    match parse(&r, &t, &w, earlier) {
        Some(g) if !g.is_empty() => Read::Grants(g),
        _ => Read::Refused,
    }
}

/// The cards the person's previous message named, all of them given to
/// them. Only that message: a newer one about something else ("and what
/// about Blake's?") means "it" isn't known.
fn earlier_named(r: &Reader, earlier: &[&str]) -> Option<Vec<usize>> {
    let v = earlier_items(r, earlier.last()?)?;
    (!v.is_empty()).then_some(v)
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
    // A clause counted over "of Spencer's" or "of those": a later bare
    // "the rest" would be unclear.
    let mut scoped = false;
    // A reason closes the message: nothing may follow it.
    let mut closed = false;
    // Where the person's own reasons are: hold-back words there are theirs
    // to say.
    let mut reasons: Vec<(usize, usize)> = vec![];
    for (a, b, question) in sentences {
        if closed {
            return None;
        }
        let (i, asked) = opening(&w[a..b].iter().map(|s| s.to_string()).collect::<Vec<_>>());
        // A question in a message about Needs you leaves it unsure:
        // "start all of Spencer's. Or just #12?"
        if question && !asked {
            return None;
        }
        // "go ahead with #12": the opening took "go ahead".
        let mut a2 = a + i;
        if w.get(a2) == Some(&"with") && a2 >= 2 && w[a2 - 2] == "go" && w[a2 - 1] == "ahead" {
            a2 -= 2;
        }
        for (ca, cb) in r.clauses(t, w, a2, b) {
            if closed {
                return None;
            }
            let c = r.clause(t, w, ca, cb)?;
            if let Some(why) = &c.reason {
                // "can you reject #15 because …?" asks; it doesn't say.
                if question || !r.reason_ok(why) {
                    return None;
                }
                reasons.push((reason_start(w, ca, cb, &c), cb));
                // A "because" reason runs to the sentence's end, so nothing
                // may follow it; "tell <giver> …" may open the message.
                closed = c.tell.is_none();
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
                        // when it named nothing yet, the previous message's.
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
            // What "them", "it" and "of those" point at: this message's
            // last named cards, or, when it named none and gave none, the
            // previous message's.
            let pointed = || -> Option<Vec<usize>> {
                if !last.is_empty() {
                    Some(last.clone())
                } else if gives.is_empty() {
                    earlier_named(r, earlier)
                } else {
                    None
                }
            };
            let in_scope = |s: &Scope| -> Option<Vec<usize>> {
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
                    Target::All(s) => in_scope(s)?,
                    Target::Rest(s) => {
                        if named.is_empty() || (scoped && *s == Scope::All) {
                            return None;
                        }
                        in_scope(s)?
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
            if c.targets.iter().any(|tg| {
                matches!(
                    tg,
                    Target::All(Scope::Giver(_) | Scope::Named)
                        | Target::Rest(Scope::Giver(_) | Scope::Named)
                        | Target::It
                        | Target::Them
                )
            }) {
                scoped = true;
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
        let (card, refs, question) = (it.id.clone(), it.refs.clone(), it.question.clone());
        out.push(match verb {
            Verb::Start => Grant::Start {
                card,
                refs,
                question,
            },
            Verb::NotNow => Grant::NotNow {
                card,
                refs,
                question,
            },
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
    // The spend side checks a give again by card id: none of these, and no
    // card in two gives, however the model spells it.
    let shut: Vec<String> = r.snap.items.iter().map(|it| it.id.clone()).collect();
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
        out.push(Grant::Give {
            to,
            name,
            cards,
            shut: shut.clone(),
        });
    }
    Some(out)
}

/// Where clause `c`'s reason starts in `w[a..b]` (`b` when it has none).
fn reason_start(w: &[&str], a: usize, b: usize, c: &Clause) -> usize {
    if c.reason.is_none() {
        return b;
    }
    for k in a..b {
        if c.tell.is_some() && w[k] == "tell" {
            return k;
        }
        if w[k] == "because" {
            return k + 1;
        }
    }
    b
}

/// A word of an earlier message as a ref may be read: "#12's" is #12; a
/// "#" this reader can't read ("#12/#13", "'#12'") makes the message
/// unclear, so `None`.
fn earlier_ref(w: &str) -> Option<&str> {
    let x = w
        .strip_suffix("'s")
        .or_else(|| w.strip_suffix('\''))
        .unwrap_or(w);
    if x.contains('#') && !is_ref(x) {
        return None;
    }
    Some(x)
}

/// The one card the person's previous message named.
fn earlier_one_ref(earlier: &[&str]) -> Option<String> {
    let mut refs: Vec<String> = vec![];
    for t in toks(earlier.last()?) {
        let w = earlier_ref(&t.w)?;
        if is_ref(w) {
            let r = norm_ref(w);
            if !refs.contains(&r) {
                refs.push(r);
            }
        }
    }
    match refs.len() {
        1 => refs.pop(),
        _ => None,
    }
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
            question: String::new(),
        }
    }
    fn not_now(id: &str) -> Grant {
        Grant::NotNow {
            card: id.into(),
            refs: refs(id),
            question: String::new(),
        }
    }
    fn back(id: &str, why: &str) -> Grant {
        Grant::GiveBack {
            card: id.into(),
            refs: refs(id),
            reason: why.into(),
        }
    }

    /// A give's `shut` is the snapshot's ids, checked on its own below.
    fn unshut(v: Vec<Grant>) -> Vec<Grant> {
        v.into_iter()
            .map(|x| match x {
                Grant::Give {
                    to, name, cards, ..
                } => Grant::Give {
                    to,
                    name,
                    cards,
                    shut: vec![],
                },
                x => x,
            })
            .collect()
    }

    fn g(text: &str, s: &Snapshot) -> Vec<Grant> {
        unshut(grants_for(text, &[], s))
    }

    fn gf(text: &str, earlier: &[&str], s: &Snapshot) -> Vec<Grant> {
        unshut(grants_for(text, earlier, s))
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
                "accept #12, #13 and #14, reject the rest, tell Spencer it's too big",
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
            (
                "decline #13 because it's too vague",
                vec![back("t13", "it's too vague")],
            ),
            (
                "reject #15 because it duplicates another card",
                vec![back("t15", "it duplicates another card")],
            ),
            ("give #15 back", vec![back("t15", "")]),
            ("send back #15", vec![back("t15", "")]),
            ("pass on #15", vec![back("t15", "")]),
            ("start Spencer's", all_spencers.clone()),
            ("start all of Blake Moss's", vec![start("t7")]),
            ("accept #14 and #15", vec![start("t14"), start("t15")]),
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
                "give back #15 because it's big and old",
                vec![back("t15", "it's big and old")],
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
            g("give back #15 because I can't do this one", &s),
            vec![back("t15", "I can't do this one")]
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
            shut: vec![],
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
            gf("give it to Ann", &["what's #3?"], &s),
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

    /// A message that opens as a close is the close reader's: "start #12"
    /// after it is no close, and it starts nothing.
    #[test]
    fn a_close_first_message_grants_only_its_close() {
        assert_eq!(
            g("close #5. start #12", &snap()),
            vec![Grant::Close { card: "#5".into() }]
        );
        let t = "close #5 since #12 will take over";
        assert_eq!(
            g(t, &snap()),
            grants_for(t, &[], &Snapshot::default()),
            "{t}"
        );
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
            vec![]
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
            "accept #12, #13 and #14, reject the rest, tell Spencer that it's too big",
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
            shut: vec![],
        };
        assert_eq!(
            g("give #3 and #4 to Ann please", &names),
            vec![],
            "two members start with Ann: which one isn't known"
        );
        let _ = give("person:pAnn", "Ann", &["#3"]);
        for text in [
            "give #3 and #4 to the team",
            "give #3 to me",
            "give #3 to someone",
        ] {
            assert_eq!(g(text, &names), vec![], "{text}");
        }
    }

    /// The advisor's three, and abbreviations: dash and colon reasons are
    /// read strictly; a snapshot that left out late cards takes refs only;
    /// "it" reaches back to the previous message alone; "e.g." ends no
    /// sentence.
    #[test]
    fn second_thoughts_late_cards_and_old_pronouns_grant_nothing() {
        let s = snap();
        for text in [
            "give back #15 - not yet",
            "give back #15: don't",
            "reject #15 - no, keep it",
            "reject #15: not mine",
            "decline #13 \u{2014} never mind",
        ] {
            assert_eq!(g(text, &s), vec![], "{text}");
        }
        assert_eq!(
            g("give back #15 because it's not mine", &s),
            vec![back("t15", "it's not mine")]
        );
        assert_eq!(
            g(
                "give back #15 because it's out of scope for this sprint",
                &s
            ),
            vec![back("t15", "it's out of scope for this sprint")]
        );

        let mut late = snap();
        late.partial = true;
        for text in [
            "start all",
            "accept the first two",
            "start all of Spencer's",
            "start #12, reject the rest",
        ] {
            assert_eq!(g(text, &late), vec![], "{text}");
        }
        assert_eq!(g("start #12", &late), vec![start("t12")]);

        assert_eq!(
            grants_for(
                "start it",
                &["what's ops#12?", "and what about Blake's?"],
                &s
            ),
            vec![]
        );
        assert_eq!(
            grants_for("give it to Ann", &["what's #3?", "ok thanks"], &s),
            vec![]
        );
    }

    /// Round two of the adversarial pass on #1226: each of these granted
    /// more than the person said. Each now grants nothing.
    #[test]
    fn round_two_cases_grant_nothing() {
        let s = snap();
        for text in [
            // An exception or a second thought inside a reason.
            "reject all because I'm swamped, but not #12",
            "reject all - except Blake's",
            "reject all, tell Spencer I'm swamped, but not Blake's",
            "give back #15 \u{2014} on second thought, keep it",
            "reject #15 - hmm, no",
            "reject #15 - scratch that",
            "give back #15: hold off",
            "reject #15 - nvm",
            "give back #15 - later",
            "give back #15 because it's too big - no, don't",
            "give back #15 because it's too big, never mind",
            "give back #15 because it's too big, but only after Spencer replies",
            "reject #15, tell Spencer it's too big, never mind",
            // A sentence after a reason, or a reason inside a question.
            "Give back #15 because Spencer said no. Don't.",
            "Give back #15 because it's fixed in v1.2. Not now.",
            "can you reject #15 because it's big, or should I keep it?",
            "reject all because I'm swamped, tell Spencer sorry",
            "reject #15 because it's too big; start Blake's",
            // Punctuation stuck to a word.
            "give back #15 - wait\u{2014}keep it",
            "give back #15: don\u{00b4}t",
            "give back #15 because it's done?\u{1f914}",
            // Positions.
            "accept the first two, reject the rest",
            "accept the first two of Spencer's, reject the rest",
            // Idioms around a give.
            "give #3 a go",
            "send #3 for review",
        ] {
            assert_eq!(g(text, &s), vec![], "{text}");
        }
        assert_eq!(
            g(
                &format!("give back #15 because {}", "x".repeat(MAX_REASON + 1)),
                &s
            ),
            vec![]
        );
        // A pronoun after a give doesn't reach back past it.
        for (text, before) in [
            ("give #3 to Ann, then start it", "what's #12?"),
            (
                "give #3 and #4 to Blake, then reject them",
                "what are #12 and #13?",
            ),
        ] {
            assert_eq!(gf(text, &[before], &s), vec![], "{text}");
        }
        // "the rest" after a scoped clause is unclear.
        assert_eq!(
            gf(
                "accept all of them, reject the rest",
                &["what are ops#13 and ops#14?"],
                &s
            ),
            vec![]
        );
        // Names: no run across a comma; another member's whole name is never
        // beaten; an unmatchable namesake makes it ambiguous; no recipient
        // away from the verb; no time word as a giver.
        let person = |s: &mut Snapshot, k: &str, n: &str| {
            s.people.push(SnapPerson {
                person_key: k.into(),
                name: n.into(),
            })
        };
        let mut ann = snap();
        person(&mut ann, "person:pAC", "Ann Close");
        assert_eq!(g("give #3 to Ann, close #5", &ann), vec![]);
        for extra in ["Ann Thx", "Ann Lee"] {
            let mut two = snap();
            person(&mut two, "person:pX", extra);
            assert_eq!(g("give #3 to Ann", &two), vec![], "{extra}");
        }
        let mut lee = snap();
        person(&mut lee, "person:pLee", "Ann Lee");
        assert_eq!(
            g("give #3 to Ann Lee", &lee),
            vec![],
            "Ann's whole name is also a match for the first word"
        );
        let mut two = snap();
        person(&mut two, "person:pAnn2", "Ann 2");
        assert_eq!(g("give #3 to Ann", &two), vec![]);
        let mut go = snap();
        person(&mut go, "person:pGo", "Go Takahashi");
        assert_eq!(gf("give it a go", &["what's #3?"], &go), vec![]);
        let mut today = snap();
        today.items.push(
            Snapshot::new(
                vec![item(
                    "x1",
                    "dev#1",
                    ("person:pToday", "Today"),
                    ItemState::Asked,
                )],
                vec![],
            )
            .items
            .remove(0),
        );
        assert_eq!(g("start today's", &today), vec![]);
        let sb = Snapshot::new(
            vec![
                item("t15", "ops#15", (SPENCER, "Spencer"), ItemState::Asked),
                item(
                    "x1",
                    "dev#1",
                    ("person:pSB", "Spencer Blake"),
                    ItemState::Asked,
                ),
            ],
            people(),
        );
        assert_eq!(
            g("reject #15 and dev#1, tell Spencer: Blake has it", &sb),
            vec![]
        );
        // What still reads.
        assert_eq!(
            g(
                "give back #15 because it's out of scope for this sprint",
                &s
            ),
            vec![back("t15", "it's out of scope for this sprint")]
        );
        // A "because" reason never rides on a set word; a tell's may.
        assert_eq!(
            g("reject all of Spencer's because they're stale", &s),
            vec![]
        );
        assert_eq!(
            g("reject all of Spencer's, tell Spencer they're stale", &s),
            vec![
                back("t12", "they're stale"),
                back("t13", "they're stale"),
                back("t14", "they're stale"),
                back("t15", "they're stale"),
            ]
        );
    }

    /// A give carries the snapshot's ids, and the spend side refuses any of
    /// them, and any card two open gives name, however it is spelled.
    #[test]
    fn a_give_is_refused_at_spend_for_a_given_card_or_a_card_given_twice() {
        let s = snap();
        let got = grants_for("give #3 to Ann", &[], &s);
        let Some(Grant::Give { shut, .. }) = got.first() else {
            panic!("{got:?}");
        };
        assert_eq!(shut.len(), s.items.len());
        let resolve = |r: &str| -> Option<String> {
            let r = r.strip_prefix("task:").unwrap_or(r);
            match r {
                "#3" | "p3" => Some("p3".into()),
                "old#9" => Some("t12".into()),
                "ops#40" | "#40" | "p40" => Some("p40".into()),
                _ => None,
            }
        };
        let ids = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let gs = Grants::default();
        // An old ref that resolves to a card given to them.
        let run = gs.begin(vec![Grant::Give {
            to: "person:pAnn".into(),
            name: "Ann".into(),
            cards: ids(&["old#9"]),
            shut: s.items.iter().map(|i| i.id.clone()).collect(),
        }]);
        assert_eq!(gs.take_give("Ann", &ids(&["old#9"]), &resolve), None);
        gs.end(run);
        // One card, two people, two spellings.
        let run = gs.begin(vec![
            Grant::Give {
                to: "person:pAnn".into(),
                name: "Ann".into(),
                cards: ids(&["ops#40"]),
                shut: vec![],
            },
            Grant::Give {
                to: BLAKE.into(),
                name: "Blake Moss".into(),
                cards: ids(&["#40"]),
                shut: vec![],
            },
        ]);
        assert_eq!(gs.take_give("Ann", &ids(&["ops#40"]), &resolve), None);
        assert_eq!(gs.take_give(BLAKE, &ids(&["#40"]), &resolve), None);
        gs.end(run);
        let run = gs.begin(grants_for("give #3 to Ann", &[], &s));
        assert_eq!(
            gs.take_give("Ann", &ids(&["#3"]), &resolve),
            Some(("person:pAnn".into(), ids(&["task:p3"])))
        );
        gs.end(run);
    }

    /// Round three of the adversarial pass on #1226, against the shape
    /// rules: a reason is one plain clause with no ref and no member's
    /// name; a "because" reason never rides on a set word; a tell takes
    /// one name word; names never look past a stop.
    #[test]
    fn round_three_cases_grant_nothing() {
        let s = snap();
        for text in [
            "reject all because I'm swamped, not Blake's",
            "reject all because I'm swamped; leave Blake's",
            "reject all, tell Spencer I'm swamped, not Blake's",
            "Give back #15 because it's fixed in v1.2... Not now.",
            "Give back #15 because it's fixed in v1.2\u{2026} Not now.",
            "Give back #15 because it's fixed, etc. Not now.",
            "give back #15 because it's too big, just kidding",
            "give back #15 because it's too big, j/k",
            "give back #15 because it's too big, do it anyway",
            "give back #15 because it's too big - hang on",
            "give back #15 because it's too big, forget it",
            "give back #15 because not now",
            "reject #15, tell Spencer it's too big, just kidding",
            "give back #15 because it's too big, as long as Spencer agrees",
            "give back #15 because it's done\u{ff1f}",
            "reject #15, tell Spencer is this even yours\u{ff1f}",
            "reject #15 and #7, tell Spencer and Blake it's too big",
            "reject #15 and #7 because Spencer, these are dupes",
            "reject #15 for now",
            "reject #15, tell Spencer \"it's too big, the rest later\"",
            "reject #15, tell Spencer \u{201c}it's too big, start #12\u{201d}",
            "give back #15 because it's too big, do not",
            "give back #15 because it's too big, not yet",
            "reject #15, tell Spencer it's too big, not yet",
            "reject everything; Blake's okay",
            "reject all, Blake's ok",
        ] {
            assert_eq!(g(text, &s), vec![], "{text}");
        }
        assert_eq!(
            gf(
                "accept them, reject the rest",
                &["what are ops#13 and ops#14?"],
                &s
            ),
            vec![]
        );
        assert_eq!(
            gf("start it", &["is #12's scope smaller than #13?"], &s),
            vec![]
        );
        assert_eq!(gf("give it to Ann", &["is #3's fix in #4?"], &s), vec![]);
        let with_person = |n: &str| {
            let mut x = snap();
            x.people.push(SnapPerson {
                person_key: "person:pX".into(),
                name: n.into(),
            });
            x
        };
        assert_eq!(g("send #3 to trash", &with_person("Trash")), vec![]);
        assert_eq!(
            g(
                "give #3 to Spencer's assistant",
                &with_person("Spencer's Assistant")
            ),
            vec![]
        );
        let mut jose = snap();
        for (k, n) in [
            ("person:pJ1", "Jos\u{e9}"),
            ("person:pJ2", "Jose Ruiz"),
            ("person:pJo", "Jo\u{1f642}"),
            ("person:pJL", "Jo Lee"),
        ] {
            jose.people.push(SnapPerson {
                person_key: k.into(),
                name: n.into(),
            });
        }
        assert_eq!(g("give #3 to Jose", &jose), vec![]);
        assert_eq!(g("give #3 to Jo", &jose), vec![]);
        let al = ("person:pAL", "Ann Lee");
        let mut ann = Snapshot::new(
            vec![
                item("x1", "dev#1", al, ItemState::Asked),
                item("x2", "dev#2", al, ItemState::Asked),
            ],
            people(),
        );
        ann.people.push(SnapPerson {
            person_key: "person:pAL".into(),
            name: "Ann Lee".into(),
        });
        for text in [
            "start Ann's",
            "reject all of Ann's",
            "reject dev#1, tell Ann it's too big",
            "tell Ann it's too big, reject dev#1",
        ] {
            assert_eq!(g(text, &ann), vec![], "{text}");
        }
        let little = Snapshot::new(
            vec![item(
                "x1",
                "dev#1",
                ("person:pSL", "Spencer Little"),
                ItemState::Asked,
            )],
            people(),
        );
        let got = g(
            "reject dev#1, tell Spencer little chance we get to it",
            &little,
        );
        assert!(
            !got.iter().any(
                |x| matches!(x, Grant::GiveBack { reason, .. } if reason == "chance we get to it")
            ),
            "{got:?}"
        );
    }

    /// Member-controlled names never add, widen or redirect a grant: for
    /// each message, a member (and a giver of one card) is named after each
    /// word, pair of words and possessive in it. A message that granted
    /// grants the same or nothing; one that granted nothing still does.
    #[test]
    fn no_member_name_changes_what_a_message_grants() {
        let granted = [
            "start ops#13",
            "Please accept #12 and #13.",
            "can you start ops#13?",
            "go ahead with #12",
            "ok, start an agent on #12",
            "park #12",
            "#12 later",
            "not now #12",
            "start #12 and #13 later",
            "decline #13 because it's too vague",
            "give #15 back",
            "send back #15",
            "pass on #15",
            "give back #15 because it's not mine",
            "tell Spencer it's too big, give back #15",
            "give #3 and #4 to Blake",
            "send Spencer #3",
            "hand ops#40 to Ann",
            "start all of Spencer's",
            "start #12, give the others back",
            "accept #12, #13 and #14, reject the rest, tell Spencer it's too big",
        ];
        let refused = [
            "don't start #12",
            "maybe start #12",
            "start #12 when the tests pass",
            "should I start #12?",
            "start #12. Or not?",
            "start #99",
            "start #12, tell Spencer thanks",
            "start #12 and close #5",
            "take a look at #12",
            "start the first six",
            "reject the rest",
            "start it",
            "start #12, but not #13",
            "give back Spencer's #15 because it's not mine",
            "start #13 first",
            "give #3 a go",
            "reject #15 for now",
            "reject all because I'm swamped, not Blake's",
            "reject everything; Blake's okay",
            "give back #15 - not yet",
        ];
        let names_in = |text: &str| -> Vec<String> {
            let words: Vec<String> = text
                .to_lowercase()
                .split_whitespace()
                .map(|w| w.trim_matches(|c: char| ",.!?;:\"".contains(c)).to_string())
                .filter(|w| !w.is_empty())
                .collect();
            let mut out: Vec<String> = vec![];
            for (i, w) in words.iter().enumerate() {
                let bare = w.strip_suffix("'s").unwrap_or(w).to_string();
                for n in [w.clone(), bare.clone()] {
                    if !out.contains(&n) {
                        out.push(n);
                    }
                }
                if let Some(next) = words.get(i + 1) {
                    let pair = format!("{bare} {}", next.strip_suffix("'s").unwrap_or(next));
                    if !out.contains(&pair) {
                        out.push(pair);
                    }
                }
            }
            out
        };
        let set_word = |t: &str| {
            let l = t.to_lowercase();
            ["all", "everything", "rest", "others"]
                .iter()
                .any(|x| l.split(|c: char| !c.is_alphanumeric()).any(|w| w == *x))
        };
        let mut runs = 0;
        for (text, must_grant) in granted
            .iter()
            .map(|t| (*t, true))
            .chain(refused.iter().map(|t| (*t, false)))
        {
            let base = g(text, &snap());
            assert_eq!(!base.is_empty(), must_grant, "{text}: {base:?}");
            for name in names_in(text) {
                // As another member.
                let mut p = snap();
                p.people.push(SnapPerson {
                    person_key: "person:pEvil".into(),
                    name: name.clone(),
                });
                // As a giver of one card, too, unless a set word would
                // rightly count that card.
                let mut variants = vec![p];
                if !set_word(text) {
                    let mut gv = snap();
                    gv.items.push(
                        Snapshot::new(
                            vec![item(
                                "x99",
                                "evil#777",
                                ("person:pEvil", &name),
                                ItemState::Asked,
                            )],
                            vec![],
                        )
                        .items
                        .remove(0),
                    );
                    variants.push(gv);
                }
                for v in variants {
                    runs += 1;
                    let got = g(text, &v);
                    assert!(
                        got.is_empty() || got == base,
                        "{text} with a member named {name:?}: {got:?} (alone: {base:?})"
                    );
                }
            }
        }
        assert!(runs > 300, "{runs}");
    }
}
