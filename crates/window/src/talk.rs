//! Talk (card #389; docs/design/talk-channels.md): the person's channels,
//! DMs and threads, read and written through the Core's Talk calls. A
//! thread's posts are its comments, so posting to one is the comment box's
//! call ([`Window::comment`]).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Window;
use crate::ts;

/// One side of a conversation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkMember {
    /// `person` or `agent`.
    pub kind: String,
    pub key: String,
    /// What it goes by ("you" for the person).
    #[serde(default)]
    pub name: String,
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
    /// A DM between two agents: the person reads it only.
    pub read_only: bool,
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
    /// The card made from it (always null in Talk 1).
    #[specta(optional)]
    pub card: Option<String>,
    #[specta(type = ts::Time)]
    pub at: gocompat::time::GoTime,
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

    /// Talk's list: every open channel, every DM (agents' own read-only)
    /// and every open thread, newest first.
    pub fn talk_conversations(&self) -> Result<Vec<TalkConversation>, String> {
        let cs = self.core.talk_conversations(&self.me())?;
        Ok(cs
            .into_iter()
            .map(|c| TalkConversation {
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
                    })
                    .collect(),
                last_at: c.last_at,
                read_only: c.read_only,
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
                card: p.card,
                at: p.at,
            })
            .collect())
    }

    /// Posts as the person; returns the post's id. A thread's post is a
    /// comment on it (agents woken when `agents_now`, else told at their
    /// next turn); a channel's or DM's is a `talk.post` (a DM with an agent
    /// wakes it, an @name in the text wakes that agent).
    pub fn talk_post(
        &self,
        conv: &str,
        text: &str,
        agents_now: bool,
        thread_from: Option<String>,
    ) -> Result<String, String> {
        let conv = crate::trim(conv);
        if conv.is_empty() {
            return Err("pick a conversation".into());
        }
        if is_thread_id(conv) {
            let tell = if agents_now { "wake" } else { "next_turn" };
            let r = self.comment(conv, text, tell, &[], &[])?;
            return Ok(r.comment.id);
        }
        let p = self.core.talk_post(
            &self.core.user_sender(),
            conv,
            text,
            &[],
            agents_now,
            thread_from,
        )?;
        Ok(p.post.post_id)
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

    /// Makes a channel the person is in; returns its id.
    pub fn talk_channel_create(&self, name: &str, topic: Option<&str>) -> Result<String, String> {
        let c = self
            .core
            .talk_channel_create(&self.me(), name, topic.unwrap_or(""))?;
        Ok(c.id)
    }

    /// The person's DM with `m` (an agent), opened if it's new; returns its
    /// id for talk_posts and talk_post.
    pub fn talk_dm(&self, m: &TalkMemberRef) -> Result<String, String> {
        let me = self.me();
        self.core
            .talk_open_dm(&coord::talk::Member::of(&me), &member(m), &me)
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
