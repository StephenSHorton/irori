//! A card's thread, pure (the React app's lib/thread.ts): entry headers
//! and badges, blocker tones and chips, who heard the person's comment,
//! and the live working line under a thread (cards #299, #317, #431).

use std::collections::{HashMap, HashSet};

use irori_window::cards::{AgentRef, CardBlocker, CardThreadItem};
use irori_window::roster::RosterAgent;

use super::agent_status::{Who, agent_status, ms};
use super::cards::{RUNNING_JOB, Tone, blocker, names_line};
use super::names::display_name;

pub const KIND_COMMENT: &str = "comment";
pub const KIND_MESSAGE: &str = "message";
pub const KIND_EVENT: &str = "event";

/// How many of the newest entries show before "Show earlier".
pub const THREAD_PAGE: usize = 50;

/// The comment that offers "Start an agent with this" (card #452): the
/// person's latest comment on the thread (not a blocker answer), or "".
pub fn start_agent_item(items: &[CardThreadItem]) -> String {
    items
        .iter()
        .rev()
        .find(|it| it.kind == KIND_COMMENT && it.who.status == "person" && it.blocker_id.is_empty())
        .map(|it| it.id.clone())
        .unwrap_or_default()
}

/// A blocker as the person reads it in a line: its question, quoted.
pub fn blocker_name(gist: &str) -> String {
    if gist.is_empty() {
        "a blocker".into()
    } else {
        format!("“{gist}”")
    }
}

/// The element key of a blocker on the card page, to scroll to it.
pub fn blocker_item_id(id: &str) -> String {
    format!("blocker-item-{id}")
}

/// The element key of a thread entry, to scroll to it.
pub fn thread_entry_id(id: &str) -> String {
    format!("thread-entry-{id}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Badge {
    pub label: String,
    pub tone: Tone,
}

fn badge(label: impl Into<String>, tone: Tone) -> Badge {
    Badge {
        label: label.into(),
        tone,
    }
}

/// An entry's header: who wrote it, who a message went to, the device it
/// was written on ("" for this one), its badges, and under the person's
/// comment who still has to hear it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadLabel {
    pub author: String,
    pub arrow: Option<String>,
    pub device: String,
    pub badges: Vec<Badge>,
    pub waiting: Vec<String>,
}

/// The header for one entry; `in_blocker` leaves off the blocker badge
/// for an entry already under its item.
pub fn thread_label(it: &CardThreadItem, in_blocker: bool) -> ThreadLabel {
    let mut b = Vec::new();
    if !it.blocker_id.is_empty() && it.kind != KIND_EVENT && !in_blocker {
        b.push(badge(
            format!("on {}", blocker_name(&it.blocker_gist)),
            Tone::Slate,
        ));
    }
    if it.gone {
        b.push(badge("recipient gone", Tone::Warning));
    }
    if it.unheard {
        b.push(badge("nobody has heard this yet", Tone::Warning));
    }
    if it.held {
        b.push(badge(
            if it.dismissed {
                "held · dismissed"
            } else {
                "held"
            },
            Tone::Slate,
        ));
    }
    if it.edited {
        b.push(badge("edited", Tone::Slate));
    }
    if it.files > 0 {
        b.push(badge(
            if it.files == 1 {
                "1 file".to_string()
            } else {
                format!("{} files", it.files)
            },
            Tone::Slate,
        ));
    }
    ThreadLabel {
        author: display_name(&it.who),
        arrow: it.to.as_ref().map(display_name),
        device: it.device.clone(),
        badges: b,
        waiting: it.pending.clone(),
    }
}

/// The entries to show: all, or the newest `n`.
pub fn visible<T>(items: &[T], show_all: bool, n: usize) -> &[T] {
    if show_all || items.len() <= n {
        items
    } else {
        &items[items.len() - n..]
    }
}

/// A message entry has more than the card shows: cut, held or with files.
pub fn opens_in_full(it: &CardThreadItem) -> bool {
    it.kind == KIND_MESSAGE
        && (it.held || it.files > 0 || it.text.ends_with(&format!("(full: msg:{})", it.id)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockerTone {
    Amber,
    Grey,
    Green,
    Closed,
}

/// Amber while it waits on the person, grey once answered, green when
/// resolved, and the closed look.
pub fn blocker_tone(state: &str) -> BlockerTone {
    match state {
        blocker::ANSWERED => BlockerTone::Grey,
        blocker::RESOLVED => BlockerTone::Green,
        blocker::CLOSED => BlockerTone::Closed,
        _ => BlockerTone::Amber,
    }
}

/// The person can still answer the item.
pub fn answerable(state: &str) -> bool {
    state != blocker::RESOLVED && state != blocker::CLOSED
}

/// An open item that waits on other work, never the person's (#481).
pub fn is_wait(b: &CardBlocker) -> bool {
    b.resolved.is_none() && b.waits_on.is_some()
}

/// A blocker thread entry that is an agent revising the item.
pub fn is_revision(it: &CardThreadItem) -> bool {
    it.kind == KIND_EVENT && it.revised
}

/// An item's state chip: "Sent to X" while answered, "Replied", "Ready
/// for you" on an open done check, "Revised"; none otherwise.
pub fn blocker_chip(b: &CardBlocker) -> Option<Badge> {
    let open = b.state.is_empty() || b.state == blocker::OPEN;
    if b.state == blocker::ANSWERED {
        let to = b.sent_to.as_ref().unwrap_or(&b.raised_by);
        return Some(badge(
            format!(
                "Sent to {}{}",
                display_name(to),
                if to.live { "" } else { " · session ended" }
            ),
            Tone::Slate,
        ));
    }
    if b.state == blocker::REPLIED {
        return Some(badge("Replied", Tone::Warning));
    }
    if b.done_check && open {
        return Some(badge("Ready for you", Tone::Success));
    }
    if open && b.thread.iter().any(is_revision) {
        return Some(badge("Revised", Tone::Warning));
    }
    None
}

/// Who heard the person's comment: who answers it, woken or at their next
/// turn, and the watchers.
pub fn heard_line(answers: &[AgentRef], watching: &[AgentRef], mode: &str) -> String {
    let names = |rs: &[AgentRef]| names_line(&rs.iter().map(display_name).collect::<Vec<_>>());
    let how = |rs: &[AgentRef], woken: bool| {
        let many = rs.len() > 1;
        let tail = match (woken, many) {
            (true, true) => "are woken",
            (true, false) => "is woken",
            (false, true) => "hear it at their next turn",
            (false, false) => "hears it at its next turn",
        };
        format!("{} {tail}", names(rs))
    };
    if answers.is_empty() && watching.is_empty() {
        return "Nobody has heard this yet.".into();
    }
    let mut parts = Vec::new();
    if !answers.is_empty() && mode == "stop" {
        parts.push(format!(
            "{} {} stopped and will answer here first.",
            names(answers),
            if answers.len() > 1 { "are" } else { "is" }
        ));
    } else if !answers.is_empty() {
        parts.push(format!(
            "{} and will answer here.",
            how(answers, mode == "wake")
        ));
    }
    if !watching.is_empty() {
        parts.push(if !answers.is_empty() {
            format!(
                "{}, watching: they answer only if asked.",
                how(watching, false)
            )
        } else {
            format!(
                "{}, watching.",
                how(watching, mode == "wake" || mode == "stop")
            )
        });
    }
    parts.join(" ")
}

/// Each agent's live roster state by key (cards #299, #453, #489, #544).
#[derive(Clone, Debug, Default)]
pub struct LiveAgent {
    pub status: String,
    pub updated_at: Option<i64>,
    pub status_since: Option<i64>,
    pub device_state: String,
    #[allow(dead_code)]
    pub device_via: String,
    pub last_tool: String,
    // thread.ts LiveAgent's whole shape, which Talk fills too; the hover
    // card reads these from the roster itself (widgets::agent_hover).
    #[allow(dead_code)]
    pub offers_cards: bool,
    #[allow(dead_code)]
    pub spawned: bool,
    #[allow(dead_code)]
    pub here: bool,
}

/// The live roster by key.
pub fn live_roster(agents: &[RosterAgent]) -> HashMap<String, LiveAgent> {
    agents
        .iter()
        .map(|a| {
            (
                a.card.key.clone(),
                LiveAgent {
                    status: a.card.status.clone(),
                    updated_at: ms(&a.card.updated_at),
                    status_since: ms(&a.card.status_since),
                    device_state: a.card.device_state.clone(),
                    device_via: a.card.device_via.clone(),
                    last_tool: a.card.last_tool.clone(),
                    offers_cards: a.offers_cards,
                    spawned: a.spawned,
                    here: a.here,
                },
            )
        })
        .collect()
}

/// How long an agent must sit idle before a thread says it stopped.
pub const STOPPED_GRACE: i64 = 3000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineTone {
    Working,
    Stopped,
}

/// The live line under a thread: its text and tone; a working line also
/// gives its parts so the view can swap in a changing word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkingLine {
    pub text: String,
    pub tone: LineTone,
    pub who: String,
    pub verb: &'static str,
    pub doing: String,
}

/// The working line's changing words while no tool says more (card #317).
pub const PONDER_WORDS: [&str; 8] = [
    "working",
    "pondering",
    "thinking",
    "mulling it over",
    "tinkering",
    "puzzling it out",
    "figuring",
    "noodling",
];

/// How often the word changes, in ms.
pub const PONDER_EVERY: i64 = 3000;

/// The word for the tick'th change; Reduce Motion keeps `ponder_word(0)`.
pub fn ponder_word(tick: i64) -> &'static str {
    PONDER_WORDS[tick.rem_euclid(PONDER_WORDS.len() as i64) as usize]
}

/// The live line under a thread: "oak is working…" while an agent on it is
/// in a turn, "oak stopped without replying" once it read the person's
/// latest comment, had a turn and went idle with nothing since. `working`
/// and `jobs` are who the card's own rule says (workingNow,
/// runningJobNow); `None` reads the live roster.
pub fn working_line(
    agents: &[AgentRef],
    items: &[CardThreadItem],
    live: &HashMap<String, LiveAgent>,
    now: i64,
    over: bool,
    working: Option<&[AgentRef]>,
    jobs: Option<&[AgentRef]>,
) -> Option<WorkingLine> {
    if over {
        return None;
    }
    let mut seen = HashSet::new();
    let on: Vec<&AgentRef> = agents
        .iter()
        .filter(|a| a.status != "person" && a.live && !a.offline && seen.insert(a.key.clone()))
        .collect();
    let status = |a: &AgentRef| {
        live.get(&a.key)
            .map_or(a.status.clone(), |l| l.status.clone())
    };
    let names =
        |rs: &[&AgentRef]| names_line(&rs.iter().map(|a| display_name(a)).collect::<Vec<_>>());
    let busy: Vec<&AgentRef> = match working {
        Some(w) => w.iter().collect(),
        None => on.iter().copied().filter(|a| status(a) == "busy").collect(),
    };
    if !busy.is_empty() {
        let doing = if busy.len() == 1 {
            tool_phrase(live.get(&busy[0].key).map_or("", |l| l.last_tool.as_str()))
        } else {
            String::new()
        };
        let who = names(&busy);
        let verb = if busy.len() > 1 { "are" } else { "is" };
        return Some(WorkingLine {
            text: format!(
                "{who} {verb} {}…",
                if doing.is_empty() { "working" } else { &doing }
            ),
            tone: LineTone::Working,
            who,
            verb,
            doing,
        });
    }
    let running: Vec<&AgentRef> = match (jobs, working) {
        (Some(j), _) => j.iter().collect(),
        (None, Some(_)) => vec![],
        (None, None) => on
            .iter()
            .copied()
            .filter(|a| status(a) == RUNNING_JOB)
            .collect(),
    };
    if !running.is_empty() {
        let who = names(&running);
        let verb = if running.len() > 1 { "are" } else { "is" };
        let since = if running.len() == 1 {
            live.get(&running[0].key).and_then(|l| l.status_since)
        } else {
            None
        };
        let doing = agent_status(&Who::status(RUNNING_JOB), since, now).text;
        return Some(WorkingLine {
            text: format!("{who} {verb} {doing}…"),
            tone: LineTone::Working,
            who,
            verb,
            doing,
        });
    }
    // The person's latest comment, and whether anyone answered it since.
    let p = items
        .iter()
        .rposition(|it| it.kind != KIND_EVENT && it.who.status == "person")?;
    let comment = &items[p];
    let at = ms(&comment.at)?;
    let after = &items[p + 1..];
    let stopped: Vec<&AgentRef> = on
        .iter()
        .copied()
        .filter(|a| {
            if !comment.heard_by.contains(&a.key) || status(a) != "idle" {
                return false;
            }
            if after.iter().any(|it| it.who.key == a.key) {
                return false;
            }
            live.get(&a.key)
                .and_then(|l| l.updated_at)
                .is_some_and(|since| since > at && now - since >= STOPPED_GRACE)
        })
        .collect();
    if stopped.is_empty() {
        return None;
    }
    Some(WorkingLine {
        text: format!("{} stopped without replying", names(&stopped)),
        tone: LineTone::Stopped,
        who: String::new(),
        verb: "",
        doing: String::new(),
    })
}

/// What a running tool is doing ("running a command"); "" for none.
pub fn tool_phrase(tool: &str) -> String {
    if tool.is_empty() {
        return String::new();
    }
    let fixed = match tool {
        "Bash" | "BashOutput" | "PowerShell" => "running a command",
        "Read" => "reading files",
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => "editing files",
        "Grep" | "Glob" => "searching files",
        "WebSearch" => "searching the web",
        "WebFetch" => "reading a web page",
        "Agent" | "Task" => "running a subagent",
        "TodoWrite" => "planning",
        "advisor" => "asking for advice",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.into();
    }
    if let Some(rest) = tool.strip_prefix("mcp__")
        && let Some(i) = rest.find("__")
        && i > 0
    {
        let (server, name) = (&rest[..i], &rest[i + 2..]);
        if server == "irori" {
            return if name.starts_with("board") {
                "on the Board"
            } else if name == "send" || name == "reply" {
                "writing a message"
            } else if name == "search" {
                "searching Irori"
            } else {
                "using Irori"
            }
            .into();
        }
        if server.contains("chrome") || server.contains("browser") {
            return "using the browser".into();
        }
        return format!("using {}", server.replace(['-', '_'], " "));
    }
    format!("using {tool}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gocompat::time::GoTime;

    fn otter() -> AgentRef {
        AgentRef {
            key: "dev/otter-session".into(),
            name: "sample-otter".into(),
            status: "busy".into(),
            live: true,
            ..AgentRef::default()
        }
    }
    fn wren() -> AgentRef {
        AgentRef {
            key: "mac/wren-session".into(),
            name: "mac/wren-session".into(),
            status: "gone".into(),
            ..AgentRef::default()
        }
    }
    fn item(kind: &str, f: impl Fn(&mut CardThreadItem)) -> CardThreadItem {
        let mut it = CardThreadItem {
            kind: kind.into(),
            who: otter(),
            ..CardThreadItem::default()
        };
        f(&mut it);
        it
    }
    fn sb(l: &str, t: Tone) -> Badge {
        badge(l, t)
    }

    #[test]
    fn labels() {
        let l = thread_label(&item("comment", |i| i.text = "hi".into()), false);
        assert_eq!(
            l,
            ThreadLabel {
                author: "sample-otter".into(),
                arrow: None,
                device: "".into(),
                badges: vec![],
                waiting: vec![]
            }
        );
        let l = thread_label(
            &item("message", |i| {
                i.to = Some(wren());
                i.device = "MacBook".into();
                i.gone = true;
                i.held = true;
                i.files = 2;
            }),
            false,
        );
        assert_eq!(l.arrow.as_deref(), Some("unknown agent"));
        assert_eq!(l.device, "MacBook");
        assert_eq!(
            l.badges,
            [
                sb("recipient gone", Tone::Warning),
                sb("held", Tone::Slate),
                sb("2 files", Tone::Slate)
            ]
        );
        let it = item("comment", |i| {
            i.to = Some(wren());
            i.blocker_id = "b2".into();
            i.blocker_gist = "Ship today, or wait…".into();
        });
        assert_eq!(
            thread_label(&it, false).badges,
            [sb("on “Ship today, or wait…”", Tone::Slate)]
        );
        assert_eq!(
            thread_label(&item("comment", |i| i.blocker_id = "b2".into()), false).badges,
            [sb("on a blocker", Tone::Slate)]
        );
        assert!(thread_label(&it, true).badges.is_empty());
        assert!(
            thread_label(&item("event", |i| i.blocker_id = "b2".into()), false)
                .badges
                .is_empty()
        );
        assert_eq!(
            thread_label(
                &item("message", |i| {
                    i.to = Some(wren());
                    i.files = 1
                }),
                false
            )
            .badges,
            [sb("1 file", Tone::Slate)]
        );
        assert_eq!(
            thread_label(&item("comment", |i| i.unheard = true), false).badges,
            [sb("nobody has heard this yet", Tone::Warning)]
        );
        let lines = vec!["a".to_string(), "b".to_string()];
        let l = thread_label(&item("comment", |i| i.pending = lines.clone()), false);
        assert_eq!(l.waiting, lines);
        assert!(l.badges.is_empty());
    }

    #[test]
    fn visible_pages() {
        let items: Vec<usize> = (0..THREAD_PAGE + 7).collect();
        let v = visible(&items, false, THREAD_PAGE);
        assert_eq!(
            (v.len(), v[0], v[v.len() - 1]),
            (THREAD_PAGE, 7, THREAD_PAGE + 6)
        );
        assert_eq!(visible(&items, true, THREAD_PAGE).len(), items.len());
        assert_eq!(visible(&[1, 2], false, THREAD_PAGE), [1, 2]);
        assert_eq!(
            visible(&items, false, 3),
            [THREAD_PAGE + 4, THREAD_PAGE + 5, THREAD_PAGE + 6]
        );
    }

    #[test]
    fn opens_in_full_cases() {
        let msg = |f: &dyn Fn(&mut CardThreadItem)| {
            item("message", |i| {
                i.id = "msg_1".into();
                i.text = "hi".into();
                f(i)
            })
        };
        assert!(!opens_in_full(&msg(&|_| {})));
        assert!(opens_in_full(&msg(
            &|i| i.text = "xxx… (full: msg:msg_1)".into()
        )));
        assert!(opens_in_full(&msg(&|i| {
            i.text = String::new();
            i.held = true
        })));
        assert!(opens_in_full(&msg(&|i| i.files = 2)));
        assert!(!opens_in_full(&item("comment", |i| {
            i.id = "c1".into();
            i.held = true
        })));
        assert!(!opens_in_full(&msg(
            &|i| i.text = "see (full: msg:msg_2)".into()
        )));
    }

    #[test]
    fn blocker_states() {
        for (st, tone, open) in [
            (blocker::OPEN, BlockerTone::Amber, true),
            (blocker::ANSWERED, BlockerTone::Grey, true),
            (blocker::REPLIED, BlockerTone::Amber, true),
            (blocker::RESOLVED, BlockerTone::Green, false),
            (blocker::CLOSED, BlockerTone::Closed, false),
        ] {
            assert_eq!(blocker_tone(st), tone);
            assert_eq!(answerable(st), open);
        }
        assert_eq!(blocker_tone(""), BlockerTone::Amber);
        let mut b = CardBlocker {
            state: "open".into(),
            waits_on: Some(Default::default()),
            ..CardBlocker::default()
        };
        assert!(is_wait(&b));
        assert!(!is_wait(&CardBlocker {
            state: "open".into(),
            ..CardBlocker::default()
        }));
        b.state = "closed".into();
        b.resolved = Some(Default::default());
        assert!(!is_wait(&b));
    }

    fn bl(state: &str, f: impl Fn(&mut CardBlocker)) -> CardBlocker {
        let mut b = CardBlocker {
            id: "b1".into(),
            raised_by: otter(),
            state: state.into(),
            ..CardBlocker::default()
        };
        f(&mut b);
        b
    }

    #[test]
    fn chips() {
        assert_eq!(
            blocker_chip(&bl("open", |b| b.done_check = true)),
            Some(sb("Ready for you", Tone::Success))
        );
        assert_eq!(
            blocker_chip(&bl("answered", |b| {
                b.done_check = true;
                b.sent_to = Some(otter())
            }))
            .unwrap()
            .label,
            "Sent to sample-otter"
        );
        assert_eq!(
            blocker_chip(&bl("answered", |b| b.sent_to = Some(otter()))),
            Some(sb("Sent to sample-otter", Tone::Slate))
        );
        assert_eq!(
            blocker_chip(&bl("answered", |b| b.sent_to = Some(wren()))),
            Some(sb("Sent to unknown agent · session ended", Tone::Slate))
        );
        assert_eq!(
            blocker_chip(&bl("answered", |_| {})).unwrap().label,
            "Sent to sample-otter"
        );
        assert_eq!(
            blocker_chip(&bl("replied", |_| {})),
            Some(sb("Replied", Tone::Warning))
        );
        for st in ["open", "resolved", "closed", ""] {
            assert_eq!(blocker_chip(&bl(st, |_| {})), None);
        }
    }

    #[test]
    fn revisions() {
        let rev = item("event", |i| {
            i.id = "d:3".into();
            i.blocker_id = "b1".into();
            i.revised = true;
        });
        let reply = item("comment", |i| {
            i.id = "c2".into();
            i.blocker_id = "b1".into();
        });
        assert!(is_revision(&rev));
        assert!(!is_revision(&reply));
        let mut c = rev.clone();
        c.kind = "comment".into();
        assert!(!is_revision(&c));
        let with = |st: &str, t: Vec<CardThreadItem>| bl(st, move |b| b.thread = t.clone());
        assert_eq!(
            blocker_chip(&with("open", vec![reply.clone(), rev.clone()])),
            Some(sb("Revised", Tone::Warning))
        );
        assert_eq!(
            blocker_chip(&with("", vec![rev.clone()])).unwrap().label,
            "Revised"
        );
        assert_eq!(blocker_chip(&with("open", vec![reply.clone()])), None);
        assert_eq!(
            blocker_chip(&with("answered", vec![reply, rev]))
                .unwrap()
                .label,
            "Sent to sample-otter"
        );
    }

    #[test]
    fn heard_lines() {
        assert_eq!(heard_line(&[], &[], ""), "Nobody has heard this yet.");
        assert_eq!(
            heard_line(&[otter()], &[wren()], "wake"),
            "sample-otter is woken and will answer here. unknown agent hears it at its next turn, watching: they answer only if asked."
        );
        assert_eq!(
            heard_line(&[otter(), wren()], &[], "next_turn"),
            "sample-otter and unknown agent hear it at their next turn and will answer here."
        );
        assert_eq!(
            heard_line(&[], &[otter()], "wake"),
            "sample-otter is woken, watching."
        );
    }

    const T0: i64 = 1_793_682_000_000; // 2026-11-03T05:00:00Z
    fn at(s: i64) -> i64 {
        T0 + s * 1000
    }
    fn agent(key: &str, name: &str) -> AgentRef {
        AgentRef {
            key: key.into(),
            name: name.into(),
            status: "idle".into(),
            live: true,
            ..AgentRef::default()
        }
    }
    fn you() -> AgentRef {
        AgentRef {
            key: "user:me".into(),
            name: "You".into(),
            status: "person".into(),
            live: true,
            ..AgentRef::default()
        }
    }
    fn comment() -> CardThreadItem {
        CardThreadItem {
            id: "c1".into(),
            kind: "comment".into(),
            who: you(),
            text: "how's it going?".into(),
            at: GoTime::unix_milli(at(0)),
            pending: vec!["oak read it".into()],
            heard_by: vec!["mac/oak".into()],
            ..CardThreadItem::default()
        }
    }
    fn roster(m: &[(&str, &str, i64)]) -> HashMap<String, LiveAgent> {
        m.iter()
            .map(|(k, st, s)| {
                (
                    k.to_string(),
                    LiveAgent {
                        status: st.to_string(),
                        updated_at: Some(at(*s)),
                        ..LiveAgent::default()
                    },
                )
            })
            .collect()
    }

    #[test]
    fn working_lines() {
        let oak = agent("mac/oak", "oak");
        let elm = agent("mac/elm", "elm");
        let c = comment();
        let l = working_line(
            &[oak.clone()],
            &[c.clone()],
            &roster(&[("mac/oak", "busy", 1)]),
            at(2),
            false,
            None,
            None,
        );
        assert_eq!(
            l,
            Some(WorkingLine {
                text: "oak is working…".into(),
                tone: LineTone::Working,
                who: "oak".into(),
                verb: "is",
                doing: "".into()
            })
        );
        let l = working_line(
            &[oak.clone(), elm.clone()],
            &[],
            &roster(&[("mac/oak", "busy", 1), ("mac/elm", "busy", 1)]),
            at(2),
            false,
            None,
            None,
        );
        assert_eq!(l.unwrap().text, "oak and elm are working…");
        let l = working_line(
            &[oak.clone()],
            &[c.clone()],
            &roster(&[("mac/oak", "idle", 20)]),
            at(30),
            false,
            None,
            None,
        );
        assert_eq!(
            l.map(|l| (l.text, l.tone)),
            Some(("oak stopped without replying".into(), LineTone::Stopped))
        );
        let reply = CardThreadItem {
            id: "r1".into(),
            kind: "comment".into(),
            who: oak.clone(),
            at: GoTime::unix_milli(at(10)),
            ..CardThreadItem::default()
        };
        assert!(
            working_line(
                &[oak.clone()],
                &[c.clone(), reply],
                &roster(&[("mac/oak", "idle", 20)]),
                at(30),
                false,
                None,
                None
            )
            .is_none()
        );
        assert!(
            working_line(
                &[oak.clone()],
                &[c.clone()],
                &roster(&[("mac/oak", "idle", 20)]),
                at(20) + STOPPED_GRACE - 1,
                false,
                None,
                None
            )
            .is_none()
        );
        assert!(
            working_line(
                &[oak.clone()],
                &[c.clone()],
                &roster(&[("mac/oak", "idle", -5)]),
                at(30),
                false,
                None,
                None
            )
            .is_none()
        );
        let mut waiting = c.clone();
        waiting.pending = vec!["queued for oak".into()];
        waiting.heard_by = vec![];
        assert!(
            working_line(
                &[oak.clone()],
                &[waiting],
                &roster(&[("mac/oak", "idle", 20)]),
                at(30),
                false,
                None,
                None
            )
            .is_none()
        );
        assert!(
            working_line(
                &[oak.clone()],
                &[c.clone()],
                &roster(&[("mac/oak", "busy", 1)]),
                at(2),
                true,
                None,
                None
            )
            .is_none()
        );
        let gone = AgentRef {
            live: false,
            ..oak.clone()
        };
        let away = AgentRef {
            offline: true,
            ..oak.clone()
        };
        assert!(
            working_line(
                &[gone, away],
                &[c.clone()],
                &roster(&[("mac/oak", "busy", 1)]),
                at(2),
                false,
                None,
                None
            )
            .is_none()
        );
        let handed = AgentRef {
            assigned: true,
            ..oak.clone()
        };
        assert_eq!(
            working_line(
                &[handed],
                &[c],
                &roster(&[("mac/oak", "busy", 1)]),
                at(2),
                false,
                None,
                None
            )
            .unwrap()
            .tone,
            LineTone::Working
        );
    }

    #[test]
    fn working_line_on_a_card_page() {
        let oak = agent("mac/oak", "oak");
        let elm = agent("mac/elm", "elm");
        let busy = roster(&[("mac/oak", "busy", 0), ("mac/elm", "busy", 0)]);
        let l = working_line(
            &[oak.clone(), elm.clone()],
            &[],
            &busy,
            at(0),
            false,
            Some(&[elm.clone()]),
            None,
        )
        .unwrap();
        assert_eq!(
            (l.tone, l.who.as_str(), l.verb),
            (LineTone::Working, "elm", "is")
        );
        assert!(working_line(&[oak.clone()], &[], &busy, at(0), false, Some(&[]), None).is_none());
        let shell = roster(&[("mac/oak", "shell", 0)]);
        assert_eq!(
            working_line(
                &[oak.clone()],
                &[],
                &shell,
                at(0),
                false,
                Some(&[]),
                Some(&[oak.clone()])
            ),
            Some(WorkingLine {
                text: "oak is running a job…".into(),
                tone: LineTone::Working,
                who: "oak".into(),
                verb: "is",
                doing: "running a job".into()
            })
        );
        assert!(
            working_line(
                &[oak.clone()],
                &[],
                &shell,
                at(0),
                false,
                Some(&[]),
                Some(&[])
            )
            .is_none()
        );
        let since = |s: i64| {
            HashMap::from([(
                "mac/oak".to_string(),
                LiveAgent {
                    status: "shell".into(),
                    status_since: Some(at(s)),
                    ..LiveAgent::default()
                },
            )])
        };
        let now = at(12 * 60);
        assert_eq!(
            working_line(
                &[oak.clone()],
                &[],
                &since(0),
                now,
                false,
                Some(&[]),
                Some(&[oak.clone()])
            )
            .unwrap()
            .text,
            "oak is running a job · 12 min…"
        );
        assert_eq!(
            working_line(
                &[oak.clone()],
                &[],
                &since(11 * 60 + 30),
                now,
                false,
                Some(&[]),
                Some(&[oak.clone()])
            )
            .unwrap()
            .text,
            "oak is running a job…"
        );
    }

    #[test]
    fn working_line_for_a_background_job() {
        let oak = agent("mac/oak", "oak");
        let mut c = comment();
        c.pending = vec![];
        let later = at(60);
        assert_eq!(
            working_line(
                &[oak.clone()],
                &[c.clone()],
                &roster(&[("mac/oak", "shell", 5)]),
                later,
                false,
                None,
                None
            )
            .unwrap()
            .text,
            "oak is running a job…"
        );
        assert_eq!(
            working_line(
                &[oak],
                &[c],
                &roster(&[("mac/oak", "idle", 5)]),
                later,
                false,
                None,
                None
            )
            .map(|l| l.text),
            Some("oak stopped without replying".into())
        );
    }

    #[test]
    fn ponder_words() {
        assert_eq!(ponder_word(0), "working");
        assert_ne!(ponder_word(1), ponder_word(0));
        assert_eq!(ponder_word(PONDER_WORDS.len() as i64), ponder_word(0));
        assert_eq!(ponder_word(-1), PONDER_WORDS[PONDER_WORDS.len() - 1]);
    }

    #[test]
    fn tool_phrases() {
        assert_eq!(tool_phrase(""), "");
        assert_eq!(tool_phrase("Bash"), "running a command");
        assert_eq!(tool_phrase("Edit"), "editing files");
        assert_eq!(tool_phrase("mcp__irori__board_comment"), "on the Board");
        assert_eq!(
            tool_phrase("mcp__claude-in-chrome__navigate"),
            "using the browser"
        );
        assert_eq!(tool_phrase("mcp__blender__look"), "using blender");
        assert_eq!(tool_phrase("Frobnicate"), "using Frobnicate");
        let oak = agent("mac/oak", "oak");
        let live = HashMap::from([(
            "mac/oak".to_string(),
            LiveAgent {
                status: "busy".into(),
                last_tool: "Grep".into(),
                ..LiveAgent::default()
            },
        )]);
        assert_eq!(
            working_line(&[oak], &[], &live, 0, false, None, None)
                .unwrap()
                .text,
            "oak is searching files…"
        );
    }

    #[test]
    fn start_an_agent_with_this() {
        let it = |id: &str, kind: &str, status: &str, b: &str| CardThreadItem {
            id: id.into(),
            kind: kind.into(),
            who: AgentRef {
                status: status.into(),
                ..AgentRef::default()
            },
            blocker_id: b.into(),
            ..CardThreadItem::default()
        };
        assert_eq!(
            start_agent_item(&[
                it("c1", "comment", "person", ""),
                it("c2", "comment", "idle", ""),
                it("c3", "comment", "person", "b1"),
                it("e1", "event", "person", ""),
            ]),
            "c1"
        );
        assert_eq!(start_agent_item(&[it("c2", "comment", "idle", "")]), "");
    }
}
