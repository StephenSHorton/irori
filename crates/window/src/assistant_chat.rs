//! The Office Assistant's one conversation as the chat reads it (card
//! #1102, docs/design/one-assistant.md §4): the Core's fold of the private
//! `assistant.*` rows, the same on every one of the person's devices, a
//! page at a time. `assistant:changed` says the fold changed; the chat
//! reads the list again.
//!
//! A question's screen snapshot stays out of the list: only the assistant's
//! computer needs it, for the turn.

use irori_core::assistant_chat::{self as chat, ChatEntry};
use serde::Serialize;
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
    /// The files that came with it.
    pub files: Vec<crate::assistant::AssistantFile>,
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
                files: a
                    .files
                    .into_iter()
                    .map(|f| crate::assistant::AssistantFile {
                        sha256: f.sha256,
                        name: f.name,
                        size: f.size,
                        mime: f.mime,
                        device: f.device,
                        can_read: f.can_read,
                    })
                    .collect(),
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

    /// The person posted a reply's draft as a thread (Post thread): every
    /// device then hides the button.
    pub fn assistant_chat_posted(
        &self,
        turn: &str,
        id: &str,
        short: &str,
        tag: &str,
    ) -> Result<(), String> {
        self.core
            .assistant_chat_write(chat::ChatRow::Posted(chat::PostedThread {
                turn: turn.to_string(),
                posted: chat::Posted {
                    id: id.to_string(),
                    short: short.to_string(),
                    tag: tag.to_string(),
                },
            }))
            .map(|_| ())
    }

    /// New chat: hides every turn, line and draft older than this row.
    pub fn assistant_chat_clear(&self) -> Result<(), String> {
        let by = format!("user:{}", self.core.device_id());
        self.core
            .assistant_chat_write(chat::ChatRow::Cleared(chat::Cleared {
                by,
                at: self.core.assistant_chat_now(),
            }))
            .map(|_| ())
    }

    /// A follow-up line the chat says about a thread it posted (the first
    /// row per `key` wins on every device).
    pub fn assistant_chat_update(&self, u: AssistantChatUpdateIn) -> Result<(), String> {
        let at = self.core.assistant_chat_now();
        self.core
            .assistant_chat_write(chat::ChatRow::Update(chat::Update {
                key: u.key,
                thread: u.thread,
                text: u.text,
                act: u.act.map(|a| chat::Act {
                    kind: a.kind,
                    card: a.card,
                    blocker: a.blocker,
                }),
                at,
                ..chat::Update::default()
            }))
            .map(|_| ())
    }
}

/// A follow-up line to write (see [`Window::assistant_chat_update`]).
#[derive(Debug, Clone, serde::Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantChatUpdateIn {
    pub key: String,
    pub thread: String,
    pub text: String,
    pub act: Option<AssistantChatActIn>,
}

#[derive(Debug, Clone, serde::Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantChatActIn {
    pub kind: String,
    pub card: String,
    pub blocker: String,
}
