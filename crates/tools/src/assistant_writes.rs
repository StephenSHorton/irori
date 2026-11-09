//! What the window's assistant may change (card #987): close or resolve a
//! card or thread, and tick a step, only when the person's own message
//! asks for exactly that. Card #1193 adds two more: message a named agent
//! and comment on a named card, where the words sent are the person's own,
//! kept in the grant, never the model's.
//!
//! The chat reads card text written by agents and others, so the model's
//! judgement can't be what allows a write. Before a run the window turns
//! the person's message, and nothing else, into [`Grant`]s
//! ([`grants_for`]); the API lets a write through only against a matching
//! grant, once ([`Grants::take`]). Text the model read can't add one, and
//! the model's own earlier words are never looked at: `earlier` is only
//! what the person typed.
//!
//! [`grants_for`] is deliberately narrow. A message it doesn't recognise
//! as a plain instruction grants nothing, and the person is asked to say
//! it plainly.

use std::sync::Mutex;

use crate::args::Ty;
use crate::{CLIENT_ASSISTANT, Caller, Inner, decode, err_text, map_err, trim};

/// Closes a card, or resolves a thread, as the person.
pub const TOOL_CLOSE: &str = "assistant_close";
/// Ticks a card's step as the person.
pub const TOOL_TICK: &str = "assistant_tick";
/// Messages a named agent as the person.
pub const TOOL_SEND: &str = "assistant_send";
/// Comments on a named card as the person.
pub const TOOL_COMMENT: &str = "assistant_comment";
/// The tools the assistant may change things with, each only against a
/// [`Grant`]. Beside `ASSISTANT_TOOLS`, which only read.
pub const ASSISTANT_WRITE_TOOLS: [&str; 4] = [TOOL_CLOSE, TOOL_TICK, TOOL_SEND, TOOL_COMMENT];

/// One change the person's message asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grant {
    Close {
        card: String,
    },
    Tick {
        card: String,
        step: String,
    },
    /// `to` is the agent as the person named it, lower case; `text` is the
    /// person's own words.
    Send {
        to: String,
        text: String,
    },
    Comment {
        card: String,
        text: String,
    },
}

#[derive(Default)]
struct State {
    run: u64,
    open: Vec<Grant>,
    changed: Vec<String>,
}

/// The grants of the run going now, and what it changed. The chat runs
/// one question at a time, so one set per Core is enough; [`Grants::begin`]
/// replaces whatever a previous run left.
#[derive(Default)]
pub struct Grants {
    state: Mutex<State>,
}

impl Grants {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A new run starts with these grants; the number is what ends it.
    /// What a run stopped a moment ago changed stays to be said.
    pub fn begin(&self, grants: Vec<Grant>) -> u64 {
        let mut s = self.lock();
        s.run += 1;
        s.open = grants;
        s.run
    }

    /// Spends the grant for this write; `false` when there is none.
    pub fn take(&self, tool: &str, task_id: &str, step_id: &str) -> bool {
        let card = norm_ref(task_id);
        let want = match tool {
            TOOL_CLOSE => Grant::Close { card },
            TOOL_TICK => Grant::Tick {
                card,
                step: step_id.trim().to_lowercase(),
            },
            _ => return false, // send and comment go through take_text
        };
        let mut s = self.lock();
        match s.open.iter().position(|g| *g == want) {
            Some(i) => {
                s.open.remove(i);
                true
            }
            None => false,
        }
    }

    /// Spends the send or comment grant for `target` (an agent's name or a
    /// card's ref) and gives back the target as granted with the person's
    /// words to send: the model never supplies them.
    pub fn take_text(&self, tool: &str, target: &str) -> Option<(String, String)> {
        let mut s = self.lock();
        let i = s.open.iter().position(|g| match (tool, g) {
            (TOOL_SEND, Grant::Send { to, .. }) => *to == norm_agent(target),
            (TOOL_COMMENT, Grant::Comment { card, .. }) => *card == norm_ref(target),
            _ => false,
        })?;
        match s.open.remove(i) {
            Grant::Send { to, text } => Some((to, text)),
            Grant::Comment { card, text } => Some((card, text)),
            _ => None,
        }
    }

    /// Notes a change the run made, as the chat will say it.
    pub fn changed(&self, line: &str) {
        self.lock().changed.push(line.to_string());
    }

    /// Run `run` is over: no grant outlives it, though a stopped run ending
    /// late leaves the next run's alone. Returns what has changed.
    pub fn end(&self, run: u64) -> Vec<String> {
        let mut s = self.lock();
        if s.run == run {
            s.open.clear();
        }
        std::mem::take(&mut s.changed)
    }
}

// ---- reading the person's message -------------------------------------------

/// Any of these and the sentence isn't a plain instruction to do it now.
const HOLD_BACK: &[&str] = &[
    "don't",
    "dont",
    "do",
    "not",
    "never",
    "won't",
    "wont",
    "shouldn't",
    "shouldnt",
    "can't",
    "cant",
    "without",
    "unless",
    "if",
    "when",
    "once",
    "after",
    "before",
    "until",
    "maybe",
    "whether",
    "but",
    "wait",
    "actually",
];
const OPENERS: &[&str] = &[
    "please", "pls", "kindly", "ok", "okay", "yes", "yep", "yeah", "sure", "now", "then", "and",
    "also", "hey", "great", "thanks", "cool", "good", "perfect",
];
const CLOSE_VERBS: &[&str] = &["resolve", "close", "complete", "finish"];
const MARKED: &[&str] = &[
    "resolved",
    "closed",
    "done",
    "complete",
    "completed",
    "finished",
    "ticked",
];
/// Words that may stand between a verb and what it acts on.
const FILLER: &[&str] = &[
    "the", "this", "that", "it", "thread", "card", "task", "ticket", "one", "out", "as", "on",
    "of", "for", "step",
];
const POINTING: &[&str] = &["this", "that", "it"];

/// What the person's `text` asks to change. `earlier` is what they typed
/// before it, oldest first: "close it" means the one they last named.
pub fn grants_for(text: &str, earlier: &[&str]) -> Vec<Grant> {
    // A paste or a list isn't a plain instruction, and a hold-back word
    // anywhere ("Close #5. But only if it passes.") takes the whole message.
    if text.trim().contains('\n') {
        return vec![];
    }
    // The words after "tell ringo" are the person's to say whatever they
    // are, so this reads before the hold-back words do.
    if let Some(g) = message_grant(text) {
        return vec![g];
    }
    let sentences = sentences(text);
    if sentences
        .iter()
        .any(|(w, _)| w.iter().any(|x| HOLD_BACK.contains(&x.as_str())))
    {
        return vec![];
    }
    let mut out: Vec<Grant> = vec![];
    for (i, (words, question)) in sentences.iter().enumerate() {
        // Named earlier: in this message first, then in the ones before.
        let before = || {
            sentences[..i]
                .iter()
                .rev()
                .map(|(w, _)| w.clone())
                .chain(earlier.iter().rev().flat_map(|t| {
                    let mut all: Vec<String> = vec![];
                    for (w, _) in sentences_of(t) {
                        all.extend(w);
                    }
                    Some(all)
                }))
                .find_map(|w| {
                    let mut refs: Vec<String> = vec![];
                    for r in w.iter().filter(|x| is_ref(x)).map(|x| norm_ref(x)) {
                        if !refs.contains(&r) {
                            refs.push(r);
                        }
                    }
                    (!refs.is_empty()).then_some(refs)
                })
                .filter(|r| r.len() == 1)
                .map(|mut r| r.remove(0))
        };
        let Some(found) = sentence(words, *question, &before) else {
            return vec![];
        };
        for g in found {
            if !out.contains(&g) {
                out.push(g);
            }
        }
    }
    out
}

fn sentences(text: &str) -> Vec<(Vec<String>, bool)> {
    sentences_of(text)
}

/// `text`'s sentences as lower-case words, each with whether it was a
/// question.
fn sentences_of(text: &str) -> Vec<(Vec<String>, bool)> {
    let mut out = vec![];
    let mut cur = String::new();
    let flush = |cur: &mut String, q: bool, out: &mut Vec<(Vec<String>, bool)>| {
        let w = words(cur);
        if !w.is_empty() {
            out.push((w, q));
        }
        cur.clear();
    };
    for ch in text.chars() {
        match ch {
            '?' => flush(&mut cur, true, &mut out),
            '.' | '!' | ';' | '\n' => flush(&mut cur, false, &mut out),
            _ => cur.push(ch),
        }
    }
    flush(&mut cur, false, &mut out);
    out
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .replace('\u{2019}', "'")
        .split_whitespace()
        .map(|w| {
            let t = w.trim_matches(|c: char| ",:()[]{}\"*_`".contains(c));
            let t = if t.contains('#') {
                t.strip_suffix("'s").unwrap_or(t)
            } else {
                t
            };
            t.to_string()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// A card's short ref: `#12`, `#12a`, or `tag#12` on a space's board.
fn is_ref(w: &str) -> bool {
    let Some((tag, n)) = w.split_once('#') else {
        return false;
    };
    let letters = n.trim_start_matches(|c: char| c.is_ascii_digit());
    letters.len() < n.len()
        && letters.bytes().all(|b| b.is_ascii_lowercase())
        && tag
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

fn is_number(w: &str) -> bool {
    !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit())
}

/// A ref as grants compare it.
fn norm_ref(s: &str) -> String {
    let t = s.trim().to_lowercase();
    if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) {
        format!("#{t}")
    } else {
        t
    }
}

fn step_word(w: &str) -> Option<String> {
    let n = w.strip_prefix('s')?;
    (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).then(|| w.to_string())
}

fn is_verb(words: &[String], i: usize) -> bool {
    let w = words[i].as_str();
    CLOSE_VERBS.contains(&w)
        || w == "mark"
        || w == "tick"
        || (w == "check" && words.get(i + 1).is_some_and(|n| n == "off"))
}

/// Past the opening words; also whether they asked ("can you").
fn opening(w: &[String]) -> (usize, bool) {
    let at = |i: usize, s: &str| w.get(i).is_some_and(|x| x == s);
    let (mut i, mut asked) = (0, false);
    loop {
        if w.get(i).is_some_and(|x| OPENERS.contains(&x.as_str())) {
            i += 1;
        } else if ["can", "could", "would", "will"].iter().any(|a| at(i, a)) && at(i + 1, "you") {
            i += 2;
            asked = true;
        } else if at(i, "go") && at(i + 1, "ahead") {
            i += 2;
        } else if ((at(i, "i") && (at(i + 1, "want") || at(i + 1, "need")))
            || (at(i, "i'd") && at(i + 1, "like")))
            && at(i + 2, "you")
            && at(i + 3, "to")
        {
            i += 4;
        } else {
            return (i, asked);
        }
    }
}

/// What one sentence asks. `None`: it starts as an instruction but says
/// more than this reader understands, so the whole message grants nothing.
fn sentence(
    w: &[String],
    question: bool,
    before: &dyn Fn() -> Option<String>,
) -> Option<Vec<Grant>> {
    let (mut i, asked) = opening(w);
    if question && !asked {
        return Some(vec![]);
    }
    if !question && let Some(g) = stated(&w[i..], before) {
        return Some(vec![g]);
    }
    let mut out = vec![];
    while i < w.len() && is_verb(w, i) {
        i = clause(w, i, before, &mut out)?;
        // "close #5 and tick s2 on #6": on to the next verb, if any.
        if w.get(i).is_some_and(|x| x == "and") {
            i += 1;
        }
    }
    Some(out)
}

/// "#5 is resolved", "that's resolved", "this thread is resolved".
fn stated(w: &[String], before: &dyn Fn() -> Option<String>) -> Option<Grant> {
    let last = w.last()?;
    if last != "resolved" {
        return None;
    }
    let body = &w[..w.len() - 1];
    let target: Vec<&String> = match body.last().map(String::as_str) {
        Some("is") => body[..body.len() - 1].iter().collect(),
        Some("that's" | "it's") if body.len() == 1 => {
            return before().map(|card| Grant::Close { card });
        }
        _ => return None,
    };
    let refs: Vec<&&String> = target.iter().filter(|x| is_ref(x)).collect();
    if target
        .iter()
        .any(|x| !is_ref(x) && !["this", "that", "it", "thread", "card"].contains(&x.as_str()))
    {
        return None;
    }
    match refs.as_slice() {
        [r] if target.len() == 1 => Some(Grant::Close { card: norm_ref(r) }),
        [] if !target.is_empty() => before().map(|card| Grant::Close { card }),
        _ => None,
    }
}

/// The clause whose verb is at `i`; returns where it stopped. `None`
/// when words are left over that it doesn't understand.
fn clause(
    w: &[String],
    i: usize,
    before: &dyn Fn() -> Option<String>,
    out: &mut Vec<Grant>,
) -> Option<usize> {
    let verb = w[i].as_str();
    let (mark, tick) = (verb == "mark", verb == "tick" || verb == "check");
    let mut j = i + if verb == "check" { 2 } else { 1 };
    let (mut refs, mut steps): (Vec<String>, Vec<String>) = (vec![], vec![]);
    let (mut pointing, mut marked) = (false, false);
    while j < w.len() {
        let x = w[j].as_str();
        if is_ref(x) {
            refs.push(norm_ref(x));
        } else if let Some(s) = step_word(x).filter(|_| tick || mark) {
            steps.push(s);
        } else if x == "step" && (tick || mark) && w.get(j + 1).is_some_and(|n| is_number(n)) {
            // As the card lists them: the card shows no ids.
            steps.push(w[j + 1].clone());
            j += 1;
        } else if x == "and" {
            if w.get(j + 1).is_none() || is_verb(w, j + 1) {
                break;
            }
        } else if mark && MARKED.contains(&x) {
            marked = true;
        } else if FILLER.contains(&x)
            && (refs.is_empty() || ((tick || mark) && ["on", "step", "as", "of"].contains(&x)))
        {
            pointing |= POINTING.contains(&x);
        } else {
            break;
        }
        j += 1;
    }
    // The clause must end here: anything else may change what it means.
    match w.get(j).map(String::as_str) {
        None | Some("and") => {}
        Some("because" | "since") => j = w.len(),
        Some("please" | "now" | "thanks") if j + 1 == w.len() => j = w.len(),
        Some(_) => return None,
    }
    let tick = tick || (mark && !steps.is_empty());
    if mark && !marked {
        return None;
    }
    if refs.is_empty() && (pointing || tick) {
        refs.extend(before());
    }
    if tick {
        // Steps belong to one card.
        if let [card] = refs.as_slice() {
            for step in steps {
                out.push(Grant::Tick {
                    card: card.clone(),
                    step,
                });
            }
        }
    } else {
        for card in refs {
            out.push(Grant::Close { card });
        }
    }
    Some(j)
}

// ---- telling an agent, commenting on a card ----------------------------------

/// The longest body either tool sends.
const MAX_BODY: usize = 4000;
/// Words that stand where an agent's name should but name nobody.
const NOT_A_NAME: &[&str] = &[
    "me",
    "you",
    "him",
    "her",
    "them",
    "it",
    "us",
    "everyone",
    "everybody",
    "all",
    "anyone",
    "someone",
    "somebody",
    "the",
    "a",
    "an",
    "my",
    "your",
    "that",
    "this",
    "there",
    "they",
    "agent",
    "agents",
    "team",
    "to",
    "about",
    "what",
];

/// An agent's name as grants compare it.
fn norm_agent(s: &str) -> String {
    s.trim().trim_start_matches('@').to_lowercase()
}

/// The word with the marks around it taken off, lower case.
fn bare(w: &str) -> String {
    w.to_lowercase()
        .replace('\u{2019}', "'")
        .trim_matches(|c: char| ",.!;:()[]{}\"*_`".contains(c))
        .to_string()
}

/// `w` as a name: no `@`, no closing `,` or `:`; a plain word only.
fn agent_name(w: &str) -> Option<String> {
    let n = w.trim_end_matches([',', ':']).trim_start_matches('@');
    let ok = !n.is_empty()
        && n.chars().count() <= 40
        && n.chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        && !NOT_A_NAME.contains(&n.to_lowercase().as_str());
    ok.then(|| n.to_lowercase())
}

/// "tell ringo the allow prompt is cleared", "comment on #1182: looks
/// good". The message must open with the instruction (after the usual
/// pleasantries) and name one agent or one card; everything after that is
/// the body, as typed. Anything else is not a grant.
fn message_grant(text: &str) -> Option<Grant> {
    // Each word with where it starts, so the body is cut from the text.
    let mut toks: Vec<(usize, &str)> = vec![];
    let mut at = None;
    for (i, ch) in text.char_indices() {
        match (ch.is_whitespace(), at) {
            (false, None) => at = Some(i),
            (true, Some(s)) => {
                toks.push((s, &text[s..i]));
                at = None;
            }
            _ => {}
        }
    }
    if let Some(s) = at {
        toks.push((s, &text[s..]));
    }
    let w: Vec<String> = toks.iter().map(|(_, t)| bare(t)).collect();
    let (i, asked) = opening(&w);
    let is = |k: usize, s: &str| w.get(k).is_some_and(|x| x == s);
    // (what is named, where the body starts, words that may open it)
    enum To {
        Agent(String),
        Card(String),
    }
    let (to, from, lead): (To, usize, &[&str]) = if is(i, "tell") || is(i, "message") {
        let name = agent_name(toks.get(i + 1)?.1)?;
        (
            To::Agent(name),
            i + 2,
            if is(i, "tell") { &["to", "that"] } else { &[] },
        )
    } else if is(i, "let") && is(i + 2, "know") {
        (To::Agent(agent_name(toks.get(i + 1)?.1)?), i + 3, &["that"])
    } else if is(i, "send")
        && (is(i + 2, "a") || is(i + 2, "an"))
        && (is(i + 3, "message") || is(i + 3, "note"))
    {
        (
            To::Agent(agent_name(toks.get(i + 1)?.1)?),
            i + 4,
            &["saying", "that"],
        )
    } else if is(i, "send")
        && is(i + 1, "a")
        && (is(i + 2, "message") || is(i + 2, "note"))
        && is(i + 3, "to")
    {
        (
            To::Agent(agent_name(toks.get(i + 4)?.1)?),
            i + 5,
            &["saying", "that"],
        )
    } else {
        // comment on #12 / add a comment to #12 / post a comment on #12
        let mut k = i;
        if ["add", "post", "leave", "write"].iter().any(|v| is(k, v)) {
            k += 1;
            if is(k, "a") {
                k += 1;
            }
        }
        if !is(k, "comment") || !(is(k + 1, "on") || is(k + 1, "to")) {
            return None;
        }
        let r = bare(toks.get(k + 2)?.1);
        if !is_ref(&r) {
            return None;
        }
        (To::Card(norm_ref(&r)), k + 3, &["saying", "that"])
    };
    let mut from = from;
    // "tell ringo to X": the words after "to" are what is said.
    if lead.iter().any(|l| is(from, l)) && w.get(from + 1).is_some() {
        from += 1;
    }
    let start = toks.get(from)?.0;
    let mut body = text[start..].trim();
    if asked {
        body = body.strip_suffix('?').unwrap_or(body).trim_end();
    }
    if body.is_empty()
        || body.chars().count() > MAX_BODY
        || !body.chars().any(char::is_alphanumeric)
    {
        return None;
    }
    let text = body.to_string();
    Some(match to {
        To::Agent(to) => Grant::Send { to, text },
        To::Card(card) => Grant::Comment { card, text },
    })
}

// ---- the tools --------------------------------------------------------------

const CLOSE_ARGS: &[(&str, Ty)] = &[("task_id", Ty::Str)];
const TICK_ARGS: &[(&str, Ty)] = &[("task_id", Ty::Str), ("step_id", Ty::Str)];

const ONLY_THE_CHAT: &str = "only the window's assistant has this tool.";

/// The card as the chat's line says it: the chat shows its ref with its
/// title, so the line needs no title of its own.
fn named(t: &coord::Task) -> String {
    t.short_ref()
}

const SEND_ARGS: &[(&str, Ty)] = &[("to", Ty::Str), ("text", Ty::Str)];
const COMMENT_ARGS: &[(&str, Ty)] = &[("task_id", Ty::Str), ("text", Ty::Str)];

/// One line of what the person said, for the chat's note.
fn said(t: &str) -> String {
    let one: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= 100 {
        return one;
    }
    let mut c: String = one.chars().take(99).collect();
    c.push('\u{2026}');
    c
}

/// assistant_send: the person's own words to one agent. The API has put
/// the granted words in `text`; the model's never reach here.
pub(crate) fn assistant_send(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    if c.sender.kind != CLIENT_ASSISTANT {
        return err_text(ONLY_THE_CHAT);
    }
    let a = match decode(raw, SEND_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let (to, text) = (a.str("to").to_string(), a.str("text").to_string());
    let body = serde_json::json!({ "to": to, "text": text }).to_string();
    let out = crate::messages::send(s, c, body.as_bytes());
    if out.starts_with("Error") {
        return out;
    }
    format!("Messaged {to}: \u{201c}{}\u{201d}\n{out}", said(&text))
}

/// assistant_comment: the person's own words on one card.
pub(crate) fn assistant_comment(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    if c.sender.kind != CLIENT_ASSISTANT {
        return err_text(ONLY_THE_CHAT);
    }
    let a = match decode(raw, COMMENT_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let text = a.str("text").to_string();
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    let body = serde_json::json!({ "task_id": a.str("task_id"), "text": text }).to_string();
    let out = crate::board::board_comment(s, c, body.as_bytes());
    if out.starts_with("Error") {
        return out;
    }
    format!(
        "Commented on {}: \u{201c}{}\u{201d}\n{out}",
        s.short_of(&id),
        said(&text)
    )
}

/// assistant_close: what the window's Resolve does for a thread, and its
/// close for a card.
pub(crate) fn assistant_close(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    if c.sender.kind != CLIENT_ASSISTANT {
        return err_text(ONLY_THE_CHAT);
    }
    let a = match decode(raw, CLOSE_ARGS) {
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
    let thread = co
        .get_task(&id)
        .is_some_and(|t| t.open_ask().is_some_and(|a| a.thread));
    if thread {
        return match co.resolve_thread(&as_[0], &id) {
            Ok(t) => format!("Resolved thread {}.", named(&t)),
            Err(e) => map_err(e),
        };
    }
    match co.set_task_state(
        &as_[0],
        &id,
        coord::COMPLETED,
        "closed from the Office Assistant",
    ) {
        Ok(t) => format!("Closed {}: it is {} now.", named(&t), t.state),
        Err(e) => map_err(e),
    }
}

/// assistant_tick: ticks one step, nothing else about it.
pub(crate) fn assistant_tick(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    if c.sender.kind != CLIENT_ASSISTANT {
        return err_text(ONLY_THE_CHAT);
    }
    let a = match decode(raw, TICK_ARGS) {
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
    let named_step = trim(a.str("step_id")).to_lowercase();
    let Some(card) = co.get_task(&id) else {
        return err_text("no such card.");
    };
    // "2" is the second as the card lists them; "s2" is that step's id.
    let found = if is_number(&named_step) {
        named_step
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|n| card.steps.get(n))
    } else {
        card.steps.iter().find(|st| st.id == named_step)
    };
    let Some(step) = found else {
        return err_text(&format!("{} has no such step.", card.short_ref()));
    };
    if step.done {
        // Nothing changes, so the chat has no change to say.
        return err_text(&format!(
            "step {} on {} is already ticked.",
            step.id,
            card.short_ref()
        ));
    }
    let sid = step.id.clone();
    let update = coord::StepUpdate {
        done: Some(true),
        doing: None,
    };
    match co.set_step(&as_[0], &id, &sid, update) {
        Ok(t) => {
            let (d, n) = t.progress();
            format!("Ticked step {sid} on {} ({d} of {n} done).", named(&t))
        }
        Err(e) => map_err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(c: &str) -> Grant {
        Grant::Close { card: c.into() }
    }
    fn tick(c: &str, s: &str) -> Grant {
        Grant::Tick {
            card: c.into(),
            step: s.into(),
        }
    }

    #[test]
    fn plain_asks_grant_what_they_name() {
        for (text, want) in [
            ("resolve #983", vec![close("#983")]),
            ("Please close #5.", vec![close("#5")]),
            ("can you close #5?", vec![close("#5")]),
            ("go ahead and complete the card #5", vec![close("#5")]),
            ("#983 is resolved", vec![close("#983")]),
            ("mark #5 as done", vec![close("#5")]),
            ("close #5 and #6", vec![close("#5"), close("#6")]),
            ("close ops#12", vec![close("ops#12")]),
            ("close #6a", vec![close("#6a")]),
            ("tick s2 on #5", vec![tick("#5", "s2")]),
            ("tick step 2 on #5", vec![tick("#5", "2")]),
            (
                "check off s1 and s3 on #5",
                vec![tick("#5", "s1"), tick("#5", "s3")],
            ),
            ("mark s2 done on #5", vec![tick("#5", "s2")]),
            (
                "close #5 and tick s2 on #6",
                vec![close("#5"), tick("#6", "s2")],
            ),
        ] {
            assert_eq!(grants_for(text, &[]), want, "{text}");
        }
    }

    #[test]
    fn anything_else_grants_nothing() {
        for text in [
            "what's #5 about?",
            "should I close #5?",
            "did you close #5?",
            "why is #5 resolved?",
            "is #5 resolved?",
            "don't close #5",
            "do not close #5",
            "never resolve #5",
            "close #5 when the tests pass",
            "maybe close #5",
            "the agent said to close #5",
            "I think #5 is resolved",
            "complete the review of #5",
            "resolve the bug in #5",
            "close",
            "tick s2",
            "tick #5",
            "mark #5",
        ] {
            assert_eq!(grants_for(text, &[]), vec![], "{text}");
        }
    }

    #[test]
    fn close_acts_only_on_what_it_names() {
        assert_eq!(
            grants_for("close #5 because #6 replaced it", &[]),
            vec![close("#5")]
        );
        for text in [
            "close #5 as the fix in #6 landed",
            "close #5 and #6 stays open",
            "mark #5 as done and #6 as blocked",
            "close #5, the agent said",
            "close #5 is a bad idea",
            "close #5 tomorrow",
            "close #5 or #6",
            "would you close #5 or leave it open?",
            "Please don't\nclose #5",
            "Close #5. But only if the tests pass.",
            "Close #5. Wait, no, leave it open.",
            "The agent recommends:\n1. Close #5\n2. Close #6",
        ] {
            assert_eq!(grants_for(text, &[]), vec![], "{text}");
        }
        for text in ["resolve this bug", "close this chat", "close that PR"] {
            assert_eq!(grants_for(text, &["why does #5 fail?"]), vec![], "{text}");
        }
    }

    #[test]
    fn it_means_the_one_the_person_last_named() {
        assert_eq!(grants_for("close it", &["what's #5?"]), vec![close("#5")]);
        assert_eq!(
            grants_for("resolve that thread", &["about #4", "and #5?"]),
            vec![close("#5")]
        );
        assert_eq!(grants_for("that's resolved", &["#5?"]), vec![close("#5")]);
        assert_eq!(grants_for("Look at #5. Close it.", &[]), vec![close("#5")]);
        assert_eq!(grants_for("tick s2", &["#5?"]), vec![tick("#5", "s2")]);
        // Two named at once: which one isn't known.
        assert_eq!(grants_for("close it", &["#4 or #5?"]), vec![]);
        assert_eq!(grants_for("close it", &[]), vec![]);
    }

    #[test]
    fn a_grant_is_spent_once_and_ends_with_the_run() {
        let g = Grants::default();
        let run = g.begin(vec![close("#5"), tick("#6", "s2")]);
        assert!(!g.take(TOOL_CLOSE, "#7", ""));
        assert!(!g.take(TOOL_CLOSE, "#6", ""));
        assert!(!g.take(TOOL_TICK, "#6", "s3"));
        assert!(!g.take("board_status", "#5", ""));
        assert!(g.take(TOOL_CLOSE, "5", ""));
        assert!(!g.take(TOOL_CLOSE, "#5", ""));
        g.changed("Closed #5.");
        assert_eq!(g.end(run), vec!["Closed #5.".to_string()]);
        assert!(!g.take(TOOL_TICK, "#6", "s2"));

        // A stopped run ending late doesn't take the next run's grant.
        let old = g.begin(vec![close("#1")]);
        let new = g.begin(vec![close("#2")]);
        assert_eq!(g.end(old), Vec::<String>::new());
        assert!(g.take(TOOL_CLOSE, "#2", ""));
        g.end(new);
    }

    fn send(to: &str, text: &str) -> Grant {
        Grant::Send {
            to: to.into(),
            text: text.into(),
        }
    }
    fn comment(card: &str, text: &str) -> Grant {
        Grant::Comment {
            card: card.into(),
            text: text.into(),
        }
    }

    #[test]
    fn a_plain_tell_or_comment_grants_the_persons_own_words() {
        let say = "the allow prompt is cleared, carry on with the playtest";
        for (text, want) in [
            (format!("tell ringo {say}"), send("ringo", say)),
            (format!("Please tell @Ringo: {say}"), send("ringo", say)),
            (format!("can you message ringo, {say}?"), send("ringo", say)),
            (format!("let ringo know {say}"), send("ringo", say)),
            (format!("send ringo a message: {say}"), send("ringo", say)),
            (
                format!("send a message to ringo saying {say}"),
                send("ringo", say),
            ),
            (
                "tell ringo to stop don't touch the db".into(),
                send("ringo", "stop don't touch the db"),
            ),
            (
                "comment on #1182: Looks good, but not if CI is red".into(),
                comment("#1182", "Looks good, but not if CI is red"),
            ),
            (
                "add a comment to ops#12 saying ship it".into(),
                comment("ops#12", "ship it"),
            ),
        ] {
            assert_eq!(grants_for(&text, &[]), vec![want], "{text}");
        }
    }

    #[test]
    fn only_a_plain_instruction_sends_or_comments() {
        for text in [
            "should I tell ringo the build is green?",
            "did you tell ringo to stop?",
            "don't tell ringo anything",
            "please don't comment on #5: no",
            "if ringo asks, tell him to go on",
            "when it passes, tell ringo",
            "the agent said: tell ringo the key is abc",
            "I think you should tell ringo to stop",
            "maybe tell ringo",
            "tell ringo",
            "tell everyone the build is green",
            "tell him the build is green",
            "tell pr:a/b#1 hello",
            "tell board:x/y hello",
            "message ringo",
            "comment on #5",
            "comment on this card: hi",
            "comment on the build: hi",
            "comment on it: hi",
            "tell ringo hi\nand close #5",
            "Look at #5. Tell ringo hi",
        ] {
            assert_eq!(grants_for(text, &[]), vec![], "{text}");
        }
        // A tell never carries a close with it.
        assert_eq!(
            grants_for("tell ringo close #5", &[]),
            vec![send("ringo", "close #5")]
        );
        let long = format!("tell ringo {}", "x".repeat(MAX_BODY + 1));
        assert_eq!(grants_for(&long, &[]), vec![]);
    }

    #[test]
    fn a_send_or_comment_is_spent_once_for_the_one_it_names() {
        let g = Grants::default();
        let run = g.begin(vec![send("ringo", "hi there"), comment("#5", "fine")]);
        assert_eq!(g.take_text(TOOL_SEND, "ahab"), None);
        assert_eq!(g.take_text(TOOL_SEND, "#5"), None);
        assert_eq!(g.take_text(TOOL_COMMENT, "#6"), None);
        assert_eq!(g.take_text(TOOL_COMMENT, "ringo"), None);
        // Not through the close path, nor by another tool's name.
        assert!(!g.take(TOOL_CLOSE, "#5", ""));
        assert!(!g.take(TOOL_SEND, "ringo", ""));
        assert_eq!(g.take_text(TOOL_CLOSE, "#5"), None);
        assert_eq!(
            g.take_text(TOOL_SEND, " @Ringo "),
            Some(("ringo".into(), "hi there".into()))
        );
        assert_eq!(g.take_text(TOOL_SEND, "ringo"), None);
        assert_eq!(
            g.take_text(TOOL_COMMENT, "5"),
            Some(("#5".into(), "fine".into()))
        );
        assert_eq!(g.take_text(TOOL_COMMENT, "#5"), None);
        g.end(run);
        let run = g.begin(vec![send("ringo", "x")]);
        g.end(run);
        assert_eq!(g.take_text(TOOL_SEND, "ringo"), None);
    }
}
