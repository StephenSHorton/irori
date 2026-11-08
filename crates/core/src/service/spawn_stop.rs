//! When a card can't get an agent, stop and ask once (card #480).
//!
//! Before this, a card whose repo had no checkout on the device that hands
//! it out was asked for again on every sweep: the live log has about 57,800
//! requested/failed pairs each for #447, #451, #452 and #290, all reading
//! "no checkout of remote:github.com/StephenSHorton/irori on 05f1844a…".
//! #467 stopped the asking but left the card silent. Now:
//!
//! - No checkout: after the first failure the card gets one plain blocker,
//!   "No device has a copy of irori. Choose its folder on Desktop
//!   Computer?". The person replies with the folder's path and the next
//!   agent starts there; a reply that isn't a folder here is quoted back
//!   and asked again (card #531: a question reads as one, not as a bad
//!   path). If a copy turns up here (a Where, or a checkout Irori sees),
//!   the blocker resolves itself and the agent starts.
//! - Card #681: a path in a sentence ("put the clone in ~/projects/rune")
//!   is the folder. One that isn't there yet but whose parent is (a clone
//!   on its way) isn't asked about again: Irori says it waits, and each
//!   sweep looks again, starting the agent once it is a folder. The
//!   person's own agents may answer too, but only with a checkout of the
//!   card's repo here: an agent can't point a card at any folder.
//! - A card that names no repo is never asked about (card #600): its
//!   agent starts where an ask's does and works the repo out itself.
//! - The spawner itself fails [`SPAWN_TRIES`] times in a row (the person's
//!   number from Settings › Agents, if they set one): one blocker,
//!   "Irori couldn't start an agent for this card: try again?".
//!
//! Every line here names devices and repos the way the person does: the
//! device's own name and the repo's short name, never a key or an id.

use std::path::Path;

use coord::{BlockedBy, Blocker, BlockerAnswer, BlockerOption, NewBlocker, Task, TaskFilter};

use super::Services;
use crate::logf;

pub(crate) const SWEEPER: &str = "irori";

/// In every "no copy" question: how a sweep finds its own blocker, whatever
/// the device is called today.
pub const CHOOSE_FOLDER: &str = "Choose its folder";

/// The question when the spawner keeps failing.
pub const START_FAILED: &str = "Irori couldn't start an agent for this card: try again?";

/// Its one option.
pub const START_AGAIN: &str = "Try again";

/// How many spawner failures in a row raise [`START_FAILED`]. A missing
/// checkout is asked about after the first: nothing changes between tries.
pub const SPAWN_TRIES: u32 = 3;

/// Whether a failed request's reason is a missing checkout, in the words of
/// any version (before #480 it named the key and the device id).
pub fn no_folder_reason(reason: &str) -> bool {
    reason.starts_with("no checkout of ")
        || reason.starts_with("no copy of ")
        || reason.starts_with("this card names no repo")
}

/// A missing checkout in the card line's words (card #480): the repo's short
/// name, or that the card names none, and the device's name.
pub fn no_folder_words(repo_key: &str, device: &str) -> String {
    if repo_key.is_empty() {
        format!("this card names no repo, and {device} has no folder for it")
    } else {
        format!("no copy of {} on {device}", coord::repo_name(repo_key))
    }
}

/// Whether b is one of Irori's "no copy" blockers.
pub fn is_no_copy(b: &Blocker) -> bool {
    b.raised_by == SWEEPER && b.question.contains(CHOOSE_FOLDER)
}

fn is_start_failed(b: &Blocker) -> bool {
    b.raised_by == SWEEPER && b.question == START_FAILED
}

/// Whether Irori itself acts on an answer to b (card #531): its "no copy"
/// and "couldn't start" asks, which the sweep reads, and its crash question,
/// whose answer lets the next agent start (card #540).
pub fn irori_acts_on(b: &Blocker) -> bool {
    is_no_copy(b) || is_start_failed(b) || coord::is_crash_question(b)
}

/// A leading `~` as the home folder.
fn expand_home(s: &str) -> String {
    if s == "~" || s.starts_with("~/") || s.starts_with("~\\") {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        if let Some(h) = home {
            return format!("{}{}", h.to_string_lossy(), &s[1..]);
        }
    }
    s.to_string()
}

/// The lines of a reply, trimmed, with no code fences.
fn raw_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("```") && !l.is_empty())
}

/// A line without the quotes or backticks people put around paths.
fn unquoted(l: &str) -> &str {
    l.trim_matches(|c: char| matches!(c, '`' | '"' | '\'' | '“' | '”' | '‘' | '’'))
        .trim()
}

/// The lines of a reply as a person means them: trimmed, with no code
/// fences and without the quotes or backticks people put around paths.
fn reply_lines(text: &str) -> impl Iterator<Item = &str> {
    raw_lines(text).map(unquoted).filter(|l| !l.is_empty())
}

/// Whether `w`, as typed, is only a root or the home folder ("/", "~",
/// "C:\"): never a repo's folder, as in "later / tomorrow".
fn bare_root(w: &str) -> bool {
    let w = w.trim_end_matches(['/', '\\']);
    let b = w.as_bytes();
    w.is_empty() || w == "~" || (b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

/// `w`, as typed, as a path: `~` expanded, when it starts at a root and is
/// more than one.
fn as_path(w: &str) -> Option<String> {
    let w = w.trim();
    if bare_root(w) {
        return None;
    }
    let f = expand_home(w);
    rooted(&f).then_some(f)
}

/// The quote pairs a span in a reply may sit in.
const QUOTES: [(char, char); 5] = [('`', '`'), ('"', '"'), ('\'', '\''), ('“', '”'), ('‘', '’')];

/// The spans of `line` in quotes, and `line` without them. A quote opens
/// only at the start of a word and closes only at its end, so the
/// apostrophe in "it's" opens nothing.
fn quoted_spans(line: &str) -> (Vec<String>, String) {
    let cs: Vec<char> = line.chars().collect();
    let opens = |c: Option<&char>| c.is_none_or(|c| c.is_whitespace() || "([{,;:".contains(*c));
    let closes = |c: Option<&char>| c.is_none_or(|c| c.is_whitespace() || ".,;:!?)]}".contains(*c));
    let (mut spans, mut rest) = (Vec::new(), String::new());
    let mut i = 0;
    while i < cs.len() {
        let open = QUOTES
            .iter()
            .find(|(o, _)| cs[i] == *o)
            .filter(|_| opens(i.checked_sub(1).and_then(|j| cs.get(j))));
        let close = open
            .and_then(|(_, c)| (i + 1..cs.len()).find(|&j| cs[j] == *c && closes(cs.get(j + 1))));
        if let Some(j) = close {
            spans.push(cs[i + 1..j].iter().collect());
            rest.push(' ');
            i = j + 1;
        } else {
            rest.push(cs[i]);
            i += 1;
        }
    }
    (spans, rest)
}

/// The paths a line names (card #681: "put the clone in ~/projects/rune"):
/// the line itself when it is one word or a folder here (a path with
/// spaces), else each span in quotes, then each word, that starts at a root
/// or at `~`, without the punctuation around it. A word in a sentence needs
/// a folder past its root ("/clone" is a command, "/" a slash).
fn paths_in(line: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |f: Option<String>| {
        if let Some(f) = f
            && !out.contains(&f)
        {
            out.push(f);
        }
    };
    let bare = unquoted(line);
    if let Some(w) = as_path(bare)
        && (!bare.contains(char::is_whitespace) || Path::new(&w).is_dir())
    {
        add(Some(w));
        return out;
    }
    let (spans, rest) = quoted_spans(line);
    for span in &spans {
        add(as_path(span));
    }
    for w in rest.split_whitespace() {
        let w = w.trim_start_matches(['(', '[', '{', '`', '"', '\'', '“', '‘']);
        let w = w.trim_end_matches([
            '.', ',', ';', ':', '!', '?', ')', ']', '}', '`', '"', '\'', '”', '’',
        ]);
        let deep = w
            .trim_start_matches(['/', '\\'])
            .trim_end_matches(['/', '\\'])
            .contains(['/', '\\']);
        let drive = w.as_bytes().get(1) == Some(&b':');
        if deep || drive || w.starts_with('~') {
            add(as_path(w));
        }
    }
    out
}

/// The paths a reply names, over all its lines. A question names none ("is
/// it /Users/me/irori?", card #531).
fn paths_said(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in raw_lines(text).filter(|l| !unquoted(l).ends_with('?')) {
        for f in paths_in(line) {
            if !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out
}

/// The folder a person's reply names: its one path, a line that is a path
/// (spaces and all) or a path in words (card #681), with a leading `~` as
/// the home folder. With none, or several (Irori can't tell which was
/// meant: "not ~/a, use ~/b"), the reply's first line, which is quoted
/// back to the person.
fn as_folder(text: &str) -> String {
    match paths_said(text).as_slice() {
        [f] => f.clone(),
        _ => expand_home(reply_lines(text).next().unwrap_or("")),
    }
}

/// `s` cut to `n` characters, marking the cut with `…`.
fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// A reply quoted back in a line: short enough for any blocker field.
fn quoted(reply: &str) -> String {
    format!("“{}”", cut(&as_folder(reply), 120))
}

/// Whether f starts at a root, on this device or any other: a person on a
/// Mac can paste a Windows path, and the other way round.
fn rooted(f: &str) -> bool {
    let b = f.as_bytes();
    Path::new(f).is_absolute()
        || f.starts_with(['/', '\\'])
        || (b.len() > 2
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && matches!(b[2], b'/' | b'\\'))
}

/// Whether f was meant as a path, not words to Irori: "i don't understand
/// this blocker" or "where is that?" is no bad path (card #531), nor is
/// "what does folder/repo mean": a relative path with a space reads as
/// words, as most replies with both are.
fn reads_as_path(f: &str) -> bool {
    !f.ends_with('?')
        && (rooted(f) || (f.contains(['/', '\\']) && !f.contains(char::is_whitespace)))
}

/// The folder a reply names when it isn't there yet but could be (card
/// #681: "put the clone in ~/projects/rune" while it clones): a full path
/// here whose parent is a folder. Its parent missing reads as a typo.
fn awaited(reply: &str) -> Option<String> {
    let [f] = <[String; 1]>::try_from(paths_said(reply)).ok()?;
    let p = Path::new(&f);
    (reads_as_path(&f) && p.is_absolute() && !p.is_dir() && p.parent().is_some_and(Path::is_dir))
        .then_some(f)
}

/// Why a reply wasn't used as the card's folder.
fn not_a_folder_line(reply: &str, dev: &str) -> String {
    let f = as_folder(reply);
    if paths_said(reply).len() > 1 {
        format!(
            "{} names more than one folder, so Irori can't tell which to start the agent in and is asking again.",
            quoted(reply)
        )
    } else if awaited(reply).is_some() {
        format!(
            "{} isn't a folder on {dev} yet. Irori starts the agent there as soon as it is, with no need to answer again.",
            quoted(reply)
        )
    } else if !reads_as_path(&f) {
        format!(
            "{} doesn't name a folder, so Irori can't start the agent yet and is asking again.",
            quoted(reply)
        )
    } else if rooted(&f) && !Path::new(&f).is_dir() {
        format!("{} isn't a folder on {dev}.", quoted(reply))
    } else {
        // Relative, or a real folder named without its drive on Windows.
        format!("{} isn't a full path on {dev}.", quoted(reply))
    }
}

/// Words on a "no copy" blocker: the person's, or one of their own agents'
/// (card #681: told to answer it, an agent resolves it with the path).
#[derive(Debug, Clone)]
pub(crate) struct Said {
    text: String,
    person: bool,
}

impl Services {
    /// This device as the person calls it ("Desktop Computer").
    pub(crate) fn this_device_name(&self) -> String {
        let n = self
            .device_labels()
            .remove(&self.device.id)
            .unwrap_or_default();
        if n.trim().is_empty() {
            "this device".into()
        } else {
            n
        }
    }

    /// A checkout of t's repo that is a folder on this device. Checkouts
    /// replicate, so one seen on another device is skipped.
    pub(crate) fn checkout_here(&self, repo_key: &str) -> Option<String> {
        if repo_key.is_empty() {
            return None;
        }
        // Never a job's temp clone (card #565): with only one, there is no
        // checkout here, and the card asks for one as for any other.
        let roots = self.capture.repo_roots(repo_key);
        let root = crate::treeclean::pick_root(&roots);
        if root.is_none()
            && let Some(tmp) = crate::treeclean::only_temp(&roots)
        {
            self.say_temp_root(repo_key, &tmp);
        }
        root
    }

    /// What the person and their own agents said on blockers `on` of t,
    /// newest first: their comments, and the answer a blocker was resolved
    /// with (the window resolves an Irori ask with the answer, so it is on
    /// no comment, card #531; an agent's board_unblock note is its answer,
    /// card #681). Irori's own resolve lines and a close aren't anyone's
    /// answer, even a close by the person ("back on the Board: …").
    fn said_on(&self, t: &Task, on: &[&Blocker]) -> Vec<Said> {
        let said_by = |by: &str, text: String| {
            let person = self.chain.is_owner(by);
            (person || (by != SWEEPER && self.own_agent(by))).then_some(Said { text, person })
        };
        let mut said: Vec<(i64, Said)> = Vec::new();
        for b in on {
            if let Some(r) = &b.resolved
                && !r.closed
                && let Some(s) = said_by(&r.by, r.answer.clone())
            {
                said.push((r.at.unix_nano(), s));
            }
        }
        for c in self.coord.comments(&t.id).unwrap_or_default() {
            let on_it = on
                .iter()
                .any(|b| b.id == c.blocker_id && c.at.unix_nano() >= b.at.unix_nano());
            if on_it && let Some(s) = said_by(&c.by, c.text) {
                said.push((c.at.unix_nano(), s));
            }
        }
        said.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        said.into_iter().map(|(_, s)| s).collect()
    }

    /// The reply a "no copy" ask answers: the person's newest words on b,
    /// or an agent's that name a path. An agent saying anything else there
    /// ("looking into it") is no answer to quote back.
    fn reply_on(&self, t: &Task, b: &Blocker) -> Option<Said> {
        self.said_on(t, &[b])
            .into_iter()
            .find(|s| s.person || rooted(&as_folder(&s.text)))
    }

    /// Whether `dir` is a checkout of repo `repo_key` here: it shares a
    /// main checkout with one Irori has seen for the repo, or git names
    /// its repo as this one. Unlike a person's, an agent's folder counts
    /// only when it is (card #681), so it can't point a card anywhere.
    pub(crate) fn checkout_of(&self, dir: &str, repo_key: &str) -> bool {
        if repo_key.is_empty() {
            return false;
        }
        let main = |d: &str| crate::helpertree::main_root(Path::new(d));
        let mine = main(dir);
        if mine.is_some()
            && self
                .capture
                .repo_roots(repo_key)
                .iter()
                .filter(|r| Path::new(r).is_dir())
                .any(|r| main(r) == mine)
        {
            return true;
        }
        let k = self.capture.repo_key_of(dir);
        !k.is_empty() && coord::same_repo_key(&k, repo_key)
    }

    /// The folder the person (or one of their agents, with a checkout of
    /// the card's repo) named in reply to a "no copy" blocker on t, newest
    /// first, when it is a folder here. A folder named before it was there
    /// counts once it is (card #681): each sweep looks again.
    pub(crate) fn chosen_folder(&self, t: &Task) -> Option<String> {
        let on: Vec<&Blocker> = t.blockers.iter().filter(|b| is_no_copy(b)).collect();
        if on.is_empty() {
            return None;
        }
        self.said_on(t, &on).into_iter().find_map(|s| {
            let f = as_folder(&s.text);
            let here = !f.is_empty() && Path::new(&f).is_absolute() && Path::new(&f).is_dir();
            (here && (s.person || self.checkout_of(&f, &t.repo_key))).then_some(f)
        })
    }

    /// Why reply `said` wasn't used as card t's folder on `dev`.
    fn not_used_line(&self, t: &Task, said: &Said, dev: &str) -> String {
        let f = as_folder(&said.text);
        if !said.person && Path::new(&f).is_absolute() && Path::new(&f).is_dir() {
            let name = coord::repo_name(&t.repo_key);
            return format!(
                "{} isn't a checkout of {name} on {dev}, and only the person can choose a folder that isn't, so Irori is asking again.",
                quoted(&said.text)
            );
        }
        not_a_folder_line(&said.text, dev)
    }

    /// The "no copy" question, its context and its options for t on this
    /// device. `reply` is what the person said to the last one, if it
    /// wasn't a folder: the new question quotes it, so it can't read as the
    /// same one coming back.
    fn no_copy_ask(&self, t: &Task, reply: Option<&Said>) -> (String, String, Vec<BlockerOption>) {
        let dev = self.this_device_name();
        let name = coord::repo_name(&t.repo_key);
        let (question, why) = if self.capture.repo_roots(&t.repo_key).is_empty() {
            (
                format!("No device has a copy of {name}. {CHOOSE_FOLDER} on {dev}?"),
                format!(
                    "Irori starts this card's agent in a copy of {name}, and none of your devices has one it knows of."
                ),
            )
        } else {
            (
                format!("{dev} has no copy of {name}. {CHOOSE_FOLDER} on {dev}?"),
                format!(
                    "Irori starts this card's agent on {dev}, where the card was made, and none of the folders it has seen for {name} is on {dev} now."
                ),
            )
        };
        let question = match reply {
            Some(r) if let Some(f) = awaited(&r.text) => {
                format!(
                    "Irori starts the agent in “{}” once it's there. {CHOOSE_FOLDER} on {dev} instead?",
                    cut(&f, 60)
                )
            }
            Some(r) => format!(
                "Irori still needs a folder after the reply “{}”. {CHOOSE_FOLDER} on {dev}?",
                cut(&as_folder(&r.text), 60)
            ),
            None => question,
        };
        let mut context = String::new();
        if let Some(r) = reply {
            context += &self.not_used_line(t, r, &dev);
            context.push(' ');
        }
        context += &why;
        context.push(' ');
        context += &format!(
            "Reply with the folder's full path on {dev} (clone it first if it isn't there) and its agent starts in it."
        );
        let options = Vec::new();
        (
            cut(&question, 200),
            cut(&context, coord::MAX_CONTEXT),
            options,
        )
    }

    /// Card t got no folder here: ask the person once. Called after its
    /// failed request, and on each sweep that finds the card asked for
    /// again (their reply put it back on the Board).
    pub(crate) fn stop_no_copy(&self, t: &Task) {
        let Some(t) = self.coord.get_task(&t.id) else {
            return;
        };
        // A card that names no repo isn't asked about (card #600): its
        // agent works the repo out. It has no folder here only when this
        // device has no checkout of any repo; its failed request says so.
        if is_helper(&t) || coord::is_terminal(&t.state) || t.repo_key.is_empty() {
            return;
        }
        let dev = self.this_device_name();
        // The person's reply on the newest "no copy" blocker, open or closed
        // (their answer puts a holderless card back on the Board, which
        // closes its blockers). It wasn't a folder here, or it would have
        // been used: say so and ask again.
        let latest = t
            .blockers
            .iter()
            .filter(|b| is_no_copy(b))
            .max_by_key(|b| b.at.unix_nano());
        let reply = latest.and_then(|b| self.reply_on(&t, b));
        if let Some(b) = latest.filter(|b| b.open()) {
            // Still blocked and nobody replied: it is already asking.
            if reply.is_none() && t.state == coord::BLOCKED {
                return;
            }
            // Put back on the Board ("Start an agent", or a reply): still no
            // folder, so this one closes and a fresh one asks.
            let text = match &reply {
                Some(r) => self.not_used_line(&t, r, &dev),
                None => format!("Still no folder for it on {dev}."),
            };
            let a = BlockerAnswer {
                text,
                resolve: true,
                ..BlockerAnswer::default()
            };
            if let Err(e) = self.coord.answer(SWEEPER, &t.id, &b.id, a) {
                logf(format_args!("no copy on {}: {e}", t.short_ref()));
                return;
            }
        }
        let (question, context, options) = self.no_copy_ask(&t, reply.as_ref());
        let nb = NewBlocker {
            question,
            context,
            options,
            blocked_by: Some(BlockedBy {
                kind: coord::BLOCKED_ON_PERSON.into(),
                ..BlockedBy::default()
            }),
            ..NewBlocker::default()
        };
        if let Err(e) = self.coord.raise(SWEEPER, &t.id, nb) {
            logf(format_args!("no copy on {}: {e}", t.short_ref()));
        }
    }

    /// The spawner failed for t: after the setting's tries in a row, else
    /// [`SPAWN_TRIES`], stop and ask once.
    pub(crate) fn note_start_failed(&self, t: &Task, err: &str) {
        let tries = match self.spawn_setting("").start_tries {
            n if n >= 1 => n as u32,
            _ => SPAWN_TRIES,
        };
        let n = {
            let mut m = self
                .spawn
                .start_fails
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let n = m.entry(t.id.clone()).or_insert(0);
            *n += 1;
            *n
        };
        if n < tries {
            return;
        }
        // One short of the limit: after Try again, the next failure asks
        // straight away. A start clears it (note_started).
        self.spawn
            .start_fails
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(t.id.clone(), tries - 1);
        if t.blockers.iter().any(|b| b.open() && is_start_failed(b)) {
            return;
        }
        let dev = self.this_device_name();
        let lead = format!("Irori couldn't start an agent for this card on {dev}.");
        let nb = NewBlocker {
            question: START_FAILED.into(),
            context: cut(
                &format!("{lead} The last error: {}", err.trim()),
                coord::MAX_CONTEXT,
            ),
            options: vec![BlockerOption {
                answer: START_AGAIN.into(),
                why: "Irori tries once more; if it fails again it asks again.".into(),
                by: SWEEPER.into(),
                recommended: true,
                ..BlockerOption::default()
            }],
            blocked_by: Some(BlockedBy {
                kind: coord::BLOCKED_ON_PERSON.into(),
                ..BlockedBy::default()
            }),
            ..NewBlocker::default()
        };
        if let Err(e) = self.coord.raise(SWEEPER, &t.id, nb) {
            logf(format_args!("start failed on {}: {e}", t.short_ref()));
        }
    }

    /// An agent started for t: its failure count starts over.
    pub(crate) fn note_started(&self, id: &str) {
        self.spawn
            .start_fails
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    /// Resolves t's open "no copy" blocker, if any, now that its agent has a
    /// folder here. Whether one was resolved.
    pub(crate) fn resolve_no_copy(&self, t: &Task, folder: &str, trusted: bool) -> bool {
        let Some(b) = t.blockers.iter().find(|b| b.open() && is_no_copy(b)) else {
            return false;
        };
        let name = coord::repo_name(&t.repo_key);
        let dev = self.this_device_name();
        let folder = cut(folder, 200);
        let found = if name.is_empty() {
            format!("Irori starts its agent in {folder} on {dev}, and it works out the card's repo")
        } else {
            format!("Irori found a copy of {name} on {dev} ({folder})")
        };
        let text = if trusted {
            format!("{found}: its agent starts there.")
        } else {
            format!(
                "{found}, but it isn't trusted for agents yet: Trust and start it in Needs you."
            )
        };
        let a = BlockerAnswer {
            text: cut(&text, 400),
            resolve: true,
            ..BlockerAnswer::default()
        };
        match self.coord.answer(SWEEPER, &t.id, &b.id, a) {
            Ok(_) => true,
            Err(e) => {
                logf(format_args!("no copy on {}: {e}", t.short_ref()));
                false
            }
        }
    }

    /// Closes the questions Irori asked because no agent could start for a
    /// card, once an agent holds it (card #717): "no copy here", "couldn't
    /// start: try again?" and "crashed: start another?" all ask about
    /// starting one, which no longer happens. Only the device that hands
    /// the card out, which asked them, closes them. The crash count stays,
    /// so if the holder goes before it's done the crash question comes back.
    pub(crate) fn drop_held_waits(&self) {
        let open = TaskFilter {
            open: true,
            ..TaskFilter::default()
        };
        for (_, co) in self.agent_coords() {
            for t in co.list_tasks(&open) {
                let Some(who) = agent_holder(&t) else {
                    continue;
                };
                let asks: Vec<&Blocker> = t
                    .blockers
                    .iter()
                    .filter(|b| b.open() && irori_acts_on(b))
                    .collect();
                if asks.is_empty() || !co.hands_out_here(&t.id) {
                    continue;
                }
                let name = match self.agent_name(who) {
                    n if n.is_empty() => "An agent".to_string(),
                    n => n,
                };
                for b in asks {
                    let a = BlockerAnswer {
                        text: format!("{name} took the card, so no agent needs to start for it."),
                        resolve: true,
                        ..BlockerAnswer::default()
                    };
                    if let Err(e) = co.answer(SWEEPER, &t.id, &b.id, a) {
                        logf(format_args!("held {}: {e}", t.short_ref()));
                    }
                }
            }
        }
    }

    /// Lifts the "no copy" blockers this device raised once a copy is here
    /// (a Where, a checkout, or a folder the person named): the card then
    /// starts as any other does.
    pub(crate) fn lift_no_copy(&self, sp: &super::spawn::Spawner) {
        let me = self.device.id.clone();
        let open = self.coord.list_tasks(&TaskFilter {
            open: true,
            ..TaskFilter::default()
        });
        for t in open {
            if is_helper(&t) || !t.blockers.iter().any(|b| b.open() && is_no_copy(b)) {
                continue;
            }
            // Only the device whose request failed lifts it: another one
            // finding a copy doesn't start the agent here.
            if !t.spawn_request().is_some_and(|s| s.device == me) {
                continue;
            }
            let Some(folder) = self.spawn_folder(&t) else {
                continue;
            };
            // Not trusted: the request fails as such and Needs you offers
            // Trust and start. Resolving moves the card on past its failed
            // request, so the sweep asks again without waiting out the
            // retry; it isn't the person's ask, so the queue order still
            // holds (card #576).
            let trusted = (sp.trusted)(Path::new(&folder));
            self.resolve_no_copy(&t, &folder, trusted);
        }
    }
}

/// The first of card t's holders that is an agent, not a person: one on
/// it means no agent needs to start for it (card #717).
pub(crate) fn agent_holder(t: &Task) -> Option<&String> {
    t.holders.iter().find(|k| is_agent_key(k))
}

/// The agent card t is with (card #717): one holding it, else one it was
/// given to. A person holding it (a card given to them, say) isn't one:
/// their agent still has to start.
pub fn agent_on(t: &Task) -> Option<&String> {
    agent_holder(t).or_else(|| t.assigned.iter().find(|k| is_agent_key(k)))
}

/// Whether `k` is an agent's key, not a person's.
pub fn is_agent_key(k: &str) -> bool {
    !k.starts_with(irori_identity::kinds::PERSON_ACTOR_PREFIX) && !k.starts_with("user:")
}

/// A helper's part (spawned agents part 7): its folder is a worktree of the
/// lead's, so a missing one is the lead's matter, not a question.
pub(crate) fn is_helper(t: &Task) -> bool {
    t.spawn && !t.parent.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Card #717: a person holding a card (one given to them) isn't an
    /// agent on it; an agent holding it, or given it, is.
    #[test]
    fn only_an_agent_is_on_a_card() {
        let mut t = Task {
            holders: vec!["person:p1".into()],
            assigned: vec!["person:p1".into()],
            ..Task::default()
        };
        assert_eq!(agent_on(&t), None);
        t.holders.push("user:dev1".into());
        assert_eq!(agent_on(&t), None);
        t.assigned.push("agent:c1".into());
        assert_eq!(agent_on(&t).map(String::as_str), Some("agent:c1"));
        t.holders.push("agent:c2".into());
        assert_eq!(agent_on(&t).map(String::as_str), Some("agent:c2"));
    }

    #[test]
    fn replies_name_folders() {
        assert_eq!(
            as_folder("  `C:\\Users\\me\\irori`  "),
            "C:\\Users\\me\\irori"
        );
        assert_eq!(as_folder("\"/Users/me/irori\"\nthanks"), "/Users/me/irori");
        assert_eq!(as_folder(""), "");
        assert_eq!(as_folder("```\n/Users/me/irori\n```"), "/Users/me/irori");
        assert_eq!(
            as_folder("```sh\n`/Users/me/irori`\n```"),
            "/Users/me/irori"
        );
        if let Some(h) = std::env::var_os("HOME") {
            assert_eq!(
                as_folder("~/irori"),
                format!("{}/irori", h.to_string_lossy())
            );
        }
        let long = "x".repeat(5000);
        assert!(quoted(&long).chars().count() <= 122);
        assert_eq!(
            not_a_folder_line("work/irori", "Desktop Computer"),
            "“work/irori” isn't a full path on Desktop Computer."
        );
        assert_eq!(
            not_a_folder_line("C:\\nope\\irori", "Macbook Pro"),
            "“C:\\nope\\irori” isn't a folder on Macbook Pro."
        );
        assert_eq!(
            not_a_folder_line("/nope/irori", "Desktop Computer"),
            "“/nope/irori” isn't a folder on Desktop Computer."
        );
    }

    /// Card #681: a path in a sentence is the folder, `~` and all.
    #[test]
    fn a_path_in_words_is_read() {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(|h| h.to_string_lossy().into_owned());
        if let Some(h) = &home {
            assert_eq!(
                as_folder("put the clone in ~/projects/rune"),
                format!("{h}/projects/rune")
            );
            assert_eq!(
                as_folder("Sure.\nClone it to ~/projects/rune, thanks!"),
                format!("{h}/projects/rune")
            );
        }
        assert_eq!(as_folder("clone it to /x/y."), "/x/y");
        assert_eq!(as_folder("use `C:\\src\\rune`, thanks"), "C:\\src\\rune");
        assert_eq!(as_folder("it's in (/opt/rune)"), "/opt/rune");
        assert_eq!(as_folder("clone it to /tmp/rune/"), "/tmp/rune/");
        // In quotes, a path keeps its spaces; an apostrophe opens nothing.
        for r in [
            "put it in \"/Users/me/My Projects/rune\" please",
            "“/Users/me/My Projects/rune”",
            "it's in '/Users/me/My Projects/rune'",
            "put it in ‘/Users/me/My Projects/rune’",
        ] {
            assert_eq!(as_folder(r), "/Users/me/My Projects/rune", "{r}");
        }
        // Words with no path are quoted back as they were: a slash, the
        // home folder alone, a command, a URL, a question.
        for r in [
            "I'll clone it later",
            "I'll clone it later / tomorrow",
            "clone it to ~ or wherever",
            "I'll run /clone later",
            "see https://github.com/x/rune",
            "is it /Users/me/irori?",
        ] {
            assert_eq!(as_folder(r), r, "{r}");
            assert!(!reads_as_path(&as_folder(r)) || r.contains("?"), "{r}");
        }

        let d = tempfile::tempdir().unwrap();
        let real = d.path().display().to_string();
        // A line that is a folder here keeps its spaces.
        let spaced = d.path().join("My Projects");
        std::fs::create_dir(&spaced).unwrap();
        let spaced = spaced.display().to_string();
        assert_eq!(as_folder(&spaced), spaced);
        // A path then words: the path.
        assert_eq!(
            as_folder(&format!("{real}/rune (cloning it now)")),
            format!("{real}/rune")
        );

        // Two paths: Irori can't tell which was meant, whichever is there.
        for r in [
            format!("not {real}, that's stale: use {real}/rune"),
            format!("In {real} I cloned it as {real}/rune"),
        ] {
            assert_eq!(paths_said(&r).len(), 2, "{r}");
            assert!(!Path::new(&as_folder(&r)).is_dir(), "{r}");
            assert_eq!(awaited(&r), None, "{r}");
            let line = not_a_folder_line(&r, "Macbook Pro");
            assert!(line.contains("names more than one folder"), "{line}");
        }

        // A sentence about a folder on its way: Irori waits for it.
        let line = not_a_folder_line(&format!("put the clone in {real}/rune"), "Macbook Pro");
        assert_eq!(
            line,
            format!(
                "“{real}/rune” isn't a folder on Macbook Pro yet. Irori starts the agent there as soon as it is, with no need to answer again."
            )
        );
        assert_eq!(
            awaited(&format!("put the clone in {real}/rune")),
            Some(format!("{real}/rune"))
        );
        // A missing parent is a typo, not a clone on its way.
        assert_eq!(awaited("put the clone in /nope/at/all/rune"), None);
        assert_eq!(awaited(&format!("use {real}")), None);
    }

    /// A real folder named without its drive is there, but not a full path.
    #[cfg(windows)]
    #[test]
    fn a_folder_without_its_drive_is_not_a_full_path() {
        let cwd = std::env::current_dir().unwrap();
        let s = cwd.to_string_lossy();
        let line = not_a_folder_line(&s[2..], "Desktop Computer");
        assert!(
            line.ends_with("isn't a full path on Desktop Computer."),
            "{line}"
        );
    }

    /// Card #531: words to Irori aren't called a bad path.
    #[test]
    fn a_question_is_not_a_bad_path() {
        for r in [
            "i don't understand this blocker",
            "I'll clone it later",
            "what folder?",
            "is it /Users/me/irori?",
            "what does folder/repo mean",
            "huh",
        ] {
            let line = not_a_folder_line(r, "Macbook Pro");
            assert!(line.starts_with(&format!("“{r}”")), "{line}");
            assert!(line.contains("asking again"), "{line}");
            assert!(!line.contains("isn't a"), "{line}");
        }
    }

    #[test]
    fn old_and_new_reasons_are_no_folder() {
        assert!(no_folder_reason(
            "no checkout of remote:github.com/StephenSHorton/irori on 05f1844a"
        ));
        assert!(no_folder_reason("no checkout of  on 05f1844a"));
        assert!(no_folder_reason("no copy of irori on Desktop Computer"));
        assert!(no_folder_reason(
            "this card names no repo, and Desktop Computer has no folder for it"
        ));
        assert!(!no_folder_reason("not trusted: /r on pc"));
    }
}
