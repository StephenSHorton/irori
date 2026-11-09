//! When a huddle's assistant looks (card #1048, docs/design/call-agents.md
//! §5 and §6).
//!
//! Pure Rust, no I/O, no clock of its own: every function takes `now`, so each
//! rule is tested with a made-up clock. The feed (card 7b) and the digests
//! (card 10) call it; nothing here reads rows, runs the model or writes the
//! canvas.
//!
//! - **The matcher** ([`call_target`]): which line calls which assistant. Loose
//!   in spelling, strict in position.
//! - **The layers** ([`Brain`]): L1 addressed, L2 own person asks, L3 own digest,
//!   L3b routed question, L4 end, with the skips, the throttles, one look at a
//!   time, the hold for the rest of the sentence, freshness and the cap.
//! - **The question-to-card match** ([`route_question`]) for L3b.
//! - **Blocks** ([`admit`]): what a look's reply may write.
//! - **The chair, from rows** ([`chair`], [`due_writer`]).

// The feed and the digests (cards 7b and 10) are the callers.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, SystemTime};

// ---------------------------------------------------------------- constants

/// A line triggers L1 or L2 only if it ended within this long of now. Older
/// rows (a host back from sleep) are context only.
pub(super) const FRESH: Duration = Duration::from_secs(60);
/// After a match, wait this long for the speaker's next line.
pub(super) const HOLD: Duration = Duration::from_millis(1500);
/// A line from the same speaker within this long of a look starts one more.
pub(super) const FOLLOW_UP: Duration = Duration::from_secs(8);
/// Another person's L1: at most one look per caller per this long.
pub(super) const L1_OTHERS_EVERY: Duration = Duration::from_secs(15);
/// L2 at most this often.
pub(super) const L2_EVERY: Duration = Duration::from_secs(15);
/// L3 at least this far apart.
pub(super) const L3_APART: Duration = Duration::from_secs(30);
/// L3 needs this much of its own person's talk since the last look.
pub(super) const L3_TALK: Duration = Duration::from_secs(15);
/// ... and then this long without a line (or the speaker changing).
pub(super) const L3_GAP: Duration = Duration::from_secs(4);
/// L3 is forced at this much of its person's talk.
pub(super) const L3_FORCE: Duration = Duration::from_secs(60);
/// Looks per huddle per assistant.
pub(super) const CAP: u32 = 300;
/// A look the runner never reported done is given up after this long.
pub(super) const BUSY_MAX: Duration = Duration::from_secs(90);
/// Looks kept waiting behind the running one.
const QUEUE_MAX: usize = 4;
/// One line's talk counts for at most this much (`MAX_PHRASE`).
const LINE_TALK_MAX: Duration = Duration::from_secs(25);
/// A connected assistant whose host has been away this long counts as off.
pub(super) const AWAY_AFTER: Duration = Duration::from_secs(60);
/// How long the chair has to write the summary or end mark before the next
/// assistant does.
pub(super) const WRITE_WAIT: Duration = Duration::from_secs(3 * 60);
/// A reply writes at most this many items per look, each this long.
pub(super) const ITEMS_PER_LOOK: usize = 5;
pub(super) const ITEM_CHARS: usize = 2000;

fn since(now: SystemTime, then: SystemTime) -> Duration {
    now.duration_since(then).unwrap_or_default()
}

// ---------------------------------------------------------------- tokens

/// One word of a line.
#[derive(Debug, Clone)]
struct Tok {
    /// Lower-case, without the possessive mark.
    word: String,
    /// "Addam's" is `addam` plus a possessive mark ("who's" is `who` plus one).
    poss: bool,
    /// A `, ? . ! ; :` follows it.
    brk: bool,
    /// A comma follows it.
    comma: bool,
}

impl Tok {
    /// The word as spoken, possessive mark put back.
    fn plain(&self) -> String {
        if self.poss {
            format!("{}'s", self.word)
        } else {
            self.word.clone()
        }
    }
}

fn push_word(cur: &mut String, out: &mut Vec<Tok>) {
    if cur.is_empty() {
        return;
    }
    let mut word = std::mem::take(cur);
    let mut poss = false;
    if let Some(stem) = word.strip_suffix("'s") {
        word = stem.to_string();
        poss = true;
    } else if let Some(stem) = word.strip_suffix('\'') {
        word = stem.to_string();
        poss = true;
    }
    if word.is_empty() {
        return;
    }
    out.push(Tok {
        word,
        poss,
        brk: false,
        comma: false,
    });
}

/// Lower-case, strip punctuation, split into words.
fn tokens(text: &str) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || ((c == '\'' || c == '’') && !cur.is_empty()) {
            if c == '’' {
                cur.push('\'');
            } else {
                cur.extend(c.to_lowercase());
            }
        } else {
            push_word(&mut cur, &mut out);
            if matches!(c, ',' | '?' | '.' | '!' | ';' | ':')
                && let Some(t) = out.last_mut()
            {
                t.brk = true;
                if c == ',' {
                    t.comma = true;
                }
            }
        }
    }
    push_word(&mut cur, &mut out);
    out
}

fn words_of(name: &str) -> Vec<String> {
    tokens(name).into_iter().map(|t| t.word).collect()
}

// ---------------------------------------------------------------- spelling

fn lev(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Soundex of the Latin letters of a word; `None` with none.
fn soundex(word: &str) -> Option<String> {
    fn code(c: char) -> u8 {
        match c {
            'b' | 'f' | 'p' | 'v' => b'1',
            'c' | 'g' | 'j' | 'k' | 'q' | 's' | 'x' | 'z' => b'2',
            'd' | 't' => b'3',
            'l' => b'4',
            'm' | 'n' => b'5',
            'r' => b'6',
            _ => 0,
        }
    }
    let letters: Vec<char> = word
        .chars()
        .filter(char::is_ascii_alphabetic)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let first = *letters.first()?;
    let mut out = String::new();
    out.push(first.to_ascii_uppercase());
    let mut last = code(first);
    for &c in &letters[1..] {
        let k = code(c);
        if k != 0 && k != last {
            out.push(k as char);
        }
        // h and w don't separate two letters of one code; vowels do.
        if c != 'h' && c != 'w' {
            last = k;
        }
        if out.len() == 4 {
            break;
        }
    }
    while out.len() < 4 {
        out.push('0');
    }
    Some(out)
}

/// A heard word matches a name word if it's equal, or within edit distance 1
/// for names of 4+ letters. Names of 5+ letters also match on the same phonetic
/// key, but only within edit distance 2: Soundex puts Juno, John, June and Jean
/// together, so short names never match phonetically.
fn word_matches(heard: &str, name: &str) -> bool {
    if heard == name {
        return true;
    }
    let n = name.chars().count();
    let d = lev(heard, name);
    if n >= 4 && d <= 1 {
        return true;
    }
    n >= 5 && d <= 2 && soundex(heard).is_some() && soundex(heard) == soundex(name)
}

// ---------------------------------------------------------------- the matcher

/// A person in the huddle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Person {
    pub key: String,
    /// Profile name, else membership name.
    pub name: String,
}

/// An assistant connected to the huddle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Assistant {
    /// Whose assistant it is.
    pub person: String,
    /// The chosen name; none means "<profile name>'s assistant".
    pub chosen: Option<String>,
}

/// Who is in the room: the people and their connected assistants.
#[derive(Debug, Clone, Copy)]
pub(super) struct Room<'a> {
    pub people: &'a [Person],
    pub assistants: &'a [Assistant],
}

impl Room<'_> {
    fn name_of(&self, key: &str) -> Option<&str> {
        self.people
            .iter()
            .find(|p| p.key == key)
            .map(|p| p.name.as_str())
    }

    /// Every word of every person's name.
    fn person_words(&self) -> BTreeSet<String> {
        self.people.iter().flat_map(|p| words_of(&p.name)).collect()
    }
}

/// What a line called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Called {
    /// Nobody.
    No,
    /// This person's assistant.
    One(String),
    /// Several assistants answer to the form and none is the speaker's: no
    /// look runs and a note goes on the canvas.
    Clash {
        /// The form as spoken, for the note.
        form: String,
        /// Their people, sorted.
        people: Vec<String>,
    },
}

/// "Two assistants answer to Juno. Say whose."
pub(super) fn clash_note(form: &str) -> String {
    let title: Vec<String> = form
        .split(' ')
        .map(|w| {
            let mut cs = w.chars();
            match cs.next() {
                Some(f) => f.to_uppercase().chain(cs).collect(),
                None => String::new(),
            }
        })
        .collect();
    format!("Two assistants answer to {}. Say whose.", title.join(" "))
}

/// Every matching host sees the clash; the one whose person id sorts first
/// writes the note.
pub(super) fn clash_writer(people: &[String]) -> Option<&str> {
    people.iter().map(String::as_str).min()
}

fn is_filler(w: &str) -> bool {
    matches!(w, "hey" | "ok" | "okay" | "so" | "and")
}

/// Words that address the assistant when they follow its name.
const ADDRESS: &[&str] = &[
    "you", "can", "could", "do", "does", "what", "who", "find", "check", "put", "add", "please",
    "where", "how", "tell", "show", "look", "list",
];
/// Third-person verbs: the line is about the assistant, not to it.
const THIRD: &[&str] = &["said", "says", "thinks", "is", "was", "told", "posted"];

/// Rule 5: spoken to, not spoken about.
fn spoken_to(toks: &[Tok], end: usize) -> bool {
    if end >= toks.len() || toks[end - 1].brk {
        return true;
    }
    let next = toks[end].word.as_str();
    if next == "is" && toks.get(end + 1).is_some_and(|t| t.word == "there") {
        return true;
    }
    if THIRD.contains(&next) {
        return false;
    }
    ADDRESS.contains(&next)
}

/// Rule 4: within the first three words, right after a filler, or at the
/// line's end after a comma ("…right, Juno?").
fn address_position(toks: &[Tok], start: usize, end: usize) -> bool {
    if start <= 2 {
        return true;
    }
    let prev = &toks[start - 1];
    is_filler(&prev.word) || (end >= toks.len() && prev.comma)
}

/// The person-name forms to try: the first word, and the whole name.
fn person_forms(name: &str) -> Vec<Vec<String>> {
    let all = words_of(name);
    let mut forms = Vec::new();
    if let Some(first) = all.first() {
        forms.push(vec![first.clone()]);
    }
    if all.len() > 1 {
        forms.push(all);
    }
    forms
}

/// `words` as consecutive tokens from `i`; the end if all match.
fn match_seq(toks: &[Tok], i: usize, words: &[String]) -> Option<usize> {
    if words.is_empty() || i + words.len() > toks.len() {
        return None;
    }
    for (k, w) in words.iter().enumerate() {
        if !word_matches(&toks[i + k].word, w) {
            return None;
        }
    }
    Some(i + words.len())
}

/// A person's name with the possessive on its last word ("Addam's",
/// "Addam Lee's"). Whisper often drops the apostrophe ("Addams").
fn match_possessive(toks: &[Tok], i: usize, words: &[String]) -> Option<usize> {
    let last = words.len().checked_sub(1)?;
    if i + words.len() > toks.len() {
        return None;
    }
    for (k, w) in words.iter().enumerate() {
        let t = &toks[i + k];
        let ok = if k < last || t.poss {
            word_matches(&t.word, w)
        } else if let Some(stem) = t.word.strip_suffix('s') {
            word_matches(stem, w)
        } else {
            false
        };
        if !ok {
            return None;
        }
    }
    Some(i + words.len())
}

/// "assistant", loosely.
fn match_assistant_word(toks: &[Tok], i: usize) -> Option<usize> {
    toks.get(i)
        .filter(|t| word_matches(&t.word, "assistant"))
        .map(|_| i + 1)
}

/// How specific a form is. Lower wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    /// "Stephen's Juno": a person before a chosen name, to pick one of two.
    Qualified,
    /// "Addam's assistant", "my assistant".
    Assistant,
    /// "Juno".
    Bare,
}

#[derive(Debug, Clone, Copy)]
struct Found {
    rank: Rank,
    start: usize,
    end: usize,
}

fn find_call(toks: &[Tok], a: &Assistant, speaker: &str, room: &Room<'_>) -> Option<Found> {
    let persons = room.person_words();
    let chosen = a.chosen.as_deref().map(words_of).filter(|w| !w.is_empty());
    let forms = room
        .name_of(&a.person)
        .map(person_forms)
        .unwrap_or_default();
    // A word that is the name of a person in the huddle never matches an
    // assistant's chosen name.
    let chosen_at = |i: usize| -> Option<usize> {
        let words = chosen.as_ref()?;
        let end = match_seq(toks, i, words)?;
        if toks[i..end].iter().any(|t| persons.contains(&t.word)) {
            return None;
        }
        Some(end)
    };
    let mut best: Option<Found> = None;
    let mut offer = |rank: Rank, start: usize, end: usize| {
        if !address_position(toks, start, end) || !spoken_to(toks, end) {
            return;
        }
        let f = Found { rank, start, end };
        if best.is_none_or(|b| (f.rank, f.start) < (b.rank, b.start)) {
            best = Some(f);
        }
    };
    for i in 0..toks.len() {
        for pw in &forms {
            if let Some(j) = match_possessive(toks, i, pw) {
                if let Some(end) = chosen_at(j) {
                    offer(Rank::Qualified, i, end);
                }
                if let Some(end) = match_assistant_word(toks, j) {
                    offer(Rank::Assistant, i, end);
                }
            }
        }
        if speaker == a.person
            && toks[i].word == "my"
            && let Some(end) = match_assistant_word(toks, i + 1)
        {
            offer(Rank::Assistant, i, end);
        }
        if let Some(end) = chosen_at(i) {
            offer(Rank::Bare, i, end);
        }
    }
    best
}

/// Which assistant a line calls (§5). `speaker` is the person key who said it.
///
/// Forms: the chosen name, "<person>'s assistant", "my assistant" from its own
/// person only, and "<person>'s <chosen>" (tried first). Several matching: the
/// speaker's own wins; otherwise [`Called::Clash`].
pub(super) fn call_target(text: &str, speaker: &str, room: &Room<'_>) -> Called {
    let toks = tokens(text);
    let mut hits: Vec<(&Assistant, Found)> = room
        .assistants
        .iter()
        .filter_map(|a| find_call(&toks, a, speaker, room).map(|f| (a, f)))
        .collect();
    let Some(best) = hits.iter().map(|(_, f)| f.rank).min() else {
        return Called::No;
    };
    hits.retain(|(_, f)| f.rank == best);
    if let Some((a, _)) = hits.iter().find(|(a, _)| a.person == speaker) {
        return Called::One(a.person.clone());
    }
    if hits.len() == 1 {
        return Called::One(hits[0].0.person.clone());
    }
    let (_, f) = hits[0];
    let form = toks[f.start..f.end]
        .iter()
        .map(Tok::plain)
        .collect::<Vec<_>>()
        .join(" ");
    let mut people: Vec<String> = hits.iter().map(|(a, _)| a.person.clone()).collect();
    people.sort();
    people.dedup();
    Called::Clash { form, people }
}

// ---------------------------------------------------------------- L2

/// Tag questions, which ask nothing of the room.
const TAGS: &[&str] = &[
    "right",
    "yeah",
    "ok",
    "okay",
    "make sense",
    "does that make sense",
    "you know",
    "no",
    "huh",
    "eh",
];
/// Starts of a request.
const STEMS: &[&str] = &[
    "do we",
    "does anyone",
    "does someone",
    "can someone",
    "can anyone",
    "can we",
    "could we",
    "could someone",
    "let's make",
    "let's add",
    "who's",
    "what's",
    "who has",
    "is there",
    "are there",
    "should we",
    "where's",
];

/// A tag question: the line's last `?` clause is after a comma and is one of
/// [`TAGS`] ("…ship it Friday, right?").
fn ends_in_tag(toks: &[Tok]) -> bool {
    let Some(last) = toks.last() else {
        return false;
    };
    for n in 1..=3usize.min(toks.len()) {
        let tail = &toks[toks.len() - n..];
        let phrase = tail.iter().map(Tok::plain).collect::<Vec<_>>().join(" ");
        let starts_clause = toks.len() == n || toks[toks.len() - n - 1].comma;
        if TAGS.contains(&phrase.as_str()) && starts_clause && last.brk {
            return true;
        }
    }
    false
}

/// A person in the huddle named to their face: "Addam, did you push it?". The
/// name has no possessive and is at the start (or after a filler), has a comma
/// after it, or ends the line after a comma.
fn names_a_person(toks: &[Tok], speaker: &str, room: &Room<'_>) -> bool {
    for p in room.people.iter().filter(|p| p.key != speaker) {
        for form in person_forms(&p.name) {
            for i in 0..toks.len() {
                let Some(end) = match_seq(toks, i, &form) else {
                    continue;
                };
                if toks[i..end].iter().any(|t| t.poss) {
                    continue;
                }
                let at_start = i == 0 || is_filler(&toks[i - 1].word);
                let comma_after = toks[end - 1].comma;
                let at_end = end >= toks.len() && i > 0 && toks[i - 1].comma;
                if at_start || comma_after || at_end {
                    return true;
                }
            }
        }
    }
    false
}

/// Whether the speaker's own assistant should look at this line (L2): it ends
/// in `?` or starts with a request stem, and isn't a skip: under 4 words, a tag
/// question, or a question to a person named in the huddle.
pub(super) fn own_asks(text: &str, speaker: &str, room: &Room<'_>) -> bool {
    let toks = tokens(text);
    if toks.len() < 4 {
        return false;
    }
    let plain = toks.iter().map(Tok::plain).collect::<Vec<_>>().join(" ");
    let question = text.trim_end().ends_with('?');
    let stem = STEMS.iter().any(|s| {
        plain
            .strip_prefix(s)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
    });
    if !question && !stem {
        return false;
    }
    if ends_in_tag(&toks) {
        return false;
    }
    !names_a_person(&toks, speaker, room)
}

// ---------------------------------------------------------------- L3b

/// An open card in the packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Card {
    /// Its ref, "#412".
    pub id: String,
    pub title: String,
    /// The people who hold it: its holders and, for an agent holding it, that
    /// agent's person.
    pub holders: Vec<String>,
}

/// Words that say nothing about which card a question is about.
const COMMON: &[&str] = &[
    "the",
    "and",
    "for",
    "with",
    "that",
    "this",
    "these",
    "those",
    "from",
    "have",
    "has",
    "had",
    "are",
    "was",
    "were",
    "been",
    "will",
    "would",
    "could",
    "should",
    "can",
    "does",
    "did",
    "not",
    "any",
    "anyone",
    "someone",
    "somebody",
    "everyone",
    "who",
    "what",
    "when",
    "where",
    "which",
    "why",
    "how",
    "our",
    "your",
    "their",
    "its",
    "his",
    "her",
    "them",
    "they",
    "you",
    "about",
    "there",
    "here",
    "just",
    "like",
    "need",
    "needs",
    "want",
    "got",
    "get",
    "make",
    "made",
    "let",
    "know",
    "knows",
    "think",
    "working",
    "work",
    "works",
    "worked",
    "doing",
    "done",
    "going",
    "task",
    "tasks",
    "card",
    "cards",
    "thing",
    "things",
    "issue",
    "issues",
    "bug",
    "bugs",
    "fix",
    "fixes",
    "status",
    "update",
    "anything",
    "something",
    "yet",
    "still",
    "now",
    "then",
    "also",
    "than",
    "into",
    "onto",
    "over",
    "out",
    "one",
    "all",
    "but",
    "who's",
    "ticket",
    "project",
    "please",
    "maybe",
    "much",
    "many",
    "some",
    "take",
    "took",
    "taking",
    "look",
    "looking",
    "ask",
    "asked",
    "tell",
    "told",
    "say",
    "said",
    "own",
    "owns",
    "owner",
    "handle",
    "handling",
    "per",
    "via",
    "huddle",
    "call",
    "team",
    "whether",
    "being",
];

/// A light stem so "signing" and "signs" meet.
fn stem(w: &str) -> String {
    let w = w.trim_end_matches('s');
    for suf in ["ing", "ed"] {
        if w.len() > suf.len() + 3
            && let Some(s) = w.strip_suffix(suf)
        {
            return s.to_string();
        }
    }
    w.to_string()
}

fn significant(text: &str) -> BTreeSet<String> {
    tokens(text)
        .into_iter()
        .map(|t| t.word)
        .filter(|w| w.chars().count() >= 3 && !COMMON.contains(&w.as_str()))
        .map(|w| stem(&w))
        .collect()
}

/// The people who should hear a routed question, and the cards that matched
/// for each: "words of the question, minus common words, against card
/// titles". A card matches when the question names its ref, or shares as many
/// significant words with its title as the question has, up to 2. A question
/// of only common words matches nothing.
pub(super) fn route_question(text: &str, cards: &[Card]) -> BTreeMap<String, Vec<String>> {
    let q = significant(text);
    let digits: BTreeSet<String> = tokens(text).into_iter().map(|t| t.word).collect();
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for c in cards {
        let by_ref = {
            let n = c.id.trim_start_matches('#');
            !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && digits.contains(n)
        };
        let by_words = !q.is_empty() && {
            let t = significant(&c.title);
            q.intersection(&t).count() >= q.len().min(2)
        };
        if by_ref || by_words {
            for h in &c.holders {
                let list = out.entry(h.clone()).or_default();
                if !list.contains(&c.id) {
                    list.push(c.id.clone());
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------- blocks

/// The kinds of canvas item (§8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Question,
    Answer,
    Decision,
    Task,
    Note,
    Summary,
}

impl Kind {
    /// The kind a row or block names.
    pub(super) fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "question" => Kind::Question,
            "answer" => Kind::Answer,
            "decision" => Kind::Decision,
            "task" => Kind::Task,
            "note" => Kind::Note,
            "summary" => Kind::Summary,
            _ => return None,
        })
    }

    /// The word a `call.item` row carries.
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Kind::Question => "question",
            Kind::Answer => "answer",
            Kind::Decision => "decision",
            Kind::Task => "task",
            Kind::Note => "note",
            Kind::Summary => "summary",
        }
    }
}

impl Layer {
    /// What a `call.item` row's `why` says of the look that wrote it.
    pub(super) fn why(self) -> &'static str {
        match self {
            Layer::Named => "name",
            Layer::Ask => "ask",
            Layer::Own => "own",
            Layer::Digest => "digest",
            Layer::Routed => "routed",
            Layer::End => "end",
        }
    }
}

/// Only a `question` item fires a look (L3b); nothing else on the canvas
/// triggers anything, so assistants can't set each other off in a loop.
pub(super) fn triggers_look(kind: Kind) -> bool {
    kind == Kind::Question
}

/// One block of a look's reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Block {
    Item {
        kind: Kind,
        reply: Option<String>,
        refs: Vec<String>,
        text: String,
    },
    Title(String),
}

/// What a look may write. Drops, in order:
/// - an L3b look's blocks that aren't `answer` items;
/// - a title or summary from an assistant that isn't the chair;
/// - `refs` on anything but a summary;
/// - empty items.
///
/// Then clips each item to [`ITEM_CHARS`] and keeps at most
/// [`ITEMS_PER_LOOK`] items.
pub(super) fn admit(layer: Layer, chair: bool, blocks: Vec<Block>) -> Vec<Block> {
    let mut items = 0;
    let mut out = Vec::new();
    for b in blocks {
        match b {
            Block::Title(t) => {
                let t = t.trim().to_string();
                if chair && layer != Layer::Routed && !t.is_empty() {
                    out.push(Block::Title(t));
                }
            }
            Block::Item {
                kind,
                reply,
                refs,
                text,
            } => {
                if layer == Layer::Routed && kind != Kind::Answer {
                    continue;
                }
                if kind == Kind::Summary && !chair {
                    continue;
                }
                let text: String = text.trim().chars().take(ITEM_CHARS).collect();
                if text.is_empty() || items >= ITEMS_PER_LOOK {
                    continue;
                }
                items += 1;
                let refs = if kind == Kind::Summary {
                    refs
                } else {
                    Vec::new()
                };
                out.push(Block::Item {
                    kind,
                    reply,
                    refs,
                    text,
                });
            }
        }
    }
    out
}

fn normalise(text: &str) -> String {
    tokens(text)
        .iter()
        .map(Tok::plain)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether the canvas already has an item with this text (the Core drops it).
pub(super) fn is_duplicate<'a>(existing: impl IntoIterator<Item = &'a str>, text: &str) -> bool {
    let n = normalise(text);
    existing.into_iter().any(|e| normalise(e) == n)
}

// ---------------------------------------------------------------- the layers

/// What woke a look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Layer {
    /// L1: a line called the assistant.
    Named,
    /// L1: its own person used Ask on the page.
    Ask,
    /// L2: its own person asked.
    Own,
    /// L3: its own digest.
    Digest,
    /// L3b: a routed question.
    Routed,
    /// L4: the end.
    End,
}

/// A transcript line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Line {
    pub id: String,
    /// The speaker's person key.
    pub speaker: String,
    pub text: String,
    pub began: SystemTime,
    pub ended: SystemTime,
}

/// A canvas item that landed, for L3b.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Item {
    pub id: String,
    pub kind: Kind,
    pub text: String,
    /// The person who signed it.
    pub by: String,
    /// Written by that person's assistant.
    pub assistant: bool,
    pub at: SystemTime,
}

/// A look to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Look {
    pub layer: Layer,
    /// Who started it (L1): the line's speaker, or the Ask user.
    pub caller: Option<String>,
    /// The line that woke it (and the one it waited for).
    pub lines: Vec<String>,
    /// L3b: the question item.
    pub question: Option<String>,
    /// L3b: the matching cards.
    pub cards: Vec<String>,
    /// Started by a line right after a look of the same speaker (§6, 8 s).
    pub follow_up: bool,
}

impl Look {
    fn new(layer: Layer) -> Look {
        Look {
            layer,
            caller: None,
            lines: Vec::new(),
            question: None,
            cards: Vec::new(),
            follow_up: false,
        }
    }
}

/// What happened.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Step {
    /// A look to start now.
    pub look: Option<Look>,
    /// Write the canvas note "<name> has done its looks for this huddle." Once.
    pub cap_note: bool,
}

impl Step {
    fn merge(&mut self, other: Step) {
        if self.look.is_none() {
            self.look = other.look;
        } else {
            debug_assert!(other.look.is_none(), "two looks at once");
        }
        self.cap_note |= other.cap_note;
    }
}

/// An assistant's settings (§12). They only turn things off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Settings {
    /// "Let my assistant add notes from what I say": off stops L3.
    pub digest: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { digest: true }
    }
}

#[derive(Debug)]
struct Hold {
    look: Look,
    speaker: String,
    since: SystemTime,
}

#[derive(Debug)]
struct Recent {
    look: Look,
    speaker: String,
    at: SystemTime,
}

/// One assistant's state for one huddle: the rules of §6 for the person it
/// belongs to. Each host runs one per assistant it serves, fed the same lines
/// from the rows, so the layers need no coordination.
#[derive(Debug)]
pub(super) struct Brain {
    me: String,
    settings: Settings,
    looks: u32,
    cap_noted: bool,
    busy: Option<SystemTime>,
    queued: Vec<Look>,
    hold: Option<Hold>,
    recent: Option<Recent>,
    last_l2: Option<SystemTime>,
    last_l3: Option<SystemTime>,
    last_l1: BTreeMap<String, SystemTime>,
    /// Its own person's talk since its last look.
    talk: Duration,
    last_line_at: Option<SystemTime>,
    last_speaker: Option<String>,
    routed: BTreeSet<String>,
    ended: bool,
}

impl Brain {
    pub(super) fn new(me: &str, settings: Settings) -> Brain {
        Brain {
            me: me.to_string(),
            settings,
            looks: 0,
            cap_noted: false,
            busy: None,
            queued: Vec::new(),
            hold: None,
            recent: None,
            last_l2: None,
            last_l3: None,
            last_l1: BTreeMap::new(),
            talk: Duration::ZERO,
            last_line_at: None,
            last_speaker: None,
            routed: BTreeSet::new(),
            ended: false,
        }
    }

    pub(super) fn looks(&self) -> u32 {
        self.looks
    }

    /// "Let my assistant add notes from what I say" changed (card #1053):
    /// off stops L3 from now on; the other layers are not touched.
    pub(super) fn set_digest(&mut self, on: bool) {
        self.settings.digest = on;
    }

    /// The cap is reached (the roster says so).
    pub(super) fn capped(&self) -> bool {
        self.looks >= CAP
    }

    pub(super) fn busy(&self) -> bool {
        self.busy.is_some()
    }

    /// L4 ran and its look is over (written, empty or failed): there is
    /// nothing more to wait for (card #1053).
    pub(super) fn end_finished(&self) -> bool {
        self.ended && self.busy.is_none() && self.queued.is_empty()
    }

    /// The next time `tick` has something to do, if any: the hold's end, or a
    /// digest's gap.
    pub(super) fn wake_at(&self) -> Option<SystemTime> {
        let hold = self.hold.as_ref().map(|h| h.since + HOLD);
        let gap = (self.talk >= L3_TALK && self.settings.digest)
            .then(|| self.last_line_at.map(|t| t + L3_GAP))
            .flatten();
        let busy = self.busy.map(|t| t + BUSY_MAX);
        [hold, gap, busy].into_iter().flatten().min()
    }

    /// A live line (the feed passes each once). Stale lines trigger nothing.
    pub(super) fn line(&mut self, now: SystemTime, line: &Line, room: &Room<'_>) -> Step {
        let mut step = Step::default();
        if since(now, line.ended) > FRESH {
            return step;
        }
        let own = line.speaker == self.me;
        let prev = self.last_speaker.replace(line.speaker.clone());
        self.last_line_at = Some(now);
        if own {
            self.talk += since(line.ended, line.began).min(LINE_TALK_MAX);
        }
        // The next line from whoever we held for is the rest of the sentence.
        let mut continued = false;
        if self
            .hold
            .as_ref()
            .is_some_and(|h| h.speaker == line.speaker)
        {
            let mut h = self.hold.take().expect("checked");
            h.look.lines.push(line.id.clone());
            step.merge(self.launch(now, h.look));
            continued = true;
        }
        if !continued {
            step.merge(self.triggered(now, line, room));
        }
        let changed = prev.as_deref() == Some(self.me.as_str()) && !own;
        step.merge(self.digest(now, changed));
        step
    }

    fn triggered(&mut self, now: SystemTime, line: &Line, room: &Room<'_>) -> Step {
        let mut look = match call_target(&line.text, &line.speaker, room) {
            Called::One(p) if p == self.me => {
                let mut l = Look::new(Layer::Named);
                l.caller = Some(line.speaker.clone());
                Some(l)
            }
            // Called someone else, or a clash: not ours.
            Called::One(_) | Called::Clash { .. } => return Step::default(),
            Called::No => (line.speaker == self.me && own_asks(&line.text, &self.me, room))
                .then(|| Look::new(Layer::Own)),
        };
        if look.is_none() {
            // Right after a look: the rest of a sentence from a slow speaker.
            if let Some(r) = self
                .recent
                .take_if(|r| r.speaker == line.speaker && since(now, r.at) <= FOLLOW_UP)
            {
                let mut l = r.look;
                l.lines = vec![line.id.clone()];
                l.follow_up = true;
                // It is already the rest of a sentence: no second hold.
                return self.accept(now, l, None);
            }
            return Step::default();
        }
        let l = look.as_mut().expect("checked");
        l.lines.push(line.id.clone());
        self.accept(now, look.expect("checked"), Some(line))
    }

    /// Throttles and the cap, then the hold (for a line) or the launch.
    fn accept(&mut self, now: SystemTime, look: Look, line: Option<&Line>) -> Step {
        let mine = look.caller.as_deref() == Some(self.me.as_str());
        match look.layer {
            Layer::Named | Layer::Ask => {
                if !mine && !look.follow_up {
                    let caller = look.caller.clone().unwrap_or_default();
                    if self
                        .last_l1
                        .get(&caller)
                        .is_some_and(|t| since(now, *t) < L1_OTHERS_EVERY)
                    {
                        return Step::default();
                    }
                    if self.capped() {
                        return self.cap_note();
                    }
                    self.last_l1.insert(caller, now);
                }
            }
            Layer::Own => {
                if self.capped() {
                    return Step::default();
                }
                if !look.follow_up {
                    if self.last_l2.is_some_and(|t| since(now, t) < L2_EVERY) {
                        return Step::default();
                    }
                    self.last_l2 = Some(now);
                }
            }
            Layer::Digest | Layer::Routed | Layer::End => {}
        }
        let Some(line) = line else {
            return self.launch(now, look);
        };
        // Hold for the rest of the sentence. A hold still open for another
        // speaker goes first.
        let mut step = Step::default();
        if let Some(h) = self.hold.take() {
            step.merge(self.launch(now, h.look));
        }
        self.hold = Some(Hold {
            look,
            speaker: line.speaker.clone(),
            since: now,
        });
        step
    }

    fn cap_note(&mut self) -> Step {
        let note = !self.cap_noted;
        self.cap_noted = true;
        Step {
            look: None,
            cap_note: note,
        }
    }

    /// Start a look, or queue it behind the running one.
    fn launch(&mut self, now: SystemTime, look: Look) -> Step {
        let mine = look.caller.as_deref() == Some(self.me.as_str());
        let allowed = match look.layer {
            Layer::End => true,
            Layer::Named | Layer::Ask => mine || !self.capped(),
            _ => !self.capped(),
        };
        if !allowed {
            return if matches!(look.layer, Layer::Named | Layer::Ask) {
                self.cap_note()
            } else {
                Step::default()
            };
        }
        if self.busy.is_some() {
            if self.queued.len() < QUEUE_MAX {
                self.queued.push(look);
            }
            return Step::default();
        }
        self.looks += 1;
        self.busy = Some(now);
        self.talk = Duration::ZERO;
        if look.layer == Layer::Digest {
            self.last_l3 = Some(now);
        }
        if matches!(look.layer, Layer::Named | Layer::Own)
            && !look.follow_up
            && let Some(speaker) = look.caller.clone().or_else(|| Some(self.me.clone()))
        {
            self.recent = Some(Recent {
                look: look.clone(),
                speaker,
                at: now,
            });
        }
        Step {
            look: Some(look),
            cap_note: false,
        }
    }

    /// The runner finished (or gave up on) the look. Starts the next queued.
    pub(super) fn done(&mut self, now: SystemTime) -> Step {
        self.busy = None;
        if self.queued.is_empty() {
            return Step::default();
        }
        let next = self.queued.remove(0);
        self.launch(now, next)
    }

    /// Time passing: the hold's end, a digest's gap, a look that never ended.
    pub(super) fn tick(&mut self, now: SystemTime) -> Step {
        let mut step = Step::default();
        if self.busy.is_some_and(|t| since(now, t) > BUSY_MAX) {
            step.merge(self.done(now));
        }
        if self
            .hold
            .as_ref()
            .is_some_and(|h| since(now, h.since) >= HOLD)
        {
            let h = self.hold.take().expect("checked");
            step.merge(self.launch(now, h.look));
        }
        step.merge(self.digest(now, false));
        step
    }

    /// L3: its own person has spoken at least [`L3_TALK`] since the last look,
    /// then a natural gap ([`L3_GAP`] with no line, or the speaker changes).
    /// Forced at [`L3_FORCE`] of talk. Never if the person hasn't spoken, never
    /// within [`L3_APART`] of the last digest, never behind a running look.
    fn digest(&mut self, now: SystemTime, speaker_changed: bool) -> Step {
        if !self.settings.digest
            || self.busy.is_some()
            || self.capped()
            || self.talk < L3_TALK
            || self.last_l3.is_some_and(|t| since(now, t) < L3_APART)
        {
            return Step::default();
        }
        let gap = self.last_line_at.is_some_and(|t| since(now, t) >= L3_GAP);
        if self.talk >= L3_FORCE || speaker_changed || gap {
            return self.launch(now, Look::new(Layer::Digest));
        }
        Step::default()
    }

    /// The chair's one look for a title (card #1053): a digest-shaped look
    /// that may carry a `<<title>>`.
    pub(super) fn title(&mut self, now: SystemTime) -> Step {
        self.launch(now, Look::new(Layer::Digest))
    }

    /// The person used Ask on the page (L1, no hold, not throttled).
    pub(super) fn ask(&mut self, now: SystemTime) -> Step {
        self.ask_about(now, None)
    }

    /// [`Brain::ask`] for the question item `item` (card #1051): the look
    /// carries its id, so a look that waits behind another still knows what
    /// it was asked.
    pub(super) fn ask_about(&mut self, now: SystemTime, item: Option<&str>) -> Step {
        let mut l = Look::new(Layer::Ask);
        l.caller = Some(self.me.clone());
        l.question = item.map(str::to_string);
        self.accept(now, l, None)
    }

    /// A `question` item landed (L3b). `cards` is the packet's open cards;
    /// holders are people. A look only for a person who holds a matching card,
    /// once per question, never for the assistant's own output.
    pub(super) fn question(&mut self, now: SystemTime, item: &Item, cards: &[Card]) -> Step {
        if !triggers_look(item.kind)
            || since(now, item.at) > FRESH
            || !self.routed.insert(item.id.clone())
            || self.capped()
            || (item.by == self.me && item.assistant)
        {
            return Step::default();
        }
        let Some(mine) = route_question(&item.text, cards).remove(&self.me) else {
            return Step::default();
        };
        let mut l = Look::new(Layer::Routed);
        l.caller = Some(item.by.clone());
        l.question = Some(item.id.clone());
        l.cards = mine;
        self.launch(now, l)
    }

    /// L4: once per huddle, for the assistant that is due to write the summary
    /// (the chair, or the next one, [`due_writer`]).
    pub(super) fn end(
        &mut self,
        now: SystemTime,
        over: bool,
        has_summary: bool,
        due: bool,
    ) -> Step {
        if !over || has_summary || !due || self.ended {
            return Step::default();
        }
        self.ended = true;
        self.launch(now, Look::new(Layer::End))
    }
}

// ---------------------------------------------------------------- the chair

/// One assistant's seat, from the huddle's `call.assist` rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Seat {
    pub person: String,
    /// The time of its first `call.assist on` row for this huddle.
    pub first_on: SystemTime,
    /// Its newest row says `on`.
    pub on: bool,
    /// Its newest row says `chair: true`.
    pub chair: bool,
    /// How long its host has been away in presence.
    pub away: Duration,
    /// Its person is still in the call.
    pub present: bool,
    /// "Let my assistant chair huddles I start" is not off.
    pub will_chair: bool,
}

impl Seat {
    fn connected(&self) -> bool {
        self.on && self.away < AWAY_AFTER
    }
}

/// Connected seats in the order of their first `on` row (ties by person id).
fn order(seats: &[Seat]) -> Vec<&Seat> {
    let mut v: Vec<&Seat> = seats.iter().filter(|s| s.connected()).collect();
    v.sort_by(|a, b| (a.first_on, &a.person).cmp(&(b.first_on, &b.person)));
    v
}

/// Who chairs, and who must stand down.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Chairing {
    pub chair: Option<String>,
    /// Connected seats that claim the chair and shouldn't: their hosts write
    /// `chair: false`.
    pub stand_down: Vec<String>,
}

/// The chair, from rows (§6). The starter's assistant takes it if connected.
/// Otherwise (or when its person leaves while others are still in the call, or
/// it's off, or its host has been away for [`AWAY_AFTER`]) the next connected
/// assistant by first `on` row takes it. Two that both claim it settle on the
/// earlier. A chair that is already settled stays: it isn't handed back to the
/// starter's assistant when that one returns.
pub(super) fn chair(starter: &str, seats: &[Seat]) -> Chairing {
    let nobody_present = !seats.iter().any(|s| s.present);
    let eligible: Vec<&Seat> = order(seats)
        .into_iter()
        .filter(|s| s.will_chair && (s.present || nobody_present))
        .collect();
    let holder = eligible
        .iter()
        .find(|s| s.chair)
        .or_else(|| eligible.iter().find(|s| s.person == starter))
        .or_else(|| eligible.first())
        .map(|s| s.person.clone());
    let stand_down = order(seats)
        .into_iter()
        .filter(|s| s.chair && Some(&s.person) != holder.as_ref())
        .map(|s| s.person.clone())
        .collect();
    Chairing {
        chair: holder,
        stand_down,
    }
}

/// Who writes something the chair owes (the summary at L4, or the end mark
/// after a crash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Writer {
    /// This person's assistant's host.
    Person(String),
    /// No assistant is connected: any member device that judges the huddle over
    /// (end marks only, after [`WRITE_WAIT`]).
    AnyDevice,
    /// No assistant is connected and nobody else may write it.
    Nobody,
}

/// The chair writes it; if it isn't there [`WRITE_WAIT`] after `over`, the next
/// connected assistant in order does, and one more [`WRITE_WAIT`] later the one
/// after. `end_mark` allows any device when no assistant is connected at all.
pub(super) fn due_writer(
    starter: &str,
    seats: &[Seat],
    over_for: Duration,
    end_mark: bool,
) -> Writer {
    let connected = order(seats);
    if connected.is_empty() {
        return if end_mark && over_for >= WRITE_WAIT {
            Writer::AnyDevice
        } else {
            Writer::Nobody
        };
    }
    let steps = (over_for.as_secs() / WRITE_WAIT.as_secs()) as usize;
    let chair = chair(starter, seats).chair;
    let first = chair.clone().unwrap_or_else(|| connected[0].person.clone());
    if steps == 0 {
        return Writer::Person(first);
    }
    let rest: Vec<&&Seat> = connected.iter().filter(|s| s.person != first).collect();
    match rest.get(steps - 1).or(rest.last()) {
        Some(s) => Writer::Person(s.person.clone()),
        None => Writer::Person(first),
    }
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_000_000;

    fn at(s: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(T0 + s)
    }

    fn at_ms(ms: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(T0) + Duration::from_millis(ms)
    }

    fn person(key: &str, name: &str) -> Person {
        Person {
            key: key.into(),
            name: name.into(),
        }
    }

    fn assistant(person: &str, chosen: Option<&str>) -> Assistant {
        Assistant {
            person: person.into(),
            chosen: chosen.map(Into::into),
        }
    }

    struct Fixture {
        people: Vec<Person>,
        assistants: Vec<Assistant>,
    }

    impl Fixture {
        fn room(&self) -> Room<'_> {
            Room {
                people: &self.people,
                assistants: &self.assistants,
            }
        }
        fn call(&self, speaker: &str, text: &str) -> Called {
            call_target(text, speaker, &self.room())
        }
    }

    /// Addam (assistant Juno), Stephen (no chosen name).
    fn duo() -> Fixture {
        Fixture {
            people: vec![person("addam", "Addam Lee"), person("stephen", "Stephen")],
            assistants: vec![assistant("addam", Some("Juno")), assistant("stephen", None)],
        }
    }

    fn one(p: &str) -> Called {
        Called::One(p.into())
    }

    // ------------------------------------------------ the matcher: the forms

    #[test]
    fn chosen_name_calls_its_assistant() {
        let f = duo();
        assert_eq!(f.call("stephen", "Juno, who's on signing?"), one("addam"));
        assert_eq!(
            f.call("stephen", "Hey Juno what's the status"),
            one("addam")
        );
        assert_eq!(f.call("stephen", "ok so Juno can you check"), one("addam"));
        assert_eq!(f.call("stephen", "Is that right, Juno?"), one("addam"));
    }

    #[test]
    fn person_assistant_calls_it_with_or_without_a_chosen_name() {
        let f = duo();
        // Addam has a chosen name; the person form still works.
        assert_eq!(
            f.call("stephen", "Addam's assistant, find the card"),
            one("addam")
        );
        assert_eq!(
            f.call("stephen", "Addam Lee's assistant, find the card"),
            one("addam")
        );
        // Stephen has none: the person form is the only one.
        assert_eq!(
            f.call("addam", "Stephen's assistant, who has signing?"),
            one("stephen")
        );
        // Whisper drops the apostrophe.
        assert_eq!(
            f.call("addam", "Stephens assistant can you check this"),
            one("stephen")
        );
    }

    #[test]
    fn my_assistant_only_from_its_own_person() {
        let f = duo();
        assert_eq!(
            f.call("stephen", "My assistant, what's on signing?"),
            one("stephen")
        );
        assert_eq!(
            f.call("addam", "My assistant, what's on signing?"),
            one("addam")
        );
        // Nobody else's.
        let f = Fixture {
            people: f.people.clone(),
            assistants: vec![assistant("addam", Some("Juno"))],
        };
        assert_eq!(
            f.call("stephen", "My assistant, what's on signing?"),
            Called::No
        );
    }

    #[test]
    fn a_person_before_a_chosen_name_picks_one_of_two() {
        let f = Fixture {
            people: vec![
                person("addam", "Addam"),
                person("stephen", "Stephen"),
                person("blake", "Blake"),
            ],
            assistants: vec![
                assistant("addam", Some("Juno")),
                assistant("stephen", Some("Juno")),
            ],
        };
        assert_eq!(
            f.call("blake", "Stephen's Juno, can you check"),
            one("stephen")
        );
        assert_eq!(f.call("blake", "Addam's Juno, can you check"), one("addam"));
        // Bare, from someone neither's: a clash.
        assert!(matches!(
            f.call("blake", "Juno, can you check"),
            Called::Clash { .. }
        ));
    }

    #[test]
    fn near_spellings_match() {
        let f = duo();
        // Whisper writes "Adam's assistant" for "Addam's assistant".
        assert_eq!(
            f.call("stephen", "Adam's assistant, find the card"),
            one("addam")
        );
        assert_eq!(
            f.call("stephen", "Hey Juna, who's on signing?"),
            one("addam")
        );
        // A two-word name is two adjacent words.
        let g = Fixture {
            people: vec![person("addam", "Addam"), person("stephen", "Stephen")],
            assistants: vec![assistant("addam", Some("Office Pal"))],
        };
        assert_eq!(
            g.call("stephen", "Office Pal, who's on signing?"),
            one("addam")
        );
        assert_eq!(g.call("stephen", "Office, who's on signing?"), Called::No);
    }

    #[test]
    fn short_names_never_match_phonetically() {
        let f = duo();
        // Soundex puts Juno, John, June and Jean together; a 4-letter name
        // must not.
        assert_eq!(f.call("stephen", "Jean, can you check"), Called::No);
        assert_eq!(f.call("stephen", "John, can you check this"), Called::No);
        // A 5+ letter name does, within edit distance 2.
        let g = Fixture {
            people: vec![person("addam", "Addam"), person("stephen", "Stephen")],
            assistants: vec![assistant("addam", Some("Marcus"))],
        };
        assert_eq!(g.call("stephen", "Markus, can you check"), one("addam"));
        assert_eq!(g.call("stephen", "Margaret, can you check"), Called::No);
    }

    // ------------------------------------------------ the matcher: position

    #[test]
    fn position_is_strict() {
        let f = duo();
        assert_eq!(
            f.call("stephen", "I was telling Addam's assistant yesterday"),
            Called::No
        );
        assert_eq!(
            f.call("stephen", "We should ask Juno what to do about it"),
            Called::No
        );
        // Right after a filler, deep in the line.
        assert_eq!(
            f.call(
                "stephen",
                "We have been going back and forth and Juno what's the status"
            ),
            one("addam")
        );
        // At the line's end after a comma.
        assert_eq!(
            f.call("stephen", "We should make that the default, right, Juno?"),
            one("addam")
        );
        // At the line's end with no comma: a mention.
        assert_eq!(
            f.call("stephen", "We should make that the default with Juno"),
            Called::No
        );
    }

    #[test]
    fn spoken_to_not_spoken_about() {
        let f = duo();
        assert_eq!(
            f.call("addam", "My assistant said we should ship it"),
            Called::No
        );
        assert_eq!(f.call("addam", "I'll ask my assistant later"), Called::No);
        assert_eq!(f.call("stephen", "Juno said we should ship it"), Called::No);
        assert_eq!(f.call("stephen", "Juno is looking at it"), Called::No);
        assert_eq!(f.call("stephen", "Juno thinks it is done"), Called::No);
        assert_eq!(f.call("stephen", "Juno posted the answer"), Called::No);
        assert_eq!(f.call("stephen", "Juno was right about it"), Called::No);
        assert_eq!(f.call("stephen", "Juno told me it was done"), Called::No);
        // But "is there" is a question to it.
        assert_eq!(
            f.call("stephen", "Juno is there anyone on signing"),
            one("addam")
        );
        // A comma or a question mark is enough.
        assert_eq!(f.call("stephen", "Juno, one more thing"), one("addam"));
        assert_eq!(f.call("stephen", "Juno?"), one("addam"));
        for w in [
            "you", "can", "could", "do", "what", "who", "find", "check", "put", "add",
        ] {
            assert_eq!(
                f.call("stephen", &format!("Juno {w} the thing")),
                one("addam"),
                "{w}"
            );
        }
    }

    #[test]
    fn lookalikes_are_not_called() {
        let f = duo();
        assert_eq!(f.call("stephen", "June third is the deadline"), Called::No);
        // John beside a Juno.
        assert_eq!(f.call("stephen", "John, can you push it"), Called::No);
        let g = Fixture {
            people: vec![
                person("addam", "Addam"),
                person("stephen", "Stephen"),
                person("john", "John"),
            ],
            assistants: vec![assistant("addam", Some("Juno"))],
        };
        assert_eq!(g.call("stephen", "John, can you push it"), Called::No);
        assert_eq!(g.call("stephen", "June third is the deadline"), Called::No);
        assert_eq!(g.call("stephen", "Juno, can you push it"), one("addam"));
    }

    #[test]
    fn a_chosen_name_that_is_a_person_never_matches() {
        let f = Fixture {
            people: vec![
                person("addam", "Addam"),
                person("stephen", "Stephen"),
                person("june", "June"),
            ],
            assistants: vec![assistant("addam", Some("Juno"))],
        };
        // June is in the room: "June, can you…" is June.
        assert_eq!(f.call("stephen", "June, can you push it"), Called::No);
        assert_eq!(f.call("stephen", "Juno, can you push it"), one("addam"));
    }

    // ------------------------------------------------ the matcher: collisions

    #[test]
    fn two_assistants_one_form_the_speakers_own_wins() {
        let f = Fixture {
            people: vec![
                person("a", "Stephen Horton"),
                person("b", "Stephen Park"),
                person("c", "Blake"),
            ],
            assistants: vec![assistant("a", None), assistant("b", None)],
        };
        assert_eq!(f.call("a", "Stephen's assistant, find the card"), one("a"));
        assert_eq!(f.call("b", "Stephen's assistant, find the card"), one("b"));
        // Neither is Blake's: nobody looks, and a note says whose to say.
        let Called::Clash { form, people } = f.call("c", "Stephen's assistant, find the card")
        else {
            panic!("expected a clash");
        };
        assert_eq!(people, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(
            clash_note(&form),
            "Two assistants answer to Stephen's Assistant. Say whose."
        );
        // The whole name tells them apart.
        assert_eq!(
            f.call("c", "Stephen Park's assistant, find the card"),
            one("b")
        );
    }

    #[test]
    fn two_junos_clash_and_the_first_person_id_writes_the_note() {
        let f = Fixture {
            people: vec![
                person("b", "Addam"),
                person("a", "Stephen"),
                person("c", "Blake"),
            ],
            assistants: vec![assistant("b", Some("Juno")), assistant("a", Some("Juno"))],
        };
        let Called::Clash { form, people } = f.call("c", "Juno, who's on signing?") else {
            panic!("expected a clash");
        };
        assert_eq!(
            clash_note(&form),
            "Two assistants answer to Juno. Say whose."
        );
        assert_eq!(clash_writer(&people), Some("a"));
        // The speaker's own wins.
        assert_eq!(f.call("a", "Juno, who's on signing?"), one("a"));
    }

    // ------------------------------------------------ L2

    #[test]
    fn l2_asks_by_question_mark_or_stem() {
        let f = duo();
        let r = f.room();
        assert!(own_asks(
            "do we have anyone working on signing?",
            "stephen",
            &r
        ));
        assert!(own_asks("can someone take the signing card", "stephen", &r));
        assert!(own_asks("let's make a card for signing", "stephen", &r));
        assert!(own_asks("who's on signing right now", "stephen", &r));
        assert!(own_asks("what's the plan for signing", "stephen", &r));
        // Not a question.
        assert!(!own_asks("we should ship signing on Friday", "stephen", &r));
    }

    #[test]
    fn l2_skips() {
        let f = duo();
        let r = f.room();
        // Under 4 words.
        assert!(!own_asks("who's on it?", "stephen", &r));
        assert!(!own_asks("what's up?", "stephen", &r));
        // Tag questions.
        assert!(!own_asks(
            "we should ship it on Friday, right?",
            "stephen",
            &r
        ));
        assert!(!own_asks(
            "that is the plan for signing, make sense?",
            "stephen",
            &r
        ));
        assert!(!own_asks(
            "we are doing signing first, yeah?",
            "stephen",
            &r
        ));
        assert!(!own_asks(
            "this fixes the problem we saw, okay?",
            "stephen",
            &r
        ));
        // A question to a person in the huddle.
        assert!(!own_asks(
            "Addam, did you push the signing fix?",
            "stephen",
            &r
        ));
        assert!(!own_asks(
            "hey Addam did you push the signing fix?",
            "stephen",
            &r
        ));
        assert!(!own_asks(
            "did you push the signing fix, Addam?",
            "stephen",
            &r
        ));
        // A person named in the middle isn't addressed.
        assert!(own_asks(
            "can someone ask Addam's team about signing?",
            "stephen",
            &r
        ));
        // A question that merely contains "right" isn't a tag.
        assert!(own_asks("is the signing key the right one?", "stephen", &r));
    }

    // ------------------------------------------------ the layers

    fn line(id: &str, speaker: &str, text: &str, ended: u64) -> Line {
        Line {
            id: id.into(),
            speaker: speaker.into(),
            text: text.into(),
            began: at(ended.saturating_sub(2)),
            ended: at(ended),
        }
    }

    fn lay(step: &Step) -> Option<Layer> {
        step.look.as_ref().map(|l| l.layer)
    }

    /// Feeds a line, then lets the hold run out.
    fn feed(b: &mut Brain, f: &Fixture, l: &Line, now: u64) -> Step {
        let mut s = b.line(at(now), l, &f.room());
        if s.look.is_none() {
            s.merge(b.tick(at(now + 2)));
        }
        s
    }

    #[test]
    fn a_name_call_looks_after_the_hold() {
        let f = duo();
        let mut b = Brain::new("addam", Settings::default());
        let l = line("l1", "stephen", "Juno, who's on signing?", 10);
        let s = b.line(at(10), &l, &f.room());
        assert_eq!(s, Step::default(), "held for the rest of the sentence");
        assert_eq!(b.wake_at(), Some(at_ms(10_000 + 1500)));
        assert_eq!(b.tick(at_ms(11_000)), Step::default());
        let s = b.tick(at_ms(11_500));
        let look = s.look.expect("look after 1.5 s");
        assert_eq!(look.layer, Layer::Named);
        assert_eq!(look.caller.as_deref(), Some("stephen"));
        assert_eq!(look.lines, vec!["l1".to_string()]);
    }

    #[test]
    fn the_next_line_from_the_speaker_ends_the_hold_early() {
        let f = duo();
        let mut b = Brain::new("addam", Settings::default());
        assert_eq!(
            b.line(
                at(10),
                &line("l1", "stephen", "Juno, do we have anyone", 10),
                &f.room()
            ),
            Step::default()
        );
        // Someone else talking doesn't end it.
        assert_eq!(
            b.line(at(10), &line("x", "addam", "mm", 10), &f.room())
                .look,
            None
        );
        let s = b.line(
            at(11),
            &line("l2", "stephen", "working on signing?", 11),
            &f.room(),
        );
        let look = s.look.expect("the continuation starts the look");
        assert_eq!(look.lines, vec!["l1".to_string(), "l2".to_string()]);
        // And the continuation line didn't start a second one.
        assert_eq!(b.tick(at(20)), Step::default());
    }

    #[test]
    fn a_mid_line_mention_and_other_peoples_questions_wake_nothing() {
        let f = duo();
        let mut b = Brain::new("addam", Settings::default());
        for (i, (who, text)) in [
            ("stephen", "I was telling Addam's assistant yesterday"),
            ("stephen", "do we have anyone working on signing?"),
            ("stephen", "My assistant said we should"),
        ]
        .into_iter()
        .enumerate()
        {
            let s = feed(
                &mut b,
                &f,
                &line(&format!("l{i}"), who, text, 10 + 20 * i as u64),
                10 + 20 * i as u64,
            );
            assert_eq!(s, Step::default(), "{text}");
        }
    }

    #[test]
    fn l2_looks_for_its_own_person_only_and_not_when_the_line_calls_someone() {
        let f = duo();
        let mut b = Brain::new("addam", Settings::default());
        let s = feed(
            &mut b,
            &f,
            &line("l1", "addam", "do we have anyone working on signing?", 10),
            10,
        );
        assert_eq!(lay(&s), Some(Layer::Own));
        let mut c = Brain::new("addam", Settings::default());
        // The line calls Stephen's assistant: not an L2 for Addam's.
        let s = feed(
            &mut c,
            &f,
            &line(
                "l1",
                "addam",
                "Stephen's assistant, do we have anyone on signing?",
                10,
            ),
            10,
        );
        assert_eq!(s, Step::default());
        // And Stephen's does get an L1, Addam being someone else.
        let mut d = Brain::new("stephen", Settings::default());
        let s = feed(
            &mut d,
            &f,
            &line(
                "l1",
                "addam",
                "Stephen's assistant, do we have anyone on signing?",
                10,
            ),
            10,
        );
        assert_eq!(lay(&s), Some(Layer::Named));
    }

    #[test]
    fn one_look_at_a_time_and_the_rest_wait() {
        let f = duo();
        let mut b = Brain::new("addam", Settings::default());
        let s = feed(
            &mut b,
            &f,
            &line("l1", "addam", "Juno, who's on signing?", 10),
            10,
        );
        assert_eq!(lay(&s), Some(Layer::Named));
        assert!(b.busy());
        // A second call lands during the look: nothing starts.
        let s = feed(
            &mut b,
            &f,
            &line("l2", "addam", "Juno, what about Windows?", 14),
            14,
        );
        assert_eq!(s, Step::default());
        // The look ends: the waiting one starts.
        let s = b.done(at(20));
        let look = s.look.expect("queued look");
        assert_eq!(look.lines, vec!["l2".to_string()]);
        assert_eq!(b.done(at(25)), Step::default());
    }

    #[test]
    fn others_calls_are_throttled_per_caller_the_owners_are_not() {
        let f = Fixture {
            people: vec![
                person("addam", "Addam"),
                person("stephen", "Stephen"),
                person("blake", "Blake"),
            ],
            assistants: vec![assistant("addam", Some("Juno"))],
        };
        let mut b = Brain::new("addam", Settings::default());
        let mut t = 10;
        // Stephen calls; the look runs and ends.
        let s = feed(
            &mut b,
            &f,
            &line("a", "stephen", "Juno, who's on signing?", t),
            t,
        );
        assert_eq!(lay(&s), Some(Layer::Named));
        b.done(at(t + 4));
        // Stephen again within 15 s: dropped.
        t += 8;
        let s = feed(
            &mut b,
            &f,
            &line("b", "stephen", "Juno, who's on signing?", t),
            t,
        );
        assert_eq!(s, Step::default());
        assert_eq!(b.looks(), 1);
        // Blake is a different caller: allowed.
        t += 1;
        let s = feed(
            &mut b,
            &f,
            &line("c", "blake", "Juno, who's on signing?", t),
            t,
        );
        assert_eq!(lay(&s), Some(Layer::Named));
        b.done(at(t + 4));
        // Stephen after 15 s: allowed.
        t += 20;
        let s = feed(
            &mut b,
            &f,
            &line("d", "stephen", "Juno, who's on signing?", t),
            t,
        );
        assert_eq!(lay(&s), Some(Layer::Named));
        b.done(at(t + 4));
        // Addam's own are never throttled by this rule (a 2 s gap, past the
        // follow-up window, one look at a time aside).
        for i in 0..3 {
            t += 12;
            let s = feed(
                &mut b,
                &f,
                &line(&format!("o{i}"), "addam", "Juno, any news on signing?", t),
                t,
            );
            assert_eq!(lay(&s), Some(Layer::Named), "own call {i}");
            b.done(at(t + 3));
        }
    }

    #[test]
    fn l2_is_throttled() {
        let f = duo();
        let mut b = Brain::new("addam", Settings::default());
        let s = feed(
            &mut b,
            &f,
            &line("a", "addam", "do we have anyone on signing?", 10),
            10,
        );
        assert_eq!(lay(&s), Some(Layer::Own));
        b.done(at(14));
        // Past the 8 s follow-up window, but inside 15 s of the last L2.
        let s = feed(
            &mut b,
            &f,
            &line("b", "addam", "is there anyone on Windows?", 20),
            20,
        );
        assert_eq!(s, Step::default());
        let s = feed(
            &mut b,
            &f,
            &line("c", "addam", "is there anyone on Linux?", 30),
            30,
        );
        assert_eq!(lay(&s), Some(Layer::Own));
    }

    #[test]
    fn a_line_right_after_a_look_from_the_same_speaker_starts_one_more() {
        let f = duo();
        let mut b = Brain::new("addam", Settings::default());
        let s = feed(
            &mut b,
            &f,
            &line("a", "stephen", "Juno, do we have anyone", 10),
            10,
        );
        assert_eq!(lay(&s), Some(Layer::Named));
        b.done(at(13));
        // A phone speaker's rest of the sentence, 5 s later.
        let s = b.line(
            at(15),
            &line("b", "stephen", "working on signing?", 15),
            &f.room(),
        );
        let look = s.look.expect("one more");
        assert!(look.follow_up);
        assert_eq!(look.caller.as_deref(), Some("stephen"));
        b.done(at(18));
        // Only one more.
        let s = feed(&mut b, &f, &line("c", "stephen", "and Windows?", 19), 19);
        assert_eq!(s, Step::default());
        // Another speaker doesn't start one.
        let mut c = Brain::new("addam", Settings::default());
        feed(
            &mut c,
            &f,
            &line("a", "stephen", "Juno, do we have anyone", 10),
            10,
        );
        c.done(at(13));
        let s = c.line(
            at(15),
            &line("b", "addam", "working on signing?", 15),
            &f.room(),
        );
        assert_eq!(s.look, None);
        // Nor does one past 8 s.
        let mut d = Brain::new("addam", Settings::default());
        feed(
            &mut d,
            &f,
            &line("a", "stephen", "Juno, do we have anyone", 10),
            10,
        );
        d.done(at(13));
        let s = d.line(
            at(40),
            &line("b", "stephen", "working on signing?", 40),
            &f.room(),
        );
        assert_eq!(s.look, None);
    }

    #[test]
    fn stale_lines_trigger_nothing() {
        let f = duo();
        let mut b = Brain::new("addam", Settings::default());
        // A host back from sleep: rows from five minutes ago, all at once.
        for (i, text) in [
            "Juno, who's on signing?",
            "do we have anyone working on signing?",
        ]
        .into_iter()
        .enumerate()
        {
            let who = if i == 0 { "stephen" } else { "addam" };
            let l = line(&format!("o{i}"), who, text, 10);
            assert_eq!(b.line(at(310), &l, &f.room()), Step::default());
        }
        assert_eq!(b.tick(at(320)), Step::default());
        assert_eq!(b.looks(), 0);
        // 60 s old still counts as fresh; 61 s doesn't.
        let l = line("edge", "stephen", "Juno, who's on signing?", 10);
        b.line(at(70), &l, &f.room());
        assert!(b.tick(at(72)).look.is_some());
        let mut c = Brain::new("addam", Settings::default());
        c.line(at(71), &l, &f.room());
        assert_eq!(c.tick(at(73)).look, None);
    }

    #[test]
    fn ask_is_a_look_for_its_own_person() {
        let mut b = Brain::new("addam", Settings::default());
        let s = b.ask(at(5));
        let look = s.look.expect("ask looks at once");
        assert_eq!(look.layer, Layer::Ask);
        assert_eq!(look.caller.as_deref(), Some("addam"));
    }

    #[test]
    fn the_cap() {
        let f = duo();
        let mut b = Brain::new("addam", Settings::default());
        let mut t = 100;
        // Reach the cap with Ask.
        for _ in 0..CAP {
            assert!(b.ask(at(t)).look.is_some());
            b.done(at(t));
            t += 1;
        }
        assert!(b.capped());
        // L2 stops.
        let s = feed(
            &mut b,
            &f,
            &line("a", "addam", "do we have anyone on signing?", t),
            t,
        );
        assert_eq!(s, Step::default());
        // Digests stop.
        let mut talk = line(
            "d",
            "addam",
            "we are going to do the signing work now",
            t + 30,
        );
        talk.began = at(t);
        assert_eq!(b.line(at(t + 30), &talk, &f.room()), Step::default());
        assert_eq!(b.tick(at(t + 40)), Step::default());
        // Routed questions stop.
        let item = Item {
            id: "q".into(),
            kind: Kind::Question,
            text: "who is on signing".into(),
            by: "stephen".into(),
            assistant: false,
            at: at(t + 40),
        };
        let cards = [Card {
            id: "#1".into(),
            title: "Windows signing".into(),
            holders: vec!["addam".into()],
        }];
        assert_eq!(b.question(at(t + 40), &item, &cards), Step::default());
        // Another person's call gets one note, then nothing.
        t += 50;
        let s = feed(
            &mut b,
            &f,
            &line("n1", "stephen", "Juno, who's on signing?", t),
            t,
        );
        assert!(s.look.is_none() && s.cap_note);
        t += 20;
        let s = feed(
            &mut b,
            &f,
            &line("n2", "stephen", "Juno, who's on signing?", t),
            t,
        );
        assert_eq!(s, Step::default());
        // The owner's own call goes on.
        t += 20;
        let s = feed(
            &mut b,
            &f,
            &line("n3", "addam", "Juno, who's on signing?", t),
            t,
        );
        assert_eq!(lay(&s), Some(Layer::Named));
        // L4 isn't capped.
        b.done(at(t + 3));
        assert_eq!(lay(&b.end(at(t + 4), true, false, true)), Some(Layer::End));
    }

    // ------------------------------------------------ L3

    fn talks(id: &str, from: u64, to: u64) -> Line {
        Line {
            id: id.into(),
            speaker: "addam".into(),
            text: "we are going to do the signing work now".into(),
            began: at(from),
            ended: at(to),
        }
    }

    #[test]
    fn digest_needs_talk_then_a_gap() {
        let f = duo();
        let mut b = Brain::new("addam", Settings::default());
        // 10 s of talk: not enough.
        assert_eq!(
            b.line(at(10), &talks("a", 0, 10), &f.room()),
            Step::default()
        );
        assert_eq!(b.tick(at(20)), Step::default());
        // 10 s more: 20 s of talk, but no gap yet.
        assert_eq!(
            b.line(at(21), &talks("b", 11, 21), &f.room()),
            Step::default()
        );
        assert_eq!(b.tick(at(23)), Step::default());
        // 4 s with no line.
        let s = b.tick(at(25));
        assert_eq!(lay(&s), Some(Layer::Digest));
        // Talk since the last look starts again.
        b.done(at(30));
        assert_eq!(b.tick(at(60)), Step::default());
    }

    #[test]
    fn digest_on_a_speaker_change_and_forced_at_a_minute() {
        let f = duo();
        let mut b = Brain::new("addam", Settings::default());
        assert_eq!(
            b.line(at(10), &talks("a", 0, 10), &f.room()),
            Step::default()
        );
        assert_eq!(
            b.line(at(20), &talks("b", 11, 20), &f.room()),
            Step::default()
        );
        // Stephen speaks: the speaker changes.
        let s = b.line(
            at(21),
            &line("c", "stephen", "yes let's do that", 21),
            &f.room(),
        );
        assert_eq!(lay(&s), Some(Layer::Digest));

        let mut c = Brain::new("addam", Settings::default());
        let mut last = Step::default();
        let mut t = 0;
        // Back to back lines, never a 4 s gap.
        while t < 80 && last.look.is_none() {
            last = c.line(at(t + 10), &talks(&format!("t{t}"), t, t + 10), &f.room());
            t += 10;
        }
        assert_eq!(lay(&last), Some(Layer::Digest));
        assert!(t <= 60, "forced by 60 s of talk, was {t}");
    }

    #[test]
    fn digest_never_without_its_person_talking_and_not_when_off() {
        let f = duo();
        let mut b = Brain::new("addam", Settings::default());
        // Stephen talks for a minute; Addam's assistant digests nothing.
        for i in 0..8 {
            let l = Line {
                id: format!("s{i}"),
                speaker: "stephen".into(),
                text: "we are going to do the signing work now".into(),
                began: at(i * 10),
                ended: at(i * 10 + 10),
            };
            assert_eq!(b.line(at(i * 10 + 10), &l, &f.room()), Step::default());
        }
        assert_eq!(b.tick(at(120)), Step::default());
        // Off: still none.
        let mut c = Brain::new("addam", Settings { digest: false });
        c.line(at(30), &talks("a", 0, 30), &f.room());
        assert_eq!(c.tick(at(40)), Step::default());
    }

    #[test]
    fn digests_are_thirty_seconds_apart() {
        let f = duo();
        let mut b = Brain::new("addam", Settings::default());
        b.line(at(20), &talks("a", 0, 20), &f.room());
        assert_eq!(lay(&b.tick(at(25))), Some(Layer::Digest));
        b.done(at(26));
        // 20 s more of talk and a gap, but only 8 s after the last digest...
        b.line(at(46), &talks("b", 26, 46), &f.room());
        assert_eq!(lay(&b.tick(at(50))), None, "26 s after the last digest");
        assert_eq!(lay(&b.tick(at(56))), Some(Layer::Digest));
    }

    // ------------------------------------------------ L3b

    fn cards() -> Vec<Card> {
        vec![
            Card {
                id: "#412".into(),
                title: "Windows signing".into(),
                holders: vec!["addam".into()],
            },
            Card {
                id: "#413".into(),
                title: "Android release build".into(),
                holders: vec!["stephen".into(), "blake".into()],
            },
            Card {
                id: "#414".into(),
                title: "Do the thing".into(),
                holders: vec!["blake".into()],
            },
        ]
    }

    #[test]
    fn the_question_match_picks_the_holders() {
        let m = route_question("do we have anyone working on Windows signing?", &cards());
        assert_eq!(m.keys().collect::<Vec<_>>(), vec!["addam"]);
        assert_eq!(m["addam"], vec!["#412".to_string()]);
        // Several holders of one card each get it.
        let m = route_question("who has the android build", &cards());
        assert_eq!(m.keys().collect::<Vec<_>>(), vec!["blake", "stephen"]);
        // Word forms meet.
        let m = route_question("is anyone signing the windows installer", &cards());
        assert!(m.contains_key("addam"));
        // A ref names the card.
        let m = route_question("what's going on with 413", &cards());
        assert_eq!(m.keys().collect::<Vec<_>>(), vec!["blake", "stephen"]);
    }

    #[test]
    fn the_question_match_ignores_common_words() {
        // Only common words in common with "Do the thing".
        assert!(route_question("do we have anyone working on the thing?", &cards()).is_empty());
        assert!(route_question("who is working on it", &cards()).is_empty());
        // One shared real word isn't enough when the question has two.
        assert!(route_question("who knows the Windows installer", &cards()).is_empty());
        // But is when it's the only real word.
        assert!(!route_question("who has Windows?", &cards()).is_empty());
    }

    fn question(id: &str, text: &str, by: &str, assistant: bool, t: u64) -> Item {
        Item {
            id: id.into(),
            kind: Kind::Question,
            text: text.into(),
            by: by.into(),
            assistant,
            at: at(t),
        }
    }

    #[test]
    fn a_routed_look_once_per_question_for_a_holder() {
        let q = question("q1", "who is on Windows signing?", "stephen", false, 10);
        let mut addam = Brain::new("addam", Settings::default());
        let s = addam.question(at(11), &q, &cards());
        let look = s.look.expect("addam holds #412");
        assert_eq!(look.layer, Layer::Routed);
        assert_eq!(look.question.as_deref(), Some("q1"));
        assert_eq!(look.cards, vec!["#412".to_string()]);
        addam.done(at(14));
        // Once.
        assert_eq!(addam.question(at(15), &q, &cards()), Step::default());
        // Stephen holds no matching card.
        let mut stephen = Brain::new("stephen", Settings::default());
        assert_eq!(stephen.question(at(11), &q, &cards()), Step::default());
    }

    #[test]
    fn only_a_fresh_question_item_not_an_assistants_own_wakes_a_routed_look() {
        let mut b = Brain::new("addam", Settings::default());
        // Old.
        let old = question("q1", "who is on Windows signing?", "stephen", false, 10);
        assert_eq!(b.question(at(200), &old, &cards()), Step::default());
        // Its own assistant's output (the L2 look wrote the question).
        let own = question("q2", "who is on Windows signing?", "addam", true, 10);
        assert_eq!(b.question(at(11), &own, &cards()), Step::default());
        // Its person's own typed question does count.
        let typed = question("q3", "who is on Windows signing?", "addam", false, 10);
        assert!(b.question(at(11), &typed, &cards()).look.is_some());
        // Nothing but a question triggers.
        for k in [
            Kind::Answer,
            Kind::Decision,
            Kind::Task,
            Kind::Note,
            Kind::Summary,
        ] {
            assert!(!triggers_look(k));
        }
        assert!(triggers_look(Kind::Question));
        let mut note = question("n", "Windows signing", "stephen", false, 10);
        note.kind = Kind::Note;
        let mut c = Brain::new("addam", Settings::default());
        assert_eq!(c.question(at(11), &note, &cards()), Step::default());
    }

    #[test]
    fn a_routed_look_writes_only_answers() {
        let blocks = vec![
            Block::Item {
                kind: Kind::Answer,
                reply: Some("q1".into()),
                refs: vec![],
                text: "Addam has #412.".into(),
            },
            Block::Item {
                kind: Kind::Question,
                reply: None,
                refs: vec![],
                text: "And Windows?".into(),
            },
            Block::Item {
                kind: Kind::Task,
                reply: None,
                refs: vec![],
                text: "Do a thing".into(),
            },
            Block::Item {
                kind: Kind::Note,
                reply: None,
                refs: vec![],
                text: "A note".into(),
            },
            Block::Item {
                kind: Kind::Summary,
                reply: None,
                refs: vec!["x".into()],
                text: "Summary".into(),
            },
            Block::Title("A title".into()),
        ];
        let out = admit(Layer::Routed, true, blocks.clone());
        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            Block::Item { kind: Kind::Answer, reply: Some(r), .. } if r == "q1"
        ));
        // The chair's other looks keep their title and summary.
        let out = admit(Layer::End, true, blocks.clone());
        assert_eq!(out.len(), 6);
        // A non-chair's title and summary go.
        let out = admit(Layer::Digest, false, blocks);
        assert_eq!(out.len(), 4);
        assert!(!out.iter().any(|b| matches!(
            b,
            Block::Title(_)
                | Block::Item {
                    kind: Kind::Summary,
                    ..
                }
        )));
    }

    #[test]
    fn blocks_are_bounded() {
        let item = |t: &str| Block::Item {
            kind: Kind::Note,
            reply: None,
            refs: vec!["a".into()],
            text: t.into(),
        };
        let mut blocks: Vec<Block> = (0..9).map(|i| item(&format!("note {i}"))).collect();
        blocks.push(item("   "));
        let out = admit(Layer::Digest, false, blocks);
        assert_eq!(out.len(), ITEMS_PER_LOOK);
        // Refs only on a summary.
        assert!(
            out.iter()
                .all(|b| matches!(b, Block::Item { refs, .. } if refs.is_empty()))
        );
        let long = admit(Layer::Digest, false, vec![item(&"x".repeat(5000))]);
        assert!(matches!(&long[0], Block::Item { text, .. } if text.chars().count() == ITEM_CHARS));
        assert!(is_duplicate(
            ["Addam has #412 on signing."],
            "addam has #412 on signing"
        ));
        assert!(!is_duplicate(["Addam has #412"], "Stephen has #413"));
    }

    // ------------------------------------------------ L4

    #[test]
    fn l4_once_when_over_and_nothing_summarised() {
        let mut b = Brain::new("addam", Settings::default());
        assert_eq!(b.end(at(1), false, false, true), Step::default());
        assert_eq!(b.end(at(1), true, true, true), Step::default());
        assert_eq!(b.end(at(1), true, false, false), Step::default());
        assert_eq!(lay(&b.end(at(1), true, false, true)), Some(Layer::End));
        b.done(at(5));
        assert_eq!(b.end(at(6), true, false, true), Step::default());
    }

    // ------------------------------------------------ the chair

    fn seat(person: &str, first_on: u64) -> Seat {
        Seat {
            person: person.into(),
            first_on: at(first_on),
            on: true,
            chair: false,
            away: Duration::ZERO,
            present: true,
            will_chair: true,
        }
    }

    #[test]
    fn the_starters_assistant_chairs_if_connected() {
        // Blake's connected first, but Addam started it.
        let seats = [seat("blake", 1), seat("addam", 2)];
        assert_eq!(chair("addam", &seats).chair.as_deref(), Some("addam"));
        // The starter's isn't connected: the earliest `on` row.
        let mut s = seats.clone();
        s[1].on = false;
        assert_eq!(chair("addam", &s).chair.as_deref(), Some("blake"));
        // Ties go by person id.
        let s = [seat("zed", 5), seat("blake", 5)];
        assert_eq!(chair("nobody", &s).chair.as_deref(), Some("blake"));
    }

    #[test]
    fn the_chair_passes_on() {
        let mut seats = [seat("addam", 1), seat("blake", 2), seat("stephen", 3)];
        seats[0].chair = true;
        assert_eq!(chair("addam", &seats).chair.as_deref(), Some("addam"));
        // Its person leaves while others are still in the call.
        seats[0].present = false;
        assert_eq!(chair("addam", &seats).chair.as_deref(), Some("blake"));
        // Its host has been away 60 s.
        seats[0].present = true;
        seats[0].away = Duration::from_secs(61);
        assert_eq!(chair("addam", &seats).chair.as_deref(), Some("blake"));
        // Its assistant is off.
        seats[0].away = Duration::ZERO;
        seats[0].on = false;
        assert_eq!(chair("addam", &seats).chair.as_deref(), Some("blake"));
        // "Let my assistant chair huddles I start" is off.
        seats[0].on = true;
        seats[0].will_chair = false;
        assert_eq!(chair("addam", &seats).chair.as_deref(), Some("blake"));
        // A person leaving at the very end, with nobody else present, doesn't
        // hand the role on.
        let mut end = [seat("addam", 1), seat("blake", 2)];
        end[0].chair = true;
        end[0].present = false;
        end[1].present = false;
        assert_eq!(chair("addam", &end).chair.as_deref(), Some("addam"));
    }

    #[test]
    fn two_claimants_settle_on_the_earlier_and_a_settled_chair_stays() {
        let mut seats = [seat("addam", 1), seat("blake", 2)];
        seats[0].chair = true;
        seats[1].chair = true;
        let c = chair("blake", &seats);
        assert_eq!(c.chair.as_deref(), Some("addam"));
        assert_eq!(c.stand_down, vec!["blake".to_string()]);
        // A chair that took over stays when the starter's assistant returns.
        let mut seats = [seat("addam", 1), seat("blake", 2)];
        seats[1].chair = true;
        let c = chair("addam", &seats);
        assert_eq!(c.chair.as_deref(), Some("blake"));
        assert!(c.stand_down.is_empty());
        // No assistant connected: no chair.
        let mut none = [seat("addam", 1)];
        none[0].on = false;
        assert_eq!(chair("addam", &none), Chairing::default());
    }

    #[test]
    fn the_next_assistant_writes_what_the_chair_didnt() {
        let mut seats = [seat("addam", 1), seat("blake", 2), seat("stephen", 3)];
        seats[0].chair = true;
        let min = |m: u64| Duration::from_secs(m * 60);
        assert_eq!(
            due_writer("addam", &seats, min(1), false),
            Writer::Person("addam".into())
        );
        assert_eq!(
            due_writer("addam", &seats, min(3), false),
            Writer::Person("blake".into())
        );
        assert_eq!(
            due_writer("addam", &seats, min(6), false),
            Writer::Person("stephen".into())
        );
        assert_eq!(
            due_writer("addam", &seats, min(60), false),
            Writer::Person("stephen".into())
        );
        // The chair's host is gone: the next, at once.
        seats[0].away = Duration::from_secs(120);
        assert_eq!(
            due_writer("addam", &seats, min(1), false),
            Writer::Person("blake".into())
        );
        // No assistant at all: the end mark falls to any device after 3
        // minutes; a summary to nobody.
        let off: Vec<Seat> = seats
            .iter()
            .cloned()
            .map(|mut s| {
                s.on = false;
                s
            })
            .collect();
        assert_eq!(due_writer("addam", &off, min(1), true), Writer::Nobody);
        assert_eq!(due_writer("addam", &off, min(3), true), Writer::AnyDevice);
        assert_eq!(due_writer("addam", &off, min(3), false), Writer::Nobody);
    }

    // ------------------------------------------------ small things

    #[test]
    fn spelling_helpers() {
        assert_eq!(lev("addam", "adam"), 1);
        assert_eq!(lev("juno", "john"), 3);
        assert_eq!(soundex("Robert"), soundex("Rupert"));
        assert_eq!(soundex("Juno"), soundex("John"));
        assert!(word_matches("adam", "addam"));
        assert!(!word_matches("john", "juno"));
        assert!(
            word_matches("june", "juno"),
            "edit distance 1 on a 4-letter name"
        );
        let t = tokens("Addam's, \"who’s\" on it?");
        assert_eq!(t[0].word, "addam");
        assert!(t[0].poss && t[0].comma);
        assert_eq!(t[1].plain(), "who's");
        assert!(t[3].brk);
    }
}
