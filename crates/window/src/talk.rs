//! Talk (card #389; docs/design/talk-channels.md): the person's channels,
//! DMs and threads, read and written through the Core's Talk calls. A
//! thread's posts are its comments, so posting to one is the comment box's
//! call ([`Window::comment`]).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Window;
use crate::board::UploadFile;
use crate::threads::ThreadFrom;
use crate::ts;

/// One side of a conversation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkMember {
    /// `person` or `agent`.
    pub kind: String,
    pub key: String,
    /// What it goes by ("you" for the person). Never a key (card #478):
    /// "BillingEng1 · Spencer" for another member's agent, "unknown agent"
    /// when nothing names it.
    #[serde(default)]
    pub name: String,
    /// Another member's agent's board status (`busy`, `idle`, `gone`); ""
    /// when this device knows none, which isn't gone.
    #[serde(default)]
    pub status: String,
    /// Whose agent it is when it is another member's ("Spencer"), else "".
    #[serde(default)]
    pub owner: String,
    /// It is its owner's Office Assistant (kind `assistant` and the
    /// owner's profile points at it). `name` is then the label.
    #[serde(default)]
    #[specta(optional)]
    pub assistant: bool,
    /// The assistant's chosen name, "" for the default.
    #[serde(default)]
    #[specta(optional)]
    pub assistant_name: String,
}

/// Who to add to or take out of a conversation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkMemberRef {
    /// `person` or `agent`.
    pub kind: String,
    pub key: String,
}

/// One conversation in Talk's list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkConversation {
    pub conv_id: String,
    /// `thread`, `channel` or `dm`.
    pub kind: String,
    pub name: String,
    #[specta(optional)]
    pub topic: Option<String>,
    pub unread: i64,
    pub mentions: i64,
    pub members: Vec<TalkMember>,
    /// The newest post's time (the zero time when it has none).
    #[specta(type = ts::Time)]
    pub last_at: gocompat::time::GoTime,
    /// The newest post's text on one line, clipped (card #486): the list's
    /// subtitle. "" when it has none.
    pub last_text: String,
    /// Who wrote the newest post: a member's key, "" when none.
    pub last_by: String,
    /// A DM between two agents: the person reads it only.
    pub read_only: bool,
    /// A thread's "Agents answer now" (card #391): its toggle. False for a
    /// channel or DM, whose composer starts from the default for what the
    /// text names.
    pub agents_now: bool,
    /// The shared space a channel or thread is in (card #461, #558); absent
    /// for Personal and DMs. Matches the switcher's space id (its first
    /// board's).
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub space_id: String,
    /// The space's #office-assistants channel (assistants-talk.md §4): the
    /// list puts it first. Absent otherwise.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub pinned: bool,
    /// A DM with another person that hasn't reached them yet (card #538):
    /// `them` ("Waiting for Bob's Irori") or `update` ("Bob's Irori needs
    /// updating"). Absent otherwise. `name` is who they are.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub waiting: String,
}

/// Someone the person can DM (card #538): the new-DM picker's people.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkPerson {
    /// `person:<id>`, as talk_dm takes it with kind `person`.
    pub key: String,
    pub name: String,
    /// Their own colour pick, "" for none.
    pub color: String,
    /// Their avatar, "" for none.
    pub avatar: String,
}

/// One post.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkPost {
    pub post_id: String,
    pub conv_id: String,
    pub by: String,
    pub text: String,
    pub mentions: Vec<String>,
    pub agents_now: bool,
    #[specta(optional)]
    pub thread_from: Option<String>,
    /// The post it answers (an assistant's answer or waiting line).
    #[specta(optional)]
    pub reply_to: Option<String>,
    /// `ask` or `owner` on an assistant's waiting line.
    #[specta(optional)]
    pub waiting: Option<String>,
    /// The card made from it (always null in Talk 1).
    #[specta(optional)]
    pub card: Option<String>,
    /// The files sent with it (card #643), as a comment's are.
    #[specta(type = Vec<ts::FileRef>)]
    pub attachments: Vec<coord::FileRef>,
    /// A huddle's start or end (#654); null on any other post.
    #[specta(optional)]
    pub call: Option<TalkCall>,
    #[specta(type = ts::Time)]
    pub at: gocompat::time::GoTime,
}

/// What a huddle's start or end post says about it (#654).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkCall {
    pub call_id: String,
    /// `start` or `end`.
    pub op: String,
    /// How long it ran, on an end; 0 on a start.
    pub duration_ms: i64,
    /// Who took part, by name ("you" for the person), never a key. Empty
    /// on a start.
    pub people: Vec<String>,
    /// On an end: the huddle has a transcript (#656), which
    /// talk_call_transcript reads.
    pub transcript: bool,
    /// The huddle's title (card #1043); empty when it has none.
    pub title: String,
}

/// One huddle in Talk's Huddles list (card #1044), newest first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkHuddle {
    pub call_id: String,
    /// The channel or DM it was in.
    pub conv_id: String,
    /// The channel's name, or the DM's other person.
    pub conv_name: String,
    /// `channel` or `dm`.
    pub conv_kind: String,
    /// The huddle's title; empty when it has none.
    pub title: String,
    pub title_by_assistant: bool,
    /// When it started, Unix ms; null when its start post isn't here.
    pub started_at_ms: Option<i64>,
    /// When its newest row was written, Unix ms.
    pub last_at_ms: i64,
    pub ended: bool,
    /// How long it ran, once ended; 0 before.
    pub duration_ms: i64,
    /// The huddle is over as this device can tell (see
    /// [`TalkTranscript::over`]).
    pub over: bool,
    /// Who took part, by name ("you" for the person), never a key.
    pub people: Vec<String>,
    /// How many lines its transcript has.
    pub lines: i64,
}

/// A huddle's transcript (#656), as the person reads it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkTranscript {
    pub call_id: String,
    /// The channel or DM it was in.
    pub conv_id: String,
    /// The channel's name, or the DM's other person.
    pub conv_name: String,
    /// `channel`, `personal` (a Personal channel) or `dm`.
    pub conv_kind: String,
    /// Some line was said under the notice that lets agents read it (card
    /// #1047): a person DM's agents read only those lines.
    pub agents: bool,
    /// When it started, Unix ms; null when its start post isn't here.
    pub started_at_ms: Option<i64>,
    pub ended: bool,
    /// How long it ran, once ended; 0 before.
    pub duration_ms: i64,
    /// The huddle's title (card #1043); empty when it has none.
    pub title: String,
    /// Whether an assistant proposed the title, not a person.
    pub title_by_assistant: bool,
    /// The huddle is over as this device can tell (card #1043): an end
    /// mark, or a crash that left the call quiet. False when unsure.
    pub over: bool,
    /// Each person's assistant in a live huddle (card #1049): connected,
    /// reconnecting or not connected. Empty once it is over.
    pub assistants: Vec<TalkAssistant>,
    /// The canvas (card #1051), oldest first.
    pub canvas: Vec<TalkCanvasItem>,
    /// What was said, oldest first.
    pub lines: Vec<TalkTranscriptLine>,
}

/// One item on a huddle's canvas (card #1051), with the names it shows by.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkCanvasItem {
    pub item_id: String,
    /// `question`, `answer`, `decision`, `task`, `note` or `summary`.
    pub kind: String,
    pub text: String,
    /// `open`, `settled`, `dropped` or `carded`; empty for a note or summary.
    pub status: String,
    /// The item this answers, when it names one.
    pub reply: String,
    /// The items a summary judges answered or superseded.
    pub refs: Vec<String>,
    /// What woke the look that wrote it (`name`, `own`, `digest`, `routed`,
    /// `end`, `ask`), else empty.
    pub why: String,
    /// Written by a person's assistant: never shown as the person's words.
    pub assistant: bool,
    /// Who signed it.
    pub by_name: String,
    /// "Juno \u{b7} Addam's assistant" for an assistant's item, else empty.
    pub label: String,
    /// Who asked, when the item says; empty groups it under the signer.
    pub asker_name: String,
    /// The signer is this person.
    pub mine: bool,
    /// The asker is this person.
    pub asker_mine: bool,
    pub at_ms: i64,
    pub x: Option<f64>,
    pub y: Option<f64>,
    /// The ref of the card it became, once a person divided the canvas.
    pub card: String,
    /// Who made that card, by name ("" when it is not known).
    pub card_by_name: String,
    /// This person made it.
    pub card_mine: bool,
}

impl From<irori_core::CanvasEntry> for TalkCanvasItem {
    fn from(e: irori_core::CanvasEntry) -> TalkCanvasItem {
        let i = e.item;
        TalkCanvasItem {
            item_id: i.item_id,
            kind: i.kind,
            text: i.text,
            status: i.status,
            reply: i.reply.unwrap_or_default(),
            refs: i.refs,
            why: i.why.unwrap_or_default(),
            assistant: i.assistant,
            by_name: e.by_name,
            label: e.label,
            asker_name: e.asker_name,
            mine: e.mine,
            asker_mine: e.asker_mine,
            at_ms: i.at.unix_milli_value(),
            x: i.x,
            y: i.y,
            card: i.card,
            card_by_name: e.card_by_name,
            card_mine: e.card_by_mine,
        }
    }
}

/// One person's assistant in a huddle's roster (card #1049).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkAssistant {
    /// The person, as a key.
    pub person: String,
    /// The person's name ("you" for this person), never a key (card #1052).
    pub name: String,
    /// The person is this one.
    pub mine: bool,
    /// `connected`, `reconnecting` or `not connected`.
    pub state: String,
    /// Why it isn't connected, when this device knows; else empty.
    pub why: String,
}

/// One line of a huddle's transcript.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkTranscriptLine {
    /// Who said it, by name ("you" for the person), never a key.
    pub name: String,
    /// The person said it.
    pub mine: bool,
    /// Said under the notice that lets agents read it (card #1047); the
    /// roster says when a person's lines aren't (card #1052).
    pub agents: bool,
    /// When the phrase began, Unix ms.
    pub at_ms: i64,
    pub text: String,
}

/// The agents replying in one channel or DM (card #645): woken by a post
/// there and not answered yet. The row reads "jasmine and kestrel are
/// replying…".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkReplying {
    pub conv_id: String,
    /// The earliest woken first.
    pub agents: Vec<TalkReplier>,
}

/// One agent replying.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkReplier {
    pub key: String,
    /// Its name, never its key.
    pub name: String,
    /// When the post that woke it was sent: the row drops it 30 minutes on.
    #[specta(type = ts::Time)]
    pub since: gocompat::time::GoTime,
}

/// The most posts one page returns, and the default.
const MAX_LIMIT: i64 = coord::talk::MAX_PAGE as i64;
const DEFAULT_LIMIT: i64 = 50;

fn is_thread_id(conv: &str) -> bool {
    !coord::talk::is_channel(conv) && !coord::talk::is_dm(conv)
}

impl Window {
    fn me(&self) -> String {
        self.core.user_sender().key
    }

    /// Who is replying in each channel and DM (card #645); conversations
    /// with nobody left out.
    pub fn talk_replying(&self) -> Vec<TalkReplying> {
        self.core
            .talk_replying(&self.me())
            .into_iter()
            .map(|r| TalkReplying {
                conv_id: r.conv_id,
                agents: r
                    .agents
                    .into_iter()
                    .map(|a| TalkReplier {
                        key: a.key,
                        name: a.name,
                        since: a.since,
                    })
                    .collect(),
            })
            .collect()
    }

    /// Talk's list: every open channel, every DM (agents' own read-only)
    /// and every open thread, newest first.
    pub fn talk_conversations(&self) -> Result<Vec<TalkConversation>, String> {
        let cs = self.core.talk_conversations(&self.me())?;
        Ok(cs
            .into_iter()
            .map(|c| {
                let pinned = coord::talk::is_assistants_channel(&c.conv_id, &c.space_id);
                (c, pinned)
            })
            .map(|(c, pinned)| TalkConversation {
                conv_id: c.conv_id,
                kind: c.kind,
                name: c.name,
                topic: (!c.topic.is_empty()).then_some(c.topic),
                unread: c.unread as i64,
                mentions: c.mentions as i64,
                members: c
                    .members
                    .into_iter()
                    .map(|m| TalkMember {
                        kind: m.kind,
                        key: m.key,
                        name: m.name,
                        status: m.status,
                        owner: m.owner,
                        assistant: m.assistant,
                        assistant_name: m.assistant_name,
                    })
                    .collect(),
                last_at: c.last_at,
                last_text: c.last_text,
                last_by: c.last_by,
                read_only: c.read_only,
                agents_now: c.agents_now,
                pinned,
                space_id: c.space_id,
                waiting: c.waiting,
            })
            .collect())
    }

    /// Up to `limit` posts of `conv` before post `before` (none: the
    /// newest), oldest first.
    pub fn talk_posts(
        &self,
        conv: &str,
        before: Option<&str>,
        limit: i64,
    ) -> Result<Vec<TalkPost>, String> {
        let limit = if limit <= 0 {
            DEFAULT_LIMIT
        } else {
            limit.min(MAX_LIMIT)
        };
        let ps = self
            .core
            .talk_posts(&self.me(), conv, before, limit as usize)?;
        // A huddle's people by name (#654), every key on the page at once.
        let mut keys: Vec<String> = Vec::new();
        for k in ps
            .iter()
            .filter_map(|p| p.call.as_ref())
            .flat_map(|c| &c.people)
        {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
        let names: std::collections::HashMap<String, String> = keys
            .iter()
            .cloned()
            .zip(self.core.talk_names(&keys))
            .collect();
        let talk = self.core.talk();
        Ok(ps
            .into_iter()
            .map(|p| TalkPost {
                post_id: p.post_id,
                conv_id: p.conv_id,
                by: p.by,
                text: p.text,
                mentions: p.mentions,
                agents_now: p.agents_now,
                thread_from: p.thread_from,
                reply_to: p.reply_to,
                waiting: p.waiting,
                card: p.card,
                attachments: p.files.iter().map(crate::coord_ref).collect(),
                call: p.call.map(|c| TalkCall {
                    title: talk.call_title(&c.call_id).unwrap_or_default(),
                    call_id: c.call_id,
                    op: c.op,
                    duration_ms: c.duration_ms,
                    people: c
                        .people
                        .iter()
                        .filter_map(|k| names.get(k).cloned())
                        .collect(),
                    transcript: c.transcript,
                }),
                at: p.at,
            })
            .collect())
    }

    /// Every huddle in a conversation this person reads (card #1044),
    /// newest first.
    pub fn talk_huddles(&self) -> Vec<TalkHuddle> {
        let talk = self.core.talk();
        let me_board = format!(
            "{}{}",
            store::kinds::PERSON_ACTOR_PREFIX,
            self.core.chain().person_id()
        );
        let calls: Vec<_> = talk
            .calls()
            .into_iter()
            .filter(|c| {
                if coord::talk::is_channel(&c.conv_id) {
                    talk.channel(&c.conv_id).is_some()
                } else {
                    coord::talk::is_dm(&c.conv_id)
                }
            })
            .collect();
        let mut keys: Vec<String> = Vec::new();
        for c in &calls {
            if !coord::talk::is_channel(&c.conv_id)
                && let Some(k) = talk.pair_other(&c.conv_id)
                && !keys.contains(&k)
            {
                keys.push(k);
            }
            for k in &c.people {
                if !keys.contains(k) {
                    keys.push(k.clone());
                }
            }
        }
        let names: std::collections::HashMap<String, String> = keys
            .iter()
            .cloned()
            .zip(self.core.talk_names(&keys))
            .collect();
        let name = |k: &str| {
            if coord::talk::is_person_key(k) || k == me_board {
                return "you".to_string();
            }
            names
                .get(k)
                .cloned()
                .unwrap_or_else(|| tools::talk::UNKNOWN_PERSON.into())
        };
        calls
            .into_iter()
            .map(|c| {
                let channel = coord::talk::is_channel(&c.conv_id);
                let conv_name = if channel {
                    talk.channel(&c.conv_id).map(|x| x.name).unwrap_or_default()
                } else {
                    talk.pair_other(&c.conv_id)
                        .map(|k| name(&k))
                        .unwrap_or_else(|| tools::talk::UNKNOWN_PERSON.into())
                };
                TalkHuddle {
                    over: self.core.call_over(&c.call_id),
                    ended: c.end.is_some(),
                    duration_ms: c.end.as_ref().map(|m| m.duration_ms).unwrap_or(0),
                    started_at_ms: c.start_at.map(|at| at.unix_milli_value()),
                    last_at_ms: c.last.unix_milli_value(),
                    people: c.people.iter().map(|k| name(k)).collect(),
                    lines: c.lines as i64,
                    title: c.title,
                    title_by_assistant: c.title_by_assistant,
                    conv_kind: if channel { "channel" } else { "dm" }.into(),
                    conv_name,
                    conv_id: c.conv_id,
                    call_id: c.call_id,
                }
            })
            .collect()
    }

    /// Sets huddle `call_id`'s title as the person (card #1044); returns
    /// the title it now shows.
    pub fn talk_call_title_set(&self, call_id: &str, title: &str) -> Result<String, String> {
        self.core.call_title_set(crate::trim(call_id), title)
    }

    /// The canvas item `item_id` of huddle `call_id`, named (card #1051).
    fn canvas_item(&self, call_id: &str, item_id: &str) -> Result<TalkCanvasItem, String> {
        self.core
            .call_canvas(call_id)
            .into_iter()
            .find(|e| e.item.item_id == item_id)
            .map(TalkCanvasItem::from)
            .ok_or_else(|| "that item isn't on this canvas".to_string())
    }

    /// Adds an item to a huddle's canvas as the person (card #1051). `kind`
    /// is `question`, `answer`, `decision`, `task`, `note` or `summary`.
    pub fn huddle_item_add(
        &self,
        call_id: &str,
        kind: &str,
        text: &str,
    ) -> Result<TalkCanvasItem, String> {
        let id = crate::trim(call_id);
        let i = self.core.huddle_item_add(id, kind, text, "")?;
        self.canvas_item(id, &i.item_id)
    }

    /// Changes a canvas item as the person (card #1051): its text (their own
    /// item's), its status (`open`, `settled`, `dropped`; `carded` only once
    /// the huddle is over), or its place. Each field is optional.
    pub fn huddle_item_set(
        &self,
        call_id: &str,
        item_id: &str,
        text: Option<String>,
        status: Option<String>,
        x: Option<f64>,
        y: Option<f64>,
        card: Option<String>,
    ) -> Result<TalkCanvasItem, String> {
        let id = crate::trim(call_id);
        let item = crate::trim(item_id);
        self.core.huddle_item_set(
            id,
            coord::talk::NewItemSet {
                item_id: item.to_string(),
                text,
                status,
                x,
                y,
                card,
                ..coord::talk::NewItemSet::default()
            },
        )?;
        self.canvas_item(id, item)
    }

    /// Puts a question to the person's own assistant in a live huddle
    /// (card #1051): an item of theirs, and a look.
    pub fn huddle_ask(&self, call_id: &str, text: &str) -> Result<TalkCanvasItem, String> {
        let id = crate::trim(call_id);
        let i = self.core.huddle_ask(id, text)?;
        self.canvas_item(id, &i.item_id)
    }

    /// Huddle `call_id`'s transcript (#656), every line, when the person
    /// may read the conversation it was in.
    pub fn talk_call_transcript(&self, call_id: &str) -> Result<TalkTranscript, String> {
        let id = crate::trim(call_id);
        if id.is_empty() {
            return Err("pick a huddle".into());
        }
        // The read rule, `tools::talk::call_parent`'s for the person (the
        // reader here is always their own key): the huddle's conversation
        // is here, and a channel still is. A DM is always theirs to read.
        let talk = self.core.talk();
        let conv_id = talk
            .call_conv(id)
            .ok_or_else(|| "that huddle isn't here".to_string())?;
        if coord::talk::is_channel(&conv_id) && talk.channel(&conv_id).is_none() {
            return Err("that huddle's channel isn't here".into());
        }
        let lines = talk.call_posts(id);
        let (conv_kind, mut keys) = if coord::talk::is_channel(&conv_id) {
            // A Personal channel lives in no space (card #1047).
            let personal = talk
                .channel(&conv_id)
                .is_some_and(|c| c.space_id.is_empty());
            (if personal { "personal" } else { "channel" }, Vec::new())
        } else {
            ("dm", talk.pair_other(&conv_id).into_iter().collect())
        };
        for p in &lines {
            if !keys.contains(&p.by) {
                keys.push(p.by.clone());
            }
        }
        let assistants = self.core.call_assistants(id);
        for a in &assistants {
            if !keys.contains(&a.person) {
                keys.push(a.person.clone());
            }
        }
        let names: std::collections::HashMap<String, String> = keys
            .iter()
            .cloned()
            .zip(self.core.talk_names(&keys))
            .collect();
        let name = |k: &str| {
            names
                .get(k)
                .cloned()
                .unwrap_or_else(|| tools::talk::UNKNOWN_PERSON.into())
        };
        let conv_name = if conv_kind != "dm" {
            talk.channel(&conv_id).map(|c| c.name).unwrap_or_default()
        } else {
            talk.pair_other(&conv_id)
                .map(|k| name(&k))
                .unwrap_or_else(|| tools::talk::UNKNOWN_PERSON.into())
        };
        // The person as a space's rows name them, should a line keep it.
        let me_board = format!(
            "{}{}",
            store::kinds::PERSON_ACTOR_PREFIX,
            self.core.chain().person_id()
        );
        // Its start and end, from the huddle's own marks.
        let info = talk.calls().into_iter().find(|c| c.call_id == id);
        let start = info
            .as_ref()
            .and_then(|c| c.start_at)
            .map(|at| at.unix_milli_value());
        let (title, title_by_assistant) = info
            .as_ref()
            .map(|c| (c.title.clone(), c.title_by_assistant))
            .unwrap_or_default();
        let end = info.and_then(|c| c.end).map(|m| m.duration_ms);
        Ok(TalkTranscript {
            call_id: id.to_string(),
            conv_id,
            conv_name,
            conv_kind: conv_kind.into(),
            agents: lines.iter().any(|p| p.agents),
            started_at_ms: start,
            ended: end.is_some(),
            duration_ms: end.unwrap_or(0),
            title,
            title_by_assistant,
            over: self.core.call_over(id),
            assistants: assistants
                .into_iter()
                .map(|a| {
                    let mine = coord::talk::is_person_key(&a.person) || a.person == me_board;
                    TalkAssistant {
                        name: if mine { "you".into() } else { name(&a.person) },
                        mine,
                        person: a.person,
                        state: a.state,
                        why: a.why,
                    }
                })
                .collect(),
            canvas: self
                .core
                .call_canvas(id)
                .into_iter()
                .map(TalkCanvasItem::from)
                .collect(),
            lines: lines
                .into_iter()
                .map(|p| {
                    let mine = coord::talk::is_person_key(&p.by) || p.by == me_board;
                    TalkTranscriptLine {
                        name: if mine { "you".into() } else { name(&p.by) },
                        mine,
                        agents: p.agents,
                        at_ms: p.at.unix_milli_value(),
                        text: p.text,
                    }
                })
                .collect(),
        })
    }

    /// Posts as the person; returns the post's id. A thread's post is a
    /// comment on it (agents woken when `agents_now`, else told at their
    /// next turn); a channel's or DM's is a `talk.post` (a DM with an agent
    /// wakes it, an @name in the text wakes that agent). `mentions` are
    /// the keys the composer's picker recorded: one names exactly the agent
    /// picked, another person's in a space too (card #461). `files` go
    /// with it (card #643), the text then optional: a thread's with its
    /// comment, under the comment's rules; a channel's or DM's are stored
    /// here and named on the post, and in a space's channel or a DM with
    /// another person offered in that log so its members can fetch them.
    pub fn talk_post(
        &self,
        conv: &str,
        text: &str,
        mentions: &[String],
        agents_now: bool,
        thread_from: Option<String>,
        files: &[UploadFile],
    ) -> Result<String, String> {
        let conv = crate::trim(conv);
        if conv.is_empty() {
            return Err("pick a conversation".into());
        }
        if is_thread_id(conv) {
            let tell = if agents_now { "wake" } else { "next_turn" };
            let r = self.comment(conv, text, tell, &[], files)?;
            return Ok(r.comment.id);
        }
        let refs = self.store_uploads(files)?;
        let p = self.core.talk_post(
            &self.core.user_sender(),
            conv,
            text,
            mentions,
            agents_now,
            thread_from,
            refs,
        )?;
        Ok(p.post.post_id)
    }

    /// The person asks about channel `conv`'s post `post_id` in a new
    /// thread (design b2, card #454): the thread records the channel (and,
    /// in your own log, its repo, [`Window::channel_repo`]), and the channel
    /// gets a one-line pointer to it. `text` is the thread's whole ask.
    ///
    /// A space's channel starts it in the space's log, so its members and
    /// their agents get it, and the pointer carries the tag, as a bare #N
    /// reads across every log (card #558). With your threads in that space
    /// set to private it stays in your own log, and the channel gets no
    /// pointer: nobody else could open it.
    pub fn talk_thread_from(
        &self,
        conv: &str,
        post_id: &str,
        text: &str,
    ) -> Result<crate::board::TaskResult, String> {
        let (conv, post_id) = (crate::trim(conv), crate::trim(post_id));
        let channel = coord::talk::is_channel(conv)
            .then(|| self.core.talk().channel(conv))
            .flatten()
            .ok_or_else(|| format!("no channel {conv}"))?;
        if !self
            .core
            .talk()
            .posts(conv)
            .iter()
            .any(|p| p.post_id == post_id)
        {
            return Err(format!("no post {post_id} in that channel"));
        }
        let space = channel.space_id.as_str();
        let private = !space.is_empty()
            && self.core.inbox().threads_mode(space) == inbox::threads_mode::PRIVATE;
        // The pointer is refused while a device can't read the space's rows:
        // refuse first, or the thread would start with nothing to show for it.
        if !space.is_empty() && !private {
            self.core.space_gate(space)?;
        }
        let r = self.start_thread_in(
            text,
            &[],
            &[],
            space,
            None,
            ThreadFrom {
                channel: conv,
                assistant: false,
            },
        )?;
        if !private {
            let pointer = r.task.short_ref();
            self.talk_post(conv, &pointer, &[], false, Some(post_id.to_string()), &[])?;
        }
        Ok(r)
    }

    /// The repo a Talk channel is about (card #454): the one repo with a
    /// checkout on this device whose name is the channel's ("irori" for
    /// `remote:github.com/a/irori`), any case. "" when none or several
    /// match, or `conv` isn't a channel: its thread then starts where an
    /// ask naming no repo does.
    pub fn channel_repo(&self, conv: &str) -> String {
        let conv = crate::trim(conv);
        if conv.is_empty() {
            return String::new();
        }
        let Some(c) = self.core.talk().channel(conv) else {
            return String::new();
        };
        let name = c.name.trim().trim_start_matches('#').to_lowercase();
        if name.is_empty() {
            return String::new();
        }
        let hits: Vec<String> = self
            .core
            .capture()
            .repos()
            .into_iter()
            .filter(|(k, roots)| {
                k.starts_with("remote:")
                    // A root on this device: roots can name another
                    // device's checkout (a Mac path on the PC).
                    && roots.iter().any(|r| std::path::Path::new(r).is_dir())
                    && ledger::repo_name(k).to_lowercase() == name
            })
            .map(|(k, _)| ledger::canonical_repo_key(&k))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        match hits.as_slice() {
            [k] => k.clone(),
            _ => String::new(),
        }
    }

    /// The person made card `task_id` from post `post_id` (card #390,
    /// "Make a card"): the post shows the card's chip.
    pub fn talk_link_card(&self, post_id: &str, task_id: &str) -> Result<(), String> {
        self.core
            .talk_link_card(post_id, task_id, &self.me())
            .map(|_| ())
    }

    /// The person has read `conv` up to post `up_to`.
    pub fn talk_read(&self, conv: &str, up_to: &str) -> Result<(), String> {
        self.core.talk_mark_read(conv, up_to, &self.me())
    }

    /// Makes a channel the person is in, in space `space` (None or empty:
    /// Personal); returns its id. A space's channel reaches its members
    /// (card #392).
    pub fn talk_channel_create(
        &self,
        name: &str,
        topic: Option<&str>,
        space: Option<&str>,
    ) -> Result<String, String> {
        let c = self.core.talk_channel_create_in(
            &self.me(),
            name,
            topic.unwrap_or(""),
            space.unwrap_or(""),
        )?;
        Ok(c.id)
    }

    /// The person's DM with `m`, opened if it's new; returns its id for
    /// talk_posts and talk_post. `m` is one of their agents, or another
    /// person by `person:<id>` (card #538): that opens a draft, which
    /// writes nothing until the first post. Another person's agent is
    /// refused.
    pub fn talk_dm(&self, m: &TalkMemberRef) -> Result<String, String> {
        self.core.talk_dm(&self.me(), &m.key)
    }

    /// Everyone the person can DM (card #538): whoever shares a space with
    /// them, by name.
    pub fn talk_dm_people(&self) -> Vec<TalkPerson> {
        self.core
            .talk_dm_people()
            .into_iter()
            .map(|p| TalkPerson {
                key: p.key,
                name: p.name,
                color: p.color,
                avatar: p.avatar,
            })
            .collect()
    }

    /// Leaves DM `conv` with another person (card #538): what's in it goes
    /// from all the person's devices. A later DM from either side starts a
    /// new one.
    pub fn talk_dm_leave(&self, conv: &str) -> Result<(), String> {
        self.core.talk_dm_leave(conv)
    }

    /// Adds `m` to a channel or DM, as the person.
    pub fn talk_join(&self, conv: &str, m: &TalkMemberRef) -> Result<(), String> {
        self.core
            .talk_join(conv, &member(m), &self.me())
            .map(|_| ())
    }

    /// Takes `m` out of a channel or DM, as the person.
    pub fn talk_leave(&self, conv: &str, m: &TalkMemberRef) -> Result<(), String> {
        self.core
            .talk_leave(conv, &member(m), &self.me())
            .map(|_| ())
    }
}

fn member(m: &TalkMemberRef) -> coord::talk::Member {
    coord::talk::Member {
        kind: m.kind.clone(),
        key: m.key.clone(),
    }
}
