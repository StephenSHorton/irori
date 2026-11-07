//! Talk (card #389; docs/design/talk-channels.md §2, §3, §7): the
//! `talk_*` tools, and the conversation views the window reads through
//! the Core, so agents and the person see the same thing.
//!
//! - Channels and DMs are `talk.*` rows ([`coord::talk`]).
//! - A thread's conversation is its task's: the ask, then its comments,
//!   mapped into posts. Posting to one goes through the card's comment
//!   path (`board_comment`), so the ask fold sees the reply. A thread lives
//!   in the private log or a space's (card #558): a read routes by its id,
//!   and shows its keys as this device knows them.
//! - Messages agents send each other (`send`/`reply`, `message.sent`) show
//!   as DMs between the two, read-only for the person; so do messages
//!   between the person and an agent. They are mirrored when read, never
//!   written as posts. A message that is itself a post (a DM to an agent
//!   goes out as a message whose id is the post's) shows once. The
//!   person's Wind down (reason wind_down) and any reply to it are not
//!   mirrored (card #484): one Wind down reaches every agent, so it would
//!   open a DM per agent, and its "wound down" answers belong to the Wind
//!   down dialog and Home's summary line, not to unread DMs.
//! - A mention in a channel or a DM wakes the mentioned agent with a
//!   mention notice, and an agent mentioned in a channel joins it. A post
//!   to a DM with an agent goes to it as a message that wakes it.
//! - A DM with another person rides on a hidden space (card #538): it wakes
//!   nobody, agents can't read or write it, and a person can't DM another
//!   person's agent.
//! - "Agents answer now" is stored on each post; what it changes is card
//!   #391's.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use coord::mentions::{Miss, Resolved};
use coord::talk::{self as ct, Member, NewPost, Post, Talk};
use coord::{Coord, Mention, NOTICE_MENTIONED, Notice, Task};
use gocompat::time::GoTime;
use inbox::notice::NoticeResult;
use inbox::{Address, Delivery, SendOptions, Sender, addr, mode};
use serde::Serialize;

use crate::args::Ty;
use crate::format::{ago, one_line, trim};
use crate::{Backend, Caller, Inner, decode, err_text, untrusted};

/// The most posts talk_read shows at once.
const READ_PAGE: usize = 30;
/// How much of each post talk_read shows.
const READ_TEXT: usize = 1200;

/// One member of a conversation, with the name it goes by.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvMember {
    pub kind: String,
    pub key: String,
    pub name: String,
    /// Another member's agent on a shared board (card #478): its board
    /// card's status (`busy`, `idle`, `gone`), "" when this device knows
    /// none. The window reads an agent missing from its own roster as
    /// gone only when this says so.
    pub status: String,
    /// Whose agent it is, when it is another member's ("Spencer"); ""
    /// for the person's own agents.
    pub owner: String,
}

/// One conversation in a reader's list.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub conv_id: String,
    /// `thread`, `channel` or `dm`.
    pub kind: String,
    pub name: String,
    pub topic: String,
    pub unread: usize,
    pub mentions: usize,
    pub members: Vec<ConvMember>,
    /// The newest post's time (zero: none yet).
    pub last_at: GoTime,
    /// The newest post's text on one line, clipped to [`LAST_TEXT`] runes
    /// (card #486): the list's subtitle, and how it sees one message sent
    /// to many agents.
    pub last_text: String,
    /// Who wrote the newest post ("" when none).
    pub last_by: String,
    /// The reader is in it (a channel they joined, their own DM, a thread
    /// they're on). The person also sees agents' DMs, read-only.
    pub joined: bool,
    /// The reader can only read it: a DM between two others.
    pub read_only: bool,
    /// A thread's short ref, for agents: `12` in the private log, `mlg#12`
    /// in a space's, whose bare number could be another log's.
    pub short: String,
    /// A thread's "Agents answer now" (card #391); false for a channel or
    /// DM, whose posts each carry their own.
    pub agents_now: bool,
    /// The space a channel or thread is in (card #461, #558): a shared
    /// space's id (its first board's), empty for Personal. A private thread
    /// about a space is that space's. DMs are always empty: a DM shows under
    /// every space, even when its rows ride on a hidden space (card #538).
    pub space_id: String,
    /// A DM with another person that hasn't reached them yet (card #538):
    /// [`WAITING_FOR_THEM`] or [`WAITING_FOR_UPDATE`], else "".
    pub waiting: String,
}

/// A DM's first posts wait for one of the other person's devices.
pub const WAITING_FOR_THEM: &str = "them";
/// The other person's devices that are online are too old to take a DM.
pub const WAITING_FOR_UPDATE: &str = "update";

/// Why an agent can't DM another person (card #538).
const NO_DM_TO_PEOPLE: &str =
    "agents don't DM other people: reach them on a card or in a channel you share";
/// Why nobody can DM another person's agent (card #538).
const NO_DM_TO_THEIR_AGENT: &str =
    "nobody DMs other people's agents: reach them on a card or in a channel you share";

/// What a post came to.
#[derive(Debug, Clone, Default)]
pub struct Posted {
    pub post: Post,
    /// The mentioned agents' notices.
    pub told: Vec<NoticeResult>,
    /// Mentions as resolved, and the names that resolved to no agent.
    pub mentions: Vec<Mention>,
    pub misses: Vec<Miss>,
    /// A DM to an agent: the message that woke it.
    pub sent: Option<Delivery>,
}

/// What a `conv` argument names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Channel(String),
    /// A DM's id, and the other party when the argument named it.
    Dm(String, Option<Member>),
    /// A card's or thread's task id.
    Thread(String),
    /// A huddle's transcript, `call:<id>` (card #656): read-only.
    Call(String),
}

impl Target {
    pub fn id(&self) -> &str {
        match self {
            Target::Channel(id) | Target::Dm(id, _) | Target::Thread(id) | Target::Call(id) => id,
        }
    }
}

/// What a huddle's transcript can't be.
const CALL_READ_ONLY: &str = "a huddle's transcript is read-only";

fn talk_of(b: &dyn Backend) -> Result<&Arc<Talk>, String> {
    b.talk()
        .ok_or_else(|| "Talk is off on this Core".to_string())
}

/// The roster as coord's mention resolver reads it.
fn cards(b: &dyn Backend) -> Vec<coord::roster::AgentCard> {
    b.agents()
        .into_iter()
        .map(|a| coord::roster::AgentCard {
            key: a.key,
            device_id: a.device_id,
            name: a.name,
            cwd: a.cwd,
            status: a.status,
            repo_key: a.repo_key,
            ..Default::default()
        })
        .collect()
}

/// Names for keys: the roster's, else another member's agent on a shared
/// board, else the last name an agent went by. A key never stands in for
/// a name (card #478): one nobody can name is "unknown agent".
struct Book {
    cards: HashMap<String, String>,
    /// Other members' agents, by board card id: name, owner, status.
    board: HashMap<String, (String, String, String)>,
    known: BTreeMap<String, String>,
}

/// What a name says when nothing names the key (card #478).
pub const UNKNOWN_AGENT: &str = "unknown agent";
/// What another person on a board is called when the board doesn't say.
pub const UNKNOWN_PERSON: &str = "another person";

/// The board card id a key names: `agent:<cardId>` as a board writes it,
/// or `board:<boardId>/<cardId>` as the bridge sends from it.
fn board_card(key: &str) -> Option<&str> {
    if let Some(id) = key.strip_prefix(store::kinds::AGENT_ACTOR_PREFIX) {
        return Some(id);
    }
    let (_, rest) = key.strip_prefix("board:")?.split_once('/')?;
    (!rest.starts_with(store::kinds::PERSON_ACTOR_PREFIX)).then_some(rest)
}

/// The person a `board:<boardId>/person:<personId>` key names, as
/// `person:<personId>`.
fn board_person(key: &str) -> Option<&str> {
    let (_, rest) = key.strip_prefix("board:")?.split_once('/')?;
    rest.starts_with(store::kinds::PERSON_ACTOR_PREFIX)
        .then_some(rest)
}

/// "BillingEng1 · Spencer"; "an agent of Spencer's" without a name.
pub fn board_agent_name(name: &str, owner: &str) -> String {
    match (name.trim(), owner.trim()) {
        ("", "") => UNKNOWN_AGENT.into(),
        ("", o) => format!("an agent of {o}'s"),
        (n, "") => n.into(),
        (n, o) => format!("{n} · {o}"),
    }
}

impl Book {
    fn new(b: &dyn Backend) -> Book {
        Book {
            cards: b
                .agents()
                .into_iter()
                .map(|a| (a.key.clone(), a.name))
                .collect(),
            board: b
                .board_agents()
                .into_iter()
                .filter_map(|a| {
                    let id = board_card(&a.card.key)?.to_string();
                    Some((id, (a.card.name, a.person, a.card.status)))
                })
                .collect(),
            known: b.known_names(),
        }
    }

    fn known(&self, key: &str) -> Option<String> {
        self.known
            .get(key)
            .filter(|n| !n.trim().is_empty())
            .cloned()
    }

    fn name(&self, key: &str) -> String {
        if ct::is_person_key(key) {
            return "you".into();
        }
        if let Some(n) = self.cards.get(key).filter(|n| !n.trim().is_empty()) {
            return n.clone();
        }
        if let Some(p) = board_person(key) {
            return self.known(p).unwrap_or_else(|| UNKNOWN_PERSON.into());
        }
        if ct::is_other_person_key(key) {
            return self.known(key).unwrap_or_else(|| UNKNOWN_PERSON.into());
        }
        if let Some(id) = board_card(key) {
            let last = || self.known(&format!("{}{id}", store::kinds::AGENT_ACTOR_PREFIX));
            if let Some((n, owner, _)) = self.board.get(id) {
                let n = if n.trim().is_empty() {
                    last().unwrap_or_default()
                } else {
                    n.clone()
                };
                return board_agent_name(&n, owner);
            }
            return last().unwrap_or_else(|| UNKNOWN_AGENT.into());
        }
        self.known(key).unwrap_or_else(|| UNKNOWN_AGENT.into())
    }

    fn member(&self, m: &Member) -> ConvMember {
        let (status, owner) = board_card(&m.key)
            .and_then(|id| self.board.get(id))
            .map(|(_, o, s)| (s.clone(), o.clone()))
            .unwrap_or_default();
        ConvMember {
            kind: m.kind.clone(),
            key: m.key.clone(),
            name: self.name(&m.key),
            status,
            owner,
        }
    }
}

/// What each key goes by, as a conversation's members are named: "you"
/// for the person, never a key (card #478). A huddle's people use it
/// (#654).
pub fn names(b: &dyn Backend, keys: &[String]) -> Vec<String> {
    if keys.is_empty() {
        return Vec::new();
    }
    let book = Book::new(b);
    keys.iter().map(|k| book.name(k)).collect()
}

fn is_thread(t: &Task) -> bool {
    t.ask.as_ref().is_some_and(|a| a.thread)
}

fn thread_open(t: &Task) -> bool {
    !t.withdrawn
        && !coord::is_terminal(&t.state)
        && t.ask.as_ref().is_some_and(|a| a.outcome.is_none())
}

/// A task's conversation: what it asked, then its comments (not the
/// messages recorded on it), as posts.
pub fn thread_posts(co: &Coord, t: &Task) -> Vec<Post> {
    let mut out = vec![Post {
        post_id: t.id.clone(),
        conv_id: t.id.clone(),
        by: t.created_by.clone(),
        text: t.objective.clone(),
        agents_now: t.ask.as_ref().is_some_and(|a| a.wake),
        at: t.created_at,
        ..Post::default()
    }];
    for c in co.comments(&t.id).unwrap_or_default() {
        if !c.msg_id.is_empty() {
            continue;
        }
        out.push(Post {
            post_id: c.id.clone(),
            conv_id: t.id.clone(),
            by: c.by.clone(),
            text: c.text.clone(),
            mentions: c.mentions.iter().map(|m| m.key.clone()).collect(),
            agents_now: c.wake,
            files: c
                .attachments
                .iter()
                .map(crate::attachments::from_coord)
                .collect(),
            at: c.at,
            ord: (c.hlc, c.device.clone(), c.seq),
            ..Post::default()
        });
    }
    ct::sort_posts(&mut out[1..]);
    out
}

/// A key of a thread's log as this device knows it: `agent:<cardId>` of
/// this person's own agent is its session, so "you wrote this" holds.
fn own_key(b: &dyn Backend, key: &str) -> String {
    b.router()
        .map_or_else(|| key.to_string(), |r| r.private_key(key))
}

/// [`thread_posts`] with its keys as this device knows them.
fn own_posts(b: &dyn Backend, co: &Coord, t: &Task) -> Vec<Post> {
    let mut ps = thread_posts(co, t);
    for p in &mut ps {
        p.by = own_key(b, &p.by);
        for m in &mut p.mentions {
            *m = own_key(b, m);
        }
    }
    ps
}

/// The logs threads live in: the private one (""), then each shared
/// space's by its id. Its inner boards share the space's log.
fn thread_logs(b: &dyn Backend) -> Vec<(String, Arc<Coord>)> {
    let mut out = vec![(String::new(), b.coord().clone())];
    if let Some(sb) = b.shared_boards() {
        for x in sb.boards() {
            // No keys: a key would put the reader on the board.
            if x.space_board.is_empty()
                && let Ok((co, _)) = sb.board_coord(&x.board_id, &[])
            {
                out.push((x.board_id, co));
            }
        }
    }
    out
}

/// The card `id` names and the log that holds it.
fn thread_log(b: &dyn Backend, id: &str) -> Option<(Arc<Coord>, Task)> {
    let co = match b.router() {
        Some(r) => r.task_coord(id, &[]).ok()?.0,
        None => b.coord().clone(),
    };
    let t = co.get_task(id)?;
    Some((co, t))
}

/// The card `conv` names, if it names one: a tagged ref or an id in the log
/// that holds it; a bare `#12` in space `space` when one is given, else
/// across every log, where a number two logs have is refused naming both
/// (card #558). It is never read as the private log's alone.
fn card_named(b: &dyn Backend, conv: &str, space: &str) -> Result<Option<String>, String> {
    let bare = coord::parse_ref(conv).is_some_and(|(tag, _)| tag.is_empty());
    let found = if bare && !space.is_empty() {
        let Some(Ok((co, _))) = b.shared_boards().map(|sb| sb.board_coord(space, &[])) else {
            return Ok(None);
        };
        co.task_ref(conv)
    } else {
        match b.router() {
            Some(r) => r.task_ref(conv),
            None => b.coord().task_ref(conv),
        }
    };
    match found {
        Ok(id) => Ok(thread_log(b, &id).map(|_| id)),
        Err(e) if e.is(coord::ErrorKind::Ambiguous) => Err(e.to_string()),
        Err(_) => Ok(None),
    }
}

/// Who a thread is between: whoever asked, the agents it went to, and
/// its holders and watchers.
fn thread_members(b: &dyn Backend, t: &Task) -> Vec<Member> {
    let mut keys = vec![t.created_by.clone()];
    if let Some(a) = &t.ask {
        keys.extend(a.heard.iter().cloned());
        keys.push(a.taker.clone());
    }
    keys.extend(t.holders.iter().cloned());
    keys.extend(t.watchers.iter().cloned());
    let mut out: Vec<Member> = Vec::new();
    for k in keys {
        let k = own_key(b, &k);
        if !k.is_empty() && !out.iter().any(|m| ct::canon(&m.key) == ct::canon(&k)) {
            out.push(Member::of(&k));
        }
    }
    out
}

/// Whether a message can show as a DM ([`mirrored`]): one an agent or the
/// person sent, not a notice, a stop, Irori's own, a mention's wake or a
/// Wind down (card #484). The Core tells the window Talk changed only for
/// these (card #468). A row bridged from a shared board (card #478) isn't
/// one either: it is the card's, shown on the card, and every holder's and
/// watcher's copy would show again ("raised a blocker on ..." ten times
/// over).
pub fn mirrors(m: &inbox::Message) -> bool {
    !(m.notice
        || m.stop
        || m.from.kind == "irori"
        || m.from.kind == inbox::BOARD_KIND
        || m.from.key.starts_with("board:")
        || m.reason == inbox::REASON_MENTIONED
        || inbox::is_wind_down(m)
        || m.from.key.is_empty())
}

/// Whether `m` answers a Wind down (`wind_downs` holds their ids): a
/// "wound down" sent as a message, as replies were before card #307. It
/// stays out of the DMs with the Wind down itself (card #484). Only a
/// reply counts: parent_id also links whatever an agent sends in the turn
/// the Wind down woke (a hand-off, a failure report), and those still show.
fn answers_wind_down(m: &inbox::Message, wind_downs: &std::collections::HashSet<String>) -> bool {
    !m.reply_to.is_empty() && wind_downs.contains(&m.reply_to)
}

/// The DMs mirrored from messages: by DM id, its two parties and posts.
fn mirrored(b: &dyn Backend, talk: &Talk) -> BTreeMap<String, (Vec<Member>, Vec<Post>)> {
    let ids = talk.post_ids();
    let mut seen = std::collections::HashSet::new();
    let mut out: BTreeMap<String, (Vec<Member>, Vec<Post>)> = BTreeMap::new();
    let all = b.inbox().list(usize::MAX);
    let wind_downs: std::collections::HashSet<String> = all
        .iter()
        .filter(|d| inbox::is_wind_down(&d.message))
        .map(|d| d.message.id.clone())
        .collect();
    for d in all {
        let m = &d.message;
        if !mirrors(m)
            || answers_wind_down(m, &wind_downs)
            || d.to.is_empty()
            || ct::canon(&m.from.key) == ct::canon(&d.to)
            || ids.contains(&m.id)
            || !seen.insert(m.id.clone())
        {
            continue;
        }
        let conv = ct::dm_id(&m.from.key, &d.to);
        let e = out
            .entry(conv.clone())
            .or_insert_with(|| (vec![Member::of(&m.from.key), Member::of(&d.to)], Vec::new()));
        e.1.push(Post {
            post_id: m.id.clone(),
            conv_id: conv,
            by: m.from.key.clone(),
            text: m.text.clone(),
            mentions: m.mentions.iter().map(|x| x.key.clone()).collect(),
            agents_now: d.mode == mode::WAKE,
            files: m
                .attachments
                .iter()
                .map(crate::attachments::from_inbox)
                .collect(),
            at: m.sent_at,
            ..Post::default()
        });
    }
    for (_, ps) in out.values_mut() {
        ct::sort_posts(ps);
    }
    out
}

/// A DM's posts: its rows and the messages mirrored into it.
fn dm_posts(talk: &Talk, mirror: Option<&(Vec<Member>, Vec<Post>)>, id: &str) -> Vec<Post> {
    let mut ps = talk.posts(id);
    if let Some((_, more)) = mirror {
        ps.extend(more.iter().cloned());
        ct::sort_posts(&mut ps);
    }
    ps
}

fn counts(talk: &Talk, posts: &[Post], reader: &str, conv: &str) -> (usize, usize) {
    ct::unread(posts, talk.cursor(reader, conv).as_deref(), reader)
}

fn last_at(posts: &[Post]) -> GoTime {
    posts.last().map(|p| p.at).unwrap_or_default()
}

/// How many runes of the newest post a conversation carries (card #486):
/// the list shows one line of it, and compares it whole to tell one
/// message sent to many agents from two that only start alike.
pub const LAST_TEXT: usize = 2000;

/// The newest post's text on one line, and who wrote it. A post of files
/// alone (card #643) shows their names.
fn last_line(posts: &[Post]) -> (String, String) {
    posts
        .last()
        .map(|p| (one_line(&post_line(p), LAST_TEXT), p.by.clone()))
        .unwrap_or_default()
}

/// A post's text, or, with none, its files as one line (card #643):
/// "📎 <first name>", plus " +N" for the rest, as the window's
/// `postLine` shows it. Everywhere a post is summed up (Talk's list, a
/// notification, a reading notice, a spawned agent's brief) uses it.
pub fn post_line(p: &Post) -> String {
    if !p.text.trim().is_empty() {
        return p.text.clone();
    }
    let Some(first) = p.files.first() else {
        return p.text.clone();
    };
    let mut s = format!("📎 {}", first.name);
    if p.files.len() > 1 {
        s += &format!(" +{}", p.files.len() - 1);
    }
    s
}

/// `reader`'s conversations: the person sees every open channel, every
/// DM (agents' own read-only) and every open thread; an agent sees the
/// channels it joined, its DMs and the open threads it is on, in the
/// private log and every space's. Newest first.
pub fn conversations(b: &dyn Backend, reader: &str) -> Result<Vec<Conversation>, String> {
    let talk = talk_of(b)?;
    let names = Book::new(b);
    let person = ct::is_person_key(reader);
    let me = ct::canon(reader).to_string();
    let mut out = Vec::new();

    for c in talk.channels() {
        let joined = talk.is_member(&c.id, reader);
        if c.archived || (!person && !joined) {
            continue;
        }
        let ps = talk.posts(&c.id);
        let (unread, mentions) = counts(talk, &ps, reader, &c.id);
        let (last_text, last_by) = last_line(&ps);
        out.push(Conversation {
            conv_id: c.id.clone(),
            kind: ct::KIND_CHANNEL.into(),
            name: c.name.clone(),
            topic: c.topic.clone(),
            unread,
            mentions,
            members: talk
                .members(&c.id)
                .iter()
                .map(|m| names.member(m))
                .collect(),
            last_at: last_at(&ps),
            last_text,
            last_by,
            joined,
            space_id: c.space_id.clone(),
            ..Conversation::default()
        });
    }

    let mirror = mirrored(b, talk);
    let mut dms: BTreeMap<String, Vec<Member>> = talk.dms().into_iter().collect();
    for (id, (parties, _)) in &mirror {
        dms.entry(id.clone()).or_insert_with(|| parties.clone());
    }
    for (id, parties) in dms {
        let mine = parties.iter().any(|m| ct::canon(&m.key) == me);
        if !mine && !person {
            continue;
        }
        let ps = dm_posts(talk, mirror.get(&id), &id);
        // A DM opened with a shared board's sender (from a Board notice)
        // and never written in lists nothing (card #478): what that
        // sender wrote is on the card, not here.
        if ps.is_empty() && parties.iter().any(|m| m.key.starts_with("board:")) {
            continue;
        }
        let (unread, mentions) = if mine {
            counts(talk, &ps, reader, &id)
        } else {
            (0, 0)
        };
        let others: Vec<String> = parties
            .iter()
            .filter(|m| ct::canon(&m.key) != me)
            .map(|m| names.name(&m.key))
            .collect();
        let (last_text, last_by) = last_line(&ps);
        let waiting = talk
            .pair_other(&id)
            .zip(b.shared_boards())
            .map(|(o, sb)| sb.dm_waiting(&o).to_string())
            .unwrap_or_default();
        out.push(Conversation {
            conv_id: id.clone(),
            kind: ct::KIND_DM.into(),
            name: others.join(", "),
            unread,
            mentions,
            members: parties.iter().map(|m| names.member(m)).collect(),
            last_at: last_at(&ps),
            last_text,
            last_by,
            joined: mine,
            read_only: !mine,
            waiting,
            ..Conversation::default()
        });
    }

    for (space, co) in thread_logs(b) {
        for t in co.list_tasks(&coord::TaskFilter::default()) {
            if !is_thread(&t) || !thread_open(&t) {
                continue;
            }
            let members = thread_members(b, &t);
            let ps = own_posts(b, &co, &t);
            // On it: asked, told, holding, watching or answered.
            let on = members.iter().any(|m| ct::canon(&m.key) == me)
                || ps.iter().any(|p| ct::canon(&p.by) == me);
            if !person && !on {
                continue;
            }
            let (unread, mentions) = counts(talk, &ps, reader, &t.id);
            let (last_text, last_by) = last_line(&ps);
            out.push(Conversation {
                conv_id: t.id.clone(),
                kind: ct::KIND_THREAD.into(),
                name: t.title.clone(),
                unread,
                mentions,
                members: members.iter().map(|m| names.member(m)).collect(),
                last_at: last_at(&ps),
                last_text,
                last_by,
                joined: on || person,
                short: if space.is_empty() {
                    t.short.clone()
                } else {
                    t.short_ref()
                },
                agents_now: t.ask.as_ref().is_some_and(|a| a.wake),
                space_id: if space.is_empty() {
                    t.ask.as_ref().map(|a| a.board.clone()).unwrap_or_default()
                } else {
                    space.clone()
                },
                ..Conversation::default()
            });
        }
    }

    out.sort_by(|a, b| coord_cmp(&b.last_at, &a.last_at).then_with(|| a.conv_id.cmp(&b.conv_id)));
    Ok(out)
}

fn coord_cmp(a: &GoTime, b: &GoTime) -> std::cmp::Ordering {
    a.unix_nano().cmp(&b.unix_nano())
}

/// The DM between `me` and another person: `key` is their `person:<id>`.
/// Agents have none (card #538).
fn person_dm(talk: &Talk, me: &str, key: &str) -> Result<Target, String> {
    if !ct::is_person_key(me) {
        return Err(NO_DM_TO_PEOPLE.into());
    }
    if talk.person_key().is_none() {
        return Err("DMs with other people aren't on here".into());
    }
    Ok(Target::Dm(talk.dm_of(me, key), Some(Member::of(key))))
}

/// What a key in `conv` comes to when it names someone who isn't `me`'s
/// own agent (card #538); `None`: it doesn't, or isn't a key. Your own
/// person key is the person. A board's `board:<id>/person:<id>` isn't a
/// person DM, so the person gets no target for it.
fn key_target(talk: &Talk, me: &str, conv: &str) -> Option<Result<Target, String>> {
    let key = conv.trim_start_matches('@');
    if talk.person_key().as_deref() == Some(key) {
        if ct::is_person_key(me) {
            return Some(Err(
                "that's you: a DM needs an agent or another person".into()
            ));
        }
        let p = Member::of("user");
        return Some(Ok(Target::Dm(ct::dm_id(me, &p.key), Some(p))));
    }
    if key.starts_with(store::kinds::PERSON_ACTOR_PREFIX) {
        return Some(person_dm(talk, me, key));
    }
    if ct::is_other_person_key(key) && !ct::is_person_key(me) {
        return Some(Err(NO_DM_TO_PEOPLE.into()));
    }
    board_card(key).map(|_| Err(NO_DM_TO_THEIR_AGENT.into()))
}

/// The people `name` names among those `me` can DM (card #538), any case.
fn people_named(b: &dyn Backend, name: &str) -> Vec<crate::PersonInfo> {
    let name = name.trim().to_lowercase();
    b.shared_boards()
        .map(|sb| sb.dm_people())
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.name.trim().to_lowercase() == name)
        .collect()
}

/// Whether `name` is what another person's agent goes by (card #538).
fn their_agent_named(b: &dyn Backend, name: &str) -> bool {
    let name = name.trim().to_lowercase();
    b.board_agents()
        .iter()
        .any(|a| a.card.name.trim().to_lowercase() == name)
}

/// The refusal for a name that fits more than one of `keys`.
fn many_named(name: &str, what: &str, keys: &[String]) -> String {
    format!(
        "{} names more than one {what}: give the key ({})",
        one_line(name, 80),
        keys.join(", ")
    )
}

/// What `conv` names for `me`: a channel or DM id, `#name` (a channel)
/// or `#12` / `task:<id>` (a thread or card), `@name` or an agent's key
/// (the DM with it), `person` (the DM with the person), or another person
/// by `person:<id>` or by name among those `me` shares a space with (the
/// DM with them; card #538). An agent is refused a person, and anyone is
/// refused another person's agent. A channel's name is looked up in space
/// `space` only (empty: Personal); a name that lives in another space is
/// refused with where it is, never followed there (card #461). `call:<id>`
/// is a huddle's transcript (card #656).
pub fn resolve(b: &dyn Backend, me: &str, conv: &str, space: &str) -> Result<Target, String> {
    let talk = talk_of(b)?;
    let conv = trim(conv);
    if conv.is_empty() {
        return Err(
            "give conv: a channel (#name), a DM (@agent, or person), or a thread (#12)".into(),
        );
    }
    if let Some(id) = conv.strip_prefix(ct::CALL_PREFIX) {
        return match talk.call_conv(id) {
            Some(_) => Ok(Target::Call(format!("{}{}", ct::CALL_PREFIX, id.trim()))),
            None => Err(format!(
                "no huddle {}. talk_list shows yours.",
                one_line(id, 80)
            )),
        };
    }
    if ct::is_channel(conv) {
        return match talk.channel(conv) {
            Some(c) => Ok(Target::Channel(c.id)),
            None => Err(format!(
                "no channel {}. talk_list shows yours.",
                one_line(conv, 80)
            )),
        };
    }
    if ct::is_dm(conv) {
        return Ok(Target::Dm(conv.to_string(), None));
    }
    if matches!(conv, "person" | "the person" | "user" | "you" | "@person") {
        let p = Member::of("user");
        return Ok(Target::Dm(ct::dm_id(me, &p.key), Some(p)));
    }
    if let Some(t) = key_target(talk, me, conv) {
        return t;
    }
    if let Some(name) = conv.strip_prefix('#')
        && let Some(id) = named_in(talk, name, space)?
    {
        return Ok(Target::Channel(id));
    }
    if let Some(id) = card_named(b, conv, space)? {
        return Ok(Target::Thread(id));
    }
    if let Some(id) = named_in(talk, conv, space)? {
        return Ok(Target::Channel(id));
    }
    // Only spaces this person still has can be named back (a space being
    // left, or not ready yet, isn't one).
    let elsewhere: Vec<ct::Channel> = talk
        .channels_named(conv)
        .into_iter()
        .filter(|c| c.space_id.is_empty() || space_known(b, &c.space_id))
        .collect();
    if !elsewhere.is_empty() {
        return Err(elsewhere_text(b, conv, space, &elsewhere));
    }
    let name = conv.trim_start_matches('@');
    let r = coord::mentions::resolve(&cards(b), me, "", &[name.to_string()]);
    let people = people_named(b, name);
    let human = ct::is_person_key(me);
    if let Some(m) = r.mentions.first() {
        let other = Member::of(&m.key);
        // The person's own agent and another person can share a name.
        if human && !people.is_empty() {
            let mut keys = vec![other.key];
            keys.extend(people.into_iter().map(|p| p.key));
            return Err(many_named(name, "agent or person", &keys));
        }
        return Ok(Target::Dm(ct::dm_id(me, &other.key), Some(other)));
    }
    if let Some(x) = r.misses.first()
        && x.reason == coord::mentions::MISS_AMBIGUOUS
    {
        return Err(format!(
            "{} names more than one agent: give its key (the agents tool lists them)",
            one_line(name, 80)
        ));
    }
    match people.as_slice() {
        [] => {}
        [one] => return person_dm(talk, me, &one.key),
        _ if !human => return Err(NO_DM_TO_PEOPLE.into()),
        many => {
            let keys: Vec<String> = many.iter().map(|p| p.key.clone()).collect();
            return Err(many_named(name, "person", &keys));
        }
    }
    if their_agent_named(b, name) {
        return Err(NO_DM_TO_THEIR_AGENT.into());
    }
    Err(format!(
        "{} is no channel, thread or agent here. talk_list shows your conversations; the agents tool lists agents.",
        one_line(conv, 80)
    ))
}

/// The id of the open channel named `name` in space `space`, if one is.
/// Names are unique per space where a channel is made, but two Cores of a
/// space can each make one before they hear of the other's: then the name
/// is refused with both ids, never one picked (card #461).
fn named_in(talk: &Talk, name: &str, space: &str) -> Result<Option<String>, String> {
    let space = space.trim();
    let hits: Vec<ct::Channel> = talk
        .channels_named(name)
        .into_iter()
        .filter(|c| c.space_id == space)
        .collect();
    match hits.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(one.id.clone())),
        _ => Err(format!(
            "more than one #{} is here: give its conv ({}).",
            one_line(name.trim_start_matches('#'), 80),
            hits.iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Whether shared space `id` is one this person has now (its own board is
/// listed, so a `space` argument can name it).
fn space_known(b: &dyn Backend, id: &str) -> bool {
    b.shared_boards().is_some_and(|sb| {
        sb.boards()
            .iter()
            .any(|x| x.board_id == id && x.space_board.is_empty())
    })
}

/// The refusal for channel name `conv` looked up in `space` but found only
/// in other spaces: where each one is, and how to name it.
fn elsewhere_text(b: &dyn Backend, conv: &str, space: &str, chs: &[ct::Channel]) -> String {
    let name = one_line(conv.trim_start_matches('#'), 80);
    let here = if space.is_empty() {
        "Personal".to_string()
    } else {
        format!("space {}", space_tag(b, space))
    };
    let at: Vec<String> = chs
        .iter()
        .map(|c| {
            if c.space_id.is_empty() {
                format!("in Personal (conv={}): leave out space", c.id)
            } else {
                let tag = space_tag(b, &c.space_id);
                format!(
                    "in space {tag} (conv={}): pass space={}",
                    c.id,
                    space_name_arg(b, &c.space_id)
                )
            }
        })
        .collect();
    format!(
        "no channel #{name} in {here}. #{name} is {}, or give that conv.",
        at.join("; ")
    )
}

/// Space `id`'s tag as the tools show it (its own board's), else the id.
pub(crate) fn space_tag(b: &dyn Backend, id: &str) -> String {
    b.shared_boards()
        .and_then(|sb| {
            sb.boards()
                .into_iter()
                .find(|x| x.board_id == id && x.space_board.is_empty())
        })
        .map(|x| x.tag)
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| id.to_string())
}

/// What to pass as `space` to name shared space `id`: its tag when that
/// names it alone, else its id (a tag `personal`, or one two boards share,
/// would name something else).
fn space_name_arg(b: &dyn Backend, id: &str) -> String {
    let tag = space_tag(b, id);
    let all = b.shared_boards().map(|sb| sb.boards()).unwrap_or_default();
    let alone = all
        .iter()
        .filter(|x| x.tag.eq_ignore_ascii_case(&tag))
        .count()
        == 1;
    if tag == id || tag.eq_ignore_ascii_case("personal") || !alone {
        id.to_string()
    } else {
        tag
    }
}

/// The space a tool's `space` argument names, as the id channels carry:
/// empty (or `personal`) is Personal, else a shared space by tag or id
/// (card #461). The error is the tool's text.
fn space_arg(s: &Inner, v: &str) -> Result<String, String> {
    let v = trim(v);
    if v.is_empty() || v.eq_ignore_ascii_case("personal") {
        return Ok(ct::PERSONAL_SPACE.to_string());
    }
    if s.b.shared_boards().is_none() {
        return Err(err_text(
            "shared spaces aren't on here; leave out space for a Personal channel.",
        ));
    }
    s.find_board(v).map(|x| x.board_id)
}

/// Opens `me`'s DM with `key`, an agent or another person, and returns its
/// id. Opening one with a person writes nothing: the first post does (card
/// #538). A person you share no space with, and no DM, is refused here
/// rather than at the first post, and so is another person's agent.
pub fn open_dm(b: &dyn Backend, me: &str, key: &str) -> Result<String, String> {
    let talk = talk_of(b)?;
    let key = trim(key);
    if let Some(t) = key_target(talk, me, key) {
        let Target::Dm(id, Some(other)) = t? else {
            return Err("that isn't someone to DM".into());
        };
        if other.kind == ct::PERSON
            && !ct::is_person_key(&other.key)
            && talk.members(&id).is_empty()
            && !b
                .shared_boards()
                .is_some_and(|sb| sb.dm_people().iter().any(|p| p.key == other.key))
        {
            return Err("you don't share a space with this person".into());
        }
        return talk
            .open_dm(&Member::of(me), &other, me)
            .map_err(|e| e.to_string());
    }
    talk.open_dm(&Member::of(me), &Member::of(key), me)
        .map_err(|e| e.to_string())
}

/// Every post of `conv` (oldest first) as `reader` may read it: anyone
/// reads a channel or a thread; a DM, its parties and the person. A
/// huddle's lines (`call:<id>`) go by its conversation's rule, and an agent
/// never reads a person DM's (card #656).
pub fn posts(b: &dyn Backend, reader: &str, conv: &str) -> Result<Vec<Post>, String> {
    let talk = talk_of(b)?;
    let conv = trim(conv);
    if let Some(id) = conv.strip_prefix(ct::CALL_PREFIX) {
        call_parent(b, talk, reader, id)?;
        return Ok(talk.call_posts(id));
    }
    if ct::is_channel(conv) {
        if talk.channel(conv).is_none() {
            return Err(format!("no channel {}", one_line(conv, 80)));
        }
        return Ok(talk.posts(conv));
    }
    if ct::is_dm(conv) {
        let mirror = mirrored(b, talk);
        let m = mirror.get(conv);
        let mut parties = talk.members(conv);
        if parties.is_empty()
            && let Some((p, _)) = m
        {
            parties = p.clone();
        }
        let me = ct::canon(reader);
        if !ct::is_person_key(reader) && !parties.iter().any(|p| ct::canon(&p.key) == me) {
            return Err("that DM isn't yours".into());
        }
        return Ok(dm_posts(talk, m, conv));
    }
    match thread_log(b, conv) {
        Some((co, t)) => Ok(own_posts(b, &co, &t)),
        None => Err(format!(
            "no conversation {}. talk_list shows yours.",
            one_line(conv, 80)
        )),
    }
}

/// The conversation huddle `id` was in, when `reader` may read its lines:
/// as they may read that conversation, and never an agent in a person DM.
fn call_parent(b: &dyn Backend, talk: &Talk, reader: &str, id: &str) -> Result<String, String> {
    let parent = talk
        .call_conv(id)
        .ok_or_else(|| format!("no huddle {}. talk_list shows yours.", one_line(id, 80)))?;
    if talk.pair_other(&parent).is_some() && !ct::is_person_key(reader) {
        return Err("a huddle between two people is theirs: agents don't read it".into());
    }
    if ct::is_channel(&parent) {
        if talk.channel(&parent).is_none() {
            return Err(format!("no channel {}", one_line(&parent, 80)));
        }
    } else if !can_read(b, reader, &parent) {
        posts(b, reader, &parent)?;
    }
    Ok(parent)
}

/// Whether `reader` may read `conv`, by [`posts`]'s rule, without listing
/// its posts (card #643: a file served from a post asks per request).
/// Only a DM whose members are all gone needs its mirrored messages for
/// its parties, and only then is it as slow as [`posts`].
pub fn can_read(b: &dyn Backend, reader: &str, conv: &str) -> bool {
    let Ok(talk) = talk_of(b) else {
        return false;
    };
    let conv = trim(conv);
    if let Some(id) = conv.strip_prefix(ct::CALL_PREFIX) {
        return call_parent(b, talk, reader, id).is_ok();
    }
    if ct::is_channel(conv) {
        return talk.channel(conv).is_some();
    }
    if ct::is_dm(conv) {
        if ct::is_person_key(reader) {
            return true;
        }
        let parties = talk.members(conv);
        if !parties.is_empty() {
            let me = ct::canon(reader);
            return parties.iter().any(|p| ct::canon(&p.key) == me);
        }
    }
    posts(b, reader, conv).is_ok()
}

/// Posts `text` to a channel or a DM as `from`. Mentions are `given`
/// (names or keys) and the `@names` in the text; each live one is woken
/// with a mention notice, and in a channel joins it. A DM with an agent
/// goes to it as a message that wakes it (the inbox's limits apply: a
/// refused message posts nothing). A thread isn't Talk's to write: post
/// there with the card's comments. `files` are stored files sent with it
/// (card #643); with files the text may be empty.
#[allow(clippy::too_many_arguments)]
pub fn post(
    b: &dyn Backend,
    from: &Sender,
    target: &Target,
    text: &str,
    given: &[String],
    agents_now: bool,
    thread_from: Option<String>,
    files: &[blob::FileRef],
) -> Result<Posted, String> {
    if matches!(target, Target::Call(_)) {
        return Err(CALL_READ_ONLY.into());
    }
    let talk = talk_of(b)?;
    let by = from.key.clone();
    let files = ct::check_files(files).map_err(|e| e.to_string())?;
    let text_ok = ct::check_post(&by, text, !files.is_empty()).map_err(|e| e.to_string())?;
    // "person" is the person, not an agent to resolve (card #395).
    let agents_given: Vec<String> = given
        .iter()
        .filter(|g| !is_person_mention(g))
        .cloned()
        .collect();
    // In a space's channel the space's other people and their agents may
    // be named too, by board key (card #461): their own Cores wake them.
    let local = cards(b);
    let mut agents = local.clone();
    if let Target::Channel(id) = target
        && let Some(c) = talk.channel(id)
        && !c.space_id.is_empty()
        && let Some(sb) = b.shared_boards()
    {
        agents.extend(sb.board_mentionables(&c.space_id));
    }
    let mut res: Resolved = coord::mentions::resolve(&agents, &by, &text_ok, &agents_given);
    prefer_own(&mut res, &local);
    let mut live: Vec<String> = res
        .mentions
        .iter()
        .filter(|m| !m.gone)
        .map(|m| m.key.clone())
        .collect();
    // An agent mentions the person in a channel with mention "person"
    // (card #395): it notifies their phone. A DM with them already does.
    if matches!(target, Target::Channel(_))
        && !ct::is_person_key(&by)
        && given.iter().any(|g| is_person_mention(g))
    {
        live.push(PERSON_MENTION.to_string());
    }
    let mut out = Posted {
        mentions: res.mentions.clone(),
        misses: res.misses.clone(),
        ..Posted::default()
    };
    let (conv, title) = match target {
        Target::Call(_) => return Err(CALL_READ_ONLY.into()),
        Target::Thread(_) => {
            return Err("a thread's posts are its comments: post with board_comment".into());
        }
        Target::Channel(id) => {
            let c = talk
                .channel(id)
                .ok_or_else(|| format!("no channel {}", one_line(id, 80)))?;
            (id.clone(), format!("#{}", c.name))
        }
        Target::Dm(id, other) => {
            let other = match other {
                Some(o) => o.clone(),
                None => talk
                    .pair_other(id)
                    .map(|k| Member::of(&k))
                    .or_else(|| {
                        talk.members(id)
                            .into_iter()
                            .find(|m| ct::canon(&m.key) != ct::canon(&by))
                    })
                    .ok_or_else(|| {
                        "that DM has no one in it yet: name the other side, conv=@agent or conv=person"
                            .to_string()
                    })?,
            };
            if ct::canon(&other.key) == ct::canon(&by) {
                return Err("that's you: a DM needs another agent or the person".into());
            }
            // Agents don't DM other people, and nobody DMs another
            // person's agent (card #538, talk-dms.md §4).
            if ct::is_other_person_key(&other.key) && !ct::is_person_key(&by) {
                return Err(NO_DM_TO_PEOPLE.into());
            }
            if board_card(&other.key).is_some() {
                return Err(NO_DM_TO_THEIR_AGENT.into());
            }
            if talk.dm_of(&by, &other.key) != *id {
                return Err("that DM isn't yours".into());
            }
            talk.open_dm(&Member::of(&by), &other, &by)
                .map_err(|e| e.to_string())?;
            if other.kind == ct::AGENT {
                let o = SendOptions {
                    mode: mode::WAKE.into(),
                    ref_: format!("talk:{id}"),
                    mentions: crate::mention::to_inbox(&res.mentions),
                    attachments: files.iter().map(crate::attachments::to_inbox).collect(),
                    ..SendOptions::default()
                };
                let (m, d) = b
                    .inbox()
                    .send_with(from, &Address::new(addr::AGENT, &other.key), &text_ok, o)
                    .map_err(|e| crate::map_err(e).trim_start_matches("Error: ").to_string())?;
                out.sent = Some(d);
                // The post is the message: one id, shown once.
                let p = talk
                    .post(NewPost {
                        post_id: m.id.clone(),
                        conv_id: id.clone(),
                        text: text_ok.clone(),
                        mentions: live.clone(),
                        agents_now,
                        thread_from: thread_from.clone(),
                        files: files.clone(),
                        by: by.clone(),
                        stay_out: false,
                        call: None,
                        transcript: None,
                    })
                    .map_err(|e| e.to_string())?;
                out.post = p;
                let skip = other.key.clone();
                out.told = tell_mentions(b, from, &out.post, &live, &skip, "your DM");
                return Ok(out);
            }
            // Nobody is woken through a DM with a person (card #538).
            if talk.pair_other(id).is_some() {
                live.clear();
                out.mentions.clear();
                out.misses.clear();
            }
            (id.clone(), "a DM".to_string())
        }
    };
    let p = talk
        .post(NewPost {
            conv_id: conv.clone(),
            text: text_ok,
            mentions: live.clone(),
            agents_now,
            thread_from,
            files,
            by: by.clone(),
            // An agent Irori started posts without joining (card #454).
            stay_out: b.coord().is_spawned(&by),
            ..NewPost::default()
        })
        .map_err(|e| e.to_string())?;
    if ct::is_channel(&conv) {
        for k in live
            .iter()
            .filter(|k| !ct::is_person_key(k) && !ct::is_other_person_key(k))
        {
            // An agent mentioned in a channel joins it (§3); another
            // person joins their space's channels themselves.
            let _ = talk.join(&conv, &Member::of(k), &by);
        }
    }
    // "Agents answer now" (card #391): on, the agents it names are woken;
    // off, nobody is. Every other agent in it reads it at its next turn.
    let woken: Vec<String> = if agents_now { live.clone() } else { Vec::new() };
    out.told = tell_mentions(b, from, &p, &woken, "", &title);
    out.told
        .extend(tell_readers(b, talk, from, &p, &woken, &title, agents_now));
    out.post = p;
    Ok(out)
}

/// The next-turn reading of a channel or DM post (card #391,
/// docs/design/talk-channels.md §3): each agent in `p`'s conversation,
/// but its author and the `woken`, hears what it hasn't read there (after
/// its read cursor and its own last post): at most
/// [`coord::READING_LINES`] lines and a count, a reply optional. A newer
/// reading replaces one still waiting.
fn tell_readers(
    b: &dyn Backend,
    talk: &Talk,
    from: &Sender,
    p: &Post,
    woken: &[String],
    where_: &str,
    agents_now: bool,
) -> Vec<NoticeResult> {
    let conv = &p.conv_id;
    let posts = talk.posts(conv);
    let names = Book::new(b);
    let mut out = Vec::new();
    for m in talk.members(conv) {
        let k = &m.key;
        // Another person's agent (a board key) reads on its own Core.
        if m.kind != ct::AGENT
            || coord::board_form(k)
            || ct::canon(k) == ct::canon(&p.by)
            || woken.iter().any(|w| ct::canon(w) == ct::canon(k))
        {
            continue;
        }
        let after = ct::since(&posts, talk.cursor(k, conv).as_deref());
        // What it wrote, it has read.
        let from_i = after
            .iter()
            .rposition(|q| ct::canon(&q.by) == ct::canon(k))
            .map_or(0, |i| i + 1);
        let unread = &after[from_i..];
        if unread.is_empty() {
            continue;
        }
        let n = unread.len();
        let mut text = format!(
            "Since you last read {where_} (Talk, conv={conv}): {n} new post{}",
            if n == 1 { "" } else { "s" }
        );
        if n > coord::READING_LINES {
            text.push_str(&format!(", the last {} below", coord::READING_LINES));
        }
        text.push_str(".\n");
        for q in &unread[n.saturating_sub(coord::READING_LINES)..] {
            let who = if ct::is_person_key(&q.by) {
                "the person".to_string()
            } else {
                one_line(&names.name(&q.by), 80)
            };
            text.push_str(&format!("- {who}: {}\n", one_line(&post_line(q), 200)));
        }
        if !agents_now {
            text.push_str("\"Agents answer now\" was off for it, so nobody was woken. ");
        }
        text.push_str(&format!(
            "A reply is optional: answer with talk_post conv={conv} only if you have something to add or a correction. talk_read conv={conv} shows the rest.\n"
        ));
        let n = Notice {
            kind: coord::NOTICE_READING.into(),
            reference: format!("talk:{conv}"),
            title: where_.into(),
            to: vec![k.clone()],
            by: from.key.clone(),
            text,
            wake: false,
            ..Notice::default()
        };
        out.extend(crate::notice::route_notice(b.inbox(), from, &n));
    }
    out
}

/// The key a post's mentions name the person by: Talk counts every one of
/// their devices as it ([`ct::canon`]).
pub const PERSON_MENTION: &str = "user";

/// Whether a `mention` entry names the person: "person" (as a DM's conv
/// does), "you", or their key.
fn is_person_mention(g: &str) -> bool {
    let g = g.trim().trim_start_matches('@').trim();
    g.eq_ignore_ascii_case("person") || g.eq_ignore_ascii_case("you") || ct::is_person_key(g)
}

/// Wakes the agents a post mentions, through the mention notice.
fn tell_mentions(
    b: &dyn Backend,
    from: &Sender,
    p: &Post,
    keys: &[String],
    skip: &str,
    where_: &str,
) -> Vec<NoticeResult> {
    let to: Vec<String> = keys
        .iter()
        // A board key is another person or their agent in a space: their
        // own Core wakes them from the row (card #461).
        .filter(|k| k.as_str() != skip && !ct::is_person_key(k) && !coord::board_form(k))
        .cloned()
        .collect();
    if to.is_empty() {
        return Vec::new();
    }
    let who = if ct::is_person_key(&from.key) {
        "The person".to_string()
    } else {
        let n = Book::new(b).name(&from.key);
        one_line(&n, 80)
    };
    let mut body = p.text.clone();
    for f in &p.files {
        if !body.is_empty() {
            body.push('\n');
        }
        body += &format!("file: {} (attachment_get)", f.describe());
    }
    let text = format!(
        "{who} mentioned you in {where_} (Talk, conv={conv}):\n{body}\n\nAnswer there with talk_post conv={conv} if you have something to add; talk_read conv={conv} shows the rest.\n",
        conv = p.conv_id,
    );
    let n = Notice {
        kind: NOTICE_MENTIONED.into(),
        reference: format!("talk:{}", p.conv_id),
        title: where_.into(),
        to,
        by: from.key.clone(),
        text,
        wake: true,
        ..Notice::default()
    };
    crate::notice::route_notice(b.inbox(), from, &n)
}

// ---- the tools ----------------------------------------------------------

fn me(c: &Caller) -> &str {
    &c.sender.key
}

const POST_ARGS: &[(&str, Ty)] = &[
    ("conv", Ty::Str),
    ("text", Ty::Str),
    ("mention", Ty::Strs),
    ("space", Ty::Str),
    ("attach", Ty::Strs),
];

pub(crate) fn talk_post(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, POST_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let b = s.b.as_ref();
    let space = match space_arg(s, a.str("space")) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let target = match resolve(b, me(c), a.str("conv"), &space) {
        Ok(t) => t,
        Err(e) => return err_text(&e),
    };
    if let Target::Thread(id) = &target {
        // A thread's posts are its card's comments.
        let mut args = serde_json::json!({"task_id": format!("task:{id}"), "text": a.str("text")});
        let ms = a.strs("mention");
        if !ms.is_empty() {
            args["mention"] = serde_json::json!(ms);
        }
        // Its files go with the comment, stored by board_comment (#643).
        let at = a.strs("attach");
        if !at.is_empty() {
            args["attach"] = serde_json::json!(at);
        }
        // Through the shared-space rules, as the tool itself runs: a card
        // in a space replicates to its members (card #558).
        let raw = args.to_string();
        return s.around_board("board_comment", raw.as_bytes(), || {
            crate::board::board_comment(s, c, raw.as_bytes())
        });
    }
    let files = match s.attach_files(c, &a.strs("attach")) {
        Ok(f) => f,
        Err(m) => return m,
    };
    match post(
        b,
        &c.sender,
        &target,
        a.str("text"),
        &a.strs("mention"),
        true,
        None,
        &files,
    ) {
        Ok(p) => {
            let n = s.roster();
            let mut out = format!(
                "Posted {} to {}{}.",
                p.post.post_id,
                p.post.conv_id,
                crate::attachments::attached_line(&p.post.files)
            );
            if let Some(d) = &p.sent {
                out += &s.outcome(d);
            }
            out += &crate::mention::mention_line(&outcomes(&p), &p.misses, &n);
            out
        }
        Err(e) => err_text(&e),
    }
}

const THREAD_ARGS: &[(&str, Ty)] = &[
    ("text", Ty::Str),
    ("conv", Ty::Str),
    ("mention", Ty::Strs),
    ("space", Ty::Str),
];

/// talk_thread (card #631): the agent opens a thread with the person and
/// holds it. Irori never hands it out or starts another agent for it, even
/// once this one ends, until the person writes in it; the agents it names
/// are woken as for the person's thread. It goes in Personal, a space (its
/// log, or the person's own when their threads there are private), and
/// names a channel it's about when `conv` gives one, with a pointer post
/// there.
pub(crate) fn talk_thread(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, THREAD_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let b = s.b.as_ref();
    let mut space = match space_arg(s, a.str("space")) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let mut channel = String::new();
    if !trim(a.str("conv")).is_empty() {
        match resolve(b, me(c), a.str("conv"), &space) {
            Ok(Target::Channel(id)) => {
                if let Some(ch) = talk_of(b).ok().and_then(|t| t.channel(&id)) {
                    space = if ch.space_id.is_empty() {
                        ct::PERSONAL_SPACE.to_string()
                    } else {
                        ch.space_id.clone()
                    };
                }
                channel = id;
            }
            Ok(Target::Call(_)) => return err_text(CALL_READ_ONLY),
            Ok(_) => {
                return err_text(
                    "conv is the channel the thread is about (#name); leave it out for Personal. To write in a DM or a thread, use talk_post.",
                );
            }
            Err(e) => return err_text(&e),
        }
    }
    let text = a.str("text");
    let shared = space != ct::PERSONAL_SPACE
        && b.inbox().threads_mode(&space) != inbox::threads_mode::PRIVATE;
    let (t, out) = if shared {
        match crate::shared::thread_on_board(s, c, &space, text, &a.strs("mention"), &channel) {
            Ok(v) => v,
            Err(e) => return e,
        }
    } else {
        let res = s.resolve_mentions(c, "", text, &a.strs("mention"));
        let input = coord::NewAsk {
            text: text.to_string(),
            thread: true,
            mentions: res.mentions.clone(),
            board: if space == ct::PERSONAL_SPACE {
                String::new()
            } else {
                space.clone()
            },
            channel: channel.clone(),
            ..coord::NewAsk::default()
        };
        let (t, n) = match b.coord().post_agent_thread(me(c), input) {
            Ok(v) => v,
            Err(e) => return err_text(&crate::map_err(e)),
        };
        let names = s.roster();
        let mut out = String::new();
        if let Some(n) = n {
            out += &s.told(&s.tell(&c.sender, n), &names);
        }
        out += &crate::mention::mention_line(&[], &res.misses, &names);
        (t, out)
    };
    let short = t.short_ref();
    let mut said = format!(
        "Opened thread {short} (task:{}). You hold it: the person's replies come to you, and no other agent is started for it, even after you end, until they write in it. Answer there with talk_post conv={short}; only the person resolves it.",
        t.id
    );
    // A pointer in the channel it's about, as the person's thread from a
    // channel post gets one; nobody is woken by it.
    if !channel.is_empty() && (space == ct::PERSONAL_SPACE || shared) {
        let pointer = format!("Started a thread: {short} {}", one_line(&t.title, 120));
        if let Err(e) = post(
            b,
            &c.sender,
            &Target::Channel(channel.clone()),
            &pointer,
            &[],
            false,
            None,
            &[],
        ) {
            said += &format!(" The pointer in the channel failed: {e}.");
        }
    }
    said + &out
}

/// An `@name` both this person's agent and another person's in the space
/// answer to names this person's own (card #461), as it did before the
/// space's agents could be named; the other one is named by its key (the
/// window's picker passes it).
fn prefer_own(res: &mut Resolved, local: &[coord::roster::AgentCard]) {
    let mut keep = Vec::new();
    for m in std::mem::take(&mut res.misses) {
        let own: Vec<&String> = m
            .candidates
            .iter()
            .filter(|k| local.iter().any(|a| &a.key == *k))
            .collect();
        if m.reason == coord::mentions::MISS_AMBIGUOUS
            && own.len() == 1
            && res.mentions.len() < coord::MAX_MENTIONS
        {
            if !res.mentions.iter().any(|x| &x.key == own[0]) {
                res.mentions.push(Mention {
                    key: own[0].clone(),
                    name: m.name.clone(),
                    gone: false,
                });
            }
            continue;
        }
        keep.push(m);
    }
    res.misses = keep;
}

/// What each of a post's mentions came to: another person's agent named
/// by board key is on the space, its own Core wakes it (card #461).
pub(crate) fn outcomes(p: &Posted) -> Vec<(Mention, crate::mention::Got)> {
    crate::mention::outcomes(&p.mentions, &p.told)
        .into_iter()
        .map(|(m, g)| match g {
            crate::mention::Got::Board => (m, crate::mention::Got::Space),
            g => (m, g),
        })
        .collect()
}

const READ_ARGS: &[(&str, Ty)] = &[("conv", Ty::Str), ("since", Ty::Str), ("space", Ty::Str)];

pub(crate) fn talk_read(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, READ_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let b = s.b.as_ref();
    let Some(talk) = b.talk() else {
        return err_text("Talk is off on this Core");
    };
    let space = match space_arg(s, a.str("space")) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let target = match resolve(b, me(c), a.str("conv"), &space) {
        Ok(t) => t,
        Err(e) => return err_text(&e),
    };
    let conv = target.id().to_string();
    let all = match posts(b, me(c), &conv) {
        Ok(p) => p,
        Err(e) => return err_text(&e),
    };
    let since = trim(a.str("since"));
    let is_call = matches!(target, Target::Call(_));
    let mark = if is_call {
        Lines::parse(talk.cursor(me(c), &conv).as_deref(), &all)
    } else {
        Lines::default()
    };
    let cursor = if since.is_empty() {
        talk.cursor(me(c), &conv)
    } else {
        Some(since.to_string())
    };
    let after: Vec<Post> = if is_call && since.is_empty() {
        all.iter().filter(|p| !mark.has(p)).cloned().collect()
    } else {
        ct::since(&all, cursor.as_deref()).to_vec()
    };
    let (shown, head) = if is_call && since.is_empty() && mark.is_empty() {
        // A transcript is read from its start (card #656).
        (
            all[..all.len().min(READ_PAGE)].to_vec(),
            "The transcript, from the start",
        )
    } else if after.is_empty() || (!is_call && since.is_empty() && after.len() == all.len()) {
        // Nothing new, or never read: the latest page.
        let p = ct::page(&all, None, READ_PAGE);
        let h = if after.is_empty() && !all.is_empty() {
            "Nothing new; the latest posts"
        } else {
            "The latest posts"
        };
        (p, h)
    } else {
        (
            after[..after.len().min(READ_PAGE)].to_vec(),
            "Posts since you last read",
        )
    };
    if all.is_empty() {
        return format!("No posts in {conv} yet.");
    }
    let n = s.roster();
    let now = s.now();
    let mut out = String::new();
    if let Target::Call(_) = &target {
        out += &call_head(talk, &n, &conv);
    }
    out += &format!("{head} in {conv}, oldest first:\n");
    for p in &shown {
        out += &format!(
            "- [{}] {} ({}): {}\n",
            p.post_id,
            n.who(&p.by),
            ago(&now, &p.at),
            one_line(&p.text, READ_TEXT)
        );
        for f in &p.files {
            out += &format!("    file: {} (attachment_get)\n", f.describe());
        }
    }
    if let Some(last) = shown.last() {
        let at = all
            .iter()
            .position(|p| p.post_id == last.post_id)
            .unwrap_or(0);
        let left = match (is_call && since.is_empty(), mark.is_empty()) {
            // A transcript's unread lines need not follow each other.
            (true, false) => after.len().saturating_sub(shown.len()),
            _ => all.len() - 1 - at,
        };
        if left > 0 {
            out += &format!(
                "{left} more: talk_read conv={conv} since={}\n",
                last.post_id
            );
        }
        if is_call {
            // A transcript's cursor is each speaker's newest line read, so
            // a line that arrives late with an earlier time is still unread.
            let mut next = mark;
            next.read(&all[..=at]);
            if let Some(up_to) = next.encode() {
                let _ = talk.mark_read(&conv, &up_to, me(c));
            }
            return out + untrusted();
        }
        // Reading moves the caller's cursor (never back).
        let cur = talk.cursor(me(c), &conv);
        let pos = |id: &str| all.iter().position(|p| p.post_id == id);
        let newer = match cur.as_deref().and_then(pos) {
            Some(i) => pos(&last.post_id).is_some_and(|j| j > i),
            None => true,
        };
        if newer {
            let _ = talk.mark_read(&conv, &last.post_id, me(c));
        }
    }
    out + untrusted()
}

pub(crate) fn talk_list(s: &Inner, c: &Caller, _: &[u8]) -> String {
    let b = s.b.as_ref();
    let Some(talk) = b.talk() else {
        return err_text("Talk is off on this Core");
    };
    let cs = match conversations(b, me(c)) {
        Ok(cs) => cs,
        Err(e) => return err_text(&e),
    };
    let n = s.roster();
    let mut out = String::new();
    if cs.is_empty() {
        out += "You're in no conversations yet.\n";
    } else {
        out += "Your conversations, newest first:\n";
    }
    for x in &cs {
        let what = match x.kind.as_str() {
            ct::KIND_CHANNEL => format!(
                "#{} (channel{}, conv={})",
                one_line(&x.name, 80),
                in_space(b, talk, &x.conv_id),
                x.conv_id
            ),
            ct::KIND_DM => {
                let other: Vec<String> = x
                    .members
                    .iter()
                    .filter(|m| ct::canon(&m.key) != ct::canon(me(c)))
                    .map(|m| n.who(&m.key))
                    .collect();
                format!("DM with {} (conv={})", other.join(", "), x.conv_id)
            }
            _ => format!(
                "thread {} {} (conv={})",
                x.short,
                crate::format::quoted(&x.name, 80),
                x.conv_id
            ),
        };
        let mut tail = Vec::new();
        if x.unread > 0 {
            tail.push(format!("{} unread", x.unread));
        }
        if x.mentions > 0 {
            tail.push(format!("{} mention you", x.mentions));
        }
        let tail = if tail.is_empty() {
            String::new()
        } else {
            format!(": {}", tail.join(", "))
        };
        let topic = if x.topic.is_empty() {
            String::new()
        } else {
            format!(" - {}", one_line(&x.topic, 160))
        };
        out += &format!("- {what}{tail}{topic}\n");
    }
    out += &huddles(talk, &n, me(c));
    let others: Vec<String> = talk
        .channels()
        .into_iter()
        .filter(|ch| !ch.archived && !talk.is_member(&ch.id, me(c)))
        .map(|ch| {
            format!(
                "#{} ({}{})",
                one_line(&ch.name, 80),
                ch.id,
                in_space(b, talk, &ch.id)
            )
        })
        .collect();
    if !others.is_empty() {
        out += &format!(
            "Channels you aren't in (talk_join to follow one): {}\n",
            others.join(", ")
        );
    }
    out + untrusted()
}

/// The most huddles talk_list shows.
const HUDDLES_SHOWN: usize = 10;

/// A transcript's read cursor (card #656): the newest line read from each
/// source (a device's rows, or one fallback post), as `lines:src=n,…`.
/// Lines sort by when they were said, but arrive in each source's order,
/// so one post id can't say what has been read.
#[derive(Debug, Default, PartialEq)]
struct Lines(BTreeMap<String, i64>);

const LINES_PREFIX: &str = "lines:";

impl Lines {
    /// The cursor `up_to` over transcript `all`. An older cursor that names
    /// a line counts every line up to it as read.
    fn parse(up_to: Option<&str>, all: &[Post]) -> Lines {
        let mut out = Lines::default();
        let Some(up_to) = up_to else {
            return out;
        };
        if let Some(rest) = up_to.strip_prefix(LINES_PREFIX) {
            for e in rest.split(',') {
                if let Some((k, n)) = e.rsplit_once('=')
                    && let Ok(n) = n.parse()
                    && !k.is_empty()
                {
                    out.0.insert(k.to_string(), n);
                }
            }
        } else if let Some(i) = all.iter().position(|p| p.post_id == up_to) {
            out.read(&all[..=i]);
        }
        out
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Where line `p` comes from, and its place there.
    fn key(p: &Post) -> (String, i64) {
        match p.post_id.split_once('#') {
            Some((post, i)) => (post.to_string(), i.parse().unwrap_or(p.ord.2)),
            None => (p.device.clone(), p.ord.2),
        }
    }

    fn has(&self, p: &Post) -> bool {
        let (k, n) = Lines::key(p);
        self.0.get(&k).is_some_and(|m| *m >= n)
    }

    /// Counts `lines` read (never back).
    fn read(&mut self, lines: &[Post]) {
        for p in lines {
            let (k, n) = Lines::key(p);
            let m = self.0.entry(k).or_insert(n);
            *m = (*m).max(n);
        }
    }

    fn encode(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let es: Vec<String> = self.0.iter().map(|(k, n)| format!("{k}={n}")).collect();
        Some(format!("{LINES_PREFIX}{}", es.join(",")))
    }
}

/// Where a huddle was, for a line about it: `#name`, or the DM with whom.
fn call_place(talk: &Talk, n: &crate::Names<'_>, conv: &str) -> String {
    if let Some(ch) = talk.channel(conv) {
        return format!("#{}", one_line(&ch.name, 80));
    }
    match talk.pair_other(conv) {
        Some(other) => format!("the DM with {}", n.who(&other)),
        None => conv.to_string(),
    }
}

/// The huddle `conv` (`call:<id>`) names, as talk_list knows it.
fn call_info(talk: &Talk, conv: &str) -> Option<ct::CallInfo> {
    let id = conv.strip_prefix(ct::CALL_PREFIX)?;
    talk.calls().into_iter().find(|c| c.call_id == id)
}

/// talk_read's first line for a huddle's transcript (card #656).
fn call_head(talk: &Talk, n: &crate::Names<'_>, conv: &str) -> String {
    let Some(c) = call_info(talk, conv) else {
        return String::new();
    };
    format!(
        "The transcript of the huddle in {} ({}): each line is what its speaker said, at the time they said it.\n",
        call_place(talk, n, &c.conv_id),
        if c.end.is_some() { "ended" } else { "live" },
    )
}

/// talk_list's huddles with a transcript (card #656), newest first: the
/// person sees all of them, an agent those of the channels it is in, never
/// a person DM's.
fn huddles(talk: &Talk, n: &crate::Names<'_>, me: &str) -> String {
    let person = ct::is_person_key(me);
    let shown: Vec<ct::CallInfo> = talk
        .calls()
        .into_iter()
        .filter(|c| c.lines > 0)
        .filter(|c| {
            if talk.pair_other(&c.conv_id).is_some() {
                return person;
            }
            if !ct::is_channel(&c.conv_id) || talk.channel(&c.conv_id).is_none() {
                return false;
            }
            person || talk.is_member(&c.conv_id, me)
        })
        .collect();
    if shown.is_empty() {
        return String::new();
    }
    let mut out = "Huddles with a transcript, newest first:\n".to_string();
    for c in shown.iter().take(HUDDLES_SHOWN) {
        let conv = format!("{}{}", ct::CALL_PREFIX, c.call_id);
        let lines = talk.call_posts(&c.call_id);
        let mark = Lines::parse(talk.cursor(me, &conv).as_deref(), &lines);
        let unread = lines
            .iter()
            .filter(|p| !mark.has(p) && ct::canon(&p.by) != ct::canon(me))
            .count();
        let tail = if unread > 0 {
            format!(": {unread} unread")
        } else {
            String::new()
        };
        out += &format!(
            "- huddle in {} (conv={conv}, {} line{}, {}){tail}\n",
            call_place(talk, n, &c.conv_id),
            c.lines,
            if c.lines == 1 { "" } else { "s" },
            if c.end.is_some() { "ended" } else { "live" },
        );
    }
    if shown.len() > HUDDLES_SHOWN {
        out += &format!("({} older not shown)\n", shown.len() - HUDDLES_SHOWN);
    }
    out
}

/// What a channel's agents hear when its huddle ends (card #656), the
/// same from the device that ended it and from another person's.
pub fn call_end_text(channel: &str, call_id: &str, lines: usize) -> String {
    let count = if lines == 0 {
        "its lines may still be arriving".to_string()
    } else {
        format!("{lines} line{}", if lines == 1 { "" } else { "s" })
    };
    format!(
        "The huddle in #{} ended. talk_read conv={}{call_id} has the transcript ({count}).",
        one_line(channel, 80),
        ct::CALL_PREFIX,
    )
}

/// The bridged row a huddle's end goes by (card #656), so every end post
/// of one huddle wakes an agent once.
pub fn call_end_row(call_id: &str) -> String {
    format!("call-end/{}", call_id.trim())
}

/// Tells this person's agents in channel `conv` that huddle `call_id`
/// ended and where its transcript is (card #656): one reading each, with
/// a wake, its reference the channel (`talk:<conv>`) so a reply goes there.
/// Nobody hears of a DM's huddle, and an agent another person's end post
/// already woke for it (the bridge) isn't told again. Otherwise each call
/// tells again: call it once per huddle (the last device out).
pub fn tell_call_end(
    b: &dyn Backend,
    from: &Sender,
    conv: &str,
    call_id: &str,
) -> Vec<NoticeResult> {
    let Ok(talk) = talk_of(b) else {
        return Vec::new();
    };
    let conv = trim(conv);
    if !ct::is_channel(conv) || talk.pair_other(conv).is_some() {
        return Vec::new();
    }
    let Some(ch) = talk.channel(conv) else {
        return Vec::new();
    };
    let row = call_end_row(call_id);
    let mut to: Vec<String> = Vec::new();
    for m in talk.members(conv) {
        // Another person's agent (a board key) hears it on its own Core;
        // one that heard another person's end of it already isn't told again.
        if m.kind == ct::AGENT
            && !coord::board_form(&m.key)
            && !to.iter().any(|k| ct::canon(k) == ct::canon(&m.key))
            && !b.inbox().bridged(&ch.space_id, &row, &m.key)
        {
            to.push(m.key);
        }
    }
    if to.is_empty() {
        return Vec::new();
    }
    let n = Notice {
        kind: coord::NOTICE_READING.into(),
        reference: format!("talk:{conv}"),
        title: format!("#{}", ch.name),
        to,
        by: from.key.clone(),
        text: call_end_text(&ch.name, call_id.trim(), talk.call_posts(call_id).len()),
        wake: true,
        ..Notice::default()
    };
    crate::notice::route_notice(b.inbox(), from, &n)
}

/// " in space <tag>" for a space's channel, so two channels of one name
/// tell apart in talk_list (card #461); empty for a Personal one.
fn in_space(b: &dyn Backend, talk: &Talk, conv: &str) -> String {
    match talk.channel(conv) {
        Some(c) if !c.space_id.is_empty() => format!(" in space {}", space_tag(b, &c.space_id)),
        _ => String::new(),
    }
}

const CONV_ARGS: &[(&str, Ty)] = &[("conv", Ty::Str), ("space", Ty::Str)];

fn channel_arg(s: &Inner, c: &Caller, raw: &[u8]) -> Result<(Arc<Talk>, String, String), String> {
    let a = decode(raw, CONV_ARGS)?;
    let b = s.b.as_ref();
    let talk = talk_of(b).map_err(|e| err_text(&e))?.clone();
    let space = space_arg(s, a.str("space"))?;
    match resolve(b, me(c), a.str("conv"), &space) {
        Ok(Target::Channel(id)) => {
            let name = talk.channel(&id).map(|ch| ch.name).unwrap_or_default();
            Ok((talk, id, name))
        }
        Ok(_) => Err(err_text(
            "only channels are joined or left: a DM is always yours, and a thread is its card's (board_watch / board_unwatch)",
        )),
        Err(e) => Err(err_text(&e)),
    }
}

pub(crate) fn talk_join(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let (talk, id, name) = match channel_arg(s, c, raw) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match talk.join(&id, &Member::of(me(c)), me(c)) {
        Ok(true) => format!(
            "Joined #{} ({id}). talk_read conv={id} shows what's been said.",
            one_line(&name, 80)
        ),
        Ok(false) => format!("You're already in #{} ({id}).", one_line(&name, 80)),
        Err(e) => err_text(&e.to_string()),
    }
}

pub(crate) fn talk_leave(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let (talk, id, name) = match channel_arg(s, c, raw) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match talk.leave(&id, &Member::of(me(c)), me(c)) {
        Ok(true) => format!(
            "Left #{} ({id}). A mention brings you back.",
            one_line(&name, 80)
        ),
        Ok(false) => format!("You weren't in #{} ({id}).", one_line(&name, 80)),
        Err(e) => err_text(&e.to_string()),
    }
}
