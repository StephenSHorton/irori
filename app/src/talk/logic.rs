//! Talk's pure logic: the React app's lib/conversations.ts (its non-sample
//! half), lib/talk-sections.ts, the helpers of components/talk/talk-page.tsx,
//! conversation.tsx and threads.tsx, and the pure parts of lib/talk.ts. No
//! Rune here; the `*.test.ts` cases are ported as this module's tests.

use std::collections::{HashMap, HashSet};

use irori_window::talk::{TalkConversation, TalkMember, TalkPost, TalkReplier};
use irori_window::threads::{THREAD_ANSWERED, THREAD_HANDED, THREAD_RESOLVED, THREAD_WAITING, ThreadSummary};

use crate::logic::agent_status::{Who, agent_status};
use crate::logic::cards::{Tone, names_line};
use crate::logic::names::{display_name, is_key_like, is_others_key, member_name};
use crate::logic::plain::{plain_line, relative_time};
use crate::logic::spawn_line::{SpawnState, spawn_line};
use crate::logic::thread::LiveAgent;
use irori_window::cards::AgentRef;

/// The longest an agent shows as replying after its wake (card #645).
pub const REPLYING_CAP: i64 = 30 * 60 * 1000;

/// How many posts a conversation loads at a time.
pub const POSTS_PAGE: i64 = 100;

// ---------- conversations.ts ----------

/// The default for a post's "Agents answer now" (design §3): on when the
/// text names nobody or names an agent, off when it names only people.
pub fn agents_now_for(kinds: &[&str]) -> bool {
    kinds.is_empty() || kinds.iter().any(|k| *k == "agent")
}

/// Who the box's mentions name: person keys start "person:", the rest are
/// agents.
pub fn mention_kinds<'a>(keys: impl IntoIterator<Item = &'a str>) -> Vec<&'static str> {
    keys.into_iter()
        .map(|k| if k.starts_with("person:") { "person" } else { "agent" })
        .collect()
}

/// A Go time as ms, 0 for none.
pub fn at_ms(t: &gocompat::time::GoTime) -> i64 {
    if t.is_zero() { 0 } else { t.unix_milli_value() }
}

/// Channels by name and DMs newest first, as the Talk list shows them;
/// threads are left out.
pub fn listed(convs: &[&TalkConversation]) -> (Vec<TalkConversation>, Vec<TalkConversation>) {
    let mut channels: Vec<TalkConversation> =
        convs.iter().filter(|c| c.kind == "channel").map(|c| (*c).clone()).collect();
    channels.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.name.cmp(&b.name)));
    let mut dms: Vec<TalkConversation> = convs.iter().filter(|c| c.kind == "dm").map(|c| (*c).clone()).collect();
    dms.sort_by_key(|c| std::cmp::Reverse(at_ms(&c.last_at)));
    (channels, dms)
}

/// Whether `m` is another person (card #538): `person:<id>`; the person's
/// own is `user` or `user:<device>`, and a shared board's sender isn't one.
pub fn is_other_person(m: &TalkMember) -> bool {
    m.kind == "person" && m.key.starts_with("person:")
}

/// The other person in a DM between two people, or None for a DM with an
/// agent (or a channel).
pub fn dm_person(c: &TalkConversation) -> Option<&TalkMember> {
    if c.kind != "dm" || c.members.iter().any(|m| m.kind != "person") {
        return None;
    }
    c.members.iter().find(|m| is_other_person(m))
}

/// What a DM with another person waits on, "" when it isn't waiting.
pub fn waiting_text(waiting: &str, who: &str) -> String {
    match waiting {
        "them" => format!("Waiting for {who}'s Irori"),
        "update" => format!("{who}'s Irori needs updating"),
        _ => String::new(),
    }
}

/// The person on any of their devices: "user" or "user:<device>".
pub fn is_you(key: &str) -> bool {
    key == "user" || key.starts_with("user:")
}

/// The member `key` names: its own row, else the person by whichever of
/// their keys the row has, else the person as "you" when nothing lists
/// them (card #642).
pub fn member_of<'a>(members: impl IntoIterator<Item = &'a TalkMember>, key: &str) -> Option<TalkMember> {
    let mut you: Option<&TalkMember> = None;
    for m in members {
        if m.key == key {
            return Some(m.clone());
        }
        if m.kind == "person" && is_you(&m.key) {
            you = Some(m);
        }
    }
    if !is_you(key) {
        return None;
    }
    Some(you.cloned().unwrap_or_else(|| TalkMember {
        kind: "person".into(),
        key: key.into(),
        name: "you".into(),
        ..TalkMember::default()
    }))
}

/// A DM's other side (an agent DM has one), or the two agents of a
/// read-only agent↔agent DM.
pub fn dm_with(c: &TalkConversation) -> Vec<&TalkMember> {
    c.members.iter().filter(|m| m.kind != "person").collect()
}

/// Whether a DM's agents have all gone from the live roster (card #473).
pub fn dm_gone(c: &TalkConversation, live: &HashMap<String, LiveAgent>) -> bool {
    let other = dm_with(c);
    !other.is_empty() && other.iter().all(|m| live.get(&m.key).map_or("gone", |l| l.status.as_str()) == "gone")
}

/// The conversations that wait on the person (card #484): DMs with
/// something new (not with gone agents) and channels that mention them.
pub fn talk_waiting(convs: &[TalkConversation], live: &HashMap<String, LiveAgent>) -> usize {
    convs
        .iter()
        .filter(|c| match c.kind.as_str() {
            "channel" => c.mentions > 0,
            "dm" => !c.read_only && c.unread > 0 && !dm_gone(c, live),
            _ => false,
        })
        .count()
}

/// The row under a conversation's posts (#645).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyLine {
    pub who: String,
    pub verb: &'static str,
    pub text: String,
}

/// "heron is replying…", "heron and otter are replying…"; agents past
/// [`REPLYING_CAP`] drop off, and a name that is a key is left out.
pub fn replying_line(agents: &[TalkReplier], now: i64) -> Option<ReplyLine> {
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for a in agents {
        let at = at_ms(&a.since);
        if at > 0 && now - at > REPLYING_CAP {
            continue;
        }
        let name = plain_line(&a.name);
        if name.is_empty() || is_key_like(&name) || seen.contains(&a.key) {
            continue;
        }
        seen.insert(a.key.clone());
        names.push(name);
    }
    if names.is_empty() {
        return None;
    }
    let who = names_line(&names);
    let verb = if names.len() > 1 { "are" } else { "is" };
    Some(ReplyLine {
        text: format!("{who} {verb} replying…"),
        who,
        verb,
    })
}

/// A post on one line: its text, or its file's name when it has only files.
pub fn post_line(p: Option<&TalkPost>) -> String {
    let Some(p) = p else { return String::new() };
    let names: Vec<&str> = p.attachments.iter().map(|f| f.name.as_str()).collect();
    text_line(&p.text, &names)
}

/// [`post_line`] of a text and its files' names.
pub fn text_line(text: &str, files: &[&str]) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if !text.is_empty() {
        return text;
    }
    match files {
        [] => String::new(),
        [one] => format!("📎 {one}"),
        [first, rest @ ..] => format!("📎 {first} +{}", rest.len()),
    }
}

/// A file as [`from_post`] names it.
pub struct PostFile<'a> {
    pub name: &'a str,
    pub size: i64,
    pub sha256: &'a str,
}

/// The New card form a message fills (Make a card): its first line as the
/// title (cut to 80), the message and where it was said as the objective;
/// files named with their sha256 so the card's agent can fetch them.
pub fn from_post(text: &str, where_: &str, files: &[PostFile]) -> (String, String) {
    let t = text.trim();
    let first = t.lines().next().unwrap_or("").to_string();
    let first = if first.is_empty() { files.first().map(|f| f.name.to_string()).unwrap_or_default() } else { first };
    let title = if first.chars().count() > 80 {
        let cut: String = first.chars().take(79).collect();
        format!("{}…", cut.trim_end())
    } else {
        first
    };
    let body = if t.is_empty() { String::new() } else { format!("{t}\n\n") };
    let names = if files.is_empty() {
        String::new()
    } else {
        let lines: Vec<String> = files
            .iter()
            .map(|f| format!("file: {} ({}, sha256={})", f.name, crate::text::attach::human_size(f.size), f.sha256))
            .collect();
        format!("{}\n\n", lines.join("\n"))
    };
    (title, format!("{body}{names}(From a message in {where_}.)"))
}

// ---------- talk-sections.ts ----------

/// A section of Talk's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SectionKey {
    Threads,
    Channels,
    Dms,
    Gone,
    Agents,
}

impl SectionKey {
    pub const ALL: [SectionKey; 5] =
        [SectionKey::Threads, SectionKey::Channels, SectionKey::Dms, SectionKey::Gone, SectionKey::Agents];

    pub fn name(self) -> &'static str {
        match self {
            SectionKey::Threads => "threads",
            SectionKey::Channels => "channels",
            SectionKey::Dms => "dms",
            SectionKey::Gone => "gone",
            SectionKey::Agents => "agents",
        }
    }

    /// Each section's fold before the person touches it: the Archive and
    /// Agents talking start folded.
    pub fn folded_by_default(self) -> bool {
        matches!(self, SectionKey::Gone | SectionKey::Agents)
    }
}

/// How many rows an unfolded section lists before "Show N more".
pub const SHOWN_AT_ONCE: usize = 8;

/// DMs split three ways.
#[derive(Clone, Debug, Default)]
pub struct DmSections {
    pub yours: Vec<TalkConversation>,
    pub gone: Vec<TalkConversation>,
    pub agents: Vec<TalkConversation>,
}

/// DMs (newest first) split: yours with a running agent or a person, the
/// Archive of yours whose agents have gone (card #473), and agent↔agent DMs
/// the person only reads. `keep` is the one opened from yours: it stays
/// there while open.
pub fn dm_sections(dms: &[TalkConversation], gone: impl Fn(&TalkConversation) -> bool, keep: &str) -> DmSections {
    let mut out = DmSections::default();
    for c in dms {
        if c.read_only {
            out.agents.push(c.clone());
        } else if archived(c, &gone, keep) {
            out.gone.push(c.clone());
        } else {
            out.yours.push(c.clone());
        }
    }
    out
}

/// Whether one of the person's DMs goes in the Archive.
pub fn archived(c: &TalkConversation, gone: impl Fn(&TalkConversation) -> bool, keep: &str) -> bool {
    c.conv_id != keep && gone(c)
}

/// What a section lists: nothing when folded, else the newest few (all
/// with `all`), keeping the open one. The second is how many it leaves out.
pub fn section_rows<'a>(
    items: &'a [TalkConversation],
    folded: bool,
    open: &str,
    all: bool,
) -> (Vec<&'a TalkConversation>, usize) {
    if folded {
        return (vec![], items.len());
    }
    if all || items.len() <= SHOWN_AT_ONCE {
        return (items.iter().collect(), 0);
    }
    let mut rows: Vec<&TalkConversation> = items[..SHOWN_AT_ONCE].iter().collect();
    if let Some(at) = items.iter().position(|c| c.conv_id == open)
        && at >= SHOWN_AT_ONCE
    {
        rows.push(&items[at]);
    }
    let hidden = items.len() - rows.len();
    (rows, hidden)
}

/// One message an agent sent to many agents (card #486), one row.
#[derive(Clone, Debug)]
pub struct Broadcast {
    /// "broadcast:", the sender and the text: stable while copies come in.
    pub id: String,
    pub by: String,
    pub text: String,
    pub last_at: i64,
    /// The DMs it went to, newest first.
    pub convs: Vec<TalkConversation>,
}

/// One row of Agents talking.
#[derive(Clone, Debug)]
pub enum AgentRow {
    Dm(TalkConversation),
    Broadcast(Broadcast),
}

impl AgentRow {
    pub fn id(&self) -> &str {
        match self {
            AgentRow::Dm(c) => &c.conv_id,
            AgentRow::Broadcast(b) => &b.id,
        }
    }
}

/// How far apart one sender's copies of the same text may be and still be
/// one broadcast.
pub const BROADCAST_WINDOW: i64 = 10 * 60_000;

/// The most rows Agents talking lists, even after "Show more".
pub const AGENTS_CAP: usize = 50;

/// Read-only DMs (newest first) whose newest post has the same sender and
/// text, each copy within [`BROADCAST_WINDOW`] of the next, grouped into
/// one broadcast row where its newest DM was.
pub fn agent_rows(items: &[TalkConversation]) -> Vec<AgentRow> {
    let mut out: Vec<AgentRow> = Vec::new();
    // The key's broadcast, by its place in `out`.
    let mut growing: HashMap<String, usize> = HashMap::new();
    for c in items {
        let text = &c.last_text;
        let by = &c.last_by;
        if !c.read_only || text.is_empty() || by.is_empty() || !c.members.iter().any(|m| &m.key == by) {
            out.push(AgentRow::Dm(c.clone()));
            continue;
        }
        let key = format!("{by}\u{0}{text}");
        if let Some(&i) = growing.get(&key)
            && let AgentRow::Broadcast(b) = &mut out[i]
            && at_ms(&b.convs[b.convs.len() - 1].last_at) - at_ms(&c.last_at) <= BROADCAST_WINDOW
        {
            b.convs.push(c.clone());
            continue;
        }
        growing.insert(key.clone(), out.len());
        out.push(AgentRow::Broadcast(Broadcast {
            id: format!("broadcast:{key}"),
            by: by.clone(),
            text: text.clone(),
            last_at: at_ms(&c.last_at),
            convs: vec![c.clone()],
        }));
    }
    out.into_iter()
        .map(|r| match r {
            AgentRow::Broadcast(b) if b.convs.len() == 1 => AgentRow::Dm(b.convs.into_iter().next().unwrap_or_default()),
            r => r,
        })
        .collect()
}

/// Who a broadcast went to: each DM's other side.
pub fn broadcast_to(b: &Broadcast) -> Vec<TalkMember> {
    b.convs
        .iter()
        .map(|c| {
            c.members
                .iter()
                .find(|m| m.key != b.by)
                .or_else(|| c.members.first())
                .cloned()
                .unwrap_or_default()
        })
        .collect()
}

/// Whether a row holds conversation `id`.
pub fn agent_row_has(r: &AgentRow, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    match r {
        AgentRow::Dm(c) => c.conv_id == id,
        AgentRow::Broadcast(b) => b.convs.iter().any(|c| c.conv_id == id),
    }
}

/// What Agents talking lists, with how many more "Show more" would list
/// and how many lie past [`AGENTS_CAP`].
pub struct AgentSection<'a> {
    pub rows: Vec<&'a AgentRow>,
    pub hidden: usize,
    pub capped: usize,
}

pub fn agent_section_rows<'a>(rows: &'a [AgentRow], folded: bool, open: &str, all: bool, searching: bool) -> AgentSection<'a> {
    if folded {
        return AgentSection { rows: vec![], hidden: 0, capped: 0 };
    }
    let i = rows.iter().position(|r| agent_row_has(r, open));
    if searching {
        return AgentSection { rows: rows.iter().collect(), hidden: 0, capped: 0 };
    }
    let limit = if all { AGENTS_CAP } else { SHOWN_AT_ONCE };
    let mut shown: Vec<&AgentRow> = rows.iter().take(limit).collect();
    if let Some(i) = i
        && i >= limit
    {
        shown.push(&rows[i]);
    }
    let reach = rows.len().min(AGENTS_CAP);
    let beyond = usize::from(i.is_some_and(|i| i >= AGENTS_CAP));
    AgentSection {
        hidden: reach.saturating_sub(shown.len() - beyond),
        capped: rows.len().saturating_sub(AGENTS_CAP + beyond),
        rows: shown,
    }
}

/// A section's conversations with something new for the person.
pub fn unread_in(items: &[TalkConversation]) -> usize {
    items.iter().filter(|c| !c.read_only && c.unread > 0).count()
}

/// What a section's name shows besides its count: the "N new" pill's count
/// and whether it offers Mark all read (the opened Archive's alone).
pub fn section_header(k: SectionKey, folded: bool, items: &[TalkConversation]) -> (usize, bool) {
    let unread = unread_in(items);
    if k != SectionKey::Gone {
        return (unread, false);
    }
    if folded {
        return (0, false);
    }
    (unread, unread > 0)
}

// ---------- talk-page.tsx ----------

/// A thread's ref: "#12", or "mlg#12" in a space.
pub fn thread_ref(tag: &str, short: &str) -> String {
    if short.is_empty() { String::new() } else { format!("{tag}#{short}") }
}

/// Whether a thread fits the list's search: its title, what was asked,
/// its #ref (with or without its tag), or the newest reply.
pub fn talk_matches(t: &ThreadSummary, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    let r = thread_ref(&t.tag, &t.short);
    let bare = thread_ref("", &t.short);
    [t.title.as_str(), t.text.as_str(), r.as_str(), bare.as_str(), t.last_text.as_str()]
        .iter()
        .any(|s| !s.is_empty() && s.to_lowercase().contains(&q))
}

/// Whether a thread waits on the person: answered and not resolved, or no
/// computer can start its agent (card #541).
pub fn needs_you(state: &str, no_agent: &str) -> bool {
    state == THREAD_ANSWERED || !no_agent.is_empty()
}

/// Whether a channel or DM fits the search: its name, topic, newest line
/// or a member's name.
pub fn conv_matches(c: &TalkConversation, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    std::iter::once(c.name.as_str())
        .chain(std::iter::once(c.topic.as_deref().unwrap_or("")))
        .chain(std::iter::once(c.last_text.as_str()))
        .chain(c.members.iter().map(|m| m.name.as_str()))
        .any(|s| s.to_lowercase().contains(&q))
}

/// What a DM row says (card #486): its newest post on one line ("owl:
/// Rebased onto yours" for agents talking, "You: " for the person's own),
/// and when.
pub fn last_line(c: &TalkConversation, now: i64) -> (String, String) {
    let text = plain_line(&c.last_text);
    let at = at_ms(&c.last_at);
    let when = if at > 0 { relative_time(at, now) } else { String::new() };
    if text.is_empty() {
        return (String::new(), when);
    }
    let by = if c.last_by.is_empty() { None } else { member_of(&c.members, &c.last_by) };
    let mine = by.as_ref().is_some_and(|b| b.kind == "person" && (is_you(&b.key) || b.name == "you"));
    let who = match &by {
        Some(b) if c.read_only => format!("{}: ", member_name(&b.kind, &b.name)),
        _ if mine => "You: ".to_string(),
        _ => String::new(),
    };
    (format!("{who}{text}"), when)
}

/// A new channel's name: lowercase, no spaces or #.
pub fn channel_name(raw: &str) -> String {
    let s = raw.trim().trim_start_matches('#').to_lowercase();
    let mut out = String::new();
    let mut dash = false;
    for ch in s.chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-' {
            out.push(ch);
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

// ---------- conversation.tsx ----------

/// A member's status now: "person", the live roster's for an agent, else
/// its board status for another member's agent, else "gone"; another
/// member's agent this device knows nothing about is "unknown" (#478).
pub fn status_of(m: &TalkMember, live: &HashMap<String, LiveAgent>) -> String {
    if m.kind == "person" {
        return "person".into();
    }
    if let Some(l) = live.get(&m.key)
        && !l.status.is_empty()
    {
        return l.status.clone();
    }
    if !m.status.is_empty() {
        return m.status.clone();
    }
    if !m.owner.is_empty() || is_others_key(&m.key) { "unknown".into() } else { "gone".into() }
}

/// A status in words, as agentStatus says it.
pub fn status_word(status: &str) -> String {
    agent_status(&Who::status(status), None, 0).text
}

/// How a member stands in words, with its device's state and how long its
/// job has run; another member's agent reads "Spencer's agent · idle".
pub fn member_status(m: &TalkMember, live: &HashMap<String, LiveAgent>, now: i64) -> String {
    let a = if m.kind == "person" { None } else { live.get(&m.key) };
    let who = Who {
        status: status_of(m, live),
        device_state: a.map(|a| a.device_state.clone()).unwrap_or_default(),
        ..Who::default()
    };
    let word = agent_status(&who, a.and_then(|a| a.status_since), now).text;
    with_owner(m, &word)
}

/// The line under another member's agent.
pub fn whose_word(m: &TalkMember, status: &str) -> String {
    with_owner(m, &status_word(status))
}

fn with_owner(m: &TalkMember, word: &str) -> String {
    if m.owner.is_empty() {
        return word.to_string();
    }
    let whose = format!("{}'s agent", plain_line(&m.owner));
    if word.is_empty() { whose } else { format!("{whose} · {word}") }
}

/// A member as a face needs it, with its live status.
pub fn ref_of(m: &TalkMember, live: &HashMap<String, LiveAgent>) -> AgentRef {
    let status = status_of(m, live);
    let l = if m.kind == "person" { None } else { live.get(&m.key) };
    AgentRef {
        key: m.key.clone(),
        name: if m.kind == "person" && m.name == "you" { "You".into() } else { member_name(&m.kind, &m.name) },
        live: status != "gone" && status != "unknown",
        offline: l.is_some_and(|l| l.device_state == "offline"),
        status,
        ..AgentRef::default()
    }
}

/// A member's name as Talk says it: "You" for the person.
pub fn shown_name(m: &TalkMember) -> String {
    if m.kind == "person" && m.name == "you" { "You".into() } else { member_name(&m.kind, &m.name) }
}

/// A civil date: (year, month 1-12, day).
pub type Ymd = (i64, u32, u32);

fn days_from_civil((y, m, d): Ymd) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

const WEEKDAYS: [&str; 7] = ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// The line between days: Today, Yesterday, or "Tuesday, Nov 3".
pub fn day_label(at: Ymd, now: Ymd) -> String {
    let days = days_from_civil(now) - days_from_civil(at);
    match days {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => {
            let wd = WEEKDAYS[days_from_civil(at).rem_euclid(7) as usize];
            format!("{wd}, {} {}", MONTHS[(at.1 as usize).saturating_sub(1).min(11)], at.2)
        }
    }
}

/// A time as this computer's civil date and clock: (date, "4:30 PM").
pub fn local(ms: i64) -> (Ymd, String) {
    let t = gocompat::time::GoTime::unix_milli(ms);
    let (y, mo, d, h, mi, _) = t.with_offset(sys::tz::local_offset(ms.div_euclid(1000))).civil();
    let (h12, ap) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    ((y, mo, d), format!("{h12:02}:{mi:02} {ap}"))
}

/// The new thread's ask from a channel message: the question, then the
/// message quoted and where it was said.
pub fn thread_text(question: &str, author: &str, text: &str, channel: &str) -> String {
    let quoted: Vec<String> = text.trim().lines().map(|l| format!("> {l}")).collect();
    format!("{}\n\n{}\n({author}, in #{channel})", question.trim(), quoted.join("\n"))
}

// ---------- threads.tsx ----------

/// Whether the Resolved list is unfolded: the person's pick, or a resolved
/// thread open.
pub fn resolved_shown(pick: bool, open: &str, threads: &[(&str, &str)]) -> bool {
    pick || threads.iter().any(|(id, st)| *id == open && *st == THREAD_RESOLVED)
}

/// A click on Resolved: the pick it sets, and whether to close the open
/// thread (folding with a resolved thread open closes it).
pub fn resolved_toggle(pick: bool, open: &str, threads: &[(&str, &str)]) -> (bool, bool) {
    if !resolved_shown(pick, open, threads) {
        return (true, false);
    }
    let open_resolved = threads.iter().any(|(id, st)| *id == open && *st == THREAD_RESOLVED);
    (false, open_resolved)
}

/// The thread to keep open once `id` is resolved.
pub fn open_after_resolve(open: &str, id: &str) -> String {
    if open == id { String::new() } else { open.to_string() }
}

/// A thread state's badge on its title line.
pub fn state_badge(state: &str) -> Option<(&'static str, Tone)> {
    match state {
        THREAD_WAITING => Some(("Waiting", Tone::Warning)),
        THREAD_HANDED => Some(("Answering", Tone::Info)),
        THREAD_ANSWERED => Some(("Answered", Tone::Success)),
        THREAD_RESOLVED => Some(("Resolved", Tone::Slate)),
        _ => None,
    }
}

/// A card chip's tone by the card's state.
pub fn card_state_tone(state: &str) -> Tone {
    match state {
        "working" => Tone::Info,
        "input_required" | "blocked" | "in_review" => Tone::Warning,
        "completed" => Tone::Success,
        _ => Tone::Slate,
    }
}

/// A thread's spawn fields, as the spawn line reads them.
pub fn spawn_state(t: &ThreadSummary) -> SpawnState {
    SpawnState {
        starting: t.starting.clone(),
        at_cap: t.at_cap.clone(),
        queue_place: t.queue_place,
        usage_limit: t.usage_limit.filter(|x| !x.is_zero()),
        cant_start: t.cant_start.clone(),
        no_agent: t.no_agent.clone(),
    }
}

/// Where a thread stands, under its title in the list.
pub fn state_line(t: &ThreadSummary, why: &str) -> String {
    if t.state == THREAD_ANSWERED && !t.no_agent.is_empty() {
        return t.no_agent.clone();
    }
    match t.state.as_str() {
        THREAD_ANSWERED => match &t.last_by {
            Some(by) if !t.last_text.is_empty() => format!("{}: {}", display_name(by), plain_line(&t.last_text)),
            _ => "Answered".into(),
        },
        THREAD_HANDED => match t.told.last() {
            Some(w) => format!("{} is answering", display_name(w)),
            None => "An agent is answering".into(),
        },
        THREAD_RESOLVED => "Resolved".into(),
        _ => spawn_line(&spawn_state(t))
            .map(|l| l.text)
            .unwrap_or_else(|| if why.is_empty() { "Waiting for an agent".into() } else { why.to_string() }),
    }
}

/// "alpha opened this thread".
pub fn opened_line(who: &AgentRef) -> String {
    format!("{} opened this thread", display_name(who))
}

/// What a thread with no replies yet says in its pane.
pub fn empty_line(t: &ThreadSummary) -> String {
    if let Some(o) = &t.opened_by {
        return empty_opened(o);
    }
    if !t.no_agent.is_empty() {
        return t.no_agent.clone();
    }
    if t.state == THREAD_WAITING && t.agents_now {
        return "No answer yet. An agent with Offer it cards on picks it up, or Irori starts one when spawning is on."
            .into();
    }
    if t.agents_now {
        return "No answer yet.".into();
    }
    "No answer yet. Agents answer now is off: agents in it read it at their next turn.".into()
}

/// An agent's own thread before anyone writes in it.
pub fn empty_opened(who: &AgentRef) -> String {
    format!(
        "No replies yet. {} hears yours; no other agent is started for it until you write.",
        display_name(who)
    )
}

// ---------- talk.ts (its pure helpers) ----------

// talk.ts's nameLookup is inbox::words::Names: one lookup names agent
// keys ("you" for the person, else the first name any source gives, else
// "unknown agent"; never a key, card #478).

/// Per agent key: deliveries not yet picked up, and held for the person.
/// `deliveries` are each delivery's (to, state).
pub fn counts_by_agent<'a>(deliveries: impl IntoIterator<Item = (&'a str, &'a str)>) -> HashMap<String, (u32, u32)> {
    let mut out: HashMap<String, (u32, u32)> = HashMap::new();
    for (to, state) in deliveries {
        if to.is_empty() {
            continue;
        }
        let c = out.entry(to.to_string()).or_default();
        match state {
            "queued" | "delivered" => c.0 += 1,
            "held" => c.1 += 1,
            _ => {}
        }
    }
    out.retain(|_, c| c.0 + c.1 > 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gocompat::time::GoTime;

    fn t(s: &str) -> GoTime {
        GoTime::parse_rfc3339(s).expect("time")
    }

    fn conv(f: impl FnOnce(&mut TalkConversation)) -> TalkConversation {
        let mut c = TalkConversation::default();
        f(&mut c);
        c
    }

    fn m(kind: &str, key: &str, name: &str) -> TalkMember {
        TalkMember {
            kind: kind.into(),
            key: key.into(),
            name: name.into(),
            ..TalkMember::default()
        }
    }

    fn agent(key: &str) -> TalkMember {
        m("agent", key, key)
    }

    fn live(rows: &[(&str, &str)]) -> HashMap<String, LiveAgent> {
        rows.iter()
            .map(|(k, s)| {
                (
                    k.to_string(),
                    LiveAgent {
                        status: s.to_string(),
                        ..LiveAgent::default()
                    },
                )
            })
            .collect()
    }

    // conversations.test.ts

    #[test]
    fn agents_now_by_default() {
        assert!(agents_now_for(&[]));
        assert!(agents_now_for(&["agent"]));
        assert!(agents_now_for(&["person", "agent"]));
        assert!(!agents_now_for(&["person"]));
    }

    #[test]
    fn listed_sorts_channels_by_name_and_dms_newest_first() {
        let cs = [
            conv(|c| {
                c.conv_id = "c2".into();
                c.kind = "channel".into();
                c.name = "suzuri".into()
            }),
            conv(|c| {
                c.conv_id = "d1".into();
                c.kind = "dm".into();
                c.last_at = t("2026-11-03T10:00:00Z")
            }),
            conv(|c| {
                c.conv_id = "t".into();
                c.kind = "thread".into()
            }),
            conv(|c| {
                c.conv_id = "c1".into();
                c.kind = "channel".into();
                c.name = "irori".into()
            }),
            conv(|c| {
                c.conv_id = "d2".into();
                c.kind = "dm".into();
                c.last_at = t("2026-11-03T11:00:00Z")
            }),
        ];
        let refs: Vec<&TalkConversation> = cs.iter().collect();
        let (ch, dm) = listed(&refs);
        assert_eq!(ch.iter().map(|c| c.conv_id.as_str()).collect::<Vec<_>>(), ["c1", "c2"]);
        assert_eq!(dm.iter().map(|c| c.conv_id.as_str()).collect::<Vec<_>>(), ["d2", "d1"]);
    }

    #[test]
    fn talk_count() {
        let you = m("person", "person:me", "you");
        let l = live(&[("heron", "idle"), ("ended", "gone")]);
        let cs = [
            conv(|c| {
                c.kind = "dm".into();
                c.unread = 2;
                c.members = vec![you.clone(), agent("heron")]
            }),
            conv(|c| {
                c.kind = "dm".into();
                c.members = vec![you.clone(), agent("heron")]
            }),
            conv(|c| {
                c.kind = "dm".into();
                c.unread = 3;
                c.read_only = true;
                c.members = vec![agent("heron"), agent("otter")]
            }),
            conv(|c| {
                c.kind = "channel".into();
                c.unread = 5
            }),
            conv(|c| {
                c.kind = "channel".into();
                c.unread = 2;
                c.mentions = 1
            }),
        ];
        assert_eq!(talk_waiting(&cs, &l), 2);
    }

    #[test]
    fn a_dm_with_a_gone_agent_leaves_the_count() {
        let you = m("person", "person:me", "you");
        let l = live(&[("heron", "idle"), ("ended", "gone")]);
        let gone = conv(|c| {
            c.kind = "dm".into();
            c.unread = 1;
            c.members = vec![you.clone(), agent("tulip")]
        });
        let ended = conv(|c| {
            c.kind = "dm".into();
            c.unread = 1;
            c.members = vec![you.clone(), agent("ended")]
        });
        assert!(dm_gone(&gone, &l));
        assert!(dm_gone(&ended, &l));
        assert_eq!(talk_waiting(&[gone.clone(), ended], &l), 0);
        assert!(!dm_gone(
            &conv(|c| {
                c.kind = "dm".into();
                c.members = vec![you.clone()]
            }),
            &l
        ));
        assert_eq!(talk_waiting(&[gone], &live(&[("tulip", "working")])), 1);
    }

    #[test]
    fn make_a_card_from_a_message() {
        let (title, obj) = from_post("Tag 0.10.0 once #96 merges\nnot before", "#releases", &[]);
        assert_eq!(title, "Tag 0.10.0 once #96 merges");
        assert_eq!(obj, "Tag 0.10.0 once #96 merges\nnot before\n\n(From a message in #releases.)");
        let (title, _) = from_post(&"x".repeat(200), "a DM", &[]);
        assert_eq!(title.chars().count(), 80);
        assert!(title.ends_with('…'));
        let sha = "a".repeat(64);
        let files = [
            PostFile { name: "shot.png", size: 10, sha256: &sha },
            PostFile { name: "log.txt", size: 10, sha256: &sha },
        ];
        let (title, obj) = from_post("", "#suzuri", &files);
        assert_eq!(title, "shot.png");
        assert_eq!(
            obj,
            format!("file: shot.png (10 B, sha256={sha})\nfile: log.txt (10 B, sha256={sha})\n\n(From a message in #suzuri.)")
        );
    }

    #[test]
    fn post_line_is_the_text_else_the_files() {
        assert_eq!(post_line(None), "");
        let p = TalkPost {
            text: " a\n b ".into(),
            ..TalkPost::default()
        };
        assert_eq!(post_line(Some(&p)), "a b");
        assert_eq!(text_line("", &["x.png", "y.png"]), "📎 x.png +1");
        assert_eq!(text_line("  ", &["x.png"]), "📎 x.png");
    }

    #[test]
    fn a_dm_with_another_person() {
        let bob = conv(|c| {
            c.kind = "dm".into();
            c.members = vec![m("person", "user:dev1", "you"), m("person", "person:bob", "Bob")]
        });
        assert_eq!(dm_person(&bob).map(|m| m.key.as_str()), Some("person:bob"));
        let with_agent = conv(|c| {
            c.kind = "dm".into();
            c.members = vec![m("person", "user", "you"), m("agent", "api:a1", "A1")]
        });
        assert!(dm_person(&with_agent).is_none());
        let sender = conv(|c| {
            c.kind = "dm".into();
            c.members = vec![m("person", "user", "you"), m("person", "board:b1/person:sam", "Sam")]
        });
        assert!(dm_person(&sender).is_none());
        let ch = conv(|c| {
            c.kind = "channel".into();
            c.members = bob.members.clone()
        });
        assert!(dm_person(&ch).is_none());
        assert_eq!(waiting_text("them", "Bob"), "Waiting for Bob's Irori");
        assert_eq!(waiting_text("update", "Bob"), "Bob's Irori needs updating");
        assert_eq!(waiting_text("", "Bob"), "");
        let bob2 = conv(|c| {
            c.kind = "dm".into();
            c.members = vec![m("person", "user", "you"), m("person", "person:bob", "Bob")]
        });
        assert!(!dm_gone(&bob2, &HashMap::new()));
    }

    fn replier(name: &str, secs: i64, key: &str, now: i64) -> TalkReplier {
        TalkReplier {
            key: key.into(),
            name: name.into(),
            since: GoTime::unix_milli(now - secs * 1000),
        }
    }

    #[test]
    fn replying_lines() {
        let now = t("2026-11-06T12:00:00Z").unix_milli_value();
        let a = |n: &str, s: i64| replier(n, s, &format!("k-{n}"), now);
        assert_eq!(replying_line(&[], now), None);
        assert_eq!(replying_line(&[a("jasmine", 5)], now).unwrap().text, "jasmine is replying…");
        assert_eq!(
            replying_line(&[a("jasmine", 5), a("kestrel", 5)], now),
            Some(ReplyLine {
                who: "jasmine and kestrel".into(),
                verb: "are",
                text: "jasmine and kestrel are replying…".into()
            })
        );
        assert_eq!(replying_line(&[a("a", 5), a("b", 5), a("c", 5)], now).unwrap().who, "a, b and c");
        assert_eq!(
            replying_line(&[a("jasmine", 31 * 60), a("kestrel", 60)], now).unwrap().text,
            "kestrel is replying…"
        );
        assert_eq!(replying_line(&[a("jasmine", 31 * 60)], now), None);
        let key = "3dc30c94c6a74af3c59d39f4ca02d6f1/7ff86a62-6cc4-4916-be2d-602034fc281a";
        assert_eq!(replying_line(&[a(key, 5), a(" ", 5)], now), None);
        assert_eq!(
            replying_line(&[replier("oak", 5, "k1", now), replier("oak", 5, "k1", now)], now).unwrap().text,
            "oak is replying…"
        );
    }

    #[test]
    fn the_person_in_a_dm_an_agent_opened() {
        let you = m("person", "user", "you");
        let jasmine = m("agent", "dev/s1", "jasmine");
        assert_eq!(member_of([&you, &jasmine], "user:mac"), Some(you.clone()));
        assert_eq!(member_of([&jasmine, &you], "user:phone"), Some(you.clone()));
        let mac = m("person", "user:mac", "you");
        assert_eq!(member_of([&you, &mac], "user:mac"), Some(mac.clone()));
        assert_eq!(member_of([&you, &jasmine], "dev/s1"), Some(jasmine.clone()));
        assert_eq!(member_of([&you, &jasmine], "dev/s2"), None);
        assert_eq!(member_of([&jasmine], "user:mac"), Some(m("person", "user:mac", "you")));
        assert_eq!(member_of([&jasmine], "person:bob"), None);
        assert!(is_you("user"));
        assert!(!is_you("person:bob"));
        assert!(!is_you("username/s1"));
    }

    // talk-sections.test.ts

    fn dm(id: &str, f: impl FnOnce(&mut TalkConversation)) -> TalkConversation {
        conv(|c| {
            c.conv_id = id.into();
            c.kind = "dm".into();
            c.name = id.into();
            f(c)
        })
    }

    fn ids(cs: &[TalkConversation]) -> Vec<&str> {
        cs.iter().map(|c| c.conv_id.as_str()).collect()
    }

    #[test]
    fn dms_split_three_ways() {
        let s = dm_sections(&[dm("a", |_| {}), dm("b", |c| c.read_only = true), dm("c", |_| {})], |c| c.conv_id == "c", "");
        assert_eq!(ids(&s.yours), ["a"]);
        assert_eq!(ids(&s.gone), ["c"]);
        assert_eq!(ids(&s.agents), ["b"]);
    }

    #[test]
    fn gone_dms_archive_except_the_kept_one() {
        let s = dm_sections(
            &[
                dm("read", |_| {}),
                dm("unread", |c| c.unread = 1),
                dm("named", |c| c.mentions = 1),
                dm("kept", |_| {}),
                dm("watched", |c| c.read_only = true),
            ],
            |_| true,
            "kept",
        );
        assert_eq!(ids(&s.yours), ["kept"]);
        assert_eq!(ids(&s.gone), ["read", "unread", "named"]);
        assert_eq!(ids(&s.agents), ["watched"]);
    }

    #[test]
    fn a_dm_whose_agent_came_back_leaves_the_archive() {
        let c = dm("back", |_| {});
        assert!(archived(&c, |_| true, ""));
        assert!(!archived(&c, |_| false, ""));
        assert_eq!(ids(&dm_sections(&[c], |_| false, "").yours), ["back"]);
    }

    #[test]
    fn folded_by_default() {
        let got: Vec<bool> = SectionKey::ALL.iter().map(|k| k.folded_by_default()).collect();
        assert_eq!(got, [false, false, false, true, true]);
    }

    #[test]
    fn a_folded_section_lists_no_rows() {
        let items = [
            dm("quiet", |_| {}),
            dm("new", |c| c.unread = 2),
            dm("named", |c| {
                c.mentions = 1;
                c.unread = 1
            }),
            dm("open", |_| {}),
            dm("watched", |c| {
                c.read_only = true;
                c.unread = 9
            }),
        ];
        let (rows, hidden) = section_rows(&items, true, "open", false);
        assert!(rows.is_empty());
        assert_eq!(hidden, 5);
        assert_eq!(unread_in(&items), 2);
        assert!(section_rows(&items, true, "", true).0.is_empty());
    }

    #[test]
    fn an_unfolded_section_keeps_the_open_one() {
        let items: Vec<TalkConversation> = (0..20).map(|i| dm(&format!("c{i}"), |_| {})).collect();
        let (rows, hidden) = section_rows(&items, false, "c15", false);
        assert_eq!(rows.len(), SHOWN_AT_ONCE + 1);
        assert_eq!(rows.last().unwrap().conv_id, "c15");
        assert_eq!(hidden, 20 - SHOWN_AT_ONCE - 1);
        let (rows, hidden) = section_rows(&items, false, "", true);
        assert_eq!(rows.len(), 20);
        assert_eq!(hidden, 0);
    }

    #[test]
    fn the_archive_header() {
        let items = [dm("a", |c| c.unread = 1), dm("b", |c| c.unread = 4), dm("c", |_| {})];
        assert_eq!(section_header(SectionKey::Gone, true, &items), (0, false));
        assert_eq!(section_header(SectionKey::Gone, false, &items), (2, true));
        assert_eq!(section_header(SectionKey::Gone, false, &[dm("c", |_| {})]), (0, false));
        for k in [SectionKey::Threads, SectionKey::Channels, SectionKey::Dms, SectionKey::Agents] {
            assert_eq!(section_header(k, true, &items), (2, false));
            assert_eq!(section_header(k, false, &items), (2, false));
        }
        assert_eq!(
            unread_in(&[dm("a", |c| c.unread = 1), dm("b", |c| {
                c.read_only = true;
                c.unread = 3
            }), dm("c", |_| {})]),
            1
        );
    }

    // talk-broadcasts.test.ts

    const T0: &str = "2026-11-01T18:31:54Z";

    fn t0() -> i64 {
        t(T0).unix_milli_value()
    }

    fn named(key: &str, name: &str) -> TalkMember {
        m("agent", key, name)
    }

    fn copy(to: &str, sec: i64, text: &str, by: &TalkMember) -> TalkConversation {
        conv(|c| {
            c.conv_id = format!("dm:{}:{to}", by.key);
            c.kind = "dm".into();
            c.name = format!("{} ↔ {to}", by.name);
            c.members = vec![by.clone(), named(&format!("dev/{to}"), to)];
            c.read_only = true;
            c.last_at = GoTime::unix_milli(t0() + sec * 1000);
            c.last_text = text.into();
            c.last_by = by.key.clone();
        })
    }

    fn owl() -> TalkMember {
        named("dev/owl", "owl")
    }

    fn newest_first(mut cs: Vec<TalkConversation>) -> Vec<TalkConversation> {
        cs.sort_by_key(|c| std::cmp::Reverse(at_ms(&c.last_at)));
        cs
    }

    fn shape(rows: &[AgentRow]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                AgentRow::Dm(c) => c.conv_id.clone(),
                AgentRow::Broadcast(b) => format!("{} → {}", b.by, b.convs.len()),
            })
            .collect()
    }

    #[test]
    fn one_message_to_sixteen_agents_is_one_row() {
        let names: Vec<String> = (0..16).map(|i| format!("a{i}")).collect();
        let heron = named("dev/heron", "heron");
        let mut cs: Vec<TalkConversation> =
            names.iter().enumerate().map(|(i, n)| copy(n, i as i64 * 2, "WIND DOWN NOW", &owl())).collect();
        cs.push(copy("zed", 100, "Rebased onto yours.", &heron));
        let rows = agent_rows(&newest_first(cs));
        assert_eq!(shape(&rows), ["dm:dev/heron:zed", "dev/owl → 16"]);
        let AgentRow::Broadcast(b) = &rows[1] else { panic!("a broadcast") };
        assert_eq!(b.text, "WIND DOWN NOW");
        assert_eq!(b.last_at, t0() + 30_000);
        let mut rev = names.clone();
        rev.reverse();
        assert_eq!(broadcast_to(b).iter().map(|m| m.name.clone()).collect::<Vec<_>>(), rev);
    }

    #[test]
    fn an_answered_dm_keeps_its_own_row() {
        let mut answered = copy("a2", 4, "WIND DOWN NOW", &owl());
        answered.last_text = "Done, going idle.".into();
        answered.last_by = "dev/a2".into();
        answered.last_at = GoTime::unix_milli(t0() + 60_000);
        let id = answered.conv_id.clone();
        let rows = agent_rows(&newest_first(vec![
            copy("a0", 0, "WIND DOWN NOW", &owl()),
            copy("a1", 2, "WIND DOWN NOW", &owl()),
            answered,
        ]));
        assert_eq!(shape(&rows), [id, "dev/owl → 2".into()]);
    }

    #[test]
    fn later_or_from_someone_else_is_not_the_same_broadcast() {
        let later = copy("a2", BROADCAST_WINDOW / 1000 + 60, "WIND DOWN NOW", &owl());
        let from_heron = copy("a3", 1, "WIND DOWN NOW", &named("dev/heron", "heron"));
        let (l, h) = (later.conv_id.clone(), from_heron.conv_id.clone());
        let rows = agent_rows(&newest_first(vec![
            copy("a0", 0, "WIND DOWN NOW", &owl()),
            copy("a1", 2, "WIND DOWN NOW", &owl()),
            later,
            from_heron,
        ]));
        assert_eq!(shape(&rows), [l, "dev/owl → 2".into(), h]);
    }

    #[test]
    fn a_broadcast_keeps_its_id() {
        let ids = |rows: &[AgentRow]| -> Vec<String> {
            rows.iter()
                .filter_map(|r| match r {
                    AgentRow::Broadcast(b) => Some(b.id.clone()),
                    _ => None,
                })
                .collect()
        };
        let o = owl();
        let before = agent_rows(&newest_first(vec![
            copy("a0", 0, "WIND DOWN NOW", &o),
            copy("a1", 2, "WIND DOWN NOW", &o),
            copy("a2", 4, "WIND DOWN NOW", &o),
        ]));
        let mut answered = copy("a2", 4, "WIND DOWN NOW", &o);
        answered.last_text = "Done.".into();
        answered.last_by = "dev/a2".into();
        answered.last_at = GoTime::unix_milli(t0() + 60_000);
        let after = agent_rows(&newest_first(vec![
            copy("a0", 0, "WIND DOWN NOW", &o),
            copy("a1", 2, "WIND DOWN NOW", &o),
            answered,
            copy("a3", 6, "WIND DOWN NOW", &o),
        ]));
        assert_eq!(ids(&after), ids(&before));
    }

    #[test]
    fn lone_copies_and_own_dms_stay_dms() {
        let head = "Stephen asked the Mac agents to test and finish. ".repeat(8);
        let rows = agent_rows(&newest_first(vec![
            copy("a0", 0, &format!("{head}take #470"), &owl()),
            copy("a1", 2, &format!("{head}take #471"), &owl()),
        ]));
        assert!(rows.iter().all(|r| matches!(r, AgentRow::Dm(_))));
        let mut mine = copy("a1", 1, "WIND DOWN NOW", &owl());
        mine.read_only = false;
        let mut blank = copy("a2", 2, "WIND DOWN NOW", &owl());
        blank.last_text = String::new();
        let rows = agent_rows(&newest_first(vec![copy("a0", 0, "WIND DOWN NOW", &owl()), mine, blank]));
        assert!(rows.iter().all(|r| matches!(r, AgentRow::Dm(_))));
    }

    fn many(n: i64) -> Vec<AgentRow> {
        agent_rows(&(0..n).map(|i| copy(&format!("x{i}"), -i, &format!("msg {i}"), &owl())).collect::<Vec<_>>())
    }

    #[test]
    fn rows_and_the_cap() {
        let rows = many(80);
        let some = agent_section_rows(&rows, false, "", false, false);
        assert_eq!(some.rows.len(), SHOWN_AT_ONCE);
        assert_eq!(some.hidden, AGENTS_CAP - SHOWN_AT_ONCE);
        let all = agent_section_rows(&rows, false, "", true, false);
        assert_eq!(all.rows.len(), AGENTS_CAP);
        assert_eq!(all.hidden, 0);
        assert_eq!(all.capped, 80 - AGENTS_CAP);
    }

    #[test]
    fn folded_open_past_the_cap_and_search() {
        let rows = many(80);
        let deep = rows[70].id().to_string();
        assert!(agent_section_rows(&rows, true, &deep, false, false).rows.is_empty());
        assert!(agent_section_rows(&rows, true, "", false, false).rows.is_empty());
        let some = agent_section_rows(&rows, false, &deep, false, false);
        assert_eq!(some.rows.last().unwrap().id(), deep);
        assert_eq!(some.hidden, AGENTS_CAP - SHOWN_AT_ONCE);
        let all = agent_section_rows(&rows, false, &deep, true, false);
        assert_eq!(all.rows.last().unwrap().id(), deep);
        assert_eq!(all.capped, 80 - AGENTS_CAP - 1);
        assert_eq!(agent_section_rows(&rows, false, "", false, true).rows.len(), 80);
    }

    #[test]
    fn a_broadcast_counts_as_one_row() {
        let rows = agent_rows(&newest_first(vec![
            copy("a0", 0, "WIND DOWN NOW", &owl()),
            copy("a1", 2, "WIND DOWN NOW", &owl()),
        ]));
        assert_eq!(rows.len(), 1);
        assert_eq!(agent_section_rows(&rows, false, "dm:dev/owl:a0", false, false).rows.len(), 1);
    }

    #[test]
    fn a_rows_subtitle_is_who_said_what_and_when() {
        let now = t0() + 5 * 60_000;
        let c = copy("a0", 0, "Rebased   #412\nonto yours.", &owl());
        assert_eq!(last_line(&c, now), ("owl: Rebased #412 onto yours.".into(), "5m ago".into()));
        let you = m("person", "person:me", "you");
        let mine = conv(|c| {
            c.conv_id = "d".into();
            c.kind = "dm".into();
            c.name = "heron".into();
            c.members = vec![you.clone(), named("dev/heron", "heron")];
            c.last_at = GoTime::unix_milli(t0());
            c.last_text = "Is #470 on main?".into();
            c.last_by = you.key.clone();
        });
        assert_eq!(last_line(&mine, now), ("You: Is #470 on main?".into(), "5m ago".into()));
        let mut opened = mine.clone();
        opened.members = vec![m("person", "user", "you"), named("dev/heron", "heron")];
        opened.last_by = "user:mac".into();
        assert_eq!(last_line(&opened, now).0, "You: Is #470 on main?");
        let mut blank = mine.clone();
        blank.last_text = String::new();
        blank.last_at = GoTime::default();
        assert_eq!(last_line(&blank, now), (String::new(), String::new()));
    }

    // talk-page.test.ts

    fn thread(f: impl FnOnce(&mut ThreadSummary)) -> ThreadSummary {
        let mut t = ThreadSummary {
            id: "x".into(),
            short: "12".into(),
            title: "Why is the Mac build slow?".into(),
            text: "It takes ten minutes since Tuesday".into(),
            state: "waiting".into(),
            ..ThreadSummary::default()
        };
        f(&mut t);
        t
    }

    #[test]
    fn talk_search() {
        assert!(talk_matches(&thread(|_| {}), "mac build"));
        assert!(talk_matches(&thread(|_| {}), "TUESDAY"));
        assert!(talk_matches(&thread(|_| {}), "#12"));
        assert!(talk_matches(&thread(|t| t.last_text = "it was the linker".into()), "linker"));
        assert!(!talk_matches(&thread(|_| {}), "lobby"));
        assert!(talk_matches(&thread(|_| {}), "  "));
        assert!(talk_matches(&thread(|t| t.tag = "mlg".into()), "mlg#12"));
        assert!(talk_matches(&thread(|t| t.tag = "mlg".into()), "#12"));
        assert!(!talk_matches(&thread(|_| {}), "mlg#12"));
    }

    #[test]
    fn talks_count_in_the_sidebar() {
        assert!(needs_you("answered", ""));
        for st in ["waiting", "handed", "resolved"] {
            assert!(!needs_you(st, ""));
        }
    }

    #[test]
    fn channel_names_are_cleaned() {
        assert_eq!(channel_name("  #Release Notes! "), "release-notes");
        assert_eq!(channel_name("###"), "");
    }

    #[test]
    fn search_matches_a_channel_or_member() {
        let c = conv(|c| {
            c.kind = "dm".into();
            c.name = "dm".into();
            c.topic = Some("the move".into());
            c.members = vec![m("agent", "k", "kowhai")];
        });
        assert!(conv_matches(&c, "KOW"));
        assert!(conv_matches(&c, "move"));
        assert!(!conv_matches(&c, "lobby"));
    }

    #[test]
    fn day_lines() {
        assert_eq!(day_label((2026, 11, 3), (2026, 11, 3)), "Today");
        assert_eq!(day_label((2026, 11, 2), (2026, 11, 3)), "Yesterday");
        let far = day_label((2026, 10, 30), (2026, 11, 3));
        assert!(far != "Today" && far != "Yesterday");
        assert_eq!(far, "Friday, Oct 30");
        assert_eq!(day_label((2026, 11, 3), (2026, 11, 10)), "Tuesday, Nov 3");
    }

    #[test]
    fn members_read_their_status() {
        let l = live(&[("d/oak", "shell"), ("d/elm", "unresponsive"), ("d/fir", "starting")]);
        let say = |st: &str| status_word(st);
        let ag = |k: &str| say(&status_of(&m("agent", k, ""), &l));
        assert_eq!(say("busy"), "working");
        assert_eq!(say("idle"), "idle");
        assert_eq!(say(&status_of(&m("person", "person:p1", ""), &l)), "");
        assert_eq!(ag("d/oak"), "running a job");
        assert_eq!(ag("d/elm"), "not answering");
        assert_eq!(ag("d/fir"), "starting");
        assert_eq!(ag("d/ash"), "gone");
    }

    #[test]
    fn another_members_agent() {
        let mut sp = m("agent", "board:b1/c1", "BillingEng1 · Spencer");
        sp.owner = "Spencer".into();
        let l = HashMap::new();
        assert_eq!(status_of(&sp, &l), "unknown");
        assert_eq!(member_status(&sp, &l, 0), "Spencer's agent");
        sp.status = "idle".into();
        assert_eq!(member_status(&sp, &l, 0), "Spencer's agent · idle");
        assert_eq!(whose_word(&sp, "busy"), "Spencer's agent · working");
    }

    #[test]
    fn mention_kinds_by_key() {
        assert_eq!(mention_kinds(["person:p1", "dev/s1"]), ["person", "agent"]);
    }

    #[test]
    fn a_thread_from_a_channel_message() {
        assert_eq!(
            thread_text(" Why the Firewall? ", "kowhai", "Only the PC transfer.\nOne click.", "suzuri"),
            "Why the Firewall?\n\n> Only the PC transfer.\n> One click.\n(kowhai, in #suzuri)"
        );
    }

    // threads.test.ts

    const THREADS: [(&str, &str); 2] = [("live", "open"), ("done", "resolved")];

    #[test]
    fn the_resolved_fold() {
        assert!(resolved_shown(false, "done", &THREADS));
        assert!(!resolved_shown(false, "live", &THREADS));
        assert!(!resolved_shown(false, "", &THREADS));
        let (pick, close) = resolved_toggle(false, "done", &THREADS);
        assert_eq!((pick, close), (false, true));
        assert!(!resolved_shown(pick, "", &THREADS));
        assert_eq!(resolved_toggle(true, "live", &THREADS), (false, false));
        assert_eq!(resolved_toggle(false, "", &THREADS), (true, false));
        assert_eq!(resolved_toggle(false, "live", &THREADS), (true, false));
    }

    #[test]
    fn resolving_a_thread() {
        let open = open_after_resolve("done", "done");
        assert_eq!(open, "");
        assert!(!resolved_shown(false, &open, &THREADS));
        assert_eq!(open_after_resolve("live", "done"), "live");
    }

    fn waiting() -> ThreadSummary {
        ThreadSummary {
            id: "t".into(),
            short: "27".into(),
            title: "What did we ship?".into(),
            text: "What did we ship?".into(),
            state: "waiting".into(),
            agents_now: true,
            ..ThreadSummary::default()
        }
    }

    #[test]
    fn a_thread_irori_starts_an_agent_for() {
        let mut w = waiting();
        w.starting = "Macbook Pro (this one)".into();
        assert_eq!(state_line(&w, "No agent takes offered work"), "starting an agent on Macbook Pro (this one)");
        let mut w = waiting();
        w.cant_start = "not trusted: /r on mac".into();
        assert_eq!(state_line(&w, ""), "can't start: its repo isn't trusted here");
        assert_eq!(state_line(&waiting(), ""), "Waiting for an agent");
        let why = "No agent can start: Mac is offline. It starts by itself when it's back.";
        let mut a = waiting();
        a.state = "answered".into();
        a.last_by = Some(AgentRef {
            key: "user:x".into(),
            ..AgentRef::default()
        });
        a.last_text = "And?".into();
        a.no_agent = why.into();
        assert_eq!(state_line(&a, ""), why);
        let mut w = waiting();
        w.no_agent = why.into();
        assert_eq!(state_line(&w, ""), why);
        assert_eq!(empty_line(&w), why);
        assert!(empty_line(&waiting()).contains("when spawning is on"));
    }

    #[test]
    fn a_thread_an_agent_opened() {
        let alpha = AgentRef {
            key: "dev/a".into(),
            name: "alpha".into(),
            status: "idle".into(),
            ..AgentRef::default()
        };
        let mut o = waiting();
        o.state = "answered".into();
        o.last_by = Some(alpha.clone());
        o.last_text = "Which fix?".into();
        o.opened_by = Some(alpha.clone());
        assert_eq!(state_line(&o, ""), "alpha: Which fix?");
        assert_eq!(opened_line(&alpha), "alpha opened this thread");
        assert_eq!(
            empty_opened(&alpha),
            "No replies yet. alpha hears yours; no other agent is started for it until you write."
        );
    }

    // talk.ts

    #[test]
    fn names_never_keys() {
        let mut names = crate::inbox::words::Names::default();
        for (k, n) in [("dev/a", "oak"), ("dev/a", "elm"), ("dev/b", "")] {
            names.add(k, n);
        }
        assert_eq!(names.name("dev/a"), "oak");
        assert_eq!(names.name("dev/b"), crate::logic::names::UNKNOWN_AGENT);
        assert_eq!(names.name("user:mac"), "you");
        assert_eq!(names.name(""), "");
    }

    #[test]
    fn counts_per_agent() {
        let c = counts_by_agent([("a", "queued"), ("a", "delivered"), ("a", "held"), ("b", "answered"), ("", "held")]);
        assert_eq!(c.get("a"), Some(&(2, 1)));
        assert!(!c.contains_key("b"));
    }
}
