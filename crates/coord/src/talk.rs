//! Talk (card #389; docs/design/talk-channels.md §2, §6): channels, direct
//! messages and the read cursors over them, folded from the `talk.*` rows
//! of the private log, and of every space's log (card #392).
//!
//! A channel lives where its `spaceId` says: empty is Personal (the
//! private log); otherwise the space's log, so it replicates to the
//! space's members. Its `talk.member` and `talk.post` rows go to the same
//! log. DMs with an agent and read cursors stay in the private log; a DM
//! between two people rides on a hidden two-person space (card #538,
//! docs/design/talk-dms.md). The Core writes and reads a space's rows
//! through [`SpaceLogs`], which turns this device's keys into the space's
//! (`person:<id>`, `agent:<cardId>`) and back, and refuses a write until
//! every device in the space reads `talk.*` rows.
//!
//! Three kinds of conversation share one shape (a `convId` and its posts):
//!
//! - a **channel**: `convId` is its `channelId` (`ch_<id>`); permanent,
//!   with members who joined or were added (`talk.member`). Whoever makes
//!   a channel joins it, and so does anyone who posts in it.
//! - a **DM**: `convId` is [`dm_id`] of its two parties, or [`pair_dm_id`]
//!   for two people, so both sides (and every device of the person) name
//!   the same conversation without a row to agree on it. The first post
//!   writes a join for each party, which is how the list knows who a DM
//!   is between. Opening a DM with a person writes nothing: it waits as a
//!   draft in this fold until the first post.
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

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use gocompat::time::GoTime;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use store::kinds::PERSON_ACTOR_PREFIX;
use store::{Event, Filter, Store, kinds};

use crate::board::{Error, ErrorKind, MAX_COMMENT, MAX_MENTIONS, MAX_PERSON_TEXT, Result, new_id};

/// The rows Talk folds.
pub const KINDS: [&str; 9] = [
    kinds::TALK_CHANNEL_CREATED,
    kinds::TALK_CHANNEL_EDITED,
    kinds::TALK_MEMBER,
    kinds::TALK_POST,
    kinds::TALK_READ,
    kinds::TALK_CARD,
    kinds::CALL_SEGMENT,
    kinds::CALL_TITLE,
    kinds::CALL_ASSIST,
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
/// What [`pair_dm_id`] hashes before the two keys, so its ids never meet
/// [`dm_id`]'s.
pub const PAIR_DM_DOMAIN: &str = "irori.dm.pair.1";
/// A channel's `convId` starts with this.
pub const CHANNEL_PREFIX: &str = "ch_";
/// A huddle's transcript (card #656) reads as conversation `call:<callId>`.
/// No row names it: its lines are in the call's own conversation's log.
pub const CALL_PREFIX: &str = "call:";
/// The space every Talk 1 channel is in: empty is Personal (the private
/// log). The fold stores any spaceId as written, so a space log's
/// channels (Spaces, #392) fold the same.
pub const PERSONAL_SPACE: &str = "";

/// The space logs Talk writes a space's channel rows to and folds them
/// from (card #392). The Core implements it over its space logs.
pub trait SpaceLogs: Send + Sync {
    /// Appends a `talk.*` row (`payload`, with this device's keys) to
    /// space `space`'s log as `by`, and returns the row as Talk folds it:
    /// keys back in this device's form. Refused while the space can't take
    /// it (not a member, or a device in it doesn't read `talk.*` rows yet).
    fn append(
        &self,
        space: &str,
        by: &str,
        kind: &str,
        payload: serde_json::Value,
    ) -> std::result::Result<Event, String>;
    /// Every `talk.*` row of every open space log, keys in this device's
    /// form.
    fn rows(&self) -> Vec<Event>;
    /// This person's id. `None`: this Core has no person DMs, and a
    /// `person:` key is a member like any other.
    fn person_id(&self) -> Option<String> {
        None
    }
    /// The space a DM with `other` (`person:<id>`) is written to now: the
    /// one in use, or a new hidden one (card #538). Refused when this
    /// person shares no space with `other`.
    fn dm_for(&self, other: &str) -> std::result::Result<String, String> {
        Err(format!("no DM space for {other} here"))
    }
    /// The other person (`person:<id>`) of person DM `conv`, by the hidden
    /// spaces this device holds. A device that holds a DM's space may not
    /// have folded any of its rows yet.
    fn dm_route(&self, _conv: &str) -> Option<String> {
        None
    }
}

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
    if is_person_key(key) { PERSON_KEY } else { key }
}

/// Whether `key` is the person's (on any of their devices).
pub fn is_person_key(key: &str) -> bool {
    key == PERSON_KEY || key.starts_with("user:")
}

/// Whether `key` is another person in a space (`person:<personId>`, as
/// their rows in a space's log name them). This person's own rows come
/// back as `user:<deviceId>` ([`SpaceLogs`]).
pub fn is_other_person_key(key: &str) -> bool {
    key.starts_with("person:")
        || key
            .strip_prefix("board:")
            .and_then(|r| r.split_once('/'))
            .is_some_and(|(_, who)| who.starts_with("person:"))
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

/// The DM between two people (card #538): `dm:` and the first 16 hex
/// digits of the SHA-256 of [`PAIR_DM_DOMAIN`] and the two `person:<id>`
/// keys, sorted, joined by `|`. Both people's Cores compute the same id, so
/// its keys are never [`canon`]ed: that would turn each side's own key
/// into `user`, and the two would disagree.
pub fn pair_dm_id(a: &str, b: &str) -> String {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let sum = Sha256::digest(format!("{PAIR_DM_DOMAIN}|{lo}|{hi}").as_bytes());
    format!("{DM_PREFIX}{}", &hex::encode(sum)[..16])
}

/// What [`assistants_channel_id`] hashes before the space id.
pub const ASSISTANTS_DOMAIN: &str = "irori.assistants.channel.1";
/// A space's assistants channel's id starts with this. Ordinary channel ids
/// are `ch_` and an uppercase ULID, so `oa_` is unambiguous.
pub const ASSISTANTS_PREFIX: &str = "ch_oa_";
/// The assistants channel's name and topic (assistants-talk.md §4).
pub const ASSISTANTS_NAME: &str = "office-assistants";
pub const ASSISTANTS_TOPIC: &str = "Ask anyone's assistant. It answers from what is on this space.";

/// Space `space`'s assistants channel (assistants-talk.md §4): `ch_oa_` and
/// the first 16 hex digits of the SHA-256 of [`ASSISTANTS_DOMAIN`] and the
/// space id. Every member's Core computes the same id, so the fold keeps
/// one channel however many of them write it.
pub fn assistants_channel_id(space: &str) -> String {
    let sum = Sha256::digest(format!("{ASSISTANTS_DOMAIN}|{space}").as_bytes());
    format!("{ASSISTANTS_PREFIX}{}", &hex::encode(sum)[..16])
}

/// Whether `id` is space `space`'s assistants channel.
pub fn is_assistants_channel(id: &str, space: &str) -> bool {
    !space.is_empty() && id == assistants_channel_id(space)
}

/// Whether `id` is in the assistants channels' id range, whichever space.
fn in_assistants_range(id: &str) -> bool {
    id.starts_with(ASSISTANTS_PREFIX)
}

/// Whether `conv` names a DM.
pub fn is_dm(conv: &str) -> bool {
    conv.starts_with(DM_PREFIX)
}

/// Whether `conv` names a channel.
pub fn is_channel(conv: &str) -> bool {
    conv.starts_with(CHANNEL_PREFIX)
}

/// Whether `conv` names a huddle's transcript (card #656).
pub fn is_call(conv: &str) -> bool {
    conv.starts_with(CALL_PREFIX)
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
            kind: if is_person_key(key) || is_other_person_key(key) {
                PERSON
            } else {
                AGENT
            }
            .into(),
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
    /// The files sent with it (card #643), as a comment's: the bytes stay
    /// on the attaching device until another one fetches them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attach: Vec<blob::FileRef>,
    /// A huddle's start or end (#654). An older reader ignores it and
    /// shows the post's text; a mark this version can't read is dropped
    /// the same way, never the post.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_call"
    )]
    pub call: Option<CallMark>,
    /// A huddle's lines kept back while its space couldn't take them as
    /// `call.segment` rows (card #656). An older reader shows the post's
    /// text; a mark this version can't read is dropped, never the post.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_transcript"
    )]
    pub transcript: Option<TranscriptMark>,
    pub by: String,
    pub at: GoTime,
}

/// A transcript mark, or None for one of a shape this version doesn't know.
fn lenient_transcript<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<TranscriptMark>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).ok())
}

/// A fallback post's lines (card #656): which call, and each phrase the
/// writer said, at Unix ms.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TranscriptMark {
    pub call_id: String,
    pub lines: Vec<TranscriptLineMark>,
}

/// One line of a [`TranscriptMark`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TranscriptLineMark {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

/// A call mark, or None for one of a shape this version doesn't know.
fn lenient_call<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<CallMark>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).ok())
}

/// What a huddle's start or end post carries (#654): which call, and for
/// an end, how long it ran and who was in it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CallMark {
    pub call_id: String,
    /// [`CALL_START`] or [`CALL_END`].
    pub op: String,
    /// How long the call ran, on an end; 0 on a start.
    #[serde(skip_serializing_if = "is_zero")]
    pub duration_ms: i64,
    /// The people who took part, by key in this device's form, as
    /// `mentions` are (#654). Empty on a start.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<String>,
    /// The huddle has a transcript (card #656), on an end.
    #[serde(default, skip_serializing_if = "is_false")]
    pub transcript: bool,
}

/// A [`CallMark`]'s op when a huddle starts.
pub const CALL_START: &str = "start";
/// A [`CallMark`]'s op when a huddle ends.
pub const CALL_END: &str = "end";

fn is_zero(n: &i64) -> bool {
    *n == 0
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A call mark as written: an id, a known op, no negative length, each
/// person once.
fn check_call(mut c: CallMark) -> Result<CallMark> {
    c.call_id = c.call_id.trim().to_string();
    if c.call_id.is_empty() {
        return Err(invalid("a call mark needs its call's id"));
    }
    if c.op != CALL_START && c.op != CALL_END {
        return Err(invalid(format!(
            "a call mark's op is {CALL_START} or {CALL_END}, not {:?}",
            c.op
        )));
    }
    c.duration_ms = c.duration_ms.max(0);
    let mut people: Vec<String> = Vec::new();
    for k in &c.people {
        let k = k.trim();
        if !k.is_empty() && !people.iter().any(|x| x == k) {
            people.push(k.to_string());
        }
    }
    c.people = people;
    Ok(c)
}

/// A transcript mark as written or folded: an id, and only lines with
/// words and at most [`MAX_PERSON_TEXT`] bytes, each ending no earlier
/// than it starts.
fn check_transcript(mut t: TranscriptMark) -> Result<TranscriptMark> {
    t.call_id = t.call_id.trim().to_string();
    if t.call_id.is_empty() {
        return Err(invalid("a transcript needs its call's id"));
    }
    t.lines.retain_mut(|l| {
        l.text = l.text.trim().to_string();
        l.end_ms = l.end_ms.max(l.start_ms);
        !l.text.is_empty() && l.text.len() <= MAX_PERSON_TEXT
    });
    if t.lines.is_empty() {
        return Err(invalid("a transcript with no lines"));
    }
    Ok(t)
}

/// `call.segment` (card #656): one finished phrase of a huddle, in the
/// call's conversation's log, written by the speaker's own device.
/// `startMs`/`endMs` are Unix ms; `seg` counts from 0 per device per call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SegmentRow {
    pub call_id: String,
    pub conv_id: String,
    pub seg: i64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    /// The speaker's notice allowed the huddle's assistants and agents to
    /// read this line (call-agents.md section 10). Written `false` until the
    /// notice that sets it ships; an older row reads as `false`.
    pub agents: bool,
    pub by: String,
    pub at: GoTime,
}

/// The longest huddle title, in bytes.
pub const MAX_TITLE: usize = 200;

/// `call.title` (card #1043): a huddle's title, set by a person or proposed
/// by an assistant (`assistant: true`, written by the person's device under
/// their own key). A person's newest title beats an assistant's.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TitleRow {
    pub call_id: String,
    pub conv_id: String,
    pub title: String,
    pub assistant: bool,
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
    /// A huddle line the speaker's notice let agents read (call-agents.md
    /// section 10); false on every other post.
    #[serde(skip_serializing_if = "is_false")]
    pub agents: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_from: Option<String>,
    /// The card made from it (`talk.card`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
    /// The files sent with it (card #643).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<blob::FileRef>,
    /// A huddle's start or end (#654).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call: Option<CallMark>,
    /// A fallback post's lines (card #656). Such a post is folded into its
    /// call's transcript, not listed in its conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript: Option<TranscriptMark>,
    pub at: GoTime,
    /// Where it sorts: its row's place in the log, or for a post mapped
    /// from elsewhere its time.
    #[serde(skip)]
    pub ord: Ord3,
    /// The space whose log holds it; empty for the private log. A person
    /// DM can be in two logs, and only one may go (card #538).
    #[serde(skip)]
    pub space: String,
    /// The device that wrote its row (card #659); empty for a post mapped
    /// from elsewhere.
    #[serde(skip)]
    pub device: String,
}

/// A file on a Talk post, where it is (card #659): see [`Talk::attachments`].
#[derive(Debug, Clone, PartialEq)]
pub struct PostFile {
    pub conv: String,
    /// The space whose log holds the post; empty for the private log.
    pub space: String,
    /// The device that wrote the post's row.
    pub device: String,
    pub file: blob::FileRef,
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
    /// Where each post's link in `cards` came from.
    card_at: HashMap<String, Ord3>,
    /// The spaces each person DM has rows in (card #538): both people can
    /// write first, so one conversation can span two logs.
    dm_logs: HashMap<String, BTreeSet<String>>,
    /// Person DMs opened but not written in yet, with their two sides. Not
    /// in any log, so this device's alone; a refold keeps them.
    drafts: BTreeMap<String, [Member; 2]>,
    /// Each huddle's marks and lines, by call id (card #656).
    calls: HashMap<String, CallIdx>,
    /// Each huddle's titles (card #1043), kept apart from `calls` so a
    /// title that names the wrong conversation can't bind a huddle to it,
    /// and so the fold doesn't depend on which row came first.
    titles: HashMap<String, Vec<TitleRec>>,
    /// Each huddle's `call.assist` rows (card #1049), kept apart from
    /// `calls` as `titles` are.
    assists: HashMap<String, Vec<AssistRec>>,
}

/// `call.assist` (card #1049, call-agents.md §7, §8): a person's assistant is
/// hosted (`on`) or not (`off`) on `device` for a huddle. Written by the
/// hosting person's own device; the newest per device wins.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AssistRow {
    pub call_id: String,
    pub conv_id: String,
    /// The device that hosts (or stopped hosting) the assistant.
    pub device: String,
    /// `on` or `off`.
    pub state: String,
    /// Whether this assistant is the huddle's chair (card 10); written
    /// `false` until then.
    pub chair: bool,
    pub by: String,
    pub at: GoTime,
}

/// What `call.assist`'s `state` says when the assistant is hosted.
pub const ASSIST_ON: &str = "on";
/// What `call.assist`'s `state` says when it stopped.
pub const ASSIST_OFF: &str = "off";

/// One `call.assist` row as folded: the newest per conversation, person and
/// device.
#[derive(Debug, Clone)]
struct AssistRec {
    conv: String,
    device: String,
    /// The writing person, as the log names them.
    by: String,
    on: bool,
    chair: bool,
    at: Ord3,
}

/// A device's newest word on hosting a person's assistant in a huddle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistInfo {
    pub device: String,
    /// The person, as the log names them (`user:<device>` for this one).
    pub by: String,
    pub on: bool,
    pub chair: bool,
}

/// One `call.title` row as folded: the newest per conversation and kind.
#[derive(Debug, Clone)]
struct TitleRec {
    conv: String,
    title: String,
    assistant: bool,
    at: Ord3,
}

/// One huddle as Talk has it (card #656).
#[derive(Debug, Default)]
struct CallIdx {
    /// The conversation it is in; its rows may name no other.
    conv: String,
    /// Its start and end posts.
    marks: Vec<Post>,
    /// Its lines, from `call.segment` rows and fallback posts.
    lines: Vec<Post>,
    /// The fallback posts its lines came from, kept out of the
    /// conversation's posts.
    fallback: Vec<Post>,
}

impl CallIdx {
    fn is_empty(&self) -> bool {
        self.marks.is_empty() && self.lines.is_empty() && self.fallback.is_empty()
    }
}

/// A huddle with a mark or a line here (card #656): see [`Talk::calls`].
#[derive(Debug, Clone, PartialEq)]
pub struct CallInfo {
    pub call_id: String,
    /// The channel or DM it was in.
    pub conv_id: String,
    /// When it started: its start mark's time, when that is here.
    pub start_at: Option<GoTime>,
    /// Its end mark, once the huddle has ended.
    pub end: Option<CallMark>,
    /// The newest of its marks and lines.
    pub last: GoTime,
    /// How many lines its transcript has.
    pub lines: usize,
    /// Its title (card #1043): a person's newest, else an assistant's
    /// newest, else empty.
    pub title: String,
    /// Whether [`CallInfo::title`] was proposed by an assistant.
    pub title_by_assistant: bool,
    /// Who took part: the end mark's people once it ended; before that (and
    /// after a crash, which leaves no end mark) the start mark's author and
    /// everyone who spoke, by key in this device's form, each once.
    pub people: Vec<String>,
    /// The newest of its transcript lines, when it has any.
    pub last_line: Option<GoTime>,
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
    /// The files to send with it (card #643): at most
    /// [`crate::board::MAX_ATTACHMENTS`]. With files the text may be empty.
    pub files: Vec<blob::FileRef>,
    pub by: String,
    /// Post without joining the channel (card #454): an agent Irori
    /// started for a card or a thread is a member of no channel
    /// (spawned-agents.md §4 "Channels"), so it gets no next-turn readings.
    pub stay_out: bool,
    /// A huddle's start or end (#654).
    pub call: Option<CallMark>,
    /// A huddle's lines, when its space couldn't take them as rows (card
    /// #656): the post goes into the call's transcript instead of the
    /// conversation.
    pub transcript: Option<TranscriptMark>,
}

/// The `talk.*` fold, and the writes that append and fold a row. Safe for
/// concurrent use.
pub struct Talk {
    st: Option<Arc<Store>>,
    spaces: std::sync::OnceLock<Arc<dyn SpaceLogs>>,
    now: Arc<dyn Fn() -> GoTime + Send + Sync>,
    s: Mutex<State>,
    /// Counts drop_space calls, so a load that raced one folds again
    /// instead of bringing the dropped space back (card #461).
    drops: std::sync::atomic::AtomicU64,
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
            spaces: std::sync::OnceLock::new(),
            now: now.unwrap_or_else(|| {
                Arc::new(|| GoTime::from_system(std::time::SystemTime::now()).utc())
            }),
            s: Mutex::new(State::default()),
            drops: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Where a space's channel rows go and come from (card #392); set
    /// once, by the Core. Without it a space's channel is refused.
    pub fn set_spaces(&self, spaces: Arc<dyn SpaceLogs>) {
        let _ = self.spaces.set(spaces);
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.s.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Folds every `talk.*` row in the store, and in every space's log,
    /// afresh (start, and a resync of either).
    pub fn load(&self) -> Result<()> {
        let Some(st) = self.st.clone() else {
            return Ok(());
        };
        use std::sync::atomic::Ordering;
        loop {
            let seen = self.drops.load(Ordering::SeqCst);
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
            if let Some(sp) = self.spaces.get() {
                for ev in sp.rows() {
                    fold(&mut fresh, &ev);
                }
            }
            let mut s = self.lock();
            if self.drops.load(Ordering::SeqCst) == seen {
                fresh.drafts = std::mem::take(&mut s.drafts);
                *s = fresh;
                return Ok(());
            }
            // A space was dropped while this fold was read: its caller
            // already hid it from rows(), so fold again without it.
        }
    }

    /// Folds one row; reports whether anything Talk shows changed. Rows of
    /// other kinds, and a row folded before, change nothing.
    pub fn apply(&self, ev: &Event) -> bool {
        if !KINDS.contains(&ev.kind.as_str()) {
            return false;
        }
        fold(&mut self.lock(), ev)
    }

    /// Drops space `space`'s channels, with their members and posts, from
    /// the fold: the space was forgotten or left here (card #461). Reports
    /// whether anything Talk shows changed. The posts' ids leave the
    /// dedupe set, so a re-invite folds them back; read cursors and
    /// talk.card links stay (private log). Personal is never dropped.
    ///
    /// A person DM (card #538) loses that log's posts, and goes when its
    /// last log does.
    pub fn drop_space(&self, space: &str) -> bool {
        if space == PERSONAL_SPACE {
            return false;
        }
        let mut s = self.lock();
        self.drops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let gone: Vec<String> = s
            .channels
            .iter()
            .filter(|(_, c)| c.space_id == space)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &gone {
            s.channels.remove(id);
            s.members.remove(id);
            if let Some(ps) = s.posts.remove(id) {
                for p in ps {
                    s.ids.remove(&p.post_id);
                }
            }
        }
        let dms: Vec<String> = s
            .dm_logs
            .iter()
            .filter(|(_, logs)| logs.contains(space))
            .map(|(id, _)| id.clone())
            .collect();
        for id in &dms {
            let (kept, lost): (Vec<Post>, Vec<Post>) = s
                .posts
                .remove(id)
                .unwrap_or_default()
                .into_iter()
                .partition(|p| p.space != space);
            for p in lost {
                s.ids.remove(&p.post_id);
            }
            if !kept.is_empty() {
                s.posts.insert(id.clone(), kept);
            }
            let logs = s.dm_logs.get_mut(id).expect("just listed");
            logs.remove(space);
            if logs.is_empty() {
                s.dm_logs.remove(id);
                s.members.remove(id);
                s.drafts.remove(id);
            }
        }
        // A huddle's marks and lines go with the log that held them, and
        // all of a gone channel's.
        let State { calls, ids, .. } = &mut *s;
        let mut lines_gone = false;
        calls.retain(|_, c| {
            let n = c.marks.len() + c.lines.len() + c.fallback.len();
            let all = gone.contains(&c.conv);
            c.marks.retain(|p| !all && p.space != space);
            c.lines.retain(|p| !all && p.space != space);
            c.fallback.retain(|p| {
                let keep = !all && p.space != space;
                if !keep {
                    ids.remove(&p.post_id);
                }
                keep
            });
            lines_gone |= c.marks.len() + c.lines.len() + c.fallback.len() != n;
            !c.is_empty()
        });
        !gone.is_empty() || !dms.is_empty() || lines_gone
    }

    /// Appends a row to `space`'s log (empty: the private log) and folds it.
    fn write<P: Serialize>(
        &self,
        space: &str,
        actor_key: &str,
        kind: &str,
        p: &P,
    ) -> Result<Event> {
        if !space.is_empty() {
            let Some(sp) = self.spaces.get() else {
                return Err(Error::raw(
                    ErrorKind::NotAllowed,
                    "talk: this Core keeps no spaces",
                ));
            };
            let v = serde_json::to_value(p)
                .map_err(|e| Error::raw(ErrorKind::Store, format!("talk: {e}")))?;
            let ev = sp
                .append(space, actor_key, kind, v)
                .map_err(|e| Error::raw(ErrorKind::NotAllowed, format!("talk: {e}")))?;
            fold(&mut self.lock(), &ev);
            return Ok(ev);
        }
        let Some(st) = &self.st else {
            return Err(Error::raw(ErrorKind::Store, "talk: no store"));
        };
        let actor = if is_person_key(actor_key) {
            "user".to_string()
        } else {
            format!("agent:{actor_key}")
        };
        let ev = st.append(&actor, kind, p)?;
        fold(&mut self.lock(), &ev);
        Ok(ev)
    }

    // ---- writes ---------------------------------------------------------

    /// Makes a Personal channel named `name` (unique among Personal's open
    /// channels, any case); `by` joins it.
    pub fn create_channel(&self, by: &str, name: &str, topic: &str) -> Result<Channel> {
        self.create_channel_in(by, name, topic, PERSONAL_SPACE)
    }

    /// Makes a channel named `name` in space `space` (empty: Personal);
    /// `by` joins it. A space's channel, and its members and posts, go in
    /// the space's log ([`SpaceLogs`]). The name is unique within the
    /// space only: two spaces may each have a #general (card #461).
    pub fn create_channel_in(
        &self,
        by: &str,
        name: &str,
        topic: &str,
        space: &str,
    ) -> Result<Channel> {
        let by = need_by(by)?;
        let space = space.trim();
        let name = clean_name(name)?;
        let topic = clean_topic(topic)?;
        if self.channel_named_in(&name, space).is_some() {
            return Err(invalid(format!(
                "there's already a channel named {name} {}",
                if space.is_empty() {
                    "in Personal"
                } else {
                    "in this space"
                }
            )));
        }
        let id = format!("{CHANNEL_PREFIX}{}", new_id(&(self.now)()));
        self.write(
            space,
            by,
            kinds::TALK_CHANNEL_CREATED,
            &ChannelCreatedRow {
                channel_id: id.clone(),
                space_id: space.into(),
                name,
                topic: (!topic.is_empty()).then_some(topic),
                by: by.into(),
            },
        )?;
        self.join(&id, &Member::of(by), by)?;
        self.channel(&id)
            .ok_or_else(|| Error::raw(ErrorKind::Store, "talk: the channel didn't fold"))
    }

    /// Makes space `space`'s assistants channel as `irori` unless Talk has
    /// folded it already (archived or not, so an archived one stays so).
    /// True when it wrote the row. Nobody joins it: it isn't
    /// [`Self::create_channel_in`].
    pub fn ensure_assistants_channel(&self, space: &str) -> Result<bool> {
        let space = space.trim();
        if space.is_empty() {
            return Ok(false);
        }
        let id = assistants_channel_id(space);
        if self.channel(&id).is_some() {
            return Ok(false);
        }
        self.write(
            space,
            "irori",
            kinds::TALK_CHANNEL_CREATED,
            &ChannelCreatedRow {
                channel_id: id,
                space_id: space.into(),
                name: ASSISTANTS_NAME.into(),
                topic: Some(ASSISTANTS_TOPIC.into()),
                by: "irori".into(),
            },
        )?;
        Ok(true)
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
        let mut space = String::new();
        let pair = self.pair_other(conv);
        if pair.is_some() && op == LEAVE {
            return Err(not_allowed(
                "a DM with a person is left by leaving it, not by a member row",
            ));
        }
        if is_channel(conv) {
            match self.channel(conv) {
                None => return Err(not_found(format!("no channel {conv}"))),
                Some(c) => space = c.space_id,
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
        if let Some(other) = &pair {
            space = self.dm_space(other)?;
        }
        self.write(
            &space,
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

    /// Links post `post_id` to the card made from it (card #390, "Make a
    /// card"): the post shows the card's chip. Linking it again to the same
    /// card is a no-op (false). The link stays on this person's devices.
    pub fn link_card(&self, post_id: &str, task_id: &str, by: &str) -> Result<bool> {
        self.link_card_in(post_id, task_id, by, PERSONAL_SPACE)
    }

    /// [`Self::link_card`] for a card in `log` (empty: the private log). A
    /// post in that space's channel links in the space, so its members see
    /// the chip (card #558); a card they can't open stays this person's
    /// link, as does one the space won't take yet.
    pub fn link_card_in(&self, post_id: &str, task_id: &str, by: &str, log: &str) -> Result<bool> {
        let by = need_by(by)?;
        let (post_id, task_id) = (post_id.trim(), task_id.trim());
        if task_id.is_empty() {
            return Err(invalid("link_card needs the card's id"));
        }
        if !self.has_post(post_id) {
            return Err(not_found(format!("no post {post_id}")));
        }
        if self.card_of(post_id).as_deref() == Some(task_id) {
            return Ok(false);
        }
        let row = CardRow {
            post_id: post_id.into(),
            task_id: task_id.into(),
        };
        let in_space = !log.is_empty() && self.post_space(post_id).as_deref() == Some(log);
        if !in_space || self.write(log, by, kinds::TALK_CARD, &row).is_err() {
            self.write(PERSONAL_SPACE, by, kinds::TALK_CARD, &row)?;
        }
        Ok(true)
    }

    /// This person's `person:<id>`, where person DMs exist.
    pub fn person_key(&self) -> Option<String> {
        let id = self.spaces.get()?.person_id()?;
        Some(format!("{PERSON_ACTOR_PREFIX}{id}"))
    }

    /// Whether `key` is another person, as a person DM names them: a plain
    /// `person:<id>`, not the board form a shared board's notice carries.
    fn other_person(&self, key: &str) -> bool {
        key.starts_with(PERSON_ACTOR_PREFIX) && self.person_key().is_some_and(|me| me != key)
    }

    /// The DM between keys `a` and `b`: [`pair_dm_id`] for this person and
    /// another, else [`dm_id`].
    pub fn dm_of(&self, a: &str, b: &str) -> String {
        match self.person_key() {
            Some(me) if is_person_key(a) && self.other_person(b) => pair_dm_id(&me, b),
            Some(me) if is_person_key(b) && self.other_person(a) => pair_dm_id(&me, a),
            _ => dm_id(a, b),
        }
    }

    /// The other person of person DM `conv` (`person:<id>`), from its
    /// members or a draft, else from the hidden spaces this device holds.
    /// `None`: `conv` isn't a person DM.
    pub fn pair_other(&self, conv: &str) -> Option<String> {
        let me = self.person_key()?;
        if !is_dm(conv) {
            return None;
        }
        let known = {
            let s = self.lock();
            let drafted = s.drafts.get(conv).into_iter().flatten().cloned();
            let joined = s
                .members
                .get(conv)
                .into_iter()
                .flatten()
                .map(|m| m.member.clone());
            joined
                .chain(drafted)
                .map(|m| m.key)
                .find(|k| k.starts_with(PERSON_ACTOR_PREFIX) && pair_dm_id(&me, k) == conv)
        };
        known.or_else(|| self.spaces.get()?.dm_route(conv))
    }

    /// Takes back a DM opened and never written in (card #538): leaving a
    /// DM takes its logs, and this is what's left of it.
    pub fn drop_draft(&self, conv: &str) -> bool {
        self.lock().drafts.remove(conv).is_some()
    }

    /// The space a person DM with `other` writes to now.
    fn dm_space(&self, other: &str) -> Result<String> {
        let Some(sp) = self.spaces.get() else {
            return Err(not_allowed("this Core keeps no spaces"));
        };
        sp.dm_for(other).map_err(not_allowed)
    }

    /// The DM between `a` and `b`, with both joined (written the first
    /// time). `by` is whoever opens it. A DM with another person writes
    /// nothing here: it is listed as a draft until the first post (card
    /// #538), and an agent has no such DM.
    pub fn open_dm(&self, a: &Member, b: &Member, by: &str) -> Result<String> {
        let (a, b) = (clean_member(a)?, clean_member(b)?);
        if a.same(&b) {
            return Err(invalid("a DM needs two different parties"));
        }
        if self.other_person(&a.key) || self.other_person(&b.key) {
            if !is_person_key(&a.key) && !is_person_key(&b.key) {
                return Err(not_allowed("agents don't DM other people"));
            }
            let id = self.dm_of(&a.key, &b.key);
            let mut s = self.lock();
            if !s.members.contains_key(&id) {
                s.drafts.insert(id.clone(), [a, b]);
            }
            return Ok(id);
        }
        let id = dm_id(&a.key, &b.key);
        self.join(&id, &a, by)?;
        self.join(&id, &b, by)?;
        Ok(id)
    }

    /// Posts to a channel or a DM. The text is trimmed and must not be
    /// empty: at most [`MAX_COMMENT`] characters from an agent, and
    /// [`MAX_PERSON_TEXT`] bytes from the person, as comments. A post in a
    /// channel joins its writer, unless it stays out (`stay_out`).
    pub fn post(&self, p: NewPost) -> Result<Post> {
        let by = need_by(&p.by)?.to_string();
        let conv = p.conv_id.trim().to_string();
        let files = check_files(&p.files)?;
        let text = check_post(&by, &p.text, !files.is_empty())?;
        let mut space = String::new();
        let pair = self.pair_other(&conv);
        if is_channel(&conv) {
            match self.channel(&conv) {
                None => return Err(not_found(format!("no channel {conv}"))),
                Some(c) if c.archived => {
                    return Err(invalid(format!("#{} is archived", c.name)));
                }
                Some(c) => space = c.space_id,
            }
        } else if !is_dm(&conv) {
            return Err(invalid(format!(
                "{conv} isn't a channel or a DM (a thread's posts are its comments)"
            )));
        } else if pair.is_some() {
            if !is_person_key(&by) {
                return Err(not_allowed("agents don't DM other people"));
            }
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
        if pair.is_some() {
            // Nobody is woken through a DM with a person (card #538).
            mentions.clear();
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
        let call = p.call.map(check_call).transpose()?;
        let transcript = p.transcript.map(check_transcript).transpose()?;
        if is_channel(&conv) && !p.stay_out {
            self.join(&conv, &Member::of(&by), &by)?;
        }
        if let Some(other) = &pair {
            space = self.dm_space(other)?;
            self.join_log(&conv, &space, &by, other)?;
        }
        let hidden = transcript.as_ref().map(|t| t.call_id.clone());
        self.write(
            &space,
            &by,
            kinds::TALK_POST,
            &PostRow {
                post_id: post_id.clone(),
                conv_id: conv.clone(),
                text,
                mentions,
                agents_now: p.agents_now,
                thread_from,
                attach: files,
                call,
                transcript,
                by: by.clone(),
                at: (self.now)(),
            },
        )?;
        let s = self.lock();
        match hidden {
            // A fallback post is its call's, not the conversation's.
            Some(call) => s
                .calls
                .get(&call)
                .and_then(|c| c.fallback.iter().find(|x| x.post_id == post_id).cloned()),
            None => s
                .posts
                .get(&conv)
                .and_then(|ps| ps.iter().find(|x| x.post_id == post_id).cloned()),
        }
        .ok_or_else(|| Error::raw(ErrorKind::Store, "talk: the post didn't fold"))
    }

    /// Writes one finished phrase of huddle `call_id` in `conv` as a
    /// `call.segment` row (card #656), by its speaker `by`, into the log
    /// `conv`'s posts go to: a channel's (Personal: the private log) or a
    /// person DM's. `start_ms`/`end_ms` are Unix ms; `seg` counts from 0
    /// per device per call. Returns the line as [`Talk::call_posts`] has
    /// it.
    #[allow(clippy::too_many_arguments)]
    pub fn write_segment(
        &self,
        conv: &str,
        call_id: &str,
        seg: i64,
        start_ms: i64,
        end_ms: i64,
        text: &str,
        by: &str,
    ) -> Result<Post> {
        self.write_segment_marked(conv, call_id, seg, start_ms, end_ms, text, by, false)
    }

    /// [`Talk::write_segment`] with the `agents` mark (card #1047): `true`
    /// when the speaker's notice said agents hear the huddle, so a person
    /// DM's agents may read the line (call-agents.md section 10).
    #[allow(clippy::too_many_arguments)]
    pub fn write_segment_marked(
        &self,
        conv: &str,
        call_id: &str,
        seg: i64,
        start_ms: i64,
        end_ms: i64,
        text: &str,
        by: &str,
        agents: bool,
    ) -> Result<Post> {
        let by = need_by(by)?.to_string();
        let (conv, call_id) = (conv.trim().to_string(), call_id.trim().to_string());
        if call_id.is_empty() {
            return Err(invalid("a transcript line needs its call's id"));
        }
        if seg < 0 {
            return Err(invalid("a transcript line's seg counts from 0"));
        }
        let text = check_text(&by, text)?;
        // A line is the person's own voice: never an agent's.
        if !is_person_key(&by) {
            return Err(not_allowed("only a person speaks in a huddle"));
        }
        let space = self.call_log(&conv, &by)?;
        let ev = self.write(
            &space,
            &by,
            kinds::CALL_SEGMENT,
            &SegmentRow {
                call_id: call_id.clone(),
                conv_id: conv,
                seg,
                start_ms,
                end_ms: end_ms.max(start_ms),
                text,
                agents,
                by: by.clone(),
                at: (self.now)(),
            },
        )?;
        let id = segment_id(&call_id, &ev.device_id, seg);
        self.lock()
            .calls
            .get(&call_id)
            .and_then(|c| c.lines.iter().find(|x| x.post_id == id).cloned())
            .ok_or_else(|| Error::raw(ErrorKind::Store, "talk: the line didn't fold"))
    }

    /// The log a huddle's rows in `conv` go to, as `by` (card #656, #1043):
    /// a channel's space (Personal: the private log, ""), or a person DM's,
    /// whose log is joined first.
    fn call_log(&self, conv: &str, by: &str) -> Result<String> {
        if is_channel(conv) {
            match self.channel(conv) {
                None => Err(not_found(format!("no channel {conv}"))),
                Some(c) if c.archived => Err(invalid(format!("#{} is archived", c.name))),
                Some(c) => Ok(c.space_id),
            }
        } else if let Some(other) = self.pair_other(conv) {
            let space = self.dm_space(&other)?;
            self.join_log(conv, &space, by, &other)?;
            Ok(space)
        } else {
            Err(invalid(format!(
                "{conv} isn't a channel or a DM with a person: no huddle is there"
            )))
        }
    }

    /// Writes huddle `call_id`'s title (card #1043) as a `call.title` row,
    /// by the person `by`, into the log `conv`'s posts go to. Anyone in the
    /// conversation may. `assistant` marks a title an assistant proposed:
    /// it shows only while no person has set one. Returns the title the
    /// huddle now shows.
    pub fn write_title(
        &self,
        conv: &str,
        call_id: &str,
        title: &str,
        assistant: bool,
        by: &str,
    ) -> Result<String> {
        let by = need_by(by)?.to_string();
        let (conv, call_id) = (conv.trim().to_string(), call_id.trim().to_string());
        if call_id.is_empty() {
            return Err(invalid("a huddle title needs its call's id"));
        }
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(invalid("a huddle's title can't be empty"));
        }
        if title.len() > MAX_TITLE {
            return Err(invalid(format!(
                "a huddle's title is at most {MAX_TITLE} bytes"
            )));
        }
        if !is_person_key(&by) {
            return Err(not_allowed("only a person titles a huddle"));
        }
        // The huddle must be this conversation's, or the row would fold to
        // nothing.
        if let Some(c) = self.call_conv(&call_id)
            && c != conv
        {
            return Err(invalid(format!("huddle {call_id} isn't in {conv}")));
        }
        let space = self.call_log(&conv, &by)?;
        self.write(
            &space,
            &by,
            kinds::CALL_TITLE,
            &TitleRow {
                call_id: call_id.clone(),
                conv_id: conv.clone(),
                title,
                assistant,
                by: by.clone(),
                at: (self.now)(),
            },
        )?;
        Ok(self.call_title(&call_id).unwrap_or_default())
    }

    /// Huddle `call_id`'s title, when it has one (card #1043).
    pub fn call_title(&self, call_id: &str) -> Option<String> {
        let s = self.lock();
        let id = call_id.trim();
        let conv = s.calls.get(id).map(|c| c.conv.as_str())?;
        let (t, _) = title_of(&s, id, conv);
        (!t.is_empty()).then_some(t)
    }

    /// Writes huddle `call_id`'s `call.assist` row (card #1049) as the
    /// person `by`: the assistant is hosted on `device` (`on`) or stopped
    /// (`off`). Anyone in the conversation may write their own; the fold
    /// keeps the newest per device.
    pub fn write_assist(
        &self,
        conv: &str,
        call_id: &str,
        device: &str,
        on: bool,
        by: &str,
    ) -> Result<()> {
        let by = need_by(by)?.to_string();
        let (conv, call_id) = (conv.trim().to_string(), call_id.trim().to_string());
        if call_id.is_empty() || device.trim().is_empty() {
            return Err(invalid("an assistant row needs its call and its device"));
        }
        if !is_person_key(&by) {
            return Err(not_allowed("only a person's device hosts an assistant"));
        }
        if let Some(c) = self.call_conv(&call_id)
            && c != conv
        {
            return Err(invalid(format!("huddle {call_id} isn't in {conv}")));
        }
        let space = self.call_log(&conv, &by)?;
        self.write(
            &space,
            &by,
            kinds::CALL_ASSIST,
            &AssistRow {
                call_id,
                conv_id: conv,
                device: device.trim().to_string(),
                state: if on { ASSIST_ON } else { ASSIST_OFF }.into(),
                chair: false,
                by: by.clone(),
                at: (self.now)(),
            },
        )?;
        Ok(())
    }

    /// Huddle `call_id`'s hosting rows (card #1049): each device's newest
    /// per person, on or off, in no particular order. Rows naming another
    /// conversation than the huddle's don't count; a huddle this device has
    /// no mark or line of yet has none.
    pub fn call_assists(&self, call_id: &str) -> Vec<AssistInfo> {
        let s = self.lock();
        let id = call_id.trim();
        let Some(conv) = s.calls.get(id).map(|c| c.conv.as_str()) else {
            return Vec::new();
        };
        s.assists
            .get(id)
            .into_iter()
            .flatten()
            .filter(|a| a.conv == conv)
            .map(|a| AssistInfo {
                device: a.device.clone(),
                by: a.by.clone(),
                on: a.on,
                chair: a.chair,
            })
            .collect()
    }

    /// Writes both people's joins into `space`, the first time this person
    /// DM uses that log: a log carries its own members, so the DM outlives
    /// the other log it may share the conversation with.
    fn join_log(&self, conv: &str, space: &str, by: &str, other: &str) -> Result<()> {
        if self
            .lock()
            .dm_logs
            .get(conv)
            .is_some_and(|l| l.contains(space))
        {
            return Ok(());
        }
        for key in [by, other] {
            self.write(
                space,
                by,
                kinds::TALK_MEMBER,
                &MemberRow {
                    conv_id: conv.into(),
                    member: Member::of(key),
                    op: JOIN.into(),
                    by: by.into(),
                },
            )?;
        }
        Ok(())
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
            PERSONAL_SPACE,
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

    /// The open Personal channel named `name` (any case, a leading `#`
    /// dropped). A space's channel is found with [`Talk::channel_named_in`].
    pub fn channel_named(&self, name: &str) -> Option<Channel> {
        self.channel_named_in(name, PERSONAL_SPACE)
    }

    /// The open channel named `name` in space `space` (empty: Personal).
    /// Names are unique per space, so a lookup never crosses into another
    /// space (card #461).
    pub fn channel_named_in(&self, name: &str, space: &str) -> Option<Channel> {
        let (want, space) = (fold_name(name), space.trim());
        self.lock()
            .channels
            .values()
            .find(|c| !c.archived && c.space_id == space && c.name.to_lowercase() == want)
            .cloned()
    }

    /// Every open channel named `name`, in any space, sorted by space: for
    /// telling a caller where a name it looked up in one space lives.
    pub fn channels_named(&self, name: &str) -> Vec<Channel> {
        let want = fold_name(name);
        let mut out: Vec<Channel> = self
            .lock()
            .channels
            .values()
            .filter(|c| !c.archived && c.name.to_lowercase() == want)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.space_id.cmp(&b.space_id).then_with(|| a.id.cmp(&b.id)));
        out
    }

    /// Who is in `conv` now, in the order they joined. A draft's two sides
    /// count until its first row folds.
    pub fn members(&self, conv: &str) -> Vec<Member> {
        let s = self.lock();
        let mut ms: Vec<&MemberState> = s
            .members
            .get(conv)
            .map(|v| v.iter().filter(|m| m.joined).collect())
            .unwrap_or_default();
        if ms.is_empty()
            && let Some(sides) = s.drafts.get(conv)
        {
            return sides.to_vec();
        }
        ms.sort_by(|a, b| a.at.cmp(&b.at));
        ms.into_iter().map(|m| m.member.clone()).collect()
    }

    /// Whether `key` (any of the person's keys counts as the person) is in
    /// `conv`.
    pub fn is_member(&self, conv: &str, key: &str) -> bool {
        let k = canon(key);
        let s = self.lock();
        match s.members.get(conv) {
            Some(v) => v.iter().any(|m| m.joined && canon(&m.member.key) == k),
            None => s
                .drafts
                .get(conv)
                .is_some_and(|d| d.iter().any(|m| canon(&m.key) == k)),
        }
    }

    /// Every DM with a member row or a draft, and who it's between.
    pub fn dms(&self) -> Vec<(String, Vec<Member>)> {
        let s = self.lock();
        let ids: BTreeSet<String> = s
            .members
            .keys()
            .chain(s.drafts.keys())
            .filter(|k| is_dm(k))
            .cloned()
            .collect();
        drop(s);
        ids.into_iter()
            .map(|id| {
                let ms = self.members(&id);
                (id, ms)
            })
            .collect()
    }

    /// The posts with the file whose hash is `sha` (card #643), the
    /// earliest first, each with the conversation it is in, the log and
    /// device that wrote it (card #659) and the file as that post names
    /// it. Empty when no post has it. A caller that checks a reader takes
    /// the first conversation the reader may read.
    pub fn attachments(&self, sha: &str) -> Vec<PostFile> {
        let s = self.lock();
        let mut found: Vec<(&Ord3, &String, &Post, &blob::FileRef)> = Vec::new();
        for (conv, ps) in &s.posts {
            for p in ps {
                if let Some(f) = p.files.iter().find(|f| f.sha256 == sha) {
                    found.push((&p.ord, conv, p, f));
                }
            }
        }
        found.sort_by(|a, b| a.0.cmp(b.0));
        found
            .into_iter()
            .map(|(_, conv, p, f)| PostFile {
                conv: conv.clone(),
                space: p.space.clone(),
                device: p.device.clone(),
                file: f.clone(),
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

    /// The space of the channel post `id` is in ("": Personal); None when
    /// it is in no channel here.
    fn post_space(&self, id: &str) -> Option<String> {
        let s = self.lock();
        let (conv, _) = s
            .posts
            .iter()
            .find(|(_, ps)| ps.iter().any(|p| p.post_id == id))?;
        s.channels.get(conv).map(|c| c.space_id.clone())
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

    /// Post `post_id` in `conv`, when it should notify `reader` (card
    /// #395, design §8): someone else's, not yet read, and either in a DM
    /// `reader` is in or @-mentioning them. A plain channel post only
    /// bumps the unread count.
    pub fn notifies(&self, conv: &str, post_id: &str, reader: &str) -> Option<Post> {
        let posts = self.posts(conv);
        let cursor = self.cursor(reader, conv);
        // A fallback transcript (card #656) is in no list, so it is unread
        // when it came after the reader's cursor.
        let read_to = cursor
            .as_deref()
            .and_then(|c| posts.iter().find(|p| p.post_id == c))
            .map(|p| p.ord.clone());
        let hidden = self
            .fallback_post(conv, post_id)
            .filter(|h| read_to.as_ref().is_none_or(|o| h.ord > *o));
        let p = since(&posts, cursor.as_deref())
            .iter()
            .find(|p| p.post_id == post_id)
            .or(hidden.as_ref())?;
        let me = canon(reader);
        if canon(&p.by) == me || p.thread_from.is_some() {
            return None;
        }
        let dm = is_dm(conv) && self.is_member(conv, reader);
        let mentioned = p.mentions.iter().any(|m| canon(m) == me);
        (dm || mentioned).then(|| p.clone())
    }

    /// The card made from post `id` (`talk.card`).
    pub fn card_of(&self, id: &str) -> Option<String> {
        self.lock().cards.get(id).cloned()
    }

    /// The channel or DM huddle `call_id` was in (card #656), once a mark
    /// or a line of it is here.
    pub fn call_conv(&self, call_id: &str) -> Option<String> {
        let s = self.lock();
        s.calls.get(call_id.trim()).map(|c| c.conv.clone())
    }

    /// Huddle `call_id`'s transcript (card #656): one post per line, in
    /// conversation `call:<id>`, by its speaker at the time they said it,
    /// oldest first (by speaker at the same time).
    pub fn call_posts(&self, call_id: &str) -> Vec<Post> {
        let s = self.lock();
        let mut out = s
            .calls
            .get(call_id.trim())
            .map(|c| c.lines.clone())
            .unwrap_or_default();
        out.sort_by(|a, b| {
            crate::gotime::cmp(&a.at, &b.at)
                .then_with(|| a.by.cmp(&b.by))
                .then_with(|| a.ord.cmp(&b.ord))
        });
        out
    }

    /// Every huddle with a mark or a line here (card #656), the latest
    /// first.
    pub fn calls(&self) -> Vec<CallInfo> {
        let s = self.lock();
        let mut out: Vec<CallInfo> = s
            .calls
            .iter()
            .map(|(id, c)| {
                let end = c
                    .marks
                    .iter()
                    .filter_map(|p| p.call.as_ref())
                    .rfind(|m| m.op == CALL_END)
                    .cloned();
                let start_at = c
                    .marks
                    .iter()
                    .find(|p| p.call.as_ref().is_some_and(|m| m.op == CALL_START))
                    .map(|p| p.at);
                let last = c
                    .marks
                    .iter()
                    .chain(&c.lines)
                    .chain(&c.fallback)
                    .map(|p| p.at)
                    .max_by(crate::gotime::cmp)
                    .unwrap_or_default();
                let (title, title_by_assistant) = title_of(&s, id, &c.conv);
                let people = match &end {
                    Some(e) => e.people.clone(),
                    None => {
                        let mut out: Vec<String> = Vec::new();
                        let starter = c
                            .marks
                            .iter()
                            .find(|p| p.call.as_ref().is_some_and(|m| m.op == CALL_START))
                            .map(|p| p.by.as_str());
                        let speakers = c.lines.iter().map(|p| p.by.as_str());
                        for k in starter.into_iter().chain(speakers) {
                            if !out.iter().any(|x| canon(x) == canon(k)) {
                                out.push(k.to_string());
                            }
                        }
                        out
                    }
                };
                CallInfo {
                    call_id: id.clone(),
                    conv_id: c.conv.clone(),
                    start_at,
                    end,
                    last,
                    lines: c.lines.len(),
                    title,
                    title_by_assistant,
                    people,
                    last_line: c.lines.iter().map(|p| p.at).max_by(crate::gotime::cmp),
                }
            })
            .collect();
        out.sort_by(|a, b| {
            crate::gotime::cmp(&b.last, &a.last).then_with(|| a.call_id.cmp(&b.call_id))
        });
        out
    }

    /// Fallback post `post_id` in `conv` (card #656), kept out of `conv`'s
    /// posts.
    fn fallback_post(&self, conv: &str, post_id: &str) -> Option<Post> {
        let s = self.lock();
        s.calls
            .values()
            .filter(|c| c.conv == conv)
            .find_map(|c| c.fallback.iter().find(|p| p.post_id == post_id))
            .cloned()
    }
}

/// Huddle `call_id`'s title in conversation `conv` (card #1043): a person's
/// newest, else an assistant's newest. Rows naming another conversation
/// than the huddle's don't count. The flag says an assistant proposed it.
fn title_of(s: &State, call_id: &str, conv: &str) -> (String, bool) {
    let newest = |assistant: bool| {
        s.titles
            .get(call_id)?
            .iter()
            .filter(|t| t.conv == conv && t.assistant == assistant)
            .max_by(|a, b| a.at.cmp(&b.at))
    };
    match (newest(false), newest(true)) {
        (Some(t), _) => (t.title.clone(), false),
        (None, Some(t)) => (t.title.clone(), true),
        (None, None) => (String::new(), false),
    }
}

/// A `call.segment` line's id: one per call, writing device and seg.
fn segment_id(call_id: &str, device: &str, seg: i64) -> String {
    format!("seg_{call_id}_{device}_{seg}")
}

/// Huddle `call_id`'s entry, for a row in `conv`; None when the huddle is
/// already in another conversation.
fn call_idx<'a>(s: &'a mut State, call_id: &str, conv: &str) -> Option<&'a mut CallIdx> {
    let c = s.calls.entry(call_id.to_string()).or_default();
    if c.conv.is_empty() {
        c.conv = conv.to_string();
    }
    (c.conv == conv).then_some(c)
}

/// Adds `p` to `v`, which stays sorted by `ord`.
fn insert_sorted(v: &mut Vec<Post>, p: Post) {
    let i = v.partition_point(|x| x.ord <= p.ord);
    v.insert(i, p);
}

/// Unix ms as a time.
fn at_ms(ms: i64) -> GoTime {
    crate::gotime::from_ns(i128::from(ms) * 1_000_000)
}

/// Folds one row into `s`; whether it changed anything.
/// The space a row came in from, when a space's log carried it: the Core
/// tags each such row with `spaceId` (card #461). None for this person's
/// own log.
fn row_space(ev: &Event) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(&ev.payload).ok()?;
    v.get("spaceId")?.as_str().map(str::to_string)
}

/// Whether a row from space `space`'s log may touch conversation `conv`:
/// only that space's channels, never another space's (or Personal's)
/// channel, so one space's members can't write into the rest of this
/// person's Talk (card #461). A channel not folded yet may be.
///
/// A DM has one door (card #538): `dm_conv`, the `dmConv` the Core stamps
/// on a row of a hidden space once every check of talk-dms.md §4 holds.
/// It strips one a row arrived with, so no row of any log can say it
/// itself.
fn space_may_touch(s: &State, space: &str, conv: &str, dm_conv: Option<&str>) -> bool {
    if is_dm(conv) {
        return dm_conv == Some(conv);
    }
    is_channel(conv)
        && s.channels
            .get(conv)
            .is_none_or(|c| c.id.is_empty() || c.space_id == space)
}

fn fold(s: &mut State, ev: &Event) -> bool {
    let at = ord_of(ev);
    if ev.kind != kinds::TALK_CHANNEL_CREATED
        && let Some(space) = row_space(ev)
    {
        let v = serde_json::from_slice::<serde_json::Value>(&ev.payload).ok();
        let field = |k: &str| v.as_ref()?.get(k)?.as_str();
        let conv = field("convId").or_else(|| field("channelId"));
        let dm_conv = field("dmConv");
        if let Some(conv) = conv {
            if !space_may_touch(s, &space, conv, dm_conv) {
                return false;
            }
            if dm_conv.is_some() {
                s.dm_logs
                    .entry(conv.to_string())
                    .or_default()
                    .insert(space.clone());
            }
        }
    }
    match ev.kind.as_str() {
        kinds::TALK_CHANNEL_CREATED => {
            let Ok(r) = serde_json::from_slice::<ChannelCreatedRow>(&ev.payload) else {
                return false;
            };
            if r.channel_id.is_empty() {
                return false;
            }
            // The fold keeps the first copy, so a member of two spaces
            // could make S1's assistants channel inside S2. `space_id` is
            // the log that carried the row, not the row's say-so.
            if in_assistants_range(&r.channel_id)
                && !is_assistants_channel(&r.channel_id, &r.space_id)
            {
                return false;
            }
            let c = s.channels.entry(r.channel_id.clone()).or_default();
            if !c.id.is_empty() && (c.created <= at || c.space_id != r.space_id) {
                // Folded before, or an older copy wins; and a channel never
                // moves to another space (card #461).
                return false;
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
            if r.post_id.is_empty() || r.conv_id.is_empty() || s.ids.contains(&r.post_id) {
                return false;
            }
            // A mark with no usable lines leaves a plain post.
            let transcript = r.transcript.and_then(|t| check_transcript(t).ok());
            // A huddle's posts go in its index too (card #656). One naming a
            // huddle of another conversation is shown as a plain post.
            let call_id = transcript
                .as_ref()
                .map(|t| t.call_id.clone())
                .or_else(|| r.call.as_ref().map(|c| c.call_id.clone()))
                .filter(|id| !id.is_empty() && s.calls.get(id).is_none_or(|c| c.conv == r.conv_id));
            s.ids.insert(r.post_id.clone());
            let p = Post {
                card: s.cards.get(&r.post_id).cloned(),
                post_id: r.post_id,
                conv_id: r.conv_id.clone(),
                by: r.by,
                text: r.text,
                mentions: r.mentions,
                agents_now: r.agents_now,
                agents: false,
                thread_from: r.thread_from,
                files: r
                    .attach
                    .into_iter()
                    .filter(|f| blob::valid_id(&f.sha256))
                    .collect(),
                call: r.call,
                transcript,
                at: r.at,
                ord: at,
                space: row_space(ev).unwrap_or_default(),
                device: ev.device_id.clone(),
            };
            let Some(id) = call_id else {
                insert_sorted(s.posts.entry(r.conv_id).or_default(), p);
                return true;
            };
            let c = call_idx(s, &id, &r.conv_id).expect("checked above");
            let Some(t) = &p.transcript else {
                // A start or end: shown in its conversation as ever.
                insert_sorted(&mut c.marks, p.clone());
                insert_sorted(s.posts.entry(r.conv_id).or_default(), p);
                return true;
            };
            // A fallback post: its lines are the call's, and the post is in
            // no conversation's list.
            for (i, l) in t.lines.iter().enumerate() {
                insert_sorted(
                    &mut c.lines,
                    Post {
                        post_id: format!("{}#{i}", p.post_id),
                        conv_id: format!("{CALL_PREFIX}{id}"),
                        by: p.by.clone(),
                        text: l.text.clone(),
                        at: at_ms(l.start_ms),
                        ord: (l.start_ms.max(0) as u64, p.device.clone(), i as i64),
                        space: p.space.clone(),
                        device: p.device.clone(),
                        ..Post::default()
                    },
                );
            }
            insert_sorted(&mut c.fallback, p);
            true
        }
        kinds::CALL_SEGMENT => {
            let Ok(r) = serde_json::from_slice::<SegmentRow>(&ev.payload) else {
                return false;
            };
            let conv_ok = is_channel(&r.conv_id) || is_dm(&r.conv_id);
            let text = r.text.trim();
            if r.call_id.is_empty()
                || !conv_ok
                || text.is_empty()
                || text.len() > MAX_PERSON_TEXT
                || r.seg < 0
            {
                return false;
            }
            let id = segment_id(&r.call_id, &ev.device_id, r.seg);
            if s.calls
                .get(&r.call_id)
                .is_some_and(|c| c.conv != r.conv_id || c.lines.iter().any(|p| p.post_id == id))
            {
                return false;
            }
            let c = call_idx(s, &r.call_id, &r.conv_id).expect("checked above");
            insert_sorted(
                &mut c.lines,
                Post {
                    post_id: id,
                    conv_id: format!("{CALL_PREFIX}{}", r.call_id),
                    by: r.by,
                    text: text.to_string(),
                    agents: r.agents,
                    at: at_ms(r.start_ms),
                    ord: (r.start_ms.max(0) as u64, ev.device_id.clone(), r.seg),
                    space: row_space(ev).unwrap_or_default(),
                    device: ev.device_id.clone(),
                    ..Post::default()
                },
            );
            true
        }
        kinds::CALL_TITLE => {
            let Ok(r) = serde_json::from_slice::<TitleRow>(&ev.payload) else {
                return false;
            };
            let title = r.title.trim();
            // Only a person sets or proposes a title (an assistant's is the
            // person's row, flagged); never an agent.
            if r.call_id.is_empty()
                || !(is_channel(&r.conv_id) || is_dm(&r.conv_id))
                || title.is_empty()
                || title.len() > MAX_TITLE
                || !(is_person_key(&r.by) || is_other_person_key(&r.by))
            {
                return false;
            }
            let recs = s.titles.entry(r.call_id).or_default();
            let rec = TitleRec {
                conv: r.conv_id,
                title: title.to_string(),
                assistant: r.assistant,
                at,
            };
            match recs
                .iter_mut()
                .find(|x| x.conv == rec.conv && x.assistant == rec.assistant)
            {
                Some(x) => {
                    if x.at >= rec.at {
                        return false;
                    }
                    *x = rec;
                }
                None => recs.push(rec),
            }
            true
        }
        kinds::CALL_ASSIST => {
            let Ok(r) = serde_json::from_slice::<AssistRow>(&ev.payload) else {
                return false;
            };
            let device = r.device.trim();
            let on = r.state == ASSIST_ON;
            // A person's device only; never an agent. The device is the
            // row's own writer: a device speaks for no other.
            if r.call_id.is_empty()
                || !(is_channel(&r.conv_id) || is_dm(&r.conv_id))
                || device.is_empty()
                || !(on || r.state == ASSIST_OFF)
                || !(is_person_key(&r.by) || is_other_person_key(&r.by))
                || ev.device_id != device
            {
                return false;
            }
            let recs = s.assists.entry(r.call_id).or_default();
            let rec = AssistRec {
                conv: r.conv_id,
                device: device.to_string(),
                by: r.by,
                on,
                chair: r.chair,
                at,
            };
            match recs
                .iter_mut()
                .find(|x| x.conv == rec.conv && x.device == rec.device)
            {
                Some(x) => {
                    if x.at >= rec.at {
                        return false;
                    }
                    *x = rec;
                }
                None => recs.push(rec),
            }
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
            // From a space's log: only onto a post in that space's channels.
            if let Some(space) = row_space(ev)
                && let Some(conv) = s
                    .posts
                    .iter()
                    .find(|(_, ps)| ps.iter().any(|p| p.post_id == r.post_id))
                    .map(|(c, _)| c.clone())
                && !space_may_touch(s, &space, &conv, None)
            {
                return false;
            }
            // Two people can make a card from one post while apart, and the
            // link is in a shared log (card #558): the later row wins, so
            // every member shows one card, whichever row reached them first.
            let at = ord_of(ev);
            if s.card_at.get(&r.post_id).is_some_and(|o| *o >= at) {
                return false;
            }
            s.card_at.insert(r.post_id.clone(), at);
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

fn not_allowed(detail: impl std::fmt::Display) -> Error {
    Error::raw(ErrorKind::NotAllowed, format!("talk: {detail}"))
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

/// `name` as channel names compare: trimmed, a leading `#` dropped, any
/// case.
fn fold_name(name: &str) -> String {
    name.trim().trim_start_matches('#').to_lowercase()
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
    check_post(by, text, false)
}

/// [`check_text`] for a post that may carry files: with files it may have
/// no text, a pasted screenshot on its own (card #643).
pub fn check_post(by: &str, text: &str, files: bool) -> Result<String> {
    let t = text.trim();
    if t.is_empty() && !files {
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

/// A post's files, checked: at most [`crate::board::MAX_ATTACHMENTS`], each
/// a stored file's hash; the same file twice goes once.
pub fn check_files(files: &[blob::FileRef]) -> Result<Vec<blob::FileRef>> {
    if files.len() > crate::board::MAX_ATTACHMENTS {
        return Err(invalid(format!(
            "at most {} files on a post",
            crate::board::MAX_ATTACHMENTS
        )));
    }
    let mut out: Vec<blob::FileRef> = Vec::with_capacity(files.len());
    for f in files {
        if !blob::valid_id(&f.sha256) {
            return Err(invalid(format!("{} isn't a stored file", f.name)));
        }
        if !out.iter().any(|x| x.sha256 == f.sha256) {
            out.push(f.clone());
        }
    }
    Ok(out)
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

    /// Card #538: both people's Cores reach one id, it isn't [`dm_id`]'s,
    /// and it isn't canon'd: each side's own key is a `person:` key here.
    #[test]
    fn pair_dm_id_is_one_per_pair_and_never_canons() {
        let (a, b, c) = ("person:a", "person:b", "person:c");
        assert_eq!(pair_dm_id(a, b), pair_dm_id(b, a));
        assert_ne!(pair_dm_id(a, b), pair_dm_id(a, c));
        assert_ne!(pair_dm_id(a, b), dm_id(a, b));
        assert!(is_dm(&pair_dm_id(a, b)));
        assert_eq!(pair_dm_id(a, b).len(), DM_PREFIX.len() + 16);
    }
}
