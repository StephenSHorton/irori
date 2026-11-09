//! The Office Assistant's one conversation as the chat reads it (card
//! #1102, docs/design/one-assistant.md §4): the Core's fold of the private
//! `assistant.*` rows, the same on every one of the person's devices, a
//! page at a time. `assistant:changed` says the fold changed; the chat
//! reads the list again.
//!
//! A question's screen snapshot stays out of the list: only the assistant's
//! computer needs it, for the turn.

use irori_core::assistant_chat::{self as chat, ChatEntry};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Window;

/// `assistant:changed`: a row of the assistant's conversation folded, here
/// or on another of the person's devices. It never carries a reply's text.
pub const EVENT_ASSISTANT_CHANGED: &str = "assistant:changed";

/// A thread a reply posted, or the person posted from its draft.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantChatPosted {
    pub id: String,
    pub short: String,
    /// Its board's tag: numbers are per board.
    pub tag: String,
}

/// The assistant's reply to a question.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantChatReply {
    pub text: String,
    /// What its write tools changed, a line each (card #987).
    pub changed: Vec<String>,
    /// The label of the screen it looked at.
    pub looked: String,
    pub error: String,
    /// The assistant's computer didn't get to it.
    pub skipped: bool,
    /// The assistant's computer.
    pub device: String,
    /// Unix milliseconds.
    pub at: i64,
}

/// One question and its reply, once there is one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantChatTurn {
    pub turn: String,
    pub text: String,
    /// The device the person typed on.
    pub device: String,
    /// Unix milliseconds on that device.
    pub at: i64,
    /// The turn this one asks again.
    pub again: String,
    /// Brought in from the assistant's computer's old history.
    pub imported: bool,
    pub call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub reply: Option<AssistantChatReply>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub posted: Option<AssistantChatPosted>,
    /// The person stopped it.
    pub stopped: bool,
}

/// What a follow-up line lets the person do.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantChatAct {
    /// Today `answer`.
    pub kind: String,
    pub card: String,
    pub blocker: String,
}

/// A follow-up line about a thread the chat posted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantChatUpdate {
    pub key: String,
    pub thread: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub act: Option<AssistantChatAct>,
    pub at: i64,
    pub imported: bool,
}

/// An owner's-OK draft of an answer to another person.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantChatDraft {
    pub request: String,
    pub post: String,
    pub space: String,
    pub text: String,
    pub device: String,
    pub at: i64,
}

/// One entry of the conversation: exactly one of `turn`, `update` and
/// `draft`, as `kind` says.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantChatItem {
    /// Pass it as `before` for the entries older than this one.
    pub cursor: String,
    /// `turn`, `update` or `draft`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub turn: Option<AssistantChatTurn>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub update: Option<AssistantChatUpdate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub draft: Option<AssistantChatDraft>,
}

/// A page of the conversation, newest first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantChatPage {
    pub items: Vec<AssistantChatItem>,
    /// Older entries remain.
    pub more: bool,
}

fn posted(p: chat::Posted) -> AssistantChatPosted {
    AssistantChatPosted {
        id: p.id,
        short: p.short,
        tag: p.tag,
    }
}

fn item(i: chat::ChatItem) -> AssistantChatItem {
    let mut out = AssistantChatItem {
        cursor: i.cursor,
        ..AssistantChatItem::default()
    };
    match i.entry {
        ChatEntry::Turn(t) => {
            out.kind = "turn".into();
            let a = t.asked;
            out.turn = Some(AssistantChatTurn {
                turn: a.turn,
                text: a.text,
                device: if a.device.is_empty() {
                    t.asked_on
                } else {
                    a.device
                },
                at: a.at,
                again: a.again,
                imported: a.imported,
                call_id: a.call_id,
                reply: t.said.map(|s| AssistantChatReply {
                    text: s.text,
                    changed: s.changed,
                    looked: s.looked,
                    error: s.error,
                    skipped: s.skipped,
                    device: s.device,
                    at: s.at,
                }),
                posted: t.posted.map(posted),
                stopped: t.stopped,
            });
        }
        ChatEntry::Update(u) => {
            out.kind = "update".into();
            out.update = Some(AssistantChatUpdate {
                key: u.key,
                thread: u.thread,
                text: u.text,
                act: u.act.map(|a| AssistantChatAct {
                    kind: a.kind,
                    card: a.card,
                    blocker: a.blocker,
                }),
                at: u.at,
                imported: u.imported,
            });
        }
        ChatEntry::Draft(d) => {
            out.kind = "draft".into();
            out.draft = Some(AssistantChatDraft {
                request: d.request,
                post: d.post,
                space: d.space,
                text: d.text,
                device: d.device,
                at: d.at,
            });
        }
    }
    out
}

impl Window {
    /// A page of the assistant's conversation, newest first: up to `limit`
    /// entries (0 is 50) older than the one whose cursor is `before` (""
    /// starts at the newest).
    pub fn assistant_chat_list(
        &self,
        before: &str,
        limit: usize,
    ) -> Result<AssistantChatPage, String> {
        let p = self.core.assistant_chat_list(before, limit)?;
        Ok(AssistantChatPage {
            items: p.items.into_iter().map(item).collect(),
            more: p.more,
        })
    }
}

/// A thread an old history entry posted.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct AssistantImportPosted {
    pub id: String,
    pub short: String,
    pub tag: String,
}

/// What an old follow-up line lets the person do.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct AssistantImportAct {
    pub kind: String,
    pub card: String,
    pub blocker: String,
}

/// One entry of the old localStorage history (`ChatEntry` in
/// `lib/assistant.ts`), for `assistant_chat_import`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct AssistantImportEntry {
    /// `person` or `assistant`.
    pub role: String,
    pub text: String,
    pub note: bool,
    pub changed: bool,
    pub posted: Option<AssistantImportPosted>,
    /// The thread an assistant line follows.
    pub update: String,
    pub act: Option<AssistantImportAct>,
    pub to: String,
    /// The text the reply read off the screen; only its label is kept.
    pub looked: String,
}

/// What `assistant_chat_import` wrote.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantImported {
    pub turns: u32,
    pub updates: u32,
}

impl From<AssistantImportEntry> for chat::ImportEntry {
    fn from(e: AssistantImportEntry) -> Self {
        let looked = if e.looked.trim().is_empty() {
            String::new()
        } else {
            let l = crate::assistant::screen_label(&e.looked);
            if l.is_empty() {
                e.looked
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .chars()
                    .take(90)
                    .collect()
            } else {
                l
            }
        };
        chat::ImportEntry {
            role: e.role,
            text: e.text,
            note: e.note,
            changed: e.changed,
            posted: e.posted.map(|p| chat::Posted {
                id: p.id,
                short: p.short,
                tag: p.tag,
            }),
            update: e.update,
            act: e.act.map(|a| chat::Act {
                kind: a.kind,
                card: a.card,
                blocker: a.blocker,
            }),
            to: e.to,
            looked,
        }
    }
}

impl Window {
    /// Brings this computer's old chat history in, once (card #1105).
    pub fn assistant_chat_import(
        &self,
        entries: Vec<AssistantImportEntry>,
    ) -> Result<AssistantImported, String> {
        let entries: Vec<chat::ImportEntry> = entries.into_iter().map(Into::into).collect();
        let d = self.core.assistant_chat_import(&entries)?;
        Ok(AssistantImported {
            turns: d.turns as u32,
            updates: d.updates as u32,
        })
    }
}
