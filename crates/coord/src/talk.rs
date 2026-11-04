//! Talk (card #389; docs/design/talk-channels.md §2, §6): channels, direct
//! messages and the read cursors over them, folded from the `talk.*` rows
//! of the private log. Talk 1 keeps every row in the private log; Spaces
//! (Talk 4) move channels to a space's log later.
//!
//! Three kinds of conversation share one shape (a `convId` and its posts):
//!
//! - a **channel**: `convId` is its `channelId` (`ch_<id>`); permanent,
//!   with members who joined or were added (`talk.member`). Whoever makes
//!   a channel joins it, and so does anyone who posts in it.
//! - a **DM**: `convId` is [`dm_id`] of its two parties, so both sides
//!   (and every device of the person) name the same conversation without
//!   a row to agree on it. The first post writes a join for each party,
//!   which is how the list knows who a DM is between.
//! - a **thread**: `convId` is its task id. Its rows stay the ask's and the
//!   card's comments (the Core maps them into posts); nothing here writes
//!   them.
//!
//! Every device of the person writes as `user:<deviceId>`; Talk counts
//! those as one party ([`canon`]), for DM ids, members, read cursors and
//! unread counts alike.
//!
//! Posts sort by the row's place in the log (HLC, device, seq), as every
//! fold here does. A read cursor (`talk.read`, the person's own devices
//! only) names the newest post its reader has seen; the last one written
//! wins.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use gocompat::time::GoTime;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use store::{Event, Filter, Store, kinds};

use crate::board::{Error, ErrorKind, MAX_COMMENT, MAX_MENTIONS, MAX_PERSON_TEXT, Result, new_id};

/// The rows Talk folds.
pub const KINDS: [&str; 6] = [
    kinds::TALK_CHANNEL_CREATED,
    kinds::TALK_CHANNEL_EDITED,
    kinds::TALK_MEMBER,
    kinds::TALK_POST,
    kinds::TALK_READ,
    kinds::TALK_CARD,
];

/// A member's kind: the person.
pub const PERSON: &str = "person";
/// A member's kind: an agent.
pub const AGENT: &str = "agent";
/// `talk.member` ops.
pub const JOIN: &str = "join";
pub const LEAVE: &str = "leave";

/// What a conversation is.
pub const KIND_CHANNEL: &str = "channel";
pub const KIND_DM: &str = "dm";
pub const KIND_THREAD: &str = "thread";

/// A DM's `convId` starts with this.
pub const DM_PREFIX: &str = "dm:";
/// A channel's `convId` starts with this.
pub const CHANNEL_PREFIX: &str = "ch_";
/// The space every Talk 1 channel is in: empty is Personal (the private
/// log). The fold stores any spaceId as written, so a space log's
/// channels (Spaces, #392) fold the same.
pub const PERSONAL_SPACE: &str = "";

/// The longest channel name and topic, in characters.
pub const MAX_NAME: usize = 80;
pub const MAX_TOPIC: usize = 300;
/// The most posts one page returns.
pub const MAX_PAGE: usize = 200;

/// The one key every device of the person counts as.
const PERSON_KEY: &str = "user";

/// `key` as Talk counts it: every device's person key (`user:<deviceId>`)
/// is the person, `user`; any other key is itself.
pub fn canon(key: &str) -> &str {
    // Only this person's own device keys fold into one; on a space each
    // person's board key (person:<id>) is that person.
    if key == PERSON_KEY || key.starts_with("user:") {
        PERSON_KEY
    } else {
        key
    }
}

/// Whether `key` is the person's (on any of their devices).
pub fn is_person_key(key: &str) -> bool {
    key == PERSON_KEY || key.starts_with("user:") || key.starts_with("person:")
}

/// The DM between `a` and `b`: `dm:` and the first 16 hex digits of the
/// SHA-256 of the two [`canon`] keys, sorted, joined by `|`. Either side
/// computes the same id, on any device, with no row to agree on it.
pub fn dm_id(a: &str, b: &str) -> String {
    let (a, b) = (canon(a), canon(b));
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let sum = Sha256::digest(format!("{lo}|{hi}").as_bytes());
    format!("{DM_PREFIX}{}", &hex::encode(sum)[..16])
}

/// Whether `conv` names a DM.
pub fn is_dm(conv: &str) -> bool {
    conv.starts_with(DM_PREFIX)
}

/// Whether `conv` names a channel.
pub fn is_channel(conv: &str) -> bool {
    conv.starts_with(CHANNEL_PREFIX)
}

/// One side of a conversation: `{kind: person|agent, key}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub key: String,
}

impl Member {
    /// The member `key` is: the person or an agent.
    pub fn of(key: &str) -> Member {
        Member {
            kind: if is_person_key(key) { PERSON } else { AGENT }.into(),
            key: key.into(),
        }
    }

    fn same(&self, other: &Member) -> bool {
        canon(&self.key) == canon(&other.key)
    }
}

// ---- rows ---------------------------------------------------------------

/// `talk.channel.created`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChannelCreatedRow {
    pub channel_id: String,
    pub space_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    pub by: String,
}

/// `talk.channel.edited`: only the fields it sets change.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChannelEditedRow {
    pub channel_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
}

/// `talk.member`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MemberRow {
    pub conv_id: String,
    pub member: Member,
    pub op: String,
    pub by: String,
}

/// `talk.post`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PostRow {
    pub post_id: String,
    pub conv_id: String,
    pub text: String,
    pub mentions: Vec<String>,
    pub agents_now: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attach: Option<Vec<serde_json::Value>>,
    pub by: String,
    pub at: GoTime,
}

/// `talk.read`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReadRow {
    pub conv_id: String,
    pub up_to: String,
    pub by: String,
}

/// `talk.card`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CardRow {
    pub post_id: String,
    pub task_id: String,
}

// ---- folded state -------------------------------------------------------

/// A row's place in the log: `(hlc, device, seq)`.
pub type Ord3 = (u64, String, i64);

fn ord_of(ev: &Event) -> Ord3 {
    (ev.hlc.0, ev.device_id.clone(), ev.seq)
}

/// One channel as it stands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: String,
    pub space_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub topic: String,
    pub archived: bool,
    pub by: String,
    #[serde(skip)]
    created: Ord3,
    #[serde(skip)]
    name_at: Ord3,
    #[serde(skip)]
    topic_at: Ord3,
    #[serde(skip)]
    archived_at: Ord3,
}

/// One post, from a `talk.post` row or mapped from a thread's comment or
/// an agent's message (the Core's).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Post {
    pub post_id: String,
    pub conv_id: String,
    pub by: String,
    pub text: String,
    pub mentions: Vec<String>,
    pub agents_now: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_from: Option<String>,
    /// The card made from it (`talk.card`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
    pub at: GoTime,
    /// Where it sorts: its row's place in the log, or for a post mapped
    /// from elsewhere its time.
    #[serde(skip)]
    pub ord: Ord3,
}

#[derive(Debug, Clone)]
struct MemberState {
    member: Member,
    joined: bool,
    at: Ord3,
}

#[derive(Debug, Clone)]
struct Cursor {
    up_to: String,
    at: Ord3,
}

#[derive(Debug, Default)]
struct State {
    channels: BTreeMap<String, Channel>,
    members: HashMap<String, Vec<MemberState>>,
    posts: HashMap<String, Vec<Post>>,
    ids: HashSet<String>,
    reads: HashMap<(String, String), Cursor>,
    cards: HashMap<String, String>,
}

/// What to post.
#[derive(Debug, Clone, Default)]
pub struct NewPost {
    /// The post's id; empty makes one. The Core passes a message's id
    /// when the post also went out as that message (a DM to an agent).
    pub post_id: String,
    pub conv_id: String,
    pub text: String,
    /// The keys it mentions.
    pub mentions: Vec<String>,
    pub agents_now: bool,
    pub thread_from: Option<String>,
    pub by: String,
}

/// The `talk.*` fold, and the writes that append and fold a row. Safe for
/// concurrent use.
pub struct Talk {
    st: Option<Arc<Store>>,
    /// The space whose log this folds (card #392); empty for Personal, the
    /// private log.
    space: String,
    now: Arc<dyn Fn() -> GoTime + Send + Sync>,
    s: Mutex<State>,
}

impl std::fmt::Debug for Talk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Talk").finish_non_exhaustive()
    }
}

impl Talk {
    /// An empty fold writing to `st` (`None`: read-only, for tests that
    /// fold rows by hand), stamping posts with `now` (`None`: the clock).
    pub fn new(st: Option<Arc<Store>>, now: Option<Arc<dyn Fn() -> GoTime + Send + Sync>>) -> Talk {
        Talk {
            st,
            space: PERSONAL_SPACE.into(),
            now: now.unwrap_or_else(|| {
                Arc::new(|| GoTime::from_system(std::time::SystemTime::now()).utc())
            }),
            s: Mutex::new(State::default()),
        }
    }

    /// The fold of space `space`'s log (card #392). Its rows go on a space
    /// log, which takes only board keys: the writer is `person:<id>` or
    /// `agent:<cardId>`, never a private key.
    pub fn for_space(
        st: Arc<Store>,
        space: &str,
        now: Option<Arc<dyn Fn() -> GoTime + Send + Sync>>,
    ) -> Talk {
        let mut t = Talk::new(Some(st), now);
        t.space = space.to_string();
        t
    }

    /// The space this folds; empty for Personal.
    pub fn space(&self) -> &str {
        &self.space
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.s.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Folds every `talk.*` row in the store afresh (start, and a resync).
    pub fn load(&self) -> Result<()> {
        let Some(st) = self.st.clone() else {
            return Ok(());
        };
        let mut f = Filter {
            kinds: KINDS.iter().map(|k| k.to_string()).collect(),
            limit: store::MAX_LIST_LIMIT,
            ..Filter::default()
        };
        let mut fresh = State::default();
        loop {
            let evs = st.list(&f)?;
            for ev in &evs {
                fold(&mut fresh, ev);
            }
            if (evs.len() as i64) < f.limit {
                break;
            }
            f.after = evs[evs.len() - 1].cursor();
        }
        *self.lock() = fresh;
        Ok(())
    }

    /// Folds one row; reports whether anything Talk shows changed. Rows of
    /// other kinds, and a row folded before, change nothing.
    pub fn apply(&self, ev: &Event) -> bool {
        if !KINDS.contains(&ev.kind.as_str()) {
            return false;
        }
        fold(&mut self.lock(), ev)
    }

    fn write<P: Serialize>(&self, actor_key: &str, kind: &str, p: &P) -> Result<Event> {
        let Some(st) = &self.st else {
            return Err(Error::raw(ErrorKind::Store, "talk: no store"));
        };
        let actor = if !self.space.is_empty() {
            // A space log names people and agents by their board keys.
            if !(actor_key.starts_with("person:") || actor_key.starts_with("agent:")) {
                return Err(invalid(format!(
                    "a space names its writer by a board key (person: or agent:), not {actor_key}"
                )));
            }
            actor_key.to_string()
        } else if is_person_key(actor_key) {
            "user".to_string()
        } else {
            format!("agent:{actor_key}")
        };
        let ev = st.append(&actor, kind, p)?;
        fold(&mut self.lock(), &ev);
        Ok(ev)
    }

    // ---- writes ---------------------------------------------------------

    /// Makes a channel named `name` (unique among open channels, any
    /// case); `by` joins it.
    pub fn create_channel(&self, by: &str, name: &str, topic: &str) -> Result<Channel> {
        let by = need_by(by)?;
        let name = clean_name(name)?;
        let topic = clean_topic(topic)?;
        if self.channel_named(&name).is_some() {
            return Err(invalid(format!("there's already a channel named {name}")));
        }
        let id = format!("{CHANNEL_PREFIX}{}", new_id(&(self.now)()));
        self.write(
            by,
            kinds::TALK_CHANNEL_CREATED,
            &ChannelCreatedRow {
                channel_id: id.clone(),
                space_id: self.space.clone(),
                name,
                topic: (!topic.is_empty()).then_some(topic),
                by: by.into(),
            },
        )?;
        self.join(&id, &Member::of(by), by)?;
        self.channel(&id)
            .ok_or_else(|| Error::raw(ErrorKind::Store, "talk: the channel didn't fold"))
    }

    /// Adds `m` to conversation `conv` (a channel or a DM). Joining twice
    /// is a no-op (false).
    pub fn join(&self, conv: &str, m: &Member, by: &str) -> Result<bool> {
        self.member_op(conv, m, by, JOIN)
    }

    /// Takes `m` out of conversation `conv`. Leaving one it isn't in is a
    /// no-op (false).
    pub fn leave(&self, conv: &str, m: &Member, by: &str) -> Result<bool> {
        self.member_op(conv, m, by, LEAVE)
    }

    fn member_op(&self, conv: &str, m: &Member, by: &str, op: &str) -> Result<bool> {
        let by = need_by(by)?;
        let conv = conv.trim();
        if is_channel(conv) {
            if self.channel(conv).is_none() {
                return Err(not_found(format!("no channel {conv}")));
            }
        } else if !is_dm(conv) {
            return Err(invalid(format!(
                "{conv} isn't a channel or a DM (a thread's members are its card's)"
            )));
        }
        let m = clean_member(m)?;
        if self.is_member(conv, &m.key) == (op == JOIN) {
            return Ok(false);
        }
        self.write(
            by,
            kinds::TALK_MEMBER,
            &MemberRow {
                conv_id: conv.into(),
                member: m,
                op: op.into(),
                by: by.into(),
            },
        )?;
        Ok(true)
    }

    /// The DM between `a` and `b`, with both joined (written the first
    /// time). `by` is whoever opens it.
    pub fn open_dm(&self, a: &Member, b: &Member, by: &str) -> Result<String> {
        let (a, b) = (clean_member(a)?, clean_member(b)?);
        if a.same(&b) {
            return Err(invalid("a DM needs two different parties"));
        }
        let id = dm_id(&a.key, &b.key);
        self.join(&id, &a, by)?;
        self.join(&id, &b, by)?;
        Ok(id)
    }

    /// Posts to a channel or a DM. The text is trimmed and must not be
    /// empty: at most [`MAX_COMMENT`] characters from an agent, and
    /// [`MAX_PERSON_TEXT`] bytes from the person, as comments. A post in a
    /// channel joins its writer.
    pub fn post(&self, p: NewPost) -> Result<Post> {
        let by = need_by(&p.by)?.to_string();
        let conv = p.conv_id.trim().to_string();
        let text = check_text(&by, &p.text)?;
        if is_channel(&conv) {
            match self.channel(&conv) {
                None => return Err(not_found(format!("no channel {conv}"))),
                Some(c) if c.archived => {
                    return Err(invalid(format!("#{} is archived", c.name)));
                }
                Some(_) => {}
            }
        } else if !is_dm(&conv) {
            return Err(invalid(format!(
                "{conv} isn't a channel or a DM (a thread's posts are its comments)"
            )));
        } else if !self.is_member(&conv, &by) {
            return Err(Error::raw(
                ErrorKind::NotAllowed,
                format!("talk: {by} isn't in DM {conv}"),
            ));
        }
        let mut mentions: Vec<String> = Vec::new();
        for m in &p.mentions {
            let m = m.trim();
            if !m.is_empty() && canon(m) != canon(&by) && !mentions.iter().any(|x| x == m) {
                mentions.push(m.to_string());
            }
        }
        if mentions.len() > MAX_MENTIONS {
            return Err(invalid(format!("mention at most {MAX_MENTIONS} agents")));
        }
        let thread_from = p
            .thread_from
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Some(t) = &thread_from
            && !self
                .lock()
                .posts
                .get(&conv)
                .is_some_and(|ps| ps.iter().any(|x| x.post_id == *t))
        {
            return Err(not_found(format!("no post {t} in {conv}")));
        }
        let post_id = if p.post_id.trim().is_empty() {
            format!("post_{}", new_id(&(self.now)()))
        } else {
            p.post_id.trim().to_string()
        };
        if self.lock().ids.contains(&post_id) {
            return Err(invalid(format!("post {post_id} already exists")));
        }
        if is_channel(&conv) {
            self.join(&conv, &Member::of(&by), &by)?;
        }
        self.write(
            &by,
            kinds::TALK_POST,
            &PostRow {
                post_id: post_id.clone(),
                conv_id: conv.clone(),
                text,
                mentions,
                agents_now: p.agents_now,
                thread_from,
                attach: None,
                by: by.clone(),
                at: (self.now)(),
            },
        )?;
        self.lock()
            .posts
            .get(&conv)
            .and_then(|ps| ps.iter().find(|x| x.post_id == post_id).cloned())
            .ok_or_else(|| Error::raw(ErrorKind::Store, "talk: the post didn't fold"))
    }

    /// Records that `by` has read conversation `conv` up to post `up_to`.
    /// Only the person's own devices write cursors (the row never goes on a
    /// shared board); an agent's reading isn't kept.
    pub fn mark_read(&self, conv: &str, up_to: &str, by: &str) -> Result<()> {
        let by = need_by(by)?;
        let (conv, up_to) = (conv.trim(), up_to.trim());
        if conv.is_empty() || up_to.is_empty() {
            return Err(invalid("give the conversation and the post read up to"));
        }
        if self.cursor(by, conv).as_deref() == Some(up_to) {
            return Ok(());
        }
        self.write(
            by,
            kinds::TALK_READ,
            &ReadRow {
                conv_id: conv.into(),
                up_to: up_to.into(),
                by: by.into(),
            },
        )?;
        Ok(())
    }

    // ---- reads ----------------------------------------------------------

    /// Every channel, archived ones too, by name.
    pub fn channels(&self) -> Vec<Channel> {
        let mut out: Vec<Channel> = self.lock().channels.values().cloned().collect();
        out.sort_by_key(|a| a.name.to_lowercase());
        out
    }

    pub fn channel(&self, id: &str) -> Option<Channel> {
        self.lock().channels.get(id.trim()).cloned()
    }

    /// The open channel named `name` (any case, a leading `#` dropped).
    pub fn channel_named(&self, name: &str) -> Option<Channel> {
        let want = name.trim().trim_start_matches('#').to_lowercase();
        self.lock()
            .channels
            .values()
            .find(|c| !c.archived && c.name.to_lowercase() == want)
            .cloned()
    }

    /// Who is in `conv` now, in the order they joined.
    pub fn members(&self, conv: &str) -> Vec<Member> {
        let s = self.lock();
        let mut ms: Vec<&MemberState> = s
            .members
            .get(conv)
            .map(|v| v.iter().filter(|m| m.joined).collect())
            .unwrap_or_default();
        ms.sort_by(|a, b| a.at.cmp(&b.at));
        ms.into_iter().map(|m| m.member.clone()).collect()
    }

    /// Whether `key` (any of the person's keys counts as the person) is in
    /// `conv`.
    pub fn is_member(&self, conv: &str, key: &str) -> bool {
        let k = canon(key);
        self.lock()
            .members
            .get(conv)
            .is_some_and(|v| v.iter().any(|m| m.joined && canon(&m.member.key) == k))
    }

    /// Every DM with a member row, and who it's between.
    pub fn dms(&self) -> Vec<(String, Vec<Member>)> {
        let ids: Vec<String> = self
            .lock()
            .members
            .keys()
            .filter(|k| is_dm(k))
            .cloned()
            .collect();
        ids.into_iter()
            .map(|id| {
                let ms = self.members(&id);
                (id, ms)
            })
            .collect()
    }

    /// The posts of `conv`, oldest first.
    pub fn posts(&self, conv: &str) -> Vec<Post> {
        self.lock()
            .posts
            .get(conv.trim())
            .cloned()
            .unwrap_or_default()
    }

    /// Whether a `talk.post` has id `id`.
    pub fn has_post(&self, id: &str) -> bool {
        self.lock().ids.contains(id)
    }

    /// Every post id (the Core leaves these messages out of a DM's
    /// mirrored history: they are already its posts).
    pub fn post_ids(&self) -> HashSet<String> {
        self.lock().ids.clone()
    }

    /// The post `reader` has read `conv` up to, if any.
    pub fn cursor(&self, reader: &str, conv: &str) -> Option<String> {
        self.lock()
            .reads
            .get(&(canon(reader).to_string(), conv.trim().to_string()))
            .map(|c| c.up_to.clone())
    }

    /// The card made from post `id` (`talk.card`).
    pub fn card_of(&self, id: &str) -> Option<String> {
        self.lock().cards.get(id).cloned()
    }
}

/// Folds one row into `s`; whether it changed anything.
fn fold(s: &mut State, ev: &Event) -> bool {
    let at = ord_of(ev);
    match ev.kind.as_str() {
        kinds::TALK_CHANNEL_CREATED => {
            let Ok(r) = serde_json::from_slice::<ChannelCreatedRow>(&ev.payload) else {
                return false;
            };
            if r.channel_id.is_empty() {
                return false;
            }
            let c = s.channels.entry(r.channel_id.clone()).or_default();
            if !c.id.is_empty() && c.created <= at {
                return false; // folded before, or an older copy wins
            }
            c.id = r.channel_id;
            c.space_id = r.space_id;
            c.by = r.by;
            c.created = at.clone();
            if c.name_at < at {
                c.name = r.name;
                c.name_at = at.clone();
            }
            if c.topic_at < at {
                c.topic = r.topic.unwrap_or_default();
                c.topic_at = at;
            }
            true
        }
        kinds::TALK_CHANNEL_EDITED => {
            let Ok(r) = serde_json::from_slice::<ChannelEditedRow>(&ev.payload) else {
                return false;
            };
            if r.channel_id.is_empty() {
                return false;
            }
            // An edit may arrive before its channel: the fields wait in a
            // channel with no id until the created row comes.
            let c = s.channels.entry(r.channel_id).or_default();
            let mut changed = false;
            if let Some(n) = r.name
                && c.name_at < at
            {
                c.name = n;
                c.name_at = at.clone();
                changed = true;
            }
            if let Some(t) = r.topic
                && c.topic_at < at
            {
                c.topic = t;
                c.topic_at = at.clone();
                changed = true;
            }
            if let Some(a) = r.archived
                && c.archived_at < at
            {
                c.archived = a;
                c.archived_at = at;
                changed = true;
            }
            changed && !c.id.is_empty()
        }
        kinds::TALK_MEMBER => {
            let Ok(r) = serde_json::from_slice::<MemberRow>(&ev.payload) else {
                return false;
            };
            if r.conv_id.is_empty() || r.member.key.is_empty() {
                return false;
            }
            let joined = match r.op.as_str() {
                JOIN => true,
                LEAVE => false,
                _ => return false,
            };
            let v = s.members.entry(r.conv_id).or_default();
            match v.iter_mut().find(|m| m.member.same(&r.member)) {
                Some(m) => {
                    if m.at >= at {
                        return false;
                    }
                    let changed = m.joined != joined;
                    m.joined = joined;
                    m.at = at;
                    changed
                }
                None => {
                    v.push(MemberState {
                        member: r.member,
                        joined,
                        at,
                    });
                    true
                }
            }
        }
        kinds::TALK_POST => {
            let Ok(r) = serde_json::from_slice::<PostRow>(&ev.payload) else {
                return false;
            };
            if r.post_id.is_empty() || r.conv_id.is_empty() || !s.ids.insert(r.post_id.clone()) {
                return false;
            }
            let p = Post {
                card: s.cards.get(&r.post_id).cloned(),
                post_id: r.post_id,
                conv_id: r.conv_id.clone(),
                by: r.by,
                text: r.text,
                mentions: r.mentions,
                agents_now: r.agents_now,
                thread_from: r.thread_from,
                at: r.at,
                ord: at,
            };
            let v = s.posts.entry(r.conv_id).or_default();
            let i = v.partition_point(|x| x.ord <= p.ord);
            v.insert(i, p);
            true
        }
        kinds::TALK_READ => {
            let Ok(r) = serde_json::from_slice::<ReadRow>(&ev.payload) else {
                return false;
            };
            if r.conv_id.is_empty() || r.up_to.is_empty() || r.by.is_empty() {
                return false;
            }
            let k = (canon(&r.by).to_string(), r.conv_id);
            if s.reads.get(&k).is_some_and(|c| c.at >= at) {
                return false;
            }
            s.reads.insert(k, Cursor { up_to: r.up_to, at });
            true
        }
        kinds::TALK_CARD => {
            let Ok(r) = serde_json::from_slice::<CardRow>(&ev.payload) else {
                return false;
            };
            if r.post_id.is_empty() || r.task_id.is_empty() {
                return false;
            }
            for ps in s.posts.values_mut() {
                if let Some(p) = ps.iter_mut().find(|p| p.post_id == r.post_id) {
                    p.card = Some(r.task_id.clone());
                }
            }
            s.cards.insert(r.post_id, r.task_id);
            true
        }
        _ => false,
    }
}

// ---- pages and counts (over any conversation's posts) ---------------------

/// Up to `limit` posts before post `before` (`None`: the newest), oldest
/// first. An unknown `before` gives the newest.
pub fn page(posts: &[Post], before: Option<&str>, limit: usize) -> Vec<Post> {
    let limit = limit.clamp(1, MAX_PAGE);
    let end = before
        .filter(|b| !b.is_empty())
        .and_then(|b| posts.iter().position(|p| p.post_id == b))
        .unwrap_or(posts.len());
    posts[end.saturating_sub(limit)..end].to_vec()
}

/// The posts after `cursor` (all of them when there is none, or it names
/// no post here).
pub fn since<'a>(posts: &'a [Post], cursor: Option<&str>) -> &'a [Post] {
    match cursor.and_then(|c| posts.iter().position(|p| p.post_id == c)) {
        Some(i) => &posts[i + 1..],
        None => posts,
    }
}

/// `reader`'s unread posts after `cursor` (not their own), and how many of
/// those mention them.
pub fn unread(posts: &[Post], cursor: Option<&str>, reader: &str) -> (usize, usize) {
    let me = canon(reader);
    let mut n = 0;
    let mut mentions = 0;
    for p in since(posts, cursor) {
        if canon(&p.by) == me {
            continue;
        }
        n += 1;
        if p.mentions.iter().any(|m| canon(m) == me) {
            mentions += 1;
        }
    }
    (n, mentions)
}

/// Sorts mixed posts (rows and mapped ones) by time, then place.
pub fn sort_posts(posts: &mut [Post]) {
    posts.sort_by(|a, b| {
        crate::gotime::cmp(&a.at, &b.at)
            .then_with(|| a.ord.cmp(&b.ord))
            .then_with(|| a.post_id.cmp(&b.post_id))
    });
}

// ---- checks -------------------------------------------------------------

fn invalid(detail: impl std::fmt::Display) -> Error {
    Error::raw(ErrorKind::Invalid, format!("talk: {detail}"))
}

fn not_found(detail: impl std::fmt::Display) -> Error {
    Error::raw(ErrorKind::NotFound, format!("talk: {detail}"))
}

fn need_by(by: &str) -> Result<&str> {
    let by = by.trim();
    if by.is_empty() {
        return Err(invalid("no caller"));
    }
    Ok(by)
}

fn one_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A channel name as kept: one line, trimmed, no leading `#`, 1 to
/// [`MAX_NAME`] characters.
pub fn clean_name(name: &str) -> Result<String> {
    let n = one_line(name.trim().trim_start_matches('#'));
    if n.is_empty() {
        return Err(invalid("give the channel a name"));
    }
    if n.chars().count() > MAX_NAME {
        return Err(invalid(format!(
            "a channel name is at most {MAX_NAME} characters"
        )));
    }
    Ok(n)
}

fn clean_topic(topic: &str) -> Result<String> {
    let t = one_line(topic);
    if t.chars().count() > MAX_TOPIC {
        return Err(invalid(format!(
            "a topic is at most {MAX_TOPIC} characters"
        )));
    }
    Ok(t)
}

fn clean_member(m: &Member) -> Result<Member> {
    let key = m.key.trim();
    if key.is_empty() {
        return Err(invalid("a member needs a key"));
    }
    let kind = m.kind.trim();
    let kind = if kind.is_empty() {
        Member::of(key).kind
    } else if kind == PERSON || kind == AGENT {
        kind.to_string()
    } else {
        return Err(invalid(format!(
            "a member is a person or an agent, not {kind}"
        )));
    };
    Ok(Member {
        kind,
        key: key.into(),
    })
}

/// A post's text, trimmed and checked as a comment's is.
pub fn check_text(by: &str, text: &str) -> Result<String> {
    let t = text.trim();
    if t.is_empty() {
        return Err(invalid("an empty post"));
    }
    if is_person_key(by) {
        if t.len() > MAX_PERSON_TEXT {
            return Err(invalid(format!(
                "a post is at most {} KB",
                MAX_PERSON_TEXT >> 10
            )));
        }
    } else if t.chars().count() > MAX_COMMENT {
        return Err(invalid(format!(
            "a post is at most {MAX_COMMENT} characters"
        )));
    }
    Ok(t.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dm_id_is_one_per_pair() {
        assert_eq!(dm_id("a", "b"), dm_id("b", "a"));
        assert_ne!(dm_id("a", "b"), dm_id("a", "c"));
        assert_eq!(dm_id("user:dev-a", "x"), dm_id("x", "user:dev-b"));
        assert!(dm_id("a", "b").starts_with(DM_PREFIX));
        assert_eq!(dm_id("a", "b").len(), DM_PREFIX.len() + 16);
    }
}
