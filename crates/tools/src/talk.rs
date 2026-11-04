//! Talk (card #389; docs/design/talk-channels.md §2, §3, §7): the
//! `talk_*` tools, and the conversation views the window reads through
//! the Core, so agents and the person see the same thing.
//!
//! - Channels and DMs are `talk.*` rows ([`coord::talk`]).
//! - A thread's conversation is its task's: the ask, then its comments,
//!   mapped into posts. Posting to one goes through the card's comment
//!   path (`board_comment`), so the ask fold sees the reply.
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
    /// The reader is in it (a channel they joined, their own DM, a thread
    /// they're on). The person also sees agents' DMs, read-only.
    pub joined: bool,
    /// The reader can only read it: a DM between two others.
    pub read_only: bool,
    /// A thread's short ref (`#12`), for agents.
    pub short: String,
    /// A thread's "Agents answer now" (card #391); false for a channel or
    /// DM, whose posts each carry their own.
    pub agents_now: bool,
    /// The space a channel is in (card #461): a shared space's id (its
    /// first board's), empty for Personal. DMs and threads are always empty:
    /// DMs are you and your agents, and these threads are the private log's.
    pub space_id: String,
}

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
}

impl Target {
    pub fn id(&self) -> &str {
        match self {
            Target::Channel(id) | Target::Dm(id, _) | Target::Thread(id) => id,
        }
    }
}

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

/// Names for keys: the roster's, else the last name an agent went by.
struct Book {
    cards: HashMap<String, String>,
    known: BTreeMap<String, String>,
}

impl Book {
    fn new(b: &dyn Backend) -> Book {
        Book {
            cards: b
                .agents()
                .into_iter()
                .map(|a| (a.key.clone(), a.name))
                .collect(),
            known: b.known_names(),
        }
    }

    fn name(&self, key: &str) -> String {
        if ct::is_person_key(key) {
            return "you".into();
        }
        self.cards
            .get(key)
            .or_else(|| self.known.get(key))
            .filter(|n| !n.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }

    fn member(&self, m: &Member) -> ConvMember {
        ConvMember {
            kind: m.kind.clone(),
            key: m.key.clone(),
            name: self.name(&m.key),
        }
    }
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
            at: c.at,
            ord: (c.hlc, c.device.clone(), c.seq),
            ..Post::default()
        });
    }
    ct::sort_posts(&mut out[1..]);
    out
}

/// Who a thread is between: whoever asked, the agents it went to, and
/// its holders and watchers.
fn thread_members(t: &Task) -> Vec<Member> {
    let mut keys = vec![t.created_by.clone()];
    if let Some(a) = &t.ask {
        keys.extend(a.heard.iter().cloned());
        keys.push(a.taker.clone());
    }
    keys.extend(t.holders.iter().cloned());
    keys.extend(t.watchers.iter().cloned());
    let mut out: Vec<Member> = Vec::new();
    for k in keys {
        if !k.is_empty() && !out.iter().any(|m| ct::canon(&m.key) == ct::canon(&k)) {
            out.push(Member::of(&k));
        }
    }
    out
}

/// The DMs mirrored from messages: by DM id, its two parties and posts.
/// Whether a message can show as a DM ([`mirrored`]): one an agent or the
/// person sent, not a notice, a stop, Irori's own, a mention's wake or a
/// Wind down (card #484). The Core tells the window Talk changed only for
/// these (card #468).
pub fn mirrors(m: &inbox::Message) -> bool {
    !(m.notice
        || m.stop
        || m.from.kind == "irori"
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

/// `reader`'s conversations: the person sees every open channel, every
/// DM (agents' own read-only) and every open thread; an agent sees the
/// channels it joined, its DMs and the open threads it is on. Newest
/// first.
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
        out.push(Conversation {
            conv_id: id.clone(),
            kind: ct::KIND_DM.into(),
            name: others.join(", "),
            unread,
            mentions,
            members: parties.iter().map(|m| names.member(m)).collect(),
            last_at: last_at(&ps),
            joined: mine,
            read_only: !mine,
            ..Conversation::default()
        });
    }

    let co = b.coord();
    for t in co.list_tasks(&coord::TaskFilter::default()) {
        if !is_thread(&t) || !thread_open(&t) {
            continue;
        }
        let members = thread_members(&t);
        let ps = thread_posts(co, &t);
        // On it: asked, told, holding, watching or answered.
        let on = members.iter().any(|m| ct::canon(&m.key) == me)
            || ps.iter().any(|p| ct::canon(&p.by) == me);
        if !person && !on {
            continue;
        }
        let (unread, mentions) = counts(talk, &ps, reader, &t.id);
        out.push(Conversation {
            conv_id: t.id.clone(),
            kind: ct::KIND_THREAD.into(),
            name: t.title.clone(),
            unread,
            mentions,
            members: members.iter().map(|m| names.member(m)).collect(),
            last_at: last_at(&ps),
            joined: on || person,
            short: t.short.clone(),
            agents_now: t.ask.as_ref().is_some_and(|a| a.wake),
            ..Conversation::default()
        });
    }

    out.sort_by(|a, b| coord_cmp(&b.last_at, &a.last_at).then_with(|| a.conv_id.cmp(&b.conv_id)));
    Ok(out)
}

fn coord_cmp(a: &GoTime, b: &GoTime) -> std::cmp::Ordering {
    a.unix_nano().cmp(&b.unix_nano())
}

/// What `conv` names for `me`: a channel or DM id, `#name` (a channel)
/// or `#12` / `task:<id>` (a thread or card), `@name` or an agent's key
/// (the DM with it), or `person` (the DM with the person). A channel's
/// name is looked up in space `space` only (empty: Personal); a name that
/// lives in another space is refused with where it is, never followed
/// there (card #461).
pub fn resolve(b: &dyn Backend, me: &str, conv: &str, space: &str) -> Result<Target, String> {
    let talk = talk_of(b)?;
    let conv = trim(conv);
    if conv.is_empty() {
        return Err(
            "give conv: a channel (#name), a DM (@agent, or person), or a thread (#12)".into(),
        );
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
    if let Some(name) = conv.strip_prefix('#')
        && let Some(id) = named_in(&talk, name, space)?
    {
        return Ok(Target::Channel(id));
    }
    let co = b.coord();
    if let Ok(id) = co.task_ref(conv)
        && co.get_task(&id).is_some()
    {
        return Ok(Target::Thread(id));
    }
    if let Some(id) = conv.strip_prefix("task:")
        && co.get_task(trim(id)).is_some()
    {
        return Ok(Target::Thread(trim(id).to_string()));
    }
    if let Some(id) = named_in(&talk, conv, space)? {
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
    if let Some(m) = r.mentions.first() {
        let other = Member::of(&m.key);
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

/// Every post of `conv` (oldest first) as `reader` may read it: anyone
/// reads a channel or a thread; a DM, its parties and the person.
pub fn posts(b: &dyn Backend, reader: &str, conv: &str) -> Result<Vec<Post>, String> {
    let talk = talk_of(b)?;
    let conv = trim(conv);
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
    let co = b.coord();
    match co.get_task(conv) {
        Some(t) => Ok(thread_posts(co, &t)),
        None => Err(format!(
            "no conversation {}. talk_list shows yours.",
            one_line(conv, 80)
        )),
    }
}

/// Posts `text` to a channel or a DM as `from`. Mentions are `given`
/// (names or keys) and the `@names` in the text; each live one is woken
/// with a mention notice, and in a channel joins it. A DM with an agent
/// goes to it as a message that wakes it (the inbox's limits apply: a
/// refused message posts nothing). A thread isn't Talk's to write: post
/// there with the card's comments.
pub fn post(
    b: &dyn Backend,
    from: &Sender,
    target: &Target,
    text: &str,
    given: &[String],
    agents_now: bool,
    thread_from: Option<String>,
) -> Result<Posted, String> {
    let talk = talk_of(b)?;
    let by = from.key.clone();
    let text_ok = ct::check_text(&by, text).map_err(|e| e.to_string())?;
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
                    .members(id)
                    .into_iter()
                    .find(|m| ct::canon(&m.key) != ct::canon(&by))
                    .ok_or_else(|| {
                        "that DM has no one in it yet: name the other side, conv=@agent or conv=person"
                            .to_string()
                    })?,
            };
            if ct::canon(&other.key) == ct::canon(&by) {
                return Err("that's you: a DM needs another agent or the person".into());
            }
            if ct::dm_id(&by, &other.key) != *id {
                return Err("that DM isn't yours".into());
            }
            talk.open_dm(&Member::of(&by), &other, &by)
                .map_err(|e| e.to_string())?;
            if other.kind == ct::AGENT {
                let o = SendOptions {
                    mode: mode::WAKE.into(),
                    ref_: format!("talk:{id}"),
                    mentions: crate::mention::to_inbox(&res.mentions),
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
                        by: by.clone(),
                    })
                    .map_err(|e| e.to_string())?;
                out.post = p;
                let skip = other.key.clone();
                out.told = tell_mentions(b, from, &out.post, &live, &skip, "your DM");
                return Ok(out);
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
            by: by.clone(),
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
            text.push_str(&format!("- {who}: {}\n", one_line(&q.text, 200)));
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
    let text = format!(
        "{who} mentioned you in {where_} (Talk, conv={conv}):\n{body}\n\nAnswer there with talk_post conv={conv} if you have something to add; talk_read conv={conv} shows the rest.\n",
        conv = p.conv_id,
        body = p.text,
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
        return crate::board::board_comment(s, c, args.to_string().as_bytes());
    }
    match post(
        b,
        &c.sender,
        &target,
        a.str("text"),
        &a.strs("mention"),
        true,
        None,
    ) {
        Ok(p) => {
            let n = s.roster();
            let mut out = format!("Posted {} to {}.", p.post.post_id, p.post.conv_id);
            if let Some(d) = &p.sent {
                out += &s.outcome(d);
            }
            out += &crate::mention::mention_line(&outcomes(&p), &p.misses, &n);
            out
        }
        Err(e) => err_text(&e),
    }
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
    let cursor = if since.is_empty() {
        talk.cursor(me(c), &conv)
    } else {
        Some(since.to_string())
    };
    let after = ct::since(&all, cursor.as_deref());
    let (shown, head) = if after.is_empty() || (since.is_empty() && after.len() == all.len()) {
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
    let mut out = format!("{head} in {conv}, oldest first:\n");
    for p in &shown {
        out += &format!(
            "- [{}] {} ({}): {}\n",
            p.post_id,
            n.who(&p.by),
            ago(&now, &p.at),
            one_line(&p.text, READ_TEXT)
        );
    }
    if let Some(last) = shown.last() {
        let left = all.len()
            - 1
            - all
                .iter()
                .position(|p| p.post_id == last.post_id)
                .unwrap_or(0);
        if left > 0 {
            out += &format!(
                "{left} more: talk_read conv={conv} since={}\n",
                last.post_id
            );
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
